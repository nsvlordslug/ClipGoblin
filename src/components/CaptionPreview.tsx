import React, { useCallback, useMemo, useRef, useState, useEffect } from 'react'
import type { CaptionToken } from '../lib/captionEmphasis'
import { EMPHASIS_STYLES } from '../lib/captionEmphasis'
import type { CaptionStyle } from '../lib/editTypes'
import { clampCaptionFontScale, fitCaptionFontSize } from '../lib/captionSizing'
import { findActiveSegment } from '../lib/subtitleUtils'
import type { SubtitleSegment } from '../lib/subtitleUtils'

interface Props {
  segments: SubtitleSegment[]
  emphasisTokens?: CaptionToken[]
  captionStyle: CaptionStyle
  fontScale?: number
  currentTime: number
  trimStart?: number
  trimEnd?: number
  position: 'top' | 'center' | 'bottom'
  yPercent?: number
  emphasisEnabled: boolean
  outputWidth?: number
}

const DESIGN_WIDTH = 1080

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
  const activeSegment = useMemo(
    () => findActiveSegment(visibleSegments, currentTime),
    [visibleSegments, currentTime],
  )

  // ── Layout computation ──
  const ar = frameWidth / Math.max(frameHeight, 1)
  const isVertical = ar < 0.7
  const isLandscape = ar > 1.5
  const isCardboard = cs.presentation === 'cardboard'
  const activeSegmentIndex = activeSegment ? visibleSegments.indexOf(activeSegment) : -1
  const previousSegmentText = activeSegmentIndex > 0
    ? visibleSegments[activeSegmentIndex - 1].text.trim()
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
  const rawFontSize = cs.fontSize * scale * safeFontScale
  const baseFontSize = fitCaptionFontSize({
    requestedPx: rawFontSize,
    frameWidth,
    isVertical,
    text: activeSegment?.text || '',
    characterWidthFactor: cs.characterWidthFactor,
    safeWidthRatio: cs.safeWidthRatio,
  })

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

  if (!activeSegment) return null

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
    overflow: 'hidden',
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
                    return (
                      <React.Fragment key={ti}>
                        <span style={{
                          fontSize: `${fontSize}px`,
                          fontWeight: isEmph && emphasisStyle.bold && cs.fontWeight >= 700 ? 900 : cs.fontWeight,
                          color: isEmph ? emphasisStyle.color : baseFontColor,
                          textTransform: (isEmph && emphasisStyle.uppercase) || cs.uppercase ? 'uppercase' : 'none',
                          transition: 'font-size 0.12s ease, color 0.12s ease',
                          display: 'inline',
                          textShadow: isEmph && emphasisStyle.shadow ? emphasisStyle.shadow : undefined,
                        }}>
                          {leading}{bare}{trailing}
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
