import type { AnalysisResult, CandidateClip, ScoreReport } from '../types/analysis'

function report(
  dims: [number, number, number, number, number],
  explanation: string,
): ScoreReport {
  const [hook, emotion, clarity, visual, speech] = dims
  const weighted = hook * 0.30 + emotion * 0.28 + clarity * 0.14 + visual * 0.14 + speech * 0.14
  return {
    raw_signals: { audio_score: hook * 0.9, speech_score: speech * 0.95, scene_score: visual * 0.85, vision_score: null },
    dimensions: {
      hook_strength: hook,
      emotional_intensity: emotion,
      context_clarity: clarity,
      visual_activity: visual,
      speech_punch: speech,
    },
    dimension_weighted: weighted,
    bonuses: [
      { label: 'Multi-signal', value: 0.08 },
      { label: 'Good length', value: 0.04 },
    ],
    bonus_total: 0.12,
    penalties: [],
    penalty_total: 0,
    rank_score: weighted + 0.12,
    confidence: Math.min(weighted * 1.35 + 0.10, 1.0),
    explanation,
    key_dimensions: dims
      .map((v, i) => ({ v, label: ['Hook', 'Emotion', 'Clarity', 'Visual', 'Speech'][i] }))
      .sort((a, b) => b.v - a.v)
      .slice(0, 3)
      .map(d => `${d.label} (${d.v.toFixed(2)})`),
  }
}

function clip(
  id: string,
  start: number,
  end: number,
  title: string,
  hook: string,
  confidence: number,
  tags: string[],
  transcript: string,
  signals: ('audio' | 'transcript' | 'scene_change')[],
  dims: [number, number, number, number, number],
  explanation: string,
): CandidateClip {
  return {
    id,
    start_time: start,
    end_time: end,
    confidence_score: confidence,
    score_breakdown: {
      audio_score: dims[0] * 0.9,
      speech_score: dims[4] * 0.95,
      scene_score: dims[3] * 0.85,
      vision_score: null,
    },
    score_report: report(dims, explanation),
    title,
    hook,
    transcript_excerpt: transcript,
    signal_sources: signals,
    tags,
    fingerprint: tags.slice(0, 2).join('+'),
    preview_thumbnail_path: undefined,
  }
}

export const MOCK_RESULT: AnalysisResult = {
  clips: [
    clip(
      'clip-1', 118, 143,
      'Jumpscare Reaction',
      'Wait for it...',
      0.88,
      ['shock', 'reaction', 'scream'],
      'OH MY GOD WHAT WAS THAT!!!',
      ['audio', 'transcript', 'scene_change'],
      [0.72, 0.68, 0.45, 0.52, 0.70],
      'Loud reaction \u2014 3 signals agree',
    ),
    clip(
      'clip-2', 348, 375,
      'Clutch Play Celebration',
      'Let\'s go!',
      0.82,
      ['hype', 'celebration', 'fight'],
      'LET\'S GO let\'s go let\'s go!!!',
      ['audio', 'transcript', 'scene_change'],
      [0.65, 0.60, 0.50, 0.70, 0.62],
      'Dramatic speech \u2014 3 signals agree',
    ),
    clip(
      'clip-3', 496, 520,
      'Rage Quit Moment',
      'Not like this...',
      0.71,
      ['frustration', 'emotional', 'reaction'],
      'I\'m done. I\'m actually done. This is bullshit.',
      ['audio', 'transcript'],
      [0.55, 0.72, 0.60, 0.20, 0.75],
      'Dramatic speech \u2014 confirmed by 2 signals',
    ),
    clip(
      'clip-4', 720, 745,
      'Panic Chase',
      'Pure chaos',
      0.64,
      ['panic', 'reaction', 'fight'],
      'RUN RUN RUN oh god oh god',
      ['audio', 'scene_change'],
      [0.60, 0.55, 0.30, 0.65, 0.50],
      'Audio spike \u2014 confirmed by 2 signals',
    ),
    clip(
      'clip-5', 1050, 1072,
      'Disbelief at Snipe',
      'No way...',
      0.58,
      ['disbelief', 'shock', 'keyword'],
      'How. How did that hit me.',
      ['transcript'],
      [0.30, 0.45, 0.55, 0.20, 0.65],
      'Notable speech \u2014 strong hook',
    ),
    clip(
      'clip-6', 1380, 1405,
      'Extended Fight Sequence',
      'Things get intense',
      0.52,
      ['fight', 'rapid_cuts', 'hype'],
      '',
      ['audio', 'scene_change'],
      [0.50, 0.40, 0.25, 0.72, 0.15],
      'Visual chaos \u2014 confirmed by 2 signals',
    ),
  ],
  ranked: [],
  warnings: [],
  signals_used: ['audio', 'transcript', 'scene'],
  total_signals: 24,
}
