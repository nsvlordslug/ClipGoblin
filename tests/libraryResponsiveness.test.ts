import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

const appStore = readFileSync(new URL('../src/stores/appStore.ts', import.meta.url), 'utf8')
const scheduleStore = readFileSync(new URL('../src/stores/scheduleStore.ts', import.meta.url), 'utf8')
const vodsPage = readFileSync(new URL('../src/pages/Vods.tsx', import.meta.url), 'utf8')
const clipsPage = readFileSync(new URL('../src/pages/Clips.tsx', import.meta.url), 'utf8')
const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8')

test('VOD first load reads the local cache before a non-blocking Twitch reconciliation', () => {
  const ensureStart = appStore.indexOf('ensureVods: async')
  const fetchStart = appStore.indexOf('fetchVods: async')
  const ensureBlock = appStore.slice(ensureStart, fetchStart)

  assert.match(ensureBlock, /get_cached_vods/)
  assert.match(ensureBlock, /void get\(\)\.fetchVods\(channelId\)/)
  assert.match(appStore, /const cachedVodsInFlight = new Map/)
  assert.match(appStore, /const remoteVodsInFlight = new Map/)
  assert.match(appStore, /shouldRefreshVods/)
})

test('ordinary VOD navigation preserves deleted-record history and cached rows', () => {
  const mountStart = vodsPage.indexOf('void ensureVods(loggedInUser.id)')
  assert.ok(mountStart >= 0)
  const nearbyMount = vodsPage.slice(Math.max(0, mountStart - 250), mountStart + 250)
  assert.doesNotMatch(nearbyMount, /restore_deleted_vods/)
  assert.match(
    vodsPage,
    /vods\.length === 0 && \(!vodsResource\.loaded \|\| vodsResource\.loading\)/,
  )

  const restoreCalls = vodsPage.match(/restore_deleted_vods/g) ?? []
  assert.equal(restoreCalls.length, 2, 'deleted VOD history should clear only in explicit restore flows')
  assert.match(vodsPage, /invalidateVods\(loggedInUser\.id\)[\s\S]*?fetchVods\(loggedInUser\.id, \{ force: true \}\)/)
})

test('Clips reuses the session metadata map and asks only for missing VOD details', () => {
  assert.match(clipsPage, /state => state\.vodDetailsById/)
  assert.match(clipsPage, /void ensureVodDetails\(vodIds\)/)
  assert.doesNotMatch(clipsPage, /useState<Record<string, Vod>>/)
  assert.doesNotMatch(clipsPage, /Promise\.all\(missing\.map\(id =>\s*invoke<Vod>\('get_vod_detail'/)
  assert.match(appStore, /const missing = requested\.filter\(vodId => !seededDetails\[vodId\]\)/)
  assert.match(appStore, /const vodDetailInFlight = new Map/)
})

test('ordinary Clips and schedule revisits are cache-aware, while mutations force refreshes', () => {
  const clipsLoadStart = clipsPage.indexOf('// ── Load data ──')
  const clipsLoadEnd = clipsPage.indexOf('const importLocalMedia', clipsLoadStart)
  const clipsLoadBlock = clipsPage.slice(clipsLoadStart, clipsLoadEnd)
  assert.match(clipsLoadBlock, /fetchClips\(\)/)
  assert.match(clipsLoadBlock, /fetchHighlights\(\)/)
  assert.doesNotMatch(clipsLoadBlock, /loadSchedules/)

  assert.match(appStore, /shouldLoadResource\(get\(\)\.clipsResource, requestAccountId, options\.force\)/)
  assert.match(appStore, /shouldLoadResource\(get\(\)\.highlightsResource, requestAccountId, options\.force\)/)
  assert.match(scheduleStore, /if \(get\(\)\.loaded && !options\.force\) return/)
  assert.match(scheduleStore, /get\(\)\.load\(\{ force: true \}\)/)
  assert.match(clipsPage, /fetchClips\(\{ force: true \}\)/)
  assert.match(clipsPage, /fetchHighlights\(undefined, \{ force: true \}\)/)
})

test('account and external-import changes invalidate or force the shared resources', () => {
  assert.match(appStore, /if \(get\(\)\.loginChecked\) return/)
  assert.match(appStore, /previousAccountId !== nextAccountId/)
  assert.match(appStore, /vodDetailsById: \{\}/)
  assert.match(appStore, /clipsResource: emptySessionResourceState\(nextAccountId\)/)
  assert.match(appStore, /highlightsResource: emptySessionResourceState\(nextAccountId\)/)
  assert.match(app, /fetchClips\(\{ force: true \}\)/)
  assert.match(app, /fetchHighlights\(undefined, \{ force: true \}\)/)
})
