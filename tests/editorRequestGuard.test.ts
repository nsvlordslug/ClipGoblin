import test from 'node:test'
import assert from 'node:assert/strict'
import { canPersistEditorState, LatestRequestGate } from '../src/lib/editorRequestGuard.ts'

test('only the newest editor load request may update state', () => {
  const gate = new LatestRequestGate()
  const first = gate.begin()
  const second = gate.begin()

  assert.equal(gate.isCurrent(first), false)
  assert.equal(gate.isCurrent(second), true)

  gate.cancel(second)
  assert.equal(gate.isCurrent(second), false)
})

test('editor persistence requires state loaded for the current route clip', () => {
  assert.equal(canPersistEditorState('clip-b', 'clip-a'), false)
  assert.equal(canPersistEditorState('clip-b', null), false)
  assert.equal(canPersistEditorState(undefined, 'clip-a'), false)
  assert.equal(canPersistEditorState('clip-b', 'clip-b'), true)
})
