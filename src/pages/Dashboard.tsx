import { useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import {
  Tv,
  Video,
  Sparkles,
  Scissors,
  Plus,
  FolderOpen,
  Clock,
  Upload,
} from 'lucide-react'
import { useAppStore } from '../stores/appStore'
import { formatConfidence } from '../lib/uiFormat'
import heroGoblinImg from '../assets/hero-goblin-v2.png'
import { useScheduleStore } from '../stores/scheduleStore'
import { PLATFORM_INFO } from '../stores/platformStore'

// Compress legacy virality_score to calibrated confidence.
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

export default function Dashboard() {
  const { channels, vods, highlights, clips, checkLogin, fetchHighlights, fetchClips } =
    useAppStore()
  const { uploads: scheduledUploads, load: loadSchedules } = useScheduleStore()
  const navigate = useNavigate()

  useEffect(() => {
    checkLogin()
    fetchHighlights()
    fetchClips()
    loadSchedules()
  }, [checkLogin, fetchHighlights, fetchClips, loadSchedules])

  const pendingScheduled = scheduledUploads.filter(u => u.status === 'pending' || u.status === 'uploading')
  const clipMap = Object.fromEntries(clips.map(c => [c.id, c]))

  const stats = [
    { label: 'Total Channels', value: channels.length, icon: Tv, color: 'text-violet-400' },
    { label: 'VODs Analyzed', value: vods.filter((v) => v.analysis_status === 'completed').length, icon: Video, color: 'text-blue-400' },
    { label: 'Highlights Found', value: highlights.length, icon: Sparkles, color: 'text-amber-400' },
    { label: 'Clips Created', value: clips.length, icon: Scissors, color: 'text-emerald-400' },
    ...(pendingScheduled.length > 0 ? [{ label: 'Scheduled', value: pendingScheduled.length, icon: Clock, color: 'text-amber-400' }] : []),
  ]

  const topHighlights = [...highlights]
    .sort((a, b) => b.virality_score - a.virality_score)
    .slice(0, 5)

  return (
    <div className="space-y-6">
      {/* ── TOP SECTION: goblin floats behind stats ── */}
      <div className="dashboard-top">
        <img className="hero-goblin-bg" src={heroGoblinImg} alt="" />
        <div className="space-y-5" style={{ position: 'relative', zIndex: 1, maxWidth: '65%' }}>
          <h1 className="text-3xl font-bold text-white">Dashboard</h1>
          <div className="grid grid-cols-2 gap-4">
            {stats.map(({ label, value, icon: Icon, color }) => (
              <div key={label} className="glass-card p-5 flex items-center gap-4">
                <div className={`p-2.5 rounded-lg bg-surface-700 ${color}`}>
                  <Icon className="w-5 h-5" />
                </div>
                <div>
                  <p className="text-2xl font-bold text-white">{value}</p>
                  <p className="text-sm text-slate-400">{label}</p>
                </div>
              </div>
            ))}
          </div>
        </div>
      </div>

      {/* ── BOTTOM SECTION: full-width highlights table ── */}
      <section>
        <h2 className="text-lg font-semibold text-white mb-4">Recent Highlights</h2>
        {topHighlights.length === 0 ? (
          <div className="glass-card p-8 text-center">
            <Sparkles className="w-10 h-10 text-slate-600 mx-auto mb-3" />
            <p className="text-slate-400">No highlights yet. Analyze some VODs to find highlights.</p>
          </div>
        ) : (
          <div className="glass-card divide-y divide-surface-700">
            {topHighlights.map((h) => (
              <div key={h.id} className="flex items-center justify-between px-5 py-4">
                <div className="flex-1 min-w-0">
                  <p className="text-sm text-white font-medium truncate">
                    {h.description || h.transcript_snippet || 'Untitled highlight'}
                  </p>
                  <p className="text-xs text-slate-500 mt-0.5">
                    {Math.floor(h.start_seconds / 60)}:{String(Math.floor(h.start_seconds % 60)).padStart(2, '0')}
                    {' - '}
                    {Math.floor(h.end_seconds / 60)}:{String(Math.floor(h.end_seconds % 60)).padStart(2, '0')}
                  </p>
                </div>
                <div className="flex items-center gap-3 ml-4">
                  {(() => {
                    const score = h.confidence_score ?? legacyToConfidence(h.virality_score)
                    const conf = formatConfidence(score)
                    return (
                      <span className={`text-xs font-semibold px-2.5 py-1 rounded-full border border-surface-600 ${conf.color}`}>
                        {conf.text} ({Math.round(score * 100)}%)
                      </span>
                    )
                  })()}
                </div>
              </div>
            ))}
          </div>
        )}
      </section>

      {/* ── Scheduled Uploads ── */}
      {pendingScheduled.length > 0 && (
        <section>
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-lg font-semibold text-white">Upcoming Scheduled Uploads</h2>
            <button
              onClick={() => navigate('/scheduled')}
              className="text-xs text-violet-400 hover:text-violet-300 transition-colors cursor-pointer"
            >
              View All
            </button>
          </div>
          <div className="glass-card divide-y divide-surface-700">
            {pendingScheduled.slice(0, 5).map(u => {
              const clip = clipMap[u.clip_id]
              const platformInfo = PLATFORM_INFO[u.platform]
              return (
                <div key={u.id} className="flex items-center justify-between px-5 py-3">
                  <div className="flex-1 min-w-0">
                    <p className="text-sm text-white font-medium truncate">{clip?.title || u.clip_id}</p>
                    <p className="text-xs text-slate-500 mt-0.5">
                      {platformInfo?.name || u.platform} &middot; {new Date(u.scheduled_time).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })}
                    </p>
                  </div>
                  <span className="flex items-center gap-1.5 text-xs px-2.5 py-1 rounded-full border border-surface-600 text-amber-300">
                    <Upload className="w-3 h-3" />
                    Scheduled
                  </span>
                </div>
              )
            })}
          </div>
        </section>
      )}
    </div>
  )
}
            