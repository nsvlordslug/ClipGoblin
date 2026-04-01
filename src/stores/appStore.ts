import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { TwitchChannel, Vod, Highlight, Clip } from '../types'

interface AppState {
  channels: TwitchChannel[]
  vods: Vod[]
  highlights: Highlight[]
  clips: Clip[]
  loggedInUser: TwitchChannel | null
  isLoading: boolean
  error: string | null

  checkLogin: () => Promise<void>
  twitchLogin: () => Promise<void>
  twitchLogout: () => Promise<void>
  fetchVods: (channelId: string) => Promise<void>
  refreshVods: (channelId: string) => Promise<void>
  removeVod: (vodId: string) => void
  updateVod: (vodId: string, patch: Partial<import('../types').Vod>) => void
  removeClipsForVod: (vodId: string) => void
  fetchHighlights: (vodId?: string) => Promise<void>
  fetchClips: () => Promise<void>
  clearError: () => void
}

export const useAppStore = create<AppState>((set) => ({
  channels: [],
  vods: [],
  highlights: [],
  clips: [],
  loggedInUser: null,
  isLoading: false,
  error: null,

  checkLogin: async () => {
    try {
      const user = await invoke<TwitchChannel | null>('get_logged_in_user')
      set({ loggedInUser: user || null, channels: user ? [user] : [] })
    } catch (err) {
      console.error('Failed to check login:', err)
    }
  },

  twitchLogin: async () => {
    set({ isLoading: true, error: null })
    try {
      const channel = await invoke<TwitchChannel>('twitch_login')
      set({ loggedInUser: channel, channels: [channel] })
    } catch (err) {
      const msg = String(err)
      console.error('Failed to login:', msg)
      set({ error: msg })
      throw err
    } finally {
      set({ isLoading: false })
    }
  },

  twitchLogout: async () => {
    try {
      await invoke('twitch_logout')
      set({ loggedInUser: null, channels: [], vods: [], highlights: [], clips: [] })
    } catch (err) {
      console.error('Failed to logout:', err)
    }
  },

  fetchVods: async (channelId: string) => {
    set({ isLoading: true })
    try {
      const vods = await invoke<Vod[]>('get_vods', { channelId })
      set({ vods })
    } catch (err) {
      console.error('Failed to fetch VODs from API, falling back to cache:', err)
      // Fall back to cached VODs from DB when API fails (e.g. expired token)
      try {
        const vods = await invoke<Vod[]>('get_cached_vods', { channelId })
        set({ vods })
      } catch (cacheErr) {
        console.error('Failed to fetch cached VODs:', cacheErr)
      }
    } finally {
      set({ isLoading: false })
    }
  },

  refreshVods: async (channelId: string) => {
    try {
      const vods = await invoke<Vod[]>('get_cached_vods', { channelId })
      set({ vods })
    } catch (err) {
      console.error('Failed to refresh VODs:', err)
    }
  },

  removeVod: (vodId: string) => {
    set((state) => ({
      vods: state.vods.filter(v => v.id !== vodId),
      highlights: state.highlights.filter(h => h.vod_id !== vodId),
    }))
  },

  updateVod: (vodId: string, patch: Partial<Vod>) => {
    set((state) => ({
      vods: state.vods.map(v => v.id === vodId ? { ...v, ...patch } : v),
    }))
  },

  removeClipsForVod: (vodId: string) => {
    set((state) => ({
      clips: state.clips.filter(c => c.vod_id !== vodId),
    }))
  },

  fetchHighlights: async (vodId?: string) => {
    try {
      const highlights = vodId
        ? await invoke<Highlight[]>('get_highlights', { vodId })
        : await invoke<Highlight[]>('get_all_highlights')
      set({ highlights })
    } catch (err) {
      console.error('Failed to fetch highlights:', err)
    }
  },

  fetchClips: async () => {
    set({ isLoading: true })
    try {
      const clips = await invoke<Clip[]>('get_clips')
      set({ clips })
    } catch (err) {
      console.error('Failed to fetch clips:', err)
    } finally {
      set({ isLoading: false })
    }
  },

  clearError: () => set({ error: null }),
}))
