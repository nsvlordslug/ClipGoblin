import { useEffect, useRef, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { Video, Download, Search, Eye, Tv, LogIn, Check, RotateCcw, RefreshCw, Trash2, X, Gamepad2, Plus } from 'lucide-react'
import { useAppStore } from '../stores/appStore'
import { invoke } from '@tauri-apps/api/core'

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

function analysisStageText(progress: number): string {
  if (progress < 5) return 'Starting analysis...'
  if (progress < 15) return 'Extracting audio...'
  if (progress < 20) return 'Audio extracted'
  if (progress < 40) return 'Transcribing audio...'
  if (progress < 50) return 'Analyzing chat activity...'
  if (progress < 60) return 'Selecting clip candidates...'
  if (progress < 75) return 'Scoring highlights...'
  if (progress < 83) return 'Generating captions...'
  if (progress < 88) return 'Creating clips...'
  if (progress < 98) return 'Generating thumbnails...'
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

export default function Vods() {
  const { loggedInUser, vods, isLoading, checkLogin, fetchVods, refreshVods, removeVod, updateVod, removeClipsForVod } = useAppStore()
  const navigate = useNavigate()
  const [refreshingId, setRefreshingId] = useState<string | null>(null)
  const [refreshedId, setRefreshedId] = useState<string | null>(null)
  const [deleteConfirmId, setDeleteConfirmId] = useState<string | null>(null)
  const [diskUsage, setDiskUsage] = useState<any>(null)
  const [deleting, setDeleting] = useState(false)
  const [editingGameId, setEditingGameId] = useState<string | null>(null)
  const [gameInput, setGameInput] = useState('')
  const [restoringVods, setRestoringVods] = useState(false)
  const [detectionStats, setDetectionStats] = useState<Record<string, { candidatesFound: number; candidatesRejected: number; duplicatesSuppressed: number; clipsSelected: number; sensitivity: string }>>({})

  // Load detection stats for completed VODs
  useEffect(() => {
    const loadStats = async () => {
      for (const vod of vods) {
        if (vod.analysis_status === 'completed' && !detectionStats[vod.id]) {
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
          } catch { /* stats not available for this VOD */ }
        }
      }
    }
    if (vods.length > 0) loadStats()
  }, [vods])

  useEffect(() => {
    checkLogin()
  }, [checkLogin])

  useEffect(() => {
    if (loggedInUser) {
      fetchVods(loggedInUser.id)
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

  const handleDownload = async (vodId: string) => {
    try {
      await invoke('download_vod', { vodId })
    } catch (err) {
      alert(`Download failed: ${err}`)
    }
    // Refresh immediately from cache to show "downloading" status
    if (loggedInUser) refreshVods(loggedInUser.id)
  }

  const handleAnalyze = async (vodId: string) => {
    try {
      await invoke('analyze_vod', { vodId })
      // Analysis runs in background — poll DB to track progress.
      // Stale detection: if progress doesn't change for a long time, mark as failed.
      // Transcription alone can take 10-20+ min for long VODs, so we use a generous timeout.
      let lastProgress = -1
      let lastActivityTime = Date.now()
      // 5 minutes with zero progress change = likely stuck.
      // The backend sends heartbeats that update progress during transcription,
      // so even slow transcription will show incremental progress updates.
      const STALE_TIMEOUT_MS = 5 * 60 * 1000

      const poll = setInterval(async () => {
        try {
          if (loggedInUser) refreshVods(loggedInUser.id)
          const vod = await invoke<{ analysis_status: string; analysis_progress?: number }>('get_vod_detail', { vodId })
          if (vod.analysis_status === 'completed') {
            clearInterval(poll)
            if (loggedInUser) refreshVods(loggedInUser.id)
            navigate('/clips')
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
              // No progress change for 5 minutes — backend likely crashed
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
      const usage = await invoke('get_vod_disk_usage', { vodId })
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
        download_status: 'pending' as any,
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
        <h1 className="text-2xl font-bold text-white">VODs</h1>
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
            onClick={() => navigate('/channels')}
            className="inline-flex items-center gap-2 px-5 py-2.5 bg-[#9146FF] hover:bg-[#7c3aed] text-white text-sm font-semibold rounded-lg transition-colors cursor-pointer"
          >
            <LogIn className="w-4 h-4" />
            Log in with Twitch
          </button>
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold text-white">VODs</h1>
        <div className="flex items-center gap-3">
          <button
            onClick={handleRestoreDeletedVods}
            disabled={restoringVods}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs text-slate-300 bg-surface-700 border border-surface-600 rounded-lg hover:bg-surface-600 hover:text-white transition-colors cursor-pointer disabled:opacity-50"
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
        <div className="bg-surface-800 border border-surface-700 rounded-xl p-12 text-center">
          <p className="text-slate-400 text-sm">Loading VODs...</p>
        </div>
      ) : vods.length === 0 ? (
        <div className="bg-surface-800 border border-surface-700 rounded-xl p-12 text-center">
          <Video className="w-12 h-12 text-slate-600 mx-auto mb-4" />
          <h3 className="text-lg font-medium text-white mb-2">No VODs found</h3>
          <p className="text-slate-400 text-sm">
            Your channel has no VODs available yet. Start streaming to create VODs!
          </p>
        </div>
      ) : (
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
          {vods.map((vod) => (
            <div
              key={vod.id}
              className="bg-surface-800 border border-surface-700 rounded-xl overflow-hidden flex flex-col"
            >
              {/* Thumbnail */}
              <div className="relative aspect-video bg-surface-700">
                {vod.thumbnail_url ? (
                  <img
                    src={vod.thumbnail_url}
                    alt={vod.title}
                    className="w-full h-full object-cover"
                  />
                ) : (
                  <div className="w-full h-full flex items-center justify-center">
                    <Video className="w-8 h-8 text-slate-600" />
                  </div>
                )}
                <span className="absolute bottom-2 right-2 bg-black/80 text-white text-xs px-1.5 py-0.5 rounded">
                  {formatDuration(vod.duration_seconds)}
                </span>
              </div>

              {/* Info */}
              <div className="p-4 flex-1 flex flex-col gap-3">
                <h3 className="text-sm font-medium text-white line-clamp-2 leading-snug">
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

                {/* Actions — primary row */}
                <div className="flex gap-2 mt-auto">
                  {vod.download_status === 'downloaded' ? (
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
                    <p className="text-xs text-red-300 font-medium">Delete this VOD?</p>
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
                          Delete file only {diskUsage ? `(frees ${formatBytes(diskUsage.vod_size)})` : ''} — keeps clips
                        </button>
                      )}
                      <button
                        onClick={() => handleDeleteVodAndClips(vod.id)}
                        disabled={deleting}
                        className="w-full text-left px-2.5 py-1.5 text-xs rounded-lg bg-red-900/30 border border-red-500/30 text-red-300 hover:bg-red-900/50 transition-colors cursor-pointer disabled:opacity-40"
                      >
                        Delete everything {diskUsage ? `(frees ${formatBytes(diskUsage.total_size)})` : ''} — removes VOD + clips
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
    </div>
  )
}
