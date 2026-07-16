import test from 'node:test'
import assert from 'node:assert/strict'

import { EMPTY_TIKTOK_COMPLIANCE } from '../src/lib/tiktokCompliance.ts'

test('TikTok publishing defaults to Direct Post for existing workflows', () => {
  assert.equal(EMPTY_TIKTOK_COMPLIANCE.publishMode, 'direct')
})
