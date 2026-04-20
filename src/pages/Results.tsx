import { useEffect } from 'react'
import { useResultsStore, getSelectedClip } from '../stores/resultsStore'
import { MOCK_RESULT } from '../mocks/mockAnalysis'
import ClipCard from '../components/results/ClipCard'
import ClipDetailPanel from '../components/results/ClipDetailPanel'
import ProgressOverlay from '../components/results/ProgressOverlay'

export default function Results() {
  const result = useResultsStore(s => s.result)
  const selectedId = useResultsStore(s => s.selectedId)
  const analyzing = useResultsStore(s => s.analyzing)
  const progress = useResultsStore(s => s.progress)
  const setResult = useResultsStore(s => s.setResult)
  const selectClip = useResultsStore(s => s.selectClip)

  const selectedClip = useResultsStore(getSelectedClip)
  const clips = result?.clips ?? []

  // Load mock data on mount
  useEffect(() => {
    if (!result) {
      setResult(MOCK_RESULT)
    }
  }, [result, setResult])

  return (
    <>
      <ProgressOverlay progress={progress} analyzing={analyzing} />

      {/* Header */}
      <div className="v4-page-header">
        <div>
          <div className="v4-page-title">Results ✨</div>
          <div className="v4-page-sub">
            {clips.length} clip{clips.length !== 1 ? 's' : ''} found
            {result && result.signals_used.length > 0 && (
              <> &middot; {result.signals_used.join(' + ')}</>
            )}
          </div>
        </div>
      </div>

      {clips.length === 0 ? (
        <div className="flex items-center justify-center h-64 text-surface-500">
          No clips found. Try analyzing a different VOD.
        </div>
      ) : (
        <div className="flex gap-6">
          {/* Left: Clip list */}
          <div className="w-[38%] shrink-0 space-y-2 max-h-[calc(100vh-180px)] overflow-y-auto pr-1">
            {clips.map(clip => (
              <ClipCard
                key={clip.id}
                clip={clip}
                selected={clip.id === selectedId}
                onClick={() => selectClip(clip.id)}
              />
            ))}
          </div>

          {/* Right: Detail panel */}
          <div className="flex-1 min-w-0">
            {selectedClip ? (
              <ClipDetailPanel clip={selectedClip} />
            ) : (
              <div className="flex items-center justify-center h-64 text-surface-500 text-sm">
                Select a clip to see details
              </div>
            )}
          </div>
        </div>
      )}
    </>
  )
}
