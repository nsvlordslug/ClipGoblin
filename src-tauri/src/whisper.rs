//! Whisper.cpp transcription via whisper-rs.
//!
//! Provides model management (download check, paths) and audio transcription
//! with automatic GPU acceleration when CUDA is available at runtime.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;

use whisper_rs::{
    DtwMode, DtwModelPreset, FullParams, SamplingStrategy, WhisperContext,
    WhisperContextParameters, WhisperSegment,
};

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
pub struct TranscriptWord {
    pub word: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TranscriptSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub words: Vec<TranscriptWord>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TranscriptResult {
    pub segments: Vec<TranscriptSegment>,
    pub language: String,
    pub duration: f64,
}

#[derive(Debug)]
struct TimedTokenPiece {
    text: String,
    start: f64,
    end: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SpeechAudioStream {
    index: u32,
    label: String,
}

#[derive(Debug, serde::Deserialize)]
struct AudioProbeOutput {
    #[serde(default)]
    streams: Vec<AudioProbeStream>,
}

#[derive(Debug, serde::Deserialize)]
struct AudioProbeStream {
    index: u32,
    #[serde(default)]
    tags: AudioProbeTags,
}

#[derive(Debug, Default, serde::Deserialize)]
struct AudioProbeTags {
    name: Option<String>,
    title: Option<String>,
    handler_name: Option<String>,
}

fn microphone_label_score(label: &str) -> u8 {
    let normalized: String = label
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect();
    let words: Vec<&str> = normalized.split_whitespace().collect();

    if words.contains(&"microphone") {
        2
    } else if words.contains(&"mic") {
        1
    } else {
        0
    }
}

fn select_microphone_audio_stream(probe_json: &str) -> Option<SpeechAudioStream> {
    let probe: AudioProbeOutput = serde_json::from_str(probe_json).ok()?;
    let mut best: Option<(u8, SpeechAudioStream)> = None;

    for stream in probe.streams {
        for label in [
            stream.tags.name.as_deref(),
            stream.tags.title.as_deref(),
            stream.tags.handler_name.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            let score = microphone_label_score(label);
            let is_better = score > 0
                && best
                    .as_ref()
                    .map(|(best_score, _)| score > *best_score)
                    .unwrap_or(true);
            if is_better {
                best = Some((
                    score,
                    SpeechAudioStream {
                        index: stream.index,
                        label: label.to_string(),
                    },
                ));
            }
        }
    }

    best.map(|(_, stream)| stream)
}

fn preferred_transcription_audio_stream(audio_path: &str) -> Option<SpeechAudioStream> {
    let ffprobe = crate::bin_manager::ffprobe_path().ok()?;
    let mut command = Command::new(ffprobe);
    command
        .arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("a")
        .arg("-show_entries")
        .arg("stream=index:stream_tags=name,title,handler_name")
        .arg("-of")
        .arg("json")
        .arg(audio_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let probe_json = std::str::from_utf8(&output.stdout).ok()?;
    select_microphone_audio_stream(probe_json)
}

fn value_after_marker(line: &str, marker: &str) -> Option<f64> {
    line.split_once(marker)?
        .1
        .trim_start()
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn speech_windows_from_silence_log(stderr: &str, duration: f64) -> Vec<(f64, f64)> {
    if !duration.is_finite() || duration <= 0.0 {
        return Vec::new();
    }

    let mut silences = Vec::new();
    let mut open_silence = None;
    let mut saw_silence_event = false;
    for line in stderr.lines() {
        if let Some(start) = value_after_marker(line, "silence_start:") {
            saw_silence_event = true;
            open_silence = Some(start.clamp(0.0, duration));
        }
        if let Some(end) = value_after_marker(line, "silence_end:") {
            saw_silence_event = true;
            let start = open_silence.take().unwrap_or(0.0).clamp(0.0, duration);
            let end = end.clamp(start, duration);
            silences.push((start, end));
        }
    }
    if let Some(start) = open_silence {
        silences.push((start, duration));
    }
    if !saw_silence_event {
        return vec![(0.0, duration)];
    }

    silences.sort_by(|left, right| left.0.total_cmp(&right.0));
    let mut raw_windows = Vec::new();
    let mut cursor: f64 = 0.0;
    for (silence_start, silence_end) in silences {
        if silence_start > cursor {
            raw_windows.push((cursor, silence_start));
        }
        cursor = cursor.max(silence_end);
    }
    if cursor < duration {
        raw_windows.push((cursor, duration));
    }

    let mut windows: Vec<(f64, f64)> = Vec::new();
    for (start, end) in raw_windows {
        if end - start < 0.12 {
            continue;
        }
        let padded_start = (start - 0.08).max(0.0);
        let padded_end = (end + 0.08).min(duration);
        if let Some(previous) = windows.last_mut() {
            if padded_start <= previous.1 {
                previous.1 = previous.1.max(padded_end);
                continue;
            }
        }
        windows.push((padded_start, padded_end));
    }
    windows
}

fn detect_speech_windows(
    audio_path: &std::path::Path,
    duration: f64,
    ffmpeg: &PathBuf,
) -> Option<Vec<(f64, f64)>> {
    let mut command = Command::new(ffmpeg);
    command
        .arg("-hide_banner")
        .arg("-nostats")
        .arg("-i")
        .arg(audio_path)
        .arg("-af")
        .arg("silencedetect=noise=-40dB:d=0.28")
        .arg("-f")
        .arg("null")
        .arg("-")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let windows = speech_windows_from_silence_log(&stderr, duration);
    if windows.len() > 80 {
        log::warn!(
            "[Whisper] Detected {} speech windows; falling back to one clip window",
            windows.len()
        );
        return Some(vec![(0.0, duration)]);
    }
    Some(windows)
}

fn merge_token_pieces(pieces: Vec<TimedTokenPiece>) -> Vec<TranscriptWord> {
    let mut words = Vec::new();
    let mut current_text = String::new();
    let mut current_start = 0.0;
    let mut current_end = 0.0;

    let flush = |words: &mut Vec<TranscriptWord>, text: &mut String, start: f64, end: f64| {
        if !text.is_empty() && end > start {
            words.push(TranscriptWord {
                word: std::mem::take(text),
                start,
                end,
            });
        } else {
            text.clear();
        }
    };

    for piece in pieces {
        let starts_new_word = piece
            .text
            .chars()
            .next()
            .map(char::is_whitespace)
            .unwrap_or(false);
        let text = piece.text.trim();
        if text.is_empty() || text.starts_with("<|") {
            continue;
        }

        if starts_new_word && !current_text.is_empty() {
            flush(&mut words, &mut current_text, current_start, current_end);
        }
        if current_text.is_empty() {
            current_start = piece.start;
            current_end = piece.end;
        } else {
            current_end = current_end.max(piece.end);
        }
        current_text.push_str(text);
    }

    flush(&mut words, &mut current_text, current_start, current_end);
    words
}

fn normalized_word(word: &str) -> String {
    word.chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn distribute_words(words: &[String], start: f64, end: f64) -> Vec<TranscriptWord> {
    if words.is_empty() || end <= start {
        return Vec::new();
    }

    let weights: Vec<usize> = words
        .iter()
        .map(|word| word.chars().filter(|character| character.is_alphanumeric()).count().max(1))
        .collect();
    let total_weight = weights.iter().sum::<usize>().max(1) as f64;
    let mut cursor = start;

    words
        .iter()
        .zip(weights)
        .enumerate()
        .map(|(index, (word, weight))| {
            let word_end = if index + 1 == words.len() {
                end
            } else {
                cursor + (end - start) * weight as f64 / total_weight
            };
            let timed = TranscriptWord {
                word: word.clone(),
                start: cursor,
                end: word_end,
            };
            cursor = word_end;
            timed
        })
        .collect()
}

fn reconcile_segment_words(
    text: &str,
    segment_start: f64,
    segment_end: f64,
    timed_words: Vec<TranscriptWord>,
) -> Vec<TranscriptWord> {
    let text_words: Vec<String> = text.split_whitespace().map(str::to_string).collect();
    if text_words.is_empty() {
        return Vec::new();
    }

    let mut aligned: Vec<Option<TranscriptWord>> = vec![None; text_words.len()];
    let mut search_from = 0;
    for timed_word in timed_words {
        let needle = normalized_word(&timed_word.word);
        if needle.is_empty() {
            continue;
        }
        let Some(relative_index) = text_words[search_from..]
            .iter()
            .position(|word| normalized_word(word) == needle)
        else {
            continue;
        };
        let index = search_from + relative_index;
        aligned[index] = Some(TranscriptWord {
            word: text_words[index].clone(),
            start: timed_word.start,
            end: timed_word.end,
        });
        search_from = index + 1;
        if search_from >= text_words.len() {
            break;
        }
    }

    if aligned.iter().all(Option::is_none) {
        return distribute_words(&text_words, segment_start, segment_end);
    }

    let mut index = 0;
    while index < aligned.len() {
        if aligned[index].is_some() {
            index += 1;
            continue;
        }

        let run_start = index;
        while index < aligned.len() && aligned[index].is_none() {
            index += 1;
        }
        let run_end = index;
        let fill_start = if run_start == 0 {
            segment_start
        } else {
            aligned[run_start - 1]
                .as_ref()
                .map(|word| word.end)
                .unwrap_or(segment_start)
        };
        let fill_end = aligned
            .get(run_end)
            .and_then(Option::as_ref)
            .map(|word| word.start)
            .unwrap_or(segment_end);

        if fill_end <= fill_start {
            return distribute_words(&text_words, segment_start, segment_end);
        }
        for (offset, word) in distribute_words(
            &text_words[run_start..run_end],
            fill_start,
            fill_end,
        )
        .into_iter()
        .enumerate()
        {
            aligned[run_start + offset] = Some(word);
        }
    }

    aligned.into_iter().flatten().collect()
}

fn clamp_segment_to_window(
    mut segment: TranscriptSegment,
    window_start: f64,
    window_end: f64,
) -> Option<TranscriptSegment> {
    let mut words = Vec::new();
    for mut word in segment.words {
        if word.end <= window_start || word.start >= window_end {
            continue;
        }
        word.start = word.start.max(window_start);
        word.end = word.end.min(window_end);
        if word.end > word.start {
            words.push(word);
        }
    }
    if words.is_empty() {
        return None;
    }

    segment.start = words.first()?.start;
    segment.end = words.last()?.end;
    segment.text = words
        .iter()
        .map(|word| word.word.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    segment.words = words;
    Some(segment)
}

fn convert_segment(segment: &WhisperSegment<'_>, time_offset: f64) -> Option<TranscriptSegment> {
    let text = segment.to_str().unwrap_or("").trim().to_string();
    if text.is_empty() {
        return None;
    }

    let start = time_offset + segment.start_timestamp() as f64 / 100.0;
    let end = time_offset + segment.end_timestamp() as f64 / 100.0;
    let mut pieces = Vec::new();
    for token_index in 0..segment.n_tokens() {
        let Some(token) = segment.get_token(token_index) else {
            continue;
        };
        let data = token.token_data();
        if data.t0 < 0 || data.t1 <= data.t0 {
            continue;
        }
        let Ok(token_text) = token.to_str_lossy() else {
            continue;
        };
        pieces.push(TimedTokenPiece {
            text: token_text.into_owned(),
            start: time_offset + data.t0 as f64 / 100.0,
            end: time_offset + data.t1 as f64 / 100.0,
        });
    }

    let words = reconcile_segment_words(&text, start, end, merge_token_pieces(pieces));

    Some(TranscriptSegment {
        start,
        end,
        text,
        words,
    })
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
    transcribe_internal(audio_path, model, use_gpu, false, on_progress)
}

fn transcribe_internal<F>(
    audio_path: &str,
    model: WhisperModel,
    use_gpu: bool,
    align_word_timestamps: bool,
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
    if align_word_timestamps {
        params.dtw_parameters.mode = DtwMode::ModelPreset {
            model_preset: match model {
                WhisperModel::Base => DtwModelPreset::Base,
                WhisperModel::Medium => DtwModelPreset::Medium,
            },
        };
    }
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
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_debug_mode(false);
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
        if let Some(converted) = convert_segment(&segment, 0.0) {
            segments.push(converted);
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

/// Transcribe only one clip range. The temporary audio slice keeps subtitle
/// regeneration fast even when the source VOD is several hours long.
pub fn transcribe_range<F>(
    audio_path: &str,
    start_seconds: f64,
    end_seconds: f64,
    model: WhisperModel,
    use_gpu: bool,
    on_progress: F,
) -> Result<TranscriptResult, String>
where
    F: Fn(u32) + Send + Sync + 'static,
{
    let duration = end_seconds - start_seconds;
    if !start_seconds.is_finite() || !end_seconds.is_finite() || start_seconds < 0.0 || duration <= 0.0 {
        return Err("Invalid clip range for transcription".to_string());
    }

    let ffmpeg = find_ffmpeg()?;
    let temp_path = std::env::temp_dir().join(format!(
        "clipviral-caption-{}.wav",
        uuid::Uuid::new_v4()
    ));
    let speech_stream = preferred_transcription_audio_stream(audio_path);
    let audio_map = speech_stream
        .as_ref()
        .map(|stream| format!("0:{}", stream.index))
        .unwrap_or_else(|| "0:a:0".to_string());
    if let Some(stream) = speech_stream.as_ref() {
        log::info!(
            "[Whisper] Using speech-focused audio stream {} ({}) for clip captions",
            audio_map,
            stream.label
        );
    } else {
        log::debug!(
            "[Whisper] No labeled microphone track found; using default audio stream for clip captions"
        );
    }
    let mut command = Command::new(&ffmpeg);
    command
        .arg("-y")
        .arg("-ss")
        .arg(format!("{start_seconds:.3}"))
        .arg("-i")
        .arg(audio_path)
        .arg("-t")
        .arg(format!("{duration:.3}"))
        .arg("-map")
        .arg(&audio_map)
        .arg("-vn")
        .arg("-ar")
        .arg("16000")
        .arg("-ac")
        .arg("1")
        .arg("-c:a")
        .arg("pcm_s16le")
        .arg(&temp_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    let output = command
        .output()
        .map_err(|e| format!("Failed to extract clip audio: {e}"))?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&temp_path);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail = stderr
            .chars()
            .rev()
            .take(800)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        return Err(format!("Failed to extract clip audio: {}", tail.trim()));
    }

    let result = match temp_path.to_str() {
        Some(path) => {
            let windows = detect_speech_windows(&temp_path, duration, &ffmpeg)
                .unwrap_or_else(|| vec![(0.0, duration)]);
            log::info!(
                "[Whisper] Clip captions: transcribing {} speech window(s) across {:.1}s",
                windows.len(),
                duration
            );
            if windows.is_empty() {
                on_progress(100);
                Ok(TranscriptResult {
                    segments: Vec::new(),
                    language: "en".to_string(),
                    duration,
                })
            } else {
                transcribe_windows_internal(path, &windows, model, use_gpu, true, on_progress)
            }
        }
        None => Err("Temporary caption path has invalid encoding".to_string()),
    };
    let _ = std::fs::remove_file(&temp_path);
    result
}

/// Two-pass transcription: only transcribe specific time windows, not the
/// whole VOD. Used by the analysis pipeline to skip long stretches that
/// the audio + chat + emote pre-selector already determined are not clip-
/// worthy. For a 7h gaming VOD where ~10% of the duration carries signal,
/// this is a ~10× speedup vs full-VOD transcription with identical accuracy
/// on the windows that matter.
///
/// `windows` is a slice of `(start_seconds, end_seconds)` pairs in original
/// VOD time. The returned `TranscriptResult.segments` have timestamps in
/// the SAME original-VOD reference frame (the slice-local timestamps
/// whisper returns are added to each window's start before being recorded),
/// so downstream code that queries the transcript by VOD time works
/// without modification.
///
/// Implementation note: we extract full PCM once and slice in memory rather
/// than re-running ffmpeg per window. The PCM extraction is fast (<1 min
/// for 7h on SSD) and per-window ffmpeg invocations would have >2× the
/// overhead for typical 5-30 window counts. Memory cost: ~1.5 GB for a 7h
/// VOD (16kHz × 4 bytes × 7h × 3600s) — same as the existing single-pass
/// `transcribe` function.
///
/// Falls back gracefully on empty input (returns Err so caller can decide
/// whether to retry with full-VOD transcription).
pub fn transcribe_windows<F>(
    audio_path: &str,
    windows: &[(f64, f64)],
    model: WhisperModel,
    use_gpu: bool,
    on_progress: F,
) -> Result<TranscriptResult, String>
where
    F: Fn(u32) + Send + Sync + 'static,
{
    transcribe_windows_internal(audio_path, windows, model, use_gpu, false, on_progress)
}

fn transcribe_windows_internal<F>(
    audio_path: &str,
    windows: &[(f64, f64)],
    model: WhisperModel,
    use_gpu: bool,
    align_word_timestamps: bool,
    on_progress: F,
) -> Result<TranscriptResult, String>
where
    F: Fn(u32) + Send + Sync + 'static,
{
    if windows.is_empty() {
        return Err("transcribe_windows called with empty window list".to_string());
    }

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

    // 2. Extract full PCM via ffmpeg (single pass, ~1 min for 7h on SSD)
    let ffmpeg = find_ffmpeg()?;
    log::info!(
        "[Whisper] Two-pass: extracting full PCM from {} ({} window(s) to transcribe)",
        audio_path,
        windows.len()
    );
    progress_fn(5);

    let samples = extract_pcm_audio(audio_path, &ffmpeg)?;
    let total_duration = samples.len() as f64 / 16000.0;

    // Compute total samples we'll actually transcribe (sum of window sizes,
    // clipped to actual sample count). This is what we use for the speedup
    // log and as the denominator for progress mapping.
    let total_window_samples: usize = windows
        .iter()
        .map(|&(s, e)| {
            let start_idx = (s * 16000.0).max(0.0) as usize;
            let end_idx = ((e * 16000.0) as usize).min(samples.len());
            end_idx.saturating_sub(start_idx)
        })
        .sum();

    let coverage_pct = if !samples.is_empty() {
        (total_window_samples as f64 / samples.len() as f64) * 100.0
    } else {
        0.0
    };
    log::info!(
        "[Whisper] Two-pass: transcribing {:.1}s of {:.1}s total ({:.1}% coverage, {:.1}% reduction vs full-VOD)",
        total_window_samples as f64 / 16000.0,
        total_duration,
        coverage_pct,
        100.0 - coverage_pct,
    );
    progress_fn(15);

    // 3. Load whisper model ONCE — reuse across all windows
    log::info!(
        "[Whisper] Loading model: {} (GPU={})",
        mpath.display(),
        if use_gpu { "enabled" } else { "disabled" },
    );
    let mut ctx_params = WhisperContextParameters::default();
    ctx_params.use_gpu = use_gpu;
    if align_word_timestamps {
        ctx_params.dtw_parameters.mode = DtwMode::ModelPreset {
            model_preset: match model {
                WhisperModel::Base => DtwModelPreset::Base,
                WhisperModel::Medium => DtwModelPreset::Medium,
            },
        };
    }
    let ctx = WhisperContext::new_with_params(
        mpath.to_str().ok_or("Invalid model path encoding")?,
        ctx_params,
    )
    .map_err(|e| format!("Failed to load whisper model: {}", e))?;

    progress_fn(20);

    // 4. Run inference per window, mapping each segment's timestamps back
    //    into the original-VOD reference frame.
    let mut all_segments: Vec<TranscriptSegment> = Vec::new();
    let mut samples_processed: usize = 0;

    for (i, &(window_start_secs, window_end_secs)) in windows.iter().enumerate() {
        let start_idx = (window_start_secs * 16000.0).max(0.0) as usize;
        let end_idx = ((window_end_secs * 16000.0) as usize).min(samples.len());
        if start_idx >= end_idx {
            log::warn!(
                "[Whisper] Window {}/{} ({:.1}-{:.1}s) is empty after sample mapping, skipping",
                i + 1,
                windows.len(),
                window_start_secs,
                window_end_secs
            );
            continue;
        }

        let slice = &samples[start_idx..end_idx];
        log::debug!(
            "[Whisper] Window {}/{}: {:.1}-{:.1}s ({} samples)",
            i + 1,
            windows.len(),
            window_start_secs,
            window_end_secs,
            slice.len()
        );

        // Per-window inference params — set fresh each time because progress
        // callback closure captures the running totals at call time.
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("en"));
        params.set_token_timestamps(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_debug_mode(false);
        params.set_n_threads(optimal_thread_count());

        // Map per-window progress to the overall (20-95%) range, weighted by
        // how many samples this window represents in the total transcription
        // workload. So the bar moves in proportion to actual work done, not
        // window count (a 60s window contributes 6× as much as a 10s window).
        let progress_fn_clone = Arc::clone(&progress_fn);
        let samples_so_far = samples_processed;
        let slice_size = slice.len();
        let total_size = total_window_samples.max(1);
        params.set_progress_callback_safe(move |inner_progress: i32| {
            let inner_done = (slice_size * inner_progress.max(0) as usize) / 100;
            let overall_frac = (samples_so_far + inner_done) as f64 / total_size as f64;
            let mapped = 20 + (overall_frac * 75.0) as u32;
            progress_fn_clone(mapped.min(95));
        });

        let mut state = ctx
            .create_state()
            .map_err(|e| format!("Failed to create whisper state: {}", e))?;

        state
            .full(params, slice)
            .map_err(|e| format!("Whisper inference failed on window {}: {}", i + 1, e))?;

        // Map slice-local segment timestamps back to original VOD time.
        let num_segments = state.full_n_segments();
        for j in 0..num_segments {
            if let Some(segment) = state.get_segment(j) {
                if let Some(converted) = convert_segment(&segment, window_start_secs) {
                    if let Some(clamped) = clamp_segment_to_window(
                        converted,
                        window_start_secs,
                        window_end_secs,
                    ) {
                        all_segments.push(clamped);
                    }
                }
            }
        }

        samples_processed += slice.len();
    }

    progress_fn(100);

    log::info!(
        "[Whisper] Two-pass transcription complete: {} segments across {} windows ({:.1}s of audio)",
        all_segments.len(),
        windows.len(),
        total_window_samples as f64 / 16000.0,
    );

    Ok(TranscriptResult {
        segments: all_segments,
        language: "en".to_string(),
        duration: total_duration,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_whisper_subword_tokens_into_timed_words() {
        let words = merge_token_pieces(vec![
            TimedTokenPiece { text: " Not".into(), start: 1.0, end: 1.2 },
            TimedTokenPiece { text: " Sta".into(), start: 1.2, end: 1.4 },
            TimedTokenPiece { text: "cie".into(), start: 1.4, end: 1.6 },
            TimedTokenPiece { text: " stabbing".into(), start: 1.6, end: 2.0 },
            TimedTokenPiece { text: ".".into(), start: 2.0, end: 2.1 },
        ]);

        assert_eq!(words.len(), 3);
        assert_eq!(words[0].word, "Not");
        assert_eq!(words[1].word, "Stacie");
        assert_eq!(words[1].start, 1.2);
        assert_eq!(words[1].end, 1.6);
        assert_eq!(words[2].word, "stabbing.");
    }

    #[test]
    fn restores_recognized_words_that_lack_token_timestamps() {
        let words = reconcile_segment_words(
            "I got a quick proc",
            0.0,
            1.4,
            vec![
                TranscriptWord { word: "I".into(), start: 0.0, end: 0.1 },
                TranscriptWord { word: "quick".into(), start: 0.6, end: 1.0 },
                TranscriptWord { word: "proc".into(), start: 1.0, end: 1.4 },
            ],
        );

        assert_eq!(
            words.iter().map(|word| word.word.as_str()).collect::<Vec<_>>(),
            vec!["I", "got", "a", "quick", "proc"]
        );
        assert!(words[1].start >= words[0].end);
        assert!(words[2].end <= words[3].start);
    }

    #[test]
    fn drops_words_that_whisper_stretches_past_a_speech_window() {
        let segment = TranscriptSegment {
            start: 8.6,
            end: 10.5,
            text: "We're not in the".into(),
            words: vec![
                TranscriptWord { word: "We're".into(), start: 8.6, end: 9.1 },
                TranscriptWord { word: "not".into(), start: 9.1, end: 9.7 },
                TranscriptWord { word: "in".into(), start: 9.7, end: 10.0 },
            ],
        };
        let clamped = clamp_segment_to_window(segment, 8.5, 9.3).unwrap();

        assert_eq!(
            clamped.words.iter().map(|word| word.word.as_str()).collect::<Vec<_>>(),
            vec!["We're", "not"]
        );
        assert_eq!(clamped.words[1].end, 9.3);
        assert_eq!(clamped.text, "We're not");
    }

    #[test]
    fn turns_long_silences_into_separate_speech_windows() {
        let log = r#"
            silence_start: 0.0
            silence_end: 12.221 | silence_duration: 12.221
            silence_start: 14.645
            silence_end: 25.993 | silence_duration: 11.348
            silence_start: 26.639
        "#;
        let windows = speech_windows_from_silence_log(log, 30.0);

        assert_eq!(windows.len(), 2);
        assert!((windows[0].0 - 12.141).abs() < 0.001);
        assert!((windows[0].1 - 14.725).abs() < 0.001);
        assert!((windows[1].0 - 25.913).abs() < 0.001);
        assert!((windows[1].1 - 26.719).abs() < 0.001);
    }

    #[test]
    fn handles_fully_active_and_fully_silent_audio() {
        assert_eq!(
            speech_windows_from_silence_log("", 5.0),
            vec![(0.0, 5.0)]
        );
        assert!(speech_windows_from_silence_log("silence_start: 0.0", 5.0).is_empty());
    }

    #[test]
    fn selects_medal_microphone_track_over_mixed_audio() {
        let probe = r#"{
            "streams": [
                {"index": 1, "tags": {"name": "All Audio"}},
                {"index": 2, "tags": {"name": "All PC Audio"}},
                {"index": 3, "tags": {"name": "Microphone"}}
            ]
        }"#;

        assert_eq!(
            select_microphone_audio_stream(probe),
            Some(SpeechAudioStream {
                index: 3,
                label: "Microphone".to_string(),
            })
        );
    }

    #[test]
    fn recognizes_common_obs_microphone_labels() {
        let probe = r#"{
            "streams": [
                {"index": 1, "tags": {"title": "Desktop Audio"}},
                {"index": 4, "tags": {"handler_name": "Mic/Aux"}}
            ]
        }"#;

        assert_eq!(select_microphone_audio_stream(probe).unwrap().index, 4);
    }

    #[test]
    fn falls_back_when_probe_has_no_labeled_microphone() {
        let probe = r#"{
            "streams": [
                {"index": 1, "tags": {"name": "All Audio"}},
                {"index": 2}
            ]
        }"#;

        assert_eq!(select_microphone_audio_stream(probe), None);
        assert_eq!(select_microphone_audio_stream("not json"), None);
    }

}
