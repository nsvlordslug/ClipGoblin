//! Per-game detection config system.
//!
//! Resolves detection thresholds for a VOD by walking a 4-layer hierarchy:
//!   1. default.toml — universal baseline
//!   2. _<genre>.toml — genre-level overrides (e.g., _horror.toml)
//!   3. <game_name>.toml — per-game overrides for outliers (e.g., dead_by_daylight.toml)
//!   4. Sensitivity multiplier (Low / Medium / High) applied to threshold knobs
//!
//! Game→genre mapping comes from _known_games.toml. Unknown games skip layers
//! 2 and 3 and use defaults only — same behavior as pre-v1.3.11 hardcoded
//! thresholds, so unrecognized games are not regressed.
//!
//! See docs/superpowers/specs/2026-04-30-per-game-detection-configs-design.md
//! for the full design rationale.

use serde::Deserialize;

// ── Sensitivity ──

/// User-facing sensitivity setting from Settings → Detection.
/// Lower multiplier = lower thresholds = more clips detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sensitivity {
    Low,
    Medium,
    High,
}

impl Sensitivity {
    /// Multiplier applied to threshold-style knobs.
    /// `Low = 1.2` (higher thresholds = fewer clips, only standout moments)
    /// `Medium = 1.0` (no adjustment)
    /// `High = 0.8` (lower thresholds = more clips, catch subtle moments)
    pub fn multiplier(self) -> f64 {
        match self {
            Sensitivity::Low => 1.2,
            Sensitivity::Medium => 1.0,
            Sensitivity::High => 0.8,
        }
    }

    /// Parse from a database/string value (case-insensitive).
    /// Falls back to Medium for any unrecognized value.
    pub fn from_str_or_default(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "low" => Sensitivity::Low,
            "high" => Sensitivity::High,
            _ => Sensitivity::Medium,
        }
    }
}

// ── Resolved (final) config types ──
// These are what the analysis pipeline consumes. Every field is non-Option:
// the resolver guarantees defaults are filled before returning.

#[derive(Debug, Clone)]
pub struct AudioConfig {
    pub spike_threshold: f64,
}

#[derive(Debug, Clone)]
pub struct ChatConfig {
    pub rate_min_msgs_per_window: u32,
    pub emote_burst_threshold: u32,
}

#[derive(Debug, Clone)]
pub struct TranscriptConfig {
    pub weight: f64,
}

#[derive(Debug, Clone)]
pub struct SelectorConfig {
    pub min_clip_duration: u32,
    pub max_clip_duration: u32,
    pub min_gap_between_clips: u32,
}

#[derive(Debug, Clone, Default)]
pub struct TitleConfig {
    pub preferred_categories: Vec<String>,
    pub disabled_categories: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub audio: AudioConfig,
    pub chat: ChatConfig,
    pub transcript: TranscriptConfig,
    pub selector: SelectorConfig,
    pub titles: TitleConfig,
}

// ── Partial config types ──
// These are what TOML parsing produces. Every field is Option<T> so genre and
// per-game files can override only specific knobs (sparse override pattern).
// The resolver merges PartialConfig instances onto a ResolvedConfig.

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct PartialAudio {
    pub spike_threshold: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct PartialChat {
    pub rate_min_msgs_per_window: Option<u32>,
    pub emote_burst_threshold: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct PartialTranscript {
    pub weight: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct PartialSelector {
    pub min_clip_duration: Option<u32>,
    pub max_clip_duration: Option<u32>,
    pub min_gap_between_clips: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct PartialTitles {
    pub preferred_categories: Option<Vec<String>>,
    pub disabled_categories: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct PartialConfig {
    #[serde(default)]
    pub audio: PartialAudio,
    #[serde(default)]
    pub chat: PartialChat,
    #[serde(default)]
    pub transcript: PartialTranscript,
    #[serde(default)]
    pub selector: PartialSelector,
    #[serde(default)]
    pub titles: PartialTitles,
}
