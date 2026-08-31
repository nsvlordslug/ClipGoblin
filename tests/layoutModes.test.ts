import test from 'node:test'
import assert from 'node:assert/strict'

import { LAYOUT_OPTIONS, previewObjectFitForLayout, recommendedLayoutForAspect } from '../src/lib/editTypes.ts'

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

test('vertical gameplay recommends Context Fit while 16:9 recommends Landscape', () => {
  assert.equal(recommendedLayoutForAspect('9:16'), 'context_fit')
  assert.equal(recommendedLayoutForAspect('16:9'), 'landscape')
})

test('Landscape is a distinct 16:9 full-composition layout without changing vertical modes', () => {
  const landscape = LAYOUT_OPTIONS.find(option => option.id === 'landscape')
  const contextFit = LAYOUT_OPTIONS.find(option => option.id === 'context_fit')
  const fullFrame = LAYOUT_OPTIONS.find(option => option.id === 'none')

  assert.ok(landscape)
  assert.ok(contextFit)
  assert.ok(fullFrame)
  assert.equal(landscape.outputAspectRatio, '16:9')
  assert.equal(fullFrame.outputAspectRatio, '9:16')
  assert.equal(contextFit.outputAspectRatio, '9:16')
  assert.match(landscape.name, /Landscape \/ Widescreen/i)
  assert.match(landscape.description, /full game and HUD visible/i)
  assert.match(landscape.description, /without rotating/i)
  assert.match(fullFrame.description, /crop/i)
  assert.match(contextFit.description, /blurred, black-bar, or branded background/i)
  assert.equal(previewObjectFitForLayout('landscape'), 'contain')
  assert.equal(previewObjectFitForLayout('context_fit'), 'contain')
  assert.equal(previewObjectFitForLayout('none'), 'cover')
})
