//! Clip scoring and ranking engine.
//!
//! Takes [`CandidateClip`] entries from the fusion stage and produces
//! a final ranked list with explainable, per-factor score breakdowns.
//!
//! # Scoring formula
//!
//! Each clip's `selection_score` is composed of three layers:
//!
//! ```text
//!   raw_quality     ← weighted sum of signal scores (audio, speech, scene, vision)
//! + signal_bonus    ← reward for multi-signal corroboration
//! + duration_bonus  ← sweet-spot bonus for ideal clip length
//! - diversity_cost  ← penalty for being too similar to already-selected clips
//! ─────────────────
//! = selection_score
//! ```
//!
//! The weights for `raw_quality` change between LocalOnly and BYOK modes
//! and are fully configurable via [`WeightProfile`].
//!
//! # Ranking algorithm
//!
//! Greedy selection: pick the highest `selection_score` clip, add it
//! to the output, then recompute diversity costs for all remaining
//! candidates against the growing selected set.  Repeat until the
//! output cap is reached or no candidate exceeds the minimum score.
//!
//! This naturally produces a varied set — the second-best clip at the
//! same timestamp gets penalized, pushing a clip from a different
//! part of the VOD into the output instead.

use crate::pipeline::{
    AnalysisMode, CandidateClip, ClipScoreBreakdown, DimensionScores,
    DimensionWeights, ScoreFactor, ScoreReport,
};

// ═══════════════════════════════════════════════════════════════════
//  Weight profile
// ═══════════════════════════════════════════════════════════════════

/// Central scoring configuration.
///
/// This is the single source of truth for every tunable scoring
/// parameter.  It replaces the old `WeightProfile` with a richer
/// structure: dimension weights, bonuses, penalties, thresholds.
///
/// Implements `Serialize + Deserialize` so users can export, tweak,
/// and reimport a profile from the settings UI.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScoringConfig {
    /// Weights for the five quality dimensions (must sum to 1.0).
    pub dimension_weights: DimensionWeights,
    /// Bonus parameters.
    pub bonuses: BonusConfig,
    /// Penalty parameters.
    pub penalties: PenaltyConfig,
    /// Minimum thresholds.
    pub thresholds: ThresholdConfig,
}

/// Configurable bonus values.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BonusConfig {
    /// Bonus per additional signal source beyond 1.
    pub multi_signal: f64,
    /// Boost for strong single-source clips.
    /// Only applied when `best_raw >= solo_min_signal` AND
    /// `hook_strength >= solo_min_hook`.
    pub solo_signal: f64,
    /// Minimum raw signal score to qualify for solo boost.
    pub solo_min_signal: f64,
    /// Minimum hook_strength dimension to qualify for solo boost.
    pub solo_min_hook: f64,
    /// Bonus for clips whose hook_strength dimension exceeds this threshold.
    pub strong_open: f64,
    /// Hook dimension must exceed this to earn the strong_open bonus.
    pub strong_open_threshold: f64,
    /// Maximum bonus for clips near the ideal duration.
    pub duration_max: f64,
    /// Ideal clip duration in seconds (peak of duration bonus curve).
    pub ideal_duration: f64,
}

/// Configurable penalty values.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PenaltyConfig {
    /// Scaling factor for timeline proximity to already-selected clips.
    pub diversity_scale: f64,
    /// Seconds within which two clips are considered "nearby".
    pub diversity_window: f64,
    /// Extra penalty when two clips share the same fingerprint.
    pub same_fingerprint: f64,
    /// Minimum gap (seconds) between any two selected clips.
    /// Clips closer than this are hard-blocked from co-selection.
    pub min_clip_separation: f64,
}

/// Minimum thresholds for inclusion.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ThresholdConfig {
    /// Minimum rank_score to include a clip in the output.
    pub min_rank_score: f64,
}

impl ScoringConfig {
    /// Standard mode: local signals only.
    ///
    /// Emotional intensity and hook strength carry the most weight
    /// because they're the strongest predictors of viewer engagement
    /// from local signals alone.
    pub fn standard() -> Self {
        Self {
            dimension_weights: DimensionWeights {
                hook:    0.30,  // bumped — the opening matters most for short-form
                emotion: 0.28,
                context: 0.14,
                visual:  0.14,
                speech:  0.14,
            },
            bonuses: BonusConfig {
                multi_signal: 0.04,
                solo_signal: 0.03,
                solo_min_signal: 0.75,
                solo_min_hook: 0.50,
                strong_open: 0.03,
                strong_open_threshold: 0.55,
                duration_max: 0.04,
                ideal_duration: 25.0,
            },
            penalties: PenaltyConfig {
                diversity_scale: 0.25,
                diversity_window: 90.0,
                same_fingerprint: 0.15,
                min_clip_separation: 20.0,
            },
            thresholds: ThresholdConfig {
                min_rank_score: 0.18,
            },
        }
    }

    /// Select the appropriate config for an analysis mode.
    /// Analysis always runs locally, so this always returns `standard()`.
    pub fn for_mode(_mode: &AnalysisMode) -> Self {
        Self::standard()
    }

    /// Validate all internal consistency rules.
    pub fn validate(&self) -> Result<(), String> {
        self.dimension_weights.validate()
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Ranked clip (output)
// ═══════════════════════════════════════════════════════════════════

/// A clip with its final rank.
///
/// This is the top-level output that the frontend renders.
/// The full scoring breakdown lives on `clip.score_report`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RankedClip {
    /// 1-based rank (1 = best).
    pub rank: usize,
    /// The underlying candidate clip (with score_report populated).
    pub clip: CandidateClip,
}

// ═══════════════════════════════════════════════════════════════════
//  Public API
// ═══════════════════════════════════════════════════════════════════

/// Rank a set of candidate clips using dimension-based scoring.
///
/// `max_clips` is the final output cap — the single source of truth
/// for how many clips the pipeline produces.  It flows from
/// [`crate::engine::AnalysisRequest::top_n`].
///
/// Scoring formula per clip:
/// ```text
///   dimensions   = DimensionScores::from_signals(raw)
///   dim_weighted = weighted sum of 5 dimensions
///   bonuses      = multi_signal + solo_boost + duration
///   penalties    = diversity + fingerprint
///   rank_score   = (dim_weighted + bonuses - penalties).clamp(0, 1.0)
/// ```
///
/// Uses greedy diversity-aware selection.
pub fn rank(
    candidates: &[CandidateClip],
    config: &ScoringConfig,
    max_clips: usize,
) -> Vec<RankedClip> {
    if candidates.is_empty() {
        return Vec::new();
    }

    let min = config.thresholds.min_rank_score;
    let sep = config.penalties.min_clip_separation;

    // Pre-score every candidate (without diversity — computed per-round)
    let mut pool: Vec<(usize, f64)> = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| c.is_accepted())
        .map(|(i, c)| {
            let report = build_report(c, config, &[]);
            (i, report.rank_score)
        })
        .filter(|(_, score)| *score >= min)
        .collect();

    let mut selected: Vec<RankedClip> = Vec::new();

    while selected.len() < max_clips && !pool.is_empty() {
        // Recompute with diversity against current selected set
        let sel_clips: Vec<&CandidateClip> = selected.iter().map(|r| &r.clip).collect();
        for (idx, score) in &mut pool {
            let report = build_report(&candidates[*idx], config, &sel_clips);
            *score = report.rank_score;
        }

        pool.retain(|(_, s)| *s >= min);
        pool.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Pick the best that isn't too close to an already-selected clip
        let mut picked = None;
        for (pos, &(idx, _)) in pool.iter().enumerate() {
            let too_close = selected.iter().any(|r| {
                let gap = (candidates[idx].start_time - r.clip.start_time).abs();
                gap < sep
            });
            if !too_close {
                picked = Some(pos);
                break;
            }
        }

        let Some(pos) = picked else { break; };
        let (best_idx, _) = pool.remove(pos);

        let sel_clips: Vec<&CandidateClip> = selected.iter().map(|r| &r.clip).collect();
        let report = build_report(&candidates[best_idx], config, &sel_clips);

        let mut clip = candidates[best_idx].clone();
        clip.confidence_score = report.confidence;
        clip.score_report = Some(report);

        selected.push(RankedClip {
            rank: selected.len() + 1,
            clip,
        });
    }

    log::info!("Ranking: {} candidates → {} ranked clips", candidates.len(), selected.len());
    selected
}

/// Rescore with a new config without re-running signal extraction.
pub fn rerank(
    candidates: &[CandidateClip],
    config: &ScoringConfig,
    max_clips: usize,
) -> Vec<RankedClip> {
    rank(candidates, config, max_clips)
}

// ═══════════════════════════════════════════════════════════════════
//  Scoring internals
// ═══════════════════════════════════════════════════════════════════

/// Build a complete [`ScoreReport`] for one clip.
///
/// This is the core scoring function.  It computes dimensions from
/// raw signals, applies bonuses and penalties, and generates a
/// human-readable explanation.
fn build_report(
    clip: &CandidateClip,
    config: &ScoringConfig,
    selected: &[&CandidateClip],
) -> ScoreReport {
    let raw = &clip.score_breakdown;

    // ── Dimensions ──
    let dims = DimensionScores::from_signals(raw);
    let dim_weighted = dims.weighted(&config.dimension_weights);

    // ── Bonuses ──
    let mut bonuses: Vec<ScoreFactor> = Vec::new();
    let count = raw.active_signal_count();
    let b = &config.bonuses;

    // Multi-signal corroboration
    if count >= 2 {
        let val = (count as f64 - 1.0) * b.multi_signal;
        bonuses.push(ScoreFactor { label: "Multi-signal".into(), value: val });
    }
    // Conditional solo boost — only if the signal is genuinely strong
    // AND the clip has a decent hook
    else if count == 1
        && raw.best_raw() >= b.solo_min_signal
        && dims.hook_strength >= b.solo_min_hook
    {
        bonuses.push(ScoreFactor { label: "Solo detection".into(), value: b.solo_signal });
    }

    // Strong open bonus — clips that hook attention immediately
    if dims.hook_strength >= b.strong_open_threshold {
        bonuses.push(ScoreFactor { label: "Strong opening".into(), value: b.strong_open });
    }

    // Duration bonus
    let dur = clip.duration();
    let dur_dist = (dur - b.ideal_duration).abs();
    if dur_dist < b.ideal_duration {
        let val = b.duration_max * (1.0 - dur_dist / b.ideal_duration);
        if val > 0.001 {
            bonuses.push(ScoreFactor { label: "Good length".into(), value: val });
        }
    }

    let bonus_total: f64 = bonuses.iter().map(|f| f.value).sum();

    // ── Penalties ──
    let mut penalties: Vec<ScoreFactor> = Vec::new();

    if !selected.is_empty() {
        let div = compute_diversity_penalty(clip, selected, config);
        if div > 0.001 {
            penalties.push(ScoreFactor { label: "Diversity".into(), value: div });
        }
    }

    let penalty_total: f64 = penalties.iter().map(|f| f.value).sum();

    // ── Scores ──
    //
    // rank_score: internal, drives selection (0.0–1.0)
    // confidence: user-facing, piecewise-rescaled so that:
    //   - decent clips land in a believable middle range (40–70%)
    //   - strong clips reach 70–88%
    //   - only exceptional multi-signal clips break 90%
    //   - 95+ is rare, 99 is near-impossible
    //
    // Bonuses/penalties only affect rank_score, not confidence —
    // the user sees "how good is this clip" not "how did it rank".
    // Multi-signal agreement nudges confidence slightly (+1–3%)
    // because more signals = higher actual confidence in the result.
    let rank_score = (dim_weighted + bonus_total - penalty_total).clamp(0.0, 1.0);
    let confidence = rescale_confidence(dim_weighted, count);

    // ── Explanation ──
    let top_dims = dims.as_labeled_pairs();
    let key_dimensions: Vec<String> = top_dims
        .iter()
        .filter(|(_, v)| *v > 0.05)
        .map(|(label, v)| format!("{label} ({v:.2})"))
        .collect();

    let explanation = build_explanation(raw, &dims, count, penalty_total > 0.05);

    ScoreReport {
        raw_signals: raw.clone(),
        dimensions: dims,
        dimension_weighted: dim_weighted,
        bonuses,
        bonus_total,
        penalties,
        penalty_total,
        rank_score,
        confidence,
        explanation,
        key_dimensions,
    }
}

/// Map internal `dim_weighted` to a user-facing confidence score.
///
/// Piecewise-linear curve calibrated so that:
///   - most detected clips land 55–80%
///   - strong clips: 80–90%
///   - exceptional clips: 90–95%
///   - 95+ is rare, 99 is unreachable
///
/// Real dim_weighted values for reference:
///   decent 3-signal (0.7/0.5/0.3) → dw≈0.42 → 57%
///   good 3-signal   (0.8/0.7/0.5) → dw≈0.55 → 68%
///   strong 3-signal  (0.9/0.8/0.6) → dw≈0.63 → 79%
fn rescale_confidence(dim_weighted: f64, signal_count: usize) -> f64 {
    // Piecewise-linear anchors: tighter at the top
    const ANCHORS: [(f64, f64); 8] = [
        (0.00, 0.00),
        (0.25, 0.25),
        (0.40, 0.55),   // bottom of "most clips"
        (0.50, 0.65),   // middle
        (0.60, 0.77),   // top of "most clips"
        (0.70, 0.84),   // strong
        (0.80, 0.89),   // very strong
        (0.90, 0.93),   // exceptional
    ];

    let base = if dim_weighted >= 0.90 {
        // Hard compression: 0.90–1.0 → 0.93–0.95
        (0.93 + (dim_weighted - 0.90) * 0.20).min(0.95)
    } else {
        let mut out = 0.0;
        for i in 1..ANCHORS.len() {
            if dim_weighted <= ANCHORS[i].0 {
                let (x0, y0) = ANCHORS[i - 1];
                let (x1, y1) = ANCHORS[i];
                let t = (dim_weighted - x0) / (x1 - x0);
                out = y0 + t * (y1 - y0);
                break;
            }
        }
        out
    };

    // Signal nudge: only 4+ signals get a small bump
    let nudge = if signal_count >= 4 { 0.01 } else { 0.0 };

    (base + nudge).clamp(0.0, 0.96)
}

/// Compute diversity penalty against already-selected clips.
fn compute_diversity_penalty(
    candidate: &CandidateClip,
    selected: &[&CandidateClip],
    config: &ScoringConfig,
) -> f64 {
    let mut max_penalty = 0.0_f64;
    let p = &config.penalties;

    for kept in selected {
        let gap = (candidate.start_time - kept.start_time)
            .abs()
            .min((candidate.end_time - kept.end_time).abs());

        if gap >= p.diversity_window {
            continue;
        }

        let proximity = 1.0 - (gap / p.diversity_window);
        let mut penalty = proximity * p.diversity_scale;

        if !candidate.fingerprint.is_empty() && candidate.fingerprint == kept.fingerprint {
            penalty += p.same_fingerprint;
        }

        max_penalty = max_penalty.max(penalty);
    }

    max_penalty
}

/// Build a factual explanation: which signals fired at what strength,
/// how many are active, and which dimension is the top contributor.
///
/// Examples:
///   "3 signals — strongest: audio (80%). Top dimension: hook strength (56%)"
///   "1 signal — audio (72%). Top dimension: hook strength (40%)"
fn build_explanation(
    raw: &ClipScoreBreakdown,
    dims: &DimensionScores,
    signal_count: usize,
    has_penalty: bool,
) -> String {
    // Strongest raw signal
    let strongest = strongest_signal_label(raw);

    // Top dimension
    let top_dim = dims.as_labeled_pairs();
    let (dim_name, dim_val) = top_dim
        .first()
        .map(|(n, v)| (*n, *v))
        .unwrap_or(("n/a", 0.0));

    let base = format!(
        "{} signal{} — strongest: {}. Top dimension: {} ({:.0}%)",
        signal_count,
        if signal_count != 1 { "s" } else { "" },
        strongest,
        dim_name.to_lowercase(),
        dim_val * 100.0,
    );

    if has_penalty {
        format!("{base} (nearby clip reduces rank)")
    } else {
        base
    }
}

/// "audio (80%)" — label + percentage for the strongest raw signal.
fn strongest_signal_label(raw: &ClipScoreBreakdown) -> String {
    let mut signals: Vec<(f64, &str)> = vec![
        (raw.audio_score, "audio"),
        (raw.speech_score, "speech"),
        (raw.scene_score, "scene"),
    ];
    if let Some(v) = raw.vision_score {
        signals.push((v, "vision"));
    }
    signals.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let (val, name) = signals.first().copied().unwrap_or((0.0, "none"));
    format!("{} ({:.0}%)", name, val * 100.0)
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::SignalType;

    fn make_clip(
        start: f64,
        end: f64,
        audio: f64,
        speech: f64,
        scene: f64,
        vision: Option<f64>,
    ) -> CandidateClip {
        let mut c = CandidateClip::new(
            start,
            end,
            ClipScoreBreakdown::new(audio, speech, scene, vision),
            {
                let mut s = vec![SignalType::Audio, SignalType::Transcript, SignalType::SceneChange];
                if vision.is_some() {
                    s.push(SignalType::Vision);
                }
                s.retain(|t| match t {
                    SignalType::Audio => audio > 0.0,
                    SignalType::Transcript => speech > 0.0,
                    SignalType::SceneChange => scene > 0.0,
                    SignalType::Vision => vision.unwrap_or(0.0) > 0.0,
                });
                s
            },
        );
        c.fingerprint = "test+clip".into();
        c.tags = vec!["test".into()];
        c
    }

    fn cfg() -> ScoringConfig { ScoringConfig::standard() }

    // ── Config validation ──

    #[test]
    fn standard_weights_sum_to_one() {
        assert!(ScoringConfig::standard().validate().is_ok());
    }

    #[test]
    fn vision_weights_sum_to_one() {
        assert!(ScoringConfig::standard().validate().is_ok());
    }

    #[test]
    fn invalid_weights_rejected() {
        let mut c = cfg();
        c.dimension_weights.hook = 0.9;
        assert!(c.validate().is_err());
    }

    #[test]
    fn for_mode_returns_standard_config() {
        let config = ScoringConfig::for_mode(&AnalysisMode::local());
        let standard = ScoringConfig::standard();
        assert!((config.dimension_weights.hook - standard.dimension_weights.hook).abs() < f64::EPSILON);
    }

    // ── Dimension scoring ──

    #[test]
    fn higher_signals_produce_higher_rank() {
        let strong = make_clip(0.0, 25.0, 0.9, 0.8, 0.6, None);
        let weak = make_clip(200.0, 225.0, 0.3, 0.2, 0.1, None);
        let ranked = rank(&[strong, weak], &cfg(), 10);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].rank, 1);
        assert!(ranked[0].clip.confidence_score > ranked[1].clip.confidence_score);
    }

    #[test]
    fn multi_signal_bonus_exceeds_solo_boost() {
        let multi = make_clip(0.0, 25.0, 0.7, 0.6, 0.5, None);
        let single = make_clip(200.0, 225.0, 0.7, 0.0, 0.0, None);
        let r_multi = build_report(&multi, &cfg(), &[]);
        let r_single = build_report(&single, &cfg(), &[]);
        assert!(r_multi.bonus_total > r_single.bonus_total);
    }

    #[test]
    fn solo_boost_requires_strong_signal_and_hook() {
        // 0.90 audio → qualifies (>= 0.75 signal, hook = 0.90*0.55 = 0.495...
        // hmm, hook_strength from audio 0.90 alone = 0.90*0.55 = 0.495 which is
        // below solo_min_hook 0.50. So we need a slightly higher audio or add scene.
        let qualifies = make_clip(0.0, 25.0, 0.95, 0.0, 0.0, None);
        // hook = 0.95*0.55 = 0.5225 >= 0.50 ✓, best_raw = 0.95 >= 0.75 ✓
        let too_weak = make_clip(200.0, 225.0, 0.60, 0.0, 0.0, None);
        // best_raw = 0.60 < 0.75 ✗
        let r_qual = build_report(&qualifies, &cfg(), &[]);
        let r_weak = build_report(&too_weak, &cfg(), &[]);
        assert!(r_qual.bonus_total > r_weak.bonus_total,
            "strong solo ({:.3}) should get boost, weak ({:.3}) should not",
            r_qual.bonus_total, r_weak.bonus_total);
    }

    #[test]
    fn duration_bonus_peaks_at_ideal() {
        let c = cfg();
        let ideal = make_clip(0.0, 25.0, 0.7, 0.5, 0.3, None);
        let short = make_clip(200.0, 212.0, 0.7, 0.5, 0.3, None);
        let r_ideal = build_report(&ideal, &c, &[]);
        let r_short = build_report(&short, &c, &[]);
        assert!(r_ideal.bonus_total > r_short.bonus_total,
            "ideal duration should get larger bonus");
    }

    // ── Dimensions computed correctly ──

    #[test]
    fn dimensions_derived_from_signals() {
        let clip = make_clip(0.0, 25.0, 0.8, 0.6, 0.4, None);
        let report = build_report(&clip, &cfg(), &[]);
        let d = &report.dimensions;
        // hook_strength = 0.8*0.55 + 0.4*0.30 + 0*0.15 = 0.56
        assert!((d.hook_strength - 0.56).abs() < 0.01,
            "hook_strength: expected ~0.56, got {:.3}", d.hook_strength);
        // speech_punch = 0.6*0.65 + 0.8*0.25 + 0.4*0.10 = 0.63
        assert!((d.speech_punch - 0.63).abs() < 0.01,
            "speech_punch: expected ~0.63, got {:.3}", d.speech_punch);
    }

    // ── Diversity ──

    #[test]
    fn nearby_clip_gets_diversity_penalty() {
        let c = cfg();
        let selected = make_clip(100.0, 125.0, 0.9, 0.8, 0.6, None);
        let nearby = make_clip(130.0, 155.0, 0.8, 0.7, 0.5, None);
        let far = make_clip(500.0, 525.0, 0.8, 0.7, 0.5, None);
        let p_near = compute_diversity_penalty(&nearby, &[&selected], &c);
        let p_far = compute_diversity_penalty(&far, &[&selected], &c);
        assert!(p_near > 0.0);
        assert!((p_far - 0.0).abs() < 1e-9);
    }

    #[test]
    fn same_fingerprint_extra_penalty() {
        let c = cfg();
        let selected = make_clip(100.0, 125.0, 0.9, 0.8, 0.6, None);
        let mut same_fp = make_clip(130.0, 155.0, 0.8, 0.7, 0.5, None);
        same_fp.fingerprint = selected.fingerprint.clone();
        let mut diff_fp = make_clip(130.0, 155.0, 0.8, 0.7, 0.5, None);
        diff_fp.fingerprint = "different+type".into();
        let p_same = compute_diversity_penalty(&same_fp, &[&selected], &c);
        let p_diff = compute_diversity_penalty(&diff_fp, &[&selected], &c);
        assert!(p_same > p_diff);
    }

    #[test]
    fn greedy_selection_produces_variety() {
        let clips = vec![
            make_clip(100.0, 125.0, 0.95, 0.8, 0.6, None),
            make_clip(120.0, 145.0, 0.90, 0.7, 0.5, None),
            make_clip(500.0, 525.0, 0.60, 0.5, 0.3, None),
        ];
        let ranked = rank(&clips, &cfg(), 10);
        assert!(ranked.len() >= 2);
        let starts: Vec<f64> = ranked.iter().map(|r| r.clip.start_time).collect();
        assert!(starts.contains(&100.0) && starts.contains(&500.0),
            "should select diverse clips: {:?}", starts);
    }

    // ── Score report ──

    #[test]
    fn score_report_has_dimensions_and_explanation() {
        let clip = make_clip(0.0, 25.0, 0.8, 0.6, 0.4, None);
        let report = build_report(&clip, &cfg(), &[]);
        assert!(report.dimension_weighted > 0.0);
        assert!(!report.explanation.is_empty());
        assert!(!report.key_dimensions.is_empty());
        assert!(report.rank_score > 0.0 && report.rank_score <= 1.0);
    }

    #[test]
    fn score_report_populated_on_ranked_clip() {
        let clips = vec![make_clip(0.0, 25.0, 0.8, 0.6, 0.4, None)];
        let ranked = rank(&clips, &cfg(), 5);
        assert!(!ranked.is_empty());
        let report = ranked[0].clip.score_report.as_ref().expect("report should be set");
        assert!(report.rank_score > 0.0);
        assert!(!report.key_dimensions.is_empty());
    }

    #[test]
    fn score_report_final_never_negative() {
        let selected = make_clip(100.0, 125.0, 0.9, 0.8, 0.6, None);
        let mut twin = make_clip(100.0, 125.0, 0.3, 0.2, 0.1, None);
        twin.fingerprint = selected.fingerprint.clone();
        let report = build_report(&twin, &cfg(), &[&selected]);
        assert!(report.rank_score >= 0.0);
    }

    // ── Edge cases ──

    #[test]
    fn empty_input_returns_empty() {
        assert!(rank(&[], &cfg(), 10).is_empty());
    }

    #[test]
    fn rejected_clips_excluded() {
        let mut clip = make_clip(0.0, 25.0, 0.8, 0.6, 0.4, None);
        clip.rejection_reason = Some("too boring".into());
        assert!(rank(&[clip], &cfg(), 10).is_empty());
    }

    #[test]
    fn capped_at_max_clips() {
        let clips: Vec<CandidateClip> = (0..20)
            .map(|i| make_clip(i as f64 * 200.0, i as f64 * 200.0 + 25.0, 0.8, 0.6, 0.4, None))
            .collect();
        let ranked = rank(&clips, &cfg(), 5);
        assert!(ranked.len() <= 5);
    }

    #[test]
    fn ranks_are_sequential() {
        let clips = vec![
            make_clip(0.0, 25.0, 0.9, 0.7, 0.5, None),
            make_clip(200.0, 225.0, 0.7, 0.5, 0.3, None),
            make_clip(400.0, 425.0, 0.6, 0.4, 0.2, None),
        ];
        let ranked = rank(&clips, &cfg(), 10);
        for (i, r) in ranked.iter().enumerate() {
            assert_eq!(r.rank, i + 1);
        }
    }

    // ── Confidence recalibration ──

    #[test]
    fn confidence_decent_clip_in_middle_range() {
        // A decent 3-signal clip should land in the "worth reviewing" range
        let decent = make_clip(0.0, 25.0, 0.7, 0.5, 0.3, None);
        let report = build_report(&decent, &cfg(), &[]);
        assert!(report.confidence >= 0.40 && report.confidence <= 0.75,
            "decent clip confidence should be in middle range, got {:.3}", report.confidence);
    }

    #[test]
    fn confidence_strong_clip_below_90() {
        // A strong 3-signal clip should stay below 90%
        let strong = make_clip(0.0, 25.0, 0.8, 0.7, 0.5, None);
        let report = build_report(&strong, &cfg(), &[]);
        assert!(report.confidence < 0.90,
            "strong clip should be below 90%%, got {:.3}", report.confidence);
    }

    #[test]
    fn confidence_exceptional_can_exceed_90() {
        // Exceptional 4-signal clip can reach 90%+
        let exceptional = make_clip(0.0, 25.0, 0.95, 0.9, 0.85, Some(0.9));
        let report = build_report(&exceptional, &ScoringConfig::standard(), &[]);
        assert!(report.confidence >= 0.90,
            "exceptional multi-signal clip should exceed 90%%, got {:.3}", report.confidence);
    }

    #[test]
    fn confidence_capped_at_99() {
        // All signals at 1.0 (unrealistic in practice) caps at 0.99.
        // This is the theoretical maximum — real clips won't reach it.
        let max = make_clip(0.0, 25.0, 1.0, 1.0, 1.0, Some(1.0));
        let report = build_report(&max, &ScoringConfig::standard(), &[]);
        assert!(report.confidence <= 0.99,
            "confidence should cap at 99%%, got {:.3}", report.confidence);
        // A strong-but-not-max clip should stay well below 95%
        let strong = make_clip(200.0, 225.0, 0.85, 0.75, 0.6, None);
        let r_strong = build_report(&strong, &cfg(), &[]);
        assert!(r_strong.confidence < 0.90,
            "strong 3-signal clip should be below 90%%, got {:.3}", r_strong.confidence);
    }

    #[test]
    fn confidence_monotonic_with_quality() {
        // Higher dim_weighted should always produce higher confidence
        let weak   = make_clip(0.0, 25.0, 0.3, 0.2, 0.1, None);
        let medium = make_clip(200.0, 225.0, 0.6, 0.4, 0.3, None);
        let strong = make_clip(400.0, 425.0, 0.9, 0.8, 0.6, None);
        let r_w = build_report(&weak, &cfg(), &[]);
        let r_m = build_report(&medium, &cfg(), &[]);
        let r_s = build_report(&strong, &cfg(), &[]);
        assert!(r_s.confidence > r_m.confidence && r_m.confidence > r_w.confidence,
            "confidence should increase with quality: {:.3} > {:.3} > {:.3}",
            r_s.confidence, r_m.confidence, r_w.confidence);
    }

    // ── Existing tests ──

    #[test]
    fn rerank_with_different_config() {
        let clips = vec![
            make_clip(0.0, 25.0, 0.9, 0.2, 0.1, None),
            make_clip(200.0, 225.0, 0.2, 0.9, 0.1, None),
        ];
        let ranked = rank(&clips, &cfg(), 10);
        assert!(!ranked.is_empty());

        let mut custom = cfg();
        custom.dimension_weights = DimensionWeights {
            hook: 0.05, emotion: 0.05, context: 0.05, visual: 0.05, speech: 0.80,
        };
        let re = rerank(&clips, &custom, 10);
        // With speech weight at 0.80, the speech-heavy clip should rank first
        assert!((re[0].clip.score_breakdown.speech_score - 0.9).abs() < 0.01);
    }
}
