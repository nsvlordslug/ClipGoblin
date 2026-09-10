/** Unknown standalone-file duration must not become a display or seek bound. */
export function playbackDuration(fullFile: boolean, clipStart: number, clipEnd: number, naturalDuration: number): number | null {
  const duration = fullFile ? naturalDuration : clipEnd - clipStart
  if (!Number.isFinite(duration) || duration < 0 || duration >= Number.MAX_SAFE_INTEGER) return null
  return fullFile && duration === 0 ? null : duration
}

export function playbackTimeLabel(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds) || seconds < 0 || seconds >= Number.MAX_SAFE_INTEGER) return '--:--'
  return `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, '0')}`
}
