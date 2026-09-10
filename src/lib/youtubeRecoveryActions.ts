import { invoke } from '@tauri-apps/api/core'
import { useScheduleStore } from '../stores/scheduleStore'
import type { absenceReviewArgs, YouTubeRecoveryAspect, YouTubeRecoveryResult } from './youtubeRecovery'

export async function recoverYouTubeUpload(clipId: string, aspectRatio: YouTubeRecoveryAspect): Promise<YouTubeRecoveryResult> {
  const result = await invoke<YouTubeRecoveryResult>('recover_youtube_upload', { clipId, aspectRatio })
  // The persisted schedule changes even if the initiating editor has since unmounted.
  // Callers gate their own UI updates only after this shared refresh finishes.
  if (result.status === 'completed') await useScheduleStore.getState().load({ force: true })
  return result
}

export async function recordYouTubeAbsenceReview(args: ReturnType<typeof absenceReviewArgs>): Promise<void> {
  await invoke('review_youtube_upload_absent', args)
  await useScheduleStore.getState().load({ force: true })
}
