import { useEffect, useRef, useState, useMemo, useCallback } from 'react'
import { useParams, useNavigate } from 'react-router-dom'
import { ArrowLeft, Save, Download, Check, Loader2, Type, MessageSquare, Upload, Film, Link2, Undo2, Redo2, RotateCcw, RefreshCw, Bookmark, ChevronDown, X, Plus, Clock, CalendarClock } from 'lucide-react'
import { invoke, convertFileSrc } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
// Use our own Tauri command to open URLs in the system default browser
const openUrl = (url: string) => invoke('open_url', { url })
import type { Clip, Vod } from '../types'
import type { UploadResult } from '../stores/platformStore'
import { CAPTION_STYLES, EXPORT_PRESETS, LAYOUT_OPTIONS } from '../lib/editTypes'
import type { TextOverlay } from '../lib/editTypes'
import type { Highlight } from '../types'
import ClipPlayer from '../components/ClipPlayer'
import TrimTimeline from '../components/TrimTimeline'
import CaptionPreview from '../components/CaptionPreview'
import type { TimelineMarker } from '../components/TrimTimeline'
import { analyzeEmphasis, getEmphasisSummary } from '../lib/captionEmphasis'
import type { CaptionToken } from '../lib/captionEmphasis'
import ThumbnailSelector from '../components/ThumbnailSelector'
import LayoutPicker from '../components/LayoutPicker'
import FacecamEditor, { DraggablePipOverlay, DraggableSplitDivider, DEFAULT_FACECAM, computeSubtitleCollision } from '../components/FacecamEditor'
import type { FacecamSettings } from '../components/FacecamEditor'
import SubtitleEditor from '../components/SubtitleEditor'
import { parseSrt, serializeSrt, findActiveSegment } from '../lib/subtitleUtils'
import type { SubtitleSegment } from '../lib/subtitleUtils'
import { usePlaybackStore } from '../stores/playbackStore'
import { usePlatformStore, PLATFORM_INFO } from '../stores/platformStore'
import { useMontageStore } from '../stores/montageStore'
import Tooltip from '../components/Tooltip'
import PublishComposer from '../components/PublishComposer'
import type { PublishMetadata } from '../components/PublishComposer'
import { useScheduleStore } from '../stores/scheduleStore'
import PlatformUploadSelector, { getPresetForPlatform, getDefaultVisibility, getDefaultYouTubeSubFormat, expandYouTubeSubFormat } from '../components/PlatformUploadSelector'
import type { PlatformUploadState, YouTubeSubFormat } from '../components/PlatformUploadSelector'
import { useEditorHistory } from '../hooks/useEditorHistory'
import type { EditorSnapshot } from '../hooks/useEditorHistory'
import ExportProgressBar from '../components/ExportProgressBar'
import { useTemplateStore } from '../stores/templateStore'
import { useAiStore } from '../stores/aiStore'
import type { ClipTemplate } from '../stores/templateStore'
import { generateStandaloneTitle } from '../lib/publishCopyGenerator'
import type { ClipContext } from '../lib/publishCopyGenerator'

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
function ActionsBar({ clipId, clip, saving, saved, exporting, exportProgress, exportDone, exportError, vodPath, exportPreset, onSave, onExportForFormat, publishMeta, clipTitle, uploadHistory, onUploadHistoryChange }: {
  clipId: string; clip: Clip | null; saving: boolean; saved: boolean
  exporting: boolean; exportProgress: number; exportDone: boolean; exportError: string | null
  vodPath: boolean
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

  // Schedule state
  const { schedule: scheduleUpload, getForClip: getScheduledForClip } = useScheduleStore()
  const [scheduleMode, setScheduleMode] = useState(false)
  const [scheduleTime, setScheduleTime] = useState('')
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

  const selectedPlatforms = Object.entries(platformSelections)
    .filter(([_, checked]) => checked)
    .flatMap(([platform]) =>
      platform === 'youtube' ? expandYouTubeSubFormat(youtubeSubFormat) : [platform]
    )

  const anyUploading = multiUploading
  const allDone = selectedPlatforms.length > 0 &&
    selectedPlatforms.every(p => platformStates[p]?.status === 'done')

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
    return {
      clip_id: clipId,
      title: clipTitle || clip?.title || 'Untitled Clip',
      description,
      tags,
      visibility: platformVisibilities[platform] || getDefaultVisibility(platform),
      force,
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
        onUploadHistoryChange(platform, url)
        return { status: 'done', progress: 100, videoUrl: url }
      } else if (result.status.status === 'duplicate') {
        return { status: 'done', progress: 100, duplicateUrl: result.status.existing_url }
      } else if (result.status.status === 'failed') {
        return { status: 'error', progress: 0, error: result.status.error }
      }
      return { status: 'done', progress: 100 }
    } catch (e: any) {
      return { status: 'error', progress: 0, error: typeof e === 'string' ? e : e?.message || 'Upload failed' }
    }
  }

  // Multi-platform upload orchestrator — always saves + exports first, then uploads
  const handleMultiUpload = async () => {
    if (!clipId || selectedPlatforms.length === 0) return
    setMultiUploading(true)

    // Save first
    onSave()

    // Group platforms by required aspect ratio
    const groups: Record<string, string[]> = {}
    for (const platform of selectedPlatforms) {
      const preset = getPresetForPlatform(platform)
      const ar = preset.aspectRatio
      if (!groups[ar]) groups[ar] = []
      groups[ar].push(platform)
    }

    const initStates: Record<string, PlatformUploadState> = {}
    for (const p of selectedPlatforms) initStates[p] = { status: 'waiting', progress: 0 }
    setPlatformStates(prev => ({ ...prev, ...initStates }))

    for (const [aspectRatio, platforms] of Object.entries(groups)) {
      // Always export before uploading (ensures latest settings are baked in)
      for (const p of platforms) {
        setPlatformStates(prev => ({ ...prev, [p]: { status: 'exporting', progress: 0 } }))
      }
      try {
        await onExportForFormat(aspectRatio)
        await new Promise(r => setTimeout(r, 500))
      } catch (e: any) {
        for (const p of platforms) {
          setPlatformStates(prev => ({
            ...prev,
            [p]: { status: 'error', progress: 0, error: `Export failed: ${e}` },
          }))
        }
        continue
      }

      for (const platform of platforms) {
        setPlatformStates(prev => ({
          ...prev,
          [platform]: { status: 'uploading', progress: 50 },
        }))
        const result = await uploadToPlatform(platform)
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
      } catch (e: any) {
        console.error('[Schedule] Export failed:', e)
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
        } catch (e: any) {
          console.error(`[Schedule] Failed to schedule ${platform}:`, e)
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
    <div className="sticky bottom-0 bg-surface-900/80 backdrop-blur-sm p-2 -mx-1 rounded-lg space-y-2">
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
            disabled={downloading || exporting || !vodPath}
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
            invoke('open_url', { url: folder }).catch(() => {})
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
            try { await connect(platform) } catch {}
          }}
          youtubeSubFormat={youtubeSubFormat}
          onYouTubeSubFormatChange={handleYouTubeSubFormatChange}
          clipDuration={clipDuration}
        />

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
                min={new Date(Date.now() + 60000).toISOString().slice(0, 16)}
                className="w-full bg-surface-700 border border-surface-600 rounded-lg px-3 py-1.5 text-sm text-white focus:border-violet-500 focus:outline-none"
              />
            )}

            {/* Action button: Upload Now or Schedule */}
            {scheduleMode ? (
              <button
                onClick={handleScheduleUpload}
                disabled={scheduling || !scheduleTime || !vodPath}
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
                onClick={handleMultiUpload}
                disabled={anyUploading || !vodPath || allDone}
                className={`w-full flex items-center justify-center gap-2 px-3 py-2 text-xs font-medium rounded-lg transition-colors cursor-pointer border ${
                  allDone
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
                ) : allDone ? (
                  <><Check className="w-3.5 h-3.5" /> All uploads complete</>
                ) : (
                  <><Upload className="w-3.5 h-3.5" />
                    Export & Upload to {selectedPlatforms.length === 1
                      ? (selectedPlatforms[0] === 'youtube_shorts' ? 'YouTube Shorts' : PLATFORM_INFO[selectedPlatforms[0]]?.name || selectedPlatforms[0])
                      : `${selectedPlatforms.length} platforms`}
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
  const stopAll = usePlaybackStore(s => s.stopAll)

  // Stop all other playback when editor opens
  useEffect(() => { stopAll() }, [])

  const [clip, setClip] = useState<Clip | null>(null)
  const [highlight, setHighlight] = useState<Highlight | null>(null)
  const [vod, setVod] = useState<Vod | null>(null)
  const [videoSrc, setVideoSrc] = useState<string | null>(null)
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
  const [aiEmphasisEnabled, setAiEmphasisEnabled] = useState(true)
  const [generatingCaptions, setGeneratingCaptions] = useState(false)
  const [captionError, setCaptionError] = useState('')
  const [facecamLayout, setFacecamLayout] = useState('none')
  const [facecamSettings, setFacecamSettings] = useState<FacecamSettings>(DEFAULT_FACECAM)
  const [layoutPickerOpen, setLayoutPickerOpen] = useState(false)
  const [exportPresetId, setExportPresetId] = useState('tiktok')
  const [textOverlays, setTextOverlays] = useState<TextOverlay[]>([])
  const [game, setGame] = useState<string>('')
  const [publishMeta, setPublishMeta] = useState<PublishMetadata>({
    title: '', description: '', hashtags: [], visibility: 'public',
  })

  const clipDuration = Math.max(0, endSeconds - startSeconds)
  const captionStyle = CAPTION_STYLES.find(s => s.id === captionStyleId) || CAPTION_STYLES[0]
  const exportPreset = EXPORT_PRESETS.find(p => p.id === exportPresetId) || EXPORT_PRESETS[0]

  // ── Templates ──
  const templateStore = useTemplateStore()
  const aiStore = useAiStore()
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
    setExportPresetId(tmpl.exportPresetId)
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
      captionTone: 'punchy', // default — user can change later
      hashtags: publishMeta.hashtags,
      exportPresetId,
    })
    setTemplateSaveName('')
    setTemplateSaveOpen(false)
    setTemplateSaved(true)
    setTimeout(() => setTemplateSaved(false), 2000)
  }, [templateSaveName, captionStyleId, captionsPosition, publishMeta.hashtags, exportPresetId, templateStore])

  // ── Undo / Redo history ──
  const history = useEditorHistory()
  const [historyTick, setHistoryTick] = useState(0) // bumped on undo/redo to re-render buttons
  const historyRestoringRef = useRef(false)          // true while applying a snapshot

  const takeSnapshot = useCallback((): EditorSnapshot => ({
    title, startSeconds, endSeconds, captionsText,
    captionsPosition, captionStyleId, captionYOffset,
    publishTitle: publishMeta.title,
    publishDescription: publishMeta.description,
    publishHashtags: publishMeta.hashtags,
  }), [title, startSeconds, endSeconds, captionsText, captionsPosition, captionStyleId, captionYOffset, publishMeta])

  const applySnapshot = useCallback((snap: EditorSnapshot) => {
    historyRestoringRef.current = true
    setTitle(snap.title)
    setStartSeconds(snap.startSeconds)
    setEndSeconds(snap.endSeconds)
    setCaptionsText(snap.captionsText)
    setCaptionsPosition(snap.captionsPosition)
    setCaptionStyleId(snap.captionStyleId)
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
  }, [title, startSeconds, endSeconds, captionsText, captionsPosition, captionStyleId, captionYOffset, publishMeta, history, takeSnapshot])

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

  // Sync: external captionsText changes → parse into segments
  useEffect(() => {
    if (captionsText !== captionsTextRef.current) {
      captionsTextRef.current = captionsText
      setSubtitleSegments(parseSrt(captionsText))
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

  // SRT-relative playback time (0 = originalStart)
  const srtTime = Math.max(0, playbackTime - originalStart)
  // SRT-relative trim bounds
  const srtTrimStart = startSeconds - originalStart
  const srtTrimEnd = endSeconds - originalStart

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
    playerSeekRef.current?.(originalStart + srtTimeTarget)
  }, [originalStart])

  const handleGenerateCaptions = async () => {
    if (!clipId || generatingCaptions) return
    setGeneratingCaptions(true)
    setCaptionError('')
    try {
      const srt = await invoke<string>('generate_clip_captions', { clipId })
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
        const absTime = originalStart + phrase.time
        if (absTime >= originalStart && absTime <= originalEnd) {
          const markerType = phrase.type === 'urgency' ? 'reaction' as const
            : phrase.type === 'payoff' || phrase.type === 'punchline' ? 'payoff' as const
            : 'event' as const
          markers.push({ time: absTime, type: markerType, label: `"${phrase.text}" (${phrase.type})`, confidence: 0.7 })
        }
      }
    }

    return { timelineMarkers: markers, suggestedHookStart: hookSuggestion }
  }, [highlight, originalStart, originalEnd, startSeconds, hasSrtCaptions, aiEmphasisEnabled, emphasisSummary])

  // ── Load clip data ──
  useEffect(() => {
    if (!clipId) return
    publishMetaInitRef.current = false // reset so next publishMeta update from load is skipped
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
        setCaptionStyleId(c.caption_style || 'clean')
        setFacecamLayout(c.facecam_layout || 'none')
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
          captionYOffset: 0,
          publishTitle: c.title,
          publishDescription: '',
          publishHashtags: [],
        })
        setHistoryTick(0)

        // Fetch linked highlight for marker data
        let loadedHighlight: Highlight | undefined
        try {
          const highlights = await invoke<Highlight[]>('get_all_highlights')
          loadedHighlight = highlights.find(h => h.id === c.highlight_id)
          if (loadedHighlight) setHighlight(loadedHighlight)
        } catch { /* non-critical */ }

        const v = await invoke<Vod>('get_vod_detail', { vodId: c.vod_id })
        setVod(v)

        // Load game from stored values — clip.game takes priority, then VOD.game_name
        const storedGame = c.game || v.game_name || ''
        console.log('[Editor] Game loaded — clip.game:', JSON.stringify(c.game), '| vod.game_name:', JSON.stringify(v.game_name), '→', JSON.stringify(storedGame))
        setGame(storedGame)

        if (v.local_path) setVideoSrc(convertFileSrc(v.local_path))

        // Auto-generate publish description via AI when BYOK is active and no saved description
        if (!c.publish_description && aiStore.isByok() && clipId) {
          console.log('[Editor] No saved publish_description + BYOK active — auto-generating via AI')
          invoke<{ captions: Array<{ mode: string; text: string }>; source: string }>('generate_post_captions', {
            clipId,
            seed: (Date.now() % 1_000_000) >>> 0,
            transcriptText: null,
            currentTitle: c.title || null,
            currentGame: c.game || storedGame || null,
            selectedMode: 'punchy',
          }).then(bc => {
            if (bc.source === 'llm' && bc.captions.length > 0) {
              const aiDesc = bc.captions[0].text
              console.log('[Editor] AI auto-generated publish description:', aiDesc.substring(0, 80))
              setPublishMeta(prev => prev.description ? prev : { ...prev, description: aiDesc })
            }
          }).catch(err => {
            console.warn('[Editor] AI publish description auto-gen failed:', err)
          })
        }
      } catch (err) {
        console.error('Failed to load clip:', err)
      }
    })()
  }, [clipId])

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
        facecamLayout,
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
      captionsPosition, captionStyle: captionStyleId, facecamLayout,
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

  // ── Text overlay management ──
  const addOverlay = () => {
    setTextOverlays(prev => [...prev, {
      id: crypto.randomUUID(),
      text: 'Text',
      startTime: 0,
      endTime: Math.min(clipDuration, 5),
      position: 'top',
      style: 'label',
      fontSize: 48,
      color: '#FFFFFF',
    }])
  }

  const updateOverlay = (id: string, patch: Partial<TextOverlay>) => {
    setTextOverlays(prev => prev.map(o => o.id === id ? { ...o, ...patch } : o))
  }

  const removeOverlay = (id: string) => {
    setTextOverlays(prev => prev.filter(o => o.id !== id))
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

  // Caption Y position — respects facecam regions
  // In split layout, captions should stay in the game region (above the split line)
  const splitClampMax = facecamLayout === 'split' ? (facecamSettings.splitRatio * 100 - 5) : 95
  const captionBaseY = captionsPosition === 'top' ? 8
    : captionsPosition === 'center'
      ? (facecamLayout === 'split' ? facecamSettings.splitRatio * 50 : 50) // center within game region for split
      : Math.min(85, splitClampMax)
  const captionY = Math.max(3, Math.min(splitClampMax, captionBaseY + captionYOffset))
  const captionInSafeZone = captionY >= 10 && captionY <= (facecamLayout === 'split' ? splitClampMax : 90)
  const captionCollision = computeSubtitleCollision(captionY, facecamLayout, facecamSettings)
  const captionPositionStyle = { top: `${captionY}%`, transform: captionsPosition === 'center' ? 'translateY(-50%)' : undefined }

  // Plain-text caption scale — uses same frame width as the preview container
  // CaptionPreview (SRT path) measures its own frame via ResizeObserver, but
  // the plain-text path needs an approximate scale computed here.
  const effectiveFrameW = frameWidth > 0 ? frameWidth : 442 // 442 is typical landscape width in the panel
  const formatMultiplier = aspectRatio === '9:16' ? 1.15 : aspectRatio === '16:9' ? 0.85 : 1.0
  const plainS = (effectiveFrameW / 1080) * formatMultiplier
  const captionPreviewStyle: React.CSSProperties = {
    fontFamily: captionStyle.fontFamily,
    fontSize: `${captionStyle.fontSize * plainS}px`,
    fontWeight: captionStyle.fontWeight,
    color: captionStyle.fontColor,
    textTransform: captionStyle.uppercase ? 'uppercase' : 'none',
    letterSpacing: `${captionStyle.letterSpacing}em`,
    lineHeight: captionStyle.lineHeight,
    textShadow: captionStyle.shadow === 'none' ? 'none'
      : captionStyle.shadow.replace(/(\d+)px/g, (_, n) => `${Math.max(1, Math.round(parseInt(n) * plainS))}px`),
    backgroundColor: captionStyle.bgColor || undefined,
    padding: captionStyle.bgPadding > 0 ? `${captionStyle.bgPadding * plainS * 0.6}px ${captionStyle.bgPadding * plainS}px` : undefined,
    borderRadius: captionStyle.bgRadius > 0 ? `${captionStyle.bgRadius * plainS}px` : undefined,
  }

  if (!clip) {
    return <div className="flex items-center justify-center h-64"><p className="text-slate-400">Loading editor...</p></div>
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center gap-4">
        <button onClick={() => navigate('/clips')} className="p-2 rounded-lg bg-surface-800 hover:bg-surface-700 text-slate-400 hover:text-white transition-colors cursor-pointer">
          <ArrowLeft className="w-5 h-5" />
        </button>
        <h1 className="text-2xl font-bold text-white truncate flex-1">Edit Clip</h1>
        <div className="flex items-center gap-1.5">
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

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* ── Left: Preview ── */}
        <div className="space-y-4">
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
                  clipStart={startSeconds}
                  clipEnd={endSeconds}
                  mode="full"
                  controlsOverlay
                  className="h-full"
                  onTimeUpdate={setPlaybackTime}
                  onPlayChange={setIsPlaying}
                  seekRef={playerSeekRef}
                  overlay={<>
                    {/* ── Layout mode overlay — interactive ── */}
                    {facecamLayout === 'split' && (
                      <>
                        <div className="absolute inset-0 pointer-events-none z-[8]">
                          <div className="absolute left-1 text-[7px] text-white/40 font-mono" style={{ top: '2%' }}>GAME</div>
                          <div className="absolute left-1 text-[7px] text-white/40 font-mono"
                            style={{ top: `${facecamSettings.splitRatio * 100 + 2}%` }}>FACECAM</div>
                          {/* Facecam region preview */}
                          <div className="absolute left-0 right-0 bottom-0"
                            style={{ top: `${facecamSettings.splitRatio * 100}%`, background: 'rgba(60,20,100,0.25)' }} />
                        </div>
                        <DraggableSplitDivider
                          settings={facecamSettings}
                          onChange={setFacecamSettings}
                          frameHeight={frameHeightPx}
                        />
                      </>
                    )}
                    {facecamLayout === 'pip' && (
                      <DraggablePipOverlay
                        settings={facecamSettings}
                        onChange={setFacecamSettings}
                        frameWidth={frameWidthPx}
                        frameHeight={frameHeightPx}
                      />
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
                            wordBreak: 'normal',
                            overflowWrap: 'break-word',
                            whiteSpace: 'normal',
                            display: 'inline-block',
                          }}>
                            {captionsText}
                          </span>
                        </div>
                      ) : (
                        <div className="absolute bottom-[8%] left-0 right-0 flex justify-center pointer-events-none z-10">
                          <span className="text-center text-xs text-slate-400/80 bg-black/50 px-3 py-1.5 rounded">
                            {generatingCaptions ? 'Generating subtitles...' : 'No subtitles available'}
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

          {/* Export status */}
          {exportDone && clip.output_path && (
            <div className="bg-emerald-500/10 border border-emerald-500/30 rounded-xl p-4">
              <div className="flex items-center gap-2 text-emerald-400 text-sm font-medium mb-1">
                <Check className="w-4 h-4" />
                Exported successfully
              </div>
              <p className="text-xs text-slate-400 font-mono truncate">{clip.output_path}</p>
            </div>
          )}

          {/* Thumbnail selector */}
          <div className="v4-panel" style={{padding: 16}}>
            <h2 className="text-sm font-semibold text-slate-300 mb-3">Thumbnail</h2>
            <ThumbnailSelector
              clipId={clipId!}
              currentTime={playbackTime}
              thumbnailPath={thumbnailPath}
              onThumbnailSet={(path) => {
                setThumbnailPath(path)
                setClip(prev => prev ? { ...prev, thumbnail_path: path } : prev)
              }}
            />
          </div>

          {/* Duration warning */}
          {clipDuration > exportPreset.maxDuration && (
            <div className="bg-amber-500/10 border border-amber-500/30 rounded-xl p-3">
              <p className="text-xs text-amber-400">
                Clip is {formatTime(clipDuration)} — {exportPreset.name} max is {formatTime(exportPreset.maxDuration)}.
                Consider trimming to fit.
              </p>
            </div>
          )}
        </div>

        {/* ── Right: Settings ── */}
        <div className="space-y-4 overflow-y-auto max-h-[calc(100vh-10rem)] pr-1">
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
                      eventTags: highlight ? (highlight.tags || []) as string[] : [],
                      emotionTags: [],
                      transcriptExcerpt: highlight?.transcript_snippet || undefined,
                      eventSummary: highlight?.event_summary || undefined,
                      transcript: subtitleSegments.length > 0
                        ? subtitleSegments.map(s => s.text).join(' ')
                        : undefined,
                      vodTitle: vod?.title || undefined,
                      game: game || undefined,
                      duration: clipDuration,
                    }

                    let newTitle: string | null = null

                    // Try AI title generation first (uses BYOK provider if configured)
                    if (clipId) {
                      try {
                        const transcriptText = subtitleSegments.length > 0
                          ? subtitleSegments.map(s => s.text).join(' ')
                          : null
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
                    eventTags: highlight ? (highlight.tags || []) as string[] : [],
                    emotionTags: [],
                    transcriptExcerpt: highlight?.transcript_snippet || undefined,
                    eventSummary: highlight?.event_summary || undefined,
                    transcript: subtitleSegments.length > 0
                      ? subtitleSegments.map(s => s.text).join(' ')
                      : undefined,
                    vodTitle: vod?.title || undefined,
                    game: game || undefined,
                    duration: clipDuration,
                  }}
                />
              )
            })()}
          </Section>

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
                current={facecamLayout as any}
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
              />
            )}
            {facecamLayout === 'none' && (
              <p className="text-[9px] text-slate-600 mt-2">Select Split or PiP layout to add facecam controls.</p>
            )}
          </Section>

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
                        <span className="text-[9px] text-slate-600">Click time to seek</span>
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
                          {vod?.local_path ? (
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
                                Requires Python with faster-whisper installed. You can also type a caption above for a static overlay.
                              </p>
                            </>
                          ) : (
                            <p className="text-[10px] text-slate-500 leading-relaxed">
                              VOD not downloaded. Download the VOD first to generate timed subtitles, or type a caption above.
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
                        'bold-white': { bg: 'bg-surface-900', border: 'border-slate-400/60', textShadow: '2px 2px 0 #000, -1px -1px 0 #000, 1px -1px 0 #000, -1px 1px 0 #000' },
                        boxed:       { bg: 'bg-surface-900', border: 'border-slate-500/40', textShadow: 'none' },
                        neon:        { bg: 'bg-surface-900', border: 'border-emerald-500/40', textShadow: '0 0 6px #00ff8880, 0 0 2px #000' },
                        minimal:     { bg: 'bg-surface-900', border: 'border-slate-600/40', textShadow: '0 1px 3px rgba(0,0,0,0.6)' },
                        fire:        { bg: 'bg-surface-900', border: 'border-orange-500/40', textShadow: '0 0 6px #FF450088, 0 0 2px #000, 1px 1px 0 #000' },
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
                            style={s.id === 'boxed' ? {
                              background: isActive ? undefined : 'linear-gradient(135deg, rgba(30,30,50,0.9) 0%, rgba(15,15,30,0.95) 100%)',
                            } : undefined}>
                            {/* Boxed style: show mini box behind text */}
                            {s.id === 'boxed' ? (
                              <span style={{
                                fontFamily: s.fontFamily,
                                fontWeight: s.fontWeight,
                                color: s.fontColor,
                                fontSize: '11px',
                                background: 'rgba(10,10,20,0.8)',
                                padding: '2px 6px',
                                borderRadius: '4px',
                                textShadow: hint.textShadow,
                              }}>{s.name}</span>
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
            vodPath={!!vod?.local_path}
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
    </div>
  )
}
   