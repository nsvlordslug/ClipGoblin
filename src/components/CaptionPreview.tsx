import React, { useCallback, useMemo, useRef, useState, useEffect } from 'react'
import type { CaptionToken } from '../lib/captionEmphasis'
import { EMPHASIS_STYLES } from '../lib/captionEmphasis'
import type { CaptionStyle } from '../lib/editTypes'
import type { SubtitleSegment } from '../lib/subtitleUtils'

interface Props {
  segments: SubtitleSegment[]
  emphasisTokens?: CaptionToken[]
  captionStyle: CaptionStyle
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
  trimStart, trimEnd, position, yPercent, emphasisEnabled, outputWidth: _ow,
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

  const activeSegment = useMemo(() => {
    for (const s of segments) {
      if (trimStart != null && trimEnd != null) {
        if (s.endTime <= trimStart || s.startTime >= trimEnd) continue
      }
      if (currentTime >= s.startTime - 0.05 && currentTime <= s.endTime + 0.05) return s
    }
    return null
  }, [segments, currentTime, trimStart, trimEnd])

  // ── Layout computation ──
  const ar = frameWidth / Math.max(frameHeight, 1)
  const isVertical = ar < 0.7
  const isLandscape = ar > 1.5

  // Scale: frame width relative to 1080 design, with format boost
  const baseScale = frameWidth / DESIGN_WIDTH
  const boost = isVertical ? 1.15 : isLandscape ? 0.85 : 1.0
  const scale = Math.max(0.15, Math.min(0.55, baseScale * boost))

  // Font size: cap to prevent overflow on narrow frames
  const maxFontPx = frameWidth * (isVertical ? 0.085 : 0.065) // max ~8.5% of frame width for vertical
  const rawFontSize = cs.fontSize * scale
  const baseFontSize = Math.min(rawFontSize, maxFontPx)

  // Safe margins: left/right padding inside the frame
  const safeMarginPx = Math.round(frameWidth * 0.05) // 5% each side
  const maxTextWidth = frameWidth - safeMarginPx * 2
  // Account for background padding in available text width
  const bgPad = cs.bgPadding > 0 ? cs.bgPadding * scale * 2 : 0
  const textAreaWidth = Math.max(50, maxTextWidth - bgPad)

  // Bottom safe zone: at least 6% from bottom edge
  const bottomSafe = Math.max(Math.round(frameHeight * 0.06), 10)

  // Position
  const useCustomY = yPercent != null
  let posTop: string | undefined
  let posBottom: string | undefined
  let transform: string | undefined

  if (useCustomY) {
    posTop = `${yPercent}%`
    if (position === 'center') transform = 'translateY(-50%)'
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
    : cs.shadow.replace(/(\d+)px/g, (_, n) => `${Math.max(1, Math.round(parseInt(n) * Math.min(scale, 0.4)))}px`)

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

  return (
    <div ref={containerRef}
      className="absolute left-0 right-0 flex justify-center pointer-events-none z-10"
      style={{ top: posTop, bottom: posBottom, transform }}>

      {/* Bounded subtitle container — all text stays inside this box */}
      <div style={{
        maxWidth: `${maxTextWidth}px`,
        maxHeight: `${Math.round(frameHeight * 0.35)}px`, // subtitle block can't exceed 35% of frame height
        width: `${maxTextWidth}px`,
        overflow: 'hidden',
        textAlign: 'center',
        background: cs.bgColor || undefined,
        padding: cs.bgPadding > 0
          ? `${Math.round(cs.bgPadding * scale * 0.5)}px ${Math.round(cs.bgPadding * scale * 0.8)}px`
          : `0 ${safeMarginPx * 0.3}px`,
        borderRadius: cs.bgRadius > 0 ? `${Math.round(cs.bgRadius * scale)}px` : undefined,
        boxSizing: 'border-box',
      }}>
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
          color: cs.fontColor,
          wordBreak: 'keep-all',
          overflowWrap: 'normal',
          whiteSpace: 'normal',
          WebkitFontSmoothing: 'antialiased',
        } as React.CSSProperties}>
          {/* ── Layer 3: render — one code path, style config is data only ── */}
          {groups.map((group, gi) => {
            const isEmph = group.emphasized
            const fontSize = isEmph
              ? Math.min(baseFontSize * emphasisStyle.scale, maxFontPx * 1.2)
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
                          fontWeight: isEmph ? 900 : cs.fontWeight,
                          color: isEmph ? emphasisStyle.color : cs.fontColor,
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
