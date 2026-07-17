import test from 'node:test'
import assert from 'node:assert/strict'

import { filterAvailableMontageClips, montageSourceGroup, nextMontageClipId } from '../src/lib/montage.ts'
import type { Clip } from '../src/types.ts'

function clip(id: string, sourceKind: string, title: string, game: string | null = null): Clip {
  return {
    id,
    highlight_id: `highlight-${id}`,
    vod_id: `vod-${id}`,
    title,
    start_seconds: 0,
    end_seconds: 30,
    aspect_ratio: '9:16',
    crop_x: null,
    crop_y: null,
    crop_width: null,
    crop_height: null,
    captions_enabled: 0,
    captions_text: null,
    captions_position: 'bottom',
    caption_style: 'clean',
    caption_font_scale: 1,
    facecam_layout: 'none',
    render_status: 'pending',
    output_path: null,
    thumbnail_path: null,
    game,
    publish_description: null,
    publish_hashtags: null,
    cam_region_norm_override: null,
    cam_fit_mode: null,
    source_kind: sourceKind,
  }
}

test('montage source groups keep Twitch variants together', () => {
  assert.equal(montageSourceGroup(clip('vod', 'twitch_vod', 'VOD clip')), 'twitch')
  assert.equal(montageSourceGroup(clip('viewer', 'twitch_community', 'Viewer clip')), 'twitch')
  assert.equal(montageSourceGroup(clip('medal', 'medal', 'Medal clip')), 'medal')
  assert.equal(montageSourceGroup(clip('manual', 'manual', 'Local clip')), 'local')
})

test('montage filtering excludes selected clips and searches title or game', () => {
  const clips = [
    clip('one', 'twitch_vod', 'Great escape', 'Dead by Daylight'),
    clip('two', 'medal', 'Ranked win', 'Valorant'),
    clip('three', 'obs', 'Funny reaction', 'Dead by Daylight'),
  ]
  assert.deepEqual(
    filterAvailableMontageClips(clips, ['one'], 'all', '').map(item => item.id),
    ['two', 'three'],
  )
  assert.deepEqual(
    filterAvailableMontageClips(clips, [], 'medal', '').map(item => item.id),
    ['two'],
  )
  assert.deepEqual(
    filterAvailableMontageClips(clips, [], 'all', 'dead by daylight').map(item => item.id),
    ['one', 'three'],
  )
})

test('montage preview advances in order and stops after the final clip', () => {
  const sequence = ['one', 'two', 'three']
  assert.equal(nextMontageClipId(sequence, 'one'), 'two')
  assert.equal(nextMontageClipId(sequence, 'two'), 'three')
  assert.equal(nextMontageClipId(sequence, 'three'), null)
  assert.equal(nextMontageClipId(sequence, 'missing'), null)
  assert.equal(nextMontageClipId([], null), null)
})
