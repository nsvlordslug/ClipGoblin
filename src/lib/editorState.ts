export function initialPublishMetadata(clip: {
  title: string
  publish_description?: string | null
  publish_hashtags?: string | null
}) {
  return {
    title: clip.title,
    description: clip.publish_description || '',
    hashtags: clip.publish_hashtags ? clip.publish_hashtags.split(',').filter(Boolean) : [],
  }
}

/** A render from before hydration must never enqueue a later persistence task. */
export function scheduleHydratedEditorTask(
  hydrated: boolean,
  isCurrent: () => boolean,
  task: () => void,
  delayMs: number,
): () => void {
  if (!hydrated) return () => {}
  const timer = setTimeout(() => { if (isCurrent()) task() }, delayMs)
  return () => clearTimeout(timer)
}

/** Save the displayed range before a backend operation that reads the clip row. */
export async function runAfterEditorSave<T>(
  save: () => Promise<void>,
  run: () => Promise<T>,
  isCurrent: () => boolean,
): Promise<T | undefined> {
  if (!isCurrent()) return undefined
  await save()
  if (!isCurrent()) return undefined
  const result = await run()
  return isCurrent() ? result : undefined
}

export function editorTrimError(start: number, end: number, duration: number | null): string | null {
  if (!Number.isFinite(start) || !Number.isFinite(end) || start < 0 || end <= start) {
    return 'Trim start must be at least 0 and before trim end.'
  }
  if (duration !== null && (!Number.isFinite(duration) || duration <= 0 || end > duration)) {
    return 'Trim end must stay within the source video.'
  }
  return null
}

export function clampEditorTrim(start: number, end: number, duration: number): [number, number] {
  const minimumLength = Math.min(0.1, duration)
  const boundedEnd = Math.max(minimumLength, Math.min(duration, end))
  return [Math.max(0, Math.min(start, boundedEnd - minimumLength)), boundedEnd]
}

export function canMarkEditorExportComplete(
  currentClipId: string | null,
  exportedClipId: string,
  currentSnapshotKey: string,
  exportedSnapshotKey: string,
  renderStatus: string,
): boolean {
  return currentClipId === exportedClipId
    && currentSnapshotKey === exportedSnapshotKey
    && renderStatus === 'completed'
}
