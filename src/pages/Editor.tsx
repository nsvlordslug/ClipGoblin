import { useEffect, useRef, useState, useMemo, useCallback } from 'react'
import { useParams, useNavigate, useLocation } from 'react-router-dom'
import { ArrowLeft, Save, Download, Check, Loader2, MessageSquare, Upload, Film, Link2, Undo2, Redo2, RefreshCw, Bookmark, ChevronDown, ChevronLeft, ChevronRight, X, Plus, Clock, CalendarClock, ImagePlus } from 'lucide-react'
import { invoke, convertFileSrc } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
// Use our own Tauri command to open URLs in the system default browser
const openUrl = (url: string) => invoke('open_url', { url })
import type { Clip, Vod } from '../types'
import type { UploadResult } from '../stores/platformStore'
import { CAPTION_STYLES, EXPORT_PRESETS, LAYOUT_OPTIONS } from '../lib/editTypes'
import type { LayoutMode, TextOverlay } from '../lib/editTypes'
import type { Highlight } from '../types'
import ClipPlayer from '../components/ClipPlayer'
import TrimTimeline from '../components/TrimTimeline'
import CaptionPreview from '../components/CaptionPreview'
import type { TimelineMarker } from '../components/TrimTimeline'
import { analyzeEmphasis, getEmphasisSummary } from '../lib/captionEmphasis'
import type { CaptionToken } from '../lib/captionEmphasis'
import { clampCaptionFontScale, fitCaptionFontSize } from '../lib/captionSizing'
import ThumbnailSelector from '../components/ThumbnailSelector'
import LayoutPicker from '../components/LayoutPicker'
import CamRegionRow from '../components/CamRegionRow'
import CamRegionModal from '../components/CamRegionModal'
import CamRegionPreview from '../components/CamRegionPreview'
import type { RegionNorm } from '../components/CamRegionSetter'
import FacecamEditor, { DraggablePipOverlay, DraggableSplitDivider } from '../components/FacecamEditor'
import { DEFAULT_FACECAM, computeSubtitleCollision, parseFacecamSettings } from '../lib/facecam'
import type { FacecamSettings } from '../lib/facecam'
import SubtitleEditor from '../components/SubtitleEditor'
import { parseSrt, serializeSrt, findActiveSegment, shiftSubtitleSegments, splitSubtitleSegmentsByWord } from '../lib/subtitleUtils'
import type { SubtitleSegment } from '../lib/subtitleUtils'
import { usePlaybackStore } from '../stores/playbackStore'
import { usePlatformStore, PLATFORM_INFO } from '../stores/platformStore'
import { useMontageStore } from '../stores/montageStore'
import Tooltip from '../components/Tooltip'
import PublishComposer from '../components/PublishComposer'
import type { PublishMetadata } from '../components/PublishComposer'
import TikTokComplianceFields from '../components/TikTokComplianceFields'
import { EMPTY_TIKTOK_COMPLIANCE } from '../lib/tiktokCompliance'
import type { TikTokComplianceValue } from '../lib/tiktokCompliance'
import { useScheduleStore } from '../stores/scheduleStore'
import PlatformUploadSelector from '../components/PlatformUploadSelector'
import { expandYouTubeSubFormat, getDefaultVisibility, getDefaultYouTubeSubFormat, getPresetForPlatform, isSuccessfulUploadHandoff, isTikTokInboxDelivered, shouldOfferForcedReupload } from '../lib/platformUpload'
import type { PlatformUploadState, YouTubeSubFormat } from '../lib/platformUpload'
import { useEditorHistory } from '../hooks/useEditorHistory'
import type { EditorSnapshot } from '../hooks/useEditorHistory'
import ExportProgressBar from '../components/ExportProgressBar'
import { useTemplateStore } from '../stores/templateStore'
import type { ClipTemplate } from '../stores/templateStore'
import { generateStandaloneTitle } from '../lib/publishCopyGenerator'
import type { ClipContext } from '../lib/publishCopyGenerator'
import { errorMessage } from '../lib/errors'
import { localDateTimeAfter } from '../lib/dateTime'
import { parseStoredTags } from '../lib/tags'
import TwitchProvenanceBadges from '../components/TwitchProvenanceBadges'
import { getNextEditorWorkspace, isEditorWorkspaceId } from '../lib/editorWorkspace'
import type { EditorWorkspaceId, EditorWorkspaceNavigationKey } from '../lib/editorWorkspace'
import { canGenerateTimedCaptions, getCaptionTimelineStart, hasUsableSourceMedia } from '../lib/editorCaptions'
import {
  brandingAssetName,
  contextVideoPositionLabel,
  DEFAULT_CONTEXT_BLUR_STRENGTH,
  DEFAULT_CONTEXT_VIDEO_Y,
  normalizeContextBackgroundMode,
  normalizeContextBlurStrength,
  normalizeContextVideoY,
} from '../lib/contextFit'
import type { ContextBackgroundMode } from '../lib/contextFit'

function formatTime(seconds: number) {
  const m = Math.floor(seconds / 60)
  const s = Math.floor(seconds % 60)
  return `${m}:${String(s).padStart(2, '0')}`
}
// ── Reusable section component ──
function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="v4-panel" style={{padding: 16}}>
      <h3 className="v4-section-label" style={{marginBottom: 12}}>{title}</h3>
      {children}
    </div>
  )
}
// ── Pill selector ──
function PillGroup<T extends string>({ value, options, onChange }: {
  value: T
  options: { value: T; label: string; desc?: string; tooltip?: string }[]
  onChange: (v: T) => void
}) {
  return (
    <div className="flex gap-2">
      {options.map(opt => {
        const btn = (
          <button
            key={opt.value}
            onClick={() => onChange(opt.value)}
            className={`flex-1 px-3 py-2.5 rounded-lg text-sm font-medium transition-colors cursor-pointer border ${
              value === opt.value
                ? 'bg-violet-600/20 text-violet-400 border-violet-500/40'
                : 'bg-surface-900 text-slate-400 border-surface-600 hover:bg-surface-700'
            }`}
          >
            <div>{opt.label}</div>
            {opt.desc && <div className="text-xs opacity-60">{opt.desc}</div>}
          </button>
        )
        return opt.tooltip
          ? <Tooltip key={opt.value} text={opt.tooltip} position="bottom">{btn}</Tooltip>
          : <span key={opt.value}>{btn}</span>
      })}
    </div>
  )
}

/** Action buttons — extracted so platform hooks are called at component level */
function ActionsBar({ clipId, clip, saving, saved, exporting, exportProgress, exportDone, exportError, mediaAvailable, exportPreset, onSave, onExportForFormat, publishMeta, clipTitle, uploadHistory, onUploadHistoryChange }: {
  clipId: string; clip: Clip | null; saving: boolean; saved: boolean
  exporting: boolean; exportProgress: number; exportDone: boolean; exportError: string | null
  mediaAvailable: boolean
  exportPreset: { id: string; aspectRatio: string; name: string }
  onSave: () => void
  /** Export with a specific aspect ratio override (for multi-platform re-export) */
  onExportForFormat: (aspectRatio: string) => Promise<void>
  publishMeta?: { title: string; description: string; hashtags: string[]; visibility: string }
  /** The main clip title from the editor (single source of truth for upload title) */
  clipTitle: string
  /** Previously uploaded platform URLs (persisted in DB) */
  uploadHistory: Record<string, string>
  /** Callback to update upload history after a new upload */
  onUploadHistoryChange: (platform: string, url: string) => void
}) {
  const { connect, isConnected } = usePlatformStore()
  const { projects, addClip, createProject } = useMontageStore()
  const navigate = useNavigate()
  const [downloading, setDownloading] = useState(false)
  const [downloadResult, setDownloadResult] = useState<string | null>(null)

  // Multi-platform upload state
  const [platformSelections, setPlatformSelections] = useState<Record<string, boolean>>({})
  const [platformStates, setPlatformStates] = useState<Record<string, PlatformUploadState>>({})
  const [platformVisibilities, setPlatformVisibilities] = useState<Record<string, string>>({})
  const [multiUploading, setMultiUploading] = useState(false)

  useEffect(() => {
    invoke<Array<{
      platform: string
      status: string
      video_url: string | null
      last_error: string | null
    }>>('get_clip_upload_history', { clipId })
      .then(rows => {
        const tiktok = rows.find(row => row.platform === 'tiktok')
        if (!tiktok) return
        setPlatformStates(prev => {
          if (prev.tiktok?.status === 'uploading') return prev
          if (tiktok.status === 'inbox_delivered') {
            return { ...prev, tiktok: { status: 'done', progress: 100, draftHandoff: true } }
          }
          if (tiktok.status === 'processing' || tiktok.status === 'uploading') {
            return { ...prev, tiktok: { status: 'processing', progress: 100 } }
          }
          if (tiktok.status === 'failed') {
            return {
              ...prev,
              tiktok: { status: 'error', progress: 0, error: tiktok.last_error || 'TikTok processing failed' },
            }
          }
          if (tiktok.status === 'completed') {
            return {
              ...prev,
              tiktok: {
                status: 'done',
                progress: 100,
                videoUrl: tiktok.video_url || undefined,
                acceptedWithoutLink: !tiktok.video_url,
              },
            }
          }
          return prev
        })
      })
      .catch(() => {})
  }, [clipId])

  // Schedule state
  const { schedule: scheduleUpload, getForClip: getScheduledForClip } = useScheduleStore()
  const [scheduleMode, setScheduleMode] = useState(false)
  const [scheduleTime, setScheduleTime] = useState('')
  const [minimumScheduleTime, setMinimumScheduleTime] = useState('')
  const [scheduledUploads, setScheduledUploads] = useState<Array<{ id: string; platform: string; scheduled_time: string }>>([])
  const [scheduling, setScheduling] = useState(false)

  // Load existing scheduled uploads for this clip
  useEffect(() => {
    if (clipId) {
      getScheduledForClip(clipId).then(uploads => {
        setScheduledUploads(uploads.filter(u => u.status === 'pending').map(u => ({
          id: u.id, platform: u.platform, scheduled_time: u.scheduled_time,
        })))
      }).catch(() => {})
    }
  }, [clipId, getScheduledForClip])

  // YouTube sub-format state
  const clipDuration = clip ? Math.max(0, clip.end_seconds - clip.start_seconds) : 0
  const [youtubeSubFormat, setYoutubeSubFormat] = useState<YouTubeSubFormat>(() =>
    getDefaultYouTubeSubFormat(clipDuration, exportPreset.aspectRatio)
  )
  const [ytSubManuallySet, setYtSubManuallySet] = useState(false)
  useEffect(() => {
    if (ytSubManuallySet || !clip) return
    const newDefault = getDefaultYouTubeSubFormat(clipDuration, exportPreset.aspectRatio)
    setYoutubeSubFormat(newDefault)
  }, [clipDuration, exportPreset.aspectRatio, clip, ytSubManuallySet])

  const handleYouTubeSubFormatChange = (sub: YouTubeSubFormat) => {
    setYtSubManuallySet(true)
    setYoutubeSubFormat(sub)
    for (const key of expandYouTubeSubFormat(sub)) {
      if (!platformVisibilities[key]) {
        setPlatformVisibilities(prev => ({ ...prev, [key]: getDefaultVisibility(key) }))
      }
    }
  }

  const togglePlatform = (platform: string) => {
    setPlatformSelections(prev => ({ ...prev, [platform]: !prev[platform] }))
    if (platform === 'youtube') {
      for (const key of expandYouTubeSubFormat(youtubeSubFormat)) {
        if (!platformVisibilities[key]) {
          setPlatformVisibilities(prev => ({ ...prev, [key]: getDefaultVisibility(key) }))
        }
      }
    } else if (!platformVisibilities[platform]) {
      setPlatformVisibilities(prev => ({ ...prev, [platform]: getDefaultVisibility(platform) }))
    }
  }

  // TikTok Content Posting API compliance state — only consumed when TikTok is
  // a selected target. The panel reports validity so we can gate the buttons.
  const [tiktokCompliance, setTiktokCompliance] = useState<TikTokComplianceValue>(EMPTY_TIKTOK_COMPLIANCE)
  const [tiktokComplianceValid, setTiktokComplianceValid] = useState(false)

  const selectedPlatforms = Object.entries(platformSelections)
    .filter(([, checked]) => checked)
    .flatMap(([platform]) =>
      platform === 'youtube' ? expandYouTubeSubFormat(youtubeSubFormat) : [platform]
    )

  const anyUploading = multiUploading
  const hasProcessingUpload = selectedPlatforms.some(
    platform => platformStates[platform]?.status === 'processing',
  )
  const allSubmitted = selectedPlatforms.length > 0 &&
    selectedPlatforms.every(platform =>
      isSuccessfulUploadHandoff(platformStates[platform]?.status)
    )
  const forcedReuploadPlatforms = selectedPlatforms.filter(platform =>
    shouldOfferForcedReupload(platformStates[platform])
  )
  const hasForcedReuploadOption = forcedReuploadPlatforms.length > 0
  const tiktokAcceptedWithoutLink = platformStates.tiktok?.status === 'done'
    && platformStates.tiktok.acceptedWithoutLink === true
  const tiktokDraftHandoff = platformStates.tiktok?.status === 'done'
    && platformStates.tiktok.draftHandoff === true
  const tiktokProcessing = platformStates.tiktok?.status === 'processing'
  const tiktokPreviouslyAccepted = platformStates.tiktok?.status === 'duplicate'

  // Build upload metadata — includes title from main field, caption, and hashtags
  const buildUploadMeta = (platform: string, force = false) => {
    const baseDesc = publishMeta?.description || ''
    const tags = publishMeta?.hashtags || []
    const hashtagSuffix = tags.length > 0 ? tags.map(t => `#${t}`).join(' ') : ''
    // For YouTube: hashtags go in both tags array AND appended to description
    // For TikTok: hashtags appended to description
    const description = hashtagSuffix
      ? (baseDesc ? baseDesc + '\n\n' + hashtagSuffix : hashtagSuffix)
      : baseDesc
    const isTikTok = platform === 'tiktok'
    return {
      clip_id: clipId,
      title: clipTitle || clip?.title || 'Untitled Clip',
      description,
      tags,
      // TikTok: the compliance panel's privacy dropdown is the source of truth
      // (a real TikTok enum from creator_info). Other platforms keep their picker.
      visibility: isTikTok && tiktokCompliance.privacyLevel
        ? tiktokCompliance.privacyLevel
        : (platformVisibilities[platform] || getDefaultVisibility(platform)),
      force,
      ...(isTikTok ? {
        disable_comment: tiktokCompliance.disableComment,
        disable_duet: tiktokCompliance.disableDuet,
        disable_stitch: tiktokCompliance.disableStitch,
        brand_organic: tiktokCompliance.yourBrand,
        branded_content: tiktokCompliance.brandedContent,
        tiktok_publish_mode: tiktokCompliance.publishMode,
      } : {}),
    }
  }

  // Upload to a single platform
  const uploadToPlatform = async (platform: string, force = false): Promise<PlatformUploadState> => {
    const adapterPlatform = platform === 'youtube_shorts' ? 'youtube' : platform
    try {
      if (!isConnected(adapterPlatform)) {
        await connect(adapterPlatform)
      }
      const result = await invoke<UploadResult>('upload_to_platform', {
        platform: adapterPlatform,
        meta: buildUploadMeta(platform, force),
      })
      if (result.status.status === 'complete') {
        const url = result.status.video_url
        if (url) onUploadHistoryChange(platform, url)
        return {
          status: 'done',
          progress: 100,
          videoUrl: url ?? undefined,
          acceptedWithoutLink: adapterPlatform === 'tiktok' && !url,
        }
      } else if (result.status.status === 'duplicate') {
        const duplicateUrl = result.status.existing_url ?? undefined
        return duplicateUrl
          ? { status: 'done', progress: 100, duplicateUrl }
          : { status: 'duplicate', progress: 100 }
      } else if (result.status.status === 'failed') {
        return { status: 'error', progress: 0, error: result.status.error }
      } else if (isTikTokInboxDelivered(result.status.status)) {
        return { status: 'done', progress: 100, draftHandoff: true }
      } else if (result.status.status === 'processing') {
        return { status: 'processing', progress: 100 }
      } else if (result.status.status === 'uploading') {
        return { status: 'uploading', progress: result.status.progress_pct }
      }
      return { status: 'error', progress: 0, error: 'Unexpected upload state' }
    } catch (error: unknown) {
      return { status: 'error', progress: 0, error: errorMessage(error, 'Upload failed') }
    }
  }

  // Live upload-status events from the backend (chunk progress + platform-side
  // processing phase). Only applies while a platform row is mid-upload, so stale
  // or cross-clip events can't clobber terminal states (done/error).
  useEffect(() => {
    const unlisten = listen<{
      platform: string
      clip_id: string
      phase: string
      progress_pct?: number
      error?: string
      video_url?: string | null
    }>(
      'upload-status',
      (e) => {
        const p = e.payload
        if (p.clip_id !== clipId) return
        setPlatformStates(prev => {
          const cur = prev[p.platform]
          if (!cur || (cur.status !== 'uploading' && cur.status !== 'processing')) return prev
          if (p.phase === 'inbox_delivered') {
            return {
              ...prev,
              [p.platform]: { status: 'done' as const, progress: 100, draftHandoff: true },
            }
          }
          if (p.phase === 'complete') {
            return {
              ...prev,
              [p.platform]: {
                status: 'done' as const,
                progress: 100,
                videoUrl: p.video_url || undefined,
                acceptedWithoutLink: !p.video_url,
              },
            }
          }
          if (p.phase === 'failed') {
            return {
              ...prev,
              [p.platform]: { status: 'error' as const, progress: 0, error: p.error || 'Upload failed' },
            }
          }
          if (p.phase === 'processing') {
            return { ...prev, [p.platform]: { ...cur, status: 'processing' as const, progress: 100 } }
          }
          if (p.phase === 'uploading') {
            return { ...prev, [p.platform]: { ...cur, status: 'uploading' as const, progress: p.progress_pct ?? cur.progress } }
          }
          return prev
        })
      },
    )
    return () => { unlisten.then(f => f()) }
  }, [clipId])

  // Multi-platform upload orchestrator — always saves + exports first, then uploads
  const handleMultiUpload = async (forcePlatforms: Set<string> = new Set()) => {
    if (!clipId || selectedPlatforms.length === 0) return
    setMultiUploading(true)

    const platformsForRun = forcePlatforms.size > 0
      ? selectedPlatforms.filter(platform => forcePlatforms.has(platform))
      : selectedPlatforms

    // Save first
    onSave()

    // Group platforms by required aspect ratio
    const groups: Record<string, string[]> = {}
    for (const platform of platformsForRun) {
      const preset = getPresetForPlatform(platform)
      const ar = preset.aspectRatio
      if (!groups[ar]) groups[ar] = []
      groups[ar].push(platform)
    }

    const initStates: Record<string, PlatformUploadState> = {}
    for (const p of platformsForRun) initStates[p] = { status: 'waiting', progress: 0 }
    setPlatformStates(prev => ({ ...prev, ...initStates }))

    for (const [aspectRatio, platforms] of Object.entries(groups)) {
      // Always export before uploading (ensures latest settings are baked in)
      for (const p of platforms) {
        setPlatformStates(prev => ({ ...prev, [p]: { status: 'exporting', progress: 0 } }))
      }
      try {
        await onExportForFormat(aspectRatio)
        await new Promise(r => setTimeout(r, 500))
      } catch (error: unknown) {
        for (const p of platforms) {
          setPlatformStates(prev => ({
            ...prev,
            [p]: { status: 'error', progress: 0, error: `Export failed: ${errorMessage(error, 'Unknown error')}` },
          }))
        }
        continue
      }

      for (const platform of platforms) {
        setPlatformStates(prev => ({
          ...prev,
          [platform]: { status: 'uploading', progress: 0 },
        }))
        const result = await uploadToPlatform(platform, forcePlatforms.has(platform))
        setPlatformStates(prev => ({ ...prev, [platform]: result }))
      }
    }

    setMultiUploading(false)
  }

  // Schedule upload for later
  const handleScheduleUpload = async () => {
    if (!clipId || selectedPlatforms.length === 0 || !scheduleTime) return
    setScheduling(true)

    // Save first
    onSave()

    // Export before scheduling (so the file is ready when the schedule fires)
    const groups: Record<string, string[]> = {}
    for (const platform of selectedPlatforms) {
      const preset = getPresetForPlatform(platform)
      const ar = preset.aspectRatio
      if (!groups[ar]) groups[ar] = []
      groups[ar].push(platform)
    }

    for (const [aspectRatio, platforms] of Object.entries(groups)) {
      try {
        await onExportForFormat(aspectRatio)
        await new Promise(r => setTimeout(r, 500))
      } catch (error: unknown) {
        console.error('[Schedule] Export failed:', error)
        setScheduling(false)
        return
      }

      const isoTime = new Date(scheduleTime).toISOString()
      for (const platform of platforms) {
        const adapterPlatform = platform === 'youtube_shorts' ? 'youtube' : platform
        const meta = buildUploadMeta(platform, false)
        const metaJson = JSON.stringify(meta)
        try {
          const id = await scheduleUpload(clipId, adapterPlatform, isoTime, metaJson)
          setScheduledUploads(prev => [...prev, { id, platform: adapterPlatform, scheduled_time: isoTime }])
        } catch (error: unknown) {
          console.error(`[Schedule] Failed to schedule ${platform}:`, error)
        }
      }
    }

    setScheduling(false)
    setScheduleMode(false)
    setScheduleTime('')
  }

  const handleAddToMontage = () => {
    if (!clip) return
    let projectId = projects[0]?.id
    if (!projectId) projectId = createProject('My Montage')
    addClip(projectId, {
      clipId, clipTitle: clip.title,
      startSeconds: clip.start_seconds, endSeconds: clip.end_seconds,
      thumbnailPath: clip.thumbnail_path,
    })
    navigate('/montage')
  }

  // Combine persisted upload history with current session states for "View on" links
  const allUploadUrls: Record<string, string> = { ...uploadHistory }
  for (const [platform, state] of Object.entries(platformStates)) {
    if (state.status === 'done' && state.videoUrl) {
      allUploadUrls[platform] = state.videoUrl
    } else if (state.status === 'done' && state.duplicateUrl) {
      allUploadUrls[platform] = state.duplicateUrl
    }
  }

  const PLATFORM_LABELS: Record<string, string> = {
    youtube: 'YouTube',
    youtube_shorts: 'YouTube Shorts',
    tiktok: 'TikTok',
    instagram: 'Instagram',
  }

  return (
    <div className="v4-editor-publish-controls sticky bottom-0 bg-surface-900/80 backdrop-blur-sm p-2 -mx-1 rounded-lg space-y-2">
      {/* Save button */}
      <div className="flex gap-2">
        {/* Save button */}
        <Tooltip text="Save changes without exporting" position="top">
          <button onClick={onSave} disabled={saving}
            className="flex-1 flex items-center justify-center gap-2 px-4 py-2.5 bg-surface-700 hover:bg-surface-600 disabled:opacity-50 text-white text-sm font-medium rounded-lg transition-colors cursor-pointer">
            {saved ? <Check className="w-4 h-4" /> : <Save className="w-4 h-4" />}
            {saved ? 'Saved!' : saving ? 'Saving...' : 'Save'}
          </button>
        </Tooltip>

        {/* Download button */}
        <Tooltip text="Export clip and save video file to your download folder" position="top">
          <button
            disabled={downloading || exporting || !mediaAvailable}
            onClick={async () => {
              if (!clipId) return
              setDownloading(true)
              setDownloadResult(null)
              try {
                // Save settings first
                onSave()
                // Export if not already exported
                const fresh = await invoke<Clip>('get_clip_detail', { clipId })
                if (fresh.render_status !== 'completed') {
                  await onExportForFormat(exportPreset.aspectRatio)
                }
                // Save to configured folder (or prompt to pick one)
                const savedPath = await invoke<string | null>('save_clip_to_disk', { clipId })
                if (savedPath) {
                  setDownloadResult(savedPath)
                  setTimeout(() => setDownloadResult(null), 5000)
                }
              } catch (err) {
                console.error('[Download] Failed:', err)
              } finally {
                setDownloading(false)
              }
            }}
            className="flex-1 flex items-center justify-center gap-2 px-4 py-2.5 bg-emerald-600/20 border border-emerald-500/40 hover:bg-emerald-600/30 disabled:opacity-50 text-emerald-400 text-sm font-medium rounded-lg transition-colors cursor-pointer">
            {downloadResult
              ? <><Check className="w-4 h-4" /> Saved!</>
              : downloading
                ? <><Loader2 className="w-4 h-4 animate-spin" /> {exporting ? 'Exporting...' : 'Saving...'}</>
                : <><Download className="w-4 h-4" /> Download</>
            }
          </button>
        </Tooltip>
      </div>

      {/* Download success banner */}
      {downloadResult && (
        <div className="flex items-center gap-2 px-3 py-2 bg-emerald-600/10 border border-emerald-500/30 rounded-lg">
          <Check className="w-3.5 h-3.5 text-emerald-400 shrink-0" />
          <span className="text-[11px] text-emerald-300 truncate" title={downloadResult}>
            Saved to {downloadResult.replace(/^.*[/\\]/, '')}
          </span>
          <button onClick={() => {
            const sep = downloadResult.includes('\\') ? '\\' : '/'
            const folder = downloadResult.substring(0, downloadResult.lastIndexOf(sep))
            invoke('open_folder', { path: folder }).catch((e) => console.error('Failed to open folder:', e))
          }}
            className="ml-auto text-[10px] text-emerald-400 hover:text-emerald-300 underline shrink-0 cursor-pointer">
            Open folder
          </button>
        </div>
      )}

      {/* Export progress bar — shown during checkbox upload export phase */}
      {(exporting || exportError) && (
        <ExportProgressBar
          progress={exportProgress}
          done={exportDone}
          error={exportError}
          active={exporting}
        />
      )}

      {/* Platform upload section */}
      <div className="space-y-1.5">
        <PlatformUploadSelector
          selected={platformSelections}
          onToggle={togglePlatform}
          visibilities={platformVisibilities}
          onVisibilityChange={(platform, vis) =>
            setPlatformVisibilities(prev => ({ ...prev, [platform]: vis }))
          }
          states={platformStates}
          currentPresetId={exportPreset.id}
          disabled={anyUploading}
          onViewUrl={(url) => openUrl(url)}
          onConnect={async (platform) => {
            try {
              await connect(platform)
            } catch (error: unknown) {
              setPlatformStates((previous) => ({
                ...previous,
                [platform]: {
                  status: 'error',
                  progress: 0,
                  error: errorMessage(error, 'Connection failed'),
                },
              }))
            }
          }}
          youtubeSubFormat={youtubeSubFormat}
          onYouTubeSubFormatChange={handleYouTubeSubFormatChange}
          clipDuration={clipDuration}
        />

        {/* TikTok Content Posting API compliance panel (required for audit) */}
        {selectedPlatforms.includes('tiktok')
          && !isSuccessfulUploadHandoff(platformStates.tiktok?.status)
          && (
          <TikTokComplianceFields
            value={tiktokCompliance}
            onChange={setTiktokCompliance}
            onValidityChange={setTiktokComplianceValid}
            clipDurationSec={clipDuration}
          />
        )}

        {tiktokProcessing && (
          <div role="status" className="border-l-2 border-cyan-400 bg-cyan-500/5 px-3 py-2 text-[11px] leading-relaxed text-cyan-100">
            TikTok received the video and is still processing it. TikTok has not confirmed an
            Inbox handoff or finished post yet. ClipGoblin will update this screen when TikTok
            reports the next state. Do not upload another copy while waiting.
          </div>
        )}

        {tiktokDraftHandoff && (
          <div role="status" className="border-l-2 border-cyan-400 bg-cyan-500/5 px-3 py-2 text-[11px] leading-relaxed text-cyan-100">
            TikTok received the draft. Open the notification in TikTok's Inbox to edit it,
            add the caption and audience, then publish. ClipGoblin will keep checking for the
            final post in the background.
          </div>
        )}

        {(tiktokAcceptedWithoutLink || tiktokPreviouslyAccepted) && (
          <div role="status" className="border-l-2 border-amber-400 bg-amber-500/5 px-3 py-2 text-[11px] leading-relaxed text-amber-200">
            {tiktokAcceptedWithoutLink
              ? 'TikTok reported that the private post was accepted. Private posts do not return a link, so check Profile > Private (the lock tab) in the TikTok mobile app. It can take several minutes and occasionally longer to appear. Do not upload another copy while waiting.'
              : 'TikTok previously reported this clip as accepted, so ClipGoblin did not send another copy. Check Profile > Private (the lock tab). Use the button below only if the post is missing.'}
          </div>
        )}

        {/* Schedule toggle + Upload/Schedule buttons */}
        {selectedPlatforms.length > 0 && (
          <div className="space-y-1.5">
            {/* Schedule toggle */}
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={scheduleMode}
                onChange={(e) => setScheduleMode(e.target.checked)}
                disabled={anyUploading || scheduling}
                className="w-3.5 h-3.5 rounded border-surface-600 bg-surface-700 text-violet-500 focus:ring-violet-500"
              />
              <Clock className="w-3.5 h-3.5 text-slate-400" />
              <span className="text-xs text-slate-300">Schedule for later</span>
            </label>

            {/* Date/time picker (shown when schedule mode is on) */}
            {scheduleMode && (
              <input
                type="datetime-local"
                value={scheduleTime}
                onChange={(e) => setScheduleTime(e.target.value)}
                onFocus={() => setMinimumScheduleTime(localDateTimeAfter(60_000))}
                min={minimumScheduleTime}
                className="w-full bg-surface-700 border border-surface-600 rounded-lg px-3 py-1.5 text-sm text-white focus:border-violet-500 focus:outline-none"
              />
            )}

            {/* Action button: Upload Now or Schedule */}
            {scheduleMode ? (
              <button
                onClick={handleScheduleUpload}
                disabled={scheduling || !scheduleTime || !mediaAvailable || (selectedPlatforms.includes('tiktok') && !tiktokComplianceValid)}
                className="w-full flex items-center justify-center gap-2 px-3 py-2 text-xs font-medium rounded-lg transition-colors cursor-pointer border bg-amber-600/80 text-white border-amber-500 hover:bg-amber-500 disabled:opacity-60"
              >
                {scheduling ? (
                  <><Loader2 className="w-3.5 h-3.5 animate-spin" /> Scheduling...</>
                ) : (
                  <><CalendarClock className="w-3.5 h-3.5" />
                    Schedule for {scheduleTime ? new Date(scheduleTime).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' }) : '...'}
                  </>
                )}
              </button>
            ) : (
              <button
                onClick={() => void handleMultiUpload(new Set(forcedReuploadPlatforms))}
                disabled={anyUploading || !mediaAvailable || (allSubmitted && !hasForcedReuploadOption) || (selectedPlatforms.includes('tiktok') && !tiktokComplianceValid)}
                className={`w-full flex items-center justify-center gap-2 px-3 py-2 text-xs font-medium rounded-lg transition-colors cursor-pointer border ${
                  allSubmitted && !hasForcedReuploadOption
                    ? 'bg-green-600/20 text-green-400 border-green-500/30'
                    : anyUploading
                    ? 'bg-violet-600/20 text-violet-400 border-violet-500/30'
                    : 'bg-violet-600 text-white border-violet-500 hover:bg-violet-500'
                } disabled:opacity-60`}
              >
                {anyUploading ? (
                  <><Loader2 className="w-3.5 h-3.5 animate-spin" />
                    {exporting ? 'Exporting...' : 'Uploading...'}
                  </>
                ) : hasForcedReuploadOption ? (
                  <><RefreshCw className="w-3.5 h-3.5" />
                    Upload another copy to {forcedReuploadPlatforms.length === 1
                      ? (PLATFORM_INFO[forcedReuploadPlatforms[0]]?.name || forcedReuploadPlatforms[0])
                      : `${forcedReuploadPlatforms.length} platforms`}
                  </>
                ) : allSubmitted ? (
                  <><Check className="w-3.5 h-3.5" />
                    {hasProcessingUpload
                      ? 'Upload submitted - processing'
                      : tiktokDraftHandoff && selectedPlatforms.length === 1
                        ? 'Draft sent to TikTok inbox'
                      : tiktokAcceptedWithoutLink && selectedPlatforms.length === 1
                        ? 'TikTok accepted private post'
                        : 'All uploads complete'}
                  </>
                ) : (
                  <><Upload className="w-3.5 h-3.5" />
                    {selectedPlatforms.length === 1
                      && selectedPlatforms[0] === 'tiktok'
                      && tiktokCompliance.publishMode === 'draft'
                      ? 'Export & Send to TikTok drafts'
                      : `Export & Upload to ${selectedPlatforms.length === 1
                        ? (selectedPlatforms[0] === 'youtube_shorts' ? 'YouTube Shorts' : PLATFORM_INFO[selectedPlatforms[0]]?.name || selectedPlatforms[0])
                        : `${selectedPlatforms.length} platforms`}`}
                  </>
                )}
              </button>
            )}
          </div>
        )}

        {/* Scheduled uploads for this clip */}
        {scheduledUploads.length > 0 && (
          <div className="space-y-1">
            {scheduledUploads.map(su => (
              <div key={su.id} className="flex items-center gap-2 px-3 py-1.5 text-xs bg-amber-500/10 border border-amber-500/20 rounded-lg">
                <Clock className="w-3 h-3 text-amber-400 shrink-0" />
                <span className="text-amber-300 truncate">
                  Scheduled for {new Date(su.scheduled_time).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })}
                  {' '}on {PLATFORM_INFO[su.platform]?.name || su.platform}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* "View on [Platform]" links — persisted from DB + current session */}
      {Object.keys(allUploadUrls).length > 0 && (
        <div className="space-y-1">
          {Object.entries(allUploadUrls).map(([platform, url]) => (
            <button
              key={platform}
              onClick={() => openUrl(url)}
              className="w-full flex items-center justify-center gap-1.5 px-3 py-1.5 text-xs text-emerald-400 bg-emerald-500/10 border border-emerald-500/20 rounded-lg hover:bg-emerald-500/20 transition-colors cursor-pointer"
            >
              <Link2 className="w-3 h-3" />
              View on {PLATFORM_LABELS[platform] || platform}
            </button>
          ))}
        </div>
      )}

      {/* Add to Montage */}
      <button onClick={handleAddToMontage}
        className="w-full flex items-center justify-center gap-1.5 px-3 py-2 text-xs text-slate-400 bg-surface-800 border border-surface-600 rounded-lg hover:text-white hover:border-violet-500/40 transition-colors cursor-pointer">
        <Film className="w-3.5 h-3.5" />
        Add to Montage
      </button>
    </div>
  )
}

export default function Editor() {
  const { clipId } = useParams()
  const navigate = useNavigate()
  const location = useLocation()
  const stopAll = usePlaybackStore(s => s.stopAll)

  // Stop all other playback when editor opens
  useEffect(() => { stopAll() }, [stopAll])

  const [clip, setClip] = useState<Clip | null>(null)
  const [highlight, setHighlight] = useState<Highlight | null>(null)
  const [vod, setVod] = useState<Vod | null>(null)
  // Cam region edit-mode plumbing -- editing happens in a full-screen modal
  // (CamRegionModal) so the user can drag on the UNCROPPED source frame.
  type RegionEditScope = 'vod' | 'clip-override' | null
  const [regionEditScope, setRegionEditScope] = useState<RegionEditScope>(null)
  // Shared ref to the main ClipPlayer's underlying <video> element. The
  // CamRegionPreview draws frames from THIS element to a canvas (instead of
  // opening its own decoder, which Tauri's WebView doesn't reliably support
  // when two <video> tags target the same asset URL).
  const mainVideoElementRef = useRef<HTMLVideoElement | null>(null)
  // Whether per-clip cam-region override is enabled in Settings.
  // Lifted here (vs. internal to CamRegionRow) so the live preview can
  // resolve the effective region using the same override precedence.
  const [allowPerClipCamOverride, setAllowPerClipCamOverride] = useState(false)
  useEffect(() => {
    invoke<boolean>('get_allow_per_clip_override')
      .then(setAllowPerClipCamOverride)
      .catch(() => setAllowPerClipCamOverride(false))
  }, [])
  // Narrow refetch: refresh only cam-region columns on the vod/clip rows
  // without re-running the full clip-load (which would reset editor state
  // like facecamLayout to the DB's saved value, wiping unsaved layout picks).
  const refetchCamRegions = async () => {
    try {
      if (clip?.id) {
        const c = await invoke<Clip>('get_clip_detail', { clipId: clip.id })
        setClip(prev => prev ? {
          ...prev,
          cam_region_norm_override: c.cam_region_norm_override,
          cam_fit_mode: c.cam_fit_mode,
        } : prev)
      }
      if (vod?.id) {
        const v = await invoke<Vod>('get_vod_detail', { vodId: vod.id })
        setVod(prev => prev ? { ...prev, cam_region_norm: v.cam_region_norm } : prev)
      }
    } catch (e) {
      console.error('[Editor] refetch cam regions failed', e)
    }
  }
  const [videoSrc, setVideoSrc] = useState<string | null>(null)
  const [editorLoadError, setEditorLoadError] = useState<string | null>(null)
  const [editorLoadAttempt, setEditorLoadAttempt] = useState(0)
  const [saving, setSaving] = useState(false)
  const [exporting, setExporting] = useState(false)
  const [exportProgress, setExportProgress] = useState(0)
  const [exportDone, setExportDone] = useState(false)
  const [exportError, setExportError] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)
  // Persisted upload history: { platform: videoUrl } loaded from DB
  const [uploadHistory, setUploadHistory] = useState<Record<string, string>>({})
  const [originalStart, setOriginalStart] = useState(0)
  const [originalEnd, setOriginalEnd] = useState(0)
  const [captionTimelineStart, setCaptionTimelineStart] = useState(0)
  const [thumbnailPath, setThumbnailPath] = useState<string | null>(null)

  // ── Editor state ──
  const [title, setTitle] = useState('')
  const [startSeconds, setStartSeconds] = useState(0)
  const [endSeconds, setEndSeconds] = useState(0)
  const [aspectRatio, setAspectRatio] = useState('9:16')
  const [captionsEnabled, setCaptionsEnabled] = useState(true)
  const [captionsText, setCaptionsText] = useState('')
  const [captionsPosition, setCaptionsPosition] = useState('bottom')
  const [captionYOffset, setCaptionYOffset] = useState(0) // % offset from preset position (-20 to +20)
  const [captionStyleId, setCaptionStyleId] = useState('clean')
  const [captionFontScale, setCaptionFontScale] = useState(1)
  const [aiEmphasisEnabled, setAiEmphasisEnabled] = useState(true)
  const [generatingCaptions, setGeneratingCaptions] = useState(false)
  const [captionError, setCaptionError] = useState('')
  const [facecamLayout, setFacecamLayout] = useState<LayoutMode>('none')
  const [facecamSettings, setFacecamSettings] = useState<FacecamSettings>(DEFAULT_FACECAM)
  const [contextBackgroundPath, setContextBackgroundPath] = useState<string | null>(null)
  const [contextBackgroundMode, setContextBackgroundMode] = useState<ContextBackgroundMode>('blur')
  const [contextBlurStrength, setContextBlurStrength] = useState(DEFAULT_CONTEXT_BLUR_STRENGTH)
  const [contextVideoY, setContextVideoY] = useState(DEFAULT_CONTEXT_VIDEO_Y)
  const [pickingBranding, setPickingBranding] = useState(false)
  const [brandingError, setBrandingError] = useState('')
  const [layoutPickerOpen, setLayoutPickerOpen] = useState(false)
  const [exportPresetId, setExportPresetId] = useState('tiktok')
  const [textOverlays] = useState<TextOverlay[]>([])
  const [game, setGame] = useState<string>('')
  const [publishMeta, setPublishMeta] = useState<PublishMetadata>({
    title: '', description: '', hashtags: [], visibility: 'public',
  })
  const [editorWorkspace, setEditorWorkspace] = useState<EditorWorkspaceId>(() => {
    const workspace = (location.state as { workspace?: unknown } | null)?.workspace
    return isEditorWorkspaceId(workspace) ? workspace : 'edit'
  })
  const editorWorkspaceTabRefs = useRef<Partial<Record<EditorWorkspaceId, HTMLButtonElement | null>>>({})

  const handlePickContextBranding = async () => {
    setPickingBranding(true)
    setBrandingError('')
    try {
      const path = await invoke<string | null>('pick_context_branding_asset')
      if (path) {
        setContextBackgroundPath(path)
        setContextBackgroundMode('branding')
      }
    } catch (error) {
      setBrandingError(errorMessage(error, 'Could not add this branding asset'))
    } finally {
      setPickingBranding(false)
    }
  }

  const handleEditorWorkspaceKeyDown = (
    event: React.KeyboardEvent<HTMLButtonElement>,
    workspace: EditorWorkspaceId,
  ) => {
    const key = event.key as EditorWorkspaceNavigationKey
    if (!['ArrowDown', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'End', 'Home'].includes(key)) return

    event.preventDefault()
    const nextWorkspace = getNextEditorWorkspace(workspace, key)
    setEditorWorkspace(nextWorkspace)
    requestAnimationFrame(() => editorWorkspaceTabRefs.current[nextWorkspace]?.focus())
  }

  const clipDuration = Math.max(0, endSeconds - startSeconds)
  // When the clip is backed by a standalone community-clip MP4, the preview
  // plays that whole file (no VOD seek/trim window). Used to switch ClipPlayer
  // into fullFile mode and to skip the VOD-relative trim timeline.
  const isCommunityClip = !!clip?.community_clip_mp4_path
  const captionStyle = CAPTION_STYLES.find(s => s.id === captionStyleId) || CAPTION_STYLES[0]
  const exportPreset = EXPORT_PRESETS.find(p => p.id === exportPresetId) || EXPORT_PRESETS[0]

  // ── Templates ──
  const templateStore = useTemplateStore()
  const [templateDropdownOpen, setTemplateDropdownOpen] = useState(false)
  const [templateSaveOpen, setTemplateSaveOpen] = useState(false)
  const [templateSaveName, setTemplateSaveName] = useState('')
  const [templateSaved, setTemplateSaved] = useState(false)
  const templateDropdownRef = useRef<HTMLDivElement>(null)

  // Close template dropdown on outside click
  useEffect(() => {
    if (!templateDropdownOpen) return
    const handler = (e: MouseEvent) => {
      if (templateDropdownRef.current && !templateDropdownRef.current.contains(e.target as Node)) {
        setTemplateDropdownOpen(false)
      }
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [templateDropdownOpen])

  const handleLoadTemplate = useCallback((tmpl: ClipTemplate) => {
    setCaptionStyleId(tmpl.captionStyleId)
    setCaptionsPosition(tmpl.captionPosition)
    setCaptionFontScale(clampCaptionFontScale(tmpl.captionFontScale ?? 1))
    setCaptionYOffset(Math.max(-20, Math.min(20, tmpl.captionYOffset ?? 0)))
    setExportPresetId(tmpl.exportPresetId)
    if ('contextBackgroundPath' in tmpl) {
      setContextBackgroundPath(tmpl.contextBackgroundPath || null)
    }
    if (tmpl.contextBackgroundMode) {
      setContextBackgroundMode(normalizeContextBackgroundMode(tmpl.contextBackgroundMode))
    }
    if (tmpl.contextBlurStrength != null) {
      setContextBlurStrength(normalizeContextBlurStrength(tmpl.contextBlurStrength))
    }
    if (tmpl.contextVideoY != null) {
      setContextVideoY(normalizeContextVideoY(tmpl.contextVideoY))
    }
    setPublishMeta(prev => ({
      ...prev,
      hashtags: [...tmpl.hashtags],
    }))
    setTemplateDropdownOpen(false)
  }, [])

  const handleSaveTemplate = useCallback(() => {
    const name = templateSaveName.trim()
    if (!name) return
    templateStore.create(name, {
      captionStyleId,
      captionPosition: captionsPosition as 'top' | 'center' | 'bottom',
      captionFontScale,
      captionYOffset,
      captionTone: 'punchy', // default — user can change later
      hashtags: publishMeta.hashtags,
      exportPresetId,
      contextBackgroundPath,
      contextBackgroundMode,
      contextBlurStrength,
      contextVideoY,
    })
    setTemplateSaveName('')
    setTemplateSaveOpen(false)
    setTemplateSaved(true)
    setTimeout(() => setTemplateSaved(false), 2000)
  }, [templateSaveName, captionStyleId, captionsPosition, captionFontScale, captionYOffset, publishMeta.hashtags, exportPresetId, contextBackgroundPath, contextBackgroundMode, contextBlurStrength, contextVideoY, templateStore])

  // ── Undo / Redo history ──
  const history = useEditorHistory()
  const [, setHistoryTick] = useState(0) // bumped on undo/redo to re-render buttons
  const historyRestoringRef = useRef(false)          // true while applying a snapshot

  const takeSnapshot = useCallback((): EditorSnapshot => ({
    title, startSeconds, endSeconds, captionsText,
    captionsPosition, captionStyleId, captionFontScale, captionYOffset,
    publishTitle: publishMeta.title,
    publishDescription: publishMeta.description,
    publishHashtags: publishMeta.hashtags,
  }), [title, startSeconds, endSeconds, captionsText, captionsPosition, captionStyleId, captionFontScale, captionYOffset, publishMeta])

  const applySnapshot = useCallback((snap: EditorSnapshot) => {
    historyRestoringRef.current = true
    setTitle(snap.title)
    setStartSeconds(snap.startSeconds)
    setEndSeconds(snap.endSeconds)
    setCaptionsText(snap.captionsText)
    setCaptionsPosition(snap.captionsPosition)
    setCaptionStyleId(snap.captionStyleId)
    setCaptionFontScale(clampCaptionFontScale(snap.captionFontScale))
    setCaptionYOffset(snap.captionYOffset)
    setPublishMeta(prev => ({
      ...prev,
      title: snap.publishTitle,
      description: snap.publishDescription,
      hashtags: snap.publishHashtags,
    }))
    // Allow next snapshot push after React processes the state updates
    requestAnimationFrame(() => { historyRestoringRef.current = false })
  }, [])

  const handleUndo = useCallback(() => {
    const snap = history.undo()
    if (snap) { applySnapshot(snap); setHistoryTick(t => t + 1) }
  }, [history, applySnapshot])

  const handleRedo = useCallback(() => {
    const snap = history.redo()
    if (snap) { applySnapshot(snap); setHistoryTick(t => t + 1) }
  }, [history, applySnapshot])

  // Push snapshot on tracked state changes (debounced to batch rapid edits)
  useEffect(() => {
    if (historyRestoringRef.current) return // don't push while restoring
    const timer = setTimeout(() => {
      if (!historyRestoringRef.current) {
        history.push(takeSnapshot())
        setHistoryTick(t => t + 1)
      }
    }, 400)
    return () => clearTimeout(timer)
  }, [title, startSeconds, endSeconds, captionsText, captionsPosition, captionStyleId, captionFontScale, captionYOffset, publishMeta, history, takeSnapshot])

  // Keyboard shortcuts: Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey
      if (!mod) return
      if (e.key === 'z' && !e.shiftKey) {
        e.preventDefault()
        handleUndo()
      } else if (e.key === 'y' || (e.key === 'z' && e.shiftKey) || (e.key === 'Z' && e.shiftKey)) {
        e.preventDefault()
        handleRedo()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [handleUndo, handleRedo])

  // ── Auto-save publish metadata (description + hashtags) to DB ──
  const publishMetaInitRef = useRef(false) // skip initial load write-back
  useEffect(() => {
    if (!clipId) return
    // Skip the first update (initial load from DB)
    if (!publishMetaInitRef.current) {
      publishMetaInitRef.current = true
      return
    }
    const timer = setTimeout(() => {
      const hashtagStr = publishMeta.hashtags.length > 0 ? publishMeta.hashtags.join(',') : null
      invoke('set_clip_publish_meta', {
        clipId,
        description: publishMeta.description || null,
        hashtags: hashtagStr,
      }).catch(err => console.warn('[Editor] Failed to auto-save publish meta:', err))
    }, 500) // debounce 500ms
    return () => clearTimeout(timer)
  }, [clipId, publishMeta.description, publishMeta.hashtags])

  // ── Player state (declared before subtitle/marker code that depends on it) ──
  const playerSeekRef = useRef<((time: number) => void) | null>(null)
  const [playbackTime, setPlaybackTime] = useState(0)
  const [isPlaying, setIsPlaying] = useState(false)

  // ── Subtitle segments (direct state — NOT derived from SRT on every keystroke) ──
  const [subtitleSegments, setSubtitleSegments] = useState<SubtitleSegment[]>([])
  const captionsTextRef = useRef(captionsText)
  const hasSrtCaptions = subtitleSegments.length > 0
  const canGenerateSubtitles = canGenerateTimedCaptions(clip, vod)

  // Sync: external captionsText changes → parse into segments
  useEffect(() => {
    if (captionsText !== captionsTextRef.current) {
      captionsTextRef.current = captionsText
      setSubtitleSegments(splitSubtitleSegmentsByWord(parseSrt(captionsText)))
    }
  }, [captionsText])

  // Sync: segment edits → captionsText (debounced, for save/export/emphasis)
  useEffect(() => {
    const timeout = setTimeout(() => {
      const srt = serializeSrt(subtitleSegments.filter(s => s.text.trim()))
      captionsTextRef.current = srt
      setCaptionsText(srt)
    }, 300)
    return () => clearTimeout(timeout)
  }, [subtitleSegments])

  // SRT timestamps remain tied to the source position where captions were
  // generated, even if the clip is trimmed again later.
  const srtTime = playbackTime - captionTimelineStart
  // SRT-relative trim bounds
  const srtTrimStart = startSeconds - captionTimelineStart
  const srtTrimEnd = endSeconds - captionTimelineStart
  const trimmedTranscriptText = useMemo(() => {
    const visibleSegments = subtitleSegments.filter(segment =>
      segment.endTime >= srtTrimStart && segment.startTime <= srtTrimEnd
    )
    const relevantSegments = visibleSegments.length > 0 ? visibleSegments : subtitleSegments
    const transcript = relevantSegments.map(segment => segment.text.trim()).filter(Boolean).join(' ')
    return transcript || undefined
  }, [subtitleSegments, srtTrimStart, srtTrimEnd])

  // Active subtitle segment
  const activeSubtitle = useMemo(
    () => findActiveSegment(subtitleSegments, srtTime),
    [subtitleSegments, srtTime]
  )

  // ── Analyze caption emphasis ──
  const captionTokens: CaptionToken[] = useMemo(() => {
    if (!hasSrtCaptions) return []
    const originalDuration = originalEnd - originalStart
    return analyzeEmphasis(captionsText, originalDuration > 0 ? originalDuration : clipDuration)
  }, [captionsText, hasSrtCaptions, originalStart, originalEnd, clipDuration])

  const emphasisSummary = useMemo(
    () => getEmphasisSummary(captionTokens),
    [captionTokens]
  )

  // ── Subtitle editing callbacks (direct state update — no SRT roundtrip) ──
  const handleSubtitleEdit = useCallback((id: string, text: string) => {
    setSubtitleSegments(prev => prev.map(s => s.id === id ? { ...s, text } : s))
  }, [])
  const handleSubtitleDelete = useCallback((id: string) => {
    setSubtitleSegments(prev => prev.filter(s => s.id !== id))
  }, [])
  const handleSubtitleSeek = useCallback((srtTimeTarget: number) => {
    playerSeekRef.current?.(captionTimelineStart + srtTimeTarget)
  }, [captionTimelineStart])
  const handleSubtitleShift = useCallback((deltaSeconds: number) => {
    const shifted = shiftSubtitleSegments(subtitleSegments, deltaSeconds)
    const srt = serializeSrt(shifted.filter(segment => segment.text.trim()))
    captionsTextRef.current = srt
    setSubtitleSegments(shifted)
    setCaptionsText(srt)
    setSaved(false)
  }, [subtitleSegments])

  const handleGenerateCaptions = async () => {
    if (!clipId || generatingCaptions) return
    setGeneratingCaptions(true)
    setCaptionError('')
    try {
      const srt = await invoke<string>('generate_clip_captions', { clipId })
      setCaptionTimelineStart(isCommunityClip ? 0 : startSeconds)
      setCaptionsText(srt)

      // Game detection is manual — no auto-inference after subtitle generation
    } catch (err) {
      const raw = String(err)
      console.error('Caption generation error:', raw)
      // The backend now returns specific error messages — display them directly
      // when they're user-friendly, fall back to generic mapping otherwise.
      const lower = raw.toLowerCase()
      if (lower.includes('faster-whisper is not installed')) {
        // Backend detected missing package and included the pip command
        setCaptionError(raw.replace('Transcription error: ', ''))
      } else if (lower.includes('python not found')) {
        setCaptionError('Python not found. Install Python 3.10+ to generate subtitles.')
      } else if (lower.includes('transcribe.py not found')) {
        setCaptionError('Transcription script missing. Place transcribe.py in ai_engine/ folder.')
      } else if (lower.includes('version mismatch')) {
        setCaptionError('Transcription script needs updating. Replace ai_engine/transcribe.py with the latest version.')
      } else if (lower.includes('no speech detected')) {
        setCaptionError('No speech detected in this clip.')
      } else if (lower.includes('vod not downloaded')) {
        setCaptionError('VOD not downloaded. Download the VOD first.')
      } else if (lower.includes('failed to load model')) {
        setCaptionError('Whisper model failed to load. Check disk space and internet connection.')
      } else {
        setCaptionError(raw.replace('Transcription error: ', '') || 'Subtitle generation failed.')
      }
    } finally {
      setGeneratingCaptions(false)
    }
  }

  // ── Build timeline markers (memoized to avoid rebuilding on every render) ──
  const { timelineMarkers, suggestedHookStart } = useMemo(() => {
    const markers: TimelineMarker[] = []
    let hookSuggestion: number | undefined

    if (highlight) {
      markers.push({ time: originalStart, type: 'event', label: 'Detected moment start', confidence: highlight.confidence_score ?? Math.min(highlight.virality_score * 0.85 - 0.10, 0.95) })

      if (highlight.audio_score > 0.6) {
        const hookTime = Math.max(0, originalStart + 1)
        markers.push({ time: hookTime, type: 'hook', label: `Strong hook (${Math.round(highlight.audio_score * 100)}% audio)`, confidence: highlight.audio_score })
        if (Math.abs(hookTime - startSeconds) > 1) hookSuggestion = hookTime
      }

      if (highlight.visual_score > 0.5) {
        const reactionTime = originalStart + (originalEnd - originalStart) * 0.4
        markers.push({ time: reactionTime, type: 'reaction', label: `Reaction peak (${Math.round(highlight.visual_score * 100)}%)`, confidence: highlight.visual_score })
      }

      markers.push({ time: originalEnd - 2, type: 'payoff', label: 'Payoff / resolution', confidence: 0.5 })
    }

    if (hasSrtCaptions && aiEmphasisEnabled) {
      for (const phrase of emphasisSummary) {
        const absTime = captionTimelineStart + phrase.time
        if (absTime >= originalStart && absTime <= originalEnd) {
          const markerType = phrase.type === 'urgency' ? 'reaction' as const
            : phrase.type === 'payoff' || phrase.type === 'punchline' ? 'payoff' as const
            : 'event' as const
          markers.push({ time: absTime, type: markerType, label: `"${phrase.text}" (${phrase.type})`, confidence: 0.7 })
        }
      }
    }

    return { timelineMarkers: markers, suggestedHookStart: hookSuggestion }
  }, [highlight, originalStart, originalEnd, startSeconds, captionTimelineStart, hasSrtCaptions, aiEmphasisEnabled, emphasisSummary])

  // ── Load clip data ──
  useEffect(() => {
    if (!clipId) return
    publishMetaInitRef.current = false // reset so next publishMeta update from load is skipped
    setEditorLoadError(null)
    setClip(null)
    setHighlight(null)
    setVod(null)
    setVideoSrc(null)
    ;(async () => {
      try {
        const c = await invoke<Clip>('get_clip_detail', { clipId })
        setClip(c)
        setTitle(c.title)
        setStartSeconds(c.start_seconds)
        setEndSeconds(c.end_seconds)
        setAspectRatio(c.aspect_ratio || '9:16')
        // Sync export preset to match the clip's saved aspect ratio
        const matchingPreset = EXPORT_PRESETS.find(p => p.aspectRatio === c.aspect_ratio)
        if (matchingPreset) setExportPresetId(matchingPreset.id)
        setCaptionsEnabled(c.captions_enabled === 1)
        setCaptionsText(c.captions_text || '')
        setCaptionsPosition(c.captions_position || 'bottom')
        setCaptionYOffset(Math.max(-20, Math.min(20, c.caption_y_offset ?? 0)))
        setCaptionStyleId(c.caption_style || 'clean')
        setCaptionFontScale(clampCaptionFontScale(c.caption_font_scale ?? 1))
        setCaptionTimelineStart(getCaptionTimelineStart(c))
        const savedLayout = LAYOUT_OPTIONS.some((layout) => layout.id === c.facecam_layout)
          ? c.facecam_layout as LayoutMode
          : 'none'
        setFacecamLayout(savedLayout)
        setFacecamSettings(parseFacecamSettings(c.facecam_settings))
        setContextBackgroundPath(c.context_background_path || null)
        setContextBackgroundMode(normalizeContextBackgroundMode(c.context_background_mode))
        setContextBlurStrength(normalizeContextBlurStrength(c.context_blur_strength))
        setContextVideoY(normalizeContextVideoY(c.context_video_y))
        setBrandingError('')
        setExportDone(c.render_status === 'completed')
        setOriginalStart(c.start_seconds)
        setOriginalEnd(c.end_seconds)
        setThumbnailPath(c.thumbnail_path)
        setPlaybackTime(c.start_seconds)
        console.log('[Editor] Clip loaded — clip.game:', JSON.stringify(c.game), '| captions_text length:', c.captions_text?.length ?? 0)
        setPublishMeta(prev => ({
          ...prev,
          title: c.title,
          description: c.publish_description || '',
          hashtags: c.publish_hashtags ? c.publish_hashtags.split(',').filter(Boolean) : [],
        }))

        // Load persisted upload history (View on YouTube/TikTok links)
        invoke<Array<{ platform: string; video_url: string | null }>>('get_clip_upload_history', { clipId })
          .then(rows => {
            const hist: Record<string, string> = {}
            for (const r of rows) {
              if (r.video_url) hist[r.platform] = r.video_url
            }
            setUploadHistory(hist)
          })
          .catch(() => {})

        // Initialize undo/redo history with the loaded clip state
        history.reset({
          title: c.title,
          startSeconds: c.start_seconds,
          endSeconds: c.end_seconds,
          captionsText: c.captions_text || '',
          captionsPosition: c.captions_position || 'bottom',
          captionStyleId: c.caption_style || 'clean',
          captionFontScale: clampCaptionFontScale(c.caption_font_scale ?? 1),
          captionYOffset: Math.max(-20, Math.min(20, c.caption_y_offset ?? 0)),
          publishTitle: c.title,
          publishDescription: '',
          publishHashtags: [],
        })
        setHistoryTick(0)

        // Fetch linked highlight for marker data
        let loadedHighlight: Highlight | undefined
        try {
          const highlights = await invoke<Array<Omit<Highlight, 'tags'> & { tags: unknown }>>('get_all_highlights')
          const rawHighlight = highlights.find(h => h.id === c.highlight_id)
          loadedHighlight = rawHighlight
            ? { ...rawHighlight, tags: parseStoredTags(rawHighlight.tags) }
            : undefined
          if (loadedHighlight) setHighlight(loadedHighlight)
        } catch { /* non-critical */ }

        if (c.source_media_path) {
          setVod(null)
          setGame(c.game || '')
          const previewPath = await invoke<string>('prepare_clip_preview_source', { clipId })
          setVideoSrc(convertFileSrc(previewPath))
        } else {
          const v = await invoke<Vod>('get_vod_detail', { vodId: c.vod_id })
          setVod(v)

          // Load game from stored values — clip.game takes priority, then VOD.game_name
          const storedGame = c.game || v.game_name || ''
          console.log('[Editor] Game loaded — clip.game:', JSON.stringify(c.game), '| vod.game_name:', JSON.stringify(v.game_name), '→', JSON.stringify(storedGame))
          setGame(storedGame)

          // Community-clip MP4 is already trimmed, so its preview is 0-based.
          if (c.community_clip_mp4_path) {
            setVideoSrc(convertFileSrc(c.community_clip_mp4_path))
          } else if (v.local_path) {
            setVideoSrc(convertFileSrc(v.local_path))
          }
        }

        invoke('record_clip_opened', { clipId }).catch(() => {})

      } catch (err) {
        console.error('Failed to load clip:', err)
        setEditorLoadError(String(err))
      }
    })()
  }, [clipId, history, editorLoadAttempt])

  // ── Sync aspect ratio when export preset changes ──
  useEffect(() => {
    setAspectRatio(exportPreset.aspectRatio)
  }, [exportPreset.aspectRatio])

  const handleSave = async () => {
    if (!clipId) return
    setSaving(true)
    try {
      await invoke('update_clip_settings', {
        clipId,
        title,
        startSeconds,
        endSeconds,
        aspectRatio,
        captionsEnabled: captionsEnabled ? 1 : 0,
        captionsText: captionsText || null,
        captionsPosition,
        captionStyle: captionStyleId,
        captionFontScale,
        captionYOffset,
        facecamLayout,
        facecamSettings: JSON.stringify(facecamSettings),
        contextBackgroundPath,
        contextBackgroundMode,
        contextBlurStrength,
        contextVideoY,
        game: game || null,
      })
      setSaved(true)
      setExportDone(false)
      setTimeout(() => setSaved(false), 2000)
    } catch (err) {
      alert(`Save failed: ${err}`)
    } finally {
      setSaving(false)
    }
  }

  const exportUnlistenRef = useRef<(() => void) | null>(null)
  useEffect(() => () => { exportUnlistenRef.current?.() }, [])

  /**
   * Export with a specific aspect ratio (for multi-platform auto-re-export).
   * Saves clip with the target aspect ratio, exports, waits for completion, then restores.
   */
  const handleExportForFormat = async (targetAspectRatio: string) => {
    if (!clipId) throw new Error('No clip')
    const originalAR = aspectRatio

    // Save with target aspect ratio
    await invoke('update_clip_settings', {
      clipId, title, startSeconds, endSeconds,
      aspectRatio: targetAspectRatio,
      captionsEnabled: captionsEnabled ? 1 : 0,
      captionsText: captionsText || null,
      captionsPosition, captionStyle: captionStyleId, captionFontScale, captionYOffset, facecamLayout,
      facecamSettings: JSON.stringify(facecamSettings),
      contextBackgroundPath, contextBackgroundMode, contextBlurStrength, contextVideoY,
      game: game || null,
    })

    // Export and wait for completion
    return new Promise<void>((resolve, reject) => {
      setExporting(true)
      setExportDone(false)
      setExportError(null)
      setExportProgress(0)

      const jobId = `export-${clipId}`
      let unlistenFn: (() => void) | null = null

      listen<{ jobId: string; progress: number; status: string; error?: string }>('job-progress', (event) => {
        if (event.payload.jobId !== jobId) return
        const { progress, status, error } = event.payload
        setExportProgress(progress)

        if (status === 'completed') {
          setExporting(false)
          setExportDone(true)
          unlistenFn?.()
          invoke<Clip>('get_clip_detail', { clipId }).then(c => setClip(c)).catch(() => {})
          // Restore original aspect ratio in state (DB was changed, but UI stays consistent)
          setAspectRatio(originalAR)
          resolve()
        } else if (status === 'failed') {
          setExporting(false)
          setExportError(error || 'Export failed')
          unlistenFn?.()
          setAspectRatio(originalAR)
          reject(new Error(error || 'Export failed'))
        }
      }).then(fn => {
        unlistenFn = fn
        invoke('export_clip', { clipId }).catch(err => {
          setExporting(false)
          unlistenFn?.()
          setAspectRatio(originalAR)
          reject(err)
        })
      })
    })
  }

  // ── Preview frame sizing (single source of truth for frame dimensions) ──
  const FRAME_WIDTHS: Record<string, number> = { '9:16': 270, '16:9': 0 } // 0 = w-full
  const frameWidth = FRAME_WIDTHS[aspectRatio] || 0

  const previewAspect = aspectRatio === '9:16' ? 'aspect-[9/16]' : 'aspect-video'

  // Tailwind needs static class names — can't use dynamic `w-[${n}px]`
  const previewWidth = aspectRatio === '9:16' ? 'w-[270px]' : 'w-full'

  // Compute actual frame pixel dimensions for facecam overlays
  const frameHeightPx = aspectRatio === '9:16' ? 480 : 249
  const frameWidthPx = frameWidth > 0 ? frameWidth : 442

  // Caption Y position -- mirrors the export pipeline (commands/export.rs).
  // For each position, captionY represents the ANCHOR of the text:
  //   top:    text top edge sits at captionY% Y
  //   center: text center sits at captionY% Y (via translateY(-50%))
  //   bottom: text bottom edge sits at captionY% Y (anchored from frame bottom)
  // This makes multi-line / large styles grow IN THE RIGHT DIRECTION instead
  // of overflowing the frame.
  const splitClampMax = 97
  const captionBaseY = captionsPosition === 'top' ? 8
    : captionsPosition === 'center' ? 50
    : 97  // text bottom edge ~3% above frame bottom (above play bar)
  const captionY = Math.max(3, Math.min(splitClampMax, captionBaseY + captionYOffset))
  const captionInSafeZone = captionY >= 5 && captionY <= 97

  // Cam region parsers (VOD JSON -> RegionNorm object)
  const parseRegion = (s: string | null | undefined): RegionNorm | null => {
    if (!s) return null
    try {
      const o = JSON.parse(s)
      if (typeof o.x === 'number' && typeof o.y === 'number' && typeof o.w === 'number' && typeof o.h === 'number') {
        return { x: o.x, y: o.y, w: o.w, h: o.h }
      }
    } catch { /* swallow */ }
    return null
  }
  const vodRegion = parseRegion(vod?.cam_region_norm ?? null)
  const clipOverride = parseRegion(clip?.cam_region_norm_override ?? null)
  // Effective region for live preview -- mirrors resolve_effective_region in
  // src-tauri/src/cam_region.rs so the editor's cam slot shows the same pixels
  // ffmpeg will render at export time.
  const effectiveRegion: RegionNorm | null =
    (allowPerClipCamOverride && clipOverride) ? clipOverride : (vodRegion ?? null)
  // Layout-aware fit-mode default. PiP slots are tall+narrow, so Fit gives
  // tiny letterboxed content; default Fill there. Split keeps Fit as default.
  // 'fit' stored from a previous Split session is overridden to Fill when
  // we're now in PiP, matching the backend resolver in commands/export.rs.
  const effectiveFitMode: 'fit' | 'fill' | 'stretch' = (() => {
    const stored = clip?.cam_fit_mode
    if (stored === 'fill' || stored === 'stretch') return stored
    if (facecamLayout === 'pip') return 'fill'
    return 'fit'
  })()
  const captionCollision = computeSubtitleCollision(captionY, facecamLayout, facecamSettings)
  const layoutSupportsBranding = facecamLayout === 'context_fit'
    || facecamLayout === 'split'
    || facecamLayout === 'pip'
  const brandingActive = layoutSupportsBranding
    && contextBackgroundMode === 'branding'
    && !!contextBackgroundPath
  const brandingMediaSrc = brandingActive && contextBackgroundPath
    ? convertFileSrc(contextBackgroundPath)
    : null
  const secondaryContentLabel = brandingActive ? 'Branding' : 'Facecam'
  // CSS positioning: 'bottom' anchors from the bottom edge so tall/multi-line
  // captions grow UPWARD instead of off the bottom of the frame.
  const captionPositionStyle: React.CSSProperties =
    captionsPosition === 'bottom'
      ? { bottom: `${Math.max(0, 100 - captionY)}%`, top: 'auto' }
      : captionsPosition === 'center'
        ? { top: `${captionY}%`, transform: 'translateY(-50%)' }
        : { top: `${captionY}%` }

  // Plain-text caption scale — uses same frame width as the preview container
  // CaptionPreview (SRT path) measures its own frame via ResizeObserver, but
  // the plain-text path needs an approximate scale computed here.
  const effectiveFrameW = frameWidth > 0 ? frameWidth : 442 // 442 is typical landscape width in the panel
  const formatMultiplier = aspectRatio === '9:16' ? 1.15 : aspectRatio === '16:9' ? 0.85 : 1.0
  const plainS = (effectiveFrameW / 1080) * formatMultiplier
  const plainFontSize = fitCaptionFontSize({
    requestedPx: captionStyle.fontSize * plainS * captionFontScale,
    frameWidth: effectiveFrameW,
    isVertical: aspectRatio === '9:16',
    text: captionsText,
    characterWidthFactor: captionStyle.characterWidthFactor,
    safeWidthRatio: Math.min(captionStyle.safeWidthRatio ?? 0.84, 0.78),
  })
  const plainShadowScale = plainS * captionFontScale
  const captionPreviewStyle: React.CSSProperties = {
    fontFamily: captionStyle.fontFamily,
    fontSize: `${plainFontSize}px`,
    fontWeight: captionStyle.fontWeight,
    color: captionStyle.fontColor,
    textTransform: captionStyle.uppercase ? 'uppercase' : 'none',
    letterSpacing: `${captionStyle.letterSpacing}em`,
    lineHeight: captionStyle.lineHeight,
    textShadow: captionStyle.shadow === 'none' ? 'none'
      : captionStyle.shadow.replace(/(\d+)px/g, (_, n) => `${Math.max(1, Math.round(parseInt(n) * plainShadowScale))}px`),
    WebkitTextStroke: captionStyle.strokeWidth > 0 && captionStyle.strokeColor
      ? `${Math.max(0.5, captionStyle.strokeWidth * plainShadowScale)}px ${captionStyle.strokeColor}`
      : undefined,
    paintOrder: 'stroke fill',
    ...(captionStyle.presentation === 'cardboard' ? {
      width: '76%',
      minHeight: `${Math.max(30, plainFontSize * 1.8)}px`,
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      backgroundColor: captionStyle.bgColor,
      backgroundImage: 'repeating-linear-gradient(0deg, rgba(82,45,20,0.11) 0 1px, transparent 1px 4px), repeating-linear-gradient(90deg, rgba(255,255,255,0.045) 0 7px, rgba(83,45,20,0.045) 7px 8px), linear-gradient(90deg, rgba(74,38,15,0.14), transparent 13%, transparent 87%, rgba(74,38,15,0.14))',
      padding: `${Math.max(5, captionStyle.bgPadding * plainS * 0.55)}px ${Math.max(10, captionStyle.bgPadding * plainS)}px`,
      clipPath: 'polygon(2% 4%, 8% 1%, 15% 3%, 24% 0%, 34% 2%, 44% 1%, 55% 3%, 66% 0%, 77% 2%, 87% 1%, 98% 4%, 100% 17%, 98% 32%, 100% 50%, 98% 69%, 100% 84%, 97% 97%, 87% 99%, 77% 97%, 66% 100%, 55% 98%, 44% 100%, 34% 97%, 23% 99%, 13% 97%, 2% 100%, 0% 83%, 2% 68%, 0% 50%, 2% 31%, 0% 16%)',
      boxShadow: 'inset 0 0 0 1px rgba(75,39,17,0.28), inset 0 0 18px rgba(80,43,20,0.22)',
      filter: 'drop-shadow(0 3px 3px rgba(0,0,0,0.55))',
    } : {
      backgroundColor: captionStyle.bgColor || undefined,
      padding: captionStyle.bgPadding > 0 ? `${captionStyle.bgPadding * plainS * 0.6}px ${captionStyle.bgPadding * plainS}px` : undefined,
      borderRadius: captionStyle.bgRadius > 0 ? `${captionStyle.bgRadius * plainS}px` : undefined,
    }),
  }

  if (!clip) {
    if (editorLoadError) {
      return (
        <div className="flex min-h-64 items-center justify-center">
          <div className="max-w-lg border-l-2 border-red-400 bg-red-500/5 px-5 py-4">
            <h2 className="text-sm font-semibold text-white">Could not open this clip</h2>
            <p className="mt-1 break-words text-xs text-red-200">{editorLoadError}</p>
            <div className="mt-4 flex gap-2">
              <button type="button" className="v4-btn primary" onClick={() => setEditorLoadAttempt(attempt => attempt + 1)}>
                Retry
              </button>
              <button type="button" className="v4-btn ghost" onClick={() => navigate('/clips')}>
                Back to clips
              </button>
            </div>
          </div>
        </div>
      )
    }
    return (
      <div className="flex h-64 items-center justify-center gap-2 text-sm text-slate-400" role="status">
        <Loader2 className="h-4 w-4 animate-spin" /> Loading editor
      </div>
    )
  }

  return (
    <div className="space-y-6">
      {editorLoadError && (
        <div role="alert" className="flex items-center justify-between gap-3 border-l-2 border-red-400 bg-red-500/5 px-3 py-2 text-xs text-red-200">
          <span className="min-w-0 break-words">Preview could not be prepared: {editorLoadError}</span>
          <button type="button" className="v4-btn ghost shrink-0" onClick={() => setEditorLoadAttempt(attempt => attempt + 1)}>
            Retry
          </button>
        </div>
      )}
      {/* Cam region edit modal -- shown on top of everything when in edit mode */}
      {regionEditScope && vod && videoSrc && (
        <CamRegionModal
          videoSrc={videoSrc}
          startTime={startSeconds}
          initial={
            regionEditScope === 'vod'
              ? (vodRegion ?? { x: 0.05, y: 0.70, w: 0.25, h: 0.25 })
              : (clipOverride ?? vodRegion ?? { x: 0.05, y: 0.70, w: 0.25, h: 0.25 })
          }
          onSave={async (r) => {
            try {
              const regionJson = JSON.stringify(r)
              if (regionEditScope === 'vod') {
                await invoke('set_vod_cam_region', { vodId: vod.id, region: r })
                // Optimistic local update -- avoids a full clip refetch that
                // would reset unsaved editor state (facecamLayout, etc.).
                setVod(v => v ? { ...v, cam_region_norm: regionJson } : v)
              } else {
                await invoke('set_clip_cam_region_override', { clipId: clip.id, region: r })
                setClip(c => c ? { ...c, cam_region_norm_override: regionJson } : c)
              }
              setRegionEditScope(null)
            } catch (e) {
              console.error('[Editor] save cam region failed', e)
            }
          }}
          onCancel={() => setRegionEditScope(null)}
        />
      )}
      {/* Header */}
      <div className="flex items-center gap-4">
        <button onClick={() => navigate('/clips')} className="p-2 rounded-lg bg-surface-800 hover:bg-surface-700 text-slate-400 hover:text-white transition-colors cursor-pointer">
          <ArrowLeft className="w-5 h-5" />
        </button>
        <div className="min-w-0 flex-1">
          <h1 className="text-2xl font-bold text-white truncate">Edit Clip</h1>
          <TwitchProvenanceBadges
            tags={highlight?.tags}
            signalSources={highlight?.signal_sources}
            className="mt-1.5"
          />
        </div>
        <div className="flex items-center gap-1.5">
          <Tooltip text="Save clip changes" position="bottom">
            <button
              onClick={handleSave}
              disabled={saving}
              className={`v4-editor-save-button ${saved ? 'saved' : ''}`}
            >
              {saving
                ? <Loader2 className="w-4 h-4 animate-spin" />
                : saved
                  ? <Check className="w-4 h-4" />
                  : <Save className="w-4 h-4" />}
              <span>{saving ? 'Saving...' : saved ? 'Saved' : 'Save'}</span>
            </button>
          </Tooltip>
          <Tooltip text={history.canUndo() ? `Undo (${history.undoCount()} step${history.undoCount() === 1 ? '' : 's'})` : 'Nothing to undo'} position="bottom">
            <button onClick={handleUndo} disabled={!history.canUndo()}
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-surface-800 border border-surface-600 text-slate-400 hover:text-white hover:bg-surface-700 hover:border-surface-500 transition-colors cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed">
              <Undo2 className="w-4 h-4" />
              <span className="text-xs font-medium">Undo</span>
            </button>
          </Tooltip>
          <Tooltip text={history.canRedo() ? `Redo (${history.redoCount()} step${history.redoCount() === 1 ? '' : 's'})` : 'Nothing to redo'} position="bottom">
            <button onClick={handleRedo} disabled={!history.canRedo()}
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-surface-800 border border-surface-600 text-slate-400 hover:text-white hover:bg-surface-700 hover:border-surface-500 transition-colors cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed">
              <Redo2 className="w-4 h-4" />
              <span className="text-xs font-medium">Redo</span>
            </button>
          </Tooltip>
        </div>
      </div>

      <div className="v4-editor-layout">
        {/* ── Left: Preview ── */}
        <div className="v4-editor-preview-column space-y-4">
          <div className="v4-panel" style={{padding: 16}}>
            <div className="flex items-center justify-between mb-3">
              <h2 className="text-sm font-semibold text-slate-300">Preview</h2>
              <span className="text-[9px] font-mono text-slate-500 bg-surface-900 px-1.5 py-0.5 rounded border border-surface-600">
                {exportPreset.resolution.w}x{exportPreset.resolution.h} {exportPreset.platform}
              </span>
            </div>
            <div className="flex justify-center">
              <div className={`relative rounded-lg overflow-hidden ${previewAspect} ${previewWidth} transition-all duration-300 ease-in-out`}>
                <ClipPlayer
                  src={videoSrc}
                  poster={thumbnailPath ? convertFileSrc(thumbnailPath) : null}
                  clipStart={startSeconds}
                  clipEnd={endSeconds}
                  fullFile={isCommunityClip}
                  mode="full"
                  controlsOverlay
                  className="h-full"
                  onTimeUpdate={setPlaybackTime}
                  onPlayChange={setIsPlaying}
                  seekRef={playerSeekRef}
                  videoElementRef={mainVideoElementRef}
                  objectFit={facecamLayout === 'context_fit' ? 'contain' : 'cover'}
                  blurBackground={facecamLayout === 'context_fit'
                    && contextBackgroundMode !== 'black'
                    && !brandingActive}
                  blackBackground={facecamLayout === 'context_fit' && contextBackgroundMode === 'black'}
                  backgroundMedia={facecamLayout === 'context_fit' ? brandingMediaSrc : null}
                  backgroundBlurStrength={contextBlurStrength}
                  objectPositionY={contextVideoY}
                  overlay={<>
                    {/* ── Layout mode overlay — interactive ── */}
                    {facecamLayout === 'split' && (
                      <>
                        <div className="absolute inset-0 pointer-events-none z-[8]">
                          <div className="absolute left-1 text-[7px] text-white/40 font-mono" style={{ top: '2%' }}>GAME</div>
                          <div className="absolute left-1 text-[7px] text-white/40 font-mono"
                            style={{ top: `${facecamSettings.splitRatio * 100 + 2}%` }}>
                            {brandingActive ? 'BRANDING' : 'FACECAM'}
                          </div>
                          {/* Secondary panel preview — branding or facecam crop */}
                          <div className="absolute left-0 right-0 bottom-0 overflow-hidden"
                            style={{ top: `${facecamSettings.splitRatio * 100}%`, background: 'rgba(60,20,100,0.25)' }}>
                            {brandingActive && brandingMediaSrc ? (
                              <img
                                src={brandingMediaSrc}
                                alt=""
                                className="h-full w-full object-cover"
                              />
                            ) : effectiveRegion && videoSrc ? (
                              <CamRegionPreview
                                sourceVideoRef={mainVideoElementRef}
                                region={effectiveRegion}
                                fitMode={effectiveFitMode}
                              />
                            ) : null}
                          </div>
                        </div>
                        <DraggableSplitDivider
                          settings={facecamSettings}
                          onChange={setFacecamSettings}
                          frameHeight={frameHeightPx}
                        />
                      </>
                    )}
                    {facecamLayout === 'pip' && (
                      <>
                        {/* PiP content sits beneath the draggable position/size overlay. */}
                        {brandingActive && brandingMediaSrc ? (
                          <div
                            className="absolute overflow-hidden pointer-events-none"
                            style={{
                              left: `${facecamSettings.pipX}%`,
                              top: `${facecamSettings.pipY}%`,
                              width: `${facecamSettings.pipW}%`,
                              height: `${facecamSettings.pipH}%`,
                              zIndex: 4,
                            }}
                          >
                            <img src={brandingMediaSrc} alt="" className="h-full w-full object-contain" />
                          </div>
                        ) : effectiveRegion && videoSrc ? (
                          <div
                            className="absolute overflow-hidden pointer-events-none"
                            style={{
                              left: `${facecamSettings.pipX}%`,
                              top: `${facecamSettings.pipY}%`,
                              width: `${facecamSettings.pipW}%`,
                              height: `${facecamSettings.pipH}%`,
                              zIndex: 4,
                            }}
                          >
                            <CamRegionPreview
                              sourceVideoRef={mainVideoElementRef}
                              region={effectiveRegion}
                              fitMode={effectiveFitMode}
                            />
                          </div>
                        ) : null}
                        <DraggablePipOverlay
                          settings={facecamSettings}
                          onChange={setFacecamSettings}
                          frameWidth={frameWidthPx}
                          frameHeight={frameHeightPx}
                          transparent={brandingActive || !!effectiveRegion}
                          contentLabel={brandingActive ? 'BRANDING' : 'FACECAM'}
                        />
                      </>
                    )}

                    {/* ── Safe zone guides (adapt to facecam layout) ── */}
                    <div className="absolute inset-0 pointer-events-none z-[7]">
                      <div className="absolute left-0 right-0 top-0 border-b border-dashed border-cyan-400/15" style={{ height: '10%' }} />
                      {facecamLayout === 'split' ? (
                        // In split: safe zone is above the split line
                        <div className="absolute left-0 right-0 border-t border-dashed border-cyan-400/15"
                          style={{ top: `${(facecamSettings.splitRatio * 100) - 5}%`, height: '5%' }} />
                      ) : (
                        <div className="absolute left-0 right-0 bottom-0 border-t border-dashed border-cyan-400/15" style={{ height: '10%' }} />
                      )}
                    </div>

                    {/* ── Format indicator on video ── */}
                    <div className="absolute top-2 left-2 pointer-events-none z-[6]">
                      <span className="text-[8px] font-mono text-white/30 bg-black/30 px-1 py-0.5 rounded">
                        {exportPreset.aspectRatio}
                      </span>
                    </div>

                    {/* ── Caption preview ── */}
                    {captionsEnabled && (
                      hasSrtCaptions ? (
                        <CaptionPreview
                          segments={subtitleSegments}
                          emphasisTokens={captionTokens}
                          captionStyle={captionStyle}
                          fontScale={captionFontScale}
                          currentTime={srtTime}
                          trimStart={srtTrimStart}
                          trimEnd={srtTrimEnd}
                          position={captionsPosition as 'top' | 'center' | 'bottom'}
                          yPercent={captionY}
                          emphasisEnabled={aiEmphasisEnabled}
                          outputWidth={exportPreset.resolution.w}
                        />
                      ) : captionsText ? (
                        <div className="absolute left-0 right-0 flex justify-center pointer-events-none z-10"
                          style={captionPositionStyle}>
                          <span className="text-center max-w-[80%]" style={{
                            ...captionPreviewStyle,
                            wordBreak: 'break-word',
                            overflowWrap: 'anywhere',
                            whiteSpace: 'normal',
                            display: 'inline-block',
                          }}>
                            {captionsText}
                          </span>
                        </div>
                      ) : (
                        <div className="absolute bottom-[8%] left-0 right-0 flex justify-center pointer-events-none z-10">
                          <span className="text-center text-xs text-slate-400/80 bg-black/50 px-3 py-1.5 rounded">
                            {generatingCaptions ? 'Generating subtitles...' : 'No subtitles yet'}
                          </span>
                        </div>
                      )
                    )}

                    {/* ── Draggable caption position handle ── */}
                    {captionsEnabled && (captionsText || hasSrtCaptions) && (
                      <div
                        className="absolute left-0 right-0 z-[12] flex justify-center cursor-ns-resize group/drag"
                        style={{ top: `${captionY}%`, height: 20, marginTop: -10 }}
                        onMouseDown={e => {
                          e.preventDefault()
                          e.stopPropagation()
                          const container = (e.currentTarget.parentElement as HTMLElement)
                          const onMove = (ev: MouseEvent) => {
                            const rect = container.getBoundingClientRect()
                            const pct = ((ev.clientY - rect.top) / rect.height) * 100
                            setCaptionYOffset(Math.round(Math.max(3, Math.min(95, pct)) - captionBaseY))
                          }
                          const onUp = () => {
                            window.removeEventListener('mousemove', onMove)
                            window.removeEventListener('mouseup', onUp)
                          }
                          window.addEventListener('mousemove', onMove)
                          window.addEventListener('mouseup', onUp)
                        }}
                      >
                        {/* Drag indicator line */}
                        <div className="w-8 h-1 bg-white/20 rounded-full group-hover/drag:bg-white/50 transition-colors" />
                      </div>
                    )}

                    {/* ── Text overlay previews ── */}
                    {textOverlays.map(o => (
                      <div key={o.id} className={`absolute left-0 right-0 flex justify-center pointer-events-none z-10 ${
                        o.position === 'top' ? 'top-[5%]' : o.position === 'center' ? 'top-1/2 -translate-y-1/2' : 'bottom-[5%]'
                      }`}>
                        <span className="px-2 py-1 text-center" style={{
                          fontSize: `${o.fontSize * 0.35}px`, color: o.color,
                          fontWeight: o.style === 'title' ? 800 : 600,
                          textShadow: '2px 2px 6px rgba(0,0,0,0.9)',
                        }}>{o.text}</span>
                      </div>
                    ))}
                  </>}
                />
              </div>
            </div>
          </div>

        </div>

        {/* ── Right: Settings ── */}
        <aside className="v4-editor-tools">
          <nav className="v4-editor-tabs" role="tablist" aria-label="Clip editor workspaces">
            {([
              { id: 'edit' as const, label: 'Edit', Icon: Film },
              { id: 'captions' as const, label: 'Captions', Icon: MessageSquare },
              { id: 'publish' as const, label: 'Publish', Icon: Upload },
            ]).map(({ id, label, Icon }) => {
              const isActive = editorWorkspace === id
              return (
                <button
                  key={id}
                  ref={node => { editorWorkspaceTabRefs.current[id] = node }}
                  id={`editor-tab-${id}`}
                  type="button"
                  role="tab"
                  tabIndex={isActive ? 0 : -1}
                  aria-selected={isActive}
                  aria-controls={`editor-panel-${id}`}
                  className={`v4-editor-tab ${isActive ? 'active' : ''}`}
                  onClick={() => {
                    setEditorWorkspace(id)
                    if (id !== 'edit') {
                      setTemplateDropdownOpen(false)
                      setLayoutPickerOpen(false)
                    }
                  }}
                  onKeyDown={event => handleEditorWorkspaceKeyDown(event, id)}
                >
                  <Icon className="w-4 h-4" aria-hidden="true" />
                  <span>{label}</span>
                </button>
              )
            })}
          </nav>

          <div className="v4-editor-workspace-scroll">
            <div
              id="editor-panel-edit"
              role="tabpanel"
              aria-labelledby="editor-tab-edit"
              className="v4-editor-workspace"
              hidden={editorWorkspace !== 'edit'}
            >
          {/* Title */}
          <Section title="Title">
            <div className="flex gap-1.5">
              <input type="text" value={title} onChange={e => setTitle(e.target.value)}
                onBlur={() => {
                  if (clipId) {
                    invoke('set_clip_title', { clipId, title: title || null }).catch(err =>
                      console.warn('[Editor] Failed to save title on blur:', err)
                    )
                  }
                }}
                className="flex-1 px-3 py-2 bg-surface-900 border border-surface-600 rounded-lg text-white text-sm focus:outline-none focus:border-violet-500" />
              <Tooltip text={game ? `Generate new title with ${game} context` : 'Generate new title (set a game for game-specific titles)'} position="left">
                <button
                  onClick={async () => {
                    const ctx: ClipContext = {
                      title: '', // don't seed from current title — generate fresh
                      eventTags: highlight?.tags ?? [],
                      emotionTags: [],
                      transcriptExcerpt: highlight?.transcript_snippet || undefined,
                      eventSummary: highlight?.event_summary || undefined,
                      transcript: trimmedTranscriptText,
                      vodTitle: vod?.title || undefined,
                      game: game || undefined,
                      duration: clipDuration,
                    }

                    let newTitle: string | null = null

                    // Try AI title generation first (uses BYOK provider if configured)
                    if (clipId) {
                      try {
                        const transcriptText = trimmedTranscriptText || null
                        const aiTitle = await invoke<string>('generate_ai_title', {
                          clipId,
                          transcriptText,
                          currentGame: game || null,
                          currentTitle: title || null,
                        })
                        // If AI returned something different from the current title, use it
                        if (aiTitle && aiTitle !== title) {
                          newTitle = aiTitle
                        }
                      } catch (err) {
                        console.warn('[Editor] AI title generation failed, using local patterns:', err)
                      }
                    }

                    // Fallback to local pattern generator
                    if (!newTitle) {
                      newTitle = generateStandaloneTitle(ctx)
                    }

                    setTitle(newTitle)
                    if (clipId) {
                      invoke('set_clip_title', { clipId, title: newTitle }).catch(err =>
                        console.warn('[Editor] Failed to save regenerated title:', err)
                      )
                    }
                  }}
                  className="px-2 py-2 bg-surface-900 border border-surface-600 rounded-lg text-slate-400 hover:text-violet-400 hover:border-violet-500/40 transition-colors cursor-pointer"
                >
                  <RefreshCw className="w-3.5 h-3.5" />
                </button>
              </Tooltip>
            </div>
          </Section>

          {/* Timing */}
          <Section title="Hook Trim">
            {isCommunityClip ? (
              /* Community-clip MP4: the file is already the full, trimmed clip
                 (0-based, standalone). There is no VOD sub-range to trim, so the
                 VOD-relative timeline/handles don't apply here. */
              <p className="text-xs text-slate-400">
                This is an imported Twitch clip — it plays as a standalone, already-trimmed file, so there's no VOD trim window to adjust.
              </p>
            ) : (
              <>
                <TrimTimeline
                  startTime={startSeconds}
                  endTime={endSeconds}
                  originalStart={originalStart}
                  originalEnd={originalEnd}
                  videoDuration={vod?.duration_seconds || endSeconds + 30}
                  currentTime={playbackTime}
                  isPlaying={isPlaying}
                  markers={timelineMarkers}
                  suggestedHookStart={suggestedHookStart}
                  onChange={(s, e) => { setStartSeconds(s); setEndSeconds(e) }}
                  onSeekTo={(t) => playerSeekRef.current?.(t)}
                />

                {/* Precise time inputs (collapsed, for fine-tuning) */}
                <details className="mt-3">
                  <summary className="text-[10px] text-slate-500 cursor-pointer hover:text-slate-400">Fine-tune seconds</summary>
                  <div className="grid grid-cols-2 gap-2 mt-2">
                    <div>
                      <label className="block text-[10px] text-slate-500 mb-0.5">Start</label>
                      <input type="number" value={startSeconds} onChange={e => setStartSeconds(parseFloat(e.target.value) || 0)}
                        step="0.1" min="0"
                        className="w-full px-2 py-1 bg-surface-900 border border-surface-600 rounded text-white text-xs focus:outline-none focus:border-violet-500 font-mono" />
                    </div>
                    <div>
                      <label className="block text-[10px] text-slate-500 mb-0.5">End</label>
                      <input type="number" value={endSeconds} onChange={e => setEndSeconds(parseFloat(e.target.value) || 0)}
                        step="0.1" min="0"
                        className="w-full px-2 py-1 bg-surface-900 border border-surface-600 rounded text-white text-xs focus:outline-none focus:border-violet-500 font-mono" />
                    </div>
                  </div>
                </details>
              </>
            )}
          </Section>

          {/* Export Preset */}
          <Section title="Export For">
            <div className="grid grid-cols-2 gap-2">
              {EXPORT_PRESETS.filter(p => !p.hidden).map(p => (
                <Tooltip key={p.id} text={`Export in ${p.name} format (${p.aspectRatio})`} position="bottom">
                  <button onClick={() => setExportPresetId(p.id)}
                    className={`w-full px-3 py-2.5 rounded-lg text-sm font-medium transition-colors cursor-pointer border ${
                      exportPresetId === p.id
                        ? 'bg-violet-600/20 text-violet-400 border-violet-500/40'
                        : 'bg-surface-900 text-slate-400 border-surface-600 hover:bg-surface-700'
                    }`}>
                    <div>{p.name}</div>
                    <div className="text-xs opacity-60">{p.description}</div>
                  </button>
                </Tooltip>
              ))}
            </div>
          </Section>

          {/* Templates */}
          <Section title="Templates">
            <div className="flex gap-2">
              {/* Load Template dropdown */}
              <div className="relative flex-1" ref={templateDropdownRef}>
                <button
                  onClick={() => setTemplateDropdownOpen(!templateDropdownOpen)}
                  className="w-full flex items-center justify-between gap-2 px-3 py-2 rounded-lg bg-surface-900 border border-surface-600 text-sm text-slate-400 hover:text-white hover:border-surface-500 transition-colors cursor-pointer"
                >
                  <span className="flex items-center gap-1.5">
                    <Bookmark className="w-3.5 h-3.5" />
                    Load Template
                  </span>
                  <ChevronDown className={`w-3.5 h-3.5 transition-transform ${templateDropdownOpen ? 'rotate-180' : ''}`} />
                </button>
                {templateDropdownOpen && (
                  <div className="absolute z-40 top-full left-0 right-0 mt-1 bg-surface-800 border border-surface-600 rounded-lg shadow-xl overflow-hidden max-h-64 overflow-y-auto">
                    {templateStore.templates.length === 0 ? (
                      <p className="px-3 py-2 text-xs text-slate-500 italic">No templates yet</p>
                    ) : (
                      <>
                        {/* Built-in templates */}
                        {templateStore.templates.filter(t => t.builtIn).length > 0 && (
                          <div className="px-3 pt-2 pb-1">
                            <p className="text-[9px] text-slate-500 uppercase tracking-wider font-semibold">Starter Templates</p>
                          </div>
                        )}
                        {templateStore.templates.filter(t => t.builtIn).map(tmpl => {
                          const presetMatch = EXPORT_PRESETS.find(p => p.id === tmpl.exportPresetId)
                          const styleMatch = CAPTION_STYLES.find(s => s.id === tmpl.captionStyleId)
                          return (
                            <button key={tmpl.id} onClick={() => handleLoadTemplate(tmpl)}
                              className="w-full text-left px-3 py-2 hover:bg-surface-700 transition-colors cursor-pointer">
                              <div className="text-sm text-white">{tmpl.name}</div>
                              <div className="text-[10px] text-slate-500">
                                {styleMatch?.name || tmpl.captionStyleId} &middot; {presetMatch?.aspectRatio || ''} &middot; {tmpl.hashtags.slice(0, 3).map(h => `#${h}`).join(' ')}
                              </div>
                            </button>
                          )
                        })}
                        {/* Custom templates */}
                        {templateStore.templates.filter(t => !t.builtIn).length > 0 && (
                          <div className="px-3 pt-2 pb-1 border-t border-surface-600">
                            <p className="text-[9px] text-slate-500 uppercase tracking-wider font-semibold">My Templates</p>
                          </div>
                        )}
                        {templateStore.templates.filter(t => !t.builtIn).map(tmpl => {
                          const presetMatch = EXPORT_PRESETS.find(p => p.id === tmpl.exportPresetId)
                          const styleMatch = CAPTION_STYLES.find(s => s.id === tmpl.captionStyleId)
                          return (
                            <button key={tmpl.id} onClick={() => handleLoadTemplate(tmpl)}
                              className="w-full text-left px-3 py-2 hover:bg-surface-700 transition-colors cursor-pointer">
                              <div className="text-sm text-white">{tmpl.name}</div>
                              <div className="text-[10px] text-slate-500">
                                {styleMatch?.name || tmpl.captionStyleId} &middot; {presetMatch?.aspectRatio || ''} &middot; {tmpl.hashtags.slice(0, 3).map(h => `#${h}`).join(' ')}
                              </div>
                            </button>
                          )
                        })}
                      </>
                    )}
                  </div>
                )}
              </div>

              {/* Save as Template */}
              {!templateSaveOpen ? (
                <Tooltip text="Save current settings as a reusable template" position="bottom">
                  <button onClick={() => { setTemplateSaveOpen(true); setTemplateSaveName('') }}
                    className={`flex items-center gap-1.5 px-3 py-2 rounded-lg border text-sm font-medium transition-colors cursor-pointer ${
                      templateSaved
                        ? 'bg-emerald-600/20 text-emerald-400 border-emerald-500/40'
                        : 'bg-surface-900 border-surface-600 text-slate-400 hover:text-white hover:border-surface-500'
                    }`}>
                    {templateSaved ? <Check className="w-3.5 h-3.5" /> : <Plus className="w-3.5 h-3.5" />}
                    {templateSaved ? 'Saved!' : 'Save'}
                  </button>
                </Tooltip>
              ) : (
                <div className="flex items-center gap-1.5">
                  <input
                    type="text"
                    value={templateSaveName}
                    onChange={e => setTemplateSaveName(e.target.value)}
                    onKeyDown={e => { if (e.key === 'Enter') handleSaveTemplate(); if (e.key === 'Escape') setTemplateSaveOpen(false) }}
                    placeholder="Template name..."
                    autoFocus
                    className="w-32 px-2 py-1.5 bg-surface-900 border border-surface-600 rounded-lg text-sm text-white placeholder:text-slate-500 focus:outline-none focus:border-violet-500"
                  />
                  <button onClick={handleSaveTemplate} disabled={!templateSaveName.trim()}
                    className="p-1.5 rounded-lg bg-violet-600 hover:bg-violet-500 text-white disabled:opacity-30 transition-colors cursor-pointer">
                    <Check className="w-3.5 h-3.5" />
                  </button>
                  <button onClick={() => setTemplateSaveOpen(false)}
                    className="p-1.5 rounded-lg bg-surface-900 border border-surface-600 text-slate-400 hover:text-white transition-colors cursor-pointer">
                    <X className="w-3.5 h-3.5" />
                  </button>
                </div>
              )}
            </div>
          </Section>

          {/* Game / Category */}
          <Section title="Game">
            <Tooltip text="Enter the game name to improve hashtags, titles, and captions for this clip.">
              <input
                type="text"
                value={game}
                onChange={e => setGame(e.target.value)}
                onBlur={() => {
                  // Persist game to clip DB on blur so it survives navigation
                  if (clipId) {
                    invoke('set_clip_game', { clipId, game: game || null }).catch(err =>
                      console.warn('[Editor] Failed to save game on blur:', err)
                    )
                  }
                }}
                placeholder="e.g. Dead by Daylight, Valorant"
                className="w-full px-3 py-2 rounded-lg bg-surface-900 border border-surface-600 text-sm text-white placeholder:text-slate-500 focus:outline-none focus:border-violet-500 transition-colors"
              />
            </Tooltip>
            <p className="text-[10px] text-slate-500 mt-1">Used for hashtags, titles, and captions. Set once per VOD on the VODs page to apply to all clips.</p>
          </Section>

          {/* Layout */}
          <Section title="Layout">
            {/* Current layout display + change button */}
            <div className="flex items-center gap-3">
              {/* Mini diagram of current layout */}
              <div className={`rounded border border-surface-600 bg-surface-900 relative overflow-hidden shrink-0 ${
                aspectRatio === '9:16' ? 'w-8 aspect-[9/16]' : 'w-14 aspect-video'
              }`}>
                {LAYOUT_OPTIONS.find(l => l.id === facecamLayout)?.regions.map((r, i) => (
                  <div key={i} className="absolute flex items-center justify-center"
                    style={{ left: `${r.x}%`, top: `${r.y}%`, width: `${r.w}%`, height: `${r.h}%`, background: r.fill, borderRadius: r.w < 50 ? '2px' : undefined }}>
                    <span className="text-[4px] font-mono text-white/40">{r.label}</span>
                  </div>
                ))}
              </div>
              <div className="flex-1 min-w-0">
                <p className="text-xs text-white font-medium">{LAYOUT_OPTIONS.find(l => l.id === facecamLayout)?.name || 'Full Frame'}</p>
                <p className="text-[10px] text-slate-500 truncate">{LAYOUT_OPTIONS.find(l => l.id === facecamLayout)?.description}</p>
              </div>
              <button onClick={() => setLayoutPickerOpen(true)}
                className="shrink-0 px-3 py-1.5 bg-surface-900 border border-surface-600 rounded-lg text-xs text-slate-300 hover:text-white hover:border-violet-500/40 transition-colors cursor-pointer">
                Change
              </button>
            </div>
            {/* Layout picker modal */}
            {layoutPickerOpen && (
              <LayoutPicker
                current={facecamLayout}
                aspectRatio={aspectRatio as '9:16' | '16:9'}
                platformName={exportPreset.platform}
                onSelect={(layout) => { setFacecamLayout(layout); setLayoutPickerOpen(false) }}
                onClose={() => setLayoutPickerOpen(false)}
              />
            )}
            {/* Facecam settings — shown when layout uses camera */}
            {(facecamLayout === 'split' || facecamLayout === 'pip') && (
              <FacecamEditor
                layout={facecamLayout}
                settings={facecamSettings}
                onChange={setFacecamSettings}
                contentLabel={secondaryContentLabel}
              />
            )}
            {/* Cam region (crop from source) — visible whenever layout has a cam slot */}
            {clip && vod && !brandingActive && (facecamLayout === 'split' || facecamLayout === 'pip') && (
              <div className="mt-3">
                <CamRegionRow
                  vodId={vod.id}
                  clipId={clip.id}
                  vodRegion={vodRegion}
                  clipOverride={clipOverride}
                  fitMode={(clip.cam_fit_mode ?? null) as 'fit' | 'fill' | 'stretch' | null}
                  layoutHasCamSlot={facecamLayout === 'split' || facecamLayout === 'pip'}
                  layoutKind={facecamLayout === 'pip' ? 'pip' : facecamLayout === 'split' ? 'split' : 'other'}
                  onEnterVodEditMode={() => setRegionEditScope('vod')}
                  onEnterClipOverrideMode={() => setRegionEditScope('clip-override')}
                  onChanged={refetchCamRegions}
                />
              </div>
            )}
            {layoutSupportsBranding && (
              <div className="mt-4 border-t border-surface-600/70 pt-4 space-y-4">
                <div>
                  <div className="flex items-center justify-between mb-2">
                    <span className="text-[10px] font-medium uppercase text-slate-500">
                      {facecamLayout === 'context_fit'
                        ? 'Background'
                        : facecamLayout === 'split'
                          ? 'Secondary panel'
                          : 'Floating panel'}
                    </span>
                    <span className="text-[10px] text-slate-500">
                      {contextBackgroundMode === 'branding'
                        ? 'Branding'
                        : facecamLayout === 'context_fit'
                          ? contextBackgroundMode === 'black' ? 'Black bars' : 'Video blur'
                          : 'Facecam'}
                    </span>
                  </div>
                  <div className={`grid gap-1 rounded-md bg-surface-900 p-1 border border-surface-600 ${
                    facecamLayout === 'context_fit' ? 'grid-cols-3' : 'grid-cols-2'
                  }`}>
                    <button
                      type="button"
                      aria-pressed={facecamLayout === 'context_fit'
                        ? contextBackgroundMode === 'blur'
                        : contextBackgroundMode !== 'branding'}
                      onClick={() => setContextBackgroundMode('blur')}
                      className={`h-8 rounded text-xs font-medium transition-colors cursor-pointer ${
                        (facecamLayout === 'context_fit'
                          ? contextBackgroundMode === 'blur'
                          : contextBackgroundMode !== 'branding')
                          ? 'bg-cyan-500/15 text-cyan-300'
                          : 'text-slate-500 hover:text-slate-300'
                      }`}
                    >
                      {facecamLayout === 'context_fit' ? 'Video blur' : 'Facecam'}
                    </button>
                    {facecamLayout === 'context_fit' && (
                      <button
                        type="button"
                        aria-pressed={contextBackgroundMode === 'black'}
                        onClick={() => setContextBackgroundMode('black')}
                        className={`h-8 rounded text-xs font-medium transition-colors cursor-pointer ${
                          contextBackgroundMode === 'black'
                            ? 'bg-cyan-500/15 text-cyan-300'
                            : 'text-slate-500 hover:text-slate-300'
                        }`}
                      >
                        Black bars
                      </button>
                    )}
                    <button
                      type="button"
                      aria-pressed={contextBackgroundMode === 'branding'}
                      onClick={() => {
                        if (contextBackgroundPath) setContextBackgroundMode('branding')
                        else void handlePickContextBranding()
                      }}
                      className={`h-8 rounded text-xs font-medium transition-colors cursor-pointer ${
                        contextBackgroundMode === 'branding'
                          ? 'bg-cyan-500/15 text-cyan-300'
                          : 'text-slate-500 hover:text-slate-300'
                      }`}
                    >
                      Branding
                    </button>
                  </div>
                  {brandingError && <p className="mt-1.5 text-[10px] text-red-400">{brandingError}</p>}
                </div>

                {facecamLayout === 'context_fit' && contextBackgroundMode === 'blur' && (
                  <label className="block">
                    <span className="flex items-center justify-between text-[10px] text-slate-500 mb-1.5">
                      <span>Background softness</span>
                      <span>{Math.round(contextBlurStrength * 100)}%</span>
                    </span>
                    <input
                      type="range"
                      min="0"
                      max="100"
                      step="1"
                      value={Math.round(contextBlurStrength * 100)}
                      onChange={(event) => setContextBlurStrength(
                        normalizeContextBlurStrength(Number(event.target.value) / 100),
                      )}
                      className="w-full accent-cyan-400 cursor-pointer"
                    />
                  </label>
                )}

                {contextBackgroundMode === 'branding' && (
                  <div>
                    {contextBackgroundPath ? (
                      <div className="flex items-center gap-3">
                        <img
                          src={convertFileSrc(contextBackgroundPath)}
                          alt=""
                          className="h-14 w-10 shrink-0 rounded object-cover border border-surface-600"
                        />
                        <div className="min-w-0 flex-1">
                          <p className="truncate text-xs text-slate-300">{brandingAssetName(contextBackgroundPath)}</p>
                          <p className="text-[9px] text-slate-600">PNG, JPG, WebP, or animated GIF</p>
                        </div>
                        <button
                          type="button"
                          title="Replace branding"
                          aria-label="Replace branding"
                          disabled={pickingBranding}
                          onClick={() => void handlePickContextBranding()}
                          className="flex h-8 w-8 shrink-0 items-center justify-center rounded border border-surface-600 text-slate-400 hover:border-cyan-500/40 hover:text-cyan-300 disabled:opacity-50 cursor-pointer"
                        >
                          {pickingBranding
                            ? <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            : <ImagePlus className="h-3.5 w-3.5" />}
                        </button>
                        <button
                          type="button"
                          title="Remove branding"
                          aria-label="Remove branding"
                          onClick={() => {
                            setContextBackgroundPath(null)
                            setContextBackgroundMode('blur')
                          }}
                          className="flex h-8 w-8 shrink-0 items-center justify-center rounded border border-surface-600 text-slate-500 hover:border-red-500/40 hover:text-red-400 cursor-pointer"
                        >
                          <X className="h-3.5 w-3.5" />
                        </button>
                      </div>
                    ) : (
                      <button
                        type="button"
                        disabled={pickingBranding}
                        onClick={() => void handlePickContextBranding()}
                        className="flex h-9 w-full items-center justify-center gap-2 rounded border border-dashed border-surface-500 text-xs text-slate-400 hover:border-cyan-500/50 hover:text-cyan-300 disabled:opacity-50 cursor-pointer"
                      >
                        {pickingBranding
                          ? <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          : <ImagePlus className="h-3.5 w-3.5" />}
                        Choose branding
                      </button>
                    )}
                  </div>
                )}

                {facecamLayout === 'context_fit' && (
                  <label className="block">
                  <span className="flex items-center justify-between text-[10px] text-slate-500 mb-1.5">
                    <span>Video placement</span>
                    <span>{contextVideoPositionLabel(contextVideoY)}</span>
                  </span>
                  <input
                    type="range"
                    min="0"
                    max="100"
                    step="1"
                    value={Math.round(contextVideoY * 100)}
                    onChange={(event) => setContextVideoY(
                      normalizeContextVideoY(Number(event.target.value) / 100),
                    )}
                    className="w-full accent-cyan-400 cursor-pointer"
                  />
                  <div className="mt-1 flex justify-between text-[9px] text-slate-600">
                    <span>Top</span>
                    <span>Bottom</span>
                  </div>
                  </label>
                )}
              </div>
            )}
            {facecamLayout === 'none' && (
              <p className="text-[9px] text-slate-600 mt-2">
                Full Frame fills the canvas with a center crop. Choose Context Fit to preserve the whole scene.
              </p>
            )}
          </Section>
            </div>

            <div
              id="editor-panel-captions"
              role="tabpanel"
              aria-labelledby="editor-tab-captions"
              className="v4-editor-workspace"
              hidden={editorWorkspace !== 'captions'}
            >
              {/* Captions */}
              <Section title="Captions">
            <div className="flex items-center justify-between mb-3">
              <span className="text-xs text-slate-400">Subtitles</span>
              <label className="flex items-center gap-2 cursor-pointer">
                <input type="checkbox" checked={captionsEnabled} onChange={e => setCaptionsEnabled(e.target.checked)}
                  className="rounded border-surface-600 bg-surface-900 text-violet-500 focus:ring-violet-500" />
                <span className="text-xs text-slate-400">{captionsEnabled ? 'On' : 'Off'}</span>
              </label>
            </div>
            {captionsEnabled && (
              <div className="space-y-3">
                {/* Caption text */}
                <div>
                  {hasSrtCaptions ? (
                    /* SRT segments — full subtitle editor */
                    <>
                      <div className="flex items-center justify-between mb-1">
                        <label className="text-xs text-slate-400">Subtitle Segments ({subtitleSegments.length})</label>
                        <div className="flex items-center gap-1">
                          <Tooltip text="Regenerate subtitles from the clip audio" position="left">
                            <button
                              type="button"
                              onClick={handleGenerateCaptions}
                              disabled={generatingCaptions}
                              aria-label="Regenerate subtitles"
                              className="inline-flex h-7 w-7 items-center justify-center rounded-md border border-surface-600 text-slate-400 transition-colors hover:border-violet-500/60 hover:text-violet-300 disabled:cursor-wait disabled:opacity-50"
                            >
                              <RefreshCw className={`h-3.5 w-3.5 ${generatingCaptions ? 'animate-spin' : ''}`} />
                            </button>
                          </Tooltip>
                        </div>
                      </div>
                      <div className="mb-2 flex items-center justify-between gap-2 border-y border-surface-600/70 py-1.5">
                        <span className="text-[10px] font-medium text-slate-400">Timing</span>
                        <div className="flex items-center gap-1">
                          <Tooltip text="Move every subtitle 0.1 seconds earlier" position="top">
                            <button
                              type="button"
                              onClick={() => handleSubtitleShift(-0.1)}
                              aria-label="Move subtitles earlier"
                              className="inline-flex h-7 items-center gap-1 rounded-md border border-surface-600 px-2 text-[10px] text-slate-300 transition-colors hover:border-cyan-500/60 hover:text-cyan-300"
                            >
                              <ChevronLeft className="h-3 w-3" />
                              Earlier
                            </button>
                          </Tooltip>
                          <span className="px-1 text-[9px] tabular-nums text-slate-600">0.1s</span>
                          <Tooltip text="Move every subtitle 0.1 seconds later" position="top">
                            <button
                              type="button"
                              onClick={() => handleSubtitleShift(0.1)}
                              aria-label="Move subtitles later"
                              className="inline-flex h-7 items-center gap-1 rounded-md border border-surface-600 px-2 text-[10px] text-slate-300 transition-colors hover:border-cyan-500/60 hover:text-cyan-300"
                            >
                              Later
                              <ChevronRight className="h-3 w-3" />
                            </button>
                          </Tooltip>
                        </div>
                      </div>
                      <SubtitleEditor
                        segments={subtitleSegments}
                        activeId={activeSubtitle?.id || null}
                        currentTime={srtTime}
                        trimStart={srtTrimStart}
                        trimEnd={srtTrimEnd}
                        onEdit={handleSubtitleEdit}
                        onDelete={handleSubtitleDelete}
                        onSeek={handleSubtitleSeek}
                      />
                      {captionError && (
                        <p className="mt-2 text-[10px] text-red-400">{captionError}</p>
                      )}
                    </>
                  ) : (
                    /* No SRT — manual text input + empty state info */
                    <>
                      <label className="block text-xs text-slate-400 mb-1">Caption Text</label>
                      <input type="text" value={captionsText} onChange={e => setCaptionsText(e.target.value)}
                        placeholder="Type a caption to display..."
                        className="w-full px-3 py-2 bg-surface-900 border border-surface-600 rounded-lg text-white text-sm focus:outline-none focus:border-violet-500 placeholder-slate-500" />
                      {!captionsText && (
                        <div className="mt-2 p-2.5 bg-surface-900 border border-surface-600 rounded-lg space-y-2">
                          {canGenerateSubtitles ? (
                            <>
                              <button
                                onClick={handleGenerateCaptions}
                                disabled={generatingCaptions}
                                className="w-full flex items-center justify-center gap-2 px-3 py-2 bg-violet-600/20 border border-violet-500/40 rounded-lg text-xs text-violet-400 hover:bg-violet-600/30 transition-colors cursor-pointer disabled:opacity-50"
                              >
                                {generatingCaptions ? (
                                  <><Loader2 className="w-3.5 h-3.5 animate-spin" /> Generating subtitles...</>
                                ) : (
                                  <><MessageSquare className="w-3.5 h-3.5" /> Generate Subtitles (Speech-to-Text)</>
                                )}
                              </button>
                              {generatingCaptions && (
                                <p className="text-[10px] text-slate-500">
                                  Running speech-to-text analysis. This may take a minute...
                                </p>
                              )}
                              {captionError && (
                                <p className="text-[10px] text-red-400">{captionError}</p>
                              )}
                              <p className="text-[10px] text-slate-600">
                                Uses ClipGoblin's local Whisper model. Imported source videos are not changed.
                              </p>
                            </>
                          ) : (
                            <p className="text-[10px] text-slate-500 leading-relaxed">
                              Source video unavailable. Download the Twitch VOD or restore the imported video to generate timed subtitles.
                            </p>
                          )}
                        </div>
                      )}
                    </>
                  )}
                </div>

                {/* Position */}
                <div>
                  <label className="block text-xs text-slate-400 mb-1">Position</label>
                  <PillGroup value={captionsPosition} onChange={v => { setCaptionsPosition(v); setCaptionYOffset(0) }}
                    options={[
                      { value: 'top', label: 'Top', tooltip: 'Position subtitles at the top of the frame' },
                      { value: 'center', label: 'Center', tooltip: 'Position subtitles at the center of the frame' },
                      { value: 'bottom', label: 'Bottom', tooltip: 'Position subtitles at the bottom of the frame' },
                    ]} />

                  {/* Fine offset slider */}
                  <div className="mt-2 flex items-center gap-2">
                    <span className="text-[9px] text-slate-500 w-12 shrink-0">Offset</span>
                    <input type="range" min={-20} max={20} step={1} value={captionYOffset}
                      onChange={e => setCaptionYOffset(parseInt(e.target.value))}
                      className="flex-1 h-1 accent-violet-500 cursor-pointer" />
                    <span className="text-[9px] font-mono text-slate-500 w-8 text-right">
                      {captionYOffset > 0 ? `+${captionYOffset}` : captionYOffset}%
                    </span>
                  </div>

                  {/* Safe zone warning */}
                  {!captionInSafeZone && (
                    <p className="text-[9px] text-amber-400 mt-1">
                      Captions are outside the safe zone — may be clipped on some devices.
                    </p>
                  )}
                  {captionCollision.collides && (
                    <p className="text-[9px] text-amber-400 mt-1">
                      Captions overlap the facecam — <button
                        onClick={() => setCaptionYOffset(Math.round(captionCollision.safeY - captionBaseY))}
                        className="underline cursor-pointer hover:text-amber-300">move to safe position</button>
                    </p>
                  )}

                  <p className="text-[9px] text-slate-600 mt-1">
                    Drag the caption on the preview to reposition, or use the slider above.
                  </p>
                </div>

                {/* Style preset */}
                <div>
                  <label className="block text-xs text-slate-400 mb-1">Style</label>
                  <div className="grid grid-cols-3 gap-2">
                    {CAPTION_STYLES.map(s => {
                      const isActive = captionStyleId === s.id
                      // Per-style button backgrounds and border accents
                      const styleHints: Record<string, { bg: string; border: string; textShadow: string }> = {
                        clean:       { bg: 'bg-surface-900', border: 'border-slate-500/50', textShadow: '0 1px 4px rgba(0,0,0,0.9), 0 0 2px rgba(0,0,0,0.8)' },
                        'bold-white': { bg: 'bg-surface-900', border: 'border-amber-700/50', textShadow: 'none' },
                        boxed:       { bg: 'bg-surface-900', border: 'border-pink-400/50', textShadow: '1px 1px 0 #f05bd8, 2px 2px 0 #6d28d9' },
                        neon:        { bg: 'bg-surface-900', border: 'border-emerald-500/40', textShadow: '0 0 6px #00ff8880, 0 0 2px #000' },
                        minimal:     { bg: 'bg-surface-900', border: 'border-red-500/50', textShadow: '0 1px 0 #7a0000, 0 2px 3px #000' },
                        fire:        { bg: 'bg-surface-900', border: 'border-yellow-400/40', textShadow: '1px 0 0 #000, -1px 0 0 #000, 0 1px 0 #000, 0 -1px 0 #000' },
                        'comic-pop': { bg: 'bg-surface-900', border: 'border-cyan-400/50', textShadow: '1px 1px 0 #f05bd8, 2px 2px 0 #55206f' },
                      }
                      const hint = styleHints[s.id] || styleHints.clean
                      return (
                        <Tooltip key={s.id} text={`Apply ${s.name} subtitle style`} position="bottom">
                          <button onClick={() => setCaptionStyleId(s.id)}
                            className={`w-full px-2 py-2.5 rounded-lg text-xs font-medium transition-all cursor-pointer border ${
                              isActive
                                ? 'ring-1 ring-violet-500/60 border-violet-500/50 bg-violet-950/40'
                                : `${hint.bg} ${hint.border} hover:bg-surface-800`
                            }`}
                            style={s.presentation === 'cardboard' ? {
                              backgroundColor: isActive ? undefined : '#B97E43',
                              backgroundImage: isActive ? undefined : 'repeating-linear-gradient(0deg, rgba(82,45,20,0.12) 0 1px, transparent 1px 4px), repeating-linear-gradient(90deg, rgba(255,255,255,0.05) 0 7px, rgba(83,45,20,0.05) 7px 8px)',
                            } : undefined}>
                            {s.presentation === 'cardboard' ? (
                              <span style={{
                                fontFamily: s.fontFamily,
                                fontWeight: s.fontWeight,
                                fontSize: '10px',
                                textTransform: 'uppercase',
                                letterSpacing: '0.01em',
                              }}>
                                <span style={{ color: '#15100C' }}>Card</span>
                                <span style={{ color: s.fontColor }}>board</span>
                              </span>
                            ) : (
                              <span style={{
                                fontFamily: s.fontFamily,
                                fontWeight: s.fontWeight,
                                color: s.fontColor,
                                textTransform: s.uppercase ? 'uppercase' : 'none',
                                fontSize: '11px',
                                letterSpacing: s.letterSpacing > 0.03 ? `${s.letterSpacing}em` : undefined,
                                textShadow: hint.textShadow,
                              }}>{s.name}</span>
                            )}
                          </button>
                        </Tooltip>
                      )
                    })}
                  </div>
                  <div className="mt-3 flex items-center gap-2">
                    <span className="text-[9px] text-slate-500 w-12 shrink-0">Size</span>
                    <input
                      type="range"
                      min={75}
                      max={125}
                      step={5}
                      value={Math.round(captionFontScale * 100)}
                      onChange={event => setCaptionFontScale(clampCaptionFontScale(Number(event.target.value) / 100))}
                      aria-label="Subtitle font size"
                      className="flex-1 h-1 accent-violet-500 cursor-pointer"
                    />
                    <span className="text-[9px] font-mono text-slate-500 w-9 text-right">
                      {Math.round(captionFontScale * 100)}%
                    </span>
                  </div>
                </div>
                {/* AI Emphasis */}
                {hasSrtCaptions && (
                  <div className="space-y-2">
                    <div className="flex items-center justify-between">
                      <label className="text-xs text-slate-400">AI Word Emphasis</label>
                      <Tooltip text="Automatically bold or highlight key words in subtitles" position="left">
                        <label className="flex items-center gap-2 cursor-pointer">
                          <input type="checkbox" checked={aiEmphasisEnabled} onChange={e => setAiEmphasisEnabled(e.target.checked)}
                            className="rounded border-surface-600 bg-surface-900 text-violet-500 focus:ring-violet-500" />
                          <span className="text-xs text-slate-400">{aiEmphasisEnabled ? 'On' : 'Off'}</span>
                        </label>
                      </Tooltip>
                    </div>
                  </div>
                )}
              </div>
            )}
              </Section>
            </div>

            <div
              id="editor-panel-publish"
              role="tabpanel"
              aria-labelledby="editor-tab-publish"
              className="v4-editor-workspace"
              hidden={editorWorkspace !== 'publish'}
            >
              {/* Publish Metadata */}
              <Section title="Post Details">
                {(() => {
                  const platformKey = exportPreset.id === 'tiktok' ? 'tiktok'
                    : exportPreset.id === 'reels' ? 'instagram'
                    : exportPreset.id === 'shorts' || exportPreset.id === 'youtube' ? 'youtube'
                    : 'tiktok'
                  return (
                    <PublishComposer
                      platform={platformKey}
                      metadata={publishMeta}
                      onChange={setPublishMeta}
                      clipId={clipId}
                      clipContext={{
                        title: title,
                        eventTags: highlight?.tags ?? [],
                        emotionTags: [],
                        transcriptExcerpt: highlight?.transcript_snippet || undefined,
                        eventSummary: highlight?.event_summary || undefined,
                        transcript: trimmedTranscriptText,
                        vodTitle: vod?.title || undefined,
                        game: game || undefined,
                        duration: clipDuration,
                      }}
                    />
                  )
                })()}
              </Section>

              <Section title="Thumbnail">
                <ThumbnailSelector
                  clipId={clipId!}
                  currentTime={playbackTime}
                  thumbnailPath={thumbnailPath}
                  onThumbnailSet={(path) => {
                    setThumbnailPath(path)
                    setClip(prev => prev ? { ...prev, thumbnail_path: path } : prev)
                  }}
                />
              </Section>

              {clipDuration > exportPreset.maxDuration && (
                <div className="bg-amber-500/10 border border-amber-500/30 rounded-xl p-3">
                  <p className="text-xs text-amber-400">
                    Clip is {formatTime(clipDuration)} — {exportPreset.name} max is {formatTime(exportPreset.maxDuration)}.
                    Consider trimming to fit.
                  </p>
                </div>
              )}

              {exportDone && clip.output_path && (
                <div className="bg-emerald-500/10 border border-emerald-500/30 rounded-xl p-4">
                  <div className="flex items-center gap-2 text-emerald-400 text-sm font-medium mb-1">
                    <Check className="w-4 h-4" />
                    Exported successfully
                  </div>
                  <p className="text-xs text-slate-400 font-mono truncate">{clip.output_path}</p>
                </div>
              )}

              {/* Actions: Save / Export / Upload */}
              <ActionsBar
                clipId={clipId || ''}
                clip={clip}
                saving={saving}
                saved={saved}
                exporting={exporting}
                exportProgress={exportProgress}
                exportDone={exportDone}
                exportError={exportError}
                mediaAvailable={hasUsableSourceMedia(clip, vod)}
                exportPreset={exportPreset}
                onSave={handleSave}
                onExportForFormat={handleExportForFormat}
                publishMeta={publishMeta}
                clipTitle={title}
                uploadHistory={uploadHistory}
                onUploadHistoryChange={(platform, url) => setUploadHistory(prev => ({ ...prev, [platform]: url }))}
              />
            </div>
          </div>
        </aside>
      </div>
    </div>
  )
}
