import type { Clip } from '../types'

export type MontageSourceFilter = 'all' | 'twitch' | 'medal' | 'obs' | 'meld' | 'local'
export type MontageTransitionMode = 'cut' | 'crossfade'

export const MONTAGE_CROSSFADE_SECONDS = 0.5
export const YOUTUBE_SHORTS_MAX_SECONDS = 180

export function montageCrossfadeDuration(segmentDurations: number[]): number {
  const durations = segmentDurations.filter(duration => Number.isFinite(duration) && duration > 0)
  if (durations.length < 2) return 0
  return Math.min(MONTAGE_CROSSFADE_SECONDS, Math.min(...durations) / 2)
}

export function montageCrossfadeProgress(
  absoluteTime: number,
  clipEnd: number,
  crossfadeDuration: number,
): number {
  if (crossfadeDuration <= 0) return 0
  const transitionStart = clipEnd - crossfadeDuration
  return Math.max(0, Math.min(1, (absoluteTime - transitionStart) / crossfadeDuration))
}

export function exceedsYouTubeShortsLimit(durationSeconds: number): boolean {
  return durationSeconds > YOUTUBE_SHORTS_MAX_SECONDS + 1 / 30
}

export function montageDuration(
  segmentDurations: number[],
  transition: MontageTransitionMode,
): number {
  const durations = segmentDurations.map(duration => Math.max(0, duration))
  const total = durations.reduce((sum, duration) => sum + duration, 0)
  if (transition !== 'crossfade' || durations.length < 2) return total

  const overlap = montageCrossfadeDuration(durations)
  return Math.max(0, total - overlap * (durations.length - 1))
}

export function montageSourceGroup(clip: Pick<Clip, 'source_kind'>): Exclude<MontageSourceFilter, 'all'> {
  if (clip.source_kind === 'medal') return 'medal'
  if (clip.source_kind === 'obs') return 'obs'
  if (clip.source_kind === 'meld') return 'meld'
  if (clip.source_kind === 'manual') return 'local'
  return 'twitch'
}

export function filterAvailableMontageClips(
  clips: Clip[],
  selectedClipIds: string[],
  sourceFilter: MontageSourceFilter,
  search: string,
) {
  const selected = new Set(selectedClipIds)
  const normalizedSearch = search.trim().toLocaleLowerCase()
  return clips.filter(clip => {
    if (selected.has(clip.id)) return false
    if (sourceFilter !== 'all' && montageSourceGroup(clip) !== sourceFilter) return false
    return !normalizedSearch
      || clip.title.toLocaleLowerCase().includes(normalizedSearch)
      || (clip.game || '').toLocaleLowerCase().includes(normalizedSearch)
  })
}

export function nextMontageClipId(clipIds: string[], currentClipId: string | null): string | null {
  if (clipIds.length === 0) return null
  const currentIndex = currentClipId ? clipIds.indexOf(currentClipId) : -1
  return currentIndex >= 0 && currentIndex < clipIds.length - 1
    ? clipIds[currentIndex + 1]
    : null
}
