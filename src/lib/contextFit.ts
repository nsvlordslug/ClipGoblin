export const DEFAULT_CONTEXT_BLUR_STRENGTH = 0.25
export const DEFAULT_CONTEXT_VIDEO_Y = 0.5

export function clampContextUnit(value: number, fallback: number): number {
  return Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : fallback
}

export function normalizeContextBlurStrength(value: number | null | undefined): number {
  return clampContextUnit(value ?? DEFAULT_CONTEXT_BLUR_STRENGTH, DEFAULT_CONTEXT_BLUR_STRENGTH)
}

export function normalizeContextVideoY(value: number | null | undefined): number {
  return clampContextUnit(value ?? DEFAULT_CONTEXT_VIDEO_Y, DEFAULT_CONTEXT_VIDEO_Y)
}

export function contextBlurPixels(strength: number): number {
  return 1.5 + normalizeContextBlurStrength(strength) * 10
}

export function contextVideoPositionLabel(position: number): string {
  const normalized = normalizeContextVideoY(position)
  if (normalized <= 0.15) return 'Top'
  if (normalized < 0.4) return 'Upper'
  if (normalized <= 0.6) return 'Center'
  if (normalized < 0.85) return 'Lower'
  return 'Bottom'
}

export function brandingAssetName(path: string | null | undefined): string {
  if (!path) return ''
  return path.split(/[\\/]/).pop() || path
}
