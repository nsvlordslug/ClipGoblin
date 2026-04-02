import { useEffect, useState, type ReactNode } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { Download, CheckCircle, AlertCircle, Loader2 } from 'lucide-react'
import logoImg from '../assets/logo.png'

interface ModelStatus {
  base: { downloaded: boolean; size_bytes: number };
  medium: { downloaded: boolean; size_bytes: number };
}

type SetupState = 'checking' | 'needed' | 'downloading' | 'done' | 'error' | 'ready'

export default function FirstRunSetup({ children }: { children: ReactNode }) {
  const [state, setState] = useState<SetupState>('checking')
  const [progress, setProgress] = useState(0)
  const [downloadedMb, setDownloadedMb] = useState(0)
  const [errorMsg, setErrorMsg] = useState('')

  const totalMb = 142

  useEffect(() => {
    let cancelled = false
    const check = async () => {
      try {
        const status = await invoke<ModelStatus>('check_model_status')
        if (cancelled) return
        if (status.base.downloaded || status.medium.downloaded) {
          setState('ready')
        } else {
          setState('needed')
        }
      } catch {
        if (!cancelled) setState('ready') // Don't block app on check failure
      }
    }
    check()
    return () => { cancelled = true }
  }, [])

  useEffect(() => {
    if (state !== 'downloading') return
    const unlisten = listen<{ model: string; percent: number; downloaded_bytes: number; total_bytes: number }>(
      'model-download-progress',
      (event) => {
        setProgress(event.payload.percent)
        setDownloadedMb(Math.round(event.payload.downloaded_bytes / 1024 / 1024))
        if (event.payload.percent >= 100) {
          setState('done')
        }
      }
    )
    return () => { unlisten.then(fn => fn()) }
  }, [state])

  const startDownload = async () => {
    setState('downloading')
    setProgress(0)
    setDownloadedMb(0)
    try {
      await invoke('download_model', { modelName: 'base' })
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

  return (
    <div className="flex items-center justify-center h-screen bg-surface-950">
      <div className="text-center max-w-md px-8">
        {/* Logo */}
        <div className="w-20 h-20 mx-auto mb-6 rounded-2xl overflow-hidden shadow-lg shadow-violet-500/20">
          <img src={logoImg} alt="" className="w-full h-full object-cover" />
        </div>

        <h1 className="text-2xl font-bold text-white mb-2">Welcome to ClipGoblin!</h1>

        {state === 'needed' && (
          <>
            <p className="text-sm text-slate-400 mb-2">
              Before we can analyze your VODs, we need to download a small AI model for speech recognition.
            </p>
            <p className="text-xs text-slate-500 mb-8">
              This is a one-time ~{totalMb} MB download.
            </p>
            <button
              onClick={startDownload}
              className="flex items-center gap-2 mx-auto px-6 py-3 bg-violet-600 hover:bg-violet-500 text-white text-sm font-medium rounded-xl transition-colors cursor-pointer shadow-lg shadow-violet-600/30"
            >
              <Download className="w-4 h-4" />
              Download &amp; Get Started
            </button>
          </>
        )}

        {state === 'downloading' && (
          <>
            <p className="text-sm text-slate-400 mb-6">
              Downloading speech recognition model...
            </p>
            {/* Progress bar */}
            <div className="w-full bg-surface-800 rounded-full h-3 mb-3 border border-surface-700 overflow-hidden">
              <div
                className="h-full bg-gradient-to-r from-violet-600 to-violet-400 rounded-full transition-all duration-300"
                style={{ width: `${Math.min(progress, 100)}%` }}
              />
            </div>
            <p className="text-xs text-slate-400">
              Downloading... {downloadedMb} MB / {totalMb} MB ({Math.min(progress, 100)}%)
            </p>
          </>
        )}

        {state === 'done' && (
          <>
            <div className="flex items-center justify-center gap-2 mb-4">
              <CheckCircle className="w-6 h-6 text-emerald-400" />
              <span className="text-lg text-emerald-400 font-medium">Setup complete!</span>
            </div>
            <p className="text-sm text-slate-400 mb-6">
              The speech recognition model is ready. Let's start clipping!
            </p>
            <button
              onClick={() => setState('ready')}
              className="flex items-center gap-2 mx-auto px-6 py-3 bg-violet-600 hover:bg-violet-500 text-white text-sm font-medium rounded-xl transition-colors cursor-pointer shadow-lg shadow-violet-600/30"
            >
              Launch ClipGoblin
            </button>
          </>
        )}

        {state === 'error' && (
          <>
            <div className="flex items-center justify-center gap-2 mb-4">
              <AlertCircle className="w-6 h-6 text-red-400" />
              <span className="text-lg text-red-400 font-medium">Download failed</span>
            </div>
            <p className="text-sm text-slate-400 mb-2">
              Something went wrong downloading the model.
            </p>
            <p className="text-xs text-red-400/80 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2 mb-6 break-words">
              {errorMsg}
            </p>
            <div className="flex gap-3 justify-center">
              <button
                onClick={startDownload}
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
