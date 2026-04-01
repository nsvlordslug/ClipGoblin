//! Scene-change and motion analysis for clip detection.
//!
//! Uses two ffmpeg filter passes to find visually interesting moments:
//!
//! | Detector       | ffmpeg filter          | Finds                                    |
//! |----------------|------------------------|------------------------------------------|
//! | Scene change   | `select + showinfo`    | Hard cuts, camera switches, transitions  |
//! | Motion energy  | `mestimate + metadata` | Fast action, camera pans, rapid movement |
//! | Cut clustering | (post-processing)      | Rapid edit sequences (montages)          |
//!
//! Entirely local — no API keys, no GPU required.
//!
//! # Usage
//!
//! ```ignore
//! let segments = scene_signal::analyze("video.mp4", &ffmpeg_path)?;
//! ```

use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::AppError;
use crate::pipeline::{SceneChangeKind, SignalMetadata, SignalSegment, SignalType};

// ═══════════════════════════════════════════════════════════════════
//  Configuration
// ═══════════════════════════════════════════════════════════════════

/// Scene-change score threshold (0.0–1.0).  ffmpeg's `scene` filter
/// outputs a score per frame; we keep frames above this value.
/// 0.25 catches hard cuts while ignoring minor lighting changes.
const SCENE_THRESHOLD: f64 = 0.25;

/// Seconds within which multiple scene detections are merged.
const SCENE_MERGE_WINDOW: f64 = 2.0;

/// Analysis FPS for motion estimation.  Lower = faster but coarser.
/// 2 fps captures motion at half-second resolution — good enough for
/// streaming content and keeps the pass under a minute for 1-hour VODs.
const MOTION_FPS: u32 = 2;

/// Seconds per bucket when aggregating per-frame motion into a timeline.
const MOTION_BUCKET_SECS: usize = 1;

/// Window size (in seconds) for the motion surge detector.
const MOTION_WINDOW_SECS: usize = 3;

/// Minimum consecutive seconds of high motion for a sustained detection.
const MIN_SUSTAINED_MOTION_SECS: usize = 4;

/// Maximum cuts within a window to qualify as a rapid-edit cluster.
const RAPID_CUT_WINDOW_SECS: f64 = 10.0;
/// Minimum cuts inside the window to qualify.
const RAPID_CUT_MIN_CUTS: usize = 4;

/// Maximum segments returned.
const MAX_SEGMENTS: usize = 20;

/// Seconds within which overlapping segments are merged.
const DEDUP_GAP_SECS: f64 = 6.0;

// ═══════════════════════════════════════════════════════════════════
//  Intermediate representations
// ═══════════════════════════════════════════════════════════════════

/// A single scene-change detection from ffmpeg.
#[derive(Debug, Clone)]
pub struct SceneDetection {
    /// Timestamp in seconds.
    pub time: f64,
    /// ffmpeg scene score (0.0–1.0).
    pub score: f64,
}

/// Per-second motion energy extracted from ffmpeg's motion estimator.
#[derive(Debug, Clone)]
pub struct MotionProfile {
    /// Average motion vector magnitude per second, normalized 0.0–1.0.
    pub energy: Vec<f64>,
    /// Global average across the entire file.
    pub avg: f64,
    /// Standard deviation.
    pub std_dev: f64,
}

impl MotionProfile {
    pub fn from_energy(energy: Vec<f64>) -> Self {
        let n = energy.len().max(1) as f64;
        let avg = energy.iter().sum::<f64>() / n;
        let variance = energy.iter().map(|v| (v - avg).powi(2)).sum::<f64>() / n;
        Self { energy, avg, std_dev: variance.sqrt() }
    }

    pub fn avg_in_range(&self, start: usize, end: usize) -> f64 {
        let s = start.min(self.energy.len());
        let e = end.min(self.energy.len());
        if e <= s { return 0.0; }
        self.energy[s..e].iter().sum::<f64>() / (e - s) as f64
    }

    pub fn peak_in_range(&self, start: usize, end: usize) -> f64 {
        let s = start.min(self.energy.len());
        let e = end.min(self.energy.len());
        self.energy[s..e].iter().cloned().fold(0.0_f64, f64::max)
    }

    pub fn duration_secs(&self) -> usize { self.energy.len() }
}

// ═══════════════════════════════════════════════════════════════════
//  Public API
// ═══════════════════════════════════════════════════════════════════

/// Analyze a media file's visuals and return scored signal segments.
///
/// Runs two ffmpeg passes (scene detection + motion estimation),
/// then applies three detectors: scene changes, motion surges,
/// and rapid-cut clustering.
pub fn analyze(vod_path: &str, ffmpeg: &Path) -> Result<Vec<SignalSegment>, AppError> {
    let scenes = extract_scene_changes(vod_path, ffmpeg)?;
    let motion = extract_motion_energy(vod_path, ffmpeg)?;
    Ok(detect_signals(&scenes, &motion))
}

/// Run detection on pre-extracted data (for testing without ffmpeg).
pub fn detect_signals(scenes: &[SceneDetection], motion: &MotionProfile) -> Vec<SignalSegment> {
    let mut all = Vec::new();
    all.extend(detect_scene_cuts(scenes));
    all.extend(detect_rapid_cuts(scenes));
    all.extend(detect_motion_surges(motion));
    all.extend(detect_sustained_motion(motion));

    merge_and_rank(&mut all);
    all.truncate(MAX_SEGMENTS);
    all
}

// ═══════════════════════════════════════════════════════════════════
//  ffmpeg pass 1 — Scene-change detection
// ═══════════════════════════════════════════════════════════════════
//
//  Filter chain:
//    select='gt(scene,THRESHOLD)',showinfo
//
//  The `select` filter computes a scene-change score (0–1) per frame
//  and passes only frames above the threshold.  `showinfo` prints
//  each selected frame's `pts_time` to stderr, which we parse.

pub fn extract_scene_changes(
    vod_path: &str,
    ffmpeg: &Path,
) -> Result<Vec<SceneDetection>, AppError> {
    let mut cmd = Command::new(ffmpeg);
    cmd.arg("-i").arg(vod_path)
        .arg("-vf")
        .arg(format!(
            "select='gt(scene\\,{})',metadata=print",
            SCENE_THRESHOLD
        ))
        .arg("-an")
        .arg("-f").arg("null")
        .arg("-")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let output = cmd
        .output()
        .map_err(|e| AppError::Ffmpeg(format!("Scene detection launch failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // ffmpeg often returns non-zero for null output; only fail if
        // stderr contains a real error, not just stats.
        if stderr.contains("Error") || stderr.contains("No such file") {
            return Err(AppError::Ffmpeg(format!(
                "Scene detection failed: {}",
                stderr.lines().take(3).collect::<Vec<_>>().join(" | ")
            )));
        }
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let scenes = parse_scene_output(&stderr);

    log::info!("Scene detection: {} cuts found", scenes.len());
    Ok(scenes)
}

/// Parse ffmpeg's showinfo/metadata stderr output for scene changes.
///
/// We look for two patterns per frame:
///   `pts_time:123.456`         — timestamp
///   `lavfi.scene_score=0.789`  — scene score
fn parse_scene_output(stderr: &str) -> Vec<SceneDetection> {
    let mut detections = Vec::new();
    let mut current_time: Option<f64> = None;

    for line in stderr.lines() {
        // Extract pts_time from showinfo lines
        if let Some(pos) = line.find("pts_time:") {
            let rest = &line[pos + 9..];
            let end = rest
                .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
                .unwrap_or(rest.len());
            if let Ok(t) = rest[..end].parse::<f64>() {
                // If we still have a pending time without a score,
                // the previous frame had no score metadata — use the
                // threshold as a floor (the select filter already
                // proved it exceeded it).
                if let Some(prev_t) = current_time {
                    if !detections.iter().any(|d: &SceneDetection| (d.time - prev_t).abs() < 0.1) {
                        detections.push(SceneDetection { time: prev_t, score: SCENE_THRESHOLD });
                    }
                }
                current_time = Some(t);
            }
        }

        // Extract scene_score from metadata lines
        if let Some(pos) = line.find("lavfi.scene_score=") {
            let rest = &line[pos + 18..];
            let end = rest
                .find(|c: char| !c.is_ascii_digit() && c != '.')
                .unwrap_or(rest.len());
            if let Ok(score) = rest[..end].parse::<f64>() {
                if let Some(t) = current_time.take() {
                    detections.push(SceneDetection { time: t, score: score.min(1.0) });
                }
            }
        }
    }

    // Flush any trailing unpaired timestamp
    if let Some(t) = current_time {
        if !detections.iter().any(|d| (d.time - t).abs() < 0.1) {
            detections.push(SceneDetection { time: t, score: SCENE_THRESHOLD });
        }
    }

    // Merge detections very close together (sub-frame jitter)
    detections.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged = Vec::with_capacity(detections.len());
    for det in detections {
        if let Some(last) = merged.last_mut() {
            let last: &mut SceneDetection = last;
            if det.time - last.time < SCENE_MERGE_WINDOW {
                if det.score > last.score {
                    *last = det;
                }
                continue;
            }
        }
        merged.push(det);
    }

    merged
}

// ═══════════════════════════════════════════════════════════════════
//  ffmpeg pass 2 — Motion estimation
// ═══════════════════════════════════════════════════════════════════
//
//  Filter chain:
//    fps=MOTION_FPS,mestimate=method=esa,metadata=mode=print
//
//  The `mestimate` filter computes block-based motion vectors per
//  frame.  We read the mean motion vector magnitude from metadata
//  and aggregate into per-second buckets.

pub fn extract_motion_energy(
    vod_path: &str,
    ffmpeg: &Path,
) -> Result<MotionProfile, AppError> {
    let temp_file = std::env::temp_dir()
        .join("clipviral_motion")
        .join(format!("{}.txt", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(temp_file.parent().unwrap()).ok();

    let escaped = temp_file
        .to_str()
        .unwrap_or_default()
        .replace('\\', "/")
        .replace(':', "\\:");

    let mut cmd = Command::new(ffmpeg);
    cmd.arg("-i").arg(vod_path)
        .arg("-vf")
        .arg(format!(
            "fps={},mestimate=method=esa,metadata=mode=print:file='{}'",
            MOTION_FPS, escaped
        ))
        .arg("-an")
        .arg("-f").arg("null")
        .arg("-")
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let status = cmd
        .status()
        .map_err(|e| AppError::Ffmpeg(format!("Motion analysis launch failed: {e}")))?;

    if !status.success() {
        if let Err(e) = std::fs::remove_file(&temp_file) {
            log::warn!("Failed to clean up temp file {:?}: {}", temp_file, e);
        }
        return Err(AppError::Ffmpeg(
            "Motion analysis exited with an error".into(),
        ));
    }

    let content = std::fs::read_to_string(&temp_file)
        .map_err(|e| AppError::Ffmpeg(format!("Failed to read motion data: {e}")))?;
    if let Err(e) = std::fs::remove_file(&temp_file) {
        log::warn!("Failed to clean up temp file {:?}: {}", temp_file, e);
    }

    let profile = parse_motion_output(&content);

    log::info!(
        "Motion extraction: {} seconds (avg={:.3}, stddev={:.3})",
        profile.duration_secs(),
        profile.avg,
        profile.std_dev,
    );

    Ok(profile)
}

/// Parse mestimate metadata output into a [`MotionProfile`].
///
/// Metadata lines look like:
///   `frame:N pts:... pts_time:T`
///   `lavfi.mestimate.mean_motion.x=...`
///   `lavfi.mestimate.mean_motion.y=...`
///
/// We compute magnitude = sqrt(x² + y²) per frame, then bucket
/// into per-second averages and normalise to 0.0–1.0.
fn parse_motion_output(content: &str) -> MotionProfile {
    let mut frame_motions: Vec<(f64, f64)> = Vec::new(); // (time, magnitude)
    let mut current_time: Option<f64> = None;
    let mut mx: Option<f64> = None;
    let mut my: Option<f64> = None;

    for line in content.lines() {
        if line.starts_with("frame:") {
            if let Some(pos) = line.find("pts_time:") {
                let rest = &line[pos + 9..];
                let end = rest
                    .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
                    .unwrap_or(rest.len());
                if let Ok(t) = rest[..end].parse::<f64>() {
                    current_time = Some(t);
                }
            }
        } else if let Some(rest) = line.strip_prefix("lavfi.mestimate.mean_motion.x=") {
            mx = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("lavfi.mestimate.mean_motion.y=") {
            my = rest.trim().parse().ok();
        }

        // When we have all three values, emit a measurement
        if let (Some(t), Some(x), Some(y)) = (current_time, mx, my) {
            frame_motions.push((t, (x * x + y * y).sqrt()));
            current_time = None;
            mx = None;
            my = None;
        }
    }

    if frame_motions.is_empty() {
        return MotionProfile::from_energy(vec![]);
    }

    // Bucket into per-second averages
    let max_sec = frame_motions.last().map(|(t, _)| *t as usize).unwrap_or(0);
    let mut buckets = vec![(0.0_f64, 0u32); max_sec + 1];

    for (t, mag) in &frame_motions {
        let sec = (*t as usize).min(max_sec);
        buckets[sec].0 += mag;
        buckets[sec].1 += 1;
    }

    let raw: Vec<f64> = buckets
        .iter()
        .map(|(sum, count)| if *count > 0 { sum / *count as f64 } else { 0.0 })
        .collect();

    // Normalise to 0.0–1.0 using the 99th percentile as ceiling
    // (avoids a single extreme frame warping the whole scale)
    let mut sorted = raw.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p99_idx = ((sorted.len() as f64 * 0.99) as usize).min(sorted.len().saturating_sub(1));
    let ceiling = sorted[p99_idx].max(1.0);

    let normalized: Vec<f64> = raw.iter().map(|v| (v / ceiling).min(1.0)).collect();

    MotionProfile::from_energy(normalized)
}

// ═══════════════════════════════════════════════════════════════════
//  Detector 1 — Individual scene cuts
// ═══════════════════════════════════════════════════════════════════
//
//  Each scene change above the threshold becomes a signal segment.
//  Score = scene_score normalised against the strongest cut.
//  Higher scores → more dramatic visual change.

fn detect_scene_cuts(scenes: &[SceneDetection]) -> Vec<SignalSegment> {
    if scenes.is_empty() {
        return Vec::new();
    }

    let max_score = scenes
        .iter()
        .map(|s| s.score)
        .fold(0.0_f64, f64::max)
        .max(0.01);

    scenes
        .iter()
        .map(|det| {
            let normalized = (det.score / max_score * 0.85 + 0.10).clamp(0.0, 1.0);
            let kind = classify_cut(det.score);

            SignalSegment {
                signal_type: SignalType::SceneChange,
                start_time: (det.time - 0.5).max(0.0),
                end_time: det.time + 1.5,
                score: normalized,
                tags: tags_for_cut(kind),
                metadata: Some(SignalMetadata::SceneChange {
                    magnitude: det.score,
                    change_type: kind,
                }),
            }
        })
        .collect()
}

/// Classify a scene change by its magnitude.
fn classify_cut(score: f64) -> SceneChangeKind {
    if score >= 0.65 {
        SceneChangeKind::HardCut
    } else if score >= 0.40 {
        SceneChangeKind::FastMotion
    } else {
        SceneChangeKind::Transition
    }
}

fn tags_for_cut(kind: SceneChangeKind) -> Vec<String> {
    match kind {
        SceneChangeKind::HardCut => vec!["cut", "camera_change"],
        SceneChangeKind::FastMotion => vec!["fast_motion", "action"],
        SceneChangeKind::Transition => vec!["transition"],
    }
    .into_iter()
    .map(String::from)
    .collect()
}

// ═══════════════════════════════════════════════════════════════════
//  Detector 2 — Rapid-cut clustering
// ═══════════════════════════════════════════════════════════════════
//
//  Finds bursts of scene changes close together — montages, replays,
//  action sequences with fast editing.
//
//  Algorithm:
//    Slide a window of RAPID_CUT_WINDOW_SECS over the scene list.
//    When ≥ RAPID_CUT_MIN_CUTS land inside the window, emit a
//    detection spanning the cluster.
//
//  Score = (cuts_in_window / max_cuts_in_any_window) * 0.85 + 0.10

fn detect_rapid_cuts(scenes: &[SceneDetection]) -> Vec<SignalSegment> {
    if scenes.len() < RAPID_CUT_MIN_CUTS {
        return Vec::new();
    }

    // For each scene detection, count how many others fall within the window
    struct Cluster {
        start: f64,
        end: f64,
        count: usize,
        peak_score: f64,
    }

    let mut clusters: Vec<Cluster> = Vec::new();

    for (i, anchor) in scenes.iter().enumerate() {
        let window_end = anchor.time + RAPID_CUT_WINDOW_SECS;
        let cuts: Vec<&SceneDetection> = scenes[i..]
            .iter()
            .take_while(|s| s.time <= window_end)
            .collect();

        if cuts.len() >= RAPID_CUT_MIN_CUTS {
            let peak = cuts.iter().map(|c| c.score).fold(0.0_f64, f64::max);
            let end = cuts.last().map(|c| c.time).unwrap_or(window_end);
            clusters.push(Cluster {
                start: anchor.time,
                end: end + 1.0,
                count: cuts.len(),
                peak_score: peak,
            });
        }
    }

    if clusters.is_empty() {
        return Vec::new();
    }

    // Deduplicate overlapping clusters (keep highest count)
    clusters.sort_by(|a, b| b.count.cmp(&a.count));
    let mut kept: Vec<Cluster> = Vec::new();
    'outer: for c in clusters {
        for k in &kept {
            if c.start < k.end && c.end > k.start {
                continue 'outer;
            }
        }
        kept.push(c);
    }

    let max_count = kept.iter().map(|c| c.count).max().unwrap_or(1).max(1);

    kept.iter()
        .map(|c| {
            let score = (c.count as f64 / max_count as f64 * 0.85 + 0.10).clamp(0.0, 1.0);
            SignalSegment {
                signal_type: SignalType::SceneChange,
                start_time: c.start,
                end_time: c.end,
                score,
                tags: vec![
                    "rapid_cuts".to_string(),
                    "montage".to_string(),
                    "action".to_string(),
                ],
                metadata: Some(SignalMetadata::SceneChange {
                    magnitude: c.peak_score,
                    change_type: SceneChangeKind::HardCut,
                }),
            }
        })
        .collect()
}

// ═══════════════════════════════════════════════════════════════════
//  Detector 3 — Motion surges (sudden movement)
// ═══════════════════════════════════════════════════════════════════
//
//  Finds moments where motion energy spikes above baseline.
//  Scoring: z-score of the window energy, normalized to 0–1.

fn detect_motion_surges(profile: &MotionProfile) -> Vec<SignalSegment> {
    let len = profile.duration_secs();
    if len < MOTION_WINDOW_SECS * 2 {
        return Vec::new();
    }

    let threshold = profile.avg + 1.5 * profile.std_dev;

    struct Hit { sec: usize, energy: f64, z: f64 }

    let mut hits: Vec<Hit> = Vec::new();

    for i in 0..len.saturating_sub(MOTION_WINDOW_SECS) {
        let energy = profile.avg_in_range(i, i + MOTION_WINDOW_SECS);
        if energy <= threshold {
            continue;
        }
        let z = if profile.std_dev > 0.001 {
            (energy - profile.avg) / profile.std_dev
        } else {
            energy / profile.avg.max(0.001)
        };
        hits.push(Hit { sec: i, energy, z });
    }

    if hits.is_empty() {
        return Vec::new();
    }

    hits.sort_by(|a, b| b.z.partial_cmp(&a.z).unwrap_or(std::cmp::Ordering::Equal));
    let max_z = hits[0].z.max(1.0);

    // Deduplicate nearby detections
    let mut used: Vec<usize> = Vec::new();
    let mut segments = Vec::new();

    for hit in &hits {
        if used.iter().any(|&u| (hit.sec as i64 - u as i64).unsigned_abs() < DEDUP_GAP_SECS as u64) {
            continue;
        }
        used.push(hit.sec);

        let score = (hit.z / max_z * 0.85 + 0.10).clamp(0.0, 1.0);

        segments.push(SignalSegment {
            signal_type: SignalType::SceneChange,
            start_time: hit.sec as f64,
            end_time: (hit.sec + MOTION_WINDOW_SECS) as f64,
            score,
            tags: vec!["fast_motion".to_string(), "action".to_string()],
            metadata: Some(SignalMetadata::SceneChange {
                magnitude: hit.energy,
                change_type: SceneChangeKind::FastMotion,
            }),
        });
    }

    segments
}

// ═══════════════════════════════════════════════════════════════════
//  Detector 4 — Sustained motion (prolonged action)
// ═══════════════════════════════════════════════════════════════════
//
//  Finds contiguous runs where motion stays above 1.3× average.
//  Score favors longer and more intense runs.

fn detect_sustained_motion(profile: &MotionProfile) -> Vec<SignalSegment> {
    let threshold = (profile.avg * 1.3).max(0.15);

    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut run_start: Option<usize> = None;

    for (i, &val) in profile.energy.iter().enumerate() {
        if val > threshold {
            if run_start.is_none() {
                run_start = Some(i);
            }
        } else if let Some(start) = run_start {
            if i - start >= MIN_SUSTAINED_MOTION_SECS {
                runs.push((start, i));
            }
            run_start = None;
        }
    }
    if let Some(start) = run_start {
        let end = profile.energy.len();
        if end - start >= MIN_SUSTAINED_MOTION_SECS {
            runs.push((start, end));
        }
    }

    if runs.is_empty() {
        return Vec::new();
    }

    struct Run { start: usize, end: usize, avg_e: f64, peak: f64, raw: f64 }

    let scored: Vec<Run> = runs
        .iter()
        .map(|&(s, e)| {
            let avg_e = profile.avg_in_range(s, e);
            let peak = profile.peak_in_range(s, e);
            let duration = (e - s) as f64;
            let raw = (avg_e / profile.avg.max(0.001)) * (duration / 20.0).min(1.0);
            Run { start: s, end: e, avg_e, peak, raw }
        })
        .collect();

    let max_raw = scored.iter().map(|r| r.raw).fold(1.0_f64, f64::max);

    scored
        .iter()
        .map(|run| {
            let score = (run.raw / max_raw * 0.80 + 0.10).clamp(0.0, 1.0);
            SignalSegment {
                signal_type: SignalType::SceneChange,
                start_time: run.start as f64,
                end_time: run.end as f64,
                score,
                tags: vec!["sustained_motion".to_string(), "action".to_string()],
                metadata: Some(SignalMetadata::SceneChange {
                    magnitude: run.peak,
                    change_type: SceneChangeKind::FastMotion,
                }),
            }
        })
        .collect()
}

// ═══════════════════════════════════════════════════════════════════
//  Merge & rank (shared with audio_signal pattern)
// ═══════════════════════════════════════════════════════════════════

fn merge_and_rank(segments: &mut Vec<SignalSegment>) {
    segments.sort_by(|a, b| {
        a.start_time.partial_cmp(&b.start_time).unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut merged: Vec<SignalSegment> = Vec::with_capacity(segments.len());
    for seg in segments.drain(..) {
        if let Some(last) = merged.last_mut() {
            if seg.start_time <= last.end_time + DEDUP_GAP_SECS {
                if seg.score > last.score {
                    let start = last.start_time.min(seg.start_time);
                    let end = last.end_time.max(seg.end_time);
                    *last = seg;
                    last.start_time = start;
                    last.end_time = end;
                } else {
                    last.end_time = last.end_time.max(seg.end_time);
                }
                continue;
            }
        }
        merged.push(seg);
    }

    merged.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    *segments = merged;
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn make_scenes(times_and_scores: &[(f64, f64)]) -> Vec<SceneDetection> {
        times_and_scores
            .iter()
            .map(|&(time, score)| SceneDetection { time, score })
            .collect()
    }

    fn make_motion(len: usize, spikes: &[(usize, usize, f64)]) -> MotionProfile {
        let mut energy = vec![0.2; len];
        for &(start, end, val) in spikes {
            for i in start..end.min(len) {
                energy[i] = val;
            }
        }
        MotionProfile::from_energy(energy)
    }

    // ── MotionProfile ──

    #[test]
    fn motion_profile_statistics() {
        let p = MotionProfile::from_energy(vec![0.2, 0.4, 0.6]);
        assert!((p.avg - 0.4).abs() < 1e-9);
        assert!(p.std_dev > 0.0);
    }

    #[test]
    fn motion_profile_range_clamped() {
        let p = MotionProfile::from_energy(vec![0.1, 0.2, 0.3]);
        assert!(p.avg_in_range(10, 20) == 0.0);
    }

    // ── Scene cut detector ──

    #[test]
    fn scene_cuts_produce_segments() {
        let scenes = make_scenes(&[(5.0, 0.8), (30.0, 0.5), (60.0, 0.9)]);
        let motion = MotionProfile::from_energy(vec![]);
        let result = detect_signals(&scenes, &motion);

        assert!(!result.is_empty());
        assert!(result.iter().all(|s| s.signal_type == SignalType::SceneChange));
    }

    #[test]
    fn harder_cut_scores_higher() {
        let scenes = make_scenes(&[(10.0, 0.9), (50.0, 0.3)]);
        let segs = detect_scene_cuts(&scenes);
        assert!(segs.len() == 2);
        assert!(segs[0].score != segs[1].score, "different magnitudes should produce different scores");
        // The 0.9 cut should appear with higher score
        let high = segs.iter().find(|s| s.start_time < 15.0).unwrap();
        let low = segs.iter().find(|s| s.start_time > 40.0).unwrap();
        assert!(high.score > low.score);
    }

    #[test]
    fn hard_cut_classified_correctly() {
        let scenes = make_scenes(&[(10.0, 0.8)]);
        let segs = detect_scene_cuts(&scenes);
        match &segs[0].metadata {
            Some(SignalMetadata::SceneChange { change_type, .. }) => {
                assert_eq!(*change_type, SceneChangeKind::HardCut);
            }
            other => panic!("Expected SceneChange metadata, got {:?}", other),
        }
    }

    #[test]
    fn transition_classified_correctly() {
        let scenes = make_scenes(&[(10.0, 0.30)]);
        let segs = detect_scene_cuts(&scenes);
        match &segs[0].metadata {
            Some(SignalMetadata::SceneChange { change_type, .. }) => {
                assert_eq!(*change_type, SceneChangeKind::Transition);
            }
            other => panic!("Expected SceneChange metadata, got {:?}", other),
        }
    }

    // ── Rapid-cut clustering ──

    #[test]
    fn rapid_cuts_detected() {
        // 6 cuts within 8 seconds — classic montage/replay
        let scenes = make_scenes(&[
            (10.0, 0.7), (11.5, 0.6), (13.0, 0.8),
            (14.5, 0.5), (16.0, 0.7), (17.5, 0.6),
        ]);
        let segs = detect_rapid_cuts(&scenes);
        assert!(!segs.is_empty());
        assert!(segs[0].tags.contains(&"rapid_cuts".to_string()));
    }

    #[test]
    fn sparse_cuts_no_cluster() {
        // Cuts spread out over 60 seconds — not rapid
        let scenes = make_scenes(&[(10.0, 0.6), (30.0, 0.5), (50.0, 0.7)]);
        let segs = detect_rapid_cuts(&scenes);
        assert!(segs.is_empty());
    }

    // ── Motion surge detector ──

    #[test]
    fn motion_surge_detected() {
        // Quiet baseline with a 3-second spike at second 30
        let profile = make_motion(60, &[(30, 33, 0.9)]);
        let segs = detect_motion_surges(&profile);
        assert!(!segs.is_empty());
        assert!(segs[0].start_time >= 28.0 && segs[0].start_time <= 32.0);
    }

    #[test]
    fn flat_motion_no_surges() {
        let profile = MotionProfile::from_energy(vec![0.3; 60]);
        let segs = detect_motion_surges(&profile);
        assert!(segs.is_empty());
    }

    // ── Sustained motion detector ──

    #[test]
    fn sustained_motion_detected() {
        let profile = make_motion(60, &[(20, 35, 0.8)]);
        let segs = detect_sustained_motion(&profile);
        assert!(!segs.is_empty());
        let best = &segs[0];
        assert!(best.duration() >= 10.0);
        assert!(best.tags.contains(&"sustained_motion".to_string()));
    }

    #[test]
    fn short_burst_not_sustained() {
        let profile = make_motion(60, &[(30, 32, 0.9)]);
        let segs = detect_sustained_motion(&profile);
        assert!(segs.is_empty(), "2-second burst should not qualify as sustained");
    }

    // ── Full pipeline ──

    #[test]
    fn all_scores_normalised() {
        let scenes = make_scenes(&[(10.0, 0.8), (40.0, 0.5)]);
        let motion = make_motion(60, &[(25, 30, 0.9)]);
        let segs = detect_signals(&scenes, &motion);
        for s in &segs {
            assert!(s.score >= 0.0 && s.score <= 1.0, "score {} out of range", s.score);
        }
    }

    #[test]
    fn segments_capped_at_max() {
        // Many scene changes
        let scenes: Vec<SceneDetection> = (0..100)
            .map(|i| SceneDetection { time: i as f64 * 15.0, score: 0.6 })
            .collect();
        let motion = MotionProfile::from_energy(vec![]);
        let result = detect_signals(&scenes, &motion);
        assert!(result.len() <= MAX_SEGMENTS);
    }

    #[test]
    fn empty_inputs_no_crash() {
        assert!(detect_signals(&[], &MotionProfile::from_energy(vec![])).is_empty());
    }

    // ── Parser ──

    #[test]
    fn parse_scene_output_extracts_detections() {
        let stderr = "\
[Parsed_showinfo_1 @ 0x...] n:0 pts:12345 pts_time:5.123 ...\n\
lavfi.scene_score=0.856\n\
[Parsed_showinfo_1 @ 0x...] n:1 pts:72345 pts_time:30.456 ...\n\
lavfi.scene_score=0.432\n";

        let dets = parse_scene_output(stderr);
        assert_eq!(dets.len(), 2);
        assert!((dets[0].time - 5.123).abs() < 0.01);
        assert!((dets[0].score - 0.856).abs() < 0.01);
        assert!((dets[1].time - 30.456).abs() < 0.01);
    }

    #[test]
    fn parse_scene_output_merges_close_detections() {
        let stderr = "\
[info] n:0 pts_time:10.0\nlavfi.scene_score=0.5\n\
[info] n:1 pts_time:10.8\nlavfi.scene_score=0.9\n";

        let dets = parse_scene_output(stderr);
        assert_eq!(dets.len(), 1, "detections within merge window should merge");
        assert!((dets[0].score - 0.9).abs() < 0.01, "higher score should win");
    }

    #[test]
    fn parse_motion_output_produces_profile() {
        let content = "\
frame:0    pts:0      pts_time:0.0\n\
lavfi.mestimate.mean_motion.x=5.0\n\
lavfi.mestimate.mean_motion.y=3.0\n\
frame:1    pts:12000  pts_time:0.5\n\
lavfi.mestimate.mean_motion.x=2.0\n\
lavfi.mestimate.mean_motion.y=1.0\n\
frame:2    pts:24000  pts_time:1.0\n\
lavfi.mestimate.mean_motion.x=10.0\n\
lavfi.mestimate.mean_motion.y=8.0\n";

        let profile = parse_motion_output(content);
        assert_eq!(profile.energy.len(), 2); // seconds 0 and 1
        // Second 0: two frames, magnitudes ~5.83 and ~2.24, avg ~4.04
        // Second 1: one frame, magnitude ~12.81
        assert!(profile.energy[1] > profile.energy[0], "second 1 should have more motion");
    }

    #[test]
    fn parse_motion_output_empty() {
        let profile = parse_motion_output("");
        assert!(profile.energy.is_empty());
    }
}
