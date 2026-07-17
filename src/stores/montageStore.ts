import { create } from 'zustand'
import { createJSONStorage, persist } from 'zustand/middleware'

export type MontageExportPreset = 'youtube' | 'shorts'

export interface MontageSegment {
  clipId: string
  clipTitle: string
  startSeconds: number
  endSeconds: number
  thumbnailPath: string | null
}

export interface MontageProject {
  id: string
  title: string
  segments: MontageSegment[]
  exportPreset: MontageExportPreset
  publishTitle?: string
  publishDescription?: string
  publishHashtags?: string[]
  visibility?: 'public' | 'unlisted' | 'private'
  createdAt: string
}

interface MontageState {
  projects: MontageProject[]
  activeProjectId: string | null

  createProject: (title: string) => string  // returns project id
  deleteProject: (id: string) => void
  setActive: (id: string | null) => void
  getActive: () => MontageProject | null

  addClip: (projectId: string, segment: MontageSegment) => void
  removeClip: (projectId: string, clipId: string) => void
  reorderClips: (projectId: string, fromIndex: number, toIndex: number) => void
  updateProject: (projectId: string, patch: Partial<MontageProject>) => void
}

export const useMontageStore = create<MontageState>()(persist((set, get) => ({
  projects: [],
  activeProjectId: null,

  createProject: (title) => {
    const id = crypto.randomUUID()
    set(state => ({
      projects: [...state.projects, {
        id, title, segments: [], exportPreset: 'youtube',
        publishTitle: '', publishDescription: '', publishHashtags: [], visibility: 'public',
        createdAt: new Date().toISOString(),
      }],
      activeProjectId: id,
    }))
    return id
  },

  deleteProject: (id) => {
    set(state => ({
      projects: state.projects.filter(p => p.id !== id),
      activeProjectId: state.activeProjectId === id ? null : state.activeProjectId,
    }))
  },

  setActive: (id) => set({ activeProjectId: id }),

  getActive: () => {
    const { projects, activeProjectId } = get()
    return projects.find(p => p.id === activeProjectId) || null
  },

  addClip: (projectId, segment) => {
    set(state => ({
      projects: state.projects.map(p =>
        p.id === projectId && !p.segments.some(s => s.clipId === segment.clipId)
          ? { ...p, segments: [...p.segments, segment] }
          : p
      ),
    }))
  },

  removeClip: (projectId, clipId) => {
    set(state => ({
      projects: state.projects.map(p =>
        p.id === projectId
          ? { ...p, segments: p.segments.filter(s => s.clipId !== clipId) }
          : p
      ),
    }))
  },

  reorderClips: (projectId, fromIndex, toIndex) => {
    set(state => ({
      projects: state.projects.map(p => {
        if (p.id !== projectId) return p
        const segs = [...p.segments]
        const [moved] = segs.splice(fromIndex, 1)
        segs.splice(toIndex, 0, moved)
        return { ...p, segments: segs }
      }),
    }))
  },

  updateProject: (projectId, patch) => {
    set(state => ({
      projects: state.projects.map(p => p.id === projectId ? { ...p, ...patch } : p),
    }))
  },
}), {
  name: 'clipgoblin-montage-projects',
  storage: createJSONStorage(() => localStorage),
  partialize: state => ({
    projects: state.projects,
    activeProjectId: state.activeProjectId,
  }),
}))
