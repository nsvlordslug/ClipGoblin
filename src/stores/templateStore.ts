import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { CopyTone } from '../lib/publishCopyGenerator'

// ── Types ──

export interface ClipTemplate {
  id: string
  name: string
  /** If true, this is a built-in template that cannot be deleted/renamed */
  builtIn?: boolean
  captionStyleId: string
  captionPosition: 'top' | 'center' | 'bottom'
  /** Preferred caption tone for generation */
  captionTone: CopyTone
  /** Default hashtags applied when template is loaded */
  hashtags: string[]
  /** Export preset id (tiktok, youtube, reels, shorts) */
  exportPresetId: string
  createdAt: string
}

// ── Built-in starter templates ──

const BUILTIN_TEMPLATES: ClipTemplate[] = [
  {
    id: '__builtin_tiktok_gaming',
    name: 'TikTok Gaming',
    builtIn: true,
    captionStyleId: 'neon',
    captionPosition: 'bottom',
    captionTone: 'hype',
    hashtags: ['gaming', 'fyp', 'clips', 'twitch', 'gamer'],
    exportPresetId: 'tiktok',
    createdAt: '2025-01-01T00:00:00Z',
  },
  {
    id: '__builtin_youtube_highlights',
    name: 'YouTube Highlights',
    builtIn: true,
    captionStyleId: 'bold-white',
    captionPosition: 'bottom',
    captionTone: 'search',
    hashtags: ['highlights', 'gaming', 'gameplay', 'bestmoments'],
    exportPresetId: 'youtube',
    createdAt: '2025-01-01T00:00:00Z',
  },
  {
    id: '__builtin_reels_minimal',
    name: 'Reels Clean',
    builtIn: true,
    captionStyleId: 'clean',
    captionPosition: 'bottom',
    captionTone: 'clean',
    hashtags: ['reels', 'gaming', 'clips'],
    exportPresetId: 'reels',
    createdAt: '2025-01-01T00:00:00Z',
  },
  {
    id: '__builtin_shorts_fire',
    name: 'Shorts Fire',
    builtIn: true,
    captionStyleId: 'fire',
    captionPosition: 'bottom',
    captionTone: 'punchy',
    hashtags: ['shorts', 'gaming', 'viral', 'fire'],
    exportPresetId: 'shorts',
    createdAt: '2025-01-01T00:00:00Z',
  },
]

const SETTINGS_KEY = 'clip_templates'

// ── Store ──

interface TemplateStore {
  templates: ClipTemplate[]
  loaded: boolean
  load: () => Promise<void>
  save: () => Promise<void>
  /** Create a new custom template. Returns the new template. */
  create: (name: string, data: Omit<ClipTemplate, 'id' | 'name' | 'builtIn' | 'createdAt'>) => ClipTemplate
  /** Delete a custom template by id. Built-ins are protected. */
  remove: (id: string) => void
  /** Rename a custom template. Built-ins are protected. */
  rename: (id: string, newName: string) => void
}

export const useTemplateStore = create<TemplateStore>((set, get) => ({
  templates: [...BUILTIN_TEMPLATES],
  loaded: false,

  load: async () => {
    try {
      const raw = await invoke<string | null>('get_setting', { key: SETTINGS_KEY })
      if (raw) {
        const custom = JSON.parse(raw) as ClipTemplate[]
        // Merge: always keep latest built-ins + persisted custom templates
        set({ templates: [...BUILTIN_TEMPLATES, ...custom.filter(t => !t.builtIn)], loaded: true })
      } else {
        set({ loaded: true })
      }
    } catch {
      set({ loaded: true })
    }
  },

  save: async () => {
    // Only persist custom templates — built-ins are hardcoded
    const custom = get().templates.filter(t => !t.builtIn)
    const json = JSON.stringify(custom)
    await invoke('save_setting', { key: SETTINGS_KEY, value: json })
  },

  create: (name, data) => {
    const template: ClipTemplate = {
      ...data,
      id: `custom_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`,
      name,
      builtIn: false,
      createdAt: new Date().toISOString(),
    }
    set(s => ({ templates: [...s.templates, template] }))
    setTimeout(() => get().save().catch(() => {}), 0)
    return template
  },

  remove: (id) => {
    const target = get().templates.find(t => t.id === id)
    if (!target || target.builtIn) return
    set(s => ({ templates: s.templates.filter(t => t.id !== id) }))
    setTimeout(() => get().save().catch(() => {}), 0)
  },

  rename: (id, newName) => {
    const target = get().templates.find(t => t.id === id)
    if (!target || target.builtIn) return
    set(s => ({
      templates: s.templates.map(t => t.id === id ? { ...t, name: newName } : t),
    }))
    setTimeout(() => get().save().catch(() => {}), 0)
  },
}))
