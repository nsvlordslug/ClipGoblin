import { useState, useEffect, useMemo } from 'react'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft, Plus, Trash2, Download, Film, Clock } from 'lucide-react'
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
  const [showClipPicker, setShowClipPicker] = useState(false)
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
      {/* Header */}
      <div className="flex items-center gap-4">
        <button onClick={() => navigate('/clips')} className="p-2 rounded-lg bg-surface-800 hover:bg-surface-700 text-slate-400 hover:text-white transition-colors cursor-pointer">
          <ArrowLeft className="w-5 h-5" />
        </button>
        <div className="flex-1">
          <input type="text" value={project.title}
            onChange={e => updateProject(project.id, { title: e.target.value })}
            className="text-2xl font-bold text-white bg-transparent border-none focus:outline-none w-full" />
        </div>
        <div className="flex items-center gap-2 text-sm text-slate-400">
          <Film className="w-4 h-4" />
          <span>{project.segments.length} clips</span>
          <Clock className="w-4 h-4 ml-2" />
          <span>{fmt(totalDuration)}</span>
        </div>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* ── Left: Clip sequence ── */}
        <div className="lg:col-span-2 space-y-4">
          <div className="bg-surface-800 border border-surface-700 rounded-xl p-4">
            <div className="flex items-center justify-between mb-3">
              <h2 className="text-sm font-semibold text-slate-300">Clip Sequence</h2>
              <button onClick={() => setShowClipPicker(true)}
                className="flex items-center gap-1 px-2 py-1 bg-violet-600/20 border border-violet-500/40 rounded text-xs text-violet-400 hover:bg-violet-600/30 transition-colors cursor-pointer">
                <Plus className="w-3 h-3" /> Add Clip
              </button>
            </div>

            {project.segments.length === 0 ? (
              <div className="py-8 text-center">
                <Film className="w-8 h-8 text-slate-600 mx-auto mb-2" />
                <p className="text-sm text-slate-500">No clips added yet</p>
                <p className="text-xs text-slate-600 mt-1">Click "Add Clip" to start building your montage</p>
              </div>
            ) : (
              <div className="space-y-2">
                {project.segments.map((seg, idx) => (
                  <div key={seg.clipId} className="flex items-center gap-3 p-3 bg-surface-900 border border-surface-600 rounded-lg group">
                    {/* Order controls */}
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

                    {/* Clip info */}
                    <div className="flex-1 min-w-0">
                      <p className="text-xs text-white font-medium truncate">{seg.clipTitle || 'Untitled'}</p>
                      <p className="text-[10px] text-slate-500 font-mono">
                        {fmt(seg.endSeconds - seg.startSeconds)} duration
                      </p>
                    </div>

                    {/* Actions */}
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

            {/* Duration bar */}
            {project.segments.length > 0 && (
              <div className="mt-4 flex items-center gap-2">
                <div className="flex-1 h-2 bg-surface-700 rounded-full overflow-hidden flex">
                  {project.segments.map((seg, i) => {
                    const dur = seg.endSeconds - seg.startSeconds
                    const pct = totalDuration > 0 ? (dur / totalDuration) * 100 : 0
                    const colors = ['bg-violet-500', 'bg-blue-500', 'bg-emerald-500', 'bg-amber-500', 'bg-rose-500']
                    return <div key={i} className={`h-full ${colors[i % colors.length]}`} style={{ width: `${pct}%` }} />
                  })}
                </div>
                <span className="text-[10px] text-slate-500 font-mono shrink-0">{fmt(totalDuration)}</span>
              </div>
            )}
          </div>
        </div>

        {/* ── Right: Metadata + Export ── */}
        <div className="space-y-4">
          <div className="bg-surface-800 border border-surface-700 rounded-xl p-4">
            <h2 className="text-sm font-semibold text-slate-300 mb-3">YouTube Details</h2>
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
          </div>

          <button onClick={handleExport} disabled={project.segments.length === 0}
            className="w-full flex items-center justify-center gap-2 px-4 py-3 bg-violet-600 hover:bg-violet-500 disabled:opacity-40 text-white text-sm font-medium rounded-lg transition-colors cursor-pointer">
            <Download className="w-4 h-4" />
            Export Montage ({fmt(totalDuration)})
          </button>
        </div>
      </div>

      {/* ── Clip picker modal ── */}
      {showClipPicker && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
          onClick={e => { if (e.target === e.currentTarget) setShowClipPicker(false) }}>
          <div className="bg-surface-800 border border-surface-600 rounded-2xl shadow-2xl w-full max-w-md max-h-[70vh] overflow-hidden flex flex-col">
            <div className="flex items-center justify-between px-5 py-4 border-b border-surface-700">
              <h2 className="text-base font-semibold text-white">Add Clips to Montage</h2>
              <button onClick={() => setShowClipPicker(false)} className="text-slate-400 hover:text-white cursor-pointer text-lg">
                ×
              </button>
            </div>
            <div className="flex-1 overflow-y-auto p-4 space-y-2">
              {availableClips.length === 0 ? (
                <p className="text-sm text-slate-500 text-center py-6">All clips are already in the montage</p>
              ) : (
                availableClips.map(clip => (
                  <button key={clip.id}
                    onClick={() => { handleAddClip(clip); setShowClipPicker(false) }}
                    className="w-full flex items-center gap-3 p-3 bg-surface-900 border border-surface-600 rounded-lg hover:border-violet-500/40 transition-colors cursor-pointer text-left">
                    <Plus className="w-4 h-4 text-violet-400 shrink-0" />
                    <div className="flex-1 min-w-0">
                      <p className="text-xs text-white font-medium truncate">{clip.title}</p>
                      <p className="text-[10px] text-slate-500 font-mono">{fmt(clip.end_seconds - clip.start_seconds)}</p>
                    </div>
                  </button>
                ))
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
