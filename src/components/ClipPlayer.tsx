import { useEffect, useRef, useState, useCallback, useId } from 'react'
import { Play, Pause, Volume2, VolumeX, RotateCcw } from 'lucide-react'
import { usePlaybackStore } from '../stores/playbackStore'
import { contextBlurPixels, normalizeContextVideoY } from '../lib/contextFit'

const PLAYBACK_FAILED_MSG = 'Playback failed'

function fmt(seconds: number) {
  const m = Math.floor(seconds / 60)
  const s = Math.floor(seconds % 60)
  return `${m}:${String(s).padStart(2, '0')}`
}

interface Props {
  /** Video source URL (from convertFileSrc) */
  src: string | null
  /** Poster/thumbnail URL */
  poster?: string | null
  /** Clip start within the source video (seconds) */
  clipStart: number
  /** Clip end within the source video (seconds) */
  clipEnd: number
  /**
   * When true, `src` is a STANDALONE, already-trimmed file (e.g. a downloaded
   * Twitch community clip). The player ignores clipStart/clipEnd and plays the
   * WHOLE file (0 → the file's natural duration) with no seek/trim. Default:
   * false — keeps the normal VOD-seek behavior driven by clipStart/clipEnd.
   */
  fullFile?: boolean
  /** 'compact' = card-sized (no volume), 'full' = editor-sized (all controls) */
  mode?: 'compact' | 'full'
  /** Extra CSS class for the container */
  className?: string
  /** Overlay content rendered on top of the video (e.g. captions preview) */
  overlay?: React.ReactNode
  /** Called when playback state changes */
  onPlayChange?: (playing: boolean) => void
  /** Called before playback when the parent still needs to prepare a local media URL. */
  onRequestSource?: () => void | Promise<void>
  /** Called once when playback reaches the end of this clip window. */
  onEnded?: () => void
  /** Start playback after a newly supplied source has loaded. */
  autoPlay?: boolean
  /** Called when the current source has loaded enough metadata to play. */
  onReady?: () => void
  /** Keep this player independent from the app-wide single-player coordinator. */
  coordinatePlayback?: boolean
  /** Hide this instance's controls when it is used as a transition layer. */
  showControls?: boolean
  /** Temporary gain applied without changing the user's volume setting. */
  volumeMultiplier?: number
  /** Called on each time update with the ABSOLUTE video time (not clip-relative) */
  onTimeUpdate?: (absoluteTime: number) => void
  /** Render controls overlaid on the video instead of below it */
  controlsOverlay?: boolean
  /** Ref that receives a seek function — allows external code to seek the player */
  seekRef?: React.MutableRefObject<((absoluteTime: number) => void) | null>
  /** How the video fits its container: 'cover' crops to fill, 'contain' fits inside with bars. Default: 'cover' */
  objectFit?: 'cover' | 'contain'
  /** Show a softened poster behind contained video to preview a context-preserving vertical export. */
  blurBackground?: boolean
  /** Use a true black canvas behind contained video instead of a live blur. */
  blackBackground?: boolean
  /** Optional app-managed image/GIF shown behind Context Fit video instead of the live blur. */
  backgroundMedia?: string | null
  /** Normalized 0..1 softness for the live-video background. */
  backgroundBlurStrength?: number
  /** Normalized 0..1 placement of contained video from top to bottom. */
  objectPositionY?: number
  /** Optional ref that receives the underlying <video> element so external
   * code (e.g. the cam-region preview) can read frames via canvas drawImage
   * without opening a second decoder on the same source. */
  videoElementRef?: React.MutableRefObject<HTMLVideoElement | null>
}

export default function ClipPlayer({
  src, poster, clipStart, clipEnd, mode = 'compact', className = '', overlay,
  controlsOverlay = false, onPlayChange, onRequestSource, onEnded, autoPlay = false,
  onReady, coordinatePlayback = true, showControls = true, volumeMultiplier = 1,
  onTimeUpdate, seekRef: externalSeekRef,
  objectFit = 'cover', blurBackground = false, blackBackground = false, backgroundMedia = null,
  backgroundBlurStrength = 0.25, objectPositionY = 0.5, videoElementRef, fullFile = false,
}: Props) {
  const videoRef = useRef<HTMLVideoElement>(null)
  const backgroundCanvasRef = useRef<HTMLCanvasElement>(null)
  // Mirror our internal video ref into the external ref (if provided)
  // so callers can drawImage frames without opening a second decoder.
  useEffect(() => {
    if (videoElementRef) videoElementRef.current = videoRef.current
    return () => { if (videoElementRef) videoElementRef.current = null }
  }, [videoElementRef])
  const seekRef = useRef<HTMLDivElement>(null)
  // Keep a stable ref to onTimeUpdate to avoid re-registering the timeupdate
  // listener on every parent render (the callback fires on every video frame)
  const onTimeUpdateRef = useRef(onTimeUpdate)
  const onPlayChangeRef = useRef(onPlayChange)
  const onRequestSourceRef = useRef(onRequestSource)
  const onEndedRef = useRef(onEnded)
  const onReadyRef = useRef(onReady)
  const pendingPlayRef = useRef(false)
  const completedRef = useRef(false)

  useEffect(() => {
    onTimeUpdateRef.current = onTimeUpdate
    onPlayChangeRef.current = onPlayChange
    onRequestSourceRef.current = onRequestSource
    onEndedRef.current = onEnded
    onReadyRef.current = onReady
  }, [onEnded, onPlayChange, onReady, onRequestSource, onTimeUpdate])

  const [loaded, setLoaded] = useState(false)
  const [playing, setPlaying] = useState(false)
  const [error, setError] = useState('')
  const [currentTime, setCurrentTime] = useState(0)
  const [volume, setVolume] = useState(0.7)
  const [muted, setMuted] = useState(false)
  const [draggingSeek, setDraggingSeek] = useState(false)
  const [draggingVol, setDraggingVol] = useState(false)
  const [showVolume, setShowVolume] = useState(false)
  // Natural duration of the loaded file — only used in fullFile mode (a
  // standalone community-clip MP4) to bound playback to the file itself.
  const [fileDuration, setFileDuration] = useState(0)
  const fullFileRef = useRef(fullFile)
  const clipStartRef = useRef(clipStart)

  useEffect(() => {
    fullFileRef.current = fullFile
    clipStartRef.current = clipStart
  }, [clipStart, fullFile])

  // Effective clip bounds. In fullFile mode the source IS the clip, so we play
  // 0 → the file's own duration and ignore the VOD-relative clipStart/clipEnd.
  // Until metadata loads (fileDuration === 0) we use a sentinel so the boundary
  // check below never trims the file early. When fullFile is false these are
  // exactly clipStart/clipEnd, so normal VOD clips are unaffected.
  const effClipStart = fullFile ? 0 : clipStart
  const effClipEnd = fullFile ? (fileDuration > 0 ? fileDuration : Number.MAX_SAFE_INTEGER) : clipEnd

  const clipDuration = Math.max(0, effClipEnd - effClipStart)
  const elapsed = Math.max(0, currentTime - effClipStart)
  const progress = clipDuration > 0 && clipDuration < Number.MAX_SAFE_INTEGER ? Math.min(1, elapsed / clipDuration) : 0
  const isFull = mode === 'full'

  // ── Centralized playback: register this player, coordinate with others ──
  const playerId = useId()
  const { requestPlay, notifyPause, register } = usePlaybackStore()

  const setPlayingState = useCallback((state: boolean) => {
    setPlaying(state)
    onPlayChangeRef.current?.(state)
    if (!coordinatePlayback) return
    if (state) {
      requestPlay(playerId)
    } else {
      notifyPause(playerId)
    }
  }, [coordinatePlayback, notifyPause, playerId, requestPlay])

  // Register a pause callback so other players can pause this one
  useEffect(() => {
    if (!coordinatePlayback) return
    const unregister = register(playerId, () => {
      const video = videoRef.current
      if (video && !video.paused) {
        video.pause()
        setPlaying(false)
        onPlayChangeRef.current?.(false)
      }
    })
    return unregister
  }, [coordinatePlayback, playerId, register])

  // ── Load video when src changes ──
  useEffect(() => {
    const video = videoRef.current
    if (!video) return

    const onReset = () => {
      setLoaded(false)
      setError('')
      setPlaying(false)
      setFileDuration(0)
      completedRef.current = false
    }
    const onMeta = () => {
      if (fullFileRef.current) {
        // Standalone clip: it's already trimmed — start at 0 and bound to the
        // file's own duration. No VOD-relative seek.
        setFileDuration(Number.isFinite(video.duration) ? video.duration : 0)
        video.currentTime = 0
        setCurrentTime(0)
      } else {
        video.currentTime = clipStartRef.current
        setCurrentTime(clipStartRef.current)
      }
      setLoaded(true)
      onReadyRef.current?.()
      if (pendingPlayRef.current) {
        pendingPlayRef.current = false
        completedRef.current = false
        video.play()
          .then(() => setPlayingState(true))
          .catch(() => setError(PLAYBACK_FAILED_MSG))
      }
    }
    const onErr = () => {
      const mediaErr = video.error
      const code = mediaErr?.code ?? 'unknown'
      const msg = mediaErr?.message ?? ''
      console.error(`[ClipPlayer] Video error — code: ${code}, message: ${msg}, src: ${src}`)
      setError('Cannot play video')
    }
    video.addEventListener('loadstart', onReset)
    video.addEventListener('emptied', onReset)
    video.addEventListener('loadedmetadata', onMeta, { once: true })
    video.addEventListener('error', onErr, { once: true })

    if (src) {
      video.src = src
      video.load()
    } else {
      video.pause()
      video.removeAttribute('src')
      video.load()
    }

    return () => {
      video.removeEventListener('loadstart', onReset)
      video.removeEventListener('emptied', onReset)
      video.removeEventListener('loadedmetadata', onMeta)
      video.removeEventListener('error', onErr)
    }
  }, [setPlayingState, src])

  useEffect(() => {
    const video = videoRef.current
    if (!autoPlay || !loaded || playing || error || !video) return
    completedRef.current = false
    if ((effClipEnd < Number.MAX_SAFE_INTEGER && video.currentTime >= effClipEnd - 0.05) || video.currentTime < effClipStart) {
      video.currentTime = effClipStart
    }
    video.play().then(() => setPlayingState(true)).catch(() => setError(PLAYBACK_FAILED_MSG))
  }, [autoPlay, effClipEnd, effClipStart, error, loaded, playing, setPlayingState])

  // ── Time tracking + boundary enforcement ──
  useEffect(() => {
    const video = videoRef.current
    if (!video) return
    let videoFrameId: number | null = null
    let animationFrameId: number | null = null
    let disposed = false
    let lastReportedTime = Number.NEGATIVE_INFINITY

    const drawBlurBackground = () => {
      if (!blurBackground || backgroundMedia || video.readyState < HTMLMediaElement.HAVE_CURRENT_DATA) return
      const canvas = backgroundCanvasRef.current
      if (!canvas || video.videoWidth <= 0 || video.videoHeight <= 0) return

      const width = Math.max(1, Math.round(canvas.clientWidth))
      const height = Math.max(1, Math.round(canvas.clientHeight))
      if (canvas.width !== width || canvas.height !== height) {
        canvas.width = width
        canvas.height = height
      }

      const sourceRatio = video.videoWidth / video.videoHeight
      const targetRatio = width / height
      let sx = 0
      let sy = 0
      let sw = video.videoWidth
      let sh = video.videoHeight
      if (sourceRatio > targetRatio) {
        sw = video.videoHeight * targetRatio
        sx = (video.videoWidth - sw) / 2
      } else {
        sh = video.videoWidth / targetRatio
        sy = (video.videoHeight - sh) / 2
      }

      const context = canvas.getContext('2d')
      if (!context) return
      try {
        context.drawImage(video, sx, sy, sw, sh, 0, 0, width, height)
      } catch {
        // The poster fallback remains visible if this WebView cannot sample the video.
      }
    }

    const reportTime = (time: number, force = false) => {
      if (!force && Math.abs(time - lastReportedTime) < 1 / 30) return
      lastReportedTime = time
      setCurrentTime(time)
      onTimeUpdateRef.current?.(time)
    }

    const syncPlaybackTime = (force = false) => {
      drawBlurBackground()
      const time = video.currentTime
      if (time >= effClipEnd) {
        if (completedRef.current) return
        completedRef.current = true
        video.pause()
        video.currentTime = effClipStart
        reportTime(effClipStart, true)
        setPlayingState(false)
        onEndedRef.current?.()
        return
      }
      reportTime(time, force)
    }

    const cancelScheduledFrame = () => {
      if (videoFrameId != null && typeof video.cancelVideoFrameCallback === 'function') {
        video.cancelVideoFrameCallback(videoFrameId)
      }
      if (animationFrameId != null) cancelAnimationFrame(animationFrameId)
      videoFrameId = null
      animationFrameId = null
    }

    const scheduleFrame = () => {
      if (disposed || video.paused || video.ended || videoFrameId != null || animationFrameId != null) return
      if (typeof video.requestVideoFrameCallback === 'function') {
        videoFrameId = video.requestVideoFrameCallback(() => {
          videoFrameId = null
          syncPlaybackTime()
          scheduleFrame()
        })
      } else {
        animationFrameId = requestAnimationFrame(() => {
          animationFrameId = null
          syncPlaybackTime()
          scheduleFrame()
        })
      }
    }

    const onPlay = () => {
      completedRef.current = false
      scheduleFrame()
    }
    const onPause = () => {
      cancelScheduledFrame()
      syncPlaybackTime(true)
    }
    const onTime = () => syncPlaybackTime()
    const onSeek = () => syncPlaybackTime(true)
    const onEnded = () => syncPlaybackTime(true)
    const onLoadedData = () => drawBlurBackground()
    const resizeObserver = typeof ResizeObserver === 'undefined'
      ? null
      : new ResizeObserver(drawBlurBackground)

    video.addEventListener('play', onPlay)
    video.addEventListener('pause', onPause)
    video.addEventListener('timeupdate', onTime)
    video.addEventListener('seeking', onSeek)
    video.addEventListener('seeked', onSeek)
    video.addEventListener('ended', onEnded)
    video.addEventListener('loadeddata', onLoadedData)
    if (backgroundCanvasRef.current) resizeObserver?.observe(backgroundCanvasRef.current)
    drawBlurBackground()
    if (!video.paused) scheduleFrame()

    return () => {
      disposed = true
      cancelScheduledFrame()
      video.removeEventListener('play', onPlay)
      video.removeEventListener('pause', onPause)
      video.removeEventListener('timeupdate', onTime)
      video.removeEventListener('seeking', onSeek)
      video.removeEventListener('seeked', onSeek)
      video.removeEventListener('ended', onEnded)
      video.removeEventListener('loadeddata', onLoadedData)
      resizeObserver?.disconnect()
    }
  }, [backgroundMedia, blurBackground, effClipStart, effClipEnd, setPlayingState])

  // ── Expose seek function to parent via ref ──
  useEffect(() => {
    if (externalSeekRef) {
      externalSeekRef.current = (absoluteTime: number) => {
        const video = videoRef.current
        if (!video) return
        video.currentTime = absoluteTime
        setCurrentTime(absoluteTime)
        onTimeUpdateRef.current?.(absoluteTime)
      }
    }
    return () => { if (externalSeekRef) externalSeekRef.current = null }
  }, [externalSeekRef])

  // ── Sync volume/muted to video element ──
  useEffect(() => {
    const video = videoRef.current
    if (!video) return
    video.volume = Math.max(0, Math.min(1, volume * volumeMultiplier))
    video.muted = muted
  }, [muted, volume, volumeMultiplier])

  // ── Play / Pause ──
  const togglePlay = useCallback(async () => {
    if (error) return
    const video = videoRef.current
    if (!video) return

    if (!loaded) {
      pendingPlayRef.current = true
      if (!src && onRequestSourceRef.current) {
        try {
          await onRequestSourceRef.current()
        } catch {
          pendingPlayRef.current = false
          setError(PLAYBACK_FAILED_MSG)
        }
      }
      return
    }

    if (playing) {
      video.pause()
      setPlayingState(false)
    } else {
      completedRef.current = false
      if ((effClipEnd < Number.MAX_SAFE_INTEGER && video.currentTime >= effClipEnd - 0.5) || video.currentTime < effClipStart) {
        video.currentTime = effClipStart
      }
      video.play().then(() => setPlayingState(true)).catch(() => setError(PLAYBACK_FAILED_MSG))
    }
  }, [loaded, playing, error, effClipStart, effClipEnd, setPlayingState, src])

  const restart = () => {
    const video = videoRef.current
    if (!video || !loaded) return
    completedRef.current = false
    video.currentTime = effClipStart
    setCurrentTime(effClipStart)
    video.play().then(() => setPlayingState(true)).catch(() => {})
  }

  // ── Seek (scrub bar) ──
  const seekTo = useCallback((clientX: number) => {
    const bar = seekRef.current
    const video = videoRef.current
    if (!bar || !video || !loaded) return
    const rect = bar.getBoundingClientRect()
    const pct = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width))
    const t = effClipStart + pct * clipDuration
    video.currentTime = t
    setCurrentTime(t)
  }, [loaded, effClipStart, clipDuration])

  const onSeekDown = (e: React.MouseEvent) => {
    e.preventDefault(); e.stopPropagation()
    setDraggingSeek(true)
    seekTo(e.clientX)
  }

  useEffect(() => {
    if (!draggingSeek) return
    const onMove = (e: MouseEvent) => seekTo(e.clientX)
    const onUp = () => setDraggingSeek(false)
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    return () => { window.removeEventListener('mousemove', onMove); window.removeEventListener('mouseup', onUp) }
  }, [draggingSeek, seekTo])

  // ── Volume slider drag ──
  const volRef = useRef<HTMLDivElement>(null)

  const setVolFrom = useCallback((clientX: number) => {
    const bar = volRef.current
    if (!bar) return
    const rect = bar.getBoundingClientRect()
    const v = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width))
    setVolume(v)
    if (v > 0 && muted) setMuted(false)
  }, [muted])

  const onVolDown = (e: React.MouseEvent) => {
    e.preventDefault(); e.stopPropagation()
    setDraggingVol(true)
    setVolFrom(e.clientX)
  }

  useEffect(() => {
    if (!draggingVol) return
    const onMove = (e: MouseEvent) => setVolFrom(e.clientX)
    const onUp = () => setDraggingVol(false)
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    return () => { window.removeEventListener('mousemove', onMove); window.removeEventListener('mouseup', onUp) }
  }, [draggingVol, setVolFrom])

  const toggleMute = (e: React.MouseEvent) => {
    e.stopPropagation()
    setMuted(m => !m)
  }

  const useLiveBlur = blurBackground && !backgroundMedia
  const blurPixels = contextBlurPixels(backgroundBlurStrength)
  const containedVideoPosition = `${normalizeContextVideoY(objectPositionY) * 100}%`

  return (
    <div className={`flex flex-col ${className}`}>
      {/* ── Video area ── */}
      <div className={`relative group flex-1 min-h-0 ${showControls ? 'cursor-pointer' : ''} ${blackBackground ? 'bg-black' : 'bg-surface-900'}`} onClick={showControls ? togglePlay : undefined}>
        {backgroundMedia && (
          <img
            src={backgroundMedia}
            alt=""
            aria-hidden="true"
            className="absolute inset-0 h-full w-full object-cover"
          />
        )}
        {useLiveBlur && poster && (
          <img
            src={poster}
            alt=""
            aria-hidden="true"
            className="absolute -inset-3 h-[calc(100%+1.5rem)] w-[calc(100%+1.5rem)] scale-105 object-cover opacity-90"
            style={{ filter: `blur(${blurPixels}px) brightness(0.97) saturate(1.04)` }}
          />
        )}
        {useLiveBlur && (
          <canvas
            ref={backgroundCanvasRef}
            aria-hidden="true"
            className="absolute -inset-3 h-[calc(100%+1.5rem)] w-[calc(100%+1.5rem)] scale-105 opacity-95"
            style={{ filter: `blur(${blurPixels}px) brightness(0.97) saturate(1.04)` }}
          />
        )}
        {useLiveBlur && <div className="absolute inset-0 bg-black/5" aria-hidden="true" />}
        <video
          ref={videoRef}
          className={`absolute inset-0 z-[1] w-full h-full ${objectFit === 'contain' ? 'object-contain' : 'object-cover'}`}
          style={objectFit === 'contain' ? { objectPosition: `center ${containedVideoPosition}` } : undefined}
          playsInline
          poster={poster || undefined}
        />

        {/* Overlay content (captions, text overlays) */}
        {overlay}

        {/* Play/Pause center icon */}
        {showControls && !playing && !error && (
          <div className="absolute inset-0 flex items-center justify-center bg-black/40 group-hover:bg-black/30 transition-colors">
            <Play className={`${isFull ? 'w-14 h-14' : 'w-10 h-10'} text-white/90 drop-shadow`} />
          </div>
        )}
        {showControls && !playing && error && (
          <div className="absolute inset-0 flex items-center justify-center bg-black/40">
            <span className="text-red-400 text-xs px-3 text-center">{error}</span>
          </div>
        )}
        {showControls && playing && (
          <div className="absolute inset-0 flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity">
            <Pause className={`${isFull ? 'w-14 h-14' : 'w-10 h-10'} text-white/90 drop-shadow`} />
          </div>
        )}

        {/* Duration badge (compact only) */}
        {showControls && !isFull && !controlsOverlay && (
          <span className="absolute bottom-2 right-2 bg-black/80 text-white text-[10px] px-1.5 py-0.5 rounded font-mono">
            {fmt(clipDuration)}
          </span>
        )}

        {/* Controls overlaid inside video area */}
        {showControls && controlsOverlay && (
          <div className="absolute bottom-0 left-0 right-0 z-[15] flex items-center gap-2 px-3 py-2 bg-gradient-to-t from-black/80 via-black/40 to-transparent opacity-0 group-hover:opacity-100 transition-opacity duration-200">
            <button onClick={e => { e.stopPropagation(); togglePlay() }} className="shrink-0 p-1 rounded text-white/80 hover:text-white cursor-pointer" title={playing ? 'Pause' : 'Play'}>
              {playing ? <Pause className="w-3.5 h-3.5" /> : <Play className="w-3.5 h-3.5" />}
            </button>
            <button onClick={e => { e.stopPropagation(); restart() }} className="shrink-0 p-1 rounded text-white/80 hover:text-white cursor-pointer" title="Restart">
              <RotateCcw className="w-3.5 h-3.5" />
            </button>
            <div ref={seekRef} className="flex-1 h-1.5 bg-white/20 rounded-full cursor-pointer relative group/bar" onMouseDown={onSeekDown}>
              <div className="h-full bg-white rounded-full transition-[width] duration-75 relative" style={{ width: `${progress * 100}%` }}>
                <div className="absolute right-0 top-1/2 -translate-y-1/2 w-3 h-3 bg-white rounded-full shadow opacity-0 group-hover/bar:opacity-100 transition-opacity" />
              </div>
            </div>
            <span className="text-[10px] text-white/70 font-mono shrink-0 tabular-nums">{loaded ? fmt(elapsed) : '0:00'}/{fmt(clipDuration)}</span>
            <div className="flex w-24 shrink-0 items-center gap-1.5">
              <button onClick={toggleMute} className="shrink-0 p-1 rounded text-white/80 hover:text-white cursor-pointer" title={muted ? 'Unmute' : 'Mute'}>
                {muted || volume === 0 ? <VolumeX className="w-3.5 h-3.5" /> : <Volume2 className="w-3.5 h-3.5" />}
              </button>
              <div
                ref={volRef}
                className="group/vol relative h-1.5 flex-1 cursor-pointer rounded-full bg-white/20"
                onMouseDown={onVolDown}
                role="slider"
                aria-label="Preview volume"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={Math.round((muted ? 0 : volume) * 100)}
              >
                <div className="relative h-full rounded-full bg-white" style={{ width: `${(muted ? 0 : volume) * 100}%` }}>
                  <div className="absolute right-0 top-1/2 h-2.5 w-2.5 -translate-y-1/2 rounded-full bg-white shadow opacity-0 transition-opacity group-hover/vol:opacity-100" />
                </div>
              </div>
            </div>
          </div>
        )}
      </div>

      {/* Controls below video (non-overlay mode) */}
      {showControls && !controlsOverlay && (
        <div className={`flex items-center gap-2 ${isFull ? 'px-3 py-2' : 'px-3 pt-2 pb-1'}`}>
          <button onClick={e => { e.stopPropagation(); togglePlay() }} className="shrink-0 p-1 rounded text-slate-400 hover:text-white transition-colors cursor-pointer" title={playing ? 'Pause' : 'Play'}>
            {playing ? <Pause className="w-3.5 h-3.5" /> : <Play className="w-3.5 h-3.5" />}
          </button>
          {isFull && (
            <button onClick={e => { e.stopPropagation(); restart() }} className="shrink-0 p-1 rounded text-slate-400 hover:text-white transition-colors cursor-pointer" title="Restart">
              <RotateCcw className="w-3.5 h-3.5" />
            </button>
          )}
          <div ref={seekRef} className={`flex-1 ${isFull ? 'h-2' : 'h-1.5'} bg-surface-600 rounded-full cursor-pointer relative group/bar`} onMouseDown={onSeekDown}>
            <div className="h-full bg-violet-500 rounded-full transition-[width] duration-75 relative" style={{ width: `${progress * 100}%` }}>
              <div className={`absolute right-0 top-1/2 -translate-y-1/2 ${isFull ? 'w-3.5 h-3.5' : 'w-3 h-3'} bg-white rounded-full shadow opacity-0 group-hover/bar:opacity-100 transition-opacity`} />
            </div>
          </div>
          <span className={`${isFull ? 'text-xs' : 'text-[10px]'} text-slate-500 font-mono shrink-0 tabular-nums`}>{loaded ? fmt(elapsed) : '0:00'}/{fmt(clipDuration)}</span>
          {isFull ? (
            <div className="flex items-center gap-1.5 shrink-0 relative" onMouseEnter={() => setShowVolume(true)} onMouseLeave={() => { if (!draggingVol) setShowVolume(false) }}>
              <button onClick={toggleMute} className="p-1 rounded text-slate-400 hover:text-white transition-colors cursor-pointer" title={muted ? 'Unmute' : 'Mute'}>
                {muted || volume === 0 ? <VolumeX className="w-4 h-4" /> : <Volume2 className="w-4 h-4" />}
              </button>
              <div className={`overflow-hidden transition-all duration-200 ${showVolume || draggingVol ? 'w-20 opacity-100' : 'w-0 opacity-0'}`}>
                <div ref={volRef} className="h-1.5 bg-surface-600 rounded-full cursor-pointer relative group/vol w-full" onMouseDown={onVolDown}>
                  <div className="h-full bg-violet-400 rounded-full" style={{ width: `${(muted ? 0 : volume) * 100}%` }}>
                    <div className="absolute right-0 top-1/2 -translate-y-1/2 w-3 h-3 bg-white rounded-full shadow opacity-0 group-hover/vol:opacity-100 transition-opacity" />
                  </div>
                </div>
              </div>
            </div>
          ) : (
            <div
              className="relative shrink-0"
              onMouseEnter={() => setShowVolume(true)}
              onMouseLeave={() => { if (!draggingVol) setShowVolume(false) }}
            >
              <button
                onClick={toggleMute}
                className="p-0.5 rounded text-slate-500 hover:text-white transition-colors cursor-pointer"
                title={`${muted ? 'Unmute' : 'Mute'} (hover to adjust volume)`}
                aria-label={`${muted ? 'Unmute' : 'Mute'}; hover to adjust volume`}
              >
                {muted || volume === 0 ? <VolumeX className="w-3 h-3" /> : <Volume2 className="w-3 h-3" />}
              </button>
              <div
                className={`absolute bottom-full right-0 z-30 w-28 rounded-md border border-surface-600 bg-surface-900 px-2.5 py-2 shadow-xl transition-opacity ${showVolume || draggingVol ? 'visible opacity-100' : 'invisible opacity-0'}`}
              >
                <div className="flex items-center gap-2">
                  <div
                    ref={volRef}
                    className="relative h-1.5 flex-1 cursor-pointer rounded-full bg-surface-600 group/vol"
                    onMouseDown={onVolDown}
                    role="slider"
                    aria-label="Clip volume"
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={Math.round((muted ? 0 : volume) * 100)}
                  >
                    <div className="relative h-full rounded-full bg-violet-400" style={{ width: `${(muted ? 0 : volume) * 100}%` }}>
                      <div className="absolute right-0 top-1/2 h-2.5 w-2.5 -translate-y-1/2 rounded-full bg-white shadow opacity-0 transition-opacity group-hover/vol:opacity-100" />
                    </div>
                  </div>
                  <span className="w-7 text-right text-[9px] tabular-nums text-slate-400">
                    {Math.round((muted ? 0 : volume) * 100)}%
                  </span>
                </div>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  )
}
