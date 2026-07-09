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

/// Returns `true` if the bundled yt-dlp should be refreshed: no prior
/// refresh recorded, an unparseable timestamp, or the last refresh is
/// older than `threshold_days`. Pure — caller supplies the stored value.
pub fn ytdlp_is_stale(last_refresh_rfc3339: Option<&str>, threshold_days: i64) -> bool {
    match last_refresh_rfc3339 {
        None => true,
        Some(s) => match chrono::DateTime::parse_from_rfc3339(s) {
            Ok(ts) => {
                let age = chrono::Utc::now().signed_duration_since(ts.with_timezone(&chrono::Utc));
                age > chrono::Duration::days(threshold_days)
            }
            Err(_) => true,
        },
    }
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

/// Read the first two bytes of a file (its "magic"), for the download
/// integrity gate. Errors if the file is shorter than two bytes — which for a
/// freshly-downloaded binary is itself a sign of a broken transfer.
async fn read_leading_bytes(path: &std::path::Path) -> Result<[u8; 2], AppError> {
    use tokio::io::AsyncReadExt;
    let mut f = tokio::fs::File::open(path)
        .await
        .map_err(|e| AppError::Download(format!("reopen for magic: {e}")))?;
    let mut buf = [0u8; 2];
    f.read_exact(&mut buf)
        .await
        .map_err(|e| AppError::Download(format!("read magic: {e}")))?;
    Ok(buf)
}

/// Conservative minimum sizes for the download integrity gate. An error page is
/// KB-sized while the real binaries are many MB, so a 1 MB floor cleanly
/// separates them without risking a false reject if upstream repackaging shifts
/// the exact size.
const YTDLP_MIN_BYTES: u64 = 1_000_000;
const FFMPEG_ZIP_MIN_BYTES: u64 = 1_000_000;

/// Integrity gate for a freshly-downloaded binary before its tmp file is
/// promoted into place. Both binaries track the upstream `latest` release, so
/// there is no stable checksum to pin — but we can still reject the failure
/// modes that actually occur: a silently-truncated transfer (byte count won't
/// match the advertised `Content-Length`), a suspiciously tiny response (below
/// `min_size` — the backstop when no `Content-Length` is sent), and a non-binary
/// response such as an HTML error page (wrong leading magic bytes). This is
/// integrity-against-corruption, not supply-chain attestation — TLS already
/// authenticates the GitHub origin the bytes come from.
fn validate_download(
    name: &str,
    downloaded: u64,
    total: u64,
    magic: &[u8],
    expect_magic: &[u8],
    min_size: u64,
) -> Result<(), AppError> {
    if total > 0 && downloaded != total {
        return Err(AppError::Download(format!(
            "{name} download incomplete: got {downloaded} of {total} bytes (truncated transfer)"
        )));
    }
    if downloaded < min_size {
        return Err(AppError::Download(format!(
            "{name} download too small: {downloaded} bytes (expected at least {min_size})"
        )));
    }
    if magic.len() < expect_magic.len() || &magic[..expect_magic.len()] != expect_magic {
        return Err(AppError::Download(format!(
            "{name} download is not a valid binary (got a truncated or error-page response)"
        )));
    }
    Ok(())
}

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

    // Integrity gate: reject a truncated/error-page download before it is
    // promoted to yt-dlp.exe (a bad exe would only surface at run time). Any
    // failure here — including a file too short to even read the magic — cleans
    // up the tmp file.
    let gate = read_leading_bytes(&tmp_path).await.and_then(|magic| {
        validate_download("yt-dlp", downloaded, total, &magic, b"MZ", YTDLP_MIN_BYTES)
    });
    if let Err(e) = gate {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(e);
    }

    let _ = tokio::fs::remove_file(&final_path).await;
    tokio::fs::rename(&tmp_path, &final_path).await
        .map_err(|e| AppError::Download(format!("rename: {e}")))?;

    log::info!("[bin_manager] yt-dlp installed to {}", final_path.display());
    Ok(())
}

/// Settings key holding the RFC3339 timestamp of the last successful
/// bundled-yt-dlp refresh.
pub const YTDLP_LAST_REFRESH_KEY: &str = "ytdlp_last_refresh";

/// Force a yt-dlp refresh regardless of staleness. Downloads the latest
/// release over the bundled binary and returns Ok on success. Caller is
/// responsible for recording the refresh timestamp (it owns the DB conn).
pub async fn force_refresh_ytdlp(progress: &ProgressCb) -> Result<(), AppError> {
    log::info!("[bin_manager] force-refreshing yt-dlp");
    download_ytdlp(progress).await
}

/// Background startup refresh: only acts when a *bundled* yt-dlp exists
/// AND the recorded last refresh is missing/older than 7 days. Never
/// errors hard — a stale binary still beats no binary. Returns `true`
/// if a refresh was performed (so the caller can record the timestamp).
pub async fn refresh_ytdlp_if_stale(last_refresh_rfc3339: Option<String>) -> bool {
    // Strictly gate on a bundled binary — never touch a user's PATH yt-dlp.
    if bundled_path("yt-dlp.exe").is_none() {
        log::info!("[bin_manager] no bundled yt-dlp; skipping staleness refresh (system-PATH user)");
        return false;
    }
    if !ytdlp_is_stale(last_refresh_rfc3339.as_deref(), 7) {
        log::info!("[bin_manager] bundled yt-dlp is fresh; no refresh needed");
        return false;
    }
    log::info!("[bin_manager] bundled yt-dlp is stale; refreshing in background");
    let noop: ProgressCb = Box::new(|_, _| {});
    match download_ytdlp(&noop).await {
        Ok(()) => {
            log::info!("[bin_manager] background yt-dlp refresh complete");
            true
        }
        Err(e) => {
            log::warn!("[bin_manager] background yt-dlp refresh failed (keeping existing binary): {}", e);
            false
        }
    }
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

    // Integrity gate before extraction: a truncated/error-page download would
    // otherwise fail deeper in the zip parser with a murkier message. Any
    // failure here cleans up the tmp zip.
    let gate = read_leading_bytes(&zip_path).await.and_then(|magic| {
        validate_download("ffmpeg", downloaded, total, &magic, b"PK", FFMPEG_ZIP_MIN_BYTES)
    });
    if let Err(e) = gate {
        let _ = tokio::fs::remove_file(&zip_path).await;
        return Err(e);
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ytdlp_is_stale_logic() {
        use chrono::{Duration, Utc};
        // No prior refresh recorded → stale.
        assert!(ytdlp_is_stale(None, 7));
        // Refreshed just now → fresh.
        let now = Utc::now().to_rfc3339();
        assert!(!ytdlp_is_stale(Some(&now), 7));
        // Refreshed 3 days ago, 7-day threshold → fresh.
        let three_days = (Utc::now() - Duration::days(3)).to_rfc3339();
        assert!(!ytdlp_is_stale(Some(&three_days), 7));
        // Refreshed 8 days ago, 7-day threshold → stale.
        let eight_days = (Utc::now() - Duration::days(8)).to_rfc3339();
        assert!(ytdlp_is_stale(Some(&eight_days), 7));
        // Unparseable timestamp → treat as stale (safe default).
        assert!(ytdlp_is_stale(Some("not-a-date"), 7));
    }

    #[test]
    fn validate_download_accepts_complete_binary() {
        // Exact size match + correct magic, comfortably above the floor.
        assert!(validate_download("yt-dlp", 100, 100, b"MZxx", b"MZ", 10).is_ok());
        // Unknown Content-Length (0) skips the exact check; magic + floor enforced.
        assert!(validate_download("yt-dlp", 100, 0, b"MZxx", b"MZ", 10).is_ok());
        assert!(validate_download("ffmpeg", 50, 50, b"PK\x03\x04", b"PK", 10).is_ok());
    }

    #[test]
    fn validate_download_rejects_truncated_transfer() {
        // Magic is correct; only the byte count is short → still rejected.
        assert!(validate_download("yt-dlp", 90, 100, b"MZxx", b"MZ", 10).is_err());
    }

    #[test]
    fn validate_download_rejects_undersized_download() {
        // No Content-Length, correct magic, but far below the sane floor
        // (e.g. a tiny error page that happens to start with "MZ").
        assert!(validate_download("yt-dlp", 20, 0, b"MZxx", b"MZ", 1000).is_err());
    }

    #[test]
    fn validate_download_rejects_non_binary_response() {
        // Full size but an HTML error page ("<!doctype…") instead of a PE.
        assert!(validate_download("yt-dlp", 100, 100, b"<!", b"MZ", 10).is_err());
        // Fewer bytes than the signature length → rejected.
        assert!(validate_download("ffmpeg", 1, 0, b"P", b"PK", 0).is_err());
    }

    /// Real network + filesystem test. Downloads ~150 MB to
    /// `%APPDATA%\clipviral\bin\`. Run explicitly:
    ///   cargo test --lib bin_manager::tests::download_real -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn download_real() {
        let dir = bin_dir().expect("bin_dir");
        let yt = dir.join("yt-dlp.exe");
        let ff = dir.join("ffmpeg.exe");
        let fp = dir.join("ffprobe.exe");
        let _ = std::fs::remove_file(&yt);
        let _ = std::fs::remove_file(&ff);
        let _ = std::fs::remove_file(&fp);

        let cb: ProgressCb = Box::new(|d, t| {
            if t > 0 && d % (5 * 1024 * 1024) < 64 * 1024 {
                eprintln!("  progress: {} / {} MB ({}%)", d / 1_000_000, t / 1_000_000, (d * 100) / t);
            }
        });

        eprintln!("-- downloading yt-dlp --");
        download_ytdlp(&cb).await.expect("yt-dlp download failed");
        assert!(yt.exists(), "yt-dlp.exe missing after download");
        let yt_size = std::fs::metadata(&yt).unwrap().len();
        eprintln!("  yt-dlp.exe {} MB", yt_size / 1_000_000);
        assert!(yt_size > 5_000_000, "yt-dlp.exe suspiciously small: {yt_size}");

        eprintln!("-- downloading ffmpeg --");
        download_ffmpeg(&cb).await.expect("ffmpeg download failed");
        assert!(ff.exists(), "ffmpeg.exe missing after extract");
        assert!(fp.exists(), "ffprobe.exe missing after extract");
        let ff_size = std::fs::metadata(&ff).unwrap().len();
        let fp_size = std::fs::metadata(&fp).unwrap().len();
        eprintln!("  ffmpeg.exe {} MB", ff_size / 1_000_000);
        eprintln!("  ffprobe.exe {} MB", fp_size / 1_000_000);
        assert!(ff_size > 10_000_000);
        assert!(fp_size > 10_000_000);
    }
}
