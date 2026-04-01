import { create } from 'zustand'
import type { AnalysisResult, CandidateClip } from '../types/analysis'

interface ResultsState {
  result: AnalysisResult | null
  selectedId: string | null
  analyzing: boolean
  progress: number

  setResult: (result: AnalysisResult) => void
  selectClip: (id: string) => void
  setAnalyzing: (v: boolean) => void
  setProgress: (v: number) => void
}

export const useResultsStore = create<ResultsState>((set) => ({
  result: null,
  selectedId: null,
  analyzing: false,
  progress: 0,

  setResult: (result) =>
    set({ result, selectedId: result.clips[0]?.id ?? null }),

  selectClip: (id) => set({ selectedId: id }),
  setAnalyzing: (analyzing) => set({ analyzing }),
  setProgress: (progress) => set({ progress }),
}))

// ── Selectors (call inside components) ──

export function getSelectedClip(state: ResultsState): CandidateClip | null {
  if (!state.result || !state.selectedId) return null
  return state.result.clips.find(c => c.id === state.selectedId) ?? null
}

export function getTopClips(state: ResultsState, count: number): CandidateClip[] {
  return state.result?.clips.slice(0, count) ?? []
}
