import test from 'node:test'
import assert from 'node:assert/strict'
import { getCaptionFallbackNotice } from '../src/lib/aiFallbackNotice.ts'

test('explains when paid caption generation is deliberately disabled', () => {
  const notice = getCaptionFallbackNotice({
    provider: 'claude',
    useForCaptions: false,
    hasActiveKey: true,
    reason: 'provider-returned-free',
  })

  assert.match(notice ?? '', /turned off in Settings/)
  assert.match(notice ?? '', /Caption generation \(TikTok copy\)/)
})

test('distinguishes a missing key from a provider request failure', () => {
  const missingKey = getCaptionFallbackNotice({
    provider: 'openai',
    useForCaptions: true,
    hasActiveKey: false,
    reason: 'provider-returned-free',
  })
  const failedRequest = getCaptionFallbackNotice({
    provider: 'gemini',
    useForCaptions: true,
    hasActiveKey: true,
    reason: 'request-failed',
  })

  assert.match(missingKey ?? '', /no API key is saved/)
  assert.match(failedRequest ?? '', /could not generate this caption/)
})
