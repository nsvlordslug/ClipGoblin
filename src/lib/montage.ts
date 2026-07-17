import type { Clip } from '../types'

export type MontageSourceFilter = 'all' | 'twitch' | 'medal' | 'obs' | 'meld' | 'local'

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
