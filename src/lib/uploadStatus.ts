export interface RetryableUploadState {
  status: string
  acceptedWithoutLink?: boolean
}

export function isSuccessfulUploadHandoff(status: string | undefined): boolean {
  return status === 'done' || status === 'processing'
}

export function isTikTokInboxDelivered(status: string | undefined): boolean {
  return status === 'inbox_delivered'
}

export function shouldOfferForcedReupload(state: RetryableUploadState | undefined): boolean {
  return state?.status === 'duplicate'
}
