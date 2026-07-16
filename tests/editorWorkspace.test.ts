import test from 'node:test'
import assert from 'node:assert/strict'
import { getNextEditorWorkspace, isEditorWorkspaceId } from '../src/lib/editorWorkspace.ts'

test('editor workspace navigation moves between edit, captions, and publish', () => {
  assert.equal(getNextEditorWorkspace('edit', 'ArrowRight'), 'captions')
  assert.equal(getNextEditorWorkspace('captions', 'ArrowRight'), 'publish')
  assert.equal(getNextEditorWorkspace('publish', 'ArrowLeft'), 'captions')
})

test('editor workspace navigation wraps and supports Home and End', () => {
  assert.equal(getNextEditorWorkspace('publish', 'ArrowRight'), 'edit')
  assert.equal(getNextEditorWorkspace('edit', 'ArrowLeft'), 'publish')
  assert.equal(getNextEditorWorkspace('captions', 'Home'), 'edit')
  assert.equal(getNextEditorWorkspace('captions', 'End'), 'publish')
})

test('editor workspace route state accepts only known workspaces', () => {
  assert.equal(isEditorWorkspaceId('publish'), true)
  assert.equal(isEditorWorkspaceId('captions'), true)
  assert.equal(isEditorWorkspaceId('settings'), false)
  assert.equal(isEditorWorkspaceId(null), false)
})
