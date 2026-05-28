import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'

type RegionNorm = { x: number; y: number; w: number; h: number }
type FitMode = 'fit' | 'fill' | 'stretch'

type Props = {
  vodId: string
  clipId: string
  /** VOD-level region (parsed from `vod.cam_region_norm`). Null = no region set. */
  vodRegion: RegionNorm | null
  /** Per-clip override (parsed from `clip.cam_region_norm_override`). Null = no override. */
  clipOverride: RegionNorm | null
  /** Current fit mode for this clip. Null/undefined treated as layout default. */
  fitMode: FitMode | null
  /** Whether the current layout uses a cam slot (false for GameplayFocus). */
  layoutHasCamSlot: boolean
  /** Active layout kind. PiP slots are tall+narrow so Fit gives tiny
   *  letterboxed output; we drop the Fit option for PiP and default to Fill. */
  layoutKind: 'pip' | 'split' | 'other'
  /** Called when the user clicks "Set region..." — parent should enter edit mode on the source player. */
  onEnterVodEditMode: () => void
  /** Called when the user clicks "Override for this clip..." — parent enters override-edit mode. */
  onEnterClipOverrideMode: () => void
  /** Called after any DB-mutating action so parent can re-fetch state. */
  onChanged: () => void
}

function regionLabel(r: RegionNorm | null): string {
  if (!r) return 'Not set'
  return `${Math.round(r.x * 100)}%, ${Math.round(r.y * 100)}% · ${Math.round(r.w * 100)}×${Math.round(r.h * 100)}%`
}

export default function CamRegionRow({
  vodId, clipId, vodRegion, clipOverride, fitMode, layoutHasCamSlot, layoutKind,
  onEnterVodEditMode, onEnterClipOverrideMode, onChanged,
}: Props) {
  const [allowOverride, setAllowOverride] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    invoke<boolean>('get_allow_per_clip_override')
      .then(setAllowOverride)
      .catch(() => setAllowOverride(false))
  }, [])

  const clearVod = async () => {
    if (busy) return
    setBusy(true); setError(null)
    try {
      await invoke('clear_vod_cam_region', { vodId })
      onChanged()
    } catch (e) { setError(String(e)) } finally { setBusy(false) }
  }

  const clearOverride = async () => {
    if (busy) return
    setBusy(true); setError(null)
    try {
      await invoke('clear_clip_cam_region_override', { clipId })
      onChanged()
    } catch (e) { setError(String(e)) } finally { setBusy(false) }
  }

  const setFit = async (mode: FitMode) => {
    if (busy) return
    setBusy(true); setError(null)
    try {
      await invoke('set_clip_fit_mode', { clipId, mode })
      onChanged()
    } catch (e) { setError(String(e)) } finally { setBusy(false) }
  }

  // Layout-aware default: Fill for PiP, Fit elsewhere. Mirrors the backend
  // resolver in commands/export.rs.
  const layoutDefault: FitMode = layoutKind === 'pip' ? 'fill' : 'fit'
  // If layout is PiP but a stored 'fit' lingers from when this clip was Split,
  // treat as the layout default (Fill) since 'fit' isn't offered in PiP.
  const effectiveFit: FitMode =
    layoutKind === 'pip' && fitMode === 'fit'
      ? 'fill'
      : (fitMode ?? layoutDefault)
  const fitDisabledReason = !layoutHasCamSlot
    ? 'No cam slot in this layout'
    : (!vodRegion && !clipOverride)
      ? 'Set a cam region first'
      : null

  return (
    <div className="space-y-2">
      {/* VOD-level region row */}
      <div className="bg-surface-800 border border-surface-700 rounded p-2">
        <div className="text-[10px] uppercase tracking-wider text-violet-300 font-semibold mb-1">
          Cam region <span className="text-slate-500 font-normal normal-case tracking-normal">(per-VOD)</span>
        </div>
        <div className="flex items-center gap-2 text-xs text-slate-200">
          <span className="flex-1 font-mono">{regionLabel(vodRegion)}</span>
          <button
            type="button"
            disabled={busy || !layoutHasCamSlot}
            onClick={onEnterVodEditMode}
            className="px-2 py-1 rounded bg-surface-700 hover:bg-surface-600 disabled:opacity-40 cursor-pointer"
            title={!layoutHasCamSlot ? 'No cam slot in this layout' : 'Set the cam region by dragging on the source player'}
          >
            Set region&hellip;
          </button>
          {vodRegion && (
            <button
              type="button"
              disabled={busy}
              onClick={clearVod}
              className="px-2 py-1 rounded bg-red-500/20 hover:bg-red-500/30 text-red-300 disabled:opacity-40 cursor-pointer"
              title="Clear cam region for this VOD"
            >
              Clear
            </button>
          )}
        </div>
        <div className="text-[10px] text-slate-500 mt-1">Same region used by every clip in this VOD.</div>
      </div>

      {/* Fit mode dropdown */}
      <div className="flex items-center gap-2">
        <span className="text-[10px] uppercase tracking-wider text-amber-300 font-semibold flex-1">
          Fit mode <span className="text-slate-500 font-normal normal-case tracking-normal">(per-clip)</span>
        </span>
        <select
          value={effectiveFit}
          disabled={busy || !!fitDisabledReason}
          onChange={(e) => setFit(e.target.value as FitMode)}
          className="px-2 py-1 text-xs bg-surface-800 border border-surface-700 text-slate-200 rounded disabled:opacity-40"
          title={fitDisabledReason ?? 'How the source region fits into the cam slot'}
        >
          {layoutKind === 'pip' ? (
            <>
              <option value="fill">Fill (default)</option>
              <option value="stretch">Stretch</option>
            </>
          ) : (
            <>
              <option value="fit">Fit (default)</option>
              <option value="fill">Fill</option>
              <option value="stretch">Stretch</option>
            </>
          )}
        </select>
      </div>

      {/* Per-clip override sub-row — only when the global toggle is on */}
      {allowOverride && layoutHasCamSlot && (
        <div className="bg-surface-800/70 border border-surface-700 rounded p-2">
          <div className="text-[10px] uppercase tracking-wider text-emerald-300 font-semibold mb-1">
            Per-clip override
          </div>
          <div className="flex items-center gap-2 text-xs text-slate-300">
            <span className="flex-1 font-mono">
              {clipOverride ? regionLabel(clipOverride) : 'Using VOD default'}
            </span>
            <button
              type="button"
              disabled={busy}
              onClick={onEnterClipOverrideMode}
              className="px-2 py-1 rounded bg-surface-700 hover:bg-surface-600 disabled:opacity-40 cursor-pointer"
            >
              {clipOverride ? 'Edit…' : 'Override…'}
            </button>
            {clipOverride && (
              <button
                type="button"
                disabled={busy}
                onClick={clearOverride}
                className="px-2 py-1 rounded bg-surface-700 hover:bg-surface-600 text-slate-200 disabled:opacity-40 cursor-pointer"
                title="Use VOD default region instead"
              >
                Reset to VOD
              </button>
            )}
          </div>
        </div>
      )}

      {error && <div className="text-xs text-red-400">{error}</div>}
    </div>
  )
}
