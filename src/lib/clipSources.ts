export type ClipSourceTab = 'all' | 'twitch' | 'medal' | 'obs' | 'meld' | 'local'

export interface ClipSourceLike {
  source_kind?: string | null
}

export const CLIP_SOURCE_TABS: Array<{ id: ClipSourceTab; label: string }> = [
  { id: 'all', label: 'All' },
  { id: 'twitch', label: 'Twitch' },
  { id: 'medal', label: 'Medal' },
  { id: 'obs', label: 'OBS' },
  { id: 'meld', label: 'Meld' },
  { id: 'local', label: 'Local' },
]

export function clipSourceTabFor(clip: ClipSourceLike): Exclude<ClipSourceTab, 'all'> {
  const source = clip.source_kind?.trim().toLowerCase() || 'twitch_vod'
  if (source === 'medal' || source === 'obs' || source === 'meld') return source
  if (source === 'manual') return 'local'
  if (source === 'twitch_vod' || source === 'twitch_community' || source.startsWith('twitch')) {
    return 'twitch'
  }
  return 'local'
}

export function clipMatchesSourceTab(clip: ClipSourceLike, tab: ClipSourceTab): boolean {
  return tab === 'all' || clipSourceTabFor(clip) === tab
}

export function countClipsBySource(clips: ClipSourceLike[]): Record<ClipSourceTab, number> {
  const counts: Record<ClipSourceTab, number> = {
    all: clips.length,
    twitch: 0,
    medal: 0,
    obs: 0,
    meld: 0,
    local: 0,
  }
  for (const clip of clips) counts[clipSourceTabFor(clip)] += 1
  return counts
}
