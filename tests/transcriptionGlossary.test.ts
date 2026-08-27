import test from 'node:test'
import assert from 'node:assert/strict'
import {
  mergeTranscriptionGlossaryTerms,
  normalizeTranscriptionGlossary,
} from '../src/lib/transcriptionGlossary.ts'

test('glossary normalizes separators and deduplicates without replacing spoken text', () => {
  assert.equal(
    normalizeTranscriptionGlossary(" Slug ; Slug's\nslug "),
    "Slug, Slug's",
  )
  assert.equal(
    normalizeTranscriptionGlossary('and shlex pings go hard'),
    'and shlex pings go hard',
  )
})

test('glossary merges approved proper-name forms with existing terms', () => {
  assert.equal(
    mergeTranscriptionGlossaryTerms('Dead by Daylight', ['Slug', "Slug's"]),
    "Dead by Daylight, Slug, Slug's",
  )
})
