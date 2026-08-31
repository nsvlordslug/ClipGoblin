import test from 'node:test'
import assert from 'node:assert/strict'

import {
  buildFacebookShareUrl,
  buildSuggestedXCaption,
  buildThreadsComposeUrl,
  buildXComposeUrl,
  canOfferXHandoff,
  createXHandoff,
  MANUAL_SHARE_UNAVAILABLE_MESSAGE,
  MAX_X_SUGGESTED_TEXT_LENGTH,
} from '../src/lib/xHandoff.ts'

test('builds the official X compose intent with separately encoded text and URL', () => {
  const caption = 'That ending & the comeback? #gaming\nWorth it.'
  const publishedUrl = 'https://youtu.be/abc123?si=one&feature=share'
  const parsed = new URL(buildXComposeUrl(caption, publishedUrl))

  assert.equal(parsed.origin, 'https://x.com')
  assert.equal(parsed.pathname, '/intent/tweet')
  assert.equal(parsed.searchParams.get('text'), caption)
  assert.equal(parsed.searchParams.get('url'), publishedUrl)
  assert.equal(parsed.searchParams.size, 2)
})

test('builds the Facebook browser share with only the safely encoded published link', () => {
  const publishedUrl = 'https://www.youtube.com/watch?v=abc123&list=highlights#moment'
  const parsed = new URL(buildFacebookShareUrl(publishedUrl))

  assert.equal(parsed.origin, 'https://www.facebook.com')
  assert.equal(parsed.pathname, '/sharer/sharer.php')
  assert.equal(parsed.searchParams.get('u'), publishedUrl)
  assert.equal(parsed.searchParams.get('text'), null)
  assert.equal(parsed.searchParams.size, 1)
})

test('builds the Threads composer with separately encoded text and URL', () => {
  const caption = 'That ending & comeback? #gaming\nIt went hard.'
  const publishedUrl = 'https://www.tiktok.com/@creator/video/123?lang=en&share=1'
  const parsed = new URL(buildThreadsComposeUrl(caption, publishedUrl))

  assert.equal(parsed.origin, 'https://www.threads.com')
  assert.equal(parsed.pathname, '/intent/post')
  assert.equal(parsed.searchParams.get('text'), caption)
  assert.equal(parsed.searchParams.get('url'), publishedUrl)
  assert.equal(parsed.searchParams.size, 2)
})

test('creates handoffs for canonical YouTube and TikTok publish URLs', () => {
  const youtube = createXHandoff('youtube', 'https://www.youtube.com/watch?v=abc', 'A close call')
  const tiktok = createXHandoff('tiktok', 'https://www.tiktok.com/@creator/video/123', 'A close call')

  assert.equal(youtube?.platformLabel, 'YouTube')
  assert.equal(tiktok?.platformLabel, 'TikTok')
  assert.match(youtube?.suggestedCaption ?? '', /Watch on YouTube:/)
  assert.match(tiktok?.suggestedCaption ?? '', /Watch on TikTok:/)
})

test('rejects non-platform, insecure, and hostname-confusion URLs', () => {
  assert.equal(createXHandoff('youtube', 'javascript:alert(1)', 'Clip'), null)
  assert.equal(createXHandoff('youtube', 'http://youtu.be/abc', 'Clip'), null)
  assert.equal(createXHandoff('youtube', 'https://youtube.com.evil.example/watch?v=abc', 'Clip'), null)
  assert.equal(createXHandoff('tiktok', 'https://tiktok.com.evil.example/video/123', 'Clip'), null)
  assert.equal(createXHandoff('instagram', 'https://www.instagram.com/reel/123', 'Clip'), null)
  assert.equal(canOfferXHandoff('youtube', null), false)
})

test('offers manual share actions only after a confirmed public TikTok or YouTube URL exists', () => {
  assert.equal(canOfferXHandoff('youtube', 'https://www.youtube.com/watch?v=abc'), true)
  assert.equal(canOfferXHandoff('youtube_shorts', 'https://youtu.be/abc'), true)
  assert.equal(canOfferXHandoff('tiktok', 'https://www.tiktok.com/@creator/video/123'), true)
  assert.equal(canOfferXHandoff('youtube', 'https://youtube.com.evil.example/watch?v=abc'), false)
  assert.equal(canOfferXHandoff('tiktok', 'https://example.com/@creator/video/123'), false)
  assert.equal(canOfferXHandoff('instagram', 'https://www.instagram.com/reel/123'), false)
})

test('explains the verified-public-link gate without implying automatic publishing', () => {
  assert.equal(
    MANUAL_SHARE_UNAVAILABLE_MESSAGE,
    'Available after TikTok or YouTube provides a verified public link. ClipGoblin prepares a manual share; you review and post it yourself.',
  )
  assert.equal(canOfferXHandoff('youtube'), false)
  assert.equal(canOfferXHandoff('tiktok', ''), false)
})

test('keeps the editable suggestion concise even with a very long clip title', () => {
  const caption = buildSuggestedXCaption('youtube_shorts', 'A'.repeat(600))
  assert.ok(Array.from(caption).length <= MAX_X_SUGGESTED_TEXT_LENGTH)
  assert.match(caption, /Watch on YouTube Shorts:$/)
})
