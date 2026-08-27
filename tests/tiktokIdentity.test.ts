import test from 'node:test'
import assert from 'node:assert/strict'

import { describeTikTokIdentity } from '../src/lib/tiktokIdentity.ts'

test('verified TikTok identity uses the API handle as primary', () => {
  assert.deepEqual(describeTikTokIdentity('lord_slug', 'Lord Slug'), {
    verified: true,
    primary: '@lord_slug',
    secondary: 'Lord Slug',
  })
})

test('TikTok identity never promotes a nickname into a handle', () => {
  assert.deepEqual(describeTikTokIdentity('', 'Lord Slug'), {
    verified: false,
    primary: 'TikTok identity unverified',
    secondary: 'Lord Slug',
  })
})

test('TikTok identity normalizes a leading at-sign without duplicating it', () => {
  assert.deepEqual(describeTikTokIdentity('  @lord_slug  ', 'lord_slug'), {
    verified: true,
    primary: '@lord_slug',
    secondary: null,
  })
})
