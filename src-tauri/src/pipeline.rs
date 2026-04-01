//! Shared data models for the clip-detection pipeline.
//!
//! These types are the contract between every stage of the pipeline:
//! signal extraction, fusion, scoring, and ranking.  They are also
//! the Tauri JSON boundary — every struct that reaches the frontend
//! derives `Serialize`.
//!
//! # Pipeline stages and which types they produce
//!
//! ```text
//! Signal extraction  →  Vec<SignalSegment>
//! Fusion + scoring   →  Vec<CandidateClip>
//! Ranking            →  Vec<CandidateClip>   (sorted, trimmed)
//! ```

// ═══════════════════════════════════════════════════════════════════
//  Signal layer
// ═══════════════════════════════════════════════════════════════════

/// The four signal sources that feed the detection pipeline.
///
/// Each source runs independently (and in parallel where possible)
/// and produces [`SignalSegment`]s that are later fused together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalType {
    /// Audio intensity / volume spike analysis via ffmpeg.
    Audio,
    /// Speech-to-text keyword and exclamation detection.
    Transcript,
    /// Hard cuts, camera changes, and rapid visual motion.
    SceneChange,
    /// AI vision-model analysis (Claude Vision / Gemini Vision).
    Vision,
}

impl SignalType {
    /// All signal types that run without an external API.
    pub const LOCAL: &[SignalType] = &[
        SignalType::Audio,
        SignalType::Transcript,
        SignalType::SceneChange,
    ];

    /// Human-readable label for UI display and logging.
    pub fn label(self) -> &'static str {
        match self {
            Self::Audio => "Audio",
            Self::Transcript => "Transcript",
            Self::SceneChange => "Scene Change",
            Self::Vision => "Vision Model",
        }
    }

    /// Whether this signal type requires a remote API key.
    pub fn requires_api(self) -> bool {
        matches!(self, Self::Vision)
    }
}

impl std::fmt::Display for SignalType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Extra detail attached to a [`SignalSegment`].
///
/// Each variant carries data that only its source can produce.
/// Downstream scoring uses these fields to make evidence-quality
/// judgments beyond the raw confidence number.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SignalMetadata {
    /// Audio-specific measurements.
    Audio {
        /// RMS delta from the preceding quiet window (0.0–1.0 linear).
        rms_delta: f64,
        /// Absolute RMS at the peak second.
        peak_rms: f64,
        /// How much louder the spike was relative to the VOD average.
        ratio_above_avg: f64,
    },
    /// Transcript-specific context.
    Transcript {
        /// The spoken words around the detection.
        text: String,
        /// The specific viral keyword that triggered (e.g. "no way").
        keyword: Option<String>,
        /// Detected language code (e.g. "en").
        language: Option<String>,
    },
    /// Scene-change measurements.
    SceneChange {
        /// Frame-difference magnitude (0.0 = identical, 1.0 = total change).
        magnitude: f64,
        /// Type of visual change detected.
        change_type: SceneChangeKind,
    },
    /// Vision-model output (not used in local analysis, kept for type compatibility).
    Vision {
        /// Model-generated description of the moment ("streamer slams desk").
        description: String,
        /// Per-dimension scores the model returned.
        model_scores: VisionDimensionScores,
        /// Which provider produced this result (e.g. "Claude", "Gemini").
        provider: String,
    },
}

/// Categorisation of a detected scene change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneChangeKind {
    /// Hard cut between two different shots.
    HardCut,
    /// Rapid camera movement / zoom.
    FastMotion,
    /// Gradual transition (fade, dissolve).
    Transition,
}

/// Per-dimension scores from a vision model response.
///
/// Each field is 0.0–1.0.  These get blended with local heuristic
/// scores during the quality-scoring phase.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct VisionDimensionScores {
    pub hook_strength: f64,
    pub emotional_spike: f64,
    pub payoff_clarity: f64,
    pub loopability: f64,
    pub context_simplicity: f64,
}

/// A timestamped detection from a single signal source.
///
/// This is the universal output of Phase 1.  Every signal provider
/// produces `Vec<SignalSegment>` which the pipeline later fuses
/// into candidate clips.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SignalSegment {
    /// Which signal source produced this detection.
    pub signal_type: SignalType,
    /// Start of the interesting moment (seconds from VOD start).
    pub start_time: f64,
    /// End of the interesting moment.
    pub end_time: f64,
    /// Source-specific confidence, 0.0 (noise) – 1.0 (certain).
    pub score: f64,
    /// Semantic labels (e.g. ["fight", "reaction", "shock"]).
    pub tags: Vec<String>,
    /// Source-specific detail.  `None` when the provider has no
    /// extra information beyond the score.
    pub metadata: Option<SignalMetadata>,
}

impl SignalSegment {
    /// Duration of this segment in seconds.
    pub fn duration(&self) -> f64 {
        self.end_time - self.start_time
    }

    /// Midpoint timestamp (useful for fusion clustering).
    pub fn center(&self) -> f64 {
        (self.start_time + self.end_time) / 2.0
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Score breakdown — raw signals
// ═══════════════════════════════════════════════════════════════════

/// Raw per-signal-type scores from the detection modules.
///
/// Set by the fusion stage.  The ranker reads these to compute
/// [`DimensionScores`] and the final [`ScoreReport`].
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ClipScoreBreakdown {
    /// Strength of the audio evidence (0.0–1.0).
    pub audio_score: f64,
    /// Strength of the speech/transcript evidence (0.0–1.0).
    pub speech_score: f64,
    /// Strength of the scene-change evidence (0.0–1.0).
    pub scene_score: f64,
    /// Strength of the vision-model evidence, if a model was used.
    /// `None` in local-only mode.
    pub vision_score: Option<f64>,
}

impl ClipScoreBreakdown {
    /// Build a breakdown from raw signal scores (clamped to 0.0–1.0).
    pub fn new(audio: f64, speech: f64, scene: f64, vision: Option<f64>) -> Self {
        Self {
            audio_score: audio.clamp(0.0, 1.0),
            speech_score: speech.clamp(0.0, 1.0),
            scene_score: scene.clamp(0.0, 1.0),
            vision_score: vision.map(|v| v.clamp(0.0, 1.0)),
        }
    }

    /// How many non-zero signal sources contributed.
    pub fn active_signal_count(&self) -> usize {
        let mut n = 0;
        if self.audio_score > 0.0 { n += 1; }
        if self.speech_score > 0.0 { n += 1; }
        if self.scene_score > 0.0 { n += 1; }
        if self.vision_score.unwrap_or(0.0) > 0.0 { n += 1; }
        n
    }

    /// Best raw signal score across all types.
    pub fn best_raw(&self) -> f64 {
        self.audio_score
            .max(self.speech_score)
            .max(self.scene_score)
            .max(self.vision_score.unwrap_or(0.0))
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Score breakdown — quality dimensions
// ═══════════════════════════════════════════════════════════════════

/// Five quality dimensions that assess *why* a clip is interesting.
///
/// Computed by the ranker from the raw signal scores.  Each dimension
/// cross-cuts the signals — a single audio spike can contribute to
/// both `hook_strength` and `emotional_intensity`.
///
/// Every field is 0.0–1.0.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DimensionScores {
    /// Does the opening arrest attention?  Derived from:
    /// audio energy near the start + scene cuts + vision hook.
    pub hook_strength: f64,
    /// How emotionally intense is the peak?  Derived from:
    /// audio peak + speech keywords/exclamations + vision reactions.
    pub emotional_intensity: f64,
    /// Can a viewer instantly understand the outcome?  Derived from:
    /// speech transcript + vision description + scene clarity.
    pub context_clarity: f64,
    /// Is there on-screen action?  Derived from:
    /// scene cuts/motion + vision action + audio energy.
    pub visual_activity: f64,
    /// Is the speech impactful?  Derived from:
    /// speech keywords/exclamations + audio loudness.
    pub speech_punch: f64,
}

impl DimensionScores {
    /// Compute dimensions from raw signal scores.
    ///
    /// Each dimension blends multiple signals with fixed derivation
    /// weights.  These are *not* configurable — they define what
    /// each dimension *means*.  The configurable part is the dimension
    /// *weights* that compute the final score.
    pub fn from_signals(raw: &ClipScoreBreakdown) -> Self {
        let a = raw.audio_score;
        let s = raw.speech_score;
        let sc = raw.scene_score;
        let v = raw.vision_score.unwrap_or(0.0);

        Self {
            hook_strength:      (a * 0.55 + sc * 0.30 + v * 0.15).min(1.0),
            emotional_intensity:(a * 0.40 + s * 0.35 + v * 0.25).min(1.0),
            context_clarity:    (s * 0.55 + v * 0.35 + sc * 0.10).min(1.0),
            visual_activity:    (sc * 0.60 + v * 0.25 + a * 0.15).min(1.0),
            speech_punch:       (s * 0.65 + a * 0.25 + sc * 0.10).min(1.0),
        }
    }

    /// Weighted composite of all dimensions.
    pub fn weighted(&self, w: &DimensionWeights) -> f64 {
        (self.hook_strength * w.hook
            + self.emotional_intensity * w.emotion
            + self.context_clarity * w.context
            + self.visual_activity * w.visual
            + self.speech_punch * w.speech)
            .min(0.99)
    }

    /// Return dimensions as labeled pairs, sorted strongest first.
    pub fn as_labeled_pairs(&self) -> Vec<(&'static str, f64)> {
        let mut pairs = vec![
            ("Hook strength", self.hook_strength),
            ("Emotional intensity", self.emotional_intensity),
            ("Context clarity", self.context_clarity),
            ("Visual activity", self.visual_activity),
            ("Speech punch", self.speech_punch),
        ];
        pairs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        pairs
    }
}

/// Weights for each quality dimension.  Must sum to 1.0.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DimensionWeights {
    pub hook: f64,
    pub emotion: f64,
    pub context: f64,
    pub visual: f64,
    pub speech: f64,
}

impl DimensionWeights {
    pub fn validate(&self) -> Result<(), String> {
        let sum = self.hook + self.emotion + self.context + self.visual + self.speech;
        if (sum - 1.0).abs() > 0.001 {
            return Err(format!("Dimension weights must sum to 1.0, got {sum:.4}"));
        }
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Score report — the complete breakdown
// ═══════════════════════════════════════════════════════════════════

/// Complete scoring breakdown stored on each ranked clip.
///
/// Contains every number that affected the ranking — raw signals,
/// quality dimensions, bonuses, penalties, and the final score.
/// The frontend can render all of this for transparency.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScoreReport {
    /// Raw per-signal scores from the detection modules.
    pub raw_signals: ClipScoreBreakdown,
    /// Quality dimensions derived from the raw signals.
    pub dimensions: DimensionScores,
    /// Weighted sum of dimensions (before bonuses/penalties).
    pub dimension_weighted: f64,
    /// Individual bonuses applied.
    pub bonuses: Vec<ScoreFactor>,
    /// Sum of all bonus values.
    pub bonus_total: f64,
    /// Individual penalties applied.
    pub penalties: Vec<ScoreFactor>,
    /// Sum of all penalty values (positive number, subtracted from score).
    pub penalty_total: f64,
    /// Internal ranking score used for ordering.
    /// `dimension_weighted + bonus_total - penalty_total`, clamped 0.0–1.0.
    /// This drives the greedy selection and is NOT shown to users.
    pub rank_score: f64,
    /// User-facing confidence score (0.0–1.0).
    /// `dimension_weighted` rescaled to be intuitive: a strong clip
    /// with good signals reads as 0.7–0.9, not 0.3–0.5.
    pub confidence: f64,
    /// Human-readable explanation.
    pub explanation: String,
    /// Top contributing dimensions, strongest first.
    pub key_dimensions: Vec<String>,
}

/// A single named factor (bonus or penalty).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScoreFactor {
    pub label: String,
    pub value: f64,
}

// ═══════════════════════════════════════════════════════════════════
//  Candidate clip
// ═══════════════════════════════════════════════════════════════════

/// A fully scored highlight clip ready for the frontend.
///
/// Produced after signal fusion, quality scoring, and boundary
/// optimization.  The frontend displays these directly in the
/// clip list / timeline view.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CandidateClip {
    /// Unique identifier (UUID v4).
    pub id: String,

    // ── Timing ──
    /// Start time in the VOD (seconds), snapped to action.
    pub start_time: f64,
    /// End time in the VOD (seconds), trimmed of dead air.
    pub end_time: f64,

    // ── Scoring ──
    /// Overall confidence that this is a good clip (0.0–1.0).
    /// Set by the ranker from `score_report.final_score`.
    pub confidence_score: f64,
    /// Raw per-signal scores from the detection modules.
    pub score_breakdown: ClipScoreBreakdown,
    /// Full scoring report (dimensions + bonuses + penalties).
    /// `None` before the ranker runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_report: Option<ScoreReport>,

    // ── Content ──
    /// Short title describing the moment (generated or model-provided).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Longer summary / description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// One-line hook / tease for the UI (e.g. "Wait for it...")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook: Option<String>,
    /// Path to a preview thumbnail image on disk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_thumbnail_path: Option<String>,
    /// Transcript excerpt captured during this time range.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript_excerpt: Option<String>,

    // ── Signal provenance ──
    /// Which signal types contributed evidence for this clip.
    pub signal_sources: Vec<SignalType>,
    /// Semantic event tags merged from all contributing signals.
    pub tags: Vec<String>,

    // ── Selection metadata ──
    /// Fingerprint for deduplication (e.g. "fight+shock").
    pub fingerprint: String,
    /// Hard-reject reason if this candidate was filtered out.
    /// `None` means the clip passed all quality gates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,

    // ── Event summary ──
    /// One-sentence description of what happened in the clip.
    /// Synthesized from transcript + tags + context signals.
    /// Set during labeling; `None` before the labeler runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_summary: Option<String>,

    // ── Post captions ──
    /// TikTok-ready caption variants (casual, funny, hype + hashtags).
    /// Set after labeling; `None` before the caption generator runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_captions: Option<crate::post_captions::PostCaptions>,
}

impl CandidateClip {
    /// Duration of the clip in seconds.
    pub fn duration(&self) -> f64 {
        self.end_time - self.start_time
    }

    /// Whether this clip passed all quality gates.
    pub fn is_accepted(&self) -> bool {
        self.rejection_reason.is_none()
    }

    /// How many distinct signal types contributed.
    pub fn signal_count(&self) -> usize {
        self.signal_sources.len()
    }

    /// Create a new candidate with an auto-generated UUID.
    ///
    /// `confidence_score` starts at 0.0 — it is set by the ranker
    /// after applying the weight profile.
    pub fn new(
        start_time: f64,
        end_time: f64,
        score_breakdown: ClipScoreBreakdown,
        signal_sources: Vec<SignalType>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            start_time,
            end_time,
            confidence_score: 0.0,
            score_breakdown,
            score_report: None,
            title: None,
            summary: None,
            hook: None,
            preview_thumbnail_path: None,
            transcript_excerpt: None,
            signal_sources,
            tags: Vec::new(),
            fingerprint: String::new(),
            rejection_reason: None,
            event_summary: None,
            post_captions: None,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Clip explanation (transparency for the UI)
// ═══════════════════════════════════════════════════════════════════

/// Structured explanation of why a clip was selected and how it scored.
// ClipExplanation has been replaced by ScoreReport (above).
// The explanation, key dimensions, bonuses, and penalties all
// live on ScoreReport — the single source of scoring truth.

// Note: Clip duration constraints, rejection thresholds, and output
// caps are defined in clip_ranker::WeightProfile — the single source
// of truth for all scoring parameters.

// ═══════════════════════════════════════════════════════════════════
//  Analysis mode & vision provider
// ═══════════════════════════════════════════════════════════════════

/// Analysis mode for the pipeline.
///
/// Clip detection always runs locally — no external APIs, no API keys.
/// The provider system (BYOK) is only used for caption and title generation.
///
/// ```text
/// LocalOnly  →  Audio + Transcript + SceneChange
/// ```
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum AnalysisMode {
    /// Run local signal providers only.  No network calls, no API key
    /// required.  This is the only analysis mode.
    LocalOnly,
}

impl AnalysisMode {
    /// Convenience constructor for local-only mode.
    pub fn local() -> Self {
        Self::LocalOnly
    }

    /// Always true — analysis is always local.
    pub fn is_local_only(&self) -> bool {
        true
    }

    /// Which signal types are active in this mode.
    pub fn enabled_signals(&self) -> Vec<SignalType> {
        vec![
            SignalType::Audio,
            SignalType::Transcript,
            SignalType::SceneChange,
        ]
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── SignalType ──

    #[test]
    fn signal_type_local_does_not_include_vision() {
        assert!(!SignalType::LOCAL.contains(&SignalType::Vision));
        assert_eq!(SignalType::LOCAL.len(), 3);
    }

    #[test]
    fn signal_type_requires_api_only_for_vision() {
        assert!(!SignalType::Audio.requires_api());
        assert!(!SignalType::Transcript.requires_api());
        assert!(!SignalType::SceneChange.requires_api());
        assert!(SignalType::Vision.requires_api());
    }

    // ── SignalSegment ──

    #[test]
    fn signal_segment_duration_and_center() {
        let seg = SignalSegment {
            signal_type: SignalType::Audio,
            start_time: 10.0,
            end_time: 20.0,
            score: 0.8,
            tags: vec![],
            metadata: None,
        };
        assert!((seg.duration() - 10.0).abs() < f64::EPSILON);
        assert!((seg.center() - 15.0).abs() < f64::EPSILON);
    }

    // ── ClipScoreBreakdown ──

    #[test]
    fn score_breakdown_active_signal_count() {
        assert_eq!(ClipScoreBreakdown::new(0.5, 0.0, 0.3, None).active_signal_count(), 2);
        assert_eq!(ClipScoreBreakdown::new(0.5, 0.5, 0.5, Some(0.5)).active_signal_count(), 4);
        assert_eq!(ClipScoreBreakdown::default().active_signal_count(), 0);
    }

    #[test]
    fn score_breakdown_clamps_inputs() {
        let b = ClipScoreBreakdown::new(1.5, -0.2, 0.5, Some(2.0));
        assert!((b.audio_score - 1.0).abs() < f64::EPSILON);
        assert!((b.speech_score - 0.0).abs() < f64::EPSILON);
        assert!((b.vision_score.unwrap() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn score_breakdown_best_raw() {
        let b = ClipScoreBreakdown::new(0.8, 0.6, 0.4, None);
        assert!((b.best_raw() - 0.8).abs() < 1e-9);
    }

    #[test]
    fn score_breakdown_best_raw_includes_vision() {
        let b = ClipScoreBreakdown::new(0.5, 0.3, 0.2, Some(0.9));
        assert!((b.best_raw() - 0.9).abs() < 1e-9);
    }

    // ── CandidateClip ──

    #[test]
    fn candidate_clip_new_generates_id() {
        let scores = ClipScoreBreakdown::new(0.7, 0.5, 0.3, None);
        let clip = CandidateClip::new(10.0, 30.0, scores, vec![SignalType::Audio]);
        assert!(!clip.id.is_empty());
        assert!((clip.duration() - 20.0).abs() < f64::EPSILON);
        assert!(clip.is_accepted());
        assert_eq!(clip.signal_count(), 1);
    }

    #[test]
    fn candidate_clip_confidence_starts_at_zero() {
        let scores = ClipScoreBreakdown::new(0.8, 0.6, 0.4, None);
        let clip = CandidateClip::new(0.0, 20.0, scores, vec![]);
        assert!((clip.confidence_score - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn candidate_clip_has_score_report_field() {
        let scores = ClipScoreBreakdown::new(0.5, 0.5, 0.5, None);
        let clip = CandidateClip::new(0.0, 20.0, scores, vec![]);
        assert!(clip.hook.is_none());
        assert!(clip.score_report.is_none());
    }

    // ── DimensionScores ──

    #[test]
    fn dimensions_from_signals_normalized() {
        let raw = ClipScoreBreakdown::new(1.0, 1.0, 1.0, Some(1.0));
        let dims = DimensionScores::from_signals(&raw);
        assert!(dims.hook_strength <= 1.0);
        assert!(dims.emotional_intensity <= 1.0);
        assert!(dims.context_clarity <= 1.0);
        assert!(dims.visual_activity <= 1.0);
        assert!(dims.speech_punch <= 1.0);
    }

    #[test]
    fn dimensions_zero_for_zero_signals() {
        let raw = ClipScoreBreakdown::default();
        let dims = DimensionScores::from_signals(&raw);
        assert!((dims.hook_strength - 0.0).abs() < 1e-9);
        assert!((dims.speech_punch - 0.0).abs() < 1e-9);
    }

    #[test]
    fn dimension_weights_validate() {
        let w = DimensionWeights {
            hook: 0.25, emotion: 0.30, context: 0.15, visual: 0.15, speech: 0.15,
        };
        assert!(w.validate().is_ok());

        let bad = DimensionWeights {
            hook: 0.9, emotion: 0.3, context: 0.1, visual: 0.1, speech: 0.1,
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn dimensions_as_labeled_pairs_sorted() {
        let raw = ClipScoreBreakdown::new(0.9, 0.3, 0.6, None);
        let dims = DimensionScores::from_signals(&raw);
        let pairs = dims.as_labeled_pairs();
        // Should be sorted by value descending
        for w in pairs.windows(2) {
            assert!(w[0].1 >= w[1].1, "pairs not sorted: {:?}", pairs);
        }
    }

    // ── AnalysisMode ──

    #[test]
    fn analysis_mode_local_has_three_signals() {
        let mode = AnalysisMode::local();
        assert!(mode.is_local_only());
        assert_eq!(mode.enabled_signals().len(), 3);
    }

    // ── Serialization ──

    #[test]
    fn signal_type_serializes_as_snake_case() {
        assert_eq!(serde_json::to_string(&SignalType::SceneChange).unwrap(), "\"scene_change\"");
        assert_eq!(serde_json::to_string(&SignalType::Vision).unwrap(), "\"vision\"");
    }

    #[test]
    fn signal_metadata_tagged_serialization() {
        let meta = SignalMetadata::Audio {
            rms_delta: 0.35,
            peak_rms: 0.72,
            ratio_above_avg: 2.1,
        };
        let json = serde_json::to_value(&meta).unwrap();
        assert_eq!(json["type"], "audio");
        assert!(json.get("rms_delta").is_some());
    }

    #[test]
    fn candidate_clip_skips_none_fields() {
        let scores = ClipScoreBreakdown::new(0.5, 0.5, 0.5, None);
        let clip = CandidateClip::new(0.0, 20.0, scores, vec![SignalType::Audio]);
        let json = serde_json::to_value(&clip).unwrap();
        // Optional fields set to None should not appear in JSON.
        assert!(json.get("title").is_none());
        assert!(json.get("summary").is_none());
        assert!(json.get("hook").is_none());
        assert!(json.get("preview_thumbnail_path").is_none());
        assert!(json.get("explanation").is_none());
        assert!(json.get("rejection_reason").is_none());
        // Required fields should always be present.
        assert!(json.get("id").is_some());
        assert!(json.get("confidence_score").is_some());
        assert!(json.get("score_breakdown").is_some());
    }

    #[test]
    fn scene_change_kind_round_trips() {
        let meta = SignalMetadata::SceneChange {
            magnitude: 0.85,
            change_type: SceneChangeKind::HardCut,
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: SignalMetadata = serde_json::from_str(&json).unwrap();
        match back {
            SignalMetadata::SceneChange { change_type, .. } => {
                assert_eq!(change_type, SceneChangeKind::HardCut);
            }
            _ => panic!("wrong variant"),
        }
    }
}
