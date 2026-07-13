import test from 'node:test'
import assert from 'node:assert/strict'
import { requiresHighDetectionCostConsent } from '../src/lib/detectionCostConsent.ts'

test('requires consent before a BYOK user switches from medium to high', () => {
  assert.equal(requiresHighDetectionCostConsent({
    currentSensitivity: 'medium',
    nextSensitivity: 'high',
    byokProviderSelected: true,
  }), true)
})

test('requires consent before a BYOK user switches from low to high', () => {
  assert.equal(requiresHighDetectionCostConsent({
    currentSensitivity: 'low',
    nextSensitivity: 'high',
    byokProviderSelected: true,
  }), true)
})

test('does not warn free-mode users because High has no BYOK bill', () => {
  assert.equal(requiresHighDetectionCostConsent({
    currentSensitivity: 'medium',
    nextSensitivity: 'high',
    byokProviderSelected: false,
  }), false)
})

test('does not warn for non-High choices or an already active High setting', () => {
  assert.equal(requiresHighDetectionCostConsent({
    currentSensitivity: 'medium',
    nextSensitivity: 'low',
    byokProviderSelected: true,
  }), false)
  assert.equal(requiresHighDetectionCostConsent({
    currentSensitivity: 'high',
    nextSensitivity: 'high',
    byokProviderSelected: true,
  }), false)
})
