import type { CandidateClip } from '../../types/analysis'
import { formatConfidence, formatDuration, formatTag } from '../../lib/uiFormat'

interface Props {
  clip: CandidateClip
  selected: boolean
  onClick: () => void
}

export default function ClipCard({ clip, selected, onClick }: Props) {
  const conf = formatConfidence(clip.confidence_score)
  const dur = formatDuration(clip.end_time - clip.start_time)
  const tags = clip.tags.slice(0, 3)

  return (
    <button
      onClick={onClick}
      className={`w-full text-left flex gap-3 p-2.5 rounded-xl border transition-colors ${
        selected
          ? 'bg-surface-800 border-violet-500'
          : 'bg-surface-800/50 border-surface-700 hover:border-surface-600'
      }`}
    >
      {/* Thumbnail placeholder */}
      <div className="w-24 shrink-0 aspect-video rounded-lg bg-surface-700 flex items-center justify-center overflow-hidden relative">
        {clip.preview_thumbnail_path ? (
          <img src={clip.preview_thumbnail_path} alt="" className="w-full h-full object-cover" />
        ) : (
          <span className="text-[10px] text-surface-500">No thumb</span>
        )}
        <span className="absolute bottom-0.5 right-0.5 bg-black/70 text-[9px] text-white font-mono px-1 rounded">
          {dur}
        </span>
      </div>

      {/* Text content */}
      <div className="flex-1 min-w-0 py-0.5">
        <p className="text-sm font-medium text-white truncate">
          {clip.title || 'Untitled'}
        </p>
        {clip.hook && (
          <p className="text-xs text-surface-400 italic truncate mt-0.5">
            {clip.hook}
          </p>
        )}
        <div className="flex items-center gap-2 mt-1.5">
          <span className={`text-xs ${conf.color}`}>{conf.text}</span>
          <span className="text-surface-600">·</span>
          <div className="flex gap-1">
            {tags.map(tag => (
              <span key={tag} className="text-[10px] bg-surface-700 text-surface-300 px-1.5 py-0.5 rounded">
                {formatTag(tag)}
              </span>
            ))}
          </div>
        </div>
      </div>
    </button>
  )
}
