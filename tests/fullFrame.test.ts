import assert from 'node:assert/strict'
import test from 'node:test'

import {
  DEFAULT_FULL_FRAME_SCALE,
  MIN_FULL_FRAME_SCALE,
  fullFrameZoomOutPercent,
  normalizeFullFrameScale,
} from '../src/lib/fullFrame.ts'

test('Full Frame zoom clamps persisted values to a modest pullback', () => {
  assert.equal(normalizeFullFrameScale(undefined), DEFAULT_FULL_FRAME_SCALE)
  assert.equal(normalizeFullFrameScale(Number.NaN), DEFAULT_FULL_FRAME_SCALE)
  assert.equal(normalizeFullFrameScale(-4), MIN_FULL_FRAME_SCALE)
  assert.equal(normalizeFullFrameScale(9), DEFAULT_FULL_FRAME_SCALE)
})

test('Full Frame zoom presents scale as an intuitive zoom-out percentage', () => {
  assert.equal(fullFrameZoomOutPercent(1), 0)
  assert.equal(fullFrameZoomOutPercent(0.9), 10)
  assert.equal(fullFrameZoomOutPercent(0.7), 30)
})
