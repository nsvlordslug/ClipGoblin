import { useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { invoke } from '@tauri-apps/api/core'
import { BarChart3, RefreshCw } from 'lucide-react'
import { useAppStore } from '../stores/appStore'
import { useScheduleStore } from '../stores/scheduleStore'
import { legacyToConfidence } from '../lib/uiFormat'
import { formatViewerCount } from '../hooks/useStreamStatus'

interface RefreshStatsSummary {
  updated: number
  skipped: number
  failed: number
}

function percent(n: number, d: number): string {
  if (d === 0) return '—'
  return `${Math.round((n / d) * 100)}%`
}

export default function Analytics() {
  const navigate = useNavigate()
  const { clips, highlights, vods, fetchClips, fetchHighlights } = useAppStore()
  const { uploads, load: loadUploads } = useScheduleStore()
  const [refreshing, setRefreshing] = useState(false)
  const [refreshStatus, setRefreshStatus] = useState<string | null>(null)
  const [sortMode, setSortMode] = useState<'score' | 'views'>('score')

  useEffect(() => {
    fetchClips()
    fetchHighlights()
    loadUploads()
  }, [fetchClips, fetchHighlights, loadUploads])

  const refreshStats = async () => {
    if (refreshing) return
    setRefreshing(true)
    setRefreshStatus('Refreshing view counts...')
    try {
      const summary = await invoke<RefreshStatsSummary>('refresh_upload_stats')
      await loadUploads()
      const parts: string[] = []
      if (summary.updated) parts.push(`${summary.updated} updated`)
      if (summary.skipped) parts.push(`${summary.skipped} skipped`)
      if (summary.failed) parts.push(`${summary.failed} failed`)
      setRefreshStatus(parts.length ? parts.join(' · ') : 'Nothing to refresh yet')
      // Auto-sort by views when we just got data
      if (summary.updated > 0) setSortMode('views')
      setTimeout(() => setRefreshStatus(null), 4000)
    } catch (e) {
      setRefreshStatus(`Failed: ${e}`)
      setTimeout(() => setRefreshStatus(null), 5000)
    } finally {
      setRefreshing(false)
    }
  }

  const completed = uploads.filter(u => u.status === 'completed')
  const pending = uploads.filter(u => u.status === 'pending' || u.status === 'uploading')
  const failed = uploads.filter(u => u.status === 'failed')

  const avgConfidence = useMemo(() => {
    if (highlights.length === 0) return 0
    const sum = highlights.reduce((s, h) => s + (h.confidence_score ?? legacyToConfidence(h.virality_score)), 0)
    return sum / highlights.length
  }, [highlights])

  const shipRate = clips.length > 0 ? completed.length / clips.length : 0

  const platformCounts = useMemo(() => {
    const counts: Record<string, number> = { youtube: 0, tiktok: 0, instagram: 0 }
    for (const u of completed) {
      if (counts[u.platform] !== undefined) counts[u.platform]++
    }
    const total = Object.values(counts).reduce((s, n) => s + n, 0)
    return { counts, total }
  }, [completed])

  // Aggregate view counts per clip across every completed upload on every platform.
  const viewsByClip = useMemo(() => {
    const map = new Map<string, number>()
    for (const u of completed) {
      if (u.view_count == null) continue
      map.set(u.clip_id, (map.get(u.clip_id) ?? 0) + u.view_count)
    }
    return map
  }, [completed])

  const topClips = useMemo(() => {
    const scored = clips.map(c => {
      const hl = highlights.find(h => h.id === c.highlight_id)
      const score = hl?.confidence_score ?? (hl ? legacyToConfidence(hl.virality_score) : 0)
      const views = viewsByClip.get(c.id) ?? null
      return { clip: c, score, views }
    })
    if (sortMode === 'views') {
      // Pull clips with real view data to the top, then fall back to score
      return [...scored]
        .sort((a, b) => {
          const av = a.views ?? -1
          const bv = b.views ?? -1
          if (av !== bv) return bv - av
          return b.score - a.score
        })
        .slice(0, 5)
    }
    return [...scored].sort((a, b) => b.score - a.score).slice(0, 5)
  }, [clips, highlights, viewsByClip, sortMode])

  const bestClipTitle = topClips[0]?.clip?.title || '—'

  // Total views across all published uploads. Null when no stats have ever landed.
  const totalViews = useMemo(() => {
    const contributors = completed.filter(u => u.view_count != null)
    if (contributors.length === 0) return null
    return contributors.reduce((sum, u) => sum + (u.view_count ?? 0), 0)
  }, [completed])

  return (
    <div className="space-y-4">
      <div className="v4-page-header">
        <div>
          <div className="v4-page-title">Analytics 📈</div>
          <div className="v4-page-sub">Overview of your clip production and publishing performance.</div>
        </div>
        <div className="v4-page-actions">
          {refreshStatus && (
            <span className="text-xs text-slate-400 mr-2">{refreshStatus}</span>
          )}
          <button
            onClick={refreshStats}
            disabled={refreshing || completed.length === 0}
            className="v4-btn primary"
            title={completed.length === 0 ? 'Ship at least one clip first' : 'Pull view counts from YouTube / TikTok'}
          >
            <RefreshCw className={`w-3.5 h-3.5 ${refreshing ? 'animate-spin' : ''}`} />
            {refreshing ? 'Refreshing...' : 'Refresh stats'}
          </button>
        </div>
      </div>

      {totalViews == null && (
        <div className="v4-tip">
          💡 Click <b>Refresh stats</b> to pull view counts from YouTube / TikTok for your published clips.
        </div>
      )}

      {/* KPI grid */}
      <div className="v4-analytics-grid">
        <div className="v4-kpi">
          <div className="v4-kpi-label">Total views</div>
          <div className="v4-kpi-value">{totalViews != null ? formatViewerCount(totalViews) : '—'}</div>
          <div className="v4-kpi-delta neutral">
            {totalViews != null ? `across ${completed.length} upload${completed.length !== 1 ? 's' : ''}` : 'Connect analytics to track'}
          </div>
        </div>
        <div className="v4-kpi">
          <div className="v4-kpi-label">Clips shipped</div>
          <div className="v4-kpi-value">{completed.length}</div>
          <div className="v4-kpi-delta">▲ {pending.length} pending</div>
        </div>
        <div className="v4-kpi">
          <div className="v4-kpi-label">Ship rate</div>
          <div className="v4-kpi-value">{percent(completed.length, clips.length)}</div>
          <div className="v4-kpi-delta neutral">of {clips.length} clip{clips.length !== 1 ? 's' : ''}</div>
        </div>
        <div className="v4-kpi">
          <div className="v4-kpi-label">Avg confidence</div>
          <div className="v4-kpi-value">{highlights.length > 0 ? `${Math.round(avgConfidence * 100)}%` : '—'}</div>
          <div className="v4-kpi-delta neutral">{highlights.length} highlights</div>
        </div>
      </div>

      {/* Platform split */}
      <div className="v4-big-chart-card">
        <div className="flex items-center justify-between mb-3">
          <div>
            <div className="text-[15px] font-bold text-white">Platform mix</div>
            <div className="text-xs text-slate-500 mt-0.5">
              {platformCounts.total} completed upload{platformCounts.total !== 1 ? 's' : ''}
            </div>
          </div>
          <BarChart3 className="w-5 h-5 text-violet-400" />
        </div>
        {platformCounts.total === 0 ? (
          <div className="text-sm text-slate-500 py-4 text-center">
            Nothing published yet — ship your first clip to see your platform mix.
          </div>
        ) : (
          <div className="v4-platform-split">
            {platformCounts.counts.youtube > 0 && (
              <div
                className="v4-platform-bar yt"
                style={{ flex: platformCounts.counts.youtube }}
                title={`YouTube: ${platformCounts.counts.youtube}`}
              >
                YT · {percent(platformCounts.counts.youtube, platformCounts.total)}
              </div>
            )}
            {platformCounts.counts.tiktok > 0 && (
              <div
                className="v4-platform-bar tt"
                style={{ flex: platformCounts.counts.tiktok }}
                title={`TikTok: ${platformCounts.counts.tiktok}`}
              >
                TT · {percent(platformCounts.counts.tiktok, platformCounts.total)}
              </div>
            )}
            {platformCounts.counts.instagram > 0 && (
              <div
                className="v4-platform-bar ig"
                style={{ flex: platformCounts.counts.instagram }}
                title={`Instagram: ${platformCounts.counts.instagram}`}
              >
                IG · {percent(platformCounts.counts.instagram, platformCounts.total)}
              </div>
            )}
          </div>
        )}
      </div>

      {/* Top clips */}
      <div className="v4-panel">
        <div className="flex items-center justify-between mb-3">
          <div>
            <div className="text-[15px] font-bold text-white">
              Top clips by {sortMode === 'views' ? 'views' : 'confidence'}
            </div>
            <div className="text-xs text-slate-500 mt-0.5">
              {sortMode === 'views'
                ? 'Ranked by aggregated platform view counts'
                : 'Highest-scoring highlights across all your VODs'}
            </div>
          </div>
          <div className="flex items-center gap-2">
            {totalViews != null && (
              <button
                onClick={() => setSortMode(sortMode === 'views' ? 'score' : 'views')}
                className="v4-btn ghost"
                title="Toggle sort between confidence score and view count"
              >
                Sort: {sortMode === 'views' ? 'Views' : 'Score'} ↕
              </button>
            )}
            <button className="v4-btn ghost" onClick={() => navigate('/clips')}>Open library</button>
          </div>
        </div>
        {topClips.length === 0 ? (
          <div className="text-sm text-slate-500 py-6 text-center">
            No clips yet — analyze a VOD to populate this list.
          </div>
        ) : (
          <div>
            {topClips.map(({ clip, score, views }, i) => (
              <div
                key={clip.id}
                className="v4-clip-row"
                onClick={() => navigate(`/editor/${clip.id}`)}
              >
                <div className={`v4-clip-thumb ${['a','b','c','d','e','f'][i % 6]}`}>
                  <span className="v4-clip-dur">
                    {Math.floor((clip.end_seconds - clip.start_seconds) / 60)}:
                    {String(Math.floor((clip.end_seconds - clip.start_seconds) % 60)).padStart(2,'0')}
                  </span>
                </div>
                <div className="v4-clip-info">
                  <div className="v4-clip-title">{clip.title || 'Untitled clip'}</div>
                  <div className="v4-clip-meta">
                    <span>Score {Math.round(score * 100)}%</span>
                    {score >= 0.9 && <span className="v4-viral-badge">🔥 VIRAL</span>}
                  </div>
                </div>
                <div className="v4-views">
                  <span className="v4-views-num">{views != null ? formatViewerCount(views) : '—'}</span>
                  <span className="v4-views-lbl">VIEWS</span>
                  {views != null && views >= 10_000 && (
                    <span className="v4-viral-badge">🔥 TRENDING</span>
                  )}
                </div>
                <div />
                <button
                  className="v4-clip-action"
                  onClick={(e) => { e.stopPropagation(); navigate(`/editor/${clip.id}`) }}
                >
                  Open →
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Pipeline summary */}
      <div className="v4-panel">
        <div className="text-[15px] font-bold text-white mb-2">At a glance</div>
        <div className="v4-perf-strip">
          <div className="v4-perf">
            <div className="text-[10px] text-slate-500 tracking-wider uppercase">VODs analyzed</div>
            <div className="text-lg font-bold text-white mt-1">
              {vods.filter(v => v.analysis_status === 'completed').length}
            </div>
          </div>
          <div className="v4-perf">
            <div className="text-[10px] text-slate-500 tracking-wider uppercase">Highlights</div>
            <div className="text-lg font-bold text-white mt-1">{highlights.length}</div>
          </div>
          <div className="v4-perf">
            <div className="text-[10px] text-slate-500 tracking-wider uppercase">Clips</div>
            <div className="text-lg font-bold text-white mt-1">{clips.length}</div>
          </div>
          <div className="v4-perf">
            <div className="text-[10px] text-slate-500 tracking-wider uppercase">Published</div>
            <div className="text-lg font-bold text-white mt-1">{completed.length}</div>
          </div>
          <div className="v4-perf">
            <div className="text-[10px] text-slate-500 tracking-wider uppercase">Failed</div>
            <div className="text-lg font-bold text-white mt-1">{failed.length}</div>
          </div>
          <div className="v4-perf">
            <div className="text-[10px] text-slate-500 tracking-wider uppercase">Ship rate</div>
            <div className="text-lg font-bold text-white mt-1">{percent(completed.length, clips.length)}</div>
          </div>
        </div>
        <div className="text-xs text-slate-500 mt-3">
          Best clip: <b className="text-white">{bestClipTitle}</b>
          {shipRate >= 0.5 && (
            <span className="ml-2 v4-viral-badge">🔥 On a roll</span>
          )}
        </div>
      </div>
    </div>
  )
}
