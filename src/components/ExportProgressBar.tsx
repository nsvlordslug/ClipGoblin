import { Loader2, Check, AlertCircle } from 'lucide-react'

// ── Export stage labels mapped to progress ranges ──

function getStageLabel(progress: number): string {
  if (progress <= 5)  return 'Preparing video...'
  if (progress <= 15) return 'Analyzing clip...'
  if (progress <= 35) return 'Rendering subtitles...'
  if (progress <= 70) return 'Encoding video...'
  if (progress <= 90) return 'Processing audio...'
  if (progress <= 98) return 'Finalizing...'
  return 'Wrapping up...'
}

interface Props {
  /** 0-100 progress from backend */
  progress: number
  /** Export completed */
  done: boolean
  /** Error message or null */
  error: string | null
  /** Currently exporting */
  active: boolean
}

export default function ExportProgressBar({ progress, done, active, error }: Props) {
  if (!active && !done && !error) return null

  const stage = getStageLabel(progress)
  const pct = Math.min(100, Math.max(0, progress))

  return (
    <div className="space-y-1.5">
      {/* Status header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1.5">
          {active && <Loader2 className="w-3 h-3 text-violet-400 animate-spin" />}
          {done && <Check className="w-3 h-3 text-green-400" />}
          {error && <AlertCircle className="w-3 h-3 text-red-400" />}
          <span className={`text-[11px] font-medium ${
            done ? 'text-green-400' : error ? 'text-red-400' : 'text-slate-300'
          }`}>
            {done ? 'Export complete' : error ? 'Export failed' : stage}
          </span>
        </div>
        <span className={`text-[11px] font-mono tabular-nums ${
          done ? 'text-green-400' : error ? 'text-red-400' : 'text-violet-400'
        }`}>
          {done ? '100%' : error ? '—' : `${pct}%`}
        </span>
      </div>

      {/* Progress bar track */}
      <div className="relative h-2 bg-surface-900 rounded-full overflow-hidden border border-surface-600">
        {/* Fill bar */}
        <div
          className={`absolute inset-y-0 left-0 rounded-full transition-all duration-300 ease-out ${
            done ? 'bg-green-500' : error ? 'bg-red-500' : 'bg-violet-500'
          }`}
          style={{ width: `${done ? 100 : error ? pct : pct}%` }}
        />
        {/* Animated shimmer overlay while active */}
        {active && pct < 100 && (
          <div
            className="absolute inset-y-0 left-0 rounded-full overflow-hidden"
            style={{ width: `${pct}%` }}
          >
            <div className="absolute inset-0 bg-gradient-to-r from-transparent via-white/15 to-transparent animate-shimmer" />
          </div>
        )}
      </div>

      {/* Error detail */}
      {error && (
        <p className="text-[10px] text-red-400/80 truncate" title={error}>{error}</p>
      )}
    </div>
  )
}
