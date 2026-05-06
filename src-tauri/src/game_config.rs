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

// ── Bundled config files (embedded at compile time) ──

const DEFAULT_TOML: &str = include_str!("../config/games/default.toml");

const KNOWN_GAMES_TOML: &str = include_str!("../config/games/_known_games.toml");

#[derive(Debug, Deserialize)]
struct GameEntry {
    genre: String,
}

#[derive(Debug, Deserialize)]
struct KnownGames {
    games: std::collections::HashMap<String, GameEntry>,
}

// ── Resolver ──

/// Parse the universal baseline `default.toml` into a fully-populated
/// ResolvedConfig. Panics at startup if `default.toml` is malformed or
/// missing any required field — those would be developer bugs in the
/// bundled config that ship with the binary, not user errors.
pub(crate) fn parse_default() -> ResolvedConfig {
    let partial: PartialConfig = toml::from_str(DEFAULT_TOML)
        .expect("default.toml must be valid TOML");

    ResolvedConfig {
        audio: AudioConfig {
            spike_threshold: partial.audio.spike_threshold
                .expect("default.toml must define audio.spike_threshold"),
        },
        chat: ChatConfig {
            rate_min_msgs_per_window: partial.chat.rate_min_msgs_per_window
                .expect("default.toml must define chat.rate_min_msgs_per_window"),
            emote_burst_threshold: partial.chat.emote_burst_threshold
                .expect("default.toml must define chat.emote_burst_threshold"),
        },
        transcript: TranscriptConfig {
            weight: partial.transcript.weight
                .expect("default.toml must define transcript.weight"),
        },
        selector: SelectorConfig {
            min_clip_duration: partial.selector.min_clip_duration
                .expect("default.toml must define selector.min_clip_duration"),
            max_clip_duration: partial.selector.max_clip_duration
                .expect("default.toml must define selector.max_clip_duration"),
            min_gap_between_clips: partial.selector.min_gap_between_clips
                .expect("default.toml must define selector.min_gap_between_clips"),
        },
        titles: TitleConfig {
            preferred_categories: partial.titles.preferred_categories.unwrap_or_default(),
            disabled_categories: partial.titles.disabled_categories.unwrap_or_default(),
        },
    }
}

impl ResolvedConfig {
    /// Resolve detection config for a VOD by walking the layer hierarchy:
    ///
    ///   1. default.toml — universal baseline
    ///   2. _<genre>.toml — genre file (if game is in _known_games.toml)
    ///   3. <game_name>.toml — per-game override (if a file exists)
    ///   4. Sensitivity multiplier on threshold-style knobs
    ///
    /// Unknown games skip layers 2 and 3 → behavior identical to pre-v1.3.11
    /// hardcoded defaults.
    pub fn resolve(game_name: Option<&str>, sensitivity: Sensitivity) -> Self {
        // Layer 1: Default
        let mut config = parse_default();

        // Layers 2-3 will be added in subsequent tasks.
        let _ = game_name;

        // Layer 4 (sensitivity multiplier) will be added in a later task.
        let _ = sensitivity;

        config
    }
}

/// Apply a partial config (sparse — most fields Optional) onto an existing
/// ResolvedConfig. Only fields present in the partial replace the resolved
/// values. Used for layering genre / per-game files onto the default.
///
/// Returns Err if the TOML is malformed. Caller decides whether to log + skip
/// or propagate the error.
pub(crate) fn apply_partial(
    config: &mut ResolvedConfig,
    toml_str: &str,
) -> Result<(), toml::de::Error> {
    let partial: PartialConfig = toml::from_str(toml_str)?;

    if let Some(v) = partial.audio.spike_threshold {
        config.audio.spike_threshold = v;
    }
    if let Some(v) = partial.chat.rate_min_msgs_per_window {
        config.chat.rate_min_msgs_per_window = v;
    }
    if let Some(v) = partial.chat.emote_burst_threshold {
        config.chat.emote_burst_threshold = v;
    }
    if let Some(v) = partial.transcript.weight {
        config.transcript.weight = v;
    }
    if let Some(v) = partial.selector.min_clip_duration {
        config.selector.min_clip_duration = v;
    }
    if let Some(v) = partial.selector.max_clip_duration {
        config.selector.max_clip_duration = v;
    }
    if let Some(v) = partial.selector.min_gap_between_clips {
        config.selector.min_gap_between_clips = v;
    }
    if let Some(v) = partial.titles.preferred_categories {
        config.titles.preferred_categories = v;
    }
    if let Some(v) = partial.titles.disabled_categories {
        config.titles.disabled_categories = v;
    }

    Ok(())
}

/// Look up `game_name` in the bundled `_known_games.toml` and return the
/// corresponding genre slug (e.g., "horror", "fps"). Returns `None` if the
/// game is not in the list — caller should skip layer-2/3 lookups and use
/// the default config only.
///
/// Match is exact + case-sensitive to avoid false matches between similarly-
/// named games. Twitch's API returns canonical capitalization.
pub(crate) fn genre_for_game(game_name: Option<&str>) -> Option<String> {
    let name = game_name?.trim();
    if name.is_empty() {
        return None;
    }
    // Parse the known-games file lazily on each call. Cheap (~30-50 entries)
    // and avoids the complexity of OnceLock / lazy_static for v1.3.11. Can
    // optimize later if profiling shows it matters.
    let known: KnownGames = toml::from_str(KNOWN_GAMES_TOML)
        .expect("_known_games.toml must be valid TOML");
    known.games.get(name).map(|e| e.genre.clone())
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_toml_parses_with_all_required_fields() {
        let config = parse_default();
        assert!((config.audio.spike_threshold - 0.55).abs() < 1e-6);
        assert_eq!(config.chat.rate_min_msgs_per_window, 5);
        assert_eq!(config.chat.emote_burst_threshold, 3);
        assert!((config.transcript.weight - 1.0).abs() < 1e-6);
        assert_eq!(config.selector.min_clip_duration, 15);
        assert_eq!(config.selector.max_clip_duration, 30);
        assert_eq!(config.selector.min_gap_between_clips, 30);
        assert!(config.titles.preferred_categories.is_empty());
        assert!(config.titles.disabled_categories.is_empty());
    }

    #[test]
    fn genre_override_replaces_only_specified_knobs() {
        // Simulate a genre TOML that overrides audio threshold only.
        let genre_toml = r#"
[audio]
spike_threshold = 0.45
"#;
        let mut config = parse_default();
        apply_partial(&mut config, genre_toml).expect("valid TOML");

        // Audio threshold overridden:
        assert!((config.audio.spike_threshold - 0.45).abs() < 1e-6);
        // Everything else still at default:
        assert_eq!(config.chat.rate_min_msgs_per_window, 5);
        assert_eq!(config.chat.emote_burst_threshold, 3);
        assert!((config.transcript.weight - 1.0).abs() < 1e-6);
        assert_eq!(config.selector.min_clip_duration, 15);
    }

    #[test]
    fn genre_override_can_replace_multiple_knobs() {
        // Realistic genre file — _horror.toml-shaped.
        let genre_toml = r#"
[audio]
spike_threshold = 0.45

[chat]
emote_burst_threshold = 5

[transcript]
weight = 0.7
"#;
        let mut config = parse_default();
        apply_partial(&mut config, genre_toml).expect("valid TOML");

        assert!((config.audio.spike_threshold - 0.45).abs() < 1e-6);
        assert_eq!(config.chat.emote_burst_threshold, 5);
        assert!((config.transcript.weight - 0.7).abs() < 1e-6);
        // Untouched knobs remain at default:
        assert_eq!(config.chat.rate_min_msgs_per_window, 5);
        assert_eq!(config.selector.min_clip_duration, 15);
    }

    #[test]
    fn known_game_resolves_to_genre() {
        assert_eq!(genre_for_game(Some("Dead by Daylight")), Some("horror".to_string()));
        assert_eq!(genre_for_game(Some("VALORANT")), Some("fps".to_string()));
        assert_eq!(genre_for_game(Some("Stardew Valley")), Some("cozy".to_string()));
    }

    #[test]
    fn unknown_game_returns_none() {
        assert_eq!(genre_for_game(Some("Some Indie Game That Doesnt Exist")), None);
        assert_eq!(genre_for_game(Some("")), None);
        assert_eq!(genre_for_game(None), None);
    }

    #[test]
    fn resolve_unknown_game_returns_default_unmodified() {
        let resolved = ResolvedConfig::resolve(
            Some("Totally Made Up Game"),
            Sensitivity::Medium,
        );
        // Should match parse_default() output exactly.
        let baseline = parse_default();
        assert!((resolved.audio.spike_threshold - baseline.audio.spike_threshold).abs() < 1e-6);
        assert_eq!(resolved.chat.rate_min_msgs_per_window, baseline.chat.rate_min_msgs_per_window);
        assert_eq!(resolved.chat.emote_burst_threshold, baseline.chat.emote_burst_threshold);
        assert_eq!(resolved.selector.min_clip_duration, baseline.selector.min_clip_duration);
    }

    #[test]
    fn resolve_with_none_game_returns_default() {
        let resolved = ResolvedConfig::resolve(None, Sensitivity::Medium);
        let baseline = parse_default();
        assert!((resolved.audio.spike_threshold - baseline.audio.spike_threshold).abs() < 1e-6);
    }
}
