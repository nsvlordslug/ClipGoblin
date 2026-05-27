import { useEffect, useRef, useState } from 'react'

export type RegionNorm = { x: number; y: number; w: number; h: number }

type Props = {
  /** Initial region (normalized 0..1). Used as starting position when entering edit mode. */
  initial: RegionNorm
  /** The bounding rect of the underlying source video element (in CSS px). */
  containerRect: DOMRect
  /** Fired on every drag/resize while the user is interacting (no DB write yet). */
  onChange: (r: RegionNorm) => void
  /** Fired when user clicks Save. */
  onSave: (r: RegionNorm) => void
  /** Fired when user clicks Cancel or presses Esc. */
  onCancel: () => void
}

const MIN_DIM_NORM = 0.05  // matches Rust MIN_REGION_DIM

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v))

type Handle = 'move' | 'tl' | 'tr' | 'bl' | 'br' | 't' | 'b' | 'l' | 'r'

export default function CamRegionSetter({ initial, containerRect, onChange, onSave, onCancel }: Props) {
  const [region, setRegion] = useState<RegionNorm>(initial)
  const dragRef = useRef<{ handle: Handle; startX: number; startY: number; startR: RegionNorm } | null>(null)

  // Esc -> cancel
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel()
      if (e.key === 'Enter') onSave(region)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [region, onSave, onCancel])

  // Pixel rect of the region inside the container.
  const px = {
    x: region.x * containerRect.width,
    y: region.y * containerRect.height,
    w: region.w * containerRect.width,
    h: region.h * containerRect.height,
  }

  const onMouseDown = (handle: Handle) => (e: React.MouseEvent) => {
    e.preventDefault()
    e.stopPropagation()
    dragRef.current = { handle, startX: e.clientX, startY: e.clientY, startR: region }
  }

  // Listen for mousemove + mouseup on window so the drag continues outside the overlay.
  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (!dragRef.current) return
      const { handle, startX, startY, startR } = dragRef.current
      const dxNorm = (e.clientX - startX) / containerRect.width
      const dyNorm = (e.clientY - startY) / containerRect.height
      let { x, y, w, h } = startR
      switch (handle) {
        case 'move':
          x = clamp(startR.x + dxNorm, 0, 1 - startR.w)
          y = clamp(startR.y + dyNorm, 0, 1 - startR.h)
          break
        case 'tl': x = clamp(startR.x + dxNorm, 0, startR.x + startR.w - MIN_DIM_NORM); y = clamp(startR.y + dyNorm, 0, startR.y + startR.h - MIN_DIM_NORM); w = startR.x + startR.w - x; h = startR.y + startR.h - y; break
        case 'tr': y = clamp(startR.y + dyNorm, 0, startR.y + startR.h - MIN_DIM_NORM); w = clamp(startR.w + dxNorm, MIN_DIM_NORM, 1 - startR.x); h = startR.y + startR.h - y; break
        case 'bl': x = clamp(startR.x + dxNorm, 0, startR.x + startR.w - MIN_DIM_NORM); w = startR.x + startR.w - x; h = clamp(startR.h + dyNorm, MIN_DIM_NORM, 1 - startR.y); break
        case 'br': w = clamp(startR.w + dxNorm, MIN_DIM_NORM, 1 - startR.x); h = clamp(startR.h + dyNorm, MIN_DIM_NORM, 1 - startR.y); break
        case 't':  y = clamp(startR.y + dyNorm, 0, startR.y + startR.h - MIN_DIM_NORM); h = startR.y + startR.h - y; break
        case 'b':  h = clamp(startR.h + dyNorm, MIN_DIM_NORM, 1 - startR.y); break
        case 'l':  x = clamp(startR.x + dxNorm, 0, startR.x + startR.w - MIN_DIM_NORM); w = startR.x + startR.w - x; break
        case 'r':  w = clamp(startR.w + dxNorm, MIN_DIM_NORM, 1 - startR.x); break
      }
      const next = { x, y, w, h }
      setRegion(next)
      onChange(next)
    }
    const onUp = () => { dragRef.current = null }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    return () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
  }, [containerRect, onChange])

  // Overlay positioned absolutely over the source player container.
  return (
    <>
      {/* Dim layer outside the region — gives focus to the picked area */}
      <div className="absolute inset-0 pointer-events-none" style={{
        background: `linear-gradient(to right, rgba(0,0,0,0.45) 0, rgba(0,0,0,0.45) ${px.x}px, transparent ${px.x}px, transparent ${px.x + px.w}px, rgba(0,0,0,0.45) ${px.x + px.w}px)`,
      }} />
      {/* Top/bottom dim bands */}
      <div className="absolute pointer-events-none" style={{ left: px.x, top: 0, width: px.w, height: px.y, background: 'rgba(0,0,0,0.45)' }} />
      <div className="absolute pointer-events-none" style={{ left: px.x, top: px.y + px.h, width: px.w, bottom: 0, background: 'rgba(0,0,0,0.45)' }} />

      {/* The draggable rectangle itself */}
      <div
        className="absolute border-2 border-violet-400 bg-violet-400/10 cursor-move"
        style={{ left: px.x, top: px.y, width: px.w, height: px.h }}
        onMouseDown={onMouseDown('move')}
      >
        {/* Corner handles */}
        {(['tl','tr','bl','br'] as const).map(h => (
          <div
            key={h}
            className="absolute w-3 h-3 bg-violet-400 border border-white"
            style={{
              cursor: (h === 'tl' || h === 'br') ? 'nwse-resize' : 'nesw-resize',
              left: h.includes('l') ? -6 : undefined,
              right: h.includes('r') ? -6 : undefined,
              top: h.includes('t') ? -6 : undefined,
              bottom: h.includes('b') ? -6 : undefined,
            }}
            onMouseDown={onMouseDown(h)}
          />
        ))}
        {/* Edge handles */}
        {(['t','b','l','r'] as const).map(h => (
          <div
            key={h}
            className="absolute bg-violet-400/40"
            style={{
              cursor: (h === 't' || h === 'b') ? 'ns-resize' : 'ew-resize',
              top: h === 't' ? -3 : h === 'b' ? undefined : '20%',
              bottom: h === 'b' ? -3 : undefined,
              left: h === 'l' ? -3 : h === 'r' ? undefined : '20%',
              right: h === 'r' ? -3 : undefined,
              width: (h === 't' || h === 'b') ? '60%' : 6,
              height: (h === 'l' || h === 'r') ? '60%' : 6,
            }}
            onMouseDown={onMouseDown(h)}
          />
        ))}
      </div>

      {/* Save / Cancel toolbar pinned to bottom of the player container */}
      <div className="absolute left-0 right-0 bottom-0 px-3 py-2 bg-black/70 flex items-center gap-3 text-xs text-slate-200 z-10">
        <span className="text-violet-300">Drag the rectangle on the source. Press Enter to save, Esc to cancel.</span>
        <span className="ml-auto font-mono text-[10px] text-slate-400">
          {Math.round(region.x * 100)}%, {Math.round(region.y * 100)}% &middot; {Math.round(region.w * 100)}&times;{Math.round(region.h * 100)}%
        </span>
        <button
          type="button"
          onClick={() => onSave(region)}
          className="px-3 py-1 rounded bg-violet-500 hover:bg-violet-400 text-black font-semibold"
        >
          Save
        </button>
        <button
          type="button"
          onClick={onCancel}
          className="px-3 py-1 rounded bg-surface-700 hover:bg-surface-600 text-slate-200"
        >
          Cancel
        </button>
      </div>
    </>
  )
}
