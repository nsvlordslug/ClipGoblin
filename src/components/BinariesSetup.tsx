import { useEffect, useState, type ReactNode } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { Download, CheckCircle, AlertCircle, Loader2 } from 'lucide-react'
import logoImg from '../assets/logo.png'

interface BinaryStatus {
  ffmpegAvailable: boolean
  ffprobeAvailable: boolean
  ytdlpAvailable: boolean
  ffmpegBundled: boolean
  ytdlpBundled: boolean
}

type State = 'checking' | 'needed' | 'downloading' | 'done' | 'error' | 'ready'

interface ProgressEvent {
  binary: 'ffmpeg' | 'yt-dlp'
  downloaded: number
  total: number
  phase: 'downloading' | 'extracting' | 'done'
}

const FFMPEG_APPROX_MB = 220
const YTDLP_APPROX_MB = 20

export default function BinariesSetup({ children }: { children: ReactNode }) {
  const [state, setState] = useState<State>('checking')
  const [ffmpegProgress, setFfmpegProgress] = useState<{ downloaded: number; total: number; phase: string } | null>(null)
  const [ytdlpProgress, setYtdlpProgress] = useState<{ downloaded: number; total: number; phase: string } | null>(null)
  const [errorMsg, setErrorMsg] = useState('')
  const [needsFfmpeg, setNeedsFfmpeg] = useState(false)
  const [needsYtdlp, setNeedsYtdlp] = useState(false)

  useEffect(() => {
    let cancelled = false
    ;(async () => {
      try {
        const status = await invoke<BinaryStatus>('check_binary_status')
        if (cancelled) return
        const missingFfmpeg = !status.ffmpegAvailable || !status.ffprobeAvailable
        const missingYtdlp = !status.ytdlpAvailable
        setNeedsFfmpeg(missingFfmpeg)
        setNeedsYtdlp(missingYtdlp)
        setState(missingFfmpeg || missingYtdlp ? 'needed' : 'ready')
      } catch {
        if (!cancelled) setState('ready')
      }
    })()
    return () => { cancelled = true }
  }, [])

  useEffect(() => {
    if (state !== 'downloading') return
    const unlisten = listen<ProgressEvent>('download-progress', (ev) => {
      const { binary, downloaded, total, phase } = ev.payload
      if (binary === 'ffmpeg') setFfmpegProgress({ downloaded, total, phase })
      else if (binary === 'yt-dlp') setYtdlpProgress({ downloaded, total, phase })
    })
    return () => { unlisten.then(fn => fn()) }
  }, [state])

  const start = async () => {
    setState('downloading')
    setFfmpegProgress(null)
    setYtdlpProgress(null)
    setErrorMsg('')
    try {
      await invoke('download_binaries')
      setState('done')
    } catch (err) {
      setErrorMsg(String(err))
      setState('error')
    }
  }

  if (state === 'checking') {
    return (
      <div className="flex items-center justify-center h-screen bg-surface-950">
        <Loader2 className="w-8 h-8 text-violet-400 animate-spin" />
      </div>
    )
  }

  if (state === 'ready') return <>{children}</>

  const pct = (p: { downloaded: number; total: number } | null) => {
    if (!p || !p.total) return 0
    return Math.min(100, Math.round((p.downloaded / p.total) * 100))
  }

  const mb = (bytes: number) => Math.round(bytes / 1_000_000)

  return (
    <div className="flex items-center justify-center h-screen bg-surface-950">
      <div className="text-center max-w-md px-8">
        <div className="w-20 h-20 mx-auto mb-6 rounded-2xl overflow-hidden shadow-lg shadow-violet-500/20">
          <img src={logoImg} alt="" className="w-full h-full object-cover" />
        </div>

        <h1 className="text-2xl font-bold text-white mb-2">One-time setup</h1>

        {state === 'needed' && (
          <>
            <p className="text-sm text-slate-400 mb-2">
              ClipGoblin needs a couple of helper tools to download and process your VODs.
            </p>
            <ul className="text-xs text-slate-500 mb-6 space-y-1">
              {needsFfmpeg && <li>ffmpeg (~{FFMPEG_APPROX_MB} MB) — video processing</li>}
              {needsYtdlp && <li>yt-dlp (~{YTDLP_APPROX_MB} MB) — Twitch VOD downloads</li>}
            </ul>
            <button
              onClick={start}
              className="flex items-center gap-2 mx-auto px-6 py-3 bg-violet-600 hover:bg-violet-500 text-white text-sm font-medium rounded-xl transition-colors cursor-pointer shadow-lg shadow-violet-600/30"
            >
              <Download className="w-4 h-4" />
              Download &amp; Get Started
            </button>
          </>
        )}

        {state === 'downloading' && (
          <div className="space-y-5 text-left">
            {needsYtdlp && (
              <div>
                <div className="flex justify-between text-xs text-slate-400 mb-1">
                  <span>yt-dlp{ytdlpProgress?.phase === 'done' ? ' (done)' : ''}</span>
                  <span>
                    {ytdlpProgress ? `${mb(ytdlpProgress.downloaded)} / ${ytdlpProgress.total ? mb(ytdlpProgress.total) + ' MB' : '...'}` : 'waiting...'}
                  </span>
                </div>
                <div className="w-full bg-surface-800 rounded-full h-2 border border-surface-700 overflow-hidden">
                  <div className="h-full bg-gradient-to-r from-violet-600 to-violet-400 rounded-full transition-all duration-300" style={{ width: `${pct(ytdlpProgress)}%` }} />
                </div>
              </div>
            )}
            {needsFfmpeg && (
              <div>
                <div className="flex justify-between text-xs text-slate-400 mb-1">
                  <span>ffmpeg{ffmpegProgress?.phase === 'extracting' ? ' — extracting...' : ffmpegProgress?.phase === 'done' ? ' (done)' : ''}</span>
                  <span>
                    {ffmpegProgress ? `${mb(ffmpegProgress.downloaded)} / ${ffmpegProgress.total ? mb(ffmpegProgress.total) + ' MB' : '...'}` : 'waiting...'}
                  </span>
                </div>
                <div className="w-full bg-surface-800 rounded-full h-2 border border-surface-700 overflow-hidden">
                  <div className="h-full bg-gradient-to-r from-violet-600 to-violet-400 rounded-full transition-all duration-300" style={{ width: `${pct(ffmpegProgress)}%` }} />
                </div>
              </div>
            )}
          </div>
        )}

        {state === 'done' && (
          <>
            <div className="flex items-center justify-center gap-2 mb-4">
              <CheckCircle className="w-6 h-6 text-emerald-400" />
              <span className="text-lg text-emerald-400 font-medium">Setup complete!</span>
            </div>
            <button
              onClick={() => setState('ready')}
              className="flex items-center gap-2 mx-auto px-6 py-3 bg-violet-600 hover:bg-violet-500 text-white text-sm font-medium rounded-xl transition-colors cursor-pointer shadow-lg shadow-violet-600/30"
            >
              Continue
            </button>
          </>
        )}

        {state === 'error' && (
          <>
            <div className="flex items-center justify-center gap-2 mb-4">
              <AlertCircle className="w-6 h-6 text-red-400" />
              <span className="text-lg text-red-400 font-medium">Download failed</span>
            </div>
            <p className="text-xs text-red-400/80 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2 mb-6 break-words">
              {errorMsg}
            </p>
            <p className="text-[11px] text-slate-500 mb-6">
              If antivirus blocked the download, allow it and retry.
            </p>
            <div className="flex gap-3 justify-center">
              <button
                onClick={start}
                className="flex items-center gap-2 px-5 py-2.5 bg-violet-600 hover:bg-violet-500 text-white text-sm font-medium rounded-xl transition-colors cursor-pointer"
              >
                <Download className="w-4 h-4" />
                Retry
              </button>
              <button
                onClick={() => setState('ready')}
                className="px-5 py-2.5 bg-surface-800 border border-surface-700 text-slate-400 hover:text-white text-sm rounded-xl transition-colors cursor-pointer"
              >
                Skip for now
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
