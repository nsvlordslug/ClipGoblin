interface EditorMediaClip {
  source_media_path?: string | null
  community_clip_mp4_path?: string | null
  start_seconds?: number
  captions_source_start?: number | null
}

interface EditorMediaVod {
  local_path?: string | null
}

function hasPath(value: string | null | undefined): boolean {
  return Boolean(value?.trim())
}

export function hasUsableSourceMedia(
  clip: EditorMediaClip | null | undefined,
  vod: EditorMediaVod | null | undefined,
): boolean {
  return hasPath(clip?.source_media_path)
    || hasPath(clip?.community_clip_mp4_path)
    || hasPath(vod?.local_path)
}

export function canGenerateTimedCaptions(
  clip: EditorMediaClip | null | undefined,
  vod: EditorMediaVod | null | undefined,
): boolean {
  return hasUsableSourceMedia(clip, vod)
}

export function getCaptionTimelineStart(
  clip: EditorMediaClip | null | undefined,
): number {
  const explicit = clip?.captions_source_start
  if (typeof explicit === 'number' && Number.isFinite(explicit)) {
    return Math.max(0, explicit)
  }
  if (hasPath(clip?.source_media_path) || hasPath(clip?.community_clip_mp4_path)) {
    return 0
  }
  const clipStart = clip?.start_seconds
  return typeof clipStart === 'number' && Number.isFinite(clipStart)
    ? Math.max(0, clipStart)
    : 0
}
