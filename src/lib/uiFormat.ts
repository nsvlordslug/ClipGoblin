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

/** "2h 14m 22s" countdown from an ISO time. Returns '0:00:00' when past. */
export function fmtCountdown(targetISO: string): string {
  const diff = new Date(targetISO).getTime() - Date.now()
  if (diff <= 0) return '0:00:00'
  const h = Math.floor(diff / 3_600_000)
  const m = Math.floor((diff % 3_600_000) / 60_000)
  const s = Math.floor((diff % 60_000) / 1000)
  return `${h}h ${String(m).padStart(2, '0')}m ${String(s).padStart(2, '0')}s`
}

/** "3:12:04" or "48m" style VOD-length formatter. */
export function formatVodLength(seconds: number): string {
  const h = Math.floor(seconds / 3600)
  const m = Math.floor((seconds % 3600) / 60)
  const s = Math.floor(seconds % 60)
  if (h > 0) return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`
  return `${m}:${String(s).padStart(2, '0')}`
}

/** Compress legacy virality_score (0-1) to calibrated confidence (0-1). */
export function legacyToConfidence(virality: number): number {
  const n = Math.max(0, Math.min(virality * 0.85 - 0.10, 0.99))
  const anchors: [number, number][] = [
    [0.00, 0.00], [0.25, 0.25], [0.40, 0.55], [0.50, 0.65],
    [0.60, 0.77], [0.70, 0.84], [0.80, 0.89], [0.90, 0.93],
  ]
  if (n >= 0.90) return Math.min(0.93 + (n - 0.90) * 0.20, 0.95)
  for (let i = 1; i < anchors.length; i++) {
    if (n <= anchors[i][0]) {
      const [x0, y0] = anchors[i - 1]
      const [x1, y1] = anchors[i]
      return y0 + ((n - x0) / (x1 - x0)) * (y1 - y0)
    }
  }
  return 0.95
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
