// ═══════════════════════════════════════════════════════════════
//  UI formatting helpers for the results page.
//  Pure functions, no dependencies, no state.
// ═══════════════════════════════════════════════════════════════

export function formatConfidence(score: number): { text: string; color: string } {
  if (score >= 0.85) return { text: 'High confidence', color: 'text-green-400' }
  if (score >= 0.70) return { text: 'Strong pick',    color: 'text-green-400' }
  if (score >= 0.55) return { text: 'Worth reviewing', color: 'text-yellow-400' }
  return                    { text: 'Low signal',      color: 'text-surface-500' }
}

export function formatDuration(seconds: number): string {
  const m = Math.floor(seconds / 60)
  const s = Math.floor(seconds % 60)
  return `${m}:${s.toString().padStart(2, '0')}`
}

const TAG_LABELS: Record<string, string> = {
  shock: 'Shock',
  hype: 'Hype',
  reaction: 'Reaction',
  fight: 'Action',
  frustration: 'Tilt',
  panic: 'Panic',
  celebration: 'Celebration',
  disbelief: 'Disbelief',
  ambush: 'Jumpscare',
  jumpscare: 'Jumpscare',
  scream: 'Screaming',
  explosion: 'Explosive',
  burst: 'Outburst',
  emotional: 'Emotional',
  rapid_cuts: 'Fast editing',
  keyword: 'Clear payoff',
}

export function formatTag(tag: string): string {
  return TAG_LABELS[tag] ?? tag
}

export function formatDimensionLabel(value: number): string {
  if (value >= 0.70) return 'Strong'
  if (value >= 0.50) return 'Solid'
  if (value >= 0.30) return 'Moderate'
  if (value >= 0.15) return 'Weak'
  return 'Minimal'
}

export function formatQuickVerdict(score: number, signalCount: number): string {
  if (score >= 0.85 && signalCount >= 3) return `${signalCount} signals, all above threshold`
  if (score >= 0.85)                     return 'High signal strength'
  if (score >= 0.70 && signalCount >= 3) return `${signalCount} signals confirm`
  if (score >= 0.70)                     return 'Clear detection'
  if (score >= 0.55 && signalCount >= 2) return `${signalCount} signals active`
  if (score >= 0.55)                     return 'Above threshold'
  return                                        'Below threshold — review manually'
}

export function formatScoreRationale(
  confidence: number,
  report: { dimensions: { hook_strength: number; emotional_intensity: number; context_clarity: number; visual_activity: number; speech_punch: number }; raw_signals: { audio_score: number; speech_score: number; scene_score: number; vision_score: number | null } },
): string {
  const pct = Math.round(confidence * 100)
  const d = report.dimensions
  const pairs = [
    { name: 'hook strength', val: d.hook_strength },
    { name: 'emotional intensity', val: d.emotional_intensity },
    { name: 'context clarity', val: d.context_clarity },
    { name: 'visual activity', val: d.visual_activity },
    { name: 'speech punch', val: d.speech_punch },
  ].sort((a, b) => b.val - a.val)
  const top = pairs[0]
  const r = report.raw_signals
  const strongest = [
    { name: 'audio', val: r.audio_score },
    { name: 'speech', val: r.speech_score },
    { name: 'scene', val: r.scene_score },
    ...(r.vision_score != null ? [{ name: 'vision', val: r.vision_score }] : []),
  ].sort((a, b) => b.val - a.val)[0]
  const count = [r.audio_score, r.speech_score, r.scene_score, r.vision_score ?? 0]
    .filter(s => s > 0).length

  return `Scored ${pct}% — ${count} signal${count !== 1 ? 's' : ''} active, strongest: ${strongest.name} (${Math.round(strongest.val * 100)}%). Top dimension: ${top.name} (${Math.round(top.val * 100)}%)`
}
