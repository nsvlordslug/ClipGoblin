export interface TikTokComplianceValue {
  privacyLevel: string | null
  disableComment: boolean
  disableDuet: boolean
  disableStitch: boolean
  discloseContent: boolean
  yourBrand: boolean
  brandedContent: boolean
}

export const EMPTY_TIKTOK_COMPLIANCE: TikTokComplianceValue = {
  privacyLevel: null,
  disableComment: false,
  disableDuet: false,
  disableStitch: false,
  discloseContent: false,
  yourBrand: false,
  brandedContent: false,
}
