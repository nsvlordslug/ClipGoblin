import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Loader2 } from 'lucide-react'
import { usePlatformStore } from '../stores/platformStore'
import { errorMessage } from '../lib/errors'
import { recoverYouTubeUpload, recordYouTubeAbsenceReview } from '../lib/youtubeRecoveryActions'
import {
  absenceReviewArgs, canConfirmAbsence, recoveryFormatLabel, YouTubeRecoveryRequestGate,
} from '../lib/youtubeRecovery'
import type { UncertainYouTubeUpload, YouTubeRecoveryAspect, YouTubeRecoveryResult } from '../lib/youtubeRecovery'

interface Props {
  clipId: string
  uploads: UncertainYouTubeUpload[]
  onCompleted: (aspectRatio: YouTubeRecoveryAspect, result: YouTubeRecoveryResult) => void
  onAllowed: (aspectRatio: YouTubeRecoveryAspect) => void
}

function RecoveryCard({ clipId, upload, onCompleted, onAllowed }: Omit<Props, 'uploads'> & { upload: UncertainYouTubeUpload }) {
  const account = usePlatformStore(state => state.accounts.youtube)
  const accountId = account?.account_id || null
  const [result, setResult] = useState<YouTubeRecoveryResult | null>(null)
  const [message, setMessage] = useState(upload.message)
  const [confirmedAbsent, setConfirmedAbsent] = useState(false)
  const [busy, setBusy] = useState(false)
  const busyRef = useRef(false)
  const gateRef = useRef(new YouTubeRecoveryRequestGate())
  const aspectRatio = upload.aspectRatio

  useEffect(() => {
    gateRef.current.cancel()
    busyRef.current = false
    setBusy(false)
    setConfirmedAbsent(false)
    setResult(null)
    return () => { gateRef.current.cancel() }
  }, [clipId, aspectRatio, accountId])

  const checkAndResume = async () => {
    const connectedId = usePlatformStore.getState().accounts.youtube?.account_id || null
    if (busyRef.current || !connectedId) return
    const request = gateRef.current.begin(clipId, aspectRatio, connectedId)
    const isCurrent = () => gateRef.current.isCurrent(request, clipId, aspectRatio,
      usePlatformStore.getState().accounts.youtube?.account_id || null)
    busyRef.current = true
    setBusy(true)
    setConfirmedAbsent(false)
    setResult(null)
    setMessage('Checking the original YouTube upload…')
    try {
      const recovery = await recoverYouTubeUpload(clipId, aspectRatio)
      if (!isCurrent()) return
      if (recovery.aspect_ratio !== aspectRatio) throw new Error('YouTube returned a different upload format. Check this upload again.')
      setResult(recovery)
      setMessage(recovery.message)
      if (recovery.status === 'completed') onCompleted(aspectRatio, recovery)
    } catch (error) {
      if (isCurrent()) setMessage(errorMessage(error, 'Could not check the YouTube upload. Try again later.'))
    } finally {
      if (isCurrent()) { busyRef.current = false; setBusy(false) }
    }
  }

  const allowNewUpload = async () => {
    if (busyRef.current) return
    const connectedId = usePlatformStore.getState().accounts.youtube?.account_id || null
    try {
      const args = absenceReviewArgs(clipId, aspectRatio, result, confirmedAbsent, connectedId)
      const request = gateRef.current.begin(clipId, aspectRatio, args.targetAccountId)
      const isCurrent = () => gateRef.current.isCurrent(request, clipId, aspectRatio,
        usePlatformStore.getState().accounts.youtube?.account_id || null)
      busyRef.current = true
      setBusy(true)
      try {
        await recordYouTubeAbsenceReview(args)
        if (isCurrent()) onAllowed(aspectRatio)
      } catch (error) {
        if (isCurrent()) setMessage(errorMessage(error, 'Could not save this review. Check the upload again.'))
      } finally {
        if (isCurrent()) { busyRef.current = false; setBusy(false) }
      }
    } catch (error) {
      setMessage(errorMessage(error, 'Review the destination channel first.'))
    }
  }

  const reviewAccountId = result?.account_id
  return (
    <section className="space-y-2 border-l-2 border-amber-400 bg-amber-500/5 px-3 py-3 text-[11px] text-amber-100"
      aria-label={`${recoveryFormatLabel(aspectRatio)} recovery`}>
      <p className="font-semibold">{recoveryFormatLabel(aspectRatio)} · upload needs a check</p>
      <p role="status" className="leading-relaxed">{message}</p>
      <p className="text-amber-100/70">Check and resume verifies the existing upload and can finish sending the original video.</p>
      {reviewAccountId && <p className="break-all text-amber-100/70">Destination channel for this review: {reviewAccountId}</p>}
      {!accountId && <p>Connect YouTube in Settings to check this upload.</p>}
      <div className="flex flex-wrap gap-3 items-center">
        <button type="button" onClick={() => void checkAndResume()} disabled={busy || !accountId}
          className="inline-flex items-center gap-1 rounded border border-amber-400/40 px-2 py-1.5 font-medium disabled:opacity-40 cursor-pointer">
          {busy && <Loader2 className="h-3 w-3 animate-spin" />}
          {busy ? 'Working…' : result?.status === 'retry_later' ? 'Check again later' : 'Check and resume'}
        </button>
        <button type="button" onClick={() => invoke('open_url', { url: 'https://studio.youtube.com/' })
          .catch(error => setMessage(errorMessage(error, 'Could not open YouTube Studio.')))}
          className="text-amber-200 underline cursor-pointer">Open YouTube Studio</button>
      </div>
      {result?.status === 'review_required' && (
        <div className="space-y-2 border-t border-amber-400/20 pt-2">
          <p className="break-words">{result.original_title
            ? <>Original upload title: {result.original_title}</>
            : 'Original upload title unavailable'}</p>
          {(!reviewAccountId || accountId !== reviewAccountId) && (
            <p>Connect the channel shown for this review and check again before allowing another upload.</p>
          )}
          <label className="flex items-start gap-2 leading-relaxed">
            <input type="checkbox" checked={confirmedAbsent} onChange={event => setConfirmedAbsent(event.target.checked)}
              disabled={busy || !reviewAccountId || accountId !== reviewAccountId} className="mt-0.5" />
            <span>I checked the destination channel and this video is not there</span>
          </label>
          <button type="button" onClick={() => void allowNewUpload()}
            disabled={busy || !canConfirmAbsence(clipId, aspectRatio, result, confirmedAbsent, accountId)}
            className="rounded border border-amber-400/40 px-2 py-1.5 font-medium disabled:opacity-40 cursor-pointer">
            Allow a new upload
          </button>
          <p className="text-amber-100/70">This records your review and unlocks the upload button. It does not upload a video.</p>
        </div>
      )}
    </section>
  )
}

export default function YouTubeUploadRecovery({ clipId, uploads, onCompleted, onAllowed }: Props) {
  return <>{uploads.map(upload => <RecoveryCard key={`${clipId}:${upload.aspectRatio}`} clipId={clipId}
    upload={upload} onCompleted={onCompleted} onAllowed={onAllowed} />)}</>
}
