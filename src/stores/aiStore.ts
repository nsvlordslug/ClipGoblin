import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

// ── Types ──

export type AiProvider = 'free' | 'openai' | 'claude' | 'gemini'

export interface AiSettings {
  /** Selected provider. Default: 'free'. */
  provider: AiProvider

  /** API keys — stored even when not the active provider. */
  openaiApiKey: string
  openaiModel: string
  claudeApiKey: string
  claudeModel: string
  geminiApiKey: string
  geminiModel: string

  /** Which features use the AI provider (only applies to BYOK).
   *  Clip detection always runs in Free mode. */
  useForTitles: boolean
  useForCaptions: boolean

  /** Fall back to Free mode if BYOK call fails. Default: true. */
  fallbackToFree: boolean
}

export const AI_DEFAULTS: AiSettings = {
  provider: 'free',
  openaiApiKey: '',
  openaiModel: 'gpt-4o-mini',
  claudeApiKey: '',
  claudeModel: 'claude-sonnet-4-6',
  geminiApiKey: '',
  geminiModel: 'gemini-2.5-flash',
  useForTitles: true,
  useForCaptions: true,
  fallbackToFree: true,
}

export const MODEL_OPTIONS: Record<AiProvider, { value: string; label: string }[]> = {
  free: [],
  openai: [
    { value: 'gpt-4o-mini', label: 'GPT-4o Mini (fast, cheap)' },
    { value: 'gpt-4o', label: 'GPT-4o (best quality)' },
    { value: 'gpt-4.1-mini', label: 'GPT-4.1 Mini' },
  ],
  claude: [
    { value: 'claude-sonnet-4-6', label: 'Claude Sonnet 4.6 (recommended)' },
    { value: 'claude-haiku-4-5-20251001', label: 'Claude Haiku 4.5 (fast, cheap)' },
  ],
  gemini: [
    { value: 'gemini-2.5-flash', label: 'Gemini 2.5 Flash (fast)' },
    { value: 'gemini-2.5-pro', label: 'Gemini 2.5 Pro (best quality)' },
  ],
}

export const PROVIDER_META: Record<AiProvider, { name: string; hint: string; keyPlaceholder: string }> = {
  free:   { name: 'Free',   hint: 'No key required',              keyPlaceholder: '' },
  openai: { name: 'OpenAI', hint: 'Best all-around BYOK option',  keyPlaceholder: 'sk-...' },
  claude: { name: 'Claude', hint: 'Great natural writing tone',   keyPlaceholder: 'sk-ant-...' },
  gemini: { name: 'Gemini', hint: 'Good low-cost BYOK option',   keyPlaceholder: 'AIza...' },
}

// ── Serialization ──
// Stored as a single JSON blob under the "ai_settings" key.

const SETTINGS_KEY = 'ai_settings'

// ── Store ──

// Debounce timer for auto-saving key/model changes
let _saveTimer: ReturnType<typeof setTimeout> | null = null
const SAVE_DEBOUNCE_MS = 600 // debounce key typing so we don't save on every keystroke

interface AiStore {
  settings: AiSettings
  loaded: boolean

  /** Load from backend DB. Call once at app start. */
  load: () => Promise<void>

  /** Save current settings to backend DB. */
  save: () => Promise<void>

  /** Update one or more fields. Auto-saves all changes (debounced for typing). */
  update: (patch: Partial<AiSettings>) => void

  /** Get the active API key for the selected provider (empty string if free). */
  activeKey: () => string

  /** Whether the current provider is BYOK (not free). */
  isByok: () => boolean

  /** The actual runtime mode — accounts for missing keys.
   *  e.g. provider=claude but no key → effectiveMode='free' */
  effectiveMode: () => AiProvider

  /** Human-readable status line for the UI. */
  statusText: () => string

  /** Whether BYOK is selected but can't work (no key). */
  isMisconfigured: () => boolean
}

export const useAiStore = create<AiStore>((set, get) => ({
  settings: { ...AI_DEFAULTS },
  loaded: false,

  load: async () => {
    try {
      const raw = await invoke<string | null>('get_setting', { key: SETTINGS_KEY })
      if (raw) {
        const parsed = JSON.parse(raw) as Partial<AiSettings>
        set({ settings: { ...AI_DEFAULTS, ...parsed }, loaded: true })
      } else {
        // Migrate legacy claude_api_key if it exists
        const legacyKey = await invoke<string | null>('get_setting', { key: 'claude_api_key' }).catch(() => null)
        if (legacyKey) {
          const migrated = { ...AI_DEFAULTS, provider: 'claude' as AiProvider, claudeApiKey: legacyKey }
          set({ settings: migrated, loaded: true })
          // Persist the migration so next load uses the new format
          invoke('save_setting', { key: SETTINGS_KEY, value: JSON.stringify(migrated) }).catch(() => {})
        } else {
          set({ loaded: true })
        }
      }
    } catch {
      set({ loaded: true })
    }
  },

  save: async () => {
    const json = JSON.stringify(get().settings)
    await invoke('save_setting', { key: SETTINGS_KEY, value: json })
    // Also write the active key to the legacy setting for backward compat
    // (the caption generator reads claude_api_key directly)
    const s = get().settings
    if (s.claudeApiKey) {
      await invoke('save_setting', { key: 'claude_api_key', value: s.claudeApiKey }).catch(() => {})
    }
  },

  update: (patch) => {
    set(state => ({ settings: { ...state.settings, ...patch } }))

    // Auto-save ALL changes — provider switches save immediately,
    // key/model changes are debounced so we don't hit DB on every keystroke
    const isProviderSwitch = patch.provider !== undefined
    if (isProviderSwitch) {
      // Provider switch: save immediately (clears any pending debounced save first)
      if (_saveTimer) { clearTimeout(_saveTimer); _saveTimer = null }
      setTimeout(() => get().save().catch(() => {}), 0)
    } else {
      // Key/model/flag changes: debounce
      if (_saveTimer) clearTimeout(_saveTimer)
      _saveTimer = setTimeout(() => {
        _saveTimer = null
        get().save().catch(() => {})
      }, SAVE_DEBOUNCE_MS)
    }
  },

  activeKey: () => {
    const s = get().settings
    switch (s.provider) {
      case 'openai': return s.openaiApiKey
      case 'claude': return s.claudeApiKey
      case 'gemini': return s.geminiApiKey
      default: return ''
    }
  },

  isByok: () => get().settings.provider !== 'free',

  effectiveMode: () => {
    const s = get().settings
    if (s.provider === 'free') return 'free'
    // BYOK selected — check if key exists
    const key = get().activeKey()
    return key ? s.provider : 'free'
  },

  statusText: () => {
    const s = get().settings
    if (s.provider === 'free') return 'Current mode: Free (no cost)'
    const name = PROVIDER_META[s.provider].name
    const key = get().activeKey()
    if (!key) return `${name} selected — no API key. Using Free mode.`
    const model = s[`${s.provider}Model` as keyof AiSettings] as string
    return `Current mode: ${name} (${model})`
  },

  isMisconfigured: () => {
    const s = get().settings
    return s.provider !== 'free' && !get().activeKey()
  },
}))
