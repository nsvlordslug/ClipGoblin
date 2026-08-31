export type XHandoffPlatform = 'youtube' | 'youtube_shorts' | 'tiktok'

export const MAX_X_SUGGESTED_TEXT_LENGTH = 240
export const MANUAL_SHARE_UNAVAILABLE_MESSAGE = 'Available after TikTok or YouTube provides a verified public link. ClipGoblin prepares a manual share; you review and post it yourself.'

const PLATFORM_LABELS: Record<XHandoffPlatform, string> = {
  youtube: 'YouTube',
  youtube_shorts: 'YouTube Shorts',
  tiktok: 'TikTok',
}

function isPlatformHost(hostname: string, root: string): boolean {
  return hostname === root || hostname.endsWith(`.${root}`)
}

function normalizePublishedUrl(platform: XHandoffPlatform, value: string): string | null {
  try {
    const parsed = new URL(value)
    if (parsed.protocol !== 'https:' || parsed.username || parsed.password) return null

    const hostname = parsed.hostname.toLowerCase()
    const supportedHost = platform === 'tiktok'
      ? isPlatformHost(hostname, 'tiktok.com')
      : isPlatformHost(hostname, 'youtube.com') || hostname === 'youtu.be'

    return supportedHost ? parsed.toString() : null
  } catch {
    return null
  }
}

function truncateText(value: string, maxLength: number): string {
  const characters = Array.from(value)
  if (characters.length <= maxLength) return value
  if (maxLength <= 3) return '.'.repeat(Math.max(0, maxLength))
  return `${characters.slice(0, maxLength - 3).join('').trimEnd()}...`
}

export function isXHandoffPlatform(platform: string): platform is XHandoffPlatform {
  return platform === 'youtube' || platform === 'youtube_shorts' || platform === 'tiktok'
}

export function buildSuggestedXCaption(platform: XHandoffPlatform, clipTitle: string): string {
  const platformLabel = PLATFORM_LABELS[platform]
  const title = clipTitle.replace(/\s+/g, ' ').trim() || 'New ClipGoblin clip'
  const suffix = `Watch on ${platformLabel}:`
  const titleLimit = MAX_X_SUGGESTED_TEXT_LENGTH - suffix.length - 2
  return `${truncateText(title, titleLimit)}\n\n${suffix}`
}

export function buildXComposeUrl(text: string, publishedUrl: string): string {
  const composeUrl = new URL('https://x.com/intent/tweet')
  const normalizedText = text.trim()
  if (normalizedText) composeUrl.searchParams.set('text', normalizedText)
  composeUrl.searchParams.set('url', publishedUrl)
  return composeUrl.toString()
}

export function buildFacebookShareUrl(publishedUrl: string): string {
  const shareUrl = new URL('https://www.facebook.com/sharer/sharer.php')
  shareUrl.searchParams.set('u', publishedUrl)
  return shareUrl.toString()
}

export function buildThreadsComposeUrl(text: string, publishedUrl: string): string {
  const composeUrl = new URL('https://www.threads.com/intent/post')
  const normalizedText = text.trim()
  if (normalizedText) composeUrl.searchParams.set('text', normalizedText)
  composeUrl.searchParams.set('url', publishedUrl)
  return composeUrl.toString()
}

export interface XHandoff {
  platform: XHandoffPlatform
  platformLabel: string
  publishedUrl: string
  suggestedCaption: string
}

export function createXHandoff(
  platform: string,
  publishedUrl: string,
  clipTitle: string,
): XHandoff | null {
  if (!isXHandoffPlatform(platform)) return null
  const normalizedUrl = normalizePublishedUrl(platform, publishedUrl)
  if (!normalizedUrl) return null

  return {
    platform,
    platformLabel: PLATFORM_LABELS[platform],
    publishedUrl: normalizedUrl,
    suggestedCaption: buildSuggestedXCaption(platform, clipTitle),
  }
}

export function canOfferXHandoff(platform: string, publishedUrl?: string | null): boolean {
  return Boolean(publishedUrl && createXHandoff(platform, publishedUrl, 'Clip'))
}
