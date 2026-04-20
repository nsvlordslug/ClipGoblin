import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'

export interface StreamStatus {
  is_live: boolean
  viewer_count: number
  game_name: string | null
  title: string | null
  started_at: string | null
}

const OFFLINE: StreamStatus = {
  is_live: false,
  viewer_count: 0,
  game_name: null,
  title: null,
  started_at: null,
}

const POLL_INTERVAL_MS = 60_000

/**
 * Polls Twitch Helix /streams every 60s for the given channel.
 * Pass `null` to disable the poll (e.g. logged out). Returns the latest
 * status. Errors are swallowed so one bad fetch doesn't break the UI.
 */
export function useStreamStatus(channelId: string | null | undefined): StreamStatus {
  const [status, setStatus] = useState<StreamStatus>(OFFLINE)

  useEffect(() => {
    if (!channelId) {
      setStatus(OFFLINE)
      return
    }

    let cancelled = false

    const fetchOnce = async () => {
      try {
        const s = await invoke<StreamStatus>('get_stream_status', { channelId })
        if (!cancelled) setStatus(s)
      } catch {
        // Swallow errors — channel may be offline, token expired, or running in browser.
        if (!cancelled) setStatus(OFFLINE)
      }
    }

    fetchOnce()
    const interval = setInterval(fetchOnce, POLL_INTERVAL_MS)
    return () => {
      cancelled = true
      clearInterval(interval)
    }
  }, [channelId])

  return status
}

/** "1.2k" / "12.4k" / "1.3M" compact viewer-count formatter. */
export function formatViewerCount(n: number): string {
  if (n < 1000) return String(n)
  if (n < 10_000) return `${(n / 1000).toFixed(1).replace(/\.0$/, '')}k`
  if (n < 1_000_000) return `${Math.round(n / 1000)}k`
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`
}
