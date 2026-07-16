import assert from 'node:assert/strict'
import test from 'node:test'

import {
  DEFAULT_FACECAM,
  normalizeFacecamSettings,
  parseFacecamSettings,
} from '../src/lib/facecam.ts'

test('facecam settings survive their persisted JSON representation', () => {
  const settings = {
    ...DEFAULT_FACECAM,
    pipX: 11,
    pipY: 19,
    pipW: 33,
    pipH: 25,
    splitRatio: 0.7,
  }

  assert.deepEqual(parseFacecamSettings(JSON.stringify(settings)), settings)
})

test('facecam settings clamp malformed and off-canvas values', () => {
  const settings = normalizeFacecamSettings({
    pipX: 99,
    pipY: -5,
    pipW: 80,
    pipH: 20,
    splitRatio: 2,
    cropX: 0.9,
    cropY: Number.NaN,
    cropW: 0.4,
    cropH: 0,
  })

  assert.equal(settings.pipW, 45)
  assert.equal(settings.pipX, 55)
  assert.equal(settings.pipY, 0)
  assert.equal(settings.splitRatio, 0.8)
  assert.equal(settings.cropX, 0.6)
  assert.equal(settings.cropY, DEFAULT_FACECAM.cropY)
  assert.equal(settings.cropH, 0.05)
})

test('invalid persisted facecam JSON falls back to defaults', () => {
  assert.deepEqual(parseFacecamSettings('{broken'), DEFAULT_FACECAM)
  assert.deepEqual(parseFacecamSettings(null), DEFAULT_FACECAM)
})
