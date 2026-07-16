import test from 'node:test'
import assert from 'node:assert/strict'
import {
  buildDashboardInbox,
  getDashboardInboxTarget,
  uploadFailureNeedsReconnect,
} from '../src/lib/dashboardInbox.ts'

type InboxArgs = Parameters<typeof buildDashboardInbox>

const clip = (
  id: string,
  highlightId: string,
  renderStatus: InboxArgs[1][number]['render_status'] = 'completed',
): InboxArgs[1][number] => ({
  id,
  highlight_id: highlightId,
  vod_id: `vod-${highlightId}`,
  title: `Clip ${id}`,
  start_seconds: 10,
  end_seconds: 40,
  aspect_ratio: '9:16',
  render_status: renderStatus,
  thumbnail_path: null,
})

const highlight = (
  id: string,
  confidence: number,
  rating: InboxArgs[0][number]['review_rating'] = null,
): InboxArgs[0][number] => ({
  id,
  vod_id: `vod-${id}`,
  start_seconds: 10,
  end_seconds: 40,
  virality_score: confidence,
  confidence_score: confidence,
  event_summary: `Highlight ${id}`,
  description: '',
  transcript_snippet: '',
  review_rating: rating,
})

const upload = (
  id: string,
  clipId: string,
  status: InboxArgs[2][number]['status'],
  errorMessage: string | null = null,
): InboxArgs[2][number] => ({
  id,
  clip_id: clipId,
  platform: 'tiktok',
  scheduled_time: '2026-07-14T12:00:00.000Z',
  status,
  error_message: errorMessage,
})

test('dashboard inbox prioritizes failures, then pending reviews, then ready clips', () => {
  const result = buildDashboardInbox(
    [highlight('review', 0.8), highlight('rated', 0.7, 'good'), highlight('ready', 0.9)],
    [clip('review-clip', 'review'), clip('rated-clip', 'rated'), clip('ready-clip', 'ready'), clip('failed-clip', 'failed')],
    [upload('failure', 'failed-clip', 'failed', 'Upload timed out')],
  )

  assert.deepEqual(result.items.map(item => item.kind), ['failed', 'review', 'ready', 'ready'])
  assert.deepEqual(result.counts, { failed: 1, review: 1, ready: 2 })
  assert.equal(result.total, 4)
})

test('dashboard inbox excludes handled clips and reports full counts before the display limit', () => {
  const highlights = Array.from({ length: 7 }, (_, index) => highlight(`h${index}`, 0.8))
  const clips = highlights.map(item => clip(`c-${item.id}`, item.id))
  clips.push(clip('published', 'published-highlight'))

  const result = buildDashboardInbox(
    highlights,
    clips,
    [upload('published-upload', 'published', 'completed')],
    3,
  )

  assert.equal(result.items.length, 3)
  assert.equal(result.counts.review, 7)
  assert.equal(result.counts.ready, 0)
  assert.equal(result.total, 7)
})

test('dashboard inbox targets preserve the exact clip and task', () => {
  const result = buildDashboardInbox(
    [highlight('review', 0.8), highlight('ready', 0.9)],
    [clip('review-clip', 'review'), clip('ready-clip', 'ready'), clip('failed-clip', 'failed')],
    [upload('failure', 'failed-clip', 'failed', 'Upload timed out')],
  )

  const failed = result.items.find(item => item.kind === 'failed')!
  const review = result.items.find(item => item.kind === 'review')!
  const ready = result.items.find(item => item.kind === 'ready')!

  assert.deepEqual(getDashboardInboxTarget(failed), {
    pathname: '/scheduled',
    state: { focusUploadId: 'failure', openReschedule: true },
  })
  assert.deepEqual(getDashboardInboxTarget(review), {
    pathname: '/clips',
    state: { focusClipId: 'review-clip', openReview: true },
  })
  assert.deepEqual(getDashboardInboxTarget(ready), {
    pathname: '/editor/ready-clip',
    state: { workspace: 'publish' },
  })
})

test('authentication failures route to account recovery', () => {
  assert.equal(uploadFailureNeedsReconnect('OAuth token expired'), true)
  assert.equal(uploadFailureNeedsReconnect('Network request timed out'), false)

  const result = buildDashboardInbox(
    [],
    [clip('failed-clip', 'failed')],
    [upload('failure', 'failed-clip', 'failed', 'OAuth token expired')],
  )

  assert.deepEqual(getDashboardInboxTarget(result.items[0]), {
    pathname: '/settings',
    state: { section: 'account' },
  })
})
