import test from 'node:test'
import assert from 'node:assert/strict'

import {
  canGenerateTimedCaptions,
  getCaptionTimelineStart,
  hasUsableSourceMedia,
} from '../src/lib/editorCaptions.ts'

test('allows subtitle generation from an imported Medal source without a VOD row', () => {
  assert.equal(canGenerateTimedCaptions({ source_media_path: 'E:\\Medal Clips\\clip.mp4' }, null), true)
})

test('allows subtitle generation from community clips and downloaded Twitch VODs', () => {
  assert.equal(canGenerateTimedCaptions({ community_clip_mp4_path: 'clip.mp4' }, null), true)
  assert.equal(canGenerateTimedCaptions({}, { local_path: 'vod.mp4' }), true)
})

test('rejects missing and blank media paths', () => {
  assert.equal(canGenerateTimedCaptions(null, null), false)
  assert.equal(canGenerateTimedCaptions({ source_media_path: '  ' }, { local_path: '' }), false)
})

test('enables export and upload for every supported local media source', () => {
  assert.equal(hasUsableSourceMedia({ source_media_path: 'E:\\Medal Clips\\clip.mp4' }, null), true)
  assert.equal(hasUsableSourceMedia({ community_clip_mp4_path: 'community.mp4' }, null), true)
  assert.equal(hasUsableSourceMedia({}, { local_path: 'twitch-vod.mp4' }), true)
  assert.equal(hasUsableSourceMedia({}, null), false)
})

test('keeps imported subtitle time tied to the original source after trimming', () => {
  assert.equal(getCaptionTimelineStart({
    source_media_path: 'E:\\Medal Clips\\clip.mp4',
    start_seconds: 4.5,
    captions_source_start: 0,
  }), 0)
  assert.equal(getCaptionTimelineStart({
    source_media_path: 'E:\\Medal Clips\\clip.mp4',
    start_seconds: 12,
    captions_source_start: 4.5,
  }), 4.5)
})

test('falls back to clip start for legacy Twitch captions', () => {
  assert.equal(getCaptionTimelineStart({ start_seconds: 196.3 }), 196.3)
})
