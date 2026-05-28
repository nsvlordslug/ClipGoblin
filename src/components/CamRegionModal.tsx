import { useEffect, useRef, useState } from 'react'
import CamRegionSetter, { type RegionNorm } from './CamRegionSetter'

type Props = {
  /** Source video URL (already converted via convertFileSrc). */
  videoSrc: string
  /** Time (in seconds) to seek the preview to on open. */
  startTime: number
  /** Initial region (normalized 0..1) — drag starts from here. */
  initial: RegionNorm
  /** Fired with the final region when the user clicks Save. */
  onSave: (r: RegionNorm) => void
  /** Fired when the user clicks Cancel, presses Esc, or clicks the backdrop. */
  onCancel: () => void
}

/**
 * Full-screen modal for selecting the cam region on the FULL UNCROPPED source
 * frame. The editor's main player shows the source already cropped to the
 * clip's vertical aspect (e.g. 9:16), which hides corners of the original
 * 16:9 frame. This modal shows the source at its intrinsic aspect ratio so
 * the user can mark any region of the original frame.
 */
export default function CamRegionModal({ videoSrc, startTime, initial, onSave, onCancel }: Props) {
  const videoRef = useRef<HTMLVideoElement | null>(null)
  const [videoRect, setVideoRect] = useState<DOMRect | null>(null)
  const [videoReady, setVideoReady] = useState(false)

  // Pause at startTime once metadata is loaded.
  useEffect(() => {
    const v = videoRef.current
    if (!v) return
    const onLoaded = () => {
      try { v.currentTime = startTime } catch { /* ignore seek failures */ }
      v.pause()
      setVideoReady(true)
    }
    if (v.readyState >= 1) {
      onLoaded()
    } else {
      v.addEventListener('loadedmetadata', onLoaded)
      return () => v.removeEventListener('loadedmetadata', onLoaded)
    }
  }, [startTime])

  // Measure the video element's actual rendered rect (post-letterboxing).
  useEffect(() => {
    if (!videoReady) return
    const measure = () => {
      if (videoRef.current) {
        setVideoRect(videoRef.current.getBoundingClientRect())
      }
    }
    measure()
    const id = setInterval(measure, 200)  // catch any layout settling
    window.addEventListener('resize', measure)
    return () => {
      clearInterval(id)
      window.removeEventListener('resize', measure)
    }
  }, [videoReady])

  return (
    <div
      className="fixed inset-0 z-50 bg-black/90 flex items-center justify-center p-6"
      onClick={(e) => { if (e.target === e.currentTarget) onCancel() }}
    >
      <div className="relative">
        <video
          ref={videoRef}
          src={videoSrc}
          style={{ maxWidth: '90vw', maxHeight: '85vh', display: 'block' }}
          preload="auto"
          muted
        />
        {videoReady && videoRect && (
          <CamRegionSetter
            initial={initial}
            containerRect={videoRect}
            onChange={() => { /* no-op for v1 */ }}
            onSave={onSave}
            onCancel={onCancel}
          />
        )}
        {!videoReady && (
          <div className="absolute inset-0 flex items-center justify-center text-slate-300 text-sm">
            Loading source...
          </div>
        )}
      </div>
    </div>
  )
}
