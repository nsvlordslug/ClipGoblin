import test from 'node:test'
import assert from 'node:assert/strict'
import { getNextSettingsSection, resolveSettingsSection } from '../src/lib/settingsNavigation.ts'

test('settings navigation moves forward and backward with arrow keys', () => {
  assert.equal(getNextSettingsSection('account', 'ArrowRight'), 'sources')
  assert.equal(getNextSettingsSection('sources', 'ArrowRight'), 'detection')
  assert.equal(getNextSettingsSection('detection', 'ArrowDown'), 'ai')
  assert.equal(getNextSettingsSection('ai', 'ArrowLeft'), 'detection')
  assert.equal(getNextSettingsSection('storage', 'ArrowUp'), 'editing')
})

test('settings navigation wraps and supports Home and End', () => {
  assert.equal(getNextSettingsSection('account', 'ArrowLeft'), 'appearance')
  assert.equal(getNextSettingsSection('appearance', 'ArrowRight'), 'account')
  assert.equal(getNextSettingsSection('storage', 'Home'), 'account')
  assert.equal(getNextSettingsSection('account', 'End'), 'appearance')
})

test('settings shortcuts resolve only known sections', () => {
  assert.equal(resolveSettingsSection('sources'), 'sources')
  assert.equal(resolveSettingsSection('detection'), 'detection')
  assert.equal(resolveSettingsSection('unknown'), 'account')
  assert.equal(resolveSettingsSection(null), 'account')
})
