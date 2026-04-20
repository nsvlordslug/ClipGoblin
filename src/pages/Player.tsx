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
      <div className="v4-page-header">
        <button
          onClick={() => navigate('/vods')}
          className="v4-btn ghost"
        >
          <ArrowLeft className="w-4 h-4" />
          Back to VODs
        </button>
      </div>

      {error ? (
        <div className="v4-panel text-center p-8" style={{background:'rgba(248,113,113,0.08)',borderColor:'rgba(248,113,113,0.3)'}}>
          <Video className="w-10 h-10 text-red-400 mx-auto mb-3" />
          <p className="text-red-400">{error}</p>
        </div>
      ) : vod ? (
        <div className="space-y-4">
          <div className="v4-panel">
            <h1 className="v4-page-title mb-3" style={{fontSize: 20}}>{vod.title}</h1>
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
            <p className="text-xs text-slate-500 font-mono truncate mt-3">
              {vod.local_path}
            </p>
          </div>
        </div>
      ) : (
        <div className="v4-panel text-center p-12">
          <p className="text-slate-400 text-sm">Loading video...</p>
        </div>
      )}
    </div>
  )
}
