export type TwitchProvenanceKind =
  | 'twitch'
  | 'streamer'
  | 'viewer'
  | 'featured'
  | 'consensus'
  | 'local'
  | 'ai'

export interface TwitchProvenanceBadge {
  kind: TwitchProvenanceKind
  label: string
  compactLabel: string
  tooltip: string
}

const LOCAL_SIGNAL_LABELS: Record<string, string> = {
  audio: 'audio',
  chat: 'chat activity',
  transcript: 'transcript',
  emote_burst: 'emote activity',
}

function parseStringList(value: unknown): string[] {
  let entries: unknown[]
  if (Array.isArray(value)) {
    entries = value
  } else if (typeof value === 'string') {
    const trimmed = value.trim()
    if (!trimmed) return []
    try {
      const parsed: unknown = JSON.parse(trimmed)
      entries = Array.isArray(parsed) ? parsed : trimmed.split(',')
    } catch {
      entries = trimmed.split(',')
    }
  } else {
    return []
  }

  return [...new Set(
    entries
      .filter((entry): entry is string => typeof entry === 'string')
      .map(entry => entry.trim().toLowerCase())
      .filter(Boolean),
  )]
}

function joinSignalLabels(signals: string[]): string {
  const labels = signals.map(signal => LOCAL_SIGNAL_LABELS[signal] ?? signal)
  if (labels.length <= 1) return labels[0] ?? 'local detection'
  if (labels.length === 2) return `${labels[0]} and ${labels[1]}`
  return `${labels.slice(0, -1).join(', ')}, and ${labels.at(-1)}`
}

function capitalizeFirst(value: string): string {
  return value ? `${value[0].toUpperCase()}${value.slice(1)}` : value
}

export function deriveTwitchProvenance(
  rawTags: unknown,
  rawSignalSources: unknown,
): TwitchProvenanceBadge[] {
  const tags = parseStringList(rawTags)
  const sources = parseStringList(rawSignalSources)
  const tagSet = new Set(tags)
  const sourceSet = new Set(sources)
  const isTwitchClip = tagSet.has('community-clip') || sourceSet.has('community')
  const badges: TwitchProvenanceBadge[] = []
  if (isTwitchClip) {
    if (tagSet.has('streamer-created')) {
      badges.push({
        kind: 'streamer',
        label: 'Streamer Clip',
        compactLabel: 'Streamer Clip',
        tooltip: 'The streamer created this clip on their own Twitch channel.',
      })
    } else if (tagSet.has('viewer-created')) {
      badges.push({
        kind: 'viewer',
        label: 'Viewer Clip',
        compactLabel: 'Viewer Clip',
        tooltip: 'A viewer chose this moment and clipped it on Twitch.',
      })
    } else {
      badges.push({
        kind: 'twitch',
        label: 'Twitch Clip',
        compactLabel: 'Twitch Clip',
        tooltip: 'This moment came from a published Twitch clip.',
      })
    }

    if (tagSet.has('featured-clip')) {
      badges.push({
        kind: 'featured',
        label: 'Featured on Twitch',
        compactLabel: 'Featured',
        tooltip: 'The streamer featured this clip on their Twitch channel.',
      })
    }

    const countTag = tags.find(tag => /^community-consensus:\d+$/.test(tag))
    const creatorCount = countTag ? Number.parseInt(countTag.split(':')[1], 10) : null
    if (creatorCount !== null && creatorCount > 1) {
      badges.push({
        kind: 'consensus',
        label: `${creatorCount} creators clipped this`,
        compactLabel: `${creatorCount} creators`,
        tooltip: `${creatorCount} different Twitch creators independently clipped this moment.`,
      })
    } else if (tagSet.has('community-consensus')) {
      badges.push({
        kind: 'consensus',
        label: 'Viewer consensus',
        compactLabel: 'Viewer consensus',
        tooltip: 'Multiple Twitch clips pointed to the same moment. Reanalyze this VOD to show the exact creator count.',
      })
    }
  }

  const localSignals = sources.filter(source => source in LOCAL_SIGNAL_LABELS)
  if (localSignals.length > 0) {
    const signalLabels = joinSignalLabels(localSignals)
    const signalTitle = capitalizeFirst(signalLabels)
    badges.push({
      kind: 'local',
      label: isTwitchClip ? 'Local signals agree' : `${signalTitle} detection`,
      compactLabel: isTwitchClip
        ? 'Signals agree'
        : localSignals.length === 1 ? signalTitle : 'Local signals',
      tooltip: isTwitchClip
        ? `ClipGoblin's ${signalLabels} detection also flagged this moment.`
        : `ClipGoblin selected this moment using local ${signalLabels} detection.`,
    })
  }

  if (sourceSet.has('ai') || sourceSet.has('semantic')) {
    badges.push({
      kind: 'ai',
      label: isTwitchClip ? 'AI judge agrees' : 'AI judge pick',
      compactLabel: isTwitchClip ? 'AI agrees' : 'AI pick',
      tooltip: isTwitchClip
        ? 'The optional BYOK clip judge also selected this moment.'
        : 'The optional BYOK clip judge selected this moment.',
    })
  }

  return badges
}
