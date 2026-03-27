//! Platform publishing adapters.
//!
//! Shared trait + dispatcher for YouTube, TikTok, Instagram.
//! YouTube is fully implemented; TikTok/Instagram are stubs.

pub mod youtube;
pub mod tiktok;
pub mod instagram;

use crate::db;
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UploadMeta {
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    pub visibility: String,
    pub clip_id: String,
    pub force: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status")]
pub enum UploadResultStatus {
    #[serde(rename = "uploading")]
    Uploading { progress_pct: u8 },
    #[serde(rename = "processing")]
    Processing,
    #[serde(rename = "complete")]
    Complete { video_url: String },
    #[serde(rename = "failed")]
    Failed { error: String },
    #[serde(rename = "duplicate")]
    Duplicate { existing_url: String },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UploadResult {
    pub status: UploadResultStatus,
    pub job_id: String,
}

// ═══════════════════════════════════════════════════════════════════
//  Platform adapter trait
// ═══════════════════════════════════════════════════════════════════

#[async_trait::async_trait]
pub trait PlatformAdapter: Send + Sync {
    fn platform_id(&self) -> &'static str;
    fn is_ready(&self, db: &Connection) -> Result<bool, AppError>;
    async fn start_auth(&self) -> Result<String, AppError>;
    async fn handle_callback(&self, db: &Connection, code: &str) -> Result<ConnectedAccount, AppError>;
    async fn refresh_token(&self, db: &Connection) -> Result<(), AppError>;
    async fn upload_video(&self, db: &Connection, file_path: &str, meta: &UploadMeta) -> Result<UploadResult, AppError>;
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
