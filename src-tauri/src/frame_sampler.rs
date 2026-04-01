//! Signal-guided frame sampling for vision-model analysis.
//!
//! Instead of extracting frames at blind intervals across an entire
//! VOD, this module uses local signal data (audio, transcript, scene
//! changes) to:
//!
//! 1. **Propose candidate windows** — time ranges where local signals
//!    already found something interesting.
//! 2. **Sample frames within each window** — pick a small set of
//!    visually distinct, temporally meaningful frames.
//! 3. **Pack frames into API batches** — respect token budgets and
//!    spread coverage across windows.
//!
//! # Cost model
//!
//! At 640×360 JPEG quality 5, each frame ≈ 20–40 KB ≈ 250–500
//! vision tokens.  The defaults cap total frame count so a full
//! analysis stays under ~8 000 image tokens per batch.
//!
//! ```text
//!  1-hour VOD
//!    │
//!    ├─ Old approach: 120 frames (every 30s) → 3 batches of 40 → $$$
//!    │
//!    └─ New approach: ~10 candidate windows × 4 frames each
//!       = 40 frames total → 2 batches of 20 → $
//! ```

use crate::audio_signal::AudioProfile;
use crate::pipeline::SignalSegment;
use crate::scene_signal::SceneDetection;
use crate::vision_signal::FrameCapture;

// ═══════════════════════════════════════════════════════════════════
//  Configuration
// ═══════════════════════════════════════════════════════════════════

/// Default budget limits.  Callers can override via [`SamplingConfig`].
impl SamplingConfig {
    pub fn default() -> Self {
        Self {
            max_windows: 12,
            frames_per_window: 4,
            max_total_frames: 48,
            batch_size: 20,
            min_window_gap_secs: 30.0,
            window_padding_secs: 3.0,
        }
    }

    /// Tight budget: fewer frames, cheaper API calls.
    pub fn economy() -> Self {
        Self {
            max_windows: 8,
            frames_per_window: 3,
            max_total_frames: 24,
            batch_size: 12,
            ..Self::default()
        }
    }

    /// Higher quality: more frames per window, more coverage.
    pub fn quality() -> Self {
        Self {
            max_windows: 15,
            frames_per_window: 5,
            max_total_frames: 60,
            batch_size: 20,
            ..Self::default()
        }
    }
}

/// Tuning knobs for the frame sampler.
#[derive(Debug, Clone)]
pub struct SamplingConfig {
    /// Maximum number of candidate windows to analyze.
    pub max_windows: usize,
    /// Target frames to extract per candidate window.
    pub frames_per_window: usize,
    /// Hard cap on total frames across all windows.
    pub max_total_frames: usize,
    /// Maximum frames per API batch (vision token budget).
    pub batch_size: usize,
    /// Minimum gap between window centers to avoid redundancy.
    pub min_window_gap_secs: f64,
    /// Seconds added before/after a window for context frames.
    pub window_padding_secs: f64,
}

// ═══════════════════════════════════════════════════════════════════
//  Candidate window
// ═══════════════════════════════════════════════════════════════════

/// A time range nominated by local signals for vision analysis.
#[derive(Debug, Clone)]
pub struct CandidateWindow {
    /// Start of the window (seconds, with padding applied).
    pub start: f64,
    /// End of the window (seconds, with padding applied).
    pub end: f64,
    /// Aggregated score from the local signals that proposed this window.
    pub local_score: f64,
    /// Timestamp of the loudest audio second within the window,
    /// if audio data is available.  Used for the "peak" frame.
    pub audio_peak: Option<f64>,
    /// Scene-change timestamps that fall inside this window.
    /// Used to avoid sampling two frames from the same shot.
    pub scene_cuts: Vec<f64>,
    /// Tags merged from all contributing signals.
    pub tags: Vec<String>,
}

impl CandidateWindow {
    pub fn duration(&self) -> f64 { self.end - self.start }
    pub fn center(&self) -> f64 { (self.start + self.end) / 2.0 }
}

/// Timestamps selected for frame extraction within one window.
#[derive(Debug, Clone)]
pub struct FramePlan {
    /// Which window this plan belongs to.
    pub window_index: usize,
    /// Exact timestamps (seconds) to extract from the video.
    pub timestamps: Vec<f64>,
}

// ═══════════════════════════════════════════════════════════════════
//  Stage 1 — Propose candidate windows from local signals
// ═══════════════════════════════════════════════════════════════════

/// Select the most promising time windows from local signal segments.
///
/// Merges overlapping segments, ranks by score, enforces a minimum
/// gap between windows, and caps at `config.max_windows`.
///
/// # Inputs
///
/// - `segments`: all `SignalSegment`s from audio, transcript, and
///   scene-change detectors (combined into one vec).
/// - `audio`: optional per-second RMS profile for peak-finding.
/// - `scene_cuts`: scene-change detections for visual-diversity.
/// - `duration`: total VOD length in seconds.
pub fn propose_windows(
    segments: &[SignalSegment],
    audio: Option<&AudioProfile>,
    scene_cuts: &[SceneDetection],
    duration: f64,
    config: &SamplingConfig,
) -> Vec<CandidateWindow> {
    if segments.is_empty() {
        return Vec::new();
    }

    // 1. Merge overlapping segments into coalesced windows
    let mut ranges: Vec<(f64, f64, f64, Vec<String>)> = segments
        .iter()
        .map(|s| (s.start_time, s.end_time, s.score, s.tags.clone()))
        .collect();
    ranges.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut merged: Vec<(f64, f64, f64, Vec<String>)> = Vec::new();
    for (start, end, score, tags) in ranges {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 + config.window_padding_secs {
                last.1 = last.1.max(end);
                last.2 = last.2.max(score);
                for t in tags {
                    if !last.3.contains(&t) { last.3.push(t); }
                }
                continue;
            }
        }
        merged.push((start, end, score, tags));
    }

    // 2. Apply padding and clamp to VOD bounds
    let mut windows: Vec<CandidateWindow> = merged
        .into_iter()
        .map(|(start, end, score, tags)| {
            let padded_start = (start - config.window_padding_secs).max(0.0);
            let padded_end = (end + config.window_padding_secs).min(duration);

            // Find audio peak within window
            let audio_peak = audio.and_then(|a| {
                let s = padded_start as usize;
                let e = (padded_end as usize).min(a.rms.len());
                if e <= s { return None; }
                let (peak_idx, _) = a.rms[s..e]
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))?;
                Some((s + peak_idx) as f64 + 0.5)
            });

            // Collect scene cuts within window
            let cuts: Vec<f64> = scene_cuts
                .iter()
                .filter(|c| c.time >= padded_start && c.time <= padded_end)
                .map(|c| c.time)
                .collect();

            CandidateWindow {
                start: padded_start,
                end: padded_end,
                local_score: score,
                audio_peak,
                scene_cuts: cuts,
                tags,
            }
        })
        .collect();

    // 3. Sort by local score descending and enforce minimum gap
    windows.sort_by(|a, b| b.local_score.partial_cmp(&a.local_score).unwrap_or(std::cmp::Ordering::Equal));

    let mut selected: Vec<CandidateWindow> = Vec::new();
    for w in windows {
        if selected.iter().any(|s| (w.center() - s.center()).abs() < config.min_window_gap_secs) {
            continue;
        }
        selected.push(w);
        if selected.len() >= config.max_windows {
            break;
        }
    }

    // 4. Re-sort by timestamp for chronological processing
    selected.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));

    selected
}

// ═══════════════════════════════════════════════════════════════════
//  Stage 2 — Select frames within each window
// ═══════════════════════════════════════════════════════════════════

/// Choose specific timestamps to extract from each candidate window.
///
/// For each window, selects up to `config.frames_per_window` timestamps
/// at semantically meaningful positions:
///
/// ```text
///  Window: ├──────────────────────────────────┤
///  Frames: ↓ hook    ↓ context   ↓ peak  ↓ reaction
///          start+1s  1/3 point   peak    end-1s
/// ```
///
/// Scene-change data is used to shift frames away from identical
/// shots — if two candidate timestamps fall in the same shot (no
/// cut between them), the second is nudged past the next cut.
pub fn plan_frames(
    windows: &[CandidateWindow],
    config: &SamplingConfig,
) -> Vec<FramePlan> {
    let mut total_frames = 0_usize;

    windows
        .iter()
        .enumerate()
        .map(|(idx, w)| {
            let n = config.frames_per_window.min(config.max_total_frames - total_frames);
            if n == 0 {
                return FramePlan { window_index: idx, timestamps: vec![] };
            }

            let mut ts = pick_timestamps(w, n);

            // Deduplicate near-identical timestamps (< 1.5s apart)
            ts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            ts.dedup_by(|a, b| (*a - *b).abs() < 1.5);

            // Nudge frames that share a shot past the next scene cut
            nudge_across_cuts(&mut ts, &w.scene_cuts);

            // Clamp to window bounds
            for t in &mut ts {
                *t = t.clamp(w.start, w.end);
            }

            total_frames += ts.len();

            FramePlan { window_index: idx, timestamps: ts }
        })
        .filter(|p| !p.timestamps.is_empty())
        .collect()
}

/// Choose N semantically meaningful timestamps within a window.
///
/// Position strategy by frame count:
///   1 frame  → peak (or center)
///   2 frames → hook + peak
///   3 frames → hook + peak + reaction
///   4 frames → hook + context + peak + reaction
///   5+ frames → hook + evenly spaced interior + peak + reaction
fn pick_timestamps(w: &CandidateWindow, n: usize) -> Vec<f64> {
    let dur = w.duration();
    if dur < 1.0 || n == 0 {
        return vec![];
    }

    let hook = w.start + 1.0_f64.min(dur * 0.1);
    let peak = w.audio_peak.unwrap_or(w.center());
    let reaction = w.end - 1.0_f64.min(dur * 0.1);
    let context = w.start + dur / 3.0;

    match n {
        1 => vec![peak],
        2 => vec![hook, peak],
        3 => vec![hook, peak, reaction],
        4 => vec![hook, context, peak, reaction],
        _ => {
            // Hook + peak + reaction + evenly spaced interior points
            let interior_count = n.saturating_sub(3);
            let step = dur / (interior_count + 1) as f64;
            let mut ts = vec![hook];
            for i in 1..=interior_count {
                ts.push(w.start + step * i as f64);
            }
            ts.push(peak);
            ts.push(reaction);
            ts
        }
    }
}

/// Shift timestamps that fall within the same shot so each frame
/// shows a different visual context.
///
/// If two timestamps have no scene cut between them, nudge the
/// second one to 0.5s after the next cut.
fn nudge_across_cuts(timestamps: &mut [f64], cuts: &[f64]) {
    if cuts.is_empty() || timestamps.len() < 2 {
        return;
    }

    for i in 1..timestamps.len() {
        let prev = timestamps[i - 1];
        let curr = timestamps[i];

        // Check if there's a cut between prev and curr
        let has_cut = cuts.iter().any(|&c| c > prev && c < curr);
        if has_cut {
            continue; // Different shots — no nudge needed
        }

        // Find the next cut after curr and nudge past it
        if let Some(&next_cut) = cuts.iter().find(|&&c| c > curr) {
            timestamps[i] = next_cut + 0.5;
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Stage 3 — Pack into API batches
// ═══════════════════════════════════════════════════════════════════

/// Arrange frame plans into batches for API submission.
///
/// Each batch contains at most `config.batch_size` frames.
/// Windows are not split across batches when possible.
pub fn pack_batches(
    plans: &[FramePlan],
    config: &SamplingConfig,
) -> Vec<Vec<(usize, f64)>> {
    let mut batches: Vec<Vec<(usize, f64)>> = Vec::new();
    let mut current: Vec<(usize, f64)> = Vec::new();

    for plan in plans {
        // If adding this window would exceed the batch, start a new one
        if !current.is_empty() && current.len() + plan.timestamps.len() > config.batch_size {
            batches.push(std::mem::take(&mut current));
        }

        for &ts in &plan.timestamps {
            current.push((plan.window_index, ts));
            if current.len() >= config.batch_size {
                batches.push(std::mem::take(&mut current));
            }
        }
    }

    if !current.is_empty() {
        batches.push(current);
    }

    batches
}

// ═══════════════════════════════════════════════════════════════════
//  Frame extraction (ffmpeg)
// ═══════════════════════════════════════════════════════════════════

/// Extract specific frames from a video at exact timestamps.
///
/// Uses ffmpeg's fast input-seeking (`-ss` before `-i`) for each
/// timestamp.  Returns JPEG bytes loaded into memory.
pub fn extract_frames_at_timestamps(
    vod_path: &str,
    ffmpeg: &std::path::Path,
    timestamps: &[f64],
) -> Result<Vec<FrameCapture>, crate::error::AppError> {
    use crate::error::AppError;
    use std::process::{Command, Stdio};

    let temp_dir = std::env::temp_dir()
        .join("clipviral_vframes")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| AppError::Ffmpeg(format!("Create temp dir: {e}")))?;

    let mut captures = Vec::with_capacity(timestamps.len());

    for (i, &ts) in timestamps.iter().enumerate() {
        let out_path = temp_dir.join(format!("frame_{:04}.jpg", i));

        let mut cmd = Command::new(ffmpeg);
        cmd.arg("-ss").arg(format!("{:.3}", ts))
            .arg("-i").arg(vod_path)
            .arg("-vframes").arg("1")
            .arg("-vf").arg("scale=640:-1")
            .arg("-q:v").arg("5")
            .arg("-y")
            .arg(out_path.to_str().unwrap_or_default())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }

        // Non-fatal: if one frame fails, skip it and continue
        if let Ok(status) = cmd.status() {
            if status.success() {
                if let Ok(bytes) = std::fs::read(&out_path) {
                    if !bytes.is_empty() {
                        captures.push(FrameCapture {
                            timestamp: ts,
                            jpeg_bytes: bytes,
                        });
                    }
                }
            }
        }
        std::fs::remove_file(&out_path).ok();
    }

    // Clean up temp directory
    std::fs::remove_dir(&temp_dir).ok();

    if captures.is_empty() && !timestamps.is_empty() {
        return Err(AppError::Ffmpeg("No frames could be extracted".into()));
    }

    Ok(captures)
}

// ═══════════════════════════════════════════════════════════════════
//  Convenience: full pipeline
// ═══════════════════════════════════════════════════════════════════

/// Summary of the sampling plan for logging / debugging.
#[derive(Debug, Clone)]
pub struct SamplingPlan {
    pub windows: Vec<CandidateWindow>,
    pub frame_plans: Vec<FramePlan>,
    pub batches: Vec<Vec<(usize, f64)>>,
    pub total_frames: usize,
    pub total_batches: usize,
}

/// Run the full sampling pipeline: propose windows → plan frames → pack batches.
pub fn plan(
    segments: &[SignalSegment],
    audio: Option<&AudioProfile>,
    scene_cuts: &[SceneDetection],
    duration: f64,
    config: &SamplingConfig,
) -> SamplingPlan {
    let windows = propose_windows(segments, audio, scene_cuts, duration, config);
    let frame_plans = plan_frames(&windows, config);
    let batches = pack_batches(&frame_plans, config);
    let total_frames: usize = frame_plans.iter().map(|p| p.timestamps.len()).sum();
    let total_batches = batches.len();

    log::info!(
        "Frame sampling: {} windows → {} frames → {} batches",
        windows.len(), total_frames, total_batches,
    );

    SamplingPlan { windows, frame_plans, batches, total_frames, total_batches }
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::SignalType;

    fn seg(start: f64, end: f64, score: f64) -> SignalSegment {
        SignalSegment {
            signal_type: SignalType::Audio,
            start_time: start,
            end_time: end,
            score,
            tags: vec!["test".into()],
            metadata: None,
        }
    }

    fn config() -> SamplingConfig { SamplingConfig::default() }

    // ── Window proposal ──

    #[test]
    fn empty_signals_no_windows() {
        let windows = propose_windows(&[], None, &[], 600.0, &config());
        assert!(windows.is_empty());
    }

    #[test]
    fn windows_from_signals() {
        let segments = vec![
            seg(30.0, 35.0, 0.8),
            seg(120.0, 128.0, 0.6),
            seg(300.0, 310.0, 0.9),
        ];
        let windows = propose_windows(&segments, None, &[], 600.0, &config());
        assert_eq!(windows.len(), 3);
        // Sorted chronologically
        assert!(windows[0].start < windows[1].start);
        assert!(windows[1].start < windows[2].start);
    }

    #[test]
    fn overlapping_signals_merge_into_one_window() {
        let segments = vec![
            seg(30.0, 35.0, 0.7),
            seg(33.0, 38.0, 0.9), // overlaps with first
        ];
        let windows = propose_windows(&segments, None, &[], 600.0, &config());
        assert_eq!(windows.len(), 1);
        assert!((windows[0].local_score - 0.9).abs() < 0.01, "should keep max score");
    }

    #[test]
    fn windows_capped_at_max() {
        let segments: Vec<SignalSegment> = (0..30)
            .map(|i| seg(i as f64 * 50.0, i as f64 * 50.0 + 5.0, 0.5))
            .collect();
        let windows = propose_windows(&segments, None, &[], 1500.0, &config());
        assert!(windows.len() <= config().max_windows);
    }

    #[test]
    fn nearby_windows_deduplicated() {
        let segments = vec![
            seg(100.0, 105.0, 0.9),
            seg(110.0, 115.0, 0.7), // within min_window_gap (30s)
        ];
        let windows = propose_windows(&segments, None, &[], 600.0, &config());
        assert_eq!(windows.len(), 1, "windows too close should be deduplicated");
        assert!((windows[0].local_score - 0.9).abs() < 0.01, "higher score wins");
    }

    #[test]
    fn audio_peak_found_in_window() {
        let segments = vec![seg(10.0, 20.0, 0.8)];
        let audio = AudioProfile::from_rms({
            let mut rms = vec![0.3; 30];
            rms[15] = 0.95; // peak at second 15
            rms
        });
        let windows = propose_windows(&segments, Some(&audio), &[], 30.0, &config());
        assert!(!windows.is_empty());
        assert!(windows[0].audio_peak.is_some());
        let peak = windows[0].audio_peak.unwrap();
        assert!((peak - 15.5).abs() < 1.0, "peak should be near second 15");
    }

    #[test]
    fn scene_cuts_attached_to_window() {
        let segments = vec![seg(10.0, 30.0, 0.8)];
        let cuts = vec![
            SceneDetection { time: 15.0, score: 0.7 },
            SceneDetection { time: 22.0, score: 0.5 },
            SceneDetection { time: 100.0, score: 0.9 }, // outside window
        ];
        let windows = propose_windows(&segments, None, &cuts, 600.0, &config());
        assert_eq!(windows[0].scene_cuts.len(), 2);
    }

    // ── Frame planning ──

    #[test]
    fn four_frames_per_window_default() {
        let windows = vec![CandidateWindow {
            start: 10.0, end: 40.0, local_score: 0.8,
            audio_peak: Some(25.0), scene_cuts: vec![], tags: vec![],
        }];
        let plans = plan_frames(&windows, &config());
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].timestamps.len(), 4);
    }

    #[test]
    fn frame_timestamps_within_window() {
        let windows = vec![CandidateWindow {
            start: 50.0, end: 80.0, local_score: 0.7,
            audio_peak: None, scene_cuts: vec![], tags: vec![],
        }];
        let plans = plan_frames(&windows, &config());
        for &ts in &plans[0].timestamps {
            assert!(ts >= 50.0 && ts <= 80.0, "timestamp {ts} outside window");
        }
    }

    #[test]
    fn peak_frame_included() {
        let windows = vec![CandidateWindow {
            start: 10.0, end: 40.0, local_score: 0.8,
            audio_peak: Some(25.0), scene_cuts: vec![], tags: vec![],
        }];
        let plans = plan_frames(&windows, &config());
        // The peak timestamp (25.0) should appear in the plan
        assert!(
            plans[0].timestamps.iter().any(|&t| (t - 25.0).abs() < 1.5),
            "peak timestamp should be included"
        );
    }

    #[test]
    fn total_frames_capped() {
        let windows: Vec<CandidateWindow> = (0..20)
            .map(|i| CandidateWindow {
                start: i as f64 * 60.0, end: i as f64 * 60.0 + 20.0,
                local_score: 0.5, audio_peak: None, scene_cuts: vec![], tags: vec![],
            })
            .collect();
        let plans = plan_frames(&windows, &config());
        let total: usize = plans.iter().map(|p| p.timestamps.len()).sum();
        assert!(total <= config().max_total_frames);
    }

    #[test]
    fn near_identical_timestamps_deduplicated() {
        let windows = vec![CandidateWindow {
            start: 10.0, end: 12.0, // Very short window
            local_score: 0.8,
            audio_peak: Some(11.0), scene_cuts: vec![], tags: vec![],
        }];
        let plans = plan_frames(&windows, &config());
        // Even though 4 frames are requested, dedup should collapse them
        assert!(plans[0].timestamps.len() <= 3,
            "short window should have fewer frames after dedup");
    }

    // ── Cross-cut nudging ──

    #[test]
    fn frames_nudged_across_scene_cuts() {
        let mut ts = vec![10.0, 12.0]; // no cut between them
        let cuts = vec![14.0];
        nudge_across_cuts(&mut ts, &cuts);
        // Second frame should be nudged past the cut at 14.0
        assert!(ts[1] > 14.0, "frame should be nudged past scene cut");
    }

    #[test]
    fn frames_not_nudged_when_cut_exists_between() {
        let mut ts = vec![10.0, 16.0]; // cut at 13.0 is between them
        let cuts = vec![13.0];
        nudge_across_cuts(&mut ts, &cuts);
        assert!((ts[1] - 16.0).abs() < 0.01, "frame should not be nudged");
    }

    // ── Batch packing ──

    #[test]
    fn single_window_fits_in_one_batch() {
        let plans = vec![FramePlan {
            window_index: 0,
            timestamps: vec![10.0, 15.0, 20.0, 25.0],
        }];
        let batches = pack_batches(&plans, &config());
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 4);
    }

    #[test]
    fn large_plan_splits_into_batches() {
        let plans: Vec<FramePlan> = (0..10)
            .map(|i| FramePlan {
                window_index: i,
                timestamps: vec![i as f64 * 100.0, i as f64 * 100.0 + 5.0, i as f64 * 100.0 + 10.0],
            })
            .collect();
        let cfg = SamplingConfig { batch_size: 8, ..SamplingConfig::default() };
        let batches = pack_batches(&plans, &cfg);
        // 30 total frames / 8 per batch = 4 batches
        assert!(batches.len() >= 3);
        for batch in &batches {
            assert!(batch.len() <= 8);
        }
    }

    // ── Full plan ──

    #[test]
    fn full_plan_pipeline() {
        let segments = vec![
            seg(30.0, 38.0, 0.9),
            seg(120.0, 130.0, 0.7),
            seg(250.0, 260.0, 0.8),
        ];
        let audio = AudioProfile::from_rms(vec![0.3; 300]);
        let cuts = vec![
            SceneDetection { time: 33.0, score: 0.6 },
            SceneDetection { time: 125.0, score: 0.5 },
        ];

        let plan = plan(&segments, Some(&audio), &cuts, 300.0, &config());

        assert_eq!(plan.windows.len(), 3);
        assert!(plan.total_frames > 0);
        assert!(plan.total_frames <= config().max_total_frames);
        assert!(plan.total_batches >= 1);
    }

    // ── Config presets ──

    #[test]
    fn economy_uses_fewer_frames() {
        let eco = SamplingConfig::economy();
        let def = SamplingConfig::default();
        assert!(eco.max_total_frames < def.max_total_frames);
        assert!(eco.frames_per_window <= def.frames_per_window);
    }

    #[test]
    fn quality_uses_more_frames() {
        let qual = SamplingConfig::quality();
        let def = SamplingConfig::default();
        assert!(qual.max_total_frames >= def.max_total_frames);
    }
}
