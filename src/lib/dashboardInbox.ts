import type { Clip, Highlight, ScheduledUpload } from '../types'

type InboxHighlight = Pick<
  Highlight,
  | 'id'
  | 'vod_id'
  | 'start_seconds'
  | 'end_seconds'
  | 'virality_score'
  | 'confidence_score'
  | 'event_summary'
  | 'description'
  | 'transcript_snippet'
  | 'review_rating'
>

type InboxClip = Pick<
  Clip,
  | 'id'
  | 'highlight_id'
  | 'vod_id'
  | 'title'
  | 'start_seconds'
  | 'end_seconds'
  | 'aspect_ratio'
  | 'render_status'
  | 'thumbnail_path'
>

type InboxUpload = Pick<
  ScheduledUpload,
  | 'id'
  | 'clip_id'
  | 'platform'
  | 'scheduled_time'
  | 'status'
  | 'error_message'
>

export type DashboardInboxItem =
  | {
      kind: 'failed'
      id: string
      upload: InboxUpload
      clip?: InboxClip
      needsReconnect: boolean
    }
  | {
      kind: 'review'
      id: string
      highlight: InboxHighlight
      clip?: InboxClip
      confidence: number
    }
  | {
      kind: 'ready'
      id: string
      clip: InboxClip
    }

export interface DashboardInbox {
  items: DashboardInboxItem[]
  counts: {
    failed: number
    review: number
    ready: number
  }
  total: number
}

export interface DashboardInboxTarget {
  pathname: string
  state?: Record<string, unknown>
}

export function getDashboardConfidence(highlight: InboxHighlight): number {
  if (highlight.confidence_score != null) return highlight.confidence_score

  const normalized = Math.max(0, Math.min(highlight.virality_score * 0.85 - 0.10, 0.99))
  const anchors: [number, number][] = [
    [0.00, 0.00], [0.25, 0.25], [0.40, 0.55], [0.50, 0.65],
    [0.60, 0.77], [0.70, 0.84], [0.80, 0.89], [0.90, 0.93],
  ]
  if (normalized >= 0.90) return Math.min(0.93 + (normalized - 0.90) * 0.20, 0.95)
  for (let index = 1; index < anchors.length; index += 1) {
    if (normalized <= anchors[index][0]) {
      const [x0, y0] = anchors[index - 1]
      const [x1, y1] = anchors[index]
      return y0 + ((normalized - x0) / (x1 - x0)) * (y1 - y0)
    }
  }
  return 0.95
}

export function uploadFailureNeedsReconnect(message: string | null): boolean {
  if (!message) return false
  return /\b(auth|oauth|token|expired|unauthori[sz]ed|401)\b/i.test(message)
}

export function buildDashboardInbox(
  highlights: InboxHighlight[],
  clips: InboxClip[],
  uploads: InboxUpload[],
  limit = 5,
): DashboardInbox {
  const clipByHighlight = new Map(clips.map(clip => [clip.highlight_id, clip]))
  const clipById = new Map(clips.map(clip => [clip.id, clip]))

  const failed = uploads
    .filter(upload => upload.status === 'failed')
    .sort((a, b) => new Date(b.scheduled_time).getTime() - new Date(a.scheduled_time).getTime())

  const review = highlights
    .filter(highlight => highlight.review_rating == null && getDashboardConfidence(highlight) < 0.85)
    .sort((a, b) => getDashboardConfidence(b) - getDashboardConfidence(a))

  const handledClipIds = new Set(
    uploads
      .filter(upload => upload.status !== 'cancelled')
      .map(upload => upload.clip_id),
  )
  const reviewHighlightIds = new Set(review.map(highlight => highlight.id))
  const ready = clips.filter(clip => (
    clip.render_status === 'completed'
    && !handledClipIds.has(clip.id)
    && !reviewHighlightIds.has(clip.highlight_id)
  ))

  const items: DashboardInboxItem[] = [
    ...failed.map(upload => ({
      kind: 'failed' as const,
      id: `failed:${upload.id}`,
      upload,
      clip: clipById.get(upload.clip_id),
      needsReconnect: uploadFailureNeedsReconnect(upload.error_message),
    })),
    ...review.map(highlight => ({
      kind: 'review' as const,
      id: `review:${highlight.id}`,
      highlight,
      clip: clipByHighlight.get(highlight.id),
      confidence: getDashboardConfidence(highlight),
    })),
    ...ready.map(clip => ({
      kind: 'ready' as const,
      id: `ready:${clip.id}`,
      clip,
    })),
  ].slice(0, Math.max(0, limit))

  const counts = {
    failed: failed.length,
    review: review.length,
    ready: ready.length,
  }

  return {
    items,
    counts,
    total: counts.failed + counts.review + counts.ready,
  }
}

export function getDashboardInboxTarget(item: DashboardInboxItem): DashboardInboxTarget {
  if (item.kind === 'review') {
    return item.clip
      ? {
          pathname: '/clips',
          state: { focusClipId: item.clip.id, openReview: true },
        }
      : {
          pathname: '/clips',
          state: { focusVodId: item.highlight.vod_id },
        }
  }

  if (item.kind === 'ready') {
    return {
      pathname: `/editor/${item.clip.id}`,
      state: { workspace: 'publish' },
    }
  }

  if (item.needsReconnect) {
    return { pathname: '/settings', state: { section: 'account' } }
  }

  return {
    pathname: '/scheduled',
    state: { focusUploadId: item.upload.id, openReschedule: true },
  }
}
