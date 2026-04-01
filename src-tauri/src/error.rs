//! Unified error handling for backend operations.
//!
//! [`AppError`] classifies every failure by subsystem so the frontend can
//! show targeted guidance (e.g. "install ffmpeg" vs "check API key").
//!
//! This module has **zero** Tauri dependency — event emission lives in lib.rs.

use std::fmt;

// ── Error enum ──

/// Categorised application error.
///
/// Each variant carries a human-readable detail string.
/// [`Display`] produces a user-friendly message suitable for toasts/dialogs.
#[derive(Debug, Clone)]
pub enum AppError {
    /// ffmpeg not found or execution failed.
    Ffmpeg(String),
    /// Python / transcribe.py / faster-whisper failure.
    Transcription(String),
    /// External HTTP API error (Claude, Twitch, etc.).
    Api(String),
    /// yt-dlp download failure.
    Download(String),
    /// SQLite / database error.
    Database(String),
    /// A required resource (VOD, clip, file) was not found.
    NotFound(String),
    /// Feature not yet supported (e.g. TikTok, Instagram stubs).
    NotSupported(String),
    /// Catch-all for errors that don't fit a specific category.
    Unknown(String),
}

impl AppError {
    /// Short machine-readable category tag.
    pub fn category(&self) -> &'static str {
        match self {
            Self::Ffmpeg(_) => "ffmpeg",
            Self::Transcription(_) => "transcription",
            Self::Api(_) => "api",
            Self::Download(_) => "download",
            Self::Database(_) => "database",
            Self::NotFound(_) => "not_found",
            Self::NotSupported(_) => "not_supported",
            Self::Unknown(_) => "unknown",
        }
    }

    /// The inner detail message.
    pub fn detail(&self) -> &str {
        match self {
            Self::Ffmpeg(s)
            | Self::Transcription(s)
            | Self::Api(s)
            | Self::Download(s)
            | Self::Database(s)
            | Self::NotFound(s)
            | Self::NotSupported(s)
            | Self::Unknown(s) => s,
        }
    }

    /// Build a serialisable event payload for the frontend.
    pub fn to_event(&self) -> ErrorEvent {
        ErrorEvent {
            category: self.category().to_string(),
            message: self.to_string(),
            detail: self.detail().to_string(),
        }
    }
}

// ── User-friendly Display ──

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ffmpeg(d) => write!(f, "FFmpeg error: {d}"),
            Self::Transcription(d) => write!(f, "Transcription error: {d}"),
            Self::Api(d) => write!(f, "API error: {d}"),
            Self::Download(d) => write!(f, "Download error: {d}"),
            Self::Database(d) => write!(f, "Database error: {d}"),
            Self::NotFound(d) => write!(f, "Not found: {d}"),
            Self::NotSupported(d) => write!(f, "Not supported: {d}"),
            Self::Unknown(d) => write!(f, "Error: {d}"),
        }
    }
}

// ── Backward compatibility ──
// Existing Tauri commands return Result<T, String>. This impl lets `?`
// auto-convert AppError → String so we can migrate functions one at a time.

impl From<AppError> for String {
    fn from(err: AppError) -> Self {
        err.to_string()
    }
}

// ── Source error conversions ──

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Database(e.to_string())
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        Self::Api(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::Unknown(format!("JSON parse error: {e}"))
    }
}

// Note: std::io::Error is intentionally NOT converted here because the
// category depends on context (ffmpeg IO vs transcript IO vs general).
// Use .map_err(|e| AppError::Ffmpeg(e.to_string())) at the call site.

// ── Frontend event payload ──

/// Structured error payload emitted via the `"job-error"` event.
/// Fields serialize as camelCase to match JS/TS conventions.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorEvent {
    /// Machine-readable category: "ffmpeg", "transcription", "api", etc.
    pub category: String,
    /// User-friendly message (same as Display output).
    pub message: String,
    /// Raw detail string for logging/debugging.
    pub detail: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_formats_user_friendly_message() {
        let err = AppError::Ffmpeg("not found".into());
        assert_eq!(err.to_string(), "FFmpeg error: not found");
    }

    #[test]
    fn category_returns_machine_tag() {
        assert_eq!(AppError::Api("timeout".into()).category(), "api");
        assert_eq!(AppError::Transcription("crash".into()).category(), "transcription");
        assert_eq!(AppError::Database("locked".into()).category(), "database");
    }

    #[test]
    fn into_string_uses_display() {
        let err = AppError::Download("404".into());
        let s: String = err.into();
        assert_eq!(s, "Download error: 404");
    }

    #[test]
    fn to_event_serializes_camel_case() {
        let err = AppError::Api("rate limited".into());
        let event = err.to_event();
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["category"], "api");
        assert!(json["message"].as_str().unwrap().contains("rate limited"));
        assert!(json.get("detail").is_some());
    }

    #[test]
    fn from_rusqlite_error() {
        let e = rusqlite::Error::QueryReturnedNoRows;
        let app_err: AppError = e.into();
        assert_eq!(app_err.category(), "database");
    }
}
