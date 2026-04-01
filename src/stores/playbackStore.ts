import { create } from 'zustand'

/**
 * Centralized playback controller.
 * Ensures only ONE clip plays audio at a time across the entire app.
 *
 * Each ClipPlayer instance registers a pause callback on mount.
 * When any player starts playing, all other registered players are paused.
 */

type PauseFn = () => void

interface PlaybackState {
  /** ID of the currently playing player instance (not clip ID — multiple players may show the same clip) */
  activePlayerId: string | null
  /** Registry of all mounted player instances and their pause callbacks */
  players: Map<string, PauseFn>

  /** Called by a player when it starts playing. Pauses all others. */
  requestPlay: (playerId: string) => void
  /** Called by a player when it pauses or ends naturally. */
  notifyPause: (playerId: string) => void
  /** Pause everything — used when navigating to editor, changing pages, etc. */
  stopAll: () => void
  /** Register a player instance with its pause callback. Returns unregister function. */
  register: (playerId: string, pauseFn: PauseFn) => () => void
}

export const usePlaybackStore = create<PlaybackState>((set, get) => ({
  activePlayerId: null,
  players: new Map(),

  requestPlay: (playerId: string) => {
    const { players, activePlayerId } = get()
    // Pause the currently active player if it's different
    if (activePlayerId && activePlayerId !== playerId) {
      const pauseFn = players.get(activePlayerId)
      if (pauseFn) pauseFn()
    }
    set({ activePlayerId: playerId })
  },

  notifyPause: (playerId: string) => {
    const { activePlayerId } = get()
    if (activePlayerId === playerId) {
      set({ activePlayerId: null })
    }
  },

  stopAll: () => {
    const { players } = get()
    players.forEach(pauseFn => pauseFn())
    set({ activePlayerId: null })
  },

  register: (playerId: string, pauseFn: PauseFn) => {
    const { players } = get()
    players.set(playerId, pauseFn)
    // Return cleanup function
    return () => {
      const { players: current, activePlayerId } = get()
      current.delete(playerId)
      if (activePlayerId === playerId) {
        set({ activePlayerId: null })
      }
    }
  },
}))
