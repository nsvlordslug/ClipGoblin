import type { CandidateClip } from '../../types/analysis'
import { formatConfidence, formatDuration, formatTag, formatQuickVerdict, formatScoreRationale } from '../../lib/uiFormat'
import ScoreBreakdownChart from './ScoreBreakdownChart'

interface Props {
  clip: CandidateClip
}

export default function ClipDetailPanel({ clip }: Props) {
  const conf = formatConfidence(clip.confidence_score)
  const dur = formatDuration(clip.end_time - clip.start_time)
  const verdict = formatQuickVerdict(clip.confidence_score, clip.signal_sources.length)
  const report = clip.score_report
  const tags = clip.tags.slice(0, 4)

  return (
    <div className="space-y-5">
      {/* Video preview placeholder */}
      <div className="aspect-video bg-surface-800 rounded-xl flex items-center justify-center border border-surface-700">
        <span className="text-surface-500 text-sm">Video preview</span>
      </div>

      {/* Title row */}
      <div className="flex items-start justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold text-white">
            {clip.title || 'Untitled clip'}
          </h2>
          {clip.hook && (
            <p className="text-sm text-surface-400 italic mt-0.5">
              {clip.hook}
            </p>
          )}
          <p className="text-xs text-surface-500 mt-1">{dur}</p>
        </div>
        <span className={`text-sm font-medium shrink-0 ${conf.color}`}>
          {conf.text}
        </span>
      </div>

      {/* Quick verdict + explanation + score rationale */}
      <div className="bg-surface-800 border border-surface-700 rounded-lg px-4 py-3 space-y-1.5">
        <p className="text-sm font-medium text-white">{verdict}</p>
        {report && (
          <>
            <p className="text-xs text-surface-400">{report.explanation}</p>
            <p className="text-xs text-surface-500">
              {formatScoreRationale(clip.confidence_score, report)}
            </p>
          </>
        )}
      </div>

      {/* Score breakdown chart */}
      {report && (
        <ScoreBreakdownChart dimensions={report.dimensions} />
      )}

      {/* Tags */}
      {tags.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {tags.map(tag => (
            <span key={tag} className="text-xs bg-surface-800 border border-surface-700 text-surface-300 px-2 py-1 rounded-full">
              {formatTag(tag)}
            </span>
          ))}
        </div>
      )}

      {/* Transcript excerpt */}
      {clip.transcript_excerpt && (
        <blockquote className="bg-surface-800/50 border-l-2 border-violet-500/50 px-3 py-2 text-sm text-surface-300 italic rounded-r-lg">
          &ldquo;{clip.transcript_excerpt}&rdquo;
        </blockquote>
      )}

      {/* Signal sources */}
      <div className="text-xs text-surface-500">
        Detected by: {clip.signal_sources.join(', ')}
      </div>
    </div>
  )
}
