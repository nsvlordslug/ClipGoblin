import { useState } from 'react'
import { ExternalLink, Loader2, Share2 } from 'lucide-react'
import { invoke } from '@tauri-apps/api/core'
import {
  buildFacebookShareUrl,
  buildThreadsComposeUrl,
  buildXComposeUrl,
  createXHandoff,
  MANUAL_SHARE_UNAVAILABLE_MESSAGE,
  MAX_X_SUGGESTED_TEXT_LENGTH,
} from '../lib/xHandoff'
import type { XHandoff } from '../lib/xHandoff'
import { errorMessage } from '../lib/errors'

interface XHandoffCardProps {
  platform: string
  publishedUrl: string
  clipTitle: string
  className?: string
  compact?: boolean
}

type ManualShareTarget = 'x' | 'facebook' | 'threads'

const SHARE_TARGET_LABELS: Record<ManualShareTarget, string> = {
  x: 'X',
  facebook: 'Facebook',
  threads: 'Threads',
}

const SHARE_TARGETS: ManualShareTarget[] = ['x', 'facebook', 'threads']

function disabledTooltipPosition(target: ManualShareTarget): string {
  if (target === 'x') return 'left-0'
  if (target === 'threads') return 'right-0'
  return 'left-1/2 -translate-x-1/2'
}

export function ManualShareAvailabilityNote({ className = '' }: { className?: string }) {
  return (
    <div className={`flex items-start gap-2 rounded-md border border-slate-600/60 bg-surface-900/70 px-3 py-2 ${className}`} role="note">
      <Share2 className="mt-0.5 h-3.5 w-3.5 shrink-0 text-slate-400" aria-hidden="true" />
      <p className="text-[11px] leading-relaxed text-slate-400">
        Manual Share actions appear after TikTok or YouTube provides a verified public link. ClipGoblin does not publish to social platforms; you review and post it yourself.
      </p>
    </div>
  )
}

export function ManualShareUnavailableCard({ className = '' }: { className?: string }) {
  return (
    <section className={`rounded-lg border border-slate-600/70 bg-black/20 p-3 space-y-2 ${className}`} aria-label="Manual Share unavailable">
      <div className="flex items-center gap-2 text-xs font-semibold text-white">
        <Share2 className="h-3.5 w-3.5 text-slate-400" aria-hidden="true" />
        Manual Share
      </div>

      <div className="flex flex-wrap gap-2">
        {SHARE_TARGETS.map(target => {
          const targetLabel = SHARE_TARGET_LABELS[target]
          return (
            <span
              key={target}
              tabIndex={0}
              aria-label={`${targetLabel} share unavailable. ${MANUAL_SHARE_UNAVAILABLE_MESSAGE}`}
              className="group relative min-w-[8.5rem] flex-1 rounded-md outline-none focus-visible:ring-2 focus-visible:ring-violet-400/70"
            >
              <button
                type="button"
                disabled
                tabIndex={-1}
                className="flex w-full cursor-not-allowed items-center justify-center gap-1.5 rounded-md border border-slate-600/60 bg-surface-800/60 px-3 py-2 text-xs font-semibold text-slate-500 opacity-60"
              >
                <ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />
                Share on {targetLabel}
              </button>
              <span
                role="tooltip"
                className={`pointer-events-none invisible absolute bottom-full z-50 mb-2 w-64 rounded-md border border-slate-600 bg-surface-950 px-2.5 py-2 text-center text-[10px] font-normal leading-relaxed text-slate-200 opacity-0 shadow-xl transition-opacity group-hover:visible group-hover:opacity-100 group-focus:visible group-focus:opacity-100 ${disabledTooltipPosition(target)}`}
              >
                {MANUAL_SHARE_UNAVAILABLE_MESSAGE}
              </span>
            </span>
          )
        })}
      </div>

      <p className="text-[10px] leading-relaxed text-slate-500">
        {MANUAL_SHARE_UNAVAILABLE_MESSAGE}
      </p>
    </section>
  )
}

function XHandoffCardBody({ handoff, className = '', compact = false }: {
  handoff: XHandoff
  className?: string
  compact?: boolean
}) {
  const [caption, setCaption] = useState(handoff.suggestedCaption)
  const [openingTarget, setOpeningTarget] = useState<ManualShareTarget | null>(null)
  const [openError, setOpenError] = useState<string | null>(null)

  const openShareComposer = async (target: ManualShareTarget) => {
    setOpeningTarget(target)
    setOpenError(null)
    try {
      const url = target === 'x'
        ? buildXComposeUrl(caption, handoff.publishedUrl)
        : target === 'facebook'
          ? buildFacebookShareUrl(handoff.publishedUrl)
          : buildThreadsComposeUrl(caption, handoff.publishedUrl)

      await invoke('open_url', {
        url,
      })
    } catch (error: unknown) {
      setOpenError(errorMessage(error, `Could not open ${SHARE_TARGET_LABELS[target]}`))
    } finally {
      setOpeningTarget(null)
    }
  }

  return (
    <section className={`rounded-lg border border-slate-600/70 bg-black/20 p-3 space-y-2 ${className}`} aria-label={`Manually share ${handoff.platformLabel} post`}>
      <div className="flex items-start gap-2">
        <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-slate-500/60 bg-surface-800 text-slate-200" aria-hidden="true">
          <Share2 className="h-3.5 w-3.5" />
        </span>
        <div className="min-w-0">
          <div className="flex items-center gap-1.5 text-xs font-semibold text-white">
            Share this published clip
          </div>
          <p className="mt-0.5 text-[11px] leading-relaxed text-slate-400">
            ClipGoblin only prepares a manual share. It does not publish to Facebook, Threads, or X.
          </p>
          {!compact && <p className="mt-0.5 text-[11px] leading-relaxed text-slate-500">Edit the suggestion, then review everything in the browser before posting.</p>}
        </div>
      </div>

      <label className="block space-y-1">
        <span className="text-[10px] font-medium uppercase text-slate-500">Suggested caption</span>
        <textarea
          value={caption}
          onChange={(event) => setCaption(event.target.value)}
          maxLength={MAX_X_SUGGESTED_TEXT_LENGTH}
          rows={compact ? 2 : 3}
          className="w-full resize-y rounded-md border border-surface-600 bg-surface-900 px-2.5 py-2 text-xs leading-relaxed text-white outline-none transition-colors focus:border-slate-400"
        />
      </label>

      <div className="min-w-0">
        <div className="text-[10px] font-medium uppercase text-slate-500">Published link</div>
        <div className="truncate text-[11px] text-cyan-300" title={handoff.publishedUrl}>
          {handoff.publishedUrl}
        </div>
      </div>

      {openError && <p role="alert" className="text-[11px] text-red-400">{openError}</p>}

      <div className="flex flex-wrap gap-2">
        {SHARE_TARGETS.map((target) => {
          const opening = openingTarget === target
          const targetLabel = SHARE_TARGET_LABELS[target]
          const colorClass = target === 'facebook'
            ? 'border-blue-400/30 bg-blue-600 text-white hover:bg-blue-500'
            : target === 'x'
              ? 'border-white/20 bg-white text-black hover:bg-slate-200'
              : 'border-slate-500/70 bg-surface-800 text-white hover:bg-surface-700'

          return (
            <button
              key={target}
              type="button"
              onClick={() => void openShareComposer(target)}
              disabled={openingTarget !== null}
              className={`flex min-w-[8.5rem] flex-1 items-center justify-center gap-1.5 rounded-md border px-3 py-2 text-xs font-semibold transition-colors disabled:opacity-60 ${colorClass}`}
            >
              {opening ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ExternalLink className="h-3.5 w-3.5" />}
              {opening ? `Opening ${targetLabel}...` : `Share on ${targetLabel}`}
            </button>
          )
        })}
      </div>

      <p className="text-[10px] leading-relaxed text-slate-500">
        Facebook opens with the published link only. Use the editable caption above if you want to add text there.
      </p>
    </section>
  )
}

export default function XHandoffCard(props: XHandoffCardProps) {
  const handoff = createXHandoff(props.platform, props.publishedUrl, props.clipTitle)
  if (!handoff) return null

  return (
    <XHandoffCardBody
      key={`${handoff.platform}:${handoff.publishedUrl}`}
      handoff={handoff}
      className={props.className}
      compact={props.compact}
    />
  )
}
