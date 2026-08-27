import React, { useCallback, useMemo, useRef, useState, useEffect } from 'react'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import type { CaptionToken } from '../lib/captionEmphasis'
import { EMPHASIS_STYLES } from '../lib/captionEmphasis'
import type { CaptionStyle } from '../lib/editTypes'
import {
  clampCaptionCardScale,
  clampCaptionFontScale,
  DEFAULT_CAPTION_CARD_SCALE,
  fitCaptionFontSize,
} from '../lib/captionSizing'
import { findActiveSegment, frameSafeSubtitleSegments } from '../lib/subtitleUtils'
import type { SubtitleSegment } from '../lib/subtitleUtils'

interface Props {
  segments: SubtitleSegment[]
  emphasisTokens?: CaptionToken[]
  captionStyle: CaptionStyle
  fontScale?: number
  cardScale?: number
  currentTime: number
  trimStart?: number
  trimEnd?: number
  position: 'top' | 'center' | 'bottom'
  yPercent?: number
  emphasisEnabled: boolean
  outputWidth?: number
  outputHeight?: number
  captionProvenance?: string
  onPreparingChange?: (preparing: boolean) => void
}

interface ImageCaptionAsset {
  path: string
  rendererVersion: string
}

interface ImageCaptionRenderPlan {
  presentation: CaptionStyle['presentation']
  command: string
  request: {
    styleId?: string
    cardScale?: number
    text: string
    targetWidth: number
    targetHeight: number
    fontSize: number
    anchorY: number
    alignment: number
  }
}

const DESIGN_WIDTH = 1080
const GLOSSY_PREVIEW_MIN_DIMENSION = 360
const GLOSSY_PRELOAD_CONCURRENCY = 3
const EMPTY_CAPTION_ASSETS = new Map<string, string>()

async function decodeCaptionImage(url: string): Promise<HTMLImageElement> {
  const image = new Image()
  image.decoding = 'async'
  image.src = url
  if (typeof image.decode === 'function') {
    await image.decode()
    return image
  }
  await new Promise<void>((resolve, reject) => {
    image.onload = () => resolve()
    image.onerror = () => reject(new Error('Caption preview image could not be decoded'))
  })
  return image
}

// ── Layer 1: Tokenizer (style-agnostic) ──
// Split caption text by whitespace, then re-attach any standalone
// punctuation-only tokens (e.g. whisper emits "jump . Let's go .") onto the
// previous word. Every token ends up carrying its own leading/trailing
// punctuation so "jump." is one unit, never split.
function tokenize(text: string): string[] {
  const raw = text.split(/\s+/).filter(Boolean)
  const merged: string[] = []
  for (const rw of raw) {
    if (!/\w/.test(rw) && merged.length > 0) {
      merged[merged.length - 1] += rw
    } else {
      merged.push(rw)
    }
  }
  return merged
}

// Split a token into leading punct, bare word, trailing punct.
// "jump." → { leading: '', bare: 'jump', trailing: '.' }
// Emphasis matching uses `bare`; rendering uses all three concatenated.
function splitToken(token: string): { leading: string; bare: string; trailing: string } {
  const m = token.match(/^([^\w]*)([\w][\w']*)([^\w]*)$/)
  if (!m) return { leading: '', bare: token, trailing: '' }
  return { leading: m[1], bare: m[2], trailing: m[3] }
}

function layeredFaceStyle(
  presentation: CaptionStyle['presentation'],
  emphasized: boolean,
): React.CSSProperties {
  if (presentation === 'paper-mischief') {
    return {
      color: emphasized ? '#B8FF2C' : '#F3F0E8',
      WebkitTextFillColor: emphasized ? '#B8FF2C' : '#F3F0E8',
    }
  }

  if (presentation === 'goblin-bite') {
    return {
      color: emphasized ? '#FFFFFF' : '#DFFF20',
      WebkitTextFillColor: emphasized ? '#FFFFFF' : '#DFFF20',
    }
  }

  if (presentation === 'undead-legion') {
    return {
      color: emphasized ? '#FF30CD' : '#B2FF1C',
      backgroundImage: 'linear-gradient(180deg, #C8FF3D 0%, #8FF02C 54%, #FF30CD 66%, #BE177F 100%)',
      backgroundClip: 'text',
      WebkitBackgroundClip: 'text',
      WebkitTextFillColor: 'transparent',
    }
  }

  return {}
}

function renderTapeRiotGlyphs(text: string, emphasized: boolean, seed: number) {
  let faceIndex = seed + (emphasized ? 1 : 0)
  return Array.from(text).map((glyph, index) => {
    if (!/[a-z0-9]/i.test(glyph)) return glyph
    const purple = faceIndex++ % 2 === 1
    return (
      <span key={`${index}-${glyph}`} style={{
        display: 'inline',
        color: purple ? '#7C2FE4' : '#B8FF2C',
        backgroundImage: purple
          ? 'repeating-linear-gradient(0deg, rgba(255,255,255,0.13) 0 1px, transparent 1px 4px), linear-gradient(180deg, #B66BFF 0%, #8334F2 48%, #5E20B6 100%)'
          : 'repeating-linear-gradient(0deg, rgba(255,255,255,0.17) 0 1px, rgba(66,88,16,0.10) 1px 3px, transparent 3px 5px), linear-gradient(180deg, #DBFF65 0%, #AFFF1F 48%, #79C916 100%)',
        backgroundBlendMode: 'soft-light, normal',
        backgroundClip: 'text',
        WebkitBackgroundClip: 'text',
        WebkitTextFillColor: 'transparent',
      }}>
        {glyph}
      </span>
    )
  })
}

type MaterialPresentation = 'tape-riot' | 'paper-mischief' | 'goblin-bite' | 'undead-legion'

interface MaterialCaptionTextProps {
  presentation: MaterialPresentation
  text: string
  emphasized?: boolean
  seed?: number
}

interface MaterialLayer {
  x: number
  y: number
  color: string
  stroke?: string
}

interface MaterialContactShadow {
  x: number
  y: number
  blur: number
  color: string
  stroke: string
}

interface MaterialFaceHighlight {
  x: number
  y: number
  color: string
  stroke: string
}

function buildPaperMischiefDepth(): MaterialLayer[] {
  const layers: MaterialLayer[] = []
  for (let step = 12; step >= 1; step -= 1) {
    const color = step <= 2
      ? '#8B8588'
      : step <= 5
        ? '#3B353D'
        : step <= 9
          ? '#5A2A78'
          : '#321540'
    layers.push({
      x: step * 0.0115,
      y: step * 0.0145,
      color,
      stroke: step === 12 ? '#0B080D' : undefined,
    })
  }
  return layers
}

const MATERIAL_LAYERS: Record<MaterialPresentation, {
  depth: MaterialLayer[]
  rim: string
  detailFamily: string
  detailColor: string
  accentFamily?: string
  accentColor?: string
  contactShadow?: MaterialContactShadow
  faceHighlight?: MaterialFaceHighlight
  faceStroke?: number
  rimStroke?: number
  rimX?: number
  rimY?: number
}> = {
  'tape-riot': {
    depth: [
      { x: 0.16, y: 0.18, color: '#09070C', stroke: '#030304' },
      { x: 0.10, y: 0.12, color: '#351856', stroke: '#09070C' },
      { x: 0.045, y: 0.055, color: '#151018', stroke: '#050506' },
    ],
    rim: '#08070A',
    detailFamily: "'ClipGoblin Tape Riot Seams'",
    detailColor: 'rgba(24, 12, 31, 0.72)',
    accentFamily: "'ClipGoblin Tape Riot Patches'",
    accentColor: '#FFD326',
  },
  'paper-mischief': {
    depth: buildPaperMischiefDepth(),
    rim: '#2C2730',
    detailFamily: "'ClipGoblin Paper Mischief Fiber'",
    detailColor: 'rgba(92, 86, 82, 0.46)',
    accentFamily: "'ClipGoblin Paper Mischief Tabs'",
    accentColor: '#AFFF24',
    contactShadow: {
      x: 0.205,
      y: 0.265,
      blur: 0.055,
      color: 'rgba(0, 0, 0, 0.72)',
      stroke: 'rgba(0, 0, 0, 0.80)',
    },
    faceStroke: 0.015,
    rimStroke: 0.022,
    rimX: 0.010,
    rimY: 0.012,
  },
  'goblin-bite': {
    depth: [
      { x: 0.12, y: 0.135, color: '#220C32', stroke: '#07040A' },
      { x: 0.072, y: 0.085, color: '#7A28B1', stroke: '#210D30' },
      { x: 0.03, y: 0.038, color: '#1B151E', stroke: '#080609' },
    ],
    rim: '#171119',
    detailFamily: "'ClipGoblin Goblin Bite Distress'",
    detailColor: 'rgba(48, 70, 12, 0.72)',
  },
  'undead-legion': {
    depth: [
      { x: -0.10, y: 0.18, color: '#050306', stroke: '#020103' },
      { x: -0.045, y: 0.105, color: '#9E146F', stroke: '#08050A' },
      { x: 0, y: 0.038, color: '#161018', stroke: '#080609' },
    ],
    rim: '#0C0708',
    detailFamily: "'Bangers'",
    detailColor: 'rgba(16, 26, 13, 0.20)',
  },
}

export function MaterialCaptionText({
  presentation,
  text,
  emphasized = false,
  seed = 0,
}: MaterialCaptionTextProps) {
  const material = MATERIAL_LAYERS[presentation]
  const faceStyle = layeredFaceStyle(presentation, emphasized)
  const face = presentation === 'tape-riot'
    ? renderTapeRiotGlyphs(text, emphasized, seed)
    : text
  const rimLayer = material.depth.length + 1
  const highlightLayer = rimLayer + 1
  const faceLayer = highlightLayer + (material.faceHighlight ? 1 : 0)
  const layerStyle: React.CSSProperties = {
    position: 'absolute',
    inset: 0,
    whiteSpace: 'inherit',
    pointerEvents: 'none',
    WebkitTextStroke: '0',
  }

  return (
    <span style={{ position: 'relative', display: 'inline-block', isolation: 'isolate' }}>
      {material.contactShadow && (
        <span aria-hidden="true" style={{
          ...layerStyle,
          zIndex: 0,
          color: material.contactShadow.color,
          transform: `translate(${material.contactShadow.x}em, ${material.contactShadow.y}em)`,
          WebkitTextStroke: `0.05em ${material.contactShadow.stroke}`,
          filter: `blur(${material.contactShadow.blur}em)`,
          paintOrder: 'stroke fill',
        }}>{text}</span>
      )}
      {material.depth.map((layer, index) => (
        <span key={`depth-${index}`} aria-hidden="true" style={{
          ...layerStyle,
          zIndex: index + 1,
          color: layer.color,
          transform: `translate(${layer.x}em, ${layer.y}em)`,
          WebkitTextStroke: layer.stroke ? `0.03em ${layer.stroke}` : '0',
          paintOrder: 'stroke fill',
        }}>{text}</span>
      ))}
      <span aria-hidden="true" style={{
        ...layerStyle,
        zIndex: rimLayer,
        color: material.rim,
        transform: `translate(${material.rimX ?? 0.018}em, ${material.rimY ?? 0.022}em)`,
        WebkitTextStroke: `${material.rimStroke ?? 0.045}em ${material.rim}`,
        paintOrder: 'stroke fill',
      }}>{text}</span>
      {material.faceHighlight && (
        <span aria-hidden="true" style={{
          ...layerStyle,
          zIndex: highlightLayer,
          color: material.faceHighlight.color,
          transform: `translate(${material.faceHighlight.x}em, ${material.faceHighlight.y}em)`,
          WebkitTextStroke: `0.025em ${material.faceHighlight.stroke}`,
          paintOrder: 'stroke fill',
        }}>{text}</span>
      )}
      <span style={{
        position: 'relative',
        zIndex: faceLayer,
        display: 'inline',
        WebkitTextStroke: `${material.faceStroke ?? 0.018}em ${material.rim}`,
        paintOrder: 'stroke fill',
        ...faceStyle,
      }}>{face}</span>
      <span aria-hidden="true" style={{
        ...layerStyle,
        zIndex: faceLayer + 1,
        fontFamily: material.detailFamily,
        color: material.detailColor,
      }}>{text}</span>
      {material.accentFamily && (
        <span aria-hidden="true" style={{
          ...layerStyle,
          zIndex: faceLayer + 2,
          fontFamily: material.accentFamily,
          color: material.accentColor,
          WebkitTextStroke: '0.008em rgba(45, 30, 5, 0.45)',
        }}>{text}</span>
      )}
    </span>
  )
}

// ── Layer 2: Emphasis grouper (style-agnostic) ──
interface TokenGroup {
  emphasized: boolean
  tokens: string[]
}

function groupByEmphasis(
  tokens: string[],
  isEmphasized: (token: string, index: number) => boolean,
): TokenGroup[] {
  const groups: TokenGroup[] = []
  let current: TokenGroup | null = null
  tokens.forEach((tok, i) => {
    const emph = isEmphasized(tok, i)
    if (current && current.emphasized === emph) {
      current.tokens.push(tok)
    } else {
      if (current) groups.push(current)
      current = { emphasized: emph, tokens: [tok] }
    }
  })
  if (current) groups.push(current)
  return groups
}

export default function CaptionPreview({
  segments, emphasisTokens = [], captionStyle: cs, currentTime,
  trimStart, trimEnd, position, yPercent, emphasisEnabled, fontScale = 1,
  cardScale = DEFAULT_CAPTION_CARD_SCALE,
  outputWidth = 1080, outputHeight = 1920, captionProvenance,
  onPreparingChange,
}: Props) {
  const containerRef = useRef<HTMLDivElement>(null)
  const [frameWidth, setFrameWidth] = useState(270)
  const [frameHeight, setFrameHeight] = useState(480)

  useEffect(() => {
    const el = containerRef.current?.parentElement
    if (!el) return
    const measure = () => { setFrameWidth(el.clientWidth); setFrameHeight(el.clientHeight) }
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  const visibleSegments = useMemo(
    () => trimStart != null && trimEnd != null
      ? segments.filter(s => s.endTime > trimStart && s.startTime < trimEnd)
      : segments,
    [segments, trimStart, trimEnd],
  )
  const playbackSegments = useMemo(
    () => frameSafeSubtitleSegments(visibleSegments, captionProvenance),
    [captionProvenance, visibleSegments],
  )
  const activeSegment = useMemo(
    () => findActiveSegment(playbackSegments, currentTime),
    [playbackSegments, currentTime],
  )

  // ── Layout computation ──
  const ar = frameWidth / Math.max(frameHeight, 1)
  const isVertical = ar < 0.7
  const isLandscape = ar > 1.5
  const isCardboard = cs.presentation === 'cardboard'
  const activeSegmentIndex = activeSegment ? playbackSegments.indexOf(activeSegment) : -1
  const previousSegmentText = activeSegmentIndex > 0
    ? playbackSegments[activeSegmentIndex - 1].text.trim()
    : ''
  const cardboardLeadWord = isCardboard && (
    activeSegmentIndex === 0 || /[.!?]["')\]]?$/.test(previousSegmentText)
  )
  const baseFontColor = cardboardLeadWord ? '#15100C' : cs.fontColor

  // Scale: frame width relative to 1080 design, with format boost
  const baseScale = frameWidth / DESIGN_WIDTH
  const boost = isVertical ? 1.15 : isLandscape ? 0.85 : 1.0
  const scale = Math.max(0.15, Math.min(0.55, baseScale * boost))

  // Font size: respect the user's bounded scale, then shrink only when a word
  // would escape the platform-safe horizontal area.
  const safeFontScale = clampCaptionFontScale(fontScale)
  const safeCardScale = clampCaptionCardScale(cardScale)
  const rawFontSize = cs.fontSize * scale * safeFontScale
  const baseFontSize = fitCaptionFontSize({
    requestedPx: rawFontSize,
    frameWidth,
    isVertical,
    text: activeSegment?.text || '',
    characterWidthFactor: cs.characterWidthFactor,
    safeWidthRatio: cs.safeWidthRatio,
  })

  const buildImageCaptionRequest = useCallback((captionText: string): ImageCaptionRenderPlan | null => {
    const isReferenceImageGlyph = cs.presentation === 'hellfire'
      || cs.presentation === 'horror'
      || cs.presentation === 'scary'
      || cs.presentation === 'glossy-thumbnail'
    const isPersistentCardboard = cs.presentation === 'cardboard'
    if ((cs.presentation !== 'paper-mischief'
      && cs.presentation !== 'undead-legion'
      && !isReferenceImageGlyph
      && !isPersistentCardboard) || (!captionText && !isPersistentCardboard)) return null
    const glossyPreviewScale = cs.presentation === 'glossy-thumbnail'
      ? Math.min(1, Math.max(
          GLOSSY_PREVIEW_MIN_DIMENSION / Math.max(1, outputWidth),
          GLOSSY_PREVIEW_MIN_DIMENSION / Math.max(1, outputHeight),
        ))
      : 1
    const targetWidth = Math.max(320, Math.round(outputWidth * glossyPreviewScale))
    const targetHeight = Math.max(320, Math.round(outputHeight * glossyPreviewScale))
    const anchorPercent = yPercent ?? (position === 'top' ? 8 : position === 'center' ? 50 : 97)
    const outputFontSize = Math.floor(fitCaptionFontSize({
      requestedPx: cs.fontSize * safeFontScale * glossyPreviewScale,
      frameWidth: targetWidth,
      isVertical: targetHeight > targetWidth,
      text: captionText,
      characterWidthFactor: cs.characterWidthFactor,
      safeWidthRatio: cs.safeWidthRatio,
    }))
    return {
      presentation: cs.presentation,
      command: isPersistentCardboard
        ? 'render_cardboard_caption'
        : cs.presentation === 'undead-legion'
        ? 'render_undead_legion_caption'
        : cs.presentation === 'paper-mischief'
          ? 'render_paper_mischief_caption'
          : 'render_image_glyph_caption',
      request: {
        ...(isReferenceImageGlyph ? { styleId: cs.id } : {}),
        ...(isPersistentCardboard ? { cardScale: safeCardScale } : {}),
        text: captionText,
        targetWidth,
        targetHeight,
        fontSize: outputFontSize,
        anchorY: Math.max(0, Math.min(targetHeight, Math.round(targetHeight * anchorPercent / 100))),
        alignment: position === 'top' ? 8 : position === 'center' ? 5 : 2,
      },
    }
  }, [
    cs.characterWidthFactor,
    cs.fontSize,
    cs.id,
    cs.presentation,
    cs.safeWidthRatio,
    outputHeight,
    outputWidth,
    position,
    safeFontScale,
    safeCardScale,
    yPercent,
  ])
  const imageCaptionRequest = useMemo(
    () => buildImageCaptionRequest(activeSegment?.text || ''),
    [activeSegment, buildImageCaptionRequest],
  )
  const imageCaptionRequestKey = useMemo(
    () => imageCaptionRequest ? JSON.stringify(imageCaptionRequest) : null,
    [imageCaptionRequest],
  )
  const [imageCaptionAsset, setImageCaptionAsset] = useState<{ key: string; url: string } | null>(null)
  const [glossyCaptionCache, setGlossyCaptionCache] = useState<{
    signature: string
    assets: Map<string, string>
  } | null>(null)
  const glossyDecodedImagesRef = useRef<Map<string, HTMLImageElement>>(new Map())
  const glossyPreloadRequests = useMemo(() => {
    if (cs.presentation !== 'glossy-thumbnail') return []
    const unique = new Map<string, ImageCaptionRenderPlan>()
    for (const segment of playbackSegments) {
      const plan = buildImageCaptionRequest(segment.text)
      if (plan) unique.set(JSON.stringify(plan), plan)
    }
    return Array.from(unique, ([key, plan]) => ({ key, plan }))
  }, [buildImageCaptionRequest, cs.presentation, playbackSegments])
  const glossyPreloadSignature = useMemo(
    () => JSON.stringify(glossyPreloadRequests.map(item => item.key)),
    [glossyPreloadRequests],
  )
  const glossyCaptionAssets = glossyCaptionCache?.signature === glossyPreloadSignature
    ? glossyCaptionCache.assets
    : EMPTY_CAPTION_ASSETS
  const glossyPreloading = cs.presentation === 'glossy-thumbnail'
    && glossyPreloadRequests.length > 0
    && glossyCaptionCache?.signature !== glossyPreloadSignature

  useEffect(() => {
    if (cs.presentation !== 'glossy-thumbnail' || glossyPreloadRequests.length === 0) {
      onPreparingChange?.(false)
      return
    }

    let cancelled = false
    let cursor = 0
    const decodedImages = new Map<string, HTMLImageElement>()
    const urls = new Map<string, string>()
    glossyDecodedImagesRef.current.clear()
    onPreparingChange?.(true)

    const worker = async () => {
      while (!cancelled) {
        const item = glossyPreloadRequests[cursor]
        cursor += 1
        if (!item) return
        try {
          const asset = await invoke<ImageCaptionAsset>(item.plan.command, {
            request: item.plan.request,
          })
          const url = convertFileSrc(asset.path)
          const image = await decodeCaptionImage(url)
          if (!cancelled) {
            decodedImages.set(item.key, image)
            urls.set(item.key, url)
          }
        } catch (error) {
          if (!cancelled) console.warn('Glossy Thumbnail cue preload failed', error)
        }
      }
    }

    void Promise.all(
      Array.from(
        { length: Math.min(GLOSSY_PRELOAD_CONCURRENCY, glossyPreloadRequests.length) },
        () => worker(),
      ),
    ).then(() => {
      if (cancelled) return
      glossyDecodedImagesRef.current = decodedImages
      setGlossyCaptionCache({ signature: glossyPreloadSignature, assets: urls })
      onPreparingChange?.(false)
    })

    return () => {
      cancelled = true
      onPreparingChange?.(false)
    }
  }, [cs.presentation, glossyPreloadRequests, glossyPreloadSignature, onPreparingChange])

  useEffect(() => {
    if (!imageCaptionRequest || !imageCaptionRequestKey) {
      return
    }
    if (cs.presentation === 'glossy-thumbnail'
      && (glossyPreloading || glossyCaptionAssets.has(imageCaptionRequestKey))) return
    let cancelled = false
    invoke<ImageCaptionAsset>(imageCaptionRequest.command, {
      request: imageCaptionRequest.request,
    })
      .then(asset => {
        if (!cancelled) {
          setImageCaptionAsset({ key: imageCaptionRequestKey, url: convertFileSrc(asset.path) })
        }
      })
      .catch(error => {
        if (!cancelled) {
          console.warn(`${imageCaptionRequest.presentation} image preview fell back to text rendering`, error)
        }
      })
    return () => { cancelled = true }
  }, [
    cs.presentation,
    glossyCaptionAssets,
    glossyPreloading,
    imageCaptionRequest,
    imageCaptionRequestKey,
  ])

  // Safe margins: left/right padding inside the frame
  const safeMarginPx = Math.round(frameWidth * 0.05) // 5% each side
  const maxTextWidth = frameWidth - safeMarginPx * 2
  // Bottom safe zone: at least 6% from bottom edge
  const bottomSafe = Math.max(Math.round(frameHeight * 0.06), 10)

  // Position
  const useCustomY = yPercent != null
  let posTop: string | undefined
  let posBottom: string | undefined
  let transform: string | undefined

  if (useCustomY) {
    // For 'bottom' position, anchor from the bottom edge so multi-line / tall
    // styles grow UPWARD instead of overflowing off the bottom of the frame.
    if (position === 'bottom') {
      posBottom = `${Math.max(0, 100 - (yPercent ?? 97))}%`
    } else {
      posTop = `${yPercent}%`
      if (position === 'center') transform = 'translateY(-50%)'
    }
  } else if (position === 'top') {
    posTop = `${Math.round(frameHeight * 0.08)}px`
  } else if (position === 'center') {
    posTop = '50%'
    transform = 'translateY(-50%)'
  } else {
    posBottom = `${bottomSafe}px`
  }

  // Shadow scaling
  const scaledShadow = cs.shadow === 'none' ? 'none'
    : cs.shadow.replace(/(\d+)px/g, (_, n) => `${Math.max(1, Math.round(parseInt(n) * Math.min(scale * safeFontScale, 0.5)))}px`)

  const emphasisStyle = EMPHASIS_STYLES[cs.id] || EMPHASIS_STYLES.clean
  const keepsLayeredDepth = cs.presentation === 'tape-riot'
    || cs.presentation === 'paper-mischief'
    || cs.presentation === 'goblin-bite'
    || cs.presentation === 'undead-legion'

  // ── Layer 1: tokenize (style-agnostic, runs once per segment) ──
  const tokens = useMemo(
    () => (activeSegment ? tokenize(activeSegment.text) : []),
    [activeSegment],
  )

  // Emphasis predicate: matches by bare word + approximate timing.
  // Style-agnostic — the same predicate runs for every caption style.
  const isTokenEmphasized = useCallback(
    (token: string, index: number): boolean => {
      if (!emphasisEnabled || emphasisTokens.length === 0 || !activeSegment || tokens.length === 0) return false
      const segDuration = activeSegment.endTime - activeSegment.startTime
      const wordTime = activeSegment.startTime + (index / tokens.length) * segDuration
      const bare = splitToken(token).bare.toLowerCase().replace(/[^a-z0-9]/g, '')
      if (!bare) return false
      for (const t of emphasisTokens) {
        if (t.emphasized && Math.abs(t.startTime - wordTime) < 0.8) {
          if (t.text.toLowerCase().replace(/[^a-z0-9]/g, '') === bare) return true
        }
      }
      return false
    },
    [emphasisEnabled, emphasisTokens, activeSegment, tokens.length],
  )

  // ── Layer 2: group consecutive emphasized tokens (style-agnostic) ──
  const groups = useMemo(
    () => groupByEmphasis(tokens, isTokenEmphasized),
    [tokens, isTokenEmphasized],
  )

  const glossyCaptionUrl = imageCaptionRequestKey
    ? glossyCaptionAssets.get(imageCaptionRequestKey)
    : null
  const visibleImageCaptionAsset = glossyCaptionUrl && imageCaptionRequestKey
    ? { key: imageCaptionRequestKey, url: glossyCaptionUrl }
    : imageCaptionAsset?.key === imageCaptionRequestKey
      ? imageCaptionAsset
      : isCardboard
        ? imageCaptionAsset
        : null
  if (imageCaptionRequestKey && visibleImageCaptionAsset) {
    return (
      <div ref={containerRef} className="absolute inset-0 pointer-events-none z-10">
        <img
          src={visibleImageCaptionAsset.url}
          alt=""
          aria-hidden="true"
          className="absolute inset-0 h-full w-full pointer-events-none"
          style={{ objectFit: 'fill' }}
        />
      </div>
    )
  }

  if (!activeSegment) return null

  // New reference-built styles never impersonate their atlas with a generic
  // CSS font while the native image cue is still rendering.
  if (imageCaptionRequestKey && (
    cs.presentation === 'cardboard'
    || cs.presentation === 'hellfire'
    || cs.presentation === 'horror'
    || cs.presentation === 'scary'
    || cs.presentation === 'glossy-thumbnail'
  )) return null

  const captionFrameStyle: React.CSSProperties = isCardboard ? {
    width: `${Math.round(maxTextWidth * 0.9)}px`,
    maxWidth: `${maxTextWidth}px`,
    maxHeight: `${Math.round(frameHeight * 0.35)}px`,
    minHeight: `${Math.max(30, Math.round(baseFontSize * 1.8))}px`,
    overflow: 'hidden',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    textAlign: 'center',
    backgroundColor: cs.bgColor,
    backgroundImage: [
      'repeating-linear-gradient(0deg, rgba(82,45,20,0.11) 0 1px, transparent 1px 4px)',
      'repeating-linear-gradient(90deg, rgba(255,255,255,0.045) 0 7px, rgba(83,45,20,0.045) 7px 8px)',
      'linear-gradient(90deg, rgba(74,38,15,0.14), transparent 13%, transparent 87%, rgba(74,38,15,0.14))',
    ].join(', '),
    padding: `${Math.max(5, Math.round(cs.bgPadding * scale * 0.55))}px ${Math.max(10, Math.round(cs.bgPadding * scale))}px`,
    clipPath: 'polygon(2% 4%, 8% 1%, 15% 3%, 24% 0%, 34% 2%, 44% 1%, 55% 3%, 66% 0%, 77% 2%, 87% 1%, 98% 4%, 100% 17%, 98% 32%, 100% 50%, 98% 69%, 100% 84%, 97% 97%, 87% 99%, 77% 97%, 66% 100%, 55% 98%, 44% 100%, 34% 97%, 23% 99%, 13% 97%, 2% 100%, 0% 83%, 2% 68%, 0% 50%, 2% 31%, 0% 16%)',
    boxShadow: 'inset 0 0 0 1px rgba(75,39,17,0.28), inset 0 0 18px rgba(80,43,20,0.22)',
    filter: 'drop-shadow(0 3px 3px rgba(0,0,0,0.55))',
    boxSizing: 'border-box',
  } : {
    maxWidth: `${maxTextWidth}px`,
    maxHeight: `${Math.round(frameHeight * 0.35)}px`,
    width: `${maxTextWidth}px`,
    overflow: cs.presentation === 'paper-mischief' || cs.presentation === 'undead-legion'
      ? 'visible'
      : 'hidden',
    textAlign: 'center',
    background: cs.bgColor || undefined,
    padding: cs.bgPadding > 0
      ? `${Math.round(cs.bgPadding * scale * 0.5)}px ${Math.round(cs.bgPadding * scale * 0.8)}px`
      : `0 ${safeMarginPx * 0.3}px`,
    borderRadius: cs.bgRadius > 0 ? `${Math.round(cs.bgRadius * scale)}px` : undefined,
    boxSizing: 'border-box',
  }

  return (
    <div ref={containerRef}
      className="absolute left-0 right-0 flex justify-center pointer-events-none z-10"
      style={{ top: posTop, bottom: posBottom, transform }}>

      {/* Bounded subtitle container — all text stays inside this box */}
      <div style={captionFrameStyle}>
        {/* Text block with wrapping */}
        <div style={{
          width: '100%',
          maxWidth: '100%',
          display: 'block',
          boxSizing: 'border-box',
          margin: '0 auto',
          fontFamily: cs.fontFamily,
          fontWeight: cs.fontWeight,
          fontSize: `${baseFontSize}px`,
          letterSpacing: `${cs.letterSpacing}em`,
          lineHeight: cs.lineHeight,
          textShadow: scaledShadow,
          textTransform: cs.uppercase ? 'uppercase' : 'none',
          color: baseFontColor,
          wordBreak: 'break-word',
          overflowWrap: 'anywhere',
          whiteSpace: 'normal',
          WebkitTextStroke: cs.strokeWidth > 0 && cs.strokeColor
            ? `${Math.max(0.5, cs.strokeWidth * scale * safeFontScale)}px ${cs.strokeColor}`
            : undefined,
          paintOrder: 'stroke fill',
          WebkitFontSmoothing: 'antialiased',
        } as React.CSSProperties}>
          {/* ── Layer 3: render — one code path, style config is data only ── */}
          {groups.map((group, gi) => {
            const isEmph = group.emphasized
            const fontSize = isEmph
              ? fitCaptionFontSize({
                  requestedPx: rawFontSize * emphasisStyle.scale,
                  frameWidth,
                  isVertical,
                  text: group.tokens.join(' '),
                  characterWidthFactor: cs.characterWidthFactor,
                  safeWidthRatio: cs.safeWidthRatio,
                })
              : baseFontSize
            return (
              <React.Fragment key={gi}>
                <span style={{
                  whiteSpace: isEmph ? 'nowrap' : undefined,
                  display: 'inline',
                }}>
                  {group.tokens.map((tok, ti) => {
                    const { leading, bare, trailing } = splitToken(tok)
                    const tokenText = `${leading}${bare}${trailing}`
                    return (
                      <React.Fragment key={ti}>
                        <span style={{
                          fontSize: `${fontSize}px`,
                          fontWeight: isEmph && emphasisStyle.bold && cs.fontWeight >= 700 ? 900 : cs.fontWeight,
                          color: isEmph ? emphasisStyle.color : baseFontColor,
                          textTransform: (isEmph && emphasisStyle.uppercase) || cs.uppercase ? 'uppercase' : 'none',
                          transition: 'font-size 0.12s ease, color 0.12s ease',
                          display: 'inline',
                          textShadow: isEmph && emphasisStyle.shadow && !keepsLayeredDepth
                            ? emphasisStyle.shadow
                            : undefined,
                        }}>
                          {keepsLayeredDepth ? (
                            <MaterialCaptionText
                              presentation={cs.presentation as MaterialPresentation}
                              text={tokenText}
                              emphasized={isEmph}
                              seed={gi + ti}
                            />
                          ) : tokenText}
                        </span>
                        {ti < group.tokens.length - 1 && ' '}
                      </React.Fragment>
                    )
                  })}
                </span>
                {gi < groups.length - 1 && ' '}
              </React.Fragment>
            )
          })}
        </div>
      </div>
    </div>
  )
}
