//! Manages bundled external binaries (ffmpeg, ffprobe, yt-dlp).
//!
//! Priority order when resolving a binary path:
//! 1. `%APPDATA%/clipviral/bin/<name>.exe` (bundled, auto-downloaded)
//! 2. System PATH (existing behaviour — users who already have them installed)
//! 3. Return error / trigger download UI

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Stdio;

use futures_util::StreamExt;
use serde::Serialize;
use tokio::io::AsyncWriteExt;

use crate::error::AppError;

// ── Download URLs ──

/// BtbN FFmpeg-Builds latest Win64 GPL release. Contains `bin/ffmpeg.exe` and
/// `bin/ffprobe.exe` inside a zip (~130 MB compressed).
const FFMPEG_ZIP_URL: &str =
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip";

/// yt-dlp's GitHub "latest" redirect to the most recent Windows exe (~20 MB).
const YTDLP_URL: &str =
    "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";

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

// ── Downloads ──

/// Progress callback: `(bytes_downloaded, total_bytes_or_zero_if_unknown)`.
pub type ProgressCb = Box<dyn Fn(u64, u64) + Send + Sync>;

pub async fn download_ytdlp(progress: &ProgressCb) -> Result<(), AppError> {
    let dir = bin_dir()?;
    let final_path = dir.join("yt-dlp.exe");
    let tmp_path = dir.join("yt-dlp.exe.tmp");

    log::info!("[bin_manager] downloading yt-dlp from {}", YTDLP_URL);

    let client = reqwest::Client::new();
    let resp = client.get(YTDLP_URL).send().await
        .map_err(|e| AppError::Download(format!("yt-dlp request: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Download(format!("yt-dlp HTTP {}", resp.status())));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(&tmp_path).await
        .map_err(|e| AppError::Download(format!("create tmp: {e}")))?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| AppError::Download(format!("stream: {e}")))?;
        file.write_all(&chunk).await
            .map_err(|e| AppError::Download(format!("write: {e}")))?;
        downloaded += chunk.len() as u64;
        progress(downloaded, total);
    }
    file.flush().await.map_err(|e| AppError::Download(format!("flush: {e}")))?;
    drop(file);

    let _ = tokio::fs::remove_file(&final_path).await;
    tokio::fs::rename(&tmp_path, &final_path).await
        .map_err(|e| AppError::Download(format!("rename: {e}")))?;

    log::info!("[bin_manager] yt-dlp installed to {}", final_path.display());
    Ok(())
}

/// Download the FFmpeg zip to a temp file, then extract `ffmpeg.exe` and
/// `ffprobe.exe` into `bin_dir()`.
///
/// Progress is reported during the *download* phase only (not extraction);
/// the zip is tiny to extract compared to downloading ~130 MB.
pub async fn download_ffmpeg(progress: &ProgressCb) -> Result<(), AppError> {
    let dir = bin_dir()?;
    let zip_path = dir.join("ffmpeg-download.zip.tmp");

    log::info!("[bin_manager] downloading ffmpeg from {}", FFMPEG_ZIP_URL);

    let client = reqwest::Client::new();
    let resp = client.get(FFMPEG_ZIP_URL).send().await
        .map_err(|e| AppError::Download(format!("ffmpeg request: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Download(format!("ffmpeg HTTP {}", resp.status())));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(&zip_path).await
        .map_err(|e| AppError::Download(format!("create zip tmp: {e}")))?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let _ = std::fs::remove_file(&zip_path);
                return Err(AppError::Download(format!("stream: {e}")));
            }
        };
        file.write_all(&chunk).await
            .map_err(|e| AppError::Download(format!("write zip: {e}")))?;
        downloaded += chunk.len() as u64;
        progress(downloaded, total);
    }
    file.flush().await.map_err(|e| AppError::Download(format!("flush: {e}")))?;
    drop(file);

    let zip_path_extract = zip_path.clone();
    let dir_extract = dir.clone();
    let extract_result = tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        extract_ffmpeg_bins(&zip_path_extract, &dir_extract)
    })
    .await
    .map_err(|e| AppError::Unknown(format!("extract join: {e}")))?;

    let _ = tokio::fs::remove_file(&zip_path).await;

    extract_result?;

    log::info!("[bin_manager] ffmpeg + ffprobe installed to {}", dir.display());
    Ok(())
}

fn extract_ffmpeg_bins(zip_path: &std::path::Path, dir: &std::path::Path) -> Result<(), AppError> {
    let f = std::fs::File::open(zip_path)
        .map_err(|e| AppError::Download(format!("open zip: {e}")))?;
    let mut archive = zip::ZipArchive::new(f)
        .map_err(|e| AppError::Download(format!("read zip: {e}")))?;

    let targets = ["ffmpeg.exe", "ffprobe.exe"];
    let mut extracted = [false; 2];

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)
            .map_err(|e| AppError::Download(format!("zip entry {i}: {e}")))?;
        if !entry.is_file() { continue; }

        let name = entry.name().to_string();
        let leaf = std::path::Path::new(&name)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        for (idx, target) in targets.iter().enumerate() {
            if leaf.eq_ignore_ascii_case(target) && !extracted[idx] {
                let out = dir.join(target);
                let tmp = dir.join(format!("{target}.tmp"));
                let mut buf = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut buf)
                    .map_err(|e| AppError::Download(format!("read {target}: {e}")))?;
                let mut f = std::fs::File::create(&tmp)
                    .map_err(|e| AppError::Download(format!("create {target}.tmp: {e}")))?;
                f.write_all(&buf)
                    .map_err(|e| AppError::Download(format!("write {target}: {e}")))?;
                drop(f);
                let _ = std::fs::remove_file(&out);
                std::fs::rename(&tmp, &out)
                    .map_err(|e| AppError::Download(format!("rename {target}: {e}")))?;
                extracted[idx] = true;
                break;
            }
        }

        if extracted.iter().all(|b| *b) { break; }
    }

    if !extracted[0] {
        return Err(AppError::Download("ffmpeg.exe not found in zip".into()));
    }
    if !extracted[1] {
        return Err(AppError::Download("ffprobe.exe not found in zip".into()));
    }
    Ok(())
}
