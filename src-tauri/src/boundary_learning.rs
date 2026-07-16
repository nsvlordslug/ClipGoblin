//! Local, bounded learning for clip start and end preferences.

use std::collections::{HashMap, HashSet};

use crate::db::{ClipBehaviorEventRow, ClipEditFeedbackRow};

const MIN_SIDE_SAMPLES: usize = 2;
const ISSUE_START_ADJUSTMENT_SECONDS: f64 = -2.5;
const ISSUE_END_ADJUSTMENT_SECONDS: f64 = 3.5;
const ISSUE_WEIGHT: f64 = 0.65;
const MAX_START_ADJUSTMENT_SECONDS: f64 = 6.0;
const MAX_END_ADJUSTMENT_SECONDS: f64 = 10.0;
const MIN_CLIP_DURATION_SECONDS: f64 = 12.0;
const MAX_CLIP_DURATION_SECONDS: f64 = 60.0;

#[derive(Debug, Clone, Default)]
pub struct BoundaryPreferenceProfile {
    sample_count: usize,
    start_sample_count: usize,
    end_sample_count: usize,
    confidence: f64,
    start_adjustment_seconds: f64,
    end_adjustment_seconds: f64,
}

#[derive(Debug, Default)]
struct OriginEvidence {
    start_sum: f64,
    start_weight: f64,
    end_sum: f64,
    end_weight: f64,
}

impl OriginEvidence {
    fn add_start(&mut self, adjustment: f64, weight: f64) {
        if adjustment.is_finite() && weight.is_finite() && weight > 0.0 {
            self.start_sum += adjustment * weight;
            self.start_weight += weight;
        }
    }

    fn add_end(&mut self, adjustment: f64, weight: f64) {
        if adjustment.is_finite() && weight.is_finite() && weight > 0.0 {
            self.end_sum += adjustment * weight;
            self.end_weight += weight;
        }
    }
}

#[derive(Debug, Default)]
struct TrimTotals {
    start_delta: f64,
    end_delta: f64,
    context_weight: f64,
}

impl BoundaryPreferenceProfile {
    pub fn from_evidence(
        edit_feedback: &[ClipEditFeedbackRow],
        behavior: &[ClipBehaviorEventRow],
        channel_id: Option<&str>,
        game_name: Option<&str>,
    ) -> Self {
        let mut by_origin: HashMap<String, OriginEvidence> = HashMap::new();

        for row in edit_feedback {
            let weight = context_weight(
                row.channel_id.as_deref(),
                row.game_name.as_deref(),
                channel_id,
                game_name,
            );
            if weight <= f64::EPSILON {
                continue;
            }

            let issues = parse_issues(row.issues.as_deref());
            let origin = by_origin
                .entry(format!("highlight:{}", row.highlight_id))
                .or_default();
            if issues.contains("starts_too_late") {
                origin.add_start(ISSUE_START_ADJUSTMENT_SECONDS, ISSUE_WEIGHT * weight);
            }
            if issues.contains("cuts_off_early") {
                origin.add_end(ISSUE_END_ADJUSTMENT_SECONDS, ISSUE_WEIGHT * weight);
            }
        }

        // Multiple saves from one clip are collapsed into one cumulative trim.
        // This prevents repeated slider movements from overpowering other clips.
        let mut trim_totals: HashMap<String, TrimTotals> = HashMap::new();
        for row in behavior.iter().filter(|row| row.event_type == "trim") {
            let weight = context_weight(
                row.channel_id.as_deref(),
                row.game_name.as_deref(),
                channel_id,
                game_name,
            );
            if weight <= f64::EPSILON {
                continue;
            }

            let origin_key = row
                .highlight_id
                .as_deref()
                .map(|id| format!("highlight:{id}"))
                .unwrap_or_else(|| format!("clip:{}", row.clip_id));
            let totals = trim_totals.entry(origin_key).or_default();
            totals.context_weight = totals.context_weight.max(weight);

            if let (Some(before), Some(after)) = (row.start_before, row.start_after) {
                let delta = after - before;
                if delta.is_finite() {
                    totals.start_delta += delta;
                }
            }
            if let (Some(before), Some(after)) = (row.end_before, row.end_after) {
                let delta = after - before;
                if delta.is_finite() {
                    totals.end_delta += delta;
                }
            }
        }

        for (origin_key, totals) in trim_totals {
            let origin = by_origin.entry(origin_key).or_default();
            let start_delta = totals.start_delta.clamp(-10.0, 10.0);
            let end_delta = totals.end_delta.clamp(-10.0, 10.0);
            if start_delta.abs() >= 0.25 {
                origin.add_start(start_delta, totals.context_weight);
            }
            if end_delta.abs() >= 0.25 {
                origin.add_end(end_delta, totals.context_weight);
            }
        }

        let sample_count = by_origin
            .values()
            .filter(|origin| origin.start_weight > 0.0 || origin.end_weight > 0.0)
            .count();
        let start_samples = collect_side_samples(&by_origin, true);
        let end_samples = collect_side_samples(&by_origin, false);
        let (start_adjustment, start_confidence) = build_side_profile(&start_samples);
        let (end_adjustment, end_confidence) = build_side_profile(&end_samples);

        Self {
            sample_count,
            start_sample_count: start_samples.len(),
            end_sample_count: end_samples.len(),
            confidence: start_confidence.max(end_confidence),
            start_adjustment_seconds: start_adjustment
                .clamp(-MAX_START_ADJUSTMENT_SECONDS, MAX_START_ADJUSTMENT_SECONDS),
            end_adjustment_seconds: end_adjustment
                .clamp(-MAX_START_ADJUSTMENT_SECONDS, MAX_END_ADJUSTMENT_SECONDS),
        }
    }

    pub fn is_active(&self) -> bool {
        self.start_sample_count >= MIN_SIDE_SAMPLES || self.end_sample_count >= MIN_SIDE_SAMPLES
    }

    pub fn sample_count(&self) -> usize {
        self.sample_count
    }

    pub fn confidence(&self) -> f64 {
        self.confidence
    }

    pub fn adjust_window(&self, start: f64, end: f64, duration: f64) -> (f64, f64, bool) {
        if !self.is_active()
            || !start.is_finite()
            || !end.is_finite()
            || !duration.is_finite()
            || duration <= 0.0
            || end <= start
        {
            return (start, end, false);
        }

        let mut adjusted_start = if self.start_sample_count >= MIN_SIDE_SAMPLES {
            start + self.start_adjustment_seconds
        } else {
            start
        };
        let mut adjusted_end = if self.end_sample_count >= MIN_SIDE_SAMPLES {
            end + self.end_adjustment_seconds
        } else {
            end
        };
        adjusted_start = adjusted_start.clamp(0.0, duration);
        adjusted_end = adjusted_end.clamp(0.0, duration);

        if adjusted_end - adjusted_start > MAX_CLIP_DURATION_SECONDS {
            // Preserve the ending payoff when a learned extension reaches the cap.
            adjusted_start = (adjusted_end - MAX_CLIP_DURATION_SECONDS).max(0.0);
        }
        if adjusted_end - adjusted_start < MIN_CLIP_DURATION_SECONDS {
            return (start, end, false);
        }

        let changed = (adjusted_start - start).abs() >= 0.05
            || (adjusted_end - end).abs() >= 0.05;
        (adjusted_start, adjusted_end, changed)
    }
}

fn collect_side_samples(
    by_origin: &HashMap<String, OriginEvidence>,
    start_side: bool,
) -> Vec<(f64, f64)> {
    by_origin
        .values()
        .filter_map(|origin| {
            let (sum, weight) = if start_side {
                (origin.start_sum, origin.start_weight)
            } else {
                (origin.end_sum, origin.end_weight)
            };
            (weight > 0.0).then_some((sum / weight, weight.min(1.0)))
        })
        .collect()
}

fn build_side_profile(samples: &[(f64, f64)]) -> (f64, f64) {
    if samples.len() < MIN_SIDE_SAMPLES {
        return (0.0, 0.0);
    }
    let total_weight: f64 = samples.iter().map(|(_, weight)| weight).sum();
    if total_weight <= f64::EPSILON {
        return (0.0, 0.0);
    }
    let average = samples
        .iter()
        .map(|(adjustment, weight)| adjustment * weight)
        .sum::<f64>()
        / total_weight;
    let sample_confidence = (0.35 + samples.len() as f64 * 0.075).min(1.0);
    let evidence_quality = (total_weight / (samples.len() as f64 * ISSUE_WEIGHT))
        .clamp(0.4, 1.0);
    let confidence = sample_confidence * evidence_quality;
    (average * confidence, confidence)
}

fn context_weight(
    stored_channel: Option<&str>,
    stored_game: Option<&str>,
    target_channel: Option<&str>,
    target_game: Option<&str>,
) -> f64 {
    let channel_weight = match (normalize(stored_channel), normalize(target_channel)) {
        (_, None) => 1.0,
        (Some(stored), Some(target)) if stored == target => 1.0,
        (Some(_), Some(_)) => return 0.0,
        (None, Some(_)) => 0.4,
    };
    let game_weight = match (normalize(stored_game), normalize(target_game)) {
        (_, None) => 1.0,
        (Some(stored), Some(target)) if stored == target => 1.0,
        (Some(_), Some(_)) => 0.35,
        (None, Some(_)) => 0.65,
    };
    channel_weight * game_weight
}

fn normalize(value: Option<&str>) -> Option<String> {
    let normalized = value?.trim().to_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

fn parse_issues(value: Option<&str>) -> HashSet<String> {
    value
        .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok())
        .unwrap_or_default()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feedback(id: &str, issues: &[&str], channel: &str, game: &str) -> ClipEditFeedbackRow {
        ClipEditFeedbackRow {
            highlight_id: id.to_string(),
            vod_id: "vod-1".to_string(),
            channel_id: Some(channel.to_string()),
            game_name: Some(game.to_string()),
            start_seconds: 100.0,
            end_seconds: 130.0,
            issues: Some(serde_json::to_string(issues).unwrap()),
            note: None,
        }
    }

    fn trim(id: &str, start_delta: f64, end_delta: f64) -> ClipBehaviorEventRow {
        ClipBehaviorEventRow {
            id: format!("event-{id}"),
            clip_id: format!("clip-{id}"),
            highlight_id: Some(format!("highlight-{id}")),
            vod_id: Some("vod-1".to_string()),
            channel_id: Some("creator-1".to_string()),
            game_name: Some("Valorant".to_string()),
            event_type: "trim".to_string(),
            evidence_target: Some(0.72),
            evidence_weight: 0.35,
            start_before: Some(100.0),
            end_before: Some(130.0),
            start_after: Some(100.0 + start_delta),
            end_after: Some(130.0 + end_delta),
            scoring_dimensions: None,
            signal_sources: None,
            tags: None,
            metadata_json: None,
            occurred_at: "2026-07-14T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn one_clip_cannot_activate_boundary_learning() {
        let profile = BoundaryPreferenceProfile::from_evidence(
            &[feedback("one", &["starts_too_late", "cuts_off_early"], "creator-1", "Valorant")],
            &[],
            Some("creator-1"),
            Some("Valorant"),
        );
        assert!(!profile.is_active());
    }

    #[test]
    fn both_boundary_issues_adjust_their_own_side() {
        let rows = vec![
            feedback("one", &["starts_too_late", "cuts_off_early"], "creator-1", "Valorant"),
            feedback("two", &["starts_too_late", "cuts_off_early"], "creator-1", "Valorant"),
        ];
        let profile = BoundaryPreferenceProfile::from_evidence(
            &rows,
            &[],
            Some("creator-1"),
            Some("Valorant"),
        );
        let (start, end, changed) = profile.adjust_window(100.0, 130.0, 600.0);
        assert!(profile.is_active());
        assert!(changed);
        assert!(start < 100.0);
        assert!(end > 130.0);
    }

    #[test]
    fn real_trims_teach_direction_and_keep_duration_bounded() {
        let behavior = vec![trim("one", -8.0, 10.0), trim("two", -8.0, 10.0)];
        let profile = BoundaryPreferenceProfile::from_evidence(
            &[],
            &behavior,
            Some("creator-1"),
            Some("Valorant"),
        );
        let (start, end, changed) = profile.adjust_window(100.0, 130.0, 600.0);
        assert!(changed);
        assert!(start < 100.0);
        assert!(end > 130.0);

        let (capped_start, capped_end, _) = profile.adjust_window(100.0, 155.0, 600.0);
        assert!(capped_end - capped_start <= MAX_CLIP_DURATION_SECONDS);
    }

    #[test]
    fn other_channels_do_not_change_this_creators_boundaries() {
        let rows = vec![
            feedback("one", &["cuts_off_early"], "someone-else", "Valorant"),
            feedback("two", &["cuts_off_early"], "someone-else", "Valorant"),
        ];
        let profile = BoundaryPreferenceProfile::from_evidence(
            &rows,
            &[],
            Some("creator-1"),
            Some("Valorant"),
        );
        assert!(!profile.is_active());
    }

    #[test]
    fn non_boundary_issues_never_change_timing() {
        let rows = vec![
            feedback("one", &["wrong_moment", "duplicate"], "creator-1", "Valorant"),
            feedback("two", &["too_long"], "creator-1", "Valorant"),
        ];
        let profile = BoundaryPreferenceProfile::from_evidence(
            &rows,
            &[],
            Some("creator-1"),
            Some("Valorant"),
        );
        assert!(!profile.is_active());
    }
}
