# Review UI Hidden Gesture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Hide the "Show Review Tools" Settings toggle and Review UI behind a 7-tap-on-version-number unlock, so testers don't see the dev tool by default after v1.3.12 publicized it.

**Architecture:** Two-flag gating in the existing Zustand `UiSettings` store. New `developerModeUnlocked` boolean (default false) gates whether the Developer Tools `<section>` renders in Settings AND whether the Review UI renders in Vods/Clips pages. Existing `showReviewTools` boolean stays unchanged. Tap counter logic is a pure function on (count, lastTap, now) for future testability.

**Tech Stack:** React 19 + TypeScript + Zustand store (UI settings persist to SQLite via existing `get_setting`/`save_setting` Tauri commands as JSON).

---

## Spec reference

This plan implements `docs/superpowers/specs/2026-05-07-review-ui-hidden-gesture-design.md`.

### Deviation from spec

The spec §5 calls for unit tests on the tap-counter logic. The project has **no frontend test framework** installed (`package.json` devDependencies has eslint but no vitest/jest). Adding Vitest is out of scope for this quiet patch — the tap-counter logic is small enough that thorough manual smoke testing in Task 6 substitutes. Adding Vitest is filed as a follow-up backlog item.

The function is structured so a unit test can be added later without refactoring (it's a pure function on a state object and a timestamp).

## Pre-flight: confirm these symbols exist

Before starting, confirm these resolve in the current code (line numbers may shift slightly):

- `interface UiSettings { ... showReviewTools: boolean }` at `src/stores/uiStore.ts:8-19`
- `export const UI_DEFAULTS: UiSettings = { ... showReviewTools: false }` at `src/stores/uiStore.ts:21-27`
- `<span className="text-slate-400">{appVersion}</span>` in `src/pages/Settings.tsx:974` (the version display, the tap target)
- `<section className="v4-section">` containing `🔧 Developer Tools` and the `showReviewTools` toggle at `src/pages/Settings.tsx:941-960`
- `const showReviewTools = useUiStore((s) => s.settings.showReviewTools)` at `src/pages/Vods.tsx:100` and `src/pages/Clips.tsx:142`
- Vods.tsx use site at `src/pages/Vods.tsx:655`
- Clips.tsx use sites at `src/pages/Clips.tsx:206` and `src/pages/Clips.tsx:251`

If any line numbers have shifted (other commits in flight), locate them by symbol search.

## File structure

**Frontend (TypeScript only — NO Rust changes):**

- `src/stores/uiStore.ts` — Add `developerModeUnlocked: boolean` to the `UiSettings` interface and `UI_DEFAULTS`. Add and export the pure `tryAdvanceTapCounter` helper.
- `src/pages/Settings.tsx` — Add `useRef` for tap counter. Make the version `<span>` clickable. Wrap the existing "Developer Tools" `<section>` (lines 941-960) in `{settings.developerModeUnlocked && (...)}`.
- `src/pages/Vods.tsx` — Change the `showReviewTools` selector to read both flags and AND them.
- `src/pages/Clips.tsx` — Same change as Vods.tsx.

**Files NOT changed:**

- All Rust files. The `UiSettings` JSON is stored via the generic `save_setting`/`get_setting` Tauri commands; the backend doesn't know about the schema.
- The Review UI components themselves (rate buttons, note input, export button). Their visibility is gated by the existing `showReviewTools` check; we just upstream that check.

---

## Task 1: Add `developerModeUnlocked` to UiSettings

**Goal:** Add the new boolean field to the store interface and defaults. Existing user data automatically gets `false` via the `{ ...UI_DEFAULTS, ...parsed }` merge in `load()`.

**Files:**
- Modify: `src/stores/uiStore.ts` — interface at lines 8-19, defaults at lines 21-27.

- [ ] **Step 1.1: Add field to interface**

In `src/stores/uiStore.ts`, locate the `UiSettings` interface:

```typescript
export interface UiSettings {
  /** Show hover tooltips throughout the app. Default: true. */
  showTooltips: boolean
  /** Color theme. Default: 'dark'. */
  theme: ThemeMode
  /** Auto-publish clips that score >= 0.9 confidence without review. Default: false. */
  autoShipHighConfidence: boolean
  /** Use GPU (CUDA) for transcription. Default: true (falls back to CPU if unavailable). */
  useGpu: boolean
  /** Show dev-only clip review tools (rating buttons + note + Export). Default: false. */
  showReviewTools: boolean
}
```

Add a new field at the end (before the closing `}`):

```typescript
  /**
   * Internal: true once the user has unlocked developer mode via the
   * 7-tap-on-version-number gesture in Settings. When false, all
   * dev-only toggles (currently `showReviewTools`) and the Developer
   * Tools section in Settings are hidden, and the Review UI in
   * Vods/Clips pages does not render even if `showReviewTools` is
   * true. Reset to false on Settings → Reset. Persists across
   * launches like other UI settings.
   */
  developerModeUnlocked: boolean
```

- [ ] **Step 1.2: Add field to defaults**

Locate `UI_DEFAULTS`:

```typescript
export const UI_DEFAULTS: UiSettings = {
  showTooltips: true,
  theme: 'dark',
  autoShipHighConfidence: false,
  useGpu: true,
  showReviewTools: false,
}
```

Add the new default at the end (before the closing `}`):

```typescript
export const UI_DEFAULTS: UiSettings = {
  showTooltips: true,
  theme: 'dark',
  autoShipHighConfidence: false,
  useGpu: true,
  showReviewTools: false,
  developerModeUnlocked: false,
}
```

- [ ] **Step 1.3: Verify TypeScript compiles**

Run from the project root:

```
npm run build
```

Expected: build succeeds. If TypeScript reports errors about missing `developerModeUnlocked` in object literals elsewhere, that means another file constructs a full `UiSettings` and needs updating. (No such other constructors are expected — only `UI_DEFAULTS` should be a full literal — but verify.)

- [ ] **Step 1.4: Commit**

```powershell
git add src/stores/uiStore.ts
git commit -m "feat(ui): add developerModeUnlocked field to UiSettings"
```

---

## Task 2: Add `tryAdvanceTapCounter` pure helper

**Goal:** Write the pure function that decides whether a tap should increment the counter, reset it, or fire the unlock. Pure means: takes input state + `now`, returns next state + outcome. No side effects, no DOM, no React.

**Files:**
- Modify: `src/stores/uiStore.ts` — add the helper and export it. Place it after `UI_DEFAULTS` and before `applyThemeToDOM` so consumers can import it from the same module.

- [ ] **Step 2.1: Add the helper function**

In `src/stores/uiStore.ts`, after the `UI_DEFAULTS` constant and the `SETTINGS_KEY` constant, add:

```typescript
/**
 * Pure logic for the version-number tap-counter that unlocks Developer mode.
 *
 * Caller maintains the state object and calls this on every tap, passing
 * the current timestamp. Function returns the next state and whether
 * unlock should fire.
 *
 * Rules:
 * - 7 taps within 2 seconds of each other (each gap < 2000 ms) → unlock.
 * - A tap > 2 seconds after the previous tap resets the counter to 1 (the
 *   new tap becomes the start of a fresh sequence).
 * - If alreadyUnlocked is true, this is a no-op — counter stays at 0,
 *   shouldUnlock is false. Prevents re-fire and toast spam.
 *
 * Caller is expected to:
 *   1. Initialize state to { count: 0, lastTap: 0 }.
 *   2. On every click event, call tryAdvanceTapCounter(state, Date.now(), alreadyUnlocked).
 *   3. Replace state with `next`.
 *   4. If `shouldUnlock` is true, dispatch the unlock side effect.
 */
export interface TapCounterState {
  count: number
  lastTap: number
}

export interface TapCounterResult {
  next: TapCounterState
  shouldUnlock: boolean
}

export const TAP_COUNTER_WINDOW_MS = 2000
export const TAP_COUNTER_TARGET = 7

export function tryAdvanceTapCounter(
  state: TapCounterState,
  now: number,
  alreadyUnlocked: boolean,
): TapCounterResult {
  if (alreadyUnlocked) {
    return { next: { count: 0, lastTap: 0 }, shouldUnlock: false }
  }
  const gap = now - state.lastTap
  const nextCount =
    state.count === 0 || gap > TAP_COUNTER_WINDOW_MS ? 1 : state.count + 1
  const next: TapCounterState = { count: nextCount, lastTap: now }
  const shouldUnlock = nextCount >= TAP_COUNTER_TARGET
  return { next, shouldUnlock }
}
```

- [ ] **Step 2.2: Verify TypeScript compiles**

Run:

```
npm run build
```

Expected: build succeeds (no type errors).

- [ ] **Step 2.3: Commit**

```powershell
git add src/stores/uiStore.ts
git commit -m "feat(ui): tryAdvanceTapCounter helper for version-tap unlock"
```

---

## Task 3: Wire tap counter into Settings.tsx version element

**Goal:** Make the version number `<span>` clickable. On click, advance the tap counter; on unlock, dispatch `update({ developerModeUnlocked: true })`. No toast (no system available — the toggle appearing is the feedback).

**Files:**
- Modify: `src/pages/Settings.tsx` — import the helper, add `useRef` for tap-counter state, replace the version `<span>` with a clickable variant.

- [ ] **Step 3.1: Add import for tap helper**

At the top of `src/pages/Settings.tsx`, the existing import for `useUiStore` is at line 9:

```typescript
import { useUiStore } from '../stores/uiStore'
```

Replace it with:

```typescript
import { useUiStore, tryAdvanceTapCounter, type TapCounterState } from '../stores/uiStore'
```

- [ ] **Step 3.2: Add useRef import if missing**

Look at the existing React import at line 1:

```typescript
import { useEffect, useState } from 'react'
```

If `useRef` is not in the list (it isn't currently), update to:

```typescript
import { useEffect, useState, useRef } from 'react'
```

- [ ] **Step 3.3: Add tap counter ref inside the Settings component**

Find the top of the main `Settings` component function. Look for where existing hooks like `useState` and `useUiStore` are called. The existing code has `const ui = useUiStore()` (or similar — find the actual line by searching for `useUiStore(` inside the main component body, NOT inside `TemplateManager`).

Immediately after the existing `useUiStore()` call inside the main `Settings` component, add:

```typescript
  const tapStateRef = useRef<TapCounterState>({ count: 0, lastTap: 0 })

  const handleVersionTap = () => {
    const result = tryAdvanceTapCounter(
      tapStateRef.current,
      Date.now(),
      ui.settings.developerModeUnlocked,
    )
    tapStateRef.current = result.next
    if (result.shouldUnlock) {
      ui.update({ developerModeUnlocked: true })
    }
  }
```

If the variable name from `useUiStore()` is not `ui`, adjust accordingly (e.g., if it's `uiStore`, use `uiStore.settings.developerModeUnlocked` and `uiStore.update(...)`). Use whatever the existing code uses — verify by reading the file before this edit.

- [ ] **Step 3.4: Make the version span clickable**

Find the existing version display at `src/pages/Settings.tsx:973-975`:

```typescript
          <div className="flex gap-2">
            <span className="text-slate-300">Version:</span>
            <span className="text-slate-400">{appVersion}</span>
          </div>
```

Replace the second `<span>` (the one displaying `{appVersion}`) with a clickable variant:

```typescript
          <div className="flex gap-2">
            <span className="text-slate-300">Version:</span>
            <span
              className="text-slate-400 cursor-default select-none"
              onClick={handleVersionTap}
            >
              {appVersion}
            </span>
          </div>
```

Notes on the styling:
- `cursor-default` keeps the cursor as the normal pointer, NOT a hand-pointer (`cursor-pointer`). The tap target should not advertise itself as clickable — that defeats the discoverability goal.
- `select-none` prevents text selection on rapid click — without it, 7 fast clicks would highlight the version text.
- No visual hover effect, no `:hover:` styles. The element should be visually identical to before.

- [ ] **Step 3.5: Manual sanity check (no automated tests yet)**

Run the app:

```powershell
cargo tauri dev
```

In the running app: navigate to Settings, scroll to the About section, click the version text 7 times rapidly. After the 7th click, the "🔧 Developer Tools" section should appear (this requires Task 4 to be complete — if Task 4 isn't done yet, instead inspect the React DevTools or the SQLite DB to verify `developerModeUnlocked` flipped to `true`).

If you don't have React DevTools handy, simply verify nothing crashes, then proceed. Full verification happens in Task 6's smoke test.

- [ ] **Step 3.6: Commit**

```powershell
git add src/pages/Settings.tsx
git commit -m "feat(ui): version-tap gesture wired in Settings"
```

---

## Task 4: Conditional render of Developer Tools section in Settings.tsx

**Goal:** Hide the entire "🔧 Developer Tools" `<section>` (which contains the `showReviewTools` toggle) unless `developerModeUnlocked` is true.

**Files:**
- Modify: `src/pages/Settings.tsx` — wrap the existing `<section>` at lines 941-960 in `{ui.settings.developerModeUnlocked && (...)}`.

- [ ] **Step 4.1: Wrap the section**

Find the existing block at `src/pages/Settings.tsx:941-960`:

```typescript
      {/* Developer tools — hidden behind a toggle for clip-quality investigation */}
      <section className="v4-section">
        <h3 className="v4-section-label">🔧 Developer Tools</h3>
        <div className="v4-setting-row">
          <div className="v4-setting-info">
            <div className="v4-setting-name">Show Clip Review Tools</div>
            <div className="v4-setting-desc">
              Adds rating buttons and notes to each clip card, plus an "Export review data" button on the Vods page.
              Used to gather feedback for tuning the clip scoring model. Off by default. No effect on normal clip generation.
            </div>
          </div>
          <button
            type="button"
            onClick={() => ui.update({ showReviewTools: !ui.settings.showReviewTools })}
            className={`v4-toggle ${ui.settings.showReviewTools ? 'on' : ''}`}
            aria-label="Toggle clip review tools"
            aria-pressed={ui.settings.showReviewTools}
          />
        </div>
      </section>
```

Wrap it in a conditional. Update the comment to reflect the new gating reason:

```typescript
      {/* Developer tools — hidden behind 7-tap-on-version unlock (see uiStore.ts tryAdvanceTapCounter). */}
      {ui.settings.developerModeUnlocked && (
        <section className="v4-section">
          <h3 className="v4-section-label">🔧 Developer Tools</h3>
          <div className="v4-setting-row">
            <div className="v4-setting-info">
              <div className="v4-setting-name">Show Clip Review Tools</div>
              <div className="v4-setting-desc">
                Adds rating buttons and notes to each clip card, plus an "Export review data" button on the Vods page.
                Used to gather feedback for tuning the clip scoring model. Off by default. No effect on normal clip generation.
              </div>
            </div>
            <button
              type="button"
              onClick={() => ui.update({ showReviewTools: !ui.settings.showReviewTools })}
              className={`v4-toggle ${ui.settings.showReviewTools ? 'on' : ''}`}
              aria-label="Toggle clip review tools"
              aria-pressed={ui.settings.showReviewTools}
            />
          </div>
        </section>
      )}
```

If the variable is not `ui` in this file, adjust to whatever the existing code uses (Task 3 confirmed the actual name).

- [ ] **Step 4.2: Verify TypeScript compiles**

```
npm run build
```

Expected: build succeeds.

- [ ] **Step 4.3: Commit**

```powershell
git add src/pages/Settings.tsx
git commit -m "feat(ui): hide Developer Tools section unless unlocked"
```

---

## Task 5: Update Vods.tsx and Clips.tsx to gate Review UI on both flags

**Goal:** The Review UI must NOT render even if `showReviewTools = true` is leftover in storage from v1.3.12. Gate on both flags.

**Files:**
- Modify: `src/pages/Vods.tsx:100` — selector
- Modify: `src/pages/Clips.tsx:142` — selector

The selectors are the only sites that need to change. The use sites (Vods.tsx:655, Clips.tsx:206, Clips.tsx:251) keep referencing the same local variable.

- [ ] **Step 5.1: Update Vods.tsx selector**

Find `src/pages/Vods.tsx:100`:

```typescript
  const showReviewTools = useUiStore((s) => s.settings.showReviewTools)
```

Replace with:

```typescript
  // Phase A v1.3.13: review tools require BOTH the developer-mode unlock AND
  // the explicit showReviewTools toggle. Either being false hides the UI.
  const showReviewTools = useUiStore(
    (s) => s.settings.developerModeUnlocked && s.settings.showReviewTools,
  )
```

- [ ] **Step 5.2: Update Clips.tsx selector**

Find `src/pages/Clips.tsx:142`:

```typescript
  const showReviewTools = useUiStore((s) => s.settings.showReviewTools)
```

Replace with the same shape:

```typescript
  // Phase A v1.3.13: review tools require BOTH the developer-mode unlock AND
  // the explicit showReviewTools toggle. Either being false hides the UI.
  const showReviewTools = useUiStore(
    (s) => s.settings.developerModeUnlocked && s.settings.showReviewTools,
  )
```

- [ ] **Step 5.3: Verify TypeScript compiles**

```
npm run build
```

Expected: build succeeds.

- [ ] **Step 5.4: Commit**

```powershell
git add src/pages/Vods.tsx src/pages/Clips.tsx
git commit -m "feat(ui): gate Review UI on developerModeUnlocked + showReviewTools"
```

---

## Task 6: Manual smoke test + version bump v1.3.13 + ship

**Goal:** Validate the unlock end-to-end on the live app, then bump version, tag, push.

**Files:** None modified for tasks 6.1-6.5. Version bump touches package.json, src-tauri/Cargo.toml, src-tauri/Cargo.lock, src-tauri/tauri.conf.json.

- [ ] **Step 6.1: Build verification**

Run from project root:

```
npm run build
```

Expected: TypeScript build succeeds, no errors.

Then Rust check (no Rust changes were made, but verify nothing broke):

```powershell
cd "C:\Users\cereb\Desktop\Claude projects\clipviral\src-tauri"
cargo check
```

Expected: `Finished`, same 206 pre-existing warnings as v1.3.12, 0 errors.

- [ ] **Step 6.2: Live smoke test — fresh-state path**

Test the path a fresh user (or a v1.3.12 user with `showReviewTools = false`) takes:

```powershell
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
cargo tauri dev
```

In the running app:

1. Settings → confirm there is **no** "🔧 Developer Tools" section visible.
2. Vods page → confirm there is **no** "Export review data" button on any VOD card.
3. Clips page (open any analyzed VOD) → confirm there are **no** rating buttons or note inputs on any clip card.

If any of those three are visible, something is wrong. Check the DB for `ui_settings` JSON content; `developerModeUnlocked` should be missing (giving false default) or false.

- [ ] **Step 6.3: Live smoke test — unlock path**

Still in the running app:

4. Settings → scroll to About section → tap the version text 7 times rapidly (within ~2 seconds).
5. Confirm: "🔧 Developer Tools" section appears at the previous location (above Clip Templates / above About).
6. Confirm: the "Show Clip Review Tools" toggle reflects whatever your stored value was. Most likely off if you haven't been on v1.3.12 production; could be on if you were testing.
7. Toggle "Show Clip Review Tools" ON.
8. Navigate to Vods page → confirm "Export review data" button now appears on completed VODs.
9. Navigate to Clips page (any analyzed VOD) → confirm rating buttons and note inputs appear.

- [ ] **Step 6.4: Live smoke test — persistence path**

10. Close the app entirely (not just minimize).
11. Reopen via `cargo tauri dev` (or restart from the system tray / re-run the dev command).
12. Settings → confirm the "🔧 Developer Tools" section is still visible (unlock persisted).
13. Confirm the "Show Clip Review Tools" toggle is still ON (its value persisted independently).
14. Confirm the Review UI is still rendering on Vods/Clips pages.

- [ ] **Step 6.5: Live smoke test — gap-too-big path**

Tests the 2-second window. Easiest done with React DevTools or by clearing dev mode first via SQLite:

15. Manually flip `developerModeUnlocked` back to `false` in the DB. Run:

    ```powershell
    sqlite3 "$env:APPDATA\clipviral\clipviral.db" "UPDATE settings SET value = REPLACE(value, '""developerModeUnlocked"":true', '""developerModeUnlocked"":false') WHERE key = 'ui_settings';"
    ```

    Restart the app. Confirm Developer Tools section is hidden again.

16. Tap version 3 times quickly, then wait 5 seconds, then tap 4 times. Total taps = 7 but split across two windows. Confirm the section does **NOT** appear (counter reset after the 5-second gap, the second sequence only got to 4 of 7).

17. Tap version 7 times rapidly to re-unlock. Confirm section appears.

If the gap-too-big test fails (section appears after 3+5+4 taps), the counter logic has a bug. Re-verify Task 2's `tryAdvanceTapCounter` math.

- [ ] **Step 6.6: Bump version to v1.3.13**

```powershell
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
powershell -File bump-version.ps1 1.3.13
```

Expected output: confirmation that package.json, src-tauri/Cargo.toml, src-tauri/tauri.conf.json all updated to 1.3.13.

- [ ] **Step 6.7: Commit version bump**

```powershell
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
git add package.json src-tauri/Cargo.lock src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -m "chore: bump to v1.3.13 (review-UI hidden gesture)"
```

- [ ] **Step 6.8: Tag and push**

Use a single-line tag message (this is a quiet patch — no announcement, no detailed release notes):

```powershell
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
git tag -a v1.3.13 -m "v1.3.13 -- internal: review tools hidden behind unlock gesture"
git push origin main
git push origin v1.3.13
```

GitHub Actions will produce a draft release. **Do NOT publish the GitHub release with notes** — leave it as a quiet build. The auto-updater will still pick it up.

If GitHub forces a non-empty release notes body when publishing, use a single line: "Internal patch — no user-facing changes." Don't reference the gesture or the Review UI.

---

## Self-review

(Run by the plan author with fresh eyes. Findings written here for transparency.)

### Spec coverage

Walking each spec section:

- **§1 Background** — context only, no implementation. ✓
- **§2 Goals** — Tasks 1, 4, 5 collectively achieve the "fresh user sees no Review UI / no toggle" success criterion. Task 3 achieves the "Slug can unlock in <10 seconds" criterion. Task 6 manual tests the persistence criterion. ✓
- **§2 Out of scope** — No tasks implement re-lock UI, progressive feedback, onboarding, or removing the Review UI from production. ✓
- **§3.1 New UI setting** — Task 1. ✓
- **§3.2 Two-flag gating model** — Task 5. ✓
- **§3.3 The 7-tap gesture** — Tasks 2 (logic) + 3 (wiring). ✓
- **§3.4 Settings.tsx layout changes** — Task 4. The spec said "add a Developer section header" but the section already exists in the codebase; the plan correctly just wraps it in a conditional rather than recreating it. Improvement on the spec, no functional gap. ✓
- **§3.5 Vods.tsx and Clips.tsx gating** — Task 5. ✓
- **§4 Migration** — Task 1's `UI_DEFAULTS` merge handles existing users automatically. Verified manually in Task 6.4 (persistence path). ✓
- **§5 Testing** — Spec called for unit tests; plan flags the deviation (no test framework installed) and substitutes Task 6's manual smoke test. The pure function structure preserves the option to add tests later without refactoring. ✓ (with documented deviation)
- **§6 File-level changes** — All four files in the spec are touched. No Rust changes, as spec stated. ✓
- **§7 Versioning** — Task 6.6-6.8. ✓
- **§8 Watchouts** — Task 3.3's note about reading `ref.current` inside the handler addresses the closure stale-state warning. Task 3 correctly omits the toast (spec acknowledged this is acceptable). The two-flag confusion is mitigated by the doc-comment on `developerModeUnlocked` added in Task 1. Gesture discoverability is not tested (intentional per spec). ✓

No spec gaps.

### Placeholder scan

- "TBD" / "TODO" / "implement later": none.
- "Add appropriate error handling": none. (No error paths to handle here — the helper is pure, the click handler can't fail.)
- "Write tests for the above" without code: only Task 2 mentions tests, and the plan explicitly says "no automated tests for now — manual verification" in the spec deviation note.
- "Similar to Task N": Task 5.2 says "Replace with the same shape" but then provides the full code block. ✓
- Unspecified types or methods: `tryAdvanceTapCounter`, `TapCounterState`, `TapCounterResult`, `TAP_COUNTER_WINDOW_MS`, `TAP_COUNTER_TARGET` are all defined in Task 2 and consumed in Task 3. ✓

### Type consistency

- `tryAdvanceTapCounter(state: TapCounterState, now: number, alreadyUnlocked: boolean) → TapCounterResult` — definition in Task 2, signature reused in Task 3.3.
- `TapCounterState` interface in Task 2 has fields `count: number, lastTap: number`. Task 3.3's `useRef<TapCounterState>({ count: 0, lastTap: 0 })` matches.
- Task 3.3's import statement for `tryAdvanceTapCounter, TapCounterState` matches the export shape from Task 2.
- `developerModeUnlocked` field added in Task 1 and consumed in Tasks 3.3 (predicate), 4.1 (conditional render), 5.1 + 5.2 (selectors). All references match.
- `ui.update({ developerModeUnlocked: true })` in Task 3.3 matches the `update(patch: Partial<UiSettings>)` signature in the existing store.

No type drift detected.
