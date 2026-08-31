import type { Vod } from '../types'

export interface SessionResourceState {
  loaded: boolean
  loadedAt: number | null
  refreshedAt: number | null
  accountId: string | null
  loading: boolean
  refreshing: boolean
}

export const VOD_BACKGROUND_REFRESH_MS = 2 * 60 * 1000

export function emptySessionResourceState(accountId: string | null = null): SessionResourceState {
  return {
    loaded: false,
    loadedAt: null,
    refreshedAt: null,
    accountId,
    loading: false,
    refreshing: false,
  }
}

export function resourceIsCurrent(
  resource: SessionResourceState,
  accountId: string | null,
): boolean {
  return resource.loaded && resource.accountId === accountId
}

export function shouldLoadResource(
  resource: SessionResourceState,
  accountId: string | null,
  force = false,
): boolean {
  return force || !resourceIsCurrent(resource, accountId)
}

export function shouldRefreshVods(
  resource: SessionResourceState,
  accountId: string,
  now = Date.now(),
  maxAgeMs = VOD_BACKGROUND_REFRESH_MS,
): boolean {
  if (!resourceIsCurrent(resource, accountId)) return true
  if (resource.refreshedAt === null) return true
  return now - resource.refreshedAt >= maxAgeMs
}

export function indexVods(
  current: Record<string, Vod>,
  vods: Vod[],
): Record<string, Vod> {
  const next = { ...current }
  for (const vod of vods) next[vod.id] = vod
  return next
}

export function reconcileVods(current: Vod[], incoming: Vod[]): Vod[] {
  const currentById = new Map(current.map(vod => [vod.id, vod]))
  return incoming.map(vod => {
    const previous = currentById.get(vod.id)
    if (!previous) return vod

    return {
      ...vod,
      download_progress: vod.download_status === 'downloading'
        && previous.download_status === 'downloading'
        ? Math.max(vod.download_progress ?? 0, previous.download_progress ?? 0)
        : vod.download_progress,
      analysis_progress: vod.analysis_status === 'analyzing'
        && previous.analysis_status === 'analyzing'
        ? Math.max(vod.analysis_progress ?? 0, previous.analysis_progress ?? 0)
        : vod.analysis_progress,
    }
  })
}
