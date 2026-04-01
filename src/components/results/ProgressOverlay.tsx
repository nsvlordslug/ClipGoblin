interface Props {
  progress: number
  analyzing: boolean
}

function stageText(progress: number): string {
  if (progress < 20) return 'Listening for reactions...'
  if (progress < 30) return 'Reading transcript...'
  if (progress < 40) return 'Scanning for scene changes...'
  if (progress < 50) return 'Merging signals...'
  if (progress < 80) return 'Ranking highlights...'
  if (progress < 95) return 'Generating previews...'
  return 'Finishing up...'
}

export default function ProgressOverlay({ progress, analyzing }: Props) {
  if (!analyzing) return null

  return (
    <div className="fixed inset-0 bg-black/60 backdrop-blur-sm z-50 flex items-center justify-center">
      <div className="bg-surface-900 border border-surface-700 rounded-2xl p-8 w-full max-w-sm shadow-2xl">
        <p className="text-lg font-semibold text-white text-center mb-2">
          Finding your best moments
        </p>
        <p className="text-sm text-surface-400 text-center mb-5">
          {stageText(progress)}
        </p>
        <div className="h-2 bg-surface-700 rounded-full overflow-hidden">
          <div
            className="h-full bg-violet-500 rounded-full transition-all duration-500 ease-out"
            style={{ width: `${progress}%` }}
          />
        </div>
        <p className="text-xs text-surface-500 text-center mt-2">{progress}%</p>
      </div>
    </div>
  )
}
