import { useEffect, useRef, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { Video, Download, Search, Eye, Tv, LogIn, Check, RotateCcw, RefreshCw, Trash2, X, Gamepad2, Plus, Play } from 'lucide-react'
import { useAppStore } from '../stores/appStore'
import { useUiStore } from '../stores/uiStore'
import { useAiStore } from '../stores/aiStore'
import { invoke, convertFileSrc } from '@tauri-apps/api/core'
import ImportVodDialog from '../components/ImportVodDialog'

function formatDuration(seconds: number) {
  const h = Math.floor(seconds / 3600)
  const m = Math.floor((seconds % 3600) / 60)
  if (h > 0) return `${h}h ${m}m`
  return `${m}m`
}

function formatDate(dateStr: string) {
  try {
    return new Date(dateStr).toLocaleDateString('en-US', {
      month: 'short',
      day: 'numeric',
      year: 'numeric',
    })
  } catch {
    return dateStr
  }
}

// Per-card thumbnail with robust error handling. Shows the Twitch preview as
// soon as the image loads (works before the VOD is downloaded); falls back to
// the title over the card's gradient background only when there's no URL or the
// image genuinely fails. Tracks the errored URL in React state instead of a
// sticky `display:none` DOM mutation, so a transient load failure can't
// permanently hide a valid thumbnail across the frequent progress-poll
// re-renders, and a URL healed by Re-fetch retries automatically.
function VodThumb({ vodId, downloaded, thumbnailUrl, title }: { vodId: string; downloaded: boolean; thumbnailUrl?: string | null; title: string }) {
  const [erroredUrl, setErroredUrl] = useState<string | null>(null)
  const [localThumb, setLocalThumb] = useState<string | null>(null)
  const twitchUsable = !!thumbnailUrl && erroredUrl !== thumbnailUrl

  // For a downloaded VOD whose Twitch thumbnail is missing or has 404'd (Twitch
  // deletes VOD thumbnails past its retention window), lazily grab a frame from
  // the local video file via the backend so the card still shows a real preview.
  useEffect(() => {
    if (!downloaded || twitchUsable || localThumb) return
    let cancelled = false
    invoke<string | null>('ensure_vod_thumbnail', { vodId })
      .then(p => { if (!cancelled && p) setLocalThumb(convertFileSrc(p)) })
      .catch(() => {})
    return () => { cancelled = true }
  }, [downloaded, twitchUsable, localThumb, vodId])

  if (twitchUsable) {
    return (
      <img
        src={thumbnailUrl as string}
        alt={title}
        className="w-full h-full object-cover absolute inset-0"
        onError={() => setErroredUrl(thumbnailUrl as string)}
      />
    )
  }
  if (localThumb) {
    return <img src={localThumb} alt={title} className="w-full h-full object-cover absolute inset-0" />
  }
  return (
    <div className="w-full h-full flex items-center justify-center absolute inset-0 px-3 text-center">
      <span className="text-[11px] font-medium text-white/70 line-clamp-2">{title}</span>
    </div>
  )
}

// Maps the backend's analysis_progress percentage to a human-readable stage
// label. Keep these ranges in sync with set_analysis_progress() calls in
// commands/vod.rs::run_analysis_signals — the order changed in v1.3.4 when
// the two-pass refactor moved chat analysis ahead of transcription. Old
// labels were stale post-refactor and showed misleading text like "Analyzing
// chat activity" at 45% when the backend was actually finishing transcription.
//
// Current pipeline (matches vod.rs as of v1.3.10):
//   5-15%   Audio RMS extraction (Stage 1)
//   15-18%  Chat-rate + emote-burst analysis (Stage 2)
//   18-22%  Candidate-window pre-selection (Stage 3, two-pass core)
//   22-40%  Whisper transcription on candidate windows (Stage 4)
//   40-65%  Final clip selector (Stage 5)
//   65-75%  Per-clip scoring + title generation (Stage 6)
//   75-83%  Caption generation (Stage 7)
//   83-100% Clip file output + thumbnails (Stage 8)
//
// The "(this may take several minutes)" parenthetical on the transcription
// label is intentional — that stage genuinely can run 5-30+ min on chatty
// VODs (transcription work scales with candidate-window count, not VOD
// length). Users were interpreting the slow-moving bar as a crash; the
// inline expectation-setting reduces support traffic.
function analysisStageText(progress: number): string {
  if (progress < 5) return 'Starting analysis...'
  if (progress < 15) return 'Extracting audio...'
  if (progress < 18) return 'Analyzing chat activity...'
  if (progress < 22) return 'Selecting moments to transcribe...'
  if (progress < 40) return 'Transcribing audio (this may take several minutes)...'
  if (progress < 65) return 'Selecting clip candidates...'
  if (progress < 75) return 'Scoring and ranking clips...'
  if (progress < 83) return 'Generating titles and captions...'
  if (progress < 95) return 'Creating clip files...'
  return 'Finishing up...'
}

function formatBytes(bytes: number) {
  if (bytes === 0) return '0 B'
  const k = 1024
  const sizes = ['B', 'KB', 'MB', 'GB']
  const i = Math.floor(Math.log(bytes) / Math.log(k))
  return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`
}

const statusBadge: Record<string, string> = {
  pending: 'bg-slate-500/20 text-slate-400 border-slate-500/30',
  downloading: 'bg-blue-500/20 text-blue-400 border-blue-500/30',
  downloaded: 'bg-emerald-500/20 text-emerald-400 border-emerald-500/30',
  analyzing: 'bg-amber-500/20 text-amber-400 border-amber-500/30',
  completed: 'bg-emerald-500/20 text-emerald-400 border-emerald-500/30',
  failed: 'bg-red-500/20 text-red-400 border-red-500/30',
}

// Map analysis status → v4-vod-status class variant.
function v4StatusClass(vod: { analysis_status: string; download_status: string }): string {
  if (vod.analysis_status === 'completed') return 'done'
  if (vod.analysis_status === 'analyzing') return 'analyzing'
  if (vod.analysis_status === 'failed') return 'failed'
  if (vod.download_status === 'failed') return 'failed'
  if (vod.download_status === 'downloading' || vod.download_status === 'downloaded') return 'queued'
  return 'queued'
}

function v4StatusLabel(vod: { analysis_status: string; analysis_progress?: number; download_status: string; download_progress?: number }): string {
  if (vod.analysis_status === 'completed') return 'COMPLETE'
  if (vod.analysis_status === 'analyzing') return `ANALYZING · ${vod.analysis_progress ?? 0}%`
  if (vod.analysis_status === 'failed') return 'FAILED · RETRY'
  if (vod.download_status === 'failed') return 'DOWNLOAD FAILED · RETRY'
  if (vod.download_status === 'downloading') return `DOWNLOADING · ${vod.download_progress ?? 0}%`
  if (vod.download_status === 'downloaded') return 'READY TO ANALYZE'
  return 'PENDING'
}

export default function Vods() {
  const { loggedInUser, vods, isLoading, checkLogin, fetchVods, refreshVods, removeVod, updateVod, removeClipsForVod } = useAppStore()
  // Phase A v1.3.13: review tools require BOTH the developer-mode unlock AND
  // the explicit showReviewTools toggle. Either being false hides the UI.
  const showReviewTools = useUiStore(
    (s) => s.settings.developerModeUnlocked && s.settings.showReviewTools,
  )
  // BYOK cost UI: only show estimates/actuals when a paid provider is active.
  const aiNotFree = useAiStore((s) => s.effectiveMode() !== 'free')
  const aiEstimateKey = useAiStore((s) => {
    const settings = s.settings
    const titleModel = settings[`${settings.provider}Model` as keyof typeof settings]
    return [
      settings.provider,
      titleModel,
      settings.claudeJudgeModel,
      settings.useSonnetFinalPass,
      settings.useForTitles,
    ].join(':')
  })
  const navigate = useNavigate()
  const [refreshingId, setRefreshingId] = useState<string | null>(null)
  const [refreshedId, setRefreshedId] = useState<string | null>(null)
  const [deleteConfirmId, setDeleteConfirmId] = useState<string | null>(null)
  const [diskUsage, setDiskUsage] = useState<{
    has_file: boolean
    vod_size: number
    clip_count: number
    clips_size: number
    total_size: number
  } | null>(null)
  const [deleting, setDeleting] = useState(false)
  const [editingGameId, setEditingGameId] = useState<string | null>(null)
  const [gameInput, setGameInput] = useState('')
  const [restoringVods, setRestoringVods] = useState(false)
  const [showImportDialog, setShowImportDialog] = useState(false)
  const [detectionStats, setDetectionStats] = useState<Record<string, { candidatesFound: number; candidatesRejected: number; duplicatesSuppressed: number; clipsSelected: number; sensitivity: string }>>({})
  const detectionStatsRequestedRef = useRef(new Set<string>())
  // Pre-run cost estimate per VOD (USD), keyed by vod.id. Only fetched when a
  // paid AI provider is active. null = not yet fetched / unavailable.
  const [analyzeEstimateState, setAnalyzeEstimateState] = useState<{
    key: string
    values: Record<string, number>
  }>({ key: '', values: {} })
  const analyzeEstimates = analyzeEstimateState.key === aiEstimateKey
    ? analyzeEstimateState.values
    : {}
  const analyzeEstimateRequestsRef = useRef<{ key: string; vodIds: Set<string> }>({
    key: '',
    vodIds: new Set(),
  })
  // After an analyze completes we surface the actual spend in a brief toast.
  const [analyzeCostUsd, setAnalyzeCostUsd] = useState<number | null>(null)

  // Load detection stats for completed VODs
  useEffect(() => {
    const loadStats = async () => {
      for (const vod of vods) {
        if (vod.analysis_status === 'completed' && !detectionStatsRequestedRef.current.has(vod.id)) {
          detectionStatsRequestedRef.current.add(vod.id)
          try {
            const stats = await invoke<{ candidates_found: number; candidates_rejected: number; duplicates_suppressed: number; clips_selected: number; sensitivity: string } | null>('get_detection_stats', { vodId: vod.id })
            if (stats) {
              setDetectionStats(prev => ({
                ...prev,
                [vod.id]: {
                  candidatesFound: stats.candidates_found,
                  candidatesRejected: stats.candidates_rejected,
                  duplicatesSuppressed: stats.duplicates_suppressed,
                  clipsSelected: stats.clips_selected,
                  sensitivity: stats.sensitivity,
                },
              }))
            }
          } catch {
            detectionStatsRequestedRef.current.delete(vod.id)
          }
        }
      }
    }
    if (vods.length > 0) loadStats()
  }, [vods])

  // Pre-run cost estimates: ask the backend what an analyze would cost for each
  // VOD's duration. Only when a paid provider is active; free mode shows nothing.
  useEffect(() => {
    if (analyzeEstimateRequestsRef.current.key !== aiEstimateKey) {
      analyzeEstimateRequestsRef.current = { key: aiEstimateKey, vodIds: new Set() }
    }
    if (!aiNotFree) return
    let cancelled = false
    const loadEstimates = async () => {
      for (const vod of vods) {
        if (analyzeEstimateRequestsRef.current.vodIds.has(vod.id)) continue
        analyzeEstimateRequestsRef.current.vodIds.add(vod.id)
        try {
          const usd = await invoke<number>('estimate_analyze_cost', { durationSecs: vod.duration_seconds })
          if (!cancelled) {
            setAnalyzeEstimateState((previous) => ({
              key: aiEstimateKey,
              values: {
                ...(previous.key === aiEstimateKey ? previous.values : {}),
                [vod.id]: usd,
              },
            }))
          }
        } catch {
          analyzeEstimateRequestsRef.current.vodIds.delete(vod.id)
        }
      }
    }
    if (vods.length > 0) loadEstimates()
    return () => { cancelled = true }
  }, [vods, aiNotFree, aiEstimateKey])

  useEffect(() => {
    checkLogin()
  }, [checkLogin])

  // Auto-dismiss the "this analyze cost" toast a few seconds after it appears.
  useEffect(() => {
    if (analyzeCostUsd === null) return
    const t = setTimeout(() => setAnalyzeCostUsd(null), 6000)
    return () => clearTimeout(t)
  }, [analyzeCostUsd])

  useEffect(() => {
    if (loggedInUser) {
      console.log('[Vods] Restoring deleted VODs then fetching, channelId=', loggedInUser.id)
      invoke('restore_deleted_vods')
        .then(() => fetchVods(loggedInUser.id))
        .then(() => {
          console.log('[Vods] fetchVods resolved, vods in store:', useAppStore.getState().vods.length)
        })
    }
  }, [loggedInUser, fetchVods])

  // Poll for status updates while any VOD is downloading (reads from DB only, no loading spinner)
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null)
  useEffect(() => {
    const hasDownloading = vods.some((v) => v.download_status === 'downloading')
    if (hasDownloading && loggedInUser && !pollRef.current) {
      pollRef.current = setInterval(() => {
        refreshVods(loggedInUser.id)
      }, 3000)
    } else if (!hasDownloading && pollRef.current) {
      clearInterval(pollRef.current)
      pollRef.current = null
    }
    return () => {
      if (pollRef.current) {
        clearInterval(pollRef.current)
        pollRef.current = null
      }
    }
  }, [vods, loggedInUser, refreshVods])

  const handleExportReviewData = async (vodId: string) => {
    try {
      const json = await invoke<string>('export_review_data_for_vod', { vodId })
      await navigator.clipboard.writeText(json)
      // Success path is silent — clipboard now contains the data, paste to verify.
      // Matches the alert-only-on-failure convention used throughout this file.
    } catch (e) {
      console.error('Failed to export review data:', e)
      alert(`Failed to export review data: ${e}`)
    }
  }

  const handleDownload = async (vodId: string) => {
    try {
      await invoke('download_vod', { vodId })
    } catch (err) {
      alert(`Download failed: ${err}`)
    }
    // Refresh immediately from cache to show "downloading" status
    if (loggedInUser) refreshVods(loggedInUser.id)
  }

  const [updatingYtdlpVodId, setUpdatingYtdlpVodId] = useState<string | null>(null)

  const handleUpdateYtdlpAndRetry = async (vodId: string) => {
    if (updatingYtdlpVodId) return // double-click guard
    setUpdatingYtdlpVodId(vodId)
    try {
      await invoke('force_refresh_ytdlp')
      await invoke('download_vod', { vodId })
    } catch (err) {
      alert(`Update & retry failed: ${err}`)
    } finally {
      setUpdatingYtdlpVodId(null)
      if (loggedInUser) refreshVods(loggedInUser.id)
    }
  }

  const handleAnalyze = async (vodId: string) => {
    try {
      await invoke('analyze_vod', { vodId })
      // Analysis runs in background — poll DB to track progress.
      // Stale detection: if progress doesn't change for a long time, mark as failed.
      // Transcription alone can take 10-20+ min for long VODs, so we use a generous timeout.
      let lastProgress = -1
      let lastActivityTime = Date.now()
      // 10 minutes with zero progress change = likely stuck.
      // The backend sends heartbeats that update progress during transcription,
      // so even slow transcription will show incremental progress updates.
      // Audio extraction on 4-hour VODs can run ~3 minutes without emitting
      // progress, so a 10-minute floor prevents false stall alerts.
      // 30 min of zero progress change = backend likely crashed.
      // Long VODs (e.g. 7h Otzdarva streams) transcribe in the 20-38% range
      // over 60-90 min on CPU — progress only moves ~1% every 3-5 min during
      // that stage, so the old 10 min threshold falsely fired on legitimate
      // long analyses. 30 min keeps genuine-crash detection while allowing
      // honest long transcriptions to finish.
      const STALE_TIMEOUT_MS = 30 * 60 * 1000

      const poll = setInterval(async () => {
        try {
          if (loggedInUser) refreshVods(loggedInUser.id)
          const vod = await invoke<{ analysis_status: string; analysis_progress?: number }>('get_vod_detail', { vodId })
          if (vod.analysis_status === 'completed') {
            clearInterval(poll)
            if (loggedInUser) refreshVods(loggedInUser.id)
            // Surface the actual spend for this analyze (paid providers only),
            // in an understated toast. Fetch before navigating so the value is
            // ready; a small nav delay lets the toast paint on this page first.
            if (aiNotFree) {
              try {
                const usd = await invoke<number>('get_analysis_cost', { vodId })
                if (usd > 0) setAnalyzeCostUsd(usd)
              } catch { /* cost unavailable — skip the toast */ }
            }
            // Pass the just-completed VOD's id through navigation state so the
            // Clips page can scroll directly to its section instead of landing
            // somewhere in the middle of the (potentially long) clip list.
            navigate('/clips', { state: { focusVodId: vodId } })
          } else if (vod.analysis_status === 'failed') {
            clearInterval(poll)
            if (loggedInUser) refreshVods(loggedInUser.id)
            alert('Analysis failed. Please try again.')
          } else if (vod.analysis_status === 'analyzing') {
            const currentProgress = vod.analysis_progress ?? 0
            // Reset the stale timer on ANY progress change
            if (currentProgress !== lastProgress) {
              lastProgress = currentProgress
              lastActivityTime = Date.now()
            } else if (Date.now() - lastActivityTime > STALE_TIMEOUT_MS) {
              // No progress change for 30 minutes — backend likely crashed
              clearInterval(poll)
              console.warn(`[Vods] Analysis stale for ${vodId} at ${currentProgress}% for ${STALE_TIMEOUT_MS/1000}s — marking as failed`)
              try {
                await invoke('set_vod_analysis_status', { vodId, status: 'failed' })
              } catch {
                // Best-effort status update
              }
              if (loggedInUser) refreshVods(loggedInUser.id)
              alert('Analysis appears to have stalled. Please try again.')
            }
          }
        } catch {
          // ignore poll errors
        }
      }, 2000)
    } catch (err) {
      alert(`Analysis failed: ${err}`)
      if (loggedInUser) fetchVods(loggedInUser.id)
    }
  }

  const handleOpenVod = async (vodId: string) => {
    try {
      await invoke('open_vod', { vodId })
    } catch (err) {
      console.error('Failed to open VOD:', err)
    }
  }

  const handleRefreshMetadata = async (vodId: string) => {
    setRefreshingId(vodId)
    try {
      await invoke('refresh_vod_metadata', { vodId })
      if (loggedInUser) refreshVods(loggedInUser.id)
      setRefreshedId(vodId)
      setTimeout(() => setRefreshedId(null), 2000)
    } catch (err) {
      alert(`Refresh failed: ${err}`)
    } finally {
      setRefreshingId(null)
    }
  }

  const handleShowDeleteConfirm = async (vodId: string) => {
    setDeleteConfirmId(vodId)
    try {
      const usage = await invoke<NonNullable<typeof diskUsage>>('get_vod_disk_usage', { vodId })
      setDiskUsage(usage)
    } catch {
      setDiskUsage(null)
    }
  }

  const handleDeleteVodFile = async (vodId: string) => {
    setDeleting(true)
    try {
      await invoke('delete_vod_file', { vodId })
      // Immediately update local state — file-only delete resets download status
      updateVod(vodId, {
        download_status: 'pending',
        local_path: null,
        download_progress: 0,
      })
      setDeleteConfirmId(null)
      setDiskUsage(null)
    } catch (err) {
      alert(`Delete failed: ${err}`)
    } finally {
      setDeleting(false)
    }
  }

  const handleDeleteVodAndClips = async (vodId: string) => {
    setDeleting(true)
    try {
      console.log('[Vods] Deleting VOD:', vodId)
      const freedBytes = await invoke<number>('delete_vod_and_clips', { vodId })
      console.log('[Vods] Delete succeeded, freed bytes:', freedBytes)
      // Remove from store immediately for instant UI feedback
      removeVod(vodId)
      removeClipsForVod(vodId)
      setDeleteConfirmId(null)
      setDiskUsage(null)
      // Re-fetch from Twitch so the VOD reappears as available to download again.
      // First clear its deleted_vods entry, then fetch fresh from API.
      if (loggedInUser) {
        await invoke('restore_deleted_vods')
        await fetchVods(loggedInUser.id)
      }
    } catch (err) {
      console.error('[Vods] Delete FAILED:', err)
      alert(`Delete failed: ${err}`)
    } finally {
      setDeleting(false)
    }
  }

  const handleSetGame = async (vodId: string) => {
    try {
      const trimmed = gameInput.trim()
      await invoke('set_vod_game', { vodId, gameName: trimmed || null })
      if (loggedInUser) refreshVods(loggedInUser.id)
      setEditingGameId(null)
      setGameInput('')
    } catch (err) {
      alert(`Failed to set game: ${err}`)
    }
  }

  const startEditingGame = (vodId: string, currentGame: string | null) => {
    setEditingGameId(vodId)
    setGameInput(currentGame || '')
  }

  const handleRestoreDeletedVods = async () => {
    if (!loggedInUser) return
    setRestoringVods(true)
    try {
      await invoke('restore_deleted_vods')
      await fetchVods(loggedInUser.id)
    } catch (err) {
      alert(`Failed to restore VODs: ${err}`)
    } finally {
      setRestoringVods(false)
    }
  }

  if (!loggedInUser) {
    return (
      <div className="space-y-6">
        <div className="v4-page-header">
          <div>
            <div className="v4-page-title">VODs 📺</div>
            <div className="v4-page-sub">Connect Twitch to view and analyze your VODs</div>
          </div>
        </div>
        <div className="bg-surface-800 border border-surface-700 rounded-xl p-12 text-center">
          <div className="flex items-center justify-center w-16 h-16 rounded-2xl bg-violet-600/20 mx-auto mb-5">
            <Tv className="w-8 h-8 text-violet-400" />
          </div>
          <h3 className="text-lg font-medium text-white mb-2">
            Connect Your Twitch Account
          </h3>
          <p className="text-slate-400 text-sm mb-6">
            Log in with Twitch to view and analyze your VODs.
          </p>
          <button
            onClick={() => navigate('/settings')}
            className="inline-flex items-center gap-2 px-5 py-2.5 bg-[#9146FF] hover:bg-[#7c3aed] text-white text-sm font-semibold rounded-lg transition-colors cursor-pointer"
          >
            <LogIn className="w-4 h-4" />
            Log in with Twitch
          </button>
        </div>
      </div>
    )
  }

  const vodsAnalyzing = vods.filter(v => v.analysis_status === 'analyzing').length
  const vodsComplete = vods.filter(v => v.analysis_status === 'completed').length
  const vodsFailed = vods.filter(v => v.analysis_status === 'failed').length

  return (
    <div className="space-y-6">
      <div className="v4-page-header">
        <div>
          <div className="v4-page-title">VODs 📺</div>
          <div className="v4-page-sub">
            {vods.length} total · {vodsAnalyzing} analyzing · {vodsComplete} completed{vodsFailed > 0 ? ` · ${vodsFailed} failed` : ''}
          </div>
        </div>
        <div className="v4-page-actions">
          {/* Import-by-URL is dev-only. Shipping it would let users grab any
              public Twitch VOD, which violates Twitch ToS and exposes the
              project to DMCA / streamer disputes. Vite gates `DEV` to true
              for `cargo tauri dev` and false for `cargo tauri build`. */}
          {import.meta.env.DEV && (
            <button
              onClick={() => setShowImportDialog(true)}
              className="v4-btn primary"
              title="Import a Twitch VOD by pasting its URL (dev builds only)"
            >
              📥 Import VOD
            </button>
          )}
          <button
            onClick={handleRestoreDeletedVods}
            disabled={restoringVods}
            className="v4-btn ghost"
            title="Re-fetch all VODs from Twitch, including previously deleted ones"
          >
            <Plus className="w-3.5 h-3.5" />
            {restoringVods ? 'Fetching...' : 'Re-fetch VODs'}
          </button>
          {loggedInUser.profile_image_url && (
            <img
              src={loggedInUser.profile_image_url}
              alt={loggedInUser.display_name}
              className="w-8 h-8 rounded-full bg-surface-700"
            />
          )}
          <span className="text-sm text-slate-300 font-medium">
            {loggedInUser.display_name}
          </span>
        </div>
      </div>

      {isLoading ? (
        <div className="v4-panel text-center p-12">
          <p className="text-slate-400 text-sm">Loading VODs...</p>
        </div>
      ) : vods.length === 0 ? (
        <div className="v4-panel text-center p-12">
          <Video className="w-12 h-12 text-slate-600 mx-auto mb-4" />
          <h3 className="text-lg font-medium text-white mb-2">No VODs found</h3>
          <p className="text-slate-400 text-sm">
            Your channel has no VODs available yet. Start streaming to create VODs!
          </p>
        </div>
      ) : (
        <div className="v4-vod-grid">
          {vods.map((vod) => (
            <div key={vod.id} className="v4-vod-card flex flex-col">
              {/* Thumbnail */}
              <div className="v4-vod-thumb" style={{height:150}}>
                {/* Preview: a working Twitch thumbnail, else a frame from the
                    local file for downloaded VODs, else the card gradient + title. */}
                <VodThumb
                  vodId={vod.id}
                  downloaded={vod.download_status === 'downloaded'}
                  thumbnailUrl={vod.thumbnail_url}
                  title={vod.title}
                />
                <span className={`v4-vod-status ${v4StatusClass(vod)}`}>
                  {v4StatusLabel(vod)}
                </span>
                <span className="v4-vod-dur">
                  {formatDuration(vod.duration_seconds)}
                </span>
                {vod.analysis_status === 'analyzing' && (
                  <div className="v4-vod-progress">
                    <div className="v4-vod-progress-bar" style={{width:`${vod.analysis_progress || 0}%`}} />
                  </div>
                )}
                {vod.download_status === 'downloading' && vod.analysis_status !== 'analyzing' && (
                  <div className="v4-vod-progress">
                    <div className="v4-vod-progress-bar" style={{width:`${vod.download_progress || 0}%`}} />
                  </div>
                )}
                {/* Play/Download overlay on thumbnail */}
                {vod.download_status === 'downloaded' ? (
                  <button
                    onClick={() => navigate(`/player/${vod.id}`)}
                    className="absolute inset-0 flex items-center justify-center bg-black/40 hover:bg-black/30 transition-colors cursor-pointer group/play"
                  >
                    <Play className="w-10 h-10 text-white/90 drop-shadow group-hover/play:scale-110 transition-transform" />
                  </button>
                ) : vod.download_status === 'downloading' ? (
                  <div className="absolute inset-0 flex items-center justify-center bg-black/50">
                    <div className="flex flex-col items-center gap-1">
                      <Download className="w-8 h-8 text-blue-400 animate-bounce" />
                      <span className="text-xs text-blue-300 font-medium">{vod.download_progress}%</span>
                    </div>
                  </div>
                ) : (
                  <button
                    onClick={() => handleDownload(vod.id)}
                    className="absolute inset-0 flex items-center justify-center bg-black/40 hover:bg-black/30 transition-colors cursor-pointer group/play"
                  >
                    <div className="flex flex-col items-center gap-1">
                      <Download className="w-8 h-8 text-white/80 drop-shadow group-hover/play:scale-110 transition-transform" />
                      <span className="text-[10px] text-white/70 font-medium">Download to play</span>
                    </div>
                  </button>
                )}
              </div>

              {/* Info */}
              <div className="v4-vod-body flex-1 flex flex-col gap-3">
                <h3 className="v4-vod-title" title={vod.title} style={{whiteSpace:'normal',display:'-webkit-box',WebkitLineClamp:2,WebkitBoxOrient:'vertical',overflow:'hidden'}}>
                  {vod.title}
                </h3>
                <p className="text-xs text-slate-500">{formatDate(vod.stream_date)}</p>
                {editingGameId === vod.id ? (
                  <div className="flex items-center gap-1.5">
                    <input
                      type="text"
                      value={gameInput}
                      onChange={e => setGameInput(e.target.value)}
                      onKeyDown={e => {
                        if (e.key === 'Enter') handleSetGame(vod.id)
                        if (e.key === 'Escape') { setEditingGameId(null); setGameInput('') }
                      }}
                      placeholder="e.g. Dead by Daylight, Valorant"
                      autoFocus
                      className="flex-1 min-w-0 bg-surface-900 border border-violet-500/50 text-white text-xs px-2 py-1 rounded-lg outline-none focus:border-violet-400"
                    />
                    <button
                      onClick={() => handleSetGame(vod.id)}
                      className="text-xs px-2 py-1 bg-violet-600 hover:bg-violet-500 text-white rounded-lg cursor-pointer"
                    >
                      Save
                    </button>
                    <button
                      onClick={() => { setEditingGameId(null); setGameInput('') }}
                      className="text-xs px-1.5 py-1 text-slate-400 hover:text-white cursor-pointer"
                    >
                      <X className="w-3 h-3" />
                    </button>
                  </div>
                ) : (
                  <button
                    onClick={() => startEditingGame(vod.id, vod.game_name)}
                    className={`inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded-full w-fit cursor-pointer transition-colors ${
                      vod.game_name
                        ? 'bg-violet-500/20 text-violet-400 border border-violet-500/30 hover:bg-violet-500/30'
                        : 'bg-surface-700 text-slate-500 border border-surface-600 hover:text-slate-300 hover:border-surface-500'
                    }`}
                    title={vod.game_name ? 'Click to change game (applies to all clips from this VOD)' : 'Set game for all clips from this VOD'}
                  >
                    <Gamepad2 className="w-3 h-3" />
                    {vod.game_name || 'Set game'}
                  </button>
                )}

                {/* Status badges */}
                <div className="flex gap-2 flex-wrap">
                  <span
                    className={`text-xs px-2 py-0.5 rounded-full border ${statusBadge[vod.download_status]}`}
                  >
                    DL: {vod.download_status}
                  </span>
                  <span
                    className={`text-xs px-2 py-0.5 rounded-full border ${statusBadge[vod.analysis_status]}`}
                  >
                    AI: {vod.analysis_status}
                  </span>
                </div>

                {/* Download Progress Bar */}
                {vod.download_status === 'downloading' && (
                  <div className="space-y-1">
                    <div className="flex justify-between text-xs">
                      <span className="text-blue-400">Downloading...</span>
                      <span className="text-blue-400">{vod.download_progress}%</span>
                    </div>
                    <div className="w-full bg-surface-900 rounded-full h-1.5 overflow-hidden">
                      <div
                        className="bg-blue-500 h-full rounded-full transition-all duration-500 ease-out"
                        style={{ width: `${vod.download_progress}%` }}
                      />
                    </div>
                  </div>
                )}

                {/* Analysis Progress Bar */}
                {vod.analysis_status === 'analyzing' && (
                  <div className="space-y-1">
                    <div className="flex justify-between text-xs">
                      <span className="text-violet-400">{analysisStageText(vod.analysis_progress || 0)}</span>
                      <span className="text-violet-400">{vod.analysis_progress || 0}%</span>
                    </div>
                    <div className="w-full bg-surface-900 rounded-full h-1.5 overflow-hidden">
                      <div
                        className="bg-violet-500 h-full rounded-full transition-all duration-500 ease-out"
                        style={{ width: `${vod.analysis_progress || 0}%` }}
                      />
                    </div>
                  </div>
                )}

                {/* Detection Stats */}
                {vod.analysis_status === 'completed' && detectionStats[vod.id] && (
                  <div className="text-[10px] text-slate-500 bg-surface-900 rounded-lg px-3 py-1.5">
                    Found {detectionStats[vod.id].candidatesFound} potential moments, selected top {detectionStats[vod.id].clipsSelected} clips
                    {detectionStats[vod.id].sensitivity !== 'medium' && (
                      <span className="ml-1 text-violet-400/70">({detectionStats[vod.id].sensitivity} sensitivity)</span>
                    )}
                  </div>
                )}

                {/* Pre-run AI cost estimate — paid providers only, when ready to analyze */}
                {aiNotFree && vod.download_status === 'downloaded' && vod.analysis_status !== 'analyzing'
                  && analyzeEstimates[vod.id] !== undefined && analyzeEstimates[vod.id] > 0 && (
                  <p className="text-[10px] text-slate-500" title="Estimated AI provider cost for analyzing this VOD">
                    ~${analyzeEstimates[vod.id].toFixed(2)} est.
                  </p>
                )}

                {/* Actions — primary row */}
                <div className="flex gap-2 mt-auto">
                  {vod.download_status === 'failed' ? (
                    <button
                      onClick={() => handleUpdateYtdlpAndRetry(vod.id)}
                      disabled={updatingYtdlpVodId !== null}
                      className="flex-1 flex items-center justify-center gap-1.5 px-3 py-2 text-xs rounded-lg transition-colors cursor-pointer bg-amber-500/20 text-amber-400 border border-amber-500/30 hover:bg-amber-500/30 disabled:opacity-40"
                      title="Twitch download failed — usually a stale yt-dlp. This updates yt-dlp and retries."
                    >
                      {updatingYtdlpVodId === vod.id ? 'Updating yt-dlp…' : 'Update yt-dlp & Retry'}
                    </button>
                  ) : vod.download_status === 'downloaded' ? (
                    <button
                      onClick={() => navigate(`/player/${vod.id}`)}
                      className="flex-1 flex items-center justify-center gap-1.5 px-3 py-2 text-xs rounded-lg transition-colors cursor-pointer bg-emerald-500/20 text-emerald-400 border border-emerald-500/30 hover:bg-emerald-500/30"
                    >
                      <Check className="w-3.5 h-3.5" /> Open
                    </button>
                  ) : (
                    <button
                      onClick={() => handleDownload(vod.id)}
                      disabled={vod.download_status === 'downloading'}
                      className="flex-1 flex items-center justify-center gap-1.5 px-3 py-2 text-xs rounded-lg transition-colors cursor-pointer bg-surface-700 hover:bg-surface-600 disabled:opacity-40 text-slate-200"
                    >
                      {vod.download_status === 'downloading' ? (
                        <><Download className="w-3.5 h-3.5 animate-bounce" /> {vod.download_progress}%</>
                      ) : (
                        <><Download className="w-3.5 h-3.5" /> Download</>
                      )}
                    </button>
                  )}
                  {vod.analysis_status === 'completed' && (
                    <button
                      onClick={() => navigate('/clips')}
                      className="flex-1 flex items-center justify-center gap-1.5 px-3 py-2 text-xs rounded-lg transition-colors cursor-pointer bg-violet-500/20 text-violet-400 border border-violet-500/30 hover:bg-violet-500/30"
                    >
                      <Search className="w-3.5 h-3.5" />
                      View Clips
                    </button>
                  )}
                  {showReviewTools && vod.analysis_status === 'completed' && (
                    <button
                      onClick={() => handleExportReviewData(vod.id)}
                      className="flex items-center justify-center gap-1.5 px-3 py-2 text-xs rounded-lg transition-colors cursor-pointer bg-amber-500/20 text-amber-400 border border-amber-500/30 hover:bg-amber-500/30"
                      title="Copy clip-by-clip review data to clipboard for offline analysis"
                    >
                      <Download className="w-3.5 h-3.5" />
                      Export Reviews
                    </button>
                  )}
                  <button
                    onClick={() => handleAnalyze(vod.id)}
                    disabled={vod.analysis_status === 'analyzing'}
                    className={`${vod.analysis_status === 'completed' ? '' : 'flex-1'} flex items-center justify-center gap-1.5 px-3 py-2 text-xs rounded-lg transition-colors cursor-pointer disabled:opacity-40 ${
                      vod.analysis_status === 'completed'
                        ? 'bg-surface-800 text-slate-400 border border-surface-600 hover:text-white hover:border-surface-500'
                        : vod.analysis_status === 'failed'
                          ? 'bg-red-500/10 text-red-400 border border-red-500/30 hover:bg-red-500/20'
                          : 'bg-surface-700 hover:bg-surface-600 text-slate-200'
                    }`}
                  >
                    {vod.analysis_status === 'analyzing' ? (
                      <><Search className="w-3.5 h-3.5 animate-pulse" /> Analyzing...</>
                    ) : vod.analysis_status === 'completed' ? (
                      <><RotateCcw className="w-3.5 h-3.5" /> Re-analyze</>
                    ) : vod.analysis_status === 'failed' ? (
                      <><RotateCcw className="w-3.5 h-3.5" /> Retry</>
                    ) : (
                      <><Search className="w-3.5 h-3.5" /> Analyze</>
                    )}
                  </button>
                </div>
                {/* Utility icons row */}
                <div className="flex items-center gap-2">
                  <button
                    onClick={() => handleOpenVod(vod.id)}
                    title="Watch on Twitch"
                    className="flex items-center justify-center px-2 py-1.5 bg-surface-700 hover:bg-surface-600 text-slate-400 hover:text-slate-200 rounded-lg transition-colors cursor-pointer"
                  >
                    <Eye className="w-3.5 h-3.5" />
                  </button>
                  <button
                    onClick={() => handleRefreshMetadata(vod.id)}
                    disabled={refreshingId === vod.id}
                    title="Refresh metadata from Twitch"
                    className="flex items-center justify-center px-2 py-1.5 bg-surface-700 hover:bg-surface-600 text-slate-400 hover:text-slate-200 rounded-lg transition-colors cursor-pointer disabled:opacity-40"
                  >
                    <RefreshCw className={`w-3.5 h-3.5 ${refreshingId === vod.id ? 'animate-spin' : ''}`} />
                  </button>
                  {refreshedId === vod.id && (
                    <span className="text-xs text-emerald-400">Updated!</span>
                  )}
                  <button
                    onClick={() => handleShowDeleteConfirm(vod.id)}
                    title="Delete VOD"
                    className="flex items-center justify-center px-2 py-1.5 bg-surface-700 hover:bg-red-900/40 text-slate-400 hover:text-red-400 rounded-lg transition-colors cursor-pointer"
                  >
                    <Trash2 className="w-3.5 h-3.5" />
                  </button>
                </div>

                {deleteConfirmId === vod.id && (
                  <div className="mt-3 p-3 bg-red-950/40 border border-red-500/30 rounded-lg space-y-2">
                    <p className="text-xs text-red-300 font-medium">Free up disk for this VOD?</p>
                    <p className="text-[11px] text-slate-400">Removes the local files only — the VOD stays in your list and re-downloadable from Twitch. If it's aged off Twitch, freeing it means it's gone for good.</p>
                    {diskUsage && (
                      <div className="text-xs text-slate-400 space-y-1">
                        {diskUsage.has_file && <p>VOD file: {formatBytes(diskUsage.vod_size)}</p>}
                        {diskUsage.clip_count > 0 && <p>{diskUsage.clip_count} clips: {formatBytes(diskUsage.clips_size)}</p>}
                      </div>
                    )}
                    <div className="flex flex-col gap-1.5">
                      {diskUsage?.has_file && (
                        <button
                          onClick={() => handleDeleteVodFile(vod.id)}
                          disabled={deleting}
                          className="w-full text-left px-2.5 py-1.5 text-xs rounded-lg bg-surface-800 border border-surface-600 text-slate-300 hover:border-red-500/50 hover:text-red-300 transition-colors cursor-pointer disabled:opacity-40"
                        >
                          Delete download only {diskUsage ? `(frees ${formatBytes(diskUsage.vod_size)})` : ''} — keeps clips
                        </button>
                      )}
                      <button
                        onClick={() => handleDeleteVodAndClips(vod.id)}
                        disabled={deleting}
                        className="w-full text-left px-2.5 py-1.5 text-xs rounded-lg bg-red-900/30 border border-red-500/30 text-red-300 hover:bg-red-900/50 transition-colors cursor-pointer disabled:opacity-40"
                      >
                        Delete download + clips {diskUsage ? `(frees ${formatBytes(diskUsage.total_size)})` : ''}
                      </button>
                      <button
                        onClick={() => { setDeleteConfirmId(null); setDiskUsage(null) }}
                        className="w-full text-center px-2.5 py-1.5 text-xs rounded-lg text-slate-500 hover:text-white transition-colors cursor-pointer"
                      >
                        Cancel
                      </button>
                    </div>
                  </div>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
      <ImportVodDialog
        open={showImportDialog}
        onClose={() => setShowImportDialog(false)}
      />

      {/* Post-analyze cost toast (paid providers only). Understated, auto-dismisses. */}
      {analyzeCostUsd !== null && (
        <div className="fixed bottom-6 left-1/2 -translate-x-1/2 z-50 bg-surface-700 border border-surface-600 rounded-xl px-4 py-3 shadow-2xl flex items-center gap-2 animate-slide-up">
          <span className="text-sm text-slate-300">this analyze: ~${analyzeCostUsd.toFixed(2)}</span>
        </div>
      )}
    </div>
  )
}
