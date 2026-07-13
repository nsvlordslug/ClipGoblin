export const MIN_CAPTION_FONT_SCALE = 0.75
export const MAX_CAPTION_FONT_SCALE = 1.25
export const DEFAULT_CAPTION_FONT_SCALE = 1

export function clampCaptionFontScale(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_CAPTION_FONT_SCALE
  return Math.min(MAX_CAPTION_FONT_SCALE, Math.max(MIN_CAPTION_FONT_SCALE, value))
}

export function longestCaptionWordLength(text: string): number {
  const words = text.replace(/\\N/g, ' ').trim().split(/\s+/).filter(Boolean)
  return words.reduce((longest, word) => Math.max(longest, Array.from(word).length), 1)
}

interface FitCaptionFontSizeOptions {
  requestedPx: number
  frameWidth: number
  isVertical: boolean
  text: string
  characterWidthFactor?: number
  safeWidthRatio?: number
  wraps?: boolean
}

/**
 * Caps a requested caption size to both a reasonable format maximum and the
 * width available to its longest unbreakable piece of text.
 */
export function fitCaptionFontSize({
  requestedPx,
  frameWidth,
  isVertical,
  text,
  characterWidthFactor = 0.66,
  safeWidthRatio = 0.84,
  wraps = true,
}: FitCaptionFontSizeOptions): number {
  const width = Math.max(1, frameWidth)
  const hardMax = width * (isVertical ? 0.085 : 0.065)
  const widthUnits = wraps
    ? longestCaptionWordLength(text)
    : Math.max(1, ...text.split(/\r?\n/).map(line => Array.from(line).length))
  const wordFitMax = (width * safeWidthRatio) / (widthUnits * characterWidthFactor)

  return Math.max(8, Math.min(requestedPx, hardMax, wordFitMax))
}
