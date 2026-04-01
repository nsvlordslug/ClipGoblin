// ═══════════════════════════════════════════════════════════════
//  Types matching the Rust backend's pipeline output.
//  Every field maps 1:1 to a serde-serialized struct.
// ═══════════════════════════════════════════════════════════════

export type SignalType = 'audio' | 'transcript' | 'scene_change' | 'vision'

export interface ClipScoreBreakdown {
  audio_score: number
  speech_score: number
  scene_score: number
  vision_score: number | null
}

export interface DimensionScores {
  hook_strength: number
  emotional_intensity: number
  context_clarity: number
  visual_activity: number
  speech_punch: number
}

export interface ScoreFactor {
  label: string
  value: number
}

export interface ScoreReport {
  raw_signals: ClipScoreBreakdown
  dimensions: DimensionScores
  dimension_weighted: number
  bonuses: ScoreFactor[]
  bonus_total: number
  penalties: ScoreFactor[]
  penalty_total: number
  rank_score: number
  confidence: number
  explanation: string
  key_dimensions: string[]
}

export interface CaptionVariant {
  mode: string
  label: string
  text: string
}

export interface PostCaptions {
  captions: CaptionVariant[]
  hashtags: string[]
  source: 'free' | 'llm'
  casual: string
  funny: string
  hype: string
}

export interface CandidateClip {
  id: string
  start_time: number
  end_time: number
  confidence_score: number
  score_breakdown: ClipScoreBreakdown
  score_report?: ScoreReport
  title?: string
  summary?: string
  hook?: string
  preview_thumbnail_path?: string
  transcript_excerpt?: string
  signal_sources: SignalType[]
  tags: string[]
  fingerprint: string
  rejection_reason?: string
  event_summary?: string
  post_captions?: PostCaptions
}

export interface RankedClip {
  rank: number
  clip: CandidateClip
}

export interface AnalysisResult {
  clips: CandidateClip[]
  ranked: RankedClip[]
  warnings: string[]
  signals_used: string[]
  total_signals: number
}

// ═══════════════════════════════════════════════════════════════
//  UI display helpers — computed from the backend data.
// ═══════════════════════════════════════════════════════════════

export interface ConfidenceLabel {
  text: string
  emoji: string
  color: string
}

export function confidenceLabel(score: number): ConfidenceLabel {
  if (score >= 0.85) return { text: 'High confidence', emoji: '\u{1F7E2}', color: 'text-green-400' }
  if (score >= 0.70) return { text: 'Strong pick',     emoji: '\u{1F7E2}', color: 'text-green-400' }
  if (score >= 0.55) return { text: 'Worth reviewing',  emoji: '\u{1F44D}', color: 'text-emerald-300' }
  return                    { text: 'Low signal',        emoji: '\u{1F914}', color: 'text-surface-400' }
}

export function quickVerdict(clip: CandidateClip): string {
  const s = clip.confidence_score
  const n = clip.signal_sources.length
  if (s >= 0.85 && n >= 3) return `${n} signals, all above threshold`
  if (s >= 0.85)           return 'High signal strength'
  if (s >= 0.70 && n >= 3) return `${n} signals confirm`
  if (s >= 0.70)           return 'Clear detection'
  if (s >= 0.55 && n >= 2) return `${n} signals active`
  if (s >= 0.55)           return 'Above threshold'
  return                          'Below threshold \u2014 review manually'
}

export function dimensionLabel(value: number): string {
  if (value >= 0.70) return 'Strong'
  if (value >= 0.50) return 'Solid'
  if (value >= 0.30) return 'Moderate'
  if (value >= 0.15) return 'Weak'
  return 'Minimal'
}

export function formatDuration(seconds: number): string {
  const m = Math.floor(seconds / 60)
  const s = Math.floor(seconds % 60)
  return `${m}:${s.toString().padStart(2, '0')}`
}

export function peakTimestamp(clip: CandidateClip): number {
  return clip.start_time + (clip.end_time - clip.start_time) * 0.4
}

export function heldBack(clip: CandidateClip): string[] {
  const issues: string[] = []
  const r = clip.score_report
  if (!r) return issues
  for (const p of r.penalties) issues.push(p.label)
  const d = r.dimensions
  if (d.hook_strength < 0.30) issues.push('Hook could be stronger')
  if (d.context_clarity < 0.30) issues.push('Context might confuse viewers')
  if (d.visual_activity < 0.20) issues.push('Limited visual action')
  if (d.speech_punch < 0.20) issues.push('Speech doesn\'t stand out')
  if (clip.signal_sources.length === 1) issues.push('Only one signal type detected')
  return issues
}

export const SIGNAL_DISPLAY: Record<SignalType, { icon: string; label: string }> = {
  audio:        { icon: 'Volume2',       label: 'Audio' },
  transcript:   { icon: 'MessageSquare', label: 'Speech' },
  scene_change: { icon: 'Clapperboard',  label: 'Scene' },
  vision:       { icon: 'Sparkles',      label: 'AI Vision' },
}

export const TAG_DISPLAY: Record<string, { emoji: string; label: string }> = {
  shock:        { emoji: '\u{1F631}', label: 'Shock' },
  hype:         { emoji: '\u{1F525}', label: 'Hype moment' },
  frustration:  { emoji: '\u{1F624}', label: 'Tilt' },
  panic:        { emoji: '\u{1F630}', label: 'Panic' },
  celebration:  { emoji: '\u{1F389}', label: 'Celebration' },
  disbelief:    { emoji: '\u{1F92F}', label: 'Disbelief' },
  emotional:    { emoji: '\u{1F494}', label: 'Emotional' },
  reaction:     { emoji: '\u26A1',    label: 'Reaction' },
  fight:        { emoji: '\u2694\uFE0F', label: 'Action' },
  ambush:       { emoji: '\u{1F47B}', label: 'Jumpscare' },
  jumpscare:    { emoji: '\u{1F47B}', label: 'Jumpscare' },
  scream:       { emoji: '\u{1F4E2}', label: 'Screaming' },
  explosion:    { emoji: '\u{1F4A5}', label: 'Explosive' },
  burst:        { emoji: '\u{1F4AC}', label: 'Outburst' },
  rapid_cuts:   { emoji: '\u{1F3AC}', label: 'Fast editing' },
  sustained_motion: { emoji: '\u{1F3C3}', label: 'Extended action' },
  keyword:      { emoji: '\u{1F3AF}', label: 'Clear payoff' },
  repetition:   { emoji: '\u{1F501}', label: 'Repeated hype' },
  urgency:      { emoji: '\u23F0',    label: 'Urgent' },
}

export const DIMENSION_UI: Record<keyof DimensionScores, { label: string; color: string }> = {
  hook_strength:      { label: 'Hook',    color: 'bg-violet-500' },
  emotional_intensity:{ label: 'Emotion', color: 'bg-rose-500' },
  context_clarity:    { label: 'Clarity', color: 'bg-sky-500' },
  visual_activity:    { label: 'Visual',  color: 'bg-amber-500' },
  speech_punch:       { label: 'Speech',  color: 'bg-emerald-500' },
}

export function stageMessage(progress: number, hasVision: boolean) {
  const stages = [
    { name: 'Listening for reactions',    done: progress >= 20 },
    { name: 'Reading the transcript',     done: progress >= 30 },
    { name: 'Scanning for scene changes', done: progress >= 40 },
    ...(hasVision ? [{ name: 'AI reviewing key frames', done: progress >= 70 }] : []),
    { name: 'Ranking highlights',         done: progress >= 80 },
    { name: 'Generating previews',        done: progress >= 95 },
  ]
  const current = stages.find(s => !s.done)?.name ?? 'Finishing up...'
  return { current, stages }
}
