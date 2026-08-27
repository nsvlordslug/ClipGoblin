import test from 'node:test'
import assert from 'node:assert/strict'
import {
  FRAME_SAFE_CAPTION_SECONDS,
  findActiveSegment,
  frameSafeSubtitleSegments,
  normalizeSubtitleSegments,
  parseSrt,
  serializeSrt,
  shiftSubtitleSegments,
  splitSubtitleSegmentsByWord,
} from '../src/lib/subtitleUtils.ts'

test('makes only generated sub-frame words displayable without changing their text', () => {
  const original = parseSrt(
    '1\n00:00:00,000 --> 00:00:00,010\nand\n\n'
      + "2\n00:00:00,010 --> 00:00:00,400\nSlug's\n\n"
      + '3\n00:00:08,280 --> 00:00:08,300\nleave\n\n'
      + '4\n00:00:08,300 --> 00:00:08,310\nme\n\n'
      + '5\n00:00:08,310 --> 00:00:08,410\nalone.',
  )
  const snapshot = structuredClone(original)
  const adjusted = frameSafeSubtitleSegments(original, 'aligned')

  assert.deepEqual(original, snapshot)
  assert.deepEqual(adjusted.map(segment => segment.text), original.map(segment => segment.text))
  assert.ok(adjusted.every(segment => segment.endTime - segment.startTime >= FRAME_SAFE_CAPTION_SECONDS - 1e-9))
  assert.ok(adjusted.every((segment, index) => index === 0 || segment.startTime >= adjusted[index - 1].endTime))
  assert.equal(adjusted[1].endTime, original[1].endTime)
  assert.equal(adjusted.at(-1)?.endTime, original.at(-1)?.endTime)
})

test('leaves edited and ordinary baseline caption timing exactly unchanged', () => {
  const edited = parseSrt(
    '1\n00:00:00,000 --> 00:00:00,010\nintentional\n\n'
      + '2\n00:00:00,010 --> 00:00:00,400\ntiming',
  )
  const ordinary = parseSrt(
    '1\n00:00:01,000 --> 00:00:01,400\nclean\n\n'
      + '2\n00:00:01,400 --> 00:00:01,850\nbaseline',
  )

  assert.equal(frameSafeSubtitleSegments(edited, 'edited'), edited)
  assert.equal(frameSafeSubtitleSegments(ordinary, 'aligned'), ordinary)
})

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

test('deduplicates overlap-window words and serializes the normalized timeline', () => {
  const segments = parseSrt(
    '1\r\n00:00:00,000 --> 00:00:00,800\r\nhello\r\n\r\n'
      + '2\r\n00:00:00,100 --> 00:00:00,900\r\nhello\r\n\r\n'
      + '3\r\n00:00:00,600 --> 00:00:01,200\r\nthere',
  )
  const normalized = normalizeSubtitleSegments(segments)
  const reparsed = parseSrt(serializeSrt(normalized))

  assert.deepEqual(reparsed.map(segment => segment.text), ['hello', 'there'])
  assert.ok(reparsed[0].endTime <= reparsed[1].startTime)
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
