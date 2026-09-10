import test from 'node:test'
import assert from 'node:assert/strict'
import { playbackDuration, playbackTimeLabel } from '../src/lib/playbackDuration.ts'

test('an unloaded standalone viewer clip never displays its sentinel or VOD-relative bounds', () => {
  for (const duration of [0, NaN, Infinity, -1, Number.MAX_SAFE_INTEGER]) {
    assert.equal(playbackTimeLabel(playbackDuration(true, 5033, 5041.4, duration)), '--:--')
  }
})

test('natural duration arriving later replaces the unknown label without using VOD offsets', () => {
  assert.equal(playbackDuration(true, 5033, 5041.4, 0), null)
  assert.equal(playbackDuration(true, 5033, 5041.4, 8.4), 8.4)
  assert.equal(playbackTimeLabel(playbackDuration(true, 5033, 5041.4, 8.4)), '0:08')
  assert.equal(playbackDuration(true, 5033, 5041.4, 0), null)
})

test('ordinary trimmed VOD clips retain their real duration before media metadata loads', () => {
  assert.equal(playbackTimeLabel(playbackDuration(false, 5033, 5041.4, 0)), '0:08')
  assert.equal(playbackTimeLabel(playbackDuration(false, 5, 175, 9999)), '2:50')
  assert.equal(playbackTimeLabel(playbackDuration(false, 5, 5, 0)), '0:00')
  assert.equal(playbackTimeLabel(playbackDuration(false, 5, 3, 0)), '--:--')
})
