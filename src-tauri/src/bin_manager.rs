//! Manages bundled external binaries (ffmpeg, ffprobe, yt-dlp).
//!
//! Priority order when resolving a binary path:
//! 1. `%APPDATA%/clipviral/bin/<name>.exe` (bundled, auto-downloaded)
//! 2. System PATH (existing behaviour — users who already have them installed)
//! 3. Return error / trigger download UI

use std::path::PathBuf;
use std::process::Stdio;

use serde::Serialize;

use crate::error::AppError;

// ── Paths ──

/// `%APPDATA%/clipviral/bin/`, creating it if it doesn't exist.
pub fn bin_dir() -> Result<PathBuf, AppError> {
    let base = dirs::data_dir()
        .ok_or_else(|| AppError::Unknown("no APPDATA dir on this system".into()))?;
    let dir = base.join("clipviral").join("bin");
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::Unknown(format!("create bin dir: {e}")))?;
    Ok(dir)
}

fn bundled_path(name: &str) -> Option<PathBuf> {
    let p = bin_dir().ok()?.join(name);
    if p.exists() { Some(p) } else { None }
}

/// Returns `true` if `<name>` runs successfully with `version_flag` via the
/// system PATH. Used as a fallback for users who already have the tool
/// installed system-wide.
fn in_path(name: &str, version_flag: &str) -> bool {
    let mut cmd = std::process::Command::new(name);
    cmd.arg(version_flag).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

/// Return a usable `ffmpeg` path: bundled first, then system PATH (bare name).
pub fn ffmpeg_path() -> Result<PathBuf, AppError> {
    if let Some(p) = bundled_path("ffmpeg.exe") {
        log::info!("[bin_manager] ffmpeg: bundled at {}", p.display());
        return Ok(p);
    }
    if in_path("ffmpeg", "-version") {
        log::info!("[bin_manager] ffmpeg: using system PATH");
        return Ok(PathBuf::from("ffmpeg"));
    }
    Err(AppError::Ffmpeg("ffmpeg not found (bundled or system PATH)".into()))
}

/// Return a usable `ffprobe` path: bundled first, then system PATH (bare name).
pub fn ffprobe_path() -> Result<PathBuf, AppError> {
    if let Some(p) = bundled_path("ffprobe.exe") {
        return Ok(p);
    }
    if in_path("ffprobe", "-version") {
        return Ok(PathBuf::from("ffprobe"));
    }
    Err(AppError::Ffmpeg("ffprobe not found (bundled or system PATH)".into()))
}

/// Return a usable `yt-dlp` path: bundled first, then system PATH (bare name).
pub fn ytdlp_path() -> Result<PathBuf, AppError> {
    if let Some(p) = bundled_path("yt-dlp.exe") {
        log::info!("[bin_manager] yt-dlp: bundled at {}", p.display());
        return Ok(p);
    }
    if in_path("yt-dlp", "--version") {
        log::info!("[bin_manager] yt-dlp: using system PATH");
        return Ok(PathBuf::from("yt-dlp"));
    }
    Err(AppError::Download("yt-dlp not found (bundled or system PATH)".into()))
}

// ── Status ──

/// Serialisable status of the three external binaries for the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryStatus {
    /// Either bundled ffmpeg.exe exists, or system PATH has ffmpeg.
    pub ffmpeg_available: bool,
    /// Either bundled ffprobe.exe exists, or system PATH has ffprobe.
    pub ffprobe_available: bool,
    /// Either bundled yt-dlp.exe exists, or system PATH has yt-dlp.
    pub ytdlp_available: bool,
    /// `true` when the bundled ffmpeg.exe file is present in the app bin dir.
    pub ffmpeg_bundled: bool,
    /// `true` when the bundled yt-dlp.exe file is present in the app bin dir.
    pub ytdlp_bundled: bool,
}

pub fn check_binaries() -> BinaryStatus {
    BinaryStatus {
        ffmpeg_available: ffmpeg_path().is_ok(),
        ffprobe_available: ffprobe_path().is_ok(),
        ytdlp_available: ytdlp_path().is_ok(),
        ffmpeg_bundled: bundled_path("ffmpeg.exe").is_some(),
        ytdlp_bundled: bundled_path("yt-dlp.exe").is_some(),
    }
}
