import { useRef, useState, useEffect, useCallback } from 'react'
import { Move, Maximize2 } from 'lucide-react'

export interface FacecamSettings {
  pipX: number; pipY: number; pipW: number; pipH: number
  splitRatio: number
  cropX: number; cropY: number; cropW: number; cropH: number
}

export const DEFAULT_FACECAM: FacecamSettings = {
  pipX: 68, pipY: 65, pipW: 28, pipH: 28,
  splitRatio: 0.6,
  cropX: 0, cropY: 0.6, cropW: 0.4, cropH: 0.4,
}

// ── Snap targets and logic ──

const SNAP_THRESHOLD = 4 // percent — how close before snapping

interface SnapGuide { axis: 'x' | 'y'; position: number; label: string }

function getSnapTargets(w: number, h: number): { x: number[]; y: number[] } {
  const pad = 5 // safe margin %
  return {
    x: [pad, 50 - w / 2, 100 - w - pad], // left, center, right
    y: [pad, 50 - h / 2, 100 - h - pad], // top, center, bottom
  }
}

function snapValue(val: number, targets: number[]): { snapped: number; guide: number | null } {
  for (const t of targets) {
    if (Math.abs(val - t) < SNAP_THRESHOLD) return { snapped: t, guide: t }
  }
  return { snapped: val, guide: null }
}

/** Compute subtitle collision: does captionY% overlap the facecam? */
export function computeSubtitleCollision(
  captionY: number,
  layout: string,
  settings: FacecamSettings,
): { collides: boolean; safeY: number } {
  if (layout === 'pip') {
    const pipTop = settings.pipY
    const pipBottom = settings.pipY + settings.pipH
    if (captionY > pipTop - 5 && captionY < pipBottom + 3) {
      // Collision — suggest moving above the pip
      return { collides: true, safeY: Math.max(5, pipTop - 8) }
    }
  }
  if (layout === 'split') {
    const splitLine = settings.splitRatio * 100
    if (captionY > splitLine - 5) {
      return { collides: true, safeY: Math.max(5, splitLine - 10) }
    }
  }
  return { collides: false, safeY: captionY }
}

// ── Components ──

interface Props {
  layout: 'split' | 'pip'
  settings: FacecamSettings
  onChange: (s: FacecamSettings) => void
}

export default function FacecamEditor({ layout, settings, onChange }: Props) {
  const update = (patch: Partial<FacecamSettings>) => onChange({ ...settings, ...patch })

  if (layout === 'split') {
    return (
      <div className="space-y-2 mt-3">
        <div className="flex items-center gap-2">
          <span className="text-[10px] text-slate-500 w-20">Split ratio</span>
          <input type="range" min={30} max={80} step={5} value={Math.round(settings.splitRatio * 100)}
            onChange={e => update({ splitRatio: parseInt(e.target.value) / 100 })}
            className="flex-1 h-1 accent-violet-500 cursor-pointer" />
          <span className="text-[10px] text-slate-500 font-mono w-10 text-right">{Math.round(settings.splitRatio * 100)}%</span>
        </div>
        <p className="text-[9px] text-slate-600">
          Game {Math.round(settings.splitRatio * 100)}% top — Facecam {Math.round((1 - settings.splitRatio) * 100)}% bottom. Drag the divider on the preview.
        </p>
      </div>
    )
  }

  return (
    <div className="space-y-2 mt-3">
      <div className="flex items-center gap-2">
        <Move className="w-3 h-3 text-slate-500 shrink-0" />
        <span className="text-[10px] text-slate-500">Drag facecam on the preview. It snaps to corners and edges.</span>
      </div>
      <div className="grid grid-cols-2 gap-2">
        <div>
          <label className="text-[9px] text-slate-500">Size</label>
          <input type="range" min={15} max={45} step={1} value={settings.pipW}
            onChange={e => update({ pipW: parseInt(e.target.value), pipH: parseInt(e.target.value) })}
            className="w-full h-1 accent-violet-500 cursor-pointer" />
          <span className="text-[9px] text-slate-500 font-mono">{settings.pipW}%</span>
        </div>
        <div>
          <label className="text-[9px] text-slate-500">Quick place</label>
          <div className="grid grid-cols-2 gap-1 mt-0.5">
            {[
              { label: 'TL', x: 5, y: 5 },
              { label: 'TR', x: 95 - settings.pipW, y: 5 },
              { label: 'BL', x: 5, y: 95 - settings.pipH },
              { label: 'BR', x: 95 - settings.pipW, y: 95 - settings.pipH },
            ].map(c => (
              <button key={c.label} onClick={() => update({ pipX: c.x, pipY: c.y })}
                className={`px-1 py-0.5 rounded text-[8px] border cursor-pointer transition-colors ${
                  Math.abs(settings.pipX - c.x) < 8 && Math.abs(settings.pipY - c.y) < 8
                    ? 'bg-violet-600/20 text-violet-400 border-violet-500/40'
                    : 'bg-surface-900 text-slate-500 border-surface-600 hover:text-white'
                }`}>
                {c.label}
              </button>
            ))}
          </div>
        </div>
      </div>
    </div>
  )
}

/** Draggable PiP overlay with snap-to-edge behavior and alignment guides. */
export function DraggablePipOverlay({ settings, onChange, frameWidth, frameHeight }: {
  settings: FacecamSettings; onChange: (s: FacecamSettings) => void
  frameWidth: number; frameHeight: number
}) {
  const [dragging, setDragging] = useState(false)
  const [guides, setGuides] = useState<SnapGuide[]>([])
  const startRef = useRef({ mx: 0, my: 0, ox: 0, oy: 0 })

  const snapTargets = getSnapTargets(settings.pipW, settings.pipH)

  const onDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault(); e.stopPropagation()
    setDragging(true)
    startRef.current = { mx: e.clientX, my: e.clientY, ox: settings.pipX, oy: settings.pipY }
  }, [settings.pipX, settings.pipY])

  useEffect(() => {
    if (!dragging) return
    const onMove = (e: MouseEvent) => {
      const dx = ((e.clientX - startRef.current.mx) / frameWidth) * 100
      const dy = ((e.clientY - startRef.current.my) / frameHeight) * 100
      const rawX = Math.max(2, Math.min(98 - settings.pipW, startRef.current.ox + dx))
      const rawY = Math.max(2, Math.min(98 - settings.pipH, startRef.current.oy + dy))

      const sx = snapValue(rawX, snapTargets.x)
      const sy = snapValue(rawY, snapTargets.y)

      // Build active guides
      const activeGuides: SnapGuide[] = []
      if (sx.guide !== null) activeGuides.push({ axis: 'x', position: sx.snapped, label: '' })
      if (sy.guide !== null) activeGuides.push({ axis: 'y', position: sy.snapped, label: '' })
      setGuides(activeGuides)

      onChange({ ...settings, pipX: Math.round(sx.snapped), pipY: Math.round(sy.snapped) })
    }
    const onUp = () => { setDragging(false); setGuides([]) }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    return () => { window.removeEventListener('mousemove', onMove); window.removeEventListener('mouseup', onUp) }
  }, [dragging, settings, onChange, frameWidth, frameHeight, snapTargets])

  return (
    <>
      {/* Alignment guides */}
      {guides.map((g, i) => (
        <div key={i} className="absolute pointer-events-none z-[8]"
          style={g.axis === 'x'
            ? { left: `${g.position + settings.pipW / 2}%`, top: 0, bottom: 0, width: 1, background: 'rgba(139,92,246,0.5)' }
            : { top: `${g.position + settings.pipH / 2}%`, left: 0, right: 0, height: 1, background: 'rgba(139,92,246,0.5)' }
          } />
      ))}

      {/* PiP region */}
      <div className="absolute z-[9] cursor-move group/pip"
        style={{ left: `${settings.pipX}%`, top: `${settings.pipY}%`, width: `${settings.pipW}%`, height: `${settings.pipH}%` }}
        onMouseDown={onDown}>
        <div className={`w-full h-full rounded-lg overflow-hidden border-2 transition-colors ${
          dragging ? 'border-violet-400' : 'border-white/40 group-hover/pip:border-white/70'
        }`}
          style={{ background: 'linear-gradient(135deg, #1a1a3a 0%, #2a1a4a 100%)' }}>
          <div className="w-full h-full flex items-center justify-center">
            <span className="text-[8px] text-white/50 font-mono">FACECAM</span>
          </div>
        </div>
        {/* Resize handle */}
        <div className="absolute -bottom-1 -right-1 w-3 h-3 bg-white/60 rounded-full opacity-0 group-hover/pip:opacity-100 cursor-nwse-resize"
          onMouseDown={e => {
            e.preventDefault(); e.stopPropagation()
            const startW = settings.pipW; const startMx = e.clientX
            const onMove = (ev: MouseEvent) => {
              const dw = ((ev.clientX - startMx) / frameWidth) * 100
              const nw = Math.max(15, Math.min(45, Math.round(startW + dw)))
              onChange({ ...settings, pipW: nw, pipH: nw })
            }
            const onUp = () => { window.removeEventListener('mousemove', onMove); window.removeEventListener('mouseup', onUp) }
            window.addEventListener('mousemove', onMove); window.addEventListener('mouseup', onUp)
          }}>
          <Maximize2 className="w-2 h-2 text-white/80" />
        </div>
      </div>
    </>
  )
}

/** Draggable split divider. */
export function DraggableSplitDivider({ settings, onChange, frameHeight }: {
  settings: FacecamSettings; onChange: (s: FacecamSettings) => void; frameHeight: number
}) {
  const [dragging, setDragging] = useState(false)

  useEffect(() => {
    if (!dragging) return
    const onMove = (e: MouseEvent) => {
      const frames = document.querySelectorAll('[class*="transition-all"][class*="aspect-"]')
      if (frames.length === 0) return
      const rect = frames[0].getBoundingClientRect()
      const pct = (e.clientY - rect.top) / rect.height
      onChange({ ...settings, splitRatio: Math.round(Math.max(0.3, Math.min(0.8, pct)) * 20) / 20 })
    }
    const onUp = () => setDragging(false)
    window.addEventListener('mousemove', onMove); window.addEventListener('mouseup', onUp)
    return () => { window.removeEventListener('mousemove', onMove); window.removeEventListener('mouseup', onUp) }
  }, [dragging, settings, onChange, frameHeight])

  return (
    <div className="absolute left-0 right-0 z-[9] cursor-row-resize group/split"
      style={{ top: `${settings.splitRatio * 100}%`, height: 12, marginTop: -6 }}
      onMouseDown={e => { e.preventDefault(); e.stopPropagation(); setDragging(true) }}>
      <div className="absolute left-[10%] right-[10%] top-1/2 -translate-y-1/2 h-[2px] bg-white/40 group-hover/split:bg-white/70 rounded-full transition-colors" />
      <div className="absolute left-1/2 -translate-x-1/2 top-1/2 -translate-y-1/2 w-6 h-3 bg-white/30 group-hover/split:bg-white/60 rounded-full flex items-center justify-center transition-colors">
        <span className="text-[6px] text-white/80">=</span>
      </div>
    </div>
  )
}
