import { EXPORT_PRESETS } from './editTypes'

const PLATFORM_PRESET_MAP: Record<string, string> = {
  tiktok: 'tiktok',
  youtube: 'youtube',
  youtube_shorts: 'shorts',
  instagram: 'reels',
}

export function getPresetForPlatform(platform: string) {
  const presetId = PLATFORM_PRESET_MAP[platform] || 'tiktok'
  return EXPORT_PRESETS.find((preset) => preset.id === presetId) || EXPORT_PRESETS[0]
}

export type YouTubeSubFormat = 'regular' | 'shorts' | 'both'

export function getDefaultYouTubeSubFormat(
  clipDurationSec: number,
  currentAspectRatio: string,
): YouTubeSubFormat {
  if (currentAspectRatio === '9:16' && clipDurationSec <= 60) return 'shorts'
  return 'regular'
}

export function expandYouTubeSubFormat(subFormat: YouTubeSubFormat): string[] {
  if (subFormat === 'both') return ['youtube', 'youtube_shorts']
  if (subFormat === 'shorts') return ['youtube_shorts']
  return ['youtube']
}

export interface VisibilityOption {
  value: string
  label: string
  hint: string
}

export const PLATFORM_VISIBILITY: Record<
  string,
  { options: VisibilityOption[]; default: string }
> = {
  youtube: {
    default: 'unlisted',
    options: [
      { value: 'unlisted', label: 'Unlisted', hint: 'Link only' },
      { value: 'public', label: 'Public', hint: 'Visible on channel' },
      { value: 'private', label: 'Private', hint: 'Only you' },
    ],
  },
  youtube_shorts: {
    default: 'unlisted',
    options: [
      { value: 'unlisted', label: 'Unlisted', hint: 'Link only' },
      { value: 'public', label: 'Public', hint: 'Visible on channel' },
      { value: 'private', label: 'Private', hint: 'Only you' },
    ],
  },
  tiktok: {
    default: 'private',
    options: [
      { value: 'private', label: 'Draft', hint: 'Only you' },
      { value: 'friends', label: 'Friends', hint: 'Friends only' },
      { value: 'public', label: 'Public', hint: 'On your feed' },
    ],
  },
  instagram: {
    default: 'private',
    options: [
      { value: 'private', label: 'Private', hint: 'Only you' },
      { value: 'public', label: 'Public', hint: 'On your feed' },
    ],
  },
}

export function getDefaultVisibility(platform: string): string {
  return PLATFORM_VISIBILITY[platform]?.default || 'unlisted'
}

export type PlatformUploadStatus =
  | 'idle'
  | 'waiting'
  | 'exporting'
  | 'uploading'
  | 'processing'
  | 'done'
  | 'error'

export interface PlatformUploadState {
  status: PlatformUploadStatus
  progress: number
  error?: string
  videoUrl?: string
  duplicateUrl?: string
}

export function isSuccessfulUploadHandoff(status: PlatformUploadStatus | undefined): boolean {
  return status === 'done' || status === 'processing'
}
