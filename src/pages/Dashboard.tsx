import { useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { AlertTriangle, ArrowRight, CheckCircle2, CircleDot, Loader2, MessageSquareText, RefreshCw, Send, Settings2, Sparkles, Video } from 'lucide-react'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useAppStore } from '../stores/appStore'
import { useScheduleStore } from '../stores/scheduleStore'
import { PLATFORM_INFO } from '../stores/platformStore'
import ImportVodDialog from '../components/ImportVodDialog'
import TesterChecklist from '../components/TesterChecklist'
import heroGoblinImg from '../assets/hero-goblin-v2.png'
import {
  buildDashboardInbox,
  getDashboardConfidence,
  getDashboardInboxTarget,
} from '../lib/dashboardInbox'
import type { ImportedClipResult, RecorderStatus } from '../lib/externalSources'

interface AutoShipReport {
  clips_queued: number
  platforms: string[]
  next_publish_at: string | null
}

function fmtTime(secs: number): string {
  const m = Math.floor(secs / 60)
  const s = Math.floor(secs % 60)
  return `${m}:${String(s).padStart(2, '0')}`
}

function fmtCountdown(targetISO: string): string {
  const diff = new Date(targetISO).getTime() - Date.now()
  if (diff <= 0) return '0:00:00'
  const h = Math.floor(diff / 3_600_000)
  const m = Math.floor((diff % 3_600_000) / 60_000)
  const s = Math.floor((diff % 60_000) / 1000)
  return `${h}h ${String(m).padStart(2, '0')}m ${String(s).padStart(2, '0')}s`
}

export default function Dashboard() {
  const { channels, vods, highlights, clips, checkLogin, fetchHighlights, fetchClips, fetchVods, loggedInUser } =
    useAppStore()
  const { uploads: scheduledUploads, load: loadSchedules } = useScheduleStore()
  const navigate = useNavigate()
  const [, setTick] = useState(0)
  const [hunting, setHunting] = useState(false)
  const [huntStatus, setHuntStatus] = useState<string | null>(null)
  const [showImportDialog, setShowImportDialog] = useState(false)
  const [autoShipReport, setAutoShipReport] = useState<AutoShipReport | null>(null)
  const [recorderKind, setRecorderKind] = useState<'obs' | 'meld'>('obs')
  const [recorderBusy, setRecorderBusy] = useState<'test' | 'mark' | 'replay' | null>(null)
  const [recorderStatus, setRecorderStatus] = useState<RecorderStatus | null>(null)
  const [recorderNotice, setRecorderNotice] = useState<{ ok: boolean; text: string } | null>(null)

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
  const completedScheduled = scheduledUploads.filter(u => u.status === 'completed')

  const clipMap = useMemo(() => Object.fromEntries(clips.map(c => [c.id, c])), [clips])
  const vodMap = useMemo(() => Object.fromEntries(vods.map(vod => [vod.id, vod])), [vods])
  const inbox = useMemo(
    () => buildDashboardInbox(highlights, clips, scheduledUploads),
    [highlights, clips, scheduledUploads],
  )

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
      sub: inbox.counts.review ? `${inbox.counts.review} need review` : 'all reviewed',
      subClass: inbox.counts.review ? 'good' : '',
      iconBg: 'rgba(251,191,36,0.15)', iconColor: '#fbbf24',
    },
    {
      key: 'clips', icon: '✂',
      label: 'Clips ready',
      count: inbox.counts.ready,
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

  const testRecorder = async () => {
    setRecorderBusy('test')
    setRecorderNotice(null)
    try {
      const status = await invoke<RecorderStatus>('test_recorder_connection', { kind: recorderKind })
      setRecorderStatus(status)
      setRecorderNotice({ ok: status.reachable, text: status.detail })
    } catch (error) {
      setRecorderStatus(null)
      setRecorderNotice({ ok: false, text: String(error) })
    } finally {
      setRecorderBusy(null)
    }
  }

  const markStreamMoment = async () => {
    setRecorderBusy('mark')
    setRecorderNotice(null)
    try {
      await invoke('create_stream_marker', { recorderKind, label: null })
      setRecorderNotice({ ok: true, text: 'Moment marked. The matching Twitch VOD will favor it during analysis.' })
    } catch (error) {
      setRecorderNotice({ ok: false, text: String(error) })
    } finally {
      setRecorderBusy(null)
    }
  }

  const saveRecorderReplay = async () => {
    setRecorderBusy('replay')
    setRecorderNotice(null)
    try {
      const result = await invoke<ImportedClipResult>('save_replay_and_import', { kind: recorderKind })
      await Promise.all([fetchClips(), fetchHighlights()])
      setRecorderNotice({
        ok: true,
        text: result.status === 'already_imported' ? 'That replay is already in your library.' : `${result.title} is ready in Clips.`,
      })
    } catch (error) {
      setRecorderNotice({ ok: false, text: String(error) })
    } finally {
      setRecorderBusy(null)
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
            {inbox.counts.review > 0 && (
              <>You have <b className="text-white">{inbox.counts.review} highlights</b> waiting for review. </>
            )}
            {inbox.counts.ready > 0 && (
              <><b className="text-white">{inbox.counts.ready} clips ready</b> to ship.</>
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
                const score = getDashboardConfidence(h)
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

      {/* ═════ TWO COLUMN: Priority inbox + Right rail ═════ */}
      <div className="v4-dashboard-main-grid">
        <div className="v4-panel v4-inbox-panel">
          <div className="v4-inbox-header">
            <div className="v4-inbox-heading">
              <span className="v4-inbox-heading-icon">
                <Sparkles className="w-4 h-4" aria-hidden="true" />
              </span>
              <div>
                <div className="text-[15px] font-bold text-white">Priority Inbox</div>
                <div className="text-xs text-slate-500 mt-0.5">
                  {inbox.total === 0
                    ? 'Nothing needs your attention'
                    : `${inbox.total} item${inbox.total === 1 ? '' : 's'} need your attention`}
                </div>
              </div>
            </div>
            <button
              type="button"
              className="v4-inbox-library-link"
              onClick={() => navigate('/clips')}
            >
              Open library
              <ArrowRight className="w-3.5 h-3.5" aria-hidden="true" />
            </button>
          </div>

          <div className="v4-inbox-summary" aria-label="Priority inbox totals">
            <span className={inbox.counts.failed > 0 ? 'failed' : ''}><i />{inbox.counts.failed} failed</span>
            <span className="review"><i />{inbox.counts.review} to review</span>
            <span className="ready"><i />{inbox.counts.ready} ready</span>
          </div>

          {inbox.items.length === 0 ? (
            <div className="v4-inbox-empty">
              <CheckCircle2 className="w-8 h-8" aria-hidden="true" />
              <div>
                <div className="font-semibold text-slate-200">You're caught up</div>
                <div className="text-xs text-slate-500 mt-0.5">No failed uploads, pending reviews, or ready exports.</div>
              </div>
            </div>
          ) : (
            <div className="v4-inbox-list">
              {inbox.items.map(item => {
                const target = getDashboardInboxTarget(item)
                let title: string
                let detail: string
                let action: string
                let ItemIcon = MessageSquareText
                const clipForVisual = item.clip
                const poster = clipForVisual?.thumbnail_path
                  ? convertFileSrc(clipForVisual.thumbnail_path)
                  : clipForVisual
                    ? vodMap[clipForVisual.vod_id]?.thumbnail_url || null
                    : null
                let visualTag = ''

                if (item.kind === 'failed') {
                  title = item.clip?.title || 'Upload needs attention'
                  detail = `${PLATFORM_INFO[item.upload.platform]?.name || item.upload.platform} · ${item.upload.error_message || 'Upload failed'}`
                  action = item.needsReconnect ? 'Reconnect' : 'Resolve'
                  ItemIcon = AlertTriangle
                  visualTag = (PLATFORM_INFO[item.upload.platform]?.name || item.upload.platform).slice(0, 2).toUpperCase()
                } else if (item.kind === 'review') {
                  title = item.highlight.event_summary
                    || item.highlight.description
                    || item.highlight.transcript_snippet
                    || 'Untitled highlight'
                  detail = `${Math.round(item.confidence * 100)}% confidence · ${fmtTime(item.highlight.start_seconds)}–${fmtTime(item.highlight.end_seconds)}`
                  action = 'Review'
                  visualTag = fmtTime(item.highlight.end_seconds - item.highlight.start_seconds)
                } else {
                  title = item.clip.title || 'Untitled clip'
                  detail = `${item.clip.aspect_ratio} · ${fmtTime(item.clip.end_seconds - item.clip.start_seconds)} · Export complete`
                  action = 'Publish'
                  ItemIcon = Send
                  visualTag = fmtTime(item.clip.end_seconds - item.clip.start_seconds)
                }

                return (
                  <button
                    key={item.id}
                    type="button"
                    className={`v4-inbox-row ${item.kind}`}
                    onClick={() => navigate(target.pathname, { state: target.state })}
                    aria-label={`${action}: ${title}`}
                  >
                    <span className="v4-inbox-visual" aria-hidden="true">
                      <span className="v4-inbox-visual-bars">
                        {[38, 72, 48, 88, 58, 78, 42].map((height, index) => (
                          <i key={index} style={{ height: `${height}%` }} />
                        ))}
                      </span>
                      {poster && (
                        <img
                          src={poster}
                          alt=""
                          onError={event => { event.currentTarget.style.display = 'none' }}
                        />
                      )}
                      <span className="v4-inbox-visual-shade" />
                      <span className="v4-inbox-visual-icon">
                        <ItemIcon className="w-3.5 h-3.5" />
                      </span>
                      <span className="v4-inbox-visual-tag">{visualTag}</span>
                    </span>
                    <span className="v4-inbox-row-copy">
                      <span className="v4-inbox-row-kind">
                        {item.kind === 'failed' ? 'Upload failed' : item.kind === 'review' ? 'Needs review' : 'Ready to publish'}
                      </span>
                      <span className="v4-inbox-row-title" title={title}>{title}</span>
                      <span className="v4-inbox-row-detail" title={detail}>{detail}</span>
                    </span>
                    <span className="v4-inbox-row-action">
                      {action}
                      <ArrowRight className="w-3.5 h-3.5" aria-hidden="true" />
                    </span>
                  </button>
                )
              })}
            </div>
          )}

        </div>

        {/* ─── Right rail ─── */}
        <div className="space-y-3.5">
          <div className="v4-panel" style={{padding: '16px'}}>
            <div className="flex items-center justify-between gap-3">
              <div>
                <div className="text-[13px] font-bold text-white">Recorder capture</div>
                <div className="mt-0.5 text-[11px] text-slate-500">Mark a live moment or pull the latest replay into Clips.</div>
              </div>
              <button type="button" className="rounded p-1.5 text-slate-500 hover:bg-surface-700 hover:text-white" onClick={() => navigate('/settings', { state: { settingsSection: 'sources' } })} title="Recorder settings" aria-label="Open recorder settings">
                <Settings2 className="h-4 w-4" />
              </button>
            </div>
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <div className="inline-flex rounded-md border border-surface-700 bg-surface-900 p-0.5" role="group" aria-label="Recorder">
                {(['obs', 'meld'] as const).map(kind => (
                  <button
                    key={kind}
                    type="button"
                    className={`px-2.5 py-1 text-[11px] font-semibold uppercase transition-colors ${recorderKind === kind ? 'bg-violet-500/20 text-violet-200' : 'text-slate-500 hover:text-slate-300'}`}
                    onClick={() => {
                      setRecorderKind(kind)
                      setRecorderStatus(null)
                      setRecorderNotice(null)
                    }}
                    aria-pressed={recorderKind === kind}
                  >
                    {kind}
                  </button>
                ))}
              </div>
              <button type="button" className="v4-btn ghost" onClick={() => void testRecorder()} disabled={recorderBusy !== null} title={`Test ${recorderKind.toUpperCase()} connection`}>
                {recorderBusy === 'test' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                Test
              </button>
              {recorderStatus && (
                <span className={`text-[10px] font-semibold ${recorderStatus.reachable ? 'text-emerald-400' : 'text-red-300'}`}>
                  {recorderStatus.reachable ? 'Connected' : 'Offline'}
                </span>
              )}
            </div>
            <div className="mt-2 grid grid-cols-2 gap-2">
              <button type="button" className="v4-btn ghost justify-center" onClick={() => void markStreamMoment()} disabled={recorderBusy !== null}>
                {recorderBusy === 'mark' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <CircleDot className="h-3.5 w-3.5" />}
                Mark moment
              </button>
              <button type="button" className="v4-btn primary justify-center" onClick={() => void saveRecorderReplay()} disabled={recorderBusy !== null}>
                {recorderBusy === 'replay' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Video className="h-3.5 w-3.5" />}
                Save replay
              </button>
            </div>
            {recorderNotice && (
              <div role="status" className={`mt-2 text-[11px] leading-4 ${recorderNotice.ok ? 'text-emerald-300' : 'text-red-300'}`}>
                {recorderNotice.text}
              </div>
            )}
          </div>

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
                        s + getDashboardConfidence(h), 0) / highlights.length
                      ) * 100
                    )}%
                  </b> — {inbox.counts.review} need your review.
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
                onClick={() => navigate('/settings', { state: { settingsSection: 'detection' } })}
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
