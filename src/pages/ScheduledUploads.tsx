import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { Clock, Trash2, CalendarClock, ExternalLink, AlertCircle, CheckCircle2, Loader2 } from 'lucide-react'
import { useScheduleStore } from '../stores/scheduleStore'
import { useAppStore } from '../stores/appStore'
import { PLATFORM_INFO } from '../stores/platformStore'
import { fmtCountdown } from '../lib/uiFormat'
import type { ScheduledUpload } from '../types'

const THUMB_STYLES = ['a', 'b', 'c', 'd', 'e', 'f'] as const

function formatTimeShort(iso: string): string {
  try {
    return new Date(iso).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })
  } catch { return iso }
}

function formatDayLabel(date: Date): { name: string; sub: string } {
  const today = new Date()
  const tomorrow = new Date(); tomorrow.setDate(today.getDate() + 1)
  const sameDay = (a: Date, b: Date) => a.toDateString() === b.toDateString()
  const dateTxt = date.toLocaleDateString(undefined, { month: 'long', day: 'numeric' })
  if (sameDay(date, today)) return { name: `Today · ${dateTxt}`, sub: '' }
  if (sameDay(date, tomorrow)) return { name: `Tomorrow · ${dateTxt}`, sub: '' }
  return { name: dateTxt, sub: date.toLocaleDateString(undefined, { weekday: 'long' }) }
}

function timeUntil(iso: string): string {
  const diff = new Date(iso).getTime() - Date.now()
  if (diff <= 0) return 'Overdue'
  const mins = Math.floor(diff / 60000)
  if (mins < 60) return `in ${mins}m`
  const hrs = Math.floor(mins / 60)
  if (hrs < 24) return `in ${hrs}h ${mins % 60}m`
  const days = Math.floor(hrs / 24)
  return `in ${days}d ${hrs % 24}h`
}

function groupByDay(uploads: ScheduledUpload[]): Array<{ date: Date; items: ScheduledUpload[] }> {
  const map = new Map<string, { date: Date; items: ScheduledUpload[] }>()
  for (const u of uploads) {
    const d = new Date(u.scheduled_time)
    const key = d.toDateString()
    const entry = map.get(key) ?? { date: d, items: [] }
    entry.items.push(u)
    map.set(key, entry)
  }
  return Array.from(map.values()).sort((a, b) => a.date.getTime() - b.date.getTime())
}

function isToday(d: Date): boolean {
  return d.toDateString() === new Date().toDateString()
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
  const [, setTick] = useState(0)

  useEffect(() => { load() }, [load])
  // Live countdown tick
  useEffect(() => {
    const id = setInterval(() => setTick(t => t + 1), 1000)
    return () => clearInterval(id)
  }, [])

  const clipMap = Object.fromEntries(clips.map(c => [c.id, c]))

  const pending = uploads.filter(u => u.status === 'pending' || u.status === 'uploading')
  const completed = uploads.filter(u => u.status === 'completed')
  const failed = uploads.filter(u => u.status === 'failed')

  const nextPending = [...pending].sort((a, b) => new Date(a.scheduled_time).getTime() - new Date(b.scheduled_time).getTime())[0]

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

  const renderSlot = (upload: ScheduledUpload, idx: number) => {
    const clip = clipMap[upload.clip_id]
    const clipTitle = clip?.title || 'Unknown Clip'
    const platformInfo = PLATFORM_INFO[upload.platform]
    const thumbStyle = THUMB_STYLES[idx % THUMB_STYLES.length]

    return (
      <div key={upload.id} className="v4-sched-slot relative">
        <div className="v4-slot-time">{formatTimeShort(upload.scheduled_time)}</div>
        <div className={`v4-slot-thumb v4-clip-thumb ${thumbStyle}`}>
          {clip && (
            <span className="v4-clip-dur" style={{fontSize:9}}>
              {Math.floor((clip.end_seconds - clip.start_seconds) / 60)}:
              {String(Math.floor((clip.end_seconds - clip.start_seconds) % 60)).padStart(2,'0')}
            </span>
          )}
        </div>
        <div className="min-w-0">
          <button
            onClick={() => navigate(`/editor/${upload.clip_id}`)}
            className="v4-slot-title block text-left w-full truncate hover:text-violet-300 cursor-pointer"
          >
            {clipTitle}
          </button>
          <div className="v4-slot-meta">
            {platformInfo?.name || upload.platform}
            {upload.status === 'pending' && <> · {timeUntil(upload.scheduled_time)}</>}
          </div>
          {upload.error_message && (
            <div className="text-[10px] text-red-400 mt-1 truncate" title={upload.error_message}>
              ⚠ {upload.error_message}
            </div>
          )}
        </div>
        <StatusBadge upload={upload} />
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
        {rescheduleId === upload.id && (
          <div className="absolute right-0 top-full mt-1 z-10 v4-panel flex items-center gap-2" style={{padding:10}}>
            <input
              type="datetime-local"
              value={rescheduleTime}
              onChange={(e) => setRescheduleTime(e.target.value)}
              className="v4-input"
              style={{width:'auto'}}
              min={new Date().toISOString().slice(0, 16)}
            />
            <button
              onClick={() => handleReschedule(upload.id)}
              disabled={!rescheduleTime}
              className="v4-btn primary"
              style={{padding:'6px 12px',fontSize:12}}
            >
              Save
            </button>
          </div>
        )}
      </div>
    )
  }

  const renderDayGroup = (group: { date: Date; items: ScheduledUpload[] }, section: 'pending' | 'failed' | 'completed') => {
    const { name, sub } = formatDayLabel(group.date)
    const today = isToday(group.date)
    return (
      <div key={group.date.toISOString() + section} className={`v4-timeline-day ${today && section === 'pending' ? 'today' : ''}`}>
        <div className="v4-day-header">
          <div>
            <div className="v4-day-name">{name}</div>
            <div className="v4-day-sub">
              {group.items.length} upload{group.items.length !== 1 ? 's' : ''}
              {sub ? ` · ${sub}` : ''}
            </div>
          </div>
          {today && section === 'pending' && nextPending && (
            <div className="v4-day-countdown">{fmtCountdown(nextPending.scheduled_time)}</div>
          )}
        </div>
        {group.items.map((u, i) => renderSlot(u, i))}
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <div className="v4-page-header">
        <div>
          <div className="v4-page-title">Upload Schedule 🕒</div>
          <div className="v4-page-sub">
            {pending.length} queued · {completed.length} completed · {failed.length} failed
          </div>
        </div>
        <div className="v4-page-actions">
          <button
            className="v4-btn"
            onClick={() => navigate('/clips?action=schedule')}
            title="Pick multiple clips and schedule them together"
          >
            📅 Bulk schedule
          </button>
          <button
            className="v4-btn primary"
            onClick={() => navigate('/clips?action=schedule')}
          >
            + New schedule
          </button>
        </div>
      </div>

      <div className="v4-tip">
        💡 App must be running for scheduled uploads to process. The scheduler checks for due uploads every 60 seconds.
      </div>

      {loading ? (
        <div className="v4-panel text-center p-12">
          <Loader2 className="w-6 h-6 animate-spin text-violet-400 mx-auto" />
        </div>
      ) : uploads.length === 0 ? (
        <div className="v4-panel text-center p-12">
          <Clock className="w-10 h-10 text-slate-600 mx-auto mb-3" />
          <p className="text-slate-400">No scheduled uploads yet.</p>
          <p className="text-sm text-slate-500 mt-1">Schedule uploads from the Editor or Clips page.</p>
        </div>
      ) : (
        <>
          {pending.length > 0 && (
            <section>
              <h2 className="text-xs font-bold text-slate-400 mb-2 uppercase tracking-[0.15em]">
                Upcoming ({pending.length})
              </h2>
              {groupByDay(pending).map(g => renderDayGroup(g, 'pending'))}
            </section>
          )}
          {failed.length > 0 && (
            <section>
              <h2 className="text-xs font-bold text-red-400 mb-2 uppercase tracking-[0.15em]">
                Failed ({failed.length})
              </h2>
              {groupByDay(failed).map(g => renderDayGroup(g, 'failed'))}
            </section>
          )}
          {completed.length > 0 && (
            <section>
              <h2 className="text-xs font-bold text-slate-400 mb-2 uppercase tracking-[0.15em]">
                Completed ({completed.length})
              </h2>
              {groupByDay(completed).map(g => renderDayGroup(g, 'completed'))}
            </section>
          )}
        </>
      )}
    </div>
  )
}
