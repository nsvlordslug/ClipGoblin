import { useEffect, useRef, useState } from 'react'

type RegionNorm = { x: number; y: number; w: number; h: number }
type FitMode = 'fit' | 'fill' | 'stretch'

type Props = {
  /** Source video URL (already converted via convertFileSrc). */
  videoSrc: string
  /** Normalized 0..1 source-frame region to show. */
  region: RegionNorm
  /** How the region maps into the slot. */
  fitMode: FitMode
  /** Current playback time of the main player; preview seeks here. */
  currentTime: number
  /** Slot pixel dimensions (the visible cam-slot area in the editor preview). */
  slotWidth: number
  slotHeight: number
}

/**
 * Live preview of the source-region cam content, rendered inside the editor's
 * cam slot. CSS-positioned `<video>` element with calculated transform that
 * mirrors what ffmpeg does at export time:
 *
 * - Fit:     preserve aspect, letterbox with black bars
 * - Fill:    preserve aspect, crop overflow
 * - Stretch: distort to fill the slot
 *
 * The preview pauses at `currentTime` (synced with the main player). It does
 * not render the blur backdrop (Split) or the gameplay pass-through (PiP) -
 * those layout-specific effects are export-only. The preview just shows the
 * cropped source region centered/filled per the fit mode.
 */
export default function CamRegionPreview({
  videoSrc,
  region,
  fitMode,
  currentTime,
  slotWidth,
  slotHeight,
}: Props) {
  const videoRef = useRef<HTMLVideoElement | null>(null)
  const [intrinsic, setIntrinsic] = useState<{ w: number; h: number } | null>(null)

  // Capture the source's intrinsic dimensions once metadata loads.
  useEffect(() => {
    const v = videoRef.current
    if (!v) return
    const onMeta = () => {
      if (v.videoWidth > 0 && v.videoHeight > 0) {
        setIntrinsic({ w: v.videoWidth, h: v.videoHeight })
      }
    }
    if (v.readyState >= 1) onMeta()
    else {
      v.addEventListener('loadedmetadata', onMeta)
      return () => v.removeEventListener('loadedmetadata', onMeta)
    }
  }, [videoSrc])

  // Seek the preview to the main player's currentTime whenever it changes.
  useEffect(() => {
    const v = videoRef.current
    if (!v || !intrinsic) return
    // Only seek if delta is meaningful (avoid feedback loops + jitter).
    if (Math.abs(v.currentTime - currentTime) > 0.05) {
      try { v.currentTime = currentTime } catch { /* ignore */ }
    }
    // Keep preview paused -- main player drives playback timing.
    v.pause()
  }, [currentTime, intrinsic])

  // Calculate the video's CSS transform: position+size so that the slot shows
  // only the region, with the chosen fit mode applied.
  let videoStyle: React.CSSProperties = { display: 'none' }
  if (intrinsic && slotWidth > 0 && slotHeight > 0) {
    const regionPxW = region.w * intrinsic.w
    const regionPxH = region.h * intrinsic.h

    if (fitMode === 'stretch') {
      // Distort: independent scaling on each axis.
      const scaleX = slotWidth / regionPxW
      const scaleY = slotHeight / regionPxH
      const scaledW = intrinsic.w * scaleX
      const scaledH = intrinsic.h * scaleY
      videoStyle = {
        position: 'absolute',
        left: `${-region.x * scaledW}px`,
        top: `${-region.y * scaledH}px`,
        width: `${scaledW}px`,
        height: `${scaledH}px`,
        objectFit: 'fill',
      }
    } else {
      // Fit (decrease) or Fill (increase) -- uniform scale.
      const scale = fitMode === 'fit'
        ? Math.min(slotWidth / regionPxW, slotHeight / regionPxH)
        : Math.max(slotWidth / regionPxW, slotHeight / regionPxH)
      const scaledVideoW = intrinsic.w * scale
      const scaledVideoH = intrinsic.h * scale
      const regionAtScaleW = regionPxW * scale
      const regionAtScaleH = regionPxH * scale
      const offsetX = (slotWidth - regionAtScaleW) / 2
      const offsetY = (slotHeight - regionAtScaleH) / 2
      videoStyle = {
        position: 'absolute',
        left: `${offsetX - region.x * scaledVideoW}px`,
        top: `${offsetY - region.y * scaledVideoH}px`,
        width: `${scaledVideoW}px`,
        height: `${scaledVideoH}px`,
      }
    }
  }

  return (
    <div
      className="absolute inset-0 overflow-hidden bg-black pointer-events-none"
      style={{ zIndex: 5 }}
    >
      <video
        ref={videoRef}
        src={videoSrc}
        muted
        preload="metadata"
        playsInline
        style={videoStyle}
      />
    </div>
  )
}
