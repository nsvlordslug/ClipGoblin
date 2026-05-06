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
        if s.eq_ignore_ascii_case("low") {
            Sensitivity::Low
        } else if s.eq_ignore_ascii_case("high") {
            Sensitivity::High
        } else {
            Sensitivity::Medium
        }
    }
}

// ── Resolved (final) config types ──
// These are what the analysis pipeline consumes. Every field is non-Option:
// the resolver guarantees defaults are filled before returning.

/// Audio detection thresholds. Resolved per-game from default + genre + override layers.
#[derive(Debug, Clone)]
pub struct AudioConfig {
    pub spike_threshold: f64,
}

/// Chat-rate and emote-burst detection thresholds. Resolved per-game.
#[derive(Debug, Clone)]
pub struct ChatConfig {
    pub rate_min_msgs_per_window: u32,
    pub emote_burst_threshold: u32,
}

/// Transcript signal weighting. Resolved per-game (cozy/talky games boost the weight).
#[derive(Debug, Clone)]
pub struct TranscriptConfig {
    pub weight: f64,
}

/// Final clip selection parameters — durations and pacing. Resolved per-game.
#[derive(Debug, Clone)]
pub struct SelectorConfig {
    /// Minimum clip length in seconds. Selected clips shorter than this are extended around their peak.
    pub min_clip_duration: u32,
    /// Maximum clip length in seconds. Selected clips longer than this are trimmed around their peak.
    pub max_clip_duration: u32,
    /// Minimum seconds between any two selected clips. Used as the floor on the dynamic cooldown window.
    pub min_gap_between_clips: u32,
}

/// Title generation preferences — which AftermathConfession categories to favor or disable.
#[derive(Debug, Clone, Default)]
pub struct TitleConfig {
    pub preferred_categories: Vec<String>,
    pub disabled_categories: Vec<String>,
}

/// Fully-resolved per-VOD detection config. Built once at the start of analysis
/// by walking the layer hierarchy: default.toml → _<genre>.toml → <game>.toml →
/// sensitivity multiplier. Passed by reference to each pipeline stage.
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
    pub(crate) spike_threshold: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct PartialChat {
    pub(crate) rate_min_msgs_per_window: Option<u32>,
    pub(crate) emote_burst_threshold: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct PartialTranscript {
    pub(crate) weight: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct PartialSelector {
    /// Minimum clip length in seconds.
    pub(crate) min_clip_duration: Option<u32>,
    /// Maximum clip length in seconds.
    pub(crate) max_clip_duration: Option<u32>,
    /// Minimum seconds between any two selected clips.
    pub(crate) min_gap_between_clips: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct PartialTitles {
    pub(crate) preferred_categories: Option<Vec<String>>,
    pub(crate) disabled_categories: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct PartialConfig {
    #[serde(default)]
    pub(crate) audio: PartialAudio,
    #[serde(default)]
    pub(crate) chat: PartialChat,
    #[serde(default)]
    pub(crate) transcript: PartialTranscript,
    #[serde(default)]
    pub(crate) selector: PartialSelector,
    #[serde(default)]
    pub(crate) titles: PartialTitles,
}
