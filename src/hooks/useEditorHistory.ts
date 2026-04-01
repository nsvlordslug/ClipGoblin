import { useRef, useCallback } from 'react'

// ── Editor History (Undo / Redo) ──
// Tracks snapshots of editor-relevant state.  Each snapshot is a plain
// object that can be cheaply compared by JSON equality.

const MAX_HISTORY = 50

export interface EditorSnapshot {
  title: string
  startSeconds: number
  endSeconds: number
  captionsText: string
  captionsPosition: string
  captionStyleId: string
  captionYOffset: number
  publishTitle: string
  publishDescription: string
  publishHashtags: string[]
}

export function snapshotEqual(a: EditorSnapshot, b: EditorSnapshot): boolean {
  return (
    a.title === b.title &&
    a.startSeconds === b.startSeconds &&
    a.endSeconds === b.endSeconds &&
    a.captionsText === b.captionsText &&
    a.captionsPosition === b.captionsPosition &&
    a.captionStyleId === b.captionStyleId &&
    a.captionYOffset === b.captionYOffset &&
    a.publishTitle === b.publishTitle &&
    a.publishDescription === b.publishDescription &&
    a.publishHashtags.length === b.publishHashtags.length &&
    a.publishHashtags.every((t, i) => t === b.publishHashtags[i])
  )
}

export interface EditorHistory {
  /** Push a new snapshot (no-op if identical to current) */
  push: (snapshot: EditorSnapshot) => void
  /** Undo — returns the previous snapshot or null */
  undo: () => EditorSnapshot | null
  /** Redo — returns the next snapshot or null */
  redo: () => EditorSnapshot | null
  /** Can we undo? */
  canUndo: () => boolean
  /** Can we redo? */
  canRedo: () => boolean
  /** Number of undo steps available */
  undoCount: () => number
  /** Number of redo steps available */
  redoCount: () => number
  /** Reset history (e.g. on clip load) with an initial snapshot */
  reset: (initial: EditorSnapshot) => void
}

/**
 * React hook that provides undo/redo history for editor snapshots.
 *
 * Returns a stable object whose methods mutate internal refs (no re-render
 * on push).  The caller re-renders on undo/redo because it applies the
 * restored snapshot via setState calls.
 */
export function useEditorHistory(): EditorHistory {
  // Stack of snapshots; index points at current
  const stackRef = useRef<EditorSnapshot[]>([])
  const indexRef = useRef(-1)

  const reset = useCallback((initial: EditorSnapshot) => {
    stackRef.current = [initial]
    indexRef.current = 0
  }, [])

  const push = useCallback((snapshot: EditorSnapshot) => {
    const stack = stackRef.current
    const idx = indexRef.current

    // Skip if identical to current
    if (idx >= 0 && idx < stack.length && snapshotEqual(stack[idx], snapshot)) return

    // Truncate any redo history beyond current position
    const newStack = stack.slice(0, idx + 1)
    newStack.push(snapshot)

    // Enforce max size — drop oldest entries
    if (newStack.length > MAX_HISTORY) {
      const excess = newStack.length - MAX_HISTORY
      newStack.splice(0, excess)
    }

    stackRef.current = newStack
    indexRef.current = newStack.length - 1
  }, [])

  const undo = useCallback((): EditorSnapshot | null => {
    if (indexRef.current <= 0) return null
    indexRef.current -= 1
    return stackRef.current[indexRef.current]
  }, [])

  const redo = useCallback((): EditorSnapshot | null => {
    if (indexRef.current >= stackRef.current.length - 1) return null
    indexRef.current += 1
    return stackRef.current[indexRef.current]
  }, [])

  const canUndo = useCallback(() => indexRef.current > 0, [])
  const canRedo = useCallback(() => indexRef.current < stackRef.current.length - 1, [])
  const undoCount = useCallback(() => indexRef.current, [])
  const redoCount = useCallback(() => stackRef.current.length - 1 - indexRef.current, [])

  return { push, undo, redo, canUndo, canRedo, undoCount, redoCount, reset }
}
