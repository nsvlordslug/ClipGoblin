// ── Caption Emphasis Engine ──
// Detects words and phrases that should be visually emphasized in captions.
// Works with SRT-formatted caption text (word-level timing).

// ── Types ──

export interface CaptionToken {
  text: string
  startTime: number    // seconds relative to clip start
  endTime: number
  emphasized: boolean
  emphasisType?: 'keyword' | 'reaction' | 'urgency' | 'payoff' | 'punchline'
}

export interface EmphasisStyle {
  color: string         // override text color for emphasized words
  scale: number         // font size multiplier (e.g. 1.3 = 30% larger)
  bold: boolean
  uppercase: boolean
  // TODO: animation — pop-in, shake, pulse scaffold
}

// ── Emphasis presets per caption style ──

export const EMPHASIS_STYLES: Record<string, EmphasisStyle> = {
  clean:       { color: '#FFD700', scale: 1.25, bold: true, uppercase: false },
  'bold-white': { color: '#FF4444', scale: 1.3,  bold: true, uppercase: true },
  boxed:       { color: '#00FF88', scale: 1.2,  bold: true, uppercase: false },
  neon:        { color: '#FFFFFF', scale: 1.3,  bold: true, uppercase: true },
  minimal:     { color: '#8B5CF6', scale: 1.15, bold: true, uppercase: false },
  fire:        { color: '#FFFF00', scale: 1.35, bold: true, uppercase: true },
}

// ── Detection dictionaries ──
// Each category has phrases sorted by strength (strongest first).

const REACTION_PHRASES = [
  'oh my god', 'oh my gosh', 'what the hell', 'what the fuck', 'holy shit',
  'holy crap', 'are you kidding', 'are you serious', 'i cant believe',
  'no way', 'no freaking way',
]

const URGENCY_PHRASES = [
  'behind you', 'hes right there', 'watch out', 'look out',
  'run run', 'go go go', 'go go', 'get out', 'move move',
  'hes coming', 'theyre here', 'right there',
  'hurry', 'help', 'run',
]

const PAYOFF_PHRASES = [
  'lets go', 'lets goooo', 'yes yes yes', 'we did it',
  'i got out', 'i made it', 'i survived', 'clutch',
  'gg', 'thats it', 'easy',
]

const PUNCHLINE_PHRASES = [
  'im dead', 'im done', 'i cant', 'bruh', 'bro',
  'dude what', 'wait what', 'huh',
]

const KEYWORD_WORDS = new Set([
  'no', 'yes', 'wait', 'what', 'run', 'go', 'help',
  'stop', 'please', 'why', 'how', 'come', 'kill',
  'dead', 'die', 'escape', 'save', 'clutch',
])

// Repeated words are emphatic (e.g. "no no no", "go go go")
const MIN_REPEAT_COUNT = 2

// ── Core detection ──

/** Parse SRT text into timed tokens, then detect emphasis. */
export function analyzeEmphasis(
  srtText: string,
  clipDuration: number,
): CaptionToken[] {
  const tokens = parseSrtToTokens(srtText)
  return detectEmphasis(tokens, clipDuration)
}

/** Parse SRT formatted text into CaptionToken[] */
function parseSrtToTokens(srt: string): CaptionToken[] {
  const tokens: CaptionToken[] = []
  const blocks = srt.trim().split(/\n\n+/)

  for (const block of blocks) {
    const lines = block.trim().split('\n')
    if (lines.length < 3) continue

    // Line 1: index
    // Line 2: timestamp (00:00:01,000 --> 00:00:03,500)
    // Line 3+: text
    const timeLine = lines[1]
    const match = timeLine.match(/(\d{2}:\d{2}:\d{2},\d{3})\s*-->\s*(\d{2}:\d{2}:\d{2},\d{3})/)
    if (!match) continue

    const startTime = parseSrtTime(match[1])
    const endTime = parseSrtTime(match[2])
    const text = lines.slice(2).join(' ').trim()

    if (!text) continue

    // Split block text into individual words with estimated timing
    const words = text.split(/\s+/)
    const wordDuration = (endTime - startTime) / words.length

    for (let i = 0; i < words.length; i++) {
      tokens.push({
        text: words[i],
        startTime: startTime + i * wordDuration,
        endTime: startTime + (i + 1) * wordDuration,
        emphasized: false,
      })
    }
  }

  return tokens
}

function parseSrtTime(ts: string): number {
  const [h, m, rest] = ts.split(':')
  const [s, ms] = rest.split(',')
  return parseInt(h) * 3600 + parseInt(m) * 60 + parseInt(s) + parseInt(ms) / 1000
}

/** Run emphasis detection on a list of tokens. */
function detectEmphasis(tokens: CaptionToken[], clipDuration: number): CaptionToken[] {
  if (tokens.length === 0) return tokens

  // TODO: use full text for sentence-level emphasis patterns
  // const fullText = tokens.map(t => t.text).join(' ').toLowerCase()

  // Phase 1: Multi-word phrase detection
  const phraseHits = new Map<number, { type: CaptionToken['emphasisType']; length: number }>()

  for (const phrase of REACTION_PHRASES) {
    markPhrase(tokens, phrase, 'reaction', phraseHits)
  }
  for (const phrase of URGENCY_PHRASES) {
    markPhrase(tokens, phrase, 'urgency', phraseHits)
  }
  for (const phrase of PAYOFF_PHRASES) {
    markPhrase(tokens, phrase, 'payoff', phraseHits)
  }
  for (const phrase of PUNCHLINE_PHRASES) {
    markPhrase(tokens, phrase, 'punchline', phraseHits)
  }

  // Apply phrase hits
  for (const [idx, hit] of phraseHits) {
    for (let i = idx; i < Math.min(idx + hit.length, tokens.length); i++) {
      tokens[i].emphasized = true
      tokens[i].emphasisType = hit.type
    }
  }

  // Phase 2: Single keyword detection (only if not already emphasized)
  for (let i = 0; i < tokens.length; i++) {
    if (tokens[i].emphasized) continue
    const word = tokens[i].text.toLowerCase().replace(/[^a-z]/g, '')
    if (KEYWORD_WORDS.has(word) && word.length >= 2) {
      tokens[i].emphasized = true
      tokens[i].emphasisType = 'keyword'
    }
  }

  // Phase 3: Repetition detection ("no no no", "go go go")
  for (let i = 0; i < tokens.length - 1; i++) {
    const word = tokens[i].text.toLowerCase().replace(/[^a-z]/g, '')
    if (word.length < 2) continue
    let repeatCount = 1
    let j = i + 1
    while (j < tokens.length && tokens[j].text.toLowerCase().replace(/[^a-z]/g, '') === word) {
      repeatCount++
      j++
    }
    if (repeatCount >= MIN_REPEAT_COUNT) {
      for (let k = i; k < j; k++) {
        tokens[k].emphasized = true
        tokens[k].emphasisType = tokens[k].emphasisType || 'reaction'
      }
    }
  }

  // Phase 4: Payoff-zone boost — words in the last 20% of the clip get extra emphasis
  const payoffThreshold = clipDuration * 0.8
  for (const t of tokens) {
    if (t.startTime >= payoffThreshold && t.emphasisType === 'keyword') {
      t.emphasisType = 'payoff'
    }
  }

  return tokens
}

/** Find a phrase in the token stream and mark indices. */
function markPhrase(
  tokens: CaptionToken[],
  phrase: string,
  type: CaptionToken['emphasisType'],
  hits: Map<number, { type: CaptionToken['emphasisType']; length: number }>,
) {
  const phraseWords = phrase.split(/\s+/)
  const len = phraseWords.length

  for (let i = 0; i <= tokens.length - len; i++) {
    let match = true
    for (let j = 0; j < len; j++) {
      const tokenWord = tokens[i + j].text.toLowerCase().replace(/[^a-z0-9]/g, '')
      if (tokenWord !== phraseWords[j]) { match = false; break }
    }
    if (match) {
      // Only overwrite if we have a longer or equally long match
      const existing = hits.get(i)
      if (!existing || len >= existing.length) {
        hits.set(i, { type, length: len })
      }
    }
  }
}

// ── Rendering helpers ──

/** Get the emphasis style for a token given the current caption style preset. */
export function getEmphasisStyle(
  token: CaptionToken,
  presetId: string,
): EmphasisStyle | null {
  if (!token.emphasized) return null
  return EMPHASIS_STYLES[presetId] || EMPHASIS_STYLES.clean
}

/** Generate summary of emphasized phrases for UI display. */
export function getEmphasisSummary(tokens: CaptionToken[]): {
  type: string
  text: string
  time: number
}[] {
  const groups: { type: string; text: string; time: number }[] = []
  let currentGroup: CaptionToken[] = []

  for (const t of tokens) {
    if (t.emphasized) {
      if (currentGroup.length > 0 && currentGroup[currentGroup.length - 1].emphasisType !== t.emphasisType) {
        // Different type — flush
        groups.push({
          type: currentGroup[0].emphasisType || 'keyword',
          text: currentGroup.map(c => c.text).join(' '),
          time: currentGroup[0].startTime,
        })
        currentGroup = [t]
      } else {
        currentGroup.push(t)
      }
    } else {
      if (currentGroup.length > 0) {
        groups.push({
          type: currentGroup[0].emphasisType || 'keyword',
          text: currentGroup.map(c => c.text).join(' '),
          time: currentGroup[0].startTime,
        })
        currentGroup = []
      }
    }
  }
  if (currentGroup.length > 0) {
    groups.push({
      type: currentGroup[0].emphasisType || 'keyword',
      text: currentGroup.map(c => c.text).join(' '),
      time: currentGroup[0].startTime,
    })
  }

  return groups
}
