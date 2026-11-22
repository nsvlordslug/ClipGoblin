import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import {
  PENDING_UPDATE_KEY,
  RELEASE_NOTES,
  RELEASE_NOTES_SEEN_KEY,
  getReleaseNotesPresentation,
} from '../src/data/releaseNotes.ts'

interface LocalStorageStub {
  getItem(key: string): string | null
  setItem(key: string, value: string): void
  removeItem(key: string): void
}

function withLocalStorage(values: Record<string, string>, run: () => void) {
  const storage = new Map(Object.entries(values))
  const stub: LocalStorageStub = {
    getItem: key => storage.get(key) ?? null,
    setItem: (key, value) => { storage.set(key, value) },
    removeItem: key => { storage.delete(key) },
  }
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: stub })
  try {
    run()
  } finally {
    if (previous) Object.defineProperty(globalThis, 'localStorage', previous)
    else delete (globalThis as { localStorage?: LocalStorageStub }).localStorage
  }
}

test('newest bundled release note matches the app version', () => {
  const packageJson = JSON.parse(
    readFileSync(new URL('../package.json', import.meta.url), 'utf8'),
  ) as { version: string }

  assert.equal(RELEASE_NOTES[0]?.version, packageJson.version)
})

test('upgrade from 1.7.17 presents the 1.7.18 repair notes', () => {
  withLocalStorage({
    [RELEASE_NOTES_SEEN_KEY]: '1.7.17',
    [PENDING_UPDATE_KEY]: JSON.stringify({ fromVersion: '1.7.17', toVersion: '1.7.18' }),
  }, () => {
    const presentation = getReleaseNotesPresentation('1.7.18')

    assert.equal(presentation?.previousVersion, '1.7.17')
    assert.deepEqual(presentation?.entries.map(entry => entry.version), ['1.7.18'])
  })
})

test('users who missed 1.7.17 receive both updates in one window', () => {
  withLocalStorage({
    [RELEASE_NOTES_SEEN_KEY]: '1.7.16',
    [PENDING_UPDATE_KEY]: JSON.stringify({ fromVersion: '1.7.17', toVersion: '1.7.18' }),
  }, () => {
    const presentation = getReleaseNotesPresentation('1.7.18')

    assert.equal(presentation?.previousVersion, '1.7.16')
    assert.deepEqual(presentation?.entries.map(entry => entry.version), ['1.7.18', '1.7.17'])
  })
})
