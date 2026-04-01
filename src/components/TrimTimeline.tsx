import { useRef, useState, useCallback, useEffect } from 'react'
import { Zap, ChevronLeft, ChevronRight, SkipForward } from 'lucide-react'

// ── Types ──

export interface TimelineMarker {
  time: number
  type: 'hook' | 'event' | 'reaction' | 'payoff' | 'dead-air-end'
  label: string
  confidence: number
}

interface Props {
  startTime: number
  endTime: number
  originalStart: number
  originalEnd: number
  videoDuration: number
  currentTime?: number
  isPlaying?: boolean
  markers?: TimelineMarker[]
  suggestedHookStart?: number
  onChange: (start: number, end: number) => void
  onSeekTo?: (time: number) => void
}

function fmt(s: number) {
  const m = Math.floor(s / 60)
  const sec = Math.floor(s % 60)
  const ms = Math.floor((s % 1) * 10)
  return `${m}:${String(sec).padStart(2, '0')}.${ms}`
}

const MARKER_COLORS: Record<string, string> = {
  hook: '#8b5cf6', event: '#3b82f6', reaction: '#f59e0b',
  payoff: '#10b981', 'dead-air-end': '#64748b',
}

// ── Snap logic ──

const SNAP_THRESHOLD_PX = 12 // pixels — how close the cursor must be to snap

function findSnapTarget(
  time: number,
  markers: TimelineMarker[],
  toPercent: (t: number) => number,
  trackWidth: number,
): TimelineMarker | null {
  let best: TimelineMarker | null = null
  let bestDist = Infinity
  for (const m of markers) {
    const pxDist = Math.abs(toPercent(m.time) - toPercent(time)) / 100 * trackWidth
    if (pxDist < SNAP_THRESHOLD_PX && pxDist < bestDist) {
      bestDist = pxDist
      best = m
    }
  }
  return best
}

// ── Component ──

export default function TrimTimeline({
  startTime, endTime, originalStart, originalEnd,
  videoDuration, currentTime = 0, isPlaying = false,
  markers = [], suggestedHookStart, onChange, onSeekTo,
}: Props) {
  const trackRef = useRef<HTMLDivElement>(null)
  const trackInnerRef = useRef<HTMLDivElement>(null)
  const [dragging, setDragging] = useState<'start' | 'end' | 'playhead' | null>(null)
  const [hoverTime, setHoverTime] = useState<number | null>(null)
  const [snapTarget, setSnapTarget] = useState<TimelineMarker | null>(null)

  // View window with padding
  const padding = Math.max(5, (originalEnd - originalStart) * 0.15)
  const viewStart = Math.max(0, originalStart - padding)
  const viewEnd = Math.min(videoDuration, originalEnd + padding)
  const viewDuration = viewEnd - viewStart

  const toPercent = useCallback((t: number) => ((t - viewStart) / viewDuration) * 100, [viewStart, viewDuration])

  const toTime = useCallback((clientX: number) => {
    const el = trackInnerRef.current
    if (!el) return startTime
    const rect = el.getBoundingClientRect()
    const pct = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width))
    return viewStart + pct * viewDuration
  }, [viewStart, viewDuration, startTime])

  const getTrackWidth = () => trackInnerRef.current?.getBoundingClientRect().width || 300

  // ── Snap-aware time resolution ──
  const resolveSnap = useCallback((rawTime: number): number => {
    const snap = findSnapTarget(rawTime, markers, toPercent, getTrackWidth())
    setSnapTarget(snap)
    return snap ? snap.time : rawTime
  }, [markers, toPercent])

  // ── Drag handling ──
  const onHandleDown = (handle: 'start' | 'end') => (e: React.MouseEvent) => {
    e.preventDefault(); e.stopPropagation()
    setDragging(handle)
  }

  useEffect(() => {
    if (!dragging) return
    const onMove = (e: MouseEvent) => {
      const rawTime = toTime(e.clientX)
      if (dragging === 'playhead') {
        const clamped = Math.max(startTime, Math.min(rawTime, endTime))
        onSeekTo?.(clamped)
      } else {
        const snappedToMarker = resolveSnap(rawTime)
        const snapped = Math.round(snappedToMarker * 2) / 2
        if (dragging === 'start') {
          onChange(Math.max(0, Math.min(snapped, endTime - 3)), endTime)
          onSeekTo?.(Math.max(0, Math.min(snapped, endTime - 3)))
        } else {
          onChange(startTime, Math.min(videoDuration, Math.max(snapped, startTime + 3)))
        }
      }
    }
    const onUp = () => { setDragging(null); setSnapTarget(null) }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    return () => { window.removeEventListener('mousemove', onMove); window.removeEventListener('mouseup', onUp) }
  }, [dragging, toTime, resolveSnap, startTime, endTime, videoDuration, onChange, onSeekTo])

  // ── Hover tracking ──
  const onTrackMouseMove = useCallback((e: React.MouseEvent) => {
    if (dragging) return
    const t = toTime(e.clientX)
    setHoverTime(t)
    setSnapTarget(findSnapTarget(t, markers, toPercent, getTrackWidth()))
  }, [dragging, toTime, markers, toPercent])

  const onTrackMouseLeave = () => { setHoverTime(null); setSnapTarget(null) }

  // ── Click to seek (snaps to marker if close) ──
  const onTrackClick = (e: React.MouseEvent) => {
    if (dragging) return
    const rawTime = toTime(e.clientX)
    const snap = findSnapTarget(rawTime, markers, toPercent, getTrackWidth())
    const seekTime = snap ? snap.time : rawTime
    if (seekTime >= startTime && seekTime <= endTime) {
      onSeekTo?.(seekTime)
    }
  }

  // ── Quick actions ──
  const nudgeStart = (delta: number) => {
    const ns = Math.max(0, Math.min(startTime + delta, endTime - 3))
    onChange(ns, endTime); onSeekTo?.(ns)
  }
  const snapToHook = () => {
    if (suggestedHookStart != null) {
      const ns = Math.max(0, suggestedHookStart)
      onChange(ns, endTime); onSeekTo?.(ns)
    }
  }
  const snapToMarker = (type: string) => {
    const m = markers.filter(m => m.type === type).sort((a, b) => b.confidence - a.confidence)[0]
    if (m) { const ns = Math.max(0, m.time - 1); onChange(ns, endTime); onSeekTo?.(ns) }
  }
  const resetTrim = () => { onChange(originalStart, originalEnd); onSeekTo?.(originalStart) }

  const duration = endTime - startTime
  const hasHookSuggestion = suggestedHookStart != null && suggestedHookStart !== startTime
  const bestHookMarker = markers.filter(m => m.type === 'hook').sort((a, b) => b.confidence - a.confidence)[0]

  // Effective hover position (snaps to marker if close)
  const effectiveHover = hoverTime != null
    ? (snapTarget ? snapTarget.time : hoverTime)
    : null

  return (
    <div className="space-y-3">
      {/* ── Timeline ── */}
      <div className="relative pt-2 pb-5" ref={trackRef}>
        <div
          ref={trackInnerRef}
          className="h-11 bg-surface-900 rounded-lg relative cursor-crosshair border border-surface-600"
          onClick={onTrackClick}
          onMouseMove={onTrackMouseMove}
          onMouseLeave={onTrackMouseLeave}
        >
          {/* Inactive zones */}
          <div className="absolute inset-y-0 left-0 bg-black/50 rounded-l-lg"
            style={{ width: `${toPercent(startTime)}%` }} />
          <div className="absolute inset-y-0 right-0 bg-black/50 rounded-r-lg"
            style={{ width: `${100 - toPercent(endTime)}%` }} />

          {/* Active region border */}
          <div className="absolute inset-y-0 border-y-2 border-violet-500/50"
            style={{ left: `${toPercent(startTime)}%`, width: `${toPercent(endTime) - toPercent(startTime)}%` }} />

          {/* ── Markers ── */}
          {markers.map((m, i) => (
            <div key={i}
              className="absolute top-0 bottom-0 w-0.5 cursor-pointer group/marker z-[5]"
              style={{ left: `${toPercent(m.time)}%`, backgroundColor: MARKER_COLORS[m.type] || '#888' }}
              onClick={e => { e.stopPropagation(); onSeekTo?.(m.time) }}
              title={`${m.label} (${m.type})`}
            >
              <div className="absolute -top-1 left-1/2 -translate-x-1/2 w-2.5 h-2.5 rounded-full border-2 border-surface-800"
                style={{ backgroundColor: MARKER_COLORS[m.type] || '#888' }} />
              <div className="absolute bottom-full left-1/2 -translate-x-1/2 mb-1 px-1.5 py-0.5 bg-surface-700 text-[9px] text-white rounded whitespace-nowrap opacity-0 group-hover/marker:opacity-100 transition-opacity pointer-events-none z-40">
                {m.label}
              </div>
            </div>
          ))}

          {/* Suggested hook */}
          {hasHookSuggestion && (
            <div className="absolute top-0 bottom-0 w-1 cursor-pointer z-[6]"
              style={{ left: `${toPercent(suggestedHookStart!)}%` }}
              onClick={e => { e.stopPropagation(); snapToHook() }}
              title="AI suggested hook"
            >
              <div className="absolute inset-y-0 w-full bg-violet-400 animate-pulse" />
              <div className="absolute -top-1.5 left-1/2 -translate-x-1/2">
                <Zap className="w-3 h-3 text-violet-300" />
              </div>
            </div>
          )}

          {/* ── Hover indicator — distinct from playhead ── */}
          {effectiveHover != null && !dragging && effectiveHover >= viewStart && effectiveHover <= viewEnd && (
            <div className="absolute top-0 bottom-0 pointer-events-none z-[15]"
              style={{ left: `${toPercent(effectiveHover)}%` }}
            >
              {/* Dashed line — clearly different from the solid white playhead */}
              <div className="absolute inset-y-1 left-1/2 -translate-x-1/2 w-[1px]"
                style={{
                  background: snapTarget
                    ? MARKER_COLORS[snapTarget.type] || '#fff'
                    : 'rgba(255,255,255,0.35)',
                  boxShadow: snapTarget ? `0 0 6px ${MARKER_COLORS[snapTarget.type]}` : 'none',
                }} />
              {/* Hover time label */}
              <div className="absolute left-1/2 -translate-x-1/2 pointer-events-none whitespace-nowrap"
                style={{
                  top: -20,
                  fontSize: '8px',
                  fontFamily: 'monospace',
                  color: snapTarget ? MARKER_COLORS[snapTarget.type] : 'rgba(255,255,255,0.5)',
                  fontWeight: snapTarget ? 700 : 400,
                }}>
                {snapTarget ? `⬥ ${fmt(effectiveHover)}` : fmt(effectiveHover)}
              </div>
            </div>
          )}

          {/* ── Snap indicator — highlight when handle is near a marker ── */}
          {dragging && dragging !== 'playhead' && snapTarget && (
            <div className="absolute top-0 bottom-0 pointer-events-none z-[25]"
              style={{ left: `${toPercent(snapTarget.time)}%` }}
            >
              <div className="absolute inset-y-0 left-1/2 -translate-x-1/2 w-1 rounded-full animate-pulse"
                style={{ backgroundColor: MARKER_COLORS[snapTarget.type], opacity: 0.8 }} />
              <div className="absolute left-1/2 -translate-x-1/2 pointer-events-none whitespace-nowrap font-mono"
                style={{ top: -22, fontSize: '8px', color: MARKER_COLORS[snapTarget.type], fontWeight: 700 }}>
                SNAP
              </div>
            </div>
          )}

          {/* ── Playhead — bright cyan, unmistakable ── */}
          {currentTime >= viewStart && currentTime <= viewEnd && (
            <div className="absolute z-30"
              style={{
                left: `${toPercent(currentTime)}%`,
                top: -6, bottom: -2,
                transition: isPlaying ? 'left 0.08s linear' : 'none',
              }}
            >
              {/* Glow */}
              <div className="absolute inset-y-0 left-1/2 -translate-x-1/2 w-3"
                style={{ background: 'radial-gradient(ellipse, rgba(0,230,255,0.25) 0%, transparent 70%)' }} />

              {/* Main line — 3px, bright cyan-white */}
              <div className="absolute left-1/2 -translate-x-1/2 w-[3px] rounded-full"
                style={{
                  top: 6, bottom: 2,
                  background: 'linear-gradient(180deg, #00e6ff, #ffffff)',
                  boxShadow: '0 0 6px rgba(0,230,255,0.9), 0 0 14px rgba(0,230,255,0.4)',
                }} />

              {/* Head handle — round knob */}
              <div className="absolute left-1/2 -translate-x-1/2 w-[13px] h-[13px] rounded-full cursor-grab"
                style={{
                  top: -2,
                  background: 'linear-gradient(135deg, #00e6ff, #ffffff)',
                  border: '2px solid rgba(255,255,255,0.9)',
                  boxShadow: '0 0 10px rgba(0,230,255,0.8), 0 1px 4px rgba(0,0,0,0.5)',
                }}
                onMouseDown={e => { e.preventDefault(); e.stopPropagation(); setDragging('playhead') }} />

              {/* Time label — always visible */}
              <div className="absolute left-1/2 -translate-x-1/2 px-1.5 py-0.5 rounded whitespace-nowrap font-mono pointer-events-none"
                style={{
                  bottom: -19,
                  fontSize: '9px',
                  fontWeight: 700,
                  color: '#fff',
                  background: isPlaying ? 'rgba(0,180,220,0.9)' : 'rgba(20,20,30,0.95)',
                  border: `1px solid ${isPlaying ? 'rgba(0,230,255,0.6)' : 'rgba(0,230,255,0.3)'}`,
                  boxShadow: isPlaying ? '0 0 8px rgba(0,230,255,0.4)' : '0 2px 4px rgba(0,0,0,0.4)',
                }}>
                {fmt(currentTime)}
              </div>
            </div>
          )}

          {/* ── Trim handles (violet — distinct from cyan playhead) ── */}
          {/* Start handle */}
          <div className={`absolute top-0 bottom-0 w-4 cursor-col-resize z-[20] group/handle ${dragging === 'start' ? 'bg-violet-500/20' : ''}`}
            style={{ left: `calc(${toPercent(startTime)}% - 8px)` }}
            aria-label="Drag to adjust trim start"
            role="slider"
            onMouseDown={onHandleDown('start')}>
            <div className="absolute inset-y-0.5 left-1/2 -translate-x-1/2 w-[3px] bg-violet-400 rounded-full group-hover/handle:bg-violet-300 transition-colors" />
            <div className="absolute top-1/2 -translate-y-1/2 left-1/2 -translate-x-1/2 w-3.5 h-6 bg-violet-500/90 rounded border border-violet-300/70 opacity-0 group-hover/handle:opacity-100 transition-opacity"
              style={{ boxShadow: '0 0 6px rgba(139,92,246,0.5)' }} />
          </div>
          {/* End handle */}
          <div className={`absolute top-0 bottom-0 w-4 cursor-col-resize z-[20] group/handle ${dragging === 'end' ? 'bg-violet-500/20' : ''}`}
            style={{ left: `calc(${toPercent(endTime)}% - 8px)` }}
            aria-label="Drag to adjust trim end"
            role="slider"
            onMouseDown={onHandleDown('end')}>
            <div className="absolute inset-y-0.5 left-1/2 -translate-x-1/2 w-[3px] bg-violet-400 rounded-full group-hover/handle:bg-violet-300 transition-colors" />
            <div className="absolute top-1/2 -translate-y-1/2 left-1/2 -translate-x-1/2 w-3.5 h-6 bg-violet-500/90 rounded border border-violet-300/70 opacity-0 group-hover/handle:opacity-100 transition-opacity"
              style={{ boxShadow: '0 0 6px rgba(139,92,246,0.5)' }} />
          </div>
        </div>

        {/* Time labels */}
        <div className="flex justify-between mt-1 text-[10px] text-slate-500 font-mono tabular-nums">
          <span>{fmt(startTime)}</span>
          <span className={isPlaying ? 'text-cyan-300 font-semibold' : 'text-slate-400'}>
            {currentTime >= startTime && currentTime <= endTime
              ? `${fmt(currentTime - startTime)} / ${fmt(duration)}`
              : `${fmt(duration)} clip`}
          </span>
          <span>{fmt(endTime)}</span>
        </div>
      </div>

      {/* ── Quick actions ── */}
      <div className="flex items-center gap-1.5 flex-wrap">
        <button onClick={() => nudgeStart(-0.5)} title="Start 0.5s earlier"
          className="flex items-center gap-0.5 px-2 py-1 bg-surface-900 border border-surface-600 rounded text-xs text-slate-400 hover:text-white hover:border-surface-500 transition-colors cursor-pointer">
          <ChevronLeft className="w-3 h-3" /> 0.5s
        </button>
        <button onClick={() => nudgeStart(0.5)} title="Start 0.5s later"
          className="flex items-center gap-0.5 px-2 py-1 bg-surface-900 border border-surface-600 rounded text-xs text-slate-400 hover:text-white hover:border-surface-500 transition-colors cursor-pointer">
          0.5s <ChevronRight className="w-3 h-3" />
        </button>
        <button onClick={() => nudgeStart(1)} title="Start 1s later"
          className="flex items-center gap-0.5 px-2 py-1 bg-surface-900 border border-surface-600 rounded text-xs text-slate-400 hover:text-white hover:border-surface-500 transition-colors cursor-pointer">
          1s <ChevronRight className="w-3 h-3" />
        </button>

        <div className="w-px h-4 bg-surface-600 mx-1" />

        {hasHookSuggestion && (
          <button onClick={snapToHook}
            className="flex items-center gap-1 px-2 py-1 bg-violet-600/20 border border-violet-500/40 rounded text-xs text-violet-400 hover:bg-violet-600/30 transition-colors cursor-pointer">
            <Zap className="w-3 h-3" /> Snap to Hook
          </button>
        )}
        {markers.some(m => m.type === 'event') && (
          <button onClick={() => snapToMarker('event')}
            className="flex items-center gap-1 px-2 py-1 bg-surface-900 border border-blue-500/30 rounded text-xs text-blue-400 hover:border-blue-500/50 transition-colors cursor-pointer">
            <SkipForward className="w-3 h-3" /> Event
          </button>
        )}
        {markers.some(m => m.type === 'reaction') && (
          <button onClick={() => snapToMarker('reaction')}
            className="flex items-center gap-1 px-2 py-1 bg-surface-900 border border-amber-500/30 rounded text-xs text-amber-400 hover:border-amber-500/50 transition-colors cursor-pointer">
            <SkipForward className="w-3 h-3" /> Reaction
          </button>
        )}

        <div className="flex-1" />
        <button onClick={resetTrim}
          className="px-2 py-1 bg-surface-900 border border-surface-600 rounded text-xs text-slate-500 hover:text-slate-300 transition-colors cursor-pointer">
          Reset
        </button>
      </div>

      {/* Hook suggestion banner */}
      {hasHookSuggestion && (
        <div className="flex items-center gap-2 px-3 py-2 bg-violet-500/10 border border-violet-500/20 rounded-lg">
          <Zap className="w-4 h-4 text-violet-400 shrink-0" />
          <div className="flex-1 min-w-0">
            <p className="text-xs text-violet-300">
              Hook at <span className="font-mono font-bold">{fmt(suggestedHookStart!)}</span>
              {bestHookMarker && <span className="text-violet-400/60"> — {bestHookMarker.label}</span>}
            </p>
          </div>
          <button onClick={snapToHook}
            className="shrink-0 px-2.5 py-1 bg-violet-600 hover:bg-violet-500 text-white text-xs font-medium rounded transition-colors cursor-pointer">
            Use Hook Start
          </button>
        </div>
      )}
    </div>
  )
}
