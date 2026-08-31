import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { TwitchChannel, Vod, Highlight, Clip } from '../types'
import { parseStoredTags } from '../lib/tags'
import {
  emptySessionResourceState,
  indexVods,
  reconcileVods,
  resourceIsCurrent,
  shouldLoadResource,
  shouldRefreshVods,
  type SessionResourceState,
} from '../lib/sessionResourceCache'

type HighlightPayload = Omit<Highlight, 'tags'> & { tags: unknown }
type FetchOptions = { force?: boolean }

function normalizeHighlight(highlight: HighlightPayload): Highlight {
  return { ...highlight, tags: parseStoredTags(highlight.tags) }
}

interface AppState {
  channels: TwitchChannel[]
  vods: Vod[]
  highlights: Highlight[]
  clips: Clip[]
  loggedInUser: TwitchChannel | null
  loginChecked: boolean
  isLoading: boolean
  error: string | null
  speechModelSelectionSaving: boolean
  vodsResource: SessionResourceState
  clipsResource: SessionResourceState
  highlightsResource: SessionResourceState
  vodDetailsById: Record<string, Vod>

  checkLogin: () => Promise<void>
  twitchLogin: () => Promise<void>
  twitchLogout: () => Promise<void>
  ensureVods: (channelId: string) => Promise<void>
  fetchVods: (channelId: string, options?: FetchOptions) => Promise<void>
  refreshVods: (channelId: string) => Promise<void>
  ensureVodDetails: (vodIds: string[]) => Promise<void>
  invalidateVods: (channelId?: string) => void
  removeVod: (vodId: string) => void
  updateVod: (vodId: string, patch: Partial<import('../types').Vod>) => void
  removeClipsForVod: (vodId: string) => void
  fetchHighlights: (vodId?: string, options?: FetchOptions) => Promise<void>
  fetchClips: (options?: FetchOptions) => Promise<void>
  invalidateHighlights: () => void
  invalidateClips: () => void
  clearError: () => void
  setSpeechModelSelectionSaving: (saving: boolean) => void
}

const LOCAL_ACCOUNT_KEY = '__local__'

let loginInFlight: Promise<void> | null = null
const cachedVodsInFlight = new Map<string, Promise<void>>()
const remoteVodsInFlight = new Map<string, Promise<void>>()
const refreshedVodsInFlight = new Map<string, Promise<void>>()
const clipsInFlight = new Map<string, Promise<void>>()
const highlightsInFlight = new Map<string, Promise<void>>()
const vodDetailInFlight = new Map<string, Promise<Vod | null>>()

function accountKey(accountId: string | null): string {
  return accountId ?? LOCAL_ACCOUNT_KEY
}

function currentAccountId(state: AppState): string | null {
  return state.loggedInUser?.id ?? null
}

function channelIsCurrent(state: AppState, channelId: string): boolean {
  return state.loggedInUser?.id === channelId || state.channels.some(channel => channel.id === channelId)
}

function accountIsCurrent(state: AppState, accountId: string | null): boolean {
  return currentAccountId(state) === accountId
}

function invalidatedResource(
  resource: SessionResourceState,
  accountId: string | null,
): SessionResourceState {
  return {
    ...resource,
    loaded: false,
    loadedAt: null,
    refreshedAt: null,
    accountId,
    loading: false,
    refreshing: false,
  }
}

export const useAppStore = create<AppState>((set, get) => ({
  channels: [],
  vods: [],
  highlights: [],
  clips: [],
  loggedInUser: null,
  loginChecked: false,
  isLoading: false,
  error: null,
  speechModelSelectionSaving: false,
  vodsResource: emptySessionResourceState(),
  clipsResource: emptySessionResourceState(),
  highlightsResource: emptySessionResourceState(),
  vodDetailsById: {},

  checkLogin: async () => {
    if (get().loginChecked) return
    if (loginInFlight) return loginInFlight

    const task = (async () => {
      try {
        const user = await invoke<TwitchChannel | null>('get_logged_in_user')
        const nextUser = user || null
        const previousAccountId = currentAccountId(get())
        const nextAccountId = nextUser?.id ?? null

        if (previousAccountId !== nextAccountId) {
          set({
            loggedInUser: nextUser,
            loginChecked: true,
            channels: nextUser ? [nextUser] : [],
            vods: [],
            highlights: [],
            clips: [],
            vodDetailsById: {},
            vodsResource: emptySessionResourceState(nextAccountId),
            clipsResource: emptySessionResourceState(nextAccountId),
            highlightsResource: emptySessionResourceState(nextAccountId),
          })
        } else {
          set({
            loggedInUser: nextUser,
            loginChecked: true,
            channels: nextUser ? [nextUser] : [],
          })
        }
      } catch (err) {
        console.error('Failed to check login:', err)
        set({ loginChecked: true })
      }
    })()

    loginInFlight = task
    try {
      await task
    } finally {
      if (loginInFlight === task) loginInFlight = null
    }
  },

  twitchLogin: async () => {
    set({ isLoading: true, error: null })
    try {
      const channel = await invoke<TwitchChannel>('twitch_login')
      set({
        loggedInUser: channel,
        loginChecked: true,
        channels: [channel],
        vods: [],
        highlights: [],
        clips: [],
        vodDetailsById: {},
        vodsResource: emptySessionResourceState(channel.id),
        clipsResource: emptySessionResourceState(channel.id),
        highlightsResource: emptySessionResourceState(channel.id),
      })
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
      set({
        loggedInUser: null,
        loginChecked: true,
        channels: [],
        vods: [],
        highlights: [],
        clips: [],
        vodDetailsById: {},
        vodsResource: emptySessionResourceState(),
        clipsResource: emptySessionResourceState(),
        highlightsResource: emptySessionResourceState(),
      })
    } catch (err) {
      console.error('Failed to logout:', err)
    }
  },

  ensureVods: async (channelId: string) => {
    const state = get()
    if (!channelIsCurrent(state, channelId)) return

    if (shouldLoadResource(state.vodsResource, channelId)) {
      let cachedTask = cachedVodsInFlight.get(channelId)
      if (!cachedTask) {
        cachedTask = (async () => {
          set(current => ({
            vodsResource: {
              ...current.vodsResource,
              accountId: channelId,
              loading: true,
            },
          }))
          try {
            const cachedVods = await invoke<Vod[]>('get_cached_vods', { channelId })
            if (!channelIsCurrent(get(), channelId)) return
            const now = Date.now()
            set(current => {
              const vods = reconcileVods(current.vods, cachedVods)
              return {
                vods,
                vodDetailsById: indexVods(current.vodDetailsById, vods),
                vodsResource: {
                  ...current.vodsResource,
                  loaded: true,
                  loadedAt: now,
                  accountId: channelId,
                  loading: false,
                },
              }
            })
          } catch (err) {
            console.error('Failed to load cached VODs:', err)
            if (channelIsCurrent(get(), channelId)) {
              set(current => ({
                vodsResource: { ...current.vodsResource, loading: false },
              }))
            }
          }
        })()
        cachedVodsInFlight.set(channelId, cachedTask)
        void cachedTask.finally(() => {
          if (cachedVodsInFlight.get(channelId) === cachedTask) {
            cachedVodsInFlight.delete(channelId)
          }
        })
      }
      await cachedTask
    }

    if (channelIsCurrent(get(), channelId) && shouldRefreshVods(get().vodsResource, channelId)) {
      void get().fetchVods(channelId)
    }
  },

  fetchVods: async (channelId: string, options: FetchOptions = {}) => {
    const existing = remoteVodsInFlight.get(channelId)
    if (existing) {
      await existing
      if (options.force) return get().fetchVods(channelId, { force: true })
      return
    }

    const task = (async () => {
      const currentResource = get().vodsResource
      const hasCurrentCache = resourceIsCurrent(currentResource, channelId)
      set(current => ({
        vodsResource: {
          ...current.vodsResource,
          accountId: channelId,
          loading: !hasCurrentCache,
          refreshing: hasCurrentCache,
        },
      }))

      try {
        const incoming = await invoke<Vod[]>('get_vods', { channelId })
        if (!channelIsCurrent(get(), channelId)) return
        const now = Date.now()
        set(current => {
          const vods = reconcileVods(current.vods, incoming)
          return {
            vods,
            vodDetailsById: indexVods(current.vodDetailsById, vods),
            vodsResource: {
              loaded: true,
              loadedAt: now,
              refreshedAt: now,
              accountId: channelId,
              loading: false,
              refreshing: false,
            },
          }
        })
      } catch (err) {
        console.error('Failed to fetch VODs from API, retaining cached VODs:', err)
        if (!channelIsCurrent(get(), channelId)) return
        if (!resourceIsCurrent(get().vodsResource, channelId)) {
          try {
            const cachedVods = await invoke<Vod[]>('get_cached_vods', { channelId })
            if (!channelIsCurrent(get(), channelId)) return
            const now = Date.now()
            set(current => {
              const vods = reconcileVods(current.vods, cachedVods)
              return {
                vods,
                vodDetailsById: indexVods(current.vodDetailsById, vods),
                vodsResource: {
                  ...current.vodsResource,
                  loaded: true,
                  loadedAt: now,
                  accountId: channelId,
                  loading: false,
                  refreshing: false,
                },
              }
            })
          } catch (cacheErr) {
            console.error('Failed to fetch cached VODs:', cacheErr)
          }
        }
      } finally {
        if (channelIsCurrent(get(), channelId)) {
          set(current => ({
            vodsResource: {
              ...current.vodsResource,
              refreshedAt: current.vodsResource.refreshedAt ?? Date.now(),
              loading: false,
              refreshing: false,
            },
          }))
        }
      }
    })()

    remoteVodsInFlight.set(channelId, task)
    try {
      await task
    } finally {
      if (remoteVodsInFlight.get(channelId) === task) remoteVodsInFlight.delete(channelId)
    }
  },

  refreshVods: async (channelId: string) => {
    const existing = refreshedVodsInFlight.get(channelId)
    if (existing) return existing

    const task = (async () => {
      try {
        const incoming = await invoke<Vod[]>('get_cached_vods', { channelId })
        if (!channelIsCurrent(get(), channelId)) return
        const now = Date.now()
        set(current => {
          const vods = reconcileVods(current.vods, incoming)
          return {
            vods,
            vodDetailsById: indexVods(current.vodDetailsById, vods),
            vodsResource: {
              ...current.vodsResource,
              loaded: true,
              loadedAt: now,
              accountId: channelId,
              loading: false,
            },
          }
        })
      } catch (err) {
        console.error('Failed to refresh VODs:', err)
      }
    })()

    refreshedVodsInFlight.set(channelId, task)
    try {
      await task
    } finally {
      if (refreshedVodsInFlight.get(channelId) === task) refreshedVodsInFlight.delete(channelId)
    }
  },

  ensureVodDetails: async (vodIds: string[]) => {
    const requested = [...new Set(vodIds.filter(Boolean))]
    if (requested.length === 0) return

    const initial = get()
    const seededDetails = indexVods(initial.vodDetailsById, initial.vods)
    if (seededDetails !== initial.vodDetailsById) {
      set({ vodDetailsById: seededDetails })
    }
    const missing = requested.filter(vodId => !seededDetails[vodId])
    if (missing.length === 0) return

    const requestAccountId = currentAccountId(get())
    await Promise.all(missing.map(async vodId => {
      const detailKey = `${accountKey(requestAccountId)}:${vodId}`
      let detailTask = vodDetailInFlight.get(detailKey)
      if (!detailTask) {
        detailTask = invoke<Vod>('get_vod_detail', { vodId })
          .then(vod => {
            if (accountIsCurrent(get(), requestAccountId)) {
              set(current => ({
                vodDetailsById: { ...current.vodDetailsById, [vod.id]: vod },
              }))
            }
            return vod
          })
          .catch(error => {
            console.warn(`[AppStore] Failed to load VOD detail ${vodId}:`, error)
            return null
          })
        vodDetailInFlight.set(detailKey, detailTask)
        void detailTask.finally(() => {
          if (vodDetailInFlight.get(detailKey) === detailTask) vodDetailInFlight.delete(detailKey)
        })
      }
      await detailTask
    }))
  },

  invalidateVods: (channelId?: string) => {
    set(state => ({
      vodsResource: invalidatedResource(
        state.vodsResource,
        channelId ?? currentAccountId(state),
      ),
    }))
  },

  removeVod: (vodId: string) => {
    set((state) => {
      const vodDetailsById = { ...state.vodDetailsById }
      delete vodDetailsById[vodId]
      return {
        vods: state.vods.filter(v => v.id !== vodId),
        highlights: state.highlights.filter(h => h.vod_id !== vodId),
        vodDetailsById,
      }
    })
  },

  updateVod: (vodId: string, patch: Partial<Vod>) => {
    set((state) => {
      const vods = state.vods.map(v => v.id === vodId ? { ...v, ...patch } : v)
      const existing = state.vodDetailsById[vodId] ?? vods.find(vod => vod.id === vodId)
      return {
        vods,
        vodDetailsById: existing
          ? { ...state.vodDetailsById, [vodId]: { ...existing, ...patch } }
          : state.vodDetailsById,
      }
    })
  },

  removeClipsForVod: (vodId: string) => {
    set((state) => ({
      clips: state.clips.filter(c => c.vod_id !== vodId),
    }))
  },

  fetchHighlights: async (vodId?: string, options: FetchOptions = {}) => {
    const requestAccountId = currentAccountId(get())
    const key = `${accountKey(requestAccountId)}:${vodId ?? 'all'}`
    if (!vodId && !shouldLoadResource(get().highlightsResource, requestAccountId, options.force)) {
      return
    }
    const existing = highlightsInFlight.get(key)
    if (existing) {
      await existing
      if (options.force) return get().fetchHighlights(vodId, { force: true })
      return
    }

    const task = (async () => {
      if (!vodId) {
        set(current => ({
          highlightsResource: {
            ...current.highlightsResource,
            accountId: requestAccountId,
            loading: true,
          },
        }))
      }
      try {
        const payload = vodId
          ? await invoke<HighlightPayload[]>('get_highlights', { vodId })
          : await invoke<HighlightPayload[]>('get_all_highlights')
        if (!accountIsCurrent(get(), requestAccountId)) return
        const highlights = payload.map(normalizeHighlight)
        const now = Date.now()
        set(current => vodId
          ? {
              highlights: [
                ...current.highlights.filter(highlight => highlight.vod_id !== vodId),
                ...highlights,
              ],
            }
          : {
              highlights,
              highlightsResource: {
                loaded: true,
                loadedAt: now,
                refreshedAt: now,
                accountId: requestAccountId,
                loading: false,
                refreshing: false,
              },
            })
      } catch (err) {
        console.error('Failed to fetch highlights:', err)
      } finally {
        if (!vodId && accountIsCurrent(get(), requestAccountId)) {
          set(current => ({
            highlightsResource: { ...current.highlightsResource, loading: false },
          }))
        }
      }
    })()

    highlightsInFlight.set(key, task)
    try {
      await task
    } finally {
      if (highlightsInFlight.get(key) === task) highlightsInFlight.delete(key)
    }
  },

  fetchClips: async (options: FetchOptions = {}) => {
    const requestAccountId = currentAccountId(get())
    if (!shouldLoadResource(get().clipsResource, requestAccountId, options.force)) return
    const key = accountKey(requestAccountId)
    const existing = clipsInFlight.get(key)
    if (existing) {
      await existing
      if (options.force) return get().fetchClips({ force: true })
      return
    }

    const task = (async () => {
      set(current => ({
        clipsResource: {
          ...current.clipsResource,
          accountId: requestAccountId,
          loading: true,
        },
      }))
      try {
        const clips = await invoke<Clip[]>('get_clips')
        if (!accountIsCurrent(get(), requestAccountId)) return
        const now = Date.now()
        set({
          clips,
          clipsResource: {
            loaded: true,
            loadedAt: now,
            refreshedAt: now,
            accountId: requestAccountId,
            loading: false,
            refreshing: false,
          },
        })
      } catch (err) {
        console.error('Failed to fetch clips:', err)
      } finally {
        if (accountIsCurrent(get(), requestAccountId)) {
          set(current => ({
            clipsResource: { ...current.clipsResource, loading: false },
          }))
        }
      }
    })()

    clipsInFlight.set(key, task)
    try {
      await task
    } finally {
      if (clipsInFlight.get(key) === task) clipsInFlight.delete(key)
    }
  },

  invalidateHighlights: () => {
    set(state => ({
      highlightsResource: invalidatedResource(
        state.highlightsResource,
        currentAccountId(state),
      ),
    }))
  },

  invalidateClips: () => {
    set(state => ({
      clipsResource: invalidatedResource(
        state.clipsResource,
        currentAccountId(state),
      ),
    }))
  },

  clearError: () => set({ error: null }),
  setSpeechModelSelectionSaving: (saving: boolean) => set({ speechModelSelectionSaving: saving }),
}))
