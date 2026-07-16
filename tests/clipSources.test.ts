import test from 'node:test'
import assert from 'node:assert/strict'
import {
  clipMatchesSourceTab,
  clipSourceTabFor,
  countClipsBySource,
} from '../src/lib/clipSources.ts'

test('Twitch VOD and community clips share one source tab', () => {
  assert.equal(clipSourceTabFor({ source_kind: 'twitch_vod' }), 'twitch')
  assert.equal(clipSourceTabFor({ source_kind: 'twitch_community' }), 'twitch')
  assert.equal(clipSourceTabFor({}), 'twitch')
})

test('external recorder and manual clips use distinct library tabs', () => {
  assert.equal(clipSourceTabFor({ source_kind: 'medal' }), 'medal')
  assert.equal(clipSourceTabFor({ source_kind: 'obs' }), 'obs')
  assert.equal(clipSourceTabFor({ source_kind: 'meld' }), 'meld')
  assert.equal(clipSourceTabFor({ source_kind: 'manual' }), 'local')
})

test('source counts and filters remain consistent', () => {
  const clips = [
    { source_kind: 'twitch_vod' },
    { source_kind: 'twitch_community' },
    { source_kind: 'medal' },
    { source_kind: 'obs' },
    { source_kind: 'meld' },
    { source_kind: 'manual' },
  ]
  assert.deepEqual(countClipsBySource(clips), {
    all: 6,
    twitch: 2,
    medal: 1,
    obs: 1,
    meld: 1,
    local: 1,
  })
  assert.deepEqual(
    clips.filter(clip => clipMatchesSourceTab(clip, 'medal')),
    [{ source_kind: 'medal' }],
  )
})
