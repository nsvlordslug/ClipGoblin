import { useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useAppStore } from '../stores/appStore'
import { useScheduleStore } from '../stores/scheduleStore'
import { PLATFORM_INFO } from '../stores/platformStore'
import ImportVodDialog from '../components/ImportVodDialog'
import TesterChecklist from '../components/TesterChecklist'
import { formatViewerCount } from '../hooks/useStreamStatus'
import heroGoblinImg from '../assets/hero-goblin-v2.png'

interface AutoShipReport {
  clips_queued: number
  platforms: string[]
  next_publish_at: string | null
}

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

function fmtTime(secs: number): string {
  const m = Math.floor(secs / 60)
  const s = Math.floor(secs % 60)
  return `${m}:${String(s).padStart(2, '0')}`
}

function parseTags(tags: unknown): string[] {
  if (!tags) return []
  if (Array.isArray(tags)) return tags as string[]
  if (typeof tags !== 'string') return []
  const s = tags.trim()
  if (!s) return []
  if (s.startsWith('[')) {
    try { const parsed = JSON.parse(s); return Array.isArray(parsed) ? parsed : [] } catch { return [] }
  }
  return s.split(',').map(t => t.trim()).filter(Boolean)
}

function fmtCountdown(targetISO: string): string {
  const diff = new Date(targetISO).getTime() - Date.now()
  if (diff <= 0) return '0:00:00'
  const h = Math.floor(diff / 3_600_000)
  const m = Math.floor((diff % 3_600_000) / 60_000)
  const s = Math.floor((diff % 60_000) / 1000)
  return `${h}h ${String(m).padStart(2, '0')}m ${String(s).padStart(2, '0')}s`
}

const THUMB_STYLES = ['a', 'b', 'c', 'd', 'e', 'f'] as const

type WorkbenchTab = 'review' | 'ready' | 'scheduled' | 'published' | 'failed'

export default function Dashboard() {
  const { channels, vods, highlights, clips, checkLogin, fetchHighlights, fetchClips, fetchVods, loggedInUser } =
    useAppStore()
  const { uploads: scheduledUploads, load: loadSchedules } = useScheduleStore()
  const navigate = useNavigate()
  const [tab, setTab] = useState<WorkbenchTab>('review')
  const [, setTick] = useState(0)
  const [hunting, setHunting] = useState(false)
  const [huntStatus, setHuntStatus] = useState<string | null>(null)
  const [showImportDialog, setShowImportDialog] = useState(false)
  const [autoShipReport, setAutoShipReport] = useState<AutoShipReport | null>(null)

  useEffect(() => {
    checkLogin()
    fetchHighlights()
    fetchClips()
    loadSchedules()
  }, [checkLogin, fetchHighlights, fetchClips, loadSchedules])

  // Live countdown tick
  useEffect(() => {
    const id = setInterval(() => setTick(t => t + 1), 1000)
    return () => clearInterval(id)
  }, [])

  // Listen for auto-ship events emitted by the Rust backend after analysis.
  useEffect(() => {
    let unlisten: (() => void) | undefined
    listen<AutoShipReport>('auto-ship-queued', (event) => {
      setAutoShipReport(event.payload)
      // Keep the banner visible for 30s; user can also dismiss it manually.
      setTimeout(() => setAutoShipReport(prev => prev === event.payload ? null : prev), 30_000)
      loadSchedules().catch(() => {})
    }).then(fn => { unlisten = fn }).catch(() => {})
    return () => { unlisten?.() }
  }, [loadSchedules])

  const vodsAnalyzing = vods.filter(v => v.analysis_status === 'analyzing')
  const vodsQueued = vods.filter(v => v.analysis_status === 'pending')
  const vodsComplete = vods.filter(v => v.analysis_status === 'completed')

  const pendingScheduled = scheduledUploads.filter(u => u.status === 'pending' || u.status === 'uploading')
  const failedScheduled = scheduledUploads.filter(u => u.status === 'failed')
  const completedScheduled = scheduledUploads.filter(u => u.status === 'completed')

  const clipMap = useMemo(() => Object.fromEntries(clips.map(c => [c.id, c])), [clips])
  const vodMap = useMemo(() => Object.fromEntries(vods.map(v => [v.id, v])), [vods])

  // Categorize highlights / clips for the 5 workbench tabs
  const reviewHighlights = [...highlights]
    .filter(h => {
      const score = h.confidence_score ?? legacyToConfidence(h.virality_score)
      return score < 0.85
    })
    .sort((a, b) => (b.confidence_score ?? b.virality_score) - (a.confidence_score ?? a.virality_score))
    .slice(0, 6)

  const readyClips = clips
    .filter(c => c.render_status === 'completed')
    .slice(0, 6)

  // Scrubber: most recent VOD with highlights
  const scrubberVod = vodsAnalyzing[0] ?? vodsComplete[0]
  const scrubberHighlights = scrubberVod
    ? highlights.filter(h => h.vod_id === scrubberVod.id)
    : []

  // Next scheduled
  const nextScheduled = [...pendingScheduled]
    .sort((a, b) => new Date(a.scheduled_time).getTime() - new Date(b.scheduled_time).getTime())[0]

  // Pipeline counts
  const pipelineSteps = [
    {
      key: 'vods', icon: '📺',
      label: 'VODs in queue',
      count: vodsAnalyzing.length + vodsQueued.length,
      sub: vodsAnalyzing.length
        ? `${vodsAnalyzing.length} processing · ${vodsQueued.length} queued`
        : 'idle',
      subClass: vodsAnalyzing.length ? 'warn' : '',
      iconBg: 'rgba(96,165,250,0.15)', iconColor: '#60a5fa',
      onClick: () => navigate('/vods'),
    },
    {
      key: 'highlights', icon: '✨',
      label: 'Highlights',
      count: highlights.length,
      sub: reviewHighlights.length ? `${reviewHighlights.length} need review` : 'all reviewed',
      subClass: reviewHighlights.length ? 'good' : '',
      iconBg: 'rgba(251,191,36,0.15)', iconColor: '#fbbf24',
    },
    {
      key: 'clips', icon: '✂',
      label: 'Clips ready',
      count: readyClips.length,
      sub: 'captions + titles set',
      subClass: '',
      iconBg: 'rgba(167,139,250,0.15)', iconColor: '#a78bfa',
      onClick: () => navigate('/clips'),
    },
    {
      key: 'scheduled', icon: '🕒',
      label: 'Scheduled',
      count: pendingScheduled.length,
      sub: nextScheduled
        ? `next in ${fmtCountdown(nextScheduled.scheduled_time).split(' ').slice(0,2).join(' ')}`
        : 'none queued',
      subClass: '',
      iconBg: 'rgba(244,114,182,0.15)', iconColor: '#f472b6',
      onClick: () => navigate('/scheduled'),
    },
    {
      key: 'published', icon: '🚀',
      label: 'Published',
      count: completedScheduled.length,
      sub: completedScheduled.length ? 'auto-shipped' : '—',
      subClass: completedScheduled.length ? 'good' : '',
      iconBg: 'rgba(74,222,128,0.15)', iconColor: '#4ade80',
    },
  ]

  const tabCounts = {
    review: reviewHighlights.length,
    ready: readyClips.length,
    scheduled: pendingScheduled.length,
    published: completedScheduled.length,
    failed: failedScheduled.length,
  }

  const greetingName = loggedInUser?.display_name || channels[0]?.display_name || 'streamer'

  /** Refresh VODs across every connected channel, then queue analysis on any
   *  VOD that is downloaded but not yet analyzed. Fire-and-forget. */
  const runAutoHunt = async () => {
    if (hunting) return
    setHunting(true)
    setHuntStatus('Refreshing VODs...')
    try {
      const targets = loggedInUser ? [loggedInUser] : channels
      if (targets.length === 0) {
        setHuntStatus('Connect Twitch first')
        navigate('/settings')
        return
      }
      for (const ch of targets) {
        try { await fetchVods(ch.id) } catch { /* keep hunting other channels */ }
      }
      // Re-read store after fetches
      const freshVods = useAppStore.getState().vods
      const candidates = freshVods.filter(v =>
        v.download_status === 'downloaded' &&
        v.analysis_status !== 'analyzing' &&
        v.analysis_status !== 'completed'
      )
      if (candidates.length === 0) {
        setHuntStatus('No new VODs to analyze')
        setTimeout(() => setHuntStatus(null), 2500)
        return
      }
      setHuntStatus(`Queuing ${candidates.length} VOD${candidates.length !== 1 ? 's' : ''}...`)
      for (const v of candidates) {
        try { await invoke('analyze_vod', { vodId: v.id }) } catch { /* best-effort */ }
      }
      setHuntStatus(`Hunt started · ${candidates.length} analyzing`)
      setTimeout(() => setHuntStatus(null), 3500)
    } finally {
      setHunting(false)
    }
  }

  return (
    <div className="space-y-4">
      {/* Tester checklist — auto-hides once everything's done or dismissed. */}
      <TesterChecklist />

      {/* Auto-ship banner — appears after analysis when high-confidence clips were queued. */}
      {autoShipReport && autoShipReport.clips_queued > 0 && (
        <div
          className="v4-tip flex items-center gap-3"
          style={{background: 'linear-gradient(135deg, rgba(167,139,250,0.12), rgba(244,114,182,0.08))',
                  borderColor: 'rgba(167,139,250,0.3)', color: '#e9e6f2'}}
        >
          <span className="text-xl">🤖</span>
          <div className="flex-1">
            <div className="font-semibold text-white">
              Auto-shipped {autoShipReport.clips_queued} clip{autoShipReport.clips_queued !== 1 ? 's' : ''}
            </div>
            <div className="text-xs text-slate-300 mt-0.5">
              Queued to <b>{autoShipReport.platforms.join(', ')}</b>. The scheduler
              will render any missing exports automatically before uploading.
              You have 5 minutes to cancel from <a onClick={() => navigate('/scheduled')} className="text-violet-300 underline cursor-pointer">Scheduled</a>.
            </div>
          </div>
          <button
            onClick={() => setAutoShipReport(null)}
            className="text-slate-400 hover:text-white cursor-pointer text-lg leading-none px-1"
            aria-label="Dismiss"
          >×</button>
        </div>
      )}

      {/* ═════ HERO BANNER ═════ */}
      <section className="v4-hero-banner">
        <div className="v4-hero-goblin">
          <img src={heroGoblinImg} alt="ClipGoblin" />
        </div>
        <div className="v4-hero-content">
          <span className="v4-hero-eyebrow">
            ● {vodsAnalyzing.length > 0
              ? `LIVE · ${highlights.length} HIGHLIGHTS READY`
              : highlights.length > 0
                ? `${highlights.length} HIGHLIGHTS READY`
                : 'READY TO HUNT'}
          </span>
          <h1 className="text-[26px] font-extrabold tracking-tight mb-1.5 text-white" style={{lineHeight:'1.2'}}>
            Welcome back, {greetingName} 👋
          </h1>
          <p className="text-slate-400 text-sm mb-3.5 max-w-[540px] leading-relaxed">
            {vodsAnalyzing.length > 0 && (
              <>A VOD is analyzing · <b className="text-white">{vodsAnalyzing[0].title}</b>. </>
            )}
            {reviewHighlights.length > 0 && (
              <>You have <b className="text-white">{reviewHighlights.length} highlights</b> waiting for review. </>
            )}
            {readyClips.length > 0 && (
              <><b className="text-white">{readyClips.length} clips ready</b> to ship.</>
            )}
            {highlights.length === 0 && 'Connect Twitch and import a VOD to start finding clips.'}
          </p>
          <div className="flex gap-2.5 flex-wrap items-center">
            <button
              onClick={runAutoHunt}
              disabled={hunting}
              className="px-4 py-2.5 rounded-[10px] font-semibold text-sm bg-gradient-to-br from-violet-500 to-pink-500 text-white shadow-[0_4px_16px_rgba(139,92,246,0.35)] cursor-pointer disabled:opacity-60 disabled:cursor-wait"
            >
              {hunting ? '⏳ Hunting...' : '⚡ Auto-Hunt New VODs'}
            </button>
            {/* Dev-only — see Vods.tsx Import VOD button comment for rationale. */}
            {import.meta.env.DEV && (
              <button
                onClick={() => setShowImportDialog(true)}
                className="px-4 py-2.5 rounded-[10px] font-semibold text-sm bg-surface-800 border border-surface-700 text-white cursor-pointer"
              >
                📥 Import VOD
              </button>
            )}
            <button
              onClick={() => navigate('/montage')}
              className="px-4 py-2.5 rounded-[10px] font-semibold text-sm bg-surface-800 border border-surface-700 text-white cursor-pointer"
            >
              🎬 Build Montage
            </button>
            {huntStatus && (
              <span className="text-xs text-slate-400 ml-1">{huntStatus}</span>
            )}
          </div>
        </div>
      </section>

      {/* ═════ PIPELINE ═════ */}
      <section className="v4-pipeline">
        <div className="v4-pipeline-head">
          <div className="v4-pipeline-title">Today's Pipeline</div>
          {vodsAnalyzing[0] && (
            <span
              className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-[11px] font-semibold"
              style={{background:'rgba(74,222,128,0.12)',border:'1px solid rgba(74,222,128,0.3)',color:'#4ade80'}}
            >
              <span className="w-1.5 h-1.5 rounded-full bg-green-400 shadow-[0_0_6px_#4ade80] animate-pulse" />
              Analyzing VOD · {vodsAnalyzing[0].analysis_progress}%
            </span>
          )}
        </div>

        <div className="v4-pipeline-flow">
          {pipelineSteps.map(s => (
            <div
              key={s.key}
              className="v4-pipe-step"
              onClick={s.onClick}
              style={{cursor: s.onClick ? 'pointer' : 'default'}}
            >
              <div className="v4-pipe-icon" style={{background: s.iconBg, color: s.iconColor}}>{s.icon}</div>
              <div className="v4-pipe-label">{s.label}</div>
              <div className="v4-pipe-count">{s.count}</div>
              <div className={`v4-pipe-sub ${s.subClass}`}>{s.sub}</div>
            </div>
          ))}
        </div>

        {/* VOD scrubber */}
        {scrubberVod && scrubberHighlights.length > 0 && (
          <div className="v4-vod-scrubber">
            <div className="flex justify-between text-[12px] text-slate-400 mb-2.5">
              <span>📺 {scrubberVod.title} · {scrubberHighlights.length} highlights detected</span>
              <span style={{color:'#a78bfa'}}>Click a marker to preview →</span>
            </div>
            <div className="v4-timeline">
              {scrubberHighlights.map(h => {
                const pct = (h.start_seconds / scrubberVod.duration_seconds) * 100
                const widthPct = Math.max(0.5, ((h.end_seconds - h.start_seconds) / scrubberVod.duration_seconds) * 100)
                const score = h.confidence_score ?? legacyToConfidence(h.virality_score)
                const hot = score >= 0.85
                return (
                  <div
                    key={h.id}
                    className={`v4-hl-marker ${hot ? 'hot' : ''}`}
                    style={{left: `${pct}%`, width: `${widthPct}%`}}
                    title={`${fmtTime(h.start_seconds)} · ${Math.round(score * 100)}%`}
                  />
                )
              })}
            </div>
            <div className="flex justify-between text-[10px] text-slate-500 mt-1.5 tabular-nums">
              <span>{fmtTime(0)}</span>
              <span>{fmtTime(scrubberVod.duration_seconds / 4)}</span>
              <span>{fmtTime(scrubberVod.duration_seconds / 2)}</span>
              <span>{fmtTime((scrubberVod.duration_seconds * 3) / 4)}</span>
              <span>{fmtTime(scrubberVod.duration_seconds)}</span>
            </div>
          </div>
        )}
      </section>

      {/* ═════ TWO COLUMN: Workbench + Right rail ═════ */}
      <div className="grid gap-[18px]" style={{gridTemplateColumns:'1.7fr 1fr'}}>
        {/* ─── Clip Workbench ─── */}
        <div className="v4-panel">
          <div className="flex justify-between items-center mb-3.5">
            <div>
              <div className="text-[15px] font-bold text-white">Clip Workbench</div>
              <div className="text-xs text-slate-500 mt-0.5">Review, edit, schedule, and ship in one place</div>
            </div>
            <button className="px-3 py-1.5 rounded-md text-xs font-semibold bg-surface-800 border border-surface-700 text-white cursor-pointer">
              Bulk actions ▾
            </button>
          </div>

          <div className="v4-tabs" role="tablist">
            {(['review','ready','scheduled','published','failed'] as const).map(t => (
              <button
                key={t}
                className={`v4-tab ${tab === t ? 'active' : ''}`}
                onClick={() => setTab(t)}
                style={t === 'failed' ? {color:'#f87171'} : undefined}
              >
                {t === 'review' && '⭐ Needs review'}
                {t === 'ready' && '✂ Ready'}
                {t === 'scheduled' && '🕒 Scheduled'}
                {t === 'published' && '🚀 Published'}
                {t === 'failed' && '⚠ Failed'}
                <span className="v4-tab-count">{tabCounts[t]}</span>
              </button>
            ))}
          </div>

          {tab === 'review' && (
            <div>
              <div className="v4-tip">💡 Click a highlight to open it in the editor.</div>
              {reviewHighlights.length === 0 ? (
                <div className="p-10 text-center text-sm text-slate-500">No highlights need review.</div>
              ) : reviewHighlights.map((h, i) => {
                const score = h.confidence_score ?? legacyToConfidence(h.virality_score)
                const vod = vodMap[h.vod_id]
                return (
                  <div
                    key={h.id}
                    className="v4-clip-row"
                    onClick={() => vod && navigate(`/results/${vod.id}`)}
                  >
                    <div className={`v4-clip-thumb ${THUMB_STYLES[i % THUMB_STYLES.length]}`}>
                      <span className="v4-clip-dur">{fmtTime(h.end_seconds - h.start_seconds)}</span>
                      <div className="v4-waveform">
                        {Array.from({length:10}).map((_,j) => (
                          <div key={j} className="v4-wave-bar" style={{height:`${20 + ((j * 37) % 70)}%`}}/>
                        ))}
                      </div>
                    </div>
                    <div className="v4-clip-info">
                      <div className="v4-clip-title">
                        {h.event_summary || h.description || h.transcript_snippet || 'Untitled highlight'}
                      </div>
                      <div className="v4-clip-meta">
                        <span>{fmtTime(h.start_seconds)} – {fmtTime(h.end_seconds)}</span>
                        {parseTags(h.tags).slice(0,2).map(tg => (
                          <span key={tg} className={`v4-tone-chip ${tg === 'hype' ? 'hype' : ''}`}>{tg}</span>
                        ))}
                      </div>
                    </div>
                    <div className="v4-confidence">
                      <div className={`v4-conf-label ${score < 0.75 ? 'low' : score >= 0.85 ? 'high' : ''}`}>
                        {Math.round(score * 100)}%
                      </div>
                      <div className="v4-conf-bar">
                        <div
                          className="v4-conf-fill"
                          style={{
                            width:`${Math.round(score * 100)}%`,
                            ...(score < 0.75 ? {background:'linear-gradient(90deg,#fbbf24,#f472b6)'} : {})
                          }}
                        />
                      </div>
                    </div>
                    <div className="v4-platforms">
                      <div className="v4-plat yt">▶</div>
                      <div className="v4-plat tt">𝄩</div>
                    </div>
                    <button className="v4-clip-action" onClick={(e) => { e.stopPropagation(); if (vod) navigate(`/results/${vod.id}`) }}>
                      Review
                    </button>
                  </div>
                )
              })}
            </div>
          )}

          {tab === 'ready' && (
            <div>
              <div className="v4-tip">✂ These clips are rendered. Click to publish or edit.</div>
              {readyClips.length === 0 ? (
                <div className="p-10 text-center text-sm text-slate-500">No clips ready yet.</div>
              ) : readyClips.map((c, i) => {
                const hl = highlights.find(h => h.id === c.highlight_id)
                const score = hl ? (hl.confidence_score ?? legacyToConfidence(hl.virality_score)) : 0
                const isViral = score >= 0.9
                return (
                <div
                  key={c.id}
                  className="v4-clip-row"
                  onClick={() => navigate(`/editor/${c.id}`)}
                >
                  <div className={`v4-clip-thumb ${THUMB_STYLES[i % THUMB_STYLES.length]}`}>
                    <span className="v4-clip-dur">{fmtTime(c.end_seconds - c.start_seconds)}</span>
                  </div>
                  <div className="v4-clip-info">
                    <div className="v4-clip-title">{c.title || 'Untitled clip'}</div>
                    <div className="v4-clip-meta">
                      <span>{fmtTime(c.start_seconds)} – {fmtTime(c.end_seconds)}</span>
                      <span className="v4-tone-chip">{c.aspect_ratio}</span>
                      {isViral && <span className="v4-viral-badge">🔥 VIRAL PICK</span>}
                    </div>
                  </div>
                  <div />
                  <div className="v4-platforms">
                    <div className="v4-plat yt">▶</div>
                    <div className="v4-plat tt">𝄩</div>
                  </div>
                  <button className="v4-clip-action primary" onClick={(e) => { e.stopPropagation(); navigate(`/editor/${c.id}`) }}>
                    Ship →
                  </button>
                </div>
              )})}
            </div>
          )}

          {tab === 'scheduled' && (
            <div>
              <div className="v4-tip">🕒 Scheduled uploads post automatically.</div>
              {pendingScheduled.length === 0 ? (
                <div className="p-10 text-center text-sm text-slate-500">No uploads scheduled.</div>
              ) : pendingScheduled.slice(0, 6).map((u, i) => {
                const clip = clipMap[u.clip_id]
                const platformInfo = PLATFORM_INFO[u.platform]
                return (
                  <div key={u.id} className="v4-clip-row" onClick={() => navigate('/scheduled')}>
                    <div className={`v4-clip-thumb ${THUMB_STYLES[i % THUMB_STYLES.length]}`}>
                      <span className="v4-clip-dur">
                        {clip ? fmtTime(clip.end_seconds - clip.start_seconds) : ''}
                      </span>
                    </div>
                    <div className="v4-clip-info">
                      <div className="v4-clip-title">{clip?.title || u.clip_id}</div>
                      <div className="v4-clip-meta">
                        <span>{platformInfo?.name || u.platform}</span>
                        <span className="v4-tone-chip">
                          {new Date(u.scheduled_time).toLocaleString(undefined, {month:'short', day:'numeric', hour:'numeric', minute:'2-digit'})}
                        </span>
                      </div>
                    </div>
                    <div />
                    <div className="v4-platforms">
                      <div className={`v4-plat ${u.platform === 'youtube' ? 'yt' : u.platform === 'tiktok' ? 'tt' : 'ig'}`}>
                        {u.platform === 'youtube' ? '▶' : u.platform === 'tiktok' ? '𝄩' : '○'}
                      </div>
                    </div>
                    <button className="v4-clip-action" onClick={(e) => { e.stopPropagation(); navigate('/scheduled') }}>
                      Edit
                    </button>
                  </div>
                )
              })}
            </div>
          )}

          {tab === 'published' && (
            <div>
              <div className="v4-tip">🚀 Published uploads from your schedule.</div>
              {completedScheduled.length === 0 ? (
                <div className="p-10 text-center text-sm text-slate-500">Nothing published yet.</div>
              ) : completedScheduled.slice(0, 6).map((u, i) => {
                const clip = clipMap[u.clip_id]
                return (
                  <div key={u.id} className="v4-clip-row">
                    <div className={`v4-clip-thumb ${THUMB_STYLES[i % THUMB_STYLES.length]}`}>
                      <span className="v4-clip-dur">
                        {clip ? fmtTime(clip.end_seconds - clip.start_seconds) : ''}
                      </span>
                    </div>
                    <div className="v4-clip-info">
                      <div className="v4-clip-title">{clip?.title || u.clip_id}</div>
                      <div className="v4-clip-meta">
                        <span>Published {new Date(u.scheduled_time).toLocaleDateString()}</span>
                      </div>
                    </div>
                    <div className="v4-views">
                      <span className="v4-views-num">
                        {u.view_count != null ? formatViewerCount(u.view_count) : '—'}
                      </span>
                      <span className="v4-views-lbl">
                        {u.view_count != null
                          ? (u.ctr_percent != null ? `VIEWS · ${u.ctr_percent.toFixed(1)}% CTR` : 'VIEWS')
                          : 'VIEWS · CTR'}
                      </span>
                      {u.view_count != null && u.view_count >= 10_000 && (
                        <span className="v4-viral-badge">🔥 TRENDING</span>
                      )}
                    </div>
                    <div className="v4-platforms">
                      <div className={`v4-plat ${u.platform === 'youtube' ? 'yt' : u.platform === 'tiktok' ? 'tt' : 'ig'}`}>
                        {u.platform === 'youtube' ? '▶' : u.platform === 'tiktok' ? '𝄩' : '○'}
                      </div>
                    </div>
                    <button
                      className="v4-clip-action"
                      onClick={() => u.video_url && window.open(u.video_url, '_blank')}
                    >
                      {u.video_url ? 'View' : 'Analytics'}
                    </button>
                  </div>
                )
              })}
            </div>
          )}

          {tab === 'failed' && (
            <div>
              <div className="v4-tip" style={{background:'rgba(248,113,113,0.08)',borderColor:'rgba(248,113,113,0.25)',color:'#f87171'}}>
                ⚠ Failed uploads — click Retry to attempt again.
              </div>
              {failedScheduled.length === 0 ? (
                <div className="p-10 text-center text-sm text-slate-500">No failures. 🎉</div>
              ) : failedScheduled.slice(0, 6).map((u, i) => {
                const clip = clipMap[u.clip_id]
                return (
                  <div key={u.id} className="v4-clip-row">
                    <div className={`v4-clip-thumb ${THUMB_STYLES[i % THUMB_STYLES.length]}`}>
                      <span className="v4-clip-dur">
                        {clip ? fmtTime(clip.end_seconds - clip.start_seconds) : ''}
                      </span>
                    </div>
                    <div className="v4-clip-info">
                      <div className="v4-clip-title">{clip?.title || u.clip_id}</div>
                      <div className="v4-clip-meta">
                        <span style={{color:'#f87171'}}>{u.error_message || 'Upload failed'}</span>
                      </div>
                    </div>
                    <div />
                    <div className="v4-platforms">
                      <div className={`v4-plat ${u.platform === 'youtube' ? 'yt' : u.platform === 'tiktok' ? 'tt' : 'ig'}`}>
                        {u.platform === 'youtube' ? '▶' : u.platform === 'tiktok' ? '𝄩' : '○'}
                      </div>
                    </div>
                    <button
                      className="v4-clip-action"
                      style={{background:'rgba(248,113,113,0.15)',border:'1px solid rgba(248,113,113,0.4)',color:'#f87171'}}
                      onClick={() => navigate('/scheduled')}
                    >
                      {u.error_message?.toLowerCase().includes('auth') || u.error_message?.toLowerCase().includes('token') || u.error_message?.toLowerCase().includes('expired')
                        ? 'Reconnect'
                        : `Retry ${u.platform === 'youtube' ? 'YT' : u.platform === 'tiktok' ? 'TT' : 'IG'}`}
                    </button>
                  </div>
                )
              })}
            </div>
          )}
        </div>

        {/* ─── Right rail ─── */}
        <div className="space-y-3.5">
          {/* AI Insight */}
          <div className="v4-insight">
            <div className="flex items-center gap-2 mb-2.5 relative">
              <span className="v4-insight-badge">🧠 Goblin Insight</span>
            </div>
            <div className="text-[15px] font-bold mb-1 relative text-white">
              {highlights.length > 10
                ? `Your stream produced ${highlights.length} highlights`
                : 'Run more VODs to unlock insights'}
            </div>
            <div className="text-[13px] text-slate-400 leading-relaxed relative">
              {highlights.length > 10 ? (
                <>
                  Avg confidence: <b className="text-white">
                    {Math.round(
                      (highlights.reduce((s, h) =>
                        s + (h.confidence_score ?? legacyToConfidence(h.virality_score)), 0) / highlights.length
                      ) * 100
                    )}%
                  </b> — top {reviewHighlights.length > 0 ? reviewHighlights.length : 0} need your review.
                </>
              ) : (
                'Once you have a few VODs analyzed, Goblin will surface patterns in what\'s working.'
              )}
            </div>
            <div className="flex gap-2 mt-3 relative flex-wrap">
              <button
                onClick={() => navigate('/vods')}
                className="px-3 py-1.5 rounded-lg text-xs font-semibold text-white bg-gradient-to-br from-violet-400 to-pink-400 cursor-pointer"
              >
                Go to VODs
              </button>
              <button
                onClick={() => navigate('/settings')}
                className="px-3 py-1.5 rounded-lg text-xs font-semibold text-white bg-white/5 border border-violet-400/30 cursor-pointer"
              >
                Tune detection
              </button>
            </div>
          </div>

          {/* Next Up */}
          <div className="v4-panel" style={{padding:'16px'}}>
            <div className="flex justify-between items-center mb-2.5">
              <div className="text-[13px] font-bold text-white">⏳ Next Up</div>
              <button
                onClick={() => navigate('/scheduled')}
                className="text-xs text-violet-400 hover:text-violet-300 cursor-pointer"
              >
                See all →
              </button>
            </div>
            {nextScheduled ? (
              <>
                <div className="v4-next-up">
                  <div className="v4-countdown">
                    <span className="v4-count-num">{fmtCountdown(nextScheduled.scheduled_time)}</span>
                  </div>
                  <div className="text-[13px] font-semibold mb-0.5 text-white">
                    {clipMap[nextScheduled.clip_id]?.title || nextScheduled.clip_id}
                  </div>
                  <div className="text-[11px] text-slate-500">
                    → {PLATFORM_INFO[nextScheduled.platform]?.name || nextScheduled.platform}
                  </div>
                </div>
                {pendingScheduled.slice(1, 4).map(u => {
                  const clip = clipMap[u.clip_id]
                  return (
                    <div
                      key={u.id}
                      className="flex items-center gap-2.5 py-2 text-xs border-t border-surface-700"
                    >
                      <span className="text-slate-400 tabular-nums min-w-[52px]">
                        {new Date(u.scheduled_time).toLocaleTimeString(undefined, {hour:'numeric', minute:'2-digit'})}
                      </span>
                      <span className="flex-1 truncate text-white">{clip?.title || u.clip_id}</span>
                      <div className={`v4-plat ${u.platform === 'youtube' ? 'yt' : u.platform === 'tiktok' ? 'tt' : 'ig'}`}>
                        {u.platform === 'youtube' ? '▶' : u.platform === 'tiktok' ? '𝄩' : '○'}
                      </div>
                    </div>
                  )
                })}
              </>
            ) : (
              <div className="text-[13px] text-slate-500 py-4">No uploads scheduled.</div>
            )}
          </div>

          {/* Stats strip */}
          <div className="v4-panel" style={{padding:'16px'}}>
            <div className="flex items-center justify-between mb-1">
              <div className="text-[13px] font-bold text-white">📈 At a glance</div>
              <button
                onClick={() => navigate('/analytics')}
                className="text-xs text-violet-400 hover:text-violet-300 cursor-pointer"
              >
                Analytics →
              </button>
            </div>
            <div className="v4-perf-strip">
              <div className="v4-perf">
                <div className="text-[10px] text-slate-500 tracking-wider uppercase">Channels</div>
                <div className="text-lg font-bold text-white mt-1">{channels.length}</div>
              </div>
              <div className="v4-perf">
                <div className="text-[10px] text-slate-500 tracking-wider uppercase">Clips</div>
                <div className="text-lg font-bold text-white mt-1">{clips.length}</div>
              </div>
              <div className="v4-perf">
                <div className="text-[10px] text-slate-500 tracking-wider uppercase">Ship rate</div>
                <div className="text-lg font-bold text-white mt-1">
                  {clips.length === 0 ? '—' : `${Math.round((completedScheduled.length / clips.length) * 100)}%`}
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      <ImportVodDialog
        open={showImportDialog}
        onClose={() => setShowImportDialog(false)}
      />
    </div>
  )
}
