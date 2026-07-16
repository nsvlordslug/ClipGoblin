export interface FacecamSettings {
  pipX: number
  pipY: number
  pipW: number
  pipH: number
  splitRatio: number
  cropX: number
  cropY: number
  cropW: number
  cropH: number
}

export const DEFAULT_FACECAM: FacecamSettings = {
  pipX: 68,
  pipY: 65,
  pipW: 28,
  pipH: 28,
  splitRatio: 0.6,
  cropX: 0,
  cropY: 0.6,
  cropW: 0.4,
  cropH: 0.4,
}

function finiteNumber(value: unknown, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value))
}

export function normalizeFacecamSettings(value: unknown): FacecamSettings {
  const settings = value && typeof value === 'object'
    ? value as Partial<FacecamSettings>
    : {}
  const pipW = clamp(finiteNumber(settings.pipW, DEFAULT_FACECAM.pipW), 15, 45)
  const pipH = clamp(finiteNumber(settings.pipH, DEFAULT_FACECAM.pipH), 15, 45)
  const cropW = clamp(finiteNumber(settings.cropW, DEFAULT_FACECAM.cropW), 0.05, 1)
  const cropH = clamp(finiteNumber(settings.cropH, DEFAULT_FACECAM.cropH), 0.05, 1)

  return {
    pipX: clamp(finiteNumber(settings.pipX, DEFAULT_FACECAM.pipX), 0, 100 - pipW),
    pipY: clamp(finiteNumber(settings.pipY, DEFAULT_FACECAM.pipY), 0, 100 - pipH),
    pipW,
    pipH,
    splitRatio: clamp(finiteNumber(settings.splitRatio, DEFAULT_FACECAM.splitRatio), 0.3, 0.8),
    cropX: clamp(finiteNumber(settings.cropX, DEFAULT_FACECAM.cropX), 0, 1 - cropW),
    cropY: clamp(finiteNumber(settings.cropY, DEFAULT_FACECAM.cropY), 0, 1 - cropH),
    cropW,
    cropH,
  }
}

export function parseFacecamSettings(value: unknown): FacecamSettings {
  if (typeof value !== 'string') return normalizeFacecamSettings(value)
  if (!value.trim()) return { ...DEFAULT_FACECAM }

  try {
    return normalizeFacecamSettings(JSON.parse(value))
  } catch {
    return { ...DEFAULT_FACECAM }
  }
}

/** Compute whether the caption anchor overlaps a facecam region. */
export function computeSubtitleCollision(
  captionY: number,
  layout: string,
  settings: FacecamSettings,
): { collides: boolean; safeY: number } {
  if (layout === 'pip') {
    const pipTop = settings.pipY
    const pipBottom = settings.pipY + settings.pipH
    if (captionY > pipTop - 5 && captionY < pipBottom + 3) {
      return { collides: true, safeY: Math.max(5, pipTop - 8) }
    }
  }

  if (layout === 'split') {
    const splitLine = settings.splitRatio * 100
    if (captionY > splitLine - 5) {
      return { collides: true, safeY: Math.max(5, splitLine - 10) }
    }
  }

  return { collides: false, safeY: captionY }
}
