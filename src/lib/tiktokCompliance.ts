export type TikTokPublishMode = 'direct' | 'draft'

export interface TikTokComplianceValue {
  publishMode: TikTokPublishMode
  privacyLevel: string | null
  disableComment: boolean
  disableDuet: boolean
  disableStitch: boolean
  discloseContent: boolean
  yourBrand: boolean
  brandedContent: boolean
}

export const EMPTY_TIKTOK_COMPLIANCE: TikTokComplianceValue = {
  publishMode: 'direct',
  privacyLevel: null,
  // TikTok requires every interaction permission to start unchecked so the
  // creator explicitly opts in before publishing.
  disableComment: true,
  disableDuet: true,
  disableStitch: true,
  discloseContent: false,
  yourBrand: false,
  brandedContent: false,
}
