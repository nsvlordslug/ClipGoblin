import { useEffect, useRef, useState, useMemo, useCallback } from 'react'
import { useParams, useNavigate } from 'react-router-dom'
import { ArrowLeft, Save, Download, Check, Loader2, Type, MessageSquare, Upload, Film, Link2 } from 'lucide-react'
import { invoke, convertFileSrc } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
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
import PublishComposer from '../components/PublishComposer'
import type { PublishMetadata } from '../components/PublishComposer'

function formatTime(seconds: number) {
  const m = Math.floor(seconds / 60)
  const s = Math.floor(seconds % 60)
  return `${m}:${String(s).padStart(2, '0')}`
}

// ── Reusable section component ──
function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="bg-surface-800 border border-surface-700 rounded-xl p-4">
      <h3 className="text-sm font-semibold text-slate-300 mb-3">{title}</h3>
      {children}
    </div>
  )
}

// ── Pill selector ──
function PillGroup<T extends string>({ value, options, onChange }: {
  value: T
  options: { value: T; label: string; desc?: string }[]
  onChange: (v: T) => void
}) {
  return (
    <div className="flex gap-2">
      {options.map(opt => (
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
      ))}
    </div>
  )
}

/** Action buttons — extracted so platform hooks are called at component level */
function ActionsBar({ clipId, clip, saving, saved, exporting, exportProgress, exportDone, exportError, vodPath, exportPreset, onSave, onExport, publishMeta }: {
  clipId: string; clip: Clip | null; saving: boolean; saved: boolean
  exporting: boolean; exportProgress: number; exportDone: boolean; exportError: string | null
  vodPath: boolean
  exportPreset: { id: string; aspectRatio: string; name: string }
  onSave: () => void; onExport: () => void
  publishMeta?: { title: string; description: string; hashtags: string[]; visibility: string }
}) {
  const { connect, isConnected } = usePlatformStore()
  const { projects, addClip, createProject } = useMontageStore()
  const navigate = useNavigate()

  const platformKey = exportPreset.id === 'tiktok' ? 'tiktok'
    : exportPreset.id === 'reels' ? 'instagram'
    : exportPreset.id === 'shorts' || exportPreset.id === 'youtube' ? 'youtube'
    : null

  const info = platformKey ? PLATFORM_INFO[platformKey] : null
  const connected = platformKey ? isConnected(platformKey) : false

  const [uploading, setUploading] = useState(false)
  const [uploadDone, setUploadDone] = useState(false)
  const [uploadError, setUploadError] = useState<string | null>(null)
  const [uploadUrl, setUploadUrl] = useState<string | null>(null)
  const [duplicateUrl, setDuplicateUrl] = useState<string | null>(null)

  const handleUpload = async (force = false) => {
    if (!clipId || !platformKey) return
    setUploading(true)
    setUploadError(null)
    setUploadDone(false)
    setDuplicateUrl(null)

    try {
      // If not connected, connect first (seamless connect-then-upload)
      if (!isConnected(platformKey)) {
        await connect(platformKey)
      }

      const result = await invoke<UploadResult>('upload_to_platform', {
        platform: platformKey,
        clipId,
        title: publishMeta?.title || clip?.title || 'Untitled Clip',
        description: publishMeta?.description || '',
        tags: publishMeta?.hashtags || [],
        visibility: publishMeta?.visibility || 'unlisted',
        force,
      })

      if (result.status.status === 'complete') {
        setUploadDone(true)
        setUploadUrl(result.status.video_url)
      } else if (result.status.status === 'duplicate') {
        setDuplicateUrl(result.status.existing_url)
      } else if (result.status.status === 'failed') {
        setUploadError(result.status.error)
      }
    } catch (e: any) {
      setUploadError(typeof e === 'string' ? e : e?.message || 'Upload failed')
    } finally {
      setUploading(false)
    }
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

  return (
    <div className="sticky bottom-0 bg-surface-900/80 backdrop-blur-sm p-2 -mx-1 rounded-lg space-y-2">
      <div className="flex gap-2">
        <button onClick={onSave} disabled={saving}
          className="flex-1 flex items-center justify-center gap-2 px-4 py-2.5 bg-surface-700 hover:bg-surface-600 disabled:opacity-50 text-white text-sm font-medium rounded-lg transition-colors cursor-pointer">
          {saved ? <Check className="w-4 h-4" /> : <Save className="w-4 h-4" />}
          {saved ? 'Saved!' : saving ? 'Saving...' : 'Save'}
        </button>
        <button onClick={onExport} disabled={exporting || !vodPath}
          className={`flex-1 flex items-center justify-center gap-2 px-4 py-2.5 text-white text-sm font-medium rounded-lg transition-colors cursor-pointer ${
            exportDone ? 'bg-green-600 hover:bg-green-500' : exportError ? 'bg-red-600 hover:bg-red-500' : 'bg-violet-600 hover:bg-violet-500'
          } disabled:opacity-50`}>
          {exporting ? <Loader2 className="w-4 h-4 animate-spin" /> :
           exportDone ? <Check className="w-4 h-4" /> :
           <Download className="w-4 h-4" />}
          {exporting ? `Exporting... ${exportProgress}%` :
           exportDone ? 'Exported' :
           exportError ? 'Retry Export' :
           `Export for ${exportPreset.name}`}
        </button>
      </div>

      <div className="flex gap-2">
        {/* Upload / Connect button */}
        {platformKey && info && exportDone && (
          <>
            {duplicateUrl ? (
              <div className="flex-1 flex flex-col gap-1">
                <p className="text-[10px] text-amber-400 px-1">Already uploaded to {info.name}</p>
                <div className="flex gap-1">
                  <button onClick={() => window.open(duplicateUrl, '_blank')}
                    className="flex-1 flex items-center justify-center gap-1 px-2 py-1.5 text-xs text-slate-300 bg-surface-800 border border-surface-600 rounded hover:text-white transition-colors cursor-pointer">
                    View existing
                  </button>
                  <button onClick={() => { setDuplicateUrl(null); handleUpload(true) }}
                    className="flex-1 flex items-center justify-center gap-1 px-2 py-1.5 text-xs text-amber-400 bg-amber-500/10 border border-amber-500/30 rounded hover:bg-amber-500/20 transition-colors cursor-pointer">
                    Upload again
                  </button>
                </div>
              </div>
            ) : uploadDone && uploadUrl ? (
              <button onClick={() => window.open(uploadUrl, '_blank')}
                className="flex-1 flex items-center justify-center gap-2 px-3 py-2 text-xs font-medium rounded-lg bg-green-600/20 text-green-400 border border-green-500/30 hover:bg-green-600/30 transition-colors cursor-pointer">
                <Check className="w-3.5 h-3.5" />
                Uploaded — View on {info.name}
              </button>
            ) : (
              <button
                onClick={() => handleUpload(false)}
                disabled={uploading}
                className={`flex-1 flex items-center justify-center gap-2 px-3 py-2 text-xs font-medium rounded-lg transition-colors cursor-pointer border ${
                  uploadError
                    ? 'bg-red-600/10 text-red-400 border-red-500/30 hover:bg-red-600/20'
                    : isConnected(platformKey)
                    ? 'text-white border-transparent hover:opacity-90'
                    : 'bg-surface-800 text-slate-400 border-surface-600 hover:text-white'
                }`}
                style={isConnected(platformKey) && !uploadError ? { backgroundColor: `${info.color}cc` } : undefined}
                title={uploadError || undefined}
              >
                {uploading ? (
                  <><Loader2 className="w-3.5 h-3.5 animate-spin" /> Uploading...</>
                ) : uploadError ? (
                  <><Upload className="w-3.5 h-3.5" /> Retry Upload</>
                ) : isConnected(platformKey) ? (
                  <><Upload className="w-3.5 h-3.5" /> Upload to {info.name}</>
                ) : (
                  <><Link2 className="w-3.5 h-3.5" /> Connect {info.name}</>
                )}
              </button>
            )}
          </>
        )}

        {/* Add to Montage */}
        <button onClick={handleAddToMontage}
          className="flex items-center justify-center gap-1.5 px-3 py-2 text-xs text-slate-400 bg-surface-800 border border-surface-600 rounded-lg hover:text-white hover:border-violet-500/40 transition-colors cursor-pointer">
          <Film className="w-3.5 h-3.5" />
          Add to Montage
        </button>
      </div>
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
  const [publishMeta, setPublishMeta] = useState<PublishMetadata>({
    title: '', description: '', hashtags: [], visibility: 'public',
  })

  const clipDuration = Math.max(0, endSeconds - startSeconds)
  const captionStyle = CAPTION_STYLES.find(s => s.id === captionStyleId) || CAPTION_STYLES[0]
  const exportPreset = EXPORT_PRESETS.find(p => p.id === exportPresetId) || EXPORT_PRESETS[0]

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
    ;(async () => {
      try {
        const c = await invoke<Clip>('get_clip_detail', { clipId })
        setClip(c)
        setTitle(c.title)
        setStartSeconds(c.start_seconds)
        setEndSeconds(c.end_seconds)
        setAspectRatio(c.aspect_ratio)
        setCaptionsEnabled(c.captions_enabled === 1)
        setCaptionsText(c.captions_text || '')
        setCaptionsPosition(c.captions_position || 'bottom')
        setFacecamLayout(c.facecam_layout || 'none')
        setExportDone(c.render_status === 'completed')
        setOriginalStart(c.start_seconds)
        setOriginalEnd(c.end_seconds)
        setThumbnailPath(c.thumbnail_path)
        setPlaybackTime(c.start_seconds)
        setPublishMeta(prev => ({ ...prev, title: c.title }))

        // Fetch linked highlight for marker data
        try {
          const highlights = await invoke<Highlight[]>('get_all_highlights')
          const hl = highlights.find(h => h.id === c.highlight_id)
          if (hl) setHighlight(hl)
        } catch { /* non-critical */ }

        const v = await invoke<Vod>('get_vod_detail', { vodId: c.vod_id })
        setVod(v)
        if (v.local_path) setVideoSrc(convertFileSrc(v.local_path))
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
        facecamLayout,
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

  const handleExport = async () => {
    if (!clipId) return
    await handleSave()
    setExporting(true)
    setExportDone(false)
    setExportError(null)
    setExportProgress(0)
    try {
      // Listen for job progress events from the backend
      const jobId = `export-${clipId}`
      const unlisten = await listen<{ jobId: string; progress: number; status: string; error?: string }>('job-progress', (event) => {
        if (event.payload.jobId !== jobId) return
        const { progress, status, error } = event.payload
        setExportProgress(progress)
        if (status === 'completed') {
          setExporting(false)
          setExportDone(true)
          exportUnlistenRef.current?.()
          // Refresh clip data to get output_path
          invoke<Clip>('get_clip_detail', { clipId }).then(c => setClip(c)).catch(() => {})
        } else if (status === 'failed') {
          setExporting(false)
          setExportError(error || 'Export failed')
          exportUnlistenRef.current?.()
        }
      })
      exportUnlistenRef.current = unlisten

      await invoke('export_clip', { clipId })
    } catch (err) {
      setExporting(false)
      setExportError(`Export failed: ${err}`)
      exportUnlistenRef.current?.()
    }
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
  const FRAME_WIDTHS: Record<string, number> = { '9:16': 270, '1:1': 360, '16:9': 0 } // 0 = w-full
  const frameWidth = FRAME_WIDTHS[aspectRatio] || 0

  const previewAspect = aspectRatio === '9:16' ? 'aspect-[9/16]'
    : aspectRatio === '1:1' ? 'aspect-square'
    : 'aspect-video'

  // Tailwind needs static class names — can't use dynamic `w-[${n}px]`
  const previewWidth = aspectRatio === '9:16' ? 'w-[270px]'
    : aspectRatio === '1:1' ? 'w-[360px]'
    : 'w-full'

  // Compute actual frame pixel dimensions for facecam overlays
  const frameHeightPx = aspectRatio === '9:16' ? 480 : aspectRatio === '1:1' ? 360 : 249
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
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* ── Left: Preview ── */}
        <div className="space-y-4">
          <div className="bg-surface-800 border border-surface-700 rounded-xl p-4">
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
                          <span className="text-center max-w-[80%]" style={captionPreviewStyle}>
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
          <div className="bg-surface-800 border border-surface-700 rounded-xl p-4">
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
            <input type="text" value={title} onChange={e => setTitle(e.target.value)}
              className="w-full px-3 py-2 bg-surface-900 border border-surface-600 rounded-lg text-white text-sm focus:outline-none focus:border-violet-500" />
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
            <div className="grid grid-cols-3 gap-2">
              {EXPORT_PRESETS.filter(p => ['tiktok', 'reels', 'shorts'].includes(p.id)).map(p => (
                <button key={p.id} onClick={() => setExportPresetId(p.id)}
                  className={`px-3 py-2.5 rounded-lg text-sm font-medium transition-colors cursor-pointer border ${
                    exportPresetId === p.id
                      ? 'bg-violet-600/20 text-violet-400 border-violet-500/40'
                      : 'bg-surface-900 text-slate-400 border-surface-600 hover:bg-surface-700'
                  }`}>
                  <div>{p.name}</div>
                  <div className="text-xs opacity-60">{p.description}</div>
                </button>
              ))}
            </div>
            <div className="flex gap-2 mt-2">
              {EXPORT_PRESETS.filter(p => !['tiktok', 'reels', 'shorts'].includes(p.id)).map(p => (
                <button key={p.id} onClick={() => setExportPresetId(p.id)}
                  className={`flex-1 px-2 py-1.5 rounded-lg text-xs font-medium transition-colors cursor-pointer border ${
                    exportPresetId === p.id
                      ? 'bg-violet-600/20 text-violet-400 border-violet-500/40'
                      : 'bg-surface-900 text-slate-400 border-surface-600 hover:bg-surface-700'
                  }`}>
                  {p.name}
                </button>
              ))}
            </div>
          </Section>

          {/* Layout */}
          {/* Publish Metadata */}
          <Section title="Post Details">
            {(() => {
              const platformKey = exportPreset.id === 'tiktok' ? 'tiktok'
                : exportPreset.id === 'reels' ? 'instagram'
                : exportPreset.id === 'shorts' || exportPreset.id === 'youtube' ? 'youtube'
                : exportPreset.id === 'square' ? 'twitter' : 'tiktok'
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
                aspectRatio === '9:16' ? 'w-8 aspect-[9/16]' : aspectRatio === '1:1' ? 'w-10 aspect-square' : 'w-14 aspect-video'
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
                    options={[{ value: 'top', label: 'Top' }, { value: 'center', label: 'Center' }, { value: 'bottom', label: 'Bottom' }]} />

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
                    {CAPTION_STYLES.map(s => (
                      <button key={s.id} onClick={() => setCaptionStyleId(s.id)}
                        className={`px-2 py-2 rounded-lg text-xs font-medium transition-colors cursor-pointer border ${
                          captionStyleId === s.id
                            ? 'bg-violet-600/20 text-violet-400 border-violet-500/40'
                            : 'bg-surface-900 text-slate-400 border-surface-600 hover:bg-surface-700'
                        }`}>
                        <span style={{
                          fontFamily: s.fontFamily,
                          fontWeight: s.fontWeight,
                          color: s.fontColor,
                          textTransform: s.uppercase ? 'uppercase' : 'none',
                          fontSize: '11px',
                          textShadow: s.shadow !== 'none' ? '1px 1px 2px #000' : undefined,
                        }}>{s.name}</span>
                      </button>
                    ))}
                  </div>
                </div>
                {/* AI Emphasis */}
                {hasSrtCaptions && (
                  <div className="space-y-2">
                    <div className="flex items-center justify-between">
                      <label className="text-xs text-slate-400">AI Word Emphasis</label>
                      <label className="flex items-center gap-2 cursor-pointer">
                        <input type="checkbox" checked={aiEmphasisEnabled} onChange={e => setAiEmphasisEnabled(e.target.checked)}
                          className="rounded border-surface-600 bg-surface-900 text-violet-500 focus:ring-violet-500" />
                        <span className="text-xs text-slate-400">{aiEmphasisEnabled ? 'On' : 'Off'}</span>
                      </label>
                    </div>

                    {aiEmphasisEnabled && emphasisSummary.length > 0 && (
                      <div className="bg-surface-900 rounded-lg p-2 space-y-1 max-h-24 overflow-y-auto">
                        <p className="text-[10px] text-slate-500 mb-1">{emphasisSummary.length} emphasized phrases</p>
                        {emphasisSummary.map((e, i) => (
                          <div key={i} className="flex items-center gap-1.5 text-[10px]">
                            <span className={`px-1 py-0.5 rounded text-[9px] font-medium ${
                              e.type === 'reaction' ? 'bg-amber-500/20 text-amber-400' :
                              e.type === 'urgency' ? 'bg-red-500/20 text-red-400' :
                              e.type === 'payoff' ? 'bg-emerald-500/20 text-emerald-400' :
                              e.type === 'punchline' ? 'bg-violet-500/20 text-violet-400' :
                              'bg-blue-500/20 text-blue-400'
                            }`}>{e.type}</span>
                            <span className="text-white font-medium truncate">"{e.text}"</span>
                          </div>
                        ))}
                      </div>
                    )}

                    {aiEmphasisEnabled && emphasisSummary.length === 0 && (
                      <p className="text-[10px] text-slate-500 italic">No emphasis phrases detected in captions</p>
                    )}
                  </div>
                )}
              </div>
            )}
          </Section>

          {/* Text Overlays */}
          <Section title="Text Overlays">
            {textOverlays.map(o => (
              <div key={o.id} className="mb-3 p-3 bg-surface-900 rounded-lg border border-surface-600 space-y-2">
                <div className="flex gap-2">
                  <input type="text" value={o.text} onChange={e => updateOverlay(o.id, { text: e.target.value })}
                    className="flex-1 px-2 py-1.5 bg-surface-800 border border-surface-600 rounded text-white text-xs focus:outline-none focus:border-violet-500" />
                  <button onClick={() => removeOverlay(o.id)} className="text-red-400 hover:text-red-300 text-xs cursor-pointer px-2">Remove</button>
                </div>
                <div className="grid grid-cols-3 gap-2">
                  <div>
                    <label className="block text-[10px] text-slate-500">Start (s)</label>
                    <input type="number" value={o.startTime} step="0.5" min="0" max={clipDuration}
                      onChange={e => updateOverlay(o.id, { startTime: parseFloat(e.target.value) || 0 })}
                      className="w-full px-2 py-1 bg-surface-800 border border-surface-600 rounded text-white text-xs" />
                  </div>
                  <div>
                    <label className="block text-[10px] text-slate-500">End (s)</label>
                    <input type="number" value={o.endTime} step="0.5" min="0" max={clipDuration}
                      onChange={e => updateOverlay(o.id, { endTime: parseFloat(e.target.value) || 0 })}
                      className="w-full px-2 py-1 bg-surface-800 border border-surface-600 rounded text-white text-xs" />
                  </div>
                  <div>
                    <label className="block text-[10px] text-slate-500">Position</label>
                    <select value={o.position} onChange={e => updateOverlay(o.id, { position: e.target.value as TextOverlay['position'] })}
                      className="w-full px-2 py-1 bg-surface-800 border border-surface-600 rounded text-white text-xs">
                      <option value="top">Top</option>
                      <option value="center">Center</option>
                      <option value="bottom">Bottom</option>
                    </select>
                  </div>
                </div>
              </div>
            ))}
            <button onClick={addOverlay}
              className="w-full flex items-center justify-center gap-1.5 px-3 py-2 bg-surface-900 border border-dashed border-surface-600 rounded-lg text-slate-400 hover:text-violet-400 hover:border-violet-500/40 text-xs transition-colors cursor-pointer">
              <Type className="w-3.5 h-3.5" />
              Add Text Overlay
            </button>
          </Section>

          {/* Actions */}
          <ActionsBar
            clipId={clipId!}
            clip={clip}
            saving={saving} saved={saved}
            exporting={exporting} exportProgress={exportProgress}
            exportDone={exportDone} exportError={exportError}
            vodPath={!!vod?.local_path}
            exportPreset={exportPreset}
            onSave={handleSave} onExport={handleExport}
            publishMeta={publishMeta}
          />
        </div>
      </div>

      {/* Layout picker modal */}
      {layoutPickerOpen && (
        <LayoutPicker
          current={facecamLayout as any}
          aspectRatio={aspectRatio as '9:16' | '16:9' | '1:1'}
          platformName={exportPreset.name}
          onSelect={setFacecamLayout as any}
          onClose={() => setLayoutPickerOpen(false)}
        />
      )}
    </div>
  )
}
