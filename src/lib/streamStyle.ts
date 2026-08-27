export const STREAM_STYLE_OPTIONS = [
  { value: 'auto', label: 'Auto' },
  { value: 'action', label: 'Action' },
  { value: 'cozy', label: 'Cozy' },
  { value: 'story', label: 'Story' },
  { value: 'talking', label: 'Talking' },
  { value: 'mixed', label: 'Mixed' },
] as const

export type StreamStyle = typeof STREAM_STYLE_OPTIONS[number]['value']
export type DetectedStreamStyle = Exclude<StreamStyle, 'auto'>

const VALID_STYLES = new Set<StreamStyle>(STREAM_STYLE_OPTIONS.map(option => option.value))

export function normalizeStreamStyle(value: string | null | undefined): StreamStyle {
  return value && VALID_STYLES.has(value as StreamStyle) ? value as StreamStyle : 'auto'
}

export function normalizeDetectedStreamStyle(
  value: string | null | undefined,
): DetectedStreamStyle {
  const normalized = normalizeStreamStyle(value)
  return normalized === 'auto' ? 'mixed' : normalized
}

export function streamStyleLabel(style: StreamStyle | DetectedStreamStyle): string {
  return STREAM_STYLE_OPTIONS.find(option => option.value === style)?.label ?? 'Mixed'
}

export interface StreamStyleVod {
  analysis_status: string
  stream_style?: string | null
  detected_stream_style?: string | null
  analyzed_stream_style?: string | null
}

export function effectiveStreamStyle(vod: StreamStyleVod): DetectedStreamStyle {
  const requested = normalizeStreamStyle(vod.stream_style)
  return requested === 'auto'
    ? normalizeDetectedStreamStyle(vod.detected_stream_style)
    : requested
}

export function streamStyleAnalysisKey(vod: StreamStyleVod): string {
  const requested = normalizeStreamStyle(vod.stream_style)
  return requested === 'auto'
    ? `auto:${normalizeDetectedStreamStyle(vod.detected_stream_style)}`
    : requested
}

export function needsStreamStyleReanalysis(vod: StreamStyleVod): boolean {
  if (vod.analysis_status !== 'completed' || !vod.analyzed_stream_style) return false
  return vod.analyzed_stream_style !== streamStyleAnalysisKey(vod)
}
