import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

const editor = readFileSync(new URL('../src/pages/Editor.tsx', import.meta.url), 'utf8')
const editorCss = readFileSync(new URL('../src/index.css', import.meta.url), 'utf8')

test('Publish reuses the ordinary editor preview without a preparation gate', () => {
  assert.match(editor, /editorWorkspace === 'publish' \? 'Publish preview' : 'Preview'/)
  assert.match(editor, /<ClipPlayer\s+src=\{videoSrc\}/)
  assert.doesNotMatch(editor, /Prepare exact preview/i)
  assert.doesNotMatch(editor, /Preview not prepared/i)
  assert.doesNotMatch(editor, /tiktokPreviewReady|tiktokNeedsPreview/)
})

test('upload and scheduling still request a fresh rendered artifact at action time', () => {
  const uploadStart = editor.indexOf('const handleMultiUpload')
  const scheduleStart = editor.indexOf('const handleScheduleUpload')
  const uploadBlock = editor.slice(uploadStart, scheduleStart)
  const scheduleBlock = editor.slice(scheduleStart, editor.indexOf('const handleAddToMontage'))

  assert.match(uploadBlock, /const artifact = await onExportForFormat\(aspectRatio\)/)
  assert.match(uploadBlock, /uploadToPlatform\(platform, forcePlatforms\.has\(platform\), artifact\)/)
  assert.match(scheduleBlock, /const artifact = await onExportForFormat\(aspectRatio\)/)
  assert.match(scheduleBlock, /buildUploadMeta\(platform, false, artifact\)/)
})

test('initial editor loading does not wait for caption alignment', () => {
  const loadStart = editor.indexOf('// ── Load clip data ──')
  const loadEnd = editor.indexOf('// ── Sync aspect ratio when export preset changes ──')
  const loadBlock = editor.slice(loadStart, loadEnd)

  assert.doesNotMatch(loadBlock, /ensure_clip_captions_aligned/)
  assert.match(editor, /shouldPrepareCaptionAlignment\(/)
})

test('Publish uses document scrolling and non-sticky controls', () => {
  assert.match(editor, /v4-editor-tools-publish/)
  assert.doesNotMatch(editor, /v4-editor-publish-controls sticky/)
  assert.match(editorCss, /\.v4-editor-tools-publish\s*\{[\s\S]*?max-height:\s*none;[\s\S]*?overflow:\s*visible;/)
  assert.match(editorCss, /\.v4-editor-publish-controls\s*\{[\s\S]*?position:\s*static;/)
})
