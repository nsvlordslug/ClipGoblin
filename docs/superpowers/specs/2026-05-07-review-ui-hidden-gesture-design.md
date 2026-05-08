# Review UI Hidden Gesture — Design

**Status:** Approved for implementation
**Date:** 2026-05-07
**Target release:** v1.3.13 (no announcement — quiet patch)

---

## 1. Background

v1.3.12 shipped with the Phase B Review UI gated behind a Settings toggle (`showReviewTools` in `UiSettings`). The toggle is off by default but visible to every user opening Settings → labeled "Show Review Tools." After the v1.3.12 release notes referenced the toggle, anyone reading the notes can flip it on.

The Review UI is a developer tool for ground-truth rating clips and exporting their scoring data as JSON. It's not user-facing functionality — testers who turn it on get a confusing rate-clips workflow with no clear purpose.

The fix: hide the toggle behind a 7-tap-on-version unlock so the only people who see it are people who already know about it.

## 2. Goals

- After v1.3.13 update, a fresh user opening Settings does not see any "Show Review Tools" toggle and does not see the Review UI in Vods/Clips pages.
- Slug can still access the Review UI in under 10 seconds: open Settings → tap version number 7 times → toggle appears.
- No release notes, no announcement, no in-app onboarding for the gesture.
- Users who had `showReviewTools = true` in v1.3.12 see the Review UI hidden after update (until they discover the gesture).
- Once unlocked, the unlock persists across app launches.

### Out of scope

- Re-lock UI / button (YAGNI — Settings → Reset already wipes UI settings; that's enough)
- Progressive feedback ("3 more taps to unlock")
- Onboarding tooltip pointing to the gesture
- Removing the Review UI from production builds entirely (already decided we ship it for Slug's use)

## 3. Design

### 3.1 New UI setting

Add one boolean to the `UiSettings` interface in `src/stores/uiStore.ts`:

```typescript
export interface UiSettings {
  // ...existing fields
  /** Internal: unlocked via 7-tap-on-version gesture. Gates visibility of dev-only toggles. Default: false. */
  developerModeUnlocked: boolean
}

export const UI_DEFAULTS: UiSettings = {
  // ...existing fields
  developerModeUnlocked: false,
}
```

The existing `showReviewTools` field stays untouched — its semantics don't change. The new field gates whether the toggle and any review-UI-dependent UI render at all.

**Persistence:** Reuses the existing `ui_settings` JSON-in-DB pipeline (`get_setting` / `save_setting` Tauri commands). No Rust changes. The `load()` merge `{ ...UI_DEFAULTS, ...parsed }` automatically gives every existing user `developerModeUnlocked: false` because their stored JSON doesn't have the field.

### 3.2 Two-flag gating model

After v1.3.13, the Review UI is rendered if and only if:

```
developerModeUnlocked === true  &&  showReviewTools === true
```

Both flags must be true. This means:

- Existing v1.3.12 user with `showReviewTools = true` → after v1.3.13 update, `developerModeUnlocked = false` (new field default), so Review UI is hidden. Their `showReviewTools = true` value is preserved. After 7-tap gesture sets `developerModeUnlocked = true`, the Review UI re-appears immediately (the toggle is already on from before) — no need to re-enable.
- Fresh user → both default to false. Toggle is hidden, Review UI is hidden. 7-tap to reveal toggle, then flip the toggle to enable the actual UI.

The Settings toggle for `showReviewTools` is rendered only when `developerModeUnlocked === true`.

### 3.3 The 7-tap gesture

Trigger element: the version number text in Settings.tsx (currently at line 974, displayed as `<span className="text-slate-400">{appVersion}</span>`).

Gesture rules:

- Click counter resets to 0 on the first click.
- Each subsequent click within 2 seconds of the previous click increments the counter.
- A click more than 2 seconds after the previous resets the counter to 1 (the new tap is the first of a fresh sequence).
- When the counter reaches 7, dispatch `update({ developerModeUnlocked: true })` and show a toast: "Developer mode unlocked."
- After successful unlock, the counter is irrelevant — additional taps are no-ops (the value is already true).

Edge case: if `developerModeUnlocked` is already true when the first tap happens, do nothing. No toast, no counter, no double-fire.

State storage: a single `useRef<{ count: number; lastTap: number }>` in Settings.tsx local state. No state kept in the Zustand store for the counter itself — only the resulting `developerModeUnlocked` flag.

### 3.4 Settings.tsx layout changes

Current structure (approximate):

```
[General settings]
  - showTooltips
  - theme
  - useGpu
  - autoShipHighConfidence
  - showReviewTools  <-- visible to all users in v1.3.12
[Advanced / About]
  - Version: 1.3.12
```

After v1.3.13:

```
[General settings]
  - showTooltips
  - theme
  - useGpu
  - autoShipHighConfidence
[Advanced / About]
  - Version: 1.3.13   <-- now click target for 7-tap

[Developer]   <-- entire section conditional on developerModeUnlocked
  - showReviewTools (existing toggle, unchanged behavior)
```

The "Developer" section header is rendered only when `developerModeUnlocked === true`. The header makes the section's intent explicit so once unlocked, Slug can tell at a glance which toggles are dev-only.

### 3.5 Vods.tsx and Clips.tsx gating

Currently both files check `settings.showReviewTools` directly to decide whether to render Review UI elements (rate buttons, export button, note input). Change these checks to `settings.developerModeUnlocked && settings.showReviewTools`.

Both files import `useUiStore` already — only the predicate changes.

## 4. Migration / existing users

No explicit migration needed. The `load()` merge gives every existing user `developerModeUnlocked: false` automatically when v1.3.13 first reads their stored `ui_settings` JSON. The Review UI is hidden for everyone on update; Slug re-unlocks via gesture.

A v1.3.12 user who had `showReviewTools = true` keeps that value in their stored JSON. After v1.3.13 update + 7-tap unlock, the toggle re-appears already-checked and Review UI is restored to its previous state.

## 5. Testing

### Unit (Vitest, frontend)

Pure function for the tap-counter logic — extract `tryAdvanceTapCounter(state, now)` so it can be tested without any DOM:

- 7 sequential calls within 2 seconds each → returns `{ unlocked: true }` on the 7th
- 6 sequential calls → returns `{ unlocked: false }` after each
- 7 calls but with one gap > 2 sec in the middle → counter resets, no unlock
- 7 calls when `alreadyUnlocked === true` → returns `{ unlocked: true, noOp: true }`

### Manual smoke test

- Fresh install (or after Settings → Reset): toggle is hidden, Review UI is hidden in Vods/Clips
- Tap version 7 times within 2 sec: toast appears, "Developer" section appears with toggle off
- Toggle on: Review UI appears in Vods/Clips
- Restart app: developer section still visible, toggle still on, Review UI still rendering
- Settings → Reset: returns to fresh state (toggle hidden, Review UI hidden)

## 6. File-level changes

- **Modify:** `src/stores/uiStore.ts` — add `developerModeUnlocked: boolean` to the `UiSettings` interface and `UI_DEFAULTS`. Document with a doc-comment explaining its purpose.
- **Modify:** `src/pages/Settings.tsx` —
  - Wrap the existing `showReviewTools` toggle (and a new "Developer" section header) in `{settings.developerModeUnlocked && (...)}`.
  - Add `onClick` handler + `useRef` for tap counter on the version number `<span>`.
  - Add toast call on successful unlock (use whatever toast system is already wired up — likely the same one used for "Settings saved" or similar).
- **Modify:** `src/pages/Vods.tsx` — change `settings.showReviewTools` checks to `settings.developerModeUnlocked && settings.showReviewTools` (only the predicate changes; the gated rendering blocks stay the same).
- **Modify:** `src/pages/Clips.tsx` — same change as Vods.tsx.
- **Create:** Unit tests for `tryAdvanceTapCounter` in a new test file (`src/stores/__tests__/uiStore.test.ts` or similar — match the project's existing frontend test convention; if no convention exists, add a new file alongside the store).
- **No Rust changes.**

## 7. Versioning

v1.3.13. Version bump via `bump-version.ps1`. Standard tag + push flow per CLAUDE.md rule #6.

## 8. Watchouts

- **Tap counter race:** if React re-renders the Settings page between taps (unlikely but possible), the `useRef` survives but the click handler closure might capture stale state. Use the functional form: read `ref.current` inside the handler at click time, don't capture it in the closure.
- **Toast availability:** confirm during implementation that there's an existing toast/notification mechanism. If not, the unlock can be silent (no toast) for v1.3.13 and a toast added later — the user-facing observable is the toggle appearing in Settings, which is sufficient feedback on its own.
- **Two-flag confusion:** future readers might wonder why both flags exist. The doc-comments on both fields should make the boundary clear: `developerModeUnlocked` is "do you have access to dev settings at all," `showReviewTools` is "do you want this specific dev tool turned on."
- **Gesture discoverability:** intentional. We do NOT want users discovering this. If anyone reports finding it, that's information, not a bug.

## 9. Open questions

None. All design decisions resolved during the brainstorming Q&A.
