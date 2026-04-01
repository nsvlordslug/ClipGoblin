//! Audio spike analysis for clip detection.
//!
//! Extracts per-second RMS energy from a video file via ffmpeg, then
//! runs three detection strategies to find exciting moments:
//!
//! | Detector  | Finds                                   | Tags                        |
//! |-----------|-----------------------------------------|-----------------------------|
//! | Spike     | Brief loud moments (shouts, reactions)  | reaction, scream, explosion |
//! | Surge     | Quiet-to-loud transitions (jumpscares)  | ambush, jumpscare, shock    |
//! | Sustained | Prolonged high energy (fights, chaos)    | fight, chase, hype          |
//!
//! # Usage
//!
//! ```ignore
//! let segments = audio_signal::analyze("video.mp4", &ffmpeg_path)?;
//! // segments: Vec<SignalSegment> with score 0.0–1.0
//! ```

use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::AppError;
use crate::pipeline::{SignalMetadata, SignalSegment, SignalType};

// ═══════════════════════════════════════════════════════════════════
//  Configuration constants
// ═══════════════════════════════════════════════════════════════════

/// Window size in seconds for measuring "current" energy.
const WINDOW_SECS: usize = 3;
/// Seconds of audio before a window used to measure the preceding baseline.
const LOOKBACK_SECS: usize = 5;
/// Seconds within which two detections are merged into one.
const MERGE_GAP_SECS: f64 = 8.0;
/// Maximum number of signal segments to return.
const MAX_SEGMENTS: usize = 20;
/// Floor for spike threshold — avoids false positives on very quiet VODs.
const MIN_SPIKE_THRESHOLD: f64 = 0.25;
/// Minimum consecutive seconds above threshold to qualify as sustained energy.
const MIN_SUSTAINED_SECS: usize = 5;

// ═══════════════════════════════════════════════════════════════════
//  Audio profile — intermediate representation
// ═══════════════════════════════════════════════════════════════════

/// Per-second audio energy extracted from a media file.
///
/// All values are linear-scale 0.0 (silence) – 1.0 (maximum).
/// Constructed by [`extract_rms`]; consumed by [`detect_signals`].
#[derive(Debug, Clone)]
pub struct AudioProfile {
    /// RMS energy per second, linear scale.
    pub rms: Vec<f64>,
    /// Average RMS across the entire file.
    pub avg: f64,
    /// Standard deviation of the per-second RMS values.
    pub std_dev: f64,
}

impl AudioProfile {
    /// Build a profile from raw per-second RMS values.
    pub fn from_rms(rms: Vec<f64>) -> Self {
        let n = rms.len().max(1) as f64;
        let avg = rms.iter().sum::<f64>() / n;
        let variance = rms.iter().map(|v| (v - avg).powi(2)).sum::<f64>() / n;
        let std_dev = variance.sqrt();
        Self { rms, avg, std_dev }
    }

    /// Average RMS in a range of seconds (clamped to bounds).
    pub fn avg_in_range(&self, start: usize, end: usize) -> f64 {
        let s = start.min(self.rms.len());
        let e = end.min(self.rms.len());
        if e <= s {
            return 0.0;
        }
        self.rms[s..e].iter().sum::<f64>() / (e - s) as f64
    }

    /// Peak RMS in a range of seconds.
    pub fn peak_in_range(&self, start: usize, end: usize) -> f64 {
        let s = start.min(self.rms.len());
        let e = end.min(self.rms.len());
        self.rms[s..e]
            .iter()
            .cloned()
            .fold(0.0_f64, f64::max)
    }

    pub fn duration_secs(&self) -> usize {
        self.rms.len()
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Public API
// ═══════════════════════════════════════════════════════════════════

/// Analyze a media file's audio and return scored signal segments.
///
/// Runs ffmpeg to extract per-second RMS energy, then applies spike,
/// surge, and sustained-energy detectors.  Results are merged,
/// deduplicated, and capped at [`MAX_SEGMENTS`].
///
/// # Errors
///
/// Returns `AppError::Ffmpeg` if ffmpeg cannot be launched, exits
/// with an error, or produces no parseable audio data.
pub fn analyze(vod_path: &str, ffmpeg: &Path) -> Result<Vec<SignalSegment>, AppError> {
    let profile = extract_rms(vod_path, ffmpeg)?;
    Ok(detect_signals(&profile))
}

/// Run only the detection phase on a pre-extracted profile.
///
/// Useful for testing and for callers that already have audio data.
pub fn detect_signals(profile: &AudioProfile) -> Vec<SignalSegment> {
    if profile.rms.is_empty() {
        return Vec::new();
    }

    let mut all = Vec::new();
    all.extend(detect_spikes(profile));
    all.extend(detect_surges(profile));
    all.extend(detect_sustained(profile));

    merge_and_rank(&mut all);
    all.truncate(MAX_SEGMENTS);
    all
}

// ═══════════════════════════════════════════════════════════════════
//  ffmpeg extraction
// ═══════════════════════════════════════════════════════════════════

/// Run ffmpeg to extract per-second RMS audio levels.
///
/// Uses the `astats` filter with per-frame reset to emit one
/// RMS_level measurement per audio frame, then buckets them into
/// per-second averages converted from dB to linear scale.
pub fn extract_rms(vod_path: &str, ffmpeg: &Path) -> Result<AudioProfile, AppError> {
    let temp_file = std::env::temp_dir()
        .join("clipviral_audio")
        .join(format!("{}.txt", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(temp_file.parent().unwrap()).ok();

    // Escape the path for ffmpeg filter syntax — Windows drive-letter
    // colons (C:\) conflict with ffmpeg's filter parameter separator.
    let escaped = temp_file
        .to_str()
        .unwrap_or_default()
        .replace('\\', "/")
        .replace(':', "\\:");

    let mut cmd = Command::new(ffmpeg);
    cmd.arg("-i")
        .arg(vod_path)
        .arg("-af")
        .arg(format!(
            "astats=metadata=1:reset=1,ametadata=mode=print:file='{}'",
            escaped
        ))
        .arg("-vn")
        .arg("-f")
        .arg("null")
        .arg("-")
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let status = cmd
        .status()
        .map_err(|e| AppError::Ffmpeg(format!("Failed to launch ffmpeg: {e}")))?;

    if !status.success() {
        std::fs::remove_file(&temp_file).ok();
        return Err(AppError::Ffmpeg(
            "ffmpeg audio analysis exited with an error".into(),
        ));
    }

    let content = std::fs::read_to_string(&temp_file)
        .map_err(|e| AppError::Ffmpeg(format!("Failed to read audio stats: {e}")))?;
    std::fs::remove_file(&temp_file).ok();

    let rms = parse_astats(&content)?;

    log::info!(
        "Audio extraction: {} seconds of data (avg={:.3}, stddev={:.3})",
        rms.duration_secs(),
        rms.avg,
        rms.std_dev,
    );

    Ok(rms)
}

/// Parse ffmpeg astats metadata output into an [`AudioProfile`].
///
/// The file contains interleaved `frame:` lines (with `pts_time`) and
/// `lavfi.astats.Overall.RMS_level=` lines (in dB).  We bucket the
/// dB values into per-second averages and convert to linear scale.
fn parse_astats(content: &str) -> Result<AudioProfile, AppError> {
    let mut current_time: Option<f64> = None;
    let mut current_rms: Option<f64> = None;
    let mut last_second: i64 = -1;
    let mut second_sum = 0.0_f64;
    let mut second_count = 0u32;
    let mut rms_values: Vec<f64> = Vec::new();

    for line in content.lines() {
        // Parse RMS level in dB
        if let Some(rest) = line.strip_prefix("lavfi.astats.Overall.RMS_level=") {
            if let Ok(val) = rest.trim().parse::<f64>() {
                current_rms = Some(val);
            }
        }
        // Parse frame timestamp
        else if line.starts_with("frame:") {
            if let Some(pts_pos) = line.find("pts_time:") {
                let pts_str = &line[pts_pos + 9..];
                let end = pts_str
                    .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
                    .unwrap_or(pts_str.len());
                if let Ok(t) = pts_str[..end].parse::<f64>() {
                    current_time = Some(t);
                }
            }
        }

        // Bucket into per-second averages
        if let (Some(t), Some(rms_db)) = (current_time, current_rms) {
            let sec = t as i64;
            if sec != last_second && last_second >= 0 && second_count > 0 {
                let avg_db = second_sum / second_count as f64;
                // Convert dB to linear: -60 dB → 0.0, 0 dB → 1.0
                let linear = ((avg_db + 60.0) / 60.0).clamp(0.0, 1.0);
                // Fill any gaps with silence
                while rms_values.len() < last_second as usize {
                    rms_values.push(0.0);
                }
                rms_values.push(linear);
                second_sum = 0.0;
                second_count = 0;
            }
            last_second = sec;
            second_sum += rms_db;
            second_count += 1;
            current_rms = None;
        }
    }

    // Flush the last bucket
    if second_count > 0 {
        let avg_db = second_sum / second_count as f64;
        let linear = ((avg_db + 60.0) / 60.0).clamp(0.0, 1.0);
        while rms_values.len() < last_second as usize {
            rms_values.push(0.0);
        }
        rms_values.push(linear);
    }

    if rms_values.is_empty() {
        return Err(AppError::Ffmpeg("No audio data found in file".into()));
    }

    Ok(AudioProfile::from_rms(rms_values))
}

// ═══════════════════════════════════════════════════════════════════
//  Detector 1 — Spikes (brief loud moments)
// ═══════════════════════════════════════════════════════════════════
//
//  Scoring formula:
//    raw  = (window_energy - avg) / std_dev   (z-score)
//    score = (raw / max_raw) * 0.90 + 0.10    (normalised, floor at 0.10)
//
//  A z-score of 2+ means the window is 2 standard deviations above
//  average — statistically unusual and perceptually loud.

fn detect_spikes(profile: &AudioProfile) -> Vec<SignalSegment> {
    let len = profile.duration_secs();
    if len < WINDOW_SECS * 2 {
        return Vec::new();
    }

    let threshold = (profile.avg + 2.0 * profile.std_dev).max(MIN_SPIKE_THRESHOLD);

    struct Hit {
        sec: usize,
        energy: f64,
        peak: f64,
        z_score: f64,
    }

    let mut hits: Vec<Hit> = Vec::new();

    for i in 0..len.saturating_sub(WINDOW_SECS) {
        let energy = profile.avg_in_range(i, i + WINDOW_SECS);
        if energy <= threshold {
            continue;
        }
        let peak = profile.peak_in_range(i, i + WINDOW_SECS);
        let z = if profile.std_dev > 0.001 {
            (energy - profile.avg) / profile.std_dev
        } else {
            energy / profile.avg.max(0.001)
        };
        hits.push(Hit { sec: i, energy, peak, z_score: z });
    }

    if hits.is_empty() {
        return Vec::new();
    }

    // Sort by z-score descending, then deduplicate within a time gap
    hits.sort_by(|a, b| b.z_score.partial_cmp(&a.z_score).unwrap_or(std::cmp::Ordering::Equal));
    let max_z = hits[0].z_score.max(1.0);

    let mut used: Vec<usize> = Vec::new();
    let mut segments = Vec::new();

    for hit in &hits {
        if used.iter().any(|&u| (hit.sec as i64 - u as i64).unsigned_abs() < MERGE_GAP_SECS as u64) {
            continue;
        }
        used.push(hit.sec);

        // Normalise: scale z-score into 0.10–1.00 range
        let score = ((hit.z_score / max_z) * 0.90 + 0.10).clamp(0.0, 1.0);

        let ratio = hit.energy / profile.avg.max(0.001);
        let tags = tag_by_intensity(ratio, 0.0);

        segments.push(SignalSegment {
            signal_type: SignalType::Audio,
            start_time: hit.sec as f64,
            end_time: (hit.sec + WINDOW_SECS) as f64,
            score,
            tags,
            metadata: Some(SignalMetadata::Audio {
                rms_delta: (hit.energy - profile.avg).max(0.0),
                peak_rms: hit.peak,
                ratio_above_avg: ratio,
            }),
        });
    }

    segments
}

// ═══════════════════════════════════════════════════════════════════
//  Detector 2 — Surges (quiet-to-loud transitions)
// ═══════════════════════════════════════════════════════════════════
//
//  Scoring formula:
//    delta  = window_energy - before_energy
//    ratio  = window_energy / before_energy
//    raw    = delta * 2.0 + ratio
//    score  = (raw / max_raw) * 0.90 + 0.10
//
//  A high delta + high ratio means the audio jumped dramatically
//  from a quiet baseline — classic jumpscare / reaction pattern.

fn detect_surges(profile: &AudioProfile) -> Vec<SignalSegment> {
    let len = profile.duration_secs();
    if len < LOOKBACK_SECS + WINDOW_SECS {
        return Vec::new();
    }

    struct Hit {
        sec: usize,
        delta: f64,
        before: f64,
        during: f64,
        peak: f64,
        raw: f64,
    }

    let mut hits: Vec<Hit> = Vec::new();

    for i in LOOKBACK_SECS..len.saturating_sub(WINDOW_SECS) {
        let before = profile.avg_in_range(i - LOOKBACK_SECS, i);
        let during = profile.avg_in_range(i, i + WINDOW_SECS);
        let delta = during - before;

        // Only keep positive jumps where "before" was relatively quiet
        if delta <= 0.0 || before >= profile.avg * 1.2 {
            continue;
        }
        // The jump must be meaningful
        if during <= profile.avg * 1.3 {
            continue;
        }

        let ratio = during / before.max(0.001);
        let raw = delta * 2.0 + ratio;
        let peak = profile.peak_in_range(i, i + WINDOW_SECS);
        hits.push(Hit { sec: i, delta, before, during, peak, raw });
    }

    if hits.is_empty() {
        return Vec::new();
    }

    hits.sort_by(|a, b| b.raw.partial_cmp(&a.raw).unwrap_or(std::cmp::Ordering::Equal));
    let max_raw = hits[0].raw.max(1.0);

    let mut used: Vec<usize> = Vec::new();
    let mut segments = Vec::new();

    for hit in &hits {
        if used.iter().any(|&u| (hit.sec as i64 - u as i64).unsigned_abs() < MERGE_GAP_SECS as u64) {
            continue;
        }
        used.push(hit.sec);

        let score = ((hit.raw / max_raw) * 0.90 + 0.10).clamp(0.0, 1.0);
        let ratio = hit.during / hit.before.max(0.001);

        // Surges from a quiet baseline get jumpscare/ambush tags
        let tags = if hit.before < profile.avg * 0.7 && ratio > 2.0 {
            vec!["ambush", "jumpscare", "shock"]
        } else if ratio > 2.5 {
            vec!["scream", "reaction", "shock"]
        } else {
            vec!["surge", "reaction"]
        }
        .into_iter()
        .map(String::from)
        .collect();

        segments.push(SignalSegment {
            signal_type: SignalType::Audio,
            start_time: hit.sec as f64,
            end_time: (hit.sec + WINDOW_SECS) as f64,
            score,
            tags,
            metadata: Some(SignalMetadata::Audio {
                rms_delta: hit.delta,
                peak_rms: hit.peak,
                ratio_above_avg: hit.during / profile.avg.max(0.001),
            }),
        });
    }

    segments
}

// ═══════════════════════════════════════════════════════════════════
//  Detector 3 — Sustained energy (prolonged high-intensity sections)
// ═══════════════════════════════════════════════════════════════════
//
//  Scoring formula:
//    avg_energy = average RMS over the sustained run
//    duration   = run length in seconds
//    raw        = (avg_energy / avg) * (duration / 30).min(1.0)
//    score      = (raw / max_raw) * 0.85 + 0.10
//
//  Longer and louder runs score higher.  The duration factor
//  saturates at 30 seconds to avoid over-weighting hour-long
//  action sequences.

fn detect_sustained(profile: &AudioProfile) -> Vec<SignalSegment> {
    let threshold = (profile.avg * 1.4).max(MIN_SPIKE_THRESHOLD);

    // Find contiguous runs of seconds above threshold
    let mut runs: Vec<(usize, usize)> = Vec::new(); // (start, end) inclusive
    let mut run_start: Option<usize> = None;

    for (i, &val) in profile.rms.iter().enumerate() {
        if val > threshold {
            if run_start.is_none() {
                run_start = Some(i);
            }
        } else if let Some(start) = run_start {
            if i - start >= MIN_SUSTAINED_SECS {
                runs.push((start, i));
            }
            run_start = None;
        }
    }
    // Close any open run at the end
    if let Some(start) = run_start {
        let end = profile.rms.len();
        if end - start >= MIN_SUSTAINED_SECS {
            runs.push((start, end));
        }
    }

    if runs.is_empty() {
        return Vec::new();
    }

    struct Run {
        start: usize,
        end: usize,
        avg_energy: f64,
        peak: f64,
        raw: f64,
    }

    let scored_runs: Vec<Run> = runs
        .iter()
        .map(|&(s, e)| {
            let avg_energy = profile.avg_in_range(s, e);
            let peak = profile.peak_in_range(s, e);
            let duration = (e - s) as f64;
            let duration_factor = (duration / 30.0).min(1.0);
            let raw = (avg_energy / profile.avg.max(0.001)) * duration_factor;
            Run { start: s, end: e, avg_energy, peak, raw }
        })
        .collect();

    let max_raw = scored_runs
        .iter()
        .map(|r| r.raw)
        .fold(1.0_f64, f64::max);

    scored_runs
        .iter()
        .map(|run| {
            let score = ((run.raw / max_raw) * 0.85 + 0.10).clamp(0.0, 1.0);
            let duration = run.end - run.start;

            let tags: Vec<String> = if duration >= 15 {
                vec!["fight", "chaos", "hype"]
            } else if duration >= 10 {
                vec!["fight", "encounter"]
            } else {
                vec!["chase", "encounter"]
            }
            .into_iter()
            .map(String::from)
            .collect();

            SignalSegment {
                signal_type: SignalType::Audio,
                start_time: run.start as f64,
                end_time: run.end as f64,
                score,
                tags,
                metadata: Some(SignalMetadata::Audio {
                    rms_delta: (run.avg_energy - profile.avg).max(0.0),
                    peak_rms: run.peak,
                    ratio_above_avg: run.avg_energy / profile.avg.max(0.001),
                }),
            }
        })
        .collect()
}

// ═══════════════════════════════════════════════════════════════════
//  Merge and rank
// ═══════════════════════════════════════════════════════════════════

/// Merge overlapping segments (keeping the higher scorer), then sort
/// by score descending.
fn merge_and_rank(segments: &mut Vec<SignalSegment>) {
    // Sort by start time first for the merge pass
    segments.sort_by(|a, b| {
        a.start_time
            .partial_cmp(&b.start_time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Merge overlapping or near-adjacent segments
    let mut merged: Vec<SignalSegment> = Vec::with_capacity(segments.len());
    for seg in segments.drain(..) {
        if let Some(last) = merged.last_mut() {
            // Overlaps or within merge gap?
            if seg.start_time <= last.end_time + MERGE_GAP_SECS {
                // Keep whichever has the higher score
                if seg.score > last.score {
                    // Extend the time range but use the better segment's data
                    let merged_start = last.start_time.min(seg.start_time);
                    let merged_end = last.end_time.max(seg.end_time);
                    *last = seg;
                    last.start_time = merged_start;
                    last.end_time = merged_end;
                } else {
                    last.end_time = last.end_time.max(seg.end_time);
                }
                continue;
            }
        }
        merged.push(seg);
    }

    // Sort by score descending for final ranking
    merged.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    *segments = merged;
}

// ═══════════════════════════════════════════════════════════════════
//  Helpers
// ═══════════════════════════════════════════════════════════════════

/// Assign semantic tags based on how loud a spike is relative to baseline.
fn tag_by_intensity(ratio: f64, _delta: f64) -> Vec<String> {
    if ratio > 3.0 {
        vec!["explosion", "scream", "reaction"]
    } else if ratio > 2.2 {
        vec!["scream", "reaction", "fight"]
    } else if ratio > 1.6 {
        vec!["reaction", "hype"]
    } else {
        vec!["encounter", "skirmish"]
    }
    .into_iter()
    .map(String::from)
    .collect()
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic audio profile for testing.
    /// Base energy is 0.3, with optional spikes injected.
    fn make_profile(len: usize, spikes: &[(usize, usize, f64)]) -> AudioProfile {
        let mut rms = vec![0.3; len];
        for &(start, end, val) in spikes {
            for i in start..end.min(len) {
                rms[i] = val;
            }
        }
        AudioProfile::from_rms(rms)
    }

    // ── AudioProfile ──

    #[test]
    fn profile_statistics() {
        let p = AudioProfile::from_rms(vec![0.2, 0.4, 0.6]);
        assert!((p.avg - 0.4).abs() < 1e-9);
        assert!(p.std_dev > 0.0);
    }

    #[test]
    fn profile_avg_in_range_clamped() {
        let p = AudioProfile::from_rms(vec![0.1, 0.2, 0.3, 0.4]);
        assert!((p.avg_in_range(1, 3) - 0.25).abs() < 1e-9);
        // Out-of-bounds is clamped, not panicked
        assert!(p.avg_in_range(10, 20) == 0.0);
    }

    #[test]
    fn profile_peak_in_range() {
        let p = AudioProfile::from_rms(vec![0.1, 0.5, 0.2, 0.8, 0.3]);
        assert!((p.peak_in_range(0, 5) - 0.8).abs() < 1e-9);
        assert!((p.peak_in_range(0, 2) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn empty_profile_returns_no_segments() {
        let p = AudioProfile::from_rms(vec![]);
        assert!(detect_signals(&p).is_empty());
    }

    // ── Spike detector ──

    #[test]
    fn spike_detected_on_loud_burst() {
        // 60 seconds of quiet with a 3-second shout at second 30
        let p = make_profile(60, &[(30, 33, 0.9)]);
        let segs = detect_spikes(&p);

        assert!(!segs.is_empty(), "should detect the loud burst");
        let best = &segs[0];
        assert!(best.start_time >= 28.0 && best.start_time <= 32.0);
        assert!(best.score > 0.5, "loud burst should score high");
    }

    #[test]
    fn no_spikes_on_flat_audio() {
        let p = AudioProfile::from_rms(vec![0.3; 120]);
        let segs = detect_spikes(&p);
        assert!(segs.is_empty(), "flat audio should produce no spikes");
    }

    // ── Surge detector ──

    #[test]
    fn surge_detected_on_quiet_to_loud() {
        // 10 seconds quiet, then 5 seconds loud
        let mut rms = vec![0.1; 20];
        for i in 10..15 {
            rms[i] = 0.85;
        }
        // Pad with quiet so average stays low
        rms.extend(vec![0.1; 30]);
        let p = AudioProfile::from_rms(rms);
        let segs = detect_surges(&p);

        assert!(!segs.is_empty(), "quiet-to-loud jump should be detected");
        let best = &segs[0];
        assert!(
            best.tags.iter().any(|t| t == "ambush" || t == "shock" || t == "jumpscare"),
            "surge from quiet should get jumpscare-type tags"
        );
    }

    // ── Sustained detector ──

    #[test]
    fn sustained_detected_on_long_loud_section() {
        // 10 seconds loud in a 60-second VOD
        let p = make_profile(60, &[(20, 35, 0.8)]);
        let segs = detect_sustained(&p);

        assert!(!segs.is_empty(), "15-second loud section should be detected");
        let best = &segs[0];
        assert!(best.duration() >= 10.0);
        assert!(best.tags.iter().any(|t| t == "fight" || t == "chaos"));
    }

    #[test]
    fn sustained_ignores_short_bursts() {
        // 3-second burst is too short for sustained
        let p = make_profile(60, &[(30, 33, 0.9)]);
        let segs = detect_sustained(&p);
        assert!(segs.is_empty(), "3-second burst is below MIN_SUSTAINED_SECS");
    }

    // ── Full pipeline ──

    #[test]
    fn detect_signals_combines_all_detectors() {
        // A profile with both a spike and a sustained section
        let p = make_profile(120, &[
            (30, 33, 0.95),  // spike
            (70, 85, 0.75),  // sustained 15s
        ]);
        let segs = detect_signals(&p);

        assert!(segs.len() >= 2, "should detect both events");
        // Best score should be > 0.5
        assert!(segs[0].score > 0.5);
        // All segments should be Audio type
        assert!(segs.iter().all(|s| s.signal_type == SignalType::Audio));
        // All scores in valid range
        assert!(segs.iter().all(|s| s.score > 0.0 && s.score <= 1.0));
    }

    #[test]
    fn segments_capped_at_max() {
        // Many spikes should still produce at most MAX_SEGMENTS
        let mut rms = vec![0.2; 600];
        for i in (10..580).step_by(20) {
            rms[i] = 0.95;
            rms[i + 1] = 0.90;
        }
        let p = AudioProfile::from_rms(rms);
        let segs = detect_signals(&p);
        assert!(segs.len() <= MAX_SEGMENTS);
    }

    #[test]
    fn scores_are_normalised_0_to_1() {
        let p = make_profile(120, &[
            (20, 23, 0.95),
            (50, 55, 0.70),
            (90, 93, 0.80),
        ]);
        let segs = detect_signals(&p);
        for seg in &segs {
            assert!(seg.score >= 0.0 && seg.score <= 1.0,
                "score {} out of range", seg.score);
        }
    }

    #[test]
    fn metadata_is_populated() {
        let p = make_profile(60, &[(30, 33, 0.9)]);
        let segs = detect_signals(&p);
        assert!(!segs.is_empty());
        match &segs[0].metadata {
            Some(SignalMetadata::Audio { rms_delta, peak_rms, ratio_above_avg }) => {
                assert!(*rms_delta > 0.0);
                assert!(*peak_rms > 0.0);
                assert!(*ratio_above_avg > 1.0);
            }
            other => panic!("Expected Audio metadata, got {:?}", other),
        }
    }

    // ── Merge ──

    #[test]
    fn overlapping_segments_are_merged() {
        let mut segs = vec![
            SignalSegment {
                signal_type: SignalType::Audio,
                start_time: 10.0,
                end_time: 15.0,
                score: 0.6,
                tags: vec!["a".into()],
                metadata: None,
            },
            SignalSegment {
                signal_type: SignalType::Audio,
                start_time: 12.0,
                end_time: 18.0,
                score: 0.9,
                tags: vec!["b".into()],
                metadata: None,
            },
        ];
        merge_and_rank(&mut segs);
        assert_eq!(segs.len(), 1, "overlapping segments should merge");
        assert!((segs[0].score - 0.9).abs() < f64::EPSILON, "higher score wins");
        assert!((segs[0].start_time - 10.0).abs() < f64::EPSILON, "start is the earlier");
        assert!((segs[0].end_time - 18.0).abs() < f64::EPSILON, "end is the later");
    }

    // ── astats parser ──

    #[test]
    fn parse_astats_basic() {
        let input = "\
frame:0    pts:0      pts_time:0.0
lavfi.astats.Overall.RMS_level=-30.0
frame:1    pts:48000  pts_time:1.0
lavfi.astats.Overall.RMS_level=-20.0
frame:2    pts:96000  pts_time:2.0
lavfi.astats.Overall.RMS_level=-10.0
";
        let profile = parse_astats(input).unwrap();
        assert_eq!(profile.rms.len(), 3);
        // -30 dB → (30/60) = 0.5, -20 dB → (40/60) ≈ 0.667, -10 dB → (50/60) ≈ 0.833
        assert!((profile.rms[0] - 0.5).abs() < 0.01);
        assert!(profile.rms[1] > profile.rms[0], "louder second should have higher RMS");
        assert!(profile.rms[2] > profile.rms[1]);
    }

    #[test]
    fn parse_astats_empty_returns_error() {
        assert!(parse_astats("").is_err());
        assert!(parse_astats("no valid data here").is_err());
    }
}
