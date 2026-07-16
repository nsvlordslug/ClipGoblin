export interface RetryableUploadState {
  status: string
  acceptedWithoutLink?: boolean
}

export function isSuccessfulUploadHandoff(status: string | undefined): boolean {
  return status === 'done' || status === 'processing'
}

export function shouldOfferForcedReupload(state: RetryableUploadState | undefined): boolean {
  return state?.status === 'duplicate'
}
