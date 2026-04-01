import { Check, Link2, Loader2, Upload, AlertCircle } from 'lucide-react'
import { PLATFORM_INFO, usePlatformStore } from '../stores/platformStore'
import { EXPORT_PRESETS } from '../lib/editTypes'
import Tooltip from './Tooltip'

// ── Platform → default export preset mapping ──

export const PLATFORM_PRESET_MAP: Record<string, string> = {
  tiktok: 'tiktok',            // 9:16
  youtube: 'youtube',           // 16:9
  youtube_shorts: 'shorts',     // 9:16
  instagram: 'reels',           // 9:16
}

export function getPresetForPlatform(platform: string) {
  const presetId = PLATFORM_PRESET_MAP[platform] || 'tiktok'
  return EXPORT_PRESETS.find(p => p.id === presetId) || EXPORT_PRESETS[0]
}

// ── YouTube sub-format types ──

export type YouTubeSubFormat = 'regular' | 'shorts' | 'both'

/** Determine smart default: Shorts if vertical + under 60s, else Regular */
export function getDefaultYouTubeSubFormat(clipDurationSec: number, currentAspectRatio: string): YouTubeSubFormat {
  if (currentAspectRatio === '9:16' && clipDurationSec <= 60) return 'shorts'
  return 'regular'
}

/** Expand a YouTube sub-format selection into platform upload keys */
export function expandYouTubeSubFormat(sub: YouTubeSubFormat): string[] {
  if (sub === 'both') return ['youtube', 'youtube_shorts']
  if (sub === 'shorts') return ['youtube_shorts']
  return ['youtube']
}

// ── Per-platform visibility options ──

export interface VisibilityOption {
  value: string
  label: string
  hint: string
}

export const PLATFORM_VISIBILITY: Record<string, { options: VisibilityOption[]; default: string }> = {
  youtube: {
    default: 'unlisted',
    options: [
      { value: 'unlisted', label: 'Unlisted', hint: 'Link only' },
      { value: 'public', label: 'Public', hint: 'Visible on channel' },
      { value: 'private', label: 'Private', hint: 'Only you' },
    ],
  },
  youtube_shorts: {
    default: 'unlisted',
    options: [
      { value: 'unlisted', label: 'Unlisted', hint: 'Link only' },
      { value: 'public', label: 'Public', hint: 'Visible on channel' },
      { value: 'private', label: 'Private', hint: 'Only you' },
    ],
  },
  tiktok: {
    default: 'private',
    options: [
      { value: 'private', label: 'Draft', hint: 'Only you' },
      { value: 'friends', label: 'Friends', hint: 'Friends only' },
      { value: 'public', label: 'Public', hint: 'On your feed' },
    ],
  },
  instagram: {
    default: 'private',
    options: [
      { value: 'private', label: 'Private', hint: 'Only you' },
      { value: 'public', label: 'Public', hint: 'On your feed' },
    ],
  },
}

export function getDefaultVisibility(platform: string): string {
  return PLATFORM_VISIBILITY[platform]?.default || 'unlisted'
}

// ── Per-platform upload status ──

export type PlatformUploadStatus =
  | 'idle'
  | 'waiting'       // queued, hasn't started yet
  | 'exporting'     // re-exporting in required format
  | 'uploading'     // uploading to platform
  | 'done'          // success
  | 'error'         // failed

export interface PlatformUploadState {
  status: PlatformUploadStatus
  progress: number   // 0-100
  error?: string
  videoUrl?: string
  duplicateUrl?: string
}

// ── Visibility row (reusable for each sub-format) ──

function VisibilityRow({ label, platformKey, visibilities, onVisibilityChange, disabled, isActive, isLast }: {
  label: string
  platformKey: string
  visibilities: Record<string, string>
  onVisibilityChange: (platform: string, visibility: string) => void
  disabled?: boolean
  isActive: boolean
  isLast: boolean
}) {
  const visConfig = PLATFORM_VISIBILITY[platformKey]
  if (!visConfig) return null
  const currentVis = visibilities[platformKey] || visConfig.default

  return (
    <div className={`flex items-center gap-1 px-2 py-1 bg-surface-900/60 border border-violet-500/40 border-t-0 ${isLast ? 'rounded-b-lg' : ''}`}>
      <Tooltip text={`Controls who can see your ${label}`} position="left">
        <span className="text-[9px] text-slate-500 mr-0.5 whitespace-nowrap">{label}:</span>
      </Tooltip>
      <div className="flex gap-0.5">
        {visConfig.options.map(opt => (
          <button
            key={opt.value}
            onClick={() => onVisibilityChange(platformKey, opt.value)}
            disabled={disabled || isActive}
            title={opt.hint}
            className={`px-1.5 py-0.5 rounded text-[9px] font-medium transition-colors cursor-pointer ${
              currentVis === opt.value
                ? 'bg-violet-600/30 text-violet-300 border border-violet-500/50'
                : 'text-slate-500 hover:text-slate-300 border border-transparent hover:border-surface-500'
            } disabled:opacity-50 disabled:cursor-not-allowed`}
          >
            {opt.label}
          </button>
        ))}
      </div>
      <span className="text-[9px] text-slate-600 ml-auto">
        {visConfig.options.find(o => o.value === currentVis)?.hint}
      </span>
    </div>
  )
}

// ── Status indicator (reusable) ──

function StatusIndicator({ state, platformName, onViewUrl }: {
  state: PlatformUploadState
  platformName: string
  onViewUrl?: (url: string) => void
}) {
  if (state.status === 'exporting') return (
    <div className="flex items-center gap-1 text-[9px] text-amber-400">
      <Loader2 className="w-3 h-3 animate-spin" /> Export {state.progress}%
    </div>
  )
  if (state.status === 'uploading') return (
    <div className="flex items-center gap-1 text-[9px] text-violet-400">
      <Loader2 className="w-3 h-3 animate-spin" /> Upload
    </div>
  )
  if (state.status === 'waiting') return (
    <span className="text-[9px] text-slate-500">Queued</span>
  )
  if (state.status === 'done' && state.videoUrl) return (
    <Tooltip text={`Open your video on ${platformName}`} position="left">
      <button onClick={() => onViewUrl?.(state.videoUrl!)}
        className="text-[9px] text-green-400 hover:text-green-300 cursor-pointer flex items-center gap-0.5">
        <Check className="w-3 h-3" /> View
      </button>
    </Tooltip>
  )
  if (state.status === 'done') return (
    <span className="text-[9px] text-green-400 flex items-center gap-0.5">
      <Check className="w-3 h-3" /> Done
    </span>
  )
  if (state.status === 'error') return (
    <span className="text-[9px] text-red-400 flex items-center gap-0.5" title={state.error}>
      <AlertCircle className="w-3 h-3" /> Failed
    </span>
  )
  return null
}

// ── Selector Component ──

interface Props {
  selected: Record<string, boolean>
  onToggle: (platform: string) => void
  visibilities: Record<string, string>
  onVisibilityChange: (platform: string, visibility: string) => void
  states: Record<string, PlatformUploadState>
  currentPresetId: string
  disabled?: boolean
  onViewUrl?: (url: string) => void
  onConnect?: (platform: string) => void
  /** YouTube sub-format selection */
  youtubeSubFormat: YouTubeSubFormat
  onYouTubeSubFormatChange: (sub: YouTubeSubFormat) => void
  /** Current clip duration in seconds (for smart defaults) */
  clipDuration?: number
}

export default function PlatformUploadSelector({
  selected, onToggle, visibilities, onVisibilityChange, states, currentPresetId,
  disabled, onViewUrl, onConnect, youtubeSubFormat, onYouTubeSubFormatChange, clipDuration,
}: Props) {
  const { isConnected } = usePlatformStore()

  // Platforms that have adapters (available or connected)
  const platforms = Object.entries(PLATFORM_INFO)
    .filter(([_, info]) => info.available)
    .map(([key]) => key)

  return (
    <div className="space-y-1.5">
      {platforms.map(platform => {
        const info = PLATFORM_INFO[platform]
        const connected = isConnected(platform)
        const checked = selected[platform] || false
        const isYouTube = platform === 'youtube'

        // For YouTube, show combined status from sub-formats
        const mainState = states[platform] || { status: 'idle', progress: 0 }
        const shortsState = states['youtube_shorts'] || { status: 'idle', progress: 0 }
        const isActive = mainState.status !== 'idle' && mainState.status !== 'done' && mainState.status !== 'error'
        const shortsActive = shortsState.status !== 'idle' && shortsState.status !== 'done' && shortsState.status !== 'error'
        const anyActive = isActive || (isYouTube && shortsActive)

        // For non-YouTube platforms, compute format badge normally
        const preset = getPresetForPlatform(platform)
        const needsReExport = currentPresetId !== preset.id

        // Determine if expanded panel is open (YouTube: checked; others: checked + has visibility)
        const visConfig = PLATFORM_VISIBILITY[platform]
        const hasExpandedPanel = isYouTube
          ? checked  // YouTube always shows sub-format picker when checked
          : checked && !!visConfig

        return (
          <div key={platform} className="space-y-0">
            {/* Main row */}
            <div className={`flex items-center gap-2 px-2 py-1.5 rounded-lg border transition-colors ${
              checked ? 'border-violet-500/40 bg-violet-600/10' : 'border-surface-600 bg-surface-800/50'
            } ${hasExpandedPanel ? 'rounded-b-none border-b-0' : ''}`}>

              {/* Checkbox / connect */}
              {connected ? (
                <Tooltip text={`Upload to ${info.name} after export`} position="right">
                  <label className="flex items-center cursor-pointer">
                    <input type="checkbox" checked={checked}
                      onChange={() => onToggle(platform)}
                      disabled={disabled || anyActive}
                      className="sr-only" />
                    <div className={`w-4 h-4 rounded border flex items-center justify-center transition-colors ${
                      checked ? 'bg-violet-600 border-violet-500' : 'border-surface-500 bg-surface-900'
                    }`}>
                      {checked && <Check className="w-3 h-3 text-white" />}
                    </div>
                  </label>
                </Tooltip>
              ) : (
                <Tooltip text={`Connect your ${info.name} account`} position="right">
                  <button onClick={() => onConnect?.(platform)}
                    className="flex items-center gap-1 text-[9px] text-slate-400 hover:text-white transition-colors cursor-pointer">
                    <Link2 className="w-3 h-3" />
                  </button>
                </Tooltip>
              )}

              {/* Platform icon + name */}
              <div className="w-4 h-4 rounded flex items-center justify-center text-[7px] font-bold shrink-0"
                style={{ background: `${info.color}25`, color: info.color }}>
                {info.icon}
              </div>
              <span className="text-xs text-slate-300 flex-1">{info.name}</span>

              {/* Format badge — YouTube shows selected sub-format info */}
              {isYouTube && checked ? (
                <span className="text-[9px] px-1.5 py-0.5 rounded text-slate-500 bg-surface-900">
                  {youtubeSubFormat === 'both' ? '16:9 + 9:16' : youtubeSubFormat === 'shorts' ? '9:16' : '16:9'}
                </span>
              ) : !isYouTube ? (
                <span className={`text-[9px] px-1.5 py-0.5 rounded ${
                  needsReExport ? 'text-amber-400 bg-amber-500/10' : 'text-slate-500 bg-surface-900'
                }`}>
                  {preset.aspectRatio}
                  {needsReExport && ' \u21bb'}
                </span>
              ) : null}

              {/* Status indicators */}
              {!isYouTube && <StatusIndicator state={mainState} platformName={info.name} onViewUrl={onViewUrl} />}
              {isYouTube && youtubeSubFormat !== 'both' && (
                <StatusIndicator
                  state={youtubeSubFormat === 'shorts' ? shortsState : mainState}
                  platformName={youtubeSubFormat === 'shorts' ? 'YouTube Shorts' : 'YouTube'}
                  onViewUrl={onViewUrl}
                />
              )}

              {/* Not connected label */}
              {!connected && (
                <span className="text-[9px] text-slate-500">Not connected</span>
              )}
            </div>

            {/* YouTube expanded panel: sub-format picker + per-format visibility */}
            {isYouTube && checked && (
              <div className="border border-violet-500/40 border-t-0 rounded-b-lg overflow-hidden">
                {/* Sub-format selector */}
                <div className="flex items-center gap-1 px-2 py-1.5 bg-surface-900/60">
                  <Tooltip text="Choose which YouTube format to upload" position="left">
                    <span className="text-[9px] text-slate-500 mr-1">Format:</span>
                  </Tooltip>
                  <div className="flex gap-0.5">
                    {([
                      { value: 'regular' as YouTubeSubFormat, label: 'YouTube', hint: '16:9 landscape' },
                      { value: 'shorts' as YouTubeSubFormat, label: 'Shorts', hint: '9:16 vertical' },
                      { value: 'both' as YouTubeSubFormat, label: 'Both', hint: 'Upload two versions' },
                    ]).map(opt => (
                      <Tooltip key={opt.value} text={opt.hint}>
                        <button
                          onClick={() => onYouTubeSubFormatChange(opt.value)}
                          disabled={disabled || anyActive}
                          className={`px-2 py-0.5 rounded text-[9px] font-medium transition-colors cursor-pointer ${
                            youtubeSubFormat === opt.value
                              ? 'bg-violet-600/30 text-violet-300 border border-violet-500/50'
                              : 'text-slate-500 hover:text-slate-300 border border-transparent hover:border-surface-500'
                          } disabled:opacity-50 disabled:cursor-not-allowed`}
                        >
                          {opt.label}
                        </button>
                      </Tooltip>
                    ))}
                  </div>
                  {clipDuration != null && clipDuration > 60 && (youtubeSubFormat === 'shorts' || youtubeSubFormat === 'both') && (
                    <span className="text-[9px] text-amber-400 ml-auto">Clip &gt; 60s</span>
                  )}
                </div>

                {/* Per-sub-format status + visibility rows */}
                {(youtubeSubFormat === 'regular' || youtubeSubFormat === 'both') && (
                  <>
                    {youtubeSubFormat === 'both' && (
                      <div className="flex items-center gap-1.5 px-2 py-0.5 bg-surface-950/40 border-t border-surface-700/30">
                        <span className="text-[9px] text-slate-400 font-medium">YouTube (16:9)</span>
                        <StatusIndicator state={mainState} platformName="YouTube" onViewUrl={onViewUrl} />
                      </div>
                    )}
                    <VisibilityRow
                      label={youtubeSubFormat === 'both' ? 'Visibility' : 'YouTube visibility'}
                      platformKey="youtube"
                      visibilities={visibilities}
                      onVisibilityChange={onVisibilityChange}
                      disabled={disabled}
                      isActive={isActive}
                      isLast={youtubeSubFormat === 'regular'}
                    />
                  </>
                )}
                {(youtubeSubFormat === 'shorts' || youtubeSubFormat === 'both') && (
                  <>
                    {youtubeSubFormat === 'both' && (
                      <div className="flex items-center gap-1.5 px-2 py-0.5 bg-surface-950/40 border-t border-surface-700/30">
                        <span className="text-[9px] text-slate-400 font-medium">Shorts (9:16)</span>
                        <StatusIndicator state={shortsState} platformName="YouTube Shorts" onViewUrl={onViewUrl} />
                      </div>
                    )}
                    <VisibilityRow
                      label={youtubeSubFormat === 'both' ? 'Visibility' : 'Shorts visibility'}
                      platformKey="youtube_shorts"
                      visibilities={visibilities}
                      onVisibilityChange={onVisibilityChange}
                      disabled={disabled}
                      isActive={shortsActive}
                      isLast={true}
                    />
                  </>
                )}
              </div>
            )}

            {/* Non-YouTube visibility selector */}
            {!isYouTube && checked && visConfig && (
              <VisibilityRow
                label="Visibility"
                platformKey={platform}
                visibilities={visibilities}
                onVisibilityChange={onVisibilityChange}
                disabled={disabled}
                isActive={isActive}
                isLast={true}
              />
            )}
          </div>
        )
      })}
    </div>
  )
}
