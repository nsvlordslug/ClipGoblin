import test from 'node:test'
import assert from 'node:assert/strict'
import {
  generateStandaloneCaption,
  type ClipContext,
  type CopyTone,
} from '../src/lib/publishCopyGenerator.ts'

const groundedContext: ClipContext = {
  title: 'The heal that was not a heal',
  eventTags: ['reaction'],
  emotionTags: ['shock'],
  eventSummary: 'Stacie offers to heal me, then immediately stabs me instead',
  game: 'Dead by Daylight',
  duration: 28,
}

test('local caption tones stay grounded in the detected event', () => {
  const tones: CopyTone[] = [
    'punchy',
    'clean',
    'funny',
    'hype',
    'search',
    'minimal',
    'direct_quote',
    'blame',
    'internal_thought',
    'observation',
  ]

  for (const tone of tones) {
    const caption = generateStandaloneCaption(groundedContext, tone, 0)
    assert.match(caption.toLowerCase(), /stacie/)
    assert.match(caption.toLowerCase(), /heal/)
    assert.ok(caption.length <= 240, `${tone} caption exceeded the local limit`)
  }
})

test('local generation avoids a previous caption without losing the event', () => {
  const first = generateStandaloneCaption(groundedContext, 'funny', 0)
  const rerolled = generateStandaloneCaption(groundedContext, 'funny', 1, first, [first])

  assert.notEqual(rerolled, first)
  assert.match(rerolled.toLowerCase(), /stacie/)
  assert.match(rerolled.toLowerCase(), /heal/)
})

test('local generation rejects transcript-like word vomit as its caption anchor', () => {
  const noisyContext: ClipContext = {
    ...groundedContext,
    title: "Stacie's fake heal",
    eventSummary: 'come here let me heal you oh thanks immediately stabs me',
  }

  const caption = generateStandaloneCaption(noisyContext, 'punchy', 0)

  assert.match(caption.toLowerCase(), /stacie/)
  assert.match(caption.toLowerCase(), /fake heal/)
  assert.doesNotMatch(caption.toLowerCase(), /come here let me heal you/)
})

test('local generation rejects long unpunctuated dialogue as an event summary', () => {
  const noisyContext: ClipContext = {
    ...groundedContext,
    title: "Stacie's fake heal",
    eventSummary: "not Stacie stabbing me taking more damage from Stasian than the killer I'm sorry that was funny",
  }

  const caption = generateStandaloneCaption(noisyContext, 'punchy', 0)

  assert.match(caption.toLowerCase(), /stacie/)
  assert.match(caption.toLowerCase(), /fake heal/)
  assert.doesNotMatch(caption.toLowerCase(), /more damage from stasian/)
})
