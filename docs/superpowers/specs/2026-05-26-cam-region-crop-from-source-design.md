# Crop-from-source cam region — Design

**Status:** Approved in brainstorming; pending written-spec review
**Date:** 2026-05-26
**Supersedes:** [`2026-05-17-per-vod-cam-asset-design.md`](2026-05-17-per-vod-cam-asset-design.md)
**Target release:** next user-facing (v1.3.16 or v1.4.0 — chosen at ship time)

---

## 1. Background

v1.3.16 ("per-VOD cam asset") let the user attach an *external* file (PNG / MP4 / etc.) to a VOD; that file would fill the cam slot in PiP and Split layouts. It was built end-to-end (11 commits, all green) but never shipped — when the user first hit the picker in `cargo tauri dev`, the UX mismatch was immediate: he expected to mark a *region of the source frame* (his slug-on-PC avatar was already baked into the source by OBS during streaming), not attach a separate file.

This design replaces v1.3.16 entirely. v1.3.16's 11 implementation commits are reverted before this design starts. The pre-v1.3.16 dup-source path (gameplay duplicated into the cam slot) is preserved byte-identical as the no-region fallback.

## 2. Goals & success criteria

### Goals

- Let the user mark a rectangle on the *source* VOD frame and have that exact region fill the cam slot in every clip from that VOD using PiP or Split layout.
- Keep the selection UX inside the existing editor — no modals, no separate page.
- Default behavior (no region set) is identical to pre-v1.3.16: existing clips and existing VODs export the same as they did at v1.3.14.
- Layout-appropriate bar fill: PiP bars are gameplay pass-through (avatar floats over the gameplay); Split bars are a heavy-blurred copy of the source region (Instagram-style backdrop).
- Provide a per-clip override option behind a Settings toggle, off by default.

### Success criteria

- Setting a region for a VOD takes ≤ 3 clicks: open editor → click "Set region…" → drag rectangle → click Save.
- Same region applies to every clip from that VOD; no per-clip re-drag required (in default mode).
- Existing healthy clips that use Split or PiP layout export byte-identical output before vs after the feature lands (regression-tested via `layout_filter_no_region_byte_identical_to_old` unit test).
- A clip whose VOD has a region produces ffmpeg output where the cam slot shows the region pixels — fit-letterboxed by default, with the layout-specific bar fill.
- The per-clip override toggle in Settings is off by default; turning it on reveals the override UI in the editor; turning it back off hides the UI but preserves any saved overrides in DB.

### Out of scope (deferred)

- Multiple regions per VOD (switching avatars mid-stream)
- Region preset library across VODs
- ML-based automatic region detection
- Animated regions that move over time
- Multi-avatar composition (>1 cam region per clip)
- Region-shaped overlays (circle / rounded-rect masks)
- Position / zoom within the slot (`object-position` semantics)
- Variable blur intensity slider for Split
- Real-time blur / passthrough preview in the editor's CSS/canvas preview (export-only)
- GameplayFocus cropping hint (different feature)
- `DROP COLUMN` cleanup for the dead v1.3.16 `cam_asset_path` / `cam_asset_source` columns

## 3. Design

### 3.1 The three independent knobs

| Knob | Scope | New? | Stored as |
|---|---|---|---|
| **Source region** — which rectangle of the source frame is the cam | per-VOD | new | `vods.cam_region_norm` (JSON `{x,y,w,h}` in normalized 0..1) |
| **Slot position + size in output** — where the cam goes in the exported frame | per-clip | unchanged | existing `DraggablePipOverlay` + `splitRatio` slider |
| **Fit mode** — how the source region maps into the slot | per-clip | new | `clips.cam_fit_mode` (`'fit' \| 'fill' \| 'stretch'`, NULL = `'fit'`) |

The three knobs are orthogonal: changing one never silently changes another.

### 3.2 Layout-specific composition (no knob — hardcoded)

- **PiP + region:** cam region overlays gameplay at the slot's position+size. No slot rectangle drawn; gameplay shows through where the (Fit-mode-sized) cam doesn't fill the slot bounds. Cam is centered within the slot box. Visually the avatar floats over gameplay like a real OBS overlay.
- **Split + region:** bottom slot fills with a heavy-blurred copy of the source region (`boxblur=20:5`, scaled up with `force_original_aspect_ratio=increase` so the slot is fully covered). The sharp source region is composited on top of the blur, centered, at its natural aspect (after Fit-mode scale).
- **GameplayFocus:** no cam slot exists — region is irrelevant, ignored entirely. Output byte-identical to current.

### 3.3 Data model

**SQLite migrations (idempotent, additive):**

```sql
ALTER TABLE vods  ADD COLUMN cam_region_norm           TEXT;  -- JSON {x,y,w,h} or NULL
ALTER TABLE clips ADD COLUMN cam_region_norm_override  TEXT;  -- JSON {x,y,w,h} or NULL
ALTER TABLE clips ADD COLUMN cam_fit_mode              TEXT;  -- 'fit' | 'fill' | 'stretch' or NULL (defaults to 'fit')
-- Settings key (existing k/v table):
INSERT OR IGNORE INTO settings(key,value) VALUES('allow_per_clip_cam_region_override','false');
```

**Rust:**

```rust
// New helper in commands/cam_region.rs (or similar):
#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct CamRegion { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }
// Clamps to [0.0, 1.0] on parse; rejects {w,h} < 0.05 (minimum-size guard).

#[derive(Serialize, Deserialize, Clone, Copy)]
pub enum CamFitMode { Fit, Fill, Stretch }
impl Default for CamFitMode { fn default() -> Self { CamFitMode::Fit } }

// VodRow gains:
#[serde(default)] pub cam_region_norm: Option<String>,

// ClipRow gains:
#[serde(default)] pub cam_region_norm_override: Option<String>,
#[serde(default)] pub cam_fit_mode:             Option<String>,
```

**TypeScript** (`src/types.ts`):

```typescript
interface Vod {
  // ...existing fields
  cam_region_norm: string | null;
}
interface Clip {
  // ...existing fields
  cam_region_norm_override: string | null;
  cam_fit_mode: 'fit' | 'fill' | 'stretch' | null;
}
```

**Defensive refactor (re-applied from the reverted v1.3.16 work):** extract the `vods` SELECT column list into a `VOD_SELECT` constant in `db.rs` so the three `get_vods_*` query builders share one source of truth. Same goes for `CLIP_SELECT`. Prevents the column-drift bug class.

### 3.4 UI surface

**New row in the existing Layout section of the editor's right rail (`src/pages/Editor.tsx`):**

```text
┌─────────────────────────────────────────────────┐
│ ▼ Layout  · PiP            [Change]             │
│ ┌─────────────────────────────────────────────┐ │
│ │ PiP position / size sliders     (unchanged) │ │
│ ├─────────────────────────────────────────────┤ │
│ │ CAM REGION (per-VOD)                        │ │
│ │   12%, 78% · 22×22%   [Set region…] [Clear] │ │
│ │   From source frame.                        │ │
│ ├─────────────────────────────────────────────┤ │
│ │ FIT MODE (per-clip)        [Fit ▾]          │ │
│ ├─────────────────────────────────────────────┤ │
│ │ (only when override-toggle is ON in settings)│ │
│ │ PER-CLIP OVERRIDE                           │ │
│ │   Using VOD default      [Override…]        │ │
│ └─────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────┘
```

**"Set region…" flow:**

1. Player pauses at current playhead position.
2. Bottom of the source player gets a thin purple banner: *"Drag a rectangle on the source. Save when you're happy with it."*
3. A draggable rectangle overlay appears on the source frame:
   - Starts at the last-saved region for this VOD, or `{x: 0.05, y: 0.70, w: 0.25, h: 0.25}` on first use (bottom-left, 25%×25% of source frame).
   - 4 corner + 4 edge handles (mirrors the existing `DraggablePipOverlay` pattern).
   - Drag body to move. Minimum size 5% × 5% (enforced UI-side).
4. The Cam region row flips to **Save** (primary) / **Cancel** buttons.
5. Playback controls disabled during edit mode. `Esc` = Cancel.
6. Save → writes `cam_region_norm` to the VOD row → exits mode → playback re-enabled.

**Fit mode dropdown:** plain `<select>` with three options. Disabled when layout is GameplayFocus (no cam slot) or when no region is set yet (tooltip: *"Set a cam region first"*).

**Per-clip override sub-row (only when toggle ON in Settings):**

- *"Using VOD default"* + **Override…** button, OR
- shows override coords + **Edit…** + **Reset to VOD** buttons.

The override uses the same Set Region mode UI; result saves to `clips.cam_region_norm_override` instead of the VOD column.

**Settings page (`src/pages/Settings.tsx`) adds one toggle:**

> **Per-clip cam region overrides** [ off / on ]
> *"When on, each clip can override its VOD's cam region. Off keeps the simpler one-region-per-VOD flow."*

Setting persisted in the existing `settings` k/v table (same one that holds `ytdlp_last_refresh`) under key `allow_per_clip_cam_region_override`.

### 3.5 ffmpeg implementation

**Region resolution** (computed at export time, in `clip_to_export_request` or a new helper):

```rust
let effective_region = if settings.allow_per_clip_override
    && clip.cam_region_norm_override.is_some()
{
    clip.cam_region_norm_override.as_deref()
} else {
    vod.cam_region_norm.as_deref()
};
```

If `effective_region` is `None`, the filter graph degrades to the existing dup-source path (byte-identical to v1.3.14). If `Some`, the new filter shape is used.

**Filter graph — single input** (no second `-i`); source is split internally.

**PiP + region set:**
```text
[0:v]split=2[gp_src][cam_src];
[gp_src]scale=TW:TH:force_original_aspect_ratio=increase:flags=lanczos,crop=TW:TH[main];
[cam_src]crop=iw*Rw:ih*Rh:iw*Rx:ih*Ry,<FIT_SCALE>[cam];
[main][cam]overlay=SLOT_X+(SLOT_W-w)/2:SLOT_Y+(SLOT_H-h)/2[out]
```

The overlay expression centers the (possibly smaller-than-slot) cam within the slot's bounding box; when Fit/Stretch produce a cam exactly matching slot dims, the `(SLOT_W-w)/2` term evaluates to 0 — same result, no branch.

**Split + region set:**
```text
[0:v]split=3[gp_src][cam_blur_src][cam_sharp_src];
[gp_src]scale=TW:TH_TOP:force_original_aspect_ratio=increase:flags=lanczos,crop=TW:TH_TOP[top];
[cam_blur_src]crop=iw*Rw:ih*Rh:iw*Rx:ih*Ry,scale=TW:TH_BOT:force_original_aspect_ratio=increase:flags=lanczos,crop=TW:TH_BOT,boxblur=20:5[blur_bg];
[cam_sharp_src]crop=iw*Rw:ih*Rh:iw*Rx:ih*Ry,<FIT_SCALE_BOT>[sharp_fg];
[blur_bg][sharp_fg]overlay=(W-w)/2:(H-h)/2[bottom];
[top][bottom]vstack[out]
```

**`<FIT_SCALE>` mapping** (computed from `cam_fit_mode`, default Fit). Substitute `W:H` with the slot's pixel dimensions for the layout in play:

| Mode    | ffmpeg snippet                                                                |
|---------|-------------------------------------------------------------------------------|
| Fit     | `scale=W:H:force_original_aspect_ratio=decrease:flags=lanczos`                |
| Fill    | `scale=W:H:force_original_aspect_ratio=increase:flags=lanczos,crop=W:H`       |
| Stretch | `scale=W:H:flags=lanczos`                                                     |

- **PiP slot:** `W = SLOT_W`, `H = SLOT_H` (the PiP slot is a square in current layouts, so `W == H` typically — but the substitution handles non-square correctly if that ever changes).
- **Split bottom slot:** `W = TW`, `H = TH_BOT` (wide rectangle that spans the full output width by `1 - ratio` of output height).

**Normalized coords → ffmpeg `crop` expression** uses ffmpeg's built-in source-dimension variables, so no upfront ffprobe needed:

```text
{"x":0.12,"y":0.78,"w":0.22,"h":0.22}
                  ↓
crop=iw*0.22:ih*0.22:iw*0.12:ih*0.78
```

**Caption filter wiring:** the existing `caption_filter` is appended just before `[out]` unchanged — chained onto `vstack` (Split) or `overlay` (PiP) output.

### 3.6 Backward compatibility

- All three new columns are nullable, default NULL. Existing rows have NULL → behave exactly like v1.3.14.
- The `layout_filter_no_region_byte_identical_to_old` unit test is the canary: the no-region path's filter string must be byte-identical to the pre-v1.3.16 `layout_filter` output for all three layouts.
- If a user ever ran the v1.3.16 binary, their DB has dead `cam_asset_path` / `cam_asset_source` columns sitting at NULL. They're never read or written by this design. Not removed (SQLite `DROP COLUMN` is fragile pre-3.35; risk > benefit for two unused TEXT columns).
- Invalid JSON in `cam_region_norm` / `cam_region_norm_override` → `log::warn!` and fall back to dup-source. Never blocks export.
- Region coord values clamped to `[0.0, 1.0]` at save time (UI side) AND at filter-build time (defense in depth). `w` and `h` must each be ≥ 0.05 (5% minimum); enforced UI-side.

## 4. File-level changes (logical map; the implementation plan pins exact lines)

- `src-tauri/src/db.rs` — three `ALTER TABLE` migrations; settings k/v default; `VodRow` / `ClipRow` field additions; re-introduction of `VOD_SELECT` / `CLIP_SELECT` constants for defensive query-builder consolidation.
- `src-tauri/src/commands/cam_region.rs` *(new module)* — `CamRegion` parser + clamper, `CamFitMode` enum, region-resolver helper, Tauri commands `set_vod_cam_region` / `clear_vod_cam_region` / `set_clip_cam_region_override` / `clear_clip_cam_region_override` / `set_clip_fit_mode`.
- `src-tauri/src/commands/mod.rs` — `pub mod cam_region;`.
- `src-tauri/src/lib.rs` — handler registrations for the new commands.
- `src-tauri/src/vertical_crop.rs` — new `layout_filter_with_region` (PiP+region, Split+region branches); `ExportRequest` gains `effective_region: Option<CamRegion>` and `fit_mode: CamFitMode`; existing `layout_filter` byte-unchanged for the no-region path.
- `src-tauri/src/commands/export.rs` — `clip_to_export_request` resolves the effective region using the override toggle + clip override + VOD region precedence, populates the new `ExportRequest` fields.
- `src/types.ts` — `Vod.cam_region_norm`, `Clip.cam_region_norm_override`, `Clip.cam_fit_mode`.
- `src/components/CamRegionSetter.tsx` *(new)* — draggable rectangle overlay on the source player, mode entry/exit, Save/Cancel.
- `src/components/CamRegionRow.tsx` *(new)* — the Cam region row + Fit mode dropdown + (conditional) per-clip override sub-row.
- `src/pages/Editor.tsx` — embeds `CamRegionRow` in the Layout section; wires the `CamRegionSetter` overlay onto the source player when in edit mode.
- `src/pages/Settings.tsx` — new toggle row.
- `docs/superpowers/specs/2026-05-17-per-vod-cam-asset-design.md` — add superseded banner at top.
- `docs/superpowers/plans/2026-05-17-per-vod-cam-asset.md` — add superseded banner at top.

## 5. Revert strategy

`git reset --hard 6927297` — reverts the 11 v1.3.16 implementation commits in one step, keeping the two design+plan docs in the tree as historical artifacts (with the superseded banner added). None of the 11 commits have been pushed to `origin/main` (current state: 14 commits ahead, 0 pushed). Reflog preserves them for 90 days if ever needed.

The defensive `VOD_SELECT` constant refactor that landed in v1.3.16 (commit `3d492f7`) is re-introduced in this design under the new feature work — it's good hygiene independent of v1.3.16's purpose.

## 6. Testing

### Unit tests (Rust, run in CI via `cargo test`)

- `cam_region::parse_norm_json` — valid JSON round-trips; invalid JSON → `None`; out-of-range values clamp to `[0,1]`; `w` or `h` < 0.05 → `None` (rejects).
- `cam_region::to_crop_expr` — `{x:0.12,y:0.78,w:0.22,h:0.22}` → `"iw*0.22:ih*0.22:iw*0.12:ih*0.78"`.
- `region_resolver::uses_vod_when_no_override` — clip override NULL, returns VOD region.
- `region_resolver::uses_override_when_setting_on_and_override_set` — both populated, toggle on → override wins.
- `region_resolver::ignores_override_when_setting_off_even_if_saved` — override populated, toggle off → VOD region.
- `layout_filter::no_region_byte_identical_to_old` — regression canary for all three layouts.
- `layout_filter::pip_with_region_uses_split2_and_no_slot_rect` — verifies `split=2` and no opaque slot rectangle in filter string.
- `layout_filter::split_with_region_uses_split3_with_boxblur` — verifies `split=3`, `boxblur=20:5` on path b, sharp overlay path c.
- `layout_filter::fit_mode_scale_expressions` — Fit/Fill/Stretch produce the documented substrings.
- `layout_filter::caption_filter_still_appends_to_out` — caption wiring preserved.

### Live smoke (Slug-side, after `cargo check` is green)

1. GameplayFocus regression — clip with no cam slot exports byte-identical.
2. PiP without region set — dup-source (v1.3.14 behavior preserved).
3. PiP with region set — avatar floats at slot position; gameplay shows in slot bars.
4. Split without region set — dup-source.
5. Split with region + Fit — blur background + sharp avatar centered.
6. Split with region + Fill — cropped to fill the bottom slot; no bars, no blur visible.
7. Split with region + Stretch — distorted (cosmetic only).
8. Settings toggle ON → editor shows the per-clip override row; saved override wins over VOD region.
9. Settings toggle OFF after saving an override → override hidden in UI, export uses VOD region; toggle back ON restores the saved override.
10. Set + Clear via UI; verify `cam_region_norm` persists across app restart.

## 7. Watchouts

- **Byte-identical no-region path:** the new `layout_filter_with_region` must short-circuit to the existing `layout_filter` when `effective_region.is_none()`. This is the regression-safety guarantee.
- **5% × 5% minimum region size** enforced UI-side. Smaller values rejected on save with an inline error.
- **Clamping at save AND build time** — defense in depth against partially-edited DB rows or future schema bugs.
- **`boxblur=20:5` is a starting value.** May tune to `gblur=sigma=12` if box blur looks too blocky in real footage. Single-line filter change if it needs tweaking.
- **PiP overlay centering:** cam is centered within the slot box (using `overlay=SLOT_X+(SLOT_W-w)/2:SLOT_Y+(SLOT_H-h)/2`). If user feedback prefers anchor-to-slot-top-left, that's a one-line change in the filter builder.
- **Settings toggle off ≠ override deleted** — DB values preserved; UI just hides them. Re-enabling the toggle restores the user's previous overrides.
- **The two dead v1.3.16 columns** (`cam_asset_path`, `cam_asset_source`) sit at NULL forever. Documented as known-dead in this spec.
- **`bump-version.ps1` + commit discipline** per CLAUDE.md applies at ship time. Version number (v1.3.16 vs v1.4.0) is Slug's call on the ship day.

## 8. Open questions

None — all resolved in brainstorming:

- v1.3.16 fate: replace (clean revert)
- Region scope: per-VOD by default, opt-in per-clip override via Settings toggle
- Selection UX: inline draggable rectangle on the source player
- Composition model: 3 knobs (source region, slot position+size, fit mode)
- Bar fill: PiP = gameplay pass-through; Split = blurred-source extension
- Coord system: normalized 0..1 source-frame coords (resolution-independent)
- Storage: single JSON TEXT column per region (not 4 separate float columns)
- Backward compat: NULL columns = pre-v1.3.16 dup-source behavior
- Per-clip override toggle: lives in Settings page, off by default, preserves DB state when toggled off
