import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export type JobStatus = 'queued' | 'running' | 'completed' | 'failed'

export interface JobProgress {
  jobId: string
  progress: number
  status: JobStatus
  error?: string
}

export type ErrorCategory = 'ffmpeg' | 'transcription' | 'api' | 'download' | 'database' | 'not_found' | 'unknown'

export interface AppError {
  category: ErrorCategory
  message: string
  detail: string
}

interface JobState {
  /** Live job map, keyed by jobId. */
  jobs: Record<string, JobProgress>

  /** Most recent backend errors (newest first, capped at 50). */
  errors: AppError[]

  /** Start listening for backend events. Call once at app mount.
   *  Returns a cleanup function that unsubscribes both listeners. */
  startListening: () => Promise<UnlistenFn>

  /** Fetch all jobs from backend (one-time snapshot). */
  fetchJobs: () => Promise<void>

  /** Remove a finished job from backend + local state. */
  removeJob: (jobId: string) => Promise<void>

  /** Dismiss a specific error by index. */
  dismissError: (index: number) => void

  /** Clear all errors. */
  clearErrors: () => void
}

const MAX_ERRORS = 50

/** Guard: prevent duplicate listener registration across React strict-mode / re-renders. */
let _listenerCleanup: UnlistenFn | null = null

export const useJobStore = create<JobState>((set, _get) => ({
  jobs: {},
  errors: [],

  startListening: async () => {
    // Already listening — return existing cleanup
    if (_listenerCleanup) return _listenerCleanup

    const unlistenProgress = await listen<JobProgress>('job-progress', (event) => {
      const payload = event.payload
      set((state) => ({
        jobs: { ...state.jobs, [payload.jobId]: payload },
      }))
    })

    const unlistenError = await listen<AppError>('job-error', (event) => {
      const err = event.payload
      console.error(`[${err.category}] ${err.message}`)
      set((state) => ({
        errors: [err, ...state.errors].slice(0, MAX_ERRORS),
      }))
    })

    _listenerCleanup = () => {
      unlistenProgress()
      unlistenError()
      _listenerCleanup = null
    }

    return _listenerCleanup
  },

  fetchJobs: async () => {
    try {
      const list = await invoke<Array<{ id: string; status: JobStatus; progress: number; error?: string }>>('list_jobs')
      const jobs: Record<string, JobProgress> = {}
      for (const j of list) {
        jobs[j.id] = { jobId: j.id, progress: j.progress, status: j.status, error: j.error }
      }
      set({ jobs })
    } catch (err) {
      console.error('Failed to fetch jobs:', err)
    }
  },

  removeJob: async (jobId: string) => {
    try {
      await invoke('remove_job', { id: jobId })
      set((state) => {
        const { [jobId]: _, ...rest } = state.jobs
        return { jobs: rest }
      })
    } catch (err) {
      console.error('Failed to remove job:', err)
    }
  },

  dismissError: (index: number) => {
    set((state) => ({
      errors: state.errors.filter((_, i) => i !== index),
    }))
  },

  clearErrors: () => set({ errors: [] }),
}))
