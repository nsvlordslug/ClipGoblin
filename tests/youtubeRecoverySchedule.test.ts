import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { runInNewContext } from 'node:vm'
import ts from 'typescript'
import { absenceReviewArgs, YouTubeRecoveryRequestGate } from '../src/lib/youtubeRecovery.ts'
import type { YouTubeRecoveryResult } from '../src/lib/youtubeRecovery.ts'

const require = createRequire(import.meta.url)

// Run the actual store and command helpers, isolating only Tauri's IPC/event boundary.
// Each fixture gets its own store module so loadInFlight cannot leak between tests.
function loadModule(path: string, imports: Record<string, unknown>): any {
  const source = readFileSync(new URL(path, import.meta.url), 'utf8')
  const output = ts.transpileModule(source, { compilerOptions: {
    target: ts.ScriptTarget.ES2023, module: ts.ModuleKind.CommonJS,
  } }).outputText
  const module = { exports: {} }
  runInNewContext(output, {
    module, exports: module.exports, console,
    require: (specifier: string) => {
      if (Object.hasOwn(imports, specifier)) return imports[specifier]
      throw new Error(`Unexpected test dependency: ${specifier}`)
    },
  })
  return module.exports
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: Error) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

const completed: YouTubeRecoveryResult = {
  status: 'completed', video_url: 'https://youtu.be/fixture', message: 'Existing upload confirmed.',
  review_id: null, account_id: 'channel-a', aspect_ratio: '16:9',
}
const review: YouTubeRecoveryResult = {
  ...completed, status: 'review_required', video_url: null, review_id: 'review-a',
}

function fixture() {
  let rows = [{ id: 'schedule-a', clip_id: 'clip-a', platform: 'youtube', status: 'failed',
    video_url: null as string | null, job_id: 'original-claim' as string | null }]
  const recovery = deferred<YouTubeRecoveryResult>()
  const absence = deferred<void>()
  const calls: { command: string; args: any }[] = []
  let heldRead: ReturnType<typeof deferred<typeof rows>> | null = null
  const invoke = async (command: string, args?: any) => {
    calls.push({ command, args })
    if (command === 'list_scheduled_uploads') {
      if (heldRead) { const held = heldRead; heldRead = null; return held.promise }
      return structuredClone(rows)
    }
    if (command === 'recover_youtube_upload') {
      const result = await recovery.promise
      if (result.status === 'completed') rows = rows.map(row => ({ ...row, status: 'completed', video_url: result.video_url }))
      return result
    }
    if (command === 'review_youtube_upload_absent') {
      await absence.promise
      rows = rows.map(row => ({ ...row, job_id: null }))
      return
    }
    throw new Error(`Unexpected command: ${command}`)
  }
  const { useScheduleStore: store } = loadModule('../src/stores/scheduleStore.ts', {
    zustand: require('zustand'), '@tauri-apps/api/core': { invoke },
    '@tauri-apps/api/event': { listen: async () => () => {} },
  })
  const actions = loadModule('../src/lib/youtubeRecoveryActions.ts', {
    '@tauri-apps/api/core': { invoke }, '../stores/scheduleStore': { useScheduleStore: store },
  })
  return {
    store, actions, calls, recovery, absence, invoke,
    reads: () => calls.filter(call => call.command === 'list_scheduled_uploads').length,
    holdNextRead: () => { const held = deferred<typeof rows>(); heldRead = held; return { held, snapshot: structuredClone(rows) } },
  }
}

test('completed recovery refreshes an already-loaded schedule only after persistence and survives navigation', async () => {
  const f = fixture()
  await f.store.getState().load()
  const pending = f.actions.recoverYouTubeUpload('clip-a', '16:9')
  assert.equal(f.reads(), 1, 'no list reload while recovery is unconfirmed')
  assert.equal(f.store.getState().uploads[0].status, 'failed')
  f.recovery.resolve(completed)
  assert.deepEqual(await pending, completed)
  assert.equal(f.store.getState().uploads[0].status, 'completed')
  assert.equal(f.store.getState().uploads[0].video_url, completed.video_url)
  assert.equal(f.reads(), 2)
  await f.store.getState().load() // Scheduled mounts again using its ordinary cached load.
  assert.equal(f.store.getState().uploads[0].status, 'completed')
  assert.equal(f.reads(), 2)
  assert.deepEqual(JSON.parse(JSON.stringify(f.calls[1])), {
    command: 'recover_youtube_upload', args: { clipId: 'clip-a', aspectRatio: '16:9' },
  })
})

test('confirmed absence reloads the released schedule claim after the exact review command succeeds', async () => {
  const f = fixture()
  await f.store.getState().load()
  const args = absenceReviewArgs('clip-a', '16:9', review, true, 'channel-a')
  const pending = f.actions.recordYouTubeAbsenceReview(args)
  assert.equal(f.reads(), 1)
  assert.equal(f.store.getState().uploads[0].job_id, 'original-claim')
  f.absence.resolve()
  await pending
  assert.equal(f.reads(), 2)
  assert.equal(f.store.getState().uploads[0].job_id, null)
  assert.equal(f.store.getState().uploads[0].status, 'failed')
  assert.deepEqual(f.calls.map(call => call.command), [
    'list_scheduled_uploads', 'review_youtube_upload_absent', 'list_scheduled_uploads',
  ])
  assert.deepEqual(f.calls[1].args, args)
})

test('review-required and retry-later responses keep their blocked schedule without claiming completion', async () => {
  for (const status of ['review_required', 'retry_later'] as const) {
    const f = fixture()
    await f.store.getState().load()
    const pending = f.actions.recoverYouTubeUpload('clip-a', '16:9')
    f.recovery.resolve({ ...review, status })
    assert.equal((await pending).status, status)
    assert.equal(f.reads(), 1)
    assert.equal(f.store.getState().uploads[0].status, 'failed')
  }
})

test('rejected recovery and rejected absence review do not refresh or report success', async () => {
  for (const action of ['recovery', 'absence'] as const) {
    const f = fixture()
    await f.store.getState().load()
    const pending = action === 'recovery'
      ? f.actions.recoverYouTubeUpload('clip-a', '16:9')
      : f.actions.recordYouTubeAbsenceReview(absenceReviewArgs('clip-a', '16:9', review, true, 'channel-a'))
    f[action].reject(new Error('command rejected'))
    await assert.rejects(pending, /command rejected/)
    assert.equal(f.reads(), 1)
    assert.equal(f.store.getState().uploads[0].status, 'failed')
  }
})

test('leaving the initiating clip/account cancels local callbacks while refreshing the persisted global schedule', async () => {
  for (const action of ['recovery', 'absence'] as const) for (const change of ['clip', 'account', 'unmount'] as const) {
    const f = fixture()
    await f.store.getState().load()
    const gate = new YouTubeRecoveryRequestGate()
    const request = gate.begin('clip-a', '16:9', 'channel-a')
    let localCallbacks = 0
    const command = action === 'recovery'
      ? f.actions.recoverYouTubeUpload('clip-a', '16:9')
      : f.actions.recordYouTubeAbsenceReview(absenceReviewArgs('clip-a', '16:9', review, true, 'channel-a'))
    const pending = command.then(() => {
      if (gate.isCurrent(request, change === 'clip' ? 'clip-b' : 'clip-a', '16:9', change === 'account' ? 'channel-b' : 'channel-a')) localCallbacks += 1
    })
    if (change === 'unmount') gate.cancel()
    if (action === 'recovery') f.recovery.resolve(completed)
    else f.absence.resolve()
    await pending
    assert.equal(localCallbacks, 0)
    assert.equal(f.store.getState().uploads[0].status, action === 'recovery' ? 'completed' : 'failed')
    if (action === 'absence') assert.equal(f.store.getState().uploads[0].job_id, null)
    assert.equal(f.reads(), 2)
  }
})

test('an older list request in flight cannot satisfy the recovery refresh with its stale snapshot', async () => {
  const f = fixture()
  await f.store.getState().load()
  const { held, snapshot } = f.holdNextRead()
  const staleLoad = f.store.getState().load({ force: true })
  const pending = f.actions.recoverYouTubeUpload('clip-a', '16:9')
  f.recovery.resolve(completed)
  await new Promise(resolve => setImmediate(resolve))
  assert.equal(f.reads(), 2, 'refresh waits for the earlier read to settle')
  held.resolve(snapshot)
  await staleLoad
  await pending
  assert.equal(f.reads(), 3, 'a new read follows persistence and the stale read')
  assert.equal(f.store.getState().uploads[0].status, 'completed')
})

test('the regression fixture reproduces the original stale cache when recovery bypasses the refresh helper', async () => {
  const f = fixture()
  await f.store.getState().load()
  const pending = f.invoke('recover_youtube_upload', { clipId: 'clip-a', aspectRatio: '16:9' })
  f.recovery.resolve(completed)
  await pending
  await f.store.getState().load()
  assert.equal(f.store.getState().uploads[0].status, 'failed')
  assert.equal(f.reads(), 1)
})
