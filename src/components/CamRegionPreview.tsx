import { useEffect, useRef } from 'react'

type RegionNorm = { x: number; y: number; w: number; h: number }
type FitMode = 'fit' | 'fill' | 'stretch'

type Props = {
  /** Ref to the main player's underlying <video> element. We draw frames
   * from THIS element to a canvas, avoiding a second decoder on the same
   * source (which Tauri's WebView doesn't reliably support). */
  sourceVideoRef: React.MutableRefObject<HTMLVideoElement | null>
  /** Normalized 0..1 source-frame region to show. */
  region: RegionNorm
  /** How the region maps into the slot. */
  fitMode: FitMode
}

/**
 * Live preview of the source-region cam content, rendered inside the editor's
 * cam slot. Uses a canvas that draws frames from the MAIN player's video
 * element (via shared ref) — no second decoder needed.
 *
 * Render loop: requestAnimationFrame copies the source's region rectangle to
 * the canvas with the chosen fit mode applied. The canvas auto-resizes to
 * fill its parent slot.
 */
export default function CamRegionPreview({ sourceVideoRef, region, fitMode }: Props) {
  const wrapperRef = useRef<HTMLDivElement | null>(null)
  const canvasRef = useRef<HTMLCanvasElement | null>(null)
  const rafIdRef = useRef<number | null>(null)

  // Single rAF loop: draws the current frame from the source video to canvas.
  useEffect(() => {
    const draw = () => {
      const canvas = canvasRef.current
      const wrapper = wrapperRef.current
      const source = sourceVideoRef.current
      if (!canvas || !wrapper || !source) {
        rafIdRef.current = requestAnimationFrame(draw)
        return
      }

      // Resize canvas backing store to match its CSS pixel size (so 1 CSS px
      // == 1 canvas px). DPR is handled by leaving the CSS size alone.
      const rect = wrapper.getBoundingClientRect()
      const cw = Math.max(1, Math.floor(rect.width))
      const ch = Math.max(1, Math.floor(rect.height))
      if (canvas.width !== cw) canvas.width = cw
      if (canvas.height !== ch) canvas.height = ch

      const ctx = canvas.getContext('2d')
      if (!ctx) {
        rafIdRef.current = requestAnimationFrame(draw)
        return
      }

      const vw = source.videoWidth
      const vh = source.videoHeight
      if (vw === 0 || vh === 0 || source.readyState < 2) {
        // Metadata not loaded yet -- clear to transparent and try again next frame.
        ctx.clearRect(0, 0, cw, ch)
        rafIdRef.current = requestAnimationFrame(draw)
        return
      }

      // Source rectangle (in source video pixel coords).
      const sx = region.x * vw
      const sy = region.y * vh
      const sw = region.w * vw
      const sh = region.h * vh

      // Destination rectangle on canvas, per fit mode.
      let dx = 0, dy = 0, dw = cw, dh = ch
      if (fitMode !== 'stretch') {
        const sourceAspect = sw / sh
        const slotAspect = cw / ch
        if (fitMode === 'fit') {
          // Letterbox: preserve aspect, fit entirely inside slot.
          if (sourceAspect > slotAspect) {
            dw = cw
            dh = cw / sourceAspect
            dx = 0
            dy = (ch - dh) / 2
          } else {
            dh = ch
            dw = ch * sourceAspect
            dx = (cw - dw) / 2
            dy = 0
          }
        } else {
          // Fill: preserve aspect, crop overflow.
          if (sourceAspect > slotAspect) {
            dh = ch
            dw = ch * sourceAspect
            dx = (cw - dw) / 2
            dy = 0
          } else {
            dw = cw
            dh = cw / sourceAspect
            dx = 0
            dy = (ch - dh) / 2
          }
        }
      }

      ctx.clearRect(0, 0, cw, ch)
      try {
        ctx.drawImage(source, sx, sy, sw, sh, dx, dy, dw, dh)
      } catch {
        // Cross-origin / decoder hiccup — skip this frame.
      }

      rafIdRef.current = requestAnimationFrame(draw)
    }

    rafIdRef.current = requestAnimationFrame(draw)
    return () => {
      if (rafIdRef.current !== null) cancelAnimationFrame(rafIdRef.current)
    }
  }, [sourceVideoRef, region, fitMode])

  return (
    <div
      ref={wrapperRef}
      className="absolute inset-0 overflow-hidden pointer-events-none"
      style={{ zIndex: 5 }}
    >
      <canvas
        ref={canvasRef}
        className="absolute inset-0"
        style={{ width: '100%', height: '100%', display: 'block' }}
      />
    </div>
  )
}
