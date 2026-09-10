import type { PlatformUploadState } from './platformUpload'

export type YouTubeRecoveryAspect = '9:16' | '16:9' | ''

export interface YouTubeRecoveryResult {
  status: 'completed' | 'review_required' | 'retry_later'
  video_url: string | null
  message: string
  review_id: string | null
  account_id: string | null
  original_title?: string | null
  aspect_ratio: YouTubeRecoveryAspect
}

export interface UncertainYouTubeUpload {
  aspectRatio: YouTubeRecoveryAspect
  message: string
}

export function recoveryAspect(value: string | null | undefined): YouTubeRecoveryAspect {
  return value === '9:16' || value === '16:9' ? value : ''
}

export function recoveryFormatLabel(aspectRatio: YouTubeRecoveryAspect): string {
  return aspectRatio === '9:16' ? 'YouTube Shorts (9:16)'
    : aspectRatio === '16:9' ? 'YouTube video (16:9)' : 'YouTube video (original format unknown)'
}

export function recoveryPlatformKeys(aspectRatio: YouTubeRecoveryAspect): string[] {
  return aspectRatio === '9:16' ? ['youtube_shorts']
    : aspectRatio === '16:9' ? ['youtube'] : ['youtube', 'youtube_shorts']
}

export function recoveredPlatformStates(
  states: Record<string, PlatformUploadState>,
  remainingUncertain: UncertainYouTubeUpload[],
  aspectRatio: YouTubeRecoveryAspect,
  completed: YouTubeRecoveryResult | null,
): Record<string, PlatformUploadState> {
  if (completed && completed.status !== 'completed') return states
  const blocked = new Set(remainingUncertain.flatMap(upload => recoveryPlatformKeys(upload.aspectRatio)))
  const next = { ...states }
  for (const key of recoveryPlatformKeys(aspectRatio)) {
    if (blocked.has(key)) continue
    // One recovered legacy video does not prove that both output formats were sent.
    next[key] = completed && !(aspectRatio === '' && key === 'youtube_shorts')
      ? { status: 'done', progress: 100, videoUrl: completed.video_url || undefined }
      : { status: 'idle', progress: 0 }
  }
  return next
}

interface RecoveryRequest {
  generation: number
  clipId: string
  aspectRatio: YouTubeRecoveryAspect
  accountId: string
}

/** Tokens bind a recovery response to one mounted clip, format, and account. */
export class YouTubeRecoveryRequestGate {
  private generation = 0

  begin(clipId: string, aspectRatio: YouTubeRecoveryAspect, accountId: string): RecoveryRequest {
    if (!accountId.trim()) throw new Error('Connect YouTube before checking this upload.')
    return { generation: ++this.generation, clipId, aspectRatio, accountId }
  }

  isCurrent(request: RecoveryRequest, clipId: string, aspectRatio: YouTubeRecoveryAspect, accountId: string | null): boolean {
    return request.generation === this.generation && request.clipId === clipId
      && request.aspectRatio === aspectRatio && request.accountId === accountId
  }

  cancel(): void { this.generation += 1 }
}

export function absenceReviewArgs(
  clipId: string,
  aspectRatio: YouTubeRecoveryAspect,
  review: YouTubeRecoveryResult | null,
  confirmedAbsent: boolean,
  connectedAccountId: string | null,
) {
  if (!confirmedAbsent) throw new Error('Confirm that you checked the destination channel first.')
  if (!clipId.trim() || review?.status !== 'review_required' || !review.review_id?.trim() || review.aspect_ratio !== aspectRatio) {
    throw new Error('Check this upload again before allowing a new upload.')
  }
  if (!review.account_id || !connectedAccountId || review.account_id !== connectedAccountId) {
    throw new Error('Connect the channel shown for this review before confirming it.')
  }
  return {
    clipId, aspectRatio, reviewId: review.review_id,
    targetAccountId: review.account_id, confirmedAbsent: true,
  }
}

export function canConfirmAbsence(
  clipId: string,
  aspectRatio: YouTubeRecoveryAspect,
  review: YouTubeRecoveryResult | null,
  confirmedAbsent: boolean,
  connectedAccountId: string | null,
): boolean {
  try { absenceReviewArgs(clipId, aspectRatio, review, confirmedAbsent, connectedAccountId); return true }
  catch { return false }
}
