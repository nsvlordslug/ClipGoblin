//! Platform publishing adapters.
//!
//! Shared trait + dispatcher for YouTube, TikTok, Instagram.
//! YouTube is fully implemented; TikTok/Instagram are stubs.

pub mod instagram;
pub mod tiktok;
pub mod youtube;

use crate::error::AppError;
use rusqlite::Connection;
use std::path::Path;
use std::process::{Command, Stdio};

// ═══════════════════════════════════════════════════════════════════
//  Shared types (serialized to frontend)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConnectedAccount {
    pub platform: String,
    pub account_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_handle: Option<String>,
    pub account_id: String,
    pub connected_at: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TikTokPublishMode {
    #[default]
    Direct,
    Draft,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UploadMeta {
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    pub visibility: String,
    pub clip_id: String,
    pub force: bool,
    /// Exact immutable local render selected for this handoff. New callers
    /// provide all three fields; legacy stored jobs omit all three and fall
    /// back to the clip's last output path.
    #[serde(default)]
    pub artifact_path: Option<String>,
    #[serde(default)]
    pub artifact_revision: Option<String>,
    #[serde(default)]
    pub artifact_aspect_ratio: Option<String>,
    // ── TikTok Content Posting API compliance fields ──
    // Ignored by YouTube/Instagram. `#[serde(default)]` keeps this backward
    // compatible: existing frontend callers and stored scheduled-upload JSON
    // blobs that omit these fields still deserialize.
    #[serde(default)]
    pub disable_comment: bool,
    #[serde(default)]
    pub disable_duet: bool,
    #[serde(default)]
    pub disable_stitch: bool,
    /// "Your brand" disclosure → TikTok `brand_organic_toggle`.
    #[serde(default)]
    pub brand_organic: bool,
    /// "Branded content" disclosure → TikTok `brand_content_toggle`.
    #[serde(default)]
    pub branded_content: bool,
    /// Direct Post publishes from ClipGoblin; Draft hands the video to the
    /// creator's TikTok inbox so they can finish editing and publish there.
    #[serde(default)]
    pub tiktok_publish_mode: TikTokPublishMode,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status")]
pub enum UploadResultStatus {
    #[serde(rename = "uploading")]
    Uploading { progress_pct: u8 },
    #[serde(rename = "processing")]
    Processing,
    #[serde(rename = "inbox_delivered")]
    InboxDelivered,
    #[serde(rename = "complete")]
    Complete {
        video_url: Option<String>,
        platform_video_id: Option<String>,
    },
    #[serde(rename = "failed")]
    Failed { error: String },
    #[serde(rename = "duplicate")]
    Duplicate { existing_url: Option<String> },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UploadResult {
    pub status: UploadResultStatus,
    pub job_id: String,
}

/// Emit a live `upload-status` event so the publish UI can show real phase
/// transitions (chunk progress, platform-side processing). Best-effort:
/// no-op when the app handle isn't set (unit tests, headless).
pub fn emit_upload_status(platform: &str, clip_id: &str, phase: &str, progress_pct: Option<u8>) {
    if let Some(handle) = crate::APP_HANDLE.get() {
        use tauri::Emitter;
        let _ = handle.emit(
            "upload-status",
            serde_json::json!({
                "platform": platform,
                "clip_id": clip_id,
                "phase": phase,
                "progress_pct": progress_pct,
            }),
        );
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Platform adapter trait
// ═══════════════════════════════════════════════════════════════════

#[async_trait::async_trait(?Send)]
pub trait PlatformAdapter: Send + Sync {
    fn platform_id(&self) -> &'static str;
    fn is_ready(&self, db: &Connection) -> Result<bool, AppError>;
    async fn start_auth(&self) -> Result<String, AppError>;
    async fn handle_callback(
        &self,
        db: &crate::DbConn,
        code: &str,
    ) -> Result<ConnectedAccount, AppError>;
    async fn refresh_token(&self, db: &crate::DbConn) -> Result<(), AppError>;
    /// Takes the shared `DbConn` (not a held guard) so the impl can lock only
    /// for the DB reads/refresh and the final record, releasing the lock for the
    /// long network upload in between.
    async fn upload_video(
        &self,
        db: &crate::DbConn,
        file_path: &str,
        meta: &UploadMeta,
    ) -> Result<UploadResult, AppError>;
    fn disconnect(&self, db: &Connection) -> Result<(), AppError>;
    fn get_account(&self, db: &Connection) -> Result<Option<ConnectedAccount>, AppError>;
}

// ═══════════════════════════════════════════════════════════════════
//  Dispatcher
// ═══════════════════════════════════════════════════════════════════

pub fn get_adapter(platform: &str) -> Result<Box<dyn PlatformAdapter>, AppError> {
    match platform {
        "youtube" => Ok(Box::new(youtube::YouTubeAdapter)),
        "tiktok" => Ok(Box::new(tiktok::TikTokAdapter)),
        "instagram" => Ok(Box::new(instagram::InstagramAdapter)),
        _ => Err(AppError::NotSupported(format!(
            "Unknown platform: {}",
            platform
        ))),
    }
}

// ═══════════════════════════════════════════════════════════════════
//  File validation
// ═══════════════════════════════════════════════════════════════════

pub fn validate_export_file(output_path: Option<&str>) -> Result<&str, AppError> {
    let path_str =
        output_path.ok_or_else(|| AppError::NotFound("Clip has not been exported yet".into()))?;

    let path = Path::new(path_str);

    if !path.exists() {
        return Err(AppError::NotFound(
            "Export file not found — re-export the clip first".into(),
        ));
    }

    let metadata = std::fs::metadata(path)
        .map_err(|e| AppError::Unknown(format!("Cannot read export file: {}", e)))?;

    if metadata.len() == 0 {
        return Err(AppError::NotFound(
            "Export file is empty — re-export the clip".into(),
        ));
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if !["mp4", "webm", "mov"].contains(&ext.as_str()) {
        return Err(AppError::NotSupported(format!(
            "Unsupported file format '.{}' for upload (expected .mp4, .webm, or .mov)",
            ext
        )));
    }

    Ok(path_str)
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

fn artifact_file_tag(aspect_ratio: &str) -> Result<&'static str, AppError> {
    match aspect_ratio {
        "9:16" => Ok("9x16"),
        "16:9" => Ok("16x9"),
        "1:1" => Ok("1x1"),
        other => Err(AppError::NotSupported(format!(
            "Unsupported rendered aspect ratio '{other}'",
        ))),
    }
}

fn expected_artifact_dimensions(aspect_ratio: &str) -> Result<(u32, u32), AppError> {
    match aspect_ratio {
        "9:16" => Ok((1080, 1920)),
        "16:9" => Ok((1920, 1080)),
        "1:1" => Ok((1080, 1080)),
        other => Err(AppError::NotSupported(format!(
            "Unsupported rendered aspect ratio '{other}'",
        ))),
    }
}

fn validate_artifact_identity(
    path: &Path,
    revision: &str,
    aspect_ratio: &str,
) -> Result<(), AppError> {
    if revision.len() != 64 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError::Unknown(
            "Rendered artifact has an invalid revision identity".into(),
        ));
    }
    let expected_name = format!("{}-{revision}.mp4", artifact_file_tag(aspect_ratio)?);
    let actual_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if actual_name != expected_name {
        return Err(AppError::Unknown(
            "Rendered artifact identity does not match its filename".into(),
        ));
    }
    Ok(())
}

fn probe_artifact_dimensions(path: &Path) -> Result<(u32, u32), AppError> {
    let ffprobe =
        crate::bin_manager::ffprobe_path().map_err(|error| AppError::Ffmpeg(error.to_string()))?;
    let mut command = Command::new(ffprobe);
    command
        .arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("v:0")
        .arg("-show_entries")
        .arg("stream=width,height")
        .arg("-of")
        .arg("csv=p=0:s=x")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command
        .output()
        .map_err(|error| AppError::Ffmpeg(format!("Could not inspect rendered video: {error}")))?;
    if !output.status.success() {
        return Err(AppError::Ffmpeg(
            "Could not verify the rendered video's dimensions".into(),
        ));
    }
    let dimensions = String::from_utf8_lossy(&output.stdout);
    let (width, height) = dimensions
        .trim()
        .split_once('x')
        .ok_or_else(|| AppError::Ffmpeg("Rendered video dimensions were unreadable".into()))?;
    let width = width
        .parse::<u32>()
        .map_err(|_| AppError::Ffmpeg("Rendered video width was unreadable".into()))?;
    let height = height
        .parse::<u32>()
        .map_err(|_| AppError::Ffmpeg("Rendered video height was unreadable".into()))?;
    Ok((width, height))
}

pub fn resolve_upload_artifact(
    platform: &str,
    meta: &UploadMeta,
    legacy_output_path: Option<&str>,
) -> Result<String, AppError> {
    let (artifact_path, revision, aspect_ratio) = match (
        meta.artifact_path.as_deref(),
        meta.artifact_revision.as_deref(),
        meta.artifact_aspect_ratio.as_deref(),
    ) {
        (None, None, None) => {
            return validate_export_file(legacy_output_path).map(str::to_string);
        }
        (Some(path), Some(revision), Some(aspect_ratio)) => (path, revision, aspect_ratio),
        _ => {
            return Err(AppError::Unknown(
                "Rendered artifact metadata is incomplete; prepare the clip again".into(),
            ));
        }
    };

    if matches!(platform, "tiktok" | "instagram") && aspect_ratio != "9:16" {
        return Err(AppError::Unknown(format!(
            "{} requires ClipGoblin's 9:16 render",
            if platform == "tiktok" {
                "TikTok"
            } else {
                "Instagram"
            },
        )));
    }

    let validated_path = validate_export_file(Some(artifact_path))?;
    let canonical_path = Path::new(validated_path)
        .canonicalize()
        .map_err(|error| AppError::Unknown(format!("Cannot resolve rendered video: {error}")))?;
    let export_root = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clipviral")
        .join("exports")
        .canonicalize()
        .map_err(|error| AppError::Unknown(format!("Cannot resolve export directory: {error}")))?;
    if !canonical_path.starts_with(&export_root) {
        return Err(AppError::Unknown(
            "Upload file is outside ClipGoblin's managed export directory".into(),
        ));
    }
    let expected_clip_dir = safe_path_component(&meta.clip_id);
    if canonical_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        != Some(expected_clip_dir.as_str())
    {
        return Err(AppError::Unknown(
            "Rendered artifact belongs to a different clip".into(),
        ));
    }
    validate_artifact_identity(&canonical_path, revision, aspect_ratio)?;
    let expected_dimensions = expected_artifact_dimensions(aspect_ratio)?;
    let actual_dimensions = probe_artifact_dimensions(&canonical_path)?;
    if actual_dimensions != expected_dimensions {
        return Err(AppError::Unknown(format!(
            "Rendered video is {}x{}, but {} requires {}x{}",
            actual_dimensions.0,
            actual_dimensions.1,
            aspect_ratio,
            expected_dimensions.0,
            expected_dimensions.1,
        )));
    }

    Ok(canonical_path.to_string_lossy().to_string())
}

// ═══════════════════════════════════════════════════════════════════
//  Helpers
// ═══════════════════════════════════════════════════════════════════

pub fn get_all_accounts(db: &Connection) -> Result<Vec<ConnectedAccount>, AppError> {
    let adapters: Vec<Box<dyn PlatformAdapter>> = vec![
        Box::new(youtube::YouTubeAdapter),
        Box::new(tiktok::TikTokAdapter),
        Box::new(instagram::InstagramAdapter),
    ];
    let mut accounts = Vec::new();
    for adapter in &adapters {
        if let Ok(Some(acct)) = adapter.get_account(db) {
            accounts.push(acct);
        }
    }
    Ok(accounts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_upload_metadata_defaults_tiktok_to_direct_post() {
        let meta: UploadMeta = serde_json::from_value(serde_json::json!({
            "title": "title",
            "description": "description",
            "tags": [],
            "visibility": "private",
            "clip_id": "clip-1",
            "force": false
        }))
        .unwrap();

        assert_eq!(meta.tiktok_publish_mode, TikTokPublishMode::Direct);
        assert!(meta.artifact_path.is_none());
        assert!(meta.artifact_revision.is_none());
        assert!(meta.artifact_aspect_ratio.is_none());
    }

    #[test]
    fn tiktok_draft_mode_round_trips_as_snake_case() {
        let json = serde_json::to_value(TikTokPublishMode::Draft).unwrap();
        assert_eq!(json, serde_json::json!("draft"));
        assert_eq!(
            serde_json::from_value::<TikTokPublishMode>(json).unwrap(),
            TikTokPublishMode::Draft
        );
    }

    #[test]
    fn inbox_delivery_serializes_as_a_distinct_upload_status() {
        let json = serde_json::to_value(UploadResultStatus::InboxDelivered).unwrap();
        assert_eq!(json, serde_json::json!({ "status": "inbox_delivered" }));
    }

    #[test]
    fn immutable_artifact_identity_binds_revision_and_format_to_filename() {
        let revision = "a".repeat(64);
        assert!(validate_artifact_identity(
            Path::new(&format!("9x16-{revision}.mp4")),
            &revision,
            "9:16",
        )
        .is_ok());
        assert!(validate_artifact_identity(
            Path::new(&format!("16x9-{revision}.mp4")),
            &revision,
            "9:16",
        )
        .is_err());
        assert!(validate_artifact_identity(Path::new("9x16-short.mp4"), "short", "9:16").is_err());
    }

    #[test]
    fn immutable_artifact_dimensions_are_format_specific() {
        assert_eq!(expected_artifact_dimensions("9:16").unwrap(), (1080, 1920));
        assert_eq!(expected_artifact_dimensions("16:9").unwrap(), (1920, 1080));
        assert_eq!(expected_artifact_dimensions("1:1").unwrap(), (1080, 1080));
        assert!(expected_artifact_dimensions("4:3").is_err());
    }
}
