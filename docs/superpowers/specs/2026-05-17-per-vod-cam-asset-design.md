# Per-VOD Cam Asset — Design

**Status:** Approved in brainstorming; pending written-spec review
**Date:** 2026-05-17
**Target release:** Next release post-v1.3.14 (version pending — likely v1.3.16; sequencing is a separate priority call against v1.3.15's diagnostics/cleanup scope)

---

## 1. Background

ClipGoblin's vertical export pipeline (`src-tauri/src/vertical_crop.rs`) has a `LayoutMode` enum with three variants — `GameplayFocus`, `Split { ratio }`, and `Pip { x, y, size }`. The `Split` and `Pip` modes name a **facecam slot** in their doc comments ("facecam on bottom", "facecam overlay in corner") and the database has a `facecam_layout` column per clip — so the slot is a first-class concept in the data model.

But there's a quiet gap: the **slot has no real content**. Both `Split` and `Pip` build their ffmpeg filter graph using `split` (which *duplicates the gameplay stream*) — so the "facecam area" is really a second crop of the same gameplay frame, not an actual cam source. A user picking PIP gets a smaller copy of the same gameplay tucked into the corner, with no way to put their own avatar, VTube image, or any other asset there. The code's own author already saw the gap and left a hint: an unused `CropAnchor::Offset(f64)` enum variant with the comment "*Useful for future facecam-aware cropping*".

User-surfaced via real workflow: streamer uses a slug-on-PC PNG as their on-stream avatar but has no way to make ClipGoblin clips show it in the cam slot.

This spec adds the missing piece — per-VOD attachment of an image or video asset that fills the cam slot for any `Split` or `Pip` clip derived from that VOD. Feature is purely additive and opt-in: existing clips without an attached asset render exactly as they do today.

## 2. Goals & success criteria

### Goals
- Streamers can attach an image or video file (their avatar, VTube model, looping cam loop, etc.) to a VOD; all clips from that VOD with `Split` or `Pip` layouts composite that asset into the cam slot.
- The feature is fully opt-in: a VOD with no attached asset renders exactly as it does today (no regression for existing clips or users not using the feature).
- Asset is robust to the user moving/deleting the source file: ClipGoblin owns its own copy in the app data dir.
- Reuse across VODs is one click via a recent-assets dropdown — no re-navigating to the file every time.

### Success criteria
- A VOD with an attached PNG: any clip from that VOD set to PIP shows the PNG in the corner slot (correct aspect-ratio fit), any clip set to Split shows it in the bottom panel, any clip set to GameplayFocus is unaffected.
- A VOD with an attached MP4/WebM/GIF: same as above, with the video looped to fill the clip duration. Audio from the asset is ignored — only the source VOD's audio reaches the clip.
- A VOD with NO attached asset: rendering output is byte-equivalent to current v1.3.14 behavior for every layout.
- `cam_asset_path` is set/cleared by frontend; backing file is copied into a managed directory and survives user moving the original source file.
- Recent-assets dropdown surfaces the last 5 distinct paths used across the library; picking from it copies that file into the new VOD's slot.

## 3. Scope (explicit in / out)

### In v1
- Per-VOD attachment (one asset per VOD; affects all clips derived from that VOD).
- Asset types: PNG, JPG, JPEG, WebP, MP4, WebM, MOV, GIF (image-or-video; type inferred from extension).
- ffmpeg compositing replaces the current `split`-duplicate-source filter for Pip/Split *when an asset is attached*.
- Scale-to-fit inside the cam slot, preserving aspect ratio (letterbox/pillarbox if slot and asset aspect ratios differ; no stretch).
- Recent-assets dropdown (last 5 distinct paths).
- Backward compat: VODs with no asset render with current behavior. Existing clips from existing VODs are completely unaffected.

### Explicit out of scope (deferred to v1.1+ or a separate spec)

These came up in brainstorming and were deliberately punted:

- **Crop-from-source region** (extract the streamer's baked-in webcam from a region of the source recording and use it as cam content). Needs a "draw rectangle on a sample frame" UI plus separate filter logic; meaningful extra scope. Worth its own design cycle.
- **Per-clip override** of the per-VOD asset. The existing per-clip `facecam_layout` already provides "hide cam for this clip" (just pick `GameplayFocus`); a per-clip *different asset* hasn't been requested and YAGNI for v1.
- **Asset library / named saved avatars** (a first-class library where assets are entities you name and reuse). The recent-assets dropdown is the cheap version that covers the realistic reuse pattern without the schema overhead.
- **Aspect-ratio fit options** beyond scale-to-fit (cover/crop, stretch, custom anchor). Sensible default is enough for v1; add if users ask.
- **Asset positioning/sizing independent of the layout slot.** The layout's existing `Pip { x, y, size }` and `Split { ratio }` parameters already control where and how big the cam slot is; the asset just fills whatever slot the layout defines.
- **Editor preview parity with the export layout.** The current Clip Editor preview uses an HTML5 `<video>` element rendering the raw source VOD with start/end offsets — it does **not** composite any layout (PIP/Split/GameplayFocus). Layouts are export-time-only today. The cam asset follows the same pattern: visible at export, not in the editor preview. Building a layout-composition preview pipeline is a broader layout-system feature, out of scope here and out of scope for this whole feature area until someone explicitly asks.

## 4. Design

### 4.1 Coordinate model (worth stating explicitly)

A subtle point that bit during brainstorming: **the cam slot lives in the OUTPUT frame, not in the source.** The layout's `x/y/size` and `ratio` parameters are all relative to the final 9:16 vertical frame. The attached asset gets composited into that slot directly — it is *not* extracted from any region of the source video. This is why "what if the streamer's real webcam was in a corner that the vertical crop chopped off?" doesn't apply to this feature: we're not reading from the source for cam content. (That scenario is the crop-from-source feature, deferred.)

### 4.2 Data model

Two new columns on the `vods` table:

```sql
ALTER TABLE vods ADD COLUMN cam_asset_path TEXT;
ALTER TABLE vods ADD COLUMN cam_asset_source TEXT;
```

Both additive, NULL-default; no risk to existing rows. Standard SQLite migrations in `db.rs`'s init/migration code.

- **`cam_asset_path`** — absolute path to the *managed copy* of the asset (`%APPDATA%\clipviral\assets\<vod_id>.<ext>` on Windows; mirrors how `local_path` works for downloaded VOD files). This is what ffmpeg reads at composite time. Robust to the user moving/deleting the source.
- **`cam_asset_source`** — the *original path the user picked* (verbatim, e.g. `C:\Users\Slug\Pictures\slug-on-pc.png`). Stored alongside the managed copy purely so the recent-assets dropdown can show meaningful display names and identify reuse: if the user picks the same source for VODs A and B, the managed copies are two distinct files (under different vod-ids), but `cam_asset_source` is the same string for both → the dropdown collapses them into one logical entry. If the source path doesn't exist anymore when the user re-picks from recents, the picker shows an error and falls back to the standard Choose flow.

Asset type discrimination is by file extension at composite time (no separate `kind` column needed):
- Image: `.png`, `.jpg`, `.jpeg`, `.webp`
- Video: `.mp4`, `.webm`, `.mov`, `.gif`

(GIF is treated as video for ffmpeg purposes — `-stream_loop` handles it cleanly.)

### 4.3 Storage layout

- New managed directory: `%APPDATA%\clipviral\assets\` (created on first use; mirrors `%APPDATA%\clipviral\bin\` pattern from `bin_manager`).
- On pick: copy user-selected file → `%APPDATA%\clipviral\assets\<vod_id>.<original_ext>`.
- On replace (user picks a new asset for the same VOD): overwrite the existing file.
- On clear/remove: delete the managed copy, set `cam_asset_path = NULL`.
- On VOD deletion: clean up the asset file alongside any other managed files (extend whatever path-cleanup runs for `local_path`).
- If the managed copy is missing at composite time (user manually deleted from disk between attach and clip render): fall back to the current behavior (rendered as if no asset attached) and `log::warn!`. Do not hard-fail the export.

### 4.4 UI

**Location:** in the Clip Editor's layout panel, where the user already picks `GameplayFocus` / `Split` / `Pip`.

**Visibility rule:** the cam-asset picker row is shown only when the currently-selected layout is `Split` or `Pip` (no asset matters for `GameplayFocus`).

**Row contents:**
- Label: **"Cam asset"**, scope-clarifying sub-label: *"Applies to all clips from this VOD."* (Set explicitly so a user editing one clip doesn't get surprised that the change reaches sibling clips.)
- Current state shown: small thumbnail (image) or filename + duration (video), or "None — pick one" placeholder.
- **Choose…** button: native file picker scoped to `.png .jpg .jpeg .webp .mp4 .webm .mov .gif`. On pick: invoke `set_vod_cam_asset(vodId, sourcePath)` → backend copies + writes column → UI refreshes.
- **Recent ▾** dropdown next to Choose: surfaces the last 5 distinct *`cam_asset_source`* values across the library, most-recent first. Display name is the source's basename (`slug-on-pc.png`). Picking from it re-runs the file copy using that source path as input — same flow as Choose, just without the file picker step. If the source file no longer exists at that path, surface a clear error (e.g. "Source file moved or deleted: <path>. Pick another.") and keep the row empty so the user can retry via Choose.
- **Remove** button (shown only when an asset is set): clears via `clear_vod_cam_asset(vodId)`.

**Preview:** the existing layout/preview surface in the editor should render the attached asset in its slot at preview time (using the same filter logic the export uses — see §4.5 — but on a sampled frame or low-res preview pipeline if a full filter pass is too heavy).

### 4.5 ffmpeg compositing

When `cam_asset_path` is set AND the layout is `Split` or `Pip`, the existing `split`-duplicate-source filter graph is replaced with a two-input graph. The current single-input filter (Pip and Split each build their own `-filter_complex` today in `vertical_crop.rs`'s layout-aware filter builder) gets extended to accept an optional asset path.

**For image assets** (PNG/JPG/WebP):
- Add a second input: `-loop 1 -t <clip_duration_seconds> -i "<asset_path>"`
- `filter_complex`:
  - `[0:v]` — gameplay region (scaled/cropped per the existing layout-aware logic for the gameplay portion of the output frame)
  - `[1:v]` — asset: scaled to fit the cam slot's dimensions (using `scale=<w>:<h>:force_original_aspect_ratio=decrease`), then padded to exact slot dimensions (`pad=<w>:<h>:(ow-iw)/2:(oh-ih)/2:black`)
  - For PIP: `overlay=<x_px>:<y_px>` of the asset over the gameplay
  - For Split: `vstack` (or `hstack` for hypothetical horizontal split) of the gameplay above the asset, sized per `ratio`
- Audio: passthrough from input 0 only (`-map "0:a?"`); asset's audio is ignored.

**For video assets** (MP4/WebM/MOV/GIF):
- Same as above but with `-stream_loop -1 -i "<asset_path>"` for the second input (loops the video for the duration of the clip).
- Same filter graph; ffmpeg treats the second input as a video stream identically.
- Audio: still passthrough from input 0 only — never mix in asset audio in v1 (could be a future option if anyone asks).

**For VODs with no cam asset:** no change — the existing `split`-based filter graph stays exactly as it is today. This is the bedrock of the no-regression promise.

**Aspect-ratio fit:** explicitly `scale=...:force_original_aspect_ratio=decrease` + `pad=...:black`. Letterbox/pillarbox bars are black. Stretching is never done (that distorts the user's art).

### 4.6 Per-clip override (NOT in v1)

The existing per-clip `facecam_layout` field already handles "hide cam for this clip" — just pick `GameplayFocus` on that clip and the asset is unused. So a v1 user who wants to skip the cam on a single clip already has the mechanism. A per-clip *different asset* is genuinely unsupported in v1 (deferred). If anyone asks, a `clip.cam_asset_path` override column + UI extension would be straightforward to add later.

## 5. File-level changes (logical map; plan pins exact lines)

### Backend (Rust)
- **`src-tauri/src/db.rs`** — schema migration (additive ALTER TABLE), `update_vod_cam_asset(conn, vod_id, path)` and `clear_vod_cam_asset(conn, vod_id)` helpers, include `cam_asset_path` in the `VodRow` struct + `get_vod_by_id` / `get_all_vods` SELECTs.
- **`src-tauri/src/vertical_crop.rs`** — extend the layout-aware filter builder to accept an `Option<&Path>` cam asset and emit the two-input filter graph when present. The existing single-input path stays for the `None` case.
- **`src-tauri/src/commands/`** — two new commands. Either added to `commands/vod.rs` (if it can absorb without blowing up further — it's already 2000+ lines) OR extracted into a new `commands/cam_asset.rs` module (recommended; small focused scope, cleaner module boundary). The commands:
  - `set_vod_cam_asset(vod_id: String, source_path: String, app: AppHandle, db: State<DbConn>) -> Result<(), String>` — validates the source path exists, copies file to managed dir, writes both `cam_asset_path` (managed) and `cam_asset_source` (original).
  - `clear_vod_cam_asset(vod_id: String, app: AppHandle, db: State<DbConn>) -> Result<(), String>` — deletes managed file, clears both columns.
  - `recent_cam_assets(db: State<DbConn>) -> Result<Vec<String>, String>` — returns the last 5 distinct non-NULL `cam_asset_source` values, ordered by `MAX(created_at) DESC` over the VODs that used each source. (`vods` already has `created_at`; no extra timestamp column needed for v1.)
- **`src-tauri/src/commands/export.rs`** — when invoking the filter builder for `Split`/`Pip` clips, look up the parent VOD's `cam_asset_path` and pass it through.
- **`src-tauri/src/lib.rs`** — register the 3 new commands in `generate_handler![]` and import them from the new module if extracted.

### Frontend (TypeScript/React)
- **`src/pages/ClipEditor.tsx`** — render the cam-asset row in the existing layout panel when the active layout is `Split` or `Pip`.
- **`src/components/CamAssetPicker.tsx`** (new) — encapsulates the row UI: thumbnail/filename display, Choose button (Tauri dialog plugin), Recent dropdown, Remove button. Calls the 3 new Tauri commands.
- **`src/types/editTypes.ts`** (or wherever VOD/clip types live) — extend `Vod` type to include `cam_asset_path: string | null`.
- **`src/stores/`** — any VOD-related store needs to surface `cam_asset_path` from `get_vod_detail`. Likely just a type addition; the existing fetch/store wiring carries new fields through automatically.

### Database
- One additive `ALTER TABLE vods ADD COLUMN cam_asset_path TEXT;` migration, wired into the existing migration runner in `db.rs`.

## 6. Watchouts

- **Backward compat is the prime directive.** A user who never touches this feature must see byte-identical export output for existing clips. The "no asset attached → existing filter" path is the no-regression guarantee; protect it carefully.
- **File copy at pick time.** Don't store the user's original path; copy into the managed dir. Otherwise the user moves their `slug.png` → all their clips silently break later. (Same pattern `bin_manager` uses for tools — proven.)
- **Missing managed file at export time** (user manually wiped the assets dir): fall back to the no-asset filter and `log::warn!`. Never hard-fail an export. The user gets a clip without the avatar (degraded) instead of no clip at all.
- **Video asset audio.** Always discard it in v1 (`-map "0:a?"` on input 0 only). Mixing source + asset audio is a separate UX problem.
- **GIF.** Treat as video (`-stream_loop`); never use the still-image filter path even though GIF has both. Cleaner ffmpeg behavior.
- **Path with spaces / unicode.** Copy with `std::fs::copy` (handles arbitrary paths); pass to ffmpeg quoted. Existing `vertical_crop.rs` filter-build already handles arbitrary paths in `-i`, but verify the new two-input case quotes the asset path the same way.
- **`vod.rs` is already 2000+ lines.** Strongly prefer extracting the new commands into `commands/cam_asset.rs` rather than further bloating `vod.rs`. This is consistent with the M6 backlog item from v1.3.14 review ("vod.rs growth — extract persist cap"); same pattern.
- **Preview / export divergence (acknowledged, not solved here).** The editor preview shows raw source video; the exported clip shows the layout-composited result. This is true for *all* layouts today, not new to this feature — but the cam asset makes the gap more user-visible (you pick a slug PNG, the preview still shows the dup-source dummy, then on export the slug appears). Mitigation in v1: the cam-asset-row in the editor shows a thumbnail of the asset so the user has visual confirmation of *what* is attached, even if not *how it composites*. Full preview-layout composition is deferred per §3 out-of-scope.

## 7. Open questions

None — all resolved in brainstorming:
- Source of truth: **per-VOD** (chosen by user from 4 options).
- Asset types: **image + video** (chosen by user from 3 options).
- UI location: **Editor only**, no VOD-card button in v1.
- Recent-assets dropdown: **included** in v1.
- "Out of view" clarification: cam slot is in OUTPUT-frame coordinates, asset fills it independent of source content; the crop-from-source scenario is a separate deferred feature.
