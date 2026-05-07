# Clip Scoring Investigation — Design

**Status:** Draft for review
**Date:** 2026-05-07
**Target release(s):** Investigation infrastructure ships in a v1.3.12 sub-release; the actual scoring fix ships as v1.3.12's user-facing feature once phases C and B produce the data we need.

---

## 1. Background

After v1.3.11 shipped, user testing surfaced a precision problem: clips scored 70%+ are *not reliably* clip-worthy. Two anchor examples from a 4-hour Elden Ring VOD:

- **Boring at 70%** — "she remembered" — Slug talking solo about which build to take, calm tone, no chat reaction.
- **Good at 70%** — "cardboard box diff" — Slug + friend banter, playful tone, laughter, no chat reaction.

Both scored ~70%, so the issue is precision (false positives at the 70% bar), not recall (good clips aren't being missed). The scorer treats "speech is happening" as a uniformly-positive signal and lacks dimensions that differentiate **interactive speech** (multi-voice, laughter, energetic tone, banter pacing) from **solo monologue** (one voice, flat tone, planning words).

### Why we're investigating before fixing

Two clips is not enough to confidently redesign scoring. The pipeline has at least four stages where the problem could live:

1. **Candidate-window pre-selector** — gates which time windows even get considered
2. **Detection thresholds** (tuned in v1.3.11) — which audio/chat events become candidates
3. **Scoring** — the six dimensions
4. **The 70% display threshold** — a UI label, not a selection cutoff

Charging at scoring blind risks tuning the wrong thing for a week. This investigation diagnoses *which dimension(s) leak* before any fix lands.

## 2. Goals & success criteria

### Investigation goal
Identify which scoring dimension(s) are giving high scores to non-reactive clips, and design a targeted fix.

### Success criteria for this spec
- Phase C log data lets us see, for any selected clip, exactly which dimensions contributed how much to its total score.
- Phase B review UI lets Slug rate 15–20 clips per VOD with rating + note in under 15 minutes of his time, and produces a single JSON paste containing scores + reviews.
- Combined data points us at the leaky dimension(s) without guesswork.

### Out of scope for this spec
- The actual scoring fix (Phase A — separate design once we have the data).
- Multi-voice / laughter / speaker-diarization signal addition (only proposed if data confirms it's needed).
- User-facing review affordances for end users.

## 3. Phase C — Scoring instrumentation

### Log format

One `log::info!` line per *selected* clip (not per candidate — selected only, to keep the log readable):

```
[scoring] [9255s..9285s] total=72% | hook=80% emotion=75% payoff=68% align=65% context=70% replay=72% | sources=[audio,transcript] | tags=[think,solo] | excerpt="should I go strength or dex this run"
```

Fields:
- `[start..end]` — clip time range in VOD seconds
- `total` — final score (matches what the UI displays)
- Six dimension breakdown — `hook` (`hook_strength`), `emotion` (`emotional_spike`), `payoff` (`payoff_clarity`), `align` (`event_reaction_alignment`), `context` (`context_simplicity`), `replay` (`replay_value`)
- `sources` — which detection signals fired (audio, chat-rate, chat-emote, transcript-keyword, community-clip)
- `tags` — event/emotion tags assigned to the clip
- `excerpt` — short transcript snippet so a log line can be matched back to a UI clip without timestamp arithmetic

### Where it lives

Single new `log::info!` call in `src-tauri/src/commands/vod.rs` at the point each `ClipCandidate` is converted into a `HighlightRow` for persistence (around line 1847, where `highlights.push(db::HighlightRow { ... })` is built). This co-locates the log with the DB write that stores the same dimension data, so live debugging and persisted data agree. The log shows the post-keyword-boost `virality_score` to match what the UI displays.

### Data availability check

The dimension scores stored on `ClipCandidate` (`hook_strength`, `emotional_spike`, etc.) must be set at the point we log. A pre-implementation pass verifies this. If any dimension is computed lazily after selection, we either eagerly compute it before the log point or print `n/a` rather than fudge a number.

### Cost
~1–2 hours implementation. ~15–20 lines added. No behavior changes.

## 4. Review UI

### 4.1 Data model

Add two columns to the `highlights` table (the table that stores per-clip detection metadata):

```sql
ALTER TABLE highlights ADD COLUMN review_rating TEXT;     -- 'good' | 'meh' | 'boring' | NULL
ALTER TABLE highlights ADD COLUMN review_note TEXT;       -- free-form, NULL
```

Both nullable. Existing rows on migration get NULL for both. No constraints on `review_rating` enum values at the DB layer (validation happens in the Tauri command).

### 4.2 Backend command

```rust
// src-tauri/src/commands/clip.rs (or wherever clip-CRUD lives today)
#[tauri::command]
pub fn save_clip_review(
    clip_id: String,
    rating: Option<String>,    // None = clear rating
    note: Option<String>,      // None = clear note
    db: State<DbConn>,
) -> Result<(), String> { ... }
```

Validates `rating` is one of `good` / `meh` / `boring` / `null`. Writes both columns in one transaction.

### 4.3 Frontend — Settings toggle

New row on the Settings page under a "Developer tools" section:
- Toggle: **"Show clip review tools"** — default **OFF**
- When OFF, no review UI is visible anywhere in the app
- When ON, the review controls are always visible on each clip card (no collapsed state, fastest workflow for back-to-back review)

State stored as a setting in the app's existing settings table.

### 4.4 Frontend — Clip card additions (when toggle is ON)

On each clip card on the Clips page:
- Three small rating buttons inline: `✓ Good` / `— Meh` / `✗ Boring`. Currently-selected rating is highlighted. Click again to clear.
- A small textarea labeled "Notes" below the buttons, single-line height, expands when focused. Saves on blur.
- A small badge on the card itself showing the current rating (color-coded) so during scrub-through, already-reviewed clips are visually distinct.

### 4.5 Frontend — Export button (when toggle is ON)

On each completed VOD card on the Vods page:
- New button: **"Export review data"**
- On click, assembles a single JSON blob and copies it to the clipboard
- Shows a toast: "Review data copied — paste in chat"

The JSON combines three sources:

```json
{
  "vod": {
    "id": "abc123",
    "title": "Elden Ring 4h boss grind",
    "game_name": "ELDEN RING",
    "duration_seconds": 14400
  },
  "config_resolved": {
    "audio_spike_threshold": 0.50,
    "chat_emote_burst_threshold": 3,
    "chat_rate_min_msgs_per_window": 5,
    "transcript_weight": 1.3,
    "selector_min_clip_duration": 15,
    "selector_max_clip_duration": 45,
    "selector_min_gap_between_clips": 30,
    "titles_preferred": ["disbelief+shock", "celebration+hype", "death", "fight+frustration"],
    "titles_disabled": []
  },
  "clips": [
    {
      "clip_id": "xyz789",
      "start_seconds": 9255,
      "end_seconds": 9285,
      "duration_seconds": 30,
      "total_score": 0.72,
      "dimensions": {
        "hook_strength": 0.80,
        "emotional_spike": 0.75,
        "payoff_clarity": 0.68,
        "event_reaction_alignment": 0.65,
        "context_simplicity": 0.70,
        "replay_value": 0.72
      },
      "signal_sources": ["audio", "transcript"],
      "tags": ["think", "solo"],
      "transcript_excerpt": "should I go strength or dex this run",
      "title": "she remembered",
      "review_rating": "boring",
      "review_note": "just talking to myself about builds, no reaction"
    }
  ]
}
```

The `dimensions` and `signal_sources` fields come from new columns on the `highlights` table populated at scoring time (Phase C also writes these directly to DB, not just to the log). The review fields come from the new `review_rating` / `review_note` columns. Reading from DB instead of parsing the log file keeps the export reliable and removes a class of edge cases (log rotation, format drift, missing files).

### 4.6 Visibility / shipping discipline

- Settings toggle defaults OFF, so production users never see review UI without explicit opt-in.
- Toggle persists across app launches.
- The review UI stays in the codebase long-term — reusable for future v1.3.13+ tuning passes. Cost is one Settings row + two DB columns + one bundle of frontend code, all hidden behind the toggle.

### 4.7 Edge cases & risks

- **VOD re-analysis erases ratings.** Re-analyzing a VOD generates new clips with new IDs; old `review_rating` / `review_note` orphan. Acceptable — re-analysis is rare, and forcing a fresh review after re-analysis is probably the right behavior anyway.
- **Clips analyzed before this feature shipped.** Existing highlights have `scoring_dimensions = NULL` because the columns didn't exist when they were inserted. Export still produces JSON for those clips but with `dimensions: null` and `signal_sources: null`. Re-analyzing the VOD populates the data. No log-parsing edge cases needed.
- **Note textarea XSS.** The note is stored as plain text and rendered as plain text only (never as HTML). Standard React rendering already handles this.
- **Concurrent edits.** Single-user dev mode, no concurrency concerns.

### Cost
~1 day total split across: DB migration (small), backend command + log parser (medium — log parser is the chunky part), frontend UI on clip cards (medium), export button + JSON assembly (small), settings toggle (tiny).

## 5. Phasing

**Sub-release 1 — Investigation infrastructure** (this spec)
1. Implement Phase C instrumentation
2. Implement review UI (DB + backend + frontend + export)
3. Slug pulls, enables the Settings toggle, re-analyzes one VOD
4. Slug reviews 15–20 clips, hits Export, pastes the JSON to chat

**Sub-release 2 — Phase A scoring fix** (separate spec, written after we see the data)
1. Identify the leaky dimension(s) from JSON analysis
2. Design the targeted fix
3. Implement, test, ship as v1.3.12 user-facing feature

We do *not* commit to a specific fix in this spec. The point of phases C+B is to make that decision evidence-driven.

## 6. Sparring-partner watchouts

- **Multi-voice / laughter detection might end up out of scope.** If C+B confirm that "interactive vs solo monologue" is the differentiator, the cleanest fix may require speaker diarization or laughter classification — both of which are multi-day audio-classifier projects, not weight tweaks. Better to know the cost early than discover it mid-implementation.
- **The "70% bar" might just be a UI-labeling problem.** If the dimension breakdown shows that boring clips genuinely score similarly to good clips because the *ranker is correct on a relative basis* and the absolute 70% number is just a confusing UI presentation, the fix may be to rebucket what users see (e.g. only show clips above a higher cutoff, or show a different label). That's a much smaller change than touching the scoring math.
- **Per-VOD variance.** A single VOD's data is a starting point, not a confidence interval. After Sub-release 1's first review pass we'll likely want a second VOD reviewed before locking in the fix. Plan accordingly.
- **Don't gate the fix on solving "boring" perfectly.** Some moments are subjectively boring even with perfect signals — that's a content quality issue, not a scoring issue. The bar for v1.3.12 success is "noticeably fewer false positives at the 70%+ bar," not "every 70%+ clip is great."

## 7. Open questions

None at the time of writing. All design decisions resolved during brainstorming.
