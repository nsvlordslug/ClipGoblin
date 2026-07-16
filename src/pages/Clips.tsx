import { useEffect, useState, useCallback, useMemo, useRef } from 'react'
import { useNavigate, useLocation } from 'react-router-dom'
import {
  Scissors, Trash2, Pencil, Film, CheckSquare, Square, Upload, X, Clock,
  ChevronDown, ChevronRight, ArrowUp, LocateFixed, SlidersHorizontal,
  ArrowUpDown, Eye, EyeOff, Undo2, Loader2, Brain, ListChecks, FileVideo,
} from 'lucide-react'
import { invoke, convertFileSrc } from '@tauri-apps/api/core'
import { useAppStore } from '../stores/appStore'
import { useUiStore } from '../stores/uiStore'
import ClipPlayer from '../components/ClipPlayer'
import Tooltip from '../components/Tooltip'
import BatchUploadDialog from '../components/BatchUploadDialog'
import { useScheduleStore } from '../stores/scheduleStore'
import type { Clip, Vod } from '../types'
import type { ClipReviewIssue, ClipReviewRating, PersonalizationStatus } from '../types/clipReview'
import {
  getPersonalizationStatusCopy,
  parseClipReviewIssues,
  REVIEW_ISSUE_OPTIONS,
  REVIEW_RATING_LABELS,
  REVIEW_RATING_COLORS,
  toggleExpandedReviewClip,
} from '../types/clipReview'
import TwitchProvenanceBadges from '../components/TwitchProvenanceBadges'
import {
  CLIP_SOURCE_TABS,
  clipMatchesSourceTab,
  clipSourceTabFor,
  countClipsBySource,
} from '../lib/clipSources'
import type { ClipSourceTab } from '../lib/clipSources'

// ─── Helpers ───────────────────────────────────────────────────────

function formatDate(dateStr: string) {
  try {
    return new Date(dateStr).toLocaleDateString(undefined, {
      month: 'short', day: 'numeric', year: 'numeric',
    })
  } catch {
    return dateStr
  }
}

function legacyToConfidence(virality: number): number {
  const n = Math.max(0, Math.min(virality * 0.85 - 0.10, 0.99))
  const anchors: [number, number][] = [
    [0.00, 0.00], [0.25, 0.25], [0.40, 0.55], [0.50, 0.65],
    [0.60, 0.77], [0.70, 0.84], [0.80, 0.89], [0.90, 0.93],
  ]
  if (n >= 0.90) return Math.min(0.93 + (n - 0.90) * 0.20, 0.95)
  for (let i = 1; i < anchors.length; i++) {
    if (n <= anchors[i][0]) {
      const [x0, y0] = anchors[i - 1]
      const [x1, y1] = anchors[i]
      return y0 + ((n - x0) / (x1 - x0)) * (y1 - y0)
    }
  }
  return 0.95
}

function clipDisplayTitle(
  clip: Clip,
  highlight?: { description?: string; transcript_snippet?: string },
): string {
  if (clip.title && clip.title.trim().length > 0) return clip.title
  const snippet = highlight?.transcript_snippet?.trim()
  if (snippet && snippet.length >= 5) {
    if (snippet.length <= 50) return `"${snippet}"`
    const words = snippet.split(/\s+/).slice(0, 6).join(' ')
    return `"${words}..."`
  }
  return 'Untitled Clip'
}

// ─── Persistence helpers (sessionStorage for ephemeral UI state) ──

const STORAGE_KEY_COLLAPSED = 'clips_collapsed_vods'
const STORAGE_KEY_SORT = 'clips_sort'
const STORAGE_KEY_HIDE_DONE = 'clips_hide_done'

function loadCollapsed(): Set<string> {
  try {
    const raw = sessionStorage.getItem(STORAGE_KEY_COLLAPSED)
    if (raw) return new Set(JSON.parse(raw))
  } catch { /* ignore */ }
  return new Set()
}

function saveCollapsed(ids: Set<string>) {
  try { sessionStorage.setItem(STORAGE_KEY_COLLAPSED, JSON.stringify([...ids])) } catch { /* ignore */ }
}

type SortBy = 'stream_date' | 'download_date'
type SortDir = 'desc' | 'asc'

function loadSort(): { by: SortBy; dir: SortDir } {
  try {
    const raw = sessionStorage.getItem(STORAGE_KEY_SORT)
    if (raw) return JSON.parse(raw)
  } catch { /* ignore */ }
  return { by: 'stream_date', dir: 'desc' }
}

function saveSort(s: { by: SortBy; dir: SortDir }) {
  try { sessionStorage.setItem(STORAGE_KEY_SORT, JSON.stringify(s)) } catch { /* ignore */ }
}

function loadHideDone(): boolean {
  try { return sessionStorage.getItem(STORAGE_KEY_HIDE_DONE) === 'true' } catch { return false }
}

function saveHideDone(v: boolean) {
  try { sessionStorage.setItem(STORAGE_KEY_HIDE_DONE, v ? 'true' : 'false') } catch { /* ignore */ }
}

// Settings preference for confirm-on-delete (persists across sessions)
function getConfirmDeletePref(): boolean {
  try { return localStorage.getItem('clips_confirm_delete') !== 'false' } catch { return true }
}

// ─── Last-edited clip ref (module-level so it survives this page's unmount
// while the Editor is mounted; cleared after the scroll restore runs) ───

const lastEditedClipIdRef: { current: string | null } = { current: null }

// Scroll a clip/VOD card into its REAL scroll container. The content pane has
// `overflow-y-auto` but does NOT itself overflow — the element that actually
// scrolls is a NESTED ancestor — so picking the first `overflow-y:auto` element
// (the old approach) scrolled nothing and the page stayed pinned at the top.
// We walk up for the nearest ancestor that BOTH overflows and is scrollable, and
// scroll that; if nothing overflows, fall back to the browser's own
// scrollIntoView (the document scrolls). Both the editor-return and
// scroll-to-VOD flows share this so they can't drift apart again.
function scrollCardIntoView(el: HTMLElement, block: 'start' | 'center'): void {
  let scroller: HTMLElement | null = null
  for (let p = el.parentElement; p; p = p.parentElement) {
    if (p.scrollHeight > p.clientHeight + 4) {
      const oy = window.getComputedStyle(p).overflowY
      if (oy === 'auto' || oy === 'scroll' || oy === 'overlay') { scroller = p; break }
    }
  }
  if (!scroller) {
    el.scrollIntoView({ block, behavior: 'instant' as ScrollBehavior })
    return
  }
  const elRect = el.getBoundingClientRect()
  const base = scroller.scrollTop + (elRect.top - scroller.getBoundingClientRect().top)
  const target = block === 'center' ? base - scroller.clientHeight / 2 + elRect.height / 2 : base
  scroller.scrollTop = Math.max(0, target)
}

// ─── Undo toast state ─────────────────────────────────────────────

interface PendingDelete {
  clipIds: string[]
  timer: ReturnType<typeof setTimeout>
}

// ─── Clip card ────────────────────────────────────────────────────

function ClipCard({ clip, highlight, confidence, posterSrc, onDelete, onEdit, onReviewSaved, reviewExpanded, onToggleReview, selectMode, selected, onToggleSelect, scheduledPlatforms }: {
  clip: Clip
  highlight?: {
    id?: string
    description?: string
    tags?: string | string[]
    signal_sources?: string | null
    transcript_snippet?: string
    review_rating?: 'good' | 'meh' | 'boring' | null
    review_note?: string | null
    review_issues?: string | null
  }
  confidence: number | null
  posterSrc: string | null
  onDelete: () => void
  onEdit: () => void
  onReviewSaved: () => Promise<void>
  reviewExpanded: boolean
  onToggleReview: () => void
  selectMode: boolean
  selected: boolean
  onToggleSelect: () => void
  scheduledPlatforms?: Array<{ platform: string; scheduled_time: string }>
}) {
  const [videoSrc, setVideoSrc] = useState<string | null>(null)
  const [preparingPreview, setPreparingPreview] = useState(false)
  const [previewError, setPreviewError] = useState<string | null>(null)
  const showReviewTools = useUiStore((s) => s.settings.showReviewTools)
  const fetchHighlights = useAppStore((s) => s.fetchHighlights)
  const [savingReview, setSavingReview] = useState(false)
  const [reviewError, setReviewError] = useState<string | null>(null)
  const [reviewNoteDraft, setReviewNoteDraft] = useState(highlight?.review_note ?? '')
  const reviewIssues = useMemo(
    () => parseClipReviewIssues(highlight?.review_issues),
    [highlight?.review_issues],
  )
  const hasReviewFeedback = Boolean(
    highlight?.review_rating
    || reviewIssues.length > 0
    || highlight?.review_note?.trim(),
  )

  useEffect(() => {
    setReviewNoteDraft(highlight?.review_note ?? '')
  }, [highlight?.review_note])

  const persistReview = async (
    rating: ClipReviewRating | null,
    note: string | null,
    issues: ClipReviewIssue[],
  ) => {
    const highlightId = highlight?.id ?? clip.highlight_id
    if (!highlightId) return
    setSavingReview(true)
    setReviewError(null)
    try {
      await invoke('save_clip_review', {
        highlightId,
        rating,
        note,
        issues,
      })
      await fetchHighlights()
      await onReviewSaved()
    } catch (error) {
      console.error('Failed to save clip review:', error)
      setReviewError(String(error))
    } finally {
      setSavingReview(false)
    }
  }

  const displayTitle = useMemo(
    () => clipDisplayTitle(clip, highlight),
    [clip, highlight]
  )

  const ensureSrc = useCallback(async () => {
    if (videoSrc || preparingPreview) return
    setPreviewError(null)
    if (clip.source_media_path) {
      setPreparingPreview(true)
      try {
        const path = await invoke<string>('prepare_clip_preview_source', { clipId: clip.id })
        setVideoSrc(convertFileSrc(path))
      } catch (error) {
        console.warn(`[Clips] Failed to prepare imported preview for ${clip.id}`, error)
        setPreviewError(String(error))
      } finally {
        setPreparingPreview(false)
      }
      return
    }
    // Community-clip MP4: a standalone, already-trimmed file. Play it directly
    // (no VOD lookup, no seek/trim window). Falls back to the VOD source.
    if (clip.community_clip_mp4_path) {
      setVideoSrc(convertFileSrc(clip.community_clip_mp4_path))
      return
    }
    try {
      const v = await invoke<Vod>('get_vod_detail', { vodId: clip.vod_id })
      if (v.local_path) setVideoSrc(convertFileSrc(v.local_path))
    } catch (error) {
      console.warn(`[Clips] Failed to load VOD source for clip ${clip.id}`)
      setPreviewError(String(error))
    }
  }, [videoSrc, preparingPreview, clip.id, clip.vod_id, clip.community_clip_mp4_path, clip.source_media_path])

  const handleClick = () => {
    if (selectMode) onToggleSelect()
    else ensureSrc()
  }

  const isViral = confidence !== null && confidence >= 0.9

  return (
    <div
      data-clip-id={clip.id}
      className={`v4-lib-clip relative flex flex-col transition-all ${
        selectMode && selected
          ? '!border-violet-500 ring-1 ring-violet-500/30'
          : ''
      } ${reviewExpanded ? 'review-open !border-violet-500/60' : ''} ${selectMode ? 'cursor-pointer' : ''}`}
      onClick={handleClick}
    >
      {selectMode && (
        <div className="absolute top-2 left-2 z-10">
          {selected
            ? <CheckSquare className="w-5 h-5 text-violet-400" />
            : <Square className="w-5 h-5 text-slate-500" />
          }
        </div>
      )}

      <div className="relative aspect-video overflow-hidden bg-black">
        <ClipPlayer
          src={selectMode ? null : videoSrc}
          poster={posterSrc}
          clipStart={clip.start_seconds}
          clipEnd={clip.end_seconds}
          fullFile={!!clip.community_clip_mp4_path}
          mode="compact"
          className="w-full h-full"
          objectFit="contain"
        />
        {preparingPreview && (
          <div className="absolute inset-0 z-10 flex items-center justify-center gap-2 bg-black/70 text-xs text-slate-200">
            <Loader2 className="h-4 w-4 animate-spin" /> Preparing preview
          </div>
        )}
        {previewError && !preparingPreview && (
          <button
            type="button"
            className="absolute inset-x-2 bottom-2 z-10 rounded bg-red-950/90 px-2 py-1.5 text-left text-[10px] text-red-200"
            title={previewError}
            onClick={(event) => {
              event.stopPropagation()
              void ensureSrc()
            }}
          >
            Preview unavailable. Click to retry.
          </button>
        )}
        {/* v4 score overlay — top right */}
        {confidence !== null && (
          <span className="v4-lib-score">{Math.round(confidence * 100)}%</span>
        )}
        {clip.render_status === 'completed' && clip.output_path && (
          <div className="absolute bottom-1 left-1 z-10 bg-green-600/80 backdrop-blur-sm text-white text-[10px] px-1.5 py-0.5 rounded font-medium">
            ✓ Exported
          </div>
        )}
        {showReviewTools && highlight?.review_rating && (
          <span
            className={`absolute top-1 left-1 z-10 px-1.5 py-0.5 rounded text-[10px] uppercase tracking-wide border ${REVIEW_RATING_COLORS[highlight.review_rating as ClipReviewRating]}`}
            title={highlight.review_note ?? undefined}
          >
            {highlight.review_rating}
          </span>
        )}
      </div>

      <div className="v4-lib-body flex flex-col gap-2">
        <div className="flex items-start justify-between gap-2">
          <h3 className="v4-lib-title flex-1" style={{whiteSpace:'normal',display:'-webkit-box',WebkitLineClamp:2,WebkitBoxOrient:'vertical'}}>
            {displayTitle}
          </h3>
          {!selectMode && (
            <div className="flex gap-1 shrink-0">
              {showReviewTools && (
                <Tooltip text={reviewExpanded ? 'Close clip feedback' : 'Rate clip and flag edit issues'}>
                  <button
                    type="button"
                    aria-label={reviewExpanded ? 'Close clip feedback' : 'Rate clip and flag edit issues'}
                    aria-expanded={reviewExpanded}
                    aria-controls={`clip-feedback-${clip.id}`}
                    onClick={(e) => { e.stopPropagation(); onToggleReview() }}
                    className={`relative p-1.5 rounded-lg transition-colors cursor-pointer ${
                      reviewExpanded
                        ? 'bg-violet-500/15 text-violet-300'
                        : hasReviewFeedback
                          ? 'text-emerald-400 hover:bg-emerald-500/10'
                          : 'text-slate-500 hover:text-violet-400 hover:bg-violet-500/10'
                    }`}
                  >
                    <ListChecks className="w-4 h-4" />
                    {hasReviewFeedback && !reviewExpanded && (
                      <span className="absolute right-1 top-1 h-1.5 w-1.5 rounded-full bg-emerald-400" />
                    )}
                  </button>
                </Tooltip>
              )}
              <Tooltip text="Edit clip in the Editor">
                <button
                  onClick={(e) => { e.stopPropagation(); onEdit() }}
                  className="p-1.5 rounded-lg text-slate-500 hover:text-violet-400 hover:bg-violet-500/10 transition-colors cursor-pointer"
                >
                  <Pencil className="w-4 h-4" />
                </button>
              </Tooltip>
              <Tooltip text="Delete this clip">
                <button
                  onClick={(e) => { e.stopPropagation(); onDelete() }}
                  className="p-1.5 rounded-lg text-slate-500 hover:text-red-400 hover:bg-red-500/10 transition-colors cursor-pointer"
                >
                  <Trash2 className="w-4 h-4" />
                </button>
              </Tooltip>
            </div>
          )}
        </div>
        <div className="v4-lib-meta !justify-start flex-wrap gap-1.5">
          <TwitchProvenanceBadges
            tags={highlight?.tags}
            signalSources={highlight?.signal_sources}
            compact
          />
          {isViral && <span className="v4-viral-badge">🔥 VIRAL PICK</span>}
          {scheduledPlatforms && scheduledPlatforms.length > 0 && (
            <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium bg-amber-500/20 text-amber-300 border border-amber-500/30">
              <Clock className="w-2.5 h-2.5" />
              Scheduled
            </span>
          )}
        </div>
        {showReviewTools && reviewExpanded && (
          <div
            id={`clip-feedback-${clip.id}`}
            className="mt-1 pt-2 border-t border-surface-700/50 flex flex-col gap-2"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between gap-2">
              <div className="text-[11px] font-semibold text-slate-300">Clip feedback</div>
              <div className="flex items-center gap-1">
                {savingReview && (
                  <span className="inline-flex items-center gap-1 text-[10px] text-slate-500">
                    <Loader2 className="h-3 w-3 animate-spin" /> Saving
                  </span>
                )}
                <button
                  type="button"
                  aria-label="Close clip feedback"
                  onClick={(event) => { event.stopPropagation(); onToggleReview() }}
                  className="p-1 rounded text-slate-500 hover:bg-surface-700 hover:text-white transition-colors cursor-pointer"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </div>
            </div>
            <div className="text-[10px] font-medium uppercase text-slate-500">Moment quality</div>
            <div className="flex items-center gap-1.5">
              {(['good', 'meh', 'boring'] as ClipReviewRating[]).map((r) => {
                const isActive = highlight?.review_rating === r
                return (
                  <button
                    key={r}
                    type="button"
                    disabled={savingReview}
                    onClick={(e) => {
                      e.stopPropagation()
                      const nextRating: ClipReviewRating | null = isActive ? null : r
                      void persistReview(
                        nextRating,
                        reviewNoteDraft.trim() || null,
                        reviewIssues,
                      )
                    }}
                    className={`flex-1 px-2 py-1 text-[11px] rounded border transition-colors cursor-pointer disabled:opacity-50 ${
                      isActive
                        ? REVIEW_RATING_COLORS[r]
                        : 'bg-surface-800 text-slate-400 border-surface-600 hover:text-white hover:border-surface-500'
                    }`}
                  >
                    {REVIEW_RATING_LABELS[r]}
                  </button>
                )
              })}
            </div>
            <div>
              <div className="mb-1 text-[10px] font-medium uppercase text-slate-500">
                Edit issues <span className="normal-case font-normal">(choose all that apply)</span>
              </div>
              <div className="flex flex-wrap gap-1">
                {REVIEW_ISSUE_OPTIONS.map((option) => {
                  const isActive = reviewIssues.includes(option.id)
                  return (
                    <button
                      key={option.id}
                      type="button"
                      disabled={savingReview}
                      aria-pressed={isActive}
                      onClick={(event) => {
                        event.stopPropagation()
                        const nextIssues = isActive
                          ? reviewIssues.filter((issue) => issue !== option.id)
                          : [...reviewIssues, option.id]
                        void persistReview(
                          highlight?.review_rating ?? null,
                          reviewNoteDraft.trim() || null,
                          nextIssues,
                        )
                      }}
                      className={`px-2 py-1 text-[10px] rounded border transition-colors cursor-pointer disabled:opacity-50 ${
                        isActive
                          ? 'bg-amber-500/15 text-amber-200 border-amber-500/40'
                          : 'bg-surface-800 text-slate-400 border-surface-600 hover:text-white hover:border-surface-500'
                      }`}
                    >
                      {option.label}
                    </button>
                  )
                })}
              </div>
            </div>
            <textarea
              value={reviewNoteDraft}
              maxLength={2000}
              onChange={(event) => setReviewNoteDraft(event.target.value)}
              onBlur={(e) => {
                if (
                  e.relatedTarget instanceof HTMLElement
                  && e.currentTarget.parentElement?.contains(e.relatedTarget)
                ) return
                const noteValue = e.target.value.trim() || null
                if ((highlight?.review_note ?? null) === noteValue) return  // no-op if unchanged
                void persistReview(
                  highlight?.review_rating ?? null,
                  noteValue,
                  reviewIssues,
                )
              }}
              placeholder="Optional review note (saves on blur)..."
              className="w-full text-xs px-2 py-1 rounded bg-surface-800 border border-surface-600 text-slate-200 focus:border-violet-500 focus:outline-none resize-y min-h-[2rem]"
              rows={1}
            />
            {reviewError && (
              <p className="text-[10px] text-rose-300" role="alert">
                Could not save feedback: {reviewError}
              </p>
            )}
          </div>
        )}
      </div>
    </div>
  )
}

// ─── Confirm Delete Modal ─────────────────────────────────────────

function ConfirmDeleteModal({ count, onConfirm, onCancel }: {
  count: number; onConfirm: () => void; onCancel: () => void
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
      <div className="bg-surface-800 border border-surface-600 rounded-xl p-6 max-w-sm w-full mx-4 shadow-2xl">
        <h3 className="text-lg font-semibold text-white mb-2">Delete {count > 1 ? `${count} clips` : 'clip'}?</h3>
        <p className="text-sm text-slate-400 mb-5">
          {count > 1
            ? `Are you sure you want to delete these ${count} clips? You'll have a few seconds to undo.`
            : "Are you sure you want to delete this clip? You'll have a few seconds to undo."
          }
        </p>
        <div className="flex gap-3 justify-end">
          <button
            onClick={onCancel}
            className="px-4 py-2 rounded-lg text-sm font-medium text-slate-400 hover:text-white hover:bg-surface-700 transition-colors cursor-pointer"
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            className="px-4 py-2 rounded-lg text-sm font-medium bg-red-600 hover:bg-red-500 text-white transition-colors cursor-pointer"
          >
            Delete
          </button>
        </div>
      </div>
    </div>
  )
}

// ─── Undo Toast ───────────────────────────────────────────────────

function UndoToast({ count, onUndo, secondsLeft }: {
  count: number; onUndo: () => void; secondsLeft: number
}) {
  return (
    <div className="fixed bottom-6 left-1/2 -translate-x-1/2 z-50 bg-surface-700 border border-surface-600 rounded-xl px-4 py-3 shadow-2xl flex items-center gap-3 animate-slide-up">
      <span className="text-sm text-slate-300">
        {count > 1 ? `${count} clips` : '1 clip'} deleted
      </span>
      <button
        onClick={onUndo}
        className="flex items-center gap-1.5 px-3 py-1 rounded-lg text-sm font-medium bg-violet-600 hover:bg-violet-500 text-white transition-colors cursor-pointer"
      >
        <Undo2 className="w-3.5 h-3.5" />
        Undo ({secondsLeft}s)
      </button>
    </div>
  )
}

// ─── Main Clips Page ──────────────────────────────────────────────

export default function Clips() {
  const navigate = useNavigate()
  const location = useLocation()
  // VOD id passed from Vods.tsx when an analysis just finished. We scroll to
  // this VOD's section once the clips list is rendered so the user lands
  // directly on the freshly-generated clips (instead of mid-list, which was
  // the bug — react-router default scroll behavior puts them at the top of
  // the previous page's scroll position which makes no sense for a fresh
  // analysis result that just appeared at a different point in the list).
  const routeState = location.state as {
    focusVodId?: string
    focusClipId?: string
    openReview?: boolean
  } | null
  const focusVodId = routeState?.focusVodId ?? null
  const focusClipId = routeState?.focusClipId ?? null
  const openFocusedReview = routeState?.openReview === true
  const { clips, highlights, fetchClips, fetchHighlights, refreshVods, loggedInUser } = useAppStore()
  const { uploads: scheduledUploads, load: loadSchedules } = useScheduleStore()
  const showReviewTools = useUiStore((state) => state.settings.showReviewTools)
  const [vodMap, setVodMap] = useState<Record<string, Vod>>({})
  const [personalizationStatus, setPersonalizationStatus] = useState<PersonalizationStatus | null>(null)
  const [importingMedia, setImportingMedia] = useState(false)
  const [importNotice, setImportNotice] = useState<{ ok: boolean; text: string } | null>(null)
  const loadPersonalizationStatus = useCallback(async () => {
    if (!showReviewTools) {
      setPersonalizationStatus(null)
      return
    }
    try {
      const status = await invoke<PersonalizationStatus>('get_personalization_status')
      setPersonalizationStatus(status)
    } catch (error) {
      console.error('Failed to load personalization status:', error)
    }
  }, [showReviewTools])
  const personalizationCopy = useMemo(
    () => personalizationStatus
      ? getPersonalizationStatusCopy(personalizationStatus)
      : null,
    [personalizationStatus],
  )

  // "Preparing clip previews..." banner state. When the user lands on /clips
  // right after an analysis completes (focusVodId set in location.state), we
  // show a brief banner inside that VOD's section explaining first-time
  // playback may take a few seconds. Without this, fresh-install users hit
  // a cold-cache window where the OS file metadata + webview asset:// handler
  // haven't warmed up yet — clicks on play silently fail and look like the
  // app crashed. Initial state reads focusVodId once at mount; effect dismisses
  // after a fixed window so the banner doesn't linger past the cold-cache phase.
  const [preparingVodId, setPreparingVodId] = useState<string | null>(focusVodId)
  useEffect(() => {
    if (!preparingVodId) return
    const t = setTimeout(() => setPreparingVodId(null), 5000)
    return () => clearTimeout(t)
  }, [preparingVodId])

  // Multi-select state
  const [selectMode, setSelectMode] = useState(false)
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set())
  const [showBatchUpload, setShowBatchUpload] = useState(false)
  const [activeReviewClipId, setActiveReviewClipId] = useState<string | null>(null)
  const handledRouteFocusRef = useRef<string | null>(null)
  const autoCollapsedMedalRef = useRef(false)

  // Collapsible sections
  const [collapsedVods, setCollapsedVods] = useState<Set<string>>(loadCollapsed)

  // Sort & filter
  const [sortBy, setSortBy] = useState<SortBy>(() => loadSort().by)
  const [sortDir, setSortDir] = useState<SortDir>(() => loadSort().dir)
  const [hideDone, setHideDone] = useState<boolean>(loadHideDone)
  const [showSortBar, setShowSortBar] = useState(false)
  const [sourceTab, setSourceTab] = useState<ClipSourceTab>(focusVodId ? 'twitch' : 'all')

  // Delete confirmation
  const [confirmDelete, setConfirmDelete] = useState<{ clipIds: string[] } | null>(null)

  // Undo delete
  const [pendingDelete, setPendingDelete] = useState<PendingDelete | null>(null)
  const [undoSeconds, setUndoSeconds] = useState(5)
  const undoIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null)

  // Temporarily hidden clip IDs (pending undo)
  const [hiddenIds, setHiddenIds] = useState<Set<string>>(new Set())

  // Floating button visibility
  const [showScrollTop, setShowScrollTop] = useState(false)
  const containerRef = useRef<HTMLDivElement>(null)

  // Track which VOD the user last interacted with (for "scroll to active" button)
  const [activeVodTitle, setActiveVodTitle] = useState<string | null>(null)
  const vodSectionRefs = useRef<Record<string, HTMLDivElement | null>>({})

  // ── Load data ──
  useEffect(() => {
    fetchClips()
    fetchHighlights()
    loadSchedules()
  }, [fetchClips, fetchHighlights, loadSchedules])

  useEffect(() => {
    if (autoCollapsedMedalRef.current) return
    const medalGroupIds = [...new Set(clips
      .filter(clip => clip.source_kind === 'medal')
      .map(clip => clip.game?.trim()
        ? `external:medal:${clip.game.trim().toLocaleLowerCase()}`
        : clip.vod_id))]
    if (medalGroupIds.length <= 1) return
    autoCollapsedMedalRef.current = true
    setCollapsedVods(previous => {
      const next = new Set(previous)
      medalGroupIds.forEach(id => next.add(id))
      saveCollapsed(next)
      return next
    })
  }, [clips])

  const importLocalMedia = async () => {
    setImportingMedia(true)
    setImportNotice(null)
    try {
      const results = await invoke<Array<{ status: string }>>('pick_and_import_media')
      if (results.length === 0) return
      await Promise.all([fetchClips(), fetchHighlights()])
      const imported = results.filter(result => result.status === 'imported').length
      const duplicates = results.length - imported
      setImportNotice({
        ok: true,
        text: `${imported} video${imported === 1 ? '' : 's'} imported${duplicates ? `, ${duplicates} already in your library` : ''}.`,
      })
    } catch (error) {
      setImportNotice({ ok: false, text: `Import failed: ${String(error)}` })
    } finally {
      setImportingMedia(false)
    }
  }

  useEffect(() => {
    void loadPersonalizationStatus()
  }, [loadPersonalizationStatus])

  useEffect(() => {
    if (!showReviewTools) setActiveReviewClipId(null)
  }, [showReviewTools])

  useEffect(() => {
    if (!activeReviewClipId) return
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setActiveReviewClipId(null)
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [activeReviewClipId])

  useEffect(() => {
    if (activeReviewClipId && !clips.some((clip) => clip.id === activeReviewClipId)) {
      setActiveReviewClipId(null)
    }
  }, [activeReviewClipId, clips])

  // ── Auto-enter select mode when navigated with ?action=schedule or ?action=export ──
  useEffect(() => {
    const params = new URLSearchParams(window.location.search)
    const action = params.get('action')
    if (action === 'schedule' || action === 'export') {
      setActiveReviewClipId(null)
      setSelectMode(true)
    }
  }, [])

  // ── Scroll-to-last-edited-clip on return from Editor ──
  // Runs whenever clip count changes while a pending clip ID is set, so it
  // fires as soon as the list has actually rendered the card we care about.
  useEffect(() => {
    const ssVal = (() => { try { return sessionStorage.getItem('scrollToClip') } catch { return null } })()
    const clipId = lastEditedClipIdRef.current ?? ssVal
    if (!clipId) return
    // The target card may not be in the DOM on the first frame back from the
    // editor (clips can re-render/re-fetch), so retry across frames until it
    // exists, then re-assert the scroll a few times to survive late layout
    // shifts (images/clips finishing their layout).
    let cancelled = false
    let raf = 0
    let attempts = 0
    const run = () => {
      if (cancelled) return
      const el = document.querySelector<HTMLElement>(`[data-clip-id="${clipId}"]`)
      if (!el) {
        if (attempts++ < 60) raf = requestAnimationFrame(run)
        return
      }
      scrollCardIntoView(el, 'center')
      ;[60, 160, 320, 500].forEach((ms) => setTimeout(() => {
        if (cancelled) return
        const e2 = document.querySelector<HTMLElement>(`[data-clip-id="${clipId}"]`)
        if (e2) scrollCardIntoView(e2, 'center')
      }, ms))
      // Brief highlight flash so the user can spot where they were
      el.classList.add('ring-2', 'ring-violet-500/60')
      setTimeout(() => el.classList.remove('ring-2', 'ring-violet-500/60'), 1100)
      lastEditedClipIdRef.current = null
      try { sessionStorage.removeItem('scrollToClip') } catch { /* ignore */ }
    }
    raf = requestAnimationFrame(run)
    return () => { cancelled = true; cancelAnimationFrame(raf) }
  }, [clips.length])

  // ── Scroll-to-VOD on arrival from a just-completed analysis ──
  // When Vods.tsx navigates here after an analysis finishes, it stuffs the
  // VOD's id into location.state.focusVodId. The scroll fires once the
  // target VOD's title has loaded into vodMap (so the section renders with
  // a real title, not the raw VOD-ID fallback) AND the target section
  // exists in the DOM. We use explicit scroll-container detection rather
  // than relying on `scrollIntoView`'s magic ancestor walk because the
  // app uses a non-window scrollable container (the main content pane is
  // `overflow-auto`, which scrollIntoView handles correctly but makes
  // `window.scrollY` confusing for diagnosis).
  useEffect(() => {
    if (!focusVodId) return
    if (clips.length === 0) return

    // Wait for vodMap to include the focused VOD so the section renders
    // with its real title instead of the raw VOD-ID fallback. Without this
    // gate, `groupedClips` produces a group whose title is the UUID, which
    // (a) sorts to a weird position via the stream_date='1970-01-01'
    // fallback, and (b) makes the section visually unrecognizable.
    if (!vodMap[focusVodId]) {
      // vodMap will populate via the existing fetch effect; this useEffect
      // re-runs when vodMap changes (it's in the dep array), so we'll get
      // another chance soon.
      return
    }

    const raf = requestAnimationFrame(() => {
      const el = document.querySelector<HTMLElement>(`[data-vod-id="${focusVodId}"]`)
      if (!el) return

      // Find the actual scrollable ancestor. The app's main content pane
      // is `overflow-y-auto`, so window-level scrollTo is a no-op and
      // scrollIntoView's "nearest scrollable ancestor" is what actually
      // moves. We replicate that detection explicitly so we can call
      // scrollTo on the right container with the right offset.
      scrollCardIntoView(el, 'start')

      // Brief highlight pulse so the user can confirm visually where they
      // landed. Matches the editor-return flow's pulse.
      el.classList.add('ring-2', 'ring-violet-500/60')
      setTimeout(() => el.classList.remove('ring-2', 'ring-violet-500/60'), 1200)

      // Clear the navigation state so back/forward navigation doesn't
      // re-trigger the scroll. Using window.history.replaceState directly
      // avoids triggering a react-router re-render (and another effect run).
      window.history.replaceState(null, '', location.pathname)
    })
    return () => cancelAnimationFrame(raf)
    // location.state intentionally excluded (focusVodId is derived from it).
  }, [clips.length, focusVodId, vodMap, location.pathname])

  // ── Track scroll for floating button ──
  useEffect(() => {
    const onScroll = () => setShowScrollTop(window.scrollY > 300)
    window.addEventListener('scroll', onScroll, { passive: true })
    return () => window.removeEventListener('scroll', onScroll)
  }, [])

  // ── Fetch VOD details ──
  useEffect(() => {
    const vodIds = [...new Set(clips
      .filter(c => !c.source_media_path && !c.vod_id.startsWith('external:'))
      .map(c => c.vod_id))]
    setVodMap(prev => {
      const missing = vodIds.filter(id => !prev[id])
      if (missing.length === 0) return prev
      Promise.all(missing.map(id =>
        invoke<Vod>('get_vod_detail', { vodId: id }).catch(() => null)
      )).then(vods => {
        setVodMap(current => {
          const updated = { ...current }
          vods.forEach((vod, i) => { if (vod) updated[missing[i]] = vod })
          return updated
        })
      })
      return prev
    })
  }, [clips])

  // ── Persist sort/filter prefs ──
  useEffect(() => { saveSort({ by: sortBy, dir: sortDir }) }, [sortBy, sortDir])
  useEffect(() => { saveHideDone(hideDone) }, [hideDone])

  // ── Undo countdown timer ──
  useEffect(() => {
    if (!pendingDelete) {
      if (undoIntervalRef.current) clearInterval(undoIntervalRef.current)
      return
    }
    setUndoSeconds(10)
    undoIntervalRef.current = setInterval(() => {
      setUndoSeconds(prev => {
        if (prev <= 1) {
          if (undoIntervalRef.current) clearInterval(undoIntervalRef.current)
          return 0
        }
        return prev - 1
      })
    }, 1000)
    return () => { if (undoIntervalRef.current) clearInterval(undoIntervalRef.current) }
  }, [pendingDelete])

  // ── Delete logic ──

  const requestDelete = (clipIds: string[]) => {
    if (getConfirmDeletePref()) {
      setConfirmDelete({ clipIds })
    } else {
      executeDelete(clipIds)
    }
  }

  const executeDelete = (clipIds: string[]) => {
    setActiveReviewClipId((current) => (
      current && clipIds.includes(current) ? null : current
    ))
    // Cancel any existing pending delete first (commit it immediately)
    if (pendingDelete) {
      clearTimeout(pendingDelete.timer)
      commitDelete(pendingDelete.clipIds)
    }

    // Hide clips from UI immediately
    setHiddenIds(prev => {
      const next = new Set(prev)
      clipIds.forEach(id => next.add(id))
      return next
    })

    // Start undo timer (10 seconds)
    const timer = setTimeout(() => {
      commitDelete(clipIds)
      setPendingDelete(null)
      setHiddenIds(prev => {
        const next = new Set(prev)
        clipIds.forEach(id => next.delete(id))
        return next
      })
    }, 10000)

    setPendingDelete({ clipIds, timer })

    // Exit select mode if batch deleting
    if (selectMode) {
      setSelectMode(false)
      setSelectedIds(new Set())
    }
  }

  const commitDelete = async (clipIds: string[]) => {
    for (const clipId of clipIds) {
      try {
        await invoke('delete_clip', { clipId })
      } catch (err) {
        console.error('Failed to delete clip:', err)
      }
    }
    fetchClips()
    fetchHighlights()
    if (loggedInUser) refreshVods(loggedInUser.id)
  }

  const undoDelete = () => {
    if (!pendingDelete) return
    clearTimeout(pendingDelete.timer)
    // Remove from hidden set — clips reappear
    setHiddenIds(prev => {
      const next = new Set(prev)
      pendingDelete.clipIds.forEach(id => next.delete(id))
      return next
    })
    setPendingDelete(null)
  }

  // ── Confidence lookup ──

  const getConfidence = (highlightId: string): number | null => {
    const h = highlights.find((hl) => hl.id === highlightId)
    if (!h) return null
    return h.confidence_score ?? legacyToConfidence(h.virality_score)
  }

  const getPosterSrc = (clip: Clip) => {
    if (clip.thumbnail_path) return convertFileSrc(clip.thumbnail_path)
    const vod = vodMap[clip.vod_id]
    if (vod?.thumbnail_url) return vod.thumbnail_url
    return null
  }

  // ── Select mode ──

  const toggleSelectMode = () => {
    setActiveReviewClipId(null)
    if (selectMode) {
      setSelectMode(false)
      setSelectedIds(new Set())
    } else {
      setSelectMode(true)
    }
  }

  const toggleSelect = (clipId: string) => {
    setSelectedIds(prev => {
      const next = new Set(prev)
      if (next.has(clipId)) next.delete(clipId)
      else next.add(clipId)
      return next
    })
  }

  // Filter out hidden (pending-delete) clips, then apply the source workspace.
  const sourceCounts = useMemo(() => countClipsBySource(clips), [clips])
  const visibleClips = clips.filter(c => (
    !hiddenIds.has(c.id) && clipMatchesSourceTab(c, sourceTab)
  ))

  const selectSourceTab = (tab: ClipSourceTab) => {
    setSourceTab(tab)
    setSelectedIds(new Set())
    setActiveReviewClipId(null)
  }

  const selectAll = () => setSelectedIds(new Set(visibleClips.map(c => c.id)))
  const deselectAll = () => setSelectedIds(new Set())
  const selectedClips = visibleClips.filter(c => selectedIds.has(c.id))

  // ── Collapse toggle ──

  const toggleCollapse = (vodTitle: string) => {
    setCollapsedVods(prev => {
      const next = new Set(prev)
      if (next.has(vodTitle)) next.delete(vodTitle)
      else next.add(vodTitle)
      saveCollapsed(next)
      return next
    })
  }

  const collapseAll = () => {
    const all = new Set(groupedClips.map(g => g.id))
    setCollapsedVods(all)
    saveCollapsed(all)
  }

  const expandAll = () => {
    setCollapsedVods(new Set())
    saveCollapsed(new Set())
  }

  // ── Scheduled uploads lookup ──

  const scheduledByClip = useMemo(() => {
    const map: Record<string, Array<{ platform: string; scheduled_time: string }>> = {}
    for (const u of scheduledUploads) {
      if (u.status === 'pending' || u.status === 'uploading') {
        if (!map[u.clip_id]) map[u.clip_id] = []
        map[u.clip_id].push({ platform: u.platform, scheduled_time: u.scheduled_time })
      }
    }
    return map
  }, [scheduledUploads])

  // ── Group clips by VOD ──

  const groupedClips = useMemo(() => {
    const sourceTitle = (clip: Clip) => {
      if (clip.source_kind === 'medal') return clip.game?.trim() || 'Other Medal clips'
      if (clip.source_kind === 'obs') return 'OBS replays'
      if (clip.source_kind === 'meld') return 'Meld clips'
      if (clip.source_kind === 'manual') return 'Local video imports'
      return clip.vod_id
    }
    const groupKey = (clip: Clip) => (
      clip.source_kind === 'medal' && clip.game?.trim()
        ? `external:medal:${clip.game.trim().toLocaleLowerCase()}`
        : clip.vod_id
    )
    const groupsById = new Map<string, { id: string; title: string; vod: Vod | null; clips: Clip[] }>()
    for (const clip of visibleClips) {
      const vod = vodMap[clip.vod_id] || null
      const title = vod?.title || sourceTitle(clip)
      const id = groupKey(clip)
      if (!groupsById.has(id)) {
        groupsById.set(id, { id, title, vod, clips: [] })
      }
      groupsById.get(id)!.clips.push(clip)
    }

    const groups: { id: string; title: string; vod: Vod | null; clips: Clip[] }[] = []
    for (const group of groupsById.values()) {
      group.clips.sort((a, b) => {
        if (a.source_media_path || b.source_media_path) {
          return (b.source_recorded_at || '')
            .localeCompare(a.source_recorded_at || '')
        }
        return a.start_seconds - b.start_seconds
      })
      groups.push(group)
    }

    // Sort groups
    groups.sort((a, b) => {
      if (sortBy === 'stream_date') {
        const aVal = a.vod?.stream_date || a.clips[0]?.source_recorded_at || '1970-01-01'
        const bVal = b.vod?.stream_date || b.clips[0]?.source_recorded_at || '1970-01-01'
        const cmp = aVal.localeCompare(bVal)
        return sortDir === 'desc' ? -cmp : cmp
      } else {
        // Download order: use position of first clip in the master clips array
        const aIdx = visibleClips.findIndex(c => groupKey(c) === a.id)
        const bIdx = visibleClips.findIndex(c => groupKey(c) === b.id)
        const cmp = aIdx - bIdx
        return sortDir === 'desc' ? cmp : -cmp
      }
    })

    // Filter out fully completed VOD/source groups if hideDone is true.
    if (hideDone) {
      return groups.filter(g => {
        const allExported = g.clips.every(c => c.render_status === 'completed' && c.output_path)
        return !allExported
      })
    }

    return groups
  }, [visibleClips, vodMap, sortBy, sortDir, hideDone])

  // Dashboard inbox links land on one exact card and can open its review
  // disclosure. Reveal completed/collapsed groups before attempting to scroll.
  useEffect(() => {
    if (!focusClipId || handledRouteFocusRef.current === focusClipId) return
    const targetClip = clips.find(clip => clip.id === focusClipId)
    if (!targetClip) return

    if (!clipMatchesSourceTab(targetClip, sourceTab)) {
      setSourceTab(clipSourceTabFor(targetClip))
      return
    }

    const groupTitle = targetClip.source_kind === 'medal' && targetClip.game?.trim()
      ? `external:medal:${targetClip.game.trim().toLocaleLowerCase()}`
      : targetClip.vod_id

    if (hideDone) setHideDone(false)
    if (collapsedVods.has(groupTitle)) {
      setCollapsedVods(previous => {
        const next = new Set(previous)
        next.delete(groupTitle)
        saveCollapsed(next)
        return next
      })
    }
    if (openFocusedReview && showReviewTools) setActiveReviewClipId(focusClipId)

    let cancelled = false
    let raf = 0
    let attempts = 0
    const timers: ReturnType<typeof setTimeout>[] = []
    const run = () => {
      if (cancelled) return
      const element = document.querySelector<HTMLElement>(`[data-clip-id="${focusClipId}"]`)
      if (!element) {
        if (attempts++ < 60) raf = requestAnimationFrame(run)
        return
      }

      scrollCardIntoView(element, 'center')
      element.classList.add('ring-2', 'ring-violet-500/60')
      timers.push(setTimeout(() => element.classList.remove('ring-2', 'ring-violet-500/60'), 1200))
      handledRouteFocusRef.current = focusClipId
      window.history.replaceState(null, '', location.pathname)
    }

    raf = requestAnimationFrame(run)
    return () => {
      cancelled = true
      cancelAnimationFrame(raf)
      timers.forEach(clearTimeout)
    }
  }, [
    clips,
    collapsedVods,
    focusClipId,
    hideDone,
    location.pathname,
    openFocusedReview,
    showReviewTools,
    sourceTab,
    vodMap,
  ])

  // ── VOD status stats ──

  const vodStats = useCallback((groupClips: Clip[]) => {
    const total = groupClips.length
    const exported = groupClips.filter(c => c.render_status === 'completed' && c.output_path).length
    const published = groupClips.filter(c => scheduledByClip[c.id]?.length > 0).length
    return { total, exported, published }
  }, [scheduledByClip])

  // ── Navigation with scroll save ──

  const navigateToEditor = (clipId: string) => {
    lastEditedClipIdRef.current = clipId
    try { sessionStorage.setItem('scrollToClip', clipId) } catch { /* ignore */ }
    navigate(`/editor/${clipId}`)
  }

  // ── Floating button handlers ──

  const scrollToTop = () => window.scrollTo({ top: 0, behavior: 'smooth' })

  const scrollToActiveVod = () => {
    if (activeVodTitle && vodSectionRefs.current[activeVodTitle]) {
      vodSectionRefs.current[activeVodTitle]!.scrollIntoView({ behavior: 'smooth', block: 'start' })
    }
  }

  // ── Render ──

  return (
    <div className="space-y-4" ref={containerRef}>
      {/* ── Header ── */}
      <div className="v4-page-header">
        <div>
          <div className="v4-page-title">Clip Library ✂</div>
          <div className="v4-page-sub">
            {clips.length} total clips{sourceTab !== 'all' ? ` · ${sourceCounts[sourceTab]} in ${CLIP_SOURCE_TABS.find(tab => tab.id === sourceTab)?.label}` : ''}
          </div>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => void importLocalMedia()}
            disabled={importingMedia}
            className="v4-btn ghost"
            title="Import clips recorded by Medal, OBS, Meld, or another app"
          >
            {importingMedia ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <FileVideo className="w-3.5 h-3.5" />}
            Import videos
          </button>
          {visibleClips.length > 0 && (
            <>
            {selectMode && (
              <>
                <button
                  onClick={selectedIds.size === visibleClips.length ? deselectAll : selectAll}
                  className="px-3 py-1.5 rounded-lg text-xs font-medium text-slate-400 hover:text-white hover:bg-surface-700 transition-colors cursor-pointer"
                >
                  {selectedIds.size === visibleClips.length ? 'Deselect All' : 'Select All'}
                </button>
                {selectedIds.size > 0 && (
                  <>
                    <button
                      onClick={() => setShowBatchUpload(true)}
                      className="px-3 py-1.5 rounded-lg text-xs font-medium bg-violet-600 hover:bg-violet-500 text-white transition-colors cursor-pointer flex items-center gap-1.5"
                    >
                      <Upload className="w-3.5 h-3.5" />
                      Upload {selectedIds.size}
                    </button>
                    <button
                      onClick={() => requestDelete([...selectedIds])}
                      className="px-3 py-1.5 rounded-lg text-xs font-medium bg-red-600 hover:bg-red-500 text-white transition-colors cursor-pointer flex items-center gap-1.5"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                      Delete {selectedIds.size}
                    </button>
                  </>
                )}
              </>
            )}
            <button
              onClick={toggleSelectMode}
              className={`px-3 py-1.5 rounded-lg text-xs font-medium border transition-colors cursor-pointer flex items-center gap-1.5 ${
                selectMode
                  ? 'border-violet-500 bg-violet-500/20 text-violet-300'
                  : 'border-surface-600 bg-surface-700 text-slate-400 hover:text-slate-300'
              }`}
            >
              {selectMode ? <><X className="w-3.5 h-3.5" /> Exit Select</> : <><CheckSquare className="w-3.5 h-3.5" /> Select</>}
            </button>
            </>
          )}
          <button
            onClick={() => navigate('/vods')}
            className="v4-btn primary"
            style={{padding:'6px 12px', fontSize:12}}
            title="Pick a Twitch VOD to clip from"
          >
            + New clip
          </button>
        </div>
      </div>

      <div role="tablist" aria-label="Clip source" className="flex items-center gap-1 overflow-x-auto border-b border-surface-700">
        {CLIP_SOURCE_TABS.map(tab => (
          <button
            key={tab.id}
            type="button"
            role="tab"
            aria-selected={sourceTab === tab.id}
            onClick={() => selectSourceTab(tab.id)}
            className={`flex shrink-0 items-center gap-2 border-b-2 px-3 py-2 text-xs font-medium transition-colors ${
              sourceTab === tab.id
                ? 'border-violet-400 text-white'
                : 'border-transparent text-slate-500 hover:text-slate-300'
            }`}
          >
            {tab.label}
            <span className={`text-[10px] ${sourceTab === tab.id ? 'text-violet-300' : 'text-slate-600'}`}>
              {sourceCounts[tab.id]}
            </span>
          </button>
        ))}
      </div>

      {importNotice && (
        <div role="status" className={`border-l-2 px-3 py-2 text-xs ${importNotice.ok ? 'border-emerald-400 bg-emerald-500/5 text-emerald-300' : 'border-red-400 bg-red-500/5 text-red-300'}`}>
          {importNotice.text}
        </div>
      )}

      {showReviewTools && personalizationCopy && (
        <div className={`flex items-start gap-3 border-l-2 px-3 py-2 ${
          personalizationCopy.tone === 'active'
            ? 'border-emerald-400 bg-emerald-500/5'
            : personalizationCopy.tone === 'learning'
              ? 'border-violet-400 bg-violet-500/5'
              : personalizationCopy.tone === 'attention'
                ? 'border-amber-400 bg-amber-500/5'
                : 'border-slate-500 bg-surface-800/40'
        }`}>
          <Brain className="mt-0.5 h-4 w-4 shrink-0 text-violet-300" />
          <div className="min-w-0">
            <div className="text-xs font-semibold text-slate-200">
              {personalizationCopy.label}
            </div>
            <div className="mt-0.5 text-[11px] leading-4 text-slate-400">
              {personalizationCopy.detail}
            </div>
          </div>
        </div>
      )}

      {/* ── Sort / Filter bar ── */}
      {visibleClips.length > 0 && (
        <div className="flex items-center gap-2 flex-wrap">
          <button
            onClick={() => setShowSortBar(!showSortBar)}
            className={`px-3 py-1.5 rounded-lg text-xs font-medium border transition-colors cursor-pointer flex items-center gap-1.5 ${
              showSortBar
                ? 'border-violet-500/50 bg-violet-500/10 text-violet-300'
                : 'border-surface-600 bg-surface-700 text-slate-400 hover:text-slate-300'
            }`}
          >
            <SlidersHorizontal className="w-3.5 h-3.5" />
            Sort & Filter
          </button>

          {groupedClips.length > 1 && (
            <>
              <button
                onClick={collapseAll}
                className="px-2.5 py-1.5 rounded-lg text-xs text-slate-500 hover:text-slate-300 hover:bg-surface-700 transition-colors cursor-pointer"
              >
                Collapse All
              </button>
              <button
                onClick={expandAll}
                className="px-2.5 py-1.5 rounded-lg text-xs text-slate-500 hover:text-slate-300 hover:bg-surface-700 transition-colors cursor-pointer"
              >
                Expand All
              </button>
            </>
          )}

          {showSortBar && (
            <div className="flex items-center gap-2 flex-wrap w-full mt-1">
              {/* Sort by */}
              <div className="flex items-center gap-1.5 bg-surface-800 border border-surface-600 rounded-lg px-2.5 py-1.5">
                <ArrowUpDown className="w-3.5 h-3.5 text-slate-500" />
                <select
                  value={sortBy}
                  onChange={(e) => setSortBy(e.target.value as SortBy)}
                  className="bg-transparent text-xs text-slate-300 outline-none cursor-pointer"
                >
                  <option value="stream_date">Stream Date</option>
                  <option value="download_date">Download Date</option>
                </select>
                <button
                  onClick={() => setSortDir(d => d === 'desc' ? 'asc' : 'desc')}
                  className="text-xs text-slate-500 hover:text-slate-300 transition-colors cursor-pointer"
                >
                  {sortDir === 'desc' ? '↓ Newest' : '↑ Oldest'}
                </button>
              </div>

              {/* Hide completed */}
              <button
                onClick={() => setHideDone(!hideDone)}
                className={`flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-xs border transition-colors cursor-pointer ${
                  hideDone
                    ? 'border-amber-500/50 bg-amber-500/10 text-amber-300'
                    : 'border-surface-600 bg-surface-800 text-slate-400 hover:text-slate-300'
                }`}
              >
                {hideDone ? <EyeOff className="w-3.5 h-3.5" /> : <Eye className="w-3.5 h-3.5" />}
                {hideDone ? 'Showing incomplete only' : 'Hide completed groups'}
              </button>
            </div>
          )}
        </div>
      )}

      {/* ── Selection count ── */}
      {selectMode && selectedIds.size > 0 && (
        <div className="bg-violet-500/10 border border-violet-500/30 rounded-lg px-3 py-2 text-violet-300 text-sm">
          {selectedIds.size} clip{selectedIds.size !== 1 ? 's' : ''} selected
        </div>
      )}

      {/* ── Empty state ── */}
      {clips.length === 0 ? (
        <div className="v4-panel text-center p-12">
          <Scissors className="w-12 h-12 text-slate-600 mx-auto mb-4" />
          <h3 className="text-lg font-medium text-white mb-2">No clips yet</h3>
          <p className="text-slate-400 text-sm">
            Analyze VODs to find highlights, then review them here.
          </p>
        </div>
      ) : visibleClips.length === 0 && sourceTab !== 'all' ? (
        <div className="v4-panel p-10 text-center">
          <FileVideo className="mx-auto mb-3 h-10 w-10 text-slate-600" />
          <h3 className="mb-1 text-base font-medium text-white">
            No {CLIP_SOURCE_TABS.find(tab => tab.id === sourceTab)?.label} clips yet
          </h3>
          <p className="mb-4 text-sm text-slate-400">
            Imported and detected clips will appear in their matching source tab.
          </p>
          <button type="button" onClick={() => selectSourceTab('all')} className="v4-btn ghost">
            Show all clips
          </button>
        </div>
      ) : groupedClips.length === 0 && hideDone ? (
        <div className="v4-panel text-center p-12">
          <Eye className="w-12 h-12 text-slate-600 mx-auto mb-4" />
          <h3 className="text-lg font-medium text-white mb-2">All groups completed</h3>
          <p className="text-slate-400 text-sm mb-4">
            Every clip has been exported. Nice work!
          </p>
          <button
            onClick={() => setHideDone(false)}
            className="v4-btn ghost"
          >
            Show all VODs
          </button>
        </div>
      ) : (
        /* ── VOD groups ── */
        groupedClips.map(group => {
          const isCollapsed = collapsedVods.has(group.id)
          const stats = vodStats(group.clips)

          // VOD ID is consistent across all clips in a group (grouping key
          // is the VOD), so we can derive it from any clip. Used as a stable
          // attribute selector for the post-analysis scroll-to-section
          // effect — titles can change after re-analysis but VOD IDs don't.
          const groupVodId = group.clips[0]?.vod_id ?? ''

          // Show the "preparing previews" banner inside the just-analyzed
          // VOD's section. Banner self-dismisses after 5 seconds (see the
          // preparingVodId useEffect at top of component).
          const isPreparingThis = preparingVodId === groupVodId

          return (
            <div
              key={group.id}
              className="space-y-3"
              ref={(el) => { vodSectionRefs.current[group.id] = el }}
              data-vod-id={groupVodId}
            >
              {/* "Preparing previews..." banner — visible only on the freshly-
                  analyzed VOD's section for ~5s. Bridges the cold-cache window
                  where the source VOD file is technically downloaded + analyzed
                  but the webview asset:// handler / OS file metadata cache
                  hasn't warmed up yet, so first-click playback silently fails. */}
              {isPreparingThis && (
                <div className="bg-violet-500/10 border border-violet-500/30 rounded-lg px-4 py-3 mb-3 flex items-center gap-3">
                  <Loader2 className="w-4 h-4 animate-spin text-violet-300 shrink-0" />
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-semibold text-violet-200">Preparing clip previews...</div>
                    <div className="text-xs text-violet-300/70">
                      First-time playback may take a few seconds while the source video file finishes processing.
                    </div>
                  </div>
                </div>
              )}

              {/* Collapsible VOD header */}
              <button
                onClick={() => {
                  toggleCollapse(group.id)
                  setActiveVodTitle(group.id)
                }}
                className="w-full flex items-center gap-3 px-3 py-2.5 rounded-lg bg-surface-800/60 border border-surface-700 hover:border-surface-600 transition-colors cursor-pointer group"
              >
                {isCollapsed
                  ? <ChevronRight className="w-4 h-4 text-slate-500 group-hover:text-violet-400 transition-colors shrink-0" />
                  : <ChevronDown className="w-4 h-4 text-violet-400 shrink-0" />
                }
                <Film className="w-5 h-5 text-violet-400 shrink-0" />
                <div className="min-w-0 text-left flex-1">
                  <h2 className="text-sm font-semibold text-white truncate">
                    {group.title}
                  </h2>
                  {group.vod?.stream_date && (
                    <p className="text-xs text-slate-500">{formatDate(group.vod.stream_date)}</p>
                  )}
                  {!group.vod && group.clips[0]?.source_recorded_at && (
                    <p className="text-xs text-slate-500">Latest import {formatDate(group.clips[0].source_recorded_at!)}</p>
                  )}
                </div>
                {/* Status indicators */}
                <div className="flex items-center gap-2 shrink-0 ml-auto">
                  <span className="text-xs text-slate-500">
                    {stats.total} clip{stats.total !== 1 ? 's' : ''}
                  </span>
                  {stats.exported > 0 && (
                    <span className="text-[10px] px-1.5 py-0.5 rounded bg-green-500/15 text-green-400 border border-green-500/20">
                      {stats.exported} exported
                    </span>
                  )}
                  {stats.published > 0 && (
                    <span className="text-[10px] px-1.5 py-0.5 rounded bg-amber-500/15 text-amber-400 border border-amber-500/20">
                      {stats.published} scheduled
                    </span>
                  )}
                </div>
              </button>

              {/* Clip grid (collapsible) */}
              {!isCollapsed && (
                <div className="v4-clips-library pl-1">
                  {group.clips.map((clip) => {
                    const hl = highlights.find(h => h.id === clip.highlight_id)
                    return (
                      <ClipCard
                        key={clip.id}
                        clip={clip}
                        highlight={hl ? { id: hl.id, description: hl.description, tags: hl.tags, signal_sources: hl.signal_sources, transcript_snippet: hl.transcript_snippet, review_rating: hl.review_rating, review_note: hl.review_note, review_issues: hl.review_issues } : undefined}
                        confidence={getConfidence(clip.highlight_id)}
                        posterSrc={getPosterSrc(clip)}
                        onDelete={() => requestDelete([clip.id])}
                        onEdit={() => navigateToEditor(clip.id)}
                        onReviewSaved={loadPersonalizationStatus}
                        reviewExpanded={activeReviewClipId === clip.id}
                        onToggleReview={() => {
                          setActiveReviewClipId((current) => (
                            toggleExpandedReviewClip(current, clip.id)
                          ))
                        }}
                        selectMode={selectMode}
                        selected={selectedIds.has(clip.id)}
                        onToggleSelect={() => toggleSelect(clip.id)}
                        scheduledPlatforms={scheduledByClip[clip.id]}
                      />
                    )
                  })}
                </div>
              )}
            </div>
          )
        })
      )}

      {/* ── Floating buttons ── */}
      <div className="fixed bottom-6 right-6 z-40 flex flex-col gap-2">
        {activeVodTitle && vodSectionRefs.current[activeVodTitle] && (
          <Tooltip text="Scroll to active VOD">
            <button
              onClick={scrollToActiveVod}
              className="p-3 rounded-full bg-surface-700 border border-surface-600 text-violet-400 hover:bg-surface-600 shadow-lg transition-colors cursor-pointer"
            >
              <LocateFixed className="w-5 h-5" />
            </button>
          </Tooltip>
        )}
        {showScrollTop && (
          <Tooltip text="Scroll to top">
            <button
              onClick={scrollToTop}
              className="p-3 rounded-full bg-surface-700 border border-surface-600 text-slate-400 hover:text-white hover:bg-surface-600 shadow-lg transition-colors cursor-pointer"
            >
              <ArrowUp className="w-5 h-5" />
            </button>
          </Tooltip>
        )}
      </div>

      {/* ── Confirm delete modal ── */}
      {confirmDelete && (
        <ConfirmDeleteModal
          count={confirmDelete.clipIds.length}
          onConfirm={() => {
            executeDelete(confirmDelete.clipIds)
            setConfirmDelete(null)
          }}
          onCancel={() => setConfirmDelete(null)}
        />
      )}

      {/* ── Undo toast ── */}
      {pendingDelete && (
        <UndoToast
          count={pendingDelete.clipIds.length}
          onUndo={undoDelete}
          secondsLeft={undoSeconds}
        />
      )}

      {/* ── Batch Upload Dialog ── */}
      {showBatchUpload && (
        <BatchUploadDialog
          clips={selectedClips}
          onClose={() => setShowBatchUpload(false)}
          onComplete={() => {
            setSelectMode(false)
            setSelectedIds(new Set())
          }}
        />
      )}
    </div>
  )
}
