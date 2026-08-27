// ── Subtitle segment utilities ──
// Parse, edit, and serialize SRT subtitle data.

export interface SubtitleSegment {
  id: string
  index: number      // SRT index (1-based)
  text: string
  startTime: number  // seconds (relative to original clip start)
  endTime: number
}

export const FRAME_SAFE_CAPTION_SECONDS = 0.04

function hasGeneratedCaptionTiming(provenance: string | null | undefined): boolean {
  return provenance === 'aligned' || provenance === 'analysis-draft'
}

/**
 * Derive a playback/export-safe timeline without rewriting saved SRT data.
 * Only machine-generated cues shorter than one representable video frame are
 * expanded; the following boundary moves just enough to keep word order and
 * prevent overlap. Manually edited timing is returned untouched.
 */
export function frameSafeSubtitleSegments(
  segments: SubtitleSegment[],
  provenance: string | null | undefined,
): SubtitleSegment[] {
  if (!hasGeneratedCaptionTiming(provenance)
    || !segments.some(segment => segment.endTime - segment.startTime < FRAME_SAFE_CAPTION_SECONDS)) {
    return segments
  }

  const adjusted = segments.map(segment => ({ ...segment }))
  for (let index = 0; index < adjusted.length; index += 1) {
    const cue = adjusted[index]
    if (cue.endTime - cue.startTime >= FRAME_SAFE_CAPTION_SECONDS) continue

    cue.endTime = cue.startTime + FRAME_SAFE_CAPTION_SECONDS
    const next = adjusted[index + 1]
    if (next && next.startTime < cue.endTime) {
      next.startTime = cue.endTime
    }
  }
  return adjusted
}

/** Parse raw SRT text into editable segments. */
export function parseSrt(srt: string): SubtitleSegment[] {
  if (!srt || !srt.includes('-->')) return []

  const blocks = srt.replace(/\r\n?/g, '\n').trim().split(/\n\n+/)
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

  return normalizeSubtitleSegments(segments)
}

/** Serialize segments back to SRT format. */
export function serializeSrt(segments: SubtitleSegment[]): string {
  return normalizeSubtitleSegments(segments)
    .map((s, i) => {
      const idx = i + 1
      return `${idx}\n${formatSrtTime(s.startTime)} --> ${formatSrtTime(s.endTime)}\n${s.text}`
    })
    .join('\n\n')
}

function subtitleTokenKey(text: string): string {
  return text.trim().toLocaleLowerCase().replace(/[^\p{L}\p{N}]+/gu, '')
}

/**
 * Establish one global cue timeline for editor preview, persistence, and export.
 * Duplicate overlap-window words are removed and no cue can extend into the next.
 */
export function normalizeSubtitleSegments(segments: SubtitleSegment[]): SubtitleSegment[] {
  const ordered = segments
    .filter(segment => Number.isFinite(segment.startTime)
      && Number.isFinite(segment.endTime)
      && segment.endTime > segment.startTime
      && segment.text.trim())
    .map(segment => ({
      ...segment,
      text: segment.text.trim(),
      startTime: Math.max(0, Math.round(segment.startTime * 1000) / 1000),
      endTime: Math.max(0, Math.round(segment.endTime * 1000) / 1000),
    }))
    .sort((left, right) => left.startTime - right.startTime || left.endTime - right.endTime)

  const deduplicated: SubtitleSegment[] = []
  for (const segment of ordered) {
    const previous = deduplicated.at(-1)
    const duplicate = previous
      && subtitleTokenKey(previous.text) === subtitleTokenKey(segment.text)
      && Math.abs(segment.startTime - previous.startTime) <= 0.18
      && segment.startTime < previous.endTime + 0.04
    if (!duplicate) deduplicated.push(segment)
  }

  return deduplicated.flatMap((segment, index) => {
    const nextStart = deduplicated[index + 1]?.startTime ?? Number.POSITIVE_INFINITY
    const endTime = Math.min(segment.endTime, nextStart)
    return endTime > segment.startTime
      ? [{ ...segment, index: index + 1, endTime }]
      : []
  })
}

/** Find the active segment at a given time. */
export function findActiveSegment(
  segments: SubtitleSegment[],
  time: number,
): SubtitleSegment | null {
  for (const s of segments) {
    if (time >= s.startTime && time < s.endTime) return s
  }
  return null
}

/** Shift every cue by the same amount while preserving durations and gaps. */
export function shiftSubtitleSegments(
  segments: SubtitleSegment[],
  deltaSeconds: number,
): SubtitleSegment[] {
  if (segments.length === 0 || !Number.isFinite(deltaSeconds) || deltaSeconds === 0) {
    return segments
  }

  const earliestStart = Math.min(...segments.map(segment => segment.startTime))
  const appliedDelta = Math.max(deltaSeconds, -earliestStart)
  const toMilliseconds = (seconds: number) => Math.round(seconds * 1000) / 1000

  return segments.map(segment => ({
    ...segment,
    startTime: toMilliseconds(segment.startTime + appliedDelta),
    endTime: toMilliseconds(segment.endTime + appliedDelta),
  }))
}

function visibleWordDuration(word: string): number {
  const characterCount = Math.max(1, word.replace(/[^\p{L}\p{N}]/gu, '').length)
  return Math.min(1.2, Math.max(0.475, 0.4 + characterCount * 0.075))
}

/**
 * Keep one visible word per cue. New transcriptions already contain exact
 * word-level SRT timing; the proportional path keeps older grouped SRT files
 * usable until the user regenerates them with current Whisper timing.
 */
export function splitSubtitleSegmentsByWord(segments: SubtitleSegment[]): SubtitleSegment[] {
  const split: SubtitleSegment[] = []

  for (const segment of segments) {
    const words = segment.text.trim().split(/\s+/).filter(Boolean)
    const duration = segment.endTime - segment.startTime
    if (words.length === 0 || duration <= 0) continue
    if (words.length === 1) {
      split.push({
        ...segment,
        endTime: Math.min(segment.endTime, segment.startTime + visibleWordDuration(words[0])),
      })
      continue
    }

    const weights = words.map(word => Math.max(2, word.replace(/[^\p{L}\p{N}]/gu, '').length))
    const totalWeight = weights.reduce((sum, weight) => sum + weight, 0)
    let elapsedWeight = 0

    for (let i = 0; i < words.length; i += 1) {
      const wordStart = segment.startTime + duration * (elapsedWeight / totalWeight)
      elapsedWeight += weights[i]
      const weightedEnd = segment.startTime + duration * (elapsedWeight / totalWeight)
      const wordEnd = Math.min(weightedEnd, wordStart + visibleWordDuration(words[i]))
      if (wordEnd <= wordStart) continue

      split.push({
        ...segment,
        id: `${segment.id}-word-${i}`,
        text: words[i],
        startTime: wordStart,
        endTime: wordEnd,
      })
    }
  }

  return normalizeSubtitleSegments(split)
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
  const totalMs = Math.round(Math.max(0, seconds) * 1000)
  const h = Math.floor(totalMs / 3_600_000)
  const m = Math.floor((totalMs % 3_600_000) / 60_000)
  const s = Math.floor((totalMs % 60_000) / 1000)
  const ms = totalMs % 1000
  return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')},${String(ms).padStart(3, '0')}`
}
