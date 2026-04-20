import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Download, ExternalLink, Loader2, X } from 'lucide-react'
import type { Vod } from '../types'
import { useAppStore } from '../stores/appStore'

interface Props {
  open: boolean
  onClose: () => void
  /** Called with the newly imported VOD after success. */
  onImported?: (vod: Vod) => void
}

/** Parse + validate a Twitch VOD URL or bare ID on the client side
 *  so we can preview what the server will see before hitting it. */
function parseVodId(input: string): string | null {
  const trimmed = input.trim()
  if (!trimmed) return null
  if (/^\d+$/.test(trimmed)) return trimmed
  const match = trimmed.toLowerCase().match(/videos\/(\d+)/)
  return match ? match[1] : null
}

export default function ImportVodDialog({ open, onClose, onImported }: Props) {
  const [url, setUrl] = useState('')
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [success, setSuccess] = useState<Vod | null>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const { refreshVods, loggedInUser } = useAppStore()

  // Reset state each time dialog opens; focus input
  useEffect(() => {
    if (open) {
      setUrl('')
      setError(null)
      setSuccess(null)
      setSubmitting(false)
      setTimeout(() => inputRef.current?.focus(), 50)
    }
  }, [open])

  // ESC to close
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape' && !submitting) onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open, submitting, onClose])

  if (!open) return null

  const previewId = parseVodId(url)

  const handleSubmit = async () => {
    if (!previewId || submitting) return
    setSubmitting(true)
    setError(null)
    try {
      const vod = await invoke<Vod>('import_vod_by_url', { url: url.trim() })
      setSuccess(vod)
      if (loggedInUser) await refreshVods(loggedInUser.id).catch(() => {})
      onImported?.(vod)
    } catch (e) {
      setError(String(e))
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onClick={e => { if (e.target === e.currentTarget && !submitting) onClose() }}
    >
      <div className="v4-panel w-full max-w-md mx-4 animate-slide-up" style={{padding: 24}}>
        <div className="flex items-start justify-between mb-4">
          <div>
            <div className="text-[16px] font-bold text-white">📥 Import VOD</div>
            <div className="text-xs text-slate-500 mt-1">
              Paste a Twitch VOD URL to add it to your library. Works for any public VOD, not just your own.
            </div>
          </div>
          {!submitting && (
            <button
              onClick={onClose}
              className="p-1 rounded-lg text-slate-500 hover:text-white hover:bg-surface-700 cursor-pointer"
              aria-label="Close"
            >
              <X className="w-4 h-4" />
            </button>
          )}
        </div>

        {!success ? (
          <>
            <div className="v4-form-field">
              <label className="v4-label">Twitch VOD URL or ID</label>
              <input
                ref={inputRef}
                type="text"
                value={url}
                onChange={e => setUrl(e.target.value)}
                onKeyDown={e => { if (e.key === 'Enter' && previewId && !submitting) handleSubmit() }}
                placeholder="https://www.twitch.tv/videos/2345678901"
                className="v4-input"
                disabled={submitting}
              />
              {previewId && !submitting && (
                <div className="text-[11px] text-emerald-400 mt-1.5">
                  ✓ Will import VOD <span className="font-mono text-white">#{previewId}</span>
                </div>
              )}
              {url.trim() && !previewId && (
                <div className="text-[11px] text-amber-400 mt-1.5">
                  Couldn't find a VOD ID — paste the full URL like <span className="font-mono text-white">twitch.tv/videos/123456</span>
                </div>
              )}
            </div>

            {error && (
              <div className="text-[12px] text-red-400 bg-red-500/10 border border-red-500/30 rounded-lg px-3 py-2 mb-3">
                {error}
              </div>
            )}

            <div className="flex gap-2 justify-end">
              <button
                className="v4-btn ghost"
                onClick={onClose}
                disabled={submitting}
                style={{padding: '8px 14px', fontSize: 13}}
              >
                Cancel
              </button>
              <button
                className="v4-btn primary"
                onClick={handleSubmit}
                disabled={!previewId || submitting}
                style={{padding: '8px 14px', fontSize: 13}}
              >
                {submitting
                  ? <><Loader2 className="w-4 h-4 animate-spin" /> Importing...</>
                  : <><Download className="w-4 h-4" /> Import</>}
              </button>
            </div>
          </>
        ) : (
          /* Success state */
          <div>
            <div className="text-[13px] text-slate-300 bg-emerald-500/10 border border-emerald-500/30 rounded-lg px-3 py-2.5 mb-4">
              <div className="font-semibold text-emerald-300 mb-0.5">✓ VOD imported</div>
              <div className="text-slate-400 truncate" title={success.title}>{success.title}</div>
            </div>
            <div className="flex gap-2 justify-end">
              <button
                className="v4-btn ghost"
                onClick={onClose}
                style={{padding: '8px 14px', fontSize: 13}}
              >
                Close
              </button>
              <a
                href={success.vod_url}
                target="_blank"
                rel="noopener noreferrer"
                className="v4-btn"
                style={{padding: '8px 14px', fontSize: 13, textDecoration: 'none'}}
              >
                <ExternalLink className="w-3.5 h-3.5" />
                View on Twitch
              </a>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
