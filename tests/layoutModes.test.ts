import test from 'node:test'
import assert from 'node:assert/strict'

import { LAYOUT_OPTIONS, recommendedLayoutForAspect } from '../src/lib/editTypes.ts'

test('Context Fit is available as a distinct context-preserving layout', () => {
  const contextFit = LAYOUT_OPTIONS.find(option => option.id === 'context_fit')
  const fullFrame = LAYOUT_OPTIONS.find(option => option.id === 'none')

  assert.ok(contextFit)
  assert.ok(fullFrame)
  assert.notDeepEqual(contextFit.regions, fullFrame.regions)
  assert.match(contextFit.description, /entire source composition visible/i)
  assert.match(contextFit.description, /black-bar/i)
  assert.equal(contextFit.tag, 'Preserves context')
  assert.equal(fullFrame.tag, 'Optional crop')
  assert.match(fullFrame.description, /scene edges may be trimmed/i)
})

test('vertical gameplay recommends Context Fit while landscape keeps Full Frame', () => {
  assert.equal(recommendedLayoutForAspect('9:16'), 'context_fit')
  assert.equal(recommendedLayoutForAspect('16:9'), 'none')
})
