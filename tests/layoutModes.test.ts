import test from 'node:test'
import assert from 'node:assert/strict'

import { LAYOUT_OPTIONS } from '../src/lib/editTypes.ts'

test('Context Fit is available as a distinct context-preserving layout', () => {
  const contextFit = LAYOUT_OPTIONS.find(option => option.id === 'context_fit')
  const fullFrame = LAYOUT_OPTIONS.find(option => option.id === 'none')

  assert.ok(contextFit)
  assert.ok(fullFrame)
  assert.notDeepEqual(contextFit.regions, fullFrame.regions)
  assert.match(contextFit.description, /entire video visible/i)
  assert.equal(contextFit.tag, 'Best for imports')
})
