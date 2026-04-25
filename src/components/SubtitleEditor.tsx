import { useRef, useEffect, memo } from 'react'
import { Trash2 } from 'lucide-react'
import type { SubtitleSegment } from '../lib/subtitleUtils'

interface Props {
  segments: SubtitleSegment[]
  /** Active segment ID (matches current playback time) */
  activeId: string | null
  /** SRT-relative time for highlight tracking */
  currentTime: number
  /** Trim bounds in SRT-relative time */
  trimStart: number
  trimEnd: number
  /** Callbacks */
  onEdit: (id: string, text: string) => void
  onDelete: (id: string) => void
  onSeek: (srtTime: number) => void
}

function fmtShort(s: number) {
  const m = Math.floor(s / 60)
  const sec = Math.floor(s % 60)
  const ms = Math.floor((s % 1) * 10)
  return `${m}:${String(sec).padStart(2, '0')}.${ms}`
}

export default memo(function SubtitleEditor({
  segments, activeId, currentTime, trimStart, trimEnd,
  onEdit, onDelete, onSeek,
}: Props) {
  const listRef = useRef<HTMLDivElement>(null)
  const activeRef = useRef<HTMLDivElement>(null)

  // Auto-scroll to active segment — scrolls ONLY the list container.
  // scrollIntoView would bubble up through every scrollable ancestor and drag
  // the whole editor panel along for the ride, so we compute the delta ourselves.
  useEffect(() => {
    if (activeRef.current && listRef.current) {
      const list = listRef.current
      const active = activeRef.current
      const listRect = list.getBoundingClientRect()
      const activeRect = active.getBoundingClientRect()

      if (activeRect.top < listRect.top || activeRect.bottom > listRect.bottom) {
        const activeRelativeTop = (activeRect.top - listRect.top) + list.scrollTop
        const targetTop = activeRelativeTop - (list.clientHeight - active.clientHeight) / 2
        list.scrollTo({ top: Math.max(0, targetTop), behavior: 'smooth' })
      }
    }
  }, [activeId])

  // Filter to segments within trim bounds
  const visible = segments.filter(s => s.endTime > trimStart && s.startTime < trimEnd)

  if (visible.length === 0) {
    return (
      <div className="text-center py-6">
        <p className="text-xs text-slate-500">No subtitle segments in the trimmed range.</p>
        <p className="text-[10px] text-slate-600 mt-1">Auto-captions are generated during VOD analysis when speech-to-text is available.</p>
      </div>
    )
  }

  return (
    <div ref={listRef} className="space-y-1 max-h-52 overflow-y-auto pr-1">
      {visible.map(seg => {
        const isActive = seg.id === activeId
        const isOutOfBounds = seg.startTime < trimStart || seg.endTime > trimEnd
        const progress = isActive && seg.endTime > seg.startTime
          ? Math.max(0, Math.min(1, (currentTime - seg.startTime) / (seg.endTime - seg.startTime)))
          : 0

        return (
          <div
            key={seg.id}
            ref={isActive ? activeRef : undefined}
            className={`relative rounded-lg border transition-colors ${
              isActive
                ? 'bg-cyan-500/10 border-cyan-500/40'
                : 'bg-surface-900 border-surface-600 hover:border-surface-500'
            } ${isOutOfBounds ? 'opacity-50' : ''}`}
          >
            {/* Progress bar behind the segment (visible during playback) */}
            {isActive && progress > 0 && (
              <div className="absolute inset-y-0 left-0 bg-cyan-500/10 rounded-lg transition-[width] duration-100"
                style={{ width: `${progress * 100}%` }} />
            )}

            <div className="relative flex items-start gap-2 p-2">
              {/* Time label — clickable to seek */}
              <button
                onClick={() => onSeek(seg.startTime)}
                className={`shrink-0 text-[9px] font-mono tabular-nums mt-0.5 px-1 py-0.5 rounded cursor-pointer transition-colors ${
                  isActive ? 'text-cyan-400 bg-cyan-500/20' : 'text-slate-500 hover:text-slate-300 hover:bg-surface-700'
                }`}
                title={`Seek to ${fmtShort(seg.startTime)}`}
              >
                {fmtShort(seg.startTime)}
              </button>

              {/* Editable text */}
              <input
                type="text"
                value={seg.text}
                onChange={e => onEdit(seg.id, e.target.value)}
                className={`flex-1 bg-transparent border-none text-xs focus:outline-none min-w-0 ${
                  isActive ? 'text-white' : 'text-slate-300'
                }`}
                spellCheck={false}
              />

              {/* Delete button */}
              <button
                onClick={() => onDelete(seg.id)}
                className="shrink-0 p-0.5 text-slate-600 hover:text-red-400 transition-colors cursor-pointer"
                title="Delete segment"
              >
                <Trash2 className="w-3 h-3" />
              </button>
            </div>
          </div>
        )
      })}
    </div>
  )
})
