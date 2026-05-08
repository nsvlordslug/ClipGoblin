import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

// ── Types ──

export type ThemeMode = 'dark' | 'light'

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
}

export const UI_DEFAULTS: UiSettings = {
  showTooltips: true,
  theme: 'dark',
  autoShipHighConfidence: false,
  useGpu: true,
  showReviewTools: false,
  developerModeUnlocked: false,
}

const SETTINGS_KEY = 'ui_settings'

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

/** Sync the theme class on <html> so CSS variables switch globally. */
function applyThemeToDOM(theme: ThemeMode) {
  const root = document.documentElement
  root.classList.remove('theme-dark', 'theme-light')
  root.classList.add(`theme-${theme}`)
  root.style.colorScheme = theme
}

// ── Store ──

interface UiStore {
  settings: UiSettings
  loaded: boolean
  load: () => Promise<void>
  save: () => Promise<void>
  update: (patch: Partial<UiSettings>) => void
}

export const useUiStore = create<UiStore>((set, get) => ({
  settings: { ...UI_DEFAULTS },
  loaded: false,

  load: async () => {
    try {
      const raw = await invoke<string | null>('get_setting', { key: SETTINGS_KEY })
      if (raw) {
        const parsed = JSON.parse(raw) as Partial<UiSettings>
        const merged = { ...UI_DEFAULTS, ...parsed }
        applyThemeToDOM(merged.theme)
        set({ settings: merged, loaded: true })
      } else {
        applyThemeToDOM(UI_DEFAULTS.theme)
        set({ loaded: true })
      }
    } catch {
      applyThemeToDOM(UI_DEFAULTS.theme)
      set({ loaded: true })
    }
  },

  save: async () => {
    const json = JSON.stringify(get().settings)
    await invoke('save_setting', { key: SETTINGS_KEY, value: json })
  },

  update: (patch) => {
    const next = { ...get().settings, ...patch }
    if (patch.theme) applyThemeToDOM(patch.theme)
    set({ settings: next })
    // Auto-save on every change for UI preferences
    setTimeout(() => get().save().catch(() => {}), 0)
  },
}))
