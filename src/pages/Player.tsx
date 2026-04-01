import { useEffect, useState } from 'react'
import { useParams, useNavigate } from 'react-router-dom'
import { invoke, convertFileSrc } from '@tauri-apps/api/core'
import { ArrowLeft, Video } from 'lucide-react'
import type { Vod } from '../types'
import ClipPlayer from '../components/ClipPlayer'
import { usePlaybackStore } from '../stores/playbackStore'

export default function Player() {
  const { vodId } = useParams()
  const navigate = useNavigate()
  const stopAll = usePlaybackStore(s => s.stopAll)

  const [vod, setVod] = useState<Vod | null>(null)
  const [videoSrc, setVideoSrc] = useState<string | null>(null)
  const [error, setError] = useState('')

  // Stop any other playback when VOD player opens
  useEffect(() => { stopAll() }, [])

  useEffect(() => {
    if (!vodId) return
    invoke<Vod>('get_vod_detail', { vodId })
      .then((v) => {
        setVod(v)
        if (v.local_path) {
          setVideoSrc(convertFileSrc(v.local_path))
        } else {
          setError('Video file not found. The file may have been moved or deleted.')
        }
      })
      .catch((err) => setError(String(err)))
  }, [vodId])

  return (
    <div className="space-y-4">
      <button
        onClick={() => navigate('/vods')}
        className="flex items-center gap-2 text-slate-400 hover:text-white text-sm transition-colors cursor-pointer"
      >
        <ArrowLeft className="w-4 h-4" />
        Back to VODs
      </button>

      {error ? (
        <div className="bg-red-500/10 border border-red-500/30 rounded-xl p-8 text-center">
          <Video className="w-10 h-10 text-red-400 mx-auto mb-3" />
          <p className="text-red-400">{error}</p>
        </div>
      ) : vod ? (
        <div className="space-y-4">
          <h1 className="text-xl font-bold text-white">{vod.title}</h1>
          <div className="rounded-xl overflow-hidden bg-black aspect-video max-h-[75vh]">
            <ClipPlayer
              src={videoSrc}
              clipStart={0}
              clipEnd={vod.duration_seconds}
              mode="full"
              controlsOverlay
              className="h-full"
            />
          </div>
          <p className="text-xs text-slate-500 font-mono truncate">
            {vod.local_path}
          </p>
        </div>
      ) : (
        <div className="bg-surface-800 border border-surface-700 rounded-xl p-12 text-center">
          <p className="text-slate-400 text-sm">Loading video...</p>
        </div>
      )}
    </div>
  )
}
