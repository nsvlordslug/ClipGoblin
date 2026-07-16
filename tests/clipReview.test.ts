import test from 'node:test'
import assert from 'node:assert/strict'
import {
  getPersonalizationStatusCopy,
  parseClipReviewIssues,
  toggleExpandedReviewClip,
  type PersonalizationStatus,
} from '../src/types/clipReview.ts'

test('parses only known edit issues and removes duplicates', () => {
  assert.deepEqual(
    parseClipReviewIssues(
      '["starts_too_late","cuts_off_early","starts_too_late","unknown"]',
    ),
    ['starts_too_late', 'cuts_off_early'],
  )
})

test('treats malformed or non-array issue data as empty', () => {
  assert.deepEqual(parseClipReviewIssues(null), [])
  assert.deepEqual(parseClipReviewIssues('{"cuts_off_early":true}'), [])
  assert.deepEqual(parseClipReviewIssues('not-json'), [])
})

test('keeps only one clip feedback disclosure open at a time', () => {
  assert.equal(toggleExpandedReviewClip(null, 'clip-a'), 'clip-a')
  assert.equal(toggleExpandedReviewClip('clip-a', 'clip-b'), 'clip-b')
  assert.equal(toggleExpandedReviewClip('clip-b', 'clip-b'), null)
})

test('personalization status copy explains learning progress and variety', () => {
  const base: PersonalizationStatus = {
    state: 'learning',
    total_ratings: 14,
    usable_ratings: 14,
    rating_classes: 3,
    confidence: 0.7,
    is_personalizing: true,
    target_ratings: 20,
  }

  assert.deepEqual(getPersonalizationStatusCopy(base), {
    label: 'Personalization is learning',
    detail: '14/20 usable ratings, 70% confidence. Future analyses are already being gently reordered.',
    tone: 'learning',
  })

  const variety = getPersonalizationStatusCopy({
    ...base,
    state: 'needs_variety',
    usable_ratings: 7,
    rating_classes: 1,
    confidence: 0,
    is_personalizing: false,
  })
  assert.equal(variety.label, 'More rating variety needed')
  assert.match(variety.detail, /at least two rating choices/)
})

test('personalization status distinguishes ratings from lower-weight behavior', () => {
  const copy = getPersonalizationStatusCopy({
    state: 'learning',
    total_ratings: 8,
    usable_ratings: 8,
    rating_classes: 3,
    confidence: 0.46,
    is_personalizing: true,
    target_ratings: 20,
    behavior_events: 12,
    usable_behavior_events: 5,
    total_evidence: 13,
  })

  assert.match(copy.detail, /8\/20 ratings plus 5 useful actions/)
  assert.match(copy.detail, /46% confidence/)
})

test('boundary feedback is reported even before ranking personalization activates', () => {
  const copy = getPersonalizationStatusCopy({
    state: 'empty',
    total_ratings: 0,
    usable_ratings: 0,
    rating_classes: 0,
    confidence: 0,
    is_personalizing: false,
    target_ratings: 20,
    boundary_feedback_samples: 3,
    boundary_learning_active: true,
    boundary_confidence: 0.58,
  })

  assert.equal(copy.label, 'Boundary learning is active')
  assert.match(copy.detail, /3 clips/)
  assert.match(copy.detail, /start and end/)
})
