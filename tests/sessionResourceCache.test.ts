import test from 'node:test'
import assert from 'node:assert/strict'
import type { Vod } from '../src/types.ts'
import {
  emptySessionResourceState,
  indexVods,
  reconcileVods,
  resourceIsCurrent,
  shouldLoadResource,
  shouldRefreshVods,
  VOD_BACKGROUND_REFRESH_MS,
} from '../src/lib/sessionResourceCache.ts'

function vod(id: string, patch: Partial<Vod> = {}): Vod {
  return {
    id,
    channel_id: 'channel-a',
    twitch_video_id: `twitch-${id}`,
    title: `VOD ${id}`,
    duration_seconds: 120,
    stream_date: '2026-08-31T00:00:00Z',
    thumbnail_url: '',
    download_status: 'pending',
    download_progress: 0,
    analysis_status: 'pending',
    analysis_progress: 0,
    local_path: null,
    game_name: null,
    stream_style: 'auto',
    detected_stream_style: 'mixed',
    analyzed_stream_style: null,
    cam_region_norm: null,
    ...patch,
  }
}

test('first load is required, while a valid same-account revisit reuses session data', () => {
  const empty = emptySessionResourceState('channel-a')
  assert.equal(shouldLoadResource(empty, 'channel-a'), true)

  const loaded = {
    ...empty,
    loaded: true,
    loadedAt: 1_000,
    refreshedAt: 1_000,
  }
  assert.equal(resourceIsCurrent(loaded, 'channel-a'), true)
  assert.equal(shouldLoadResource(loaded, 'channel-a'), false)
  assert.equal(shouldLoadResource(loaded, 'channel-a', true), true)
})

test('account changes invalidate otherwise loaded resources', () => {
  const loaded = {
    ...emptySessionResourceState('channel-a'),
    loaded: true,
    loadedAt: 1_000,
  }
  assert.equal(resourceIsCurrent(loaded, 'channel-b'), false)
  assert.equal(shouldLoadResource(loaded, 'channel-b'), true)
})

test('VOD refresh uses a bounded background freshness window', () => {
  const now = 10_000
  const resource = {
    ...emptySessionResourceState('channel-a'),
    loaded: true,
    loadedAt: now,
    refreshedAt: now,
  }
  assert.equal(shouldRefreshVods(resource, 'channel-a', now + 1_000), false)
  assert.equal(
    shouldRefreshVods(resource, 'channel-a', now + VOD_BACKGROUND_REFRESH_MS + 1),
    true,
  )
  assert.equal(shouldRefreshVods(resource, 'channel-b', now + 1_000), true)
})

test('VOD reconciliation preserves newer in-progress values without hiding terminal states', () => {
  const current = [vod('one', {
    download_status: 'downloading',
    download_progress: 72,
    analysis_status: 'analyzing',
    analysis_progress: 44,
  })]

  const staleIncoming = [vod('one', {
    download_status: 'downloading',
    download_progress: 30,
    analysis_status: 'analyzing',
    analysis_progress: 20,
  })]
  const reconciled = reconcileVods(current, staleIncoming)[0]
  assert.equal(reconciled.download_progress, 72)
  assert.equal(reconciled.analysis_progress, 44)

  const completed = reconcileVods(current, [vod('one', {
    download_status: 'downloaded',
    download_progress: 100,
    analysis_status: 'completed',
    analysis_progress: 100,
  })])[0]
  assert.equal(completed.download_status, 'downloaded')
  assert.equal(completed.analysis_status, 'completed')
  assert.equal(completed.analysis_progress, 100)
})

test('shared VOD metadata indexing retains prior entries and adds current rows', () => {
  const prior = { old: vod('old') }
  const indexed = indexVods(prior, [vod('new')])
  assert.equal(indexed.old.id, 'old')
  assert.equal(indexed.new.id, 'new')
  assert.notEqual(indexed, prior)
})
