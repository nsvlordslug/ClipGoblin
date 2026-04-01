import { useEffect, useRef, useState, useCallback, useId } from 'react'
import { Play, Pause, Volume2, VolumeX, RotateCcw } from 'lucide-react'
import { usePlaybackStore } from '../stores/playbackStore'

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
  /** 'compact' = card-sized (no volume), 'full' = editor-sized (all controls) */
  mode?: 'compact' | 'full'
  /** Extra CSS class for the container */
  className?: string
  /** Overlay content rendered on top of the video (e.g. captions preview) */
  overlay?: React.ReactNode
  /** Called when playback state changes */
  onPlayChange?: (playing: boolean) => void
  /** Called on each time update with the ABSOLUTE video time (not clip-relative) */
  onTimeUpdate?: (absoluteTime: number) => void
  /** Render controls overlaid on the video instead of below it */
  controlsOverlay?: boolean
  /** Ref that receives a seek function — allows external code to seek the player */
  seekRef?: React.MutableRefObject<((absoluteTime: number) => void) | null>
  /** How the video fits its container: 'cover' crops to fill, 'contain' fits inside with bars. Default: 'cover' */
  objectFit?: 'cover' | 'contain'
}

export default function ClipPlayer({
  src, poster, clipStart, clipEnd, mode = 'compact', className = '', overlay,
  controlsOverlay = false, onPlayChange, onTimeUpdate, seekRef: externalSeekRef,
  objectFit = 'cover',
}: Props) {
  const videoRef = useRef<HTMLVideoElement>(null)
  const seekRef = useRef<HTMLDivElement>(null)
  // Keep a stable ref to onTimeUpdate to avoid re-registering the timeupdate
  // listener on every parent render (the callback fires on every video frame)
  const onTimeUpdateRef = useRef(onTimeUpdate)
  onTimeUpdateRef.current = onTimeUpdate

  const [loaded, setLoaded] = useState(false)
  const [playing, setPlaying] = useState(false)
  const [error, setError] = useState('')
  const [currentTime, setCurrentTime] = useState(0)
  const [volume, setVolume] = useState(0.7)
  const [muted, setMuted] = useState(false)
  const [draggingSeek, setDraggingSeek] = useState(false)
  const [draggingVol, setDraggingVol] = useState(false)
  const [showVolume, setShowVolume] = useState(false)

  const clipDuration = Math.max(0, clipEnd - clipStart)
  const elapsed = Math.max(0, currentTime - clipStart)
  const progress = clipDuration > 0 ? Math.min(1, elapsed / clipDuration) : 0
  const isFull = mode === 'full'

  // ── Centralized playback: register this player, coordinate with others ──
  const playerId = useId()
  const { requestPlay, notifyPause, register } = usePlaybackStore()

  // Register a pause callback so other players can pause this one
  useEffect(() => {
    const unregister = register(playerId, () => {
      const video = videoRef.current
      if (video && !video.paused) {
        video.pause()
        setPlaying(false)
        onPlayChange?.(false)
      }
    })
    return unregister
  }, [playerId, register])

  // ── Load video when src changes ──
  useEffect(() => {
    const video = videoRef.current
    if (!video || !src) return
    setLoaded(false)
    setError('')
    setPlaying(false)

    video.src = src
    video.volume = volume
    video.muted = muted

    const onMeta = () => {
      video.currentTime = clipStart
      setCurrentTime(clipStart)
      setLoaded(true)
    }
    const onErr = () => {
      const mediaErr = video.error
      const code = mediaErr?.code ?? 'unknown'
      const msg = mediaErr?.message ?? ''
      console.error(`[ClipPlayer] Video error — code: ${code}, message: ${msg}, src: ${src}`)
      setError('Cannot play video')
    }
    video.addEventListener('loadedmetadata', onMeta, { once: true })
    video.addEventListener('error', onErr, { once: true })
    video.load()

    return () => {
      video.removeEventListener('loadedmetadata', onMeta)
      video.removeEventListener('error', onErr)
    }
  }, [src])

  // ── Time tracking + boundary enforcement ──
  useEffect(() => {
    const video = videoRef.current
    if (!video) return
    const onTime = () => {
      setCurrentTime(video.currentTime)
      onTimeUpdateRef.current?.(video.currentTime)
      if (video.currentTime >= clipEnd) {
        video.pause()
        video.currentTime = clipStart
        setCurrentTime(clipStart)
        setPlayingState(false)
      }
    }
    video.addEventListener('timeupdate', onTime)
    return () => video.removeEventListener('timeupdate', onTime)
  }, [clipStart, clipEnd])

  // ── Expose seek function to parent via ref ──
  useEffect(() => {
    if (externalSeekRef) {
      externalSeekRef.current = (absoluteTime: number) => {
        const video = videoRef.current
        if (!video) return
        video.currentTime = absoluteTime
        setCurrentTime(absoluteTime)
        onTimeUpdate?.(absoluteTime)
      }
    }
    return () => { if (externalSeekRef) externalSeekRef.current = null }
  }, [externalSeekRef])

  // ── Sync volume/muted to video element ──
  useEffect(() => {
    const video = videoRef.current
    if (!video) return
    video.volume = volume
    video.muted = muted
  }, [volume, muted])

  const setPlayingState = (state: boolean) => {
    setPlaying(state)
    onPlayChange?.(state)
    if (state) {
      requestPlay(playerId) // pause all other players
    } else {
      notifyPause(playerId) // clear active status
    }
  }

  // ── Play / Pause ──
  const togglePlay = useCallback(async () => {
    if (error) return
    const video = videoRef.current
    if (!video) return

    if (!loaded) {
      // First click — wait for metadata
      const onReady = () => {
        video.currentTime = clipStart
        video.play().then(() => setPlayingState(true)).catch(() => setError('Playback failed'))
      }
      if (video.readyState >= 1) onReady()
      else video.addEventListener('loadedmetadata', onReady, { once: true })
      return
    }

    if (playing) {
      video.pause()
      setPlayingState(false)
    } else {
      if (video.currentTime >= clipEnd - 0.5 || video.currentTime < clipStart) {
        video.currentTime = clipStart
      }
      video.play().then(() => setPlayingState(true)).catch(() => setError('Playback failed'))
    }
  }, [loaded, playing, error, clipStart, clipEnd])

  const restart = () => {
    const video = videoRef.current
    if (!video || !loaded) return
    video.currentTime = clipStart
    setCurrentTime(clipStart)
    video.play().then(() => setPlayingState(true)).catch(() => {})
  }

  // ── Seek (scrub bar) ──
  const seekTo = useCallback((clientX: number) => {
    const bar = seekRef.current
    const video = videoRef.current
    if (!bar || !video || !loaded) return
    const rect = bar.getBoundingClientRect()
    const pct = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width))
    const t = clipStart + pct * clipDuration
    video.currentTime = t
    setCurrentTime(t)
  }, [loaded, clipStart, clipDuration])

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

  return (
    <div className={`flex flex-col ${className}`}>
      {/* ── Video area ── */}
      <div className={`relative bg-surface-900 cursor-pointer group flex-1 min-h-0`} onClick={togglePlay}>
        <video
          ref={videoRef}
          className={`absolute inset-0 w-full h-full ${objectFit === 'contain' ? 'object-contain' : 'object-cover'}`}
          playsInline
          poster={poster || undefined}
        />

        {/* Overlay content (captions, text overlays) */}
        {overlay}

        {/* Play/Pause center icon */}
        {!playing && !error && (
          <div className="absolute inset-0 flex items-center justify-center bg-black/40 group-hover:bg-black/30 transition-colors">
            <Play className={`${isFull ? 'w-14 h-14' : 'w-10 h-10'} text-white/90 drop-shadow`} />
          </div>
        )}
        {!playing && error && (
          <div className="absolute inset-0 flex items-center justify-center bg-black/40">
            <span className="text-red-400 text-xs px-3 text-center">{error}</span>
          </div>
        )}
        {playing && (
          <div className="absolute inset-0 flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity">
            <Pause className={`${isFull ? 'w-14 h-14' : 'w-10 h-10'} text-white/90 drop-shadow`} />
          </div>
        )}

        {/* Duration badge (compact only) */}
        {!isFull && !controlsOverlay && (
          <span className="absolute bottom-2 right-2 bg-black/80 text-white text-[10px] px-1.5 py-0.5 rounded font-mono">
            {fmt(clipDuration)}
          </span>
        )}

        {/* Controls overlaid inside video area */}
        {controlsOverlay && (
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
            <button onClick={toggleMute} className="shrink-0 p-1 rounded text-white/80 hover:text-white cursor-pointer" title={muted ? 'Unmute' : 'Mute'}>
              {muted || volume === 0 ? <VolumeX className="w-3.5 h-3.5" /> : <Volume2 className="w-3.5 h-3.5" />}
            </button>
          </div>
        )}
      </div>

      {/* Controls below video (non-overlay mode) */}
      {!controlsOverlay && (
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
            <button onClick={toggleMute} className="shrink-0 p-0.5 rounded text-slate-500 hover:text-white transition-colors cursor-pointer" title={muted ? 'Unmute' : 'Mute'}>
              {muted || volume === 0 ? <VolumeX className="w-3 h-3" /> : <Volume2 className="w-3 h-3" />}
            </button>
          )}
        </div>
      )}
    </div>
  )
}
