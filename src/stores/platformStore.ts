import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

// ── Types (match Rust social::ConnectedAccount) ──

export interface ConnectedAccount {
  platform: string
  account_name: string
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
  | { status: 'complete'; video_url: string }
  | { status: 'failed'; error: string }
  | { status: 'duplicate'; existing_url: string }

// ── Platform metadata ──

export const PLATFORM_INFO: Record<string, { name: string; color: string; icon: string; available: boolean }> = {
  youtube:   { name: 'YouTube',   color: '#ff0000', icon: 'YT', available: true },
  tiktok:    { name: 'TikTok',    color: '#00f2ea', icon: 'TT', available: true },
  instagram: { name: 'Instagram', color: '#e1306c', icon: 'IG', available: false },
}

// ── Store ──

interface PlatformState {
  accounts: Record<string, ConnectedAccount | null>
  loading: Record<string, boolean>
  load: () => Promise<void>
  connect: (platform: string) => Promise<ConnectedAccount>
  disconnect: (platform: string) => Promise<void>
  isConnected: (platform: string) => boolean
  getAccount: (platform: string) => ConnectedAccount | null
}

export const usePlatformStore = create<PlatformState>((set, get) => ({
  accounts: {},
  loading: {},

  load: async () => {
    try {
      const accounts = await invoke<ConnectedAccount[]>('get_all_connected_accounts')
      const map: Record<string, ConnectedAccount | null> = {}
      for (const acct of accounts) {
        map[acct.platform] = acct
      }
      set({ accounts: map })
    } catch (e) {
      console.error('Failed to load connected accounts:', e)
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
