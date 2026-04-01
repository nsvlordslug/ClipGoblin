// ── ClipGoblin Clip Naming Module ──
// Context-aware title generation from clip analysis signals.
// Every clip passes through generateClipTitle() before display.

// ── Types ──

export interface ClipContext {
  game?: string
  transcriptExcerpt?: string
  detectedKeywords?: string[]
  eventTags?: string[]
  emotionTags?: string[]
  payoffSummary?: string
  outcomeLabel?: string
  clipReason?: string
}

interface ScoredTitle {
  title: string
  score: number
  source: string
}

// ── Tag Vocabularies ──
// Maps raw tags/keywords to clean readable forms.

const EVENT_MAP: Record<string, string> = {
  kill: 'Kill', death: 'Death', clutch: 'Clutch Play', save: 'Save',
  escape: 'Escape', chase: 'Chase', fight: 'Fight', ambush: 'Ambush',
  snipe: 'Snipe', headshot: 'Headshot', combo: 'Combo', dodge: 'Dodge',
  block: 'Block', counter: 'Counter', gank: 'Gank', wipe: 'Team Wipe',
  ace: 'Ace', steal: 'Steal', grab: 'Grab', explosion: 'Explosion',
  crash: 'Crash', jumpscare: 'Jumpscare', scare: 'Scare',
  surprise: 'Surprise', reveal: 'Reveal', hook: 'Hook',
  generator: 'Generator', repair: 'Repair', vault: 'Vault',
  pallet: 'Pallet', window: 'Window Vault', hatch: 'Hatch',
  'gen-rush': 'Gen Rush', tunnel: 'Tunnel', camp: 'Camp',
  interrupt: 'Interrupt', down: 'Down', slug: 'Slug',
  heal: 'Heal', rescue: 'Rescue', trade: 'Trade',
  flashlight: 'Flashlight Save', 'dead-hard': 'Dead Hard',
  loop: 'Loop', mindgame: 'Mind Game', '360': '360',
  juke: 'Juke', bait: 'Bait', outplay: 'Outplay',
  miss: 'Missed Hit', whiff: 'Whiff',
}

const EMOTION_MAP: Record<string, string> = {
  rage: 'Rage', scream: 'Scream', panic: 'Panic', shock: 'Shock',
  hype: 'Hype', laugh: 'Laughter', cry: 'Tears', disbelief: 'Disbelief',
  excitement: 'Excitement', frustration: 'Frustration',
  confusion: 'Confusion', joy: 'Joy', fear: 'Fear', anger: 'Anger',
  celebration: 'Celebration', grief: 'Grief', relief: 'Relief',
  tilt: 'Tilt', salt: 'Salt', despair: 'Despair',
  surprise: 'Surprise', tension: 'Tension', dread: 'Dread',
}

const OUTCOME_MAP: Record<string, string> = {
  win: 'Win', loss: 'Loss', fail: 'Fail', success: 'Success',
  comeback: 'Comeback', choke: 'Choke', miss: 'Miss',
  punish: 'Punish', throw: 'Throw', carry: 'Carry',
  'close-call': 'Close Call', 'near-miss': 'Near Miss',
  lucky: 'Lucky', unlucky: 'Unlucky', clutch: 'Clutch',
  wipe: 'Wipe', escape: 'Escape', death: 'Death',
  survive: 'Survive', sacrifice: 'Sacrifice', trade: 'Trade',
  '4k': '4K', 'team-kill': 'Team Kill',
}

// Connectors for [Event] + [Outcome] patterns
const EVENT_OUTCOME_CONNECTORS = [
  'Leads to', 'Ends in', 'Results in', 'Becomes', 'Turns Into',
]

const EVENT_EMOTION_CONNECTORS = [
  'Triggers', 'Causes', 'Sparks', 'Brings',
]

// ── Banned words ──
// These are too vague to stand alone. Must pair with specific context.

const BANNED_STANDALONE = new Set([
  'action', 'gameplay', 'moment', 'stream', 'push', 'sequence',
  'content', 'highlight', 'play', 'situation', 'event', 'instance',
  'occurrence', 'scene', 'segment', 'bit', 'part', 'section',
  'momentum', 'stride', 'groove', 'opening', 'closing',
])

// Words that make a title emotion-only (must pair with an event)
const EMOTION_ONLY_WORDS = new Set([
  'excitement', 'burst', 'intense', 'raw', 'sudden', 'pure', 'total',
  'moment', 'hype', 'energy', 'shock', 'surprise',
])

// Concrete event/action words that ground a title
const EVENT_WORDS = new Set([
  'chase', 'escape', 'fight', 'ambush', 'kill', 'death', 'save', 'hook',
  'jumpscare', 'scream', 'encounter', 'skirmish', 'interrupt', 'dodge',
  'snipe', 'headshot', 'combo', 'grab', 'steal', 'juke', 'outplay',
  'generator', 'repair', 'vault', 'down', 'rescue', 'counter', 'block',
  'bait', 'whiff', 'miss', 'crash', 'explosion', 'attack', 'hit',
  'confrontation', 'standoff', 'pursuit', 'sprint', 'rush',
])

// ── Viral transcript phrases (priority order) ──

const VIRAL_PHRASES = [
  // High-impact reactions
  'no way', 'oh my god', 'what the hell', 'what the fuck', 'holy shit',
  'are you kidding', 'are you serious', 'i cant believe',
  // Gameplay callouts
  'behind you', 'hes right there', 'watch out', 'run run', 'go go go',
  'get out', 'hes coming', 'theyre here',
  // Emotional peaks
  'lets go', 'lets goooo', 'yes yes yes', 'no no no', 'noooo',
  'i cant', 'im dead', 'im done', 'thats it',
  // Surprise/disbelief
  'wait what', 'how did', 'did that just', 'did he just', 'did i just',
  'that was close', 'almost had', 'so close',
  // Casual/funny
  'bro', 'dude what', 'bruh',
]

// ── Utility ──

function capitalize(s: string): string {
  return s.replace(/\b\w/g, c => c.toUpperCase())
}

function cleanTag(tag: string): string {
  return tag.trim().toLowerCase().replace(/[-_]/g, ' ')
}

function pickRandom<T>(arr: T[]): T {
  return arr[Math.floor(Math.random() * arr.length)]
}

// ── Core: Extract structured concepts from raw tag strings ──

function classifyTags(rawTags: string[]): {
  events: string[]
  emotions: string[]
  outcomes: string[]
} {
  const events: string[] = []
  const emotions: string[] = []
  const outcomes: string[] = []

  for (const raw of rawTags) {
    const tag = cleanTag(raw)
    for (const [key, label] of Object.entries(EVENT_MAP)) {
      if (tag.includes(key)) { events.push(label); break }
    }
    for (const [key, label] of Object.entries(EMOTION_MAP)) {
      if (tag.includes(key)) { emotions.push(label); break }
    }
    for (const [key, label] of Object.entries(OUTCOME_MAP)) {
      if (tag.includes(key)) { outcomes.push(label); break }
    }
  }

  return {
    events: [...new Set(events)],
    emotions: [...new Set(emotions)],
    outcomes: [...new Set(outcomes)],
  }
}

// ── Core: Extract keywords from transcript text ──

function extractTranscriptKeywords(text: string): string[] {
  const lower = text.toLowerCase().replace(/[^a-z0-9\s']/g, '')
  const found: string[] = []

  for (const phrase of VIRAL_PHRASES) {
    if (lower.includes(phrase)) {
      found.push(capitalize(phrase))
    }
  }

  return found.slice(0, 4) // top 4 most impactful
}

// ── Title Builders ──
// Each builder produces a candidate title from specific signal combinations.

function buildEventOutcome(events: string[], outcomes: string[]): string | null {
  if (!events.length || !outcomes.length) return null
  const ev = events[0]
  const out = outcomes[0]
  if (ev === out) return null // avoid "Escape Leads to Escape"
  return `${ev} ${pickRandom(EVENT_OUTCOME_CONNECTORS)} ${out}`
}

function buildEventEmotion(events: string[], emotions: string[]): string | null {
  if (!events.length || !emotions.length) return null
  return `${events[0]} ${pickRandom(EVENT_EMOTION_CONNECTORS)} ${emotions[0]}`
}

function buildEmotionOutcome(emotions: string[], outcomes: string[]): string | null {
  if (!emotions.length || !outcomes.length) return null
  return `${emotions[0]} ${pickRandom(EVENT_OUTCOME_CONNECTORS)} ${outcomes[0]}`
}

function buildTranscriptEmotion(keywords: string[], emotions: string[]): string | null {
  if (!keywords.length || !emotions.length) return null
  return `"${keywords[0]}" During ${emotions[0]}`
}

function buildTranscriptEvent(keywords: string[], events: string[]): string | null {
  if (!keywords.length || !events.length) return null
  return `${events[0]} Makes Them Yell "${keywords[0]}"`
}

function buildPayoffTitle(payoff: string): string | null {
  if (!payoff || payoff.length < 5) return null
  const trimmed = payoff.length > 55 ? payoff.slice(0, 52) + '...' : payoff
  return trimmed
}

function buildReasonTitle(reason: string, events: string[], emotions: string[]): string | null {
  if (!reason || reason.length < 5) return null
  // Try to enrich the reason with a tag
  if (events.length) return `${events[0]} — ${capitalize(reason.slice(0, 40))}`
  if (emotions.length) return `${emotions[0]} — ${capitalize(reason.slice(0, 40))}`
  return capitalize(reason.slice(0, 55))
}

function buildSingleSignal(events: string[], emotions: string[], outcomes: string[]): string | null {
  if (events.length >= 2) return `${events[0]} Into ${events[1]}`
  if (events.length && outcomes.length) return buildEventOutcome(events, outcomes)
  if (events.length && emotions.length) return `${events[0]} ${pickRandom(EVENT_EMOTION_CONNECTORS)} ${emotions[0]}`
  if (events.length) return `Unexpected ${events[0]}`
  // Emotion-only is NOT allowed — return null so we fall through
  if (outcomes.length) return `The ${outcomes[0]}`
  return null
}

// ── Title Scoring ──

function scoreTitle(title: string): number {
  let score = 50 // baseline

  const words = title.split(/\s+/)
  const wordCount = words.length

  // Length: ideal 4-8 words
  if (wordCount >= 4 && wordCount <= 8) score += 15
  else if (wordCount >= 3 && wordCount <= 10) score += 8
  else if (wordCount < 3 || wordCount > 12) score -= 10

  // Specificity: contains a mapped event/emotion/outcome word
  const lower = title.toLowerCase()
  const allLabels = [
    ...Object.values(EVENT_MAP),
    ...Object.values(EMOTION_MAP),
    ...Object.values(OUTCOME_MAP),
  ]
  const specificWords = allLabels.filter(l => lower.includes(l.toLowerCase()))
  score += Math.min(specificWords.length * 8, 24) // up to +24 for specificity

  // Emotional strength: has emotion or reaction word
  const emotionLabels = Object.values(EMOTION_MAP)
  if (emotionLabels.some(e => lower.includes(e.toLowerCase()))) score += 10

  // Event+Payoff structure: has a connector word
  const connectors = [...EVENT_OUTCOME_CONNECTORS, ...EVENT_EMOTION_CONNECTORS]
  if (connectors.some(c => lower.includes(c.toLowerCase()))) score += 12

  // Penalize banned standalone words
  for (const word of words) {
    if (BANNED_STANDALONE.has(word.toLowerCase()) && wordCount <= 3) {
      score -= 15
    }
  }

  // Penalize timestamps
  if (/\d+:\d+/.test(title)) score -= 20

  // Penalize quotes wrapping entire title
  if (title.startsWith('"') && title.endsWith('"') && wordCount <= 2) score -= 10

  // Bonus for transcript quotes (shows real content)
  if (title.includes('"') && !title.startsWith('"')) score += 5

  return Math.max(0, Math.min(100, score))
}

// ── Cleaning ──

function cleanTitle(title: string): string {
  let t = title.trim()

  // Remove trailing timestamps like "(12:34)"
  t = t.replace(/\s*\(\d+:\d+\)\s*$/, '')

  // Remove double spaces
  t = t.replace(/\s{2,}/g, ' ')

  // Enforce max ~60 chars
  if (t.length > 60) {
    t = t.slice(0, 57).replace(/\s+\S*$/, '') + '...'
  }

  // Capitalize first letter
  if (t.length > 0) {
    t = t[0].toUpperCase() + t.slice(1)
  }

  return t
}

function isBannedTitle(title: string): boolean {
  const lower = title.toLowerCase()
  const bannedPatterns = [
    /^(early|late|mid|peak|final)\s+(stream|game|play|push|action)/,
    /^(opening|closing|building|hitting)\s+(moments?|stride|momentum|groove)/,
    /^(warming up|getting into it|second half|down to the wire|grand finale)/,
    /^stream (clip|highlight|moment)/,
    /^(untitled|highlight|gameplay)\s*$/,
    /^(burst of|pure |raw |intense |sudden |total )(excitement|shock|hype|energy)/,
  ]
  return bannedPatterns.some(p => p.test(lower)) || isEmotionOnly(lower)
}

/** Check if a title contains at least one concrete event word. */
export function isValidTitle(title: string): boolean {
  const lower = title.toLowerCase()
  return !isEmotionOnly(lower) && hasEventWord(lower)
}

function hasEventWord(lower: string): boolean {
  for (const w of EVENT_WORDS) {
    if (lower.includes(w)) return true
  }
  return false
}

function isEmotionOnly(lower: string): boolean {
  const words = lower.split(/\s+/).filter(w => w.length > 2 &&
    !['the', 'of', 'in', 'at', 'and', 'to', 'a', 'an', 'is'].includes(w))
  if (words.length === 0) return true
  return words.every(w => EMOTION_ONLY_WORDS.has(w))
}

// ── Main Entry Point ──

export function generateClipTitle(ctx: ClipContext): string {
  // Parse raw tags into structured concepts
  const allTags = [
    ...(ctx.eventTags || []),
    ...(ctx.emotionTags || []),
    ...(ctx.detectedKeywords || []),
  ]
  const { events, emotions, outcomes } = classifyTags(allTags)

  // Extract transcript keywords
  const transcriptKeywords = ctx.transcriptExcerpt
    ? extractTranscriptKeywords(ctx.transcriptExcerpt)
    : []

  // Generate candidate titles in priority order
  const candidates: ScoredTitle[] = []

  function addCandidate(title: string | null, source: string, bonus = 0) {
    if (!title) return
    const cleaned = cleanTitle(title)
    if (isBannedTitle(cleaned)) return
    if (cleaned.length < 5) return
    candidates.push({
      title: cleaned,
      score: scoreTitle(cleaned) + bonus,
      source,
    })
  }

  // Priority 1: payoffSummary (direct description of what happened)
  addCandidate(buildPayoffTitle(ctx.payoffSummary || ''), 'payoff', 20)

  // Priority 2: eventTags + outcomeLabel
  if (ctx.outcomeLabel) {
    const outcomeTags = classifyTags([ctx.outcomeLabel])
    addCandidate(
      buildEventOutcome(events, [...outcomes, ...outcomeTags.outcomes]),
      'event+outcome', 15
    )
  }
  addCandidate(buildEventOutcome(events, outcomes), 'event+outcome', 15)

  // Priority 3: emotionTags + eventTags
  addCandidate(buildEventEmotion(events, emotions), 'event+emotion', 12)
  addCandidate(buildEmotionOutcome(emotions, outcomes), 'emotion+outcome', 10)

  // Priority 4: transcript keywords combined with tags
  addCandidate(buildTranscriptEmotion(transcriptKeywords, emotions), 'transcript+emotion', 8)
  addCandidate(buildTranscriptEvent(transcriptKeywords, events), 'transcript+event', 8)

  // Priority 5: clipReason enriched with tags
  addCandidate(
    buildReasonTitle(ctx.clipReason || '', events, emotions),
    'reason', 5
  )

  // Priority 6: single-signal fallback
  addCandidate(buildSingleSignal(events, emotions, outcomes), 'single-signal', 0)

  // Priority 7: game context + strongest signal
  if (ctx.game && events.length) {
    addCandidate(`${events[0]} in ${ctx.game}`, 'game+event', -5)
  }

  // Select highest-scoring candidate that contains an event word
  if (candidates.length > 0) {
    candidates.sort((a, b) => b.score - a.score)
    // Prefer candidates with event words
    const eventCandidate = candidates.find(c => isValidTitle(c.title))
    if (eventCandidate) return eventCandidate.title
    // If no event-grounded candidate, take best anyway
    return candidates[0].title
  }

  // Absolute last resort — should rarely happen
  if (ctx.clipReason) return capitalize(ctx.clipReason.slice(0, 55))
  return 'Untitled Clip'
}

// ── Build ClipContext from backend data ──
// Bridges the gap between what the backend stores and what the title system needs.

export function buildClipContext(
  clip: { title?: string },
  highlight?: {
    description?: string
    tags?: string | string[]
    transcript_snippet?: string
    virality_score?: number
    audio_score?: number
    chat_score?: number
  },
  _vod?: { title?: string },
): ClipContext {
  // Parse tags
  const rawTags: string[] = Array.isArray(highlight?.tags)
    ? highlight.tags
    : typeof highlight?.tags === 'string'
      ? highlight.tags.split(',').map(t => t.trim()).filter(Boolean)
      : []

  const classified = classifyTags(rawTags)

  return {
    eventTags: rawTags.filter(t => {
      const lower = cleanTag(t)
      return Object.keys(EVENT_MAP).some(k => lower.includes(k))
    }),
    emotionTags: rawTags.filter(t => {
      const lower = cleanTag(t)
      return Object.keys(EMOTION_MAP).some(k => lower.includes(k))
    }),
    detectedKeywords: rawTags,
    transcriptExcerpt: highlight?.transcript_snippet || undefined,
    payoffSummary: highlight?.description || undefined,
    outcomeLabel: classified.outcomes[0] || undefined,
    clipReason: clip.title || undefined,
    game: undefined, // could be extracted from vod title in the future
  }
}

// ── Enhance an existing title ──
// Takes a potentially generic title and tries to improve it using highlight data.

export function enhanceClipTitle(
  currentTitle: string,
  highlight?: {
    description?: string
    tags?: string | string[]
    transcript_snippet?: string
  },
  vod?: { title?: string },
): string {
  // If the current title is already good (not banned, scores well), keep it
  if (!isBannedTitle(currentTitle) && scoreTitle(currentTitle) >= 60) {
    return currentTitle
  }

  // Build context and try to generate a better title
  const ctx = buildClipContext({ title: currentTitle }, highlight, vod)
  const generated = generateClipTitle(ctx)

  // Only replace if the generated title is actually better
  if (generated === 'Untitled Clip') return currentTitle
  if (scoreTitle(generated) <= scoreTitle(currentTitle)) return currentTitle

  return generated
}
