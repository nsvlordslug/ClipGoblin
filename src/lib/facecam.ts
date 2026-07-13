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
