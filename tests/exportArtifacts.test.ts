import test from 'node:test'
import assert from 'node:assert/strict'

import {
  artifactUploadFields,
  renderSnapshotKey,
  saveThenRender,
} from '../src/lib/exportArtifacts.ts'

test('save finishes before the immutable render begins', async () => {
  const events: string[] = []
  let releaseSave: (() => void) | undefined
  const saveGate = new Promise<void>(resolve => { releaseSave = resolve })

  const operation = saveThenRender(
    async () => {
      events.push('save-start')
      await saveGate
      events.push('save-finish')
    },
    async () => {
      events.push('render-start')
      return 'artifact'
    },
  )

  await Promise.resolve()
  assert.deepEqual(events, ['save-start'])
  releaseSave?.()
  assert.equal(await operation, 'artifact')
  assert.deepEqual(events, ['save-start', 'save-finish', 'render-start'])
})

test('upload metadata carries the exact immutable artifact identity', () => {
  assert.deepEqual(artifactUploadFields({
    path: 'C:\\exports\\clip\\9x16-revision.mp4',
    revision: 'revision',
    aspectRatio: '9:16',
    width: 1080,
    height: 1920,
  }), {
    artifact_path: 'C:\\exports\\clip\\9x16-revision.mp4',
    artifact_revision: 'revision',
    artifact_aspect_ratio: '9:16',
  })
})

test('preview readiness changes when format or visual settings change', () => {
  const base = renderSnapshotKey('9:16', { captionsText: 'hello', captionStyle: 'clean' })
  assert.equal(base, renderSnapshotKey('9:16', { captionsText: 'hello', captionStyle: 'clean' }))
  assert.notEqual(base, renderSnapshotKey('16:9', { captionsText: 'hello', captionStyle: 'clean' }))
  assert.notEqual(base, renderSnapshotKey('9:16', { captionsText: 'goodbye', captionStyle: 'clean' }))
  assert.notEqual(base, renderSnapshotKey('9:16', {
    captionsText: 'hello', captionStyle: 'clean', facecamLayout: 'context_fit',
  }))
})
