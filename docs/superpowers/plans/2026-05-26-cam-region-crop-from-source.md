# Crop-from-source Cam Region Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the unshipped v1.3.16 external-file attach with a crop-from-source cam region — drag a rectangle on the source VOD frame and have that exact region fill the cam slot in every clip from that VOD using PiP or Split layout.

**Architecture:** Single ffmpeg input, source split internally into gameplay + cam_sharp (+ cam_blur for Split). Per-VOD region storage with optional per-clip override behind a Settings toggle. Three Fit modes (Fit/Fill/Stretch). Layout-specific bar fill: PiP = gameplay pass-through, Split = blurred-source extension.

**Tech Stack:** Tauri 2, Rust (`rusqlite`, `serde`, `serde_json`, ffmpeg via `Command`), React + TypeScript, existing SQLite k/v `settings` table.

**Spec:** `docs/superpowers/specs/2026-05-26-cam-region-crop-from-source-design.md`

---

## Working notes for the implementer

- **Cargo NOT available in this VM.** Rust tasks rely on static analysis + colocated unit tests; the real compiler check happens at **Task 12 (Slug-side)**. Do not attempt `cargo check`, `cargo test`, or `cargo build`.
- **npm IS available.** Use `npm run build` (which runs `tsc -b && vite build`) as the gate for TypeScript tasks.
- **Direct-on-main.** No PRs, no worktrees. Commit each task individually.
- **No version bump until Task 12.** Slug runs `bump-version.ps1 <version>` at ship time.
- **v1.3.14 cautionary tale:** that ship had Unicode-in-token-position bugs that lexer-fatal'd Rust and per-task reviews missed because cargo couldn't run in-VM. Every Rust task ends with an explicit Unicode sweep step.
- **v1.3.16 cautionary tale:** the final whole-feature review missed `VodRow { ... }` literals in `commands/vod.rs:2454` and `:3126` that lacked the new fields → E0063 errors only caught at Slug's `cargo check`. Task 2 of this plan explicitly greps for `VodRow {` and `ClipRow {` literals across the crate and patches every match.
- **Line numbers below reflect the POST-Task-1-revert state** (i.e., state at git commit `6927297`, the v1.3.16 plan commit). They do NOT reflect the current pre-revert state.

---

## File structure (informs task decomposition)

**Create:**
- `src-tauri/src/cam_region.rs` — pure helpers: `CamRegion` parser+clamper, `CamFitMode` enum, region-resolver, `to_crop_expr` formatter. Has its own `#[cfg(test)] mod tests`.
- `src-tauri/src/commands/cam_region.rs` — Tauri commands: `set_vod_cam_region`, `clear_vod_cam_region`, `set_clip_cam_region_override`, `clear_clip_cam_region_override`, `set_clip_fit_mode`, `set_allow_per_clip_override`.
- `src/components/CamRegionSetter.tsx` — drag overlay on source player (corner + edge handles, min-size enforcement, Save/Cancel, Esc=cancel).
- `src/components/CamRegionRow.tsx` — editor right-rail row (current-region display + Set/Clear + Fit dropdown + conditional override sub-row).

**Modify:**
- `src-tauri/src/db.rs` — migrations (3 columns + settings k/v default); reintroduce `VOD_SELECT` + `CLIP_SELECT` constants; extend `VodRow` + `ClipRow` with new fields; update the SELECT row-binding closures.
- `src-tauri/src/commands/mod.rs` — `pub mod cam_region;`.
- `src-tauri/src/commands/vod.rs` — patch the two `VodRow { ... }` literals AND the one `ClipRow { ... }` literal to include new fields.
- `src-tauri/src/lib.rs` — handler registrations for the new Tauri commands.
- `src-tauri/src/vertical_crop.rs` — new `layout_filter_with_region` function; `ExportRequest` gains `effective_region: Option<CamRegion>` and `fit_mode: CamFitMode`; existing `layout_filter` byte-unchanged for the no-region path.
- `src-tauri/src/commands/export.rs` — `clip_to_export_request` resolves effective region using the Settings toggle + clip override + VOD region precedence.
- `src/types.ts` — `Vod.cam_region_norm`, `Clip.cam_region_norm_override`, `Clip.cam_fit_mode`.
- `src/pages/Editor.tsx` — embed `CamRegionRow` in the Layout section; mount `CamRegionSetter` as a player overlay when in edit mode.
- `src/pages/Settings.tsx` — new toggle row.
- `docs/superpowers/specs/2026-05-17-per-vod-cam-asset-design.md` — superseded banner.
- `docs/superpowers/plans/2026-05-17-per-vod-cam-asset.md` — superseded banner.

---

## Task 1: Revert v1.3.16 + add superseded banners

**Files:**
- Operate on git history (reset to `6927297`)
- Modify: `docs/superpowers/specs/2026-05-17-per-vod-cam-asset-design.md`
- Modify: `docs/superpowers/plans/2026-05-17-per-vod-cam-asset.md`

- [ ] **Step 1.1: Confirm baseline state**

Run:

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
git status
git log --oneline -15
```

Expected: clean working tree (no uncommitted changes). `git log` should show ~14 commits ahead of `origin/main`: 11 v1.3.16 implementation commits, the E0063 fix commit `5e8b310`, the `.gitignore` brainstorm commit `cf0e7e1`, and the new spec commit `5ee81ee`. `6927297` (the v1.3.16 plan commit) should appear in the list.

If there are uncommitted changes, STOP and surface them — do not proceed with the reset.

- [ ] **Step 1.2: Reset to the plan commit**

```bash
git reset --hard 6927297
```

Verify:

```bash
git log --oneline -3
git status
```

Expected: HEAD is at `6927297 docs: implementation plan for per-VOD cam asset (9 tasks)`. The previous 11 implementation commits + the spec/gitignore commits we just made are gone from the branch. Working tree clean.

The two design+plan docs (`docs/superpowers/specs/2026-05-17-per-vod-cam-asset-design.md` and `docs/superpowers/plans/2026-05-17-per-vod-cam-asset.md`) ARE still in the tree (committed at `571c2b4` / `6927297`, both ancestor to the new HEAD).

The new spec we just wrote (`docs/superpowers/specs/2026-05-26-cam-region-crop-from-source-design.md`) is NO LONGER in the tree — it was committed at `5ee81ee` which is downstream of HEAD. The reflog preserves it (90 days). We restore it in Step 1.3.

- [ ] **Step 1.3: Restore the new spec doc + gitignore from reflog**

```bash
git checkout 5ee81ee -- docs/superpowers/specs/2026-05-26-cam-region-crop-from-source-design.md
git checkout cf0e7e1 -- .gitignore
git status
```

Expected: both files appear as staged additions (the spec is new; `.gitignore` has 3 added lines). Verify:

```bash
git diff --cached .gitignore | tail -5
```

Expected output:

```text
+
+# Brainstorm session artifacts (visual companion)
+.superpowers/
```

- [ ] **Step 1.4: Add superseded banner to the v1.3.16 design doc**

Open `docs/superpowers/specs/2026-05-17-per-vod-cam-asset-design.md`. The file currently starts with:

```markdown
# Per-VOD Cam Asset — Design

**Status:** Approved in brainstorming; pending written-spec review
```

Use the Edit tool to insert the banner immediately above the existing heading:

- **old_string:**
```
# Per-VOD Cam Asset — Design

**Status:** Approved in brainstorming; pending written-spec review
```

- **new_string:**
```
> **Superseded** — This design was implemented end-to-end (11 commits) and reverted before shipping after live UX feedback showed users want crop-from-source (use a region of the source frame as the cam) rather than attach-external-file. See [`2026-05-26-cam-region-crop-from-source-design.md`](2026-05-26-cam-region-crop-from-source-design.md) for the replacement design.

---

# Per-VOD Cam Asset — Design

**Status:** Approved in brainstorming; pending written-spec review
```

- [ ] **Step 1.5: Add superseded banner to the v1.3.16 plan doc**

Open `docs/superpowers/plans/2026-05-17-per-vod-cam-asset.md`. Use Edit:

- **old_string:**
```
# Per-VOD Cam Asset Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
```

- **new_string:**
```
> **Superseded** — This plan was fully executed (11 commits) and then reverted; no user release. See [`../plans/2026-05-26-cam-region-crop-from-source.md`](2026-05-26-cam-region-crop-from-source.md) for the replacement plan and `../specs/2026-05-26-cam-region-crop-from-source-design.md` for the replacement design.

---

# Per-VOD Cam Asset Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
```

- [ ] **Step 1.6: Commit the revert + banners + restored files**

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
git add .gitignore docs/superpowers/specs/2026-05-26-cam-region-crop-from-source-design.md docs/superpowers/specs/2026-05-17-per-vod-cam-asset-design.md docs/superpowers/plans/2026-05-17-per-vod-cam-asset.md
git status
```

Expected: 4 files staged (1 new + 3 modified).

```bash
git commit -m "chore: revert v1.3.16 cam-asset; supersede with crop-from-source design"
```

`git log --oneline -3` should show the new revert+banner commit at HEAD, the v1.3.16 plan commit underneath, and the v1.3.16 design doc commit before that.

This is the new baseline. All subsequent tasks reference line numbers relative to this state.

---

## Task 2: DB layer — migrations + struct extensions + SELECT constants + helpers + literal patches

**Files:**
- Modify: `src-tauri/src/db.rs`
- Modify: `src-tauri/src/commands/vod.rs` (patch `VodRow {` + `ClipRow {` literals)

- [ ] **Step 2.1: Add three idempotent ALTER TABLE migrations + settings default**

In `src-tauri/src/db.rs`, locate the `run_migrations` function (around line 42). At the END of that function, AFTER the final existing `conn.execute("ALTER TABLE ...", []).ok();` line (whichever is last), add a clearly-commented block:

```rust
    // ─── v1.3.16+ Cam region (crop-from-source) ──────────────────────
    // Per-VOD source-frame region; per-clip optional override; per-clip fit mode.
    // All NULL by default = pre-feature dup-source behavior preserved.
    conn.execute(
        "ALTER TABLE vods ADD COLUMN cam_region_norm TEXT",
        [],
    ).ok();
    conn.execute(
        "ALTER TABLE clips ADD COLUMN cam_region_norm_override TEXT",
        [],
    ).ok();
    conn.execute(
        "ALTER TABLE clips ADD COLUMN cam_fit_mode TEXT",
        [],
    ).ok();
    // Settings k/v default (existing settings table). Idempotent.
    conn.execute(
        "INSERT OR IGNORE INTO settings(key, value) VALUES('allow_per_clip_cam_region_override', 'false')",
        [],
    ).ok();
```

The `.ok()` swallows the "duplicate column name" error that fires on second-run startups. This matches the existing pattern in `db.rs`.

- [ ] **Step 2.2: Extend `VodRow` struct with `cam_region_norm`**

Find `pub struct VodRow` (around line 267). The struct currently ends with `pub game_name: Option<String>,` (or similar — the last field name as of the post-revert state). Add ONE new field at the END (before the closing `}`):

```rust
    /// JSON-encoded `{"x":f32,"y":f32,"w":f32,"h":f32}` in normalized 0..1 source-frame coords.
    /// NULL/absent = no region set; export falls back to dup-source.
    #[serde(default)]
    pub cam_region_norm: Option<String>,
```

The `#[serde(default)]` is critical — it ensures old JSON payloads (without this field) deserialize cleanly to `None`.

- [ ] **Step 2.3: Extend `ClipRow` struct with two new fields**

Find `pub struct ClipRow` (around line 327). Add TWO new fields at the END (before the closing `}`):

```rust
    /// Optional per-clip override of the parent VOD's `cam_region_norm`.
    /// Only consulted when the `allow_per_clip_cam_region_override` setting is true.
    /// NULL = use VOD's region.
    #[serde(default)]
    pub cam_region_norm_override: Option<String>,
    /// 'fit' | 'fill' | 'stretch'. NULL is treated as 'fit' (default).
    #[serde(default)]
    pub cam_fit_mode: Option<String>,
```

- [ ] **Step 2.4: Extract `VOD_SELECT` and `CLIP_SELECT` SQL column constants**

This is the defensive refactor that prevents the column-drift bug class. Find the three `pub fn get_vods_*` functions in `db.rs` (around lines 510–600). Each currently has a hand-written `SELECT id, channel_id, twitch_video_id, ... FROM vods WHERE ...` literal. Replace each with a `format!("{} WHERE ...", VOD_SELECT)` call.

First, add the constant near the top of the file (after the `use` declarations, before the first function). Find an appropriate spot above `pub fn init_db` and insert:

```rust
/// Full SELECT column list for the `vods` table. Centralized so that adding
/// a column to `VodRow` only requires updating this string plus the
/// row-binding closures (one per get_vods_* function). Order MUST match the
/// fields in `VodRow` so positional `row.get(idx)?` calls stay in sync.
const VOD_SELECT: &str = "SELECT id, channel_id, twitch_video_id, title, \
    duration_seconds, stream_date, thumbnail_url, vod_url, download_status, \
    local_path, file_size_bytes, analysis_status, created_at, download_progress, \
    analysis_progress, game_name, cam_region_norm FROM vods";

/// Full SELECT column list for the `clips` table. Same DRY pattern as VOD_SELECT.
const CLIP_SELECT: &str = "SELECT id, highlight_id, vod_id, title, start_seconds, \
    end_seconds, aspect_ratio, crop_x, crop_y, crop_width, crop_height, \
    captions_enabled, captions_text, captions_position, caption_style, \
    facecam_layout, render_status, output_path, thumbnail_path, created_at, \
    game, publish_description, publish_hashtags, cam_region_norm_override, \
    cam_fit_mode FROM clips";
```

Then update each `get_vods_*` function:

For `get_vods_by_channel`: change the prepare line from the existing hand-written SELECT to:

```rust
let mut stmt = conn.prepare(&format!("{} WHERE channel_id = ?1 ORDER BY stream_date DESC", VOD_SELECT))?;
```

For `get_all_vods`: 

```rust
let mut stmt = conn.prepare(&format!("{} ORDER BY stream_date DESC", VOD_SELECT))?;
```

For `get_vod_by_id`: 

```rust
let mut stmt = conn.prepare(&format!("{} WHERE id = ?1", VOD_SELECT))?;
```

In each function, extend the row-binding closure (`|row| Ok(VodRow { ... })`) to include the new field at index 16:

```rust
        cam_region_norm: row.get(16)?,
```

For the clips SELECTs: find `get_clip_by_id` and any other `SELECT ... FROM clips` queries (likely in `get_clips_by_vod`, `get_all_clips`, etc.). Apply the same pattern: replace the hand-written SELECT with `format!("{} WHERE ...", CLIP_SELECT)` and extend each row-binding closure with:

```rust
        cam_region_norm_override: row.get(23)?,
        cam_fit_mode: row.get(24)?,
```

(Indices 23 and 24 come from counting the columns in `CLIP_SELECT` above. Double-check by counting commas in the constant string.)

- [ ] **Step 2.5: Add `pub fn update_*` helpers for the new fields**

After the existing `update_vod_*` helpers in `db.rs`, add:

```rust
/// Set a VOD's cam region. `region_json` is the serde-serialized
/// `CamRegion` struct (see `crate::cam_region`). Passing `None` clears.
pub fn update_vod_cam_region(
    conn: &Connection,
    vod_id: &str,
    region_json: Option<&str>,
) -> SqliteResult<()> {
    conn.execute(
        "UPDATE vods SET cam_region_norm = ?1 WHERE id = ?2",
        params![region_json, vod_id],
    )?;
    Ok(())
}

/// Set a clip's per-VOD region override. NULL = clear override.
pub fn update_clip_cam_region_override(
    conn: &Connection,
    clip_id: &str,
    region_json: Option<&str>,
) -> SqliteResult<()> {
    conn.execute(
        "UPDATE clips SET cam_region_norm_override = ?1 WHERE id = ?2",
        params![region_json, clip_id],
    )?;
    Ok(())
}

/// Set a clip's fit mode. `mode` is the lowercase string 'fit' | 'fill' | 'stretch'.
/// NULL = revert to default ('fit').
pub fn update_clip_fit_mode(
    conn: &Connection,
    clip_id: &str,
    mode: Option<&str>,
) -> SqliteResult<()> {
    conn.execute(
        "UPDATE clips SET cam_fit_mode = ?1 WHERE id = ?2",
        params![mode, clip_id],
    )?;
    Ok(())
}
```

- [ ] **Step 2.6: Patch existing `VodRow { ... }` and `ClipRow { ... }` literals (E0063 prevention)**

This is the step that v1.3.16's final review missed. Run:

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
```

Then use the Grep tool with pattern `VodRow \{` on `src-tauri/src/` (output_mode: `content`, -n true). Expected hits (post-revert state):

- `src-tauri/src/db.rs` — the struct definition line, plus the 3 row-binding `Ok(VodRow { ... })` closures (already patched in Step 2.4).
- `src-tauri/src/commands/vod.rs:2454` — inside `get_vods`, builds a `Vec<db::VodRow>` from Twitch API responses.
- `src-tauri/src/commands/vod.rs:3126` — inside `import_vod_by_url`, builds a single `db::VodRow` from URL-parsed metadata.

Patch BOTH `commands/vod.rs` literals. For `vod.rs:2454`, find the literal that ends with `game_name: None,` (or whatever the current last field is). Use Edit:

- **old_string** (the 5-line tail of the existing literal):
```
                created_at: now,
                download_progress: Some(0),
                analysis_progress: 0,
                game_name: None,
            }
```

- **new_string:**
```
                created_at: now,
                download_progress: Some(0),
                analysis_progress: 0,
                game_name: None,
                cam_region_norm: None,
            }
```

For `vod.rs:3126`, find the literal that ends with `game_name: None,` and the closing `};`. Use Edit:

- **old_string:**
```
        created_at: chrono::Utc::now().to_rfc3339(),
        download_progress: Some(0),
        analysis_progress: 0,
        game_name: None,
    };
```

- **new_string:**
```
        created_at: chrono::Utc::now().to_rfc3339(),
        download_progress: Some(0),
        analysis_progress: 0,
        game_name: None,
        cam_region_norm: None,
    };
```

Now Grep with pattern `ClipRow \{` on `src-tauri/src/`. Expected:

- `src-tauri/src/db.rs` — struct definition + 1+ `Ok(ClipRow { ... })` closures (already patched in Step 2.4).
- `src-tauri/src/commands/vod.rs:1401` — inside an analysis or clip-creation path, builds a `db::ClipRow { ... }`.

Patch `vod.rs:1401`. Read the existing literal first to find the exact last field, then Edit to insert the two new defaults BEFORE the closing `};`:

```rust
                            // existing fields...
                            publish_description: None,
                            publish_hashtags: None,
                            cam_region_norm_override: None,
                            cam_fit_mode: None,
                        };
```

(Field order matches `ClipRow` in `db.rs` — both new fields go at the end.)

- [ ] **Step 2.7: Static self-review**

Verify:

- Three `ALTER TABLE` migrations in `run_migrations`.
- One settings `INSERT OR IGNORE`.
- `VodRow` has new `cam_region_norm` field with `#[serde(default)]`.
- `ClipRow` has new `cam_region_norm_override` + `cam_fit_mode` fields with `#[serde(default)]`.
- `VOD_SELECT` constant present; all 3 `get_vods_*` functions use it via `format!`.
- `CLIP_SELECT` constant present; all `get_clips_*` / `get_clip_by_id` functions use it.
- Row-binding closures in `get_vods_*` read `row.get(16)?` for `cam_region_norm`.
- Row-binding closures in `get_clip_*` read `row.get(23)?` and `row.get(24)?` for the new clip fields.
- Three new `pub fn update_*` helpers exist.
- Every `VodRow { ... }` literal across `src-tauri/src/` has `cam_region_norm: None` populated.
- Every `ClipRow { ... }` literal across `src-tauri/src/` has BOTH new fields populated.

Run Grep with pattern `[‘’“”]` on `src-tauri/src/db.rs` and `src-tauri/src/commands/vod.rs` to confirm no smart quotes in token positions (smart quotes in `///` comments are fine; smart quotes as code delimiters are lexer-fatal).

- [ ] **Step 2.8: Commit**

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
git add src-tauri/src/db.rs src-tauri/src/commands/vod.rs
git commit -m "feat(db): cam_region columns + VOD_SELECT/CLIP_SELECT constants + struct extensions"
```

---

## Task 3: `cam_region.rs` — pure helpers + tests (TDD)

**Files:**
- Create: `src-tauri/src/cam_region.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod cam_region;` declaration)

This task is TDD with colocated `#[cfg(test)]` tests. All helpers are pure (no I/O, no DB) — perfect for unit testing.

- [ ] **Step 3.1: Write the full module with tests in one file**

Create `src-tauri/src/cam_region.rs` with the following content (entire file):

```rust
//! Pure helpers for the per-VOD cam region feature.
//!
//! - `CamRegion` — normalized (0..1) source-frame rectangle, parsed from
//!   JSON stored in `vods.cam_region_norm` / `clips.cam_region_norm_override`.
//! - `CamFitMode` — how the cropped source region maps into the cam slot.
//! - `resolve_effective_region` — applies the override/VOD/setting precedence.
//! - `to_crop_expr` — formats a `CamRegion` into the ffmpeg `crop=...` argument.
//!
//! No DB, no IPC, no ffmpeg invocation — those live in `commands/cam_region.rs`
//! and `vertical_crop.rs` respectively. This module is colocated-tested.

use serde::{Deserialize, Serialize};

/// Minimum allowed region dimension (5% of source frame). Anything smaller is
/// rejected at parse time — prevents accidental zero-size crops from breaking
/// the ffmpeg filter graph.
pub const MIN_REGION_DIM: f32 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CamRegion {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CamFitMode {
    Fit,
    Fill,
    Stretch,
}

impl Default for CamFitMode {
    fn default() -> Self {
        CamFitMode::Fit
    }
}

impl CamFitMode {
    /// Parse a DB string. NULL or unknown values default to Fit.
    pub fn from_db(s: Option<&str>) -> Self {
        match s.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("fill") => CamFitMode::Fill,
            Some("stretch") => CamFitMode::Stretch,
            _ => CamFitMode::Fit,
        }
    }

    /// Lowercase string for DB storage.
    pub fn as_db_str(self) -> &'static str {
        match self {
            CamFitMode::Fit => "fit",
            CamFitMode::Fill => "fill",
            CamFitMode::Stretch => "stretch",
        }
    }
}

impl CamRegion {
    /// Parse a JSON string like `{"x":0.12,"y":0.78,"w":0.22,"h":0.22}`.
    /// Clamps all four values to `[0.0, 1.0]`. Rejects regions where `w` or `h`
    /// is below `MIN_REGION_DIM` (returns None — caller should fall back to
    /// dup-source behavior).
    pub fn parse_norm_json(s: &str) -> Option<Self> {
        let mut r: CamRegion = serde_json::from_str(s).ok()?;
        r.x = r.x.clamp(0.0, 1.0);
        r.y = r.y.clamp(0.0, 1.0);
        r.w = r.w.clamp(0.0, 1.0);
        r.h = r.h.clamp(0.0, 1.0);
        if r.w < MIN_REGION_DIM || r.h < MIN_REGION_DIM {
            return None;
        }
        Some(r)
    }

    /// Serialize to the canonical JSON form for DB storage.
    pub fn to_norm_json(&self) -> String {
        // Hand-format to keep the JSON minimal and predictable (no scientific
        // notation, fixed 3-decimal precision). Easier to eyeball in the DB.
        format!(
            "{{\"x\":{:.3},\"y\":{:.3},\"w\":{:.3},\"h\":{:.3}}}",
            self.x, self.y, self.w, self.h
        )
    }

    /// Format the ffmpeg crop expression. Uses `iw`/`ih` so the resolver
    /// doesn't need to know the source resolution upfront.
    /// Example: `{x:0.12,y:0.78,w:0.22,h:0.22}` → `"iw*0.22:ih*0.22:iw*0.12:ih*0.78"`.
    pub fn to_crop_expr(&self) -> String {
        format!(
            "iw*{:.4}:ih*{:.4}:iw*{:.4}:ih*{:.4}",
            self.w, self.h, self.x, self.y
        )
    }
}

/// Decide which region (if any) to use at export time.
///
/// Precedence:
/// 1. If `allow_override` is true AND `clip_override_json` parses, use it.
/// 2. Else if `vod_region_json` parses, use it.
/// 3. Else None (export falls back to dup-source).
///
/// Invalid JSON in either field is silently ignored (logged at the caller).
pub fn resolve_effective_region(
    vod_region_json: Option<&str>,
    clip_override_json: Option<&str>,
    allow_override: bool,
) -> Option<CamRegion> {
    if allow_override {
        if let Some(json) = clip_override_json {
            if let Some(r) = CamRegion::parse_norm_json(json) {
                return Some(r);
            }
        }
    }
    if let Some(json) = vod_region_json {
        return CamRegion::parse_norm_json(json);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── CamRegion::parse_norm_json ──

    #[test]
    fn parse_valid_round_trips() {
        let r = CamRegion::parse_norm_json(r#"{"x":0.12,"y":0.78,"w":0.22,"h":0.22}"#).unwrap();
        assert_eq!(r, CamRegion { x: 0.12, y: 0.78, w: 0.22, h: 0.22 });
    }

    #[test]
    fn parse_invalid_json_returns_none() {
        assert!(CamRegion::parse_norm_json("not json").is_none());
        assert!(CamRegion::parse_norm_json("{ ").is_none());
        assert!(CamRegion::parse_norm_json("[]").is_none());
    }

    #[test]
    fn parse_missing_field_returns_none() {
        assert!(CamRegion::parse_norm_json(r#"{"x":0.1,"y":0.1,"w":0.5}"#).is_none());
    }

    #[test]
    fn parse_clamps_out_of_range_values() {
        let r = CamRegion::parse_norm_json(r#"{"x":-0.2,"y":1.5,"w":2.0,"h":-0.3}"#).unwrap();
        assert_eq!(r.x, 0.0, "negative x clamps to 0");
        assert_eq!(r.y, 1.0, "y > 1 clamps to 1");
        assert_eq!(r.w, 1.0, "w > 1 clamps to 1");
        assert!(r.h < MIN_REGION_DIM, "negative h clamps to 0 then fails min check... wait");
        // ↑ Actually, h clamped to 0 then below MIN_REGION_DIM → should return None.
        // Re-run with that expectation:
        assert!(CamRegion::parse_norm_json(r#"{"x":-0.2,"y":1.5,"w":2.0,"h":-0.3}"#).is_some()
            == false, "h<MIN_REGION_DIM after clamp must reject");
    }

    #[test]
    fn parse_rejects_below_min_dim() {
        // w just below 5%
        assert!(CamRegion::parse_norm_json(r#"{"x":0.0,"y":0.0,"w":0.04,"h":0.5}"#).is_none());
        // h just below 5%
        assert!(CamRegion::parse_norm_json(r#"{"x":0.0,"y":0.0,"w":0.5,"h":0.04}"#).is_none());
    }

    #[test]
    fn parse_accepts_exactly_min_dim() {
        assert!(CamRegion::parse_norm_json(r#"{"x":0.0,"y":0.0,"w":0.05,"h":0.05}"#).is_some());
    }

    // ── CamRegion::to_norm_json ──

    #[test]
    fn to_norm_json_canonical_form() {
        let r = CamRegion { x: 0.123456, y: 0.789, w: 0.25, h: 0.25 };
        assert_eq!(r.to_norm_json(), r#"{"x":0.123,"y":0.789,"w":0.250,"h":0.250}"#);
    }

    #[test]
    fn to_norm_json_round_trips() {
        let original = CamRegion { x: 0.1, y: 0.7, w: 0.25, h: 0.25 };
        let serialized = original.to_norm_json();
        let parsed = CamRegion::parse_norm_json(&serialized).unwrap();
        assert_eq!(parsed, original);
    }

    // ── CamRegion::to_crop_expr ──

    #[test]
    fn to_crop_expr_matches_spec_example() {
        let r = CamRegion { x: 0.12, y: 0.78, w: 0.22, h: 0.22 };
        assert_eq!(r.to_crop_expr(), "iw*0.2200:ih*0.2200:iw*0.1200:ih*0.7800");
    }

    #[test]
    fn to_crop_expr_uses_iw_ih_not_pixels() {
        let r = CamRegion { x: 0.5, y: 0.5, w: 0.5, h: 0.5 };
        let expr = r.to_crop_expr();
        assert!(expr.starts_with("iw*"), "must use iw multiplier: {expr}");
        assert!(expr.contains(":ih*"), "must use ih multiplier: {expr}");
    }

    // ── CamFitMode ──

    #[test]
    fn cam_fit_mode_from_db_defaults_to_fit() {
        assert_eq!(CamFitMode::from_db(None), CamFitMode::Fit);
        assert_eq!(CamFitMode::from_db(Some("")), CamFitMode::Fit);
        assert_eq!(CamFitMode::from_db(Some("xyz")), CamFitMode::Fit);
        assert_eq!(CamFitMode::from_db(Some("fit")), CamFitMode::Fit);
    }

    #[test]
    fn cam_fit_mode_from_db_parses_fill_stretch() {
        assert_eq!(CamFitMode::from_db(Some("fill")), CamFitMode::Fill);
        assert_eq!(CamFitMode::from_db(Some("STRETCH")), CamFitMode::Stretch);
        assert_eq!(CamFitMode::from_db(Some("  Fill  ")), CamFitMode::Fill);
    }

    #[test]
    fn cam_fit_mode_db_str_round_trips() {
        for m in [CamFitMode::Fit, CamFitMode::Fill, CamFitMode::Stretch] {
            assert_eq!(CamFitMode::from_db(Some(m.as_db_str())), m);
        }
    }

    // ── resolve_effective_region ──

    const SAMPLE_JSON_VOD: &str = r#"{"x":0.1,"y":0.7,"w":0.25,"h":0.25}"#;
    const SAMPLE_JSON_OVERRIDE: &str = r#"{"x":0.5,"y":0.5,"w":0.20,"h":0.20}"#;

    #[test]
    fn resolve_uses_vod_when_no_override() {
        let r = resolve_effective_region(Some(SAMPLE_JSON_VOD), None, true).unwrap();
        assert_eq!(r.x, 0.1);
    }

    #[test]
    fn resolve_uses_override_when_setting_on_and_override_set() {
        let r = resolve_effective_region(
            Some(SAMPLE_JSON_VOD),
            Some(SAMPLE_JSON_OVERRIDE),
            true,
        ).unwrap();
        assert_eq!(r.x, 0.5, "override should win");
    }

    #[test]
    fn resolve_ignores_override_when_setting_off() {
        let r = resolve_effective_region(
            Some(SAMPLE_JSON_VOD),
            Some(SAMPLE_JSON_OVERRIDE),
            false,
        ).unwrap();
        assert_eq!(r.x, 0.1, "override must be ignored when toggle off");
    }

    #[test]
    fn resolve_returns_none_when_nothing_set() {
        assert!(resolve_effective_region(None, None, true).is_none());
        assert!(resolve_effective_region(None, None, false).is_none());
        assert!(resolve_effective_region(None, Some(SAMPLE_JSON_OVERRIDE), false).is_none());
    }

    #[test]
    fn resolve_falls_back_to_vod_when_override_invalid() {
        let r = resolve_effective_region(
            Some(SAMPLE_JSON_VOD),
            Some("garbage"),
            true,
        ).unwrap();
        assert_eq!(r.x, 0.1);
    }
}
```

Wait — there's a buggy assertion in `parse_clamps_out_of_range_values`. Let me clean that up before saving. The test mixes two cases. Replace that single test with two clearer ones:

```rust
    #[test]
    fn parse_clamps_positive_out_of_range() {
        let r = CamRegion::parse_norm_json(r#"{"x":1.5,"y":1.5,"w":0.5,"h":0.5}"#).unwrap();
        assert_eq!(r.x, 1.0);
        assert_eq!(r.y, 1.0);
    }

    #[test]
    fn parse_clamps_negative_then_min_check_rejects() {
        // Negative h clamps to 0.0, which is below MIN_REGION_DIM → reject.
        assert!(CamRegion::parse_norm_json(r#"{"x":0.0,"y":0.0,"w":0.5,"h":-0.3}"#).is_none());
    }
```

Save the entire file with the two replacement tests substituted for the buggy one. The full final test count is 16.

- [ ] **Step 3.2: Register the module in lib.rs**

In `src-tauri/src/lib.rs`, find the section where module declarations live (top of file, before the `tauri::Builder::default()` call). The existing modules are declared like `mod db;` `mod bin_manager;` etc. Add ONE line in alphabetical order:

```rust
mod cam_region;
```

- [ ] **Step 3.3: Add `serde_json` dependency check**

Run:

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral/src-tauri"
grep "^serde_json" Cargo.toml
```

Expected output: a line like `serde_json = "1"` or `serde_json = { version = "1", ... }`. This crate is required for `CamRegion::parse_norm_json`. If it's NOT there, STOP and surface — adding a dep needs Slug's review.

(`serde_json` is almost certainly already in the dependency tree because `tauri` depends on it transitively, but let's confirm it's a direct dep.)

- [ ] **Step 3.4: Static self-review**

- All 16 tests defined inside `#[cfg(test)] mod tests`.
- No `dbg!`, `println!`, or smart quotes in token positions.
- `pub use` is NOT needed — `crate::cam_region::CamRegion` is the canonical path; other modules import with `use crate::cam_region::CamRegion;`.
- The file is self-contained — no external function calls except `serde_json::from_str` and standard library.

Run Grep with pattern `[‘’“”]` on `src-tauri/src/cam_region.rs`. Must be empty.

- [ ] **Step 3.5: Commit**

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
git add src-tauri/src/cam_region.rs src-tauri/src/lib.rs
git commit -m "feat(cam_region): CamRegion + CamFitMode + resolver + crop_expr (TDD, 16 tests)"
```

---

## Task 4: `commands/cam_region.rs` — Tauri commands + handler registration

**Files:**
- Create: `src-tauri/src/commands/cam_region.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs` (add command imports + handler registrations)

- [ ] **Step 4.1: Write the commands module**

Create `src-tauri/src/commands/cam_region.rs` with this content:

```rust
//! Tauri commands for setting/clearing the per-VOD cam region, per-clip
//! override, fit mode, and the global allow-per-clip-override toggle.

use serde::Deserialize;
use tauri::State;

use crate::cam_region::{CamFitMode, CamRegion};
use crate::db;
use crate::DbConn;

#[derive(Debug, Deserialize)]
pub struct RegionInput {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RegionInput {
    fn to_region(&self) -> Option<CamRegion> {
        let r = CamRegion { x: self.x, y: self.y, w: self.w, h: self.h };
        // Round-trip through the parser to apply clamping + MIN_REGION_DIM rejection.
        CamRegion::parse_norm_json(&r.to_norm_json())
    }
}

/// Set the VOD-level cam region. Frontend passes the dragged rect as
/// `{ x, y, w, h }` in normalized 0..1 source-frame coords.
#[tauri::command]
pub async fn set_vod_cam_region(
    vod_id: String,
    region: RegionInput,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let r = region.to_region().ok_or_else(|| {
        "Region rejected: out of range or smaller than 5% × 5%".to_string()
    })?;
    let json = r.to_norm_json();
    let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
    db::update_vod_cam_region(&conn, &vod_id, Some(&json))
        .map_err(|e| format!("DB error: {e}"))
}

/// Clear the VOD-level cam region (NULL it out). Falls back to dup-source export.
#[tauri::command]
pub async fn clear_vod_cam_region(
    vod_id: String,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
    db::update_vod_cam_region(&conn, &vod_id, None)
        .map_err(|e| format!("DB error: {e}"))
}

/// Set a per-clip cam region override. Only honored when the
/// `allow_per_clip_cam_region_override` setting is true.
#[tauri::command]
pub async fn set_clip_cam_region_override(
    clip_id: String,
    region: RegionInput,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let r = region.to_region().ok_or_else(|| {
        "Region rejected: out of range or smaller than 5% × 5%".to_string()
    })?;
    let json = r.to_norm_json();
    let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
    db::update_clip_cam_region_override(&conn, &clip_id, Some(&json))
        .map_err(|e| format!("DB error: {e}"))
}

/// Clear the per-clip override; clip will fall back to its VOD's region
/// (or dup-source if the VOD has no region either).
#[tauri::command]
pub async fn clear_clip_cam_region_override(
    clip_id: String,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
    db::update_clip_cam_region_override(&conn, &clip_id, None)
        .map_err(|e| format!("DB error: {e}"))
}

/// Set the per-clip fit mode. Accepts 'fit', 'fill', or 'stretch'.
/// Unknown values are accepted-but-treated-as-fit at read time, so we don't
/// need to enforce here — but we normalize via CamFitMode for consistency.
#[tauri::command]
pub async fn set_clip_fit_mode(
    clip_id: String,
    mode: String,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let normalized = CamFitMode::from_db(Some(&mode)).as_db_str();
    let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
    db::update_clip_fit_mode(&conn, &clip_id, Some(normalized))
        .map_err(|e| format!("DB error: {e}"))
}

/// Toggle the global `allow_per_clip_cam_region_override` setting.
/// Stored in the existing `settings` k/v table as the string "true" or "false".
#[tauri::command]
pub async fn set_allow_per_clip_override(
    enabled: bool,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let val = if enabled { "true" } else { "false" };
    let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
    db::save_setting(&conn, "allow_per_clip_cam_region_override", val)
        .map_err(|e| format!("DB error: {e}"))
}

/// Read the global setting. Frontend calls this once on Editor mount to know
/// whether to render the per-clip override sub-row.
#[tauri::command]
pub async fn get_allow_per_clip_override(
    db: State<'_, DbConn>,
) -> Result<bool, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
    let val = db::get_setting(&conn, "allow_per_clip_cam_region_override")
        .map_err(|e| format!("DB error: {e}"))?;
    Ok(matches!(val.as_deref(), Some("true")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_input_round_trips_through_clamp() {
        // Valid region passes through.
        let r = RegionInput { x: 0.1, y: 0.7, w: 0.25, h: 0.25 };
        assert!(r.to_region().is_some());
    }

    #[test]
    fn region_input_below_min_dim_returns_none() {
        // 4% width should be rejected.
        let r = RegionInput { x: 0.0, y: 0.0, w: 0.04, h: 0.5 };
        assert!(r.to_region().is_none());
    }

    #[test]
    fn region_input_out_of_range_clamps_then_rejects_if_too_small() {
        // Negative h clamps to 0 then rejects (below 5% min).
        let r = RegionInput { x: 0.0, y: 0.0, w: 0.5, h: -0.1 };
        assert!(r.to_region().is_none());
    }
}
```

- [ ] **Step 4.2: Declare the module**

In `src-tauri/src/commands/mod.rs`, add:

```rust
pub mod cam_region;
```

(Alphabetical order in the existing module list.)

- [ ] **Step 4.3: Register the commands in lib.rs**

In `src-tauri/src/lib.rs`, find the `use commands::...` import block. Add:

```rust
use commands::cam_region::{
    set_vod_cam_region, clear_vod_cam_region,
    set_clip_cam_region_override, clear_clip_cam_region_override,
    set_clip_fit_mode, set_allow_per_clip_override, get_allow_per_clip_override,
};
```

Find the `tauri::generate_handler![ ... ]` macro call. Add the seven new commands to the list (anywhere — order doesn't matter for the macro). Insert after the last existing handler entry:

```rust
        set_vod_cam_region,
        clear_vod_cam_region,
        set_clip_cam_region_override,
        clear_clip_cam_region_override,
        set_clip_fit_mode,
        set_allow_per_clip_override,
        get_allow_per_clip_override,
```

- [ ] **Step 4.4: Static self-review**

- 7 Tauri commands defined.
- 3 unit tests for `RegionInput::to_region`.
- All commands return `Result<_, String>` and use `db.lock().map_err(...)` pattern (matches existing convention from `commands/cam_asset.rs` of the now-reverted v1.3.16 work).
- Module declared in `commands/mod.rs`.
- All 7 commands imported + listed in `generate_handler!` in `lib.rs`.
- Unicode sweep: Grep `[‘’“”]` on the new file and on `lib.rs` for changed regions.

- [ ] **Step 4.5: Commit**

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
git add src-tauri/src/commands/cam_region.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git commit -m "feat(commands): cam_region module — set/clear/override/fit/toggle + handler reg"
```

---

## Task 5: `vertical_crop::layout_filter_with_region` + ExportRequest extension + tests

**Files:**
- Modify: `src-tauri/src/vertical_crop.rs`

This task adds the new filter-graph builder for cam-region cases and extends `ExportRequest`. The existing `layout_filter` stays byte-unchanged (regression-safety guarantee).

- [ ] **Step 5.1: Write failing unit tests first (TDD)**

Open `src-tauri/src/vertical_crop.rs`. Find the existing `#[cfg(test)] mod tests` block at the bottom. Append these tests inside the existing module:

```rust
    use crate::cam_region::{CamRegion, CamFitMode};

    fn sample_region() -> CamRegion {
        CamRegion { x: 0.12, y: 0.78, w: 0.22, h: 0.22 }
    }

    #[test]
    fn layout_filter_with_region_no_region_byte_identical_to_layout_filter() {
        let target = OutputSize { width: 1080, height: 1920 };
        let modes = [
            LayoutMode::GameplayFocus,
            LayoutMode::Split { ratio: 0.6 },
            LayoutMode::Pip { x: 0.93, y: 0.93, size: 0.3 },
        ];
        for m in &modes {
            let (old_f, old_c) = layout_filter(m, target, None);
            let (new_f, new_c) = layout_filter_with_region(m, target, None, None, CamFitMode::Fit);
            assert_eq!(old_f, new_f, "no-region path must be byte-identical for {m:?}");
            assert_eq!(old_c, new_c);
        }
    }

    #[test]
    fn layout_filter_with_region_pip_uses_split2_and_crop_expr() {
        let target = OutputSize { width: 1080, height: 1920 };
        let mode = LayoutMode::Pip { x: 0.93, y: 0.93, size: 0.3 };
        let (f, complex) = layout_filter_with_region(&mode, target, None, Some(sample_region()), CamFitMode::Fit);
        assert!(complex, "PiP+region must be filter_complex");
        assert!(f.contains("[0:v]split=2"), "must split source: {f}");
        assert!(f.contains("crop=iw*0.2200:ih*0.2200:iw*0.1200:ih*0.7800"), "region crop expr missing: {f}");
        assert!(!f.contains("[1:v]"), "must NOT reference second input — single-input feature: {f}");
        assert!(f.ends_with("[out]"));
    }

    #[test]
    fn layout_filter_with_region_pip_passthrough_centers_cam_in_slot() {
        let target = OutputSize { width: 1080, height: 1920 };
        let mode = LayoutMode::Pip { x: 0.93, y: 0.93, size: 0.3 };
        let (f, _) = layout_filter_with_region(&mode, target, None, Some(sample_region()), CamFitMode::Fit);
        // Center expression in overlay arg: SLOT_X+(SLOT_W-w)/2 form.
        assert!(f.contains("+(") && f.contains("-w)/2"), "centering expression in overlay: {f}");
    }

    #[test]
    fn layout_filter_with_region_split_uses_split3_with_boxblur() {
        let target = OutputSize { width: 1080, height: 1920 };
        let mode = LayoutMode::Split { ratio: 0.6 };
        let (f, complex) = layout_filter_with_region(&mode, target, None, Some(sample_region()), CamFitMode::Fit);
        assert!(complex);
        assert!(f.contains("[0:v]split=3"), "Split+region must split into 3 branches: {f}");
        assert!(f.contains("boxblur=20:5"), "blurred backdrop branch missing: {f}");
        assert!(f.contains("vstack"), "Split must vstack top+bottom: {f}");
        assert!(f.ends_with("[out]"));
    }

    #[test]
    fn layout_filter_with_region_fit_mode_fill_uses_increase_then_crop() {
        let target = OutputSize { width: 1080, height: 1920 };
        let mode = LayoutMode::Pip { x: 0.93, y: 0.93, size: 0.3 };
        let (f_fit, _) = layout_filter_with_region(&mode, target, None, Some(sample_region()), CamFitMode::Fit);
        let (f_fill, _) = layout_filter_with_region(&mode, target, None, Some(sample_region()), CamFitMode::Fill);
        assert!(f_fit.contains("force_original_aspect_ratio=decrease"), "Fit uses decrease: {f_fit}");
        assert!(f_fill.contains("force_original_aspect_ratio=increase"), "Fill uses increase: {f_fill}");
    }

    #[test]
    fn layout_filter_with_region_fit_mode_stretch_drops_aspect_clause() {
        let target = OutputSize { width: 1080, height: 1920 };
        let mode = LayoutMode::Pip { x: 0.93, y: 0.93, size: 0.3 };
        let (f, _) = layout_filter_with_region(&mode, target, None, Some(sample_region()), CamFitMode::Stretch);
        assert!(!f.contains("force_original_aspect_ratio="), "Stretch must omit aspect-ratio clause: {f}");
    }

    #[test]
    fn layout_filter_with_region_gameplay_focus_ignores_region() {
        let target = OutputSize { width: 1080, height: 1920 };
        let (f_no, _) = layout_filter_with_region(&LayoutMode::GameplayFocus, target, None, None, CamFitMode::Fit);
        let (f_yes, _) = layout_filter_with_region(&LayoutMode::GameplayFocus, target, None, Some(sample_region()), CamFitMode::Fit);
        assert_eq!(f_no, f_yes, "GameplayFocus has no cam slot — region must be irrelevant");
    }

    #[test]
    fn layout_filter_with_region_caption_filter_appended() {
        let target = OutputSize { width: 1080, height: 1920 };
        let mode = LayoutMode::Pip { x: 0.93, y: 0.93, size: 0.3 };
        let caption = "drawtext=text='hi'";
        let (f, _) = layout_filter_with_region(&mode, target, Some(caption), Some(sample_region()), CamFitMode::Fit);
        assert!(f.contains(caption), "caption filter must be embedded: {f}");
        assert!(f.ends_with("[out]"));
    }
```

- [ ] **Step 5.2: (Slug-deferred) Note expected failure**

Implementer does NOT run cargo. Slug will see: `cannot find function 'layout_filter_with_region'` plus `unresolved import 'crate::cam_region::CamRegion'` in tests — both expected before Step 5.3 lands.

- [ ] **Step 5.3: Implement `layout_filter_with_region`**

Find the existing `pub fn layout_filter(mode, target, caption_filter) -> (String, bool)`. Immediately AFTER it (preserving the existing function byte-unchanged), add:

```rust
/// Layout filter builder that incorporates a per-VOD cam region (cropped
/// from the source frame, not an external file).
///
/// When `region` is `None`, this delegates to `layout_filter` so the no-region
/// path is byte-identical to the existing pre-feature behavior.
///
/// When `region` is `Some`:
/// - `GameplayFocus`: no cam slot, region irrelevant (delegates).
/// - `Pip`: source split into 2 branches; gameplay fills output; cam branch
///   is cropped to the region, scaled per `fit_mode`, overlaid at the slot's
///   position+size, centered within the slot box. No slot rectangle drawn,
///   so gameplay shows through where the cam doesn't fill the slot.
/// - `Split`: source split into 3 branches; gameplay fills the top region;
///   cam_blur branch covers the bottom slot (heavy boxblur); cam_sharp branch
///   is centered on top of the blur at its fit-mode-determined size.
pub fn layout_filter_with_region(
    mode: &LayoutMode,
    target: OutputSize,
    caption_filter: Option<&str>,
    region: Option<crate::cam_region::CamRegion>,
    fit_mode: crate::cam_region::CamFitMode,
) -> (String, bool) {
    let region = match region {
        Some(r) => r,
        None => return layout_filter(mode, target, caption_filter),
    };

    let tw = target.width;
    let th = target.height;
    let crop_expr = region.to_crop_expr();

    match mode {
        LayoutMode::GameplayFocus => {
            // No cam slot; region irrelevant.
            layout_filter(mode, target, caption_filter)
        }

        LayoutMode::Pip { x, y, size } => {
            let ps = (tw as f64 * size.clamp(0.15, 0.45)) as u32;
            let slot_x = ((tw as f64 - ps as f64) * x.clamp(0.0, 1.0)) as u32;
            let slot_y = ((th as f64 - ps as f64) * y.clamp(0.0, 1.0)) as u32;
            let scale_expr = fit_scale_expr(ps, ps, fit_mode);

            let mut f = format!(
                "[0:v]split=2[gp_src][cam_src];\
                 [gp_src]scale={tw}:{th}:force_original_aspect_ratio=increase:flags=lanczos,crop={tw}:{th}[main];\
                 [cam_src]crop={crop_expr},{scale_expr}[cam];\
                 [main][cam]overlay={slot_x}+({ps}-w)/2:{slot_y}+({ps}-h)/2"
            );
            if let Some(cf) = caption_filter {
                f.push_str(&format!("[overlaid];[overlaid]{cf}[out]"));
            } else {
                f.push_str("[out]");
            }
            (f, true)
        }

        LayoutMode::Split { ratio } => {
            let r = ratio.clamp(0.3, 0.8);
            let th_top = (th as f64 * r) as u32;
            let th_bot = th - th_top;
            let scale_expr_bot = fit_scale_expr(tw, th_bot, fit_mode);

            let mut f = format!(
                "[0:v]split=3[gp_src][cam_blur_src][cam_sharp_src];\
                 [gp_src]scale={tw}:{th_top}:force_original_aspect_ratio=increase:flags=lanczos,crop={tw}:{th_top}[top];\
                 [cam_blur_src]crop={crop_expr},scale={tw}:{th_bot}:force_original_aspect_ratio=increase:flags=lanczos,crop={tw}:{th_bot},boxblur=20:5[blur_bg];\
                 [cam_sharp_src]crop={crop_expr},{scale_expr_bot}[sharp_fg];\
                 [blur_bg][sharp_fg]overlay=(W-w)/2:(H-h)/2[bottom];\
                 [top][bottom]vstack"
            );
            if let Some(cf) = caption_filter {
                f.push_str(&format!("[stacked];[stacked]{cf}[out]"));
            } else {
                f.push_str("[out]");
            }
            (f, true)
        }
    }
}

/// Build the `<FIT_SCALE>` substring for the given fit mode + target dimensions.
/// Returns ffmpeg filter snippet WITHOUT a trailing semicolon — callers chain it.
fn fit_scale_expr(w: u32, h: u32, mode: crate::cam_region::CamFitMode) -> String {
    use crate::cam_region::CamFitMode;
    match mode {
        CamFitMode::Fit => format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease:flags=lanczos"
        ),
        CamFitMode::Fill => format!(
            "scale={w}:{h}:force_original_aspect_ratio=increase:flags=lanczos,crop={w}:{h}"
        ),
        CamFitMode::Stretch => format!(
            "scale={w}:{h}:flags=lanczos"
        ),
    }
}
```

- [ ] **Step 5.4: Extend `ExportRequest`**

Find `pub struct ExportRequest`. Add TWO new fields at the END (before the closing `}`):

```rust
    /// Optional source-frame region to use as the cam slot's content.
    /// `None` falls back to the existing dup-source layout filter.
    pub effective_region: Option<crate::cam_region::CamRegion>,
    /// Fit mode for the region within the cam slot. Defaults to Fit.
    pub fit_mode: crate::cam_region::CamFitMode,
```

- [ ] **Step 5.5: Wire ExportRequest fields into `build_ffmpeg_command` and `run_export`**

Find `pub fn build_ffmpeg_command(...)`. The first statement currently is:

```rust
    let caption = request.caption_filter.as_deref();
    let (filter, is_complex) = layout_filter(&request.layout, request.target, caption);
```

Replace with:

```rust
    let caption = request.caption_filter.as_deref();
    let (filter, is_complex) = layout_filter_with_region(
        &request.layout,
        request.target,
        caption,
        request.effective_region,
        request.fit_mode,
    );
```

Find `pub fn run_export(...)`. It has the same `let (filter, is_complex) = layout_filter(...)` pattern. Apply the identical replacement there too.

NOTE: there is no second `-i` flag to add (single-input feature). The existing input flags stay unchanged. This is one less footgun than the v1.3.16 work.

- [ ] **Step 5.6: Update any in-file `ExportRequest { ... }` literals**

Run Grep with pattern `ExportRequest \{` on `src-tauri/src/vertical_crop.rs`. Expected: 2 hits (struct def + the `sample_request()` test helper). Find the literal in `sample_request()` and add the two new fields at the end:

```rust
            cam_asset_path: None,   // ← old field from v1.3.16, should NOT exist post-revert
            effective_region: None,
            fit_mode: crate::cam_region::CamFitMode::Fit,
```

Wait — `cam_asset_path` was removed in the Task 1 revert. So the literal should already be back to its pre-v1.3.16 state without `cam_asset_path`. Just add the two new fields at the end:

```rust
            effective_region: None,
            fit_mode: crate::cam_region::CamFitMode::Fit,
```

Also Grep on `src-tauri/src/commands/export.rs` — there's one `ExportRequest { ... }` literal in `clip_to_export_request` (~line 304). Leave it alone for Task 6 (it'll be updated there).

- [ ] **Step 5.7: Static self-review**

- `layout_filter` is BYTE-UNCHANGED.
- `layout_filter_with_region` exists immediately after, delegates when `region == None`, has correct 3 branches.
- `fit_scale_expr` helper is private (no `pub`).
- `ExportRequest` has TWO new fields (`effective_region`, `fit_mode`) at the end.
- `build_ffmpeg_command` and `run_export` both call `layout_filter_with_region`.
- `sample_request()` test helper populates both new fields.
- All 8 new tests are inside the existing `mod tests`.
- Grep `[‘’“”]` on `vertical_crop.rs` — must show only pre-existing matches inside `///` doc-comments (lexer-safe), nothing in token positions.

- [ ] **Step 5.8: Commit**

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
git add src-tauri/src/vertical_crop.rs
git commit -m "feat(vertical_crop): layout_filter_with_region + ExportRequest extension (TDD, 8 tests)"
```

---

## Task 6: Wire region resolver into `clip_to_export_request`

**Files:**
- Modify: `src-tauri/src/commands/export.rs`

- [ ] **Step 6.1: Update `clip_to_export_request` signature + body**

Find `fn clip_to_export_request(clip, vod_path, output_path) -> vertical_crop::ExportRequest` (around line 289 in the post-revert state). Change the signature to:

```rust
fn clip_to_export_request(
    clip: &db::ClipRow,
    vod: &db::VodRow,
    vod_path: &str,
    output_path: &std::path::Path,
    allow_per_clip_override: bool,
) -> vertical_crop::ExportRequest {
```

(We now take the full `VodRow` instead of just `vod_path` so the function can read `vod.cam_region_norm`.)

Inside the function body, before the existing struct literal:

```rust
    // Resolve effective region using override precedence + setting toggle.
    let effective_region = crate::cam_region::resolve_effective_region(
        vod.cam_region_norm.as_deref(),
        clip.cam_region_norm_override.as_deref(),
        allow_per_clip_override,
    );
    let fit_mode = crate::cam_region::CamFitMode::from_db(clip.cam_fit_mode.as_deref());
```

Then extend the struct literal at the end with the two new fields:

```rust
    vertical_crop::ExportRequest {
        source_path: std::path::PathBuf::from(vod_path),
        output_path: output_path.to_path_buf(),
        start: clip.start_seconds,
        end: clip.end_seconds,
        platform,
        target,
        layout,
        caption_filter,
        effective_region,
        fit_mode,
    }
```

- [ ] **Step 6.2: Update both call sites to pass the full VodRow + allow-override flag**

Run Grep with pattern `clip_to_export_request` on `src-tauri/src/commands/export.rs`. Expected: 1 definition + 2 callers. Both callers need updating.

For **`export_clip` (around line 127)**: the inner block currently destructures `(clip, vod_path)`. Change it to also pull out the full `vod` row and the allow-override setting:

```rust
    let (clip, vod, vod_path, allow_override) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let clip = db::get_clip_by_id(&conn, &clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("Clip not found")?;
        let vod = db::get_vod_by_id(&conn, &clip.vod_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("VOD not found")?;
        let path = vod.local_path.clone().ok_or("VOD not downloaded — download it first to export clips")?;
        let allow = matches!(
            db::get_setting(&conn, "allow_per_clip_cam_region_override")
                .ok()
                .flatten()
                .as_deref(),
            Some("true"),
        );
        (clip, vod, path, allow)
    };
```

(Note `.clone()` on `vod.local_path` because we keep `vod` alive after pulling `path` out.)

Then update the call:

```rust
        let request = clip_to_export_request(&clip, &vod, &vod_path, &output_path, allow_override);
```

For **`render_clip_by_id` (around line 221)**: same pattern. The current destructure:

```rust
    let (clip, vod_path) = {
        let db_path = db::db_path()...
        // ...
        let path = vod.local_path.ok_or_else(...)?;
        (clip, path)
    };
```

Becomes:

```rust
    let (clip, vod, vod_path, allow_override) = {
        let db_path = db::db_path().map_err(|e| format!("DB path: {}", e))?;
        let conn = rusqlite::Connection::open(&db_path)
            .map_err(|e| format!("DB open: {}", e))?;
        let clip = db::get_clip_by_id(&conn, clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or_else(|| "Clip not found".to_string())?;
        let vod = db::get_vod_by_id(&conn, &clip.vod_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or_else(|| "VOD not found".to_string())?;
        let path = vod.local_path.clone().ok_or_else(|| "VOD not downloaded".to_string())?;
        let allow = matches!(
            db::get_setting(&conn, "allow_per_clip_cam_region_override")
                .ok()
                .flatten()
                .as_deref(),
            Some("true"),
        );
        (clip, vod, path, allow)
    };
```

And the call:

```rust
    let request = clip_to_export_request(&clip, &vod, &vod_path, &output_path, allow_override);
```

- [ ] **Step 6.3: Static self-review**

- `clip_to_export_request` signature has 5 args including `vod: &db::VodRow` and `allow_per_clip_override: bool`.
- Function body resolves `effective_region` + `fit_mode` before building the struct.
- Struct literal has both new fields populated.
- Both call sites pass the full `vod` + `allow_override`.
- `vod` is kept alive across the `vod.local_path` consumption via `.clone()`.
- Both inner blocks return 4-tuples with the new vars.
- No DB re-queries (the existing `get_vod_by_id` is reused; `get_setting` adds one extra query per export, acceptable cost).
- Unicode sweep clean.

- [ ] **Step 6.4: Commit**

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
git add src-tauri/src/commands/export.rs
git commit -m "feat(export): resolve effective cam region in clip_to_export_request"
```

---

## Task 7: Frontend Vod + Clip type extensions

**Files:**
- Modify: `src/types.ts`

- [ ] **Step 7.1: Add three optional fields**

In `src/types.ts`, find the `Vod` interface. After the existing `game_name: string | null;` field (last one before the closing brace), use Edit:

- **old_string:**
```
  local_path: string | null;
  game_name: string | null;
}
```

- **new_string:**
```
  local_path: string | null;
  game_name: string | null;
  cam_region_norm: string | null;
}
```

Find the `Clip` interface. Add TWO new fields after the existing `publish_hashtags: string | null;` (the current last field). Use Edit:

- **old_string:**
```
  publish_description: string | null;
  publish_hashtags: string | null;
}
```

- **new_string:**
```
  publish_description: string | null;
  publish_hashtags: string | null;
  cam_region_norm_override: string | null;
  cam_fit_mode: 'fit' | 'fill' | 'stretch' | null;
}
```

- [ ] **Step 7.2: Verify the build**

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
npm run build
```

Expected: `tsc -b && vite build` clean. No `Vod` / `Clip` literal constructions in `src/` need patching (verified pre-revert: only `invoke<Vod>()` / `invoke<Clip>()` typed returns are used downstream, plus `Partial<Vod>` in `appStore` which auto-allows missing fields).

- [ ] **Step 7.3: Commit**

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
git add src/types.ts
git commit -m "feat(types): Vod.cam_region_norm + Clip.cam_region_norm_override + Clip.cam_fit_mode"
```

---

## Task 8: `CamRegionSetter` component (drag overlay on source player)

**Files:**
- Create: `src/components/CamRegionSetter.tsx`

This is the draggable rectangle overlay that appears on the source player when in region-edit mode.

- [ ] **Step 8.1: Write the component**

Create `src/components/CamRegionSetter.tsx` with this content:

```tsx
import { useEffect, useRef, useState } from 'react'

export type RegionNorm = { x: number; y: number; w: number; h: number }

type Props = {
  /** Initial region (normalized 0..1). Used as starting position when entering edit mode. */
  initial: RegionNorm
  /** The bounding rect of the underlying source video element (in CSS px). */
  containerRect: DOMRect
  /** Fired on every drag/resize while the user is interacting (no DB write yet). */
  onChange: (r: RegionNorm) => void
  /** Fired when user clicks Save. */
  onSave: (r: RegionNorm) => void
  /** Fired when user clicks Cancel or presses Esc. */
  onCancel: () => void
}

const MIN_DIM_NORM = 0.05  // matches Rust MIN_REGION_DIM

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v))

type Handle = 'move' | 'tl' | 'tr' | 'bl' | 'br' | 't' | 'b' | 'l' | 'r'

export default function CamRegionSetter({ initial, containerRect, onChange, onSave, onCancel }: Props) {
  const [region, setRegion] = useState<RegionNorm>(initial)
  const dragRef = useRef<{ handle: Handle; startX: number; startY: number; startR: RegionNorm } | null>(null)

  // Esc → cancel
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel()
      if (e.key === 'Enter') onSave(region)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [region, onSave, onCancel])

  // Pixel rect of the region inside the container.
  const px = {
    x: region.x * containerRect.width,
    y: region.y * containerRect.height,
    w: region.w * containerRect.width,
    h: region.h * containerRect.height,
  }

  const onMouseDown = (handle: Handle) => (e: React.MouseEvent) => {
    e.preventDefault()
    e.stopPropagation()
    dragRef.current = { handle, startX: e.clientX, startY: e.clientY, startR: region }
  }

  // Listen for mousemove + mouseup on window so the drag continues outside the overlay.
  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (!dragRef.current) return
      const { handle, startX, startY, startR } = dragRef.current
      const dxNorm = (e.clientX - startX) / containerRect.width
      const dyNorm = (e.clientY - startY) / containerRect.height
      let { x, y, w, h } = startR
      switch (handle) {
        case 'move':
          x = clamp(startR.x + dxNorm, 0, 1 - startR.w)
          y = clamp(startR.y + dyNorm, 0, 1 - startR.h)
          break
        case 'tl': x = clamp(startR.x + dxNorm, 0, startR.x + startR.w - MIN_DIM_NORM); y = clamp(startR.y + dyNorm, 0, startR.y + startR.h - MIN_DIM_NORM); w = startR.x + startR.w - x; h = startR.y + startR.h - y; break
        case 'tr': y = clamp(startR.y + dyNorm, 0, startR.y + startR.h - MIN_DIM_NORM); w = clamp(startR.w + dxNorm, MIN_DIM_NORM, 1 - startR.x); h = startR.y + startR.h - y; break
        case 'bl': x = clamp(startR.x + dxNorm, 0, startR.x + startR.w - MIN_DIM_NORM); w = startR.x + startR.w - x; h = clamp(startR.h + dyNorm, MIN_DIM_NORM, 1 - startR.y); break
        case 'br': w = clamp(startR.w + dxNorm, MIN_DIM_NORM, 1 - startR.x); h = clamp(startR.h + dyNorm, MIN_DIM_NORM, 1 - startR.y); break
        case 't':  y = clamp(startR.y + dyNorm, 0, startR.y + startR.h - MIN_DIM_NORM); h = startR.y + startR.h - y; break
        case 'b':  h = clamp(startR.h + dyNorm, MIN_DIM_NORM, 1 - startR.y); break
        case 'l':  x = clamp(startR.x + dxNorm, 0, startR.x + startR.w - MIN_DIM_NORM); w = startR.x + startR.w - x; break
        case 'r':  w = clamp(startR.w + dxNorm, MIN_DIM_NORM, 1 - startR.x); break
      }
      const next = { x, y, w, h }
      setRegion(next)
      onChange(next)
    }
    const onUp = () => { dragRef.current = null }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    return () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
  }, [containerRect, onChange])

  // Overlay positioned absolutely over the source player container.
  return (
    <>
      {/* Dim layer outside the region — gives focus to the picked area */}
      <div className="absolute inset-0 pointer-events-none" style={{
        background: `linear-gradient(to right, rgba(0,0,0,0.45) 0, rgba(0,0,0,0.45) ${px.x}px, transparent ${px.x}px, transparent ${px.x + px.w}px, rgba(0,0,0,0.45) ${px.x + px.w}px)`,
      }} />
      {/* Top/bottom dim bands */}
      <div className="absolute pointer-events-none" style={{ left: px.x, top: 0, width: px.w, height: px.y, background: 'rgba(0,0,0,0.45)' }} />
      <div className="absolute pointer-events-none" style={{ left: px.x, top: px.y + px.h, width: px.w, bottom: 0, background: 'rgba(0,0,0,0.45)' }} />

      {/* The draggable rectangle itself */}
      <div
        className="absolute border-2 border-violet-400 bg-violet-400/10 cursor-move"
        style={{ left: px.x, top: px.y, width: px.w, height: px.h }}
        onMouseDown={onMouseDown('move')}
      >
        {/* Corner handles */}
        {(['tl','tr','bl','br'] as const).map(h => (
          <div
            key={h}
            className="absolute w-3 h-3 bg-violet-400 border border-white"
            style={{
              cursor: (h === 'tl' || h === 'br') ? 'nwse-resize' : 'nesw-resize',
              left: h.includes('l') ? -6 : undefined,
              right: h.includes('r') ? -6 : undefined,
              top: h.includes('t') ? -6 : undefined,
              bottom: h.includes('b') ? -6 : undefined,
            }}
            onMouseDown={onMouseDown(h)}
          />
        ))}
        {/* Edge handles */}
        {(['t','b','l','r'] as const).map(h => (
          <div
            key={h}
            className="absolute bg-violet-400/40"
            style={{
              cursor: (h === 't' || h === 'b') ? 'ns-resize' : 'ew-resize',
              top: h === 't' ? -3 : h === 'b' ? undefined : '20%',
              bottom: h === 'b' ? -3 : undefined,
              left: h === 'l' ? -3 : h === 'r' ? undefined : '20%',
              right: h === 'r' ? -3 : undefined,
              width: (h === 't' || h === 'b') ? '60%' : 6,
              height: (h === 'l' || h === 'r') ? '60%' : 6,
            }}
            onMouseDown={onMouseDown(h)}
          />
        ))}
      </div>

      {/* Save / Cancel toolbar pinned to bottom of the player container */}
      <div className="absolute left-0 right-0 bottom-0 px-3 py-2 bg-black/70 flex items-center gap-3 text-xs text-slate-200 z-10">
        <span className="text-violet-300">Drag the rectangle on the source. Press Enter to save, Esc to cancel.</span>
        <span className="ml-auto font-mono text-[10px] text-slate-400">
          {Math.round(region.x * 100)}%, {Math.round(region.y * 100)}% · {Math.round(region.w * 100)}×{Math.round(region.h * 100)}%
        </span>
        <button
          type="button"
          onClick={() => onSave(region)}
          className="px-3 py-1 rounded bg-violet-500 hover:bg-violet-400 text-black font-semibold"
        >
          Save
        </button>
        <button
          type="button"
          onClick={onCancel}
          className="px-3 py-1 rounded bg-surface-700 hover:bg-surface-600 text-slate-200"
        >
          Cancel
        </button>
      </div>
    </>
  )
}
```

- [ ] **Step 8.2: Verify the build**

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
npm run build
```

Expected: `tsc -b && vite build` clean. Component is unwired so no consumer compile errors.

- [ ] **Step 8.3: Commit**

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
git add src/components/CamRegionSetter.tsx
git commit -m "feat(ui): CamRegionSetter — draggable source-frame region overlay"
```

---

## Task 9: `CamRegionRow` component (right-rail row)

**Files:**
- Create: `src/components/CamRegionRow.tsx`

- [ ] **Step 9.1: Write the component**

Create `src/components/CamRegionRow.tsx` with:

```tsx
import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'

type RegionNorm = { x: number; y: number; w: number; h: number }
type FitMode = 'fit' | 'fill' | 'stretch'

type Props = {
  vodId: string
  clipId: string
  /** VOD-level region (parsed from `vod.cam_region_norm`). Null = no region set. */
  vodRegion: RegionNorm | null
  /** Per-clip override (parsed from `clip.cam_region_norm_override`). Null = no override. */
  clipOverride: RegionNorm | null
  /** Current fit mode for this clip. Null/undefined treated as 'fit'. */
  fitMode: FitMode | null
  /** Whether the current layout uses a cam slot (false for GameplayFocus). */
  layoutHasCamSlot: boolean
  /** Called when the user clicks "Set region…" — parent should enter edit mode on the source player. */
  onEnterVodEditMode: () => void
  /** Called when the user clicks "Override for this clip…" — parent enters override-edit mode. */
  onEnterClipOverrideMode: () => void
  /** Called after any DB-mutating action so parent can re-fetch state. */
  onChanged: () => void
}

function regionLabel(r: RegionNorm | null): string {
  if (!r) return 'Not set'
  return `${Math.round(r.x * 100)}%, ${Math.round(r.y * 100)}% · ${Math.round(r.w * 100)}×${Math.round(r.h * 100)}%`
}

export default function CamRegionRow({
  vodId, clipId, vodRegion, clipOverride, fitMode, layoutHasCamSlot,
  onEnterVodEditMode, onEnterClipOverrideMode, onChanged,
}: Props) {
  const [allowOverride, setAllowOverride] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    invoke<boolean>('get_allow_per_clip_override')
      .then(setAllowOverride)
      .catch(() => setAllowOverride(false))
  }, [])

  const clearVod = async () => {
    if (busy) return
    setBusy(true); setError(null)
    try {
      await invoke('clear_vod_cam_region', { vodId })
      onChanged()
    } catch (e) { setError(String(e)) } finally { setBusy(false) }
  }

  const clearOverride = async () => {
    if (busy) return
    setBusy(true); setError(null)
    try {
      await invoke('clear_clip_cam_region_override', { clipId })
      onChanged()
    } catch (e) { setError(String(e)) } finally { setBusy(false) }
  }

  const setFit = async (mode: FitMode) => {
    if (busy) return
    setBusy(true); setError(null)
    try {
      await invoke('set_clip_fit_mode', { clipId, mode })
      onChanged()
    } catch (e) { setError(String(e)) } finally { setBusy(false) }
  }

  const effectiveFit: FitMode = fitMode ?? 'fit'
  const fitDisabledReason = !layoutHasCamSlot
    ? 'No cam slot in this layout'
    : (!vodRegion && !clipOverride)
      ? 'Set a cam region first'
      : null

  return (
    <div className="space-y-2">
      {/* VOD-level region row */}
      <div className="bg-surface-800 border border-surface-700 rounded p-2">
        <div className="text-[10px] uppercase tracking-wider text-violet-300 font-semibold mb-1">
          Cam region <span className="text-slate-500 font-normal normal-case tracking-normal">(per-VOD)</span>
        </div>
        <div className="flex items-center gap-2 text-xs text-slate-200">
          <span className="flex-1 font-mono">{regionLabel(vodRegion)}</span>
          <button
            type="button"
            disabled={busy || !layoutHasCamSlot}
            onClick={onEnterVodEditMode}
            className="px-2 py-1 rounded bg-surface-700 hover:bg-surface-600 disabled:opacity-40 cursor-pointer"
            title={!layoutHasCamSlot ? 'No cam slot in this layout' : 'Set the cam region by dragging on the source player'}
          >
            Set region…
          </button>
          {vodRegion && (
            <button
              type="button"
              disabled={busy}
              onClick={clearVod}
              className="px-2 py-1 rounded bg-red-500/20 hover:bg-red-500/30 text-red-300 disabled:opacity-40 cursor-pointer"
              title="Clear cam region for this VOD"
            >
              Clear
            </button>
          )}
        </div>
        <div className="text-[10px] text-slate-500 mt-1">Same region used by every clip in this VOD.</div>
      </div>

      {/* Fit mode dropdown */}
      <div className="flex items-center gap-2">
        <span className="text-[10px] uppercase tracking-wider text-amber-300 font-semibold flex-1">
          Fit mode <span className="text-slate-500 font-normal normal-case tracking-normal">(per-clip)</span>
        </span>
        <select
          value={effectiveFit}
          disabled={busy || !!fitDisabledReason}
          onChange={(e) => setFit(e.target.value as FitMode)}
          className="px-2 py-1 text-xs bg-surface-800 border border-surface-700 text-slate-200 rounded disabled:opacity-40"
          title={fitDisabledReason ?? 'How the source region fits into the cam slot'}
        >
          <option value="fit">Fit (default)</option>
          <option value="fill">Fill</option>
          <option value="stretch">Stretch</option>
        </select>
      </div>

      {/* Per-clip override sub-row — only when the global toggle is on */}
      {allowOverride && layoutHasCamSlot && (
        <div className="bg-surface-800/70 border border-surface-700 rounded p-2">
          <div className="text-[10px] uppercase tracking-wider text-emerald-300 font-semibold mb-1">
            Per-clip override
          </div>
          <div className="flex items-center gap-2 text-xs text-slate-300">
            <span className="flex-1 font-mono">
              {clipOverride ? regionLabel(clipOverride) : 'Using VOD default'}
            </span>
            <button
              type="button"
              disabled={busy}
              onClick={onEnterClipOverrideMode}
              className="px-2 py-1 rounded bg-surface-700 hover:bg-surface-600 disabled:opacity-40 cursor-pointer"
            >
              {clipOverride ? 'Edit…' : 'Override…'}
            </button>
            {clipOverride && (
              <button
                type="button"
                disabled={busy}
                onClick={clearOverride}
                className="px-2 py-1 rounded bg-surface-700 hover:bg-surface-600 text-slate-200 disabled:opacity-40 cursor-pointer"
                title="Use VOD default region instead"
              >
                Reset to VOD
              </button>
            )}
          </div>
        </div>
      )}

      {error && <div className="text-xs text-red-400">{error}</div>}
    </div>
  )
}
```

- [ ] **Step 9.2: Verify the build**

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
npm run build
```

Expected: `tsc -b && vite build` clean.

- [ ] **Step 9.3: Commit**

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
git add src/components/CamRegionRow.tsx
git commit -m "feat(ui): CamRegionRow — right-rail row + Fit mode dropdown + override sub-row"
```

---

## Task 10: Editor integration

**Files:**
- Modify: `src/pages/Editor.tsx`

This task wires the `CamRegionRow` + `CamRegionSetter` into the existing editor. The Setter mounts as an overlay over the source player when in edit mode; the Row lives in the Layout section of the right rail.

- [ ] **Step 10.1: Add imports**

In `src/pages/Editor.tsx`, find the existing `import LayoutPicker from '../components/LayoutPicker'` line. Add immediately after:

```typescript
import CamRegionRow from '../components/CamRegionRow'
import CamRegionSetter from '../components/CamRegionSetter'
import type { RegionNorm } from '../components/CamRegionSetter'
```

- [ ] **Step 10.2: Add region-edit-mode state**

Find the section where the editor declares its state (around the `useState` calls in the component body, near line 580). Add:

```typescript
  // Cam region edit state
  type RegionEditScope = 'vod' | 'clip-override' | null
  const [regionEditScope, setRegionEditScope] = useState<RegionEditScope>(null)
  const [playerRect, setPlayerRect] = useState<DOMRect | null>(null)
  const playerContainerRef = useRef<HTMLDivElement>(null)
  // refetch flag — increments to force vod / clip re-fetch after region writes
  const [camRegionRefetchToken, setCamRegionRefetchToken] = useState(0)
  const bumpCamRegionRefetch = () => setCamRegionRefetchToken(t => t + 1)
```

- [ ] **Step 10.3: Re-fetch VOD + clip whenever the refetch token changes**

Find the existing `useEffect` that loads the VOD (around line 960). Add `camRegionRefetchToken` to its dependency array:

```typescript
  }, [clipId, camRegionRefetchToken])
```

(Replace whatever the existing dep array is — typically just `[clipId]`.)

- [ ] **Step 10.4: Parse region JSON helpers**

Near the top of the component body (after state declarations), add:

```typescript
  const parseRegion = (s: string | null | undefined): RegionNorm | null => {
    if (!s) return null
    try {
      const o = JSON.parse(s)
      if (typeof o.x === 'number' && typeof o.y === 'number' && typeof o.w === 'number' && typeof o.h === 'number') return o
    } catch { /* swallow */ }
    return null
  }
  const vodRegion = parseRegion(vod?.cam_region_norm ?? null)
  const clipOverride = parseRegion(clip?.cam_region_norm_override ?? null)
```

- [ ] **Step 10.5: Mount the Setter overlay on the source player**

Find the JSX where the source player is rendered (look for `<ClipPlayer`). Wrap it in a ref'd div, OR if a ref'd container already exists, attach `playerContainerRef` to it. Example structure:

```tsx
<div ref={playerContainerRef} className="relative">
  <ClipPlayer ... />
  {regionEditScope && playerRect && (
    <CamRegionSetter
      initial={
        regionEditScope === 'vod'
          ? (vodRegion ?? { x: 0.05, y: 0.70, w: 0.25, h: 0.25 })
          : (clipOverride ?? vodRegion ?? { x: 0.05, y: 0.70, w: 0.25, h: 0.25 })
      }
      containerRect={playerRect}
      onChange={() => { /* could update a live-preview overlay; minimal v1 */ }}
      onSave={async (r) => {
        try {
          if (regionEditScope === 'vod') {
            await invoke('set_vod_cam_region', { vodId: vod!.id, region: r })
          } else {
            await invoke('set_clip_cam_region_override', { clipId: clip!.id, region: r })
          }
          setRegionEditScope(null)
          bumpCamRegionRefetch()
        } catch (e) {
          console.error('[Editor] save cam region failed', e)
        }
      }}
      onCancel={() => setRegionEditScope(null)}
    />
  )}
</div>
```

Also add an effect to measure the player rect when in edit mode:

```typescript
  useEffect(() => {
    if (!regionEditScope) { setPlayerRect(null); return }
    const measure = () => {
      if (playerContainerRef.current) {
        setPlayerRect(playerContainerRef.current.getBoundingClientRect())
      }
    }
    measure()
    window.addEventListener('resize', measure)
    return () => window.removeEventListener('resize', measure)
  }, [regionEditScope])
```

- [ ] **Step 10.6: Embed `CamRegionRow` in the Layout section**

Find the Layout `<Section title="Layout">` block. After the existing FacecamEditor / facecamLayout-related JSX (around the same spot where v1.3.16 embedded `CamAssetPicker`), insert:

```tsx
            {/* Cam region (crop from source) — visible whenever layout has a cam slot */}
            {clip && vod && (facecamLayout === 'split' || facecamLayout === 'pip') && (
              <div className="mt-3">
                <CamRegionRow
                  vodId={vod.id}
                  clipId={clip.id}
                  vodRegion={vodRegion}
                  clipOverride={clipOverride}
                  fitMode={(clip.cam_fit_mode ?? null) as 'fit' | 'fill' | 'stretch' | null}
                  layoutHasCamSlot={facecamLayout === 'split' || facecamLayout === 'pip'}
                  onEnterVodEditMode={() => setRegionEditScope('vod')}
                  onEnterClipOverrideMode={() => setRegionEditScope('clip-override')}
                  onChanged={bumpCamRegionRefetch}
                />
              </div>
            )}
```

- [ ] **Step 10.7: Verify the build**

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
npm run build
```

Expected: `tsc -b && vite build` clean. If TS complains:
- `useRef` not imported → already in line 1 import.
- `RegionNorm` not exported from CamRegionSetter → check Task 8's `export type RegionNorm`.

- [ ] **Step 10.8: Commit**

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
git add src/pages/Editor.tsx
git commit -m "feat(editor): embed CamRegionRow + CamRegionSetter overlay for Split/Pip layouts"
```

---

## Task 11: Settings page toggle

**Files:**
- Modify: `src/pages/Settings.tsx`

- [ ] **Step 11.1: Locate the settings layout**

Open `src/pages/Settings.tsx`. The file has rows for: Twitch connect, AI provider, storage location, sensitivity, etc. Find a stable anchor — e.g., the existing sensitivity-control section. The new toggle row will go below it.

- [ ] **Step 11.2: Add state + load**

Near the top of the component body, alongside other `useState` calls, add:

```typescript
  const [allowPerClipCamOverride, setAllowPerClipCamOverride] = useState(false)
```

And inside the main "load settings on mount" `useEffect`, fetch the value:

```typescript
    invoke<boolean>('get_allow_per_clip_override')
      .then(setAllowPerClipCamOverride)
      .catch(() => setAllowPerClipCamOverride(false))
```

- [ ] **Step 11.3: Add the toggle row JSX**

Insert this row in an appropriate section (e.g., after the sensitivity slider, before the Advanced/Storage section). Match the existing styling:

```tsx
      <section className="rounded-xl bg-surface-800 border border-surface-700 p-4 space-y-2">
        <h3 className="text-sm font-semibold text-slate-200">Per-clip cam region overrides</h3>
        <p className="text-xs text-slate-400">
          When on, each clip can override its VOD's cam region. Off keeps the simpler one-region-per-VOD flow. Any saved overrides are preserved in the database when this toggle is off — they just aren't used at export time.
        </p>
        <label className="flex items-center gap-2 cursor-pointer">
          <input
            type="checkbox"
            checked={allowPerClipCamOverride}
            onChange={async (e) => {
              const next = e.target.checked
              setAllowPerClipCamOverride(next)
              try { await invoke('set_allow_per_clip_override', { enabled: next }) }
              catch (err) {
                console.error('[Settings] set_allow_per_clip_override failed', err)
                setAllowPerClipCamOverride(!next)  // revert on failure
              }
            }}
            className="rounded border-surface-600 bg-surface-900 text-violet-500 focus:ring-violet-500"
          />
          <span className="text-xs text-slate-300">
            {allowPerClipCamOverride ? 'On' : 'Off'}
          </span>
        </label>
      </section>
```

(Wrap classes/styling may differ slightly to match the existing Settings page palette. If `bg-surface-800` etc. aren't in use there, copy the nearby section's classes verbatim.)

- [ ] **Step 11.4: Verify the build**

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
npm run build
```

Expected: clean.

- [ ] **Step 11.5: Commit**

```bash
cd "C:/Users/cereb/Desktop/Claude projects/clipviral"
git add src/pages/Settings.tsx
git commit -m "feat(settings): toggle for per-clip cam region overrides"
```

---

## Task 12: Slug verification + ship

**Files:** version-bump files only (`package.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`).

This task is performed by Slug (cargo + live app required; not available in the VM).

- [ ] **Step 12.1: Compile + unit tests**

```powershell
cd "C:\Users\cereb\Desktop\Claude projects\clipviral\src-tauri"
cargo check
cargo test cam_region 2>&1 | Select-Object -Last 25
cargo test layout_filter 2>&1 | Select-Object -Last 25
cargo test 2>&1 | Select-String -Pattern "test result:|cam_region|layout_filter|FAILED"
```

Expected:
- `cargo check`: Finished with 0 errors. Pre-existing ~206 warnings remain.
- `cargo test cam_region`: 19 new tests pass (16 in `cam_region.rs` + 3 in `commands/cam_region.rs`).
- `cargo test layout_filter`: 8 new tests pass.
- Full suite: prior count + 27 new = `~468 passed; 0 failed`.

If any test fails or compile errors out, STOP — paste the failure; do not bump the version.

- [ ] **Step 12.2: Frontend build**

```powershell
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
npm run build
```

Expected: `✓ built in ~Xs`, no TS errors.

- [ ] **Step 12.3: Live smoke — fresh state**

```powershell
cargo tauri dev
```

Then in the running app:

1. **Regression baseline (no region).** Open any existing clip in Editor → pick Split or Pip layout. The new **Cam region** row appears with *"Not set"* + **Set region…**. Export. Expected: byte-identical to v1.3.14 output (dup-source gameplay in cam slot).
2. **Set a VOD region — PiP.** Click **Set region…** → player pauses + dim overlay + draggable rect. Drag/resize the rect to approximately cover the slug avatar's region. Click **Save**. Row updates to show new coords. Switch layout to PiP. Export. Expected: avatar appears in the PiP slot at slot position; gameplay shows through where the cam doesn't fill the slot (passthrough).
3. **Same VOD, Split layout.** Switch to Split. Export. Expected: bottom slot has a heavily-blurred copy of the avatar region as backdrop; sharp avatar centered on top.
4. **Fit mode toggling.** With Split layout: change Fit dropdown to **Fill** → re-export → no bars, avatar fills width (top/bottom may crop). Change to **Stretch** → re-export → avatar appears distorted (cosmetic only).
5. **GameplayFocus regression.** Switch layout to GameplayFocus → Cam region row is hidden (no cam slot in this layout). Export. Expected: full-frame gameplay, no avatar — byte-identical to v1.3.14.
6. **Clear via UI.** Switch back to PiP/Split → click **Clear** on the Cam region row → row shows *"Not set"* again → export → back to dup-source.
7. **Restart persistence.** With a region set, fully close + reopen `cargo tauri dev`. Open the same clip. The Cam region row still shows the saved region. Region is persisted in DB.
8. **Settings toggle ON.** Open Settings → toggle *"Per-clip cam region overrides"* to **On**. Return to a clip in PiP/Split with a VOD region set. The Cam region row now shows an additional **Per-clip override** sub-row. Click **Override…** → drag a different rect → Save. Re-export → output uses the override, not the VOD region.
9. **Toggle OFF preserves DB.** Set the toggle back to **Off**. The override sub-row disappears from the editor. Re-export → output uses the VOD region (NOT the saved override). Toggle back **On** → the saved override reappears in the sub-row.
10. **Esc cancels mid-drag.** Click **Set region…**, start dragging, then press **Esc** before clicking Save. Expected: edit mode exits, region unchanged.

If any of the 10 paths fail, STOP — paste the failure; do not bump the version.

- [ ] **Step 12.4: Version bump + ship**

This is a user-facing release (real new feature) and replaces the unshipped v1.3.16 work. **Pick v1.4.0** (semver-significant: replaces an entire user-facing approach, not a hotfix patch).

```powershell
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
powershell -File bump-version.ps1 1.4.0
git add package.json src-tauri/Cargo.lock src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -m "chore: bump to v1.4.0 (crop-from-source cam region)"
git tag -a v1.4.0 -m "v1.4.0 — crop-from-source cam region (drag rect on source frame)"
git push origin main
git push origin v1.4.0
```

When CI finishes, publish the v1.4.0 release directly from the CI-generated draft. **Do NOT use the web edit-by-tag link** — that mistake created a dup empty release in v1.3.14. If you accidentally do, delete the empty release via `gh api -X DELETE /repos/nsvlordslug/ClipGoblin/releases/<id>` then `gh release edit v1.4.0 --draft=false --latest --notes-file ...`.

User-facing release notes:

> **Bring your avatar to your clips — from your VOD itself.** Drag a rectangle on the source video frame to mark where your slug / vtuber / facecam is in the original stream, and any clip from that VOD using the PIP or Split layout will composite that exact region into the cam slot. PiP shows the avatar floating over gameplay; Split puts a blurred backdrop behind the sharp avatar. Set once per VOD, every clip from that VOD inherits. Optional per-clip override available in Settings.

---

## Self-review

(Plan author, fresh eyes against the spec.)

### Spec coverage

Walking each spec section against the plan:

- **§1 Background** — no implementation needed. ✓
- **§2 Goals & success criteria** — Tasks 5-11 collectively implement the feature; Task 12 verifies "byte-identical output before vs after" (§2 prime goal) via Step 12.3.1 (regression baseline) AND via `layout_filter_with_region_no_region_byte_identical_to_layout_filter` unit test in Task 5. ✓
- **§3.1 Three knobs** — Task 2 (DB), Task 3 (CamRegion + CamFitMode types), Task 9 (CamRegionRow with Set + Fit dropdown), Task 10 (slot position+size unchanged). ✓
- **§3.2 Layout-specific composition** — Task 5 (PiP uses split=2 no slot rect; Split uses split=3 with boxblur). Tests assert both. ✓
- **§3.3 Data model** — Task 2 covers all three columns + settings key + serde defaults + VOD_SELECT/CLIP_SELECT consolidation. ✓
- **§3.4 UI surface** — Task 9 (CamRegionRow), Task 8 (CamRegionSetter overlay), Task 10 (Editor wiring), Task 11 (Settings toggle). ✓
- **§3.5 ffmpeg implementation** — Task 5 implements `layout_filter_with_region` + `fit_scale_expr`. Tests cover all 3 fit modes, single-input shape, caption wiring. ✓
- **§3.6 Backward compatibility** — Task 5's `layout_filter_with_region_no_region_byte_identical_to_layout_filter` is the regression canary. Task 2's `#[serde(default)]` on all new fields. Task 12 Step 12.3.1 + 12.3.5 verify live. ✓
- **§4 File-level changes** — every file mentioned in the spec is touched in exactly one task each (no overlap; clear ownership). ✓
- **§5 Revert strategy** — Task 1 does `git reset --hard 6927297` + restores the new spec from reflog + adds superseded banners. ✓
- **§6 Testing** — 16 cam_region unit tests (Task 3) + 3 commands tests (Task 4) + 8 layout_filter tests (Task 5) + 5 resolver tests embedded in cam_region (Task 3) + 10 live smoke paths (Task 12). Exceeds the 10/10 from the spec. ✓
- **§7 Watchouts** — byte-identical canary (Task 5 test), MIN_REGION_DIM at parse + UI (Task 3 + Task 8), boxblur=20:5 (Task 5), PiP centering (Task 5), settings-off-preserves-overrides (Task 4's `set_allow_per_clip_override` writes without touching override values), dead v1.3.16 columns (left in place per spec), bump-version.ps1 (Task 12). ✓

### Placeholder scan

No "TBD", no "TODO", no "implement later", no "Similar to Task N". Each code-step has full code blocks. ✓

### Type consistency

- `CamRegion` defined in Task 3 used identically in Tasks 4 (RegionInput→to_region→CamRegion), 5 (filter), 6 (resolver call). ✓
- `CamFitMode` defined in Task 3 used in Tasks 4 (normalize), 5 (filter), 6 (from_db). ✓
- `RegionNorm` (TS) exported from CamRegionSetter (Task 8), imported in Editor (Task 10), used as prop type in CamRegionRow (Task 9). Three identical shape definitions; consistent. ✓
- Column indices: VOD_SELECT lists 17 cols → row.get(16) for the 17th (0-indexed) field = `cam_region_norm`. ✓ CLIP_SELECT lists 25 cols → row.get(23) + row.get(24) for the last two. ✓
- Tauri command argument naming: Rust uses snake_case (`vod_id`, `clip_id`), JS invoke uses camelCase (`vodId`, `clipId`) — Tauri auto-converts. Verified consistent across Tasks 4 + 9 + 10 + 11. ✓
- `allow_per_clip_cam_region_override` settings key spelled identically in Task 2 (default), Task 4 (commands), and Task 6 (resolver). ✓

No placeholders, no type drift, no spec gaps. Plan is ready for execution.
