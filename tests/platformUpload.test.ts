import test from 'node:test'
import assert from 'node:assert/strict'

import {
  isSuccessfulUploadHandoff,
  isTikTokInboxDelivered,
  shouldOfferForcedReupload,
} from '../src/lib/uploadStatus.ts'

test('a duplicate without a link is not presented as a successful new upload', () => {
  assert.equal(isSuccessfulUploadHandoff('duplicate'), false)
  assert.equal(shouldOfferForcedReupload({ status: 'duplicate' }), true)
})

test('a newly accepted private post does not immediately invite a duplicate', () => {
  assert.equal(shouldOfferForcedReupload({
    status: 'done',
    acceptedWithoutLink: true,
  }), false)
  assert.equal(shouldOfferForcedReupload({ status: 'done' }), false)
})

test('completed and processing uploads remain successful handoffs', () => {
  assert.equal(isSuccessfulUploadHandoff('done'), true)
  assert.equal(isSuccessfulUploadHandoff('processing'), true)
  assert.equal(isSuccessfulUploadHandoff('error'), false)
})

test('only TikTok inbox delivery is classified as a draft handoff', () => {
  assert.equal(isTikTokInboxDelivered('inbox_delivered'), true)
  assert.equal(isTikTokInboxDelivered('processing'), false)
  assert.equal(isTikTokInboxDelivered('complete'), false)
  assert.equal(isTikTokInboxDelivered(undefined), false)
})
