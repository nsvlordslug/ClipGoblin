//! Final output stage: thumbnail generation and result packaging.
//!
//! Takes ranked clips from the scoring/labeling stages and produces
//! polished results for the frontend:
//!
//! 1. Choose a visually meaningful timestamp for each clip's thumbnail
//! 2. Extract thumbnail frames via ffmpeg
//! 3. Return the final `Vec<CandidateClip>` with thumbnail paths set
//!
//! # Thumbnail timestamp strategy
//!
//! Rather than grabbing a frame at the clip's start (which may be
//! dead air or a loading screen), the module picks a timestamp
//! using signal data:
//!
//! ```text
//!  Clip:  ├──────────────────────────────────────────┤
//!  Try 1: ↓ audio peak (loudest second in the clip)
//!  Try 2: ↓ scene change (most dramatic visual cut)
//!  Try 3: ↓ midpoint (center of the clip)
//!  Try 4: ↓ start + 2s (just past the opening)
//! ```
//!
//! Each candidate timestamp is tried in order.  If ffmpeg produces
//! a frame that's too small (< 3 KB, likely a black frame), the
//! next candidate is tried.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::audio_signal::AudioProfile;
use crate::clip_ranker::RankedClip;
use crate::error::AppError;
use crate::pipeline::CandidateClip;
use crate::scene_signal::SceneDetection;

// ═══════════════════════════════════════════════════════════════════
//  Configuration
// ═══════════════════════════════════════════════════════════════════

/// Output stage settings.
///
/// Note: clip count is **not** configured here — it's the `max_clips`
/// parameter on [`finalize`], flowing from the single source of truth
/// in [`crate::engine::AnalysisRequest::top_n`].
#[derive(Debug, Clone)]
pub struct OutputConfig {
    /// Thumbnail width in pixels (height auto-scaled).
    pub thumb_width: u32,
    /// JPEG quality for thumbnails (1 = worst, 31 = best for ffmpeg).
    pub thumb_quality: u32,
    /// Minimum thumbnail file size in bytes.  Frames smaller than
    /// this are assumed to be black/corrupt and retried.
    pub min_thumb_bytes: u64,
    /// Directory to write thumbnails into.
    pub thumb_dir: PathBuf,
}

impl OutputConfig {
    pub fn new(thumb_dir: PathBuf) -> Self {
        Self {
            thumb_width: 640,
            thumb_quality: 5,
            min_thumb_bytes: 3_000,
            thumb_dir,
        }
    }

    /// Use the platform-standard data directory.
    pub fn with_default_dir() -> Self {
        let dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("clipviral")
            .join("thumbnails");
        Self::new(dir)
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Public API
// ═══════════════════════════════════════════════════════════════════

/// Produce the final frontend-ready clip list.
///
/// Takes ranked clips, generates thumbnails for the top N, and
/// returns the clips with `preview_thumbnail_path` populated.
///
/// Signal data (audio profile, scene cuts) is used to pick the
/// most visually interesting thumbnail timestamp per clip.
///
/// # Errors
///
/// Returns `AppError::Ffmpeg` only if ffmpeg cannot be found.
/// Individual thumbnail failures are silently skipped — the clip
/// is returned without a thumbnail rather than failing the batch.
pub fn finalize(
    ranked: &[RankedClip],
    vod_path: &str,
    ffmpeg: &Path,
    audio: Option<&AudioProfile>,
    scene_cuts: &[SceneDetection],
    config: &OutputConfig,
    max_clips: usize,
) -> Result<Vec<CandidateClip>, AppError> {
    std::fs::create_dir_all(&config.thumb_dir)
        .map_err(|e| AppError::Ffmpeg(format!("Create thumbnail dir: {e}")))?;

    let top = ranked.iter().take(max_clips);

    let clips: Vec<CandidateClip> = top
        .map(|r| {
            let mut clip = r.clip.clone();

            // Pick the best thumbnail timestamp and extract the frame
            let ts_candidates = pick_thumbnail_timestamps(&clip, audio, scene_cuts);
            let thumb_path = config.thumb_dir.join(format!("{}.jpg", clip.id));

            if let Some(_) = try_extract_thumbnail(
                ffmpeg,
                vod_path,
                &ts_candidates,
                &thumb_path,
                config,
            ) {
                clip.preview_thumbnail_path = Some(thumb_path.to_string_lossy().into_owned());
            }

            clip
        })
        .collect();

    log::info!(
        "Output: {} clips finalized, {} with thumbnails",
        clips.len(),
        clips.iter().filter(|c| c.preview_thumbnail_path.is_some()).count(),
    );

    Ok(clips)
}

/// Lightweight variant that skips thumbnail generation.
///
/// Useful for dry runs, testing, or when ffmpeg is not available.
pub fn finalize_without_thumbnails(
    ranked: &[RankedClip],
    max_clips: usize,
) -> Vec<CandidateClip> {
    ranked
        .iter()
        .take(max_clips)
        .map(|r| r.clip.clone())
        .collect()
}

// ═══════════════════════════════════════════════════════════════════
//  Thumbnail timestamp selection
// ═══════════════════════════════════════════════════════════════════

/// Choose candidate timestamps for the thumbnail, ordered by visual
/// interestingness.
///
/// Returns up to 4 timestamps.  The caller tries each in order,
/// stopping at the first one that produces a valid (non-black) frame.
pub fn pick_thumbnail_timestamps(
    clip: &CandidateClip,
    audio: Option<&AudioProfile>,
    scene_cuts: &[SceneDetection],
) -> Vec<f64> {
    let start = clip.start_time;
    let end = clip.end_time;
    let dur = clip.duration();
    let mid = start + dur / 2.0;

    let mut candidates: Vec<f64> = Vec::with_capacity(4);

    // 1. Audio peak — the loudest second in the clip (most likely to
    //    show an interesting reaction on screen)
    if let Some(audio) = audio {
        let s = start as usize;
        let e = (end as usize).min(audio.rms.len());
        if e > s {
            let (peak_offset, _) = audio.rms[s..e]
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap_or((0, &0.0));
            candidates.push((s + peak_offset) as f64 + 0.5);
        }
    }

    // 2. Most dramatic scene change within the clip
    if let Some(best_cut) = scene_cuts
        .iter()
        .filter(|c| c.time >= start && c.time <= end)
        .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal))
    {
        // Grab the frame just after the cut (the new scene)
        let after_cut = (best_cut.time + 0.3).min(end);
        if !candidates.iter().any(|&t| (t - after_cut).abs() < 2.0) {
            candidates.push(after_cut);
        }
    }

    // 3. Midpoint — generic but usually safe
    if !candidates.iter().any(|&t| (t - mid).abs() < 2.0) {
        candidates.push(mid);
    }

    // 4. Start + 2s — the "hook" frame (what the viewer sees first)
    let hook = (start + 2.0).min(end);
    if !candidates.iter().any(|&t| (t - hook).abs() < 2.0) {
        candidates.push(hook);
    }

    // Clamp all to clip bounds
    for t in &mut candidates {
        *t = t.clamp(start, end);
    }

    candidates
}

// ═══════════════════════════════════════════════════════════════════
//  Thumbnail extraction
// ═══════════════════════════════════════════════════════════════════

/// Try each candidate timestamp until one produces a valid thumbnail.
///
/// Returns the timestamp that succeeded, or `None` if all failed.
fn try_extract_thumbnail(
    ffmpeg: &Path,
    vod_path: &str,
    timestamps: &[f64],
    output_path: &Path,
    config: &OutputConfig,
) -> Option<f64> {
    for &ts in timestamps {
        if extract_single_frame(ffmpeg, vod_path, ts, output_path, config).is_ok() {
            let size = std::fs::metadata(output_path)
                .map(|m| m.len())
                .unwrap_or(0);
            if size >= config.min_thumb_bytes {
                return Some(ts);
            }
            // Frame too small (likely black) — try the next candidate
        }
    }

    // Last resort: accept whatever the last attempt produced, even if small
    if output_path.exists() {
        return timestamps.last().copied();
    }

    None
}

/// Extract a single JPEG frame from a video at a specific timestamp.
fn extract_single_frame(
    ffmpeg: &Path,
    vod_path: &str,
    timestamp: f64,
    output_path: &Path,
    config: &OutputConfig,
) -> Result<(), AppError> {
    let mut cmd = Command::new(ffmpeg);
    cmd.arg("-ss")
        .arg(format!("{:.3}", timestamp))
        .arg("-i")
        .arg(vod_path)
        .arg("-vframes")
        .arg("1")
        .arg("-vf")
        .arg(format!("scale={}:-1", config.thumb_width))
        .arg("-q:v")
        .arg(config.thumb_quality.to_string())
        .arg("-y")
        .arg(output_path.to_str().unwrap_or_default())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let status = cmd
        .status()
        .map_err(|e| AppError::Ffmpeg(format!("Thumbnail extraction failed: {e}")))?;

    // ffmpeg sometimes returns non-zero but still writes a valid frame
    if output_path.exists() && std::fs::metadata(output_path).map(|m| m.len() > 0).unwrap_or(false)
    {
        Ok(())
    } else if status.success() {
        Ok(())
    } else {
        Err(AppError::Ffmpeg("Thumbnail frame is empty".into()))
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{ClipScoreBreakdown, SignalType};

    fn make_clip(id: &str, start: f64, end: f64) -> CandidateClip {
        let mut c = CandidateClip::new(
            start,
            end,
            ClipScoreBreakdown::new(0.8, 0.6, 0.4, None),
            vec![SignalType::Audio, SignalType::Transcript],
        );
        c.id = id.into();
        c.tags = vec!["reaction".into()];
        c.title = Some("Test Clip".into());
        c
    }

    fn make_ranked(id: &str, start: f64, end: f64, rank: usize) -> RankedClip {
        RankedClip {
            rank,
            clip: make_clip(id, start, end),
        }
    }

    // ── Timestamp selection ──

    #[test]
    fn audio_peak_is_first_candidate() {
        let clip = make_clip("a", 10.0, 40.0);
        let mut rms = vec![0.3; 50];
        rms[25] = 0.95; // peak at second 25
        let audio = AudioProfile::from_rms(rms);

        let ts = pick_thumbnail_timestamps(&clip, Some(&audio), &[]);
        assert!(!ts.is_empty());
        // First candidate should be near the audio peak
        assert!(
            (ts[0] - 25.5).abs() < 1.5,
            "expected ~25.5, got {}", ts[0]
        );
    }

    #[test]
    fn scene_cut_included_as_candidate() {
        let clip = make_clip("b", 10.0, 40.0);
        let cuts = vec![SceneDetection { time: 22.0, score: 0.8 }];

        let ts = pick_thumbnail_timestamps(&clip, None, &cuts);
        // Should include a timestamp just after the cut
        assert!(
            ts.iter().any(|&t| (t - 22.3).abs() < 1.0),
            "timestamps {:?} should include scene cut at 22.3", ts
        );
    }

    #[test]
    fn midpoint_included() {
        let clip = make_clip("c", 100.0, 130.0);
        let ts = pick_thumbnail_timestamps(&clip, None, &[]);
        // Midpoint = 115.0
        assert!(
            ts.iter().any(|&t| (t - 115.0).abs() < 2.5),
            "timestamps {:?} should include midpoint ~115", ts
        );
    }

    #[test]
    fn hook_frame_included() {
        let clip = make_clip("d", 50.0, 80.0);
        let ts = pick_thumbnail_timestamps(&clip, None, &[]);
        // Hook = start + 2 = 52
        assert!(
            ts.iter().any(|&t| (t - 52.0).abs() < 2.5),
            "timestamps {:?} should include hook ~52", ts
        );
    }

    #[test]
    fn timestamps_clamped_to_clip() {
        let clip = make_clip("e", 10.0, 15.0); // short clip
        let ts = pick_thumbnail_timestamps(&clip, None, &[]);
        for &t in &ts {
            assert!(t >= 10.0 && t <= 15.0, "timestamp {} out of clip bounds", t);
        }
    }

    #[test]
    fn no_duplicate_timestamps() {
        let clip = make_clip("f", 10.0, 40.0);
        let mut rms = vec![0.3; 50];
        rms[25] = 0.95;
        let audio = AudioProfile::from_rms(rms);
        // Scene cut near the audio peak — should not duplicate
        let cuts = vec![SceneDetection { time: 25.0, score: 0.7 }];

        let ts = pick_thumbnail_timestamps(&clip, Some(&audio), &cuts);
        // Check no two timestamps are within 2s of each other
        for i in 0..ts.len() {
            for j in (i + 1)..ts.len() {
                assert!(
                    (ts[i] - ts[j]).abs() >= 2.0,
                    "timestamps too close: {} and {}", ts[i], ts[j]
                );
            }
        }
    }

    #[test]
    fn scene_cuts_outside_clip_ignored() {
        let clip = make_clip("g", 10.0, 30.0);
        let cuts = vec![
            SceneDetection { time: 5.0, score: 0.9 },  // before clip
            SceneDetection { time: 50.0, score: 0.8 },  // after clip
        ];
        let ts = pick_thumbnail_timestamps(&clip, None, &cuts);
        // None of the cut timestamps should appear
        assert!(
            ts.iter().all(|&t| t >= 10.0 && t <= 30.0),
            "all timestamps should be within clip bounds"
        );
    }

    // ── finalize_without_thumbnails ──

    #[test]
    fn finalize_without_thumbnails_caps_at_top_n() {
        let ranked: Vec<RankedClip> = (0..10)
            .map(|i| make_ranked(&format!("clip-{i}"), i as f64 * 100.0, i as f64 * 100.0 + 25.0, i + 1))
            .collect();

        let result = finalize_without_thumbnails(&ranked, 5);
        assert_eq!(result.len(), 5);
    }

    #[test]
    fn finalize_without_thumbnails_preserves_order() {
        let ranked = vec![
            make_ranked("best", 0.0, 25.0, 1),
            make_ranked("second", 100.0, 125.0, 2),
            make_ranked("third", 200.0, 225.0, 3),
        ];

        let result = finalize_without_thumbnails(&ranked, 3);
        assert_eq!(result[0].id, "best");
        assert_eq!(result[1].id, "second");
        assert_eq!(result[2].id, "third");
    }

    #[test]
    fn finalize_without_thumbnails_has_no_paths() {
        let ranked = vec![make_ranked("x", 0.0, 25.0, 1)];
        let result = finalize_without_thumbnails(&ranked, 5);
        assert!(result[0].preview_thumbnail_path.is_none());
    }

    #[test]
    fn finalize_without_thumbnails_empty_input() {
        let result = finalize_without_thumbnails(&[], 5);
        assert!(result.is_empty());
    }

    // ── OutputConfig ──

    #[test]
    fn default_config_values() {
        let c = OutputConfig::with_default_dir();
        assert_eq!(c.thumb_width, 640);
        assert!(c.min_thumb_bytes > 0);
        // Note: clip count is not on OutputConfig — it's the max_clips
        // parameter on finalize(), flowing from AnalysisRequest.top_n.
    }

    #[test]
    fn custom_config() {
        let c = OutputConfig::new(PathBuf::from("/tmp/thumbs"));
        assert_eq!(c.thumb_dir, PathBuf::from("/tmp/thumbs"));
    }
}
