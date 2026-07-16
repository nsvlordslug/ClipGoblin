import test from 'node:test'
import assert from 'node:assert/strict'
import {
  findActiveSegment,
  parseSrt,
  shiftSubtitleSegments,
  splitSubtitleSegmentsByWord,
} from '../src/lib/subtitleUtils.ts'

test('does not show a nearby subtitle before or after its cue', () => {
  const [segment] = parseSrt('1\n00:00:05,000 --> 00:00:06,000\nRight on time')

  assert.equal(findActiveSegment([segment], 4.5), null)
  assert.equal(findActiveSegment([segment], 5.0)?.text, 'Right on time')
  assert.equal(findActiveSegment([segment], 6.0), null)
})

test('splits a legacy long cue into one visible word at a time', () => {
  const original = parseSrt(
    '1\n00:00:10,000 --> 00:00:22,000\nNot Stacie stabbing me taking more damage than the killer. That was funny',
  )
  const split = splitSubtitleSegmentsByWord(original)

  assert.equal(split.length, original[0].text.split(/\s+/).length)
  assert.equal(split.map(segment => segment.text).join(' '), original[0].text)
  assert.equal(split[0].startTime, 10)
  assert.ok((split.at(-1)?.endTime ?? 0) <= 22)
  assert.ok(split.every(segment => segment.text.split(/\s+/).length === 1))
  assert.ok(split.every((segment, index) => index === 0 || segment.startTime >= split[index - 1].endTime))
})

test('leaves a real timing gap blank between spoken words', () => {
  const segments = parseSrt(
    '1\n00:00:10,000 --> 00:00:10,400\nwait\n\n2\n00:00:12,000 --> 00:00:12,400\nnow',
  )
  const split = splitSubtitleSegmentsByWord(segments)

  assert.equal(findActiveSegment(split, 11), null)
  assert.equal(findActiveSegment(split, 12.1)?.text, 'now')
})

test('normalizes overlapping cues so two words never show together', () => {
  const segments = parseSrt(
    '1\n00:00:10,000 --> 00:00:11,000\nfirst\n\n2\n00:00:10,500 --> 00:00:11,000\nsecond',
  )
  const split = splitSubtitleSegmentsByWord(segments)

  assert.equal(split[0].endTime, 10.5)
  assert.equal(findActiveSegment(split, 10.6)?.text, 'second')
})

test('shifts every subtitle while preserving durations and gaps', () => {
  const segments = parseSrt(
    '1\n00:00:01,000 --> 00:00:01,400\nfirst\n\n2\n00:00:02,000 --> 00:00:02,600\nsecond',
  )
  const shifted = shiftSubtitleSegments(segments, 0.3)

  assert.deepEqual(
    shifted.map(segment => [segment.startTime, segment.endTime]),
    [[1.3, 1.7], [2.3, 2.9]],
  )
})

test('clamps an earlier shift at zero without changing relative timing', () => {
  const segments = parseSrt(
    '1\n00:00:00,050 --> 00:00:00,300\nfirst\n\n2\n00:00:01,000 --> 00:00:01,500\nsecond',
  )
  const shifted = shiftSubtitleSegments(segments, -0.1)

  assert.deepEqual(
    shifted.map(segment => [segment.startTime, segment.endTime]),
    [[0, 0.25], [0.95, 1.45]],
  )
})
