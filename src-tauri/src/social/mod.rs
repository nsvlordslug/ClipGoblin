//! Platform publishing adapters.
//!
//! Shared trait + dispatcher for YouTube, TikTok, Instagram.
//! YouTube is fully implemented; TikTok/Instagram are stubs.

pub mod youtube;
pub mod tiktok;
pub mod instagram;

use crate::error::AppError;
use rusqlite::Connection;
use std::path::Path;

// ═══════════════════════════════════════════════════════════════════
//  Shared types (serialized to frontend)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConnectedAccount {
    pub platform: String,
    pub account_name: String,
    pub account_id: String,
    pub connected_at: String,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize,
)]
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
        _ => Err(AppError::NotSupported(format!("Unknown platform: {}", platform))),
    }
}

// ═══════════════════════════════════════════════════════════════════
//  File validation
// ═══════════════════════════════════════════════════════════════════

pub fn validate_export_file(output_path: Option<&str>) -> Result<&str, AppError> {
    let path_str = output_path
        .ok_or_else(|| AppError::NotFound("Clip has not been exported yet".into()))?;

    let path = Path::new(path_str);

    if !path.exists() {
        return Err(AppError::NotFound(
            "Export file not found — re-export the clip first".into(),
        ));
    }

    let metadata = std::fs::metadata(path)
        .map_err(|e| AppError::Unknown(format!("Cannot read export file: {}", e)))?;

    if metadata.len() == 0 {
        return Err(AppError::NotFound("Export file is empty — re-export the clip".into()));
    }

    let ext = path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if !["mp4", "webm", "mov"].contains(&ext.as_str()) {
        return Err(AppError::NotSupported(
            format!("Unsupported file format '.{}' for upload (expected .mp4, .webm, or .mov)", ext),
        ));
    }

    Ok(path_str)
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
}
