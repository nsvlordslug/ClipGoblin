import test from 'node:test'
import assert from 'node:assert/strict'
import { deriveTwitchProvenance } from '../src/lib/twitchProvenance.ts'

test('builds precise badges for a corroborated streamer clip', () => {
  const badges = deriveTwitchProvenance(
    ['community-clip', 'streamer-created', 'featured-clip', 'community-consensus', 'community-consensus:3'],
    '["community","audio","chat"]',
  )

  assert.deepEqual(
    badges.map(badge => badge.label),
    ['Streamer Clip', 'Featured on Twitch', '3 creators clipped this', 'Local signals agree'],
  )
})

test('keeps legacy consensus useful without inventing a count', () => {
  const badges = deriveTwitchProvenance(
    'community-clip,viewer-created,community-consensus',
    '["community"]',
  )

  assert.deepEqual(
    badges.map(badge => badge.label),
    ['Viewer Clip', 'Viewer consensus'],
  )
})

test('explains local-only detection sources', () => {
  const badges = deriveTwitchProvenance(['reaction', 'hype'], '["audio","transcript"]')
  assert.deepEqual(badges.map(badge => badge.label), ['Audio and transcript detection'])
})

test('identifies an AI-only pick', () => {
  const badges = deriveTwitchProvenance('banter', '["ai"]')
  assert.deepEqual(badges.map(badge => badge.label), ['AI judge pick'])
})

test('identifies legacy Twitch clips without creator provenance', () => {
  const badges = deriveTwitchProvenance('community-clip', '["community"]')
  assert.deepEqual(badges.map(badge => badge.label), ['Twitch Clip'])
})
