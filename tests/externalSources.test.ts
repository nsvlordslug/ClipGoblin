import test from 'node:test'
import assert from 'node:assert/strict'
import {
  chunkCandidateIds,
  formatSourceBytes,
  groupCandidatesByFolder,
  selectableCandidates,
  toggleCandidate,
} from '../src/lib/externalSources.ts'

test('candidate selection toggles without mutating the original set', () => {
  const original = new Set(['a'])
  const added = toggleCandidate(original, 'b')
  assert.deepEqual([...original], ['a'])
  assert.deepEqual([...added].sort(), ['a', 'b'])
  assert.deepEqual([...toggleCandidate(added, 'a')], ['b'])
})

test('already imported source candidates are not selectable', () => {
  const candidates = [
    { id: 'new', name: 'new.mp4', folderLabel: 'Game', path: 'x', sizeBytes: 1, recordedAt: '', importedClipId: null },
    { id: 'old', name: 'old.mp4', folderLabel: 'Game', path: 'y', sizeBytes: 1, recordedAt: '', importedClipId: 'clip-1' },
  ]
  assert.deepEqual(selectableCandidates(candidates).map(candidate => candidate.id), ['new'])
})

test('source byte labels stay compact', () => {
  assert.equal(formatSourceBytes(512 * 1024 * 1024), '512.0 MB')
  assert.equal(formatSourceBytes(2 * 1024 * 1024 * 1024), '2.0 GB')
})

test('source candidates group by readable game folder', () => {
  const candidates = [
    { id: 'dbd-new', name: 'escape.mp4', folderLabel: 'Dead by Daylight', path: 'a', sizeBytes: 1, recordedAt: '', importedClipId: null },
    { id: 'valorant', name: 'ace.mp4', folderLabel: 'Valorant', path: 'b', sizeBytes: 1, recordedAt: '', importedClipId: null },
    { id: 'dbd-old', name: 'chase.mp4', folderLabel: 'Dead by Daylight', path: 'c', sizeBytes: 1, recordedAt: '', importedClipId: 'clip-1' },
  ]

  const groups = groupCandidatesByFolder(candidates)
  assert.deepEqual(groups.map(group => group.label), ['Dead by Daylight', 'Valorant'])
  assert.equal(groups[0].candidates.length, 2)
  assert.deepEqual(groups[0].available.map(candidate => candidate.id), ['dbd-new'])
})

test('large imports are divided into bounded progress batches', () => {
  const ids = Array.from({ length: 430 }, (_, index) => `clip-${index}`)
  const batches = chunkCandidateIds(ids)
  assert.equal(batches.length, 9)
  assert.equal(batches[0].length, 50)
  assert.equal(batches.at(-1)?.length, 30)
  assert.deepEqual(batches.flat(), ids)
})
