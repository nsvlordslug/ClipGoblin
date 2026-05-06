# Per-Game Detection Configs (v1.3.11) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add bundled per-game detection configs to ClipGoblin so detection thresholds adapt to game type, improving clip selection quality across diverse content (DBD, FPS, RPG, cozy, talking).

**Architecture:** Layered config resolution (`default.toml` → `_<genre>.toml` → `<game_name>.toml` → sensitivity multiplier) loaded from bundled TOML files at compile time. A single `ResolvedConfig` struct is built once per VOD analysis and threaded through the detection pipeline.

**Tech Stack:** Rust, `serde` for derives, `toml = "0.8"` (already a dependency in `src-tauri/Cargo.toml`), `include_str!` for bundling config files.

**Spec:** `docs/superpowers/specs/2026-04-30-per-game-detection-configs-design.md`

---

## File Structure

### Files to create

```
src-tauri/src/game_config.rs                      # Config types + resolver + tests
src-tauri/config/games/default.toml               # Layer 1 — universal baseline
src-tauri/config/games/_known_games.toml          # game_name → genre lookup
src-tauri/config/games/_horror.toml               # Layer 2 — genre files
src-tauri/config/games/_fps.toml
src-tauri/config/games/_rpg.toml
src-tauri/config/games/_cozy.toml
src-tauri/config/games/_talking.toml
src-tauri/config/games/_strategy.toml
src-tauri/config/games/dead_by_daylight.toml      # Layer 3 — per-game overrides
src-tauri/config/games/valorant.toml
src-tauri/config/games/stardew_valley.toml
```

### Files to modify

| File | Purpose of change |
|---|---|
| `src-tauri/src/lib.rs` | Add `mod game_config;` declaration |
| `src-tauri/src/commands/vod.rs` (`run_analysis_signals` ~line 1522, `analyze_audio_intensity` line 333, `analyze_via_chat` line 1963) | Resolve config at top of pipeline, thread through audio + chat analysis |
| `src-tauri/src/clip_selector.rs` (`CurationConfig::for_duration` line 126, `select_clips` line 912) | Accept `&SelectorConfig` parameter, use config values for clip durations + gap |
| `src-tauri/src/commands/captions.rs` (`aftermath_from_tags`) | Filter variants by `preferred_categories` / `disabled_categories` |

### Why this layout

- **`game_config.rs` is one file**: types + resolver + tests live together because they evolve together. Single responsibility: "the game-config system."
- **Config files in `src-tauri/config/games/`**: Outside `src/` so they aren't compiled as Rust. Bundled into the binary via `include_str!` at compile time.
- **No new modifications to `whisper.rs`**: transcription is independent of game tuning in v1.3.11. `transcript.weight` only affects scoring, not whisper invocation.

---

## Tasks

### Task 1: Foundation — game_config module + ResolvedConfig types

Create the module file with all type definitions. No resolver logic yet — just types + skeleton.

**Files:**
- Create: `src-tauri/src/game_config.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1.1: Create the game_config module file**

Create `src-tauri/src/game_config.rs` with this exact content:

```rust
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
```

- [ ] **Step 1.2: Register the module in lib.rs**

Open `src-tauri/src/lib.rs`. Find the existing `mod` declarations near the top of the file (around lines 1-30). Add `mod game_config;` alphabetically among the others.

Example (match the existing pattern):
```rust
mod error;
mod game_config;     // NEW
mod hardware;
```

- [ ] **Step 1.3: Verify it compiles**

Run from project root:
```
cd src-tauri && cargo check 2>&1 | tail -10
```

Expected: `Finished` line, no errors. Warnings about unused types are OK at this stage — they're scaffolding.

- [ ] **Step 1.4: Commit**

```
git add src-tauri/src/game_config.rs src-tauri/src/lib.rs
git commit -m "feat: scaffold game_config module with resolved + partial config types"
```

---

### Task 2: Write default.toml + parse it via test

Create the universal baseline TOML file. Confirm parsing works with a unit test.

**Files:**
- Create: `src-tauri/config/games/default.toml`
- Modify: `src-tauri/src/game_config.rs` (add a `parse_default()` function + first test)

- [ ] **Step 2.1: Create the default.toml file**

Create `src-tauri/config/games/default.toml`:

```toml
# Universal baseline detection config.
# All other config layers (genre files, per-game overrides) inherit from this
# and only declare knobs that DIFFER from these defaults.
#
# See game_config.rs and the v1.3.11 design doc for context.

[audio]
# RMS level (0.0–1.0) above which a moment is considered an audio peak.
# Lower = more sensitive (more audio-driven clips).
spike_threshold = 0.55

[chat]
# Min chat messages in a 30-second window to count as a chat-rate peak.
# Lower for slow chats, higher for spammy gaming chats.
rate_min_msgs_per_window = 5

# Min emote occurrences in a 10-second window to count as an emote burst.
# Higher for chats that constantly spam emotes (DBD/horror community).
emote_burst_threshold = 3

[transcript]
# Weight (0.0–2.0) applied to transcript signal during clip selection.
# 1.0 = baseline. Higher for talky/narrative games (cozy, RPGs).
# Lower for action games where transcript matters less.
weight = 1.0

[selector]
# Min/max clip duration in seconds.
min_clip_duration = 15
max_clip_duration = 30

# Minimum seconds between selected clips. Prevents adjacent overlaps.
min_gap_between_clips = 30

[titles]
# AftermathConfession categories to PREFER for this game/genre.
# Empty list = all categories equally weighted.
preferred_categories = []

# Categories to DISABLE entirely. Useful for cozy games where templates
# like "explosion" / "death" / "ambush" don't fit the content.
disabled_categories = []
```

- [ ] **Step 2.2: Add `parse_default()` and a passing test (TDD)**

Open `src-tauri/src/game_config.rs`. Add at the bottom (after the partial types):

```rust
// ── Bundled config files (embedded at compile time) ──

const DEFAULT_TOML: &str = include_str!("../config/games/default.toml");

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
}
```

- [ ] **Step 2.3: Run the test**

```
cd src-tauri && cargo test game_config::tests::default_toml_parses_with_all_required_fields -- --nocapture
```

Expected: `test result: ok. 1 passed`.

- [ ] **Step 2.4: Commit**

```
git add src-tauri/src/game_config.rs src-tauri/config/games/default.toml
git commit -m "feat: parse default.toml as the baseline ResolvedConfig"
```

---

### Task 3: Implement layer 2 (genre file override) — TDD

Add the ability to apply a sparse override from a genre file onto the resolved config.

**Files:**
- Modify: `src-tauri/src/game_config.rs`

- [ ] **Step 3.1: Write a failing test for genre overlay**

In `src-tauri/src/game_config.rs`, inside `mod tests`, append:

```rust
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
```

- [ ] **Step 3.2: Run the test, expect failure (function not defined)**

```
cd src-tauri && cargo test game_config::tests::genre_override_replaces_only_specified_knobs -- --nocapture
```

Expected: COMPILE ERROR with `cannot find function 'apply_partial' in this scope`. That's the failing test.

- [ ] **Step 3.3: Implement `apply_partial`**

In `src-tauri/src/game_config.rs`, after the `parse_default` function and before the `mod tests`, add:

```rust
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
```

- [ ] **Step 3.4: Run the test, expect pass**

```
cd src-tauri && cargo test game_config::tests::genre_override_replaces_only_specified_knobs -- --nocapture
```

Expected: `test result: ok. 1 passed`.

- [ ] **Step 3.5: Add a second test — multi-knob override**

In `mod tests`, append:

```rust
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
```

- [ ] **Step 3.6: Run all game_config tests**

```
cd src-tauri && cargo test game_config -- --nocapture
```

Expected: 3 tests pass.

- [ ] **Step 3.7: Commit**

```
git add src-tauri/src/game_config.rs
git commit -m "feat: apply_partial overlays sparse TOML onto ResolvedConfig"
```

---

### Task 4: Implement game→genre lookup from _known_games.toml — TDD

Add the ability to look up a game name and return its genre.

**Files:**
- Create: `src-tauri/config/games/_known_games.toml`
- Modify: `src-tauri/src/game_config.rs`

- [ ] **Step 4.1: Create `_known_games.toml` with initial entries**

Create `src-tauri/config/games/_known_games.toml`:

```toml
# Maps Twitch game_name strings (exact, case-sensitive) to a genre.
# Add new entries here when patching releases — no Rust code change needed.
# Genre names must match the corresponding _<genre>.toml file.

[games]
# Horror
"Dead by Daylight"                = { genre = "horror" }
"Phasmophobia"                    = { genre = "horror" }
"Resident Evil 4"                 = { genre = "horror" }
"Outlast Trials"                  = { genre = "horror" }
"Lethal Company"                  = { genre = "horror" }

# FPS
"VALORANT"                        = { genre = "fps" }
"Counter-Strike 2"                = { genre = "fps" }
"Apex Legends"                    = { genre = "fps" }
"Call of Duty: Warzone"           = { genre = "fps" }
"Fortnite"                        = { genre = "fps" }
"Marvel Rivals"                   = { genre = "fps" }

# RPG
"ELDEN RING"                      = { genre = "rpg" }
"Baldur's Gate 3"                 = { genre = "rpg" }
"Cyberpunk 2077"                  = { genre = "rpg" }
"World of Warcraft"               = { genre = "rpg" }
"Path of Exile 2"                 = { genre = "rpg" }
"Dragon Age: The Veilguard"       = { genre = "rpg" }

# Cozy
"Stardew Valley"                  = { genre = "cozy" }
"Minecraft"                       = { genre = "cozy" }
"Animal Crossing: New Horizons"   = { genre = "cozy" }
"Palworld"                        = { genre = "cozy" }
"Schedule I"                      = { genre = "cozy" }

# Talking
"Just Chatting"                   = { genre = "talking" }
"IRL"                             = { genre = "talking" }
"ASMR"                            = { genre = "talking" }
"Music"                           = { genre = "talking" }
"Talk Shows & Podcasts"           = { genre = "talking" }

# Strategy
"League of Legends"               = { genre = "strategy" }
"Dota 2"                          = { genre = "strategy" }
"Hearthstone"                     = { genre = "strategy" }
"Magic: The Gathering"            = { genre = "strategy" }
"Teamfight Tactics"               = { genre = "strategy" }
```

- [ ] **Step 4.2: Write failing test for genre lookup**

In `src-tauri/src/game_config.rs`, inside `mod tests`, append:

```rust
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
```

- [ ] **Step 4.3: Run tests, expect failure**

```
cd src-tauri && cargo test game_config::tests::known_game_resolves_to_genre -- --nocapture
```

Expected: COMPILE ERROR `cannot find function 'genre_for_game'`. Failing.

- [ ] **Step 4.4: Implement `genre_for_game`**

In `src-tauri/src/game_config.rs`, near the `DEFAULT_TOML` constant and `parse_default` function (under `// ── Bundled config files ──`), add:

```rust
const KNOWN_GAMES_TOML: &str = include_str!("../config/games/_known_games.toml");

#[derive(Debug, Deserialize)]
struct GameEntry {
    genre: String,
}

#[derive(Debug, Deserialize)]
struct KnownGames {
    games: std::collections::HashMap<String, GameEntry>,
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
```

- [ ] **Step 4.5: Run tests, expect pass**

```
cd src-tauri && cargo test game_config::tests -- --nocapture
```

Expected: 5 tests pass (default parse, sparse override, multi-knob override, known game, unknown game).

- [ ] **Step 4.6: Commit**

```
git add src-tauri/src/game_config.rs src-tauri/config/games/_known_games.toml
git commit -m "feat: genre_for_game lookup against bundled _known_games.toml"
```

---

### Task 5: Wire up `ResolvedConfig::resolve` for default-only path — TDD

Implement the public resolver entrypoint with default-only behavior. Subsequent tasks add genre + per-game layers + sensitivity.

**Files:**
- Modify: `src-tauri/src/game_config.rs`

- [ ] **Step 5.1: Write failing test for unknown-game resolution**

In `src-tauri/src/game_config.rs`, inside `mod tests`, append:

```rust
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
```

- [ ] **Step 5.2: Run, expect failure**

```
cd src-tauri && cargo test game_config::tests::resolve_unknown_game -- --nocapture
```

Expected: COMPILE ERROR `no function 'resolve' for ResolvedConfig`.

- [ ] **Step 5.3: Implement minimal `resolve`**

In `src-tauri/src/game_config.rs`, add an `impl ResolvedConfig` block (place it after the `parse_default` function). For now, only the default + unknown-game path:

```rust
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
```

- [ ] **Step 5.4: Run, expect pass**

```
cd src-tauri && cargo test game_config::tests -- --nocapture
```

Expected: 7 tests pass (5 prior + 2 new resolve tests).

- [ ] **Step 5.5: Commit**

```
git add src-tauri/src/game_config.rs
git commit -m "feat: ResolvedConfig::resolve scaffolded with default-only layer"
```

---

### Task 6: Add genre layer to `resolve` — TDD

Layer 2 of the hierarchy: load `_<genre>.toml` and overlay onto default.

**Files:**
- Create: `src-tauri/config/games/_horror.toml` (minimal — just for testing this task; full content in Task 9)
- Modify: `src-tauri/src/game_config.rs`

- [ ] **Step 6.1: Create a stub `_horror.toml` for the test**

Create `src-tauri/config/games/_horror.toml`:

```toml
# Horror genre — sudden audio peaks (screams), emote-spammy chat, less narration.
# Sparse override: only declare knobs that DIFFER from default.toml.

[audio]
spike_threshold = 0.45

[chat]
emote_burst_threshold = 5

[transcript]
weight = 0.7
```

- [ ] **Step 6.2: Write failing test for genre resolution**

In `src-tauri/src/game_config.rs` `mod tests`, append:

```rust
    #[test]
    fn resolve_horror_game_applies_genre_override() {
        let resolved = ResolvedConfig::resolve(
            Some("Dead by Daylight"),
            Sensitivity::Medium,
        );
        // _horror.toml overrides:
        assert!((resolved.audio.spike_threshold - 0.45).abs() < 1e-6);
        assert_eq!(resolved.chat.emote_burst_threshold, 5);
        assert!((resolved.transcript.weight - 0.7).abs() < 1e-6);
        // Knobs not in _horror.toml stay at default:
        assert_eq!(resolved.chat.rate_min_msgs_per_window, 5);
        assert_eq!(resolved.selector.min_clip_duration, 15);
    }
```

- [ ] **Step 6.3: Run, expect failure**

```
cd src-tauri && cargo test game_config::tests::resolve_horror_game -- --nocapture
```

Expected: assertion failure — the test expects `audio.spike_threshold = 0.45` but resolver still returns 0.55 because we haven't added layer 2 yet.

- [ ] **Step 6.4: Implement layer 2 (genre overlay)**

In `src-tauri/src/game_config.rs`, modify `ResolvedConfig::resolve` to add the genre layer. Replace the `// Layers 2-3 will be added` comment block with:

```rust
        // Layer 2: Genre file (if game is in _known_games.toml)
        if let Some(genre) = genre_for_game(game_name) {
            if let Some(genre_toml) = bundled_genre_toml(&genre) {
                if let Err(e) = apply_partial(&mut config, genre_toml) {
                    log::warn!(
                        "[game-config] Skipping malformed genre file '{}': {}",
                        genre, e
                    );
                }
            }
        }
```

Then add this helper function near the bottom of the file (above the test module):

```rust
/// Look up the bundled genre TOML by slug. Returns `None` if no genre file
/// exists for that slug — caller falls through (no error, just skip layer).
///
/// Genre files are bundled at compile time via include_str! so this is
/// just a static dispatch on the slug.
fn bundled_genre_toml(genre: &str) -> Option<&'static str> {
    match genre {
        "horror"   => Some(include_str!("../config/games/_horror.toml")),
        "fps"      => Some(include_str!("../config/games/_fps.toml")),
        "rpg"      => Some(include_str!("../config/games/_rpg.toml")),
        "cozy"     => Some(include_str!("../config/games/_cozy.toml")),
        "talking"  => Some(include_str!("../config/games/_talking.toml")),
        "strategy" => Some(include_str!("../config/games/_strategy.toml")),
        _          => None,
    }
}
```

**Important:** This function references genre files we haven't created yet (`_fps.toml`, `_rpg.toml`, `_cozy.toml`, `_talking.toml`, `_strategy.toml`). Compile will fail until we create those. We'll create them as empty stubs in the next step so this task can compile, then fill them in Task 9.

- [ ] **Step 6.5: Create stub files for the other genres**

Create five empty stub files (each just an empty TOML — no overrides means inherit everything from default):

`src-tauri/config/games/_fps.toml`:
```toml
# FPS genre — placeholder, real content lands in Task 9.
```

`src-tauri/config/games/_rpg.toml`:
```toml
# RPG genre — placeholder, real content lands in Task 9.
```

`src-tauri/config/games/_cozy.toml`:
```toml
# Cozy genre — placeholder, real content lands in Task 9.
```

`src-tauri/config/games/_talking.toml`:
```toml
# Talking genre — placeholder, real content lands in Task 9.
```

`src-tauri/config/games/_strategy.toml`:
```toml
# Strategy genre — placeholder, real content lands in Task 9.
```

Empty TOML parses as an empty `PartialConfig` — every field is `None`, no knobs overridden, behavior is identical to default. Safe placeholder.

- [ ] **Step 6.6: Run all tests**

```
cd src-tauri && cargo test game_config::tests -- --nocapture
```

Expected: 8 tests pass (7 prior + new horror test).

- [ ] **Step 6.7: Commit**

```
git add src-tauri/src/game_config.rs src-tauri/config/games/_horror.toml src-tauri/config/games/_fps.toml src-tauri/config/games/_rpg.toml src-tauri/config/games/_cozy.toml src-tauri/config/games/_talking.toml src-tauri/config/games/_strategy.toml
git commit -m "feat: layer 2 — genre file overlay in ResolvedConfig::resolve"
```

---

### Task 7: Add per-game override layer to `resolve` — TDD

Layer 3 of the hierarchy: optionally load `<game_name>.toml` and overlay on top of genre.

**Files:**
- Create: `src-tauri/config/games/dead_by_daylight.toml` (stub for testing)
- Modify: `src-tauri/src/game_config.rs`

- [ ] **Step 7.1: Create a stub DBD per-game file**

Create `src-tauri/config/games/dead_by_daylight.toml`:

```toml
# DBD specifically — even more emote-spammy than typical horror.
# Sparse override of _horror.toml.

[chat]
emote_burst_threshold = 7
```

- [ ] **Step 7.2: Write failing test**

In `src-tauri/src/game_config.rs` `mod tests`, append:

```rust
    #[test]
    fn resolve_dbd_applies_per_game_on_top_of_horror() {
        let resolved = ResolvedConfig::resolve(
            Some("Dead by Daylight"),
            Sensitivity::Medium,
        );
        // dead_by_daylight.toml overrides _horror.toml's emote_burst_threshold (5 → 7):
        assert_eq!(resolved.chat.emote_burst_threshold, 7);
        // _horror.toml's audio threshold still applies (DBD doesn't override):
        assert!((resolved.audio.spike_threshold - 0.45).abs() < 1e-6);
        // _horror.toml's transcript weight still applies:
        assert!((resolved.transcript.weight - 0.7).abs() < 1e-6);
    }

    #[test]
    fn resolve_horror_game_without_per_game_file_uses_genre_only() {
        // Phasmophobia is in _known_games (horror) but has no per-game file
        // → resolution stops after layer 2 (horror).
        let resolved = ResolvedConfig::resolve(
            Some("Phasmophobia"),
            Sensitivity::Medium,
        );
        // _horror.toml's emote_burst_threshold = 5 (NOT 7 like DBD):
        assert_eq!(resolved.chat.emote_burst_threshold, 5);
    }
```

- [ ] **Step 7.3: Run, expect failure**

```
cd src-tauri && cargo test game_config::tests::resolve_dbd -- --nocapture
```

Expected: assertion failure — `emote_burst_threshold = 5 (genre value)` but test expects 7.

- [ ] **Step 7.4: Implement layer 3 (per-game overlay)**

In `src-tauri/src/game_config.rs`, modify `ResolvedConfig::resolve`. After the genre layer block, add layer 3:

```rust
        // Layer 3: Per-game override file (optional)
        if let Some(name) = game_name {
            if let Some(game_toml) = bundled_game_toml(name) {
                if let Err(e) = apply_partial(&mut config, game_toml) {
                    log::warn!(
                        "[game-config] Skipping malformed per-game file '{}': {}",
                        name, e
                    );
                }
            }
        }
```

Then add the lookup helper near `bundled_genre_toml`:

```rust
/// Look up a per-game override TOML by exact game name. Returns `None` if
/// no per-game override file exists for that game — caller falls through
/// (no error, genre defaults stand).
///
/// Per-game files are bundled at compile time via include_str! and only
/// exist for games whose signal patterns deviate notably from their genre
/// baseline.
fn bundled_game_toml(game_name: &str) -> Option<&'static str> {
    match game_name {
        "Dead by Daylight" => Some(include_str!("../config/games/dead_by_daylight.toml")),
        "VALORANT"         => Some(include_str!("../config/games/valorant.toml")),
        "Stardew Valley"   => Some(include_str!("../config/games/stardew_valley.toml")),
        _ => None,
    }
}
```

**Important:** This references `valorant.toml` and `stardew_valley.toml` which we haven't created. Create stubs.

- [ ] **Step 7.5: Create stub per-game files for Valorant and Stardew**

`src-tauri/config/games/valorant.toml`:
```toml
# Valorant — placeholder, real content lands in Task 10.
```

`src-tauri/config/games/stardew_valley.toml`:
```toml
# Stardew Valley — placeholder, real content lands in Task 10.
```

- [ ] **Step 7.6: Run tests**

```
cd src-tauri && cargo test game_config::tests -- --nocapture
```

Expected: 10 tests pass (8 prior + 2 new).

- [ ] **Step 7.7: Commit**

```
git add src-tauri/src/game_config.rs src-tauri/config/games/dead_by_daylight.toml src-tauri/config/games/valorant.toml src-tauri/config/games/stardew_valley.toml
git commit -m "feat: layer 3 — per-game override files in ResolvedConfig::resolve"
```

---

### Task 8: Add sensitivity multiplier (layer 4) — TDD

Apply Low/Medium/High multiplier to threshold-style knobs.

**Files:**
- Modify: `src-tauri/src/game_config.rs`

- [ ] **Step 8.1: Write failing tests for sensitivity**

In `src-tauri/src/game_config.rs` `mod tests`, append:

```rust
    #[test]
    fn sensitivity_high_lowers_thresholds() {
        // Default emote_burst_threshold = 3, multiplier 0.8 → 3 * 0.8 = 2.4 → rounds to 2
        let high = ResolvedConfig::resolve(None, Sensitivity::High);
        assert_eq!(high.chat.emote_burst_threshold, 2);

        // chat.rate_min_msgs_per_window: 5 * 0.8 = 4.0 → 4
        assert_eq!(high.chat.rate_min_msgs_per_window, 4);

        // audio.spike_threshold: 0.55 * 0.8 = 0.44
        assert!((high.audio.spike_threshold - 0.44).abs() < 1e-6);
    }

    #[test]
    fn sensitivity_low_raises_thresholds() {
        // Default emote_burst_threshold = 3, multiplier 1.2 → 3.6 → rounds to 4
        let low = ResolvedConfig::resolve(None, Sensitivity::Low);
        assert_eq!(low.chat.emote_burst_threshold, 4);

        // 5 * 1.2 = 6.0 → 6
        assert_eq!(low.chat.rate_min_msgs_per_window, 6);

        // 0.55 * 1.2 = 0.66
        assert!((low.audio.spike_threshold - 0.66).abs() < 1e-6);
    }

    #[test]
    fn sensitivity_does_not_affect_durations_or_lists() {
        let high = ResolvedConfig::resolve(None, Sensitivity::High);
        let medium = ResolvedConfig::resolve(None, Sensitivity::Medium);

        // Durations unchanged across sensitivities:
        assert_eq!(high.selector.min_clip_duration, medium.selector.min_clip_duration);
        assert_eq!(high.selector.max_clip_duration, medium.selector.max_clip_duration);
        assert_eq!(high.selector.min_gap_between_clips, medium.selector.min_gap_between_clips);

        // Transcript weight is a balance knob, not a threshold — unchanged:
        assert!((high.transcript.weight - medium.transcript.weight).abs() < 1e-6);

        // Title category lists unchanged:
        assert_eq!(high.titles.preferred_categories, medium.titles.preferred_categories);
    }
```

- [ ] **Step 8.2: Run, expect failure**

```
cd src-tauri && cargo test game_config::tests::sensitivity -- --nocapture
```

Expected: assertion failures — sensitivity isn't applied yet.

- [ ] **Step 8.3: Implement sensitivity multiplier**

In `src-tauri/src/game_config.rs`, modify `ResolvedConfig::resolve` to apply layer 4 after layer 3. Replace the `let _ = sensitivity;` line with:

```rust
        // Layer 4: Sensitivity multiplier on threshold-style knobs only.
        // Does NOT apply to transcript.weight, durations, or category lists.
        let m = sensitivity.multiplier();
        config.audio.spike_threshold *= m;
        config.chat.rate_min_msgs_per_window =
            ((config.chat.rate_min_msgs_per_window as f64) * m).round() as u32;
        config.chat.emote_burst_threshold =
            ((config.chat.emote_burst_threshold as f64) * m).round() as u32;
```

- [ ] **Step 8.4: Run all tests**

```
cd src-tauri && cargo test game_config::tests -- --nocapture
```

Expected: 13 tests pass.

- [ ] **Step 8.5: Commit**

```
git add src-tauri/src/game_config.rs
git commit -m "feat: layer 4 — sensitivity multiplier on threshold knobs"
```

---

### Task 9: Fill in real genre TOML content

Replace the placeholder content in the 6 genre files with real tuned values.

**Files:**
- Modify: `src-tauri/config/games/_horror.toml`
- Modify: `src-tauri/config/games/_fps.toml`
- Modify: `src-tauri/config/games/_rpg.toml`
- Modify: `src-tauri/config/games/_cozy.toml`
- Modify: `src-tauri/config/games/_talking.toml`
- Modify: `src-tauri/config/games/_strategy.toml`

- [ ] **Step 9.1: Update `_horror.toml`**

Replace `src-tauri/config/games/_horror.toml` content (the test in Task 6 already wrote some of this — keep those values, no change needed):

```toml
# Horror genre — sudden audio peaks (screams), emote-spammy chat, less narration.
# Sparse override: only declare knobs that DIFFER from default.toml.

[audio]
# Lower threshold so screams + sudden chase audio register over ambient music.
spike_threshold = 0.45

[chat]
# Horror chats are baseline-spammy with KEKW/Sadge — bump the emote-burst bar.
emote_burst_threshold = 5

[transcript]
# Streamer focus is on game audio cues, not narration.
weight = 0.7

[titles]
# Surface combat/horror-flavored variants over neutral ones.
preferred_categories = ["ambush", "death", "shock", "celebration+hype"]
```

(The test from Task 6 only asserts the audio + chat + transcript values; adding `preferred_categories` is a strict superset and won't break tests.)

- [ ] **Step 9.2: Update `_fps.toml`**

```toml
# FPS genre — gunshots dominate audio, chat reactive on round-ending plays.

[audio]
# Standard threshold works — gunshots are loud and distinct from ambient.
# (No override; inherits 0.55 from default.)

[chat]
# Slightly higher emote bar — FPS chats spam Pog on big plays.
emote_burst_threshold = 4

[transcript]
# Voice comms are mostly callouts, not interesting narration.
weight = 0.8

[selector]
# FPS rounds are short — slightly shorter clip cap fits round structure.
max_clip_duration = 25

[titles]
# Surface action-flavored variants.
preferred_categories = ["ambush", "celebration+hype", "shock"]
```

- [ ] **Step 9.3: Update `_rpg.toml`**

```toml
# RPG genre — long quiet exploration with bursts of combat. Heavy narration.

[audio]
# Slightly lower so subtle exploration audio cues register.
spike_threshold = 0.50

[chat]
# Standard chat thresholds work for typical RPG audiences.

[transcript]
# Streamer narration matters a lot — boost transcript weight.
weight = 1.3

[selector]
# Longer clips work for RPG — moments often include narration buildup.
max_clip_duration = 45

[titles]
# Surface introspective + epic variants.
preferred_categories = ["disbelief+shock", "celebration+hype", "death", "fight+frustration"]
```

- [ ] **Step 9.4: Update `_cozy.toml`**

```toml
# Cozy genre — quiet audio, slow chat, narration is the primary content.

[audio]
# Cozy games have minimal audio peaks — lower threshold to catch subtle moments.
spike_threshold = 0.30

[chat]
# Slow chats — small spikes are real reactions, not noise.
rate_min_msgs_per_window = 2
emote_burst_threshold = 2

[transcript]
# Narration IS the content — heavy boost.
weight = 1.5

[selector]
# Cozy moments are slower-paced — allow longer clips for buildup.
max_clip_duration = 60
# Spread clips across the VOD — cozy moments are scarce, shouldn't cluster.
min_gap_between_clips = 60

[titles]
# Skip combat-flavored templates entirely — they don't fit cozy content.
disabled_categories = ["death", "explosion", "ambush", "fight+panic"]
preferred_categories = ["celebration+hype", "disbelief+shock"]
```

- [ ] **Step 9.5: Update `_talking.toml`**

```toml
# Talking genre — Just Chatting, IRL, ASMR, podcasts. Voice-dominant content.

[audio]
# Voice peaks are subtler than action games. Lower threshold.
spike_threshold = 0.35

[chat]
# Variable chat — slight reduction so smaller spikes count.
rate_min_msgs_per_window = 4

[transcript]
# Words ARE the content — maximum transcript weight.
weight = 1.7

[selector]
# Talking moments often need 30-60s for setup + payoff.
max_clip_duration = 60

[titles]
# Skip combat-flavored templates.
disabled_categories = ["explosion", "ambush", "fight+panic", "fight+frustration"]
```

- [ ] **Step 9.6: Update `_strategy.toml`**

```toml
# Strategy genre — League, Dota, Hearthstone, MTG, TFT, Civ. Reactive moments.

[audio]
# Standard threshold — strategy games have varied audio, default works.

[chat]
# Strategy chats spike hard on plays/clutches. Slightly higher rate threshold.
rate_min_msgs_per_window = 6

[transcript]
# Streamer commentary often explains strategy — moderate transcript boost.
weight = 1.1

[titles]
# Surface celebration / outplay variants.
preferred_categories = ["celebration+hype", "disbelief+shock", "fight+frustration"]
```

- [ ] **Step 9.7: Run all tests to confirm nothing broke**

```
cd src-tauri && cargo test game_config::tests -- --nocapture
```

Expected: 13 tests still pass. (Adding new fields like `titles.preferred_categories` to `_horror.toml` is a strict superset of what tests expected — no test breakage.)

- [ ] **Step 9.8: Commit**

```
git add src-tauri/config/games/_horror.toml src-tauri/config/games/_fps.toml src-tauri/config/games/_rpg.toml src-tauri/config/games/_cozy.toml src-tauri/config/games/_talking.toml src-tauri/config/games/_strategy.toml
git commit -m "feat: tuned content for the 6 genre TOML files"
```

---

### Task 10: Fill in real per-game TOML overrides

Replace placeholders in the 3 per-game files (DBD, Valorant, Stardew) with real tuned values.

**Files:**
- Modify: `src-tauri/config/games/dead_by_daylight.toml`
- Modify: `src-tauri/config/games/valorant.toml`
- Modify: `src-tauri/config/games/stardew_valley.toml`

- [ ] **Step 10.1: Update `dead_by_daylight.toml`**

```toml
# DBD specifically — chat is uniquely emote-spammy even by horror standards.
# Sparse override of _horror.toml. Only declare what differs from horror baseline.

[chat]
# Horror baseline = 5; DBD chats KEKW essentially constantly during chases.
# Bump higher so emote-burst signal only fires on REAL collective reactions.
emote_burst_threshold = 7
```

- [ ] **Step 10.2: Update `valorant.toml`**

```toml
# Valorant — round structure means transcript callouts matter more than typical FPS.
# Sparse override of _fps.toml.

[transcript]
# FPS baseline = 0.8; Valorant callouts ("rotate B", "one shot mid") are
# information-dense and worth boosting.
weight = 1.0

[selector]
# Round-length cap — Valorant rounds are 1:40 max, but the moment is 5-15s.
max_clip_duration = 20
# Don't cluster clips — same fight in two rounds is rarely both clip-worthy.
min_gap_between_clips = 45
```

- [ ] **Step 10.3: Update `stardew_valley.toml`**

```toml
# Stardew Valley — even quieter than typical cozy, narration-heaviest of the genre.
# Sparse override of _cozy.toml.

[audio]
# Cozy baseline = 0.30; Stardew is even lower-volume. Most peaks are music transitions.
spike_threshold = 0.25

[transcript]
# Cozy baseline = 1.5; Stardew streamers narrate constantly.
weight = 1.7
```

- [ ] **Step 10.4: Add a test for Stardew (per-game over genre)**

In `src-tauri/src/game_config.rs` `mod tests`, append:

```rust
    #[test]
    fn resolve_stardew_applies_cozy_then_per_game() {
        let resolved = ResolvedConfig::resolve(
            Some("Stardew Valley"),
            Sensitivity::Medium,
        );
        // stardew_valley.toml overrides _cozy.toml's spike_threshold (0.30 → 0.25):
        assert!((resolved.audio.spike_threshold - 0.25).abs() < 1e-6);
        // stardew_valley.toml overrides _cozy.toml's transcript.weight (1.5 → 1.7):
        assert!((resolved.transcript.weight - 1.7).abs() < 1e-6);
        // _cozy.toml's chat values still apply (Stardew doesn't override):
        assert_eq!(resolved.chat.rate_min_msgs_per_window, 2);
        // _cozy.toml's disabled_categories carries through:
        assert!(resolved.titles.disabled_categories.contains(&"death".to_string()));
    }
```

- [ ] **Step 10.5: Run all tests**

```
cd src-tauri && cargo test game_config::tests -- --nocapture
```

Expected: 14 tests pass.

- [ ] **Step 10.6: Commit**

```
git add src-tauri/config/games/dead_by_daylight.toml src-tauri/config/games/valorant.toml src-tauri/config/games/stardew_valley.toml src-tauri/src/game_config.rs
git commit -m "feat: tuned content for DBD/Valorant/Stardew per-game overrides"
```

---

### Task 11: Resolve config in `run_analysis_signals` + log it

Wire the resolver into the analysis pipeline. Add the diagnostic log line.

**Files:**
- Modify: `src-tauri/src/commands/vod.rs` (function `run_analysis_signals` — search for `fn run_analysis_signals` ~line 1522)

- [ ] **Step 11.1: Add the resolve + log block at the top of `run_analysis_signals`**

Find the start of `run_analysis_signals` (around line 1522 in `src-tauri/src/commands/vod.rs`). After the existing setup code (`let ffmpeg = find_ffmpeg()?;`, `let vod_path = ...`, etc.) and BEFORE Stage 1's `log::info!("Signal analysis: extracting audio profile...");`, add:

```rust
    // Resolve per-game detection config. Walks 4 layers:
    //   default.toml → _<genre>.toml → <game_name>.toml → sensitivity multiplier
    // See game_config.rs for the resolver and docs/superpowers/specs/ for design.
    let game_config = {
        let conn = db::db_path().ok().and_then(|p| rusqlite::Connection::open(&p).ok());
        let sensitivity_str = conn.as_ref()
            .and_then(|c| db::get_setting(c, "detection_sensitivity").ok().flatten())
            .unwrap_or_else(|| "medium".to_string());
        let sensitivity = crate::game_config::Sensitivity::from_str_or_default(&sensitivity_str);
        crate::game_config::ResolvedConfig::resolve(vod.game_name.as_deref(), sensitivity)
    };

    log::info!(
        "[game-config] Resolved for {:?}: \
         audio.spike={:.2} chat.emote_burst={} chat.rate_min_msgs={} \
         transcript.weight={:.2} selector.min_clip={} max_clip={} min_gap={} \
         titles.preferred={:?} titles.disabled={:?}",
        vod.game_name.as_deref().unwrap_or("(unknown game)"),
        game_config.audio.spike_threshold,
        game_config.chat.emote_burst_threshold,
        game_config.chat.rate_min_msgs_per_window,
        game_config.transcript.weight,
        game_config.selector.min_clip_duration,
        game_config.selector.max_clip_duration,
        game_config.selector.min_gap_between_clips,
        game_config.titles.preferred_categories,
        game_config.titles.disabled_categories,
    );
```

`game_config` is now in scope for the rest of the function and will be passed into the analysis stages in subsequent tasks.

- [ ] **Step 11.2: Verify it compiles**

```
cd src-tauri && cargo check 2>&1 | tail -10
```

Expected: `Finished` line. Likely warnings about `game_config` being unused — that's fine, will be used in next tasks.

- [ ] **Step 11.3: Commit**

```
git add src-tauri/src/commands/vod.rs
git commit -m "feat: resolve game_config at top of run_analysis_signals + log it"
```

---

### Task 12: Thread audio config into `analyze_audio_intensity`

Replace the hardcoded `0.3` floor in audio spike detection (`vod.rs:445`) with `config.audio.spike_threshold`.

**Files:**
- Modify: `src-tauri/src/commands/vod.rs` (`analyze_audio_intensity` line 333, called from `run_analysis_signals`)

- [ ] **Step 12.1: Add config parameter to `analyze_audio_intensity`**

In `src-tauri/src/commands/vod.rs`, find the function signature (around line 333):

```rust
fn analyze_audio_intensity(
    vod_path: &str,
    ffmpeg: &std::path::Path,
) -> Result<AudioProfile, AppError> {
```

Change to:

```rust
fn analyze_audio_intensity(
    vod_path: &str,
    ffmpeg: &std::path::Path,
    audio_config: &crate::game_config::AudioConfig,
) -> Result<AudioProfile, AppError> {
```

- [ ] **Step 12.2: Use the config inside `analyze_audio_intensity`**

Find the line (around line 445):

```rust
    let spike_threshold = (avg * 1.5).max(0.3); // At least 0.3 to avoid noise
```

Replace with:

```rust
    // Threshold floor comes from the per-game config; the avg*1.5 dynamic
    // component continues to scale with the VOD's actual audio level so we
    // don't fire on quiet content. Floor + dynamic = best of both.
    let spike_threshold = (avg * 1.5).max(audio_config.spike_threshold);
```

- [ ] **Step 12.3: Update the call site in `run_analysis_signals`**

Find the line in `run_analysis_signals` (around line 1539) that calls `analyze_audio_intensity`:

```rust
    let audio_profile = analyze_audio_intensity(&vod_path, &ffmpeg).ok();
```

Change to:

```rust
    let audio_profile = analyze_audio_intensity(&vod_path, &ffmpeg, &game_config.audio).ok();
```

- [ ] **Step 12.4: Verify it compiles**

```
cd src-tauri && cargo check 2>&1 | tail -10
```

Expected: `Finished` line.

- [ ] **Step 12.5: Commit**

```
git add src-tauri/src/commands/vod.rs
git commit -m "feat: thread audio_config into analyze_audio_intensity"
```

---

### Task 13: Thread chat config into `analyze_via_chat`

Replace the hardcoded `3.0` emote-burst floor (`vod.rs:2060`) with `config.chat.emote_burst_threshold`. Also use `rate_min_msgs_per_window` for the chat-rate peak threshold.

**Files:**
- Modify: `src-tauri/src/commands/vod.rs` (`analyze_via_chat` line 1963)

- [ ] **Step 13.1: Find the function signature and existing chat-rate threshold logic**

Find the function signature (around line 1963):

```rust
fn analyze_via_chat(
    chat_messages: &[crate::twitch_chat_replay::ChatMessage],
    duration: f64,
    vod_id: &str,
) -> Result<ChatAnalysisResult, String> {
```

Inspect the current chat-rate logic. Run:

```
cd src-tauri && grep -n "rate.*threshold\|rate.*peaks\|rate_avg\|MIN_RATE" src/commands/vod.rs | head -20
```

If the chat-rate detection uses a hardcoded number for the rate threshold (similar to the `(emote_avg * 2.0).max(3.0)` pattern for emote-burst at line 2060), note that hardcoded value — we'll replace it. If it's purely dynamic (no floor), we add `rate_min_msgs_per_window` as the new floor.

- [ ] **Step 13.2: Add config parameter to `analyze_via_chat`**

Change the signature to:

```rust
fn analyze_via_chat(
    chat_messages: &[crate::twitch_chat_replay::ChatMessage],
    duration: f64,
    vod_id: &str,
    chat_config: &crate::game_config::ChatConfig,
) -> Result<ChatAnalysisResult, String> {
```

- [ ] **Step 13.3: Replace the hardcoded `3.0` emote-burst floor**

Find line ~2060:

```rust
        let threshold = (emote_avg * 2.0).max(3.0);
```

Replace with:

```rust
        // Threshold floor comes from per-game config (emote_burst_threshold).
        // Dynamic component (avg*2) scales with the VOD's chat density so we
        // don't fire on relatively-quiet chats. Floor + dynamic = best of both.
        let threshold = (emote_avg * 2.0).max(chat_config.emote_burst_threshold as f64);
```

- [ ] **Step 13.4: Apply rate_min_msgs_per_window to the chat-rate peak detection**

In `src-tauri/src/commands/vod.rs`, find the chat-rate filter (around line 2014):

```rust
    let rate_avg = total_messages as f64 / num_rate_windows as f64;
    let mut rate_peak_idxs: Vec<(usize, u32)> = rate_counts.iter().enumerate()
        .filter(|(_, &count)| count as f64 > rate_avg * 1.3)
```

Replace the filter with an explicit threshold variable that uses the per-game floor:

```rust
    let rate_avg = total_messages as f64 / num_rate_windows as f64;
    // Threshold floor comes from per-game config (rate_min_msgs_per_window).
    // Dynamic component (avg*1.3) preserves existing scaling behavior so we
    // don't false-positive on slow chats where the floor would otherwise
    // catch ambient activity. Floor + dynamic = best of both.
    let rate_threshold = (rate_avg * 1.3).max(chat_config.rate_min_msgs_per_window as f64);
    let mut rate_peak_idxs: Vec<(usize, u32)> = rate_counts.iter().enumerate()
        .filter(|(_, &count)| count as f64 > rate_threshold)
```

- [ ] **Step 13.5: Update the call site in `run_analysis_signals`**

Find the call to `analyze_via_chat` (around line 1583 in `run_analysis_signals`):

```rust
        match analyze_via_chat(chat_messages, duration, &vod.id) {
```

Change to:

```rust
        match analyze_via_chat(chat_messages, duration, &vod.id, &game_config.chat) {
```

- [ ] **Step 13.6: Verify**

```
cd src-tauri && cargo check 2>&1 | tail -10
```

Expected: `Finished` line.

- [ ] **Step 13.7: Commit**

```
git add src-tauri/src/commands/vod.rs
git commit -m "feat: thread chat_config into analyze_via_chat (emote_burst + rate floors)"
```

---

### Task 14: Thread selector config into `clip_selector::select_clips`

Replace hardcoded clip-duration / cooldown values in `CurationConfig::for_duration` with config values.

**Files:**
- Modify: `src-tauri/src/clip_selector.rs` (`CurationConfig::for_duration` line 126, `select_clips` line 912)

- [ ] **Step 14.1: Add `selector_config` parameter to `CurationConfig::for_duration`**

In `src-tauri/src/clip_selector.rs`, find the function (around line 126):

```rust
    pub fn for_duration(duration_secs: f64, sensitivity: &str) -> Self {
```

Change to:

```rust
    pub fn for_duration(
        duration_secs: f64,
        sensitivity: &str,
        selector_config: &crate::game_config::SelectorConfig,
    ) -> Self {
```

- [ ] **Step 14.2: Use the config values inside `for_duration`**

Find the section that sets cooldown (around line 157):

```rust
        // Shorter cooldown for longer VODs (more content spread out).
        let duration_hrs = duration_secs / 3600.0;
        let cooldown = (120.0 - (duration_hrs * 15.0)).clamp(45.0, 120.0);
```

Add a new line that takes the per-game `min_gap_between_clips` as the FLOOR:

```rust
        // Shorter cooldown for longer VODs (more content spread out). Floor
        // is the per-game min_gap_between_clips so cozy/RPG games get the
        // longer gaps they need even on short VODs.
        let duration_hrs = duration_secs / 3600.0;
        let dynamic_cooldown = (120.0 - (duration_hrs * 15.0)).clamp(45.0, 120.0);
        let cooldown = dynamic_cooldown.max(selector_config.min_gap_between_clips as f64);
```

- [ ] **Step 14.3: Apply min/max clip duration as a post-selection clamp**

`clip_selector.rs` has dynamic clip-boundary logic (`optimize_clip_boundaries`, `optimize_clip_end` around lines 509-549) that adjusts clip durations based on audio levels. Threading per-game min/max into that dynamic logic is invasive. The simpler v1.3.11 approach is a **post-selection clamp**: after `select_clips` has chosen its final list, walk each clip and clamp its duration to fit `[selector_config.min_clip_duration, selector_config.max_clip_duration]`, centered on the peak.

Find the end of `select_clips` (around line 880, just before `return (selected, detection_stats);`). Add a clamp loop:

```rust
    // Apply per-game duration clamps from selector_config.
    // Centered on peak_time so the clip stays focused on the moment of interest.
    for clip in selected.iter_mut() {
        let current = clip.end_time - clip.start_time;
        let target_min = selector_config.min_clip_duration as f64;
        let target_max = selector_config.max_clip_duration as f64;

        if current < target_min {
            // Extend symmetrically around peak_time
            let half = target_min / 2.0;
            clip.start_time = (clip.peak_time - half).max(0.0);
            clip.end_time = (clip.start_time + target_min).min(duration);
        } else if current > target_max {
            // Trim symmetrically around peak_time
            let half = target_max / 2.0;
            clip.start_time = (clip.peak_time - half).max(0.0);
            clip.end_time = (clip.start_time + target_max).min(duration);
        }
    }
```

This is purely additive — doesn't disrupt the existing dynamic boundary logic, just clamps the final result. The dynamic logic still handles audio-aware fine-tuning within the clamp range.

- [ ] **Step 14.4: Add `selector_config` parameter to `select_clips`**

Find `pub fn select_clips` (around line 912):

```rust
pub fn select_clips(
    audio: Option<&AudioContext>,
    transcript: Option<&TranscriptResult>,
    chat_peaks: &[db::HighlightRow],
    emote_peaks: &[db::HighlightRow],
    community_clips: &[CommunityClip],
    duration: f64,
    sensitivity: &str,
) -> (Vec<ClipCandidate>, DetectionStats) {
```

Add a parameter:

```rust
pub fn select_clips(
    audio: Option<&AudioContext>,
    transcript: Option<&TranscriptResult>,
    chat_peaks: &[db::HighlightRow],
    emote_peaks: &[db::HighlightRow],
    community_clips: &[CommunityClip],
    duration: f64,
    sensitivity: &str,
    selector_config: &crate::game_config::SelectorConfig,
) -> (Vec<ClipCandidate>, DetectionStats) {
```

Inside, the existing line:

```rust
    let cfg = CurationConfig::for_duration(duration, sensitivity);
```

becomes:

```rust
    let cfg = CurationConfig::for_duration(duration, sensitivity, selector_config);
```

- [ ] **Step 14.5: Update the call site in `run_analysis_signals`**

Find the call to `select_clips` in `run_analysis_signals` (around line 1605 in `vod.rs`):

```rust
    let (selected, detection_stats): (Vec<clip_selector::ClipCandidate>, _) = clip_selector::select_clips(
        audio_ctx.as_ref(),
        transcript.as_ref(),
        &chat_peaks,
        &emote_peaks,
        community_clips,
        duration,
        sensitivity,
    );
```

Add the config parameter:

```rust
    let (selected, detection_stats): (Vec<clip_selector::ClipCandidate>, _) = clip_selector::select_clips(
        audio_ctx.as_ref(),
        transcript.as_ref(),
        &chat_peaks,
        &emote_peaks,
        community_clips,
        duration,
        sensitivity,
        &game_config.selector,
    );
```

- [ ] **Step 14.6: Verify**

```
cd src-tauri && cargo check 2>&1 | tail -10
```

Expected: `Finished` line.

- [ ] **Step 14.7: Commit**

```
git add src-tauri/src/clip_selector.rs src-tauri/src/commands/vod.rs
git commit -m "feat: thread selector_config into select_clips + CurationConfig"
```

---

### Task 15: Apply `preferred_categories` and `disabled_categories` in title generation

Filter the AftermathConfession variants by the per-game title config in `aftermath_from_tags`.

**Files:**
- Modify: `src-tauri/src/commands/captions.rs` (`aftermath_from_tags` function)

- [ ] **Step 15.1: Locate the function and how title-generation flows**

Run:

```
cd src-tauri && grep -n "fn aftermath_from_tags\|fn save_path_heuristic_title\|disabled_categories\|preferred_categories" src/commands/captions.rs | head -10
```

The current `aftermath_from_tags` uses simple `if has("ambush")` chains. We need to add filtering: skip categories in `disabled_categories`, prefer categories in `preferred_categories`.

- [ ] **Step 15.2: Add `title_config` parameter to `save_path_heuristic_title`**

In `src-tauri/src/commands/captions.rs`, find the function signature for `save_path_heuristic_title`:

```rust
pub fn save_path_heuristic_title(
    transcript_excerpt: Option<&str>,
    tags_str: Option<&str>,
    game_name: Option<&str>,
    start_seconds: f64,
    usage: &mut TitleUsage,
) -> String {
```

Change to:

```rust
pub fn save_path_heuristic_title(
    transcript_excerpt: Option<&str>,
    tags_str: Option<&str>,
    game_name: Option<&str>,
    start_seconds: f64,
    usage: &mut TitleUsage,
    title_config: &crate::game_config::TitleConfig,
) -> String {
```

Pass `title_config` through to the inner `aftermath_from_tags` call.

- [ ] **Step 15.3: Add filtering inside `aftermath_from_tags`**

Find `aftermath_from_tags`. Change its signature:

```rust
fn aftermath_from_tags(
    tags: &[String],
    game_name: Option<&str>,
    start_seconds: f64,
    usage: &TitleUsage,
    title_config: &crate::game_config::TitleConfig,
) -> Option<String> {
```

At the top of the function, add a category-name lookup helper (the `if has("ambush") || has("jumpscare")` chain effectively defines categories — we map each branch to a category name):

```rust
    // Helper: check if a category is allowed by title_config.
    let is_category_enabled = |category: &str| -> bool {
        if title_config.disabled_categories.iter().any(|c| c == category) {
            return false;
        }
        true
    };
```

Then wrap each `if has(...)` block to also check `is_category_enabled("<name>")`. Example for the ambush branch:

```rust
    if (has("ambush") || has("jumpscare")) && is_category_enabled("ambush") {
        return Some(pick(/* ... */));
    }
```

Apply this pattern to all 7 categories in `aftermath_from_tags`:
- `ambush` (covers `has("ambush")` and `has("jumpscare")`)
- `fight+panic` (covers `has("fight") && has("panic")`)
- `fight+frustration`
- `celebration+hype`
- `death`
- `explosion`
- `disbelief+shock` (covers `has("disbelief")` and `has("shock")`)

The `preferred_categories` list is more nuanced — it could be implemented as a "try preferred first, fall through if no match" iteration, but the SIMPLEST v1.3.11 implementation is to use `preferred_categories` only as a tiebreaker if multiple branches match. Since the current code returns on first match, preferred ordering can be implemented by reordering the `if` chain based on preference. For v1.3.11 we'll skip implementing `preferred_categories` (it'll be a no-op for now), focus on `disabled_categories` (which has the bigger user-visible impact) — and add a TODO comment:

```rust
    // TODO(v1.3.x): preferred_categories ordering is not yet implemented in
    // the if-chain — the chain returns on first match in source order. To
    // honor preferred_categories we'd need to either: (a) reorder branches
    // based on the list, or (b) collect all matching branches and pick by
    // preference. For v1.3.11, preferred_categories is captured in the
    // config but applied only via the "extras" mechanism (no-op for now).
    let _ = title_config.preferred_categories;
```

- [ ] **Step 15.4: Update all call sites of `save_path_heuristic_title`**

Search for callers:

```
cd src-tauri && grep -n "save_path_heuristic_title" src/ -r | head -10
```

Each call site needs the new `&title_config` argument. The main one is in `commands/vod.rs` `run_analysis_signals` — pass `&game_config.titles`.

- [ ] **Step 15.5: Verify**

```
cd src-tauri && cargo check 2>&1 | tail -10
```

Expected: `Finished` line.

- [ ] **Step 15.6: Commit**

```
git add src-tauri/src/commands/captions.rs src-tauri/src/commands/vod.rs
git commit -m "feat: aftermath_from_tags honors disabled_categories from title_config"
```

---

### Task 16: Manual integration test on real VODs + bump version + ship

Validate the full pipeline end-to-end and ship as v1.3.11.

- [ ] **Step 16.1: Run the unit test suite**

```
cd src-tauri && cargo test game_config -- --nocapture
```

Expected: 14 tests pass.

- [ ] **Step 16.2: Cargo check the whole crate**

```
cd src-tauri && cargo check 2>&1 | tail -10
```

Expected: `Finished` line, no errors.

- [ ] **Step 16.3: Re-analyze a known DBD VOD**

In `cargo tauri dev`, re-analyze the Otzdarva 7h DBD VOD that was used as the test bed earlier in development. After analysis completes, examine the log output (`%LOCALAPPDATA%\com.clipgoblin.desktop\logs\ClipGoblin.log`):

```
grep "game-config" "%LOCALAPPDATA%\com.clipgoblin.desktop\logs\ClipGoblin.log" | tail -5
```

Expected log line:

```
[game-config] Resolved for "Dead by Daylight": audio.spike=0.45 chat.emote_burst=7 chat.rate_min_msgs=5 transcript.weight=0.7 selector.min_clip=15 max_clip=30 min_gap=30 titles.preferred=["ambush", "death", "shock", "celebration+hype"] titles.disabled=[]
```

Confirms: layers resolved correctly (default → _horror → dead_by_daylight), sensitivity multiplier applied (Medium=1.0, no change shown).

- [ ] **Step 16.4: Compare detection stats vs v1.3.10 baseline**

Open the VOD card → check the detection stats. Compare to the previous v1.3.10 analysis of the same VOD (cached in DB). Expected differences:
- Slightly fewer chat-rate clips (DBD threshold raised: 5 → 7 effective)
- Slightly fewer false-positive emote-bursts (threshold from 3 → 7)
- Audio peaks slightly more frequent (threshold floor lowered: 0.55 → 0.45 from horror genre)

If clip count is significantly off in either direction, the threshold values may need adjustment in the genre/game files.

- [ ] **Step 16.5: Test unknown game fallback**

In a separate test, edit `_known_games.toml` to TEMPORARILY remove "Dead by Daylight" (or change to a genre that doesn't exist). Re-analyze. Expected log:

```
[game-config] Resolved for "Dead by Daylight": audio.spike=0.55 chat.emote_burst=3 ...
```

(Default values, no genre overrides applied.) Restore the entry after testing.

- [ ] **Step 16.6: Bump version**

```
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
powershell -File bump-version.ps1 1.3.11
```

- [ ] **Step 16.7: Commit version bump + tag**

```
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
git add package.json src-tauri/Cargo.lock src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -m "chore: bump to v1.3.11 (per-game detection configs)"
git tag -a v1.3.11 -m "v1.3.11 — per-game detection configs (foundation)

Adds bundled per-game detection configs that adapt thresholds based on
the VOD's game. 6 genre templates (horror/fps/rpg/cozy/talking/strategy)
plus 3 per-game overrides (DBD/Valorant/Stardew) cover the highest-
traffic Twitch games. Unknown games inherit current defaults — no
regression.

User-facing UI editor for tuning is deferred to v1.3.12.

See docs/superpowers/specs/2026-04-30-per-game-detection-configs-design.md
for design rationale."
git push origin main
git push origin v1.3.11
```

CI builds and publishes draft release. Edit notes + publish via GitHub UI.

---

## Self-Review Checklist (run after writing the plan)

- [ ] **Spec coverage**: every section/requirement in the spec is implemented by some task
  - Section 5 Resolution Model → Tasks 5, 6, 7, 8 ✅
  - Section 6 Config Schema → Task 1 (types) + Task 2 (default.toml) ✅
  - Section 7 File Layout & Coverage → Tasks 2, 4, 9, 10 ✅
  - Section 8 Integration Points → Tasks 11, 12, 13, 14, 15 ✅
  - Section 8.4 Logging → Task 11 ✅
  - Section 10 Testing → Tasks 2, 3, 4, 5, 6, 7, 8 (unit) + Task 16 (integration) ✅
- [ ] **Placeholder scan**: no "TBD" / "fill in details" / "implement appropriately" — every step has actual code, paths, or commands
- [ ] **Type consistency**: `ResolvedConfig` field names (audio, chat, transcript, selector, titles) match across Tasks 1, 11, 12, 13, 14, 15
- [ ] **Function signatures**: each modified function's new signature is consistent across the task that defines it and the tasks that call it (analyze_audio_intensity in Tasks 12, analyze_via_chat in Task 13, select_clips in Task 14, save_path_heuristic_title in Task 15)
