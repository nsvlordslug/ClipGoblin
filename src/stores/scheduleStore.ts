import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { ScheduledUpload } from '../types'

interface ScheduleState {
  uploads: ScheduledUpload[]
  loading: boolean
  load: () => Promise<void>
  schedule: (clipId: string, platform: string, scheduledTime: string, metaJson: string) => Promise<string>
  cancel: (id: string) => Promise<boolean>
  reschedule: (id: string, newTime: string) => Promise<boolean>
  getForClip: (clipId: string) => Promise<ScheduledUpload[]>
  startListening: () => () => void
}

export const useScheduleStore = create<ScheduleState>((set, get) => ({
  uploads: [],
  loading: false,

  load: async () => {
    set({ loading: true })
    try {
      const uploads = await invoke<ScheduledUpload[]>('list_scheduled_uploads')
      set({ uploads, loading: false })
    } catch (e) {
      console.error('[ScheduleStore] Failed to load scheduled uploads:', e)
      set({ loading: false })
    }
  },

  schedule: async (clipId, platform, scheduledTime, metaJson) => {
    const id = await invoke<string>('schedule_upload', {
      clipId,
      platform,
      scheduledTime,
      metaJson,
    })
    await get().load()
    return id
  },

  cancel: async (id) => {
    const ok = await invoke<boolean>('cancel_scheduled_upload', { id })
    if (ok) await get().load()
    return ok
  },

  reschedule: async (id, newTime) => {
    const ok = await invoke<boolean>('reschedule_upload', { id, newTime })
    if (ok) await get().load()
    return ok
  },

  getForClip: async (clipId) => {
    return invoke<ScheduledUpload[]>('get_scheduled_uploads_for_clip', { clipId })
  },

  startListening: () => {
    const unlisten = listen<any>('scheduled-upload-status', (event) => {
      const payload = event.payload
      set(state => {
        const uploads = state.uploads.map(u => {
          if (u.id === payload.id) {
            return {
              ...u,
              status: payload.status === 'retrying' ? 'pending' as const : payload.status,
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
