import test from 'node:test'
import assert from 'node:assert/strict'
import {
  absenceReviewArgs, canConfirmAbsence, recoveredPlatformStates, recoveryAspect,
  recoveryFormatLabel, recoveryPlatformKeys, YouTubeRecoveryRequestGate,
} from '../src/lib/youtubeRecovery.ts'
import type { YouTubeRecoveryResult } from '../src/lib/youtubeRecovery.ts'
import type { PlatformUploadState } from '../src/lib/platformUpload.ts'

const review: YouTubeRecoveryResult = {
  status: 'review_required', video_url: null, message: 'The original upload session expired.',
  review_id: 'review-a', account_id: 'channel-a', aspect_ratio: '9:16',
}
const blocked: Record<string, PlatformUploadState> = {
  youtube: { status: 'error', progress: 0, retryBlocked: true },
  youtube_shorts: { status: 'error', progress: 0, retryBlocked: true },
  tiktok: { status: 'processing', progress: 100 },
}

test('expired-session review requires the exact format, review ID, channel and explicit absence confirmation', () => {
  assert.equal(canConfirmAbsence('clip-a', '9:16', review, false, 'channel-a'), false)
  assert.equal(canConfirmAbsence('clip-a', '9:16', review, true, null), false)
  assert.equal(canConfirmAbsence('clip-a', '9:16', review, true, 'channel-b'), false)
  assert.equal(canConfirmAbsence('clip-a', '16:9', review, true, 'channel-a'), false)
  assert.equal(canConfirmAbsence('clip-a', '9:16', { ...review, review_id: null }, true, 'channel-a'), false)
  assert.equal(canConfirmAbsence('clip-a', '9:16', { ...review, account_id: null }, true, 'channel-a'), false)
  assert.deepEqual(absenceReviewArgs('clip-a', '9:16', review, true, 'channel-a'), {
    clipId: 'clip-a', aspectRatio: '9:16', reviewId: 'review-a', targetAccountId: 'channel-a', confirmedAbsent: true,
  })
})

test('transient recovery failure never permits absence confirmation or unblocks retries', () => {
  const transient: YouTubeRecoveryResult = { ...review, status: 'retry_later', review_id: null }
  assert.equal(canConfirmAbsence('clip-a', '9:16', transient, true, 'channel-a'), false)
  assert.equal(recoveredPlatformStates(blocked, [], '9:16', transient), blocked)
  assert.equal(recoveredPlatformStates(blocked, [], '9:16', review), blocked)
})

test('successful recovery updates the matching format and leaves the other uncertain upload blocked', () => {
  const result: YouTubeRecoveryResult = { ...review, status: 'completed', video_url: 'https://www.youtube.com/watch?v=found', review_id: null }
  const next = recoveredPlatformStates(blocked, [{ aspectRatio: '16:9', message: 'pending check' }], '9:16', result)
  assert.deepEqual(next.youtube_shorts, { status: 'done', progress: 100, videoUrl: result.video_url })
  assert.equal(next.youtube.retryBlocked, true)
  assert.equal(next.tiktok, blocked.tiktok)
})

test('confirmed absence unlocks only the reviewed format without starting an upload', () => {
  const next = recoveredPlatformStates(blocked, [{ aspectRatio: '16:9', message: 'pending check' }], '9:16', null)
  assert.deepEqual(next.youtube_shorts, { status: 'idle', progress: 0 })
  assert.equal(next.youtube.retryBlocked, true)
  assert.ok(!Object.values(next).some(value => value.status === 'uploading' || value.status === 'exporting'))
})

test('legacy unknown-format records retain their exact empty key and do not claim two successful posts', () => {
  assert.equal(recoveryAspect(undefined), '')
  assert.match(recoveryFormatLabel(''), /unknown/)
  assert.deepEqual(recoveryPlatformKeys(''), ['youtube', 'youtube_shorts'])
  const legacyReview = { ...review, aspect_ratio: '' as const }
  assert.equal(absenceReviewArgs('clip-a', '', legacyReview, true, 'channel-a').aspectRatio, '')
  const next = recoveredPlatformStates(blocked, [], '', { ...legacyReview, status: 'completed' })
  assert.equal(next.youtube.status, 'done')
  assert.equal(next.youtube_shorts.status, 'idle')
  const overlapping = recoveredPlatformStates(blocked, [{ aspectRatio: '', message: 'legacy still uncertain' }], '9:16', null)
  assert.equal(overlapping.youtube_shorts.retryBlocked, true)
})

test('a recovery response cannot cross an account switch, clip switch, format switch, or unmount', () => {
  const gate = new YouTubeRecoveryRequestGate()
  assert.throws(() => gate.begin('clip-a', '9:16', ''), /Connect YouTube/)
  const request = gate.begin('clip-a', '9:16', 'channel-a')
  assert.equal(gate.isCurrent(request, 'clip-a', '9:16', 'channel-a'), true)
  assert.equal(gate.isCurrent(request, 'clip-b', '9:16', 'channel-a'), false)
  assert.equal(gate.isCurrent(request, 'clip-a', '16:9', 'channel-a'), false)
  assert.equal(gate.isCurrent(request, 'clip-a', '9:16', 'channel-b'), false)
  assert.equal(gate.isCurrent(request, 'clip-a', '9:16', null), false)
  gate.cancel()
  assert.equal(gate.isCurrent(request, 'clip-a', '9:16', 'channel-a'), false)
})

test('a late earlier check cannot replace the review returned by a newer check', async () => {
  const gate = new YouTubeRecoveryRequestGate()
  const first = gate.begin('clip-a', '9:16', 'channel-a')
  let resolveFirst!: (value: YouTubeRecoveryResult) => void
  let appliedReview: string | null = null
  const pending = new Promise<YouTubeRecoveryResult>(resolve => { resolveFirst = resolve }).then(result => {
    if (gate.isCurrent(first, 'clip-a', '9:16', 'channel-a')) appliedReview = result.review_id
  })
  const second = gate.begin('clip-a', '9:16', 'channel-a')
  if (gate.isCurrent(second, 'clip-a', '9:16', 'channel-a')) appliedReview = 'new-review'
  resolveFirst(review)
  await pending
  assert.equal(appliedReview, 'new-review')
})
