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
