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
use crate::whisper;
use crate::commands::captions::{
    compute_confidence,
    build_highlight_explanation, count_active_signals,
};

// â”€â”€ AudioProfile struct (local to this module) â”€â”€

/// Audio profile extracted from a video file.
#[derive(Debug, Clone)]
struct AudioProfile {
    /// RMS volume level per second (0.0 = silence, 1.0 = max)
    rms_per_second: Vec<f64>,
    /// Indices of detected volume spikes (>1.5x average)
    spike_seconds: Vec<usize>,
}

// â”€â”€ Tool finders â”€â”€

/// Find yt-dlp executable. Delegates to bin_manager (bundled → PATH).
fn find_ytdlp() -> Result<std::path::PathBuf, AppError> {
    crate::bin_manager::ytdlp_path()
}

/// Find ffmpeg executable. Delegates to bin_manager (bundled → PATH).
pub(crate) fn find_ffmpeg() -> Result<std::path::PathBuf, AppError> {
    crate::bin_manager::ffmpeg_path()
}

// â”€â”€ Download helpers â”€â”€

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

    // Spawn background task â€” returns immediately so UI stays responsive
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
///
/// Returns EVERY VOD in the library, including ones imported via
/// `import_vod_by_url` (which live under stub channels). The `channel_id` param
/// is retained for API compatibility but no longer filters — the library is
/// a single-user surface and imported VODs should appear alongside owned ones.
#[tauri::command]
pub fn get_cached_vods(channel_id: String, db: State<'_, DbConn>) -> Result<Vec<db::VodRow>, String> {
    let _ = channel_id; // retained for API compat; see doc comment
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_all_vods(&conn).map_err(|e| format!("DB error: {}", e))
}

// â”€â”€ AI Analysis â”€â”€

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

    // Escape the path for ffmpeg filter syntax â€” colons in Windows drive letters
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

// â”€â”€ Speech-to-Text (faster-whisper) â”€â”€

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

// â”€â”€ Keyword patterns for transcript scanning â”€â”€
// These match the keywords used by clip_selector::generate_transcript_candidates
const TRANSCRIPT_KEYWORDS: &[&str] = &[
    "no way", "oh my god", "what the", "holy", "let's go", "lets go",
    "clutch", "rage", "noooo", "nooo", "oh no", "run", "help",
    "behind", "dead", "done", "yes", "dude", "bro",
];

/// Convert whisper-rs TranscriptResult into the vod.rs TranscriptResult
/// expected by clip_selector and downstream pipeline.
fn convert_whisper_result(wr: &whisper::TranscriptResult) -> TranscriptResult {
    let segments: Vec<TranscriptSegment> = wr.segments.iter().map(|s| TranscriptSegment {
        start: s.start,
        end: s.end,
        text: s.text.clone(),
        words: Vec::new(), // whisper-rs doesn't provide word-level timestamps
    }).collect();

    let full_text = segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ");

    // Scan segments for keywords to generate TranscriptKeyword entries
    let mut keywords_found = Vec::new();
    for seg in &segments {
        let lower = seg.text.to_lowercase();
        for &kw in TRANSCRIPT_KEYWORDS {
            if lower.contains(kw) {
                keywords_found.push(TranscriptKeyword {
                    keyword: kw.to_string(),
                    timestamp: seg.start,
                    end_timestamp: seg.end,
                    context: seg.text.clone(),
                });
            }
        }
    }

    TranscriptResult {
        segments,
        full_text,
        language: wr.language.clone(),
        keywords_found,
    }
}

/// Run native whisper-rs transcription on a video file.
/// Returns transcript and saves JSON to disk for caching.
pub(crate) fn run_transcription_native(
    vod_path: &str,
    output_path: &str,
    vod_id: Option<&str>,
) -> Result<TranscriptResult, AppError> {
    // Resolve model + GPU preference from DB (both read from the same conn).
    // useGpu default is true (honor hardware CUDA support); users can flip it
    // off via Settings → Detection → "Use GPU (CUDA)".
    let (model, use_gpu) = {
        let conn = db::db_path().ok().and_then(|p| rusqlite::Connection::open(&p).ok());
        let model_name = conn.as_ref()
            .and_then(|c| db::get_setting(c, "whisper_model").ok().flatten())
            .unwrap_or_else(|| "base".to_string());
        let model = match model_name.as_str() {
            "medium" => whisper::WhisperModel::Medium,
            _ => whisper::WhisperModel::Base,
        };
        let ui_json = conn.as_ref()
            .and_then(|c| db::get_setting(c, "ui_settings").ok().flatten())
            .unwrap_or_default();
        let use_gpu = serde_json::from_str::<serde_json::Value>(&ui_json)
            .ok()
            .and_then(|v| v.get("useGpu").and_then(|b| b.as_bool()))
            .unwrap_or(true);
        (model, use_gpu)
    };

    // Check model is downloaded
    if !whisper::is_model_downloaded(model) {
        return Err(AppError::Transcription(format!(
            "Whisper model '{}' is not downloaded. Go to Settings â†’ Transcription Model to download it.",
            model.label()
        )));
    }

    log::info!(
        "[Transcription] Starting native whisper-rs · model={} · gpu={}",
        model.label(),
        use_gpu,
    );

    // Run transcription with progress reporting
    let vod_id_owned = vod_id.map(|s| s.to_string());
    let result = whisper::transcribe(vod_path, model, use_gpu, move |pct| {
        // Map whisper progress (0-100) to analysis progress (20-38%)
        if let Some(ref vid) = vod_id_owned {
            let mapped = 20 + (pct as i64 * 18 / 100).min(17);
            set_analysis_progress(vid, mapped);
        }
    }).map_err(|e| AppError::Transcription(e))?;

    // Convert to vod.rs format
    let transcript = convert_whisper_result(&result);

    // Save to disk for caching
    if let Ok(json) = serde_json::to_string_pretty(&transcript) {
        if let Err(e) = std::fs::write(output_path, &json) {
            log::warn!("[Transcription] Failed to cache transcript: {}", e);
        }
    }

    log::info!("[Transcription] Complete: {} segments, {} keywords found",
        transcript.segments.len(), transcript.keywords_found.len());

    Ok(transcript)
}

// â”€â”€ Legacy Python transcription (replaced by whisper-rs native) â”€â”€

/// Find Python executable path
// Replaced by whisper-rs native transcription â€” kept for potential fallback
#[allow(dead_code)]
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
// Replaced by run_transcription_native() â€” kept for potential fallback
#[allow(dead_code)]
pub(crate) fn run_transcription(vod_path: &str, output_path: &str, hw: &HardwareInfo, vod_id: Option<&str>) -> Result<TranscriptResult, AppError> {
    let python = find_python()?;
    let device = if hw.use_cuda { "cuda" } else { "cpu" };

    // Locate transcribe.py
    let script = find_transcribe_script()?;

    log::info!("Transcription: python={} script={} device={}", python.display(), script.display(), device);

    // Quick diagnostic: check if faster-whisper is importable
    let mut py_cmd = std::process::Command::new(&python);
    py_cmd.args(["-c", "import faster_whisper; print(faster_whisper.__version__)"]);
    py_cmd.env("CUDA_VISIBLE_DEVICES", "");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        py_cmd.creation_flags(0x08000000);
    }
    if let Ok(check) = py_cmd.output()
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
// Replaced by whisper-rs native transcription â€” kept for potential fallback
#[allow(dead_code)]
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
        "transcribe.py not found â€” place it in ai_engine/ next to the executable or in AppData/clipviral/ai_engine/".into()
    ))
}

// Replaced by whisper-rs native transcription â€” kept for potential fallback
#[allow(dead_code)]
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
    // which crashes if CUDA DLLs are missing â€” even with --device cpu.
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
                        // ANY output from whisper.py proves the process is alive,
                        // not just heartbeat JSON. Multi-hour CPU transcriptions
                        // emit ffmpeg progress, segment text, and warnings between
                        // explicit heartbeat lines — treating those as "silence"
                        // would falsely trigger the watchdog on long VODs (e.g.
                        // 7h Otzdarva streams take 1-2h on CPU; a single quiet
                        // ffmpeg pipe stretch can exceed the heartbeat interval).
                        heartbeat_tx.send(()).ok();

                        // Try to parse heartbeat JSON for progress % update
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                            if json.get("heartbeat").and_then(|v| v.as_bool()).unwrap_or(false) {
                                let pct = json.get("approx_pct").and_then(|v| v.as_i64()).unwrap_or(0);
                                let segs = json.get("segments_so_far").and_then(|v| v.as_i64()).unwrap_or(0);
                                log::info!("Transcription heartbeat: ~{}% done, {} segments", pct, segs);
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

    // Wait for the process. Watchdog fires only on TOTAL silence —
    // any whisper output (heartbeat OR raw stderr) resets the timer.
    // 15 min of true silence = genuinely stuck (process crashed, OOM, etc).
    // Old value was 5 min and only counted heartbeat JSON, which falsely
    // killed long CPU transcriptions where heartbeat cadence could exceed 5 min
    // during heavy ffmpeg pipe activity on multi-hour VODs.
    let no_heartbeat_timeout = std::time::Duration::from_secs(900); // 15 min total silence = stuck
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
                    log::error!("Transcription stalled â€” no heartbeat for {}s, killing process",
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
        // (stdout may contain multiple JSON lines â€” check each)
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
/// Concatenates all segment text into a single string â€” used to save a richer
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

        // Cascading analysis: signal-driven (local) â†’ position heuristic.
        let has_local_file = vod_clone.local_path.is_some();

        let mut result: Result<Vec<db::HighlightRow>, String> = Err("No analysis method available".into());

        // ── Twitch chat replay (used by both tiers) ──
        // Hits Twitch GQL directly. yt-dlp's `--write-subs --sub-lang
        // live_chat` doesn't work for Twitch (YouTube-only) so we bypass
        // it. Fetched once here in async context, passed into both tiers
        // via spawn_blocking. Failures are non-fatal.
        let chat_messages = match crate::twitch_chat_replay::fetch_chat_replay(
            &vod_clone.twitch_video_id,
        ).await {
            Ok(msgs) => {
                log::info!("[chat-replay] {} message(s) for VOD {}", msgs.len(), vod_id_bg);
                msgs
            }
            Err(e) => {
                log::warn!("[chat-replay] fetch failed: {} — continuing without chat", e);
                Vec::new()
            }
        };

        // Tier 1: Signal-driven (audio + transcript + chat) â€” fully local
        if has_ffmpeg && has_local_file {
            log::info!("Running signal-driven analysis for VOD {}", vod_id_bg);

            // ── Twitch community clips (optional detection signal) ──
            // Fetched here in async context so the sync `run_analysis_signals`
            // worker can consume them without needing tokio itself.
            let community_clips = fetch_community_clips_for_vod(&db, &vod_clone).await;
            if !community_clips.is_empty() {
                log::info!("[community-clips] {} clips feeding into selector", community_clips.len());
            }

            let vod_for_sync = vod_clone.clone();
            let hw_for_sync = hw_info.clone();
            let sens = sensitivity.clone();
            let chat_for_sync = chat_messages.clone();
            match tokio::task::spawn_blocking(move || run_analysis_signals(&vod_for_sync, &hw_for_sync, &sens, &community_clips, &chat_for_sync)).await {
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
            let chat_for_sync = chat_messages.clone();
            match tokio::task::spawn_blocking(move || run_analysis(&vod_for_sync, &chat_for_sync)).await {
                Ok(r) => { result = r; }
                Err(e) => { result = Err(format!("Task error: {e}")); }
            }
        };

        // Creating clips from highlights (82-88%)
        if let Ok(conn) = db.lock() {
            db::update_vod_analysis_progress(&conn, &vod_id_bg, 83).ok();
        }

        match result {
            Ok(mut highlights) => {
                let mut clip_thumb_info: Vec<(String, f64)> = Vec::new();
                // Clip IDs that qualify for auto-ship (confidence >= 0.9), built
                // alongside the clip-insert loop below. Sorted by confidence desc
                // so the cap picks the best ones first.
                let mut auto_ship_candidates: Vec<(String, f64)> = Vec::new();

                // Save-path Wave 3 upgrade: replace heuristic titles with LLM
                // titles when BYOK + Scope::Titles is enabled. Best-effort —
                // per-clip failures keep the heuristic title without aborting.
                let title_resolved = if let Ok(conn) = db.lock() {
                    crate::ai_provider::resolve(&conn, crate::ai_provider::Scope::Titles)
                } else {
                    crate::ai_provider::ResolvedProvider::free()
                };
                crate::commands::captions::upgrade_titles_with_llm(
                    &mut highlights,
                    &title_resolved,
                    vod_clone.game_name.as_deref(),
                    &vod_id_bg,
                    &db,
                )
                .await;

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

                        // Remember clip for auto-ship if it meets the confidence threshold.
                        let conf = h.confidence_score.unwrap_or(h.virality_score);
                        if conf >= 0.9 {
                            auto_ship_candidates.push((clip_id.clone(), conf));
                        }

                        clip_thumb_info.push((clip_id, h.start_seconds));
                    }

                    db::update_vod_analysis_progress(&conn, &vod_id_bg, 88).ok();

                    // ── Auto-ship high-confidence clips ──
                    // Sort candidates by confidence desc so the per-VOD cap picks the strongest.
                    auto_ship_candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                    let candidate_ids: Vec<String> = auto_ship_candidates.iter().map(|(id, _)| id.clone()).collect();
                    match run_auto_ship_for_vod(&conn, &vod_id_bg, &candidate_ids) {
                        Ok(report) if report.clips_queued > 0 => {
                            use tauri::Emitter;
                            log::info!(
                                "[auto-ship] queued {} clips across {:?} · next publish {:?}",
                                report.clips_queued, report.platforms, report.next_publish_at,
                            );
                            let _ = app.emit("auto-ship-queued", &report);
                        }
                        Ok(_) => { /* nothing queued — either disabled or no candidates */ }
                        Err(e) => log::warn!("[auto-ship] failed non-fatally: {}", e),
                    }
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
/// Report returned to the frontend after an auto-ship pass.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AutoShipReport {
    pub clips_queued: usize,
    pub platforms: Vec<String>,
    /// Scheduled time (ISO8601) of the earliest queued upload.
    pub next_publish_at: Option<String>,
}

/// Auto-ship MVP: after analysis produces highlights + clips, queue uploads
/// for every clip scoring 90%+ on every connected platform with a 5-minute
/// grace period. User can cancel via Scheduled page before the scheduler fires.
///
/// **Known limitation (v1):** clips must be exported manually before the grace
/// period expires — the scheduler currently refuses uploads without an
/// `output_path`. A future commit will add scheduler auto-export.
fn run_auto_ship_for_vod(
    conn: &rusqlite::Connection,
    vod_id: &str,
    candidate_clip_ids: &[String],
) -> Result<AutoShipReport, String> {
    // Read enable-flag out of the uiStore JSON blob (single source of truth).
    let ui_json = db::get_setting(conn, "ui_settings").ok().flatten().unwrap_or_default();
    let auto_ship_enabled = serde_json::from_str::<serde_json::Value>(&ui_json)
        .ok()
        .and_then(|v| v.get("autoShipHighConfidence").and_then(|b| b.as_bool()))
        .unwrap_or(false);
    if !auto_ship_enabled {
        return Ok(AutoShipReport { clips_queued: 0, platforms: Vec::new(), next_publish_at: None });
    }
    if candidate_clip_ids.is_empty() {
        return Ok(AutoShipReport { clips_queued: 0, platforms: Vec::new(), next_publish_at: None });
    }

    // Detect which platforms are connected (non-empty access token).
    let mut platforms: Vec<&'static str> = Vec::new();
    if db::get_setting(conn, "youtube_access_token").ok().flatten().map_or(false, |s| !s.is_empty()) {
        platforms.push("youtube");
    }
    if db::get_setting(conn, "tiktok_access_token").ok().flatten().map_or(false, |s| !s.is_empty()) {
        platforms.push("tiktok");
    }
    if platforms.is_empty() {
        log::info!("[auto-ship] enabled but no platforms connected for VOD {}", vod_id);
        return Ok(AutoShipReport { clips_queued: 0, platforms: Vec::new(), next_publish_at: None });
    }

    // Cap per analysis to prevent a 20-highlight VOD from flood-queuing.
    const MAX_AUTO_SHIPS_PER_ANALYSIS: usize = 3;
    let target_clips: Vec<&String> = candidate_clip_ids.iter().take(MAX_AUTO_SHIPS_PER_ANALYSIS).collect();

    let grace = chrono::Utc::now() + chrono::Duration::minutes(5);
    let scheduled_time = grace.to_rfc3339();
    let now = chrono::Utc::now().to_rfc3339();
    let mut queued: usize = 0;

    for clip_id in &target_clips {
        // Idempotency: skip if an auto-ship row already exists for this clip.
        let existing_for_clip = db::get_scheduled_uploads_for_clip(conn, clip_id)
            .unwrap_or_default();
        // Load clip to build the upload metadata stub.
        let clip = match db::get_clip_by_id(conn, clip_id) {
            Ok(Some(c)) => c,
            _ => { log::warn!("[auto-ship] clip {} not found, skipping", clip_id); continue }
        };

        for platform in &platforms {
            if existing_for_clip.iter().any(|u| u.platform == *platform) {
                log::info!("[auto-ship] {} / {} already scheduled, skipping", clip_id, platform);
                continue;
            }
            let meta = crate::social::UploadMeta {
                title: clip.title.clone(),
                description: clip.publish_description.clone().unwrap_or_default(),
                tags: clip.publish_hashtags
                    .as_deref()
                    .map(|s| s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect())
                    .unwrap_or_default(),
                visibility: "public".to_string(),
                clip_id: (*clip_id).clone(),
                force: false,
            };
            let meta_json = match serde_json::to_string(&meta) {
                Ok(s) => s,
                Err(e) => { log::error!("[auto-ship] meta serialize failed: {}", e); continue }
            };

            let row = db::ScheduledUploadRow {
                id: uuid::Uuid::new_v4().to_string(),
                clip_id: (*clip_id).clone(),
                platform: (*platform).to_string(),
                scheduled_time: scheduled_time.clone(),
                status: "pending".to_string(),
                retry_count: 0,
                error_message: None,
                video_url: None,
                upload_meta_json: Some(meta_json),
                created_at: now.clone(),
                view_count: None,
                like_count: None,
                ctr_percent: None,
                stats_updated_at: None,
            };
            if let Err(e) = db::insert_scheduled_upload(conn, &row) {
                log::error!("[auto-ship] insert failed for {} / {}: {}", clip_id, platform, e);
                continue;
            }
            log::info!("[auto-ship] queued upload {} for clip {} to {} at {}", row.id, clip_id, platform, scheduled_time);
            queued += 1;
        }
    }

    Ok(AutoShipReport {
        clips_queued: target_clips.len().min(queued),
        platforms: platforms.iter().map(|s| s.to_string()).collect(),
        next_publish_at: if queued > 0 { Some(scheduled_time) } else { None },
    })
}

/// Fetch Twitch community-created clips for this VOD and map them to the
/// selector's CommunityClip signal format. Non-fatal on any error — returns
/// an empty Vec so analysis continues without the boost signal.
async fn fetch_community_clips_for_vod(
    db: &State<'_, DbConn>,
    vod: &db::VodRow,
) -> Vec<clip_selector::CommunityClip> {
    // Read setting + token + broadcaster_id under one lock, drop before network IO.
    let (use_community, token, broadcaster_id) = {
        let conn = match db.lock() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let use_community = db::get_setting(&conn, "use_twitch_community_clips")
            .ok().flatten()
            .map(|v| v != "false")
            .unwrap_or(true);
        let token = db::get_setting(&conn, "twitch_user_access_token")
            .ok().flatten().unwrap_or_default();
        let broadcaster_id = db::get_all_channels(&conn)
            .ok()
            .and_then(|cs| cs.into_iter().find(|c| c.id == vod.channel_id))
            .map(|c| c.twitch_user_id)
            .unwrap_or_default();
        (use_community, token, broadcaster_id)
    };

    if !use_community {
        log::info!("[community-clips] disabled by user setting, skipping fetch");
        return Vec::new();
    }
    if token.is_empty() || broadcaster_id.is_empty() {
        log::info!("[community-clips] missing token or broadcaster id, skipping fetch");
        return Vec::new();
    }

    // 48h window around stream_date; filter to this VOD on the client.
    let started_at = vod.stream_date.clone();
    let ended_at = chrono::DateTime::parse_from_rfc3339(&vod.stream_date)
        .ok()
        .map(|dt| (dt + chrono::Duration::hours(48)).to_rfc3339())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    match crate::twitch::fetch_community_clips(&token, &broadcaster_id, &started_at, &ended_at).await {
        Ok(clips) => {
            let this_vod = vod.twitch_video_id.clone();
            let raw = clips.len();
            let matching: Vec<_> = clips.into_iter()
                .filter(|c| c.video_id.as_deref() == Some(this_vod.as_str()))
                .collect();
            log::info!(
                "[community-clips] broadcaster={}: fetched {} total, {} matched this VOD ({})",
                broadcaster_id, raw, matching.len(), this_vod,
            );
            matching.into_iter().filter_map(|c| {
                c.vod_offset.map(|off| clip_selector::CommunityClip {
                    vod_offset_seconds: off as f64,
                    duration_seconds: c.duration,
                    view_count: c.view_count,
                    title: c.title,
                })
            }).collect()
        }
        Err(e) => {
            log::warn!("[community-clips] fetch failed (non-fatal): {}", e);
            Vec::new()
        }
    }
}

fn run_analysis_signals(
    vod: &db::VodRow,
    _hw: &HardwareInfo,
    sensitivity: &str,
    community_clips: &[clip_selector::CommunityClip],
    chat_messages: &[crate::twitch_chat_replay::ChatMessage],
) -> Result<Vec<db::HighlightRow>, String> {
    let ffmpeg = find_ffmpeg()?;
    let vod_path = vod.local_path.clone()
        .ok_or("VOD not downloaded")?;
    let duration = vod.duration_seconds as f64;
    let vod_id = &vod.id;
    let now = chrono::Utc::now().to_rfc3339();

    // â”€â”€ Stage 1: Audio analysis (5-15%) â”€â”€
    log::info!("Signal analysis: extracting audio profile...");
    set_analysis_progress(vod_id, 5);
    let audio_profile = analyze_audio_intensity(&vod_path, &ffmpeg).ok();
    let audio_ctx = audio_profile.as_ref().map(|a| {
        clip_selector::AudioContext::new(a.rms_per_second.clone(), a.spike_seconds.clone())
    });
    set_analysis_progress(vod_id, 15);

    // â”€â”€ Stage 2: Transcription (15-40%) â”€â”€
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
    } else {
        // Native whisper-rs transcription
        set_analysis_progress(vod_id, 20);
        let out = transcript_path.to_string_lossy().to_string();
        match run_transcription_native(&vod_path, &out, Some(vod_id)) {
            Ok(result) => {
                set_analysis_progress(vod_id, 38);
                Some(result)
            }
            Err(e) => {
                log::warn!("Native transcription failed: {}. Continuing without transcript.", e.detail());
                None
            }
        }
    };
    set_analysis_progress(vod_id, 40);

    // â”€â”€ Stage 3: Chat analysis (40-50%) â”€â”€
    // Operates on pre-fetched chat replay (fetched in async context by
    // analyze_vod). Two analyses on the same input: 30s rate windows +
    // 10s emote-burst windows. No I/O here.
    log::info!("Signal analysis: analyzing chat activity...");
    set_analysis_progress(vod_id, 42);
    let (chat_peaks, emote_peaks): (Vec<db::HighlightRow>, Vec<db::HighlightRow>) =
        match analyze_via_chat(chat_messages, duration, &vod.id) {
            Ok(r) => {
                log::info!(
                    "Chat analysis: {} rate peak(s), {} emote-burst peak(s) from {} messages",
                    r.rate_peaks.len(),
                    r.emote_peaks.len(),
                    chat_messages.len(),
                );
                (r.rate_peaks, r.emote_peaks)
            }
            Err(e) => {
                log::info!("Chat analysis skipped: {} ({} messages available)", e, chat_messages.len());
                (Vec::new(), Vec::new())
            }
        };
    set_analysis_progress(vod_id, 50);

    // â”€â”€ Stage 4: Clip selection pipeline (50-65%) â”€â”€
    // Community clips are fetched in the async caller (this fn is sync because
    // it runs inside tokio::task::spawn_blocking) and passed in as `community_clips`.
    log::info!("Signal analysis: running clip selector pipeline...");
    set_analysis_progress(vod_id, 52);
    let (selected, detection_stats): (Vec<clip_selector::ClipCandidate>, _) = clip_selector::select_clips(
        audio_ctx.as_ref(),
        transcript.as_ref(),
        &chat_peaks,
        &emote_peaks,
        community_clips,
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
        return run_analysis(vod, chat_messages);
    }

    // â”€â”€ Stage 5: Scoring and ranking (60-75%) â”€â”€
    log::info!("Signal analysis: scoring {} candidates...", selected.len());
    set_analysis_progress(vod_id, 62);
    let mut highlights: Vec<db::HighlightRow> = Vec::new();
    let total_candidates = selected.len();

    for (i, c) in selected.iter().enumerate() {
        let all_tags: Vec<String> = [&c.event_tags[..], &c.emotion_tags[..]].concat();
        let tag_str = if all_tags.is_empty() { "auto".to_string() } else { all_tags.join(",") };

        let title = crate::commands::captions::save_path_heuristic_title(
            c.transcript_excerpt.as_deref(),
            Some(&tag_str),
            vod.game_name.as_deref(),
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

    // â”€â”€ Stage 6: Generate captions (75-82%) â”€â”€
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

/// Position-based fallback â€” last resort when no signals are available.
/// Only used when VOD is not downloaded or ffmpeg is missing.
fn run_analysis(
    vod: &db::VodRow,
    chat_messages: &[crate::twitch_chat_replay::ChatMessage],
) -> Result<Vec<db::HighlightRow>, String> {
    let duration = vod.duration_seconds as f64;
    let vod_id = &vod.id;
    let now = chrono::Utc::now().to_rfc3339();

    // Try chat-based analysis first. Tier-2 fallback combines rate + emote
    // peaks into one set of highlights (no fusion stage available here).
    if let Ok(r) = analyze_via_chat(chat_messages, duration, vod_id) {
        let mut combined = r.rate_peaks;
        combined.extend(r.emote_peaks);
        if !combined.is_empty() {
            return Ok(combined);
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
/// Result of one chat replay analysis. Both rate peaks and emote-burst
/// peaks come from the same fetched message stream — single API pull,
/// dual analysis.
struct ChatAnalysisResult {
    rate_peaks: Vec<db::HighlightRow>,
    emote_peaks: Vec<db::HighlightRow>,
}

/// Analyze pre-fetched chat replay messages for both rate spikes and
/// emote-burst windows. The fetch happens in async context (see
/// `crate::twitch_chat_replay::fetch_chat_replay`); this function is
/// pure CPU work and can run inside `spawn_blocking`.
fn analyze_via_chat(
    messages: &[crate::twitch_chat_replay::ChatMessage],
    duration: f64,
    vod_id: &str,
) -> Result<ChatAnalysisResult, String> {
    if messages.is_empty() {
        return Err("No chat messages available".to_string());
    }

    // Two parallel windowings:
    //   - Rate: 30s windows (legacy chat-rate detection — coarse, robust)
    //   - Emote: 10s windows (sharper, since emote bursts are quick)
    let rate_window_size = 30.0_f64.max(duration * 0.05);
    let emote_window_size = 10.0_f64;

    let num_rate_windows = ((duration / rate_window_size).ceil() as usize).max(1);
    let num_emote_windows = ((duration / emote_window_size).ceil() as usize).max(1);
    let mut rate_counts = vec![0u32; num_rate_windows];
    let mut emote_counts = vec![0u32; num_emote_windows];
    let mut total_messages = 0u32;
    let mut total_emotes = 0u32;

    for msg in messages {
        let t = msg.time_seconds;
        if t < 0.0 || t > duration {
            continue;
        }

        // Rate-peak bucket: count this message
        let rate_idx = ((t / rate_window_size) as usize).min(num_rate_windows - 1);
        rate_counts[rate_idx] += 1;
        total_messages += 1;

        // Emote-density bucket: count emotes in the message body
        let emote_count = crate::emote_signal::count_emotes(&msg.body);
        if emote_count > 0 {
            let emote_idx = ((t / emote_window_size) as usize).min(num_emote_windows - 1);
            emote_counts[emote_idx] += emote_count;
            total_emotes += emote_count;
        }
    }

    if total_messages < 5 {
        return Err("Not enough chat data".to_string());
    }

    let now = chrono::Utc::now().to_rfc3339();

    // ── Rate peaks (legacy 30s window logic, unchanged behavior) ──
    let rate_avg = total_messages as f64 / num_rate_windows as f64;
    let mut rate_peak_idxs: Vec<(usize, u32)> = rate_counts.iter().enumerate()
        .filter(|(_, &count)| count as f64 > rate_avg * 1.3)
        .map(|(i, &count)| (i, count))
        .collect();
    rate_peak_idxs.sort_by(|a, b| b.1.cmp(&a.1));
    rate_peak_idxs.truncate(5);

    let mut rate_peaks = Vec::new();
    if !rate_peak_idxs.is_empty() {
        let max_count = rate_peak_idxs[0].1 as f64;
        for (idx, count) in &rate_peak_idxs {
            let start = (*idx as f64 * rate_window_size).max(0.0);
            let end = (start + rate_window_size).min(duration);
            let chat_score = *count as f64 / max_count;
            let virality = 0.5 + chat_score * 0.45;
            let mins = (start as u32) / 60;
            let secs = (start as u32) % 60;
            rate_peaks.push(db::HighlightRow {
                id: uuid::Uuid::new_v4().to_string(),
                vod_id: vod_id.to_string(),
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
    }

    // ── Emote-burst peaks (10s windows, stricter threshold) ──
    // Threshold is 2.0× the per-window emote-rate average (vs chat's 1.3×).
    // Emote bursts should be sharper than message-rate peaks; an emote
    // density that's barely above average is just baseline reactivity, not
    // a moment. Cap at 8 peaks so the fusion stage doesn't get flooded
    // (chat-emote bursts often cluster around the same in-game beat).
    let mut emote_peaks = Vec::new();
    if total_emotes >= 5 {
        let emote_avg = total_emotes as f64 / num_emote_windows as f64;
        let threshold = (emote_avg * 2.0).max(3.0);
        let mut emote_peak_idxs: Vec<(usize, u32)> = emote_counts.iter().enumerate()
            .filter(|(_, &count)| count as f64 > threshold)
            .map(|(i, &count)| (i, count))
            .collect();
        emote_peak_idxs.sort_by(|a, b| b.1.cmp(&a.1));
        emote_peak_idxs.truncate(8);

        if !emote_peak_idxs.is_empty() {
            let max_count = emote_peak_idxs[0].1 as f64;
            for (idx, count) in &emote_peak_idxs {
                let start = (*idx as f64 * emote_window_size).max(0.0);
                let end = (start + emote_window_size).min(duration);
                let chat_score = (*count as f64 / max_count).clamp(0.0, 1.0);
                let virality = 0.55 + chat_score * 0.40;
                let mins = (start as u32) / 60;
                let secs = (start as u32) % 60;
                emote_peaks.push(db::HighlightRow {
                    id: uuid::Uuid::new_v4().to_string(),
                    vod_id: vod_id.to_string(),
                    start_seconds: start,
                    end_seconds: end,
                    virality_score: virality,
                    audio_score: virality * 0.85,
                    visual_score: virality * 0.85,
                    chat_score,
                    transcript_snippet: Some(format!("{} emote occurrences in this 10s window", count)),
                    description: Some(format!("Emote burst ({} emotes) at {}:{:02}", count, mins, secs)),
                    tags: Some("emote-burst,reaction,auto".to_string()),
                    thumbnail_path: None,
                    created_at: now.clone(),
                    confidence_score: Some(compute_confidence(virality, 1)),
                    explanation: Some(format!("1 signal — emote burst {:.0}% ({} emotes)", chat_score * 100.0, count)),
                    event_summary: Some(format!("chat hit with {} emotes in 10s", count)),
                });
            }
        }
    }

    if rate_peaks.is_empty() && emote_peaks.is_empty() {
        return Err("No engagement peaks found".to_string());
    }

    Ok(ChatAnalysisResult { rate_peaks, emote_peaks })
}

// â”€â”€ VOD info / list commands â”€â”€

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
        log::warn!("[get_vods] No access token found â€” user not logged in");
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
    // Return all VODs (own + imported via import_vod_by_url) so foreign-channel
    // imports appear in the same library list. See get_all_vods doc comment.
    let result = db::get_all_vods(&conn).map_err(|e| format!("DB error: {}", e))?;
    log::info!("[get_vods] get_all_vods returned {} VODs", result.len());
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
            log::info!("All clips deleted for VOD {} â€” reset analysis_status to pending", vid);
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

    // Fetch fresh video data from Twitch via curl fallback â€” retry with refreshed token on 401
    let url = format!("https://api.twitch.tv/helix/videos?id={}", twitch_video_id);
    let mut body = twitch::curl_twitch_get(&url, &access_token).await
        .map_err(|e| format!("Twitch API error: {}", e))?;

    // Check for 401 in the response body (curl doesn't give us HTTP status codes directly)
    if body.contains("\"status\":401") || body.contains("\"status\": 401") {
        // Token expired â€” try refreshing
        access_token = try_refresh_twitch_token(&db).await?;
        body = twitch::curl_twitch_get(&url, &access_token).await
            .map_err(|e| format!("Twitch API error: {}", e))?;
    }

    // Check for other API errors
    if let Ok(err_val) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(status) = err_val.get("status") {
            let msg = err_val.get("message").and_then(|m| m.as_str()).unwrap_or("");
            return Err(format!("Twitch API {}: {}", status, msg));
        }
    }

    let resp_json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Parse error: {}", e))?;

    let video = resp_json["data"].as_array()
        .and_then(|arr| arr.first())
        .ok_or("Video not found on Twitch")?;

    let title = video["title"].as_str().unwrap_or("").to_string();
    let thumbnail_url = video["thumbnail_url"].as_str().unwrap_or("")
        .replace("%{width}", "640")
        .replace("%{height}", "360");

    // Update VOD title and thumbnail in database.
    // Preserve game_name â€” it's user-set and should not be cleared on metadata refresh.
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

/// Set the game on a single clip (lightweight â€” used for auto-save after subtitle inference).
#[tauri::command]
pub fn set_clip_game(clip_id: String, game: Option<String>, db: State<'_, DbConn>) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let g = game.as_deref().filter(|s| !s.is_empty());
    log::info!("[set_clip_game] Setting clip {} game to: {:?}", clip_id, g);
    db::update_clip_game(&conn, &clip_id, g)
        .map_err(|e| format!("DB error: {}", e))
}

/// Set the title on a single clip (lightweight â€” used for auto-save on blur).
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
/// This is the feedback loop â€” learn what works for this creator.
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
pub async fn get_transcript(vod_id: String, db: State<'_, DbConn>, _hw: State<'_, HardwareInfo>) -> Result<serde_json::Value, String> {
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

    // Run native whisper-rs transcription
    let output_str = output_path.to_string_lossy().to_string();
    let vod_path_clone = vod_path.clone();
    let result = tokio::task::spawn_blocking(move || {
        run_transcription_native(&vod_path_clone, &output_str, None)
    }).await.map_err(|e| format!("Task error: {}", e))??;

    // Save path to VOD record
    {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::update_vod_transcript_path(&conn, &vod_id, &output_path.to_string_lossy()).ok();
    }

    serde_json::to_value(&result).map_err(|e| format!("Serialize: {}", e))
}

/// Extract the numeric Twitch video ID from common VOD URL shapes:
///   https://www.twitch.tv/videos/2345678901
///   https://twitch.tv/videos/2345678901?t=1h2m3s
///   www.twitch.tv/videos/2345678901
///   twitch.tv/videos/2345678901
///   2345678901  (bare ID)
fn parse_twitch_vod_id(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() { return None }
    // Bare numeric ID
    if trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Some(trimmed.to_string())
    }
    // Find "videos/" segment and take following digits
    let lower = trimmed.to_lowercase();
    let marker = "videos/";
    if let Some(start) = lower.find(marker) {
        let tail = &trimmed[start + marker.len()..];
        let id: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !id.is_empty() {
            return Some(id)
        }
    }
    None
}

/// Import a Twitch VOD by pasting its URL. Fetches metadata from Helix,
/// creates/updates the channel row if needed, and upserts the VOD.
/// Returns the resulting VodRow (caller should refresh the VODs list after).
#[tauri::command]
pub async fn import_vod_by_url(
    url: String,
    db: State<'_, DbConn>,
) -> Result<db::VodRow, String> {
    // ── Dev-only command ──
    // Importing arbitrary public Twitch VODs is allowed in `cargo tauri dev`
    // (so we can test signal pipelines on chatty/popular streamer chats), but
    // shipped builds must refuse. Letting end users pull any streamer's content
    // violates Twitch ToS and risks DMCA / direct legal action from creators.
    // The UI button is also gated on `import.meta.env.DEV`, but we double-gate
    // here in case the command is ever invoked directly via DevTools or tooling.
    if cfg!(not(debug_assertions)) {
        return Err("VOD import by URL is disabled in this build.".to_string());
    }

    let twitch_video_id = parse_twitch_vod_id(&url)
        .ok_or_else(|| format!("Couldn't find a Twitch VOD ID in: {}", url))?;

    // If this VOD was already imported, return it as-is (idempotent)
    {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let existing: Option<db::VodRow> = conn.query_row(
            "SELECT id FROM vods WHERE twitch_video_id = ?1",
            rusqlite::params![twitch_video_id],
            |row| row.get::<_, String>(0),
        ).ok().and_then(|id| db::get_vod_by_id(&conn, &id).ok().flatten());
        if let Some(v) = existing {
            log::info!("[import_vod_by_url] VOD {} already imported ({})", twitch_video_id, v.id);
            return Ok(v)
        }
    }

    // Access token for Helix call
    let mut access_token = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::get_setting(&conn, "twitch_user_access_token")
            .map_err(|e| format!("DB error: {}", e))?
            .unwrap_or_default()
    };
    if access_token.is_empty() {
        return Err("Not logged in. Please connect Twitch in Settings first.".into());
    }

    // Fetch video metadata — retry once on 401 with refreshed token
    let api_url = format!("https://api.twitch.tv/helix/videos?id={}", twitch_video_id);
    let mut body = twitch::curl_twitch_get(&api_url, &access_token).await
        .map_err(|e| format!("Twitch API error: {}", e))?;
    if body.contains("\"status\":401") || body.contains("\"status\": 401") {
        access_token = try_refresh_twitch_token(&db).await?;
        body = twitch::curl_twitch_get(&api_url, &access_token).await
            .map_err(|e| format!("Twitch API error: {}", e))?;
    }

    let resp: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Parse error: {}", e))?;

    if let Some(status) = resp.get("status") {
        let msg = resp.get("message").and_then(|m| m.as_str()).unwrap_or("");
        return Err(format!("Twitch API {}: {}", status, msg));
    }

    let video = resp["data"].as_array()
        .and_then(|arr| arr.first())
        .ok_or_else(|| format!("VOD {} not found on Twitch (deleted, private, or sub-only)", twitch_video_id))?;

    let title = video["title"].as_str().unwrap_or("Untitled VOD").to_string();
    let thumbnail_url = video["thumbnail_url"].as_str().unwrap_or("")
        .replace("%{width}", "640")
        .replace("%{height}", "360");
    let duration_str = video["duration"].as_str().unwrap_or("0s");
    let duration_seconds = twitch::parse_duration(duration_str);
    let stream_date = video["created_at"].as_str().unwrap_or("").to_string();
    let vod_url = video["url"].as_str().unwrap_or(&url).to_string();
    let user_id = video["user_id"].as_str().unwrap_or("").to_string();
    let user_login = video["user_login"].as_str().unwrap_or("").to_string();
    let user_name = video["user_name"].as_str().unwrap_or(&user_login).to_string();

    if user_id.is_empty() {
        return Err("Twitch response missing channel info".into());
    }

    // Find-or-create channel row
    let channel_id = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let channels = db::get_all_channels(&conn)
            .map_err(|e| format!("DB error: {}", e))?;
        if let Some(ch) = channels.into_iter().find(|c| c.twitch_user_id == user_id) {
            ch.id
        } else {
            // Create a stub channel so the VOD has something to attach to
            let new_id = uuid::Uuid::new_v4().to_string();
            db::insert_channel(&conn, &new_id, &user_id, &user_login, &user_name, "")
                .map_err(|e| format!("DB error: {}", e))?;
            log::info!("[import_vod_by_url] Created stub channel for @{} ({})", user_login, user_id);
            new_id
        }
    };

    // Build and upsert the VOD
    let vod_row = db::VodRow {
        id: uuid::Uuid::new_v4().to_string(),
        channel_id: channel_id.clone(),
        twitch_video_id: twitch_video_id.clone(),
        title,
        duration_seconds,
        stream_date,
        thumbnail_url,
        vod_url,
        download_status: "pending".to_string(),
        local_path: None,
        file_size_bytes: None,
        analysis_status: "pending".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        download_progress: Some(0),
        analysis_progress: 0,
        game_name: None,
    };
    {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::upsert_vod(&conn, &vod_row).map_err(|e| format!("DB error: {}", e))?;
        log::info!("[import_vod_by_url] Imported VOD {} ({}) from @{}", twitch_video_id, vod_row.id, user_login);
    }

    // Re-read so we return any DB-side defaults that may have been applied
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::get_vod_by_id(&conn, &vod_row.id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or_else(|| "VOD not found after insert".to_string())
}

/// Stream-live status for the sidebar channel card. All fields optional — when
/// the channel isn't live the frontend just shows their handle without the pulse.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StreamStatus {
    pub is_live: bool,
    pub viewer_count: i64,
    pub game_name: Option<String>,
    pub title: Option<String>,
    pub started_at: Option<String>,
}

/// Check whether a Twitch channel is currently streaming and pull the viewer
/// count + game. Called every ~60s by the frontend for the sidebar card.
/// Cheap Helix call — does NOT consume any analysis quota.
#[tauri::command]
pub async fn get_stream_status(
    channel_id: String,
    db: State<'_, DbConn>,
) -> Result<StreamStatus, String> {
    let (twitch_user_id, mut access_token) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let channels = db::get_all_channels(&conn)
            .map_err(|e| format!("DB error: {}", e))?;
        let channel = channels.into_iter().find(|c| c.id == channel_id)
            .ok_or("Channel not found")?;
        let token = db::get_setting(&conn, "twitch_user_access_token")
            .map_err(|e| format!("DB error: {}", e))?
            .unwrap_or_default();
        (channel.twitch_user_id, token)
    };

    if access_token.is_empty() {
        return Ok(StreamStatus {
            is_live: false,
            viewer_count: 0,
            game_name: None,
            title: None,
            started_at: None,
        });
    }

    let url = format!("https://api.twitch.tv/helix/streams?user_id={}", twitch_user_id);
    let mut body = twitch::curl_twitch_get(&url, &access_token).await
        .map_err(|e| format!("Twitch API error: {}", e))?;
    if body.contains("\"status\":401") || body.contains("\"status\": 401") {
        access_token = try_refresh_twitch_token(&db).await?;
        body = twitch::curl_twitch_get(&url, &access_token).await
            .map_err(|e| format!("Twitch API error: {}", e))?;
    }

    let resp: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Parse error: {}", e))?;

    // When the user is offline, Twitch returns { data: [] } — not an error.
    let stream = resp["data"].as_array().and_then(|arr| arr.first());
    match stream {
        None => Ok(StreamStatus {
            is_live: false,
            viewer_count: 0,
            game_name: None,
            title: None,
            started_at: None,
        }),
        Some(s) => Ok(StreamStatus {
            is_live: true,
            viewer_count: s["viewer_count"].as_i64().unwrap_or(0),
            game_name: s["game_name"].as_str().map(|x| x.to_string()).filter(|x| !x.is_empty()),
            title: s["title"].as_str().map(|x| x.to_string()).filter(|x| !x.is_empty()),
            started_at: s["started_at"].as_str().map(|x| x.to_string()),
        }),
    }
}
