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

test('pending production review exposes only TikTok private posting', () => {
  assert.deepEqual(
    visibleTikTokPrivacyOptions(
      ['PUBLIC_TO_EVERYONE', 'MUTUAL_FOLLOW_FRIENDS', 'SELF_ONLY'],
      true,
    ),
    ['SELF_ONLY'],
  )
})

test('approved production exposes every privacy option returned by TikTok', () => {
  const options = ['PUBLIC_TO_EVERYONE', 'SELF_ONLY']
  assert.deepEqual(visibleTikTokPrivacyOptions(options, false), options)
})

test('pending review preserves TikTok options when private posting is unavailable', () => {
  const options = ['PUBLIC_TO_EVERYONE']
  assert.deepEqual(visibleTikTokPrivacyOptions(options, true), options)
})
