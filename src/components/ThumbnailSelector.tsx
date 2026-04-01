import { useState } from 'react'
import { Camera, Check, Loader2, Image } from 'lucide-react'
import { invoke, convertFileSrc } from '@tauri-apps/api/core'

interface Props {
  clipId: string
  /** Current absolute playback time — the frame to capture */
  currentTime: number
  /** Current thumbnail path (if one exists) */
  thumbnailPath: string | null
  /** Called after thumbnail is successfully set */
  onThumbnailSet: (path: string) => void
}

function fmt(s: number) {
  const m = Math.floor(s / 60)
  const sec = Math.floor(s % 60)
  return `${m}:${String(sec).padStart(2, '0')}`
}

export default function ThumbnailSelector({
  clipId, currentTime, thumbnailPath, onThumbnailSet,
}: Props) {
  const [capturing, setCapturing] = useState(false)
  const [justSet, setJustSet] = useState(false)
  const [error, setError] = useState('')
  // Cache-bust: append a counter to force re-render after capture
  const [version, setVersion] = useState(0)

  const captureFrame = async () => {
    setCapturing(true)
    setError('')
    setJustSet(false)
    try {
      const path = await invoke<string>('set_clip_thumbnail', {
        clipId,
        timestamp: currentTime,
      })
      onThumbnailSet(path)
      setVersion(v => v + 1)
      setJustSet(true)
      setTimeout(() => setJustSet(false), 2500)
    } catch (err) {
      setError(String(err))
    } finally {
      setCapturing(false)
    }
  }

  const thumbSrc = thumbnailPath
    ? `${convertFileSrc(thumbnailPath)}?v=${version}`
    : null

  return (
    <div className="space-y-3">
      {/* Current thumbnail preview */}
      <div className="flex gap-3 items-start">
        <div className="w-24 h-14 bg-surface-900 rounded border border-surface-600 overflow-hidden shrink-0 flex items-center justify-center">
          {thumbSrc ? (
            <img src={thumbSrc} alt="Thumbnail" className="w-full h-full object-cover" />
          ) : (
            <Image className="w-5 h-5 text-slate-600" />
          )}
        </div>
        <div className="flex-1 min-w-0 space-y-1.5">
          <p className="text-[10px] text-slate-500">
            {thumbnailPath ? 'Current thumbnail' : 'No thumbnail set'}
          </p>

          {/* Capture button */}
          <button
            onClick={captureFrame}
            disabled={capturing}
            className={`flex items-center gap-1.5 px-2.5 py-1.5 rounded text-xs font-medium transition-colors cursor-pointer border ${
              justSet
                ? 'bg-emerald-600/20 text-emerald-400 border-emerald-500/40'
                : 'bg-surface-900 text-slate-300 border-surface-600 hover:text-white hover:border-violet-500/40'
            } disabled:opacity-50`}
          >
            {capturing ? (
              <><Loader2 className="w-3 h-3 animate-spin" /> Capturing...</>
            ) : justSet ? (
              <><Check className="w-3 h-3" /> Saved!</>
            ) : (
              <><Camera className="w-3 h-3" /> Set frame at {fmt(currentTime)} as thumbnail</>
            )}
          </button>

          {error && <p className="text-[10px] text-red-400">{error}</p>}
        </div>
      </div>

      <p className="text-[10px] text-slate-600">
        Pause playback on the frame you want, then click capture.
      </p>
    </div>
  )
}
