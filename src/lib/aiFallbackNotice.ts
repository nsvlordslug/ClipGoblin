export type CaptionAiProvider = 'free' | 'openai' | 'claude' | 'gemini'

export type CaptionFallbackReason = 'provider-returned-free' | 'request-failed' | 'no-new-caption'

interface CaptionFallbackContext {
  provider: CaptionAiProvider
  useForCaptions: boolean
  hasActiveKey: boolean
  reason: CaptionFallbackReason
}

const PROVIDER_NAMES: Record<CaptionAiProvider, string> = {
  free: 'Free mode',
  openai: 'OpenAI',
  claude: 'Claude',
  gemini: 'Gemini',
}

export function getCaptionFallbackNotice({
  provider,
  useForCaptions,
  hasActiveKey,
  reason,
}: CaptionFallbackContext): string | null {
  if (provider === 'free') return null

  const providerName = PROVIDER_NAMES[provider]
  if (!useForCaptions) {
    return `${providerName} caption generation is turned off in Settings. Free mode was used. Turn on "Caption generation (TikTok copy)" to use your key.`
  }
  if (!hasActiveKey) {
    return `${providerName} is selected, but no API key is saved. Free mode was used. Add or test the key in Settings.`
  }
  if (reason === 'no-new-caption') {
    return `${providerName} did not return a different caption for this reroll. Free mode was used instead.`
  }
  return `${providerName} could not generate this caption. Free mode was used. Test the connection in Settings and try again.`
}
