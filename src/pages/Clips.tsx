import { useEffect, useState, useCallback, useMemo, useRef } from 'react'
import { useNavigate, useLocation } from 'react-router-dom'
import {
  Scissors, Trash2, Pencil, Film, CheckSquare, Square, Upload, X, Clock,
  ChevronDown, ChevronRight, ArrowUp, LocateFixed, SlidersHorizontal,
  ArrowUpDown, Eye, EyeOff, Undo2,
} from 'lucide-react'
import { invoke, convertFileSrc } from '@tauri-apps/api/core'
import { useAppStore } from '../stores/appStore'
import ClipPlayer from '../components/ClipPlayer'
import Tooltip from '../components/Tooltip'
import BatchUploadDialog from '../components/BatchUploadDialog'
import { useScheduleStore } from '../stores/scheduleStore'
import type { Clip, Vod } from '../types'

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

// ─── Undo toast state ─────────────────────────────────────────────

interface PendingDelete {
  clipIds: string[]
  timer: ReturnType<typeof setTimeout>
}

// ─── ClipCard (unchanged logic, extracted for clarity) ────────────

function ClipCard({ clip, highlight, confidence, posterSrc, onDelete, onEdit, selectMode, selected, onToggleSelect, scheduledPlatforms }: {
  clip: Clip
  highlight?: { description?: string; tags?: string | string[]; transcript_snippet?: string }
  confidence: number | null
  posterSrc: string | null
  onDelete: () => void
  onEdit: () => void
  selectMode: boolean
  selected: boolean
  onToggleSelect: () => void
  scheduledPlatforms?: Array<{ platform: string; scheduled_time: string }>
}) {
  const [videoSrc, setVideoSrc] = useState<string | null>(null)

  const displayTitle = useMemo(
    () => clipDisplayTitle(clip, highlight),
    [clip.title, highlight]
  )

  const ensureSrc = useCallback(async () => {
    if (videoSrc) return
    try {
      const v = await invoke<Vod>('get_vod_detail', { vodId: clip.vod_id })
      if (v.local_path) setVideoSrc(convertFileSrc(v.local_path))
    } catch {
      console.warn(`[Clips] Failed to load VOD source for clip ${clip.id}`)
    }
  }, [videoSrc, clip.vod_id])

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
      } ${selectMode ? 'cursor-pointer' : ''}`}
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
          mode="compact"
          className="w-full h-full"
          objectFit="contain"
        />
        {/* v4 score overlay — top right */}
        {confidence !== null && (
          <span className="v4-lib-score">{Math.round(confidence * 100)}%</span>
        )}
        {clip.render_status === 'completed' && clip.output_path && (
          <div className="absolute bottom-1 left-1 z-10 bg-green-600/80 backdrop-blur-sm text-white text-[10px] px-1.5 py-0.5 rounded font-medium">
            ✓ Exported
          </div>
        )}
      </div>

      <div className="v4-lib-body flex flex-col gap-2">
        <div className="flex items-start justify-between gap-2">
          <h3 className="v4-lib-title flex-1" style={{whiteSpace:'normal',display:'-webkit-box',WebkitLineClamp:2,WebkitBoxOrient:'vertical' as any}}>
            {displayTitle}
          </h3>
          {!selectMode && (
            <div className="flex gap-1 shrink-0">
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
        <div className="v4-lib-meta flex-wrap">
          {isViral && <span className="v4-viral-badge">🔥 VIRAL PICK</span>}
          {scheduledPlatforms && scheduledPlatforms.length > 0 && (
            <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium bg-amber-500/20 text-amber-300 border border-amber-500/30">
              <Clock className="w-2.5 h-2.5" />
              Scheduled
            </span>
          )}
        </div>
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
  const focusVodId = (location.state as { focusVodId?: string } | null)?.focusVodId ?? null
  const { clips, highlights, fetchClips, fetchHighlights, refreshVods, loggedInUser } = useAppStore()
  const { uploads: scheduledUploads, load: loadSchedules } = useScheduleStore()
  const [vodMap, setVodMap] = useState<Record<string, Vod>>({})

  // Multi-select state
  const [selectMode, setSelectMode] = useState(false)
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set())
  const [showBatchUpload, setShowBatchUpload] = useState(false)

  // Collapsible sections
  const [collapsedVods, setCollapsedVods] = useState<Set<string>>(loadCollapsed)

  // Sort & filter
  const [sortBy, setSortBy] = useState<SortBy>(() => loadSort().by)
  const [sortDir, setSortDir] = useState<SortDir>(() => loadSort().dir)
  const [hideDone, setHideDone] = useState<boolean>(loadHideDone)
  const [showSortBar, setShowSortBar] = useState(false)

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

  // ── Auto-enter select mode when navigated with ?action=schedule or ?action=export ──
  useEffect(() => {
    const params = new URLSearchParams(window.location.search)
    const action = params.get('action')
    if (action === 'schedule' || action === 'export') {
      setSelectMode(true)
    }
  }, [])

  // ── Scroll-to-last-edited-clip on return from Editor ──
  // Runs whenever clip count changes while a pending clip ID is set, so it
  // fires as soon as the list has actually rendered the card we care about.
  useEffect(() => {
    const clipId = lastEditedClipIdRef.current
    if (!clipId) return
    if (clips.length === 0) return
    const raf = requestAnimationFrame(() => {
      const el = document.querySelector<HTMLElement>(`[data-clip-id="${clipId}"]`)
      if (el) {
        el.scrollIntoView({ block: 'center', behavior: 'instant' as ScrollBehavior })
        // Brief highlight flash so the user can spot where they were
        el.classList.add('ring-2', 'ring-violet-500/60')
        setTimeout(() => el.classList.remove('ring-2', 'ring-violet-500/60'), 900)
        lastEditedClipIdRef.current = null
      }
    })
    return () => cancelAnimationFrame(raf)
  }, [clips.length])

  // ── Scroll-to-VOD on arrival from a just-completed analysis ──
  // When Vods.tsx navigates here after an analysis finishes, it stuffs the
  // VOD's id into location.state.focusVodId. We watch clips.length so the
  // scroll fires once the list has rendered the section we're looking for
  // (initial mount has 0 clips while fetchClips() is in flight). Once we
  // scroll successfully, we replace the navigation entry with empty state
  // so a back-button bounce or page refresh doesn't re-trigger the scroll.
  useEffect(() => {
    if (!focusVodId) return
    if (clips.length === 0) return
    const raf = requestAnimationFrame(() => {
      const el = document.querySelector<HTMLElement>(`[data-vod-id="${focusVodId}"]`)
      if (el) {
        el.scrollIntoView({ block: 'start', behavior: 'instant' as ScrollBehavior })
        // Same brief highlight pulse as the editor-return flow, so the user
        // can confirm visually that this is where their fresh clips landed.
        el.classList.add('ring-2', 'ring-violet-500/60')
        setTimeout(() => el.classList.remove('ring-2', 'ring-violet-500/60'), 1200)
        // Clear the navigation state so it doesn't re-fire on back/forward.
        navigate(location.pathname, { replace: true, state: null })
      }
    })
    return () => cancelAnimationFrame(raf)
  }, [clips.length, focusVodId, navigate, location.pathname])

  // ── Track scroll for floating button ──
  useEffect(() => {
    const onScroll = () => setShowScrollTop(window.scrollY > 300)
    window.addEventListener('scroll', onScroll, { passive: true })
    return () => window.removeEventListener('scroll', onScroll)
  }, [])

  // ── Fetch VOD details ──
  useEffect(() => {
    const vodIds = [...new Set(clips.map(c => c.vod_id))]
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

  // Filter out hidden (pending-delete) clips
  const visibleClips = clips.filter(c => !hiddenIds.has(c.id))

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
    const all = new Set(groupedClips.map(g => g.title))
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
    const titleMap = new Map<string, { vod: Vod | null; clips: Clip[] }>()
    for (const clip of visibleClips) {
      const vod = vodMap[clip.vod_id] || null
      const title = vod?.title || clip.vod_id
      if (!titleMap.has(title)) titleMap.set(title, { vod, clips: [] })
      titleMap.get(title)!.clips.push(clip)
    }

    const groups: { title: string; vod: Vod | null; clips: Clip[] }[] = []
    for (const [title, group] of titleMap) {
      group.clips.sort((a, b) => a.start_seconds - b.start_seconds)
      groups.push({ title, vod: group.vod, clips: group.clips })
    }

    // Sort groups
    groups.sort((a, b) => {
      if (sortBy === 'stream_date') {
        const aVal = a.vod?.stream_date || '1970-01-01'
        const bVal = b.vod?.stream_date || '1970-01-01'
        const cmp = aVal.localeCompare(bVal)
        return sortDir === 'desc' ? -cmp : cmp
      } else {
        // Download order: use position of first clip in the master clips array
        const aIdx = visibleClips.findIndex(c => c.vod_id === (a.vod?.id || ''))
        const bIdx = visibleClips.findIndex(c => c.vod_id === (b.vod?.id || ''))
        const cmp = aIdx - bIdx
        return sortDir === 'desc' ? cmp : -cmp
      }
    })

    // Filter out fully completed VODs if hideDone is true
    if (hideDone) {
      return groups.filter(g => {
        const allExported = g.clips.every(c => c.render_status === 'completed' && c.output_path)
        return !allExported
      })
    }

    return groups
  }, [visibleClips, vodMap, sortBy, sortDir, hideDone])

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
            {clips.length} total clips{visibleClips.length !== clips.length ? ` · ${visibleClips.length} shown` : ''}
          </div>
        </div>
        {visibleClips.length > 0 && (
          <div className="flex items-center gap-2">
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
            <button
              onClick={() => navigate('/vods')}
              className="v4-btn primary"
              style={{padding:'6px 12px', fontSize:12}}
              title="Pick a VOD to clip from"
            >
              + New clip
            </button>
          </div>
        )}
        {/* Show "+ New clip" even in empty state so it's discoverable */}
        {visibleClips.length === 0 && (
          <div className="flex items-center gap-2">
            <button
              onClick={() => navigate('/vods')}
              className="v4-btn primary"
              style={{padding:'6px 12px', fontSize:12}}
            >
              + New clip
            </button>
          </div>
        )}
      </div>

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
                {hideDone ? 'Showing incomplete only' : 'Hide completed VODs'}
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
      ) : groupedClips.length === 0 && hideDone ? (
        <div className="v4-panel text-center p-12">
          <Eye className="w-12 h-12 text-slate-600 mx-auto mb-4" />
          <h3 className="text-lg font-medium text-white mb-2">All VODs completed</h3>
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
          const isCollapsed = collapsedVods.has(group.title)
          const stats = vodStats(group.clips)

          // VOD ID is consistent across all clips in a group (grouping key
          // is the VOD), so we can derive it from any clip. Used as a stable
          // attribute selector for the post-analysis scroll-to-section
          // effect — titles can change after re-analysis but VOD IDs don't.
          const groupVodId = group.clips[0]?.vod_id ?? ''

          return (
            <div
              key={group.title}
              className="space-y-3"
              ref={(el) => { vodSectionRefs.current[group.title] = el }}
              data-vod-id={groupVodId}
            >
              {/* Collapsible VOD header */}
              <button
                onClick={() => {
                  toggleCollapse(group.title)
                  setActiveVodTitle(group.title)
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
                        highlight={hl ? { description: hl.description, tags: hl.tags, transcript_snippet: hl.transcript_snippet } : undefined}
                        confidence={getConfidence(clip.highlight_id)}
                        posterSrc={getPosterSrc(clip)}
                        onDelete={() => requestDelete([clip.id])}
                        onEdit={() => navigateToEditor(clip.id)}
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
