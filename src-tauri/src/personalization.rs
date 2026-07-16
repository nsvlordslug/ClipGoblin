//! Local, bounded clip-ranking personalization.
//!
//! Explicit Good / Meh / Boring reviews are converted into a small score
//! adjustment. The base detector and its structural quality gates remain in
//! charge: personalization can only move a candidate by eight percentage
//! points at full confidence, and confidence ramps up with varied feedback.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::db::{ClipBehaviorEventRow, ClipEditFeedbackRow, DetectionFeedbackRow};

const MIN_USABLE_SAMPLES: usize = 4;
const FULL_CONFIDENCE_SAMPLES: f64 = 20.0;
const MAX_SCORE_ADJUSTMENT: f64 = 0.08;
const BASE_DIMENSION_WEIGHTS: [f64; 6] = [0.30, 0.20, 0.20, 0.15, 0.10, 0.05];

#[derive(Debug, Clone, Default)]
pub struct PersonalizationProfile {
    sample_count: usize,
    confidence: f64,
    dimension_affinity: [f64; 6],
    tag_affinity: HashMap<String, f64>,
    source_affinity: HashMap<String, f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PersonalizationStatus {
    pub state: String,
    pub total_ratings: usize,
    pub usable_ratings: usize,
    pub rating_classes: usize,
    pub confidence: f64,
    pub is_personalizing: bool,
    pub target_ratings: usize,
    pub behavior_events: usize,
    pub usable_behavior_events: usize,
    pub total_evidence: usize,
    pub boundary_feedback_samples: usize,
    pub boundary_learning_active: bool,
    pub boundary_confidence: f64,
}

#[derive(Debug, Clone)]
struct WeightedSample {
    dimensions: [f64; 6],
    target: f64,
    weight: f64,
    tags: Vec<String>,
    sources: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct StoredDimensions {
    hook: f64,
    emotion: f64,
    payoff: f64,
    align: f64,
    context: f64,
    replay: f64,
}

impl PersonalizationProfile {
    pub fn from_feedback(
        feedback: &[DetectionFeedbackRow],
        channel_id: Option<&str>,
        game_name: Option<&str>,
    ) -> Self {
        Self::from_evidence(feedback, &[], channel_id, game_name)
    }

    pub fn from_evidence(
        feedback: &[DetectionFeedbackRow],
        behavior: &[ClipBehaviorEventRow],
        channel_id: Option<&str>,
        game_name: Option<&str>,
    ) -> Self {
        let mut samples = collect_weighted_samples(feedback, channel_id, game_name);
        samples.extend(collect_behavior_samples(behavior, channel_id, game_name));

        if samples.len() < MIN_USABLE_SAMPLES {
            return Self::default();
        }

        if rating_class_count(&samples) < 2 {
            return Self::default();
        }

        let total_weight: f64 = samples.iter().map(|sample| sample.weight).sum();
        if total_weight <= f64::EPSILON {
            return Self::default();
        }
        let target_mean = samples
            .iter()
            .map(|sample| sample.target * sample.weight)
            .sum::<f64>()
            / total_weight;
        let target_variance = samples
            .iter()
            .map(|sample| sample.weight * (sample.target - target_mean).powi(2))
            .sum::<f64>()
            / total_weight;
        if target_variance <= 1e-6 {
            return Self::default();
        }

        let mut dimension_affinity = [0.0; 6];
        for index in 0..dimension_affinity.len() {
            dimension_affinity[index] = weighted_correlation(
                &samples,
                |sample| sample.dimensions[index],
                target_mean,
                target_variance,
            );
        }

        let sample_confidence = (samples.len() as f64 / FULL_CONFIDENCE_SAMPLES).clamp(0.0, 1.0);
        let diversity_confidence = (target_variance / 0.12).sqrt().clamp(0.0, 1.0);
        let confidence = sample_confidence * diversity_confidence;

        Self {
            sample_count: samples.len(),
            confidence,
            dimension_affinity,
            tag_affinity: category_affinities(&samples, target_mean, |sample| &sample.tags),
            source_affinity: category_affinities(&samples, target_mean, |sample| &sample.sources),
        }
    }

    pub fn is_active(&self) -> bool {
        self.sample_count >= MIN_USABLE_SAMPLES && self.confidence > 0.01
    }

    pub fn sample_count(&self) -> usize {
        self.sample_count
    }

    pub fn confidence(&self) -> f64 {
        self.confidence
    }

    pub fn score_adjustment(
        &self,
        dimensions: [f64; 6],
        tags: &[String],
        sources: &[&str],
    ) -> f64 {
        if !self.is_active() {
            return 0.0;
        }

        let dimension_signal = dimensions
            .iter()
            .zip(self.dimension_affinity.iter())
            .zip(BASE_DIMENSION_WEIGHTS.iter())
            .map(|((&value, &affinity), &weight)| {
                weight * affinity * ((value.clamp(0.0, 1.0) - 0.5) * 2.0)
            })
            .sum::<f64>();
        let tag_signal = mean_affinity(tags.iter().map(String::as_str), &self.tag_affinity);
        let source_signal = mean_affinity(sources.iter().copied(), &self.source_affinity);

        let raw = (dimension_signal * 0.055) + (tag_signal * 0.015) + (source_signal * 0.010);
        let limit = MAX_SCORE_ADJUSTMENT * self.confidence;
        (raw * self.confidence).clamp(-limit, limit)
    }
}

impl PersonalizationStatus {
    pub fn from_feedback(feedback: &[DetectionFeedbackRow]) -> Self {
        Self::from_all_evidence(feedback, &[], &[])
    }

    pub fn from_evidence(
        feedback: &[DetectionFeedbackRow],
        behavior: &[ClipBehaviorEventRow],
    ) -> Self {
        Self::from_all_evidence(feedback, behavior, &[])
    }

    pub fn from_all_evidence(
        feedback: &[DetectionFeedbackRow],
        behavior: &[ClipBehaviorEventRow],
        edit_feedback: &[ClipEditFeedbackRow],
    ) -> Self {
        let rating_samples = collect_weighted_samples(feedback, None, None);
        let behavior_samples = collect_behavior_samples(behavior, None, None);
        let mut samples = rating_samples.clone();
        samples.extend(behavior_samples.iter().cloned());
        let rating_classes = rating_class_count(&samples);
        let profile = PersonalizationProfile::from_evidence(feedback, behavior, None, None);
        let boundary_profile = crate::boundary_learning::BoundaryPreferenceProfile::from_evidence(
            edit_feedback,
            behavior,
            None,
            None,
        );
        let usable_ratings = rating_samples.len();
        let total_evidence = samples.len();
        let state = if total_evidence == 0 {
            "empty"
        } else if total_evidence < MIN_USABLE_SAMPLES {
            "needs_more"
        } else if rating_classes < 2 {
            "needs_variety"
        } else if usable_ratings < FULL_CONFIDENCE_SAMPLES as usize {
            "learning"
        } else {
            "active"
        };

        Self {
            state: state.to_string(),
            total_ratings: feedback.len(),
            usable_ratings,
            rating_classes,
            confidence: profile.confidence(),
            is_personalizing: profile.is_active(),
            target_ratings: FULL_CONFIDENCE_SAMPLES as usize,
            behavior_events: behavior.len(),
            usable_behavior_events: behavior_samples.len(),
            total_evidence,
            boundary_feedback_samples: boundary_profile.sample_count(),
            boundary_learning_active: boundary_profile.is_active(),
            boundary_confidence: boundary_profile.confidence(),
        }
    }
}

fn collect_weighted_samples(
    feedback: &[DetectionFeedbackRow],
    channel_id: Option<&str>,
    game_name: Option<&str>,
) -> Vec<WeightedSample> {
    let target_game = normalize_label(game_name.unwrap_or_default());
    feedback
        .iter()
        .filter_map(|row| {
            let target = rating_target(&row.rating)?;
            let stored: StoredDimensions =
                serde_json::from_str(row.scoring_dimensions.as_deref()?).ok()?;
            let dimensions = [
                stored.hook,
                stored.emotion,
                stored.payoff,
                stored.align,
                stored.context,
                stored.replay,
            ];
            if dimensions.iter().any(|value| !value.is_finite()) {
                return None;
            }

            let same_channel = channel_id
                .zip(row.channel_id.as_deref())
                .is_some_and(|(target, stored)| target == stored);
            let same_game = !target_game.is_empty()
                && row
                    .game_name
                    .as_deref()
                    .map(normalize_label)
                    .is_some_and(|stored| stored == target_game);
            let weight = match (same_channel, same_game) {
                (true, true) => 2.5,
                (true, false) => 1.75,
                (false, true) => 1.4,
                (false, false) => 1.0,
            };

            Some(WeightedSample {
                dimensions: dimensions.map(|value| value.clamp(0.0, 1.0)),
                target,
                weight,
                tags: parse_tags(row.tags.as_deref()),
                sources: parse_sources(row.signal_sources.as_deref()),
            })
        })
        .collect()
}

fn collect_behavior_samples(
    behavior: &[ClipBehaviorEventRow],
    channel_id: Option<&str>,
    game_name: Option<&str>,
) -> Vec<WeightedSample> {
    let target_game = normalize_label(game_name.unwrap_or_default());
    behavior
        .iter()
        .filter_map(|row| {
            let target = row.evidence_target?.clamp(0.0, 1.0);
            if row.evidence_weight <= 0.0 {
                return None;
            }
            let stored: StoredDimensions =
                serde_json::from_str(row.scoring_dimensions.as_deref()?).ok()?;
            let dimensions = [
                stored.hook,
                stored.emotion,
                stored.payoff,
                stored.align,
                stored.context,
                stored.replay,
            ];
            if dimensions.iter().any(|value| !value.is_finite()) {
                return None;
            }

            let same_channel = channel_id
                .zip(row.channel_id.as_deref())
                .is_some_and(|(target, stored)| target == stored);
            let same_game = !target_game.is_empty()
                && row
                    .game_name
                    .as_deref()
                    .map(normalize_label)
                    .is_some_and(|stored| stored == target_game);
            let context_multiplier = match (same_channel, same_game) {
                (true, true) => 1.5,
                (true, false) => 1.3,
                (false, true) => 1.15,
                (false, false) => 1.0,
            };
            Some(WeightedSample {
                dimensions: dimensions.map(|value| value.clamp(0.0, 1.0)),
                target,
                weight: (row.evidence_weight * context_multiplier).clamp(0.0, 1.0),
                tags: parse_tags(row.tags.as_deref()),
                sources: parse_sources(row.signal_sources.as_deref()),
            })
        })
        .collect()
}

fn rating_class_count(samples: &[WeightedSample]) -> usize {
    samples
        .iter()
        .map(|sample| (sample.target * 100.0).round() as i32)
        .collect::<HashSet<_>>()
        .len()
}

fn rating_target(rating: &str) -> Option<f64> {
    match rating {
        "good" => Some(1.0),
        "meh" => Some(0.45),
        "boring" => Some(0.0),
        _ => None,
    }
}

fn normalize_label(value: &str) -> String {
    value.trim().to_lowercase()
}

fn parse_tags(value: Option<&str>) -> Vec<String> {
    let Some(value) = value else { return Vec::new() };
    serde_json::from_str::<Vec<String>>(value)
        .unwrap_or_else(|_| value.split(',').map(str::to_string).collect())
        .into_iter()
        .map(|tag| normalize_label(&tag))
        .filter(|tag| !tag.is_empty())
        .collect()
}

fn parse_sources(value: Option<&str>) -> Vec<String> {
    let Some(value) = value else { return Vec::new() };
    serde_json::from_str::<Vec<String>>(value)
        .unwrap_or_else(|_| value.split(',').map(str::to_string).collect())
        .into_iter()
        .map(|source| normalize_label(&source))
        .filter(|source| !source.is_empty())
        .collect()
}

fn weighted_correlation(
    samples: &[WeightedSample],
    feature: impl Fn(&WeightedSample) -> f64,
    target_mean: f64,
    target_variance: f64,
) -> f64 {
    let total_weight: f64 = samples.iter().map(|sample| sample.weight).sum();
    let feature_mean = samples
        .iter()
        .map(|sample| feature(sample) * sample.weight)
        .sum::<f64>()
        / total_weight;
    let covariance = samples
        .iter()
        .map(|sample| {
            sample.weight
                * (feature(sample) - feature_mean)
                * (sample.target - target_mean)
        })
        .sum::<f64>()
        / total_weight;
    let feature_variance = samples
        .iter()
        .map(|sample| sample.weight * (feature(sample) - feature_mean).powi(2))
        .sum::<f64>()
        / total_weight;
    if feature_variance <= 1e-6 {
        return 0.0;
    }
    (covariance / (feature_variance * target_variance).sqrt()).clamp(-1.0, 1.0)
}

fn category_affinities<'a>(
    samples: &'a [WeightedSample],
    target_mean: f64,
    categories: impl Fn(&'a WeightedSample) -> &'a [String],
) -> HashMap<String, f64> {
    let mut totals: HashMap<String, (f64, f64)> = HashMap::new();
    for sample in samples {
        let unique: HashSet<&str> = categories(sample).iter().map(String::as_str).collect();
        for category in unique {
            let entry = totals.entry(category.to_string()).or_default();
            entry.0 += sample.target * sample.weight;
            entry.1 += sample.weight;
        }
    }

    totals
        .into_iter()
        .filter_map(|(category, (target_total, weight))| {
            if weight < 1.5 {
                return None;
            }
            let observed = target_total / weight;
            let shrinkage = weight / (weight + 4.0);
            let affinity = ((observed - target_mean) * 2.0).clamp(-1.0, 1.0) * shrinkage;
            Some((category, affinity))
        })
        .collect()
}

fn mean_affinity<'a>(
    values: impl Iterator<Item = &'a str>,
    affinities: &HashMap<String, f64>,
) -> f64 {
    let matches: Vec<f64> = values
        .filter_map(|value| affinities.get(&normalize_label(value)).copied())
        .collect();
    if matches.is_empty() {
        0.0
    } else {
        matches.iter().sum::<f64>() / matches.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(_id: usize, rating: &str, hook: f64, tag: &str, game: &str) -> DetectionFeedbackRow {
        DetectionFeedbackRow {
            channel_id: Some("creator-1".to_string()),
            game_name: Some(game.to_string()),
            rating: rating.to_string(),
            scoring_dimensions: Some(
                serde_json::json!({
                    "hook": hook,
                    "emotion": 0.5,
                    "payoff": 0.5,
                    "align": 0.5,
                    "context": 0.5,
                    "replay": 0.5,
                })
                .to_string(),
            ),
            signal_sources: Some(r#"["audio","transcript"]"#.to_string()),
            tags: Some(tag.to_string()),
        }
    }

    fn behavior_row(id: usize, target: f64, hook: f64) -> ClipBehaviorEventRow {
        ClipBehaviorEventRow {
            id: format!("behavior-{id}"),
            clip_id: format!("clip-{id}"),
            highlight_id: Some(format!("highlight-{id}")),
            vod_id: Some("vod-1".to_string()),
            channel_id: Some("creator-1".to_string()),
            game_name: Some("Valorant".to_string()),
            event_type: "export".to_string(),
            evidence_target: Some(target),
            evidence_weight: 0.45,
            start_before: None,
            end_before: None,
            start_after: None,
            end_after: None,
            scoring_dimensions: Some(
                serde_json::json!({
                    "hook": hook,
                    "emotion": 0.5,
                    "payoff": 0.5,
                    "align": 0.5,
                    "context": 0.5,
                    "replay": 0.5,
                })
                .to_string(),
            ),
            signal_sources: Some(r#"["audio","transcript"]"#.to_string()),
            tags: Some(r#"["clutch"]"#.to_string()),
            metadata_json: None,
            occurred_at: "2026-07-14T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn profile_waits_for_enough_varied_feedback() {
        let too_few = vec![
            row(1, "good", 0.9, "clutch", "Valorant"),
            row(2, "boring", 0.2, "idle", "Valorant"),
            row(3, "good", 0.8, "clutch", "Valorant"),
        ];
        assert!(!PersonalizationProfile::from_feedback(
            &too_few,
            Some("creator-1"),
            Some("Valorant")
        )
        .is_active());

        let one_class = (0..8)
            .map(|id| row(id, "good", 0.8, "clutch", "Valorant"))
            .collect::<Vec<_>>();
        assert!(!PersonalizationProfile::from_feedback(
            &one_class,
            Some("creator-1"),
            Some("Valorant")
        )
        .is_active());
    }

    #[test]
    fn learned_hook_preference_ranks_matching_candidate_higher() {
        let mut feedback = Vec::new();
        for id in 0..10 {
            feedback.push(row(id, "good", 0.9, "clutch", "Valorant"));
        }
        for id in 10..20 {
            feedback.push(row(id, "boring", 0.1, "idle", "Valorant"));
        }
        let profile = PersonalizationProfile::from_feedback(
            &feedback,
            Some("creator-1"),
            Some("Valorant"),
        );
        assert!(profile.is_active());

        let liked = profile.score_adjustment(
            [0.9, 0.5, 0.5, 0.5, 0.5, 0.5],
            &["clutch".to_string()],
            &["audio", "transcript"],
        );
        let disliked = profile.score_adjustment(
            [0.1, 0.5, 0.5, 0.5, 0.5, 0.5],
            &["idle".to_string()],
            &["audio", "transcript"],
        );
        assert!(liked > 0.0, "liked candidate should receive a boost");
        assert!(disliked < 0.0, "disliked candidate should receive a penalty");
        assert!(liked > disliked);
    }

    #[test]
    fn adjustment_never_exceeds_safety_bound() {
        let mut feedback = Vec::new();
        for id in 0..20 {
            feedback.push(row(id, "good", 1.0, "clutch", "Valorant"));
        }
        for id in 20..40 {
            feedback.push(row(id, "boring", 0.0, "idle", "Valorant"));
        }
        let profile = PersonalizationProfile::from_feedback(
            &feedback,
            Some("creator-1"),
            Some("Valorant"),
        );
        let adjustment = profile.score_adjustment(
            [1.0; 6],
            &["clutch".to_string()],
            &["audio"],
        );
        assert!(adjustment.abs() <= MAX_SCORE_ADJUSTMENT + f64::EPSILON);
    }

    #[test]
    fn status_explains_sample_and_variety_requirements() {
        let empty = PersonalizationStatus::from_feedback(&[]);
        assert_eq!(empty.state, "empty");
        assert!(!empty.is_personalizing);

        let too_few = vec![
            row(1, "good", 0.9, "clutch", "Valorant"),
            row(2, "boring", 0.2, "idle", "Valorant"),
        ];
        let status = PersonalizationStatus::from_feedback(&too_few);
        assert_eq!(status.state, "needs_more");
        assert_eq!(status.usable_ratings, 2);

        let one_class = (0..5)
            .map(|id| row(id, "good", 0.8, "clutch", "Valorant"))
            .collect::<Vec<_>>();
        let status = PersonalizationStatus::from_feedback(&one_class);
        assert_eq!(status.state, "needs_variety");
        assert_eq!(status.rating_classes, 1);
        assert!(!status.is_personalizing);
    }

    #[test]
    fn status_moves_from_learning_to_active() {
        let mut learning_rows = Vec::new();
        for id in 0..5 {
            learning_rows.push(row(id, "good", 0.9, "clutch", "Valorant"));
            learning_rows.push(row(id + 5, "boring", 0.1, "idle", "Valorant"));
        }
        let learning = PersonalizationStatus::from_feedback(&learning_rows);
        assert_eq!(learning.state, "learning");
        assert!(learning.is_personalizing);
        assert_eq!(learning.target_ratings, 20);

        let mut active_rows = learning_rows;
        for id in 10..15 {
            active_rows.push(row(id, "good", 0.9, "clutch", "Valorant"));
            active_rows.push(row(id + 5, "boring", 0.1, "idle", "Valorant"));
        }
        let active = PersonalizationStatus::from_feedback(&active_rows);
        assert_eq!(active.state, "active");
        assert!(active.is_personalizing);
        assert_eq!(active.usable_ratings, 20);
    }

    #[test]
    fn passive_behavior_is_lower_weight_but_usable_learning_evidence() {
        let behavior = vec![
            behavior_row(1, 0.82, 0.9),
            behavior_row(2, 0.10, 0.1),
            behavior_row(3, 0.78, 0.8),
            behavior_row(4, 0.15, 0.2),
        ];
        let status = PersonalizationStatus::from_evidence(&[], &behavior);

        assert_eq!(status.behavior_events, 4);
        assert_eq!(status.usable_behavior_events, 4);
        assert_eq!(status.total_evidence, 4);
        assert_eq!(status.state, "learning");
        assert!(status.is_personalizing);
    }
}
