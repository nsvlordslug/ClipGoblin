// ── Subtitle segment utilities ──
// Parse, edit, and serialize SRT subtitle data.

export interface SubtitleSegment {
  id: string
  index: number      // SRT index (1-based)
  text: string
  startTime: number  // seconds (relative to original clip start)
  endTime: number
}

/** Parse raw SRT text into editable segments. */
export function parseSrt(srt: string): SubtitleSegment[] {
  if (!srt || !srt.includes('-->')) return []

  const blocks = srt.trim().split(/\n\n+/)
  const segments: SubtitleSegment[] = []

  for (const block of blocks) {
    const lines = block.trim().split('\n')
    if (lines.length < 3) continue

    const index = parseInt(lines[0], 10)
    if (isNaN(index)) continue

    const timeMatch = lines[1].match(/(\d{2}:\d{2}:\d{2},\d{3})\s*-->\s*(\d{2}:\d{2}:\d{2},\d{3})/)
    if (!timeMatch) continue

    const text = lines.slice(2).join('\n').trim()
    if (!text) continue

    const startTime = parseSrtTime(timeMatch[1])
    segments.push({
      id: `sub-${index}-${startTime.toFixed(3)}`,
      index,
      text,
      startTime,
      endTime: parseSrtTime(timeMatch[2]),
    })
  }

  return segments
}

/** Serialize segments back to SRT format. */
export function serializeSrt(segments: SubtitleSegment[]): string {
  return segments
    .map((s, i) => {
      const idx = i + 1
      return `${idx}\n${formatSrtTime(s.startTime)} --> ${formatSrtTime(s.endTime)}\n${s.text}`
    })
    .join('\n\n')
}

/** Find the active segment at a given time. */
export function findActiveSegment(
  segments: SubtitleSegment[],
  time: number,
): SubtitleSegment | null {
  // Exact match
  for (const s of segments) {
    if (time >= s.startTime - 0.05 && time <= s.endTime + 0.05) return s
  }
  // Nearest within 1s
  let best: SubtitleSegment | null = null
  let bestDist = 1.0
  for (const s of segments) {
    const dist = Math.min(Math.abs(s.startTime - time), Math.abs(s.endTime - time))
    if (dist < bestDist) { bestDist = dist; best = s }
  }
  return best
}

/** Filter segments to only those within the trim window. */
export function filterByTrimBounds(
  segments: SubtitleSegment[],
  trimStart: number,
  trimEnd: number,
  originalStart: number,
): SubtitleSegment[] {
  // SRT times are relative to originalStart (0 = originalStart in the source)
  // Trim times are absolute in the source video
  // Convert trim to SRT-relative: srtTrimStart = trimStart - originalStart
  const srtTrimStart = trimStart - originalStart
  const srtTrimEnd = trimEnd - originalStart

  return segments.filter(s => {
    // Keep if any part of the segment overlaps the trim window
    return s.endTime > srtTrimStart && s.startTime < srtTrimEnd
  })
}

function parseSrtTime(ts: string): number {
  const [h, m, rest] = ts.split(':')
  const [s, ms] = rest.split(',')
  return parseInt(h) * 3600 + parseInt(m) * 60 + parseInt(s) + parseInt(ms) / 1000
}

function formatSrtTime(seconds: number): string {
  seconds = Math.max(0, seconds)
  const h = Math.floor(seconds / 3600)
  const m = Math.floor((seconds % 3600) / 60)
  const s = Math.floor(seconds % 60)
  const ms = Math.floor((seconds % 1) * 1000)
  return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')},${String(ms).padStart(3, '0')}`
}
