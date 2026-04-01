//! Signal fusion and candidate-clip generation.
//!
//! Takes raw [`SignalSegment`] entries from all detectors (audio,
//! transcript, scene change, optional vision) and produces ranked
//! [`CandidateClip`] entries ready for the frontend.
//!
//! # Pipeline
//!
//! ```text
//! Vec<SignalSegment>  (all sources, flat)
//!        │
//!        ▼
//!   ┌─ cluster_signals ──────────────────────────────────────┐
//!   │  Sort by time, absorb neighbors within FUSION_WINDOW   │
//!   │  → Vec<Cluster>                                        │
//!   └───────────────────────┬────────────────────────────────┘
//!                           ▼
//!   ┌─ expand_to_clips ─────────────────────────────────────┐
//!   │  Pad each cluster to MIN–MAX duration, center on peak  │
//!   │  Compute per-signal scores → ClipScoreBreakdown        │
//!   │  → Vec<CandidateClip>                                  │
//!   └───────────────────────┬────────────────────────────────┘
//!                           ▼
//!   ┌─ dedup_clips ─────────────────────────────────────────┐
//!   │  Merge overlapping clips, keep higher scorer           │
//!   │  → Vec<CandidateClip>                                  │
//!   └───────────────────────┬────────────────────────────────┘
//!                           ▼
//!   ┌─ reject_weak ─────────────────────────────────────────┐
//!   │  Mark clips that fail quality gates                     │
//!   │  → Vec<CandidateClip>  (some with rejection_reason)    │
//!   └───────────────────────┬────────────────────────────────┘
//!                           ▼
//!   ┌─ rank ────────────────────────────────────────────────┐
//!   │  Sort by best raw signal, cap at max_candidates        │
//!   │  → Vec<CandidateClip>  (final, accepted only)          │
//!   └───────────────────────────────────────────────────────┘
//! ```

use crate::pipeline::{
    CandidateClip, ClipScoreBreakdown, SignalMetadata, SignalSegment, SignalType,
};

// ═══════════════════════════════════════════════════════════════════
//  Configuration
// ═══════════════════════════════════════════════════════════════════

/// Tuning knobs for the fusion pipeline.
///
/// Fusion is concerned with **grouping and shaping** — merging
/// signals into clip-sized windows.  Scoring weights, rejection
/// thresholds, and output caps live in [`crate::clip_ranker::WeightProfile`].
#[derive(Debug, Clone)]
pub struct FusionConfig {
    /// Seconds within which signals are merged into a single cluster.
    pub fusion_window: f64,
    /// Minimum clip duration after expansion (seconds).
    pub min_duration: f64,
    /// Maximum clip duration after expansion (seconds).
    pub max_duration: f64,
    /// Seconds of padding added before the cluster start.
    pub pad_before: f64,
    /// Seconds of padding added after the cluster end.
    pub pad_after: f64,
    /// Maximum overlap (seconds) between two clips before they
    /// are considered duplicates and the weaker one is discarded.
    pub dedup_overlap: f64,
    /// Maximum candidate clips to produce.  Set by the engine as
    /// a multiple of the user's `max_clips` — the fusion stage
    /// generates a generous pool for the ranker to select from.
    pub max_candidates: usize,
    /// Minimum raw signal score (best of any single signal) to
    /// keep a single-source clip.  Multi-source clips bypass this.
    pub single_source_min: f64,
    /// Total VOD duration (used to clamp boundaries).
    pub vod_duration: f64,
}

impl FusionConfig {
    pub fn new(vod_duration: f64) -> Self {
        Self {
            fusion_window: 10.0,
            min_duration: 15.0,
            max_duration: 60.0,
            pad_before: 2.0,
            pad_after: 3.0,
            dedup_overlap: 8.0,
            max_candidates: 30, // overridden by the engine from top_n
            single_source_min: 0.40,
            vod_duration,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Internal cluster
// ═══════════════════════════════════════════════════════════════════

/// A group of temporally close signals before expansion to a clip.
#[derive(Debug, Clone)]
struct Cluster {
    /// Earliest signal start in the cluster.
    start: f64,
    /// Latest signal end in the cluster.
    end: f64,
    /// All signals absorbed into this cluster.
    signals: Vec<SignalSegment>,
}

impl Cluster {
    fn center(&self) -> f64 {
        (self.start + self.end) / 2.0
    }

    /// Best signal per source type (highest score wins).
    fn best_by_type(&self) -> Vec<(SignalType, &SignalSegment)> {
        let mut bests: std::collections::HashMap<SignalType, &SignalSegment> =
            std::collections::HashMap::new();

        for sig in &self.signals {
            let entry = bests.entry(sig.signal_type).or_insert(sig);
            if sig.score > entry.score {
                *entry = sig;
            }
        }

        let mut result: Vec<_> = bests.into_iter().collect();
        result.sort_by_key(|(t, _)| *t as u8);
        result
    }

    /// Distinct signal types present.
    fn source_types(&self) -> Vec<SignalType> {
        let mut types: Vec<SignalType> = self
            .signals
            .iter()
            .map(|s| s.signal_type)
            .collect();
        types.sort_by_key(|t| *t as u8);
        types.dedup();
        types
    }

    /// Merged, deduplicated tags from all signals.
    fn merged_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self
            .signals
            .iter()
            .flat_map(|s| s.tags.iter().cloned())
            .collect();
        tags.sort();
        tags.dedup();
        tags
    }

    /// Extract the best transcript snippet from any transcript signal.
    fn transcript_excerpt(&self) -> Option<String> {
        self.signals
            .iter()
            .filter(|s| s.signal_type == SignalType::Transcript)
            .filter_map(|s| match &s.metadata {
                Some(SignalMetadata::Transcript { text, .. }) if !text.is_empty() => {
                    Some(text.clone())
                }
                _ => None,
            })
            .max_by_key(|t| t.len())
    }

    /// Extract vision description if present.
    fn vision_description(&self) -> Option<String> {
        self.signals
            .iter()
            .filter(|s| s.signal_type == SignalType::Vision)
            .filter_map(|s| match &s.metadata {
                Some(SignalMetadata::Vision { description, .. }) if !description.is_empty() => {
                    Some(description.clone())
                }
                _ => None,
            })
            .next()
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Public API
// ═══════════════════════════════════════════════════════════════════

/// Run the full fusion pipeline: cluster → expand → dedup → reject → rank.
///
/// Input: flat `Vec<SignalSegment>` from all signal providers combined.
/// Output: ranked `Vec<CandidateClip>`, accepted clips only, best first.
pub fn fuse(
    segments: &[SignalSegment],
    config: &FusionConfig,
) -> Vec<CandidateClip> {
    if segments.is_empty() {
        return Vec::new();
    }

    // 1. Cluster nearby signals
    let clusters = cluster_signals(segments, config.fusion_window);

    log::info!("Fusion: {} signals → {} clusters", segments.len(), clusters.len());

    // 2. Expand clusters to clip-length candidates with score breakdowns
    let mut clips: Vec<CandidateClip> = clusters
        .iter()
        .map(|c| expand_to_clip(c, config))
        .collect();

    // 3. Deduplicate overlapping clips
    dedup_clips(&mut clips, config.dedup_overlap);

    // 4. Mark weak clips with rejection reasons
    reject_weak(&mut clips, config);

    // 5. Keep only accepted clips, sort by best raw signal, cap
    clips.retain(|c| c.is_accepted());
    clips.sort_by(|a, b| {
        best_raw(&b.score_breakdown)
            .partial_cmp(&best_raw(&a.score_breakdown))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    clips.truncate(config.max_candidates);

    log::info!("Fusion output: {} candidate clips", clips.len());
    clips
}

// ═══════════════════════════════════════════════════════════════════
//  Stage 1 — Signal clustering
// ═══════════════════════════════════════════════════════════════════
//
//  Algorithm:
//    1. Sort all segments by center timestamp.
//    2. Start a cluster at the first segment.
//    3. For each subsequent segment:
//       - If its center is within fusion_window of the cluster's
//         latest signal end, absorb it into the cluster.
//       - Otherwise, close the current cluster and start a new one.

fn cluster_signals(segments: &[SignalSegment], fusion_window: f64) -> Vec<Cluster> {
    let mut sorted: Vec<&SignalSegment> = segments.iter().collect();
    sorted.sort_by(|a, b| {
        a.center()
            .partial_cmp(&b.center())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut clusters: Vec<Cluster> = Vec::new();

    for seg in sorted {
        let absorbed = if let Some(last) = clusters.last_mut() {
            // Absorb if this signal's center is close to the cluster's range
            if seg.center() <= last.end + fusion_window {
                last.start = last.start.min(seg.start_time);
                last.end = last.end.max(seg.end_time);
                last.signals.push(seg.clone());
                true
            } else {
                false
            }
        } else {
            false
        };

        if !absorbed {
            clusters.push(Cluster {
                start: seg.start_time,
                end: seg.end_time,
                signals: vec![seg.clone()],
            });
        }
    }

    clusters
}

// ═══════════════════════════════════════════════════════════════════
//  Stage 2 — Expand cluster to candidate clip
// ═══════════════════════════════════════════════════════════════════
//
//  1. Add padding before/after the cluster's raw time range.
//  2. Enforce min/max duration by expanding or trimming symmetrically
//     around the cluster's center.
//  3. Compute per-signal-type scores from the best segment of each type.
//  4. Apply multi-signal bonus.
//  5. Build CandidateClip with metadata extracted from segments.

fn expand_to_clip(cluster: &Cluster, config: &FusionConfig) -> CandidateClip {
    // ── Timing ──
    let raw_start = cluster.start - config.pad_before;
    let raw_end = cluster.end + config.pad_after;
    let raw_dur = raw_end - raw_start;

    let (start, end) = if raw_dur < config.min_duration {
        // Too short: expand symmetrically around center
        let center = cluster.center();
        let half = config.min_duration / 2.0;
        (center - half, center + half)
    } else if raw_dur > config.max_duration {
        // Too long: trim symmetrically around center
        let center = cluster.center();
        let half = config.max_duration / 2.0;
        (center - half, center + half)
    } else {
        (raw_start, raw_end)
    };

    // Clamp to VOD bounds
    let start = start.max(0.0);
    let end = end.min(config.vod_duration);

    // ── Scoring ──
    let bests = cluster.best_by_type();

    let audio = bests
        .iter()
        .find(|(t, _)| *t == SignalType::Audio)
        .map(|(_, s)| s.score)
        .unwrap_or(0.0);
    let speech = bests
        .iter()
        .find(|(t, _)| *t == SignalType::Transcript)
        .map(|(_, s)| s.score)
        .unwrap_or(0.0);
    let scene = bests
        .iter()
        .find(|(t, _)| *t == SignalType::SceneChange)
        .map(|(_, s)| s.score)
        .unwrap_or(0.0);
    let vision = bests
        .iter()
        .find(|(t, _)| *t == SignalType::Vision)
        .map(|(_, s)| s.score);

    let breakdown = ClipScoreBreakdown::new(audio, speech, scene, vision);

    // ── Metadata ──
    let sources = cluster.source_types();
    let tags = cluster.merged_tags();
    let fingerprint = build_fingerprint(&tags);
    let transcript = cluster.transcript_excerpt();
    let title = cluster.vision_description();

    // confidence_score stays 0.0 — the ranker will set it.
    let mut clip = CandidateClip::new(start, end, breakdown, sources);
    clip.tags = tags;
    clip.fingerprint = fingerprint;
    clip.transcript_excerpt = transcript;
    clip.title = title;

    clip
}

/// Build a deduplication fingerprint from the two most prominent tags.
///
/// Sorts tags alphabetically and joins the first two with '+'.
/// Clips with the same fingerprint are treated as similar during dedup.
fn build_fingerprint(tags: &[String]) -> String {
    let mut sorted = tags.to_vec();
    sorted.sort();
    sorted
        .iter()
        .take(2)
        .cloned()
        .collect::<Vec<_>>()
        .join("+")
}

// ═══════════════════════════════════════════════════════════════════
//  Stage 3 — Deduplication
// ═══════════════════════════════════════════════════════════════════
//
//  Two clips are considered duplicates if they overlap by more than
//  `dedup_overlap` seconds.  The one with the lower score is removed.
//  Clips with different fingerprints survive a weaker overlap test.

fn dedup_clips(clips: &mut Vec<CandidateClip>, max_overlap: f64) {
    // Sort by best raw signal descending so the strongest clip wins ties
    clips.sort_by(|a, b| {
        best_raw(&b.score_breakdown)
            .partial_cmp(&best_raw(&a.score_breakdown))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut keep: Vec<CandidateClip> = Vec::with_capacity(clips.len());

    for clip in clips.drain(..) {
        let dominated = keep.iter().any(|kept| {
            let overlap = overlap_secs(kept, &clip);
            if clip.fingerprint == kept.fingerprint {
                overlap > max_overlap * 0.5 // stricter for same type
            } else {
                overlap > max_overlap
            }
        });

        if !dominated {
            keep.push(clip);
        }
    }

    *clips = keep;
}

/// Best raw signal score across all signal types.
///
/// Used for sorting before the ranker assigns weighted composites.
fn best_raw(b: &ClipScoreBreakdown) -> f64 {
    b.audio_score
        .max(b.speech_score)
        .max(b.scene_score)
        .max(b.vision_score.unwrap_or(0.0))
}

/// Compute overlap between two clips in seconds (0.0 if no overlap).
fn overlap_secs(a: &CandidateClip, b: &CandidateClip) -> f64 {
    let start = a.start_time.max(b.start_time);
    let end = a.end_time.min(b.end_time);
    (end - start).max(0.0)
}

// ═══════════════════════════════════════════════════════════════════
//  Stage 4 — Rejection
// ═══════════════════════════════════════════════════════════════════

/// Reject structurally weak clips.
///
/// This checks raw signal quality and duration only — weighted
/// score thresholds are enforced later by the ranker.
fn reject_weak(clips: &mut [CandidateClip], config: &FusionConfig) {
    for clip in clips.iter_mut() {
        let dur = clip.duration();
        if dur < config.min_duration * 0.5 {
            clip.rejection_reason = Some(format!(
                "Duration {:.1}s too short (minimum {:.0}s)",
                dur, config.min_duration * 0.5
            ));
            continue;
        }

        // Single-source clips need a strong raw signal to survive
        // without corroboration from other sources.
        if clip.signal_count() == 1
            && clip.transcript_excerpt.is_none()
            && clip.score_breakdown.vision_score.is_none()
        {
            let best_raw = clip.score_breakdown.audio_score
                .max(clip.score_breakdown.speech_score)
                .max(clip.score_breakdown.scene_score);
            if best_raw < config.single_source_min {
                clip.rejection_reason = Some(format!(
                    "Single signal source (best raw {:.2}) below {:.2}",
                    best_raw, config.single_source_min,
                ));
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn audio_seg(start: f64, end: f64, score: f64) -> SignalSegment {
        SignalSegment {
            signal_type: SignalType::Audio,
            start_time: start,
            end_time: end,
            score,
            tags: vec!["reaction".into()],
            metadata: None,
        }
    }

    fn transcript_seg(start: f64, end: f64, score: f64, text: &str) -> SignalSegment {
        SignalSegment {
            signal_type: SignalType::Transcript,
            start_time: start,
            end_time: end,
            score,
            tags: vec!["shock".into()],
            metadata: Some(SignalMetadata::Transcript {
                text: text.into(),
                keyword: None,
                language: Some("en".into()),
            }),
        }
    }

    fn scene_seg(start: f64, end: f64, score: f64) -> SignalSegment {
        SignalSegment {
            signal_type: SignalType::SceneChange,
            start_time: start,
            end_time: end,
            score,
            tags: vec!["cut".into()],
            metadata: None,
        }
    }

    fn vision_seg(start: f64, end: f64, score: f64, desc: &str) -> SignalSegment {
        use crate::pipeline::VisionDimensionScores;
        SignalSegment {
            signal_type: SignalType::Vision,
            start_time: start,
            end_time: end,
            score,
            tags: vec!["action".into()],
            metadata: Some(SignalMetadata::Vision {
                description: desc.into(),
                model_scores: VisionDimensionScores::default(),
                provider: "Claude".into(),
            }),
        }
    }

    fn config() -> FusionConfig {
        FusionConfig::new(600.0)
    }

    // ── Clustering ──

    #[test]
    fn empty_input() {
        assert!(fuse(&[], &config()).is_empty());
    }

    #[test]
    fn single_signal_produces_one_clip() {
        let clips = fuse(&[audio_seg(30.0, 35.0, 0.7)], &config());
        assert_eq!(clips.len(), 1);
    }

    #[test]
    fn nearby_signals_fuse_into_one_cluster() {
        let segments = vec![
            audio_seg(30.0, 33.0, 0.8),
            transcript_seg(32.0, 35.0, 0.7, "no way"),
            scene_seg(34.0, 36.0, 0.5),
        ];
        let clips = fuse(&segments, &config());
        assert_eq!(clips.len(), 1, "signals within fusion window should merge");
        assert_eq!(clips[0].signal_sources.len(), 3);
    }

    #[test]
    fn distant_signals_stay_separate() {
        let segments = vec![
            audio_seg(30.0, 33.0, 0.90),
            audio_seg(200.0, 203.0, 0.85),
        ];
        let clips = fuse(&segments, &config());
        assert_eq!(clips.len(), 2);
    }

    // ── Score breakdown ──

    #[test]
    fn score_breakdown_reflects_best_per_type() {
        let segments = vec![
            audio_seg(30.0, 33.0, 0.80),
            audio_seg(31.0, 34.0, 0.60), // lower audio — should not win
            transcript_seg(32.0, 35.0, 0.70, "amazing"),
        ];
        let clips = fuse(&segments, &config());
        let b = &clips[0].score_breakdown;
        assert!((b.audio_score - 0.80).abs() < 0.01, "best audio should be 0.80");
        assert!((b.speech_score - 0.70).abs() < 0.01);
    }

    #[test]
    fn multi_signal_clips_have_more_sources() {
        let one_source = fuse(&[audio_seg(30.0, 33.0, 0.8)], &config());
        let three_sources = fuse(
            &[
                audio_seg(30.0, 33.0, 0.8),
                transcript_seg(31.0, 34.0, 0.7, "wow"),
                scene_seg(32.0, 35.0, 0.5),
            ],
            &config(),
        );
        assert_eq!(one_source[0].signal_count(), 1);
        assert_eq!(three_sources[0].signal_count(), 3);
    }

    #[test]
    fn vision_score_populates_breakdown() {
        let segments = vec![
            audio_seg(30.0, 33.0, 0.7),
            vision_seg(31.0, 45.0, 0.9, "streamer screams"),
        ];
        let clips = fuse(&segments, &config());
        assert!(clips[0].score_breakdown.vision_score.is_some());
    }

    // ── Duration expansion ──

    #[test]
    fn short_cluster_expanded_to_min_duration() {
        let clips = fuse(&[audio_seg(100.0, 102.0, 0.8)], &config());
        assert!(
            clips[0].duration() >= config().min_duration,
            "clip should be expanded to at least min_duration"
        );
    }

    #[test]
    fn long_cluster_trimmed_to_max_duration() {
        let segments = vec![audio_seg(100.0, 200.0, 0.7)];
        let clips = fuse(&segments, &config());
        assert!(
            clips[0].duration() <= config().max_duration,
            "clip should be capped at max_duration"
        );
    }

    #[test]
    fn clip_boundaries_clamped_to_vod() {
        // Cluster near the very start
        let clips = fuse(&[audio_seg(1.0, 3.0, 0.8)], &config());
        assert!(clips[0].start_time >= 0.0, "start should not be negative");
    }

    // ── Metadata extraction ──

    #[test]
    fn transcript_excerpt_extracted() {
        let segments = vec![
            audio_seg(30.0, 33.0, 0.8),
            transcript_seg(31.0, 34.0, 0.6, "Oh my god no way"),
        ];
        let clips = fuse(&segments, &config());
        assert_eq!(
            clips[0].transcript_excerpt.as_deref(),
            Some("Oh my god no way")
        );
    }

    #[test]
    fn vision_description_becomes_title() {
        let segments = vec![
            audio_seg(30.0, 33.0, 0.7),
            vision_seg(31.0, 45.0, 0.9, "Player falls off bridge in panic"),
        ];
        let clips = fuse(&segments, &config());
        assert_eq!(
            clips[0].title.as_deref(),
            Some("Player falls off bridge in panic")
        );
    }

    #[test]
    fn tags_merged_and_deduplicated() {
        let mut seg1 = audio_seg(30.0, 33.0, 0.8);
        seg1.tags = vec!["reaction".into(), "hype".into()];
        let mut seg2 = scene_seg(31.0, 34.0, 0.5);
        seg2.tags = vec!["reaction".into(), "cut".into()];
        let clips = fuse(&[seg1, seg2], &config());

        // "reaction" should appear once, not twice
        let reaction_count = clips[0].tags.iter().filter(|t| *t == "reaction").count();
        assert_eq!(reaction_count, 1);
    }

    // ── Deduplication ──

    #[test]
    fn overlapping_clips_deduplicated() {
        let segments = vec![
            audio_seg(30.0, 33.0, 0.95),
            audio_seg(35.0, 38.0, 0.60), // very close, will overlap after expansion
        ];
        let clips = fuse(&segments, &config());
        assert_eq!(clips.len(), 1, "overlapping clips should merge");
        // 0.95 * 0.45 = 0.4275 — the higher-scored signal should win
        assert!(
            clips[0].score_breakdown.audio_score > 0.90,
            "higher scorer should survive"
        );
    }

    #[test]
    fn non_overlapping_clips_preserved() {
        let segments = vec![
            audio_seg(30.0, 33.0, 0.95),
            audio_seg(300.0, 303.0, 0.85),
        ];
        let clips = fuse(&segments, &config());
        assert_eq!(clips.len(), 2);
    }

    // ── Rejection ──

    #[test]
    fn weak_single_source_rejected() {
        let segments = vec![scene_seg(30.0, 33.0, 0.1)]; // raw 0.1 < single_source_min
        let clips = fuse(&segments, &config());
        assert!(clips.is_empty(), "weak single-source clips should be rejected");
    }

    #[test]
    fn single_source_low_raw_rejected() {
        let segments = vec![audio_seg(30.0, 33.0, 0.40)]; // raw score < 0.55
        let cfg = FusionConfig { single_source_min: 0.55, ..config() };
        let clips = fuse(&segments, &cfg);
        assert!(
            clips.is_empty(),
            "single-source clip with raw signal < 0.55 should be rejected"
        );
    }

    #[test]
    fn single_source_high_raw_accepted() {
        let segments = vec![audio_seg(30.0, 33.0, 0.80)]; // raw score > 0.55
        let clips = fuse(&segments, &config());
        assert_eq!(clips.len(), 1, "strong single-source clip should survive");
    }

    // ── Ranking ──

    #[test]
    fn output_sorted_by_best_raw_descending() {
        let segments = vec![
            audio_seg(30.0, 33.0, 0.6),
            audio_seg(200.0, 203.0, 0.9),
            audio_seg(400.0, 403.0, 0.7),
        ];
        let clips = fuse(&segments, &config());
        for pair in clips.windows(2) {
            let a = best_raw(&pair[0].score_breakdown);
            let b = best_raw(&pair[1].score_breakdown);
            assert!(a >= b, "clips should be sorted by best raw signal descending");
        }
    }

    #[test]
    fn output_capped_at_max() {
        let segments: Vec<SignalSegment> = (0..30)
            .map(|i| audio_seg(i as f64 * 80.0, i as f64 * 80.0 + 5.0, 0.8))
            .collect();
        let clips = fuse(&segments, &config());
        assert!(clips.len() <= config().max_candidates);
    }

    // ── Fingerprinting ──

    #[test]
    fn fingerprint_stable_regardless_of_tag_order() {
        let fp1 = build_fingerprint(&["shock".into(), "reaction".into()]);
        let fp2 = build_fingerprint(&["reaction".into(), "shock".into()]);
        assert_eq!(fp1, fp2, "fingerprint should be order-independent");
    }

    // ── Overlap calculation ──

    #[test]
    fn overlap_calculation() {
        let a = CandidateClip::new(10.0, 30.0, ClipScoreBreakdown::new(0.5, 0.0, 0.0, None), vec![]);
        let b = CandidateClip::new(20.0, 40.0, ClipScoreBreakdown::new(0.5, 0.0, 0.0, None), vec![]);
        assert!((overlap_secs(&a, &b) - 10.0).abs() < 0.01);

        let c = CandidateClip::new(50.0, 70.0, ClipScoreBreakdown::new(0.5, 0.0, 0.0, None), vec![]);
        assert!((overlap_secs(&a, &c) - 0.0).abs() < 0.01);
    }

    // ── Full pipeline ──

    #[test]
    fn realistic_vod_scenario() {
        // Simulate a 10-minute VOD with several events
        let segments = vec![
            // Event 1: audio + transcript at ~60s
            audio_seg(58.0, 63.0, 0.85),
            transcript_seg(60.0, 62.0, 0.75, "NO WAY"),
            scene_seg(61.0, 63.0, 0.40),
            // Event 2: lone audio spike at ~180s
            audio_seg(178.0, 182.0, 0.60),
            // Event 3: all four signals at ~350s
            audio_seg(348.0, 353.0, 0.90),
            transcript_seg(350.0, 352.0, 0.80, "HOLY SHIT"),
            scene_seg(349.0, 354.0, 0.65),
            vision_seg(348.0, 360.0, 0.88, "Streamer jumps out of chair"),
            // Event 4: weak scene only at ~500s
            scene_seg(498.0, 501.0, 0.20),
        ];

        let clips = fuse(&segments, &config());

        // Event 4 should be rejected (too weak)
        // Events 1, 2, 3 should survive — maybe event 2 rejected as single-source
        assert!(clips.len() >= 2, "at least two events should produce clips");
        assert!(clips.len() <= 3);

        // Event 3 (all signals) should rank highest
        assert!(
            clips[0].signal_sources.len() >= 3,
            "best clip should have multi-signal support"
        );
        assert!(clips[0].signal_count() >= 3, "best clip should have multi-signal support");

        // All clips should have valid duration
        for clip in &clips {
            let dur = clip.duration();
            assert!(dur >= 7.5, "clip too short: {dur}s"); // min_duration * 0.5
            assert!(dur <= 60.0, "clip too long: {dur}s");
        }
    }
}
