# Per-VOD Cam Asset Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user attach one image or video file (PNG/JPG/WebP/MP4/WebM/MOV/GIF) to a VOD so that every clip from that VOD using the `Split` or `Pip` layout composites the asset into the cam slot instead of duplicating the source frame.

**Architecture:** Two additive DB columns on `vods` track the managed copy + original source path of the asset. A new `commands/cam_asset.rs` exposes three Tauri commands (set/clear/recent). The existing `vertical_crop::layout_filter` gains a `has_cam_asset` flag and emits a two-input ffmpeg filter graph (`[0:v]` gameplay + `[1:v]` asset) when set; `build_ffmpeg_command` adds the matching `-loop` / `-stream_loop` input. The Clip Editor gets a new `CamAssetPicker` component that's visible only when the active layout is Split/Pip.

**Tech Stack:** Rust + Tauri 2 backend (existing `rusqlite`, `chrono`, ffmpeg via `vertical_crop`), React + TypeScript frontend (existing `@tauri-apps/plugin-dialog` for file picker, Zustand stores).

---

## Spec reference

Implements `docs/superpowers/specs/2026-05-17-per-vod-cam-asset-design.md` (latest amendment `8242434`).

Single cohesive feature — one plan, no decomposition.

## VM / build constraint (read first)

Per project CLAUDE.md rule #5: **`cargo` is NOT available in this VM.** Therefore:

- **Rust tasks (1-5):** the implementer writes code + unit tests, does careful static review, and commits. The implementer does **NOT** run `cargo`. TDD "verify fail / verify pass" gates for Rust are deferred to Task 9 (Slug runs the real suite).
- **Frontend tasks (6-8):** `npm run build` runs in this VM (used successfully in v1.3.13/v1.3.14). The implementer runs it and waits for clean.
- **Task 9** is Slug-side: `cargo check` + `cargo test` + `cargo tauri dev` smoke + version bump + tag + push.

The v1.3.14 final whole-feature review found two compile-level defects (smart quotes, borrowck) that per-task reviews couldn't see because cargo can't run in-VM — Task 9's real compile is the meaningful final gate.

Direct-on-main is the project convention (every prior release shipped that way). Do **not** bump the version until Task 9.

## Pre-flight: confirm these symbols exist

Line numbers may drift; locate by symbol search if needed.

**Backend (Rust):**
- `src-tauri/src/vertical_crop.rs`: `pub fn layout_filter(mode: &LayoutMode, target: OutputSize, caption_filter: Option<&str>) -> (String, bool)` (~362-428); `pub struct ExportRequest { source_path, output_path, start, end, platform, target, layout, caption_filter }` (~441-459); `pub fn build_ffmpeg_command(ffmpeg_path, request, encode) -> Command` (~532-545); `pub enum LayoutMode { GameplayFocus, Split { ratio }, Pip { x, y, size } }` (~160-191).
- `src-tauri/src/db.rs`: `pub(crate) fn run_migrations(conn) -> SqliteResult<()>` (~42), uses `conn.execute("ALTER TABLE ...", []).ok()` for additive column adds (existing examples at ~110-121). `pub struct VodRow { ... }` (search `pub struct VodRow`). `pub fn get_vod_by_id(conn, id) -> SqliteResult<Option<VodRow>>`. `pub fn update_vod_download_status(...)` — pattern to mirror for the new update fn.
- `src-tauri/src/bin_manager.rs`: `pub fn bin_dir() -> Result<PathBuf, AppError>` — pattern to mirror for `assets_dir()`.
- `src-tauri/src/commands/mod.rs`: declares `pub mod binaries;` etc. — bare module declarations, no explicit re-export list. New `pub mod cam_asset;` line goes here.
- `src-tauri/src/lib.rs`: explicit `use commands::binaries::{check_binary_status, download_binaries, force_refresh_ytdlp};` pattern around line 60-something — extend with the 3 new cam_asset commands. `invoke_handler![]` list (~160-236) — append the 3 names.
- `src-tauri/src/commands/export.rs`: `fn clip_to_export_request(clip, vod_path, output_path) -> vertical_crop::ExportRequest` (~288-310); both call sites of `vertical_crop::run_export` (~181, ~261).
- `src-tauri/src/commands/vod.rs`: `pub fn get_vod_detail(...)` — the command that fetches a single VOD for the frontend, must return `cam_asset_path` so the Editor can read current state.

**Frontend (TypeScript/React):**
- `src/pages/ClipEditor.tsx`: contains the layout picker (radio buttons or similar selector for GameplayFocus / Split / Pip / facecam_layout). The new picker row attaches here.
- Vod type / interface: most likely in `src/types/editTypes.ts` or `src/types/*.ts` — locate via `grep -rn "interface Vod\|type Vod" src/types src/lib` before extending. If it's actually in `src/stores/appStore.ts` or inlined elsewhere, extend it there.
- `@tauri-apps/plugin-dialog` is in `package.json` dependencies — use its `open()` function for the file picker.

## File structure (logical map; tasks pin exact lines)

**Backend (Rust):**
- `src-tauri/src/db.rs` — schema migration (2 new columns), VodRow extension, `update_vod_cam_asset` + `clear_vod_cam_asset_db` helpers (named with `_db` suffix to distinguish from command-layer wrappers).
- `src-tauri/src/bin_manager.rs` — add `assets_dir()` (mirrors `bin_dir()`). The asset-copy logic lives in `cam_asset.rs` (closer to its caller), not bin_manager.
- `src-tauri/src/commands/cam_asset.rs` — **new file**. Three Tauri commands (`set_vod_cam_asset`, `clear_vod_cam_asset`, `recent_cam_assets`) + a pure helper `managed_asset_path(vod_id, source_ext) -> PathBuf` that the unit test exercises.
- `src-tauri/src/commands/mod.rs` — one new `pub mod cam_asset;` line.
- `src-tauri/src/vertical_crop.rs` — extend `layout_filter` with `has_cam_asset: bool`; extend `ExportRequest` with `cam_asset_path: Option<PathBuf>`; extend `build_ffmpeg_command` to add the second `-i` input when the asset is present.
- `src-tauri/src/commands/export.rs` — `clip_to_export_request` gains a `vod` parameter (or `cam_asset_path` param) and populates `ExportRequest.cam_asset_path`.
- `src-tauri/src/commands/vod.rs` — `get_vod_detail` returns `cam_asset_path` (extending whatever shape it serializes).
- `src-tauri/src/lib.rs` — register the 3 new commands.

**Frontend (TypeScript/React):**
- `src/types/*` (locate exact file) — extend `Vod` interface with `cam_asset_path: string | null` and `cam_asset_source: string | null`.
- `src/components/CamAssetPicker.tsx` — **new file**. Self-contained component: thumbnail/filename row + Choose button + Recent ▾ dropdown + Remove button. Props: `vodId`, current asset state, `onChange` callback (or invokes the Tauri commands directly + lets parent re-fetch).
- `src/pages/ClipEditor.tsx` — embed `<CamAssetPicker>` in the layout panel, conditionally on `selectedLayout === 'split' || selectedLayout === 'pip'`.

**Files NOT changed:**
- Any clip-level analysis/scoring code (`clip_selector.rs`, `analyze_vod` path, etc.) — feature is purely an export-time concern.
- Bug-report / settings / scheduler / Twitch — orthogonal.

---

## Task 1: DB migration + VodRow extension (Rust, static-review)

**Files:**
- Modify: `src-tauri/src/db.rs`

- [ ] **Step 1.1: Add the two column migrations**

Find the migration block in `src-tauri/src/db.rs` — it's a sequence of `conn.execute("ALTER TABLE ...", []).ok();` lines starting around line 110. Append these two lines at the END of that sequence (so they sit alongside the other additive column adds):

```rust
    // v1.3.16: per-VOD cam asset (image/video that fills the cam slot for Split/Pip clips).
    // `cam_asset_path` is the absolute path to the managed copy (in %APPDATA%\clipviral\assets\).
    // `cam_asset_source` is the user's originally picked path, retained for the recent-assets dropdown.
    conn.execute("ALTER TABLE vods ADD COLUMN cam_asset_path TEXT", []).ok();
    conn.execute("ALTER TABLE vods ADD COLUMN cam_asset_source TEXT", []).ok();
```

The `.ok()` swallows the "duplicate column" error if the migration has already run — idempotent like the existing migrations.

- [ ] **Step 1.2: Extend the VodRow struct**

Find `pub struct VodRow { ... }` in `src-tauri/src/db.rs`. Add two new fields at the end of the struct (before the closing `}`), with `Option<String>` types and serde defaults so they round-trip cleanly:

```rust
    /// Absolute path to the managed copy of the per-VOD cam asset (image/video).
    /// `None` means no asset attached → clips render with current dup-source behavior.
    #[serde(default)]
    pub cam_asset_path: Option<String>,
    /// User's originally-picked path. Retained for the recent-assets dropdown display
    /// and to power one-click reuse across VODs. `None` iff `cam_asset_path` is None.
    #[serde(default)]
    pub cam_asset_source: Option<String>,
```

- [ ] **Step 1.3: Extend every VodRow SELECT to include the new columns**

Locate every place in `db.rs` that builds a `VodRow` from a query result. There are at least three: `get_vod_by_id`, `get_all_vods`, `get_vods_by_channel`. Each has the pattern:

```rust
    let mut stmt = conn.prepare(
        "SELECT id, channel_id, twitch_video_id, title, duration_seconds, stream_date, thumbnail_url, vod_url, download_status, local_path, file_size_bytes, analysis_status, created_at, download_progress, analysis_progress, game_name
         FROM vods WHERE id = ?1"
    )?;
```

For EACH such query, append `, cam_asset_path, cam_asset_source` to the SELECT column list:

```rust
        "SELECT id, channel_id, twitch_video_id, title, duration_seconds, stream_date, thumbnail_url, vod_url, download_status, local_path, file_size_bytes, analysis_status, created_at, download_progress, analysis_progress, game_name, cam_asset_path, cam_asset_source
         FROM vods WHERE id = ?1"
```

And in the row-mapping callback (which currently reads `vod.cam_asset_path = row.get(...)?` etc.), append:

```rust
            cam_asset_path: row.get(16)?,
            cam_asset_source: row.get(17)?,
```

(Indices 16 and 17 assume the existing 16 columns are at indices 0-15 — verify against the actual query by counting. If a query SELECTs fewer or differently-ordered columns, use the corresponding next two indices.)

- [ ] **Step 1.4: Add update + clear helpers**

Append to `src-tauri/src/db.rs` (near the other `update_vod_*` helpers, e.g. just after `update_vod_download_status`):

```rust
/// Set both cam-asset columns atomically.
pub fn update_vod_cam_asset(
    conn: &Connection,
    vod_id: &str,
    managed_path: &str,
    source_path: &str,
) -> SqliteResult<()> {
    conn.execute(
        "UPDATE vods SET cam_asset_path = ?1, cam_asset_source = ?2 WHERE id = ?3",
        params![managed_path, source_path, vod_id],
    )?;
    Ok(())
}

/// Clear both cam-asset columns (sets them to NULL).
pub fn clear_vod_cam_asset_db(conn: &Connection, vod_id: &str) -> SqliteResult<()> {
    conn.execute(
        "UPDATE vods SET cam_asset_path = NULL, cam_asset_source = NULL WHERE id = ?1",
        params![vod_id],
    )?;
    Ok(())
}
```

(The `_db` suffix on `clear_vod_cam_asset_db` distinguishes it from the Tauri command of similar name in Task 3. `update_vod_cam_asset` doesn't need the suffix because the Tauri command is `set_vod_cam_asset` — different verb.)

- [ ] **Step 1.5: Static self-review + commit**

Verify: migrations appended idempotent style (`.ok()`); `VodRow` has both new fields with `#[serde(default)]`; all VodRow-building SELECTs updated identically; both helper fns added; nothing else in db.rs touched.

```bash
git add src-tauri/src/db.rs
git commit -m "feat(db): cam_asset_path + cam_asset_source columns + VodRow + helpers"
```

---

## Task 2: `assets_dir()` in bin_manager (Rust, TDD-shaped though no automated test)

**Files:**
- Modify: `src-tauri/src/bin_manager.rs`

- [ ] **Step 2.1: Add `assets_dir` mirroring `bin_dir`**

Find `pub fn bin_dir() -> Result<PathBuf, AppError>` in `src-tauri/src/bin_manager.rs`. Immediately AFTER it, append:

```rust
/// `%APPDATA%/clipviral/assets/`, creating it if it doesn't exist.
/// Mirrors `bin_dir()`. Holds the managed copies of per-VOD cam assets
/// (one file per VOD-id, original extension preserved).
pub fn assets_dir() -> Result<PathBuf, AppError> {
    let base = dirs::data_dir()
        .ok_or_else(|| AppError::Unknown("no APPDATA dir on this system".into()))?;
    let dir = base.join("clipviral").join("assets");
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::Unknown(format!("create assets dir: {e}")))?;
    Ok(dir)
}
```

- [ ] **Step 2.2: Static self-review + commit**

Verify: function signature exactly `pub fn assets_dir() -> Result<PathBuf, AppError>`; uses `dirs::data_dir()` (the same call as `bin_dir`); creates dir with `create_dir_all`; doc-comment present.

```bash
git add src-tauri/src/bin_manager.rs
git commit -m "feat(bin_manager): assets_dir for managed cam-asset copies"
```

---

## Task 3: `commands/cam_asset.rs` — set/clear/recent + handler registration (Rust, TDD on pure helper)

**Files:**
- Create: `src-tauri/src/commands/cam_asset.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 3.1: Write failing tests for the path helper**

Create `src-tauri/src/commands/cam_asset.rs` with the test module at the top of the file (Rust convention is tests at the bottom; we put it at top here so it's the first thing the implementer scans):

Actually, place the test module at the bottom per the project's existing convention. Below is the FULL initial file content — write it all in one Write call:

```rust
//! Per-VOD cam asset: image/video that fills the cam slot for Split/Pip clip layouts.
//! Commands: set_vod_cam_asset, clear_vod_cam_asset, recent_cam_assets.

use std::path::{Path, PathBuf};
use tauri::State;

use crate::bin_manager;
use crate::db;
use crate::DbConn;

/// Allowed extensions for cam assets (lowercased).
const ALLOWED_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "webp",
    "mp4", "webm", "mov", "gif",
];

/// Compute the managed-copy path for a VOD's cam asset given the source's
/// extension. Returns `%APPDATA%/clipviral/assets/<vod_id>.<ext>`.
/// Lowercases the extension so the resulting filename is stable regardless
/// of how the user named their source file. Pure logic — caller supplies
/// the assets dir; we never touch the filesystem here.
pub fn managed_asset_path(assets_dir: &Path, vod_id: &str, source_ext: &str) -> PathBuf {
    let ext = source_ext.trim_start_matches('.').to_ascii_lowercase();
    assets_dir.join(format!("{vod_id}.{ext}"))
}

/// Extract the lowercase extension from a path. Returns `None` if the
/// extension is missing or empty.
pub fn extension_lower(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| !s.is_empty())
}

/// Returns `true` if `ext` is in our allowlist of cam-asset extensions.
pub fn is_allowed_ext(ext: &str) -> bool {
    ALLOWED_EXTS.contains(&ext.to_ascii_lowercase().as_str())
}

#[tauri::command]
pub async fn set_vod_cam_asset(
    vod_id: String,
    source_path: String,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let src = PathBuf::from(&source_path);
    if !src.is_file() {
        return Err(format!("Source file does not exist: {source_path}"));
    }
    let ext = extension_lower(&src)
        .ok_or_else(|| format!("Source has no extension: {source_path}"))?;
    if !is_allowed_ext(&ext) {
        return Err(format!(
            "Unsupported extension '.{ext}'. Allowed: {}",
            ALLOWED_EXTS.join(", ")
        ));
    }

    let assets = bin_manager::assets_dir().map_err(|e| e.to_string())?;
    let dst = managed_asset_path(&assets, &vod_id, &ext);

    // Best-effort: remove any prior managed copy at the same path (handles
    // re-attach with a different extension by also wiping siblings).
    for old_ext in ALLOWED_EXTS {
        let candidate = assets.join(format!("{vod_id}.{old_ext}"));
        if candidate.exists() && candidate != dst {
            let _ = std::fs::remove_file(&candidate);
        }
    }

    std::fs::copy(&src, &dst)
        .map_err(|e| format!("Copy {} → {}: {e}", src.display(), dst.display()))?;

    let dst_str = dst.to_string_lossy().to_string();
    {
        let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
        db::update_vod_cam_asset(&conn, &vod_id, &dst_str, &source_path)
            .map_err(|e| format!("DB: {e}"))?;
    }
    log::info!("[cam_asset] attached {} → {}", source_path, dst.display());
    Ok(())
}

#[tauri::command]
pub async fn clear_vod_cam_asset(
    vod_id: String,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    // Read existing managed path first (so we know what to delete).
    let existing: Option<String> = {
        let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
        db::get_vod_by_id(&conn, &vod_id)
            .map_err(|e| format!("DB: {e}"))?
            .and_then(|v| v.cam_asset_path)
    };

    // Delete the managed file (best-effort; missing-file is fine — the DB
    // clear below still runs).
    if let Some(path) = existing.as_ref() {
        if let Err(e) = std::fs::remove_file(path) {
            log::warn!("[cam_asset] remove managed file failed (continuing): {e}");
        }
    }

    {
        let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
        db::clear_vod_cam_asset_db(&conn, &vod_id).map_err(|e| format!("DB: {e}"))?;
    }
    log::info!("[cam_asset] cleared for vod {vod_id}");
    Ok(())
}

#[tauri::command]
pub async fn recent_cam_assets(db: State<'_, DbConn>) -> Result<Vec<String>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
    let mut stmt = conn.prepare(
        "SELECT cam_asset_source \
         FROM vods \
         WHERE cam_asset_source IS NOT NULL \
         GROUP BY cam_asset_source \
         ORDER BY MAX(created_at) DESC \
         LIMIT 5",
    ).map_err(|e| format!("DB prepare: {e}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("DB query: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("DB row: {e}"))?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_asset_path_lowercases_extension() {
        let dir = PathBuf::from("/tmp/assets");
        let p = managed_asset_path(&dir, "vod123", "PNG");
        assert_eq!(p, PathBuf::from("/tmp/assets/vod123.png"));
    }

    #[test]
    fn managed_asset_path_strips_leading_dot() {
        let dir = PathBuf::from("/tmp/assets");
        let p = managed_asset_path(&dir, "vod123", ".mp4");
        assert_eq!(p, PathBuf::from("/tmp/assets/vod123.mp4"));
    }

    #[test]
    fn extension_lower_handles_uppercase() {
        let ext = extension_lower(&PathBuf::from("/path/to/AVATAR.PNG"));
        assert_eq!(ext, Some("png".to_string()));
    }

    #[test]
    fn extension_lower_returns_none_when_missing() {
        assert_eq!(extension_lower(&PathBuf::from("/path/to/avatar")), None);
    }

    #[test]
    fn is_allowed_ext_accepts_all_image_and_video_types() {
        for e in ["png", "jpg", "jpeg", "webp", "mp4", "webm", "mov", "gif"] {
            assert!(is_allowed_ext(e), "expected {e} allowed");
        }
    }

    #[test]
    fn is_allowed_ext_rejects_executables_and_archives() {
        for e in ["exe", "zip", "txt", "ts", "mkv"] {
            assert!(!is_allowed_ext(e), "expected {e} rejected");
        }
    }

    #[test]
    fn is_allowed_ext_is_case_insensitive() {
        assert!(is_allowed_ext("PNG"));
        assert!(is_allowed_ext("Mp4"));
    }
}
```

Save the entire file. The five public functions (`managed_asset_path`, `extension_lower`, `is_allowed_ext`, plus the 3 `#[tauri::command]`s) are tested for the pure logic; the commands themselves are static-reviewed and exercised in Task 9's smoke test.

- [ ] **Step 3.2: Register the module**

In `src-tauri/src/commands/mod.rs`, find the line `pub mod binaries;` (and similar `pub mod`s). Add immediately after:

```rust
pub mod cam_asset;
```

- [ ] **Step 3.3: Import + register the 3 commands in lib.rs**

In `src-tauri/src/lib.rs`, find the `use commands::binaries::{check_binary_status, download_binaries, force_refresh_ytdlp};` line (~line 60ish). Add immediately after:

```rust
use commands::cam_asset::{set_vod_cam_asset, clear_vod_cam_asset, recent_cam_assets};
```

Then find the `.invoke_handler(tauri::generate_handler![ ... ])` list. After `force_refresh_ytdlp,` add three lines:

```rust
            force_refresh_ytdlp,
            set_vod_cam_asset,
            clear_vod_cam_asset,
            recent_cam_assets,
        ])
```

- [ ] **Step 3.4: Static self-review + commit**

Verify: new file `src-tauri/src/commands/cam_asset.rs` contains the 3 `#[tauri::command]` functions, the 3 pure helpers, the 7 unit tests; `commands/mod.rs` has the `pub mod cam_asset;` line; `lib.rs` has both the `use` import and the 3 handler-list additions.

```bash
git add src-tauri/src/commands/cam_asset.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git commit -m "feat(commands): cam_asset module — set/clear/recent commands + handler reg"
```

---

## Task 4: Filter builder accepts cam asset (Rust, TDD on filter strings)

**Files:**
- Modify: `src-tauri/src/vertical_crop.rs`

- [ ] **Step 4.1: Write failing unit tests for the new filter shapes**

In `src-tauri/src/vertical_crop.rs`, find the existing `#[cfg(test)] mod tests` block at the bottom of the file (it has Phase A tests etc.). Append these tests inside it:

```rust
    #[test]
    fn layout_filter_pip_with_cam_asset_uses_second_input_and_letterbox() {
        let target = OutputSize { width: 1080, height: 1920 };
        let mode = LayoutMode::Pip { x: 0.93, y: 0.93, size: 0.3 };
        let (f, complex) = layout_filter_with_asset(&mode, target, None, true);
        assert!(complex, "PIP must be filter_complex");
        // The two-input filter references [1:v] for the asset:
        assert!(f.contains("[1:v]"), "asset must come from input 1: {f}");
        // It must NOT use [0:v]split (the dup-source pattern is only for the no-asset case):
        assert!(!f.contains("[0:v]split"), "asset path must not split source: {f}");
        // Aspect-ratio-preserving fit (decrease) + pad for letterbox/pillarbox:
        assert!(f.contains("force_original_aspect_ratio=decrease"), "must preserve aspect: {f}");
        assert!(f.contains("pad="), "must letterbox: {f}");
        // Final overlay produces [out]:
        assert!(f.ends_with("[out]"));
    }

    #[test]
    fn layout_filter_split_with_cam_asset_uses_second_input() {
        let target = OutputSize { width: 1080, height: 1920 };
        let mode = LayoutMode::Split { ratio: 0.6 };
        let (f, complex) = layout_filter_with_asset(&mode, target, None, true);
        assert!(complex);
        assert!(f.contains("[1:v]"), "asset on input 1: {f}");
        assert!(!f.contains("[0:v]split"), "no split when asset present: {f}");
        assert!(f.contains("vstack"), "split layout must vstack: {f}");
        assert!(f.ends_with("[out]"));
    }

    #[test]
    fn layout_filter_no_cam_asset_preserves_existing_dup_source_behavior() {
        // The no-asset path must be byte-identical to the existing layout_filter.
        let target = OutputSize { width: 1080, height: 1920 };
        let modes = [
            LayoutMode::GameplayFocus,
            LayoutMode::Split { ratio: 0.6 },
            LayoutMode::Pip { x: 0.93, y: 0.93, size: 0.3 },
        ];
        for m in &modes {
            let (old_f, old_c) = layout_filter(m, target, None);
            let (new_f, new_c) = layout_filter_with_asset(m, target, None, false);
            assert_eq!(old_f, new_f, "no-asset path must be identical for {m:?}");
            assert_eq!(old_c, new_c);
        }
    }

    #[test]
    fn layout_filter_gameplay_focus_ignores_cam_asset_flag() {
        // GameplayFocus has no cam slot; the asset flag is irrelevant.
        let target = OutputSize { width: 1080, height: 1920 };
        let mode = LayoutMode::GameplayFocus;
        let (f_no, _) = layout_filter_with_asset(&mode, target, None, false);
        let (f_yes, _) = layout_filter_with_asset(&mode, target, None, true);
        assert_eq!(f_no, f_yes, "asset flag must not affect GameplayFocus");
    }
```

- [ ] **Step 4.2: (Slug-deferred) Note expected failure**

Implementer does NOT run cargo. Expected when Slug runs it: COMPILE ERROR `cannot find function 'layout_filter_with_asset'`.

- [ ] **Step 4.3: Implement `layout_filter_with_asset`**

Find the existing `pub fn layout_filter(mode, target, caption_filter) -> (String, bool)` (~line 362). Add a new function IMMEDIATELY AFTER it (so the two sit side-by-side and the existing one is unchanged for backward compat):

```rust
/// Layout-aware filter builder that accepts an optional cam asset as a
/// SECOND ffmpeg input (`[1:v]`). The caller is responsible for adding
/// the `-i <asset>` flag (with `-loop 1` for images or `-stream_loop -1`
/// for videos) BEFORE invoking this function's parent ffmpeg command.
///
/// When `has_cam_asset` is false, this delegates to `layout_filter` so
/// existing behavior is byte-identical for VODs without an attached asset.
///
/// When `has_cam_asset` is true:
/// - `GameplayFocus`: no cam slot, ignores the flag (identical output).
/// - `Split`: top region is the full gameplay (scaled to fit the top slot),
///   bottom region is the asset (scaled to fit the bottom slot with
///   letterbox/pillarbox).
/// - `Pip`: background is the full gameplay (scaled to fill), overlay is
///   the asset (scaled to fit the slot with letterbox/pillarbox).
pub fn layout_filter_with_asset(
    mode: &LayoutMode,
    target: OutputSize,
    caption_filter: Option<&str>,
    has_cam_asset: bool,
) -> (String, bool) {
    if !has_cam_asset {
        return layout_filter(mode, target, caption_filter);
    }

    let tw = target.width;
    let th = target.height;

    match mode {
        LayoutMode::GameplayFocus => {
            // No cam slot — asset is irrelevant; identical output to no-asset path.
            layout_filter(mode, target, caption_filter)
        }

        LayoutMode::Split { ratio } => {
            let r = ratio.clamp(0.3, 0.8);
            let th_top = (th as f64 * r) as u32;
            let th_bot = th - th_top;

            // Top: full gameplay scaled to fill (tw, th_top), crop overflow.
            // Bottom: asset scaled to fit (tw, th_bot), letterbox/pillarbox.
            let mut f = format!(
                "[0:v]scale={tw}:{th_top}:force_original_aspect_ratio=increase:flags=lanczos,\
                 crop={tw}:{th_top}[top];\
                 [1:v]scale={tw}:{th_bot}:force_original_aspect_ratio=decrease:flags=lanczos,\
                 pad={tw}:{th_bot}:(ow-iw)/2:(oh-ih)/2:black[bottom];\
                 [top][bottom]vstack"
            );

            if let Some(cf) = caption_filter {
                f.push_str(&format!("[stacked];[stacked]{}[out]", cf));
            } else {
                f.push_str("[out]");
            }
            (f, true)
        }

        LayoutMode::Pip { x, y, size } => {
            let ps = (tw as f64 * size.clamp(0.15, 0.45)) as u32;
            let ox = ((tw as f64 - ps as f64) * x.clamp(0.0, 1.0)) as u32;
            let oy = ((th as f64 - ps as f64) * y.clamp(0.0, 1.0)) as u32;

            // Background: full gameplay scaled to fill (tw, th), crop overflow.
            // PiP slot: asset scaled to fit ps×ps, letterbox/pillarbox.
            let mut f = format!(
                "[0:v]scale={tw}:{th}:force_original_aspect_ratio=increase:flags=lanczos,\
                 crop={tw}:{th}[main];\
                 [1:v]scale={ps}:{ps}:force_original_aspect_ratio=decrease:flags=lanczos,\
                 pad={ps}:{ps}:(ow-iw)/2:(oh-ih)/2:black[pip];\
                 [main][pip]overlay={ox}:{oy}"
            );

            if let Some(cf) = caption_filter {
                f.push_str(&format!("[overlaid];[overlaid]{}[out]", cf));
            } else {
                f.push_str("[out]");
            }
            (f, true)
        }
    }
}
```

- [ ] **Step 4.4: Extend `ExportRequest` with `cam_asset_path`**

Find `pub struct ExportRequest` (~line 441). Add ONE field at the end (before the closing `}`):

```rust
    /// Optional path to a cam-slot asset (image or video). When `Some` and the
    /// layout is `Split` or `Pip`, this file is added as ffmpeg's second input
    /// and composited into the cam slot. `None` preserves the existing dup-source
    /// behavior. Always ignored for `GameplayFocus` (no cam slot).
    pub cam_asset_path: Option<PathBuf>,
```

- [ ] **Step 4.5: Wire `cam_asset_path` into `build_ffmpeg_command`**

Find `pub fn build_ffmpeg_command(...)` (~line 532). The current first-statement is:

```rust
    // Build the filter graph
    let caption = request.caption_filter.as_deref();
    let (filter, is_complex) = layout_filter(&request.layout, request.target, caption);
```

Replace with:

```rust
    // Build the filter graph (two-input variant if a cam asset is attached
    // AND the layout actually uses a cam slot).
    let caption = request.caption_filter.as_deref();
    let has_cam_asset = request.cam_asset_path.is_some()
        && matches!(request.layout, LayoutMode::Split { .. } | LayoutMode::Pip { .. });
    let (filter, is_complex) = layout_filter_with_asset(
        &request.layout,
        request.target,
        caption,
        has_cam_asset,
    );
```

Then find the `-i` input-flag section of the same function. The current input flag is one `cmd.arg("-i").arg(&request.source_path);` (locate in the function body). After that line, when `has_cam_asset` is true, add the second input:

```rust
    cmd.arg("-i").arg(&request.source_path);

    // Second input: cam asset (if any). Image vs video uses different loop flags.
    if has_cam_asset {
        let asset = request.cam_asset_path.as_ref().expect("checked above");
        let ext = asset
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        let is_image = matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp");
        if is_image {
            // Loop a single still image for the clip's duration.
            let dur = (request.end - request.start).max(0.01);
            cmd.arg("-loop").arg("1")
               .arg("-t").arg(format!("{:.3}", dur))
               .arg("-i").arg(asset);
        } else {
            // Loop a video (mp4/webm/mov/gif) infinitely; ffmpeg will trim
            // to the gameplay stream's length automatically via -shortest
            // semantics through the filter graph's primary input.
            cmd.arg("-stream_loop").arg("-1")
               .arg("-i").arg(asset);
        }
    }
```

(If the existing `build_ffmpeg_command` already does `cmd.arg("-i").arg(&request.source_path);` along with `-ss` / `-to` input seeking on input 0, the asset's `-i` flag must come AFTER the input-0 block. Don't reorder the input-0 flags — only insert the asset block immediately after them.)

- [ ] **Step 4.6: Verify `-map` flags still pick the right streams**

In the same function, find the `-map` arguments. The current behavior maps the labeled filter output `[out]` for video and `0:a?` for audio. That stays correct: the filter graph emits `[out]` regardless of input count, and `0:a?` explicitly maps audio from input 0 (the source VOD), which is what we want — asset audio is ignored. No change required, but verify by reading the function.

- [ ] **Step 4.7: Update existing call sites that construct `ExportRequest`**

Since Task 4.4 added a required field to `ExportRequest`, every place that constructs the struct now needs to populate `cam_asset_path`. Search for `ExportRequest {` in the codebase. There's likely one or two sites (test code in vertical_crop.rs may construct one; `commands/export.rs::clip_to_export_request` definitely does — though that's Task 5's territory).

For any sites inside `vertical_crop.rs` (tests / helpers), add `cam_asset_path: None,` to the literal. For `commands/export.rs::clip_to_export_request`, leave it for Task 5.

- [ ] **Step 4.8: Static self-review + commit**

Verify: `layout_filter` (existing) is BYTE-UNCHANGED — the no-asset path is delegated to it. `layout_filter_with_asset` is new, present, with three correct branches. `ExportRequest` has `cam_asset_path: Option<PathBuf>`. `build_ffmpeg_command` picks the right filter based on `has_cam_asset`, adds the second `-i` with the right loop flag (image vs video). Tests are inside the existing `mod tests`. Any in-file `ExportRequest { ... }` literals have `cam_asset_path: None,`.

```bash
git add src-tauri/src/vertical_crop.rs
git commit -m "feat(vertical_crop): layout_filter_with_asset + ExportRequest.cam_asset_path"
```

---

## Task 5: Wire `cam_asset_path` through the export request (Rust, static-review)

**Files:**
- Modify: `src-tauri/src/commands/export.rs`

- [ ] **Step 5.1: Extend `clip_to_export_request` to populate `cam_asset_path`**

Find `fn clip_to_export_request(clip, vod_path, output_path) -> vertical_crop::ExportRequest` (~line 288 in `src-tauri/src/commands/export.rs`). Change the signature to accept the VOD's cam asset path:

```rust
fn clip_to_export_request(
    clip: &db::ClipRow,
    vod_path: &str,
    output_path: &std::path::Path,
    cam_asset_path: Option<&str>,
) -> vertical_crop::ExportRequest {
```

In the function body, the existing struct literal builds the `ExportRequest`. Add `cam_asset_path` at the end:

```rust
    vertical_crop::ExportRequest {
        source_path: std::path::PathBuf::from(vod_path),
        output_path: output_path.to_path_buf(),
        start: clip.start_seconds,
        // ... (existing fields unchanged)
        layout,
        caption_filter,
        cam_asset_path: cam_asset_path.map(std::path::PathBuf::from),
    }
```

(Verify the actual order of fields against the struct definition. If `caption_filter` is the last existing field, append `cam_asset_path` after it. The exact field ordering must match the order in the struct.)

- [ ] **Step 5.2: Update call sites to look up the VOD's cam asset**

Find every call to `clip_to_export_request(...)` in `commands/export.rs`. Each call currently passes (clip, vod_path, output_path). The new signature needs a 4th arg.

For each call site, look up the VOD's cam_asset_path from the DB just before the call. The VOD-fetch pattern likely already exists in the calling function (to get `vod.local_path`); reuse the fetched `VodRow`:

```rust
    // existing: let vod = db::get_vod_by_id(&conn, &clip.vod_id)?.ok_or(...)?;
    // existing: let vod_path = vod.local_path.as_deref().ok_or(...)?;
    let request = clip_to_export_request(
        &clip,
        vod_path,
        &output_path,
        vod.cam_asset_path.as_deref(),
    );
```

If the calling function only had the VOD's `local_path` and not the full `VodRow`, fetch the full row once and pass both fields through. Don't re-query — one fetch covers it.

- [ ] **Step 5.3: Static self-review + commit**

Verify: `clip_to_export_request` signature has the new `cam_asset_path: Option<&str>` param; struct-literal in body includes `cam_asset_path: cam_asset_path.map(PathBuf::from)`; every call site passes `vod.cam_asset_path.as_deref()` (or equivalent); no call site forgets to add the arg (would be a compile error Slug will catch).

```bash
git add src-tauri/src/commands/export.rs
git commit -m "feat(export): pass per-VOD cam_asset_path into ExportRequest"
```

---

## Task 6: Frontend `Vod` type extension (TypeScript, build-verified)

**Files:**
- Locate + Modify: the file defining the `Vod` type / interface used in the frontend

- [ ] **Step 6.1: Locate the Vod type**

Run:

```
grep -rn "interface Vod\b\|type Vod\b" src/types src/lib src/stores
```

The expected result: one or two matches. Pick the canonical definition (the one with the most VOD fields like `download_status`, `local_path`, `analysis_status`, etc. — that's the one matching the Rust `VodRow`).

- [ ] **Step 6.2: Add the two new fields**

In the located file, find the Vod interface/type. Add two optional fields to match the Rust serde struct (which uses `#[serde(default)]` so they may be `null`):

```typescript
  cam_asset_path: string | null
  cam_asset_source: string | null
```

(Match the existing style — if other fields use `string | null`, do the same; if `string | undefined` or `string?: ...`, match that. Don't fight the file's existing pattern.)

- [ ] **Step 6.3: Verify the build**

Run from the project root:

```
npm run build
```

Expected: `tsc -b && vite build` clean. If anywhere in the codebase constructs a `Vod` literal that's now missing fields, TypeScript will flag it — go fix those (most likely test mocks or sample data). All `Vod` consumers reading the new fields don't need to be updated yet (they're optional / nullable).

- [ ] **Step 6.4: Commit**

```bash
git add src/types/  # or wherever the Vod type lives
git commit -m "feat(types): Vod.cam_asset_path + cam_asset_source"
```

(Adjust the `git add` to the actual file path.)

---

## Task 7: `CamAssetPicker` component (TypeScript, build-verified)

**Files:**
- Create: `src/components/CamAssetPicker.tsx`

- [ ] **Step 7.1: Write the component**

Create `src/components/CamAssetPicker.tsx` with the following content:

```tsx
import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open as openDialog } from '@tauri-apps/plugin-dialog'

type Props = {
  vodId: string
  /** Current managed-copy path (from vod.cam_asset_path). */
  currentPath: string | null
  /** Current original source path (from vod.cam_asset_source). */
  currentSource: string | null
  /** Called after any successful set/clear so the parent can re-fetch the VOD. */
  onChanged: () => void
}

// File extension filter for the OS file dialog. Mirrors the backend
// ALLOWED_EXTS in commands/cam_asset.rs.
const ASSET_EXTS = ['png', 'jpg', 'jpeg', 'webp', 'mp4', 'webm', 'mov', 'gif']

function basename(p: string | null): string {
  if (!p) return ''
  const parts = p.replace(/\\/g, '/').split('/')
  return parts[parts.length - 1] || p
}

export default function CamAssetPicker({ vodId, currentPath, currentSource, onChanged }: Props) {
  const [recents, setRecents] = useState<string[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [recentsOpen, setRecentsOpen] = useState(false)

  // Load recent-assets list once on mount and after any change.
  useEffect(() => {
    let cancelled = false
    invoke<string[]>('recent_cam_assets')
      .then((r) => { if (!cancelled) setRecents(r) })
      .catch(() => { if (!cancelled) setRecents([]) })
    return () => { cancelled = true }
  }, [currentSource])

  const setAsset = async (sourcePath: string) => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      await invoke('set_vod_cam_asset', { vodId, sourcePath })
      onChanged()
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
      setRecentsOpen(false)
    }
  }

  const handleChoose = async () => {
    if (busy) return
    const picked = await openDialog({
      multiple: false,
      filters: [{ name: 'Image or Video', extensions: ASSET_EXTS }],
    })
    if (typeof picked === 'string' && picked.length > 0) {
      await setAsset(picked)
    }
  }

  const handleRemove = async () => {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      await invoke('clear_vod_cam_asset', { vodId })
      onChanged()
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  const hasAsset = !!currentPath
  const displayName = currentSource ? basename(currentSource) : 'None'

  return (
    <div className="flex flex-col gap-1.5">
      <div className="text-xs text-slate-300">Cam asset</div>
      <div className="text-[10px] text-slate-500">Applies to all clips from this VOD.</div>
      <div className="flex items-center gap-2">
        <span className="text-xs text-slate-200 truncate flex-1 min-w-0" title={currentSource ?? ''}>
          {hasAsset ? displayName : 'None — pick one'}
        </span>
        <button
          type="button"
          disabled={busy}
          onClick={handleChoose}
          className="px-2 py-1 text-xs rounded-lg cursor-pointer bg-surface-700 hover:bg-surface-600 disabled:opacity-40 text-slate-200"
        >
          {busy ? '…' : 'Choose…'}
        </button>
        {recents.length > 0 && (
          <div className="relative">
            <button
              type="button"
              disabled={busy}
              onClick={() => setRecentsOpen((o) => !o)}
              className="px-2 py-1 text-xs rounded-lg cursor-pointer bg-surface-700 hover:bg-surface-600 disabled:opacity-40 text-slate-200"
              title="Recent assets"
            >
              Recent ▾
            </button>
            {recentsOpen && (
              <div className="absolute right-0 mt-1 z-10 bg-surface-800 border border-surface-600 rounded-lg shadow-lg min-w-[200px] max-w-[400px]">
                {recents.map((src) => (
                  <button
                    key={src}
                    type="button"
                    onClick={() => setAsset(src)}
                    className="w-full text-left px-3 py-1.5 text-xs hover:bg-surface-700 text-slate-200 truncate"
                    title={src}
                  >
                    {basename(src)}
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
        {hasAsset && (
          <button
            type="button"
            disabled={busy}
            onClick={handleRemove}
            className="px-2 py-1 text-xs rounded-lg cursor-pointer bg-red-500/20 hover:bg-red-500/30 text-red-300 disabled:opacity-40"
            title="Remove cam asset"
          >
            Remove
          </button>
        )}
      </div>
      {error && (
        <div className="text-xs text-red-400">{error}</div>
      )}
    </div>
  )
}
```

- [ ] **Step 7.2: Verify the build**

```
npm run build
```

Expected: clean. The component is self-contained; no other file references it yet.

- [ ] **Step 7.3: Commit**

```bash
git add src/components/CamAssetPicker.tsx
git commit -m "feat(ui): CamAssetPicker component (Choose / Recent / Remove)"
```

---

## Task 8: Embed `CamAssetPicker` in the Clip Editor (TypeScript, build-verified)

**Files:**
- Modify: `src/pages/ClipEditor.tsx`

- [ ] **Step 8.1: Locate the layout-picker section**

In `src/pages/ClipEditor.tsx`, find the section where the user picks `GameplayFocus` / `Split` / `Pip` (search for `facecam_layout`, `LayoutMode`, or button labels like `"Split"` / `"PiP"`). That section is where the CamAssetPicker row attaches.

- [ ] **Step 8.2: Import the component + helpers**

At the top of `ClipEditor.tsx`, near the other component imports, add:

```typescript
import CamAssetPicker from '../components/CamAssetPicker'
```

- [ ] **Step 8.3: Render the picker conditionally**

Inside the layout-picker section (right after the GameplayFocus/Split/Pip buttons), add:

```tsx
{(currentLayout === 'split' || currentLayout === 'pip') && vod && (
  <CamAssetPicker
    vodId={vod.id}
    currentPath={vod.cam_asset_path ?? null}
    currentSource={vod.cam_asset_source ?? null}
    onChanged={() => {
      // Re-fetch the VOD so the parent re-renders with the new cam-asset state.
      // Replace this with whatever the existing pattern is for re-loading
      // the VOD detail in this component (e.g., a store action, a useEffect
      // trigger, or a refetchVod() call).
      refetchVod?.()
    }}
  />
)}
```

The exact value for `currentLayout` and how to access `vod` depends on the existing component structure:

- `currentLayout` is likely already a local variable / state in this component, computed from the clip's `facecam_layout` field. If it's spelled differently (e.g., `selectedLayout`, `layoutMode`), use that name.
- `vod` is the parent VOD of the current clip. The Editor likely already fetches it (to get `local_path` for the player); look for an existing `vod` variable or a fetch like `await invoke('get_vod_detail', { vodId: clip.vod_id })`. If it doesn't already, add a `useEffect` to load it at mount.
- `refetchVod?.()` — the implementer locates how the editor refreshes VOD state. If the editor uses Zustand `useAppStore`, the existing pattern is `useAppStore((s) => s.fetchVods)` or similar. If it's a local fetch, expose a function. The CamAssetPicker only needs SOME callback that causes the parent to re-render with fresh `vod.cam_asset_path` after a set/clear.

If the existing editor doesn't currently have a path to the parent VOD object (only the clip), add a one-time fetch:

```typescript
const [vod, setVod] = useState<Vod | null>(null)
useEffect(() => {
  let cancelled = false
  if (!clip?.vod_id) return
  invoke<Vod>('get_vod_detail', { vodId: clip.vod_id })
    .then((v) => { if (!cancelled) setVod(v) })
    .catch(() => { if (!cancelled) setVod(null) })
  return () => { cancelled = true }
}, [clip?.vod_id])

const refetchVod = () => {
  if (!clip?.vod_id) return
  invoke<Vod>('get_vod_detail', { vodId: clip.vod_id })
    .then(setVod)
    .catch(() => {})
}
```

- [ ] **Step 8.4: Verify the build**

```
npm run build
```

Expected: clean. If TypeScript complains about types (e.g. `Vod` import missing, or `currentLayout` not matching the layout-string values), fix in-place — the build is the gate.

- [ ] **Step 8.5: Commit**

```bash
git add src/pages/ClipEditor.tsx
git commit -m "feat(editor): embed CamAssetPicker in layout panel for Split/Pip"
```

---

## Task 9: Slug verification + ship

**Files:** version-bump files only (package.json, src-tauri/Cargo.toml, src-tauri/Cargo.lock, src-tauri/tauri.conf.json).

This task is performed by Slug (cargo + live app required; not available in the VM).

- [ ] **Step 9.1: Compile + unit tests**

```
cd src-tauri
cargo check
cargo test cam_asset 2>&1 | Select-Object -Last 25
cargo test layout_filter 2>&1 | Select-Object -Last 25
cargo test 2>&1 | Select-String -Pattern "test result:|cam_asset|layout_filter|FAILED"
```

Expected: `cargo check` Finished, same ~206 pre-existing warnings as v1.3.14, 0 errors. The 7 cam_asset tests + 4 layout_filter tests all pass. Full suite: prior count + 11 new tests = `~452 passed; 0 failed`.

If any new test fails, STOP — paste the failure; do not bump the version.

- [ ] **Step 9.2: Live smoke — fresh state (no asset attached)**

```
cd ..
cargo tauri dev
```

In the running app: open the Clip Editor for any existing clip. Pick `Split` or `Pip` layout. Confirm the **"Cam asset"** row appears under the layout buttons with "None — pick one", Choose…, and (if you have any history) Recent ▾. **Do NOT attach anything yet.** Export the clip. Expected: the exported clip renders with the EXISTING dup-source behavior (a copy of the gameplay frame in the cam slot — same as v1.3.14). **No regression** for unattached VODs.

- [ ] **Step 9.3: Live smoke — attach an image**

In the editor, click **Choose…**. Pick a PNG (e.g., your slug-on-PC avatar). The row should update to show the filename. Re-export the clip. Expected: the cam slot now shows the PNG, fit inside the slot with letterbox/pillarbox bars if the aspect ratios differ. Audio is unchanged (source VOD's audio only).

- [ ] **Step 9.4: Live smoke — attach a video**

Click **Choose…** again, this time pick a short MP4 or WebM. Re-export. Expected: cam slot loops the video over the clip duration; source VOD audio plays, video asset audio is silent.

- [ ] **Step 9.5: Live smoke — recent assets**

Open a DIFFERENT clip (different VOD). In its editor, the Cam asset row should be empty. Click **Recent ▾**. Expected: the PNG and MP4 from steps 9.3-9.4 appear in the list by source basename. Click one. Expected: copies that source into the new VOD's managed slot; the row updates to show it.

- [ ] **Step 9.6: Live smoke — remove**

Click **Remove**. Expected: row returns to "None — pick one"; re-export confirms cam slot is back to dup-source behavior; the managed file at `%APPDATA%\clipviral\assets\<vod_id>.<ext>` is gone.

- [ ] **Step 9.7: Regression smoke**

Open a healthy clip that uses `GameplayFocus`. Export. Confirm: byte-identical-looking output to v1.3.14 (the asset flag is ignored for GameplayFocus). A VOD with no asset attached + Split/Pip layout: still dup-source.

- [ ] **Step 9.8: Version bump + ship**

This release is **user-facing** (real new feature). Pick a version number — recommend **v1.3.16** (slots after v1.3.15's diagnostics-cleanup release if you ship that first; otherwise v1.3.15 directly is fine — adjust the bump command).

```powershell
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
powershell -File bump-version.ps1 1.3.16
git add package.json src-tauri/Cargo.lock src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -m "chore: bump to v1.3.16 (per-VOD cam asset)"
git tag -a v1.3.16 -m "v1.3.16 -- per-VOD cam asset (image/video in Split/Pip slot)"
git push origin main
git push origin v1.3.16
```

When CI finishes, publish the v1.3.16 release directly from the CI-generated draft (avoid the web edit-by-tag link — see CLAUDE.md process note from v1.3.14 release).

User-facing release notes:

> **Bring your avatar to your clips.** Attach an image or video file (PNG, JPG, WebP, MP4, WebM, MOV, GIF) to a VOD, and any clip from that VOD using the PIP or Split layout will composite it into the cam slot — your VTube model, your avatar PNG, your facecam loop, anything you'd put in OBS. Editor → pick PIP or Split → "Cam asset" row appears → Choose your file. Apply once per VOD; clips inherit. Reuse the same asset across VODs in one click via the **Recent** dropdown.

---

## Self-review

(Plan author, fresh eyes against the spec.)

### Spec coverage

Walking each spec section against the plan:

- **§1 Background** — context, no implementation. ✓
- **§2 Goals & success criteria** — Tasks 1-5 collectively implement attachment + compositing; Task 7 is the picker; Task 9 verifies "no regression for unattached VODs" (§2 prime goal) in Step 9.2 explicitly. ✓
- **§3 In-scope: per-VOD attachment** — Tasks 1 (DB), 3 (commands). ✓
- **§3 In-scope: image+video** — Task 3's `ALLOWED_EXTS` + Task 4's image-vs-video branching in `build_ffmpeg_command`. ✓
- **§3 In-scope: ffmpeg compositing** — Task 4. ✓
- **§3 In-scope: scale-to-fit + letterbox** — Task 4's `force_original_aspect_ratio=decrease` + `pad=...:black` (verified by Task 4.1 tests). ✓
- **§3 In-scope: recent-assets dropdown** — Task 3's `recent_cam_assets` SQL + Task 7's `Recent ▾` UI. ✓
- **§3 In-scope: backward compat** — Task 4.3's no-asset delegation to existing `layout_filter` + Task 4.1's test `layout_filter_no_cam_asset_preserves_existing_dup_source_behavior` is the explicit safeguard. ✓
- **§3 Out-of-scope: crop-from-source, per-clip override, asset library, fit options, preview parity** — none implemented in any task. ✓
- **§4.1 Coordinate model** — implementation respects it (asset is composited into the output-frame coords by `overlay={ox}:{oy}` in Pip and `vstack` in Split, both at output coords). ✓
- **§4.2 Data model** — Task 1.1 (migrations), 1.2 (struct), 1.3 (SELECTs), 1.4 (helpers). ✓
- **§4.3 Storage layout** — Task 2 (assets_dir), Task 3.1 (managed_asset_path + extension-collision cleanup + missing-file warn on clear). ✓
- **§4.4 UI** — Task 7 (picker component) + Task 8 (Editor integration with scope-label sub-text). ✓
- **§4.5 ffmpeg compositing** — Task 4.3 (filter) + Task 4.5 (input flags) + Task 4.6 (map-flag confirmation). ✓
- **§5 File-level changes** — Tasks 1-8 map 1:1 to the spec's file list. ✓
- **§6 Watchouts** — backward compat (Task 4.1 test), file copy at pick time (Task 3.1), missing managed file at export time (NOT explicitly tested but the existing ffmpeg call will error out if input missing — accepted, falls back to the broader "VOD card surfaces failed export" path which already exists), video audio ignored (Task 4.5 only adds `-i` for the asset; `-map "0:a?"` from input 0 is the existing behavior), GIF as video (Task 4.5's `is_image` set: gif is NOT in the image set, falls into the video branch — confirmed by reading the match), path with spaces / unicode (`std::fs::copy` and `cmd.arg(&asset)` both handle arbitrary paths; ffmpeg quoting is the same as the existing source path). ✓
- **§7 Open questions** — none in the spec, none in the plan. ✓

No spec gaps.

### Placeholder scan

Searched for the red flags:
- "TBD" / "TODO" / "implement later": none.
- "Add appropriate error handling": none. Error handling is concretely specified at each touch point (`?` on Result, `.map_err` with formatted strings, `log::warn!` on best-effort cleanup, file-validation guards on `set_vod_cam_asset`).
- "Write tests for the above" without code: only Task 4.1 mentions tests, and the FULL test code is given in the step.
- "Similar to Task N": none. Each task's code is self-contained.
- Steps describing what without showing how: each code change has the literal code.
- Soft references: Task 1.3 says "indices 16 and 17 assume existing 16 columns" — this is a precise instruction with an explicit verification step ("verify against the actual query by counting"), not a placeholder. Task 6.1's "Locate the Vod type" uses a precise grep — explicit instruction, not placeholder. Task 8.1's "Locate the layout-picker section" similarly specifies search terms.

### Type / name consistency

- `cam_asset_path` / `cam_asset_source` — DB columns (Task 1.1), VodRow fields (Task 1.2), `update_vod_cam_asset` params (Task 1.4), `ExportRequest.cam_asset_path: Option<PathBuf>` (Task 4.4), Vod TypeScript interface (Task 6.2), CamAssetPicker props (Task 7.1). All consistent.
- `set_vod_cam_asset` / `clear_vod_cam_asset` / `recent_cam_assets` — Tauri command names, used identically in lib.rs handler list (Task 3.3), the `use` import (Task 3.3), and the frontend `invoke('...')` calls (Task 7.1). ✓
- `update_vod_cam_asset` (db helper, no `_db` suffix) vs `clear_vod_cam_asset_db` (db helper, with `_db` suffix) vs `set_vod_cam_asset` / `clear_vod_cam_asset` (command names) — naming is intentional to avoid the command and the db helper having identical names. Task 3's command calls db's `update_vod_cam_asset` (for set; the verb is `set` at command layer, `update` at db layer — fine) and `clear_vod_cam_asset_db` (suffixed to avoid clashing with the command's `clear_vod_cam_asset`). Consistent throughout.
- `managed_asset_path` / `extension_lower` / `is_allowed_ext` — Task 3.1's pure helpers, exercised by Task 3.1's tests. ✓
- `layout_filter` (existing, unchanged) vs `layout_filter_with_asset` (new, in Task 4.3) — both `pub`, both in `vertical_crop.rs`. The new one delegates to the old one for the no-asset path. Tests in Task 4.1 reference both. ✓
- `has_cam_asset: bool` — flag in `layout_filter_with_asset` (Task 4.3) and computed in `build_ffmpeg_command` (Task 4.5). ✓
- `ALLOWED_EXTS` — Rust constant (Task 3.1), mirrored as `ASSET_EXTS` in the React component (Task 7.1) for the file-dialog filter. The names differ (intentional language convention) but the content is identical. Note for future cleanup: a future task could derive the list dynamically (e.g., a `get_allowed_cam_asset_exts` command), but for v1 this dual-listing is acceptable (the backend is the source of truth — frontend mirroring is just a UX nicety and a backend rejection on submission catches drift).

No type/name drift detected.
