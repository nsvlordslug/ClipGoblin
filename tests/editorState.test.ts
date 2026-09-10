import test from 'node:test'
import assert from 'node:assert/strict'
import {
  canMarkEditorExportComplete, clampEditorTrim, editorTrimError,
  initialPublishMetadata, runAfterEditorSave, scheduleHydratedEditorTask,
} from '../src/lib/editorState.ts'
import { LatestRequestGate } from '../src/lib/editorRequestGuard.ts'
import { createElement } from 'react'
import { renderToString } from 'react-dom/server'
import { useEditorHistory } from '../src/hooks/useEditorHistory.ts'
import type { EditorHistory, EditorSnapshot } from '../src/hooks/useEditorHistory.ts'

test('hydration retains saved publish copy as the initial undo state', () => {
  const saved = { title: 'Saved title', publish_description: 'Do not erase this copy', publish_hashtags: 'one,two' }
  const initial = initialPublishMetadata(saved)
  let history!: EditorHistory
  function CaptureHistory() { history = useEditorHistory(); return null }
  renderToString(createElement(CaptureHistory))
  const snapshot: EditorSnapshot = {
    title: saved.title, startSeconds: 0, endSeconds: 30, captionsText: '',
    captionsPosition: 'bottom', captionStyleId: 'clean', captionFontScale: 1,
    captionCardScale: 1, captionYOffset: 0,
    publishTitle: initial.title, publishDescription: initial.description, publishHashtags: initial.hashtags,
  }
  history.reset(snapshot)
  history.push({ ...snapshot, publishHashtags: [...initial.hashtags] })
  assert.equal(history.canUndo(), false, 'hydration itself must not create an undo step')
  history.push({ ...snapshot, title: 'Edited title' })
  const undone = history.undo()!
  assert.equal(undone.publishDescription, saved.publish_description)
  assert.deepEqual(undone.publishHashtags, ['one', 'two'])
  assert.deepEqual(initialPublishMetadata({ title: 'New clip' }), { title: 'New clip', description: '', hashtags: [] })
})

test('pre-hydration render never enqueues persistence even if loading finishes before the timer', t => {
  t.mock.timers.enable({ apis: ['setTimeout'] })
  let ready = false
  const written: string[] = []
  scheduleHydratedEditorTask(ready, () => ready, () => written.push('preload empty copy'), 500)
  ready = true
  scheduleHydratedEditorTask(ready, () => ready, () => written.push('loaded saved copy'), 500)
  t.mock.timers.tick(500)
  assert.deepEqual(written, ['loaded saved copy'])
})

test('a queued history/autosave callback cannot cross a clip reload or unmount', t => {
  t.mock.timers.enable({ apis: ['setTimeout'] })
  const gate = new LatestRequestGate()
  const firstLoad = gate.begin()
  const written: string[] = []
  scheduleHydratedEditorTask(true, () => gate.isCurrent(firstLoad), () => written.push('stale'), 400)
  const secondLoad = gate.begin()
  const cleanup = scheduleHydratedEditorTask(true, () => gate.isCurrent(secondLoad), () => written.push('unmounted'), 400)
  cleanup()
  t.mock.timers.tick(500)
  assert.deepEqual(written, [])
})

test('subtitle generation waits for the current trim save to finish', async () => {
  const calls: string[] = []
  let finishSave!: () => void
  let savedEnd = 10
  const pending = runAfterEditorSave(
    () => new Promise<void>(resolve => { calls.push('save'); finishSave = () => { savedEnd = 20; resolve() } }),
    async () => { calls.push('generate'); return savedEnd },
    () => true,
  )
  assert.deepEqual(calls, ['save'])
  finishSave()
  assert.equal(await pending, 20)
  assert.deepEqual(calls, ['save', 'generate'])
})

test('a save failure prevents transcription and a clip switch during save aborts the queued command', async () => {
  let generated = false
  await assert.rejects(runAfterEditorSave(async () => { throw new Error('invalid trim') }, async () => { generated = true }, () => true), /invalid trim/)
  assert.equal(generated, false)
  let current = true
  assert.equal(await runAfterEditorSave(async () => { current = false }, async () => { generated = true }, () => current), undefined)
  assert.equal(generated, false)
})

test('caption results for an older edit are not applied after asynchronous transcription', async () => {
  let current = true
  const result = await runAfterEditorSave(async () => {}, async () => { current = false; return 'older subtitles' }, () => current)
  assert.equal(result, undefined)
})

test('imported recording duration caps handles and invalid fine-tune ranges cannot be saved', () => {
  assert.deepEqual(clampEditorTrim(4, 90, 60), [4, 60])
  assert.deepEqual(clampEditorTrim(-10, 20, 60), [0, 20])
  assert.equal(editorTrimError(4, 60, 60), null)
  assert.match(editorTrimError(4, 90, 60)!, /within/)
  for (const [start, end] of [[10, 10], [20, 10], [-1, 10], [0, Number.NaN]]) {
    assert.match(editorTrimError(start, end, 60)!, /before/)
  }
  assert.deepEqual(clampEditorTrim(0, 3, 0.05), [0, 0.05])
})

test('old exports cannot label newer edits or other clips as completed', () => {
  assert.equal(canMarkEditorExportComplete('clip', 'clip', 'newer', 'older', 'completed'), false)
  assert.equal(canMarkEditorExportComplete('clip', 'clip', 'same', 'same', 'pending'), false)
  assert.equal(canMarkEditorExportComplete('new clip', 'clip', 'same', 'same', 'completed'), false)
  assert.equal(canMarkEditorExportComplete('clip', 'clip', 'same', 'same', 'completed'), true)
})
