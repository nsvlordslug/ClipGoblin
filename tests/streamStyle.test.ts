import test from 'node:test'
import assert from 'node:assert/strict'
import {
  effectiveStreamStyle,
  needsStreamStyleReanalysis,
  normalizeDetectedStreamStyle,
  normalizeStreamStyle,
  streamStyleAnalysisKey,
} from '../src/lib/streamStyle.ts'

test('Auto displays and tracks the detected stream style', () => {
  const vod = {
    analysis_status: 'completed',
    stream_style: 'auto',
    detected_stream_style: 'cozy',
    analyzed_stream_style: 'auto:cozy',
  }
  assert.equal(effectiveStreamStyle(vod), 'cozy')
  assert.equal(streamStyleAnalysisKey(vod), 'auto:cozy')
  assert.equal(needsStreamStyleReanalysis(vod), false)
})

test('a style correction waits for explicit reanalysis', () => {
  assert.equal(needsStreamStyleReanalysis({
    analysis_status: 'completed',
    stream_style: 'talking',
    detected_stream_style: 'cozy',
    analyzed_stream_style: 'auto:cozy',
  }), true)
})

test('Auto notices when corrected game metadata changes its detected style', () => {
  assert.equal(needsStreamStyleReanalysis({
    analysis_status: 'completed',
    stream_style: 'auto',
    detected_stream_style: 'action',
    analyzed_stream_style: 'auto:mixed',
  }), true)
})

test('legacy and malformed values fall back safely without inventing stale work', () => {
  assert.equal(normalizeStreamStyle('unknown'), 'auto')
  assert.equal(normalizeDetectedStreamStyle('auto'), 'mixed')
  assert.equal(needsStreamStyleReanalysis({
    analysis_status: 'completed',
    stream_style: 'cozy',
    detected_stream_style: 'cozy',
    analyzed_stream_style: null,
  }), false)
})
