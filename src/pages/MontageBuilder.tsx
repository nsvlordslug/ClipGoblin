import { useState, useEffect, useMemo } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft, Trash2, Download, Film, Clock } from 'lucide-react'
import { useMontageStore } from '../stores/montageStore'
import { useAppStore } from '../stores/appStore'
import type { Clip } from '../types'
import PublishComposer from '../components/PublishComposer'
import type { PublishMetadata } from '../components/PublishComposer'

function fmt(s: number) {
  const m = Math.floor(s / 60)
  const sec = Math.floor(s % 60)
  return `${m}:${String(sec).padStart(2, '0')}`
}

export default function MontageBuilder() {
  const navigate = useNavigate()
  const { projects, activeProjectId, createProject, setActive, addClip, removeClip, reorderClips, updateProject } = useMontageStore()
  const { clips, highlights, fetchClips, fetchHighlights } = useAppStore()
  const [publishMeta, setPublishMeta] = useState<PublishMetadata>({
    title: '', description: '', hashtags: [], visibility: 'public',
  })

  useEffect(() => { fetchClips(); fetchHighlights() }, [fetchClips, fetchHighlights])

  // Auto-create a project if none exists
  useEffect(() => {
    if (projects.length === 0) {
      createProject('My Montage')
    } else if (!activeProjectId) {
      setActive(projects[0].id)
    }
  }, [projects.length])

  const project = projects.find(p => p.id === activeProjectId)

  const totalDuration = project?.segments.reduce((sum, s) => sum + (s.endSeconds - s.startSeconds), 0) || 0

  // Aggregate context from all clips in the montage for metadata generation
  const montageContext = useMemo(() => {
    if (!project) return { eventTags: [] as string[], emotionTags: [] as string[], clipTitles: [] as string[], game: undefined as string | undefined }

    const eventTags = new Set<string>()
    const emotionTags = new Set<string>()
    const clipTitles: string[] = []
    const events = ['chase', 'fight', 'kill', 'ambush', 'escape', 'jumpscare', 'encounter', 'scream']
    const emotions = ['shock', 'panic', 'hype', 'rage', 'frustration', 'relief', 'surprise']

    for (const seg of project.segments) {
      clipTitles.push(seg.clipTitle)
      // Find the clip's highlight to get tags
      const clip = clips.find(c => c.id === seg.clipId)
      if (clip) {
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
      game: undefined, // TODO: detect from VOD metadata
    }
  }, [project?.segments, clips, highlights])

  const handleAddClip = (clip: Clip) => {
    if (!project) return
    addClip(project.id, {
      clipId: clip.id,
      clipTitle: clip.title,
      startSeconds: clip.start_seconds,
      endSeconds: clip.end_seconds,
      thumbnailPath: clip.thumbnail_path,
    })
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
    if (!project || project.segments.length === 0) return
    // TODO: Call backend montage export command
    // For now, show what would be exported
    alert(
      `Montage Export:\n\n` +
      `Title: ${publishMeta.title || project.title}\n` +
      `Clips: ${project.segments.length}\n` +
      `Duration: ${fmt(totalDuration)}\n` +
      `Tags: ${publishMeta.hashtags.map(t => '#' + t).join(' ') || 'none'}\n\n` +
      `Montage concatenation export is coming in a future update.`
    )
  }

  if (!project) {
    return <div className="flex items-center justify-center h-64"><p className="text-slate-400">Loading...</p></div>
  }

  // Available clips not yet in the montage
  const availableClips = clips.filter(c => !project.segments.some(s => s.clipId === c.id))

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
              onChange={e => updateProject(project.id, { title: e.target.value })}
              className="v4-page-sub bg-transparent border-none focus:outline-none w-full mt-1"
              placeholder="Untitled montage" />
          </div>
        </div>
        <div className="v4-page-actions">
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
          {availableClips.length === 0 ? (
            <p className="text-xs text-slate-500 py-4">All clips are already in this montage.</p>
          ) : (
            availableClips.map((clip, i) => (
              <button
                key={clip.id}
                onClick={() => handleAddClip(clip)}
                className="v4-lib-item w-full text-left"
              >
                <div className={`v4-lib-item-thumb v4-clip-thumb ${['a','b','c','d','e','f','g','h'][i % 8]}`} />
                <div className="flex-1 min-w-0">
                  <div className="v4-lib-item-title">{clip.title || 'Untitled'}</div>
                  <div className="v4-lib-item-meta">
                    {fmt(clip.end_seconds - clip.start_seconds)}
                  </div>
                </div>
              </button>
            ))
          )}
        </div>

        {/* ── Middle: Preview + Sequence + Timeline ── */}
        <div className="v4-montage-col">
          <h4>Preview · {fmt(totalDuration)} total</h4>
          <div className="v4-montage-preview">
            <div className="v4-play-big">▶</div>
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
                <div
                  key={seg.clipId}
                  className={`v4-tl-clip v4-clip-thumb ${['a','b','c','d','e','f','g','h'][i % 8]}`}
                  title={seg.clipTitle}
                >
                  {String.fromCharCode(65 + (i % 26))} · {fmt(seg.endSeconds - seg.startSeconds)}
                </div>
              ))}
            </div>
          )}

          {project.segments.length > 0 && (
            <div className="mt-4 space-y-2">
              <h4>Sequence · drag to reorder</h4>
              {project.segments.map((seg, idx) => (
                <div key={seg.clipId} className="flex items-center gap-3 p-2 rounded-lg group hover:bg-surface-800/60">
                  <div className="flex flex-col gap-0.5 shrink-0">
                    <button onClick={() => handleMoveUp(idx)} disabled={idx === 0}
                      className="text-slate-600 hover:text-white disabled:opacity-20 cursor-pointer text-[10px]">
                      ▲
                    </button>
                    <span className="text-[9px] text-slate-500 text-center font-mono">{idx + 1}</span>
                    <button onClick={() => handleMoveDown(idx)} disabled={idx === project.segments.length - 1}
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
                  <button onClick={() => navigate(`/editor/${seg.clipId}`)}
                    className="text-[10px] text-slate-500 hover:text-violet-400 cursor-pointer px-2">
                    Edit
                  </button>
                  <button onClick={() => removeClip(project.id, seg.clipId)}
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
          <h4>YouTube details</h4>
          <PublishComposer platform="youtube" metadata={publishMeta} onChange={setPublishMeta}
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
            disabled={project.segments.length === 0}
            className="v4-btn primary mt-3"
            style={{width: '100%', justifyContent: 'center', padding: '10px 16px'}}
          >
            <Download className="w-4 h-4" />
            Export Montage ({fmt(totalDuration)})
          </button>
        </div>
      </div>

    </div>
  )
}
