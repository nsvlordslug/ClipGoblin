//! VOD, clip, and analysis commands.

use std::io::{BufRead, BufReader};
use std::process::Stdio;
use std::path::PathBuf;

use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::db;
use crate::DbConn;
use crate::error::AppError;
use crate::hardware::HardwareInfo;
use crate::job_queue::JobQueue;
use crate::twitch;
use crate::report_error;
use crate::commands::auth::try_refresh_twitch_token;
use crate::clip_selector;
use crate::commands::captions::{
    grounded_highlight_title, compute_confidence,
    build_highlight_explanation, count_active_signals,
};

// ── AudioProfile struct (local to this module) ──

/// Audio profile extracted from a video file.
#[derive(Debug, Clone)]
struct AudioProfile {
    /// RMS volume level per second (0.0 = silence, 1.0 = max)
    rms_per_second: Vec<f64>,
    /// Indices of detected volume spikes (>1.5x average)
    spike_seconds: Vec<usize>,
}

// ── Tool finders ──

/// Find yt-dlp executable by checking common install locations and PATH.
fn find_ytdlp() -> Result<std::path::PathBuf, AppError> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();

    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        for ver in &["Python312", "Python313", "Python311", "Python310"] {
            candidates.push(
                std::path::PathBuf::from(&local)
                    .join("Programs")
                    .join("Python")
                    .join(ver)
                    .join("Scripts")
                    .join("yt-dlp.exe"),
            );
        }
    }

    if let Ok(appdata) = std::env::var("APPDATA") {
        for ver in &["Python312", "Python313", "Python311", "Python310"] {
            candidates.push(
                std::path::PathBuf::from(&appdata)
                    .join("Python")
                    .join(ver)
                    .join("Scripts")
                    .join("yt-dlp.exe"),
            );
        }
    }

    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        candidates.push(std::path::PathBuf::from(&userprofile).join(".local").join("bin").join("yt-dlp.exe"));
    }

    for path in &candidates {
        if path.exists() {
            return Ok(path.clone());
        }
    }

    // Last resort: check PATH
    if let Ok(output) = std::process::Command::new("yt-dlp").arg("--version").output() {
        if output.status.success() {
            return Ok(std::path::PathBuf::from("yt-dlp"));
        }
    }

    Err(AppError::Download(format!(
        "yt-dlp not found. Install it with: pip install yt-dlp\nSearched: {}",
        candidates.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>().join(", ")
    )))
}

/// Find ffmpeg executable by checking common install locations and PATH.
pub(crate) fn find_ffmpeg() -> Result<std::path::PathBuf, AppError> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();

    // winget installs to a tools directory
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        candidates.push(std::path::PathBuf::from(&local).join("Microsoft").join("WinGet").join("Links").join("ffmpeg.exe"));
    }

    // Common install locations
    candidates.push(std::path::PathBuf::from("C:\\ffmpeg\\bin\\ffmpeg.exe"));
    candidates.push(std::path::PathBuf::from("C:\\Program Files\\ffmpeg\\bin\\ffmpeg.exe"));

    // App data directory (bundled)
    if let Some(data) = dirs::data_dir() {
        candidates.push(data.join("clipviral").join("ffmpeg").join("ffmpeg.exe"));
    }

    for path in &candidates {
        if path.exists() {
            return Ok(path.clone());
        }
    }

    // Check PATH
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    if let Ok(status) = cmd.status() {
        if status.success() {
            return Ok(std::path::PathBuf::from("ffmpeg"));
        }
    }

    Err(AppError::Ffmpeg("Not found. Please install ffmpeg (winget install Gyan.FFmpeg).".into()))
}

// ── Download helpers ──

/// Parse yt-dlp progress output to extract download percentage.
fn parse_ytdlp_progress(line: &str) -> Option<u8> {
    if !line.contains("[download]") {
        return None;
    }
    let pct_pos = line.find('%')?;
    let before = &line[..pct_pos];
    let trimmed = before.trim_end();
    let num_start = trimmed.rfind(|c: char| !c.is_ascii_digit() && c != '.')? + 1;
    let num_str = &trimmed[num_start..];
    let val: f64 = num_str.parse().ok()?;
    Some(val.min(100.0).max(0.0) as u8)
}

/// Download a VOD using yt-dlp with real-time progress tracking.
#[tauri::command]
pub async fn download_vod(vod_id: String, app: AppHandle, db: State<'_, DbConn>) -> Result<(), String> {
    let ytdlp = find_ytdlp().map_err(|e| report_error(&app, e))?;

    // Atomic check-and-set: read status and update in a single lock scope
    let vod = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let vod = db::get_vod_by_id(&conn, &vod_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or_else(|| "VOD not found".to_string())?;

        if vod.download_status == "downloading" {
            return Err("This VOD is already downloading.".to_string());
        }

        db::update_vod_download_status(&conn, &vod_id, "downloading", None, None)
            .map_err(|e| format!("DB error: {}", e))?;
        db::update_vod_download_progress(&conn, &vod_id, 0)
            .map_err(|e| format!("DB error: {}", e))?;
        vod
    };

    // Get download directory from settings or use default
    let download_dir = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        match db::get_setting(&conn, "download_dir") {
            Ok(Some(dir)) if !dir.is_empty() => std::path::PathBuf::from(dir),
            _ => dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("clipviral")
                .join("downloads"),
        }
    };
    std::fs::create_dir_all(&download_dir).ok();

    let output_template = download_dir
        .join(format!("{}.%(ext)s", vod.twitch_video_id))
        .to_string_lossy()
        .to_string();

    let vod_url = vod.vod_url.clone();
    let twitch_video_id = vod.twitch_video_id.clone();
    let dl_dir = download_dir.clone();
    let vod_id_bg = vod_id.clone();
    let app_handle = app.clone();

    // Spawn background task — returns immediately so UI stays responsive
    tokio::task::spawn(async move {
        let vod_id_progress = vod_id_bg.clone();
        let vod_id_status = vod_id_bg;

        let result = tokio::task::spawn_blocking(move || {
            let progress_conn = db::db_path().ok().and_then(|p| rusqlite::Connection::open(p).ok());

            let mut cmd = std::process::Command::new(&ytdlp);
            cmd.arg("--force-overwrites")
                .arg("--newline")
                .arg("--no-color")
                .arg("--remux-video").arg("mp4")
                .arg("-o")
                .arg(&output_template)
                .arg(&vod_url)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            // Tell yt-dlp where ffmpeg is so it can remux MPEG-TS to proper MP4
            if let Ok(ffmpeg) = find_ffmpeg() {
                if let Some(ffmpeg_dir) = ffmpeg.parent() {
                    cmd.arg("--ffmpeg-location").arg(ffmpeg_dir);
                }
            }

            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }

            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => return Err(format!("Failed to start yt-dlp: {}", e)),
            };

            let stderr = child.stderr.take();
            let stderr_thread = std::thread::spawn(move || {
                if let Some(err) = stderr {
                    let reader = BufReader::new(err);
                    for _ in reader.lines() {}
                }
            });

            if let Some(stdout) = child.stdout.take() {
                let reader = BufReader::new(stdout);
                let mut last_reported: u8 = 0;
                for line in reader.lines().flatten() {
                    if let Some(pct) = parse_ytdlp_progress(&line) {
                        if pct != last_reported && (pct >= last_reported.saturating_add(2) || pct == 100) {
                            last_reported = pct;
                            if let Some(ref conn) = progress_conn {
                                db::update_vod_download_progress(conn, &vod_id_progress, pct as i64).ok();
                            }
                        }
                    }
                }
            }

            let _ = stderr_thread.join();
            let status = child.wait().map_err(|e| format!("yt-dlp error: {}", e))?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("yt-dlp exited with code: {:?}", status.code()))
            }
        })
        .await;

        let db: State<'_, DbConn> = app_handle.state();

        match result {
            Ok(Ok(())) => {
                let mut found_path: Option<std::path::PathBuf> = None;
                if let Ok(entries) = std::fs::read_dir(&dl_dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.starts_with(&twitch_video_id)
                            && !name.ends_with(".part")
                            && !name.ends_with(".ytdl")
                        {
                            found_path = Some(entry.path());
                            break;
                        }
                    }
                }
                let (path_str, file_size) = match &found_path {
                    Some(p) => (
                        Some(p.to_string_lossy().to_string()),
                        std::fs::metadata(p).ok().map(|m| m.len() as i64),
                    ),
                    None => (None, None),
                };
                if let Ok(conn) = db.lock() {
                    db::update_vod_download_status(
                        &conn,
                        &vod_id_status,
                        "downloaded",
                        path_str.as_deref(),
                        file_size,
                    )
                    .ok();
                    db::update_vod_download_progress(&conn, &vod_id_status, 100).ok();
                }
            }
            _ => {
                if let Ok(conn) = db.lock() {
                    db::update_vod_download_status(&conn, &vod_id_status, "failed", None, None).ok();
                }
            }
        }
    });

    Ok(())
}

/// Get cached VODs from DB only (no Twitch API call). Used for polling status.
#[tauri::command]
pub fn get_cached_vods(channel_id: String, db: State<'_, DbConn>) -> Result<Vec<db::VodRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_vods_by_channel(&conn, &channel_id).map_err(|e| format!("DB error: {}", e))
}

// ── AI Analysis ──

/// Extract per-second audio intensity from a video file using ffmpeg.
/// Returns an AudioProfile with RMS levels and detected spike positions.
fn analyze_audio_intensity(
    vod_path: &str,
    ffmpeg: &std::path::Path,
) -> Result<AudioProfile, AppError> {
    // Use ffmpeg's volumedetect + astats to get per-second RMS levels
    // We extract audio as raw PCM and analyze volume in 1-second windows
    let temp_file = std::env::temp_dir()
        .join("clipviral_audio")
        .join(format!("{}.txt", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(temp_file.parent().unwrap()).ok();

    // Escape the path for ffmpeg filter syntax — colons in Windows drive letters
    // (e.g. C:\...) conflict with ffmpeg's filter parameter separator (:)
    let escaped_path = temp_file.to_string_lossy()
        .replace('\\', "/")
        .replace(':', "\\:");

    let mut cmd = std::process::Command::new(ffmpeg);
    cmd.arg("-i").arg(vod_path)
       .arg("-af")
       .arg(format!(
           "astats=metadata=1:reset=1,ametadata=mode=print:file='{}'",
           escaped_path
       ))
       .arg("-vn")
       .arg("-f").arg("null")
       .arg("-")
       .stdout(Stdio::null())
       .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let status = cmd.status().map_err(|e| AppError::Ffmpeg(format!("Audio analysis launch failed: {e}")))?;
    if !status.success() {
        std::fs::remove_file(&temp_file).ok();
        return Err(AppError::Ffmpeg("Audio analysis exited with an error".into()));
    }

    // Parse the astats output file for RMS levels per frame
    let content = std::fs::read_to_string(&temp_file)
        .map_err(|e| AppError::Ffmpeg(format!("Read audio stats: {e}")))?;
    std::fs::remove_file(&temp_file).ok();

    let mut rms_values: Vec<f64> = Vec::new();
    let mut current_time: Option<f64> = None;
    let mut current_rms: Option<f64> = None;
    let mut last_second: i64 = -1;
    let mut second_rms_sum = 0.0_f64;
    let mut second_count = 0u32;

    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("lavfi.astats.Overall.RMS_level=") {
            if let Ok(val) = rest.trim().parse::<f64>() {
                current_rms = Some(val);
            }
        } else if line.starts_with("frame:") {
            // Each frame line contains pts_time
            if let Some(pts_pos) = line.find("pts_time:") {
                let pts_str = &line[pts_pos + 9..];
                if let Some(end) = pts_str.find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-') {
                    if let Ok(t) = pts_str[..end].parse::<f64>() {
                        current_time = Some(t);
                    }
                } else if let Ok(t) = pts_str.trim().parse::<f64>() {
                    current_time = Some(t);
                }
            }
        }

        // Accumulate RMS into per-second buckets
        if let (Some(t), Some(rms)) = (current_time, current_rms) {
            let sec = t as i64;
            if sec != last_second && last_second >= 0 && second_count > 0 {
                // Store average RMS for the previous second
                // RMS is in dB (negative), convert to linear 0..1 scale
                let avg_db = second_rms_sum / second_count as f64;
                // Clamp: -60dB = silence (0.0), 0dB = max (1.0)
                let linear = ((avg_db + 60.0) / 60.0).clamp(0.0, 1.0);
                // Fill any gaps
                while rms_values.len() < last_second as usize {
                    rms_values.push(0.0);
                }
                rms_values.push(linear);
                second_rms_sum = 0.0;
                second_count = 0;
            }
            last_second = sec;
            second_rms_sum += rms;
            second_count += 1;
            current_rms = None;
        }
    }
    // Push last second
    if second_count > 0 {
        let avg_db = second_rms_sum / second_count as f64;
        let linear = ((avg_db + 60.0) / 60.0).clamp(0.0, 1.0);
        while rms_values.len() < last_second as usize {
            rms_values.push(0.0);
        }
        rms_values.push(linear);
    }

    if rms_values.is_empty() {
        return Err(AppError::Ffmpeg("No audio data extracted".into()));
    }

    // Detect spikes: seconds where volume > 1.5x the rolling average
    let avg: f64 = rms_values.iter().sum::<f64>() / rms_values.len() as f64;
    let spike_threshold = (avg * 1.5).max(0.3); // At least 0.3 to avoid noise
    let spike_seconds: Vec<usize> = rms_values.iter().enumerate()
        .filter(|(_, &v)| v > spike_threshold)
        .map(|(i, _)| i)
        .collect();

    log::info!("Audio analysis: {} seconds, {} spikes detected (avg={:.3}, threshold={:.3})",
        rms_values.len(), spike_seconds.len(), avg, spike_threshold);

    Ok(AudioProfile { rms_per_second: rms_values, spike_seconds })
}

/// Generate a single thumbnail frame from a video at the given timestamp.
pub(crate) fn generate_thumbnail(
    ffmpeg: &std::path::Path,
    vod_path: &str,
    timestamp_secs: f64,
    output_path: &std::path::Path,
) -> Result<(), AppError> {
    let mut cmd = std::process::Command::new(ffmpeg);
    // Input-seeking (-ss before -i) is fast and accurate for MP4 files
    cmd.arg("-ss").arg(format!("{}", timestamp_secs))
       .arg("-i").arg(vod_path)
       .arg("-vframes").arg("1")
       .arg("-vf").arg("scale=640:-1")
       .arg("-q:v").arg("5")
       .arg("-y")
       .arg(output_path.to_string_lossy().as_ref())
       .stdout(Stdio::null())
       .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let status = cmd.status().map_err(|e| AppError::Ffmpeg(format!("Thumbnail launch failed: {e}")))?;
    // ffmpeg may return non-zero (e.g. 69 for MPEG-TS near end) but still write the file
    if output_path.exists() && std::fs::metadata(output_path).map(|m| m.len() > 0).unwrap_or(false) {
        Ok(())
    } else if status.success() {
        Ok(())
    } else {
        Err(AppError::Ffmpeg("Thumbnail generation failed".into()))
    }
}

// ── Speech-to-Text (faster-whisper) ──

/// Transcript data from faster-whisper
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TranscriptWord {
    word: String,
    start: f64,
    end: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TranscriptSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub words: Vec<TranscriptWord>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TranscriptKeyword {
    pub keyword: String,
    pub timestamp: f64,
    pub end_timestamp: f64,
    pub context: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TranscriptResult {
    pub segments: Vec<TranscriptSegment>,
    pub full_text: String,
    pub language: String,
    pub keywords_found: Vec<TranscriptKeyword>,
}

/// Find Python executable path
pub(crate) fn find_python() -> Result<std::path::PathBuf, AppError> {
    // Check common Windows Python paths (user-independent)
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        for ver in &["Python312", "Python313", "Python311", "Python310"] {
            candidates.push(std::path::PathBuf::from(&local).join("Programs").join("Python").join(ver).join("python.exe"));
        }
    }
    candidates.push(std::path::PathBuf::from(r"C:\Python312\python.exe"));
    candidates.push(std::path::PathBuf::from(r"C:\Python311\python.exe"));
    for p in &candidates {
        if p.exists() {
            return Ok(p.clone());
        }
    }
    // Try PATH
    which::which("python").or_else(|_| which::which("python3"))
        .map_err(|_| AppError::Transcription("Python not found. Install Python 3.10+ to enable speech-to-text.".into()))
}

/// Run faster-whisper transcription on a video file.
/// Returns transcript JSON and saves to disk.
pub(crate) fn run_transcription(vod_path: &str, output_path: &str, hw: &HardwareInfo, vod_id: Option<&str>) -> Result<TranscriptResult, AppError> {
    let python = find_python()?;
    let device = if hw.use_cuda { "cuda" } else { "cpu" };

    // Locate transcribe.py
    let script = find_transcribe_script()?;

    log::info!("Transcription: python={} script={} device={}", python.display(), script.display(), device);

    // Quick diagnostic: check if faster-whisper is importable
    if let Ok(check) = std::process::Command::new(&python)
        .args(["-c", "import faster_whisper; print(faster_whisper.__version__)"])
        .env("CUDA_VISIBLE_DEVICES", "")
        .output()
    {
        if check.status.success() {
            let ver = String::from_utf8_lossy(&check.stdout);
            log::info!("faster-whisper version: {}", ver.trim());
        } else {
            let err = String::from_utf8_lossy(&check.stderr);
            log::warn!("faster-whisper import failed: {}", err.trim());
            return Err(AppError::Transcription(format!(
                "faster-whisper is not installed for {}. Run: {} -m pip install faster-whisper",
                python.display(), python.display()
            )));
        }
    }

    // Attempt transcription. If CUDA was requested and fails, retry on CPU.
    match run_transcription_with_script(&python, &script, vod_path, output_path, device, vod_id) {
        Ok(result) => Ok(result),
        Err(first_err) if device == "cuda" => {
            log::warn!("CUDA transcription failed ({}), retrying on CPU...", first_err.detail());
            run_transcription_with_script(&python, &script, vod_path, output_path, "cpu", vod_id)
                .map_err(|cpu_err| {
                    AppError::Transcription(format!(
                        "Failed on both CUDA and CPU. CUDA: {} | CPU: {}",
                        first_err.detail(), cpu_err.detail()
                    ))
                })
        }
        Err(e) => Err(e),
    }
}

/// Locate transcribe.py by searching project directories and AppData.
fn find_transcribe_script() -> Result<std::path::PathBuf, AppError> {
    let exe = std::env::current_exe().unwrap_or_default();
    let mut dir = exe.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();

    // Walk up from the executable directory (handles dev + release layouts)
    for _ in 0..5 {
        let candidate = dir.join("ai_engine").join("transcribe.py");
        if candidate.exists() {
            return Ok(candidate);
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => break,
        }
    }

    // Fallback: AppData directory
    let data_fallback = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clipviral")
        .join("ai_engine")
        .join("transcribe.py");
    if data_fallback.exists() {
        return Ok(data_fallback);
    }

    Err(AppError::Transcription(
        "transcribe.py not found — place it in ai_engine/ next to the executable or in AppData/clipviral/ai_engine/".into()
    ))
}

fn run_transcription_with_script(
    python: &std::path::Path,
    script: &std::path::Path,
    vod_path: &str,
    output_path: &str,
    device: &str,
    vod_id: Option<&str>,
) -> Result<TranscriptResult, AppError> {
    log::info!("Running transcription: {} {} --device {} --output {}", script.display(), vod_path, device, output_path);

    let mut cmd = std::process::Command::new(python);
    cmd.arg(script)
       .arg(vod_path)
       .arg("--model").arg("small")
       .arg("--device").arg(device)
       .arg("--output").arg(output_path)
       .stdout(Stdio::piped())
       .stderr(Stdio::piped());

    // When running in CPU mode, prevent CUDA library loading entirely.
    // faster-whisper (via CTranslate2) probes for cuBLAS at import time,
    // which crashes if CUDA DLLs are missing — even with --device cpu.
    // Blanking CUDA_VISIBLE_DEVICES forces the library to skip GPU init.
    if device == "cpu" {
        cmd.env("CUDA_VISIBLE_DEVICES", "");
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    // Spawn as a child process so we can read heartbeats from stderr
    // and enforce a timeout if the process truly hangs.
    let mut child = cmd.spawn()
        .map_err(|e| AppError::Transcription(format!("Failed to launch Python: {e}")))?;

    // Read stderr in a background thread to capture heartbeats + error output.
    // Heartbeats are JSON lines like {"heartbeat":true,"approx_pct":42,...}
    // emitted every ~15s by transcribe.py so we know it's still alive.
    let stderr_handle = child.stderr.take();
    let vod_id_for_thread = vod_id.map(|s| s.to_string());
    let (heartbeat_tx, heartbeat_rx) = std::sync::mpsc::channel::<()>();
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut err) = stderr_handle {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(&mut err);
            for line in reader.lines() {
                match line {
                    Ok(line) => {
                        // Try to parse heartbeat JSON
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                            if json.get("heartbeat").and_then(|v| v.as_bool()).unwrap_or(false) {
                                let pct = json.get("approx_pct").and_then(|v| v.as_i64()).unwrap_or(0);
                                let segs = json.get("segments_so_far").and_then(|v| v.as_i64()).unwrap_or(0);
                                log::info!("Transcription heartbeat: ~{}% done, {} segments", pct, segs);
                                // Signal the main thread that we're still alive
                                heartbeat_tx.send(()).ok();
                                // Update analysis progress (20-38% range maps to transcription)
                                if let Some(ref vid) = vod_id_for_thread {
                                    let mapped = 20 + (pct as i64 * 18 / 100).min(17);
                                    set_analysis_progress(vid, mapped);
                                }
                                continue;
                            }
                        }
                        // Not a heartbeat — collect as regular stderr
                        buf.extend_from_slice(line.as_bytes());
                        buf.push(b'\n');
                    }
                    Err(_) => break,
                }
            }
        }
        buf
    });

    // Wait for the process with a generous timeout.
    // The base timeout is 30 minutes, but resets on each heartbeat.
    // If we get no heartbeat AND no process exit for 5 minutes, assume hung.
    let no_heartbeat_timeout = std::time::Duration::from_secs(300); // 5 min with no heartbeat = stuck
    let mut last_activity = std::time::Instant::now();

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                // Drain any heartbeat signals (non-blocking)
                while heartbeat_rx.try_recv().is_ok() {
                    last_activity = std::time::Instant::now();
                }
                if last_activity.elapsed() > no_heartbeat_timeout {
                    log::error!("Transcription stalled — no heartbeat for {}s, killing process",
                        no_heartbeat_timeout.as_secs());
                    child.kill().ok();
                    child.wait().ok();
                    return Err(AppError::Transcription(
                        format!("Transcription stalled after {} minutes with no progress on {}. \
                            The process may have hung or run out of memory.",
                            no_heartbeat_timeout.as_secs() / 60, device)
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            Err(e) => {
                return Err(AppError::Transcription(format!("Failed to wait for transcription process: {e}")));
            }
        }
    };

    // Collect remaining output
    let stderr_buf = stderr_thread.join().unwrap_or_default();
    let mut stdout_buf = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        use std::io::Read;
        out.read_to_end(&mut stdout_buf).ok();
    }

    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr_buf);
        let stdout = String::from_utf8_lossy(&stdout_buf);
        log::error!("Transcription script failed (exit {}). stderr: {} stdout: {}",
            status.code().unwrap_or(-1), stderr.trim(), stdout.trim());

        // Parse structured error from stdout if the script managed to output JSON
        // (stdout may contain multiple JSON lines — check each)
        for line in stdout.lines() {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(line.trim()) {
                if let Some(err_msg) = json.get("error").and_then(|e| e.as_str()) {
                    return Err(AppError::Transcription(err_msg.to_string()));
                }
            }
        }

        // Fall back to stderr content
        let err_str = stderr.trim();
        if err_str.contains("unrecognized arguments") {
            return Err(AppError::Transcription(
                "Transcription script version mismatch. Update transcribe.py from ai_engine/.".into()
            ));
        }
        return Err(AppError::Transcription(format!("Script failed: {err_str}")));
    }

    let json_str = std::fs::read_to_string(output_path)
        .map_err(|e| AppError::Transcription(format!("Failed to read transcript output: {e}")))?;

    serde_json::from_str::<TranscriptResult>(&json_str)
        .map_err(|e| AppError::Transcription(format!("Invalid transcript JSON: {e}")))
}

/// Generate an SRT subtitle file from transcript segments for a specific clip time range.
pub(crate) fn generate_srt_for_clip(
    transcript: &TranscriptResult,
    clip_start: f64,
    clip_end: f64,
    output_path: &std::path::Path,
) -> Result<(), String> {
    let mut srt = String::new();
    let mut index = 1;

    for seg in &transcript.segments {
        // Only include segments that overlap with clip range
        if seg.end < clip_start || seg.start > clip_end {
            continue;
        }

        // Use word-level timestamps if available for better timing
        if !seg.words.is_empty() {
            // Group words into subtitle chunks (max ~8 words per subtitle)
            let mut chunk_words: Vec<&TranscriptWord> = Vec::new();
            for word in &seg.words {
                if word.end < clip_start || word.start > clip_end {
                    continue;
                }
                chunk_words.push(word);

                if chunk_words.len() >= 6 {
                    // Emit subtitle
                    let start_time = (chunk_words[0].start - clip_start).max(0.0);
                    let end_time = (chunk_words.last().unwrap().end - clip_start).max(0.0);
                    let text: Vec<&str> = chunk_words.iter().map(|w| w.word.as_str()).collect();

                    srt.push_str(&format!("{}\n", index));
                    srt.push_str(&format!("{} --> {}\n", format_srt_time(start_time), format_srt_time(end_time)));
                    srt.push_str(&format!("{}\n\n", text.join(" ")));
                    index += 1;
                    chunk_words.clear();
                }
            }
            // Emit remaining words
            if !chunk_words.is_empty() {
                let start_time = (chunk_words[0].start - clip_start).max(0.0);
                let end_time = (chunk_words.last().unwrap().end - clip_start).max(0.0);
                let text: Vec<&str> = chunk_words.iter().map(|w| w.word.as_str()).collect();

                srt.push_str(&format!("{}\n", index));
                srt.push_str(&format!("{} --> {}\n", format_srt_time(start_time), format_srt_time(end_time)));
                srt.push_str(&format!("{}\n\n", text.join(" ")));
            }
        } else {
            // Fall back to segment-level timing
            let start_time = (seg.start - clip_start).max(0.0);
            let end_time = (seg.end - clip_start).max(0.0);

            srt.push_str(&format!("{}\n", index));
            srt.push_str(&format!("{} --> {}\n", format_srt_time(start_time), format_srt_time(end_time)));
            srt.push_str(&format!("{}\n\n", seg.text));
            index += 1;
        }
    }

    std::fs::write(output_path, srt).map_err(|e| format!("Failed to write SRT: {}", e))
}

fn format_srt_time(seconds: f64) -> String {
    let h = (seconds / 3600.0) as u32;
    let m = ((seconds % 3600.0) / 60.0) as u32;
    let s = (seconds % 60.0) as u32;
    let ms = ((seconds % 1.0) * 1000.0) as u32;
    format!("{:02}:{:02}:{:02},{:03}", h, m, s, ms)
}

/// Extract the full dialogue text from transcript segments that overlap a clip's time range.
/// Concatenates all segment text into a single string — used to save a richer
/// `transcript_snippet` in the highlights table so Claude gets more context.
fn extract_transcript_for_range(transcript: &TranscriptResult, start: f64, end: f64) -> Option<String> {
    let texts: Vec<&str> = transcript.segments.iter()
        .filter(|seg| seg.end >= start && seg.start <= end)
        .map(|seg| seg.text.as_str())
        .collect();
    if texts.is_empty() {
        return None;
    }
    Some(texts.join(" ").trim().to_string())
}

/// Boost virality score based on detected keywords in the transcript.
fn keyword_boost_for_range(transcript: &TranscriptResult, start: f64, end: f64) -> f64 {
    if transcript.keywords_found.is_empty() {
        return 0.0;
    }
    let keywords_in_range: Vec<&TranscriptKeyword> = transcript.keywords_found.iter()
        .filter(|kw| kw.timestamp >= start && kw.end_timestamp <= end)
        .collect();
    if keywords_in_range.is_empty() {
        return 0.0;
    }
    // Each keyword in the range boosts by 0.05, max 0.20
    (keywords_in_range.len() as f64 * 0.05).min(0.20)
}

/// Analyze a VOD to find highlight-worthy moments.
/// Uses local signal analysis (audio + transcript + chat) when ffmpeg + downloaded VOD are available.
/// Falls back to position heuristics otherwise. No external API calls are made.
#[tauri::command]
pub async fn analyze_vod(vod_id: String, app: AppHandle, db: State<'_, DbConn>, hw: State<'_, HardwareInfo>) -> Result<(), String> {
    // Atomic check-and-set: read status, validate, and update in a single lock scope
    let (vod, has_ffmpeg) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let vod = db::get_vod_by_id(&conn, &vod_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or_else(|| "VOD not found".to_string())?;

        if vod.analysis_status == "analyzing" {
            return Err("Analysis is already in progress.".to_string());
        }

        db::update_vod_analysis_status(&conn, &vod_id, "analyzing")
            .map_err(|e| format!("DB error: {}", e))?;
        db::update_vod_analysis_progress(&conn, &vod_id, 0)
            .map_err(|e| format!("DB error: {}", e))?;

        let has_ffmpeg = find_ffmpeg().is_ok();
        (vod, has_ffmpeg)
    };

    // Read sensitivity setting before moving into background task
    let sensitivity = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::get_setting(&conn, "detection_sensitivity")
            .ok()
            .flatten()
            .unwrap_or_else(|| "medium".to_string())
    };

    let vod_id_bg = vod_id.clone();
    let vod_clone = vod.clone();
    let hw_info = hw.inner().clone();

    // Run analysis in background
    tokio::task::spawn(async move {
        let db: State<'_, DbConn> = app.state();

        // Progress updates: run_analysis_signals handles 5-82% internally
        // via direct DB connection. We handle 0-5% and 82-100% here.
        if let Ok(conn) = db.lock() {
            db::update_vod_analysis_progress(&conn, &vod_id_bg, 2).ok();
        }

        // Cascading analysis: signal-driven (local) → position heuristic.
        let has_local_file = vod_clone.local_path.is_some();

        let mut result: Result<Vec<db::HighlightRow>, String> = Err("No analysis method available".into());

        // Tier 1: Signal-driven (audio + transcript + chat) — fully local
        if has_ffmpeg && has_local_file {
            log::info!("Running signal-driven analysis for VOD {}", vod_id_bg);
            let vod_for_sync = vod_clone.clone();
            let hw_for_sync = hw_info.clone();
            let sens = sensitivity.clone();
            match tokio::task::spawn_blocking(move || run_analysis_signals(&vod_for_sync, &hw_for_sync, &sens)).await {
                Ok(Ok(highlights)) => { result = Ok(highlights); }
                Ok(Err(e)) => {
                    log::warn!("Signal analysis failed, falling back to position heuristic: {e}");
                }
                Err(e) => {
                    log::warn!("Signal analysis task panicked, falling back: {e}");
                }
            }
        }

        // Tier 2: Position heuristic (always available)
        if result.is_err() {
            log::info!("Running position fallback for VOD {} (ffmpeg={}, downloaded={})",
                vod_id_bg, has_ffmpeg, has_local_file);
            if let Ok(conn) = db.lock() {
                db::update_vod_analysis_progress(&conn, &vod_id_bg, 10).ok();
            }
            let vod_for_sync = vod_clone.clone();
            match tokio::task::spawn_blocking(move || run_analysis(&vod_for_sync)).await {
                Ok(r) => { result = r; }
                Err(e) => { result = Err(format!("Task error: {e}")); }
            }
        };

        // Creating clips from highlights (82-88%)
        if let Ok(conn) = db.lock() {
            db::update_vod_analysis_progress(&conn, &vod_id_bg, 83).ok();
        }

        match result {
            Ok(highlights) => {
                let mut clip_thumb_info: Vec<(String, f64)> = Vec::new();

                if let Ok(conn) = db.lock() {
                    // Clear previous analysis
                    db::delete_clips_for_vod(&conn, &vod_id_bg).ok();
                    db::delete_highlights_for_vod(&conn, &vod_id_bg).ok();

                    let now = chrono::Utc::now().to_rfc3339();

                    for h in &highlights {
                        db::insert_highlight(&conn, h).ok();

                        // Create a clip for each highlight
                        let clip_id = uuid::Uuid::new_v4().to_string();

                        // Check if auto-captions SRT exists for this highlight
                        let captions_dir = dirs::data_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join("clipviral")
                            .join("captions");
                        let srt_path = captions_dir.join(format!("{}.srt", h.id));
                        let auto_captions = if srt_path.exists() {
                            // Read SRT content to use as captions_text
                            std::fs::read_to_string(&srt_path).ok()
                        } else {
                            None
                        };

                        let clip = db::ClipRow {
                            id: clip_id.clone(),
                            highlight_id: h.id.clone(),
                            vod_id: h.vod_id.clone(),
                            title: h.description.clone().unwrap_or_else(|| "Highlight".to_string()),
                            start_seconds: h.start_seconds,
                            end_seconds: h.end_seconds,
                            aspect_ratio: "9:16".to_string(),
                            crop_x: None,
                            crop_y: None,
                            crop_width: None,
                            crop_height: None,
                            captions_enabled: 1,
                            captions_text: auto_captions,
                            captions_position: "bottom".to_string(),
                            caption_style: "clean".to_string(),
                            facecam_layout: "none".to_string(),
                            render_status: "pending".to_string(),
                            output_path: None,
                            thumbnail_path: None,
                            created_at: now.clone(),
                            game: vod_clone.game_name.clone(),
                            publish_description: None,
                            publish_hashtags: None,
                        };
                        db::insert_clip(&conn, &clip).ok();

                        // Save auto-captions path to clip
                        if srt_path.exists() {
                            db::update_clip_auto_captions(&conn, &clip_id, &srt_path.to_string_lossy()).ok();
                        }

                        clip_thumb_info.push((clip_id, h.start_seconds));
                    }

                    db::update_vod_analysis_progress(&conn, &vod_id_bg, 88).ok();
                }
                // conn lock dropped here

                // Generate thumbnails outside DB lock (88-98%)
                if let Ok(ffmpeg_path) = find_ffmpeg() {
                    if let Some(ref vod_path) = vod_clone.local_path {
                        let thumb_dir = dirs::data_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join("clipviral")
                            .join("thumbnails");
                        std::fs::create_dir_all(&thumb_dir).ok();

                        if let Ok(thumb_conn) = db::db_path().and_then(|p| rusqlite::Connection::open(p).map_err(|e| e.to_string())) {
                            let total_thumbs = clip_thumb_info.len();
                            for (idx, (clip_id, start_secs)) in clip_thumb_info.iter().enumerate() {
                                let thumb_path = thumb_dir.join(format!("{}.jpg", clip_id));
                                let dur = vod_clone.duration_seconds as f64;
                                let candidates = [
                                    (start_secs + 2.0).min(dur),
                                    (start_secs + 10.0).min(dur),
                                    (start_secs + 5.0).min(dur),
                                    (*start_secs).max(1.0),
                                ];
                                let min_thumb_size = 3000u64;
                                let mut saved = false;
                                for ts in &candidates {
                                    if generate_thumbnail(&ffmpeg_path, vod_path, *ts, &thumb_path).is_ok() {
                                        let sz = std::fs::metadata(&thumb_path).map(|m| m.len()).unwrap_or(0);
                                        if sz >= min_thumb_size {
                                            db::update_clip_thumbnail(
                                                &thumb_conn, clip_id,
                                                Some(&thumb_path.to_string_lossy()),
                                            ).ok();
                                            saved = true;
                                            break;
                                        }
                                    }
                                }
                                if !saved {
                                    if thumb_path.exists() {
                                        db::update_clip_thumbnail(
                                            &thumb_conn, clip_id,
                                            Some(&thumb_path.to_string_lossy()),
                                        ).ok();
                                    }
                                }
                                // Update progress per thumbnail
                                if total_thumbs > 0 {
                                    let thumb_progress = 88 + ((idx + 1) as i64 * 10 / total_thumbs as i64);
                                    db::update_vod_analysis_progress(&thumb_conn, &vod_id_bg, thumb_progress).ok();
                                }
                            }
                        }
                    }
                }

                // Mark complete
                if let Ok(conn) = db.lock() {
                    db::update_vod_analysis_status(&conn, &vod_id_bg, "completed").ok();
                    db::update_vod_analysis_progress(&conn, &vod_id_bg, 100).ok();
                }
            }
            Err(e) => {
                log::error!("Analysis failed: {}", e);
                if let Ok(conn) = db.lock() {
                    db::update_vod_analysis_status(&conn, &vod_id_bg, "failed").ok();
                }
            }
        }
    });

    Ok(())
}

/// Helper: update analysis progress directly (opens its own DB connection).
/// Used inside `spawn_blocking` where the Tauri State DB isn't available.
fn set_analysis_progress(vod_id: &str, progress: i64) {
    if let Ok(path) = db::db_path() {
        if let Ok(conn) = rusqlite::Connection::open(path) {
            db::update_vod_analysis_progress(&conn, vod_id, progress).ok();
        }
    }
}

/// Signal-driven analysis using the clip_selector module.
/// Finds clips via audio spikes, transcript keywords, and chat peaks.
fn run_analysis_signals(vod: &db::VodRow, hw: &HardwareInfo, sensitivity: &str) -> Result<Vec<db::HighlightRow>, String> {
    let ffmpeg = find_ffmpeg()?;
    let vod_path = vod.local_path.clone()
        .ok_or("VOD not downloaded")?;
    let duration = vod.duration_seconds as f64;
    let vod_id = &vod.id;
    let now = chrono::Utc::now().to_rfc3339();

    // ── Stage 1: Audio analysis (5-15%) ──
    log::info!("Signal analysis: extracting audio profile...");
    set_analysis_progress(vod_id, 5);
    let audio_profile = analyze_audio_intensity(&vod_path, &ffmpeg).ok();
    let audio_ctx = audio_profile.as_ref().map(|a| {
        clip_selector::AudioContext::new(a.rms_per_second.clone(), a.spike_seconds.clone())
    });
    set_analysis_progress(vod_id, 15);

    // ── Stage 2: Transcription (15-40%) ──
    log::info!("Signal analysis: attempting transcription...");
    set_analysis_progress(vod_id, 18);
    let transcript_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clipviral")
        .join("transcripts");
    std::fs::create_dir_all(&transcript_dir).ok();
    let transcript_path = transcript_dir.join(format!("{}.json", vod_id));
    let transcript: Option<TranscriptResult> = if transcript_path.exists() {
        log::info!("Signal analysis: loading cached transcript");
        set_analysis_progress(vod_id, 25);
        std::fs::read_to_string(&transcript_path).ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    } else if let Ok(_python) = find_python() {
        set_analysis_progress(vod_id, 20);
        let out = transcript_path.to_string_lossy().to_string();
        let result = run_transcription(&vod_path, &out, hw, Some(vod_id)).ok();
        set_analysis_progress(vod_id, 38);
        result
    } else {
        None
    };
    set_analysis_progress(vod_id, 40);

    // ── Stage 3: Chat analysis (40-50%) ──
    log::info!("Signal analysis: analyzing chat activity...");
    set_analysis_progress(vod_id, 42);
    let chat_peaks: Vec<db::HighlightRow> = analyze_via_chat(vod).unwrap_or_default();
    set_analysis_progress(vod_id, 50);

    // ── Stage 4: Clip selection pipeline (50-65%) ──
    log::info!("Signal analysis: running clip selector pipeline...");
    set_analysis_progress(vod_id, 52);
    let (selected, detection_stats): (Vec<clip_selector::ClipCandidate>, _) = clip_selector::select_clips(
        audio_ctx.as_ref(),
        transcript.as_ref(),
        &chat_peaks,
        duration,
        sensitivity,
    );
    set_analysis_progress(vod_id, 60);

    // Persist detection stats for the VOD page to display
    if let Ok(db_path) = db::db_path() {
        if let Ok(conn) = rusqlite::Connection::open(&db_path) {
            let stats_json = serde_json::to_string(&detection_stats).unwrap_or_default();
            db::save_setting(&conn, &format!("detection_stats_{}", vod_id), &stats_json).ok();
        }
    }

    if selected.is_empty() {
        log::warn!("Signal analysis: selector returned no clips, falling back to position heuristic");
        set_analysis_progress(vod_id, 55);
        return run_analysis(vod);
    }

    // ── Stage 5: Scoring and ranking (60-75%) ──
    log::info!("Signal analysis: scoring {} candidates...", selected.len());
    set_analysis_progress(vod_id, 62);
    let mut highlights: Vec<db::HighlightRow> = Vec::new();
    let total_candidates = selected.len();

    for (i, c) in selected.iter().enumerate() {
        let all_tags: Vec<String> = [&c.event_tags[..], &c.emotion_tags[..]].concat();
        let tag_str = if all_tags.is_empty() { "auto".to_string() } else { all_tags.join(",") };

        let title = grounded_highlight_title(
            c.transcript_excerpt.as_deref(),
            Some(&tag_str),
            c.start_time,
        );

        let kw_boost = if let Some(ref t) = transcript {
            keyword_boost_for_range(t, c.start_time, c.end_time)
        } else {
            0.0
        };

        let raw_score = (c.total_score + kw_boost).min(0.99);
        let audio = c.hook_strength;
        let visual = c.emotional_spike;
        let chat = if c.signal_sources.contains(&clip_selector::SignalSource::Chat) {
            c.event_reaction_alignment
        } else { 0.0 };
        let has_transcript = c.transcript_excerpt.is_some();
        let sig_count = count_active_signals(audio, visual, chat, has_transcript);

        let event_summary = crate::post_captions::generate_event_summary_from_parts(
            &all_tags,
            c.transcript_excerpt.as_deref(),
            audio, visual, 0.0, c.start_time,
        );

        // Use full transcript for the clip range if available; fall back to
        // the single-sentence excerpt from signal fusion.
        let full_range_transcript = transcript.as_ref()
            .and_then(|t| extract_transcript_for_range(t, c.start_time, c.end_time))
            .or_else(|| c.transcript_excerpt.clone());

        highlights.push(db::HighlightRow {
            id: uuid::Uuid::new_v4().to_string(),
            vod_id: vod_id.clone(),
            start_seconds: c.start_time,
            end_seconds: c.end_time,
            virality_score: raw_score,
            audio_score: audio,
            visual_score: visual,
            chat_score: chat,
            transcript_snippet: full_range_transcript,
            description: Some(title),
            tags: Some(tag_str),
            thumbnail_path: None,
            created_at: now.clone(),
            confidence_score: Some(compute_confidence(raw_score, sig_count)),
            explanation: Some(build_highlight_explanation(audio, visual, chat, has_transcript)),
            event_summary: Some(event_summary),
        });

        // Update progress within scoring loop
        let scoring_progress = 62 + ((i + 1) as i64 * 13 / total_candidates as i64);
        set_analysis_progress(vod_id, scoring_progress);
    }
    set_analysis_progress(vod_id, 75);

    // ── Stage 6: Generate captions (75-82%) ──
    log::info!("Signal analysis: generating captions...");
    set_analysis_progress(vod_id, 76);
    if let Some(ref t) = transcript {
        let captions_dir = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("clipviral")
            .join("captions");
        std::fs::create_dir_all(&captions_dir).ok();

        let mut srt_count = 0;
        for h in &highlights {
            let srt_path = captions_dir.join(format!("{}.srt", h.id));
            if generate_srt_for_clip(t, h.start_seconds, h.end_seconds, &srt_path).is_ok() {
                srt_count += 1;
            }
        }
        if srt_count > 0 {
            log::info!("Signal analysis: generated {} SRT caption files", srt_count);
        }
    }
    set_analysis_progress(vod_id, 82);

    log::info!("Signal analysis: produced {} final clips", highlights.len());
    Ok(highlights)
}

/// Position-based fallback — last resort when no signals are available.
/// Only used when VOD is not downloaded or ffmpeg is missing.
fn run_analysis(vod: &db::VodRow) -> Result<Vec<db::HighlightRow>, String> {
    let duration = vod.duration_seconds as f64;
    let vod_id = &vod.id;
    let now = chrono::Utc::now().to_rfc3339();

    // Try chat-based analysis first
    if let Ok(chat_highlights) = analyze_via_chat(vod) {
        if !chat_highlights.is_empty() {
            return Ok(chat_highlights);
        }
    }

    // Fallback: duration-based heuristic analysis
    let mut highlights = Vec::new();

    if duration <= 60.0 {
        highlights.push(db::HighlightRow {
            id: uuid::Uuid::new_v4().to_string(),
            vod_id: vod_id.clone(),
            start_seconds: 0.0,
            end_seconds: duration,
            virality_score: 0.75,
            audio_score: 0.7,
            visual_score: 0.7,
            chat_score: 0.5,
            transcript_snippet: None,
            description: Some(format!("Full clip at 0:00")),
            tags: Some("full,highlight".to_string()),
            thumbnail_path: None,
            created_at: now.clone(),
            confidence_score: Some(compute_confidence(0.75, 0)),
            explanation: Some("Position-based estimate, no signal analysis".into()),
            event_summary: None,
        });
    } else {
        let clip_duration = 30.0_f64.min(duration * 0.15);
        let positions: Vec<(f64, f64)> = if duration < 300.0 {
            vec![(0.05, 0.85), (0.45, 0.78), (0.80, 0.82)]
        } else {
            vec![(0.03, 0.80), (0.20, 0.75), (0.40, 0.82), (0.60, 0.78), (0.85, 0.88)]
        };

        for (frac, score) in positions {
            let start = (duration * frac).max(0.0);
            let end = (start + clip_duration).min(duration);
            if end - start < 5.0 { continue; }

            let mins = (start as u32) / 60;
            let secs = (start as u32) % 60;

            highlights.push(db::HighlightRow {
                id: uuid::Uuid::new_v4().to_string(),
                vod_id: vod_id.clone(),
                start_seconds: start,
                end_seconds: end,
                virality_score: score,
                audio_score: score * 0.9,
                visual_score: score * 0.95,
                chat_score: 0.5,
                transcript_snippet: None,
                description: Some(format!("Estimated highlight at {}:{:02}", mins, secs)),
                tags: Some("auto,estimated".to_string()),
                thumbnail_path: None,
                created_at: now.clone(),
                confidence_score: Some(compute_confidence(score, 0)),
                explanation: Some("Position-based estimate, no signal analysis".into()),
                event_summary: None,
            });
        }
    }

    Ok(highlights)
}

/// Try to analyze a VOD using Twitch chat replay (downloaded via yt-dlp).
fn analyze_via_chat(vod: &db::VodRow) -> Result<Vec<db::HighlightRow>, String> {
    let ytdlp = find_ytdlp()?;
    let temp_dir = std::env::temp_dir().join("clipviral_chat");
    std::fs::create_dir_all(&temp_dir).ok();

    let out_template = temp_dir.join(&vod.twitch_video_id).to_string_lossy().to_string();

    let mut cmd = std::process::Command::new(&ytdlp);
    cmd.arg("--write-subs")
        .arg("--sub-lang").arg("live_chat")
        .arg("--skip-download")
        .arg("--no-warnings")
        .arg("-o").arg(&out_template)
        .arg(&vod.vod_url)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let status = cmd.status().map_err(|e| format!("yt-dlp chat: {}", e))?;
    if !status.success() {
        return Err("Chat download failed".to_string());
    }

    let chat_path = temp_dir.join(format!("{}.live_chat.json", vod.twitch_video_id));
    if !chat_path.exists() {
        return Err("No chat file found".to_string());
    }

    let content = std::fs::read_to_string(&chat_path).map_err(|e| format!("Read chat: {}", e))?;
    let duration = vod.duration_seconds as f64;
    let window_size = 30.0_f64.max(duration * 0.05);

    let num_windows = ((duration / window_size).ceil() as usize).max(1);
    let mut window_counts = vec![0u32; num_windows];
    let mut total_messages = 0u32;

    for line in content.lines() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            let offset = val.get("time_in_seconds")
                .or_else(|| val.get("content_offset_seconds"))
                .and_then(|v| v.as_f64());
            if let Some(t) = offset {
                let idx = ((t / window_size) as usize).min(num_windows - 1);
                window_counts[idx] += 1;
                total_messages += 1;
            }
        }
    }

    std::fs::remove_file(&chat_path).ok();

    if total_messages < 5 {
        return Err("Not enough chat data".to_string());
    }

    let avg = total_messages as f64 / num_windows as f64;
    let mut peaks: Vec<(usize, u32)> = window_counts.iter().enumerate()
        .filter(|(_, &count)| count as f64 > avg * 1.3)
        .map(|(i, &count)| (i, count))
        .collect();
    peaks.sort_by(|a, b| b.1.cmp(&a.1));
    peaks.truncate(5);

    if peaks.is_empty() {
        return Err("No engagement peaks found".to_string());
    }

    let max_count = peaks[0].1 as f64;
    let now = chrono::Utc::now().to_rfc3339();
    let mut highlights = Vec::new();

    for (idx, count) in &peaks {
        let start = (*idx as f64 * window_size).max(0.0);
        let end = (start + window_size).min(duration);
        let chat_score = *count as f64 / max_count;
        let virality = 0.5 + chat_score * 0.45;

        let mins = (start as u32) / 60;
        let secs = (start as u32) % 60;
        highlights.push(db::HighlightRow {
            id: uuid::Uuid::new_v4().to_string(),
            vod_id: vod.id.clone(),
            start_seconds: start,
            end_seconds: end,
            virality_score: virality,
            audio_score: virality * 0.9,
            visual_score: virality * 0.85,
            chat_score,
            transcript_snippet: Some(format!("{} chat messages in this window", count)),
            description: Some(format!("Chat spike ({} msgs) at {}:{:02}", count, mins, secs)),
            tags: Some("chat-peak,reaction,auto".to_string()),
            thumbnail_path: None,
            created_at: now.clone(),
            confidence_score: Some(compute_confidence(virality, 1)),
            explanation: Some(format!("1 signal — chat {:.0}% ({} messages)", chat_score * 100.0, count)),
            event_summary: Some(format!("chat went off with {} messages", count)),
        });
    }

    Ok(highlights)
}

// ── VOD info / list commands ──

#[tauri::command]
pub async fn open_vod(vod_id: String, app: AppHandle, db: State<'_, DbConn>) -> Result<(), String> {
    let vod = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::get_vod_by_id(&conn, &vod_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or_else(|| "VOD not found".to_string())?
    };

    // Validate URL before opening to prevent arbitrary URL injection
    if !vod.vod_url.starts_with("https://www.twitch.tv/")
        && !vod.vod_url.starts_with("https://twitch.tv/")
    {
        return Err(format!("Refusing to open non-Twitch URL: {}", vod.vod_url));
    }

    app.opener()
        .open_url(&vod.vod_url, None::<&str>)
        .map_err(|e| format!("Failed to open browser: {}", e))?;

    Ok(())
}

#[tauri::command]
pub async fn get_vods(
    channel_id: String,
    db: State<'_, DbConn>,
) -> Result<Vec<db::VodRow>, String> {
    log::info!("[get_vods] called for channel_id={}", channel_id);
    let (twitch_user_id, mut access_token) = {
        let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
        let channels = db::get_all_channels(&conn).map_err(|e| format!("DB error: {}", e))?;
        let channel = channels
            .iter()
            .find(|c| c.id == channel_id)
            .ok_or_else(|| "Channel not found".to_string())?
            .clone();
        let token = db::get_setting(&conn, "twitch_user_access_token")
            .map_err(|e| format!("DB error: {}", e))?
            .unwrap_or_default();
        (channel.twitch_user_id, token)
    };

    if access_token.is_empty() {
        log::warn!("[get_vods] No access token found — user not logged in");
        return Err("Not logged in. Please log in with Twitch first.".into());
    }

    log::info!("[get_vods] Fetching VODs for twitch_user_id={}, token_len={}", twitch_user_id, access_token.len());

    // Try fetching VODs; if 401, refresh token and retry
    let videos = match twitch::get_vods(&access_token, &twitch_user_id).await {
        Ok(v) => {
            log::info!("[get_vods] Twitch API returned {} videos", v.len());
            v
        }
        Err(e) if e.contains("401") => {
            log::warn!("[get_vods] Got 401, refreshing token and retrying");
            access_token = try_refresh_twitch_token(&db).await?;
            twitch::get_vods(&access_token, &twitch_user_id).await?
        }
        Err(e) => {
            log::error!("[get_vods] Twitch API error: {}", e);
            return Err(e);
        }
    };

    // NOTE: The Twitch /videos endpoint does NOT return game_id/game_name, and the
    // /channels endpoint only returns the CURRENT game (not the game played during a specific VOD).
    // Game detection is handled at the clip level via subtitle keyword inference (detectGame()),
    // or manually by the user via the "Set game" button on VOD cards.

    let vod_rows: Vec<db::VodRow> = videos
        .iter()
        .map(|v| {
            let vod_id = uuid::Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            db::VodRow {
                id: vod_id,
                channel_id: channel_id.clone(),
                twitch_video_id: v.id.clone(),
                title: v.title.clone(),
                duration_seconds: twitch::parse_duration(&v.duration),
                stream_date: v.created_at.clone(),
                thumbnail_url: v.thumbnail_url
                    .replace("%{width}", "640")
                    .replace("%{height}", "360"),
                vod_url: v.url.clone(),
                download_status: "pending".to_string(),
                local_path: None,
                file_size_bytes: None,
                analysis_status: "pending".to_string(),
                created_at: now,
                download_progress: Some(0),
                analysis_progress: 0,
                game_name: None,
            }
        })
        .collect();

    {
        let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
        let mut upsert_count = 0;
        for vod in &vod_rows {
            db::upsert_vod(&conn, vod).map_err(|e| format!("DB error: {}", e))?;
            upsert_count += 1;
        }
        log::info!("[get_vods] Upserted {} VODs to database for channel_id={}", upsert_count, channel_id);
    }

    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    let result = db::get_vods_by_channel(&conn, &channel_id).map_err(|e| format!("DB error: {}", e))?;
    log::info!("[get_vods] get_vods_by_channel returned {} VODs", result.len());
    Ok(result)
}

#[tauri::command]
pub fn get_highlights(
    vod_id: String,
    db: State<'_, DbConn>,
) -> Result<Vec<db::HighlightRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_highlights_by_vod(&conn, &vod_id).map_err(|e| format!("DB error: {}", e))
}

#[tauri::command]
pub fn get_all_highlights(db: State<'_, DbConn>) -> Result<Vec<db::HighlightRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_all_highlights(&conn).map_err(|e| format!("DB error: {}", e))
}

#[tauri::command]
pub fn get_clips(db: State<'_, DbConn>) -> Result<Vec<db::ClipRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_all_clips(&conn).map_err(|e| format!("DB error: {}", e))
}

#[tauri::command]
pub fn delete_clip(clip_id: String, db: State<'_, DbConn>) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;

    // Get the vod_id before deleting so we can check remaining clips
    let vod_id: Option<String> = conn.query_row(
        "SELECT vod_id FROM clips WHERE id = ?1", rusqlite::params![clip_id],
        |row| row.get(0),
    ).ok();

    db::delete_clip(&conn, &clip_id).map_err(|e| format!("DB error: {}", e))?;

    // If no clips remain for this VOD, reset analysis_status so user can re-analyze
    if let Some(vid) = vod_id {
        let remaining: i64 = conn.query_row(
            "SELECT COUNT(*) FROM clips WHERE vod_id = ?1", rusqlite::params![vid],
            |row| row.get(0),
        ).unwrap_or(0);

        if remaining == 0 {
            db::update_vod_analysis_status(&conn, &vid, "pending")
                .map_err(|e| format!("DB error: {}", e))?;
            log::info!("All clips deleted for VOD {} — reset analysis_status to pending", vid);
        }
    }

    Ok(())
}

/// Refresh VOD metadata from Twitch API (title, thumbnail, game) without re-downloading.
/// Also backfills game_name to existing clips that don't have one.
#[tauri::command]
pub async fn refresh_vod_metadata(
    vod_id: String,
    db: State<'_, DbConn>,
) -> Result<db::VodRow, String> {
    let (twitch_video_id, mut access_token) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let vod = db::get_vod_by_id(&conn, &vod_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("VOD not found")?;
        let token = db::get_setting(&conn, "twitch_user_access_token")
            .map_err(|e| format!("DB error: {}", e))?
            .unwrap_or_default();
        (vod.twitch_video_id, token)
    };

    if access_token.is_empty() {
        return Err("Not logged in. Please log in with Twitch first.".into());
    }

    // Fetch fresh video data from Twitch — retry with refreshed token on 401
    let client = reqwest::Client::new();
    let url = format!("https://api.twitch.tv/helix/videos?id={}", twitch_video_id);
    let resp = client
        .get(&url)
        .header("Client-Id", twitch::client_id())
        .header("Authorization", format!("Bearer {}", &access_token))
        .send()
        .await
        .map_err(|e| format!("Twitch API error: {}", e))?;

    let resp = if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        // Token expired — try refreshing
        access_token = try_refresh_twitch_token(&db).await?;
        client
            .get(&url)
            .header("Client-Id", twitch::client_id())
            .header("Authorization", format!("Bearer {}", &access_token))
            .send()
            .await
            .map_err(|e| format!("Twitch API error: {}", e))?
    } else {
        resp
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Twitch API {}: {}", status, body));
    }

    let resp_json: serde_json::Value = resp.json().await
        .map_err(|e| format!("Parse error: {}", e))?;

    let video = resp_json["data"].as_array()
        .and_then(|arr| arr.first())
        .ok_or("Video not found on Twitch")?;

    let title = video["title"].as_str().unwrap_or("").to_string();
    let thumbnail_url = video["thumbnail_url"].as_str().unwrap_or("")
        .replace("%{width}", "640")
        .replace("%{height}", "360");

    // Update VOD title and thumbnail in database.
    // Preserve game_name — it's user-set and should not be cleared on metadata refresh.
    {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        conn.execute(
            "UPDATE vods SET title = ?1, thumbnail_url = ?2 WHERE id = ?3",
            rusqlite::params![title, thumbnail_url, vod_id],
        ).map_err(|e| format!("DB error: {}", e))?;

        log::info!("[refresh_vod_metadata] Updated title/thumbnail for VOD {}", vod_id);
    }

    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::get_vod_by_id(&conn, &vod_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or_else(|| "VOD not found after update".to_string())
}

/// Set the game on a single clip (lightweight — used for auto-save after subtitle inference).
#[tauri::command]
pub fn set_clip_game(clip_id: String, game: Option<String>, db: State<'_, DbConn>) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let g = game.as_deref().filter(|s| !s.is_empty());
    log::info!("[set_clip_game] Setting clip {} game to: {:?}", clip_id, g);
    db::update_clip_game(&conn, &clip_id, g)
        .map_err(|e| format!("DB error: {}", e))
}

/// Set the title on a single clip (lightweight — used for auto-save on blur).
#[tauri::command]
pub fn set_clip_title(clip_id: String, title: Option<String>, db: State<'_, DbConn>) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let t = title.as_deref().filter(|s| !s.is_empty());
    log::info!("[set_clip_title] Setting clip {} title to: {:?}", clip_id, t);
    db::update_clip_title(&conn, &clip_id, t)
        .map_err(|e| format!("DB error: {}", e))
}

/// Save publish description and hashtags on a clip (auto-save on blur / after generation).
#[tauri::command]
pub fn set_clip_publish_meta(
    clip_id: String,
    description: Option<String>,
    hashtags: Option<String>,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let desc = description.as_deref().filter(|s| !s.is_empty());
    let tags = hashtags.as_deref().filter(|s| !s.is_empty());
    log::info!("[set_clip_publish_meta] clip {} desc_len={:?} tags={:?}", clip_id, desc.map(|d| d.len()), tags);
    if let Some(d) = desc {
        println!("[CLIPGOBLIN DEBUG] Publish description saved: \"{}\"", d);
    }
    db::update_clip_publish_meta(&conn, &clip_id, desc, tags)
        .map_err(|e| format!("DB error: {}", e))
}

/// Manually set the game name on a VOD and propagate to all its clips.
/// Used as a manual fallback when auto-detection doesn't work.
#[tauri::command]
pub fn set_vod_game(vod_id: String, game_name: Option<String>, db: State<'_, DbConn>) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let gn = game_name.as_deref().filter(|s| !s.is_empty());
    log::info!("[set_vod_game] Setting VOD {} game to: {:?}", vod_id, gn);
    db::update_vod_game_name(&conn, &vod_id, gn)
        .map_err(|e| format!("DB error: {}", e))?;
    // Propagate to all clips from this VOD (overwrite all, since user explicitly set it)
    if let Some(name) = gn {
        conn.execute(
            "UPDATE clips SET game = ?1 WHERE vod_id = ?2",
            rusqlite::params![name, vod_id],
        ).map_err(|e| format!("DB error: {}", e))?;
        log::info!("[set_vod_game] Propagated game to all clips for VOD {}", vod_id);
    } else {
        conn.execute(
            "UPDATE clips SET game = NULL WHERE vod_id = ?1",
            rusqlite::params![vod_id],
        ).map_err(|e| format!("DB error: {}", e))?;
    }
    Ok(())
}

/// Delete a VOD's video file only (keeps clips and metadata)
/// Returns how many bytes were freed.
#[tauri::command]
pub fn delete_vod_file(vod_id: String, db: State<'_, DbConn>) -> Result<u64, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let vod = db::get_vod_by_id(&conn, &vod_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or("VOD not found")?;

    let mut freed: u64 = 0;
    if let Some(ref path) = vod.local_path {
        let p = std::path::Path::new(path);
        if p.exists() {
            freed = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            std::fs::remove_file(p).map_err(|e| format!("Failed to delete file: {}", e))?;
        }
    }

    // Update VOD status back to pending
    conn.execute(
        "UPDATE vods SET download_status = 'pending', local_path = NULL, file_size_bytes = NULL, download_progress = 0 WHERE id = ?1",
        rusqlite::params![vod_id],
    ).map_err(|e| format!("DB error: {}", e))?;

    Ok(freed)
}

/// Delete a VOD and ALL its associated clips, highlights, and files.
/// Returns how many bytes were freed.
#[tauri::command]
pub fn delete_vod_and_clips(vod_id: String, db: State<'_, DbConn>) -> Result<u64, String> {
    println!("[delete_vod_and_clips] START vod_id={}", vod_id);
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let vod = db::get_vod_by_id(&conn, &vod_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or("VOD not found")?;
    println!("[delete_vod_and_clips] Found VOD: twitch_video_id={}", vod.twitch_video_id);

    let mut freed: u64 = 0;

    // Delete VOD video file
    if let Some(ref path) = vod.local_path {
        let p = std::path::Path::new(path);
        if p.exists() {
            freed += std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            std::fs::remove_file(p).ok();
        }
    }

    // Delete exported clip files
    let clips = db::get_clips_by_vod(&conn, &vod_id).unwrap_or_default();
    for clip in &clips {
        if let Some(ref path) = clip.output_path {
            let p = std::path::Path::new(path);
            if p.exists() {
                freed += std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
                std::fs::remove_file(p).ok();
            }
        }
        if let Some(ref path) = clip.thumbnail_path {
            let p = std::path::Path::new(path);
            if p.exists() {
                freed += std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
                std::fs::remove_file(p).ok();
            }
        }
    }

    // Delete DB records: clips, highlights, then vod
    db::delete_clips_for_vod(&conn, &vod_id).ok();
    conn.execute(
        "DELETE FROM highlights WHERE vod_id = ?1",
        rusqlite::params![vod_id],
    ).ok();
    db::delete_vod(&conn, &vod_id)
        .map_err(|e| format!("DB error deleting vod: {}", e))?;

    // Verify the VOD is gone and the twitch_video_id is in deleted_vods
    let still_exists = db::get_vod_by_id(&conn, &vod_id).ok().flatten().is_some();
    println!("[delete_vod_and_clips] DONE vod_id={} freed={} still_in_db={}", vod_id, freed, still_exists);

    Ok(freed)
}

/// Get VOD disk usage info (for delete confirmation dialog).
#[tauri::command]
pub fn get_vod_disk_usage(vod_id: String, db: State<'_, DbConn>) -> Result<serde_json::Value, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let vod = db::get_vod_by_id(&conn, &vod_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or("VOD not found")?;

    let vod_size: u64 = vod.local_path.as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .unwrap_or(0);

    let clips = db::get_clips_by_vod(&conn, &vod_id).unwrap_or_default();
    let clip_count = clips.len();
    let mut clips_size: u64 = 0;
    for clip in &clips {
        if let Some(ref p) = clip.output_path {
            clips_size += std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        }
        if let Some(ref p) = clip.thumbnail_path {
            clips_size += std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        }
    }

    Ok(serde_json::json!({
        "vod_size": vod_size,
        "clip_count": clip_count,
        "clips_size": clips_size,
        "total_size": vod_size + clips_size,
        "has_file": vod.local_path.is_some(),
    }))
}

/// Get a single VOD's details by ID.
#[tauri::command]
pub fn get_vod_detail(vod_id: String, db: State<'_, DbConn>) -> Result<db::VodRow, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::get_vod_by_id(&conn, &vod_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or_else(|| "VOD not found".to_string())
}

/// Set a VOD's analysis status (used by frontend to mark stale analyses as failed).
#[tauri::command]
pub fn set_vod_analysis_status(vod_id: String, status: String, db: State<'_, DbConn>) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::update_vod_analysis_status(&conn, &vod_id, &status)
        .map_err(|e| format!("DB error: {}", e))
}

/// Save clip performance metrics for analytics.
#[tauri::command]
pub fn save_clip_performance(
    clip_id: String,
    platform: String,
    views: i64,
    likes: i64,
    comments: i64,
    shares: i64,
    retention_rate: f64,
    first_3s_hold_rate: f64,
    completion_rate: f64,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::insert_clip_performance(
        &conn, &clip_id, &platform, views, likes, comments, shares,
        retention_rate, first_3s_hold_rate, completion_rate,
    ).map_err(|e| format!("DB error: {}", e))
}

/// Get clip performance data by clip ID.
#[tauri::command]
pub fn get_clip_performance(clip_id: String, db: State<'_, DbConn>) -> Result<Vec<db::ClipPerformanceRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::get_clip_performance(&conn, &clip_id).map_err(|e| format!("DB error: {}", e))
}

/// Get or create the creator's scoring profile.
#[tauri::command]
pub fn get_creator_profile(db: State<'_, DbConn>) -> Result<db::CreatorProfileRow, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::get_or_create_creator_profile(&conn).map_err(|e| format!("DB error: {}", e))
}

/// Recalculate creator scoring weights based on actual clip performance data.
/// This is the feedback loop — learn what works for this creator.
#[tauri::command]
pub fn update_scoring_from_performance(db: State<'_, DbConn>) -> Result<String, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let mut profile = db::get_or_create_creator_profile(&conn)
        .map_err(|e| format!("DB error: {}", e))?;

    // Get all clips with performance data
    let mut stmt = conn.prepare(
        "SELECT c.id, h.virality_score, h.audio_score, h.visual_score, h.chat_score, h.tags,
                p.retention_rate, p.first_3s_hold_rate, p.completion_rate, p.views, p.shares
         FROM clips c
         JOIN highlights h ON h.id = c.highlight_id
         JOIN clip_performance p ON p.clip_id = c.id
         WHERE p.views > 0
         ORDER BY p.retention_rate DESC"
    ).map_err(|e| format!("DB error: {}", e))?;

    let perf_data: Vec<(f64, f64, f64, f64, String, f64, f64, f64, i64, i64)> = stmt.query_map([], |row| {
        Ok((
            row.get::<_, f64>(1)?,  // virality
            row.get::<_, f64>(2)?,  // audio
            row.get::<_, f64>(3)?,  // visual
            row.get::<_, f64>(4)?,  // chat
            row.get::<_, String>(5).unwrap_or_default(),  // tags
            row.get::<_, f64>(6)?,  // retention
            row.get::<_, f64>(7)?,  // 3s hold
            row.get::<_, f64>(8)?,  // completion
            row.get::<_, i64>(9)?,  // views
            row.get::<_, i64>(10)?, // shares
        ))
    }).map_err(|e| format!("DB error: {}", e))?
    .filter_map(|r| r.ok())
    .collect();

    if perf_data.len() < 3 {
        return Ok("Not enough performance data yet (need at least 3 clips with metrics). Keep creating and tracking clips!".to_string());
    }

    // Calculate which clips performed best (top quartile)
    let top_count = (perf_data.len() / 4).max(1);
    let top_clips = &perf_data[..top_count];

    // Analyze what scores the best performers had
    let avg_3s_hold: f64 = top_clips.iter().map(|d| d.6).sum::<f64>() / top_count as f64;

    // Adjust weights: if top clips had high 3s hold rate, increase hook weight
    if avg_3s_hold > 0.7 {
        profile.avg_hook_weight = (profile.avg_hook_weight + 0.02).min(0.40);
        profile.avg_context_weight = (profile.avg_context_weight - 0.01).max(0.05);
    }

    // If top clips had high completion, boost payoff weight
    let avg_completion: f64 = top_clips.iter().map(|d| d.7).sum::<f64>() / top_count as f64;
    if avg_completion > 0.6 {
        profile.avg_payoff_weight = (profile.avg_payoff_weight + 0.02).min(0.30);
        profile.avg_loop_weight = (profile.avg_loop_weight - 0.01).max(0.05);
    }

    // Collect top-performing tags
    let mut tag_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for clip in top_clips {
        for tag in clip.4.split(',') {
            let tag = tag.trim().to_string();
            if !tag.is_empty() {
                *tag_counts.entry(tag).or_insert(0) += 1;
            }
        }
    }
    let mut sorted_tags: Vec<(String, usize)> = tag_counts.into_iter().collect();
    sorted_tags.sort_by(|a, b| b.1.cmp(&a.1));
    let top_tags: Vec<String> = sorted_tags.iter().take(10).map(|(t, _)| t.clone()).collect();
    profile.top_performing_tags = Some(top_tags.join(","));

    profile.total_clips_tracked = perf_data.len() as i64;

    // Normalize weights to sum to 1.0
    let sum = profile.avg_hook_weight + profile.avg_emotional_weight + profile.avg_payoff_weight
        + profile.avg_loop_weight + profile.avg_context_weight;
    profile.avg_hook_weight /= sum;
    profile.avg_emotional_weight /= sum;
    profile.avg_payoff_weight /= sum;
    profile.avg_loop_weight /= sum;
    profile.avg_context_weight /= sum;

    db::update_creator_profile(&conn, &profile)
        .map_err(|e| format!("DB error: {}", e))?;

    Ok(format!(
        "Scoring weights updated from {} clips! Hook: {:.0}%, Emotional: {:.0}%, Payoff: {:.0}%, Loop: {:.0}%, Context: {:.0}%. Top tags: {}",
        perf_data.len(),
        profile.avg_hook_weight * 100.0,
        profile.avg_emotional_weight * 100.0,
        profile.avg_payoff_weight * 100.0,
        profile.avg_loop_weight * 100.0,
        profile.avg_context_weight * 100.0,
        profile.top_performing_tags.as_deref().unwrap_or("none yet"),
    ))
}

/// Get transcript for a VOD (run transcription if not cached)
#[tauri::command]
pub async fn get_transcript(vod_id: String, db: State<'_, DbConn>, hw: State<'_, HardwareInfo>) -> Result<serde_json::Value, String> {
    let vod = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::get_vod_by_id(&conn, &vod_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("VOD not found")?
    };

    let vod_path = vod.local_path.ok_or("VOD not downloaded")?;

    let transcript_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clipviral")
        .join("transcripts");
    std::fs::create_dir_all(&transcript_dir).ok();
    let output_path = transcript_dir.join(format!("{}.json", vod_id));

    // Return cached transcript if it exists
    if output_path.exists() {
        let json_str = std::fs::read_to_string(&output_path)
            .map_err(|e| format!("Read error: {}", e))?;
        let val: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| format!("Parse error: {}", e))?;
        return Ok(val);
    }

    // Run transcription
    let output_str = output_path.to_string_lossy().to_string();
    let vod_path_clone = vod_path.clone();
    let hw_clone = hw.inner().clone();
    let result = tokio::task::spawn_blocking(move || {
        run_transcription(&vod_path_clone, &output_str, &hw_clone, None)
    }).await.map_err(|e| format!("Task error: {}", e))??;

    // Save path to VOD record
    {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::update_vod_transcript_path(&conn, &vod_id, &output_path.to_string_lossy()).ok();
    }

    serde_json::to_value(&result).map_err(|e| format!("Serialize: {}", e))
}
