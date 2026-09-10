import test from 'node:test'
import assert from 'node:assert/strict'
import { filterCompletedClipGroups, retainDisplayedSelection } from '../src/lib/clipSelection.ts'

const groups = [
  { id: 'done', clips: [{ id: 'exported', render_status: 'completed', output_path: 'export.mp4' }] },
  { id: 'mixed', clips: [
    { id: 'pending', render_status: 'pending', output_path: null },
    { id: 'mixed-export', render_status: 'completed', output_path: 'mixed.mp4' },
  ] },
]

test('Select All and bulk payloads include only clips in displayed groups', () => {
  const displayed = filterCompletedClipGroups(groups, true).flatMap(group => group.clips)
  const selected = new Set(displayed.map(clip => clip.id))
  assert.deepEqual([...selected], ['pending', 'mixed-export'])
  assert.deepEqual(displayed.filter(clip => selected.has(clip.id)).map(clip => clip.id), ['pending', 'mixed-export'])
  assert.equal(selected.has('exported'), false)
})

test('hiding completed groups prunes prior selection and revealing does not reselect them', () => {
  const previous = new Set(['exported', 'pending', 'deleted'])
  const remaining = retainDisplayedSelection(previous, filterCompletedClipGroups(groups, true).flatMap(group => group.clips))
  assert.deepEqual([...remaining], ['pending'])
  assert.deepEqual([...retainDisplayedSelection(remaining, filterCompletedClipGroups(groups, false).flatMap(group => group.clips))], ['pending'])
  assert.equal(retainDisplayedSelection(remaining, [{ id: 'pending' }]), remaining)
})

test('an empty filtered library has no bulk selection', () => {
  const displayed = filterCompletedClipGroups([groups[0]], true).flatMap(group => group.clips)
  assert.deepEqual(displayed, [])
  assert.equal(retainDisplayedSelection(new Set(['exported']), displayed).size, 0)
})
