import type { DimensionScores } from '../../types/analysis'
import { formatDimensionLabel } from '../../lib/uiFormat'

interface Props {
  dimensions: DimensionScores
}

const BARS: { key: keyof DimensionScores; label: string; color: string }[] = [
  { key: 'hook_strength',       label: 'Hook',    color: 'bg-violet-500' },
  { key: 'emotional_intensity', label: 'Emotion', color: 'bg-rose-500' },
  { key: 'context_clarity',     label: 'Clarity', color: 'bg-sky-500' },
  { key: 'visual_activity',     label: 'Visual',  color: 'bg-amber-500' },
  { key: 'speech_punch',        label: 'Speech',  color: 'bg-emerald-500' },
]

export default function ScoreBreakdownChart({ dimensions }: Props) {
  return (
    <div className="space-y-2">
      <h4 className="text-xs font-semibold text-surface-400 uppercase tracking-wider mb-1">
        Score breakdown
      </h4>
      {BARS.map(({ key, label, color }) => {
        const value = dimensions[key]
        const pct = Math.round(value * 100)

        return (
          <div key={key} className="flex items-center gap-3">
            <span className="w-14 text-xs text-surface-400 text-right shrink-0">
              {label}
            </span>
            <div className="flex-1 h-2 bg-surface-700 rounded-full overflow-hidden">
              <div
                className={`h-full rounded-full ${color}`}
                style={{ width: `${pct}%` }}
              />
            </div>
            <span className="w-16 text-xs text-surface-400 shrink-0">
              {formatDimensionLabel(value)}
            </span>
          </div>
        )
      })}
    </div>
  )
}
