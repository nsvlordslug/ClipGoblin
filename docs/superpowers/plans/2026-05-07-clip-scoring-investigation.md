# Clip Scoring Investigation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the diagnostic infrastructure that will let us identify which scoring dimension(s) are giving 70%+ scores to boring clips. Two halves: per-clip dimension data persisted to DB at scoring time, and a Settings-gated review UI that lets Slug rate clips and export combined JSON for analysis.

**Architecture:** Extend the existing `highlights` table with two new columns for raw scoring data (`scoring_dimensions` JSON, `signal_sources` JSON) and two for user reviews (`review_rating` text, `review_note` text). Populate the scoring columns when `ClipCandidate` is converted to `HighlightRow` during analysis. Add a Tauri command to save reviews and another to export combined data for a VOD. Frontend gets a Settings toggle that conditionally renders rating buttons and export functionality.

**Tech Stack:** Rust + Tauri 2 backend with `rusqlite` for SQLite migrations and queries; React + TypeScript frontend with the existing Zustand store pattern; `serde_json` for JSON serialization (already a dependency).

---

## Spec reference

This plan implements `docs/superpowers/specs/2026-05-07-clip-scoring-investigation-design.md`. Both halves (Phase C instrumentation + Review UI) ship as a single hidden-by-default feature behind a Settings toggle. The actual scoring fix that comes out of phases B + A will be a separate spec and plan.

## File structure

**Backend (Rust):**
- `src-tauri/src/db.rs` — Add 4 columns to `highlights` table via `ALTER TABLE`. Add fields to `HighlightRow`. Update `insert_highlight` and `get_highlights_by_vod`. Add helper `save_clip_review`. Add helper `get_highlight_by_id`.
- `src-tauri/src/commands/vod.rs` — At each point a `HighlightRow` is constructed from a `ClipCandidate` (line ~1847 and 3 other sites in `analyze_via_chat` etc.), populate the 2 scoring columns. Add the `[scoring]` log line at the same point.
- `src-tauri/src/commands/clip.rs` — Add 2 Tauri commands: `save_clip_review` and `export_review_data_for_vod`.
- `src-tauri/src/lib.rs` — Register the 2 new commands in the invoke handler.

**Frontend (TypeScript/React):**
- `src/pages/Settings.tsx` — Add "Show clip review tools" toggle in a new "Developer tools" section.
- `src/stores/appStore.ts` (or whichever store holds settings) — Track `showReviewTools` boolean.
- `src/pages/Clips.tsx` — When `showReviewTools` is true, render rating buttons + note textarea + colored badge on each clip card.
- `src/pages/Vods.tsx` — When `showReviewTools` is true, render "Export review data" button on each completed VOD card.
- `src/types/clipReview.ts` (NEW) — Types for `ClipReview` and `ExportReviewData`.

**Files NOT changed:**
- `clip_selector.rs` — All scoring logic stays as-is. We're observing, not changing.
- Any signal/detection/transcript code — diagnosis only.

## Pre-flight — verify type names

Before starting, the implementer should confirm these references resolve in the current code (we're working off line numbers that may shift slightly with each commit):

- `db::HighlightRow` struct in `src-tauri/src/db.rs:262` (per Read at plan-write time)
- `clip_selector::ClipCandidate` with fields `hook_strength`, `emotional_spike`, `payoff_clarity`, `event_reaction_alignment`, `context_simplicity`, `replay_value`, `signal_sources`, `event_tags`, `emotion_tags`, `transcript_excerpt`, `total_score`, `start_time`, `end_time`
- `clip_selector::SignalSource` enum — at least `Audio`, `Chat` variants present
- `db::insert_highlight` at `src-tauri/src/db.rs:627`
- `db::get_highlights_by_vod` at `src-tauri/src/db.rs:599`
- `db::save_setting` / `db::get_setting` at `src-tauri/src/db.rs:361`/`371`
- `commands/vod.rs` HighlightRow construction at line ~1847 and similar sites at 1931, 1965, 2075

If any of these aren't where the plan claims, locate them by symbol search before editing.

---

## Task 1: Add 4 new columns to the `highlights` table

**Goal:** Schema migration only. No behavior changes. Existing rows get `NULL` for all 4 new columns.

**Files:**
- Modify: `src-tauri/src/db.rs` — add 4 `ALTER TABLE` calls in the migration section (around line 117 where the existing pattern lives), add 4 fields to `HighlightRow`, update `insert_highlight` SQL + binding, update `get_highlights_by_vod` SQL + row mapping.

- [ ] **Step 1.1: Add migration calls**

In `src-tauri/src/db.rs`, find the migration block (immediately after the existing `ALTER TABLE highlights ADD COLUMN event_summary TEXT` call at line 117). Add right after it:

```rust
    // Clip scoring investigation (v1.3.12): per-clip diagnostic data populated
    // at scoring time, plus user-supplied review fields gated behind the
    // "Show clip review tools" Settings toggle. See
    // docs/superpowers/specs/2026-05-07-clip-scoring-investigation-design.md
    conn.execute("ALTER TABLE highlights ADD COLUMN scoring_dimensions TEXT", []).ok();
    conn.execute("ALTER TABLE highlights ADD COLUMN signal_sources TEXT", []).ok();
    conn.execute("ALTER TABLE highlights ADD COLUMN review_rating TEXT", []).ok();
    conn.execute("ALTER TABLE highlights ADD COLUMN review_note TEXT", []).ok();
```

The `.ok()` is the established pattern: if the column already exists from a prior run, the ALTER fails silently. SQLite has no `IF NOT EXISTS` for ALTER COLUMN.

- [ ] **Step 1.2: Add fields to `HighlightRow`**

In `src-tauri/src/db.rs` find the `pub struct HighlightRow` block (around line 262). After the existing `pub event_summary: Option<String>,` line, add:

```rust
    /// JSON-serialized 6-dimension breakdown from scoring time, e.g.
    /// `{"hook":0.80,"emotion":0.75,"payoff":0.68,"align":0.65,"context":0.70,"replay":0.72}`.
    /// `None` for highlights inserted before the v1.3.12 migration.
    pub scoring_dimensions: Option<String>,
    /// JSON-serialized array of signal-source identifiers that triggered this
    /// candidate, e.g. `["audio","chat","transcript"]`. `None` for legacy rows.
    pub signal_sources: Option<String>,
    /// User-supplied rating from the dev-only Review UI: one of `"good"`,
    /// `"meh"`, `"boring"`. `None` if unrated.
    pub review_rating: Option<String>,
    /// Free-form user note from the dev-only Review UI. `None` if no note set.
    pub review_note: Option<String>,
```

- [ ] **Step 1.3: Update `insert_highlight` SQL + bindings**

Find `pub fn insert_highlight` (around line 627). Replace the entire function with:

```rust
pub fn insert_highlight(conn: &Connection, h: &HighlightRow) -> SqliteResult<()> {
    conn.execute(
        "INSERT INTO highlights (id, vod_id, start_seconds, end_seconds, virality_score, audio_score, visual_score, chat_score, transcript_snippet, description, tags, thumbnail_path, created_at, confidence_score, explanation, event_summary, scoring_dimensions, signal_sources, review_rating, review_note)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
         ON CONFLICT(id) DO UPDATE SET
           vod_id = excluded.vod_id,
           start_seconds = excluded.start_seconds,
           end_seconds = excluded.end_seconds,
           virality_score = excluded.virality_score,
           audio_score = excluded.audio_score,
           visual_score = excluded.visual_score,
           chat_score = excluded.chat_score,
           transcript_snippet = excluded.transcript_snippet,
           description = excluded.description,
           tags = excluded.tags,
           thumbnail_path = excluded.thumbnail_path,
           confidence_score = excluded.confidence_score,
           explanation = excluded.explanation,
           event_summary = excluded.event_summary,
           scoring_dimensions = excluded.scoring_dimensions,
           signal_sources = excluded.signal_sources",
        params![h.id, h.vod_id, h.start_seconds, h.end_seconds, h.virality_score, h.audio_score, h.visual_score, h.chat_score, h.transcript_snippet, h.description, h.tags, h.thumbnail_path, h.created_at, h.confidence_score, h.explanation, h.event_summary, h.scoring_dimensions, h.signal_sources, h.review_rating, h.review_note],
    )?;
    Ok(())
}
```

**Important:** the `ON CONFLICT` clause does NOT update `review_rating` / `review_note`. That's deliberate — re-analyzing a VOD overwrites the scoring fields but preserves any reviews the user already wrote on the previous analysis run. New rows get `review_rating: None` from the implementer in Task 2.

- [ ] **Step 1.4: Update `get_highlights_by_vod` SQL + row mapping**

Find `pub fn get_highlights_by_vod` (around line 599). Replace with:

```rust
pub fn get_highlights_by_vod(conn: &Connection, vod_id: &str) -> SqliteResult<Vec<HighlightRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, vod_id, start_seconds, end_seconds, virality_score, audio_score, visual_score, chat_score, transcript_snippet, description, tags, thumbnail_path, created_at, confidence_score, explanation, event_summary, scoring_dimensions, signal_sources, review_rating, review_note
         FROM highlights WHERE vod_id = ?1 ORDER BY COALESCE(confidence_score, virality_score * 0.75 + 0.05) DESC"
    )?;
    let rows = stmt.query_map(params![vod_id], |row| {
        Ok(HighlightRow {
            id: row.get(0)?,
            vod_id: row.get(1)?,
            start_seconds: row.get(2)?,
            end_seconds: row.get(3)?,
            virality_score: row.get(4)?,
            audio_score: row.get(5)?,
            visual_score: row.get(6)?,
            chat_score: row.get(7)?,
            transcript_snippet: row.get(8)?,
            description: row.get(9)?,
            tags: row.get(10)?,
            thumbnail_path: row.get(11)?,
            created_at: row.get(12)?,
            confidence_score: row.get(13)?,
            explanation: row.get(14)?,
            event_summary: row.get(15)?,
            scoring_dimensions: row.get(16)?,
            signal_sources: row.get(17)?,
            review_rating: row.get(18)?,
            review_note: row.get(19)?,
        })
    })?;
    rows.collect()
}
```

- [ ] **Step 1.5: Static check — verify all `HighlightRow { ... }` constructions still compile**

Search for places where `HighlightRow` is constructed:

Run: `grep -rn "HighlightRow {" src-tauri/src/ | head -20`

Expected: a list of construction sites including ones in `src-tauri/src/commands/vod.rs` (lines ~1847, 1931, 1965, 2075). Each will need to add the 4 new fields. They're all currently incomplete — that's a deliberate compile failure signal for the next task.

The implementer should NOT fix those sites yet. Task 2 will populate the fields with real data; Task 3 (rating columns) is unaffected since those default to `None`.

For Task 1's commit, we need the build to be clean. Add **default-`None` field initializers** at every construction site found above, with a `// TODO Task 2:` comment. Edit each site to add:

```rust
            // ... existing fields ...
            event_summary: ...,
            scoring_dimensions: None,  // TODO Task 2: populate from ClipCandidate dimensions
            signal_sources: None,      // TODO Task 2: populate from ClipCandidate.signal_sources
            review_rating: None,       // user-set via Review UI
            review_note: None,         // user-set via Review UI
        });
```

This keeps each commit buildable and Task 2 is small and focused.

- [ ] **Step 1.6: Verify it compiles**

Run from project root: `cd src-tauri && cargo check 2>&1 | tail -10`

Expected: `Finished dev profile [unoptimized + debuginfo] target(s)` line. Warnings about pre-existing dead code are fine; errors are not.

If errors appear, the most likely cause is missing field at a `HighlightRow { ... }` construction site that grep didn't surface (maybe in a test or a comment-extraction site). Find and fix.

- [ ] **Step 1.7: Commit**

```bash
git add src-tauri/src/db.rs src-tauri/src/commands/vod.rs
git commit -m "feat(scoring): migrate highlights table for dimension + review columns"
```

Note: Task 2 will replace the `None` placeholders in `vod.rs` with real data. The commit is minimally invasive — just the schema and TODO scaffolding.

---

## Task 2: Populate `scoring_dimensions` + `signal_sources` at scoring time + log `[scoring]` line

**Goal:** Replace the `None` placeholders from Task 1 with real serialized data from `ClipCandidate`. Add a `[scoring]` log line at the same point so live debugging works.

**Files:**
- Modify: `src-tauri/src/commands/vod.rs` — at each of the 4 sites where `HighlightRow` is built from `ClipCandidate` data (the main one at line ~1847 plus chat-rate / chat-emote / community-clip sites that need to be located by grep). For the main site: serialize dimensions JSON, serialize sources JSON, populate fields, emit log line. For the secondary sites that build `HighlightRow` from chat/emote peaks (not from `ClipCandidate`): leave `scoring_dimensions: None` because those don't have dimension scores — they're intermediate signal events, not selected clips. Only the main "selected clip" site has full dimension data.

The implementer should treat each construction site individually — only the one that comes from a `ClipCandidate` (the one in the loop iterating over `selected: Vec<ClipCandidate>`) gets real data.

- [ ] **Step 2.1: Locate the main HighlightRow construction site**

Run: `grep -n "highlights.push(db::HighlightRow" src-tauri/src/commands/vod.rs`

Expected output: 4 hits at approximately lines 1847, 1931, 1965, 2075. The plan-relevant one is the FIRST hit (~1847) which lives inside the loop `for (i, c) in selected.iter().enumerate()` (visible by reading 30 lines above the hit).

Confirm by reading 5–10 lines above each hit; only the first should reference `c.start_time`, `c.hook_strength`, etc. The other three reference different local variables (rate-peak indexes, emote-peak indexes, community clips) and are out of scope.

- [ ] **Step 2.2: Add a serialization helper near the top of `vod.rs`**

Pick a location in `vod.rs` near other helpers (e.g., immediately after `fn count_active_signals`, but before `fn run_analysis_signals`). Add:

```rust
/// Serialize a ClipCandidate's six scoring dimensions to a compact JSON string
/// for storage in HighlightRow.scoring_dimensions. Round each to 4 decimal
/// places to keep the column readable at human-debug time without losing
/// meaningful precision.
fn serialize_scoring_dimensions(c: &clip_selector::ClipCandidate) -> String {
    fn r(x: f64) -> f64 { (x * 10000.0).round() / 10000.0 }
    serde_json::json!({
        "hook": r(c.hook_strength),
        "emotion": r(c.emotional_spike),
        "payoff": r(c.payoff_clarity),
        "align": r(c.event_reaction_alignment),
        "context": r(c.context_simplicity),
        "replay": r(c.replay_value),
    }).to_string()
}

/// Serialize a ClipCandidate's signal_sources Vec<SignalSource> to a JSON
/// array of lowercase string identifiers, e.g. `["audio","chat","transcript"]`.
fn serialize_signal_sources(sources: &[clip_selector::SignalSource]) -> String {
    let names: Vec<&'static str> = sources.iter().map(|s| match s {
        clip_selector::SignalSource::Audio => "audio",
        clip_selector::SignalSource::Chat => "chat",
        clip_selector::SignalSource::Emote => "emote",
        clip_selector::SignalSource::Transcript => "transcript",
        clip_selector::SignalSource::Community => "community",
    }).collect();
    serde_json::json!(names).to_string()
}
```

**Important:** the variants of `SignalSource` may not exactly match the list above. Before pasting, run:

`grep -n "pub enum SignalSource" src-tauri/src/clip_selector.rs`

Then read the enum definition and adjust the match arms to cover every variant. If a variant is missing from the match, the build will fail with an exhaustiveness error — which is the correct fail-loud behavior.

- [ ] **Step 2.3: Populate the scoring fields at the main site**

In the `HighlightRow { ... }` block at line ~1847, find the placeholder lines added in Task 1:

```rust
            scoring_dimensions: None,  // TODO Task 2: populate from ClipCandidate dimensions
            signal_sources: None,      // TODO Task 2: populate from ClipCandidate.signal_sources
```

Replace with:

```rust
            scoring_dimensions: Some(serialize_scoring_dimensions(c)),
            signal_sources: Some(serialize_signal_sources(&c.signal_sources)),
```

The other three secondary `HighlightRow { ... }` sites (lines ~1931, 1965, 2075) keep their `None` placeholders — those rows aren't selected clips, they're intermediate signal events that happen to share the same row type.

- [ ] **Step 2.4: Add the `[scoring]` log line**

Immediately AFTER the `highlights.push(db::HighlightRow { ... });` block (at the main site only), add:

```rust
        log::info!(
            "[scoring] [{:.0}s..{:.0}s] total={:.0}% | hook={:.0}% emotion={:.0}% payoff={:.0}% align={:.0}% context={:.0}% replay={:.0}% | sources={} | tags={:?} | excerpt={:?}",
            c.start_time, c.end_time,
            raw_score * 100.0,
            c.hook_strength * 100.0,
            c.emotional_spike * 100.0,
            c.payoff_clarity * 100.0,
            c.event_reaction_alignment * 100.0,
            c.context_simplicity * 100.0,
            c.replay_value * 100.0,
            serialize_signal_sources(&c.signal_sources),
            all_tags,
            c.transcript_excerpt.as_deref().unwrap_or(""),
        );
```

`raw_score` is the post-keyword-boost score and matches what the UI displays. `all_tags` is the local variable already in scope (combined event + emotion tags).

- [ ] **Step 2.5: Verify it compiles**

Run: `cd src-tauri && cargo check 2>&1 | tail -10`

Expected: `Finished` line.

If `serialize_signal_sources` fails to compile because of missing match arms, fix per Step 2.2's note. If `c.signal_sources` is unrecognized (wrong field name), grep `pub struct ClipCandidate` and use the actual field name.

- [ ] **Step 2.6: Commit**

```bash
git add src-tauri/src/commands/vod.rs
git commit -m "feat(scoring): persist + log per-clip dimension breakdown at selection time"
```

After this commit, re-analyzing any VOD will populate the scoring data on each highlight row AND write a single `[scoring]` line per selected clip. End of Phase C work.

---

## Task 3: `save_clip_review` Tauri command + db helper

**Goal:** Backend command that writes `review_rating` + `review_note` to a single highlight row. Validates the rating value.

**Files:**
- Modify: `src-tauri/src/db.rs` — add a small helper that updates the two columns for a given highlight id.
- Modify: `src-tauri/src/commands/clip.rs` — add `#[tauri::command] save_clip_review`.
- Modify: `src-tauri/src/lib.rs` — register the command in the `invoke_handler!` chain.

- [ ] **Step 3.1: Write the failing db unit test**

In `src-tauri/src/db.rs`, find the existing `#[cfg(test)] mod tests` block (or add one at the bottom of the file if none exists). If a tests module exists, append. If not, add at end of file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    fn insert_test_highlight(conn: &Connection, id: &str, vod_id: &str) {
        let h = HighlightRow {
            id: id.to_string(),
            vod_id: vod_id.to_string(),
            start_seconds: 0.0,
            end_seconds: 30.0,
            virality_score: 0.7,
            audio_score: 0.5,
            visual_score: 0.5,
            chat_score: 0.5,
            transcript_snippet: None,
            description: None,
            tags: None,
            thumbnail_path: None,
            created_at: "2026-05-07T00:00:00Z".to_string(),
            confidence_score: None,
            explanation: None,
            event_summary: None,
            scoring_dimensions: None,
            signal_sources: None,
            review_rating: None,
            review_note: None,
        };
        insert_highlight(conn, &h).unwrap();
    }

    #[test]
    fn set_clip_review_writes_rating_and_note() {
        let conn = fresh_db();
        insert_test_highlight(&conn, "h1", "v1");

        set_clip_review(&conn, "h1", Some("good"), Some("nice banter")).unwrap();

        let highlights = get_highlights_by_vod(&conn, "v1").unwrap();
        assert_eq!(highlights.len(), 1);
        assert_eq!(highlights[0].review_rating.as_deref(), Some("good"));
        assert_eq!(highlights[0].review_note.as_deref(), Some("nice banter"));
    }

    #[test]
    fn set_clip_review_clears_rating_and_note_when_none() {
        let conn = fresh_db();
        insert_test_highlight(&conn, "h1", "v1");
        set_clip_review(&conn, "h1", Some("good"), Some("first pass")).unwrap();

        set_clip_review(&conn, "h1", None, None).unwrap();

        let highlights = get_highlights_by_vod(&conn, "v1").unwrap();
        assert_eq!(highlights[0].review_rating, None);
        assert_eq!(highlights[0].review_note, None);
    }
}
```

The check `run_migrations` is the function name used in the codebase to run the schema setup — confirm with `grep -n "fn run_migrations\|pub fn migrate\|fn init_schema" src-tauri/src/db.rs` and adapt the helper if the symbol is different.

- [ ] **Step 3.2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test db::tests::set_clip_review -- --nocapture`

Expected: COMPILE ERROR `cannot find function 'set_clip_review' in this scope` (or similar).

- [ ] **Step 3.3: Implement `set_clip_review`**

In `src-tauri/src/db.rs`, just before the `// ── Clip helpers ──` comment around line 656, add:

```rust
/// Update review_rating and review_note on a single highlight row.
/// `rating` of `None` clears it; values must be one of `"good"`, `"meh"`,
/// `"boring"` (validated at the Tauri-command layer, not here, since this
/// helper is also used internally where invariants are already known).
pub fn set_clip_review(
    conn: &Connection,
    highlight_id: &str,
    rating: Option<&str>,
    note: Option<&str>,
) -> SqliteResult<()> {
    conn.execute(
        "UPDATE highlights SET review_rating = ?1, review_note = ?2 WHERE id = ?3",
        params![rating, note, highlight_id],
    )?;
    Ok(())
}
```

- [ ] **Step 3.4: Verify both tests pass**

Run: `cd src-tauri && cargo test db::tests::set_clip_review -- --nocapture`

Expected:
```
running 2 tests
test db::tests::set_clip_review_writes_rating_and_note ... ok
test db::tests::set_clip_review_clears_rating_and_note_when_none ... ok
```

- [ ] **Step 3.5: Add the Tauri command**

Read `src-tauri/src/commands/clip.rs` to find the existing `#[tauri::command]` patterns. Append a new function at the bottom of the file:

```rust
/// Save a user-supplied review (rating + note) for a single highlight row.
/// Used by the dev-only Review UI behind the "Show clip review tools"
/// Settings toggle. Rating must be one of "good", "meh", "boring", or
/// `None` to clear.
#[tauri::command]
pub fn save_clip_review(
    highlight_id: String,
    rating: Option<String>,
    note: Option<String>,
    db: tauri::State<'_, crate::DbConn>,
) -> Result<(), String> {
    if let Some(ref r) = rating {
        if r != "good" && r != "meh" && r != "boring" {
            return Err(format!(
                "Invalid review rating '{}'. Expected 'good', 'meh', or 'boring'.",
                r
            ));
        }
    }

    let conn = db.0.lock()
        .map_err(|e| format!("DB mutex poisoned: {}", e))?;

    crate::db::set_clip_review(&conn, &highlight_id, rating.as_deref(), note.as_deref())
        .map_err(|e| format!("DB error saving review: {}", e))
}
```

The exact name and shape of the `DbConn` state may differ — check `src-tauri/src/lib.rs` for how existing commands access the DB (look for `db: tauri::State<'_, ...>` patterns). If the state is shaped differently, mirror the existing convention.

- [ ] **Step 3.6: Register the command**

In `src-tauri/src/lib.rs`, find the `.invoke_handler(tauri::generate_handler![...])` macro call. Add `crate::commands::clip::save_clip_review,` to the list, in alphabetical order with the other `commands::clip::` entries.

- [ ] **Step 3.7: Verify it compiles**

Run: `cd src-tauri && cargo check 2>&1 | tail -10`

Expected: `Finished` line.

- [ ] **Step 3.8: Commit**

```bash
git add src-tauri/src/db.rs src-tauri/src/commands/clip.rs src-tauri/src/lib.rs
git commit -m "feat(scoring): save_clip_review Tauri command + db helper + tests"
```

---

## Task 4: `export_review_data_for_vod` Tauri command

**Goal:** Backend command that returns a single JSON string combining VOD metadata, the resolved per-game config, and per-clip data (dimensions, sources, review fields). Frontend will copy this to clipboard.

**Files:**
- Modify: `src-tauri/src/commands/clip.rs` — add `export_review_data_for_vod`.
- Modify: `src-tauri/src/lib.rs` — register the command.

- [ ] **Step 4.1: Add the command**

In `src-tauri/src/commands/clip.rs`, append after `save_clip_review`:

```rust
/// Build a single JSON blob containing everything needed for offline analysis
/// of a VOD's clip-scoring quality: VOD metadata, the resolved detection
/// config (re-resolved at export time using the VOD's game_name + the
/// current sensitivity setting), and per-clip data including dimension
/// breakdown, signal sources, and any user reviews.
///
/// Frontend consumes this via `navigator.clipboard.writeText(...)`.
/// Used by the dev-only Review UI behind the "Show clip review tools"
/// Settings toggle.
#[tauri::command]
pub fn export_review_data_for_vod(
    vod_id: String,
    db: tauri::State<'_, crate::DbConn>,
) -> Result<String, String> {
    let conn = db.0.lock()
        .map_err(|e| format!("DB mutex poisoned: {}", e))?;

    // ── VOD metadata ──
    let vod = crate::db::get_vod_by_id(&conn, &vod_id)
        .map_err(|e| format!("DB error fetching VOD: {}", e))?
        .ok_or_else(|| format!("VOD '{}' not found", vod_id))?;

    // ── Resolved config ──
    let sensitivity_str = crate::db::get_setting(&conn, "detection_sensitivity")
        .ok().flatten()
        .unwrap_or_else(|| "medium".to_string());
    let sensitivity = crate::game_config::Sensitivity::from_str_or_default(&sensitivity_str);
    let resolved = crate::game_config::ResolvedConfig::resolve(
        vod.game_name.as_deref(),
        sensitivity,
    );

    let resolved_json = serde_json::json!({
        "audio_spike_threshold": resolved.audio.spike_threshold,
        "chat_emote_burst_threshold": resolved.chat.emote_burst_threshold,
        "chat_rate_min_msgs_per_window": resolved.chat.rate_min_msgs_per_window,
        "transcript_weight": resolved.transcript.weight,
        "selector_min_clip_duration": resolved.selector.min_clip_duration,
        "selector_max_clip_duration": resolved.selector.max_clip_duration,
        "selector_min_gap_between_clips": resolved.selector.min_gap_between_clips,
        "titles_preferred": resolved.titles.preferred_categories,
        "titles_disabled": resolved.titles.disabled_categories,
        "sensitivity": sensitivity_str,
    });

    // ── Per-clip data ──
    let highlights = crate::db::get_highlights_by_vod(&conn, &vod_id)
        .map_err(|e| format!("DB error fetching highlights: {}", e))?;

    let clips_json: Vec<serde_json::Value> = highlights.iter().map(|h| {
        // Parse the stored JSON columns back into structured values so the
        // export reads as nested JSON, not as escaped strings.
        let dimensions: Option<serde_json::Value> = h.scoring_dimensions
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok());
        let sources: Option<serde_json::Value> = h.signal_sources
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok());

        serde_json::json!({
            "highlight_id": h.id,
            "start_seconds": h.start_seconds,
            "end_seconds": h.end_seconds,
            "duration_seconds": h.end_seconds - h.start_seconds,
            "total_score": h.virality_score,
            "confidence_score": h.confidence_score,
            "dimensions": dimensions,
            "signal_sources": sources,
            "tags": h.tags,
            "transcript_snippet": h.transcript_snippet,
            "event_summary": h.event_summary,
            "review_rating": h.review_rating,
            "review_note": h.review_note,
        })
    }).collect();

    let payload = serde_json::json!({
        "vod": {
            "id": vod.id,
            "title": vod.title,
            "game_name": vod.game_name,
            "duration_seconds": vod.duration_seconds,
        },
        "config_resolved": resolved_json,
        "clips": clips_json,
        "exported_at": chrono::Utc::now().to_rfc3339(),
    });

    serde_json::to_string_pretty(&payload)
        .map_err(|e| format!("JSON serialization error: {}", e))
}
```

**Note on `db::get_vod_by_id`:** verify this exists by `grep -n "pub fn get_vod_by_id" src-tauri/src/db.rs`. If the helper is named differently (e.g. `get_vod`, `vod_by_id`), use the actual name. If no single-VOD lookup exists, add a small one alongside `get_vods_by_channel`:

```rust
pub fn get_vod_by_id(conn: &Connection, vod_id: &str) -> SqliteResult<Option<VodRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, channel_id, twitch_video_id, title, duration_seconds, stream_date,
                thumbnail_url, vod_url, download_status, local_path, file_size_bytes,
                analysis_status, created_at, download_progress, analysis_progress, game_name
         FROM vods WHERE id = ?1"
    )?;
    let mut rows = stmt.query_map(params![vod_id], |row| {
        Ok(VodRow {
            id: row.get(0)?, channel_id: row.get(1)?, twitch_video_id: row.get(2)?,
            title: row.get(3)?, duration_seconds: row.get(4)?, stream_date: row.get(5)?,
            thumbnail_url: row.get(6)?, vod_url: row.get(7)?, download_status: row.get(8)?,
            local_path: row.get(9)?, file_size_bytes: row.get(10)?, analysis_status: row.get(11)?,
            created_at: row.get(12)?, download_progress: row.get(13)?, analysis_progress: row.get(14)?,
            game_name: row.get(15)?,
        })
    })?;
    Ok(rows.next().transpose()?)
}
```

- [ ] **Step 4.2: Register the command in `lib.rs`**

In `src-tauri/src/lib.rs`'s `.invoke_handler(tauri::generate_handler![...])`, add `crate::commands::clip::export_review_data_for_vod,` to the list (alphabetical with siblings).

- [ ] **Step 4.3: Verify it compiles**

Run: `cd src-tauri && cargo check 2>&1 | tail -10`

Expected: `Finished` line.

- [ ] **Step 4.4: Smoke test the JSON shape via a unit test**

In `src-tauri/src/db.rs`, append to the `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn highlight_round_trips_scoring_dimensions_json() {
        let conn = fresh_db();
        let h = HighlightRow {
            id: "h1".to_string(),
            vod_id: "v1".to_string(),
            start_seconds: 100.0,
            end_seconds: 130.0,
            virality_score: 0.72,
            audio_score: 0.6, visual_score: 0.6, chat_score: 0.6,
            transcript_snippet: None, description: None, tags: None,
            thumbnail_path: None, created_at: "2026-05-07T00:00:00Z".to_string(),
            confidence_score: None, explanation: None, event_summary: None,
            scoring_dimensions: Some(r#"{"hook":0.8,"emotion":0.75}"#.to_string()),
            signal_sources: Some(r#"["audio","transcript"]"#.to_string()),
            review_rating: None,
            review_note: None,
        };
        insert_highlight(&conn, &h).unwrap();

        let fetched = get_highlights_by_vod(&conn, "v1").unwrap();
        assert_eq!(fetched.len(), 1);
        assert_eq!(fetched[0].scoring_dimensions.as_deref(), Some(r#"{"hook":0.8,"emotion":0.75}"#));
        assert_eq!(fetched[0].signal_sources.as_deref(), Some(r#"["audio","transcript"]"#));
    }
```

Run: `cd src-tauri && cargo test db::tests::highlight_round_trips -- --nocapture`

Expected: `1 passed; 0 failed`.

- [ ] **Step 4.5: Commit**

```bash
git add src-tauri/src/db.rs src-tauri/src/commands/clip.rs src-tauri/src/lib.rs
git commit -m "feat(scoring): export_review_data_for_vod Tauri command + JSON round-trip test"
```

---

## Task 5: Frontend — Settings toggle "Show clip review tools"

**Goal:** Add the toggle UI to the Settings page. Persist via the existing settings storage. Expose the boolean via the existing app store.

**Files:**
- Modify: `src/pages/Settings.tsx` — add a new "Developer tools" section with the toggle.
- Modify: `src/stores/appStore.ts` (or whichever Zustand store holds app settings) — add `showReviewTools: boolean` and a setter.

- [ ] **Step 5.1: Locate the existing settings store and current toggle pattern**

Run from project root: `grep -rn "useAppStore\|create<.*Store" src/stores/ | head -10`

Read `src/stores/appStore.ts` (or whatever the settings store is) to understand:
1. What the existing Zustand store pattern looks like (state shape, action creators)
2. How an existing setting is loaded from the backend on mount and persisted on change

Also: `grep -n "save_setting\|get_setting" src/` to find existing JS-side settings calls. The pattern should be `invoke('save_setting', { key, value })` and `invoke('get_setting', { key })`.

- [ ] **Step 5.2: Add `showReviewTools` to the store**

In the appropriate store file, find the state interface (likely `interface AppState`) and add:

```typescript
  showReviewTools: boolean;
  setShowReviewTools: (v: boolean) => Promise<void>;
```

In the store creator, add to the initial state:

```typescript
  showReviewTools: false,
```

And to the actions, add (near other settings setters):

```typescript
  setShowReviewTools: async (v: boolean) => {
    await invoke('save_setting', { key: 'show_review_tools', value: v ? 'true' : 'false' });
    set({ showReviewTools: v });
  },
```

In the store's "load settings on mount" function (look for `loadSettings`, `hydrate`, or similar), add:

```typescript
    const showReviewTools = await invoke<string | null>('get_setting', { key: 'show_review_tools' });
    set({ showReviewTools: showReviewTools === 'true' });
```

If no such hydration function exists, the setting will load lazily from the backend on first render — fine for v1.3.12.

- [ ] **Step 5.3: Add the toggle to the Settings page**

Read `src/pages/Settings.tsx` to find an existing toggle component to copy the pattern from (e.g. a toggle for "Auto-export clips" or similar). Identify the exact component name used (likely a custom `Switch` or `Toggle`, or a Tailwind-styled `<button>`).

Find a sensible insertion point — near the bottom of the page, after the existing settings sections — and insert a new section:

```tsx
      {/* Developer tools — hidden behind a toggle for dev / clip-quality investigation use */}
      <section className="mt-8 pt-6 border-t border-surface-700">
        <h2 className="text-lg font-semibold text-slate-100 mb-1">Developer tools</h2>
        <p className="text-sm text-slate-400 mb-4">
          Enable advanced controls for diagnosing clip-detection quality.
          Off by default. These tools have no effect on normal clip generation
          and are only used when investigating scoring or detection issues.
        </p>

        <label className="flex items-center justify-between cursor-pointer">
          <div>
            <div className="text-sm font-medium text-slate-200">Show clip review tools</div>
            <div className="text-xs text-slate-400 mt-0.5">
              Adds rating buttons and note fields to each clip card, plus an
              "Export review data" button on the Vods page. Used to gather
              feedback for tuning the clip scoring model.
            </div>
          </div>
          <input
            type="checkbox"
            checked={showReviewTools}
            onChange={(e) => setShowReviewTools(e.target.checked)}
            className="w-4 h-4 rounded border-surface-600 bg-surface-800 text-violet-500 focus:ring-violet-500 cursor-pointer"
          />
        </label>
      </section>
```

Add the destructure at the top of the component:

```tsx
  const { showReviewTools, setShowReviewTools } = useAppStore();
```

If the existing Settings page uses a different toggle component than a raw `<input type="checkbox">`, mirror it for visual consistency rather than introducing a new style.

- [ ] **Step 5.4: Manual verification**

Run from project root: `cargo tauri dev`

Wait for the app window to launch. Navigate to **Settings**.

Expected:
- Bottom of the page now has a "Developer tools" section with one toggle: "Show clip review tools"
- Toggle is OFF by default
- Toggling it ON, then closing and reopening the app, preserves the ON state (validates the persistence)

If the toggle doesn't render or doesn't persist, debug before committing.

- [ ] **Step 5.5: Commit**

```bash
git add src/pages/Settings.tsx src/stores/appStore.ts
git commit -m "feat(review-ui): Show clip review tools Settings toggle"
```

---

## Task 6: Frontend — Clip card rating buttons + note + badge

**Goal:** When the toggle is ON, each clip card on the Clips page shows three rating buttons, a note textarea, and a colored badge. Rating + note saves to the backend on change. Badge color reflects the saved rating.

**Files:**
- Modify: `src/pages/Clips.tsx` — add the review section conditional on `showReviewTools`.
- Create: `src/types/clipReview.ts` — type for the rating values.

- [ ] **Step 6.1: Create the type file**

Create `src/types/clipReview.ts`:

```typescript
export type ClipReviewRating = 'good' | 'meh' | 'boring';

export interface ClipReview {
  rating: ClipReviewRating | null;
  note: string;
}

export const REVIEW_RATING_LABELS: Record<ClipReviewRating, string> = {
  good: '✓ Good',
  meh: '— Meh',
  boring: '✗ Boring',
};

export const REVIEW_RATING_COLORS: Record<ClipReviewRating, string> = {
  good: 'bg-emerald-500/20 text-emerald-300 border-emerald-500/30',
  meh: 'bg-slate-500/20 text-slate-300 border-slate-500/30',
  boring: 'bg-rose-500/20 text-rose-300 border-rose-500/30',
};
```

- [ ] **Step 6.2: Add the review block to clip cards**

In `src/pages/Clips.tsx`, find where each clip card is rendered (it's likely a `.map` over a `clips` array). Identify whether the data shape exposes `review_rating` and `review_note` already (after Task 4 they're on `HighlightRow`, but the frontend may use a different data type). If the frontend type doesn't have them, extend it.

Inside the clip card render, near the bottom of the card (after existing metadata like score and duration), add a conditional block:

```tsx
{showReviewTools && (
  <div className="mt-3 pt-3 border-t border-surface-700/50">
    <div className="flex items-center gap-2 mb-2">
      {(['good', 'meh', 'boring'] as ClipReviewRating[]).map((r) => (
        <button
          key={r}
          onClick={() => handleRatingChange(clip.id, clip.review_rating === r ? null : r)}
          className={`px-2 py-1 text-xs rounded border transition-colors cursor-pointer ${
            clip.review_rating === r
              ? REVIEW_RATING_COLORS[r]
              : 'bg-surface-800 text-slate-400 border-surface-600 hover:text-white hover:border-surface-500'
          }`}
        >
          {REVIEW_RATING_LABELS[r]}
        </button>
      ))}
      {clip.review_rating && (
        <span className={`ml-auto px-2 py-0.5 text-[10px] uppercase tracking-wide rounded ${REVIEW_RATING_COLORS[clip.review_rating as ClipReviewRating]}`}>
          rated
        </span>
      )}
    </div>
    <textarea
      defaultValue={clip.review_note ?? ''}
      onBlur={(e) => handleNoteChange(clip.id, e.target.value)}
      placeholder="Notes (saves on blur)..."
      className="w-full text-xs px-2 py-1 rounded bg-surface-800 border border-surface-600 text-slate-200 focus:border-violet-500 focus:outline-none resize-y min-h-[2rem]"
      rows={1}
    />
  </div>
)}
```

Also at the top of the file, add:

```tsx
import type { ClipReviewRating } from '../types/clipReview';
import { REVIEW_RATING_LABELS, REVIEW_RATING_COLORS } from '../types/clipReview';
```

And destructure `showReviewTools` from the store:

```tsx
const { showReviewTools } = useAppStore();
```

- [ ] **Step 6.3: Add the change handlers**

Near the top of the Clips component (where other handlers live), add:

```tsx
const handleRatingChange = async (highlightId: string, rating: ClipReviewRating | null) => {
  try {
    await invoke('save_clip_review', {
      highlightId,
      rating,
      note: null,  // Note is left untouched when only rating changes; backend SQL replaces both columns, so we need to send the existing note.
    });
    // Refresh the clips list so the badge reflects the new state
    await refreshClips();
  } catch (e) {
    console.error('Failed to save clip review rating:', e);
  }
};

const handleNoteChange = async (highlightId: string, note: string) => {
  try {
    // Find the current rating to preserve it (since save_clip_review replaces both)
    const clip = clips.find((c) => c.id === highlightId);
    await invoke('save_clip_review', {
      highlightId,
      rating: clip?.review_rating ?? null,
      note: note.trim() || null,
    });
  } catch (e) {
    console.error('Failed to save clip review note:', e);
  }
};
```

**Important architectural note for the implementer:** the backend `save_clip_review` UNCONDITIONALLY replaces both `review_rating` and `review_note` columns. So when the user only changes one, the frontend must send the current value of the other to avoid clearing it. The handlers above implement this. If the implementer changes the backend command to be field-by-field optional updates instead, the handlers can simplify.

`refreshClips` is the existing function that re-fetches clips from the backend; if it's named something else in this file (e.g. `loadClips`, `fetchClips`), use that name.

- [ ] **Step 6.4: Manual verification**

Run: `cargo tauri dev`

1. Toggle "Show clip review tools" ON in Settings
2. Navigate to Clips page
3. Each clip card now shows: 3 rating buttons + a notes textarea below
4. Click "✓ Good" — button highlights green, "rated" badge appears
5. Click again on the same button — clears the rating, badge disappears
6. Click "✗ Boring" — button highlights red
7. Type a note in the textarea, click outside — saves
8. Refresh / re-open app — saved rating + note persists
9. Toggle "Show clip review tools" OFF in Settings — review UI disappears from clip cards but data is preserved (turning toggle back on shows it again)

- [ ] **Step 6.5: Commit**

```bash
git add src/pages/Clips.tsx src/types/clipReview.ts
git commit -m "feat(review-ui): rating buttons + note + badge on clip cards"
```

---

## Task 7: Frontend — Export button on Vods page

**Goal:** Add an "Export review data" button on each completed VOD card when the toggle is ON. Click invokes the backend command, copies the resulting JSON to the clipboard, shows a success toast.

**Files:**
- Modify: `src/pages/Vods.tsx` — add the button next to the existing "Re-analyze" / "View Clips" buttons.

- [ ] **Step 7.1: Locate the VOD card actions row**

Read `src/pages/Vods.tsx` to find the row of buttons that appears on completed VOD cards (the area around line 632 where the "View Clips" button lives, per pre-flight notes).

- [ ] **Step 7.2: Add the export handler**

Near the top of the component, add:

```tsx
const handleExportReviewData = async (vodId: string, vodTitle: string) => {
  try {
    const json = await invoke<string>('export_review_data_for_vod', { vodId });
    await navigator.clipboard.writeText(json);
    // Show a toast — use whatever toast helper this project uses. If none,
    // fall back to alert(). Replace with the project's toast once verified:
    showToast?.(`Review data for "${vodTitle}" copied to clipboard`) ?? alert(`Review data for "${vodTitle}" copied to clipboard.`);
  } catch (e) {
    console.error('Failed to export review data:', e);
    alert(`Failed to export review data: ${e}`);
  }
};
```

The implementer should `grep -n "showToast\|toast(" src/` to find the existing toast utility — most projects in this stack have one. Use that. If none exists, leave the `alert()` fallback.

- [ ] **Step 7.3: Add the button next to "View Clips" / "Re-analyze"**

In the actions row of each completed VOD card, just before or after the "Re-analyze" button, add (gated on the toggle):

```tsx
{showReviewTools && vod.analysis_status === 'completed' && (
  <button
    onClick={() => handleExportReviewData(vod.id, vod.title)}
    className="flex-1 flex items-center justify-center gap-1.5 px-3 py-2 text-xs rounded-lg transition-colors cursor-pointer bg-amber-500/20 text-amber-400 border border-amber-500/30 hover:bg-amber-500/30"
    title="Copy clip-by-clip review data to clipboard for offline analysis"
  >
    <Download className="w-3.5 h-3.5" />
    Export review data
  </button>
)}
```

The `Download` icon import should already be available (lucide-react). If not: `import { Download, RotateCcw, Search } from 'lucide-react';` (the existing imports likely already include several icons; add `Download` to the list).

Add `showReviewTools` to the destructure at the top of the component (same pattern as Task 6).

- [ ] **Step 7.4: Manual verification**

Run: `cargo tauri dev`

1. Toggle "Show clip review tools" ON
2. Navigate to Vods page
3. Each completed VOD now has an "Export review data" button (amber color, Download icon)
4. Click it on a VOD that has highlights
5. Verify: toast/alert appears confirming clipboard copy
6. Paste into a text editor — should be a JSON object with `vod`, `config_resolved`, `clips`, and `exported_at` keys
7. Verify clip data: each clip should have `dimensions` populated (post-Task 2 re-analysis) or null (pre-Task 2 data)
8. Toggle OFF — button disappears

- [ ] **Step 7.5: Commit**

```bash
git add src/pages/Vods.tsx
git commit -m "feat(review-ui): Export review data button on Vods page"
```

---

## Task 8: End-to-end smoke test + final commit

**Goal:** Verify the whole flow works on a real VOD. No code changes — just exercising the path Slug will use during Phase B.

**Files:** None modified.

- [ ] **Step 8.1: Run the build**

Run: `cd src-tauri && cargo check 2>&1 | tail -5`

Expected: `Finished` line.

Run all tests:

`cd src-tauri && cargo test -- --nocapture 2>&1 | tail -30`

Expected: all `db::tests::set_clip_review_*` and `db::tests::highlight_round_trips_*` tests pass. Other tests should be unaffected by these changes.

- [ ] **Step 8.2: Run the app**

Run: `cargo tauri dev`

- [ ] **Step 8.3: End-to-end exercise**

1. **Settings setup:** open Settings, toggle "Show clip review tools" ON.
2. **Re-analyze a VOD:** navigate to Vods page, click "Re-analyze" on a previously-analyzed VOD (so we get fresh highlights with the new `scoring_dimensions` populated). Wait for analysis to finish.
3. **Verify log line:** in another PowerShell window, run:

   ```powershell
   Get-Content "$env:LOCALAPPDATA\com.clipgoblin.desktop\logs\ClipGoblin.log" | Select-String "scoring" | Select-Object -Last 5
   ```

   Expected: 5 most-recent `[scoring]` lines, each in the documented format. Example:

   ```
   [scoring] [9255s..9285s] total=72% | hook=80% emotion=75% payoff=68% align=65% context=70% replay=72% | sources=["audio","transcript"] | tags=[...] | excerpt="should I go strength or dex"
   ```

4. **Review some clips:** navigate to Clips, filter to that VOD. Pick 3 clips at random and:
   - Rate the first as Good with a short note
   - Rate the second as Boring with a short note
   - Rate the third as Meh with no note
5. **Verify ratings persist:** close and reopen the app, navigate back. Ratings should still be there.
6. **Export:** navigate to Vods page, click "Export review data" on the VOD.
7. **Inspect the clipboard contents:** paste into a text editor. Verify:
   - Top-level keys: `vod`, `config_resolved`, `clips`, `exported_at`
   - `vod.game_name` is the right game
   - `config_resolved` has the correct values for that game's genre + per-game overrides
   - `clips[].dimensions` is populated for ALL clips (since we re-analyzed)
   - `clips[].review_rating` matches what you set (good / boring / meh, others null)
   - `clips[].review_note` matches your notes

If anything in 1–7 fails, debug. If all 7 succeed, the infrastructure is ready for Phase B (Slug uses it on real VODs to gather ground truth).

- [ ] **Step 8.4: Bump version + commit**

The investigation infrastructure ships as a v1.3.12-pre release (still hidden behind the toggle, so no user-visible behavior change for anyone who doesn't enable it). We bump to v1.3.12 *after* phase A's actual scoring fix lands. For now, no version bump — just leave the smoke-test pass as the validation gate.

Run from project root:

```powershell
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
git push origin main
```

This pushes Tasks 1–7's commits to GitHub. No tag yet — v1.3.12 tag goes on the post-phase-A commit.

---

## Self-review

(Run by the plan author with fresh eyes. Findings written here for transparency.)

### Spec coverage

Walking each spec section against the plan:

- **§1 Background** — context only, no implementation needed. ✓
- **§2 Goals & success criteria** — Phase C log + DB storage covered by Tasks 1+2; Review UI covered by Tasks 3–7. The "single JSON paste contains scores + reviews" criterion is implemented in Task 4's export command. ✓
- **§3 Phase C — Scoring instrumentation** — log format covered in Task 2.4; storage location (HighlightRow construction site) covered in Task 2.3; data availability (dimensions on ClipCandidate) verified in pre-flight. ✓
- **§4.1 Data model** — Task 1 adds the 4 columns. ✓
- **§4.2 Backend command `save_clip_review`** — Task 3. ✓
- **§4.3 Settings toggle** — Task 5. ✓
- **§4.4 Clip-card UI** — Task 6. ✓
- **§4.5 Export button + JSON shape** — Task 4 (backend) + Task 7 (frontend button). ✓
- **§4.6 Visibility / shipping discipline** — Settings toggle defaults OFF, configured in Task 5.2 (`showReviewTools: false` initial state). ✓
- **§4.7 Edge cases** — VOD re-analysis preserves reviews via the `ON CONFLICT` clause in Task 1.3 (review fields excluded from update); concurrent edits aren't an issue (single-user dev mode); empty/null dimensions handled by Task 4's export command (`.and_then(serde_json::from_str)` returns `None` cleanly). ✓
- **§5 Phasing** — Tasks 1–8 cover Sub-release 1 (infrastructure). Sub-release 2 is a separate plan written after Phase B data is in. ✓

No spec gaps.

### Placeholder scan

Searched for the red-flag patterns in the No Placeholders rule. Findings:

- "TODO Task 2:" comments in Task 1.5 — these are *not* placeholder violations; they're scaffolding that Task 2 explicitly removes within a few hours. Each has a concrete action attached.
- One genuine ambiguity: in Task 6.3 the comment says "Note is left untouched when only rating changes... so we need to send the existing note." But the handler I wrote sends `note: null`, which would CLEAR the note. Bug. Fixing inline:

The corrected `handleRatingChange` in Task 6.3 should preserve the existing note:

```tsx
const handleRatingChange = async (highlightId: string, rating: ClipReviewRating | null) => {
  try {
    const clip = clips.find((c) => c.id === highlightId);
    await invoke('save_clip_review', {
      highlightId,
      rating,
      note: clip?.review_note ?? null,  // PRESERVE existing note
    });
    await refreshClips();
  } catch (e) {
    console.error('Failed to save clip review rating:', e);
  }
};
```

(That fix is now inlined above in Task 6.3.)

### Type consistency

Walking the type chain across tasks:

- `HighlightRow.scoring_dimensions: Option<String>` (Task 1.2) — used in Task 4.1's export to parse back into JSON. ✓
- `HighlightRow.signal_sources: Option<String>` — same pattern. ✓
- `HighlightRow.review_rating: Option<String>` — written by `set_clip_review` (Task 3.3), read in Task 4.1 export. ✓
- `ClipCandidate.signal_sources: Vec<SignalSource>` — assumed in Task 2.2 helper. The pre-flight calls out verifying this exact field name; if it differs, the implementer adapts. ✓
- `ClipReviewRating` type from `src/types/clipReview.ts` (Task 6.1) — used in Task 6.2's UI and Task 6.3's handler. ✓
- `showReviewTools` boolean — defined in store (Task 5.2), consumed in Settings (Task 5.3), Clips (Task 6.2), Vods (Task 7.3). All consistent. ✓
- Tauri command name `save_clip_review` — defined in Task 3.5, called from Task 6.3. Camel-case parameter handling is automatic in Tauri (Rust `highlight_id` ↔ JS `highlightId`). ✓
- Tauri command name `export_review_data_for_vod` — defined in Task 4.1, called from Task 7.2. ✓

No type drift detected after the inline fix in §placeholder-scan.
