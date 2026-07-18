import { useEffect, useMemo, useRef, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { ArrowLeft, Trash2, Download, Film, Clock, Plus, FolderOpen, Loader2, Search, CheckCircle2, AlertCircle, Scissors, Blend } from 'lucide-react'
import { useMontageStore } from '../stores/montageStore'
import type { MontageExportPreset, MontageTransition } from '../stores/montageStore'
import { useAppStore } from '../stores/appStore'
import type { Clip, Vod } from '../types'
import PublishComposer from '../components/PublishComposer'
import type { PublishMetadata } from '../components/PublishComposer'
import ClipPlayer from '../components/ClipPlayer'
import { errorMessage } from '../lib/errors'
import {
  exceedsYouTubeShortsLimit,
  filterAvailableMontageClips,
  montageCrossfadeDuration,
  montageCrossfadeProgress,
  montageDuration,
  montageSourceGroup,
  nextMontageClipId,
  YOUTUBE_SHORTS_MAX_SECONDS,
} from '../lib/montage'
import type { MontageSourceFilter } from '../lib/montage'

function fmt(s: number) {
  const m = Math.floor(s / 60)
  const sec = Math.floor(s % 60)
  return `${m}:${String(sec).padStart(2, '0')}`
}

interface MontageProgress {
  projectId: string
  progress: number
  stage: string
  currentClip: number
  totalClips: number
}

interface MontageExportResult {
  outputPath: string
  outputDirectory: string
  durationSeconds: number
}

async function resolveClipPreviewSource(clip: Clip): Promise<string> {
  let path: string | null = null
  if (clip.source_media_path) {
    path = await invoke<string>('prepare_clip_preview_source', { clipId: clip.id })
  } else if (clip.community_clip_mp4_path) {
    path = clip.community_clip_mp4_path
  } else {
    const vod = await invoke<Vod>('get_vod_detail', { vodId: clip.vod_id })
    path = vod.local_path
  }
  if (!path) throw new Error('The source video is not available on this PC.')
  return convertFileSrc(path)
}

export default function MontageBuilder() {
  const navigate = useNavigate()
  const { projects, activeProjectId, createProject, deleteProject, setActive, addClip, removeClip, reorderClips, updateProject } = useMontageStore()
  const { clips, highlights, fetchClips, fetchHighlights } = useAppStore()
  const [publishMeta, setPublishMeta] = useState<PublishMetadata>({
    title: '', description: '', hashtags: [], visibility: 'public',
  })
  const [selectedClipId, setSelectedClipId] = useState<string | null>(null)
  const [previewMedia, setPreviewMedia] = useState<{ key: string; src: string } | null>(null)
  const [clipPreviewSources, setClipPreviewSources] = useState<Record<string, string>>({})
  const [readyPreviewClipIds, setReadyPreviewClipIds] = useState<string[]>([])
  const [previewLoading, setPreviewLoading] = useState(false)
  const [previewError, setPreviewError] = useState<string | null>(null)
  const [clipSearch, setClipSearch] = useState('')
  const [sourceFilter, setSourceFilter] = useState<MontageSourceFilter>('all')
  const [exporting, setExporting] = useState(false)
  const [exportProgress, setExportProgress] = useState(0)
  const [exportStage, setExportStage] = useState('')
  const [exportError, setExportError] = useState<string | null>(null)
  const [exportResult, setExportResult] = useState<MontageExportResult | null>(null)
  const [previewMode, setPreviewMode] = useState<'clip' | 'export'>('clip')
  const [sequenceAutoPlay, setSequenceAutoPlay] = useState(false)
  const [previewTransitioning, setPreviewTransitioning] = useState(false)
  const [crossfadeProgress, setCrossfadeProgress] = useState(0)
  const loadedProjectIdRef = useRef<string | null>(null)
  const clipPreviewSourcesRef = useRef<Record<string, string>>({})
  const transitionTargetRef = useRef<string | null>(null)

  useEffect(() => { fetchClips(); fetchHighlights() }, [fetchClips, fetchHighlights])

  // Auto-create a project if none exists
  useEffect(() => {
    if (projects.length === 0) {
      createProject('My Montage')
    } else if (!activeProjectId || !projects.some(item => item.id === activeProjectId)) {
      setActive(projects[0].id)
    }
  }, [activeProjectId, createProject, projects, setActive])

  const project = projects.find(p => p.id === activeProjectId)

  const clipById = useMemo(() => new Map(clips.map(clip => [clip.id, clip])), [clips])
  const transition = project?.transition ?? 'cut'
  const segmentDurations = project?.segments.map(segment => {
    const clip = clipById.get(segment.clipId)
    return Math.max(0, clip
      ? clip.end_seconds - clip.start_seconds
      : segment.endSeconds - segment.startSeconds)
  }) ?? []
  const totalDuration = montageDuration(segmentDurations, transition)
  const selectedClip = selectedClipId ? clipById.get(selectedClipId) ?? null : null
  const selectedClipFullFile = !!selectedClip?.community_clip_mp4_path
  const selectedPreviewStart = selectedClipFullFile ? 0 : selectedClip?.start_seconds ?? 0
  const selectedPreviewEnd = selectedClipFullFile
    ? Math.max(0, (selectedClip?.end_seconds ?? 0) - (selectedClip?.start_seconds ?? 0))
    : selectedClip?.end_seconds ?? 0
  const segmentClipIds = project?.segments.map(segment => segment.clipId) ?? []
  const activeSegmentIndex = selectedClipId ? segmentClipIds.indexOf(selectedClipId) : -1
  const nextClipId = nextMontageClipId(segmentClipIds, selectedClipId)
  const nextClip = nextClipId ? clipById.get(nextClipId) ?? null : null
  const nextClipFullFile = !!nextClip?.community_clip_mp4_path
  const nextPreviewStart = nextClipFullFile ? 0 : nextClip?.start_seconds ?? 0
  const nextPreviewEnd = nextClipFullFile
    ? Math.max(0, (nextClip?.end_seconds ?? 0) - (nextClip?.start_seconds ?? 0))
    : nextClip?.end_seconds ?? 0
  const crossfadeDuration = montageCrossfadeDuration(segmentDurations)
  const currentPreviewKey = previewMode === 'export'
    ? exportResult ? `export:${exportResult.outputPath}` : null
    : selectedClip ? `clip:${selectedClip.id}` : null
  const previewSrc = previewMode === 'export'
    ? currentPreviewKey && previewMedia?.key === currentPreviewKey ? previewMedia.src : null
    : selectedClipId ? clipPreviewSources[selectedClipId] ?? null : null
  const nextPreviewSrc = nextClipId ? clipPreviewSources[nextClipId] ?? null : null
  const shortsOverLimit = project?.exportPreset === 'shorts' && exceedsYouTubeShortsLimit(totalDuration)
  const exportInputSignature = project
    ? `${project.id}|${project.exportPreset}|${transition}|${project.title}|${project.segments.map(segment => {
        const clip = clipById.get(segment.clipId)
        return `${segment.clipId}:${clip?.render_status || 'missing'}:${clip?.start_seconds || segment.startSeconds}:${clip?.end_seconds || segment.endSeconds}`
      }).join('|')}`
    : ''

  useEffect(() => {
    if (!project) return
    if (project.segments.length === 0) {
      setSelectedClipId(null)
      setPreviewMedia(null)
      setSequenceAutoPlay(false)
      setPreviewTransitioning(false)
      setCrossfadeProgress(0)
      transitionTargetRef.current = null
      return
    }
    if (!project.segments.some(segment => segment.clipId === selectedClipId)) {
      setSelectedClipId(project.segments[0].clipId)
      setPreviewMode('clip')
    }
  }, [project, selectedClipId])

  useEffect(() => {
    if (!project || loadedProjectIdRef.current === project.id) return
    loadedProjectIdRef.current = project.id
    if (project) {
      setPublishMeta({
        title: project.publishTitle || '',
        description: project.publishDescription || '',
        hashtags: project.publishHashtags || [],
        visibility: project.visibility || 'public',
      })
    }
    setExportResult(null)
    setExportError(null)
    setExportProgress(0)
    setExportStage('')
    setPreviewMode('clip')
    setSequenceAutoPlay(false)
    setPreviewTransitioning(false)
    setCrossfadeProgress(0)
    transitionTargetRef.current = null
  }, [project])

  useEffect(() => {
    setExportResult(null)
    setExportProgress(0)
    setExportStage('')
    setPreviewMode(mode => mode === 'export' ? 'clip' : mode)
    setSequenceAutoPlay(false)
    setPreviewTransitioning(false)
    setCrossfadeProgress(0)
    transitionTargetRef.current = null
  }, [exportInputSignature])

  useEffect(() => {
    if (transition !== 'cut') return
    transitionTargetRef.current = null
    setPreviewTransitioning(false)
    setCrossfadeProgress(0)
  }, [transition])

  const handlePublishMetaChange = (next: PublishMetadata) => {
    setPublishMeta(next)
    if (!project) return
    updateProject(project.id, {
      publishTitle: next.title,
      publishDescription: next.description,
      publishHashtags: next.hashtags,
      visibility: next.visibility,
    })
  }

  useEffect(() => {
    let unlisten: (() => void) | undefined
    listen<MontageProgress>('montage-export-progress', event => {
      if (event.payload.projectId !== activeProjectId) return
      setExportProgress(event.payload.progress)
      setExportStage(event.payload.stage)
    }).then(cleanup => { unlisten = cleanup })
    return () => { unlisten?.() }
  }, [activeProjectId])

  useEffect(() => {
    if (previewMode === 'export') {
      setPreviewLoading(false)
      setPreviewMedia(exportResult ? {
        key: `export:${exportResult.outputPath}`,
        src: convertFileSrc(exportResult.outputPath),
      } : null)
      setPreviewError(null)
      return
    }
    if (!selectedClip) {
      setPreviewLoading(false)
      setPreviewMedia(null)
      setPreviewError(null)
      return
    }

    let cancelled = false
    const candidates = [selectedClip, nextClip].filter((clip): clip is Clip => !!clip)
    const missing = candidates.filter(clip => !clipPreviewSourcesRef.current[clip.id])
    setPreviewLoading(!clipPreviewSourcesRef.current[selectedClip.id])
    setPreviewError(null)
    ;(async () => {
      const additions: Record<string, string> = {}
      try {
        for (const clip of missing) {
          try {
            additions[clip.id] = await resolveClipPreviewSource(clip)
          } catch (error) {
            if (clip.id === selectedClip.id && !cancelled) {
              setPreviewError(errorMessage(error, 'Could not load this clip preview'))
            }
          }
        }
      } finally {
        if (!cancelled && Object.keys(additions).length > 0) {
          clipPreviewSourcesRef.current = { ...clipPreviewSourcesRef.current, ...additions }
          setClipPreviewSources(clipPreviewSourcesRef.current)
        }
      }
      if (!cancelled) setPreviewLoading(false)
    })()
    return () => { cancelled = true }
  }, [exportResult, nextClip, previewMode, selectedClip])

  // Aggregate context from all clips in the montage for metadata generation
  const montageContext = (() => {
    if (!project) return { eventTags: [] as string[], emotionTags: [] as string[], clipTitles: [] as string[], game: undefined as string | undefined }

    const eventTags = new Set<string>()
    const emotionTags = new Set<string>()
    const gameCounts = new Map<string, number>()
    const clipTitles: string[] = []
    const events = ['chase', 'fight', 'kill', 'ambush', 'escape', 'jumpscare', 'encounter', 'scream']
    const emotions = ['shock', 'panic', 'hype', 'rage', 'frustration', 'relief', 'surprise']

    for (const seg of project.segments) {
      clipTitles.push(seg.clipTitle)
      // Find the clip's highlight to get tags
      const clip = clips.find(c => c.id === seg.clipId)
      if (clip) {
        const game = clip.game?.trim()
        if (game) gameCounts.set(game, (gameCounts.get(game) || 0) + 1)
        const hl = highlights.find(h => h.id === clip.highlight_id)
        if (hl) {
          const rawTags = hl.tags as unknown
          const tags: string[] = Array.isArray(rawTags) ? rawTags : typeof rawTags === 'string' ? (rawTags as string).split(',').map(t => t.trim()) : []
          for (const t of tags) {
            const lower = t.toLowerCase()
            if (events.some(e => lower.includes(e))) eventTags.add(lower)
            if (emotions.some(e => lower.includes(e))) emotionTags.add(lower)
          }
        }
      }
    }

    return {
      eventTags: [...eventTags],
      emotionTags: [...emotionTags],
      clipTitles,
      game: [...gameCounts.entries()].sort((left, right) => right[1] - left[1])[0]?.[0],
    }
  })()

  const handleAddClip = (clip: Clip) => {
    if (!project) return
    addClip(project.id, {
      clipId: clip.id,
      clipTitle: clip.title,
      startSeconds: clip.start_seconds,
      endSeconds: clip.end_seconds,
      thumbnailPath: clip.thumbnail_path,
    })
    if (project.segments.length === 0) setSelectedClipId(clip.id)
    setSequenceAutoPlay(false)
    setPreviewMode('clip')
  }

  const selectSequenceClip = (clipId: string) => {
    transitionTargetRef.current = null
    setPreviewTransitioning(false)
    setCrossfadeProgress(0)
    setSequenceAutoPlay(false)
    setSelectedClipId(clipId)
    setPreviewMode('clip')
  }

  const completeCrossfade = (targetClipId: string) => {
    setSelectedClipId(targetClipId)
    setSequenceAutoPlay(false)
    setPreviewTransitioning(false)
    setCrossfadeProgress(0)
    transitionTargetRef.current = null
  }

  const handleSequenceTimeUpdate = (absoluteTime: number) => {
    if (
      transition !== 'crossfade'
      || crossfadeDuration <= 0
      || !nextClipId
      || !nextPreviewSrc
      || !readyPreviewClipIds.includes(nextClipId)
    ) return

    const progress = montageCrossfadeProgress(absoluteTime, selectedPreviewEnd, crossfadeDuration)
    if (progress <= 0) return
    if (transitionTargetRef.current !== nextClipId) {
      transitionTargetRef.current = nextClipId
      setPreviewTransitioning(true)
    }
    setCrossfadeProgress(progress)
    if (progress >= 1) completeCrossfade(nextClipId)
  }

  const handleSequenceEnded = () => {
    if (nextClipId) {
      if (transition === 'crossfade' && transitionTargetRef.current === nextClipId) {
        completeCrossfade(nextClipId)
        return
      }
      setSequenceAutoPlay(true)
      setSelectedClipId(nextClipId)
      return
    }
    setSequenceAutoPlay(false)
    setPreviewTransitioning(false)
    setCrossfadeProgress(0)
    transitionTargetRef.current = null
    setSelectedClipId(segmentClipIds[0] ?? null)
  }

  const handleCreateProject = () => {
    const nextNumber = projects.length + 1
    createProject(nextNumber === 1 ? 'My Montage' : `My Montage ${nextNumber}`)
    setPublishMeta({ title: '', description: '', hashtags: [], visibility: 'public' })
  }

  const handleDeleteProject = () => {
    if (!project || !window.confirm(`Delete "${project.title || 'Untitled montage'}"? This cannot be undone.`)) return
    deleteProject(project.id)
  }

  const handleMoveUp = (idx: number) => {
    if (!project || idx <= 0) return
    reorderClips(project.id, idx, idx - 1)
  }

  const handleMoveDown = (idx: number) => {
    if (!project || idx >= project.segments.length - 1) return
    reorderClips(project.id, idx, idx + 1)
  }

  const handleExport = async () => {
    if (!project || project.segments.length === 0 || shortsOverLimit) return
    setPreviewTransitioning(false)
    setCrossfadeProgress(0)
    transitionTargetRef.current = null
    setExporting(true)
    setExportError(null)
    setExportResult(null)
    setExportProgress(1)
    setExportStage('Preparing montage')
    try {
      const result = await invoke<MontageExportResult>('export_montage', {
        projectId: project.id,
        title: publishMeta.title.trim() || project.title.trim() || 'ClipGoblin Montage',
        clipIds: project.segments.map(segment => segment.clipId),
        preset: project.exportPreset,
        transition,
      })
      setExportResult(result)
      setExportProgress(100)
      setExportStage('Montage ready')
      setSequenceAutoPlay(false)
      setPreviewTransitioning(false)
      setCrossfadeProgress(0)
      transitionTargetRef.current = null
      setPreviewMode('export')
    } catch (error) {
      setExportError(errorMessage(error, 'Montage export failed'))
      setExportStage('')
    } finally {
      setExporting(false)
    }
  }

  const handleOpenExportFolder = async () => {
    if (!exportResult) return
    try {
      await invoke('open_folder', { path: exportResult.outputDirectory })
    } catch (error) {
      setExportError(errorMessage(error, 'Could not open the export folder'))
    }
  }

  if (!project) {
    return <div className="flex items-center justify-center h-64"><p className="text-slate-400">Loading...</p></div>
  }

  // Available clips not yet in the montage, with source and title filtering.
  const normalizedSearch = clipSearch.trim().toLocaleLowerCase()
  const availableClips = filterAvailableMontageClips(
    clips,
    project.segments.map(segment => segment.clipId),
    sourceFilter,
    clipSearch,
  )
  const previewStart = previewMode === 'export' ? 0 : selectedPreviewStart
  const previewEnd = previewMode === 'export'
    ? exportResult?.durationSeconds || totalDuration
    : selectedPreviewEnd
  const previewPoster = previewMode === 'clip' && selectedClip?.thumbnail_path
    ? convertFileSrc(selectedClip.thumbnail_path)
    : null
  const nextPreviewPoster = nextClip?.thumbnail_path ? convertFileSrc(nextClip.thumbnail_path) : null
  const previewAspectClass = project.exportPreset === 'shorts'
    ? 'v4-montage-preview--shorts'
    : 'v4-montage-preview--landscape'
  const sequencePreviewLayers = previewMode === 'clip' && selectedClip && previewSrc
    ? [
        {
          clip: selectedClip,
          src: previewSrc,
          poster: previewPoster,
          start: selectedPreviewStart,
          end: selectedPreviewEnd,
          fullFile: selectedClipFullFile,
          current: true,
        },
        ...(nextClip && nextPreviewSrc ? [{
          clip: nextClip,
          src: nextPreviewSrc,
          poster: nextPreviewPoster,
          start: nextPreviewStart,
          end: nextPreviewEnd,
          fullFile: nextClipFullFile,
          current: false,
        }] : []),
      ]
    : []
  const outgoingAudioGain = previewTransitioning
    ? Math.cos(crossfadeProgress * Math.PI / 2)
    : 1
  const incomingAudioGain = previewTransitioning
    ? Math.sin(crossfadeProgress * Math.PI / 2)
    : 0

  return (
    <div className="space-y-6">
      <div className="v4-page-header">
        <div className="flex items-center gap-3">
          <button onClick={() => navigate('/clips')} className="p-2 rounded-lg bg-surface-800 hover:bg-surface-700 text-slate-400 hover:text-white transition-colors cursor-pointer">
            <ArrowLeft className="w-5 h-5" />
          </button>
          <div>
            <div className="v4-page-title">Montage Builder 🎬</div>
            <input type="text" value={project.title}
              disabled={exporting}
              onChange={e => updateProject(project.id, { title: e.target.value })}
              className="v4-page-sub bg-transparent border-none focus:outline-none w-full mt-1"
              placeholder="Untitled montage" />
          </div>
        </div>
        <div className="v4-page-actions">
          <select
            className="v4-input"
            aria-label="Active montage project"
            value={project.id}
            disabled={exporting}
            onChange={event => setActive(event.target.value)}
            style={{width: 170, padding: '7px 9px'}}
          >
            {projects.map(item => (
              <option key={item.id} value={item.id}>{item.title || 'Untitled montage'}</option>
            ))}
          </select>
          <button type="button" onClick={handleCreateProject} disabled={exporting} className="v4-btn" title="New montage">
            <Plus className="w-4 h-4" /> New
          </button>
          <button type="button" onClick={handleDeleteProject} disabled={exporting} className="v4-btn" title="Delete this montage">
            <Trash2 className="w-4 h-4" />
          </button>
          <div className="flex items-center gap-2 text-sm text-slate-400">
            <Film className="w-4 h-4" />
            <span>{project.segments.length} clips</span>
            <Clock className="w-4 h-4 ml-2" />
            <span>{fmt(totalDuration)}</span>
          </div>
        </div>
      </div>

      <div className="v4-montage-layout">
        {/* ── Left: Clip library ── */}
        <div className="v4-montage-col">
          <h4>Clip library · click to add</h4>
          <div className="relative mb-2">
            <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-slate-500" />
            <input
              className="v4-input w-full"
              aria-label="Search montage clips"
              value={clipSearch}
              onChange={event => setClipSearch(event.target.value)}
              placeholder="Search clips or games"
              style={{padding: '7px 8px 7px 30px', fontSize: 11}}
            />
          </div>
          <select
            className="v4-input mb-3"
            aria-label="Filter montage clips by source"
            value={sourceFilter}
            onChange={event => setSourceFilter(event.target.value as MontageSourceFilter)}
            style={{padding: '7px 8px', fontSize: 11}}
          >
            <option value="all">All sources</option>
            <option value="twitch">Twitch</option>
            <option value="medal">Medal</option>
            <option value="obs">OBS</option>
            <option value="meld">Meld</option>
            <option value="local">Local</option>
          </select>
          {availableClips.length === 0 ? (
            <p className="text-xs text-slate-500 py-4">
              {normalizedSearch || sourceFilter !== 'all'
                ? 'No available clips match these filters.'
                : 'All clips are already in this montage.'}
            </p>
          ) : (
            availableClips.map((clip, i) => (
              <button
                key={clip.id}
                disabled={exporting}
                onClick={() => handleAddClip(clip)}
                className="v4-lib-item w-full text-left"
              >
                <div className={`v4-lib-item-thumb v4-clip-thumb ${['a','b','c','d','e','f','g','h'][i % 8]}`}>
                  {clip.thumbnail_path && (
                    <img src={convertFileSrc(clip.thumbnail_path)} alt="" className="h-full w-full object-cover" />
                  )}
                </div>
                <div className="flex-1 min-w-0">
                  <div className="v4-lib-item-title">{clip.title || 'Untitled'}</div>
                  <div className="v4-lib-item-meta">
                    {montageSourceGroup(clip)} · {fmt(clip.end_seconds - clip.start_seconds)}
                  </div>
                </div>
              </button>
            ))
          )}
        </div>

        {/* ── Middle: Preview + Sequence + Timeline ── */}
        <div className="v4-montage-col">
          <div className="flex items-center justify-between gap-2">
            <h4>
              {previewMode === 'export'
                ? 'Finished montage'
                : project.segments.length > 0
                  ? `Montage preview · clip ${Math.max(1, activeSegmentIndex + 1)} of ${project.segments.length}`
                  : 'Montage preview'}
              {' '}· {fmt(totalDuration)} total
            </h4>
            {exportResult && (
              <button
                type="button"
                className="text-[10px] text-violet-300 hover:text-white cursor-pointer mb-3"
                onClick={() => {
                  setSequenceAutoPlay(false)
                  setPreviewTransitioning(false)
                  setCrossfadeProgress(0)
                  transitionTargetRef.current = null
                  setPreviewMode(mode => mode === 'export' ? 'clip' : 'export')
                }}
              >
                {previewMode === 'export' ? 'Show sequence preview' : 'Show finished montage'}
              </button>
            )}
          </div>
          <div className={`v4-montage-preview ${previewAspectClass} ${previewSrc ? 'has-media' : ''}`}>
            {previewMode === 'export' && previewSrc ? (
              <ClipPlayer
                key={currentPreviewKey || 'montage-preview'}
                src={previewSrc}
                poster={null}
                clipStart={previewStart}
                clipEnd={previewEnd}
                fullFile
                mode="full"
                controlsOverlay
                className="h-full w-full"
                objectFit="contain"
              />
            ) : sequencePreviewLayers.length > 0 ? (
              sequencePreviewLayers.map(layer => {
                const incomingActive = previewTransitioning && transitionTargetRef.current === layer.clip.id
                return (
                  <div
                    key={layer.clip.id}
                    className={`absolute inset-0 ${layer.current ? 'z-[2]' : 'pointer-events-none z-[3]'}`}
                    style={{ opacity: layer.current ? 1 : incomingActive ? crossfadeProgress : 0 }}
                  >
                    <ClipPlayer
                      src={layer.src}
                      poster={layer.poster}
                      clipStart={layer.start}
                      clipEnd={layer.end}
                      fullFile={layer.fullFile}
                      mode="full"
                      controlsOverlay={layer.current}
                      coordinatePlayback={false}
                      showControls={layer.current}
                      volumeMultiplier={layer.current ? outgoingAudioGain : incomingAudioGain}
                      className="h-full w-full"
                      objectFit="contain"
                      autoPlay={layer.current ? sequenceAutoPlay : incomingActive}
                      onReady={() => setReadyPreviewClipIds(current => current.includes(layer.clip.id)
                        ? current
                        : [...current, layer.clip.id])}
                      onPlayChange={layer.current ? playing => {
                        if (playing && sequenceAutoPlay) setSequenceAutoPlay(false)
                      } : undefined}
                      onTimeUpdate={layer.current ? handleSequenceTimeUpdate : undefined}
                      onEnded={layer.current ? handleSequenceEnded : undefined}
                    />
                  </div>
                )
              })
            ) : (
              <div className="relative z-[1] px-5 text-center text-xs text-slate-400">
                {project.segments.length === 0 ? 'Add a clip to begin.' : 'Preparing sequence preview.'}
              </div>
            )}
            {previewLoading && (
              <div className="absolute inset-0 z-20 flex items-center justify-center gap-2 bg-black/70 text-xs text-slate-200">
                <Loader2 className="h-4 w-4 animate-spin" /> Preparing preview
              </div>
            )}
            {previewError && !previewLoading && (
              <div className="absolute inset-x-3 bottom-3 z-20 rounded bg-red-950/90 px-3 py-2 text-xs text-red-200">
                {previewError}
              </div>
            )}
          </div>

          <h4 style={{marginTop: 12}}>Timeline · {project.segments.length} clips</h4>
          {project.segments.length === 0 ? (
            <div className="v4-tl-track">
              <span className="text-xs text-slate-500 px-2 py-3 w-full text-center">
                No clips yet — click a clip in the library to add it.
              </span>
            </div>
          ) : (
            <div className="v4-tl-track">
              {project.segments.map((seg, i) => (
                <button
                  type="button"
                  key={seg.clipId}
                  className={`v4-tl-clip v4-clip-thumb ${['a','b','c','d','e','f','g','h'][i % 8]} ${selectedClipId === seg.clipId && previewMode === 'clip' ? 'selected' : ''}`}
                  title={seg.clipTitle}
                  onClick={() => selectSequenceClip(seg.clipId)}
                >
                  <span className="relative z-[1]">{String.fromCharCode(65 + (i % 26))} · {fmt((clipById.get(seg.clipId)?.end_seconds || seg.endSeconds) - (clipById.get(seg.clipId)?.start_seconds || seg.startSeconds))}</span>
                </button>
              ))}
            </div>
          )}

          {project.segments.length > 0 && (
            <div className="mt-4 space-y-2">
              <h4>Sequence · use arrows to reorder</h4>
              {project.segments.map((seg, idx) => (
                <div
                  key={seg.clipId}
                  className={`flex items-center gap-3 p-2 rounded-lg group cursor-pointer ${selectedClipId === seg.clipId && previewMode === 'clip' ? 'bg-violet-500/10' : 'hover:bg-surface-800/60'}`}
                  onClick={() => selectSequenceClip(seg.clipId)}
                >
                  <div className="flex flex-col gap-0.5 shrink-0">
                    <button onClick={event => { event.stopPropagation(); handleMoveUp(idx) }} disabled={exporting || idx === 0}
                      className="text-slate-600 hover:text-white disabled:opacity-20 cursor-pointer text-[10px]">
                      ▲
                    </button>
                    <span className="text-[9px] text-slate-500 text-center font-mono">{idx + 1}</span>
                    <button onClick={event => { event.stopPropagation(); handleMoveDown(idx) }} disabled={exporting || idx === project.segments.length - 1}
                      className="text-slate-600 hover:text-white disabled:opacity-20 cursor-pointer text-[10px]">
                      ▼
                    </button>
                  </div>
                  <div className="flex-1 min-w-0">
                    <p className="text-xs text-white font-medium truncate">{seg.clipTitle || 'Untitled'}</p>
                    <p className="text-[10px] text-slate-500 font-mono">
                      {fmt(seg.endSeconds - seg.startSeconds)}
                    </p>
                  </div>
                  <button disabled={exporting} onClick={event => { event.stopPropagation(); navigate(`/editor/${seg.clipId}`) }}
                    className="text-[10px] text-slate-500 hover:text-violet-400 cursor-pointer px-2">
                    Edit
                  </button>
                  <button disabled={exporting} onClick={event => { event.stopPropagation(); removeClip(project.id, seg.clipId) }}
                    className="p-1 text-slate-600 hover:text-red-400 cursor-pointer opacity-0 group-hover:opacity-100 transition-opacity">
                    <Trash2 className="w-3.5 h-3.5" />
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* ── Right: YouTube details + Export ── */}
        <div className="v4-montage-col">
          <h4>Export format</h4>
          <div className="grid grid-cols-2 gap-1 rounded-lg border border-surface-700 bg-surface-900 p-1 mb-4" role="group" aria-label="Montage export format">
            {([
              ['youtube', 'YouTube 16:9'],
              ['shorts', 'Shorts 9:16'],
            ] as Array<[MontageExportPreset, string]>).map(([preset, label]) => (
              <button
                type="button"
                key={preset}
                disabled={exporting}
                className={`rounded-md px-2 py-2 text-[11px] font-medium cursor-pointer transition-colors ${project.exportPreset === preset ? 'bg-violet-600 text-white' : 'text-slate-400 hover:text-white'}`}
                onClick={() => updateProject(project.id, { exportPreset: preset })}
                aria-pressed={project.exportPreset === preset}
              >
                {label}
              </button>
            ))}
          </div>
          {project.exportPreset === 'shorts' && (
            <div className={`mb-4 flex gap-2 rounded-md border px-3 py-2 text-[11px] ${shortsOverLimit ? 'border-red-500/35 bg-red-950/35 text-red-200' : 'border-cyan-500/25 bg-cyan-950/25 text-cyan-100'}`}>
              {shortsOverLimit && <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />}
              <span>
                <b>{fmt(totalDuration)} / {fmt(YOUTUBE_SHORTS_MAX_SECONDS)}</b>
                {shortsOverLimit
                  ? ` — remove or trim ${fmt(totalDuration - YOUTUBE_SHORTS_MAX_SECONDS)} before exporting, or switch to YouTube 16:9.`
                  : ' — vertical Shorts preview and export.'}
              </span>
            </div>
          )}

          <h4>Clip transitions</h4>
          <div className="grid grid-cols-2 gap-1 rounded-lg border border-surface-700 bg-surface-900 p-1 mb-4" role="group" aria-label="Montage clip transitions">
            {([
              ['cut', 'Straight cut', Scissors],
              ['crossfade', 'Cross dissolve', Blend],
            ] as Array<[MontageTransition, string, typeof Scissors]>).map(([mode, label, Icon]) => (
              <button
                type="button"
                key={mode}
                disabled={exporting}
                className={`flex items-center justify-center gap-1.5 rounded-md px-2 py-2 text-[11px] font-medium cursor-pointer transition-colors ${transition === mode ? 'bg-violet-600 text-white' : 'text-slate-400 hover:text-white'}`}
                onClick={() => updateProject(project.id, { transition: mode })}
                aria-pressed={transition === mode}
                title={mode === 'crossfade' ? 'Blend picture and sound between clips' : 'Play clips back to back'}
              >
                <Icon className="h-3.5 w-3.5" /> {label}
              </button>
            ))}
          </div>

          <h4>Publishing copy</h4>
          <PublishComposer platform="youtube" metadata={publishMeta} onChange={handlePublishMetaChange}
            clipContext={{
              title: project.title,
              eventTags: montageContext.eventTags,
              emotionTags: montageContext.emotionTags,
              duration: totalDuration,
              game: montageContext.game,
              isMontage: true,
              clipCount: project.segments.length,
            }}
          />

          <button
            onClick={handleExport}
            disabled={project.segments.length === 0 || exporting || shortsOverLimit}
            className="v4-btn primary mt-3"
            style={{width: '100%', justifyContent: 'center', padding: '10px 16px'}}
          >
            {exporting ? <Loader2 className="w-4 h-4 animate-spin" /> : <Download className="w-4 h-4" />}
            {exporting ? exportStage || 'Exporting montage' : `Export Montage (${fmt(totalDuration)})`}
          </button>

          {(exporting || exportProgress > 0) && !exportError && (
            <div className="mt-3">
              <div className="h-1.5 overflow-hidden rounded-full bg-surface-700">
                <div className="h-full rounded-full bg-violet-500 transition-[width]" style={{width: `${exportProgress}%`}} />
              </div>
              <p className="mt-1.5 text-[10px] text-slate-500">{exportStage} · {exportProgress}%</p>
            </div>
          )}

          {exportError && (
            <div className="mt-3 flex gap-2 rounded-lg border border-red-500/30 bg-red-950/40 p-3 text-xs text-red-200">
              <AlertCircle className="h-4 w-4 shrink-0" />
              <span>{exportError}</span>
            </div>
          )}

          {exportResult && !exporting && (
            <div className="mt-3 rounded-lg border border-emerald-500/30 bg-emerald-950/30 p-3">
              <div className="flex items-center gap-2 text-xs font-medium text-emerald-300">
                <CheckCircle2 className="h-4 w-4" /> Montage exported
              </div>
              <p className="mt-1 truncate text-[10px] text-slate-500" title={exportResult.outputPath}>{exportResult.outputPath}</p>
              <button type="button" onClick={handleOpenExportFolder} className="v4-btn mt-2 w-full justify-center" style={{padding: '7px 10px', fontSize: 11}}>
                <FolderOpen className="h-3.5 w-3.5" /> Open export folder
              </button>
            </div>
          )}
        </div>
      </div>

    </div>
  )
}
