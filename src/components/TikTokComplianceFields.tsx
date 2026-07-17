import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { LockKeyhole, Send, Smartphone, UserRound } from 'lucide-react'
import type { TikTokComplianceValue } from '../lib/tiktokCompliance'

// Mirrors the Rust `TikTokCreatorInfo` struct (src-tauri/src/social/tiktok.rs).
interface CreatorInfo {
  creator_nickname: string
  creator_username: string
  creator_avatar_url: string
  privacy_level_options: string[]
  comment_disabled: boolean
  duet_disabled: boolean
  stitch_disabled: boolean
  max_video_post_duration_sec: number
}

const PRIVACY_LABELS: Record<string, string> = {
  PUBLIC_TO_EVERYONE: 'Everyone',
  MUTUAL_FOLLOW_FRIENDS: 'Friends',
  FOLLOWER_OF_CREATOR: 'Followers',
  SELF_ONLY: 'Only me (private)',
}

// TikTok-required consent links (Content Sharing Guidelines).
const MUSIC_URL = 'https://www.tiktok.com/legal/page/global/music-usage-confirmation/en'
const BRANDED_POLICY_URL = 'https://www.tiktok.com/legal/page/global/bc-policy/en'

// TikTok restricts unaudited Direct Post clients to SELF_ONLY. Flip this after
// TikTok approves ClipGoblin's Content Posting API audit.
const DIRECT_POST_AUDIT_PENDING = true

// mm:ss formatter for the per-account max-duration hint.
function fmtDuration(totalSec: number): string {
  const s = Math.max(0, Math.round(totalSec))
  const m = Math.floor(s / 60)
  return `${m}:${(s % 60).toString().padStart(2, '0')}`
}

interface Props {
  value: TikTokComplianceValue
  onChange: (v: TikTokComplianceValue) => void
  /** Reports whether the panel is in a postable state so the parent can gate the Post button. */
  onValidityChange?: (valid: boolean) => void
  /** Duration of the clip being posted, in seconds. When provided, the panel
   *  enforces the account's max_video_post_duration_sec (TikTok's Content
   *  Sharing Guidelines require checking video length before posting). */
  clipDurationSec?: number
}

/**
 * TikTok publish-compliance panel. Renders the controls TikTok's Content
 * Sharing Guidelines require on the publish screen — privacy level (sourced
 * live from creator_info, never hardcoded), interaction toggles that honor the
 * account's restrictions, a content-disclosure toggle with the brand/branded
 * sub-options, and the music-usage consent line. Used by both the single-clip
 * publish composer and the batch upload dialog.
 */
export default function TikTokComplianceFields({ value, onChange, onValidityChange, clipDurationSec }: Props) {
  const [info, setInfo] = useState<CreatorInfo | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [avatarFailed, setAvatarFailed] = useState(false)
  const [avatarLoaded, setAvatarLoaded] = useState(false)
  const seededRef = useRef(false)

  // Fetch creator_info on mount (refreshes the token backend-side).
  useEffect(() => {
    let cancelled = false
    setLoading(true)
    setError(null)
    invoke<CreatorInfo>('tiktok_get_creator_info')
      .then(ci => { if (!cancelled) { setInfo(ci); setLoading(false) } })
      .catch(e => { if (!cancelled) { setError(String(e)); setLoading(false) } })
    return () => { cancelled = true }
  }, [])

  useEffect(() => {
    setAvatarFailed(false)
    setAvatarLoaded(false)
  }, [info?.creator_avatar_url])

  // Once info loads, force the account's interaction restrictions into the value
  // (e.g. an account with duets disabled must send disable_duet=true). One-time.
  useEffect(() => {
    if (!info || seededRef.current) return
    seededRef.current = true
    const privacyLevel = DIRECT_POST_AUDIT_PENDING
      && info.privacy_level_options.includes('SELF_ONLY')
      ? 'SELF_ONLY'
      : value.privacyLevel
    onChange({
      ...value,
      privacyLevel,
      disableComment: info.comment_disabled || value.disableComment,
      disableDuet: info.duet_disabled || value.disableDuet,
      disableStitch: info.stitch_disabled || value.disableStitch,
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [info])

  // Validity: must pick a privacy level; if disclosing, pick ≥1 brand type;
  // branded content is incompatible with an "Only me" audience; the clip must
  // not exceed the account's max post duration.
  const isDraft = value.publishMode === 'draft'
  const brandedOnPrivate = value.brandedContent && value.privacyLevel === 'SELF_ONLY'
  const discloseMissing = value.discloseContent && !value.yourBrand && !value.brandedContent
  const maxDurationSec = info?.max_video_post_duration_sec ?? 0
  const directDurationExceeded = clipDurationSec != null && maxDurationSec > 0 && clipDurationSec > maxDurationSec
  const draftDurationExceeded = clipDurationSec != null && clipDurationSec > 600
  const privacyOptions = info && DIRECT_POST_AUDIT_PENDING
    ? info.privacy_level_options.includes('SELF_ONLY')
      ? ['SELF_ONLY']
      : info.privacy_level_options
    : info?.privacy_level_options ?? []
  const valid = !!info && !error && (isDraft
    ? !draftDurationExceeded
    : value.privacyLevel != null
      && privacyOptions.includes(value.privacyLevel)
      && !discloseMissing && !brandedOnPrivate && !directDurationExceeded)
  useEffect(() => {
    onValidityChange?.(valid)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [valid])

  const set = (patch: Partial<TikTokComplianceValue>) => onChange({ ...value, ...patch })

  if (loading) {
    return <div className="text-xs text-slate-400 px-3 py-2">Loading TikTok account settings…</div>
  }
  if (error || !info) {
    return (
      <div className="bg-red-500/10 border border-red-500/30 rounded-lg px-3 py-2 text-xs text-red-300">
        Couldn't load TikTok settings: {error || 'unknown error'}.
        <br />Reconnect TikTok in Settings and try again.
      </div>
    )
  }

  // Exact TikTok-required disclosure labels (Content Sharing Guidelines).
  const discloseLabel = value.brandedContent
    ? "Your video will be labeled as 'Paid partnership'."
    : value.yourBrand
      ? "Your video will be labeled as 'Promotional content'."
      : null
  return (
    <div className="space-y-3 border border-surface-600 rounded-lg p-3 bg-surface-900/40">
      {/* Posting as */}
      <div className="flex items-center gap-2">
        <div className="relative flex h-6 w-6 shrink-0 items-center justify-center overflow-hidden rounded-full bg-surface-700 text-slate-400">
          {!avatarLoaded && <UserRound className="h-3.5 w-3.5" aria-hidden="true" />}
          {info.creator_avatar_url && !avatarFailed && (
            <img
              src={info.creator_avatar_url}
              alt=""
              referrerPolicy="no-referrer"
              onLoad={() => setAvatarLoaded(true)}
              onError={() => {
                setAvatarLoaded(false)
                setAvatarFailed(true)
              }}
              className={`absolute inset-0 h-full w-full object-cover ${avatarLoaded ? 'block' : 'invisible'}`}
            />
          )}
        </div>
        <span className="text-xs text-slate-300">
          Posting to TikTok as{' '}
          <span className="font-semibold text-white">
            {info.creator_nickname || `@${info.creator_username}`}
          </span>
        </span>
      </div>

      <div>
        <label className="text-[11px] uppercase tracking-wider text-slate-400 font-semibold block mb-1">
          Send to TikTok as
        </label>
        <div className="grid grid-cols-2 rounded-md border border-surface-600 bg-surface-900 p-0.5" role="group" aria-label="TikTok publishing mode">
          <button
            type="button"
            onClick={() => set({ publishMode: 'direct' })}
            className={`flex min-w-0 items-center justify-center gap-1.5 rounded px-2 py-1.5 text-xs transition-colors ${
              !isDraft ? 'bg-violet-600 text-white' : 'text-slate-400 hover:text-white'
            }`}
          >
            <Send className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
            Post directly
          </button>
          <button
            type="button"
            onClick={() => set({ publishMode: 'draft' })}
            className={`flex min-w-0 items-center justify-center gap-1.5 rounded px-2 py-1.5 text-xs transition-colors ${
              isDraft ? 'bg-cyan-600 text-white' : 'text-slate-400 hover:text-white'
            }`}
          >
            <Smartphone className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
            Send to drafts
          </button>
        </div>
      </div>

      {isDraft && (
        <div className="space-y-1.5 rounded border border-cyan-500/30 bg-cyan-500/10 px-3 py-2 text-[11px] leading-relaxed text-cyan-100">
          <p className="font-semibold">Finish this draft in the TikTok app</p>
          <p>
            ClipGoblin sends the rendered video to your TikTok inbox without publishing it.
            Open TikTok's inbox notification to edit the video, add its caption, choose an
            audience, and publish.
          </p>
          <p className="text-cyan-200/80">
            TikTok's draft API transfers the video only, so the caption and hashtags shown
            above must be added inside TikTok.
          </p>
          {draftDurationExceeded && (
            <p className="font-medium text-red-300">TikTok drafts must be 10 minutes or shorter.</p>
          )}
        </div>
      )}

      {!isDraft && (
        <>

      {/* Max length hint + duration gate — TikTok requires checking
          max_video_post_duration_sec before posting. */}
      {maxDurationSec > 0 && (
        <p className={`text-[10px] ${directDurationExceeded ? 'text-red-400' : 'text-slate-500'}`}>
          {directDurationExceeded
            ? `This clip is ${fmtDuration(clipDurationSec!)} — your TikTok account allows videos up to ${fmtDuration(maxDurationSec)}. Trim it shorter to post.`
            : `Max video length for your TikTok account: ${fmtDuration(maxDurationSec)}.`}
        </p>
      )}

      {DIRECT_POST_AUDIT_PENDING && (
        <div className="flex items-start gap-2 rounded border border-amber-500/30 bg-amber-500/10 px-2.5 py-2 text-[10px] leading-relaxed text-amber-200">
          <LockKeyhole className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
          <span>
            TikTok approval is pending. TikTok only permits <strong>Only me (private)</strong>
            {' '}posts while ClipGoblin's Direct Post integration is under review. Wider audiences
            will unlock after TikTok approves the app.
          </span>
        </div>
      )}

      {/* Privacy level — options come straight from creator_info */}
      <div>
        <label className="text-[11px] uppercase tracking-wider text-slate-400 font-semibold block mb-1">
          Who can view this video <span className="text-red-400">*</span>
        </label>
        <select
          value={value.privacyLevel ?? ''}
          onChange={e => {
            const lvl = e.target.value || null
            // Branded content can't be private — drop it if user picks Only me.
            set({ privacyLevel: lvl, brandedContent: lvl === 'SELF_ONLY' ? false : value.brandedContent })
          }}
          className="w-full px-3 py-1.5 bg-surface-800 border border-surface-600 rounded text-sm text-white focus:outline-none focus:border-violet-500"
        >
          <option value="" disabled>Select…</option>
          {privacyOptions.map(lvl => (
            <option key={lvl} value={lvl}>{PRIVACY_LABELS[lvl] || lvl}</option>
          ))}
        </select>
      </div>

      {/* Interaction toggles — greyed + forced off where the account restricts them */}
      <div>
        <label className="text-[11px] uppercase tracking-wider text-slate-400 font-semibold block mb-1">Allow users to</label>
        <div className="flex gap-4">
          {([
            ['Comment', 'disableComment', info.comment_disabled],
            ['Duet', 'disableDuet', info.duet_disabled],
            ['Stitch', 'disableStitch', info.stitch_disabled],
          ] as const).map(([lbl, key, forced]) => (
            <label key={key} className={`flex items-center gap-1.5 text-xs ${forced ? 'opacity-40' : 'text-slate-300'}`}>
              <input
                type="checkbox"
                disabled={forced}
                checked={!value[key]}
                onChange={e => set({ [key]: !e.target.checked } as Partial<TikTokComplianceValue>)}
                className="w-3.5 h-3.5 rounded border-surface-600 bg-surface-800 text-violet-500"
              />
              {lbl}{forced ? ' (off)' : ''}
            </label>
          ))}
        </div>
      </div>

      {/* Content disclosure */}
      <div>
        <label className="flex items-center gap-2 cursor-pointer">
          <input
            type="checkbox"
            checked={value.discloseContent}
            onChange={e => set({ discloseContent: e.target.checked, yourBrand: false, brandedContent: false })}
            className="w-4 h-4 rounded border-surface-600 bg-surface-800 text-violet-500"
          />
          <span className="text-xs text-slate-300">Disclose video content</span>
        </label>
        <p className="text-[10px] text-slate-500 ml-6 mt-0.5">Turn on if this promotes a brand, product, or service.</p>

        {value.discloseContent && (
          <div className="ml-6 mt-2 space-y-1.5">
            <label className="flex items-center gap-2 text-xs text-slate-300">
              <input type="checkbox" checked={value.yourBrand}
                onChange={e => set({ yourBrand: e.target.checked })}
                className="w-3.5 h-3.5 rounded border-surface-600 bg-surface-800 text-violet-500" />
              Your brand <span className="text-slate-500">— promoting yourself / your own business</span>
            </label>
            <label className={`flex items-center gap-2 text-xs ${value.privacyLevel === 'SELF_ONLY' ? 'opacity-40' : 'text-slate-300'}`}>
              <input type="checkbox" checked={value.brandedContent}
                disabled={value.privacyLevel === 'SELF_ONLY'}
                onChange={e => set({ brandedContent: e.target.checked })}
                className="w-3.5 h-3.5 rounded border-surface-600 bg-surface-800 text-violet-500" />
              Branded content <span className="text-slate-500">— paid partnership with a brand</span>
            </label>
            {discloseMissing && <p className="text-[10px] text-amber-400">You need to indicate if your content promotes yourself, a third party, or both.</p>}
            {value.privacyLevel === 'SELF_ONLY' && (
              <p className="text-[10px] text-amber-400">Branded content can't be set to "Only me" — pick a wider audience to enable it.</p>
            )}
            {discloseLabel && <p className="text-[10px] text-slate-400">{discloseLabel}</p>}
          </div>
        )}
      </div>

      {/* Consent declaration (required; exact TikTok wording). When branded
          content is disclosed, the Branded Content Policy is listed FIRST. */}
      <p className="text-[10px] text-slate-500 leading-relaxed">
        By posting, you agree to TikTok's{' '}
        {value.brandedContent && (
          <>
            <a href={BRANDED_POLICY_URL} target="_blank" rel="noopener noreferrer" className="text-violet-400 hover:underline">Branded Content Policy</a>
            {' '}and{' '}
          </>
        )}
        <a href={MUSIC_URL} target="_blank" rel="noopener noreferrer" className="text-violet-400 hover:underline">Music Usage Confirmation</a>.
      </p>

      {/* Post-publish processing notice (required by TikTok's Content Sharing Guidelines). */}
      <p className="text-[10px] text-slate-500 leading-relaxed">
        After you post, it may take a few minutes for your video to process and be visible on TikTok.
      </p>
        </>
      )}
    </div>
  )
}
