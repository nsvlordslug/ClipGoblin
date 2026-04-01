import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { Clock, Trash2, CalendarClock, ExternalLink, AlertCircle, CheckCircle2, Loader2, Info } from 'lucide-react'
import { useScheduleStore } from '../stores/scheduleStore'
import { useAppStore } from '../stores/appStore'
import { PLATFORM_INFO } from '../stores/platformStore'
import type { ScheduledUpload } from '../types'

function formatDateTime(iso: string): string {
  try {
    const d = new Date(iso)
    return d.toLocaleString(undefined, {
      month: 'short', day: 'numeric', year: 'numeric',
      hour: 'numeric', minute: '2-digit',
    })
  } catch {
    return iso
  }
}

function timeUntil(iso: string): string {
  const now = Date.now()
  const target = new Date(iso).getTime()
  const diff = target - now
  if (diff <= 0) return 'Overdue'
  const mins = Math.floor(diff / 60000)
  if (mins < 60) return `in ${mins}m`
  const hrs = Math.floor(mins / 60)
  if (hrs < 24) return `in ${hrs}h ${mins % 60}m`
  const days = Math.floor(hrs / 24)
  return `in ${days}d ${hrs % 24}h`
}

function StatusBadge({ upload }: { upload: ScheduledUpload }) {
  switch (upload.status) {
    case 'pending':
      return (
        <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-violet-500/20 text-violet-300 border border-violet-500/30">
          <Clock className="w-3 h-3" />
          {timeUntil(upload.scheduled_time)}
        </span>
      )
    case 'uploading':
      return (
        <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-blue-500/20 text-blue-300 border border-blue-500/30">
          <Loader2 className="w-3 h-3 animate-spin" />
          Uploading
        </span>
      )
    case 'completed':
      return (
        <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-green-500/20 text-green-300 border border-green-500/30">
          <CheckCircle2 className="w-3 h-3" />
          Completed
        </span>
      )
    case 'failed':
      return (
        <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-red-500/20 text-red-300 border border-red-500/30">
          <AlertCircle className="w-3 h-3" />
          Failed
        </span>
      )
  }
}

export default function ScheduledUploads() {
  const navigate = useNavigate()
  const { uploads, loading, load, cancel, reschedule } = useScheduleStore()
  const { clips } = useAppStore()
  const [rescheduleId, setRescheduleId] = useState<string | null>(null)
  const [rescheduleTime, setRescheduleTime] = useState('')

  useEffect(() => { load() }, [load])

  const clipMap = Object.fromEntries(clips.map(c => [c.id, c]))

  const pending = uploads.filter(u => u.status === 'pending' || u.status === 'uploading')
  const completed = uploads.filter(u => u.status === 'completed')
  const failed = uploads.filter(u => u.status === 'failed')

  const handleCancel = async (id: string) => {
    await cancel(id)
  }

  const handleReschedule = async (id: string) => {
    if (!rescheduleTime) return
    const iso = new Date(rescheduleTime).toISOString()
    await reschedule(id, iso)
    setRescheduleId(null)
    setRescheduleTime('')
  }

  const renderUploadRow = (upload: ScheduledUpload) => {
    const clip = clipMap[upload.clip_id]
    const clipTitle = clip?.title || 'Unknown Clip'
    const platformInfo = PLATFORM_INFO[upload.platform]

    return (
      <div key={upload.id} className="relative flex items-center gap-4 px-4 py-3 bg-surface-800 rounded-lg border border-surface-700">
        {/* Clip info */}
        <div className="flex-1 min-w-0">
          <button
            onClick={() => navigate(`/editor/${upload.clip_id}`)}
            className="text-sm font-medium text-white hover:text-violet-400 truncate block text-left cursor-pointer"
          >
            {clipTitle}
          </button>
          <div className="flex items-center gap-2 mt-0.5">
            <span className="text-xs text-slate-500">
              {platformInfo?.name || upload.platform}
            </span>
            <span className="text-xs text-slate-600">&bull;</span>
            <span className="text-xs text-slate-500">
              {formatDateTime(upload.scheduled_time)}
            </span>
          </div>
          {upload.error_message && (
            <p className="text-xs text-red-400 mt-1 truncate" title={upload.error_message}>
              {upload.error_message}
            </p>
          )}
        </div>

        {/* Status */}
        <StatusBadge upload={upload} />

        {/* Actions */}
        <div className="flex items-center gap-1.5 shrink-0">
          {upload.status === 'completed' && upload.video_url && (
            <button
              onClick={() => window.open(upload.video_url!, '_blank')}
              className="p-1.5 rounded-lg text-emerald-400 hover:bg-emerald-500/10 transition-colors cursor-pointer"
              title="View on platform"
            >
              <ExternalLink className="w-4 h-4" />
            </button>
          )}
          {(upload.status === 'pending' || upload.status === 'failed') && (
            <>
              <button
                onClick={() => {
                  setRescheduleId(rescheduleId === upload.id ? null : upload.id)
                  setRescheduleTime('')
                }}
                className="p-1.5 rounded-lg text-slate-400 hover:text-violet-400 hover:bg-violet-500/10 transition-colors cursor-pointer"
                title="Reschedule"
              >
                <CalendarClock className="w-4 h-4" />
              </button>
              <button
                onClick={() => handleCancel(upload.id)}
                className="p-1.5 rounded-lg text-slate-400 hover:text-red-400 hover:bg-red-500/10 transition-colors cursor-pointer"
                title="Cancel"
              >
                <Trash2 className="w-4 h-4" />
              </button>
            </>
          )}
        </div>

        {/* Reschedule inline form */}
        {rescheduleId === upload.id && (
          <div className="absolute right-0 top-full mt-1 z-10 bg-surface-700 border border-surface-600 rounded-lg p-3 shadow-xl flex items-center gap-2">
            <input
              type="datetime-local"
              value={rescheduleTime}
              onChange={(e) => setRescheduleTime(e.target.value)}
              className="bg-surface-800 border border-surface-600 rounded px-2 py-1 text-sm text-white"
              min={new Date().toISOString().slice(0, 16)}
            />
            <button
              onClick={() => handleReschedule(upload.id)}
              disabled={!rescheduleTime}
              className="px-3 py-1 bg-violet-600 hover:bg-violet-500 text-white text-sm rounded disabled:opacity-40 cursor-pointer"
            >
              Save
            </button>
          </div>
        )}
      </div>
    )
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold text-white">Scheduled Uploads</h1>
      </div>

      {/* Info banner */}
      <div className="flex items-start gap-3 px-4 py-3 bg-surface-800 border border-surface-700 rounded-lg">
        <Info className="w-4 h-4 text-slate-400 mt-0.5 shrink-0" />
        <p className="text-sm text-slate-400">
          App must be running for scheduled uploads to process. The scheduler checks for due uploads every 60 seconds.
        </p>
      </div>

      {loading ? (
        <div className="flex items-center justify-center py-12">
          <Loader2 className="w-6 h-6 animate-spin text-violet-400" />
        </div>
      ) : uploads.length === 0 ? (
        <div className="glass-card p-8 text-center">
          <Clock className="w-10 h-10 text-slate-600 mx-auto mb-3" />
          <p className="text-slate-400">No scheduled uploads yet.</p>
          <p className="text-sm text-slate-500 mt-1">Schedule uploads from the Editor or Clips page.</p>
        </div>
      ) : (
        <>
          {/* Pending / Uploading */}
          {pending.length > 0 && (
            <section>
              <h2 className="text-sm font-semibold text-slate-300 mb-3 uppercase tracking-wider">
                Upcoming ({pending.length})
              </h2>
              <div className="space-y-2">
                {pending.map(renderUploadRow)}
              </div>
            </section>
          )}

          {/* Failed */}
          {failed.length > 0 && (
            <section>
              <h2 className="text-sm font-semibold text-red-400 mb-3 uppercase tracking-wider">
                Failed ({failed.length})
              </h2>
              <div className="space-y-2">
                {failed.map(renderUploadRow)}
              </div>
            </section>
          )}

          {/* Completed */}
          {completed.length > 0 && (
            <section>
              <h2 className="text-sm font-semibold text-slate-400 mb-3 uppercase tracking-wider">
                Completed ({completed.length})
              </h2>
              <div className="space-y-2">
                {completed.map(renderUploadRow)}
              </div>
            </section>
          )}
        </>
      )}
    </div>
  )
}
