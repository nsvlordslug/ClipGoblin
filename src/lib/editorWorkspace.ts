export const EDITOR_WORKSPACE_IDS = ['edit', 'captions', 'publish'] as const

export type EditorWorkspaceId = typeof EDITOR_WORKSPACE_IDS[number]

export type EditorWorkspaceNavigationKey =
  | 'ArrowDown'
  | 'ArrowLeft'
  | 'ArrowRight'
  | 'ArrowUp'
  | 'End'
  | 'Home'

export function isEditorWorkspaceId(value: unknown): value is EditorWorkspaceId {
  return typeof value === 'string' && EDITOR_WORKSPACE_IDS.includes(value as EditorWorkspaceId)
}

export function getNextEditorWorkspace(
  current: EditorWorkspaceId,
  key: EditorWorkspaceNavigationKey,
): EditorWorkspaceId {
  const currentIndex = EDITOR_WORKSPACE_IDS.indexOf(current)

  if (key === 'Home') return EDITOR_WORKSPACE_IDS[0]
  if (key === 'End') return EDITOR_WORKSPACE_IDS[EDITOR_WORKSPACE_IDS.length - 1]

  const direction = key === 'ArrowRight' || key === 'ArrowDown' ? 1 : -1
  const nextIndex = (currentIndex + direction + EDITOR_WORKSPACE_IDS.length)
    % EDITOR_WORKSPACE_IDS.length
  return EDITOR_WORKSPACE_IDS[nextIndex]
}
