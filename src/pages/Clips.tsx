import { useEffect, useState, useCallback, useMemo } from 'react'
import { useNavigate } from 'react-router-dom'
import { Scissors, Trash2, Pencil, Film, CheckSquare, Square, Upload, X, Clock } from 'lucide-react'
import { invoke, convertFileSrc } from '@tauri-apps/api/core'
import { useAppStore } from '../stores/appStore'
import { formatConfidence } from '../lib/uiFormat'
import ClipPlayer from '../components/ClipPlayer'
import Tooltip from '../components/Tooltip'
import BatchUploadDialog from '../components/BatchUploadDialog'
import { useScheduleStore } from '../stores/scheduleStore'
import { PLATFORM_INFO } from '../stores/platformStore'
import type { Clip, Vod } from '../types'

function formatDate(dateStr: string) {
  try {
    return new Date(dateStr).toLocaleDateString(undefined, {
      month: 'short', day: 'numeric', year: 'numeric',
    })
  } catch {
    return dateStr
  }
}

// Compress legacy virality_score to calibrated confidence.
// De-inflates bonus stacking, then applies piecewise curve
// matching the backend's compute_confidence().
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

// Build a display title.  Prefers the user-saved clip title (which may
// have been edited in the Editor), falls back to transcript snippet.
function clipDisplayTitle(
  clip: Clip,
  highlight?: { description?: string; transcript_snippet?: string },
): string {
  // User-saved title always wins (includes manually edited titles)
  if (clip.title && clip.title.trim().length > 0) {
    return clip.title
  }
  // Fall back to transcript snippet if no title was saved
  const snippet = highlight?.transcript_snippet?.trim()
  if (snippet && snippet.length >= 5) {
    if (snippet.length <= 50) return `"${snippet}"`
    const words = snippet.split(/\s+/).slice(0, 6).join(' ')
    return `"${words}..."`
  }
  return 'Untitled Clip'
}

function ClipCard({ clip, highlight, vod: _vod, confidence, posterSrc, onDelete, onEdit, selectMode, selected, onToggleSelect, scheduledPlatforms }: {
  clip: Clip; highlight?: { description?: string; tags?: string | string[]; transcript_snippet?: string }; vod?: Vod; confidence: number | null; posterSrc: string | null; onDelete: () => void; onEdit: () => void; selectMode: boolean; selected: boolean; onToggleSelect: () => void; scheduledPlatforms?: Array<{ platform: string; scheduled_time: string }>
}) {
  const [videoSrc, setVideoSrc] = useState<string | null>(null)

  const displayTitle = useMemo(
    () => clipDisplayTitle(clip, highlight),
    [clip.title, highlight]
  )

  // Lazy-load video source on first interaction (click triggers load)
  // Always use the source VOD so previews show original 16:9 gameplay
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
    if (selectMode) {
      onToggleSelect()
    } else {
      ensureSrc()
    }
  }

  return (
    <div
      className={`relative bg-surface-800 border rounded-xl overflow-hidden flex flex-col transition-colors ${
        selectMode && selected
          ? 'border-violet-500 ring-1 ring-violet-500/30'
          : 'border-surface-700'
      } ${selectMode ? 'cursor-pointer' : ''}`}
      onClick={handleClick}
    >
      {/* Selection overlay */}
      {selectMode && (
        <div className="absolute top-2 left-2 z-10">
          {selected
            ? <CheckSquare className="w-5 h-5 text-violet-400" />
            : <Square className="w-5 h-5 text-slate-500" />
          }
        </div>
      )}

      {/* Player — fixed 16:9 container so all cards are uniform regardless of export aspect ratio */}
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
        {/* Exported badge */}
        {clip.render_status === 'completed' && clip.output_path && (
          <div className="absolute bottom-1 right-1 z-10 bg-green-600/80 text-white text-[10px] px-1.5 py-0.5 rounded font-medium">
            Exported
          </div>
        )}
      </div>

      {/* Info */}
      <div className="px-3 pb-3 pt-1 flex flex-col gap-2">
        <div className="flex items-start justify-between gap-2">
          <h3 className="text-sm font-medium text-white line-clamp-2 leading-snug flex-1">
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
        <div className="flex items-center gap-2 flex-wrap">
          {confidence !== null && (() => {
            const conf = formatConfidence(confidence)
            return (
              <span className={`text-xs px-2 py-0.5 rounded-full border border-surface-600 ${conf.color}`}>
                {conf.text} ({Math.round(confidence * 100)}%)
              </span>
            )
          })()}
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

export default function Clips() {
  const navigate = useNavigate()
  const { clips, highlights, fetchClips, fetchHighlights, refreshVods, loggedInUser } = useAppStore()
  const { uploads: scheduledUploads, load: loadSchedules } = useScheduleStore()
  const [vodMap, setVodMap] = useState<Record<string, Vod>>({})

  // Multi-select state
  const [selectMode, setSelectMode] = useState(false)
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set())
  const [showBatchUpload, setShowBatchUpload] = useState(false)

  useEffect(() => {
    fetchClips()
    fetchHighlights()
    loadSchedules()
  }, [fetchClips, fetchHighlights, loadSchedules])

  // Fetch VOD details for all unique vod_ids (functional setState to avoid stale closure)
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
          vods.forEach((vod, i) => {
            if (vod) updated[missing[i]] = vod
          })
          return updated
        })
      })
      return prev
    })
  }, [clips])

  const handleDelete = async (clipId: string) => {
    try {
      await invoke('delete_clip', { clipId })
      fetchClips()
      fetchHighlights()
      // Refresh VOD store — backend resets analysis_status when last clip is deleted
      if (loggedInUser) refreshVods(loggedInUser.id)
    } catch (err) {
      console.error('Failed to delete clip:', err)
    }
  }

  const getConfidence = (highlightId: string): number | null => {
    const h = highlights.find((hl) => hl.id === highlightId)
    if (!h) return null
    // New rows have confidence_score set; old rows fall back to compressed virality_score
    return h.confidence_score ?? legacyToConfidence(h.virality_score)
  }

  const getPosterSrc = (clip: Clip) => {
    if (clip.thumbnail_path) return convertFileSrc(clip.thumbnail_path)
    const vod = vodMap[clip.vod_id]
    if (vod?.thumbnail_url) return vod.thumbnail_url
    return null
  }

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

  const selectAll = () => {
    setSelectedIds(new Set(clips.map(c => c.id)))
  }

  const deselectAll = () => {
    setSelectedIds(new Set())
  }

  const selectedClips = clips.filter(c => selectedIds.has(c.id))

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

  // Group clips by VOD title
  const groupedClips: { title: string; vod: Vod | null; clips: Clip[] }[] = []
  const titleMap = new Map<string, { vod: Vod | null; clips: Clip[] }>()
  for (const clip of clips) {
    const vod = vodMap[clip.vod_id] || null
    const title = vod?.title || clip.vod_id
    if (!titleMap.has(title)) {
      titleMap.set(title, { vod, clips: [] })
    }
    titleMap.get(title)!.clips.push(clip)
  }
  for (const [title, group] of titleMap) {
    group.clips.sort((a, b) => a.start_seconds - b.start_seconds)
    groupedClips.push({ title, vod: group.vod, clips: group.clips })
  }

  return (
    <div className="space-y-6">
      {/* Header with select/upload controls */}
      <div className="flex items-center justify-between gap-4">
        <h1 className="text-2xl font-bold text-white">My Clips</h1>
        {clips.length > 0 && (
          <div className="flex items-center gap-2">
            {selectMode && (
              <>
                <button
                  onClick={selectedIds.size === clips.length ? deselectAll : selectAll}
                  className="px-3 py-1.5 rounded-lg text-xs font-medium text-slate-400 hover:text-white hover:bg-surface-700 transition-colors cursor-pointer"
                >
                  {selectedIds.size === clips.length ? 'Deselect All' : 'Select All'}
                </button>
                {selectedIds.size > 0 && (
                  <button
                    onClick={() => setShowBatchUpload(true)}
                    className="px-3 py-1.5 rounded-lg text-xs font-medium bg-violet-600 hover:bg-violet-500 text-white transition-colors cursor-pointer flex items-center gap-1.5"
                  >
                    <Upload className="w-3.5 h-3.5" />
                    Upload {selectedIds.size} Clip{selectedIds.size !== 1 ? 's' : ''}
                  </button>
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
          </div>
        )}
      </div>

      {/* Selection count badge */}
      {selectMode && selectedIds.size > 0 && (
        <div className="bg-violet-500/10 border border-violet-500/30 rounded-lg px-3 py-2 text-violet-300 text-sm">
          {selectedIds.size} clip{selectedIds.size !== 1 ? 's' : ''} selected
        </div>
      )}

      {clips.length === 0 ? (
        <div className="bg-surface-800 border border-surface-700 rounded-xl p-12 text-center">
          <Scissors className="w-12 h-12 text-slate-600 mx-auto mb-4" />
          <h3 className="text-lg font-medium text-white mb-2">No clips yet</h3>
          <p className="text-slate-400 text-sm">
            Analyze VODs to find highlights, then review them here.
          </p>
        </div>
      ) : (
        groupedClips.map(group => (
          <div key={group.title} className="space-y-3">
            <div className="flex items-center gap-3 px-1">
              <Film className="w-5 h-5 text-violet-400 shrink-0" />
              <div className="min-w-0">
                <h2 className="text-base font-semibold text-white truncate">
                  {group.title}
                </h2>
                {group.vod?.stream_date && (
                  <p className="text-xs text-slate-500">{formatDate(group.vod.stream_date)}</p>
                )}
              </div>
              <span className="text-xs text-slate-500 shrink-0 ml-auto">
                {group.clips.length} clip{group.clips.length !== 1 ? 's' : ''}
              </span>
            </div>
            <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
              {group.clips.map((clip) => {
                const hl = highlights.find(h => h.id === clip.highlight_id)
                return (
                  <ClipCard
                    key={clip.id}
                    clip={clip}
                    highlight={hl ? { description: hl.description, tags: hl.tags, transcript_snippet: hl.transcript_snippet } : undefined}
                    vod={group.vod || undefined}
                    confidence={getConfidence(clip.highlight_id)}
                    posterSrc={getPosterSrc(clip)}
                    onDelete={() => handleDelete(clip.id)}
                    onEdit={() => navigate(`/editor/${clip.id}`)}
                    selectMode={selectMode}
                    selected={selectedIds.has(clip.id)}
                    onToggleSelect={() => toggleSelect(clip.id)}
                    scheduledPlatforms={scheduledByClip[clip.id]}
                  />
                )
              })}
            </div>
          </div>
        ))
      )}

      {/* Batch Upload Dialog */}
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
