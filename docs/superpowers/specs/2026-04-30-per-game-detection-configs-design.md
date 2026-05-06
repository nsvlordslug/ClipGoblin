# Per-Game Detection Configs — Design

**Date:** 2026-04-30
**Status:** Approved (brainstormed in collaboration with Slug)
**Author:** Claude
**Targets:** v1.3.11 (foundation) and v1.3.12 (user UI editor)

---

## 1. Overview

ClipGoblin's clip detection currently uses a single set of hardcoded thresholds for every VOD. A horror game with constant emote-spammy chat, a quiet cozy game with sparse chat, and a competitive FPS with predictable round structure all run through the same detection knobs — producing acceptable but not optimal clip selection on each.

**Per-game detection configs** introduce a layered configuration system that automatically selects appropriate detection thresholds based on the VOD's game (provided by Twitch's API and stored in `vod.game_name`). Different games and game genres each get tuned signal sensitivities and clip-shaping parameters.

This is split into two phased releases:

- **v1.3.11 (this design's primary target):** Backend foundation. Configs are bundled with the app, applied automatically based on game detection, but not user-editable. Ships the bulk of detection-quality improvement.
- **v1.3.12 (described but not implemented here):** User-facing UI editor in Settings. Sliders for each knob, mouseover tooltips, reset-to-defaults. Adds power-user customization on top of the foundation.

---

## 2. Problem Statement

Different games produce categorically different signal patterns:

| Game type | Audio profile | Chat profile | Transcript value | Action density |
|---|---|---|---|---|
| **DBD / horror** | Sustained ambient + sudden screams | Constantly emote-spammy (KEKW/Sadge baseline) | Sporadic, mostly reactions | Bursty (chase → quiet → chase) |
| **Elden Ring / RPG** | Combat audio + boss music | Reactive on big moments | Heavy narration/lore | Quiet exploration → big fight → quiet |
| **Valorant / FPS** | Gunshots + voice comms | Tight per-round bursts | Game callouts only | Predictable round structure |
| **Stardew Valley / cozy** | Mostly ambient music | Slow, low-emote | Heavy narration is the content | Effectively continuous, no peaks |
| **Just Chatting** | Voice-dominant | Variable | Heavy | No clear "events" |

Currently, ClipGoblin uses universal hardcoded thresholds for all of these. This produces real failure modes:

- A DBD VOD with chat normally at 8 msgs/sec triggers chat-rate peaks on every chat spike → over-detection, too many clips, false positives
- A Stardew Valley VOD with chat at 0.5 msgs/sec barely produces any chat candidates → under-detection, misses real reactions
- An Elden Ring VOD where the streamer narrates for 5 minutes between fights misses transcript-only highlights because audio/chat signals don't fire there
- Cozy VODs get clip titles like "Ambushed before I could move" because the title categories don't filter out combat-flavored variants

A single set of thresholds cannot be optimal for all of these. Per-game tuning addresses this without requiring per-user customization or ML.

---

## 3. Goals

- **Quality:** Improve clip selection accuracy on the top ~30 most-streamed games on Twitch.
- **Backwards compatibility:** Unknown games continue to behave exactly as today (no regression).
- **Maintainability:** Adding/tuning a new game in a future release should require editing a small TOML file and shipping a patch — no Rust changes.
- **Scalability:** Cover thousands of games via genre fallback, not by maintaining one config per game.
- **Phaseable:** v1.3.11 ships the foundation (bundled configs, automatic application). v1.3.12 adds user customization on top without architectural rework.

## 3.1 Non-goals (v1.3.11)

- User-editable configs (deferred to v1.3.12)
- Per-VOD overrides (deferred to v1.3.12+)
- Auto-tuning from telemetry (out of scope)
- Replacing the existing global Low/Medium/High sensitivity setting (it stays, multiplies on top of per-game configs)

---

## 4. Design Decisions

The design was brainstormed by walking through four key design questions. Decisions:

| # | Question | Decision | Rationale |
|---|---|---|---|
| 1 | Per-game vs per-genre? | **Hybrid: genre templates + per-game overrides** | Genre files cover the long tail via mapping; per-game files override only when a specific game deviates notably from its genre. ~10 files cover thousands of games. |
| 2 | User-editable or bundled-only? | **Bundled-only for v1.3.11; UI editor in v1.3.12** | Phasing keeps v1.3.11 scope manageable. UI editor (sliders, tooltips, reset-to-defaults) lands in v1.3.12 as a focused follow-up. |
| 3 | Sensitivity setting interaction? | **Sensitivity multiplies per-game thresholds** | Preserves existing user-facing UX. Per-game sets the baseline; sensitivity slider adjusts up/down from there. |
| 4 | Unknown game handling? | **Strict match → default.toml fallback** | Unknown games behave exactly as today (no regression). Adding a game later is a simple TOML edit. Avoids fragile keyword inference. |

---

## 5. Resolution Model

The detection thresholds are resolved through **four ordered layers**, each one optionally overriding the previous:

```
┌─────────────────────────────────────────────────────────────────┐
│  Layer 1: default.toml                                          │
│  Universal baseline. ALL knobs defined here.                    │
│  Used as fallback for anything not covered by later layers.     │
└─────────────────────────────────────────────────────────────────┘
                             ↓ (sparse override)
┌─────────────────────────────────────────────────────────────────┐
│  Layer 2: _<genre>.toml (e.g., _horror.toml)                    │
│  Selected by looking up vod.game_name in _known_games.toml.     │
│  Only specifies knobs that DIFFER from default. Inherits rest.  │
└─────────────────────────────────────────────────────────────────┘
                             ↓ (sparse override)
┌─────────────────────────────────────────────────────────────────┐
│  Layer 3: <game_name>.toml (e.g., dead_by_daylight.toml)        │
│  Optional. Only exists for games that deviate notably from      │
│  their genre baseline. Sparse — only knobs that differ.         │
└─────────────────────────────────────────────────────────────────┘
                             ↓ (multiplier)
┌─────────────────────────────────────────────────────────────────┐
│  Layer 4: User sensitivity setting (Low / Medium / High)        │
│  Applied as a multiplier to the resolved threshold values.      │
│  Low = ×1.2 (higher thresholds = fewer clips)                   │
│  Medium = ×1.0 (no change)                                      │
│  High = ×0.8 (lower thresholds = more clips)                    │
└─────────────────────────────────────────────────────────────────┘
```

**Sparse override** means each layer file only contains the knobs that differ from the layer below. Files stay small and focused. Adding a new game is often just 2-3 lines of TOML overriding 1-2 specific knobs.

### 5.1 Sensitivity multiplier exclusions

The Low/Medium/High multiplier applies to **threshold-style knobs only**:
- `audio.spike_threshold` (multiplied)
- `chat.rate_min_msgs_per_window` (multiplied)
- `chat.emote_burst_threshold` (multiplied)

The multiplier does NOT apply to:
- `transcript.weight` — already a balance knob, not a threshold
- `selector.min_clip_duration` / `max_clip_duration` / `min_gap_between_clips` — physical clip properties
- `titles.preferred_categories` / `disabled_categories` — categorical, not numeric

### 5.2 Worked examples

**Example 1: DBD VOD on Medium sensitivity**

```
default.toml:           chat.emote_burst_threshold = 3
_horror.toml:           chat.emote_burst_threshold = 5     (overrides default)
dead_by_daylight.toml:  chat.emote_burst_threshold = 7     (overrides genre)
sensitivity = Medium → multiplier = 1.0

Final: 7 × 1.0 = 7 emotes per 10s window
```

**Example 2: Same DBD VOD on High sensitivity**

```
Same resolution through layers 1-3 → 7
sensitivity = High → multiplier = 0.8

Final: 7 × 0.8 = 5.6 emotes per 10s window (more sensitive)
```

**Example 3: An RPG game we haven't tuned yet**

```
default.toml:           chat.emote_burst_threshold = 3
_known_games.toml maps "Some Indie RPG" → genre = "rpg"
_rpg.toml:              chat.emote_burst_threshold = 4 (override)
No per-game file for "Some Indie RPG"
sensitivity = Medium → multiplier = 1.0

Final: 4 × 1.0 = 4 emotes per 10s window
```

**Example 4: A game NOT in _known_games.toml**

```
default.toml:           chat.emote_burst_threshold = 3
No matching genre, no per-game file
sensitivity = Medium → multiplier = 1.0

Final: 3 × 1.0 = 3 (current/default behavior — no regression)
```

---

## 6. Config Schema

For v1.3.11 we expose **9 knobs across 5 categories**. These cover the levers where genre/game tuning produces visibly different clip output. Additional knobs can be added in later releases.

### 6.1 Knobs

#### `audio.spike_threshold` (float, 0.0–1.0)

RMS audio level at which a moment is considered an audio peak. Lower = more sensitive (more clips from audio).

- Default: `0.55`
- Multiplied by sensitivity: ✅
- Examples: `0.45` (horror, catch screams), `0.30` (cozy, subtle peaks)

#### `chat.rate_min_msgs_per_window` (int)

Minimum chat messages in a 30-second window to count as a chat-rate peak.

- Default: `5`
- Multiplied by sensitivity: ✅
- Examples: `8` (DBD/big streamers — only count real spikes), `2` (cozy/small streamers)

#### `chat.emote_burst_threshold` (int)

Minimum emote occurrences in a 10-second window to count as an emote burst.

- Default: `3`
- Multiplied by sensitivity: ✅
- Examples: `7` (DBD), `5` (horror genre baseline), `2` (cozy)

#### `transcript.weight` (float, 0.0–2.0)

Multiplier applied to transcript signal contribution during clip scoring. Higher = transcript-derived candidates matter more.

- Default: `1.0`
- Multiplied by sensitivity: ❌
- Examples: `1.5` (cozy/talking — narration is content), `0.7` (DBD/competitive)

#### `selector.min_clip_duration` (int, seconds)

Minimum duration of a final selected clip.

- Default: `15`
- Multiplied by sensitivity: ❌
- Examples: `20` (RPG/cozy — slower-paced moments), `10` (FPS — quick moments)

#### `selector.max_clip_duration` (int, seconds)

Maximum duration of a final selected clip.

- Default: `30`
- Multiplied by sensitivity: ❌
- Examples: `60` (cozy/RPG — buildup + payoff), `25` (FPS — round-length cap)

#### `selector.min_gap_between_clips` (int, seconds)

Minimum seconds between two selected clips.

- Default: `30`
- Multiplied by sensitivity: ❌
- Examples: `60` (cozy — moments are scarce), `15` (FPS — round-by-round)

#### `titles.preferred_categories` (list of strings)

AftermathConfession categories favored when picking title variants. Empty list = all categories equally weighted.

- Default: `[]`
- Categories: `ambush`, `fight+panic`, `fight+frustration`, `celebration+hype`, `death`, `explosion`, `disbelief+shock`
- Examples: `["ambush", "death", "shock"]` (DBD/horror)

#### `titles.disabled_categories` (list of strings)

Categories to skip entirely when picking title variants.

- Default: `[]`
- Examples: `["explosion", "death", "ambush", "fight+panic"]` (cozy)

### 6.2 Default TOML file

```toml
# default.toml — universal baseline. All other config layers inherit from this
# and override only the knobs that differ.

[audio]
# RMS level (0.0-1.0) above which a moment is an audio peak.
# Lower = more sensitive (more audio-driven clips).
spike_threshold = 0.55

[chat]
# Min chat messages in a 30s window to count as a chat-rate peak.
# Lower for slow chats, higher for spammy gaming chats.
rate_min_msgs_per_window = 5

# Min emote occurrences in a 10s window to count as an emote burst.
# Higher for chats that constantly spam emotes (DBD/horror community).
emote_burst_threshold = 3

[transcript]
# Weight (0.0-2.0) applied to transcript signal during clip selection.
# 1.0 = baseline. Higher for talky/narrative games where transcript matters
# more than audio/chat (cozy games, RPGs). Lower for action games.
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

### 6.3 Knobs deferred (not in v1.3.11)

| Knob | Why deferred |
|---|---|
| Per-tier transcript keyword weights | Too granular, requires understanding internal scoring |
| Audio `min_spike_duration` | Default works fine for most content |
| Chat window sizes (30s/10s) | Defaults reliable; tuning rarely needs these |
| Profanity detection toggle | Niche use case |
| Selector distinctness/dedup threshold | Internal mechanic, hard to reason about |
| Custom title variant additions | Could add `custom_variants = [...]` later |

---

## 7. File Layout & Initial Coverage

### 7.1 Directory structure

```
src-tauri/config/games/
├── default.toml                # Layer 1 — universal baseline
├── _known_games.toml           # game_name → genre lookup table
├── _horror.toml                # Layer 2 — genre base
├── _fps.toml
├── _rpg.toml
├── _cozy.toml
├── _talking.toml
├── _strategy.toml              # MOBA / RTS / card games
├── dead_by_daylight.toml       # Layer 3 — per-game overrides
├── valorant.toml
└── stardew_valley.toml
```

**Underscore prefix** on genre files (`_horror.toml`) makes them visually distinct from per-game files when scanning the directory.

### 7.2 Initial coverage (v1.3.11)

**1 default file:**
- `default.toml` — current hardcoded values, just relocated. Catch-all fallback.

**6 genre templates:**
- `_horror.toml` — DBD, Phasmophobia, Resident Evil, Outlast, Lethal Company
- `_fps.toml` — Valorant, CS2, Apex, Fortnite, COD, Marvel Rivals
- `_rpg.toml` — Elden Ring, BG3, Cyberpunk, Skyrim, WoW
- `_cozy.toml` — Stardew Valley, Animal Crossing, Minecraft creative, Palworld
- `_talking.toml` — Just Chatting, IRL, ASMR, podcasts
- `_strategy.toml` — League, Dota, Hearthstone, MTG, TFT, Civilization

**3 per-game overrides** (only games where signals deviate notably from genre):
- `dead_by_daylight.toml` — chat is uniquely emote-spammy even by horror standards
- `valorant.toml` — round structure means transcript matters more than typical FPS (callouts)
- `stardew_valley.toml` — even quieter than typical cozy, narration-heaviest of the cozy genre

### 7.3 `_known_games.toml` initial entries

Maps Twitch's exact `game_name` strings to a genre. ~30-40 entries covering top streamed games:

```toml
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

Implementation will round this to ~40 entries based on Twitch's current top-streamed list.

### 7.4 Bundling

Config files are bundled with the app at build time using Rust's `include_str!` macro or Tauri's resource bundling. They live inside the binary, not on disk. Users never see them in v1.3.11.

For maintenance, they're plain text in the repo. Adding a game = edit a TOML file, ship a patch.

### 7.5 Maintenance flow (post-v1.3.11)

When a new game blows up on Twitch (e.g., a new release becomes #1):
1. Edit `_known_games.toml` — add `"<New Game>" = { genre = "<closest_match>" }`
2. (Optional) If observation shows the game's signals deviate from its genre, add a per-game override file
3. Bump version, ship as patch
4. Auto-update propagates to users

Discord workflow:
- User mentions a game getting weird clip results
- We watch a sample VOD, observe what's off
- Add/tune entry, push patch
- 24-hour turnaround typical

---

## 8. Integration Points

### 8.1 New file: `src-tauri/src/game_config.rs`

Exposes:

```rust
/// Resolved detection config for a single VOD analysis run.
/// Built once at the start of run_analysis_signals via ResolvedConfig::resolve.
pub struct ResolvedConfig {
    pub audio: AudioConfig,
    pub chat: ChatConfig,
    pub transcript: TranscriptConfig,
    pub selector: SelectorConfig,
    pub titles: TitleConfig,
}

pub struct AudioConfig {
    pub spike_threshold: f64,
}

pub struct ChatConfig {
    pub rate_min_msgs_per_window: u32,
    pub emote_burst_threshold: u32,
}

pub struct TranscriptConfig {
    pub weight: f64,
}

pub struct SelectorConfig {
    pub min_clip_duration: u32,
    pub max_clip_duration: u32,
    pub min_gap_between_clips: u32,
}

pub struct TitleConfig {
    pub preferred_categories: Vec<String>,
    pub disabled_categories: Vec<String>,
}

pub enum Sensitivity {
    Low,
    Medium,
    High,
}

impl ResolvedConfig {
    /// Build the final config for a VOD by walking the layer hierarchy:
    /// default.toml → genre file → game-specific file → sensitivity multiplier
    pub fn resolve(game_name: Option<&str>, sensitivity: Sensitivity) -> Self {
        // 1. Load default
        // 2. Look up game_name in _known_games to find genre
        // 3. If genre matches a file, merge it onto default
        // 4. If a per-game file exists for game_name, merge it onto genre
        // 5. Apply sensitivity multiplier to threshold knobs
        // 6. Return resolved struct
    }
}
```

### 8.2 Modified files

| File | Changes |
|---|---|
| `src-tauri/src/lib.rs` | Add `mod game_config;` |
| `src-tauri/src/commands/vod.rs` | Resolve config at top of `run_analysis_signals`. Pass to all stages by reference. |
| `src-tauri/src/clip_selector.rs` | Replace hardcoded `audio.spike_threshold`, selector duration/gap constants with `&config.audio` / `&config.selector` references |
| `src-tauri/src/commands/captions.rs` | `aftermath_from_tags` reads `config.titles.preferred_categories` and `disabled_categories` to filter and weight variants |
| `src-tauri/src/emote_signal.rs` (or wherever emote-burst threshold lives) | Threshold becomes `config.chat.emote_burst_threshold` instead of hardcoded |

### 8.3 Pipeline flow

```rust
fn run_analysis_signals(vod: &VodRow, ...) -> Result<...> {
    // NEW: resolve once at the top
    let config = ResolvedConfig::resolve(
        vod.game_name.as_deref(),
        sensitivity_from_settings(),
    );

    log_resolved_config(&config, vod);

    // Stage 1: Audio
    let audio = analyze_audio_intensity(&path, &ffmpeg, &config.audio)?;

    // Stage 2: Chat
    let (chat_peaks, emote_peaks) = analyze_via_chat(
        &messages, duration, &vod.id, &config.chat,
    );

    // Stage 3: Candidate windows
    let windows = select_candidate_windows(
        audio.as_ref(), &chat_peaks, &emote_peaks, ..., &config.selector,
    );

    // Stage 4: Transcription (no config dependency in v1.3.11)
    let transcript = run_windowed_transcription_native(...)?;

    // Stage 5: Final selector
    let (selected, stats) = select_clips(
        audio.as_ref(), transcript.as_ref(), &chat_peaks, &emote_peaks,
        community_clips, duration, &config.selector,
    );

    // Stage 6: Per-clip title generation
    for clip in selected {
        let title = save_path_heuristic_title(
            ..., &mut title_usage, &config.titles,
        );
    }
}
```

### 8.4 Logging

When config resolves, log it once per analysis at INFO level:

```
[game-config] Resolved for "Dead by Daylight":
  layers: default → _horror → dead_by_daylight (sensitivity: Medium)
  effective: audio.spike=0.45 chat.emote_burst=7 transcript.weight=0.7
            selector.min_clip=15 max_clip=30 min_gap=30
            titles.disabled=[] titles.preferred=["ambush","death","shock","celebration+hype"]
```

This single line tells us:
- Which game was detected
- Which layers applied
- The final resolved threshold values
- The user's sensitivity setting

Critical for debugging "my clips look weird on this game" reports.

---

## 9. Phasing

### 9.1 v1.3.11 (this design's primary scope, ~4.5–5 hours)

| Task | Estimated effort |
|---|---|
| Define `ResolvedConfig` Rust types + TOML schema | 30 min |
| Write resolver (layer walking + sensitivity multiplier) | 45 min |
| Unit tests for resolver | 30 min |
| Audit codebase, find all hardcoded thresholds | 30 min |
| Replace hardcoded values with config references | 1.5 hours |
| Write `default.toml` + 6 genre files + 3 game overrides + `_known_games.toml` | 1 hour |
| Add bundling configuration to Tauri build | 15 min |
| Integration testing on real VODs | 30 min |
| **Total** | **~4.5–5 hours** |

### 9.2 v1.3.12 (described, not implemented in this spec, ~5–7 hours)

UI editor in Settings → "Per-Game Tuning":
- Game picker dropdown (lists known games + "default")
- Sliders / inputs for each knob, showing current effective value
- **Mouseover tooltip on each knob** with description (sourced from TOML comments — single source of truth, auto-synced)
- **Inline subtitle** under each slider name with one-line summary
- **Direction indicator** (← fewer clips / more clips →)
- **Default value marker** on slider track
- **Reset button** per knob (in addition to global "reset all")
- User overrides written to `%APPDATA%\clipviral\config\games\<game>.toml` (overlay on bundled defaults)

### 9.3 What v1.3.11 does NOT solve

Worth being explicit so we don't over-claim:
- Niche games still get default detection (no automatic improvement)
- Users can't customize for their specific stream until v1.3.12 UI ships
- Initial genre/game thresholds are educated guesses — will need tuning based on real-world output

---

## 10. Testing

### 10.1 Unit tests (in `game_config.rs`)

- Default config loads correctly
- Genre file overrides default
- Per-game override stacks on genre + default (sparse override pattern works)
- Unknown game falls back to default cleanly
- Sensitivity multiplier applies to threshold knobs only (not transcript weight, durations, lists)
- Sparse override pattern: missing knob in genre file uses default
- Sensitivity multiplier scaling: Low/Medium/High produce correct relative values

### 10.2 Integration test (manual, on real VODs)

- Re-analyze the Otzdarva 7h DBD VOD on v1.3.11
- Compare clip selection to v1.3.10 baseline (logs + clip output)
- Expected: similar/better quality, fewer false positives from chat-rate (DBD chat is normally spammy, threshold raised)
- Edit `_known_games.toml` to test unknown-game fallback (rename "Dead by Daylight" temporarily, confirm default kicks in)
- Test with sensitivity = High, Medium, Low — confirm multiplier visibly affects clip count

### 10.3 Regression check

Re-analyze a VOD with:
- A game NOT in `_known_games.toml` (or with `game_name = None`)

Result must be identical to v1.3.10 behavior on the same VOD (same clip count, same clips selected). Confirms the default fallback path doesn't change behavior for unrecognized games.

---

## 11. Risks & Mitigation

### 11.1 Initial genre/game thresholds may be wrong

We're tuning based on intuition about game patterns, not measured data. Some genre files may produce worse clips than the current defaults until tuned.

**Mitigation:**
- Sparse override pattern means a bad genre file only affects games mapped to it
- If a genre file is making things worse, easy to revert by emptying the override
- v1.3.12 user UI ultimately lets power users tune around bad defaults
- Discord feedback loop catches problems quickly

### 11.2 New `vod.game_name` strings appear over time

Twitch may add new game directory entries that don't exactly match our list (capitalization, punctuation differences, etc.).

**Mitigation:**
- Match is exact (case-sensitive) to avoid false matches
- We'll observe and add new strings to `_known_games.toml` in patch releases
- Users on niche games still get default behavior — not a regression

### 11.3 Adding more knobs later requires careful versioning

If v1.3.13 adds a new tunable knob, all existing config files need a default for it (otherwise loaders fail or behave unexpectedly).

**Mitigation:**
- All knobs have explicit defaults in `default.toml` — files inherit from there
- Sparse override pattern means new knobs are inherited from default until explicitly tuned
- Backward-compatible: old config files keep working when new knobs are added

### 11.4 The 9 starting knobs may not cover every needed adjustment

Real-world tuning may need access to internal selector mechanics not exposed (e.g., distinctness threshold, fusion weights).

**Mitigation:**
- Easy to add more knobs in subsequent releases — just expand the `ResolvedConfig` struct
- Design accommodates additive knob expansion without breaking existing configs
- Initial 9 knobs cover the highest-impact tuning levers based on the existing pipeline architecture

---

## 12. Open Questions

None blocking implementation. Design is ready for plan creation.

For future consideration (v1.3.12+):

- Should "Set genre" be a per-VOD override option in the UI (separate from "Set game")? Useful when game name is unknown but user knows the genre.
- Should we auto-classify by game-name keyword as a fallback (the deferred Option B from Q4)? Only worth adding if observation shows a meaningful tail of unknown games where genre keywords would help.
- Custom title variant additions per game (`custom_variants = [...]`)?
- Should sensitivity Low/Medium/High be game-specific (e.g., DBD-Low / DBD-Medium / DBD-High)? Probably not — adds complexity for marginal benefit.

---

## 13. Implementation Plan

After this design is approved, the next step is invoking the `writing-plans` skill to create a detailed step-by-step implementation plan.
