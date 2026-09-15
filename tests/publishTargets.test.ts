import test from 'node:test'
import assert from 'node:assert/strict'
import { captureUploadTargets, isUncertainUploadError, missingUploadTargets, uploadAdapterPlatform, uploadTargetFields } from '../src/lib/publishTargets.ts'
import { artifactUploadFields } from '../src/lib/exportArtifacts.ts'

test('Both formats keep distinct artifacts and the account confirmed before export', () => {
  const accounts = { youtube: { account_id: 'channel-a' } }
  const targets = captureUploadTargets(['youtube', 'youtube_shorts'], accounts)
  accounts.youtube.account_id = 'channel-b'
  const payloads = ['16:9', '9:16'].map(aspectRatio => ({
    clip_id: 'same-clip', force: false,
    ...uploadTargetFields(targets.youtube),
    ...artifactUploadFields({ path: `${aspectRatio}.mp4`, revision: aspectRatio, aspectRatio, width: 1920, height: 1080 }),
  }))
  assert.deepEqual(payloads.map(payload => payload.target_account_id), ['channel-a', 'channel-a'])
  assert.deepEqual(payloads.map(payload => payload.artifact_aspect_ratio), ['16:9', '9:16'])
  assert.ok(payloads.every(payload => payload.clip_id === 'same-clip' && payload.force === false))
  assert.equal(uploadAdapterPlatform('youtube_shorts'), 'youtube')
})

test('missing accounts cannot silently bind an upload or schedule to a later connection', () => {
  const targets = captureUploadTargets(['youtube', 'tiktok'], { tiktok: { account_id: 'tiktok-a' } })
  assert.throws(() => uploadTargetFields(targets.youtube), /Connect/)
  assert.deepEqual(uploadTargetFields(targets.tiktok), { target_account_id: 'tiktok-a' })
})

test('missing upload targets are detected before batch export or scheduling', () => {
  assert.deepEqual(
    missingUploadTargets(['youtube', 'tiktok'], { youtube: { account_id: 'channel-a' }, tiktok: null }),
    ['tiktok'],
  )
  assert.deepEqual(missingUploadTargets(['youtube'], { youtube: { account_id: 'channel-a' } }), [])
})

test('uncertain outcomes require account inspection rather than the batch retry action', () => {
  assert.equal(isUncertainUploadError(new Error('Upload outcome is uncertain. Check YouTube Studio; retries are blocked.')), true)
  assert.equal(isUncertainUploadError('API error: Upload outcome is uncertain.'), true)
  assert.equal(isUncertainUploadError('Could not connect before sending the video'), false)
})
