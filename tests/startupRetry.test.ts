import test from 'node:test'
import assert from 'node:assert/strict'

import { withStartupRetry } from '../src/lib/startupRetry.ts'

test('startup retry returns once a transient read succeeds', async () => {
  let attempts = 0
  const result = await withStartupRetry(async () => {
    attempts += 1
    if (attempts < 3) throw new Error('backend still starting')
    return 'restored'
  }, [0, 0, 0])

  assert.equal(result, 'restored')
  assert.equal(attempts, 3)
})

test('startup retry surfaces the final error after the bounded attempts', async () => {
  let attempts = 0
  await assert.rejects(
    withStartupRetry(async () => {
      attempts += 1
      throw new Error(`failure ${attempts}`)
    }, [0, 0, 0]),
    /failure 3/,
  )
  assert.equal(attempts, 3)
})
