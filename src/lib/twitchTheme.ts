import type { TwitchThemePalette } from '../stores/uiStore'

interface Candidate {
  color: [number, number, number]
  count: number
  hue: number
  saturation: number
}

function rgbToHsl([r8, g8, b8]: [number, number, number]) {
  const r = r8 / 255
  const g = g8 / 255
  const b = b8 / 255
  const max = Math.max(r, g, b)
  const min = Math.min(r, g, b)
  const delta = max - min
  let hue = 0
  if (delta) {
    if (max === r) hue = ((g - b) / delta) % 6
    else if (max === g) hue = (b - r) / delta + 2
    else hue = (r - g) / delta + 4
    hue = (hue * 60 + 360) % 360
  }
  const lightness = (max + min) / 2
  const saturation = delta ? delta / (1 - Math.abs(2 * lightness - 1)) : 0
  return { hue, saturation, lightness }
}

function toHex([r, g, b]: [number, number, number]) {
  return `#${[r, g, b].map(value => Math.round(value).toString(16).padStart(2, '0')).join('')}`
}

function hueDistance(a: number, b: number) {
  const delta = Math.abs(a - b)
  return Math.min(delta, 360 - delta)
}

function darkSurface([r, g, b]: [number, number, number]) {
  return toHex([Math.max(7, r * 0.1), Math.max(8, g * 0.1), Math.max(12, b * 0.1)])
}

function loadImage(dataUrl: string) {
  return new Promise<HTMLImageElement>((resolve, reject) => {
    const image = new Image()
    image.onload = () => resolve(image)
    image.onerror = () => reject(new Error('Twitch artwork could not be decoded'))
    image.src = dataUrl
  })
}

export async function paletteFromImage(
  dataUrl: string,
  sourceImageUrl: string,
): Promise<TwitchThemePalette> {
  const image = await loadImage(dataUrl)
  const canvas = document.createElement('canvas')
  canvas.width = 64
  canvas.height = 64
  const context = canvas.getContext('2d', { willReadFrequently: true })
  if (!context) throw new Error('Color analysis is unavailable on this device')
  context.drawImage(image, 0, 0, canvas.width, canvas.height)
  const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data
  const buckets = new Map<string, { sum: [number, number, number]; count: number }>()

  for (let index = 0; index < pixels.length; index += 16) {
    if (pixels[index + 3] < 180) continue
    const color: [number, number, number] = [pixels[index], pixels[index + 1], pixels[index + 2]]
    const { saturation, lightness } = rgbToHsl(color)
    if (saturation < 0.22 || lightness < 0.12 || lightness > 0.88) continue
    const key = color.map(value => Math.round(value / 32)).join('-')
    const bucket = buckets.get(key) ?? { sum: [0, 0, 0], count: 0 }
    bucket.sum[0] += color[0]
    bucket.sum[1] += color[1]
    bucket.sum[2] += color[2]
    bucket.count += 1
    buckets.set(key, bucket)
  }

  const candidates: Candidate[] = [...buckets.values()].map(bucket => {
    const color = bucket.sum.map(value => value / bucket.count) as [number, number, number]
    const hsl = rgbToHsl(color)
    return { color, count: bucket.count, hue: hsl.hue, saturation: hsl.saturation }
  }).sort((a, b) => (b.count * (0.55 + b.saturation)) - (a.count * (0.55 + a.saturation)))

  const fallback: Candidate[] = [
    { color: [145, 70, 255], count: 1, hue: 264, saturation: 1 },
    { color: [255, 55, 178], count: 1, hue: 325, saturation: 1 },
    { color: [40, 220, 210], count: 1, hue: 177, saturation: 0.7 },
  ]
  const primary = candidates[0] ?? fallback[0]
  const secondary = candidates.find(candidate => hueDistance(candidate.hue, primary.hue) >= 38)
    ?? fallback.find(candidate => hueDistance(candidate.hue, primary.hue) >= 38)
    ?? fallback[1]
  const accent = candidates.find(candidate =>
    hueDistance(candidate.hue, primary.hue) >= 70 && hueDistance(candidate.hue, secondary.hue) >= 38)
    ?? fallback.find(candidate =>
      hueDistance(candidate.hue, primary.hue) >= 70 && hueDistance(candidate.hue, secondary.hue) >= 38)
    ?? fallback[2]

  return {
    primary: toHex(primary.color),
    secondary: toHex(secondary.color),
    accent: toHex(accent.color),
    surface: darkSurface(primary.color),
    sourceImageUrl,
    generatedAt: new Date().toISOString(),
  }
}
