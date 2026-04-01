//! Main orchestration engine for highlight analysis.
//!
//! This is the single entry point the frontend calls to analyze a VOD.
//! It wires together every pipeline module in the correct order,
//! reports progress at each stage, and handles fallbacks gracefully.
//!
//! # Execution flow
//!
//! ```text
//!  ┌──────────────────────────────────────────────────────────┐
//!  │  analyze_vod()                                           │
//!  │                                                          │
//!  │   0%   Validate inputs, resolve ffmpeg                   │
//!  │   5%   ┌─ Stage 1: Local signal extraction (parallel) ─┐│
//!  │        │  audio_signal::analyze()                       ││
//!  │        │  run_transcription() → transcript_signal       ││
//!  │        │  scene_signal::analyze()                       ││
//!  │  40%   └────────────────────────────────────────────────┘│
//!  │  45%   Stage 2: Signal fusion → CandidateClip[]          │
//!  │  70%   Stage 3: Ranking                                  │
//!  │  75%   Stage 4: Labeling                                 │
//!  │  85%   Stage 5: Thumbnail generation                     │
//!  │  95%   Stage 6: Final assembly                           │
//!  │ 100%   Return AnalysisResult                             │
//!  └──────────────────────────────────────────────────────────┘
//! ```
//!
//! # Fallback behavior
//!
//! | Failure                  | Effect                                 |
//! |--------------------------|----------------------------------------|
//! | ffmpeg not found         | Hard error — cannot proceed            |
//! | Audio extraction fails   | Warning, continue without audio signal |
//! | Transcription fails      | Warning, continue without speech       |
//! | Scene detection fails    | Warning, continue without scene signal |
//! | _(no external APIs used)_ | _(N/A — analysis is fully local)_      |
//! | Thumbnail extraction     | Clip returned without thumbnail        |
//! | Zero signals detected    | Return empty result with warning       |

use std::path::{Path, PathBuf};

use crate::audio_signal::{self, AudioProfile};
use crate::clip_fusion::{self, FusionConfig};
use crate::clip_labeler;
use crate::clip_output::{self, OutputConfig};
use crate::clip_ranker::{self, RankedClip, ScoringConfig};
use crate::error::AppError;
use crate::hardware::HardwareInfo;
use crate::pipeline::{AnalysisMode, CandidateClip, SignalSegment};
use crate::scene_signal::{self, SceneDetection};
use crate::transcript_signal::{self, InputKeyword, InputSegment, TranscriptInput};
use crate::commands::vod::{run_transcription, find_ffmpeg};

// ═══════════════════════════════════════════════════════════════════
//  Configuration
// ═══════════════════════════════════════════════════════════════════

/// Everything the engine needs to run an analysis.
#[derive(Debug, Clone)]
pub struct AnalysisRequest {
    /// Path to the downloaded VOD file.
    pub vod_path: String,
    /// VOD / stream title (used in labels).
    pub vod_title: String,
    /// Total duration in seconds.
    pub duration_secs: f64,
    /// Analysis mode (always local).
    pub mode: AnalysisMode,
    /// Hardware profile for CUDA decisions.
    pub hardware: HardwareInfo,
    /// How many clips to return (default 5).
    pub top_n: usize,
}

/// The complete result of an analysis run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AnalysisResult {
    /// Final ranked clips with titles, scores, and thumbnails.
    pub clips: Vec<CandidateClip>,
    /// Ranked clips with full scoring explanations (superset of `clips`).
    pub ranked: Vec<RankedClip>,
    /// Warnings accumulated during analysis (non-fatal issues).
    pub warnings: Vec<String>,
    /// Which signals actually produced data.
    pub signals_used: Vec<String>,
    /// Total number of raw signal segments detected.
    pub total_signals: usize,
}

// ═══════════════════════════════════════════════════════════════════
//  Progress reporting
// ═══════════════════════════════════════════════════════════════════

/// A callback that receives progress updates (0–100).
///
/// The engine calls this at each stage transition.  Implementations
/// can forward to [`JobHandle::set_progress`], a channel, or a no-op.
pub type ProgressFn = Box<dyn Fn(u8) + Send + Sync>;

/// No-op progress callback (for testing / CLI usage).
pub fn no_progress() -> ProgressFn {
    Box::new(|_| {})
}

// ═══════════════════════════════════════════════════════════════════
//  Main entry point
// ═══════════════════════════════════════════════════════════════════

/// Analyze a VOD and return ranked highlight clips.
///
/// This is the function the Tauri command layer calls.  It runs
/// the full pipeline: signal extraction → fusion → ranking
/// → labeling → thumbnails.  All processing is local.
///
/// # Arguments
///
/// * `req`      – analysis parameters (VOD path, mode, hardware, etc.)
/// * `progress` – callback for progress updates (0–100)
///
/// # Errors
///
/// Returns `AppError::Ffmpeg` if ffmpeg cannot be found (hard requirement).
/// All other failures are logged as warnings and the pipeline continues
/// with whatever signals succeeded.
pub async fn analyze_vod(
    req: &AnalysisRequest,
    progress: &ProgressFn,
) -> Result<AnalysisResult, AppError> {
    let mut warnings: Vec<String> = Vec::new();
    let mut signals_used: Vec<String> = Vec::new();

    progress(0);

    // ── Stage 0: Resolve ffmpeg (hard requirement) ──
    let ffmpeg = find_ffmpeg_path()?;
    progress(5);

    // ── Stage 1: Local signal extraction (parallel via spawn_blocking) ──
    let vod_path = req.vod_path.clone();
    let hw = req.hardware.clone();
    let ffmpeg_clone = ffmpeg.clone();

    let (audio_result, transcript_result, scene_result) = run_local_signals(
        &vod_path, &ffmpeg_clone, &hw, progress,
    ).await;

    // Unpack audio — needed later for thumbnails and frame sampling
    let audio_profile: Option<AudioProfile> = match audio_result {
        Ok(profile) => {
            signals_used.push("audio".into());
            Some(profile)
        }
        Err(e) => {
            let msg = format!("Audio analysis failed: {e}");
            log::warn!("{msg}");
            warnings.push(msg);
            None
        }
    };

    // Unpack transcript
    let transcript_input: Option<TranscriptInput> = match transcript_result {
        Ok(input) => {
            signals_used.push("transcript".into());
            Some(input)
        }
        Err(e) => {
            let msg = format!("Transcription failed: {e}");
            log::warn!("{msg}");
            warnings.push(msg);
            None
        }
    };

    // Unpack scene detection
    let (scene_cuts, motion_profile) = match scene_result {
        Ok((cuts, motion)) => {
            signals_used.push("scene".into());
            (cuts, Some(motion))
        }
        Err(e) => {
            let msg = format!("Scene detection failed: {e}");
            log::warn!("{msg}");
            warnings.push(msg);
            (Vec::new(), None)
        }
    };

    progress(40);

    // ── Build signal segments from each source ──
    let mut all_segments: Vec<SignalSegment> = Vec::new();

    if let Some(ref profile) = audio_profile {
        all_segments.extend(audio_signal::detect_signals(profile));
    }

    if let Some(ref input) = transcript_input {
        all_segments.extend(transcript_signal::analyze(input));
    }

    // Scene segments from raw detections + motion
    let motion = motion_profile.unwrap_or_else(|| scene_signal::MotionProfile::from_energy(vec![]));
    let scene_segments = scene_signal::detect_signals(&scene_cuts, &motion);
    all_segments.extend(scene_segments);

    let total_signals = all_segments.len();
    log::info!("Stage 1 complete: {} total signal segments", total_signals);

    if all_segments.is_empty() {
        warnings.push("No signals detected in the VOD".into());
        progress(100);
        return Ok(AnalysisResult {
            clips: Vec::new(),
            ranked: Vec::new(),
            warnings,
            signals_used,
            total_signals: 0,
        });
    }

    // ── Stage 2: Signal fusion ──
    //
    // max_clips flows from req.top_n through every stage:
    //   fusion: top_n × 4 (generous pool for ranking diversity)
    //   ranker: top_n × 2 (diversity selection needs headroom)
    //   output: top_n      (final user-facing cap)
    let max_clips = req.top_n;
    let scoring = ScoringConfig::for_mode(&req.mode);
    let fusion_config = FusionConfig {
        max_candidates: max_clips * 4,
        // With only 3 local signal types, strong moments from one source
        // (e.g. a loud reaction with no keywords) are still valuable.
        single_source_min: 0.35,
        ..FusionConfig::new(req.duration_secs)
    };
    let candidates = clip_fusion::fuse(&all_segments, &fusion_config);
    log::info!("Stage 2: {} candidate clips after fusion", candidates.len());
    progress(70);

    // ── Stage 3: Ranking (max_clips × 2 for diversity headroom) ──
    let ranked = clip_ranker::rank(&candidates, &scoring, max_clips * 2);
    log::info!("Stage 3: {} clips after ranking", ranked.len());
    progress(75);

    // ── Stage 4: Labeling ──
    //
    // The ranker already set score_report (with explanation, dimensions,
    // bonuses, penalties).  The labeler adds the user-facing title and hook.
    let ranked: Vec<RankedClip> = ranked
        .into_iter()
        .map(|mut r| {
            clip_labeler::label_clip(&mut r.clip);
            r.clip.event_summary = Some(crate::post_captions::generate_event_summary(&r.clip));
            r.clip.post_captions = Some(crate::post_captions::generate(&r.clip));
            r
        })
        .collect();
    progress(80);

    // ── Stage 5: Thumbnails (final cap = max_clips) ──
    let output_config = OutputConfig::with_default_dir();

    let final_clips = match clip_output::finalize(
        &ranked,
        &req.vod_path,
        &ffmpeg,
        audio_profile.as_ref(),
        &scene_cuts,
        &output_config,
        max_clips,
    ) {
        Ok(clips) => clips,
        Err(e) => {
            let msg = format!("Thumbnail generation failed: {e}");
            log::warn!("{msg}");
            warnings.push(msg);
            clip_output::finalize_without_thumbnails(&ranked, max_clips)
        }
    };
    progress(95);

    // ── Stage 6: Done ──
    log::info!(
        "Analysis complete: {} clips (local, signals={})",
        final_clips.len(),
        signals_used.join("+"),
    );
    progress(100);

    Ok(AnalysisResult {
        clips: final_clips,
        ranked,
        warnings,
        signals_used,
        total_signals,
    })
}

// ═══════════════════════════════════════════════════════════════════
//  Stage 1: Local signal extraction
// ═══════════════════════════════════════════════════════════════════

/// Run audio, transcript, and scene analysis concurrently.
///
/// Each runs in its own `spawn_blocking` task since they're all
/// CPU-bound (ffmpeg / Python subprocesses).
async fn run_local_signals(
    vod_path: &str,
    ffmpeg: &Path,
    hw: &HardwareInfo,
    progress: &ProgressFn,
) -> (
    Result<AudioProfile, AppError>,
    Result<TranscriptInput, AppError>,
    Result<(Vec<SceneDetection>, scene_signal::MotionProfile), AppError>,
) {
    let vod = vod_path.to_string();
    let ff = ffmpeg.to_path_buf();
    let hw_clone = hw.clone();

    // Audio extraction
    let vod_a = vod.clone();
    let ff_a = ff.clone();
    let audio_handle = tokio::task::spawn_blocking(move || {
        audio_signal::extract_rms(&vod_a, &ff_a)
    });

    // Transcription
    let vod_t = vod.clone();
    let transcript_handle = tokio::task::spawn_blocking(move || {
        run_and_convert_transcript(&vod_t, &hw_clone)
    });

    // Scene detection (two ffmpeg passes)
    let vod_s = vod.clone();
    let ff_s = ff.clone();
    let scene_handle = tokio::task::spawn_blocking(move || {
        let cuts = scene_signal::extract_scene_changes(&vod_s, &ff_s)?;
        let motion = scene_signal::extract_motion_energy(&vod_s, &ff_s)?;
        Ok((cuts, motion))
    });

    progress(10);

    // Await all three (order doesn't matter, they run concurrently)
    let audio_result = audio_handle
        .await
        .unwrap_or_else(|e| Err(AppError::Unknown(format!("Audio task panic: {e}"))));

    progress(20);

    let transcript_result = transcript_handle
        .await
        .unwrap_or_else(|e| Err(AppError::Unknown(format!("Transcript task panic: {e}"))));

    progress(30);

    let scene_result = scene_handle
        .await
        .unwrap_or_else(|e| Err(AppError::Unknown(format!("Scene task panic: {e}"))));

    progress(35);

    (audio_result, transcript_result, scene_result)
}

/// Run the existing transcription pipeline and convert its output
/// to the `TranscriptInput` type expected by `transcript_signal`.
fn run_and_convert_transcript(
    vod_path: &str,
    hw: &HardwareInfo,
) -> Result<TranscriptInput, AppError> {
    // Check for cached transcript first
    let transcript_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("clipviral")
        .join("transcripts");
    std::fs::create_dir_all(&transcript_dir).ok();

    // Use a hash of the vod path as the cache key
    let vod_hash = format!("{:x}", md5_of_path(vod_path));
    let cache_path = transcript_dir.join(format!("{vod_hash}.json"));
    let cache_str = cache_path.to_string_lossy().to_string();

    // Try cache
    if cache_path.exists() {
        if let Ok(json) = std::fs::read_to_string(&cache_path) {
            if let Ok(result) = serde_json::from_str::<TranscriptResultCompat>(&json) {
                log::info!("Using cached transcript ({} segments)", result.segments.len());
                return Ok(convert_transcript(result));
            }
        }
    }

    // Run fresh transcription
    run_transcription(vod_path, &cache_str, hw, None)
        .map(|r| convert_transcript(TranscriptResultCompat {
            segments: r.segments.iter().map(|s| SegCompat {
                start: s.start,
                end: s.end,
                text: s.text.clone(),
            }).collect(),
            keywords_found: r.keywords_found.iter().map(|k| KwCompat {
                keyword: k.keyword.clone(),
                timestamp: k.timestamp,
                end_timestamp: k.end_timestamp,
                context: k.context.clone(),
            }).collect(),
            language: r.language.clone(),
        }))
}

// Minimal compat structs to avoid depending on lib.rs's private types
#[derive(serde::Deserialize)]
struct TranscriptResultCompat {
    segments: Vec<SegCompat>,
    #[serde(default)]
    keywords_found: Vec<KwCompat>,
    #[serde(default)]
    language: String,
}

#[derive(serde::Deserialize)]
struct SegCompat {
    start: f64,
    end: f64,
    text: String,
}

#[derive(serde::Deserialize)]
struct KwCompat {
    keyword: String,
    timestamp: f64,
    end_timestamp: f64,
    context: String,
}

fn convert_transcript(r: TranscriptResultCompat) -> TranscriptInput {
    TranscriptInput {
        segments: r.segments.into_iter().map(|s| InputSegment {
            start: s.start,
            end: s.end,
            text: s.text,
        }).collect(),
        keywords: r.keywords_found.into_iter().map(|k| InputKeyword {
            keyword: k.keyword,
            start: k.timestamp,
            end: k.end_timestamp,
            context: k.context,
        }).collect(),
        language: if r.language.is_empty() { "en".into() } else { r.language },
    }
}

/// Simple FNV-1a path hasher for cache keys (not cryptographic).
/// Collision risk is acceptable here — keys are short file paths and
/// a collision only causes a redundant cache miss, not data corruption.
fn md5_of_path(path: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325; // FNV offset basis
    for byte in path.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3); // FNV prime
    }
    hash
}

// ═══════════════════════════════════════════════════════════════════
//  Helpers
// ═══════════════════════════════════════════════════════════════════
/// Resolve the ffmpeg binary path (hard requirement).
fn find_ffmpeg_path() -> Result<PathBuf, AppError> {
    find_ffmpeg()
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_of_path_stable() {
        let a = md5_of_path("/some/path/video.mp4");
        let b = md5_of_path("/some/path/video.mp4");
        assert_eq!(a, b);
    }

    #[test]
    fn md5_of_path_different_inputs() {
        let a = md5_of_path("/path/a.mp4");
        let b = md5_of_path("/path/b.mp4");
        assert_ne!(a, b);
    }

    #[test]
    fn convert_transcript_maps_fields() {
        let compat = TranscriptResultCompat {
            segments: vec![SegCompat {
                start: 1.0,
                end: 3.0,
                text: "hello world".into(),
            }],
            keywords_found: vec![KwCompat {
                keyword: "hello".into(),
                timestamp: 1.0,
                end_timestamp: 2.0,
                context: "hello world".into(),
            }],
            language: "en".into(),
        };

        let input = convert_transcript(compat);
        assert_eq!(input.segments.len(), 1);
        assert_eq!(input.keywords.len(), 1);
        assert_eq!(input.language, "en");
        assert!((input.segments[0].start - 1.0).abs() < f64::EPSILON);
        assert_eq!(input.keywords[0].keyword, "hello");
    }

    #[test]
    fn convert_transcript_defaults_language() {
        let compat = TranscriptResultCompat {
            segments: vec![],
            keywords_found: vec![],
            language: "".into(),
        };
        let input = convert_transcript(compat);
        assert_eq!(input.language, "en");
    }

    #[test]
    fn analysis_request_has_sensible_defaults() {
        let req = AnalysisRequest {
            vod_path: "/tmp/test.mp4".into(),
            vod_title: "Test Stream".into(),
            duration_secs: 3600.0,
            mode: AnalysisMode::local(),
            hardware: HardwareInfo::cpu_only(),
            top_n: 5,
        };
        assert!(req.mode.is_local_only());
        assert_eq!(req.top_n, 5);
    }

    #[test]
    fn no_progress_callback_does_not_panic() {
        let p = no_progress();
        p(0);
        p(50);
        p(100);
    }
}
