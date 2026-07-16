import assert from 'node:assert/strict'
import test from 'node:test'

import {
  DEFAULT_CONTEXT_BLUR_STRENGTH,
  brandingAssetName,
  contextBlurPixels,
  contextVideoPositionLabel,
  normalizeContextBlurStrength,
  normalizeContextVideoY,
} from '../src/lib/contextFit.ts'

test('Context Fit values clamp malformed and out-of-range persisted data', () => {
  assert.equal(normalizeContextBlurStrength(undefined), DEFAULT_CONTEXT_BLUR_STRENGTH)
  assert.equal(normalizeContextBlurStrength(-4), 0)
  assert.equal(normalizeContextBlurStrength(9), 1)
  assert.equal(normalizeContextVideoY(Number.NaN), 0.5)
})

test('default Context Fit blur is intentionally gentle', () => {
  assert.ok(contextBlurPixels(DEFAULT_CONTEXT_BLUR_STRENGTH) < 5)
  assert.ok(contextBlurPixels(1) <= 12)
})

test('Context Fit placement and branding labels remain human-readable', () => {
  assert.equal(contextVideoPositionLabel(0), 'Top')
  assert.equal(contextVideoPositionLabel(0.5), 'Center')
  assert.equal(contextVideoPositionLabel(1), 'Bottom')
  assert.equal(brandingAssetName('C:\\Branding\\my-loop.gif'), 'my-loop.gif')
})
