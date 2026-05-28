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
}

/**
 * Live preview of the source-region cam content, rendered inside the editor's
 * cam slot. CSS-positioned `<video>` element with calculated transform that
 * mirrors what ffmpeg does at export time.
 *
 * The component measures its own parent (the slot) via ResizeObserver, so the
 * math always uses ACTUAL rendered pixel dimensions instead of estimates.
 *
 * For Fit/Fill modes we need the source's aspect ratio. We default to 16:9
 * (the common Twitch source aspect) and refine when the video element's
 * loadedmetadata event fires.
 */
export default function CamRegionPreview({
  videoSrc,
  region,
  fitMode,
  currentTime,
}: Props) {
  const wrapperRef = useRef<HTMLDivElement | null>(null)
  const videoRef = useRef<HTMLVideoElement | null>(null)
  const [intrinsicAspect, setIntrinsicAspect] = useState<number>(16 / 9)
  const [slotSize, setSlotSize] = useState<{ w: number; h: number }>({ w: 0, h: 0 })

  // Measure the parent slot's actual rendered dimensions.
  useEffect(() => {
    const el = wrapperRef.current
    if (!el) return
    const measure = () => {
      const r = el.getBoundingClientRect()
      if (r.width > 0 && r.height > 0) {
        setSlotSize({ w: r.width, h: r.height })
      }
    }
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    window.addEventListener('resize', measure)
    return () => {
      ro.disconnect()
      window.removeEventListener('resize', measure)
    }
  }, [])

  // Capture the source's intrinsic aspect once metadata loads.
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

  // Compute the video's CSS transform based on the REAL measured slot size.
  const { w: slotWidth, h: slotHeight } = slotSize
  let videoStyle: React.CSSProperties = {
    position: 'absolute',
    left: 0,
    top: 0,
    width: '100%',
    height: '100%',
    objectFit: 'cover',
  }

  if (slotWidth > 0 && slotHeight > 0 && region.w > 0 && region.h > 0) {
    // Logical source coords: width 1000, height 1000/aspect. Actual values
    // cancel out in the scale calculation -- only the ratio matters.
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
      ref={wrapperRef}
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
