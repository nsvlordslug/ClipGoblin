//! Whisper.cpp transcription via whisper-rs.
//!
//! Provides model management (download check, paths) and audio transcription
//! with automatic GPU acceleration when CUDA is available at runtime.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

// ── Model definitions ──

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WhisperModel {
    Base,
    Medium,
}

impl WhisperModel {
    pub fn filename(&self) -> &'static str {
        match self {
            Self::Base => "ggml-base.bin",
            Self::Medium => "ggml-medium.bin",
        }
    }

    pub fn download_url(&self) -> &'static str {
        match self {
            Self::Base => "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
            Self::Medium => "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin",
        }
    }

    /// Approximate file size in bytes.
    pub fn size_bytes(&self) -> u64 {
        match self {
            Self::Base => 148_000_000,   // ~148 MB
            Self::Medium => 1_533_000_000, // ~1.5 GB
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Base => "Base (Fast)",
            Self::Medium => "Medium (Accurate)",
        }
    }
}

// ── Transcript types ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TranscriptSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TranscriptResult {
    pub segments: Vec<TranscriptSegment>,
    pub language: String,
    pub duration: f64,
}

// ── Path helpers ──

/// Models directory: %APPDATA%/clipviral/models/
pub fn models_dir() -> Result<PathBuf, String> {
    let data = dirs::data_dir().ok_or("Cannot determine app data directory")?;
    let dir = data.join("clipviral").join("models");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create models dir: {}", e))?;
    Ok(dir)
}

/// Full path to a model file.
pub fn model_path(model: WhisperModel) -> Result<PathBuf, String> {
    Ok(models_dir()?.join(model.filename()))
}

/// Check whether a model has been downloaded.
pub fn is_model_downloaded(model: WhisperModel) -> bool {
    model_path(model)
        .map(|p| p.exists())
        .unwrap_or(false)
}

// ── FFmpeg helper ──

/// Locate ffmpeg. Delegates to bin_manager (bundled → PATH).
pub fn find_ffmpeg() -> Result<PathBuf, String> {
    crate::bin_manager::ffmpeg_path().map_err(|e| e.to_string())
}

// ── Transcription ──

/// Extract 16 kHz mono f32le PCM from a media file using ffmpeg, piped to stdout.
fn extract_pcm_audio(audio_path: &str, ffmpeg: &PathBuf) -> Result<Vec<f32>, String> {
    let mut child_cmd = Command::new(ffmpeg);
    child_cmd.args([
        "-i", audio_path,
        "-ar", "16000",
        "-ac", "1",
        "-f", "f32le",
        "-acodec", "pcm_f32le",
        "pipe:1",
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        child_cmd.creation_flags(0x08000000);
    }
    let mut child = child_cmd.spawn()
        .map_err(|e| format!("Failed to spawn ffmpeg: {}", e))?;

    let mut raw = Vec::new();
    if let Some(ref mut stdout) = child.stdout {
        stdout
            .read_to_end(&mut raw)
            .map_err(|e| format!("Failed to read ffmpeg output: {}", e))?;
    }

    let status = child
        .wait()
        .map_err(|e| format!("ffmpeg wait error: {}", e))?;
    if !status.success() {
        return Err(format!("ffmpeg exited with status: {}", status));
    }

    // Convert raw bytes to f32 samples (little-endian)
    if raw.len() % 4 != 0 {
        return Err("PCM data not aligned to 4 bytes".into());
    }
    let samples: Vec<f32> = raw
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();

    if samples.is_empty() {
        return Err("No audio samples extracted".into());
    }

    Ok(samples)
}

/// Transcribe an audio/video file using whisper-rs.
///
/// - `audio_path`: path to the media file (ffmpeg extracts audio)
/// - `model`: which whisper model to use
/// - `use_gpu`: whether to allow GPU acceleration. Pass `false` to force CPU
///   even on a CUDA-capable machine — respects the user's UI toggle.
/// - `on_progress`: callback with percentage (0–100)
pub fn transcribe<F>(
    audio_path: &str,
    model: WhisperModel,
    use_gpu: bool,
    on_progress: F,
) -> Result<TranscriptResult, String>
where
    F: Fn(u32) + Send + Sync + 'static,
{
    // Wrap callback in Arc so it can be shared with the whisper progress closure
    let progress_fn = Arc::new(on_progress);

    // 1. Verify model exists
    let mpath = model_path(model)?;
    if !mpath.exists() {
        return Err(format!(
            "Model {} not downloaded. Expected at: {}",
            model.label(),
            mpath.display()
        ));
    }

    progress_fn(2);

    // 2. Extract PCM audio via ffmpeg
    let ffmpeg = find_ffmpeg()?;
    log::info!(
        "[Whisper] Extracting 16kHz PCM from {} using {}",
        audio_path,
        ffmpeg.display()
    );
    progress_fn(5);

    let samples = extract_pcm_audio(audio_path, &ffmpeg)?;
    let duration = samples.len() as f64 / 16000.0;
    log::info!(
        "[Whisper] Extracted {:.1}s of audio ({} samples)",
        duration,
        samples.len()
    );
    progress_fn(15);

    // 3. Load whisper model
    log::info!(
        "[Whisper] Loading model: {} (GPU={})",
        mpath.display(),
        if use_gpu { "enabled" } else { "disabled" },
    );
    let mut params = WhisperContextParameters::default();
    params.use_gpu = use_gpu;
    let ctx = WhisperContext::new_with_params(
        mpath.to_str().ok_or("Invalid model path encoding")?,
        params,
    )
    .map_err(|e| format!("Failed to load whisper model: {}", e))?;

    progress_fn(25);

    // 4. Configure inference parameters
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("en"));
    params.set_token_timestamps(true);
    params.set_n_threads(optimal_thread_count());

    // Progress callback — whisper calls this periodically during inference.
    let progress_fn_clone = Arc::clone(&progress_fn);
    params.set_progress_callback_safe(move |progress: i32| {
        // Map whisper progress (0–100) to our range (30–95)
        let mapped = 30 + ((progress as u32) * 65 / 100);
        progress_fn_clone(mapped.min(95));
    });

    // 5. Run inference
    log::info!("[Whisper] Starting transcription...");
    let mut state = ctx
        .create_state()
        .map_err(|e| format!("Failed to create whisper state: {}", e))?;

    state
        .full(params, &samples)
        .map_err(|e| format!("Whisper inference failed: {}", e))?;

    progress_fn(96);

    // 6. Extract segments
    // whisper-rs 0.16: these methods return values directly, not Result
    let num_segments = state.full_n_segments();
    let mut segments = Vec::with_capacity(num_segments as usize);

    for i in 0..num_segments {
        let segment = match state.get_segment(i) {
            Some(s) => s,
            None => continue,
        };
        // whisper timestamps are in centiseconds (10ms units)
        let start = segment.start_timestamp() as f64 / 100.0;
        let end = segment.end_timestamp() as f64 / 100.0;
        let text = segment.to_str().unwrap_or("").trim().to_string();
        if !text.is_empty() {
            segments.push(TranscriptSegment { start, end, text });
        }
    }

    progress_fn(100);

    log::info!(
        "[Whisper] Transcription complete: {} segments, {:.1}s duration",
        segments.len(),
        duration
    );

    Ok(TranscriptResult {
        segments,
        language: "en".to_string(),
        duration,
    })
}

/// Determine optimal thread count for whisper inference.
/// Uses physical cores (not logical/hyperthreaded) minus 1 to keep the UI responsive.
fn optimal_thread_count() -> i32 {
    let cpus = num_cpus::get_physical();
    let threads = if cpus > 2 { cpus - 1 } else { 1 };
    log::debug!("[Whisper] Using {} threads ({} physical cores)", threads, cpus);
    threads as i32
}
