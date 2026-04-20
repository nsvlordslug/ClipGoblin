import { useEffect, useRef, useState } from 'react'
import { check, Update } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { Download, X, Loader2 } from 'lucide-react'

type Phase = 'idle' | 'checking' | 'available' | 'downloading' | 'installing' | 'done' | 'error' | 'dismissed'

/**
 * Silent on-boot update check. If a new version is available from the GitHub
 * Releases endpoint configured in `tauri.conf.json > plugins.updater`, shows a
 * floating card in the bottom-right with an "Install & restart" button.
 *
 * Skips entirely in `cargo tauri dev` because the `standalone` Cargo feature
 * gates the Rust-side plugin — the `check()` call will error fast with
 * "updater not configured" which we swallow.
 */
export default function UpdateChecker() {
  const [phase, setPhase] = useState<Phase>('idle')
  const [update, setUpdate] = useState<Update | null>(null)
  const [progress, setProgress] = useState(0)
  const [errMsg, setErrMsg] = useState<string | null>(null)
  const checkedOnce = useRef(false)

  useEffect(() => {
    if (checkedOnce.current) return
    checkedOnce.current = true
    setPhase('checking')
    ;(async () => {
      try {
        const result = await check()
        if (result) {
          setUpdate(result)
          setPhase('available')
        } else {
          setPhase('idle')
        }
      } catch (e) {
        // Dev-mode and builds without the `standalone` feature will hit this.
        console.debug('[UpdateChecker] updater unavailable:', e)
        setPhase('idle')
      }
    })()
  }, [])

  const handleInstall = async () => {
    if (!update) return
    setPhase('downloading')
    setProgress(0)
    try {
      let contentLength = 0
      let downloaded = 0
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case 'Started':
            contentLength = event.data.contentLength ?? 0
            break
          case 'Progress':
            downloaded += event.data.chunkLength
            if (contentLength > 0) {
              setProgress(Math.min(100, Math.round((downloaded / contentLength) * 100)))
            }
            break
          case 'Finished':
            setPhase('installing')
            break
        }
      })
      setPhase('done')
      // Tauri installs into place, then relaunch picks up the new binary.
      await relaunch()
    } catch (e) {
      console.error('[UpdateChecker] install failed:', e)
      setErrMsg(String(e))
      setPhase('error')
    }
  }

  if (phase === 'idle' || phase === 'checking' || phase === 'dismissed') return null

  return (
    <div
      className="fixed bottom-4 right-4 z-50 v4-panel max-w-sm animate-slide-up"
      style={{padding: 16}}
    >
      <div className="flex items-start justify-between mb-2">
        <div>
          <div className="text-sm font-bold text-white">
            {phase === 'available' && '🚀 Update available'}
            {phase === 'downloading' && '⬇ Downloading...'}
            {phase === 'installing' && '⚙ Installing...'}
            {phase === 'done' && '✓ Restarting'}
            {phase === 'error' && '⚠ Update failed'}
          </div>
          {update && (
            <div className="text-xs text-slate-400 mt-0.5">
              v{update.version}{update.date ? ` · ${new Date(update.date).toLocaleDateString()}` : ''}
            </div>
          )}
        </div>
        {(phase === 'available' || phase === 'error') && (
          <button
            onClick={() => setPhase('dismissed')}
            className="p-1 rounded text-slate-400 hover:text-white cursor-pointer"
            aria-label="Dismiss"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        )}
      </div>

      {phase === 'available' && update?.body && (
        <p className="text-xs text-slate-300 mb-3 line-clamp-3">
          {update.body}
        </p>
      )}

      {phase === 'downloading' && (
        <div className="my-2">
          <div className="h-1.5 bg-surface-700 rounded-full overflow-hidden">
            <div
              className="h-full bg-gradient-to-r from-violet-500 to-pink-500 transition-all duration-150"
              style={{width: `${progress}%`}}
            />
          </div>
          <div className="text-[10px] text-slate-500 mt-1">{progress}%</div>
        </div>
      )}

      {phase === 'error' && errMsg && (
        <p className="text-xs text-red-400 mb-3">{errMsg}</p>
      )}

      {phase === 'available' && (
        <button
          onClick={handleInstall}
          className="v4-btn primary"
          style={{width: '100%', justifyContent: 'center', padding: '8px 12px', fontSize: 13}}
        >
          <Download className="w-3.5 h-3.5" />
          Install &amp; restart
        </button>
      )}

      {phase === 'installing' && (
        <div className="flex items-center gap-2 text-xs text-slate-400">
          <Loader2 className="w-3.5 h-3.5 animate-spin" />
          Writing files...
        </div>
      )}
    </div>
  )
}
