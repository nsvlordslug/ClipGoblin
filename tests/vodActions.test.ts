import test from 'node:test'
import assert from 'node:assert/strict'
import { getVodPrimaryAction } from '../src/lib/vodActions.ts'

test('VOD primary actions follow the download to analyze to review lifecycle', () => {
  assert.equal(getVodPrimaryAction({ downloadStatus: 'pending', analysisStatus: 'pending' }).id, 'download')
  assert.equal(getVodPrimaryAction({ downloadStatus: 'downloading', analysisStatus: 'pending' }).id, 'downloading')
  assert.equal(getVodPrimaryAction({ downloadStatus: 'downloaded', analysisStatus: 'pending' }).id, 'analyze')
  assert.equal(getVodPrimaryAction({ downloadStatus: 'downloaded', analysisStatus: 'analyzing' }).id, 'analyzing')
  assert.equal(getVodPrimaryAction({ downloadStatus: 'downloaded', analysisStatus: 'completed' }).id, 'view-clips')
})

test('VOD failures replace the primary action with the correct recovery', () => {
  assert.deepEqual(getVodPrimaryAction({ downloadStatus: 'failed', analysisStatus: 'pending' }), {
    id: 'repair-download', label: 'Repair & retry', disabled: false, tone: 'warning',
  })
  assert.deepEqual(getVodPrimaryAction({ downloadStatus: 'downloaded', analysisStatus: 'failed' }), {
    id: 'retry-analysis', label: 'Retry analysis', disabled: false, tone: 'danger',
  })
  assert.equal(getVodPrimaryAction({ downloadStatus: 'pending', analysisStatus: 'failed' }).id, 'download')
})

test('completed clips remain reviewable even after the source download is removed', () => {
  assert.equal(getVodPrimaryAction({ downloadStatus: 'pending', analysisStatus: 'completed' }).id, 'view-clips')
})
