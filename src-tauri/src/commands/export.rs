//! Clip export and rendering commands.

use crate::cardboard_caption::{self, CaptionRequest as CardboardCaptionRequest};
use crate::commands::vod::{
    expected_clip_recognition_native, find_ffmpeg, generate_srt_for_clip, generate_thumbnail,
    run_clip_transcription_native,
};
use crate::db;
use crate::error::AppError;
use crate::image_glyph_caption::{self, CaptionRequest as ImageGlyphCaptionRequest};
use crate::job_queue::JobQueue;
use crate::report_error;
use crate::undead_legion::{self, CaptionRequest as UndeadLegionCaptionRequest};
use crate::vertical_crop;
use crate::DbConn;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;
use tauri::{AppHandle, Manager, State};

static ACTIVE_CLIP_EXPORTS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

const EXPORT_PIPELINE_REVISION: &str = "immutable-artifacts-v2-captions";
const PAPER_MISCHIEF_RENDERER_VERSION: &str = "paper-mischief-image-v1";
pub(crate) const CAPTION_PIPELINE_VERSION: i64 = 3;
const CAPTION_EDGE_PADDING_SECONDS: f64 = 0.35;
const FRAME_SAFE_CAPTION_SECONDS: f64 = 0.04;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperMischiefCaptionRequest {
    pub text: String,
    pub target_width: u32,
    pub target_height: u32,
    pub font_size: u32,
    pub anchor_y: i32,
    pub alignment: i32,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperMischiefCaptionAsset {
    pub path: String,
    pub renderer_version: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionAlignmentResult {
    pub srt: Option<String>,
    pub cue_count: usize,
    pub provenance: String,
    pub pipeline_version: i64,
    pub source_start: Option<f64>,
    pub captions_enabled: bool,
    pub audio_mode: String,
    pub language: Option<String>,
    pub audio_stream: Option<String>,
    pub model_used: Option<String>,
    pub changed: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptionAlignmentAction {
    Reuse,
    PreserveEdited,
    Align,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportArtifact {
    pub path: String,
    pub revision: String,
    pub aspect_ratio: String,
    pub width: u32,
    pub height: u32,
}

struct ClipExportLease {
    clip_id: String,
}

impl Drop for ClipExportLease {
    fn drop(&mut self) {
        let active = ACTIVE_CLIP_EXPORTS.get_or_init(|| Mutex::new(HashSet::new()));
        match active.lock() {
            Ok(mut clip_ids) => {
                clip_ids.remove(&self.clip_id);
            }
            Err(poisoned) => {
                poisoned.into_inner().remove(&self.clip_id);
            }
        }
    }
}

fn acquire_clip_export_lease(clip_id: &str) -> Result<ClipExportLease, String> {
    let active = ACTIVE_CLIP_EXPORTS.get_or_init(|| Mutex::new(HashSet::new()));
    let mut clip_ids = active
        .lock()
        .map_err(|_| "Export state is unavailable".to_string())?;
    if !clip_ids.insert(clip_id.to_string()) {
        return Err("An export for this clip is already queued or running.".to_string());
    }
    Ok(ClipExportLease {
        clip_id: clip_id.to_string(),
    })
}

fn export_root() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clipviral")
        .join("exports")
}

fn safe_path_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn aspect_file_tag(aspect_ratio: &str) -> Result<&'static str, String> {
    match aspect_ratio {
        "9:16" => Ok("9x16"),
        "16:9" => Ok("16x9"),
        "1:1" => Ok("1x1"),
        other => Err(format!("Unsupported export aspect ratio: {other}")),
    }
}

fn artifact_filename(aspect_ratio: &str, revision: &str) -> Result<String, String> {
    Ok(format!("{}-{revision}.mp4", aspect_file_tag(aspect_ratio)?))
}

fn file_identity(path: &std::path::Path) -> serde_json::Value {
    match std::fs::metadata(path) {
        Ok(metadata) => {
            let modified_nanos = metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos().to_string());
            serde_json::json!({
                "path": path.to_string_lossy(),
                "length": metadata.len(),
                "modifiedNanos": modified_nanos,
            })
        }
        Err(_) => serde_json::json!({
            "path": path.to_string_lossy(),
            "missing": true,
        }),
    }
}

fn export_revision(
    clip: &db::ClipRow,
    vod: Option<&db::VodRow>,
    media_path: &str,
    allow_per_clip_override: bool,
) -> Result<String, String> {
    let mut clip_snapshot = clip.clone();
    clip_snapshot.render_status.clear();
    clip_snapshot.output_path = None;
    clip_snapshot.thumbnail_path = None;
    clip_snapshot.publish_description = None;
    clip_snapshot.publish_hashtags = None;

    let branding_identity = clip
        .context_background_path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .map(std::path::Path::new)
        .map(file_identity);
    let pipeline_revision = match (clip.captions_enabled == 1, clip.caption_style.as_str()) {
        (true, "bold-white") => {
            format!(
                "{EXPORT_PIPELINE_REVISION}+{}",
                cardboard_caption::RENDERER_VERSION
            )
        }
        (true, "paper-mischief") => {
            format!("{EXPORT_PIPELINE_REVISION}+{PAPER_MISCHIEF_RENDERER_VERSION}")
        }
        (true, "undead-legion") => {
            format!(
                "{EXPORT_PIPELINE_REVISION}+{}",
                undead_legion::RENDERER_VERSION
            )
        }
        (true, style_id) if image_glyph_caption::renderer_version(style_id).is_some() => format!(
            "{EXPORT_PIPELINE_REVISION}+{}",
            image_glyph_caption::renderer_version(style_id).unwrap_or("image-glyph")
        ),
        _ => EXPORT_PIPELINE_REVISION.to_string(),
    };
    let payload = serde_json::json!({
        "pipelineRevision": pipeline_revision,
        "clip": clip_snapshot,
        "vodCamRegion": vod.and_then(|row| row.cam_region_norm.as_deref()),
        "allowPerClipCamRegionOverride": allow_per_clip_override,
        "source": file_identity(std::path::Path::new(media_path)),
        "branding": branding_identity,
    });
    let bytes = serde_json::to_vec(&payload)
        .map_err(|error| format!("Failed to snapshot export settings: {error}"))?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn prepare_export_artifact(
    clip: &db::ClipRow,
    vod: Option<&db::VodRow>,
    media_path: &str,
    allow_per_clip_override: bool,
) -> Result<(ExportArtifact, std::path::PathBuf), String> {
    let revision = export_revision(clip, vod, media_path, allow_per_clip_override)?;
    let aspect_tag = aspect_file_tag(&clip.aspect_ratio)?;
    let target = vertical_crop::Platform::from_aspect_ratio(&clip.aspect_ratio).resolution();
    let clip_dir = export_root().join(safe_path_component(&clip.id));
    std::fs::create_dir_all(&clip_dir)
        .map_err(|error| format!("Failed to create export directory: {error}"))?;
    let output_path = clip_dir.join(artifact_filename(&clip.aspect_ratio, &revision)?);
    let temp_path = clip_dir.join(format!(".{aspect_tag}-{revision}.rendering.mp4"));
    Ok((
        ExportArtifact {
            path: output_path.to_string_lossy().to_string(),
            revision,
            aspect_ratio: clip.aspect_ratio.clone(),
            width: target.width,
            height: target.height,
        },
        temp_path,
    ))
}

fn artifact_file_is_ready(path: &std::path::Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
}

fn finalize_artifact(
    temp_path: &std::path::Path,
    output_path: &std::path::Path,
) -> Result<(), String> {
    if !artifact_file_is_ready(temp_path) {
        return Err("FFmpeg completed without producing a usable video file".to_string());
    }
    if artifact_file_is_ready(output_path) {
        let _ = std::fs::remove_file(temp_path);
        return Ok(());
    }
    std::fs::rename(temp_path, output_path)
        .map_err(|error| format!("Failed to finalize rendered video: {error}"))
}

fn persist_export_success(
    conn: &rusqlite::Connection,
    clip: &db::ClipRow,
    artifact: &ExportArtifact,
) {
    db::update_clip_render_status(conn, &clip.id, "completed", Some(&artifact.path)).ok();
    let metadata = serde_json::json!({
        "aspectRatio": &artifact.aspect_ratio,
        "artifactRevision": &artifact.revision,
    })
    .to_string();
    let dedupe_key = format!(
        "export:{}:{:.1}:{:.1}:{}:{}",
        clip.id, clip.start_seconds, clip.end_seconds, artifact.aspect_ratio, artifact.revision,
    );
    let _ = db::record_clip_behavior(
        conn,
        &clip.id,
        "export",
        Some(0.82),
        0.45,
        None,
        None,
        Some(clip.start_seconds),
        Some(clip.end_seconds),
        Some(&metadata),
        &dedupe_key,
    );
}

fn caption_alignment_action(
    clip: &db::ClipRow,
    saved_aligned_text: Option<&str>,
    analysis_draft_text: Option<&str>,
) -> CaptionAlignmentAction {
    let text = clip.captions_text.as_deref().unwrap_or("");
    let has_valid_cue = valid_srt_cue_count(text) > 0;
    match db::normalize_captions_provenance(&clip.captions_provenance) {
        "edited" => CaptionAlignmentAction::PreserveEdited,
        "aligned"
            if clip.captions_pipeline_version >= CAPTION_PIPELINE_VERSION && has_valid_cue =>
        {
            CaptionAlignmentAction::Reuse
        }
        "aligned" | "analysis-draft" => CaptionAlignmentAction::Align,
        "none" => {
            if clip.captions_enabled == 1 {
                CaptionAlignmentAction::Align
            } else {
                CaptionAlignmentAction::Reuse
            }
        }
        _ if text.trim().is_empty() => {
            if clip.captions_enabled == 1 {
                CaptionAlignmentAction::Align
            } else {
                CaptionAlignmentAction::Reuse
            }
        }
        _ if saved_aligned_text.is_some_and(|saved| saved.trim() == text.trim())
            || analysis_draft_text.is_some_and(|draft| draft.trim() == text.trim()) =>
        {
            CaptionAlignmentAction::Align
        }
        _ => CaptionAlignmentAction::PreserveEdited,
    }
}

fn generated_recognition_is_current(
    clip: &db::ClipRow,
    expected: &crate::whisper::RecognitionProvenance,
) -> bool {
    matches!(
        db::normalize_captions_provenance(&clip.captions_provenance),
        "aligned" | "none"
    ) && clip
        .captions_recognition_signature
        .as_deref()
        .is_some_and(|signature| signature == expected.signature)
}

fn caption_result(
    clip: &db::ClipRow,
    changed: bool,
    message: Option<String>,
) -> CaptionAlignmentResult {
    let srt = clip
        .captions_text
        .clone()
        .filter(|text| !text.trim().is_empty());
    CaptionAlignmentResult {
        cue_count: srt.as_deref().map(valid_srt_cue_count).unwrap_or(0),
        srt,
        provenance: db::normalize_captions_provenance(&clip.captions_provenance).to_string(),
        pipeline_version: clip.captions_pipeline_version.max(0),
        source_start: clip.captions_source_start,
        captions_enabled: clip.captions_enabled == 1,
        audio_mode: db::normalize_caption_audio_mode(&clip.caption_audio_mode).to_string(),
        language: clip.captions_language.clone(),
        audio_stream: clip.caption_audio_stream.clone(),
        model_used: None,
        changed,
        message,
    }
}

fn caption_result_with_model(
    clip: &db::ClipRow,
    changed: bool,
    message: Option<String>,
    model_used: &str,
) -> CaptionAlignmentResult {
    let mut result = caption_result(clip, changed, message);
    result.model_used = Some(model_used.to_string());
    result
}

#[derive(Debug)]
struct CaptionSource {
    media_path: String,
    desired_start: f64,
    desired_end: f64,
    padded_start: f64,
    padded_end: f64,
}

fn resolve_caption_source(
    conn: &rusqlite::Connection,
    clip: &db::ClipRow,
) -> Result<CaptionSource, String> {
    let imported_source = clip
        .source_media_path
        .as_deref()
        .filter(|path| !path.trim().is_empty());
    let community_source = clip
        .community_clip_mp4_path
        .as_deref()
        .filter(|path| !path.trim().is_empty());

    let (media_path, mut desired_start, mut desired_end) = if let Some(path) = imported_source {
        (path.to_string(), clip.start_seconds, clip.end_seconds)
    } else if let Some(path) = community_source {
        let path_buf = std::path::Path::new(path);
        let duration = probe_media_duration(path_buf).unwrap_or(clip.end_seconds.max(0.1));
        (path.to_string(), 0.0, duration)
    } else {
        let vod = db::get_vod_by_id(conn, &clip.vod_id)
            .map_err(|error| format!("DB error: {error}"))?
            .ok_or_else(|| "VOD not found".to_string())?;
        (
            vod.local_path
                .ok_or_else(|| "VOD not downloaded".to_string())?,
            clip.start_seconds,
            clip.end_seconds,
        )
    };

    let media = std::path::Path::new(&media_path);
    if !media.is_file() {
        return Err(format!("The source video is missing: {}", media.display()));
    }
    let media_duration = probe_media_duration(media);
    desired_start = desired_start.max(0.0);
    if let Some(duration) = media_duration {
        desired_start = desired_start.min(duration);
        desired_end = desired_end.min(duration);
    }
    if !desired_start.is_finite() || !desired_end.is_finite() || desired_end <= desired_start {
        return Err("The clip has no valid audio range to transcribe".to_string());
    }
    let padded_start = (desired_start - CAPTION_EDGE_PADDING_SECONDS).max(0.0);
    let padded_end = media_duration
        .map(|duration| (desired_end + CAPTION_EDGE_PADDING_SECONDS).min(duration))
        .unwrap_or(desired_end + CAPTION_EDGE_PADDING_SECONDS);

    Ok(CaptionSource {
        media_path,
        desired_start,
        desired_end,
        padded_start,
        padded_end,
    })
}

fn caption_snapshot_unchanged(current: &db::ClipRow, snapshot: &db::ClipRow) -> bool {
    current.captions_text == snapshot.captions_text
        && current.captions_enabled == snapshot.captions_enabled
        && (current.start_seconds - snapshot.start_seconds).abs() < 0.001
        && (current.end_seconds - snapshot.end_seconds).abs() < 0.001
        && db::normalize_captions_provenance(&current.captions_provenance)
            == db::normalize_captions_provenance(&snapshot.captions_provenance)
        && db::normalize_caption_audio_mode(&current.caption_audio_mode)
            == db::normalize_caption_audio_mode(&snapshot.caption_audio_mode)
}

async fn ensure_clip_captions_aligned_impl(
    db_path: std::path::PathBuf,
    clip_id: &str,
    force: bool,
    audio_mode_override: Option<&str>,
) -> Result<CaptionAlignmentResult, String> {
    let (snapshot, source, audio_mode, action) = {
        let conn =
            rusqlite::Connection::open(&db_path).map_err(|error| format!("DB error: {error}"))?;
        let clip = db::get_clip_by_id(&conn, clip_id)
            .map_err(|error| format!("DB error: {error}"))?
            .ok_or_else(|| "Clip not found".to_string())?;
        let saved_aligned_text = db::get_setting(&conn, &format!("clip_{}_captions", clip_id))
            .map_err(|error| format!("DB error: {error}"))?;
        let auto_path: Option<String> = conn
            .query_row(
                "SELECT auto_captions_path FROM clips WHERE id = ?1",
                [clip_id],
                |row| row.get(0),
            )
            .ok()
            .flatten();
        let analysis_draft_text = auto_path
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok());
        let mut action = if force {
            CaptionAlignmentAction::Align
        } else {
            caption_alignment_action(
                &clip,
                saved_aligned_text.as_deref(),
                analysis_draft_text.as_deref(),
            )
        };
        let audio_mode = db::normalize_caption_audio_mode(
            audio_mode_override.unwrap_or(&clip.caption_audio_mode),
        )
        .to_string();
        let mut resolved_source = None;
        let generated_recipe_recorded = matches!(
            db::normalize_captions_provenance(&clip.captions_provenance),
            "aligned" | "none"
        ) && clip.captions_recognition_signature.is_some();
        if action == CaptionAlignmentAction::Reuse && generated_recipe_recorded {
            let source = resolve_caption_source(&conn, &clip)?;
            let expected = expected_clip_recognition_native(
                &source.media_path,
                source.padded_start,
                source.padded_end,
                &audio_mode,
                Some(&clip.vod_id),
                clip.game.as_deref(),
            )
            .map_err(|error| error.to_string())?;
            let signature_matches = generated_recognition_is_current(&clip, &expected);
            if !signature_matches {
                action = CaptionAlignmentAction::Align;
                resolved_source = Some(source);
            }
        }

        if action == CaptionAlignmentAction::Reuse {
            if clip.captions_provenance == "legacy"
                && clip
                    .captions_text
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
            {
                conn.execute(
                    "UPDATE clips SET captions_provenance = 'none', captions_enabled = 0 WHERE id = ?1",
                    [clip_id],
                )
                .map_err(|error| format!("DB error: {error}"))?;
                let mut updated = clip;
                updated.captions_provenance = "none".to_string();
                updated.captions_enabled = 0;
                return Ok(caption_result(&updated, true, None));
            }
            return Ok(caption_result(&clip, false, None));
        }
        if action == CaptionAlignmentAction::PreserveEdited {
            if clip.captions_provenance == "legacy" {
                conn.execute(
                    "UPDATE clips SET captions_provenance = 'edited', captions_pipeline_version = 0 WHERE id = ?1",
                    [clip_id],
                )
                .map_err(|error| format!("DB error: {error}"))?;
                let mut updated = clip;
                updated.captions_provenance = "edited".to_string();
                updated.captions_pipeline_version = 0;
                return Ok(caption_result(&updated, true, None));
            }
            return Ok(caption_result(&clip, false, None));
        }

        let source = match resolved_source {
            Some(source) => source,
            None => resolve_caption_source(&conn, &clip)?,
        };
        (clip, source, audio_mode, action)
    };
    debug_assert_eq!(action, CaptionAlignmentAction::Align);

    log::info!(
        "[Captions] Aligning clip {} with padded range {:.2}-{:.2}s (kept range {:.2}-{:.2}s, audio={})",
        snapshot.id,
        source.padded_start,
        source.padded_end,
        source.desired_start,
        source.desired_end,
        audio_mode,
    );
    let media_path = source.media_path.clone();
    let padded_start = source.padded_start;
    let padded_end = source.padded_end;
    let task_audio_mode = audio_mode.clone();
    let task_vod_id = snapshot.vod_id.clone();
    let task_game = snapshot.game.clone();
    let transcript = tokio::task::spawn_blocking(move || {
        run_clip_transcription_native(
            &media_path,
            padded_start,
            padded_end,
            &task_audio_mode,
            Some(&task_vod_id),
            task_game.as_deref(),
        )
    })
    .await
    .map_err(|error| format!("Caption transcription task failed: {error}"))?
    .map_err(|error| error.to_string())?;
    let recognition = transcript
        .recognition
        .clone()
        .ok_or_else(|| "Caption transcription returned no recognition provenance".to_string())?;
    let model_used = recognition.model.clone();

    let temp_srt_path = std::env::temp_dir().join(format!(
        "clipgoblin-caption-align-{}.srt",
        uuid::Uuid::new_v4()
    ));
    let relative_start = source.desired_start - source.padded_start;
    let relative_end = source.desired_end - source.padded_start;
    let cue_count =
        generate_srt_for_clip(&transcript, relative_start, relative_end, &temp_srt_path)?;
    let srt_text = std::fs::read_to_string(&temp_srt_path)
        .map_err(|error| format!("Could not read generated subtitles: {error}"))?;
    let _ = std::fs::remove_file(&temp_srt_path);

    let conn =
        rusqlite::Connection::open(&db_path).map_err(|error| format!("DB error: {error}"))?;
    let current = db::get_clip_by_id(&conn, clip_id)
        .map_err(|error| format!("DB error: {error}"))?
        .ok_or_else(|| "Clip not found".to_string())?;
    if !caption_snapshot_unchanged(&current, &snapshot) {
        return Ok(caption_result_with_model(
            &current,
            false,
            Some("Caption changes made during alignment were preserved.".to_string()),
            &model_used,
        ));
    }

    if cue_count == 0 || srt_text.trim().is_empty() {
        conn.execute(
            "UPDATE clips
                SET captions_enabled = 0,
                    captions_text = NULL,
                    captions_source_start = NULL,
                    captions_provenance = 'none',
                    captions_pipeline_version = ?1,
                    caption_audio_mode = ?2,
                    captions_recognition_signature = ?3,
                    captions_language = ?4,
                    caption_audio_stream = ?5,
                    render_status = 'pending'
              WHERE id = ?6",
            rusqlite::params![
                CAPTION_PIPELINE_VERSION,
                audio_mode,
                recognition.signature,
                recognition.resolved_language,
                recognition.audio_stream,
                clip_id,
            ],
        )
        .map_err(|error| format!("DB error: {error}"))?;
        let mut updated = current;
        updated.captions_enabled = 0;
        updated.captions_text = None;
        updated.captions_source_start = None;
        updated.captions_provenance = "none".to_string();
        updated.captions_pipeline_version = CAPTION_PIPELINE_VERSION;
        updated.caption_audio_mode = audio_mode;
        updated.captions_recognition_signature = Some(recognition.signature);
        updated.captions_language = Some(recognition.resolved_language);
        updated.caption_audio_stream = Some(recognition.audio_stream);
        return Ok(caption_result_with_model(
            &updated,
            true,
            Some("No spoken subtitle cues were found. Captions were left off.".to_string()),
            &model_used,
        ));
    }

    let captions_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clipviral")
        .join("captions");
    std::fs::create_dir_all(&captions_dir)
        .map_err(|error| format!("Could not create the captions folder: {error}"))?;
    let srt_path = captions_dir.join(format!("{}.srt", clip_id));
    std::fs::write(&srt_path, &srt_text)
        .map_err(|error| format!("Could not save aligned subtitles: {error}"))?;
    db::save_setting(&conn, &format!("clip_{}_captions", clip_id), &srt_text)
        .map_err(|error| format!("DB error: {error}"))?;
    conn.execute(
        "UPDATE clips
            SET captions_enabled = 1,
                captions_text = ?1,
                captions_source_start = ?2,
                captions_provenance = 'aligned',
                captions_pipeline_version = ?3,
                caption_audio_mode = ?4,
                captions_recognition_signature = ?5,
                captions_language = ?6,
                caption_audio_stream = ?7,
                auto_captions_path = ?8,
                render_status = 'pending'
          WHERE id = ?9",
        rusqlite::params![
            srt_text,
            source.desired_start,
            CAPTION_PIPELINE_VERSION,
            audio_mode,
            recognition.signature,
            recognition.resolved_language,
            recognition.audio_stream,
            srt_path.to_string_lossy(),
            clip_id,
        ],
    )
    .map_err(|error| format!("DB error: {error}"))?;

    let mut updated = current;
    updated.captions_enabled = 1;
    updated.captions_text = Some(srt_text);
    updated.captions_source_start = Some(source.desired_start);
    updated.captions_provenance = "aligned".to_string();
    updated.captions_pipeline_version = CAPTION_PIPELINE_VERSION;
    updated.caption_audio_mode = audio_mode;
    updated.captions_recognition_signature = Some(recognition.signature);
    updated.captions_language = Some(recognition.resolved_language);
    updated.caption_audio_stream = Some(recognition.audio_stream);
    Ok(caption_result_with_model(&updated, true, None, &model_used))
}

/// Upgrade generated draft captions to clip-specific DTW timing without
/// replacing captions that the user has edited.
#[tauri::command]
pub async fn ensure_clip_captions_aligned(
    clip_id: String,
    _db: State<'_, DbConn>,
) -> Result<CaptionAlignmentResult, String> {
    ensure_clip_captions_aligned_impl(db::db_path()?, &clip_id, false, None).await
}

/// Explicitly regenerate captions from the selected local audio source.
#[tauri::command]
pub async fn generate_clip_captions(
    clip_id: String,
    audio_mode: Option<String>,
    _db: State<'_, DbConn>,
) -> Result<CaptionAlignmentResult, String> {
    ensure_clip_captions_aligned_impl(db::db_path()?, &clip_id, true, audio_mode.as_deref()).await
}

/// Set a clip's thumbnail to a specific frame at the given absolute time.
#[tauri::command]
pub fn set_clip_thumbnail(
    clip_id: String,
    timestamp: f64,
    db: State<'_, DbConn>,
) -> Result<String, String> {
    let ffmpeg = find_ffmpeg()?;

    let media_path = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let clip = db::get_clip_by_id(&conn, &clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("Clip not found")?;
        if let Some(path) = clip
            .source_media_path
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .or_else(|| clip.community_clip_mp4_path.as_deref())
        {
            let path = std::path::PathBuf::from(path);
            if !path.is_file() {
                return Err(format!("The source video is missing: {}", path.display()));
            }
            path.to_string_lossy().to_string()
        } else {
            let vod = db::get_vod_by_id(&conn, &clip.vod_id)
                .map_err(|e| format!("DB error: {}", e))?
                .ok_or("VOD not found")?;
            vod.local_path.ok_or("VOD not downloaded")?
        }
    };

    let thumb_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clipviral")
        .join("thumbnails");
    std::fs::create_dir_all(&thumb_dir).ok();
    let thumb_path = thumb_dir.join(format!("{}.jpg", clip_id));

    generate_thumbnail(&ffmpeg, &media_path, timestamp, &thumb_path)?;

    let path_str = thumb_path.to_string_lossy().to_string();
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::update_clip_thumbnail(&conn, &clip_id, Some(&path_str))
        .map_err(|e| format!("DB error: {}", e))?;

    Ok(path_str)
}

/// Export a clip — renders the clip segment with configured settings using ffmpeg.
#[tauri::command]
pub async fn export_clip(
    clip_id: String,
    aspect_ratio: Option<String>,
    app: AppHandle,
    db: State<'_, DbConn>,
    queue: State<'_, JobQueue>,
) -> Result<ExportArtifact, String> {
    let export_lease = acquire_clip_export_lease(&clip_id)?;
    ensure_clip_captions_aligned_impl(db::db_path()?, &clip_id, false, None).await?;
    let ffmpeg = find_ffmpeg().map_err(|e| report_error(&app, e))?;

    let (mut clip, vod, media_path, allow_override) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let clip = db::get_clip_by_id(&conn, &clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("Clip not found")?;
        let vod = db::get_vod_by_id(&conn, &clip.vod_id).map_err(|e| format!("DB error: {}", e))?;
        let path = resolve_media_path(&clip, vod.as_ref())?;
        let allow = matches!(
            db::get_setting(&conn, "allow_per_clip_cam_region_override")
                .ok()
                .flatten()
                .as_deref(),
            Some("true"),
        );
        (clip, vod, path, allow)
    };
    if let Some(target_aspect_ratio) = aspect_ratio {
        aspect_file_tag(&target_aspect_ratio)?;
        clip.aspect_ratio = target_aspect_ratio;
    }
    let previous_output_path = clip.output_path.clone();
    let (artifact, temp_path) =
        prepare_export_artifact(&clip, vod.as_ref(), &media_path, allow_override)?;
    let output_path = std::path::PathBuf::from(&artifact.path);
    let returned_artifact = artifact.clone();

    let job_id = format!("export-{}", clip_id);
    let clip_id_bg = clip_id.clone();

    queue
        .add_job(job_id, move |handle| async move {
            let _export_lease = export_lease;
            // Mark rendering in DB inside the job, so status is only set once
            // the job is actually running (not stuck if app crashes before queuing).
            {
                let db_path = db::db_path().map_err(|e| format!("DB path error: {e}"))?;
                let conn =
                    rusqlite::Connection::open(db_path).map_err(|e| format!("DB error: {e}"))?;
                db::update_clip_render_status(
                    &conn,
                    &clip_id_bg,
                    "rendering",
                    previous_output_path.as_deref(),
                )
                .map_err(|e| format!("DB error: {}", e))?;
            }
            // ── Preparing ──
            handle.set_progress(5);

            if artifact_file_is_ready(&output_path) {
                let db_path = db::db_path().map_err(|e| format!("DB path error: {e}"))?;
                let conn =
                    rusqlite::Connection::open(db_path).map_err(|e| format!("DB error: {e}"))?;
                persist_export_success(&conn, &clip, &artifact);
                handle.set_progress(100);
                return Ok(());
            }
            if temp_path.exists() {
                std::fs::remove_file(&temp_path)
                    .map_err(|e| format!("Failed to clear an incomplete export: {e}"))?;
            }

            // ── Building export request ──
            handle.set_progress(5);
            let mut request = clip_to_export_request(
                &clip,
                vod.as_ref(),
                &media_path,
                &temp_path,
                allow_override,
            );
            let image_caption_clip = clip.clone();

            // ── Running ffmpeg with real progress ──
            let clip_id_ref = clip_id_bg.clone();
            let handle_ref = handle.clone();

            let result = tokio::task::spawn_blocking(move || {
                attach_image_caption_track(&ffmpeg, &image_caption_clip, &mut request);
                vertical_crop::run_export(&ffmpeg, &request, |pct| {
                    handle_ref.set_progress(pct);
                })
            })
            .await
            .map_err(|e| format!("Export task panicked: {e}"))?;

            // ── Update DB with result ──
            let db_path = db::db_path().map_err(|e| format!("DB path error: {e}"))?;
            let conn = rusqlite::Connection::open(db_path).map_err(|e| format!("DB error: {e}"))?;

            if result.success {
                if let Err(error) = finalize_artifact(&temp_path, &output_path) {
                    db::update_clip_render_status(
                        &conn,
                        &clip_id_ref,
                        "failed",
                        previous_output_path.as_deref(),
                    )
                    .ok();
                    return Err(error);
                }
                persist_export_success(&conn, &clip, &artifact);
                handle.set_progress(100);
                Ok(())
            } else {
                let _ = std::fs::remove_file(&temp_path);
                db::update_clip_render_status(
                    &conn,
                    &clip_id_ref,
                    "failed",
                    previous_output_path.as_deref(),
                )
                .ok();
                let msg = if result.stderr_tail.is_empty() {
                    "FFmpeg exited with an error".to_string()
                } else {
                    format!("FFmpeg error: {}", result.stderr_tail)
                };
                Err(msg)
            }
        })
        .map_err(|error| error.to_string())?;

    Ok(returned_artifact)
}

/// Export a clip synchronously by id. Returns the rendered file path on success.
/// Used by both the `export_clip` Tauri command (via its JobQueue wrapper) and
/// the scheduler's auto-export path when a pending upload lacks an output_path.
///
/// Opens its own `rusqlite::Connection` via `db::db_path()` so callers don't
/// need to juggle the DbConn State mutex. Safe to call from any async context;
/// the actual ffmpeg work runs inside `tokio::task::spawn_blocking`.
pub(crate) async fn render_clip_by_id(clip_id: &str) -> Result<std::path::PathBuf, String> {
    let artifact = render_clip_by_id_for_aspect(clip_id, None).await?;
    Ok(std::path::PathBuf::from(artifact.path))
}

pub(crate) async fn render_clip_by_id_for_aspect(
    clip_id: &str,
    aspect_ratio: Option<&str>,
) -> Result<ExportArtifact, String> {
    let _export_lease = acquire_clip_export_lease(clip_id)?;
    ensure_clip_captions_aligned_impl(db::db_path()?, clip_id, false, None).await?;
    let ffmpeg = find_ffmpeg().map_err(|e| e.to_string())?;

    // Load clip + vod path (sync)
    let (mut clip, vod, media_path, allow_override) = {
        let db_path = db::db_path().map_err(|e| format!("DB path: {}", e))?;
        let conn = rusqlite::Connection::open(&db_path).map_err(|e| format!("DB open: {}", e))?;
        let clip = db::get_clip_by_id(&conn, clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or_else(|| "Clip not found".to_string())?;
        let vod = db::get_vod_by_id(&conn, &clip.vod_id).map_err(|e| format!("DB error: {}", e))?;
        let path = resolve_media_path(&clip, vod.as_ref())?;
        let allow = matches!(
            db::get_setting(&conn, "allow_per_clip_cam_region_override")
                .ok()
                .flatten()
                .as_deref(),
            Some("true"),
        );
        (clip, vod, path, allow)
    };
    if let Some(target_aspect_ratio) = aspect_ratio {
        aspect_file_tag(target_aspect_ratio)?;
        clip.aspect_ratio = target_aspect_ratio.to_string();
    }
    let previous_output_path = clip.output_path.clone();
    let (artifact, temp_path) =
        prepare_export_artifact(&clip, vod.as_ref(), &media_path, allow_override)?;
    let output_path = std::path::PathBuf::from(&artifact.path);

    // Mark rendering in DB
    {
        let db_path = db::db_path().map_err(|e| format!("DB path: {}", e))?;
        let conn = rusqlite::Connection::open(&db_path).map_err(|e| format!("DB open: {}", e))?;
        db::update_clip_render_status(&conn, clip_id, "rendering", previous_output_path.as_deref())
            .map_err(|e| format!("DB error: {}", e))?;
    }

    if artifact_file_is_ready(&output_path) {
        let db_path = db::db_path().map_err(|e| format!("DB path: {}", e))?;
        let conn = rusqlite::Connection::open(&db_path).map_err(|e| format!("DB open: {}", e))?;
        persist_export_success(&conn, &clip, &artifact);
        return Ok(artifact);
    }
    if temp_path.exists() {
        std::fs::remove_file(&temp_path)
            .map_err(|e| format!("Failed to clear an incomplete export: {e}"))?;
    }

    let mut request =
        clip_to_export_request(&clip, vod.as_ref(), &media_path, &temp_path, allow_override);
    let image_caption_clip = clip.clone();
    let clip_id_owned = clip_id.to_string();

    let result = tokio::task::spawn_blocking(move || {
        attach_image_caption_track(&ffmpeg, &image_caption_clip, &mut request);
        vertical_crop::run_export(&ffmpeg, &request, |_pct| {
            // no progress callback — scheduler's simpler.
        })
    })
    .await
    .map_err(|e| format!("Export task panicked: {}", e))?;

    // Persist result
    let db_path = db::db_path().map_err(|e| format!("DB path: {}", e))?;
    let conn = rusqlite::Connection::open(&db_path).map_err(|e| format!("DB open: {}", e))?;

    if result.success {
        if let Err(error) = finalize_artifact(&temp_path, &output_path) {
            db::update_clip_render_status(
                &conn,
                &clip_id_owned,
                "failed",
                previous_output_path.as_deref(),
            )
            .ok();
            return Err(error);
        }
        persist_export_success(&conn, &clip, &artifact);
        Ok(artifact)
    } else {
        let _ = std::fs::remove_file(&temp_path);
        db::update_clip_render_status(
            &conn,
            &clip_id_owned,
            "failed",
            previous_output_path.as_deref(),
        )
        .ok();
        let msg = if result.stderr_tail.is_empty() {
            "FFmpeg exited with an error".to_string()
        } else {
            format!("FFmpeg error: {}", result.stderr_tail)
        };
        Err(msg)
    }
}

/// Probe a media file's duration (seconds) via ffprobe. Returns `None` on any
/// failure (ffprobe missing, parse error, etc.). Used by the community-clip
/// export branch to bound `-to` at the downloaded clip's full length.
pub(crate) fn probe_media_duration(path: &std::path::Path) -> Option<f64> {
    let ffprobe = crate::bin_manager::ffprobe_path().ok()?;
    let mut cmd = std::process::Command::new(&ffprobe);
    cmd.arg("-v")
        .arg("error")
        .arg("-show_entries")
        .arg("format=duration")
        .arg("-of")
        .arg("default=noprint_wrappers=1:nokey=1")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout);
    let dur: f64 = s.trim().parse().ok()?;
    if dur.is_finite() && dur > 0.0 {
        Some(dur)
    } else {
        None
    }
}

fn resolve_media_path(clip: &db::ClipRow, vod: Option<&db::VodRow>) -> Result<String, String> {
    if let Some(path) = clip
        .source_media_path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .or_else(|| {
            clip.community_clip_mp4_path
                .as_deref()
                .filter(|path| !path.trim().is_empty())
        })
    {
        let path = std::path::PathBuf::from(path);
        if path.is_file() {
            return Ok(path.to_string_lossy().to_string());
        }
        return Err(format!("The source video is missing: {}", path.display()));
    }
    vod.and_then(|vod| vod.local_path.clone())
        .ok_or_else(|| "VOD not downloaded — download it first to export clips".to_string())
}

/// Convert a DB ClipRow into an ExportRequest for the vertical_crop module.
fn clip_to_export_request(
    clip: &db::ClipRow,
    vod: Option<&db::VodRow>,
    media_path: &str,
    output_path: &std::path::Path,
    allow_per_clip_override: bool,
) -> vertical_crop::ExportRequest {
    let imported_source = clip
        .source_media_path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_file());
    // ── Community-clip source override ──
    // When this clip is backed by a downloaded Twitch clip MP4 (viewer-made
    // clip), that file IS the clip's video: export it WHOLE (0-based, no
    // start/end trim) instead of re-cutting the VOD via the unreliable
    // vod_offset. Falls back to the VOD path + start/end if the file is missing
    // or its duration can't be probed (graceful — normal clips are untouched).
    let community_source: Option<(std::path::PathBuf, f64)> = clip
        .community_clip_mp4_path
        .as_deref()
        .filter(|p| !p.is_empty())
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
        .and_then(|p| probe_media_duration(&p).map(|dur| (p, dur)));
    if let Some((ref src, _)) = community_source {
        log::info!(
            "[export] clip {} using downloaded community clip MP4 (whole, 0-based): {}",
            clip.id,
            src.display()
        );
    } else if clip
        .community_clip_mp4_path
        .as_deref()
        .map_or(false, |p| !p.is_empty())
    {
        log::warn!(
            "[export] clip {} has community_clip_mp4_path but file/duration unavailable — falling back to VOD cut",
            clip.id
        );
    }

    // Resolve platform from aspect ratio (future: store preset id in DB)
    let platform = vertical_crop::Platform::from_aspect_ratio(&clip.aspect_ratio);
    let target = platform.resolution();

    // Resolve layout and its persisted editor geometry from DB state.
    let layout_settings =
        vertical_crop::EditorLayoutSettings::from_json(clip.facecam_settings.as_deref());
    let layout = match vertical_crop::LayoutMode::from_db(&clip.facecam_layout) {
        vertical_crop::LayoutMode::Split { .. } => vertical_crop::LayoutMode::Split {
            ratio: layout_settings.split_ratio,
        },
        other => other,
    };
    let layout_supports_branding = matches!(
        &layout,
        vertical_crop::LayoutMode::ContextFit
            | vertical_crop::LayoutMode::Split { .. }
            | vertical_crop::LayoutMode::Pip { .. }
    );
    let context_background_path =
        if layout_supports_branding && clip.context_background_mode == "branding" {
            clip.context_background_path
                .as_deref()
                .map(std::path::PathBuf::from)
                .filter(|path| path.is_file())
                .filter(|path| {
                    path.extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|extension| {
                            matches!(
                                extension.to_ascii_lowercase().as_str(),
                                "png" | "jpg" | "jpeg" | "webp" | "gif"
                            )
                        })
                })
        } else {
            None
        };
    if layout_supports_branding
        && clip.context_background_mode == "branding"
        && context_background_path.is_none()
    {
        log::warn!(
            "[export] clip {} branding asset unavailable; falling back to the layout's video source",
            clip.id
        );
    }
    let context_background_mode = if matches!(&layout, vertical_crop::LayoutMode::ContextFit)
        && clip.context_background_mode == "black"
    {
        vertical_crop::ContextBackgroundMode::Black
    } else if context_background_path.is_some() {
        vertical_crop::ContextBackgroundMode::Branding
    } else {
        vertical_crop::ContextBackgroundMode::Blur
    };

    // Resolve the effective cam region using override precedence + settings toggle.
    let effective_region = crate::cam_region::resolve_effective_region(
        vod.and_then(|vod| vod.cam_region_norm.as_deref()),
        clip.cam_region_norm_override.as_deref(),
        allow_per_clip_override,
    );
    // Layout-aware fit-mode default: PiP slots are non-square so Fit produces
    // tiny letterboxed content; default to Fill instead. Split/GameplayFocus
    // default to Fit. Explicit 'fill'/'stretch' from DB always honored.
    // Special case: 'fit' stored from a previous Split session is overridden
    // to Fill when the current layout is PiP, to avoid the tiny-letterbox bug.
    let fit_mode = match (clip.cam_fit_mode.as_deref(), &layout) {
        (Some("fill"), _) => crate::cam_region::CamFitMode::Fill,
        (Some("stretch"), _) => crate::cam_region::CamFitMode::Stretch,
        (_, vertical_crop::LayoutMode::Pip { .. }) => crate::cam_region::CamFitMode::Fill,
        _ => crate::cam_region::CamFitMode::Fit,
    };

    // Source + span: community-clip file whole (0..duration) when present,
    // otherwise the VOD path trimmed to the clip's start/end.
    let (source_path, start, end) = match (imported_source, community_source) {
        (Some(src), _) => (src, clip.start_seconds, clip.end_seconds),
        (None, Some((src, dur))) => (src, 0.0, dur),
        (None, None) => (
            std::path::PathBuf::from(media_path),
            clip.start_seconds,
            clip.end_seconds,
        ),
    };
    let captions_source_start = clip
        .captions_source_start
        .filter(|value| value.is_finite())
        .unwrap_or_else(|| {
            if clip
                .source_media_path
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty())
                || clip
                    .community_clip_mp4_path
                    .as_deref()
                    .is_some_and(|path| !path.trim().is_empty())
            {
                0.0
            } else {
                clip.start_seconds
            }
        });
    let caption_filter = build_caption_filter(
        clip,
        target.width as i32,
        target.height as i32,
        start - captions_source_start,
        (end - start).max(0.0),
    );

    vertical_crop::ExportRequest {
        source_path,
        output_path: output_path.to_path_buf(),
        start,
        end,
        platform,
        target,
        layout,
        layout_settings,
        caption_filter,
        caption_overlay_path: None,
        effective_region,
        fit_mode,
        context_background_mode,
        context_background_path,
        context_blur_strength: clip.context_blur_strength,
        context_video_y: clip.context_video_y,
        full_frame_scale: clip.full_frame_scale,
    }
}

const RUBIK_DIRT_FONT_BYTES: &[u8] = include_bytes!("../../../public/fonts/RubikDirt-Regular.ttf");
const COINY_FONT_BYTES: &[u8] = include_bytes!("../../../public/fonts/Coiny-Regular.ttf");
const NOSIFER_FONT_BYTES: &[u8] = include_bytes!("../../../public/fonts/Nosifer-Regular.ttf");
const BANGERS_FONT_BYTES: &[u8] = include_bytes!("../../../public/fonts/Bangers-Regular.ttf");
const TAPE_RIOT_FONT_BYTES: &[u8] =
    include_bytes!("../../../public/fonts/ClipGoblinTapeRiot-Regular.ttf");
const TAPE_RIOT_SEAMS_FONT_BYTES: &[u8] =
    include_bytes!("../../../public/fonts/ClipGoblinTapeRiotSeams-Regular.ttf");
const TAPE_RIOT_PATCHES_FONT_BYTES: &[u8] =
    include_bytes!("../../../public/fonts/ClipGoblinTapeRiotPatches-Regular.ttf");
const PAPER_MISCHIEF_FONT_BYTES: &[u8] =
    include_bytes!("../../../public/fonts/ClipGoblinPaperMischief-Regular.ttf");
const PAPER_MISCHIEF_FIBER_FONT_BYTES: &[u8] =
    include_bytes!("../../../public/fonts/ClipGoblinPaperMischiefFiber-Regular.ttf");
const PAPER_MISCHIEF_TABS_FONT_BYTES: &[u8] =
    include_bytes!("../../../public/fonts/ClipGoblinPaperMischiefTabs-Regular.ttf");
const GOBLIN_BITE_FONT_BYTES: &[u8] =
    include_bytes!("../../../public/fonts/ClipGoblinGoblinBite-Regular.ttf");
const GOBLIN_BITE_DISTRESS_FONT_BYTES: &[u8] =
    include_bytes!("../../../public/fonts/ClipGoblinGoblinBiteDistress-Regular.ttf");
static RUBIK_DIRT_FONT_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
static COINY_FONT_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
static NOSIFER_FONT_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
static BANGERS_FONT_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
static TAPE_RIOT_FONT_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
static TAPE_RIOT_SEAMS_FONT_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
static TAPE_RIOT_PATCHES_FONT_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
static PAPER_MISCHIEF_FONT_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
static PAPER_MISCHIEF_FIBER_FONT_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
static PAPER_MISCHIEF_TABS_FONT_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
static GOBLIN_BITE_FONT_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
static GOBLIN_BITE_DISTRESS_FONT_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();

fn stage_bundled_caption_font(
    cache: &'static OnceLock<Option<std::path::PathBuf>>,
    filename: &str,
    bytes: &[u8],
) -> Option<std::path::PathBuf> {
    cache
        .get_or_init(|| {
            let font_dir = std::env::temp_dir().join("clipgoblin-caption-fonts");
            if let Err(error) = std::fs::create_dir_all(&font_dir) {
                log::warn!("Failed to create caption font directory: {error}");
                return None;
            }

            let font_path = font_dir.join(filename);
            let already_current = std::fs::read(&font_path)
                .map(|current| current == bytes)
                .unwrap_or(false);
            if !already_current {
                if let Err(error) = std::fs::write(&font_path, bytes) {
                    log::warn!("Failed to stage bundled caption font: {error}");
                    return None;
                }
            }

            Some(font_path)
        })
        .clone()
}

fn bundled_caption_font(style_id: &str) -> Option<std::path::PathBuf> {
    match style_id {
        "fire" => stage_bundled_caption_font(
            &RUBIK_DIRT_FONT_PATH,
            "RubikDirt-Regular.ttf",
            RUBIK_DIRT_FONT_BYTES,
        ),
        "boxed" => {
            stage_bundled_caption_font(&COINY_FONT_PATH, "Coiny-Regular.ttf", COINY_FONT_BYTES)
        }
        "minimal" => stage_bundled_caption_font(
            &NOSIFER_FONT_PATH,
            "Nosifer-Regular.ttf",
            NOSIFER_FONT_BYTES,
        ),
        "comic-pop" | "undead-legion" => stage_bundled_caption_font(
            &BANGERS_FONT_PATH,
            "Bangers-Regular.ttf",
            BANGERS_FONT_BYTES,
        ),
        "tape-riot" => {
            let base = stage_bundled_caption_font(
                &TAPE_RIOT_FONT_PATH,
                "ClipGoblinTapeRiot-Regular.ttf",
                TAPE_RIOT_FONT_BYTES,
            );
            let _ = stage_bundled_caption_font(
                &TAPE_RIOT_SEAMS_FONT_PATH,
                "ClipGoblinTapeRiotSeams-Regular.ttf",
                TAPE_RIOT_SEAMS_FONT_BYTES,
            );
            let _ = stage_bundled_caption_font(
                &TAPE_RIOT_PATCHES_FONT_PATH,
                "ClipGoblinTapeRiotPatches-Regular.ttf",
                TAPE_RIOT_PATCHES_FONT_BYTES,
            );
            base
        }
        "paper-mischief" => {
            let base = stage_bundled_caption_font(
                &PAPER_MISCHIEF_FONT_PATH,
                "ClipGoblinPaperMischief-Regular.ttf",
                PAPER_MISCHIEF_FONT_BYTES,
            );
            let _ = stage_bundled_caption_font(
                &PAPER_MISCHIEF_FIBER_FONT_PATH,
                "ClipGoblinPaperMischiefFiber-Regular.ttf",
                PAPER_MISCHIEF_FIBER_FONT_BYTES,
            );
            let _ = stage_bundled_caption_font(
                &PAPER_MISCHIEF_TABS_FONT_PATH,
                "ClipGoblinPaperMischiefTabs-Regular.ttf",
                PAPER_MISCHIEF_TABS_FONT_BYTES,
            );
            base
        }
        "goblin-bite" => {
            let base = stage_bundled_caption_font(
                &GOBLIN_BITE_FONT_PATH,
                "ClipGoblinGoblinBite-Regular.ttf",
                GOBLIN_BITE_FONT_BYTES,
            );
            let _ = stage_bundled_caption_font(
                &GOBLIN_BITE_DISTRESS_FONT_PATH,
                "ClipGoblinGoblinBiteDistress-Regular.ttf",
                GOBLIN_BITE_DISTRESS_FONT_BYTES,
            );
            base
        }
        _ => None,
    }
}

fn ffmpeg_filter_path(path: &std::path::Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .replace(':', "\\:")
        .replace('\'', "\\'")
}

fn paper_mischief_font_paths(
) -> Result<(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf), String> {
    let face = stage_bundled_caption_font(
        &PAPER_MISCHIEF_FONT_PATH,
        "ClipGoblinPaperMischief-Regular.ttf",
        PAPER_MISCHIEF_FONT_BYTES,
    )
    .ok_or_else(|| "Paper Mischief face font is unavailable".to_string())?;
    let fiber = stage_bundled_caption_font(
        &PAPER_MISCHIEF_FIBER_FONT_PATH,
        "ClipGoblinPaperMischiefFiber-Regular.ttf",
        PAPER_MISCHIEF_FIBER_FONT_BYTES,
    )
    .ok_or_else(|| "Paper Mischief fiber font is unavailable".to_string())?;
    let tabs = stage_bundled_caption_font(
        &PAPER_MISCHIEF_TABS_FONT_PATH,
        "ClipGoblinPaperMischiefTabs-Regular.ttf",
        PAPER_MISCHIEF_TABS_FONT_BYTES,
    )
    .ok_or_else(|| "Paper Mischief tape-tab font is unavailable".to_string())?;
    Ok((face, fiber, tabs))
}

fn paper_mischief_cache_dir() -> Result<std::path::PathBuf, String> {
    let path = std::env::temp_dir()
        .join("clipgoblin-paper-mischief")
        .join(PAPER_MISCHIEF_RENDERER_VERSION);
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("Could not create the Paper Mischief cache: {error}"))?;
    Ok(path)
}

fn validate_paper_mischief_request(request: &PaperMischiefCaptionRequest) -> Result<(), String> {
    if request.text.trim().is_empty() || !request.text.chars().any(char::is_alphanumeric) {
        return Err("Paper Mischief needs spoken caption text".to_string());
    }
    if request.text.chars().count() > 1_000 || request.text.contains('\0') {
        return Err("Paper Mischief caption text is too long".to_string());
    }
    if !(320..=3_840).contains(&request.target_width)
        || !(320..=3_840).contains(&request.target_height)
    {
        return Err("Paper Mischief output dimensions are unsupported".to_string());
    }
    if !(8..=256).contains(&request.font_size) {
        return Err("Paper Mischief font size is unsupported".to_string());
    }
    if request.anchor_y < 0 || request.anchor_y > request.target_height as i32 {
        return Err("Paper Mischief caption anchor is outside the frame".to_string());
    }
    if !matches!(request.alignment, 2 | 5 | 8) {
        return Err("Paper Mischief caption alignment is unsupported".to_string());
    }
    Ok(())
}

fn wrap_paper_mischief_text(text: &str, max_characters: usize) -> String {
    let max_characters = max_characters.max(4);
    let mut lines = Vec::new();
    for source_line in text.lines() {
        let mut current = String::new();
        for word in source_line.split_whitespace() {
            let candidate_len = if current.is_empty() {
                word.chars().count()
            } else {
                current.chars().count() + 1 + word.chars().count()
            };
            if !current.is_empty() && candidate_len > max_characters {
                lines.push(current);
                current = word.to_string();
            } else {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(word);
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines.join("\n")
}

fn paper_mischief_filter(
    request: &PaperMischiefCaptionRequest,
    text_path: &std::path::Path,
) -> Result<String, String> {
    validate_paper_mischief_request(request)?;
    let (face_font, fiber_font, tabs_font) = paper_mischief_font_paths()?;
    let face_font = ffmpeg_filter_path(&face_font);
    let fiber_font = ffmpeg_filter_path(&fiber_font);
    let tabs_font = ffmpeg_filter_path(&tabs_font);
    let text_path = ffmpeg_filter_path(text_path);
    let scale = request.font_size as f64 / 60.0;
    let offset = |value: i32| ((value as f64 * scale).round() as i32).max(1);
    let blur = (6.0 * scale).max(1.0);
    let line_spacing = (request.font_size as f64 * 0.08).round() as i32;
    let box_width = (request.target_width as f64 * 0.77).round() as i32;
    let base_y = match request.alignment {
        8 => request.anchor_y.to_string(),
        5 => format!("{}-text_h/2", request.anchor_y),
        _ => format!("{}-text_h", request.anchor_y),
    };
    let common = |font: &str| {
        format!(
            "fontfile='{font}':textfile='{text_path}':reload=0:fontsize={}:line_spacing={line_spacing}:boxw={box_width}:text_align=center",
            request.font_size,
        )
    };
    let face = common(&face_font);
    let fiber = common(&fiber_font);
    let tabs = common(&tabs_font);
    let draw = |common: &str, colour: &str, dx: i32, dy: i32, extra: &str| {
        format!(
            "drawtext={common}:fontcolor={colour}:x=(w-{box_width})/2+{dx}:y={base_y}+{dy}{extra}"
        )
    };

    let shadow = draw(&face, "black@0.82", offset(18), offset(24), "");
    let depth_steps = [
        ("#2A103B", 16, 22),
        ("#36134D", 14, 20),
        ("#451963", 12, 17),
        ("#55217A", 10, 14),
        ("#672990", 8, 11),
        ("#322A38", 5, 7),
        ("#756E7B", 3, 4),
    ];
    let depth = depth_steps
        .into_iter()
        .map(|(colour, dx, dy)| draw(&face, colour, offset(dx), offset(dy), ""))
        .collect::<Vec<_>>()
        .join(",");
    let face_layers = [
        draw(&face, "#FFFFFF", -offset(2), -offset(2), ""),
        draw(&face, "#514A55", offset(2), offset(2), ""),
        draw(&face, "#F4F0E8", 0, 0, ":borderw=1:bordercolor=#2B2630"),
        draw(&fiber, "#7B756F@0.43", 0, 0, ""),
        draw(&tabs, "#AFFF24", 0, 0, ""),
    ]
    .join(",");

    Ok(format!(
        "color=c=black@0.0:s={}x{}:r=30,format=rgba,split=4[blank][shadowbase][depthbase][facebase];\
         [shadowbase]{shadow},gblur=sigma={blur:.2}:steps=2[shadow];\
         [depthbase]{depth}[depth];\
         [facebase]{face_layers}[face];\
         [blank][shadow]overlay=format=auto[o1];\
         [o1][depth]overlay=format=auto[o2];\
         [o2][face]overlay=format=auto,format=rgba",
        request.target_width, request.target_height,
    ))
}

fn ffmpeg_error_tail(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ")
}

fn render_paper_mischief_frame(
    ffmpeg: &std::path::Path,
    request: &PaperMischiefCaptionRequest,
) -> Result<std::path::PathBuf, String> {
    validate_paper_mischief_request(request)?;
    let max_characters = ((request.target_width as f64 * 0.77) / (request.font_size as f64 * 0.76))
        .floor()
        .max(4.0) as usize;
    let wrapped_text = wrap_paper_mischief_text(&request.text.to_uppercase(), max_characters);
    let cache_dir = paper_mischief_cache_dir()?;
    let mut hasher = Sha256::new();
    hasher.update(PAPER_MISCHIEF_RENDERER_VERSION.as_bytes());
    hasher.update(serde_json::to_vec(request).map_err(|error| error.to_string())?);
    hasher.update(wrapped_text.as_bytes());
    let key = format!("{:x}", hasher.finalize());
    let output_path = cache_dir.join(format!("cue-{key}.png"));
    if artifact_file_is_ready(&output_path) {
        return Ok(output_path);
    }

    let text_path = cache_dir.join(format!("cue-{key}.txt"));
    std::fs::write(&text_path, wrapped_text)
        .map_err(|error| format!("Could not stage Paper Mischief text: {error}"))?;
    let filter = paper_mischief_filter(request, &text_path)?;
    let temp_path = cache_dir.join(format!(".cue-{key}-{}.png", uuid::Uuid::new_v4().simple()));
    let mut command = std::process::Command::new(ffmpeg);
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg(filter)
        .arg("-frames:v")
        .arg("1")
        .arg("-c:v")
        .arg("png")
        .arg("-pix_fmt")
        .arg("rgba")
        .arg(&temp_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command
        .output()
        .map_err(|error| format!("Could not start Paper Mischief rendering: {error}"))?;
    if !output.status.success() || !artifact_file_is_ready(&temp_path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!(
            "Paper Mischief rendering failed: {}",
            ffmpeg_error_tail(&output.stderr)
        ));
    }
    match std::fs::rename(&temp_path, &output_path) {
        Ok(()) => {}
        Err(_) if artifact_file_is_ready(&output_path) => {
            let _ = std::fs::remove_file(&temp_path);
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temp_path);
            return Err(format!("Could not cache Paper Mischief caption: {error}"));
        }
    }
    Ok(output_path)
}

fn render_transparent_caption_frame(
    ffmpeg: &std::path::Path,
    width: u32,
    height: u32,
) -> Result<std::path::PathBuf, String> {
    let cache_dir = paper_mischief_cache_dir()?;
    let path = cache_dir.join(format!("blank-{width}x{height}.png"));
    if artifact_file_is_ready(&path) {
        return Ok(path);
    }
    let temp = cache_dir.join(format!(
        ".blank-{width}x{height}-{}.png",
        uuid::Uuid::new_v4().simple()
    ));
    let source = format!("color=c=black@0.0:s={width}x{height}:r=30,format=rgba");
    let mut command = std::process::Command::new(ffmpeg);
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg(source)
        .arg("-frames:v")
        .arg("1")
        .arg("-c:v")
        .arg("png")
        .arg("-pix_fmt")
        .arg("rgba")
        .arg(&temp)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command
        .output()
        .map_err(|error| format!("Could not start transparent caption rendering: {error}"))?;
    if !output.status.success() || !artifact_file_is_ready(&temp) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!(
            "Transparent caption frame failed: {}",
            ffmpeg_error_tail(&output.stderr)
        ));
    }
    match std::fs::rename(&temp, &path) {
        Ok(()) => {}
        Err(_) if artifact_file_is_ready(&path) => {
            let _ = std::fs::remove_file(&temp);
        }
        Err(error) => return Err(format!("Could not cache transparent frame: {error}")),
    }
    Ok(path)
}

#[tauri::command]
pub async fn render_paper_mischief_caption(
    app: AppHandle,
    request: PaperMischiefCaptionRequest,
) -> Result<PaperMischiefCaptionAsset, String> {
    let ffmpeg = find_ffmpeg().map_err(|error| error.to_string())?;
    let path = tokio::task::spawn_blocking(move || render_paper_mischief_frame(&ffmpeg, &request))
        .await
        .map_err(|error| format!("Paper Mischief renderer stopped unexpectedly: {error}"))??;
    app.asset_protocol_scope()
        .allow_file(&path)
        .map_err(|error| format!("Could not allow Paper Mischief preview: {error}"))?;
    Ok(PaperMischiefCaptionAsset {
        path: path.to_string_lossy().to_string(),
        renderer_version: PAPER_MISCHIEF_RENDERER_VERSION.to_string(),
    })
}

fn cardboard_uses_black_text(text: &str) -> bool {
    const EMPHASIS_WORDS: &[&str] = &[
        "no", "yes", "wait", "what", "run", "go", "help", "stop", "please", "why", "how", "kill",
        "dead", "die", "escape", "save", "clutch", "bruh", "bro", "dude",
    ];
    const EMPHASIS_PHRASES: &[&str] = &[
        "oh my god",
        "no way",
        "watch out",
        "let's go",
        "lets go",
        "we did it",
        "i'm dead",
        "im dead",
        "wait what",
    ];

    let normalized: String = text
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '\'' {
                character
            } else {
                ' '
            }
        })
        .collect();
    let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");

    EMPHASIS_PHRASES
        .iter()
        .any(|phrase| normalized.contains(phrase))
        || normalized
            .split_whitespace()
            .any(|word| EMPHASIS_WORDS.contains(&word))
}

fn cardboard_ass_drawings(
    target_width: i32,
    target_height: i32,
    anchor_y: i32,
    position: &str,
) -> (String, String) {
    let board_width = if target_height > target_width {
        target_width * 82 / 100
    } else {
        target_width * 62 / 100
    };
    let board_height = if target_height > target_width {
        128
    } else {
        112
    };
    let left = (target_width - board_width) / 2;
    let desired_top = match position {
        "top" => anchor_y,
        "center" => anchor_y - board_height / 2,
        _ => anchor_y - board_height,
    };
    let top = desired_top.clamp(8, (target_height - board_height - 12).max(8));
    let bottom = top + board_height;
    let right = left + board_width;
    let actual_height = bottom - top;

    let board = format!(
        "m {x0} {y1} l {x1} {y0} {x2} {y2} {x3} {y0} {x4} {y1} {x5} {y0} {x6} {y2} {x7} {y0} {x8} {y1} {x9} {y0} {x10} {y2} {x11} {y3} {x10} {y4} {x11} {y5} {x10} {y6} {x11} {y7} {x10} {y8} {x9} {y9} {x8} {y8} {x7} {y9} {x6} {y8} {x5} {y9} {x4} {y8} {x3} {y9} {x2} {y8} {x1} {y9} {x0} {y8} {xm} {y7} {x0} {y6} {xm} {y5} {x0} {y4} {xm} {y3}",
        x0 = left,
        x1 = left + board_width * 8 / 100,
        x2 = left + board_width * 16 / 100,
        x3 = left + board_width * 25 / 100,
        x4 = left + board_width * 34 / 100,
        x5 = left + board_width * 44 / 100,
        x6 = left + board_width * 55 / 100,
        x7 = left + board_width * 66 / 100,
        x8 = left + board_width * 76 / 100,
        x9 = left + board_width * 87 / 100,
        x10 = right,
        x11 = right + 4,
        xm = left - 4,
        y0 = top,
        y1 = top + 4,
        y2 = top + 2,
        y3 = top + actual_height * 18 / 100,
        y4 = top + actual_height * 34 / 100,
        y5 = top + actual_height * 50 / 100,
        y6 = top + actual_height * 68 / 100,
        y7 = top + actual_height * 84 / 100,
        y8 = bottom - 3,
        y9 = bottom,
    );

    let line_left = left + board_width * 5 / 100;
    let line_right = right - board_width * 5 / 100;
    let line_height = 2;
    let texture = [28, 51, 74]
        .iter()
        .map(|percent| {
            let y = top + actual_height * *percent / 100;
            format!(
                "m {line_left} {y} l {line_right} {y} {line_right} {yb} {line_left} {yb}",
                yb = y + line_height,
            )
        })
        .collect::<Vec<_>>()
        .join(" ");

    (board, texture)
}

#[cfg(test)]
mod export_concurrency_tests {
    use super::acquire_clip_export_lease;

    #[test]
    fn clip_export_lease_blocks_overlapping_renderers_and_releases_after_drop() {
        let clip_id = format!("export-lease-test-{}", uuid::Uuid::new_v4());
        let first = acquire_clip_export_lease(&clip_id).unwrap();

        assert!(acquire_clip_export_lease(&clip_id).is_err());
        drop(first);
        assert!(acquire_clip_export_lease(&clip_id).is_ok());
    }
}

#[cfg(test)]
mod paper_mischief_renderer_tests {
    use super::{
        artifact_file_is_ready, find_ffmpeg, paper_mischief_filter, render_paper_mischief_frame,
        validate_paper_mischief_request, wrap_paper_mischief_text, PaperMischiefCaptionRequest,
    };

    fn request() -> PaperMischiefCaptionRequest {
        PaperMischiefCaptionRequest {
            text: "That was not the plan".into(),
            target_width: 1080,
            target_height: 1920,
            font_size: 60,
            anchor_y: 960,
            alignment: 5,
        }
    }

    #[test]
    fn renderer_filter_is_transparent_textured_and_directional() {
        let filter = paper_mischief_filter(
            &request(),
            std::path::Path::new("C:\\Temp\\paper-caption.txt"),
        )
        .expect("Paper Mischief filter");

        assert!(filter.contains("black@0.0"));
        assert!(filter.contains("format=rgba"));
        assert!(filter.contains("gblur=sigma="));
        assert!(filter.contains("#672990"));
        assert!(filter.contains("ClipGoblinPaperMischiefFiber-Regular.ttf"));
        assert!(filter.contains("ClipGoblinPaperMischiefTabs-Regular.ttf"));
        assert!(filter.contains("textfile='"));
        assert!(!filter.contains("That was not the plan"));
    }

    #[test]
    fn renderer_wraps_phrase_without_changing_words() {
        assert_eq!(
            wrap_paper_mischief_text("THAT WAS NOT THE PLAN", 12),
            "THAT WAS NOT\nTHE PLAN"
        );
    }

    #[test]
    fn renderer_rejects_invalid_frame_and_alignment() {
        let mut invalid = request();
        invalid.target_width = 0;
        assert!(validate_paper_mischief_request(&invalid).is_err());
        invalid = request();
        invalid.alignment = 7;
        assert!(validate_paper_mischief_request(&invalid).is_err());
    }

    #[test]
    fn renderer_executes_locally_when_ffmpeg_is_available() {
        let Ok(ffmpeg) = find_ffmpeg() else {
            return;
        };
        let path = render_paper_mischief_frame(&ffmpeg, &request())
            .expect("Paper Mischief frame should render with local FFmpeg");
        assert!(artifact_file_is_ready(&path));
    }
}

#[cfg(test)]
mod export_artifact_tests {
    use super::{
        artifact_filename, caption_alignment_action, caption_result, caption_result_with_model,
        export_revision, frame_safe_generated_cues, generated_recognition_is_current,
        normalized_srt_cues, CaptionAlignmentAction, CAPTION_PIPELINE_VERSION,
        FRAME_SAFE_CAPTION_SECONDS,
    };
    use crate::db::ClipRow;

    fn test_clip() -> ClipRow {
        ClipRow {
            id: "artifact-test-clip".into(),
            highlight_id: "highlight".into(),
            vod_id: "vod".into(),
            title: "Artifact test".into(),
            start_seconds: 12.0,
            end_seconds: 42.0,
            aspect_ratio: "9:16".into(),
            crop_x: None,
            crop_y: None,
            crop_width: None,
            crop_height: None,
            captions_enabled: 1,
            captions_text: Some("1\n00:00:00,000 --> 00:00:01,000\nhello\n".into()),
            captions_position: "bottom".into(),
            caption_style: "clean".into(),
            caption_font_scale: 1.0,
            caption_card_scale: crate::db::DEFAULT_CAPTION_CARD_SCALE,
            caption_y_offset: 0.0,
            captions_source_start: Some(12.0),
            captions_provenance: "aligned".into(),
            captions_pipeline_version: 1,
            caption_audio_mode: "mixed".into(),
            captions_recognition_signature: None,
            captions_language: None,
            caption_audio_stream: None,
            facecam_layout: "context_fit".into(),
            facecam_settings: None,
            context_background_path: None,
            context_background_mode: "blur".into(),
            context_blur_strength: 0.25,
            context_video_y: 0.5,
            full_frame_scale: 1.0,
            render_status: "pending".into(),
            output_path: None,
            thumbnail_path: None,
            created_at: "2026-08-25T00:00:00Z".into(),
            game: None,
            publish_description: None,
            publish_hashtags: None,
            cam_region_norm_override: None,
            cam_fit_mode: None,
            community_clip_mp4_path: None,
            source_kind: "manual".into(),
            source_media_path: None,
            source_fingerprint: Some("source-fingerprint".into()),
            source_recorded_at: None,
        }
    }

    #[test]
    fn revision_ignores_mutable_render_bookkeeping() {
        let source = std::env::temp_dir().join(format!(
            "clipgoblin-artifact-source-{}.mp4",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&source, b"test source").unwrap();
        let mut clip = test_clip();
        let first = export_revision(&clip, None, &source.to_string_lossy(), false).unwrap();
        clip.render_status = "completed".into();
        clip.output_path = Some("C:\\old-export.mp4".into());
        clip.thumbnail_path = Some("C:\\thumbnail.jpg".into());
        let second = export_revision(&clip, None, &source.to_string_lossy(), false).unwrap();
        let _ = std::fs::remove_file(source);

        assert_eq!(first, second);
    }

    #[test]
    fn revision_changes_with_format_or_caption_content() {
        let source = std::env::temp_dir().join(format!(
            "clipgoblin-artifact-source-{}.mp4",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&source, b"test source").unwrap();
        let clip = test_clip();
        let vertical = export_revision(&clip, None, &source.to_string_lossy(), false).unwrap();

        let mut landscape_clip = clip.clone();
        landscape_clip.aspect_ratio = "16:9".into();
        let landscape =
            export_revision(&landscape_clip, None, &source.to_string_lossy(), false).unwrap();

        let mut caption_clip = clip;
        caption_clip.captions_text = Some("different caption".into());
        let different_caption =
            export_revision(&caption_clip, None, &source.to_string_lossy(), false).unwrap();
        let _ = std::fs::remove_file(source);

        assert_ne!(vertical, landscape);
        assert_ne!(vertical, different_caption);
        assert_ne!(
            artifact_filename("9:16", &vertical).unwrap(),
            artifact_filename("16:9", &landscape).unwrap(),
        );
    }

    #[test]
    fn unsupported_aspect_ratio_cannot_create_an_artifact_name() {
        assert!(artifact_filename("4:3", "revision").is_err());
    }

    #[test]
    fn caption_alignment_preserves_edits_and_refreshes_generated_drafts() {
        let mut clip = test_clip();
        clip.captions_provenance = "edited".into();
        assert_eq!(
            caption_alignment_action(&clip, None, None),
            CaptionAlignmentAction::PreserveEdited,
        );

        clip.captions_provenance = "analysis-draft".into();
        assert_eq!(
            caption_alignment_action(&clip, None, None),
            CaptionAlignmentAction::Align,
        );

        clip.captions_provenance = "aligned".into();
        clip.captions_pipeline_version = CAPTION_PIPELINE_VERSION;
        assert_eq!(
            caption_alignment_action(&clip, None, None),
            CaptionAlignmentAction::Reuse,
        );
    }

    #[test]
    fn generated_caption_result_reports_the_runtime_model() {
        let clip = test_clip();
        let result = caption_result_with_model(&clip, true, None, "medium");

        assert_eq!(result.model_used.as_deref(), Some("medium"));
        assert_eq!(
            serde_json::to_value(&result).unwrap()["modelUsed"],
            serde_json::json!("medium"),
        );

        let reused = caption_result(&clip, false, None);
        assert_eq!(reused.model_used, None);
    }

    #[test]
    fn generated_caption_recipe_must_match_but_edits_are_never_reclassified() {
        let mut clip = test_clip();
        clip.captions_pipeline_version = CAPTION_PIPELINE_VERSION;
        clip.captions_recognition_signature = Some("recipe-a".into());
        let expected = crate::whisper::RecognitionProvenance {
            signature: "recipe-a".into(),
            ..Default::default()
        };
        assert!(generated_recognition_is_current(&clip, &expected));

        let changed = crate::whisper::RecognitionProvenance {
            signature: "recipe-b".into(),
            ..Default::default()
        };
        assert!(!generated_recognition_is_current(&clip, &changed));

        clip.captions_provenance = "edited".into();
        assert_eq!(
            caption_alignment_action(&clip, None, None),
            CaptionAlignmentAction::PreserveEdited,
        );
        assert!(!generated_recognition_is_current(&clip, &expected));

        clip.captions_provenance = "none".into();
        clip.captions_enabled = 0;
        clip.captions_text = None;
        clip.captions_recognition_signature = Some("recipe-a".into());
        assert!(generated_recognition_is_current(&clip, &expected));
        assert!(!generated_recognition_is_current(&clip, &changed));
    }

    #[test]
    fn normalized_export_cues_deduplicate_overlap_windows_and_never_overlap() {
        let srt = "1\n00:00:00,000 --> 00:00:00,800\nhello\n\n\
                   2\n00:00:00,100 --> 00:00:00,900\nhello\n\n\
                   3\n00:00:00,600 --> 00:00:01,200\nthere\n";
        let cues = normalized_srt_cues(srt, 0.0, 2.0);

        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "hello");
        assert_eq!(cues[1].text, "there");
        assert!(cues[0].end <= cues[1].start);
    }

    #[test]
    fn generated_sub_frame_export_cues_are_frame_safe_without_rewriting_words() {
        let srt = "1\n00:00:00,000 --> 00:00:00,010\nand\n\n\
                   2\n00:00:00,010 --> 00:00:00,400\nSlug's\n\n\
                   3\n00:00:08,280 --> 00:00:08,300\nleave\n\n\
                   4\n00:00:08,300 --> 00:00:08,310\nme\n\n\
                   5\n00:00:08,310 --> 00:00:08,410\nalone.\n";
        let original = normalized_srt_cues(srt, 0.0, 9.0);
        let adjusted = frame_safe_generated_cues(original.clone(), "aligned");

        assert_eq!(
            adjusted
                .iter()
                .map(|cue| cue.text.as_str())
                .collect::<Vec<_>>(),
            original
                .iter()
                .map(|cue| cue.text.as_str())
                .collect::<Vec<_>>(),
        );
        assert!(adjusted
            .iter()
            .all(|cue| cue.end - cue.start >= FRAME_SAFE_CAPTION_SECONDS - 1e-9));
        assert!(adjusted.windows(2).all(|pair| pair[0].end <= pair[1].start));
        assert_eq!(
            frame_safe_generated_cues(original.clone(), "edited"),
            original,
        );
    }
}

#[cfg(test)]
mod caption_style_tests {
    use super::{
        build_caption_filter, bundled_caption_font, cardboard_ass_drawings,
        cardboard_uses_black_text, find_ffmpeg, fitted_caption_font_size, get_sub_style,
        prepare_image_glyph_caption_track, prepare_undead_legion_caption_track, tape_riot_ass_text,
        BANGERS_FONT_BYTES, COINY_FONT_BYTES, GOBLIN_BITE_FONT_BYTES, NOSIFER_FONT_BYTES,
        PAPER_MISCHIEF_FONT_BYTES, RUBIK_DIRT_FONT_BYTES, TAPE_RIOT_FONT_BYTES,
    };
    use crate::db::ClipRow;

    fn clip_with_style(style: &str, captions: &str) -> ClipRow {
        ClipRow {
            id: format!("caption-style-test-{}-{style}", std::process::id()),
            highlight_id: "highlight".into(),
            vod_id: "vod".into(),
            title: "Caption style test".into(),
            start_seconds: 0.0,
            end_seconds: 2.0,
            aspect_ratio: "9:16".into(),
            crop_x: None,
            crop_y: None,
            crop_width: None,
            crop_height: None,
            captions_enabled: 1,
            captions_text: Some(captions.into()),
            captions_position: "bottom".into(),
            caption_style: style.into(),
            caption_font_scale: 1.0,
            caption_card_scale: crate::db::DEFAULT_CAPTION_CARD_SCALE,
            caption_y_offset: 0.0,
            captions_source_start: Some(0.0),
            captions_provenance: "aligned".into(),
            captions_pipeline_version: 1,
            caption_audio_mode: "mixed".into(),
            captions_recognition_signature: None,
            captions_language: None,
            caption_audio_stream: None,
            facecam_layout: "none".into(),
            facecam_settings: None,
            context_background_path: None,
            context_background_mode: "blur".into(),
            context_blur_strength: 0.25,
            context_video_y: 0.5,
            full_frame_scale: 1.0,
            render_status: "pending".into(),
            output_path: None,
            thumbnail_path: None,
            created_at: "2026-07-13T00:00:00Z".into(),
            game: None,
            publish_description: None,
            publish_hashtags: None,
            cam_region_norm_override: None,
            cam_fit_mode: None,
            community_clip_mp4_path: None,
            source_kind: "twitch_vod".to_string(),
            source_media_path: None,
            source_fingerprint: None,
            source_recorded_at: None,
        }
    }

    #[test]
    fn cardboard_emphasis_uses_semantic_words_instead_of_random_alternation() {
        assert!(cardboard_uses_black_text("wait"));
        assert!(cardboard_uses_black_text("LET'S GO!"));
        assert!(!cardboard_uses_black_text("taking more damage"));
    }

    #[test]
    fn cardboard_drawing_stays_inside_the_vertical_video_width() {
        let (board, texture) = cardboard_ass_drawings(1080, 1920, 1862, "bottom");
        assert!(board.starts_with("m 97 "));
        assert!(board.contains(" 982 "));
        assert!(!board.contains(" 1080 "));
        assert_eq!(texture.matches("m ").count(), 3);
    }

    #[test]
    fn cardboard_filter_emits_timed_board_texture_and_black_red_hierarchy() {
        let captions =
            "1\n00:00:00,000 --> 00:00:00,500\nhello\n\n2\n00:00:00,600 --> 00:00:01,000\nworld.\n";
        let clip = clip_with_style("bold-white", captions);
        let filter = build_caption_filter(&clip, 1080, 1920, 0.0, 2.0).expect("cardboard filter");
        assert!(filter.starts_with("ass='"));

        let ass_path = std::env::temp_dir().join(format!("clip_{}.ass", clip.id));
        let ass = std::fs::read_to_string(&ass_path).expect("generated ASS file");
        assert_eq!(ass.matches("Cardboard,,").count(), 2);
        assert_eq!(ass.matches("CardboardTexture,,").count(), 2);
        assert!(ass.contains("{\\an2\\pos(540,1862)\\b900\\fs65\\1c&H0C1015&}HELLO"));
        assert!(ass.contains("{\\an2\\pos(540,1862)\\b900\\fs65}WORLD."));
        let _ = std::fs::remove_file(ass_path);
    }

    #[test]
    fn highlight_filter_stages_and_loads_the_bundled_distressed_font() {
        let captions = "1\n00:00:00,000 --> 00:00:01,000\nclutch\n";
        let clip = clip_with_style("fire", captions);
        let filter = build_caption_filter(&clip, 1080, 1920, 0.0, 2.0).expect("highlight filter");
        assert!(filter.contains(":fontsdir='"));

        let font_path = bundled_caption_font("fire").expect("bundled highlight font");
        assert!(font_path.is_file());
        assert_eq!(
            std::fs::metadata(font_path).expect("font metadata").len(),
            RUBIK_DIRT_FONT_BYTES.len() as u64,
        );

        let ass_path = std::env::temp_dir().join(format!("clip_{}.ass", clip.id));
        let ass = std::fs::read_to_string(&ass_path).expect("generated ASS file");
        assert!(ass.contains("Style: Default,Rubik Dirt,75"));
        assert!(ass.contains("{\\an2\\pos(540,1862)\\b400\\fs75}CLUTCH"));
        let _ = std::fs::remove_file(ass_path);
    }

    #[test]
    fn trimmed_imported_captions_shift_to_clip_time_and_use_editor_anchor() {
        let captions =
            "1\n00:00:05,000 --> 00:00:06,000\nclutch\n\n2\n00:00:20,000 --> 00:00:21,000\nlate\n";
        let mut clip = clip_with_style("clean", captions);
        clip.captions_position = "center".into();
        clip.caption_y_offset = 10.0;

        build_caption_filter(&clip, 1080, 1920, 4.5, 10.0).expect("shifted caption filter");
        let ass_path = std::env::temp_dir().join(format!("clip_{}.ass", clip.id));
        let ass = std::fs::read_to_string(&ass_path).expect("generated ASS file");

        assert!(ass.contains("Dialogue: 2,0:00:00.50,0:00:01.50"));
        assert!(ass.contains("\\an5\\pos(540,1152)"));
        assert!(!ass.to_lowercase().contains("late"));
        let _ = std::fs::remove_file(ass_path);
    }

    #[test]
    fn fun_caption_styles_stage_their_bundled_fonts() {
        for (style, expected_name, expected_len) in [
            ("boxed", "Coiny-Regular.ttf", COINY_FONT_BYTES.len()),
            ("minimal", "Nosifer-Regular.ttf", NOSIFER_FONT_BYTES.len()),
            ("comic-pop", "Bangers-Regular.ttf", BANGERS_FONT_BYTES.len()),
            (
                "undead-legion",
                "Bangers-Regular.ttf",
                BANGERS_FONT_BYTES.len(),
            ),
            (
                "tape-riot",
                "ClipGoblinTapeRiot-Regular.ttf",
                TAPE_RIOT_FONT_BYTES.len(),
            ),
            (
                "paper-mischief",
                "ClipGoblinPaperMischief-Regular.ttf",
                PAPER_MISCHIEF_FONT_BYTES.len(),
            ),
            (
                "goblin-bite",
                "ClipGoblinGoblinBite-Regular.ttf",
                GOBLIN_BITE_FONT_BYTES.len(),
            ),
        ] {
            let font_path = bundled_caption_font(style).expect("bundled caption font");
            assert_eq!(
                font_path.file_name().and_then(|name| name.to_str()),
                Some(expected_name),
            );
            assert_eq!(
                std::fs::metadata(font_path).expect("font metadata").len(),
                expected_len as u64,
            );
        }

        let captions = "1\n00:00:00,000 --> 00:00:01,000\nclutch\n";
        let clip = clip_with_style("boxed", captions);
        build_caption_filter(&clip, 1080, 1920, 0.0, 2.0).expect("frosted filter");
        let ass_path = std::env::temp_dir().join(format!("clip_{}.ass", clip.id));
        let ass = std::fs::read_to_string(&ass_path).expect("generated ASS file");
        assert!(ass.contains("Style: Default,Coiny,73,&H00FFFFFF"));
        let _ = std::fs::remove_file(ass_path);
    }

    #[test]
    fn material_styles_render_custom_faces_depth_and_detail_layers() {
        let captions = "1\n00:00:00,000 --> 00:00:01,000\nwait\n";
        let styles = [
            (
                "tape-riot",
                "ClipGoblin Tape Riot",
                75,
                "2CFFB8",
                "86FFE9",
                "2BD7F4",
                "E42F7C",
                "3D1026",
                2,
                5,
                5,
                true,
            ),
            (
                "paper-mischief",
                "ClipGoblin Paper Mischief",
                75,
                "E8F0F3",
                "FFFFFF",
                "88858B",
                "3D353B",
                "66204F",
                2,
                3,
                7,
                false,
            ),
            (
                "goblin-bite",
                "ClipGoblin Goblin Bite",
                85,
                "20FFDF",
                "75FFF4",
                "1E151B",
                "B1287A",
                "320C22",
                2,
                3,
                4,
                true,
            ),
        ];
        for (
            style_id,
            font_name,
            font_size,
            face,
            highlight,
            edge,
            mid,
            deep,
            edge_count,
            mid_count,
            deep_count,
            face_highlight,
        ) in styles
        {
            let clip = clip_with_style(style_id, captions);
            let filter =
                build_caption_filter(&clip, 1080, 1920, 0.0, 2.0).expect("layered caption filter");
            assert!(filter.contains(":fontsdir='"));

            let ass_path = std::env::temp_dir().join(format!("clip_{}.ass", clip.id));
            let ass = std::fs::read_to_string(&ass_path).expect("generated ASS file");
            assert!(ass.contains(&format!(
                "Style: Default,{font_name},{font_size},&H00{face}",
            )));
            assert!(ass.contains(&format!(
                "Style: DepthMid,{font_name},{font_size},&H00{mid}",
            )));
            assert!(ass.contains(&format!(
                "Style: DepthEdge,{font_name},{font_size},&H00{edge}",
            )));
            assert!(ass.contains(&format!(
                "Style: DepthDeep,{font_name},{font_size},&H00{deep}",
            )));
            assert!(ass.contains(&format!(
                "Style: FaceHighlight,{font_name},{font_size},&H00{highlight}",
            )));
            assert_eq!(ass.matches("DepthEdge,,").count(), edge_count);
            assert_eq!(ass.matches("DepthMid,,").count(), mid_count);
            assert_eq!(ass.matches("DepthDeep,,").count(), deep_count);
            assert_eq!(
                ass.matches("FaceHighlight,,").count(),
                usize::from(face_highlight)
            );
            if style_id == "paper-mischief" {
                let paper_style = get_sub_style(style_id);
                assert_eq!(paper_style.outline_colour, "&H30272C");
                assert_eq!(paper_style.outline, 1);
                assert!(ass.contains("Style: DepthContact,ClipGoblin Paper Mischief,75,&H00000000"));
                assert_eq!(ass.matches("DepthContact,,").count(), 1);
                assert!(ass.contains("\\pos(552,1878)\\b400\\fs75\\alpha&H58&\\blur5"));
            } else {
                assert!(!ass.contains("Style: DepthContact,"));
                assert!(!ass.contains("DepthContact,,"));
            }
            assert!(ass.contains("Dialogue: 4,"));
            assert!(ass.contains("Style: MaterialDetail,ClipGoblin "));
            assert!(ass.contains("Dialogue: 5,"));
            if style_id == "goblin-bite" {
                assert!(!ass.contains("Style: MaterialAccent,"));
                assert!(!ass.contains("Dialogue: 6,"));
            } else {
                assert!(ass.contains("Style: MaterialAccent,ClipGoblin "));
                assert!(ass.contains("Dialogue: 6,"));
            }
            if style_id == "tape-riot" {
                assert!(ass.contains("{\\1c&HE42F7C&}W{\\1c&H2CFFB8&}A"));
            } else {
                let emphasis = if style_id == "paper-mischief" {
                    "2CFFB8"
                } else {
                    "FFFFFF"
                };
                assert!(ass.contains(&format!("\\1c&H{emphasis}&}}WAIT")));
            }
            let _ = std::fs::remove_file(ass_path);
        }
    }

    #[test]
    fn tape_riot_alternates_letter_faces_without_corrupting_line_breaks() {
        assert_eq!(
            tape_riot_ass_text("GO\\NNOW", false),
            "{\\1c&H2CFFB8&}G{\\1c&HE42F7C&}O\\N{\\1c&H2CFFB8&}N{\\1c&HE42F7C&}O{\\1c&H2CFFB8&}W",
        );
    }

    #[test]
    fn caption_font_scale_is_bounded_and_long_words_fit_the_vertical_safe_width() {
        let style = get_sub_style("comic-pop");
        let maximum = fitted_caption_font_size(&style, 99.0, "CLUTCH", 1080, 1920, true);
        let minimum = fitted_caption_font_size(&style, 0.1, "CLUTCH", 1080, 1920, true);
        let long_word = "EXTRAORDINARILYLONGREACTIONWORD";
        let long = fitted_caption_font_size(&style, 1.25, long_word, 1080, 1920, true);

        assert_eq!(maximum, 91);
        assert_eq!(minimum, 60);
        assert!(long < maximum);
        assert!(
            long as f64 * long_word.chars().count() as f64 * style.character_width_factor
                <= 1080.0 * style.safe_width_ratio
        );
    }

    #[test]
    fn established_style_defaults_match_their_previous_125_percent_baseline() {
        for (style_id, previous_size, calibrated_size) in [
            ("clean", 52_i32, 65_i32),
            ("bold-white", 52, 65),
            ("boxed", 58, 73),
            ("neon", 54, 68),
            ("fire", 60, 75),
            ("comic-pop", 64, 80),
            ("tape-riot", 60, 75),
            ("paper-mischief", 60, 75),
            ("goblin-bite", 68, 85),
        ] {
            let style = get_sub_style(style_id);
            assert_eq!(style.font_size, calibrated_size, "{style_id} base size");
            assert_eq!(
                calibrated_size,
                (previous_size as f64 * 1.25).round() as i32,
                "{style_id} no longer matches its previous 125% baseline"
            );

            let long_word = "UNCHARACTERISTICALLY";
            let fitted = fitted_caption_font_size(&style, 1.25, long_word, 1080, 1920, true);
            assert!(
                fitted as f64 * long_word.chars().count() as f64 * style.character_width_factor
                    <= 1080.0 * style.safe_width_ratio,
                "{style_id} crossed its vertical safe width"
            );
        }
    }

    #[test]
    fn glossy_thumbnail_uses_its_readable_replacement_calibration() {
        let style = get_sub_style("minimal");
        assert_eq!(style.font_size, 66);

        let long_word = "UNCHARACTERISTICALLY";
        let fitted = fitted_caption_font_size(&style, 1.25, long_word, 1080, 1920, true);
        assert!(
            fitted as f64 * long_word.chars().count() as f64 * style.character_width_factor
                <= 1080.0 * style.safe_width_ratio
        );
    }

    #[test]
    fn undead_legion_export_track_uses_native_glyph_frames() {
        let Ok(ffmpeg) = find_ffmpeg() else {
            return;
        };
        let clip = clip_with_style(
            "undead-legion",
            "1\n00:00:00,000 --> 00:00:01,000\nUNDEAD LEGION\n",
        );
        let request = crate::vertical_crop::ExportRequest {
            source_path: std::path::PathBuf::new(),
            output_path: std::path::PathBuf::new(),
            start: 0.0,
            end: 2.0,
            platform: crate::vertical_crop::Platform::TikTok,
            target: crate::vertical_crop::OutputSize::VERTICAL_720,
            layout: crate::vertical_crop::LayoutMode::GameplayFocus,
            layout_settings: crate::vertical_crop::EditorLayoutSettings::default(),
            caption_filter: None,
            caption_overlay_path: None,
            effective_region: None,
            fit_mode: crate::cam_region::CamFitMode::Fit,
            context_background_mode: crate::vertical_crop::ContextBackgroundMode::Blur,
            context_background_path: None,
            context_blur_strength: 0.3,
            context_video_y: 0.5,
            full_frame_scale: 1.0,
        };

        let track = prepare_undead_legion_caption_track(&ffmpeg, &clip, &request)
            .expect("Undead Legion track")
            .expect("captions enabled");
        assert!(std::fs::metadata(track).is_ok());
    }

    #[test]
    fn reference_image_glyph_export_tracks_use_native_frames() {
        let Ok(ffmpeg) = find_ffmpeg() else {
            return;
        };
        let request = crate::vertical_crop::ExportRequest {
            source_path: std::path::PathBuf::new(),
            output_path: std::path::PathBuf::new(),
            start: 0.0,
            end: 2.0,
            platform: crate::vertical_crop::Platform::TikTok,
            target: crate::vertical_crop::OutputSize::VERTICAL_720,
            layout: crate::vertical_crop::LayoutMode::GameplayFocus,
            layout_settings: crate::vertical_crop::EditorLayoutSettings::default(),
            caption_filter: None,
            caption_overlay_path: None,
            effective_region: None,
            fit_mode: crate::cam_region::CamFitMode::Fit,
            context_background_mode: crate::vertical_crop::ContextBackgroundMode::Blur,
            context_background_path: None,
            context_blur_strength: 0.3,
            context_video_y: 0.5,
            full_frame_scale: 1.0,
        };

        for (style, text) in [
            ("minimal", "THAT WAS WILD"),
            ("hellfire", "NO ONE ESCAPES"),
            ("horror", "DO NOT LOOK BACK"),
            ("scary", "RUN WHILE YOU CAN"),
        ] {
            let captions = format!("1\n00:00:00,000 --> 00:00:01,000\n{text}\n");
            let clip = clip_with_style(style, &captions);
            let track = prepare_image_glyph_caption_track(&ffmpeg, &clip, &request)
                .unwrap_or_else(|error| panic!("{style} track failed: {error}"))
                .expect("captions enabled");
            assert!(std::fs::metadata(track).is_ok(), "missing {style} track");
        }
    }
}

/// Per-style parameters for FFmpeg subtitle rendering.
/// Maps the frontend CaptionStyle definitions in editTypes.ts to FFmpeg filter params.
/// `font_size` matches the editTypes.ts values (designed for 1080px-wide output).
struct SubStyle {
    font_name: &'static str,
    /// Font size in pixels at 1080px-wide reference (matches editTypes.ts fontSize).
    /// Used for both SRT (via original_size) and drawtext paths.
    font_size: i32,
    /// CSS font-weight (100–900).  Mapped to ASS Bold flag (-1 for ≥700, 0 otherwise)
    /// AND injected as `\b<weight>` override for sub-bold granularity (e.g. 800).
    font_weight: i32,
    /// ASS primary colour in &HBBGGRR format (text fill).
    primary_colour: &'static str,
    /// ASS outline colour.
    outline_colour: &'static str,
    /// ASS back colour in &HAABBGGRR (used when border_style=3 for opaque box).
    back_colour: &'static str,
    outline: i32,
    shadow: i32,
    /// 1 = outline + drop shadow, 3 = opaque background box.
    border_style: i32,
    /// Letter spacing in ASS units.
    spacing: f32,
    /// ASS \blur value for the glow layer — gaussian blur radius.
    /// Only used when glow_colour is set.  0 = no glow layer.
    glow_blur: i32,
    /// Glow colour in &HAABBGGRR ASS format.  When non-empty a second "Glow"
    /// ASS style is emitted: same text, larger outline in this colour, blurred,
    /// rendered on a lower layer beneath the crisp foreground.
    glow_colour: &'static str,
    uppercase: bool,
    /// Hex colour for drawtext fontcolor (CSS-order #RRGGBB or named).
    dt_fontcolor: &'static str,
    /// drawtext border width.
    dt_borderw: i32,
    /// Optional drawtext box=1 background colour (empty = no box).
    dt_boxcolor: &'static str,
    /// Approximate average glyph width, relative to font size.
    character_width_factor: f64,
    /// Fraction of frame width captions may occupy.
    safe_width_ratio: f64,
    /// drawtext shadow colour (empty = no shadow).
    dt_shadowcolor: &'static str,
    /// drawtext x/y shadow offset.
    dt_shadow: i32,
}

#[derive(Clone, Copy)]
struct CaptionDepthStyle {
    highlight_colour: &'static str,
    face_highlight: bool,
    edge_colour: &'static str,
    mid_colour: &'static str,
    deep_colour: &'static str,
    contact_colour: &'static str,
    emphasis_colour: &'static str,
    detail_font: &'static str,
    detail_colour: &'static str,
    accent_font: &'static str,
    accent_colour: &'static str,
    edge_offset: i32,
    mid_offset: i32,
    deep_offset: i32,
    contact_offset: i32,
    contact_blur: i32,
    horizontal_scale_percent: i32,
}

fn get_caption_depth_style(id: &str) -> Option<CaptionDepthStyle> {
    match id {
        "tape-riot" => Some(CaptionDepthStyle {
            // Yellow tape edge, purple tape stack, and a dark rear layer.
            highlight_colour: "&H86FFE9",
            face_highlight: true,
            edge_colour: "&H2BD7F4",
            mid_colour: "&HE42F7C",
            deep_colour: "&H3D1026",
            contact_colour: "",
            emphasis_colour: "&HF755A8",
            detail_font: "ClipGoblin Tape Riot Seams",
            detail_colour: "&H371228",
            accent_font: "ClipGoblin Tape Riot Patches",
            accent_colour: "&H26D3FF",
            edge_offset: 2,
            mid_offset: 7,
            deep_offset: 12,
            contact_offset: 0,
            contact_blur: 0,
            horizontal_scale_percent: 100,
        }),
        "paper-mischief" => Some(CaptionDepthStyle {
            // Narrow gray, charcoal, and violet steps lift the paper face without cloning it.
            highlight_colour: "&HFFFFFF",
            face_highlight: false,
            edge_colour: "&H88858B",
            mid_colour: "&H3D353B",
            deep_colour: "&H66204F",
            contact_colour: "&H000000",
            emphasis_colour: "&H2CFFB8",
            detail_font: "ClipGoblin Paper Mischief Fiber",
            detail_colour: "&HB8B5B2",
            accent_font: "ClipGoblin Paper Mischief Tabs",
            accent_colour: "&H24FFAF",
            edge_offset: 2,
            mid_offset: 5,
            deep_offset: 12,
            contact_offset: 16,
            contact_blur: 5,
            horizontal_scale_percent: 78,
        }),
        "goblin-bite" => Some(CaptionDepthStyle {
            // A black separator and long violet extrusion create the horror-poster stack.
            highlight_colour: "&H75FFF4",
            face_highlight: true,
            edge_colour: "&H1E151B",
            mid_colour: "&HB1287A",
            deep_colour: "&H320C22",
            contact_colour: "",
            emphasis_colour: "&HFFFFFF",
            detail_font: "ClipGoblin Goblin Bite Distress",
            detail_colour: "&H0C4630",
            accent_font: "",
            accent_colour: "",
            edge_offset: 2,
            mid_offset: 5,
            deep_offset: 9,
            contact_offset: 0,
            contact_blur: 0,
            horizontal_scale_percent: 100,
        }),
        "undead-legion" => Some(CaptionDepthStyle {
            // Local image rendering is authoritative; these layers are the safe ASS fallback.
            highlight_colour: "&H75FFDC",
            face_highlight: true,
            edge_colour: "&H17100E",
            mid_colour: "&HCD30FF",
            deep_colour: "&H0C0708",
            contact_colour: "",
            emphasis_colour: "&HCD30FF",
            detail_font: "Bangers",
            detail_colour: "&H163428",
            accent_font: "",
            accent_colour: "",
            edge_offset: 2,
            mid_offset: 5,
            deep_offset: 10,
            contact_offset: 0,
            contact_blur: 0,
            horizontal_scale_percent: 100,
        }),
        _ => None,
    }
}

fn tape_riot_ass_text(text: &str, start_with_purple: bool) -> String {
    let mut output = String::with_capacity(text.len() * 8);
    let mut purple = start_with_purple;
    let mut lines = text.split("\\N").peekable();

    while let Some(line) = lines.next() {
        for glyph in line.chars() {
            if glyph.is_alphanumeric() {
                let colour = if purple { "&HE42F7C" } else { "&H2CFFB8" };
                output.push_str("{\\1c");
                output.push_str(colour);
                output.push_str("&}");
                purple = !purple;
            }
            output.push(glyph);
        }
        if lines.peek().is_some() {
            output.push_str("\\N");
        }
    }

    output
}

fn get_sub_style(id: &str) -> SubStyle {
    match id {
        // font_size values match editTypes.ts fontSize (px at 1080px-wide reference).
        // Established styles are calibrated so 100% equals their former 125% appearance.
        // font_weight values match editTypes.ts fontWeight
        "bold-white" => SubStyle {
            // Cardboard: dark red type over a timed, textured ASS sign layer.
            font_name: "Arial Black",
            font_size: 65,
            font_weight: 900,
            // #7A2118 -> ASS BGR &H18217A
            primary_colour: "&H18217A",
            outline_colour: "&H000000",
            back_colour: "&H60000000",
            outline: 0,
            shadow: 1,
            border_style: 1,
            spacing: 0.5,
            glow_blur: 0,
            glow_colour: "",
            uppercase: true,
            dt_fontcolor: "#7A2118",
            dt_borderw: 0,
            dt_boxcolor: "#C99358@0.96",
            character_width_factor: 0.72,
            safe_width_ratio: 0.68,
            dt_shadowcolor: "#3F2310@0.75",
            dt_shadow: 2,
        },
        "boxed" => SubStyle {
            // Frosted: pink candy lettering, white edge, and purple lift.
            font_name: "Coiny",
            font_size: 73,
            font_weight: 400,
            primary_colour: "&HFFFFFF",
            outline_colour: "&HFFFFFF",
            back_colour: "&H006F2055",
            outline: 3,
            shadow: 4,
            border_style: 1,
            spacing: 0.5,
            glow_blur: 5,
            glow_colour: "&HA0D85BF0",
            uppercase: true,
            dt_fontcolor: "white",
            dt_borderw: 0,
            dt_boxcolor: "",
            character_width_factor: 0.72,
            safe_width_ratio: 0.84,
            dt_shadowcolor: "#6D28D9",
            dt_shadow: 4,
        },
        "neon" => SubStyle {
            // Segoe UI is the frontend font on Windows; fall back to Arial
            font_name: "Segoe UI",
            font_size: 68,
            font_weight: 800,
            // #00FF88 → R=00 G=FF B=88 → ASS &HBBGGRR = &H88FF00
            primary_colour: "&H88FF00",
            outline_colour: "&H000000",
            back_colour: "&H00000000",
            // CSS uses 4 stacked black shadows → thick outline.  Outline=4 matches.
            outline: 4,
            shadow: 0,
            border_style: 1,
            spacing: 1.2,
            // Glow layer: bright green, gaussian-blurred behind text
            // CSS: '0 0 8px #00ff8880' (#80 hex ≈ 50% opacity)
            // ASS alpha: 00=opaque FF=transparent → &H80 = 50% transparent = 50% opaque
            glow_blur: 8,
            glow_colour: "&H8088FF00",
            uppercase: true,
            dt_fontcolor: "#00FF88",
            dt_borderw: 3,
            dt_boxcolor: "",
            character_width_factor: 0.66,
            safe_width_ratio: 0.84,
            dt_shadowcolor: "black@0.85",
            dt_shadow: 2,
        },
        "minimal" => SubStyle {
            // Glossy Thumbnail: native image renderer is authoritative. These
            // values keep the emergency ASS fallback recognizable.
            font_name: "Anton",
            font_size: 66,
            font_weight: 400,
            primary_colour: "&H00B4FF",
            outline_colour: "&HDEF8FF",
            back_colour: "&H008B00F6",
            outline: 3,
            shadow: 4,
            border_style: 1,
            spacing: 0.0,
            glow_blur: 0,
            glow_colour: "",
            uppercase: true,
            dt_fontcolor: "#FFB400",
            dt_borderw: 3,
            dt_boxcolor: "",
            character_width_factor: 0.58,
            safe_width_ratio: 0.72,
            dt_shadowcolor: "#7A2A00",
            dt_shadow: 5,
        },
        "fire" => SubStyle {
            font_name: "Rubik Dirt",
            font_size: 75,
            font_weight: 400,
            // #FFE45E -> R=FF G=E4 B=5E -> ASS &HBBGGRR = &H5EE4FF
            primary_colour: "&H5EE4FF",
            outline_colour: "&H000000",
            back_colour: "&H00000000",
            outline: 3,
            shadow: 1,
            border_style: 1,
            spacing: 0.5,
            glow_blur: 0,
            glow_colour: "",
            uppercase: true,
            dt_fontcolor: "#FFE45E",
            dt_borderw: 3,
            dt_boxcolor: "",
            character_width_factor: 0.72,
            safe_width_ratio: 0.84,
            dt_shadowcolor: "black@0.9",
            dt_shadow: 3,
        },
        "comic-pop" => SubStyle {
            // Comic Pop: cyan face with magenta/purple offset comic-book shadow.
            font_name: "Bangers",
            font_size: 80,
            font_weight: 400,
            primary_colour: "&HE6E867",
            outline_colour: "&H6F2055",
            back_colour: "&H00D85BF0",
            outline: 3,
            shadow: 4,
            border_style: 1,
            spacing: 0.8,
            glow_blur: 0,
            glow_colour: "",
            uppercase: true,
            dt_fontcolor: "#67E8E6",
            dt_borderw: 3,
            dt_boxcolor: "",
            character_width_factor: 0.68,
            safe_width_ratio: 0.84,
            dt_shadowcolor: "#F05BD8",
            dt_shadow: 4,
        },
        "tape-riot" => SubStyle {
            // Tape Riot: custom torn tape face plus seam and patch companion fonts.
            font_name: "ClipGoblin Tape Riot",
            font_size: 75,
            font_weight: 400,
            primary_colour: "&H2CFFB8",
            outline_colour: "&H1C1317",
            back_colour: "&H002BD7F4",
            outline: 2,
            shadow: 0,
            border_style: 1,
            spacing: 0.6,
            glow_blur: 0,
            glow_colour: "",
            uppercase: true,
            dt_fontcolor: "#B8FF2C",
            dt_borderw: 3,
            dt_boxcolor: "",
            character_width_factor: 0.72,
            safe_width_ratio: 0.78,
            dt_shadowcolor: "#7C2FE4",
            dt_shadow: 6,
        },
        "paper-mischief" => SubStyle {
            // Paper Mischief: custom torn face with fiber and tape-tab companions.
            font_name: "ClipGoblin Paper Mischief",
            font_size: 75,
            font_weight: 400,
            primary_colour: "&HE8F0F3",
            outline_colour: "&H30272C",
            back_colour: "&H00C0B4B9",
            outline: 1,
            shadow: 0,
            border_style: 1,
            spacing: 0.5,
            glow_blur: 0,
            glow_colour: "",
            uppercase: true,
            dt_fontcolor: "#F3F0E8",
            dt_borderw: 1,
            dt_boxcolor: "",
            character_width_factor: 0.76,
            safe_width_ratio: 0.77,
            dt_shadowcolor: "#5E2A84",
            dt_shadow: 6,
        },
        "goblin-bite" => SubStyle {
            // Goblin Bite: custom bitten silhouette with distressed face companion.
            font_name: "ClipGoblin Goblin Bite",
            font_size: 85,
            font_weight: 400,
            primary_colour: "&H20FFDF",
            outline_colour: "&H191117",
            back_colour: "&H00FF3D8B",
            outline: 2,
            shadow: 0,
            border_style: 1,
            spacing: 0.8,
            glow_blur: 0,
            glow_colour: "",
            uppercase: true,
            dt_fontcolor: "#DFFF20",
            dt_borderw: 3,
            dt_boxcolor: "",
            character_width_factor: 0.58,
            safe_width_ratio: 0.76,
            dt_shadowcolor: "#5C249B",
            dt_shadow: 6,
        },
        "undead-legion" => SubStyle {
            // Undead Legion: native atlas is authoritative; Bangers keeps failures readable.
            font_name: "Bangers",
            font_size: 66,
            font_weight: 400,
            primary_colour: "&H1CFFB2",
            outline_colour: "&H0C0708",
            back_colour: "&H00CD30FF",
            outline: 3,
            shadow: 5,
            border_style: 1,
            spacing: 0.2,
            glow_blur: 0,
            glow_colour: "",
            uppercase: false,
            dt_fontcolor: "#B2FF1C",
            dt_borderw: 3,
            dt_boxcolor: "",
            character_width_factor: 0.68,
            safe_width_ratio: 0.78,
            dt_shadowcolor: "#FF30CD",
            dt_shadow: 5,
        },
        "hellfire" => SubStyle {
            // Hellfire: native image-glyph atlas is authoritative.
            font_name: "Arial",
            font_size: 62,
            font_weight: 700,
            primary_colour: "&HD8D8D8",
            outline_colour: "&H171720",
            back_colour: "&H00341927",
            outline: 2,
            shadow: 4,
            border_style: 1,
            spacing: 0.4,
            glow_blur: 0,
            glow_colour: "",
            uppercase: false,
            dt_fontcolor: "#D8D8D8",
            dt_borderw: 2,
            dt_boxcolor: "",
            character_width_factor: 0.68,
            safe_width_ratio: 0.78,
            dt_shadowcolor: "#6F151C",
            dt_shadow: 5,
        },
        "horror" => SubStyle {
            // Horror: native disintegrating image-glyph atlas is authoritative.
            font_name: "Arial Narrow",
            font_size: 62,
            font_weight: 700,
            primary_colour: "&HECECEC",
            outline_colour: "&H161111",
            back_colour: "&H00100D12",
            outline: 1,
            shadow: 3,
            border_style: 1,
            spacing: 0.2,
            glow_blur: 0,
            glow_colour: "",
            uppercase: false,
            dt_fontcolor: "#ECECEC",
            dt_borderw: 1,
            dt_boxcolor: "",
            character_width_factor: 0.58,
            safe_width_ratio: 0.78,
            dt_shadowcolor: "#111116",
            dt_shadow: 4,
        },
        "scary" => SubStyle {
            // Scary: native cleaned dry-brush image-glyph atlas is authoritative.
            font_name: "Arial Narrow",
            font_size: 62,
            font_weight: 700,
            primary_colour: "&H1C14D4",
            outline_colour: "&H060414",
            back_colour: "&H00100709",
            outline: 1,
            shadow: 3,
            border_style: 1,
            spacing: 0.2,
            glow_blur: 0,
            glow_colour: "",
            uppercase: true,
            dt_fontcolor: "#D4141C",
            dt_borderw: 1,
            dt_boxcolor: "",
            character_width_factor: 0.58,
            safe_width_ratio: 0.78,
            dt_shadowcolor: "#140406",
            dt_shadow: 4,
        },
        // "clean" and any unknown style
        _ => SubStyle {
            font_name: "Arial",
            font_size: 65,
            font_weight: 700,
            primary_colour: "&HFFFFFF",
            outline_colour: "&H000000",
            back_colour: "&H00000000",
            outline: 2,
            shadow: 0,
            border_style: 1,
            spacing: 0.4,
            glow_blur: 0,
            glow_colour: "",
            uppercase: false,
            dt_fontcolor: "white",
            dt_borderw: 3,
            dt_boxcolor: "",
            character_width_factor: 0.66,
            safe_width_ratio: 0.84,
            dt_shadowcolor: "black@0.85",
            dt_shadow: 2,
        },
    }
}

fn caption_fit_units(text: &str, wraps: bool) -> usize {
    let normalized = text.replace("\\N", " ");
    if wraps {
        normalized
            .split_whitespace()
            .map(|word| word.chars().count())
            .max()
            .unwrap_or(1)
            .max(1)
    } else {
        normalized
            .lines()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(1)
            .max(1)
    }
}

fn fitted_caption_font_size(
    style: &SubStyle,
    font_scale: f64,
    text: &str,
    target_width: i32,
    target_height: i32,
    wraps: bool,
) -> i32 {
    let requested = style.font_size as f64 * db::normalize_caption_font_scale(font_scale);
    let hard_max_ratio = if target_height > target_width {
        0.085
    } else {
        0.065
    };
    let hard_max = target_width.max(1) as f64 * hard_max_ratio;
    let units = caption_fit_units(text, wraps) as f64;
    let width_fit = target_width.max(1) as f64 * style.safe_width_ratio
        / (units * style.character_width_factor);

    requested.min(hard_max).min(width_fit).max(8.0).floor() as i32
}

fn parse_srt_time_seconds(srt: &str) -> Option<f64> {
    let timestamp = srt.trim().split_whitespace().next()?.replace(',', ".");
    let mut parts = timestamp.split(':');
    let hours: f64 = parts.next()?.parse().ok()?;
    let minutes: f64 = parts.next()?.parse().ok()?;
    let seconds: f64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !hours.is_finite() || !minutes.is_finite() || !seconds.is_finite()
    {
        return None;
    }
    Some((hours * 3600.0 + minutes * 60.0 + seconds).max(0.0))
}

#[derive(Debug, Clone, PartialEq)]
struct SrtCue {
    start: f64,
    end: f64,
    text: String,
}

fn parse_srt_cues(srt: &str) -> Vec<SrtCue> {
    let normalized = srt.replace("\r\n", "\n").replace('\r', "\n");
    normalized
        .split("\n\n")
        .filter_map(|block| {
            let lines: Vec<&str> = block.lines().collect();
            let timing_index = lines.iter().position(|line| line.contains("-->"))?;
            let (raw_start, raw_end) = lines[timing_index].split_once("-->")?;
            let start = parse_srt_time_seconds(raw_start)?;
            let end = parse_srt_time_seconds(raw_end)?;
            let text = lines[timing_index + 1..]
                .iter()
                .map(|line| line.trim())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if end <= start || !text.chars().any(char::is_alphanumeric) {
                None
            } else {
                Some(SrtCue { start, end, text })
            }
        })
        .collect()
}

fn normalized_srt_cues(srt: &str, caption_time_offset: f64, clip_duration: f64) -> Vec<SrtCue> {
    let mut cues = parse_srt_cues(srt);
    cues.sort_by(|left, right| {
        left.start
            .total_cmp(&right.start)
            .then_with(|| left.end.total_cmp(&right.end))
    });

    let mut deduplicated: Vec<SrtCue> = Vec::new();
    for cue in cues {
        let duplicate = deduplicated.last().is_some_and(|previous| {
            previous.text.trim().eq_ignore_ascii_case(cue.text.trim())
                && (cue.start - previous.start).abs() <= 0.18
                && cue.start < previous.end + 0.04
        });
        if !duplicate {
            deduplicated.push(cue);
        }
    }

    let ordered = deduplicated.clone();
    deduplicated
        .into_iter()
        .enumerate()
        .filter_map(|(index, cue)| {
            let next_start = ordered
                .get(index + 1)
                .map(|next| next.start)
                .unwrap_or(f64::INFINITY);
            let shifted_start = cue.start - caption_time_offset;
            let shifted_end = cue.end.min(next_start) - caption_time_offset;
            if shifted_end <= 0.0 || (clip_duration > 0.0 && shifted_start >= clip_duration) {
                return None;
            }
            let start = shifted_start.max(0.0);
            let end = if clip_duration > 0.0 {
                shifted_end.min(clip_duration)
            } else {
                shifted_end
            };
            (end > start).then_some(SrtCue {
                start,
                end,
                text: cue.text,
            })
        })
        .collect()
}

fn frame_safe_generated_cues(mut cues: Vec<SrtCue>, provenance: &str) -> Vec<SrtCue> {
    if !matches!(provenance, "aligned" | "analysis-draft")
        || !cues
            .iter()
            .any(|cue| cue.end - cue.start < FRAME_SAFE_CAPTION_SECONDS)
    {
        return cues;
    }

    for index in 0..cues.len() {
        if cues[index].end - cues[index].start >= FRAME_SAFE_CAPTION_SECONDS {
            continue;
        }

        let frame_safe_end = cues[index].start + FRAME_SAFE_CAPTION_SECONDS;
        cues[index].end = frame_safe_end;
        if index + 1 < cues.len() && cues[index + 1].start < frame_safe_end {
            cues[index + 1].start = frame_safe_end;
        }
    }
    cues
}

fn valid_srt_cue_count(srt: &str) -> usize {
    parse_srt_cues(srt).len()
}

fn caption_anchor(clip: &db::ClipRow, target_height: i32) -> (i32, i32) {
    let (caption_base_y, caption_alignment) = match clip.captions_position.as_str() {
        "top" => (8.0, 8),
        "center" => (50.0, 5),
        _ => (97.0, 2),
    };
    let caption_y_percent =
        (caption_base_y + db::normalize_caption_y_offset(clip.caption_y_offset)).clamp(3.0, 97.0);
    let caption_anchor_y =
        ((target_height as f64 * caption_y_percent / 100.0).round() as i32).clamp(0, target_height);
    (caption_anchor_y, caption_alignment)
}

fn ffconcat_path(path: &std::path::Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .replace('\'', "\\'")
}

fn normalized_image_caption_cues(
    clip: &db::ClipRow,
    request: &vertical_crop::ExportRequest,
) -> Option<(Vec<SrtCue>, f64)> {
    if clip.captions_enabled != 1 {
        return None;
    }
    let srt = clip.captions_text.as_deref()?;
    let clip_duration = (request.end - request.start).max(0.0);
    if clip_duration <= 0.0 || valid_srt_cue_count(srt) == 0 {
        return None;
    }
    let captions_source_start = clip
        .captions_source_start
        .filter(|value| value.is_finite())
        .unwrap_or_else(|| {
            if clip
                .source_media_path
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty())
                || clip
                    .community_clip_mp4_path
                    .as_deref()
                    .is_some_and(|path| !path.trim().is_empty())
            {
                0.0
            } else {
                clip.start_seconds
            }
        });
    let cues = frame_safe_generated_cues(
        normalized_srt_cues(srt, request.start - captions_source_start, clip_duration),
        &clip.captions_provenance,
    );
    (!cues.is_empty()).then_some((cues, clip_duration))
}

fn render_image_caption_timeline(
    ffmpeg: &std::path::Path,
    cache_dir: &std::path::Path,
    renderer_version: &str,
    label: &str,
    target_width: u32,
    target_height: u32,
    clip_duration: f64,
    mut timeline: Vec<(std::path::PathBuf, f64)>,
) -> Result<Option<std::path::PathBuf>, String> {
    timeline.retain(|(_, duration)| duration.is_finite() && *duration > 0.000_5);
    let Some((last_path, _)) = timeline.last() else {
        return Ok(None);
    };

    let mut hasher = Sha256::new();
    hasher.update(renderer_version.as_bytes());
    hasher.update(target_width.to_le_bytes());
    hasher.update(target_height.to_le_bytes());
    hasher.update(clip_duration.to_le_bytes());
    for (path, duration) in &timeline {
        hasher.update(path.to_string_lossy().as_bytes());
        hasher.update(duration.to_le_bytes());
    }
    let key = format!("{:x}", hasher.finalize());
    let track_path = cache_dir.join(format!("track-{key}.mov"));
    if artifact_file_is_ready(&track_path) {
        return Ok(Some(track_path));
    }

    let concat_path = cache_dir.join(format!("track-{key}.ffconcat"));
    let mut concat = String::from("ffconcat version 1.0\n");
    for (path, duration) in &timeline {
        concat.push_str(&format!(
            "file '{}'\nduration {duration:.6}\n",
            ffconcat_path(path),
        ));
    }
    concat.push_str(&format!("file '{}'\n", ffconcat_path(last_path)));
    std::fs::write(&concat_path, concat)
        .map_err(|error| format!("Could not stage {label} timeline: {error}"))?;

    let temp_path = cache_dir.join(format!(
        ".track-{key}-{}.mov",
        uuid::Uuid::new_v4().simple()
    ));
    let mut command = std::process::Command::new(ffmpeg);
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-f")
        .arg("concat")
        .arg("-safe")
        .arg("0")
        .arg("-i")
        .arg(&concat_path)
        .arg("-fps_mode")
        .arg("vfr")
        .arg("-t")
        .arg(format!("{clip_duration:.6}"))
        .arg("-c:v")
        .arg("qtrle")
        .arg("-pix_fmt")
        .arg("argb")
        .arg("-an")
        .arg(&temp_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command
        .output()
        .map_err(|error| format!("Could not start {label} timeline rendering: {error}"))?;
    if !output.status.success() || !artifact_file_is_ready(&temp_path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!(
            "{label} timeline rendering failed: {}",
            ffmpeg_error_tail(&output.stderr)
        ));
    }
    match std::fs::rename(&temp_path, &track_path) {
        Ok(()) => {}
        Err(_) if artifact_file_is_ready(&track_path) => {
            let _ = std::fs::remove_file(&temp_path);
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temp_path);
            return Err(format!("Could not cache {label} timeline: {error}"));
        }
    }
    Ok(Some(track_path))
}

fn prepare_paper_mischief_caption_track(
    ffmpeg: &std::path::Path,
    clip: &db::ClipRow,
    request: &vertical_crop::ExportRequest,
) -> Result<Option<std::path::PathBuf>, String> {
    if clip.captions_enabled != 1 || clip.caption_style != "paper-mischief" {
        return Ok(None);
    }
    let Some((cues, clip_duration)) = normalized_image_caption_cues(clip, request) else {
        return Ok(None);
    };

    let style = get_sub_style("paper-mischief");
    let (anchor_y, alignment) = caption_anchor(clip, request.target.height as i32);
    let blank =
        render_transparent_caption_frame(ffmpeg, request.target.width, request.target.height)?;
    let mut timeline: Vec<(std::path::PathBuf, f64)> = Vec::new();
    let mut cursor = 0.0;
    for cue in cues {
        if cue.start > cursor + 0.001 {
            timeline.push((blank.clone(), cue.start - cursor));
        }
        let font_size = fitted_caption_font_size(
            &style,
            clip.caption_font_scale,
            &cue.text,
            request.target.width as i32,
            request.target.height as i32,
            true,
        ) as u32;
        let frame = render_paper_mischief_frame(
            ffmpeg,
            &PaperMischiefCaptionRequest {
                text: cue.text,
                target_width: request.target.width,
                target_height: request.target.height,
                font_size,
                anchor_y,
                alignment,
            },
        )?;
        timeline.push((frame, cue.end - cue.start));
        cursor = cue.end;
    }
    if cursor < clip_duration - 0.001 {
        timeline.push((blank, clip_duration - cursor));
    }
    let cache_dir = paper_mischief_cache_dir()?;
    render_image_caption_timeline(
        ffmpeg,
        &cache_dir,
        PAPER_MISCHIEF_RENDERER_VERSION,
        "Paper Mischief",
        request.target.width,
        request.target.height,
        clip_duration,
        timeline,
    )
}

fn prepare_cardboard_caption_track(
    ffmpeg: &std::path::Path,
    clip: &db::ClipRow,
    request: &vertical_crop::ExportRequest,
) -> Result<Option<std::path::PathBuf>, String> {
    if clip.captions_enabled != 1 || clip.caption_style != "bold-white" {
        return Ok(None);
    }
    let Some((cues, clip_duration)) = normalized_image_caption_cues(clip, request) else {
        return Ok(None);
    };

    let style = get_sub_style("bold-white");
    let (anchor_y, alignment) = caption_anchor(clip, request.target.height as i32);
    let card_only = cardboard_caption::render_caption_frame(&CardboardCaptionRequest {
        text: String::new(),
        target_width: request.target.width,
        target_height: request.target.height,
        font_size: style.font_size.max(8) as u32,
        card_scale: clip.caption_card_scale,
        anchor_y,
        alignment,
    })?;
    let mut timeline: Vec<(std::path::PathBuf, f64)> = Vec::new();
    let mut cursor = 0.0;
    for cue in cues {
        if cue.start > cursor + 0.001 {
            timeline.push((card_only.clone(), cue.start - cursor));
        }
        let font_size = fitted_caption_font_size(
            &style,
            clip.caption_font_scale,
            &cue.text,
            request.target.width as i32,
            request.target.height as i32,
            true,
        ) as u32;
        let frame = cardboard_caption::render_caption_frame(&CardboardCaptionRequest {
            text: cue.text,
            target_width: request.target.width,
            target_height: request.target.height,
            font_size,
            card_scale: clip.caption_card_scale,
            anchor_y,
            alignment,
        })?;
        timeline.push((frame, cue.end - cue.start));
        cursor = cue.end;
    }
    if cursor < clip_duration - 0.001 {
        timeline.push((card_only, clip_duration - cursor));
    }
    let cache_dir = cardboard_caption::cache_dir()?;
    render_image_caption_timeline(
        ffmpeg,
        &cache_dir,
        cardboard_caption::RENDERER_VERSION,
        "Cardboard",
        request.target.width,
        request.target.height,
        clip_duration,
        timeline,
    )
}

fn prepare_undead_legion_caption_track(
    ffmpeg: &std::path::Path,
    clip: &db::ClipRow,
    request: &vertical_crop::ExportRequest,
) -> Result<Option<std::path::PathBuf>, String> {
    if clip.captions_enabled != 1 || clip.caption_style != "undead-legion" {
        return Ok(None);
    }
    let Some((cues, clip_duration)) = normalized_image_caption_cues(clip, request) else {
        return Ok(None);
    };

    let style = get_sub_style("undead-legion");
    let (anchor_y, alignment) = caption_anchor(clip, request.target.height as i32);
    let blank =
        render_transparent_caption_frame(ffmpeg, request.target.width, request.target.height)?;
    let mut timeline: Vec<(std::path::PathBuf, f64)> = Vec::new();
    let mut cursor = 0.0;
    for cue in cues {
        if cue.start > cursor + 0.001 {
            timeline.push((blank.clone(), cue.start - cursor));
        }
        let font_size = fitted_caption_font_size(
            &style,
            clip.caption_font_scale,
            &cue.text,
            request.target.width as i32,
            request.target.height as i32,
            true,
        ) as u32;
        let frame = undead_legion::render_caption_frame(&UndeadLegionCaptionRequest {
            text: cue.text,
            target_width: request.target.width,
            target_height: request.target.height,
            font_size,
            anchor_y,
            alignment,
        })?;
        timeline.push((frame, cue.end - cue.start));
        cursor = cue.end;
    }
    if cursor < clip_duration - 0.001 {
        timeline.push((blank, clip_duration - cursor));
    }
    let cache_dir = undead_legion::cache_dir()?;
    render_image_caption_timeline(
        ffmpeg,
        &cache_dir,
        undead_legion::RENDERER_VERSION,
        "Undead Legion",
        request.target.width,
        request.target.height,
        clip_duration,
        timeline,
    )
}

fn prepare_image_glyph_caption_track(
    ffmpeg: &std::path::Path,
    clip: &db::ClipRow,
    request: &vertical_crop::ExportRequest,
) -> Result<Option<std::path::PathBuf>, String> {
    let Some(renderer_version) = image_glyph_caption::renderer_version(&clip.caption_style) else {
        return Ok(None);
    };
    if clip.captions_enabled != 1 {
        return Ok(None);
    }
    let Some((cues, clip_duration)) = normalized_image_caption_cues(clip, request) else {
        return Ok(None);
    };

    let style = get_sub_style(&clip.caption_style);
    let (anchor_y, alignment) = caption_anchor(clip, request.target.height as i32);
    let blank =
        render_transparent_caption_frame(ffmpeg, request.target.width, request.target.height)?;
    let mut timeline: Vec<(std::path::PathBuf, f64)> = Vec::new();
    let mut cursor = 0.0;
    for cue in cues {
        if cue.start > cursor + 0.001 {
            timeline.push((blank.clone(), cue.start - cursor));
        }
        let font_size = fitted_caption_font_size(
            &style,
            clip.caption_font_scale,
            &cue.text,
            request.target.width as i32,
            request.target.height as i32,
            true,
        ) as u32;
        let frame = image_glyph_caption::render_caption_frame(&ImageGlyphCaptionRequest {
            style_id: clip.caption_style.clone(),
            text: cue.text,
            target_width: request.target.width,
            target_height: request.target.height,
            font_size,
            anchor_y,
            alignment,
        })?;
        timeline.push((frame, cue.end - cue.start));
        cursor = cue.end;
    }
    if cursor < clip_duration - 0.001 {
        timeline.push((blank, clip_duration - cursor));
    }
    let cache_dir = image_glyph_caption::cache_dir(&clip.caption_style)?;
    let display_name =
        image_glyph_caption::display_name(&clip.caption_style).unwrap_or("Image glyph");
    render_image_caption_timeline(
        ffmpeg,
        &cache_dir,
        renderer_version,
        display_name,
        request.target.width,
        request.target.height,
        clip_duration,
        timeline,
    )
}

fn attach_image_caption_track(
    ffmpeg: &std::path::Path,
    clip: &db::ClipRow,
    request: &mut vertical_crop::ExportRequest,
) {
    let result = match clip.caption_style.as_str() {
        "bold-white" => prepare_cardboard_caption_track(ffmpeg, clip, request),
        "paper-mischief" => prepare_paper_mischief_caption_track(ffmpeg, clip, request),
        "undead-legion" => prepare_undead_legion_caption_track(ffmpeg, clip, request),
        style_id if image_glyph_caption::renderer_version(style_id).is_some() => {
            prepare_image_glyph_caption_track(ffmpeg, clip, request)
        }
        _ => return,
    };
    match result {
        Ok(Some(path)) => {
            request.caption_overlay_path = Some(path);
            request.caption_filter = None;
        }
        Ok(None) => {}
        Err(error) => {
            log::warn!(
                "[export] {} image renderer unavailable; using ASS fallback: {error}",
                clip.caption_style,
            );
        }
    }
}

/// Convert non-negative seconds to ASS timestamp "H:MM:SS.cc".
fn seconds_to_ass_time(seconds: f64) -> String {
    let total_centiseconds = (seconds.max(0.0) * 100.0).round() as u64;
    let hours = total_centiseconds / 360_000;
    let minutes = (total_centiseconds / 6_000) % 60;
    let secs = (total_centiseconds / 100) % 60;
    let centiseconds = total_centiseconds % 100;
    format!("{hours}:{minutes:02}:{secs:02}.{centiseconds:02}")
}

/// Build the caption filter string from clip settings.
/// Returns None if captions are disabled or empty.
pub(crate) fn build_caption_filter(
    clip: &db::ClipRow,
    target_width: i32,
    target_height: i32,
    caption_time_offset: f64,
    clip_duration: f64,
) -> Option<String> {
    if clip.captions_enabled != 1 {
        return None;
    }
    let text = clip.captions_text.as_ref()?;
    if text.is_empty() {
        return None;
    }

    let style = get_sub_style(&clip.caption_style);
    let is_cardboard = clip.caption_style == "bold-white";
    let depth_style = get_caption_depth_style(&clip.caption_style);
    let bundled_font_path = bundled_caption_font(&clip.caption_style);
    let is_srt = valid_srt_cue_count(text) > 0;

    // Match the editor's anchor semantics exactly: top grows downward, center
    // grows around the anchor, and bottom grows upward.
    let (caption_anchor_y, caption_alignment) = caption_anchor(clip, target_height);
    let caption_position_tag = format!(
        "\\an{}\\pos({},{})",
        caption_alignment,
        target_width / 2,
        caption_anchor_y,
    );
    let margin_h =
        ((target_width as f64 * (1.0 - style.safe_width_ratio) / 2.0).round() as i32).max(10);
    let default_font_size = fitted_caption_font_size(
        &style,
        clip.caption_font_scale,
        "caption",
        target_width,
        target_height,
        true,
    );

    if is_srt {
        // ── Convert SRT → ASS with explicit PlayRes ──
        // Writing a full ASS file with PlayResX/PlayResY matching the export
        // resolution gives us pixel-accurate FontSize control.  The default
        // SRT→ASS path in libass uses an unpredictable internal PlayRes which
        // causes wild font-size scaling.

        // ASS Bold field: -1 = bold (≥700), 0 = normal
        let bold_flag: i32 = if style.font_weight >= 700 { -1 } else { 0 };

        let has_glow = !style.glow_colour.is_empty();
        let cardboard_drawings = is_cardboard.then(|| {
            cardboard_ass_drawings(
                target_width,
                target_height,
                caption_anchor_y,
                &clip.captions_position,
            )
        });
        // ASS header — PlayRes matches export resolution so FontSize = pixels
        let mut ass = format!("\
[Script Info]\r\n\
ScriptType: v4.00+\r\n\
PlayResX: {tw}\r\n\
PlayResY: {th}\r\n\
WrapStyle: 0\r\n\
ScaledBorderAndShadow: yes\r\n\
\r\n\
[V4+ Styles]\r\n\
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\r\n\
Style: Default,{fn_},{fs},&H00{pc},&H00FFFFFF,&H00{oc},{bc},{bold},0,0,0,100,100,{sp:.1},0,{bs},{ol},{sh},{an},{mh},{mh},0,1\r\n",
            tw = target_width,
            th = target_height,
            fn_ = style.font_name,
            fs = default_font_size,
            pc = &style.primary_colour[2..],  // strip "&H" prefix — ASS V4+ uses &HAABBGGRR
            oc = &style.outline_colour[2..],
            bc = style.back_colour,
            bold = bold_flag,
            sp = style.spacing,
            bs = style.border_style,
            ol = style.outline,
            sh = style.shadow,
            mh = margin_h,
            an = caption_alignment,
        );

        if is_cardboard {
            ass.push_str("\
Style: Cardboard,Arial,20,&H005893C9,&H005893C9,&H00304B78,&H50000000,0,0,0,0,100,100,0,0,1,2,2,7,0,0,0,1\r\n\
Style: CardboardTexture,Arial,20,&H902B4C7A,&H902B4C7A,&H902B4C7A,&H00000000,0,0,0,0,100,100,0,0,1,0,0,7,0,0,0,1\r\n");
        }

        if let Some(depth) = depth_style {
            ass.push_str(&format!("\
Style: DepthDeep,{fn_},{fs},&H00{deep},&H00{deep},&H00{deep},&H00000000,{bold},0,0,0,100,100,{sp:.1},0,1,1,0,{an},{mh},{mh},0,1\r\n\
Style: DepthMid,{fn_},{fs},&H00{mid},&H00{mid},&H00{mid},&H00000000,{bold},0,0,0,100,100,{sp:.1},0,1,1,0,{an},{mh},{mh},0,1\r\n\
Style: DepthEdge,{fn_},{fs},&H00{edge},&H00{edge},&H00{edge},&H00000000,{bold},0,0,0,100,100,{sp:.1},0,1,1,0,{an},{mh},{mh},0,1\r\n\
Style: FaceHighlight,{fn_},{fs},&H00{highlight},&H00{highlight},&H00{highlight},&H00000000,{bold},0,0,0,100,100,{sp:.1},0,1,1,0,{an},{mh},{mh},0,1\r\n",
                fn_ = style.font_name,
                fs = default_font_size,
                deep = &depth.deep_colour[2..],
                mid = &depth.mid_colour[2..],
                edge = &depth.edge_colour[2..],
                highlight = &depth.highlight_colour[2..],
                bold = bold_flag,
                sp = style.spacing,
                an = caption_alignment,
                mh = margin_h,
            ));
            if depth.contact_offset > 0 {
                ass.push_str(&format!(
                    "Style: DepthContact,{fn_},{fs},&H00{colour},&H00{colour},&H00{colour},&H00000000,{bold},0,0,0,100,100,{sp:.1},0,1,2,0,{an},{mh},{mh},0,1\r\n",
                    fn_ = style.font_name,
                    fs = default_font_size,
                    colour = &depth.contact_colour[2..],
                    bold = bold_flag,
                    sp = style.spacing,
                    an = caption_alignment,
                    mh = margin_h,
                ));
            }
            ass.push_str(&format!(
                "Style: MaterialDetail,{font},{fs},&H00{colour},&H00{colour},&H00{colour},&H00000000,0,0,0,0,100,100,{sp:.1},0,1,0,0,{an},{mh},{mh},0,1\r\n",
                font = depth.detail_font,
                fs = default_font_size,
                colour = &depth.detail_colour[2..],
                sp = style.spacing,
                an = caption_alignment,
                mh = margin_h,
            ));
            if !depth.accent_font.is_empty() {
                ass.push_str(&format!(
                    "Style: MaterialAccent,{font},{fs},&H00{colour},&H00{colour},&H00{colour},&H00000000,0,0,0,0,100,100,{sp:.1},0,1,0,0,{an},{mh},{mh},0,1\r\n",
                    font = depth.accent_font,
                    fs = default_font_size,
                    colour = &depth.accent_colour[2..],
                    sp = style.spacing,
                    an = caption_alignment,
                    mh = margin_h,
                ));
            }
        }

        // Optional glow layer style: creates a luminous halo behind the crisp text.
        // - PrimaryColour: fully opaque glow colour (bright centre)
        // - OutlineColour: semi-transparent glow colour (fading edges)
        // - Large outline (8px) provides the glow spread area
        // - The \blur override in each Dialogue line gaussian-blurs everything
        if has_glow {
            // Fully opaque version of glow colour (replace alpha byte with 00)
            let glow_opaque = format!("&H00{}", &style.glow_colour[4..]);
            ass.push_str(&format!("\
Style: Glow,{fn_},{fs},{go},{go},{gc},&H00000000,{bold},0,0,0,100,100,{sp:.1},0,1,8,0,{an},{mh},{mh},0,1\r\n",
                fn_ = style.font_name,
                fs = default_font_size,
                go = glow_opaque,      // fully opaque green for primary/secondary
                gc = style.glow_colour, // semi-transparent green for outline
                bold = bold_flag,
                sp = style.spacing,
                mh = margin_h,
                an = caption_alignment,
            ));
        }

        ass.push_str(
            "\r\n\
[Events]\r\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\n",
        );

        // Preview and export share one globally ordered cue contract: duplicate
        // overlap windows are removed and every cue ends before the next begins.
        let cues = frame_safe_generated_cues(
            normalized_srt_cues(text, caption_time_offset, clip_duration),
            &clip.captions_provenance,
        );
        let mut cardboard_sentence_start = true;
        for cue in &cues {
            let start_ass = seconds_to_ass_time(cue.start);
            let end_ass = seconds_to_ass_time(cue.end);
            let sub_text = cue.text.replace('\n', "\\N");
            let sub_text = if style.uppercase {
                sub_text.to_uppercase()
            } else {
                sub_text
            };

            // \b<weight> override for precise font weight (e.g. \b800 for extra-bold)
            let weight_tag = format!("\\b{}", style.font_weight);
            let size_tag = format!(
                "\\fs{}",
                fitted_caption_font_size(
                    &style,
                    clip.caption_font_scale,
                    &sub_text,
                    target_width,
                    target_height,
                    true,
                )
            );
            let semantic_emphasis = cardboard_uses_black_text(&sub_text);
            let cardboard_black = is_cardboard && (cardboard_sentence_start || semantic_emphasis);
            let is_tape_riot = clip.caption_style == "tape-riot";
            let colour_tag = if cardboard_black {
                // #15100C -> ASS BGR &H0C1015
                "\\1c&H0C1015&".to_string()
            } else if semantic_emphasis && !is_tape_riot {
                depth_style
                    .map(|depth| format!("\\1c{}&", depth.emphasis_colour))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let foreground_text = if is_tape_riot {
                tape_riot_ass_text(&sub_text, semantic_emphasis)
            } else {
                sub_text.clone()
            };
            if is_cardboard {
                let sentence_tail = sub_text.trim_end_matches(|character: char| {
                    character.is_whitespace() || matches!(character, '"' | '\'' | ')' | ']')
                });
                cardboard_sentence_start =
                    matches!(sentence_tail.chars().last(), Some('.' | '!' | '?'));
            }

            if let Some((board, texture)) = &cardboard_drawings {
                ass.push_str(&format!(
                    "Dialogue: 0,{},{},Cardboard,,0,0,0,,{{\\an7\\pos(0,0)\\p1}}{}{{\\p0}}\r\n",
                    start_ass, end_ass, board,
                ));
                ass.push_str(&format!(
                            "Dialogue: 1,{},{},CardboardTexture,,0,0,0,,{{\\an7\\pos(0,0)\\p1}}{}{{\\p0}}\r\n",
                            start_ass, end_ass, texture,
                        ));
            }

            if let Some(depth) = depth_style {
                let depth_x = |offset: i32| {
                    target_width / 2 + (offset * depth.horizontal_scale_percent + 50) / 100
                };
                if depth.contact_offset > 0 {
                    ass.push_str(&format!(
                                "Dialogue: 0,{},{},DepthContact,,0,0,0,,{{\\an{}\\pos({},{}){wt}{size}\\alpha&H58&\\blur{blur}}}{txt}\r\n",
                                start_ass,
                                end_ass,
                                caption_alignment,
                                depth_x(depth.contact_offset),
                                caption_anchor_y + depth.contact_offset,
                                wt = weight_tag,
                                size = size_tag,
                                blur = depth.contact_blur,
                                txt = sub_text,
                            ));
                }
                for offset in ((depth.mid_offset + 1)..=depth.deep_offset).rev() {
                    ass.push_str(&format!(
                                "Dialogue: 0,{},{},DepthDeep,,0,0,0,,{{\\an{}\\pos({},{}){wt}{size}}}{txt}\r\n",
                                start_ass,
                                end_ass,
                                caption_alignment,
                                depth_x(offset),
                                caption_anchor_y + offset,
                                wt = weight_tag,
                                size = size_tag,
                                txt = sub_text,
                            ));
                }
                for offset in ((depth.edge_offset + 1)..=depth.mid_offset).rev() {
                    ass.push_str(&format!(
                                "Dialogue: 1,{},{},DepthMid,,0,0,0,,{{\\an{}\\pos({},{}){wt}{size}}}{txt}\r\n",
                                start_ass,
                                end_ass,
                                caption_alignment,
                                depth_x(offset),
                                caption_anchor_y + offset,
                                wt = weight_tag,
                                size = size_tag,
                                txt = sub_text,
                            ));
                }
                for offset in (1..=depth.edge_offset).rev() {
                    ass.push_str(&format!(
                                "Dialogue: 2,{},{},DepthEdge,,0,0,0,,{{\\an{}\\pos({},{}){wt}{size}}}{txt}\r\n",
                                start_ass,
                                end_ass,
                                caption_alignment,
                                depth_x(offset),
                                caption_anchor_y + offset,
                                wt = weight_tag,
                                size = size_tag,
                                txt = sub_text,
                            ));
                }
                if depth.face_highlight {
                    ass.push_str(&format!(
                                "Dialogue: 3,{},{},FaceHighlight,,0,0,0,,{{\\an{}\\pos({},{}){wt}{size}}}{txt}\r\n",
                                start_ass,
                                end_ass,
                                caption_alignment,
                                target_width / 2 - 1,
                                caption_anchor_y - 1,
                                wt = weight_tag,
                                size = size_tag,
                                txt = sub_text,
                            ));
                }
            }

            // If glow style exists, emit a blurred glow layer on Layer 0
            if has_glow {
                ass.push_str(&format!(
                    "Dialogue: 0,{},{},Glow,,0,0,0,,{{{pos}{wt}{size}\\blur{blur}}}{txt}\r\n",
                    start_ass,
                    end_ass,
                    pos = caption_position_tag,
                    wt = weight_tag,
                    size = size_tag,
                    blur = style.glow_blur,
                    txt = sub_text
                ));
            }
            // Crisp foreground text above glow/cardboard/depth layers.
            let foreground_layer = if depth_style.is_some() { 4 } else { 2 };
            ass.push_str(&format!(
                "Dialogue: {},{},{},Default,,0,0,0,,{{{pos}{wt}{size}{colour}}}{txt}\r\n",
                foreground_layer,
                start_ass,
                end_ass,
                pos = caption_position_tag,
                wt = weight_tag,
                size = size_tag,
                colour = colour_tag,
                txt = foreground_text
            ));
            if let Some(depth) = depth_style {
                ass.push_str(&format!(
                    "Dialogue: 5,{},{},MaterialDetail,,0,0,0,,{{{pos}{size}}}{txt}\r\n",
                    start_ass,
                    end_ass,
                    pos = caption_position_tag,
                    size = size_tag,
                    txt = sub_text,
                ));
                if !depth.accent_font.is_empty() {
                    ass.push_str(&format!(
                        "Dialogue: 6,{},{},MaterialAccent,,0,0,0,,{{{pos}{size}}}{txt}\r\n",
                        start_ass,
                        end_ass,
                        pos = caption_position_tag,
                        size = size_tag,
                        txt = sub_text,
                    ));
                }
            }
        }

        let ass_temp = std::env::temp_dir().join(format!("clip_{}.ass", clip.id));
        if let Err(e) = std::fs::write(&ass_temp, &ass) {
            log::warn!("Failed to write temp ASS for subtitles filter: {}", e);
            return None;
        }
        let ass_path = ffmpeg_filter_path(&ass_temp);

        // Use the ass filter (not subtitles) to avoid any SRT re-parsing
        let mut filter = format!("ass='{}'", ass_path);
        if let Some(font_dir) = bundled_font_path.as_ref().and_then(|path| path.parent()) {
            filter.push_str(&format!(":fontsdir='{}'", ffmpeg_filter_path(font_dir)));
        }
        Some(filter)
    } else {
        let display_text = if style.uppercase {
            text.to_uppercase()
        } else {
            text.clone()
        };
        let font_size = fitted_caption_font_size(
            &style,
            clip.caption_font_scale,
            &display_text,
            target_width,
            target_height,
            false,
        );
        let esc = display_text
            .replace('\\', "\\\\")
            .replace('\'', "'\\''")
            .replace(':', "\\:")
            .replace('%', "%%")
            .replace('[', "\\[")
            .replace(']', "\\]")
            .replace(';', "\\;");
        let anchor_ratio = caption_anchor_y as f64 / target_height.max(1) as f64;
        let ypos = match clip.captions_position.as_str() {
            "top" => format!("h*{anchor_ratio:.6}"),
            "center" => format!("h*{anchor_ratio:.6}-text_h/2"),
            _ => format!("h*{anchor_ratio:.6}-text_h"),
        };

        let mut filter = format!(
            "drawtext=text='{text}':fontsize={fs}:fontcolor={fc}:borderw={bw}:bordercolor=black:x=(w-text_w)/2:y={y}",
            text = esc, fs = font_size, fc = style.dt_fontcolor, bw = style.dt_borderw, y = ypos,
        );
        if let Some(font_path) = bundled_font_path.as_ref() {
            filter.push_str(&format!(":fontfile='{}'", ffmpeg_filter_path(font_path)));
        }
        if !style.dt_boxcolor.is_empty() {
            let border_width = if is_cardboard { 28 } else { 8 };
            filter.push_str(&format!(
                ":box=1:boxcolor={}:boxborderw={border_width}",
                style.dt_boxcolor,
            ));
        }
        if style.dt_shadow > 0 && !style.dt_shadowcolor.is_empty() {
            filter.push_str(&format!(
                ":shadowx={0}:shadowy={0}:shadowcolor={1}",
                style.dt_shadow, style.dt_shadowcolor,
            ));
        }
        Some(filter)
    }
}

// Legacy build_filter_graph — kept temporarily for reference during migration.
// TODO: Remove once vertical_crop integration is verified in production.
#[allow(dead_code)]
fn build_filter_graph(clip: &db::ClipRow) -> (String, bool) {
    let (tw, th) = match clip.aspect_ratio.as_str() {
        "9:16" => (1080, 1920),
        "1:1" => (1080, 1080),
        _ => (1920, 1080),
    };

    let captions_active =
        clip.captions_enabled == 1 && clip.captions_text.as_ref().map_or(false, |t| !t.is_empty());

    let caption_filter = if captions_active {
        let text = clip.captions_text.as_ref().unwrap();

        // Check if captions_text looks like SRT format (has timestamps like "00:00:01,000 -->")
        let is_srt = text.contains("-->") && text.lines().count() > 2;

        if is_srt {
            // Write SRT to a temp file for ffmpeg subtitles filter
            let srt_temp = std::env::temp_dir().join(format!("clip_{}.srt", clip.id));
            std::fs::write(&srt_temp, text).ok();
            let srt_path = srt_temp
                .to_string_lossy()
                .to_string()
                .replace('\\', "/") // ffmpeg needs forward slashes
                .replace(':', "\\:"); // Escape colons for filter syntax

            let ypos = match clip.captions_position.as_str() {
                "top" => 30,
                "center" => th / 2 - 30,
                _ => th - 120,
            };

            Some(format!(
                "subtitles='{}':\
                 force_style='FontSize=24,FontName=Arial,PrimaryColour=&HFFFFFF,\
                 OutlineColour=&H000000,Outline=2,Shadow=1,\
                 Alignment=2,MarginV={}'",
                srt_path, ypos
            ))
        } else {
            // Static drawtext for manually entered captions
            // Escape ffmpeg special characters to prevent text expansion injection
            let esc = text
                .replace('\\', "\\\\")
                .replace('\'', "'\\''")
                .replace(':', "\\:")
                .replace('%', "%%")
                .replace('[', "\\[")
                .replace(']', "\\]")
                .replace(';', "\\;");
            let ypos = match clip.captions_position.as_str() {
                "top" => "h*0.08",
                "center" => "(h-text_h)/2",
                _ => "h*0.85",
            };
            Some(format!(
                "drawtext=text='{}':fontsize=48:fontcolor=white:borderw=3:bordercolor=black:x=(w-text_w)/2:y={}",
                esc, ypos
            ))
        }
    } else {
        None
    };

    match clip.facecam_layout.as_str() {
        "split" => {
            let th_top = (th as f64 * 0.6) as i32;
            let th_bot = th - th_top;
            let mut f = format!(
                "[0:v]split[a][b];\
                 [a]crop=iw:ih*0.6:0:0,scale={}:{}[top];\
                 [b]crop=iw*0.4:ih*0.4:0:ih*0.6,scale={}:{}[bottom];\
                 [top][bottom]vstack",
                tw, th_top, tw, th_bot
            );
            if let Some(cf) = caption_filter {
                f.push_str(&format!("[stacked];[stacked]{}[out]", cf));
            } else {
                f.push_str("[out]");
            }
            (f, true)
        }
        "pip" => {
            let ps = (tw as f64 * 0.3) as i32;
            let mut f = format!(
                "[0:v]split[bg][ps];\
                 [bg]scale={}:{}:force_original_aspect_ratio=increase,crop={}:{}[main];\
                 [ps]crop=iw*0.3:ih*0.3:0:ih*0.7,scale={}:{}[pip];\
                 [main][pip]overlay=W-w-20:H-h-20",
                tw, th, tw, th, ps, ps
            );
            if let Some(cf) = caption_filter {
                f.push_str(&format!("[overlaid];[overlaid]{}[out]", cf));
            } else {
                f.push_str("[out]");
            }
            (f, true)
        }
        _ => {
            // Use the vertical_crop module for quality-preserving
            // crop-first-then-scale logic.  Handles landscape, vertical,
            // and undersized inputs correctly.
            let target = vertical_crop::OutputSize {
                width: tw as u32,
                height: th as u32,
            };
            let base = vertical_crop::vertical_filter(target, vertical_crop::CropAnchor::Center);
            let mut parts = vec![base];
            if let Some(cf) = caption_filter {
                parts.push(cf);
            }
            (parts.join(","), false)
        }
    }
}

#[allow(dead_code)]
fn render_clip_with_ffmpeg(
    ffmpeg: &std::path::Path,
    vod_path: &str,
    clip: &db::ClipRow,
    output_path: &std::path::Path,
) -> Result<(), AppError> {
    let (filter, is_complex) = build_filter_graph(clip);

    let mut cmd = std::process::Command::new(ffmpeg);
    cmd.arg("-ss")
        .arg(format!("{}", clip.start_seconds))
        .arg("-to")
        .arg(format!("{}", clip.end_seconds))
        .arg("-i")
        .arg(vod_path);

    if is_complex {
        cmd.arg("-filter_complex")
            .arg(&filter)
            .arg("-map")
            .arg("[out]")
            .arg("-map")
            .arg("0:a?");
    } else {
        cmd.arg("-vf").arg(&filter);
    }

    cmd.arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("medium")
        .arg("-crf")
        .arg("23")
        .arg("-c:a")
        .arg("aac")
        .arg("-b:a")
        .arg("128k")
        .arg("-movflags")
        .arg("+faststart")
        .arg("-y")
        .arg(output_path.to_string_lossy().as_ref())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let status = cmd
        .status()
        .map_err(|e| AppError::Ffmpeg(format!("Render launch failed: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(AppError::Ffmpeg(
            "Clip rendering exited with an error".into(),
        ))
    }
}
