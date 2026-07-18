//! Clip export and rendering commands.

use std::process::Stdio;
use std::sync::OnceLock;
use tauri::{AppHandle, State};
use crate::db;
use crate::DbConn;
use crate::error::AppError;
use crate::job_queue::JobQueue;
use crate::report_error;
use crate::vertical_crop;
use crate::commands::vod::{
    find_ffmpeg, generate_srt_for_clip, generate_thumbnail,
    run_clip_transcription_native,
};

/// Generate captions for a clip by transcribing its audio segment.
#[tauri::command]
pub async fn generate_clip_captions(
    clip_id: String,
    db: State<'_, DbConn>,
) -> Result<String, String> {
    let (clip, vod) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let clip = db::get_clip_by_id(&conn, &clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("Clip not found")?;
        let vod = db::get_vod_by_id(&conn, &clip.vod_id)
            .map_err(|e| format!("DB error: {}", e))?;
        (clip, vod)
    };

    let standalone_source = clip
        .source_media_path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            clip.community_clip_mp4_path
                .as_deref()
                .filter(|path| !path.trim().is_empty())
                .map(std::path::PathBuf::from)
        });
    let media_path = if let Some(path) = standalone_source.as_ref() {
        if !path.is_file() {
            return Err(format!("The source video is missing: {}", path.display()));
        }
        path.to_string_lossy().to_string()
    } else {
        vod.as_ref()
            .and_then(|vod| vod.local_path.clone())
            .ok_or("VOD not downloaded")?
    };
    let transcript_clip_start = if clip.community_clip_mp4_path.is_some() {
        0.0
    } else {
        clip.start_seconds
    };
    let transcript_clip_end = if clip.community_clip_mp4_path.is_some() {
        standalone_source
            .as_deref()
            .and_then(probe_media_duration)
            .unwrap_or(clip.end_seconds.max(0.1))
    } else {
        clip.end_seconds
    };

    // Subtitle regeneration always uses the short clip-specific DTW/VAD path.
    // Whole-VOD transcripts are optimized for detection throughput and their
    // token timestamps are not precise enough for word-by-word captions.
    log::info!(
        "[Captions] Generating speech-aligned timing for clip {} ({:.2}-{:.2}s)",
        clip.id,
        transcript_clip_start,
        transcript_clip_end
    );
    let media_path_for_task = media_path.clone();
    let clip_start = transcript_clip_start;
    let clip_end = transcript_clip_end;
    let transcript = tokio::task::spawn_blocking(move || {
        run_clip_transcription_native(&media_path_for_task, clip_start, clip_end)
    })
    .await
    .map_err(|error| format!("Caption transcription task failed: {error}"))?
    .map_err(|error| error.to_string())?;
    let transcript_start = 0.0;
    let transcript_end = (transcript_clip_end - transcript_clip_start).max(0.0);

    // Generate SRT for this clip's time range
    let captions_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clipviral")
        .join("captions");
    std::fs::create_dir_all(&captions_dir).ok();
    let srt_path = captions_dir.join(format!("{}.srt", clip.id));

    generate_srt_for_clip(&transcript, transcript_start, transcript_end, &srt_path)?;

    let srt_text = std::fs::read_to_string(&srt_path)
        .map_err(|e| format!("Read SRT: {}", e))?;

    if srt_text.trim().is_empty() {
        return Err("No speech detected in this clip's time range".to_string());
    }

    // Save to clip
    {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::save_setting(&conn, &format!("clip_{}_captions", clip_id), &srt_text).ok();
        // Keep the SRT's source-media origin with the text. Future trim edits
        // can then shift cues into the exported clip without regenerating them.
        conn.execute(
            "UPDATE clips SET captions_text = ?1, captions_source_start = ?2 WHERE id = ?3",
            rusqlite::params![srt_text, transcript_clip_start, clip_id],
        ).map_err(|e| format!("DB error: {}", e))?;
    }

    Ok(srt_text)
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
    app: AppHandle,
    db: State<'_, DbConn>,
    queue: State<'_, JobQueue>,
) -> Result<(), String> {
    let ffmpeg = find_ffmpeg().map_err(|e| report_error(&app, e))?;

    let (clip, vod, media_path, allow_override) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let clip = db::get_clip_by_id(&conn, &clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("Clip not found")?;
        let vod = db::get_vod_by_id(&conn, &clip.vod_id)
            .map_err(|e| format!("DB error: {}", e))?;
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

    let job_id = format!("export-{}", clip_id);
    let clip_id_bg = clip_id.clone();

    queue.add_job(job_id, move |handle| async move {
        // Mark rendering in DB inside the job, so status is only set once
        // the job is actually running (not stuck if app crashes before queuing).
        {
            let db_path = db::db_path().map_err(|e| format!("DB path error: {e}"))?;
            let conn = rusqlite::Connection::open(db_path)
                .map_err(|e| format!("DB error: {e}"))?;
            db::update_clip_render_status(&conn, &clip_id_bg, "rendering", None)
                .map_err(|e| format!("DB error: {}", e))?;
        }
        // ── Preparing ──
        handle.set_progress(5);

        let output_dir = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("clipviral")
            .join("exports");
        std::fs::create_dir_all(&output_dir)
            .map_err(|e| format!("Failed to create export directory: {e}"))?;
        let output_path = output_dir.join(format!("{}.mp4", clip_id_bg));

        // ── Building export request ──
        handle.set_progress(5);
        let request = clip_to_export_request(
            &clip,
            vod.as_ref(),
            &media_path,
            &output_path,
            allow_override,
        );

        // ── Running ffmpeg with real progress ──
        let output_ref = output_path.clone();
        let clip_id_ref = clip_id_bg.clone();
        let handle_ref = handle.clone();

        let result = tokio::task::spawn_blocking(move || {
            vertical_crop::run_export(&ffmpeg, &request, |pct| {
                handle_ref.set_progress(pct);
            })
        })
        .await
        .map_err(|e| format!("Export task panicked: {e}"))?;

        // ── Update DB with result ──
        let db_path = db::db_path().map_err(|e| format!("DB path error: {e}"))?;
        let conn = rusqlite::Connection::open(db_path)
            .map_err(|e| format!("DB error: {e}"))?;

        if result.success {
            db::update_clip_render_status(
                &conn, &clip_id_ref, "completed",
                Some(&output_ref.to_string_lossy()),
            ).ok();
            let metadata = serde_json::json!({ "aspectRatio": &clip.aspect_ratio }).to_string();
            let dedupe_key = format!(
                "export:{}:{:.1}:{:.1}:{}",
                clip_id_ref, clip.start_seconds, clip.end_seconds, clip.aspect_ratio
            );
            let _ = db::record_clip_behavior(
                &conn,
                &clip_id_ref,
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
            handle.set_progress(100);
            Ok(())
        } else {
            db::update_clip_render_status(&conn, &clip_id_ref, "failed", None).ok();
            let msg = if result.stderr_tail.is_empty() {
                "FFmpeg exited with an error".to_string()
            } else {
                format!("FFmpeg error: {}", result.stderr_tail)
            };
            Err(msg)
        }
    });

    Ok(())
}

/// Export a clip synchronously by id. Returns the rendered file path on success.
/// Used by both the `export_clip` Tauri command (via its JobQueue wrapper) and
/// the scheduler's auto-export path when a pending upload lacks an output_path.
///
/// Opens its own `rusqlite::Connection` via `db::db_path()` so callers don't
/// need to juggle the DbConn State mutex. Safe to call from any async context;
/// the actual ffmpeg work runs inside `tokio::task::spawn_blocking`.
pub(crate) async fn render_clip_by_id(clip_id: &str) -> Result<std::path::PathBuf, String> {
    let ffmpeg = find_ffmpeg().map_err(|e| e.to_string())?;

    // Load clip + vod path (sync)
    let (clip, vod, media_path, allow_override) = {
        let db_path = db::db_path().map_err(|e| format!("DB path: {}", e))?;
        let conn = rusqlite::Connection::open(&db_path)
            .map_err(|e| format!("DB open: {}", e))?;
        let clip = db::get_clip_by_id(&conn, clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or_else(|| "Clip not found".to_string())?;
        let vod = db::get_vod_by_id(&conn, &clip.vod_id)
            .map_err(|e| format!("DB error: {}", e))?;
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

    let output_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clipviral")
        .join("exports");
    std::fs::create_dir_all(&output_dir)
        .map_err(|e| format!("Create export dir: {}", e))?;
    let output_path = output_dir.join(format!("{}.mp4", clip_id));

    // Mark rendering in DB
    {
        let db_path = db::db_path().map_err(|e| format!("DB path: {}", e))?;
        let conn = rusqlite::Connection::open(&db_path)
            .map_err(|e| format!("DB open: {}", e))?;
        db::update_clip_render_status(&conn, clip_id, "rendering", None)
            .map_err(|e| format!("DB error: {}", e))?;
    }

    let request = clip_to_export_request(
        &clip,
        vod.as_ref(),
        &media_path,
        &output_path,
        allow_override,
    );
    let output_ref = output_path.clone();
    let clip_id_owned = clip_id.to_string();

    let result = tokio::task::spawn_blocking(move || {
        vertical_crop::run_export(&ffmpeg, &request, |_pct| {
            // no progress callback — scheduler's simpler.
        })
    })
    .await
    .map_err(|e| format!("Export task panicked: {}", e))?;

    // Persist result
    let db_path = db::db_path().map_err(|e| format!("DB path: {}", e))?;
    let conn = rusqlite::Connection::open(&db_path)
        .map_err(|e| format!("DB open: {}", e))?;

    if result.success {
        db::update_clip_render_status(&conn, &clip_id_owned, "completed",
            Some(&output_ref.to_string_lossy())).ok();
        let metadata = serde_json::json!({ "aspectRatio": &clip.aspect_ratio }).to_string();
        let dedupe_key = format!(
            "export:{}:{:.1}:{:.1}:{}",
            clip_id_owned, clip.start_seconds, clip.end_seconds, clip.aspect_ratio
        );
        let _ = db::record_clip_behavior(
            &conn,
            &clip_id_owned,
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
        Ok(output_ref)
    } else {
        db::update_clip_render_status(&conn, &clip_id_owned, "failed", None).ok();
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
    cmd.arg("-v").arg("error")
        .arg("-show_entries").arg("format=duration")
        .arg("-of").arg("default=noprint_wrappers=1:nokey=1")
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
    if dur.is_finite() && dur > 0.0 { Some(dur) } else { None }
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
            clip.id, src.display()
        );
    } else if clip.community_clip_mp4_path.as_deref().map_or(false, |p| !p.is_empty()) {
        log::warn!(
            "[export] clip {} has community_clip_mp4_path but file/duration unavailable — falling back to VOD cut",
            clip.id
        );
    }

    // Resolve platform from aspect ratio (future: store preset id in DB)
    let platform = vertical_crop::Platform::from_aspect_ratio(&clip.aspect_ratio);
    let target = platform.resolution();

    // Resolve layout and its persisted editor geometry from DB state.
    let layout_settings = vertical_crop::EditorLayoutSettings::from_json(
        clip.facecam_settings.as_deref(),
    );
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
    let context_background_path = if layout_supports_branding
        && clip.context_background_mode == "branding"
    {
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
            if clip.source_media_path.as_deref().is_some_and(|path| !path.trim().is_empty())
                || clip.community_clip_mp4_path.as_deref().is_some_and(|path| !path.trim().is_empty())
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
        effective_region,
        fit_mode,
        context_background_mode,
        context_background_path,
        context_blur_strength: clip.context_blur_strength,
        context_video_y: clip.context_video_y,
    }
}

const RUBIK_DIRT_FONT_BYTES: &[u8] =
    include_bytes!("../../../public/fonts/RubikDirt-Regular.ttf");
const COINY_FONT_BYTES: &[u8] = include_bytes!("../../../public/fonts/Coiny-Regular.ttf");
const NOSIFER_FONT_BYTES: &[u8] = include_bytes!("../../../public/fonts/Nosifer-Regular.ttf");
const BANGERS_FONT_BYTES: &[u8] = include_bytes!("../../../public/fonts/Bangers-Regular.ttf");
static RUBIK_DIRT_FONT_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
static COINY_FONT_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
static NOSIFER_FONT_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
static BANGERS_FONT_PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();

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
            let already_current = std::fs::metadata(&font_path)
                .map(|metadata| metadata.len() == bytes.len() as u64)
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
        "boxed" => stage_bundled_caption_font(
            &COINY_FONT_PATH,
            "Coiny-Regular.ttf",
            COINY_FONT_BYTES,
        ),
        "minimal" => stage_bundled_caption_font(
            &NOSIFER_FONT_PATH,
            "Nosifer-Regular.ttf",
            NOSIFER_FONT_BYTES,
        ),
        "comic-pop" => stage_bundled_caption_font(
            &BANGERS_FONT_PATH,
            "Bangers-Regular.ttf",
            BANGERS_FONT_BYTES,
        ),
        _ => None,
    }
}

fn ffmpeg_filter_path(path: &std::path::Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .replace(':', "\\:")
}

fn cardboard_uses_black_text(text: &str) -> bool {
    const EMPHASIS_WORDS: &[&str] = &[
        "no", "yes", "wait", "what", "run", "go", "help", "stop", "please", "why",
        "how", "kill", "dead", "die", "escape", "save", "clutch", "bruh", "bro", "dude",
    ];
    const EMPHASIS_PHRASES: &[&str] = &[
        "oh my god", "no way", "watch out", "let's go", "lets go", "we did it", "i'm dead",
        "im dead", "wait what",
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

    EMPHASIS_PHRASES.iter().any(|phrase| normalized.contains(phrase))
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
    let board_height = if target_height > target_width { 128 } else { 112 };
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
mod caption_style_tests {
    use super::{
        build_caption_filter, bundled_caption_font, cardboard_ass_drawings,
        cardboard_uses_black_text, fitted_caption_font_size, get_sub_style,
        BANGERS_FONT_BYTES, COINY_FONT_BYTES, NOSIFER_FONT_BYTES,
        RUBIK_DIRT_FONT_BYTES,
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
            caption_y_offset: 0.0,
            captions_source_start: Some(0.0),
            facecam_layout: "none".into(),
            facecam_settings: None,
            context_background_path: None,
            context_background_mode: "blur".into(),
            context_blur_strength: 0.25,
            context_video_y: 0.5,
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
        let captions = "1\n00:00:00,000 --> 00:00:00,500\nhello\n\n2\n00:00:00,600 --> 00:00:01,000\nworld.\n";
        let clip = clip_with_style("bold-white", captions);
        let filter = build_caption_filter(&clip, 1080, 1920, 0.0, 2.0)
            .expect("cardboard filter");
        assert!(filter.starts_with("ass='"));

        let ass_path = std::env::temp_dir().join(format!("clip_{}.ass", clip.id));
        let ass = std::fs::read_to_string(&ass_path).expect("generated ASS file");
        assert_eq!(ass.matches("Cardboard,,").count(), 2);
        assert_eq!(ass.matches("CardboardTexture,,").count(), 2);
        assert!(ass.contains("{\\an2\\pos(540,1862)\\b900\\fs52\\1c&H0C1015&}HELLO"));
        assert!(ass.contains("{\\an2\\pos(540,1862)\\b900\\fs52}WORLD."));
        let _ = std::fs::remove_file(ass_path);
    }

    #[test]
    fn highlight_filter_stages_and_loads_the_bundled_distressed_font() {
        let captions = "1\n00:00:00,000 --> 00:00:01,000\nclutch\n";
        let clip = clip_with_style("fire", captions);
        let filter = build_caption_filter(&clip, 1080, 1920, 0.0, 2.0)
            .expect("highlight filter");
        assert!(filter.contains(":fontsdir='"));

        let font_path = bundled_caption_font("fire").expect("bundled highlight font");
        assert!(font_path.is_file());
        assert_eq!(
            std::fs::metadata(font_path).expect("font metadata").len(),
            RUBIK_DIRT_FONT_BYTES.len() as u64,
        );

        let ass_path = std::env::temp_dir().join(format!("clip_{}.ass", clip.id));
        let ass = std::fs::read_to_string(&ass_path).expect("generated ASS file");
        assert!(ass.contains("Style: Default,Rubik Dirt,60"));
        assert!(ass.contains("{\\an2\\pos(540,1862)\\b400\\fs60}CLUTCH"));
        let _ = std::fs::remove_file(ass_path);
    }

    #[test]
    fn trimmed_imported_captions_shift_to_clip_time_and_use_editor_anchor() {
        let captions = "1\n00:00:05,000 --> 00:00:06,000\nclutch\n\n2\n00:00:20,000 --> 00:00:21,000\nlate\n";
        let mut clip = clip_with_style("clean", captions);
        clip.captions_position = "center".into();
        clip.caption_y_offset = 10.0;

        build_caption_filter(&clip, 1080, 1920, 4.5, 10.0)
            .expect("shifted caption filter");
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
        build_caption_filter(&clip, 1080, 1920, 0.0, 2.0)
            .expect("frosted filter");
        let ass_path = std::env::temp_dir().join(format!("clip_{}.ass", clip.id));
        let ass = std::fs::read_to_string(&ass_path).expect("generated ASS file");
        assert!(ass.contains("Style: Default,Coiny,58,&H00FFFFFF"));
        let _ = std::fs::remove_file(ass_path);
    }

    #[test]
    fn caption_font_scale_is_bounded_and_long_words_fit_the_vertical_safe_width() {
        let style = get_sub_style("comic-pop");
        let maximum = fitted_caption_font_size(&style, 99.0, "CLUTCH", 1080, 1920, true);
        let minimum = fitted_caption_font_size(&style, 0.1, "CLUTCH", 1080, 1920, true);
        let long_word = "EXTRAORDINARILYLONGREACTIONWORD";
        let long = fitted_caption_font_size(
            &style,
            1.25,
            long_word,
            1080,
            1920,
            true,
        );

        assert_eq!(maximum, 80);
        assert_eq!(minimum, 48);
        assert!(long < maximum);
        assert!(
            long as f64 * long_word.chars().count() as f64 * style.character_width_factor
                <= 1080.0 * style.safe_width_ratio
        );
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

fn get_sub_style(id: &str) -> SubStyle {
    match id {
        // font_size values match editTypes.ts fontSize (px at 1080px-wide reference)
        // font_weight values match editTypes.ts fontWeight
        "bold-white" => SubStyle {
            // Cardboard: dark red type over a timed, textured ASS sign layer.
            font_name: "Arial Black", font_size: 52, font_weight: 900,
            // #7A2118 -> ASS BGR &H18217A
            primary_colour: "&H18217A", outline_colour: "&H000000",
            back_colour: "&H60000000", outline: 0, shadow: 1, border_style: 1,
            spacing: 0.5, glow_blur: 0, glow_colour: "", uppercase: true,
            dt_fontcolor: "#7A2118", dt_borderw: 0, dt_boxcolor: "#C99358@0.96",
            character_width_factor: 0.72, safe_width_ratio: 0.68,
            dt_shadowcolor: "#3F2310@0.75", dt_shadow: 2,
        },
        "boxed" => SubStyle {
            // Frosted: pink candy lettering, white edge, and purple lift.
            font_name: "Coiny", font_size: 58, font_weight: 400,
            primary_colour: "&HFFFFFF", outline_colour: "&HFFFFFF",
            back_colour: "&H006F2055", outline: 3, shadow: 4, border_style: 1,
            spacing: 0.5, glow_blur: 5, glow_colour: "&HA0D85BF0", uppercase: true,
            dt_fontcolor: "white", dt_borderw: 0, dt_boxcolor: "",
            character_width_factor: 0.72, safe_width_ratio: 0.84,
            dt_shadowcolor: "#6D28D9", dt_shadow: 4,
        },
        "neon" => SubStyle {
            // Segoe UI is the frontend font on Windows; fall back to Arial
            font_name: "Segoe UI", font_size: 54, font_weight: 800,
            // #00FF88 → R=00 G=FF B=88 → ASS &HBBGGRR = &H88FF00
            primary_colour: "&H88FF00", outline_colour: "&H000000",
            back_colour: "&H00000000",
            // CSS uses 4 stacked black shadows → thick outline.  Outline=4 matches.
            outline: 4, shadow: 0, border_style: 1,
            spacing: 1.2,
            // Glow layer: bright green, gaussian-blurred behind text
            // CSS: '0 0 8px #00ff8880' (#80 hex ≈ 50% opacity)
            // ASS alpha: 00=opaque FF=transparent → &H80 = 50% transparent = 50% opaque
            glow_blur: 8, glow_colour: "&H8088FF00",
            uppercase: true,
            dt_fontcolor: "#00FF88", dt_borderw: 3, dt_boxcolor: "",
            character_width_factor: 0.66, safe_width_ratio: 0.84,
            dt_shadowcolor: "black@0.85", dt_shadow: 2,
        },
        "minimal" => SubStyle {
            // Drip: sharp red display face with a deep blood-red edge.
            font_name: "Nosifer", font_size: 50, font_weight: 400,
            primary_colour: "&H1F35FF", outline_colour: "&H00003B",
            back_colour: "&H00000000", outline: 2, shadow: 3, border_style: 1,
            spacing: 0.4, glow_blur: 0, glow_colour: "", uppercase: true,
            dt_fontcolor: "#FF351F", dt_borderw: 2, dt_boxcolor: "",
            character_width_factor: 0.80, safe_width_ratio: 0.84,
            dt_shadowcolor: "black@0.9", dt_shadow: 3,
        },
        "fire" => SubStyle {
            font_name: "Rubik Dirt", font_size: 60, font_weight: 400,
            // #FFE45E -> R=FF G=E4 B=5E -> ASS &HBBGGRR = &H5EE4FF
            primary_colour: "&H5EE4FF", outline_colour: "&H000000",
            back_colour: "&H00000000", outline: 3, shadow: 1, border_style: 1,
            spacing: 0.5, glow_blur: 0, glow_colour: "", uppercase: true,
            dt_fontcolor: "#FFE45E", dt_borderw: 3, dt_boxcolor: "",
            character_width_factor: 0.72, safe_width_ratio: 0.84,
            dt_shadowcolor: "black@0.9", dt_shadow: 3,
        },
        "comic-pop" => SubStyle {
            // Comic Pop: cyan face with magenta/purple offset comic-book shadow.
            font_name: "Bangers", font_size: 64, font_weight: 400,
            primary_colour: "&HE6E867", outline_colour: "&H6F2055",
            back_colour: "&H00D85BF0", outline: 3, shadow: 4, border_style: 1,
            spacing: 0.8, glow_blur: 0, glow_colour: "", uppercase: true,
            dt_fontcolor: "#67E8E6", dt_borderw: 3, dt_boxcolor: "",
            character_width_factor: 0.68, safe_width_ratio: 0.84,
            dt_shadowcolor: "#F05BD8", dt_shadow: 4,
        },
        // "clean" and any unknown style
        _ => SubStyle {
            font_name: "Arial", font_size: 52, font_weight: 700,
            primary_colour: "&HFFFFFF", outline_colour: "&H000000",
            back_colour: "&H00000000", outline: 2, shadow: 0, border_style: 1,
            spacing: 0.4, glow_blur: 0, glow_colour: "", uppercase: false,
            dt_fontcolor: "white", dt_borderw: 3, dt_boxcolor: "",
            character_width_factor: 0.66, safe_width_ratio: 0.84,
            dt_shadowcolor: "black@0.85", dt_shadow: 2,
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
    let hard_max_ratio = if target_height > target_width { 0.085 } else { 0.065 };
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
    if parts.next().is_some() || !hours.is_finite() || !minutes.is_finite() || !seconds.is_finite() {
        return None;
    }
    Some((hours * 3600.0 + minutes * 60.0 + seconds).max(0.0))
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
    let bundled_font_path = bundled_caption_font(&clip.caption_style);
    let is_srt = text.contains("-->") && text.lines().count() > 2;

    // Match the editor's anchor semantics exactly: top grows downward, center
    // grows around the anchor, and bottom grows upward.
    let (caption_base_y, caption_alignment) = match clip.captions_position.as_str() {
        "top" => (8.0, 8),
        "center" => (50.0, 5),
        _ => (97.0, 2),
    };
    let caption_y_percent = (caption_base_y
        + db::normalize_caption_y_offset(clip.caption_y_offset))
        .clamp(3.0, 97.0);
    let caption_anchor_y = ((target_height as f64 * caption_y_percent / 100.0).round() as i32)
        .clamp(0, target_height);
    let caption_position_tag = format!(
        "\\an{}\\pos({},{})",
        caption_alignment,
        target_width / 2,
        caption_anchor_y,
    );
    let margin_h = ((target_width as f64 * (1.0 - style.safe_width_ratio) / 2.0).round()
        as i32)
        .max(10);
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

        ass.push_str("\r\n\
[Events]\r\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\n");

        // Parse SRT cues and append as Dialogue lines
        // SRT format: index\n HH:MM:SS,mmm --> HH:MM:SS,mmm \n text \n\n
        let blocks: Vec<&str> = text.split("\n\n").filter(|b| !b.trim().is_empty()).collect();
        let mut cardboard_sentence_start = true;
        for block in &blocks {
            let lines: Vec<&str> = block.lines().collect();
            // Find the timing line (contains "-->")
            let timing_idx = lines.iter().position(|l| l.contains("-->"));
            if let Some(ti) = timing_idx {
                let timing = lines[ti];
                let parts: Vec<&str> = timing.split("-->").collect();
                if parts.len() == 2 {
                    let Some(raw_start) = parse_srt_time_seconds(parts[0]) else {
                        continue;
                    };
                    let Some(raw_end) = parse_srt_time_seconds(parts[1]) else {
                        continue;
                    };
                    let shifted_start = raw_start - caption_time_offset;
                    let shifted_end = raw_end - caption_time_offset;
                    if shifted_end <= 0.0
                        || (clip_duration > 0.0 && shifted_start >= clip_duration)
                    {
                        continue;
                    }
                    let clipped_start = shifted_start.max(0.0);
                    let clipped_end = if clip_duration > 0.0 {
                        shifted_end.min(clip_duration)
                    } else {
                        shifted_end
                    };
                    if clipped_end <= clipped_start {
                        continue;
                    }
                    let start_ass = seconds_to_ass_time(clipped_start);
                    let end_ass = seconds_to_ass_time(clipped_end);
                    // Remaining lines after timing are the subtitle text
                    let sub_text: String = lines[ti + 1..].iter()
                        .map(|l| l.trim())
                        .filter(|l| !l.is_empty())
                        .collect::<Vec<_>>()
                        .join("\\N"); // ASS line break
                    let sub_text = if style.uppercase { sub_text.to_uppercase() } else { sub_text };

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
                    let cardboard_black = is_cardboard
                        && (cardboard_sentence_start || cardboard_uses_black_text(&sub_text));
                    let colour_tag = if cardboard_black {
                        // #15100C -> ASS BGR &H0C1015
                        "\\1c&H0C1015&"
                    } else {
                        ""
                    };
                    if is_cardboard {
                        let sentence_tail = sub_text.trim_end_matches(|character: char| {
                            character.is_whitespace()
                                || matches!(character, '"' | '\'' | ')' | ']')
                        });
                        cardboard_sentence_start = matches!(
                            sentence_tail.chars().last(),
                            Some('.' | '!' | '?')
                        );
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

                    // If glow style exists, emit a blurred glow layer on Layer 0
                    if has_glow {
                        ass.push_str(&format!(
                            "Dialogue: 0,{},{},Glow,,0,0,0,,{{{pos}{wt}{size}\\blur{blur}}}{txt}\r\n",
                            start_ass, end_ass,
                            pos = caption_position_tag, wt = weight_tag, size = size_tag, blur = style.glow_blur, txt = sub_text
                        ));
                    }
                    // Crisp foreground text on Layer 2 (above glow/cardboard layers).
                    ass.push_str(&format!(
                        "Dialogue: 2,{},{},Default,,0,0,0,,{{{pos}{wt}{size}{colour}}}{txt}\r\n",
                        start_ass, end_ass, pos = caption_position_tag, wt = weight_tag, size = size_tag, colour = colour_tag, txt = sub_text
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
        let display_text = if style.uppercase { text.to_uppercase() } else { text.clone() };
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

    let captions_active = clip.captions_enabled == 1
        && clip.captions_text.as_ref().map_or(false, |t| !t.is_empty());

    let caption_filter = if captions_active {
        let text = clip.captions_text.as_ref().unwrap();

        // Check if captions_text looks like SRT format (has timestamps like "00:00:01,000 -->")
        let is_srt = text.contains("-->") && text.lines().count() > 2;

        if is_srt {
            // Write SRT to a temp file for ffmpeg subtitles filter
            let srt_temp = std::env::temp_dir().join(format!("clip_{}.srt", clip.id));
            std::fs::write(&srt_temp, text).ok();
            let srt_path = srt_temp.to_string_lossy().to_string()
                .replace('\\', "/")  // ffmpeg needs forward slashes
                .replace(':', "\\:");  // Escape colons for filter syntax

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
            let target = vertical_crop::OutputSize { width: tw as u32, height: th as u32 };
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
    cmd.arg("-ss").arg(format!("{}", clip.start_seconds))
       .arg("-to").arg(format!("{}", clip.end_seconds))
       .arg("-i").arg(vod_path);

    if is_complex {
        cmd.arg("-filter_complex").arg(&filter)
           .arg("-map").arg("[out]")
           .arg("-map").arg("0:a?");
    } else {
        cmd.arg("-vf").arg(&filter);
    }

    cmd.arg("-c:v").arg("libx264")
       .arg("-preset").arg("medium")
       .arg("-crf").arg("23")
       .arg("-c:a").arg("aac")
       .arg("-b:a").arg("128k")
       .arg("-movflags").arg("+faststart")
       .arg("-y")
       .arg(output_path.to_string_lossy().as_ref())
       .stdout(Stdio::null())
       .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let status = cmd.status().map_err(|e| AppError::Ffmpeg(format!("Render launch failed: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(AppError::Ffmpeg("Clip rendering exited with an error".into()))
    }
}
