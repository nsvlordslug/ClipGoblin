import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { ScheduledUpload } from '../types'

interface ScheduleState {
  uploads: ScheduledUpload[]
  loading: boolean
  loaded: boolean
  loadedAt: number | null
  load: (options?: { force?: boolean }) => Promise<void>
  schedule: (clipId: string, platform: string, scheduledTime: string, metaJson: string) => Promise<string>
  cancel: (id: string) => Promise<boolean>
  reschedule: (id: string, newTime: string) => Promise<boolean>
  getForClip: (clipId: string) => Promise<ScheduledUpload[]>
  startListening: () => () => void
}

interface ScheduledUploadStatusEvent {
  id: string
  status: ScheduledUpload['status'] | 'exporting' | 'uploading' | 'retrying'
  clip_id: string
  platform: string
  video_url?: string | null
  error?: string | null
}

let loadInFlight: Promise<void> | null = null

export const useScheduleStore = create<ScheduleState>((set, get) => ({
  uploads: [],
  loading: false,
  loaded: false,
  loadedAt: null,

  load: async (options = {}) => {
    if (get().loaded && !options.force) return
    if (loadInFlight) {
      await loadInFlight
      if (options.force) return get().load({ force: true })
      return
    }

    const task = (async () => {
      set({ loading: true })
      try {
        const uploads = await invoke<ScheduledUpload[]>('list_scheduled_uploads')
        set({ uploads, loading: false, loaded: true, loadedAt: Date.now() })
      } catch (e) {
        console.error('[ScheduleStore] Failed to load scheduled uploads:', e)
        set({ loading: false })
      }
    })()

    loadInFlight = task
    try {
      await task
    } finally {
      if (loadInFlight === task) loadInFlight = null
    }
  },

  schedule: async (clipId, platform, scheduledTime, metaJson) => {
    const id = await invoke<string>('schedule_upload', {
      clipId,
      platform,
      scheduledTime,
      metaJson,
    })
    await get().load({ force: true })
    return id
  },

  cancel: async (id) => {
    const ok = await invoke<boolean>('cancel_scheduled_upload', { id })
    if (ok) await get().load({ force: true })
    return ok
  },

  reschedule: async (id, newTime) => {
    const ok = await invoke<boolean>('reschedule_upload', { id, newTime })
    if (ok) await get().load({ force: true })
    return ok
  },

  getForClip: async (clipId) => {
    return invoke<ScheduledUpload[]>('get_scheduled_uploads_for_clip', { clipId })
  },

  startListening: () => {
    const unlisten = listen<ScheduledUploadStatusEvent>('scheduled-upload-status', (event) => {
      const payload = event.payload
      set(state => {
        const uploads = state.uploads.map(u => {
          if (u.id === payload.id) {
            const status = payload.status === 'retrying'
              ? 'pending'
              : payload.status === 'exporting' || payload.status === 'uploading'
                ? 'processing'
                : payload.status
            return {
              ...u,
              status,
              video_url: payload.video_url || u.video_url,
              error_message: payload.error || u.error_message,
            }
          }
          return u
        })
        return { uploads }
      })
    })
    // Return cleanup function
    return () => { unlisten.then(fn => fn()) }
  },
}))
