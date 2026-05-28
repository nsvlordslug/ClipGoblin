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
 * mirrors what ffmpeg does at export time.
 *
 * Implementation note: we compute the transform from the region + a source
 * aspect ratio. For the initial render we assume 16:9 (the common Twitch
 * source aspect); once the video element's loadedmetadata fires we replace
 * with the true intrinsic aspect. This lets the preview render IMMEDIATELY
 * with a reasonable approximation, instead of waiting on metadata (which
 * some browsers/WebViews delay or skip for clipped/off-screen videos).
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
  const [intrinsicAspect, setIntrinsicAspect] = useState<number>(16 / 9)

  // Capture the source's intrinsic aspect once metadata loads.
  // Falls back to 16:9 default if we never get it (most Twitch sources are 16:9).
  useEffect(() => {
    const v = videoRef.current
    if (!v) return
    const onMeta = () => {
      if (v.videoWidth > 0 && v.videoHeight > 0) {
        setIntrinsicAspect(v.videoWidth / v.videoHeight)
      }
    }
    if (v.readyState >= 1 && v.videoWidth > 0) {
      onMeta()
    } else {
      v.addEventListener('loadedmetadata', onMeta)
      v.addEventListener('loadeddata', onMeta)
      return () => {
        v.removeEventListener('loadedmetadata', onMeta)
        v.removeEventListener('loadeddata', onMeta)
      }
    }
  }, [videoSrc])

  // Seek the preview to the main player's currentTime whenever it changes.
  useEffect(() => {
    const v = videoRef.current
    if (!v) return
    if (Math.abs(v.currentTime - currentTime) > 0.05) {
      try { v.currentTime = currentTime } catch { /* ignore */ }
    }
    v.pause()
  }, [currentTime])

  // Compute the video's CSS transform. We work in NORMALIZED video coords:
  // the video element is sized so that the region (region.w x region.h normalized)
  // maps to the slot dimensions per the fit mode.
  let videoStyle: React.CSSProperties = {
    position: 'absolute',
    left: 0,
    top: 0,
    width: '100%',
    height: '100%',
    objectFit: 'cover',
  }

  if (slotWidth > 0 && slotHeight > 0 && region.w > 0 && region.h > 0) {
    // Pretend the source has intrinsic dimensions (regionW × intrinsicAspect, regionH).
    // Pick a "logical source width" of 1000 for the math; the actual value cancels out.
    const logicalSourceW = 1000
    const logicalSourceH = logicalSourceW / intrinsicAspect
    const regionPxW = region.w * logicalSourceW
    const regionPxH = region.h * logicalSourceH

    if (fitMode === 'stretch') {
      const scaleX = slotWidth / regionPxW
      const scaleY = slotHeight / regionPxH
      const scaledW = logicalSourceW * scaleX
      const scaledH = logicalSourceH * scaleY
      videoStyle = {
        position: 'absolute',
        left: `${-region.x * scaledW}px`,
        top: `${-region.y * scaledH}px`,
        width: `${scaledW}px`,
        height: `${scaledH}px`,
        objectFit: 'fill',
      }
    } else {
      // Fit (decrease) or Fill (increase) — uniform scale.
      const scale = fitMode === 'fit'
        ? Math.min(slotWidth / regionPxW, slotHeight / regionPxH)
        : Math.max(slotWidth / regionPxW, slotHeight / regionPxH)
      const scaledVideoW = logicalSourceW * scale
      const scaledVideoH = logicalSourceH * scale
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
        objectFit: 'fill',
      }
    }
  }

  return (
    <div
      className="absolute inset-0 overflow-hidden pointer-events-none"
      style={{ zIndex: 5, background: 'transparent' }}
    >
      <video
        ref={videoRef}
        src={videoSrc}
        muted
        preload="auto"
        playsInline
        style={videoStyle}
      />
    </div>
  )
}
