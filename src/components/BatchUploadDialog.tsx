import { useState, useCallback, useRef } from 'react'
import { X, Upload, CheckCircle2, AlertCircle, Loader2, ChevronDown, ChevronUp, RotateCcw, ExternalLink, Clock } from 'lucide-react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { usePlatformStore, PLATFORM_INFO, type UploadResult } from '../stores/platformStore'
import { useScheduleStore } from '../stores/scheduleStore'
import type { Clip } from '../types'

// ── Types ──

interface ClipUploadStatus {
  clipId: string
  status: 'pending' | 'exporting' | 'uploading' | 'done' | 'error' | 'skipped'
  exportProgress?: number
  error?: string
  videoUrl?: string
  duplicateUrl?: string
}

interface BatchUploadDialogProps {
  clips: Clip[]
  onClose: () => void
  onComplete: () => void
}

// ── Helpers ──

function getDefaultVisibility(platform: string): string {
  if (platform === 'youtube') return 'unlisted'
  if (platform === 'tiktok') return 'private_self_only'
  return 'public'
}

function buildMetaForClip(clip: Clip, platform: string, visibility: string, useSavedCaptions: boolean, force: boolean) {
  const title = clip.title?.trim() || 'Untitled Clip'
  let description = ''
  let tags: string[] = []

  if (useSavedCaptions) {
    description = clip.publish_description || ''
    const rawHashtags = clip.publish_hashtags || ''
    tags = rawHashtags.split(',').map(t => t.trim()).filter(Boolean)
    const hashtagSuffix = tags.length > 0 ? tags.map(t => `#${t}`).join(' ') : ''
    if (hashtagSuffix) {
      description = description ? description + '\n\n' + hashtagSuffix : hashtagSuffix
    }
  }

  return {
    clip_id: clip.id,
    title,
    description,
    tags,
    visibility,
    force,
  }
}

/** Export a clip and wait for it to finish. Returns the refreshed clip. */
function exportClip(clipId: string, onProgress: (pct: number) => void): Promise<Clip> {
  return new Promise((resolve, reject) => {
    const jobId = `export-${clipId}`
    let unlistenFn: (() => void) | null = null

    listen<{ jobId: string; progress: number; status: string; error?: string }>('job-progress', (event) => {
      if (event.payload.jobId !== jobId) return
      const { progress, status, error } = event.payload
      onProgress(progress)

      if (status === 'completed') {
        unlistenFn?.()
        invoke<Clip>('get_clip_detail', { clipId })
          .then(resolve)
          .catch(() => reject(new Error('Export completed but failed to fetch clip')))
      } else if (status === 'failed') {
        unlistenFn?.()
        reject(new Error(error || 'Export failed'))
      }
    }).then(fn => {
      unlistenFn = fn
      invoke('export_clip', { clipId }).catch(err => {
        unlistenFn?.()
        reject(err)
      })
    })
  })
}

// ── Component ──

export default function BatchUploadDialog({ clips, onClose, onComplete }: BatchUploadDialogProps) {
  const { isConnected, connect } = usePlatformStore()
  const { schedule: scheduleUpload } = useScheduleStore()

  // Platform selection
  const availablePlatforms = Object.entries(PLATFORM_INFO).filter(([_, info]) => info.available)
  const [selectedPlatforms, setSelectedPlatforms] = useState<Record<string, boolean>>(() => {
    const init: Record<string, boolean> = {}
    for (const [key, info] of availablePlatforms) {
      init[key] = info.available && isConnected(key)
    }
    if (!Object.values(init).some(Boolean)) init['youtube'] = true
    return init
  })
  const [visibility, setVisibility] = useState<Record<string, string>>(() => {
    const init: Record<string, string> = {}
    for (const [key] of availablePlatforms) {
      init[key] = getDefaultVisibility(key)
    }
    return init
  })
  const [useSavedCaptions, setUseSavedCaptions] = useState(true)

  // Schedule state
  const [scheduleMode, setScheduleMode] = useState(false)
  const [scheduleTime, setScheduleTime] = useState('')
  const [scheduleComplete, setScheduleComplete] = useState(false)

  // Upload state
  const [uploading, setUploading] = useState(false)
  const [completed, setCompleted] = useState(false)
  const [clipStatuses, setClipStatuses] = useState<Record<string, Record<string, ClipUploadStatus>>>({})
  const [expanded, setExpanded] = useState(true)
  const cancelRef = useRef(false)
  // Keep a mutable ref to the latest version of each clip (updated after export)
  const clipMapRef = useRef<Record<string, Clip>>({})
  // Initialize clip map
  if (Object.keys(clipMapRef.current).length === 0) {
    for (const c of clips) clipMapRef.current[c.id] = c
  }

  const activePlatforms = Object.entries(selectedPlatforms).filter(([_, v]) => v).map(([k]) => k)
  const missingTitleClips = clips.filter(c => !c.title?.trim())

  // ALL clips participate (not just exported ones)
  const totalJobs = clips.length * activePlatforms.length
  const doneJobs = Object.values(clipStatuses).reduce((acc, platformMap) => {
    return acc + Object.values(platformMap).filter(s => s.status === 'done' || s.status === 'error' || s.status === 'skipped').length
  }, 0)
  const failedJobs = Object.values(clipStatuses).reduce((acc, platformMap) => {
    return acc + Object.values(platformMap).filter(s => s.status === 'error').length
  }, 0)

  const togglePlatform = (platform: string) => {
    setSelectedPlatforms(prev => ({ ...prev, [platform]: !prev[platform] }))
  }

  const updateClipStatus = useCallback((platform: string, clipId: string, update: Partial<ClipUploadStatus>) => {
    setClipStatuses(prev => ({
      ...prev,
      [platform]: {
        ...(prev[platform] || {}),
        [clipId]: { ...(prev[platform]?.[clipId] || { clipId, status: 'pending' }), ...update } as ClipUploadStatus,
      }
    }))
  }, [])

  /** Ensure a clip is exported, exporting it if needed. Returns the up-to-date clip. */
  const ensureExported = useCallback(async (clip: Clip, firstPlatform: string): Promise<Clip> => {
    // Check latest state — might have been exported in a previous iteration
    const current = clipMapRef.current[clip.id] || clip
    if (current.render_status === 'completed' && current.output_path) {
      return current
    }

    // Need to export — show exporting status on the first platform
    updateClipStatus(firstPlatform, clip.id, { status: 'exporting', exportProgress: 0 })

    const exported = await exportClip(clip.id, (pct) => {
      updateClipStatus(firstPlatform, clip.id, { status: 'exporting', exportProgress: pct })
    })

    // Update the ref so subsequent platforms see it as exported
    clipMapRef.current[clip.id] = exported
    return exported
  }, [updateClipStatus])

  // ── Sequential export-then-upload ──
  const startUpload = useCallback(async (retryOnly = false) => {
    cancelRef.current = false
    setUploading(true)
    setCompleted(false)

    // Ensure all selected platforms are connected
    for (const platform of activePlatforms) {
      if (!isConnected(platform)) {
        try {
          await connect(platform)
        } catch (e: any) {
          for (const clip of clips) {
            updateClipStatus(platform, clip.id, {
              status: 'error',
              error: `Failed to connect to ${PLATFORM_INFO[platform]?.name || platform}: ${e?.message || e}`,
            })
          }
          continue
        }
      }
    }

    // Process each clip: export if needed, then upload to each platform
    for (const clip of clips) {
      if (cancelRef.current) break

      // Skip already-done clips in retry mode
      const allDoneForClip = activePlatforms.every(p => {
        const st = clipStatuses[p]?.[clip.id]
        return st?.status === 'done'
      })
      if (allDoneForClip) continue

      // Export once for all platforms
      let exportedClip: Clip
      try {
        exportedClip = await ensureExported(clip, activePlatforms[0])
      } catch (e: any) {
        // Export failed — mark all platforms as error for this clip
        for (const platform of activePlatforms) {
          updateClipStatus(platform, clip.id, {
            status: 'error',
            error: `Export failed: ${typeof e === 'string' ? e : e?.message || 'Unknown error'}`,
          })
        }
        continue
      }

      // Upload to each platform
      for (const platform of activePlatforms) {
        if (cancelRef.current) break

        const existing = clipStatuses[platform]?.[clip.id]
        if (retryOnly && existing?.status === 'done') continue
        if (!retryOnly && existing?.status === 'done') continue
        if (!retryOnly && existing?.status === 'error') continue

        updateClipStatus(platform, clip.id, { status: 'uploading', error: undefined })

        try {
          const meta = buildMetaForClip(exportedClip, platform, visibility[platform] || getDefaultVisibility(platform), useSavedCaptions, false)
          const result = await invoke<UploadResult>('upload_to_platform', { platform, meta })

          if (result.status.status === 'complete') {
            updateClipStatus(platform, clip.id, { status: 'done', videoUrl: result.status.video_url })
          } else if (result.status.status === 'duplicate') {
            updateClipStatus(platform, clip.id, { status: 'done', duplicateUrl: result.status.existing_url })
          } else if (result.status.status === 'failed') {
            updateClipStatus(platform, clip.id, { status: 'error', error: result.status.error })
          } else {
            updateClipStatus(platform, clip.id, { status: 'done' })
          }
        } catch (e: any) {
          updateClipStatus(platform, clip.id, { status: 'error', error: typeof e === 'string' ? e : e?.message || 'Upload failed' })
        }
      }
    }

    setUploading(false)
    setCompleted(true)
  }, [activePlatforms, clips, clipStatuses, visibility, useSavedCaptions, isConnected, connect, updateClipStatus, ensureExported])

  const retryFailed = useCallback(() => {
    setClipStatuses(prev => {
      const updated = { ...prev }
      for (const platform of Object.keys(updated)) {
        const platformMap = { ...updated[platform] }
        for (const clipId of Object.keys(platformMap)) {
          if (platformMap[clipId].status === 'error') {
            platformMap[clipId] = { ...platformMap[clipId], status: 'pending', error: undefined }
          }
        }
        updated[platform] = platformMap
      }
      return updated
    })
    startUpload(true)
  }, [startUpload])

  const startSchedule = useCallback(async () => {
    if (!scheduleTime || activePlatforms.length === 0) return
    cancelRef.current = false
    setUploading(true)
    const isoTime = new Date(scheduleTime).toISOString()

    for (const clip of clips) {
      if (cancelRef.current) break

      // Export if needed before scheduling
      let exportedClip: Clip
      try {
        exportedClip = await ensureExported(clip, activePlatforms[0])
      } catch (e: any) {
        for (const platform of activePlatforms) {
          updateClipStatus(platform, clip.id, {
            status: 'error',
            error: `Export failed: ${typeof e === 'string' ? e : e?.message || 'Unknown error'}`,
          })
        }
        continue
      }

      for (const platform of activePlatforms) {
        if (cancelRef.current) break
        updateClipStatus(platform, clip.id, { status: 'uploading' })
        try {
          const meta = buildMetaForClip(exportedClip, platform, visibility[platform] || getDefaultVisibility(platform), useSavedCaptions, false)
          await scheduleUpload(clip.id, platform, isoTime, JSON.stringify(meta))
          updateClipStatus(platform, clip.id, { status: 'done' })
        } catch (e: any) {
          updateClipStatus(platform, clip.id, { status: 'error', error: typeof e === 'string' ? e : e?.message || 'Schedule failed' })
        }
      }
    }

    setUploading(false)
    setScheduleComplete(true)
    setCompleted(true)
  }, [scheduleTime, activePlatforms, clips, visibility, useSavedCaptions, cancelRef, updateClipStatus, scheduleUpload, ensureExported])

  const handleCancel = () => {
    cancelRef.current = true
  }

  // ── Render ──
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
      <div className="bg-surface-800 border border-surface-600 rounded-2xl w-full max-w-lg mx-4 max-h-[85vh] flex flex-col shadow-2xl">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-surface-700">
          <h2 className="text-lg font-semibold text-white">
            {scheduleComplete ? 'Scheduling Complete' : completed ? 'Upload Complete' : uploading ? (scheduleMode ? 'Scheduling...' : 'Uploading...') : 'Batch Upload'}
          </h2>
          {!uploading && (
            <button onClick={onClose} className="p-1.5 rounded-lg text-slate-400 hover:text-white hover:bg-surface-700 transition-colors cursor-pointer">
              <X className="w-5 h-5" />
            </button>
          )}
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto px-5 py-4 space-y-4">
          {/* Warnings */}
          {missingTitleClips.length > 0 && !uploading && !completed && (
            <div className="bg-amber-500/10 border border-amber-500/30 rounded-lg px-3 py-2 text-amber-300 text-sm">
              <strong>{missingTitleClips.length}</strong> clip{missingTitleClips.length !== 1 ? 's have' : ' has'} no title and will upload as "Untitled Clip".
            </div>
          )}

          {/* Info about un-exported clips */}
          {clips.some(c => c.render_status !== 'completed' || !c.output_path) && !uploading && !completed && (
            <div className="bg-blue-500/10 border border-blue-500/30 rounded-lg px-3 py-2 text-blue-300 text-sm">
              {clips.filter(c => c.render_status !== 'completed' || !c.output_path).length} clip{clips.filter(c => c.render_status !== 'completed' || !c.output_path).length !== 1 ? 's' : ''} will be exported automatically before uploading.
            </div>
          )}

          {/* Platform selection (disabled during upload) */}
          {!uploading && !completed && (
            <>
              <div>
                <label className="text-sm font-medium text-slate-300 mb-2 block">Upload to</label>
                <div className="flex gap-2">
                  {availablePlatforms.map(([key, info]) => (
                    <button
                      key={key}
                      onClick={() => togglePlatform(key)}
                      className={`px-3 py-1.5 rounded-lg text-sm font-medium border transition-colors cursor-pointer ${
                        selectedPlatforms[key]
                          ? 'border-violet-500 bg-violet-500/20 text-violet-300'
                          : 'border-surface-600 bg-surface-700 text-slate-400 hover:text-slate-300'
                      }`}
                    >
                      {info.name}
                    </button>
                  ))}
                </div>
              </div>

              {/* Visibility per platform */}
              {activePlatforms.map(platform => (
                <div key={platform} className="flex items-center gap-3">
                  <span className="text-sm text-slate-400 w-20">{PLATFORM_INFO[platform]?.name}</span>
                  <select
                    value={visibility[platform]}
                    onChange={(e) => setVisibility(prev => ({ ...prev, [platform]: e.target.value }))}
                    className="flex-1 bg-surface-700 border border-surface-600 rounded-lg px-3 py-1.5 text-sm text-white"
                  >
                    {platform === 'youtube' && (
                      <>
                        <option value="unlisted">Unlisted</option>
                        <option value="public">Public</option>
                        <option value="private">Private</option>
                      </>
                    )}
                    {platform === 'tiktok' && (
                      <>
                        <option value="private_self_only">Private (self only)</option>
                        <option value="mutual_follow_friends">Friends</option>
                        <option value="public_to_everyone">Public</option>
                      </>
                    )}
                  </select>
                </div>
              ))}

              {/* Use saved captions toggle */}
              <label className="flex items-center gap-3 cursor-pointer">
                <input
                  type="checkbox"
                  checked={useSavedCaptions}
                  onChange={(e) => setUseSavedCaptions(e.target.checked)}
                  className="w-4 h-4 rounded border-surface-600 bg-surface-700 text-violet-500 focus:ring-violet-500"
                />
                <span className="text-sm text-slate-300">Use each clip's saved caption & hashtags</span>
              </label>

              {/* Schedule toggle */}
              <label className="flex items-center gap-3 cursor-pointer">
                <input
                  type="checkbox"
                  checked={scheduleMode}
                  onChange={(e) => setScheduleMode(e.target.checked)}
                  className="w-4 h-4 rounded border-surface-600 bg-surface-700 text-violet-500 focus:ring-violet-500"
                />
                <Clock className="w-4 h-4 text-slate-400" />
                <span className="text-sm text-slate-300">Schedule for later</span>
              </label>

              {/* Date/time picker */}
              {scheduleMode && (
                <div className="space-y-1.5">
                  <input
                    type="datetime-local"
                    value={scheduleTime}
                    onChange={(e) => setScheduleTime(e.target.value)}
                    min={new Date(Date.now() + 60000).toISOString().slice(0, 16)}
                    className="w-full bg-surface-700 border border-surface-600 rounded-lg px-3 py-2 text-sm text-white focus:border-violet-500 focus:outline-none"
                  />
                  <p className="text-xs text-slate-500">
                    Note: App must be running for scheduled uploads to process.
                  </p>
                </div>
              )}
            </>
          )}

          {/* Progress / Results */}
          {(uploading || completed) && (
            <div>
              {/* Progress bar */}
              <div className="mb-3">
                <div className="flex justify-between text-xs text-slate-400 mb-1">
                  <span>{doneJobs} of {totalJobs} uploads</span>
                  {failedJobs > 0 && <span className="text-red-400">{failedJobs} failed</span>}
                </div>
                <div className="h-2 bg-surface-700 rounded-full overflow-hidden">
                  <div
                    className={`h-full rounded-full transition-all duration-300 ${failedJobs > 0 ? 'bg-amber-500' : 'bg-violet-500'}`}
                    style={{ width: `${totalJobs > 0 ? (doneJobs / totalJobs) * 100 : 0}%` }}
                  />
                </div>
              </div>

              {/* Clip list with statuses */}
              <button
                onClick={() => setExpanded(!expanded)}
                className="flex items-center gap-2 text-sm text-slate-400 hover:text-slate-300 mb-2 cursor-pointer"
              >
                {expanded ? <ChevronUp className="w-4 h-4" /> : <ChevronDown className="w-4 h-4" />}
                {expanded ? 'Hide details' : 'Show details'}
              </button>

              {expanded && (
                <div className="space-y-1.5 max-h-60 overflow-y-auto">
                  {clips.map(clip => (
                    <div key={clip.id} className="bg-surface-700/50 rounded-lg px-3 py-2">
                      <div className="text-sm text-white truncate mb-1">{clip.title?.trim() || 'Untitled Clip'}</div>
                      <div className="flex flex-wrap gap-2">
                        {activePlatforms.map(platform => {
                          const st = clipStatuses[platform]?.[clip.id]
                          const status = st?.status || 'pending'
                          return (
                            <span key={platform} className="flex items-center gap-1 text-xs">
                              <span className="text-slate-500">{PLATFORM_INFO[platform]?.name}:</span>
                              {status === 'pending' && <span className="text-slate-500">Waiting</span>}
                              {status === 'exporting' && (
                                <>
                                  <Loader2 className="w-3 h-3 animate-spin text-blue-400" />
                                  <span className="text-blue-400">Exporting {st?.exportProgress != null ? `${Math.round(st.exportProgress)}%` : ''}</span>
                                </>
                              )}
                              {status === 'uploading' && (
                                <>
                                  <Loader2 className="w-3 h-3 animate-spin text-violet-400" />
                                  <span className="text-violet-400">Uploading</span>
                                </>
                              )}
                              {status === 'done' && (
                                <>
                                  <CheckCircle2 className="w-3 h-3 text-green-400" />
                                  {st?.videoUrl ? (
                                    <a href={st.videoUrl} target="_blank" rel="noopener noreferrer" className="text-green-400 hover:underline flex items-center gap-0.5">
                                      Done <ExternalLink className="w-3 h-3" />
                                    </a>
                                  ) : st?.duplicateUrl ? (
                                    <a href={st.duplicateUrl} target="_blank" rel="noopener noreferrer" className="text-amber-400 hover:underline flex items-center gap-0.5">
                                      Duplicate <ExternalLink className="w-3 h-3" />
                                    </a>
                                  ) : (
                                    <span className="text-green-400">Done</span>
                                  )}
                                </>
                              )}
                              {status === 'error' && (
                                <span className="text-red-400 flex items-center gap-0.5" title={st?.error}>
                                  <AlertCircle className="w-3 h-3" /> Failed
                                </span>
                              )}
                              {status === 'skipped' && <span className="text-slate-500">Skipped</span>}
                            </span>
                          )
                        })}
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}

          {/* Clip summary (before upload) */}
          {!uploading && !completed && (
            <div className="text-sm text-slate-400">
              {clips.length} clip{clips.length !== 1 ? 's' : ''} ready
              {activePlatforms.length > 0 && ` to upload to ${activePlatforms.map(p => PLATFORM_INFO[p]?.name).join(' & ')}`}.
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="px-5 py-4 border-t border-surface-700 flex justify-end gap-3">
          {!uploading && !completed && (
            <>
              <button
                onClick={onClose}
                className="px-4 py-2 rounded-lg text-sm text-slate-400 hover:text-white hover:bg-surface-700 transition-colors cursor-pointer"
              >
                Cancel
              </button>
              {scheduleMode ? (
                <button
                  onClick={startSchedule}
                  disabled={activePlatforms.length === 0 || clips.length === 0 || !scheduleTime}
                  className="px-4 py-2 rounded-lg text-sm font-medium bg-violet-600 hover:bg-violet-500 text-white disabled:opacity-40 disabled:cursor-not-allowed transition-colors cursor-pointer flex items-center gap-2"
                >
                  <Clock className="w-4 h-4" />
                  Schedule {clips.length} Clip{clips.length !== 1 ? 's' : ''}
                </button>
              ) : (
                <button
                  onClick={() => startUpload(false)}
                  disabled={activePlatforms.length === 0 || clips.length === 0}
                  className="px-4 py-2 rounded-lg text-sm font-medium bg-violet-600 hover:bg-violet-500 text-white disabled:opacity-40 disabled:cursor-not-allowed transition-colors cursor-pointer flex items-center gap-2"
                >
                  <Upload className="w-4 h-4" />
                  Upload {clips.length} Clip{clips.length !== 1 ? 's' : ''}
                </button>
              )}
            </>
          )}
          {uploading && (
            <button
              onClick={handleCancel}
              className="px-4 py-2 rounded-lg text-sm text-red-400 hover:text-red-300 hover:bg-red-500/10 transition-colors cursor-pointer"
            >
              Cancel Remaining
            </button>
          )}
          {completed && (
            <>
              {failedJobs > 0 && (
                <button
                  onClick={retryFailed}
                  className="px-4 py-2 rounded-lg text-sm font-medium border border-amber-500/30 text-amber-300 hover:bg-amber-500/10 transition-colors cursor-pointer flex items-center gap-2"
                >
                  <RotateCcw className="w-4 h-4" />
                  Retry {failedJobs} Failed
                </button>
              )}
              <button
                onClick={() => { onComplete(); onClose() }}
                className="px-4 py-2 rounded-lg text-sm font-medium bg-violet-600 hover:bg-violet-500 text-white transition-colors cursor-pointer"
              >
                Done
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  )
}
