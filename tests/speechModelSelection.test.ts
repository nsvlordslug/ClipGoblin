import test from 'node:test'
import assert from 'node:assert/strict'
import {
  isSpeechModelNavigationLocked,
  persistSpeechModelSelection,
  speechModelLabel,
} from '../src/lib/speechModelSelection.ts'

test('model selection is not confirmed until save and readback both succeed', async () => {
  const calls: string[] = []
  const selected = await persistSpeechModelSelection('medium', {
    save: async model => { calls.push(`save:${model}`) },
    read: async () => { calls.push('read'); return 'medium' },
  })

  assert.equal(selected, 'medium')
  assert.deepEqual(calls, ['save:medium', 'read'])
})

test('model selection rejects a failed readback instead of activating optimistically', async () => {
  await assert.rejects(
    persistSpeechModelSelection('medium', {
      save: async () => {},
      read: async () => 'base',
    }),
    /could not confirm/i,
  )
})

test('runtime model labels distinguish Base from Medium', () => {
  assert.equal(speechModelLabel('base'), 'Fast local (Base)')
  assert.equal(speechModelLabel('medium'), 'Quality local (Medium)')
  assert.equal(speechModelLabel('unknown'), null)
})

test('navigation stays on Settings while a model selection is being confirmed', () => {
  assert.equal(isSpeechModelNavigationLocked(true, '/clips'), true)
  assert.equal(isSpeechModelNavigationLocked(true, '/settings'), false)
  assert.equal(isSpeechModelNavigationLocked(false, '/clips'), false)
})
