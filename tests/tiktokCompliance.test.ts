import test from 'node:test'
import assert from 'node:assert/strict'

import {
  EMPTY_TIKTOK_COMPLIANCE,
  visibleTikTokPrivacyOptions,
} from '../src/lib/tiktokCompliance.ts'

test('TikTok publishing defaults to Direct Post for existing workflows', () => {
  assert.equal(EMPTY_TIKTOK_COMPLIANCE.publishMode, 'direct')
})

test('TikTok publishing requires explicit privacy and interaction choices', () => {
  assert.equal(EMPTY_TIKTOK_COMPLIANCE.privacyLevel, null)
  assert.equal(EMPTY_TIKTOK_COMPLIANCE.disableComment, true)
  assert.equal(EMPTY_TIKTOK_COMPLIANCE.disableDuet, true)
  assert.equal(EMPTY_TIKTOK_COMPLIANCE.disableStitch, true)
})

test('approved Direct Post preserves all TikTok-returned audience choices', () => {
  assert.deepEqual(
    visibleTikTokPrivacyOptions(['PUBLIC_TO_EVERYONE', 'MUTUAL_FOLLOW_FRIENDS', 'SELF_ONLY']),
    ['PUBLIC_TO_EVERYONE', 'MUTUAL_FOLLOW_FRIENDS', 'SELF_ONLY'],
  )
})

test('approved Direct Post does not invent an audience when TikTok returns none', () => {
  assert.deepEqual(visibleTikTokPrivacyOptions([]), [])
})
