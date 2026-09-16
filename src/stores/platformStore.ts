import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { withStartupRetry } from '../lib/startupRetry'

// ── Types (match Rust social::ConnectedAccount) ──

export interface ConnectedAccount {
  platform: string
  account_name: string
  account_handle?: string | null
  account_id: string
  connected_at: string
}

export interface UploadResult {
  status: UploadResultStatus
  job_id: string
}

export type UploadResultStatus =
  | { status: 'uploading'; progress_pct: number }
  | { status: 'processing' }
  | { status: 'inbox_delivered' }
  | { status: 'complete'; video_url: string | null; platform_video_id: string | null }
  | { status: 'failed'; error: string }
  | { status: 'duplicate'; existing_url: string | null }

// ── Platform metadata ──

export const PLATFORM_INFO: Record<string, { name: string; color: string; icon: string; available: boolean }> = {
  youtube:   { name: 'YouTube', color: '#ff0000', icon: 'YT', available: true },
  tiktok:    { name: 'TikTok',  color: '#00f2ea', icon: 'TT', available: true },
}

// ── Store ──

interface PlatformState {
  accounts: Record<string, ConnectedAccount | null>
  loading: Record<string, boolean>
  loaded: boolean
  load: () => Promise<void>
  connect: (platform: string) => Promise<ConnectedAccount>
  disconnect: (platform: string) => Promise<void>
  isConnected: (platform: string) => boolean
  getAccount: (platform: string) => ConnectedAccount | null
}

let loadInFlight: Promise<void> | null = null

export const usePlatformStore = create<PlatformState>((set, get) => ({
  accounts: {},
  loading: {},
  loaded: false,

  load: async () => {
    if (loadInFlight) return loadInFlight

    const task = (async () => {
      try {
        const accounts = await withStartupRetry(() =>
          invoke<ConnectedAccount[]>('get_all_connected_accounts'),
        )
        const map: Record<string, ConnectedAccount | null> = {}
        for (const acct of accounts) {
          map[acct.platform] = acct
        }
        set({ accounts: map, loaded: true })

        const tiktok = map.tiktok
        if (tiktok && !tiktok.account_handle?.trim()) {
          set(s => ({ loading: { ...s.loading, tiktok: true } }))
          try {
            const repaired = await withStartupRetry(
              () => invoke<ConnectedAccount>('repair_tiktok_account_identity'),
              [0, 250, 750],
            )
            set(s => ({
              accounts: { ...s.accounts, tiktok: repaired },
              loading: { ...s.loading, tiktok: false },
            }))
          } catch (error) {
            console.warn('TikTok account identity could not be refreshed:', error)
            set(s => ({ loading: { ...s.loading, tiktok: false } }))
          }
        }
      } catch (e) {
        console.error('Failed to load connected accounts:', e)
      }
    })()

    loadInFlight = task
    try {
      await task
    } finally {
      if (loadInFlight === task) loadInFlight = null
    }
  },

  connect: async (platform: string) => {
    set(s => ({ loading: { ...s.loading, [platform]: true } }))
    try {
      const account = await invoke<ConnectedAccount>('connect_platform', { platform })
      set(s => ({
        accounts: { ...s.accounts, [platform]: account },
        loading: { ...s.loading, [platform]: false },
      }))
      return account
    } catch (e) {
      set(s => ({ loading: { ...s.loading, [platform]: false } }))
      throw e
    }
  },

  disconnect: async (platform: string) => {
    set(s => ({ loading: { ...s.loading, [platform]: true } }))
    try {
      await invoke('disconnect_platform', { platform })
      set(s => ({
        accounts: { ...s.accounts, [platform]: null },
        loading: { ...s.loading, [platform]: false },
      }))
    } catch (e) {
      set(s => ({ loading: { ...s.loading, [platform]: false } }))
      throw e
    }
  },

  isConnected: (platform: string) => {
    return get().accounts[platform] != null
  },

  getAccount: (platform: string) => {
    return get().accounts[platform] ?? null
  },
}))
