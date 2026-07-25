export const DEFAULT_FULL_FRAME_SCALE = 1
export const MIN_FULL_FRAME_SCALE = 0.7

export function normalizeFullFrameScale(value: number | null | undefined): number {
  if (!Number.isFinite(value)) return DEFAULT_FULL_FRAME_SCALE
  return Math.min(
    DEFAULT_FULL_FRAME_SCALE,
    Math.max(MIN_FULL_FRAME_SCALE, value as number),
  )
}

export function fullFrameZoomOutPercent(scale: number): number {
  return Math.round((1 - normalizeFullFrameScale(scale)) * 100)
}
