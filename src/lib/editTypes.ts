// ── ClipGoblin Edit Types ──
// Clean data model for the clip editing system.

// ── Caption Styles ──

export interface CaptionStyle {
  id: string
  name: string
  /** Optional renderer treatment beyond the base typography fields. */
  presentation?: 'cardboard'
  fontFamily: string
  fontSize: number       // px at 1080p export resolution
  fontWeight: number     // 400–900
  fontColor: string      // hex
  strokeColor: string    // hex
  strokeWidth: number    // px at 1080p
  bgColor: string        // hex with alpha, e.g. '#00000080'
  bgPadding: number      // px at 1080p
  bgRadius: number       // px border-radius at 1080p
  uppercase: boolean
  letterSpacing: number  // em units (0 = normal, 0.05 = wide)
  lineHeight: number     // multiplier (1.2 = tight, 1.5 = comfortable)
  /** Multi-layer text shadow for crisp outlines */
  shadow: string
  /** Approximate glyph width used to keep long words inside the video safe area. */
  characterWidthFactor?: number
  /** Fraction of the frame width available to caption text. */
  safeWidthRatio?: number
}

export const CAPTION_STYLES: CaptionStyle[] = [
  {
    id: 'clean',
    name: 'Clean',
    fontFamily: "'Segoe UI', Arial, sans-serif",
    fontSize: 52, fontWeight: 700,
    fontColor: '#FFFFFF',
    strokeColor: '#000000', strokeWidth: 0,
    bgColor: '', bgPadding: 0, bgRadius: 0,
    uppercase: false,
    letterSpacing: 0.01, lineHeight: 1.3,
    shadow: '0 0 6px rgba(0,0,0,0.9), 0 0 3px rgba(0,0,0,0.95), 1px 1px 0 #000, -1px -1px 0 #000, 1px -1px 0 #000, -1px 1px 0 #000, 0 2px 6px rgba(0,0,0,0.7), 0 4px 12px rgba(0,0,0,0.5)',
  },
  {
    id: 'bold-white',
    name: 'Cardboard',
    presentation: 'cardboard',
    fontFamily: "'Arial Black', Arial, sans-serif",
    fontSize: 52, fontWeight: 900,
    fontColor: '#7A2118',
    strokeColor: '', strokeWidth: 0,
    bgColor: '#C99358', bgPadding: 32, bgRadius: 0,
    uppercase: true,
    letterSpacing: 0.01, lineHeight: 1.05,
    shadow: '0 1px 0 rgba(255,255,255,0.14), 0 2px 2px rgba(63,35,16,0.25)',
    characterWidthFactor: 0.72,
    safeWidthRatio: 0.68,
  },
  {
    id: 'boxed',
    name: 'Frosted',
    fontFamily: "'Coiny', 'Arial Black', Arial, sans-serif",
    fontSize: 58, fontWeight: 400,
    fontColor: '#FFFFFF',
    strokeColor: '#FFFFFF', strokeWidth: 3,
    bgColor: '', bgPadding: 0, bgRadius: 0,
    uppercase: true,
    letterSpacing: 0.01, lineHeight: 1.1,
    shadow: '2px 3px 0 #F05BD8, 4px 5px 0 #6D28D9',
    characterWidthFactor: 0.72,
  },
  {
    id: 'neon',
    name: 'Neon Pop',
    fontFamily: "'Segoe UI', Arial, sans-serif",
    fontSize: 54, fontWeight: 800,
    fontColor: '#00FF88',
    strokeColor: '#000000', strokeWidth: 0,
    bgColor: '', bgPadding: 0, bgRadius: 0,
    uppercase: true,
    letterSpacing: 0.03, lineHeight: 1.25,
    shadow: '0 0 8px #00ff8880, 0 0 3px #000, 0 0 3px #000, 1px 1px 0 #000, -1px -1px 0 #000, 0 2px 12px rgba(0,255,136,0.3)',
  },
  {
    id: 'minimal',
    name: 'Drip',
    fontFamily: "'Nosifer', 'Arial Black', Arial, sans-serif",
    fontSize: 50, fontWeight: 400,
    fontColor: '#FF351F',
    strokeColor: '#3B0000', strokeWidth: 2,
    bgColor: '', bgPadding: 0, bgRadius: 0,
    uppercase: true,
    letterSpacing: 0.01, lineHeight: 1.15,
    shadow: '0 2px 0 #7A0000, 0 4px 7px rgba(0,0,0,0.9)',
    characterWidthFactor: 0.8,
  },
  {
    id: 'fire',
    name: 'Highlight',
    fontFamily: "'Rubik Dirt', 'Arial Black', Arial, sans-serif",
    fontSize: 60, fontWeight: 400,
    fontColor: '#FFE45E',
    strokeColor: '#000000', strokeWidth: 3,
    bgColor: '', bgPadding: 0, bgRadius: 0,
    uppercase: true,
    letterSpacing: 0.01, lineHeight: 1.1,
    shadow: '2px 0 0 #000, -2px 0 0 #000, 0 2px 0 #000, 0 -2px 0 #000, 0 4px 9px rgba(0,0,0,0.85)',
    characterWidthFactor: 0.72,
  },
  {
    id: 'comic-pop',
    name: 'Comic Pop',
    fontFamily: "'Bangers', 'Arial Black', Arial, sans-serif",
    fontSize: 64, fontWeight: 400,
    fontColor: '#67E8E6',
    strokeColor: '#55206F', strokeWidth: 3,
    bgColor: '', bgPadding: 0, bgRadius: 0,
    uppercase: true,
    letterSpacing: 0.02, lineHeight: 1.05,
    shadow: '2px 2px 0 #F05BD8, 4px 4px 0 #55206F, 0 7px 10px rgba(0,0,0,0.75)',
    characterWidthFactor: 0.68,
  },
]

// ── Text Overlays ──

export interface TextOverlay {
  id: string
  text: string
  startTime: number   // seconds relative to clip start
  endTime: number      // seconds relative to clip start
  position: 'top' | 'center' | 'bottom'
  style: 'title' | 'label' | 'cta'  // call-to-action, label, title card
  fontSize: number
  color: string
}

// ── Export Presets ──

export interface ExportPreset {
  id: string
  name: string
  platform: string
  aspectRatio: '9:16' | '16:9'
  maxDuration: number   // seconds
  resolution: { w: number; h: number }
  fileLabel: string     // appended to filename
  description: string
  /** If true, hide this preset from the UI (but keep it available for code that references it by id). */
  hidden?: boolean
}

export const EXPORT_PRESETS: ExportPreset[] = [
  {
    id: 'tiktok',
    name: 'TikTok',
    platform: 'TikTok',
    aspectRatio: '9:16',
    maxDuration: 60,
    resolution: { w: 1080, h: 1920 },
    fileLabel: 'tiktok',
    description: '9:16 vertical, max 60s',
  },
  {
    id: 'reels',
    name: 'Instagram Reels',
    platform: 'Instagram',
    aspectRatio: '9:16',
    maxDuration: 90,
    resolution: { w: 1080, h: 1920 },
    fileLabel: 'reels',
    description: '9:16 vertical, max 90s',
    hidden: true, // Hidden until Instagram is re-enabled in platformStore (available: true)
  },
  {
    id: 'shorts',
    name: 'YouTube Shorts',
    platform: 'YouTube',
    aspectRatio: '9:16',
    maxDuration: 60,
    resolution: { w: 1080, h: 1920 },
    fileLabel: 'shorts',
    description: '9:16 vertical, max 60s',
  },
  {
    id: 'youtube',
    name: 'YouTube',
    platform: 'YouTube',
    aspectRatio: '16:9',
    maxDuration: 600,
    resolution: { w: 1920, h: 1080 },
    fileLabel: 'youtube',
    description: '16:9 landscape',
  },
]

// ── Layout Modes ──

export type LayoutMode = 'none' | 'context_fit' | 'split' | 'pip'

export interface LayoutOption {
  id: LayoutMode
  name: string
  description: string
  /** Short tag for recommendations */
  tag?: string
  /** Color accent for the card */
  accent: string
  /** Layout schematic regions (proportional) */
  regions: { label: string; x: number; y: number; w: number; h: number; fill: string }[]
}

export const LAYOUT_OPTIONS: LayoutOption[] = [
  {
    id: 'none', name: 'Full Frame', description: 'Center crop — maximizes gameplay visibility',
    tag: 'Best for gameplay', accent: '#3b82f6',
    regions: [
      { label: 'GAME', x: 0, y: 0, w: 100, h: 100, fill: '#1e3a5f' },
    ],
  },
  {
    id: 'context_fit', name: 'Context Fit', description: 'Keeps the entire video visible with a blurred, black-bar, or branded background',
    tag: 'Best for imports', accent: '#06b6d4',
    regions: [
      { label: 'BLUR', x: 0, y: 0, w: 100, h: 100, fill: '#164e63' },
      { label: 'FULL GAME', x: 4, y: 34, w: 92, h: 32, fill: '#0f172a' },
    ],
  },
  {
    id: 'split', name: 'Split View', description: 'Game on top, facecam on bottom — balanced',
    tag: 'Recommended', accent: '#8b5cf6',
    regions: [
      { label: 'GAME', x: 0, y: 0, w: 100, h: 60, fill: '#1e3a5f' },
      { label: 'FACE', x: 0, y: 62, w: 100, h: 38, fill: '#3b1e5f' },
    ],
  },
  {
    id: 'pip', name: 'Picture-in-Picture', description: 'Facecam overlay in corner — cinematic feel',
    tag: 'Best for reactions', accent: '#10b981',
    regions: [
      { label: 'GAME', x: 0, y: 0, w: 100, h: 100, fill: '#1e3a5f' },
      { label: 'CAM', x: 65, y: 65, w: 30, h: 30, fill: '#3b1e5f' },
    ],
  },
  // TODO(v2): { id: 'blur-bg', name: 'Blurred Background', description: 'Gameplay centered with blurred fill', ... }
  // TODO(v2): { id: 'facecam-focus', name: 'Facecam Focus', description: 'Large facecam, small gameplay', ... }
  // TODO(v2): { id: 'dynamic', name: 'AI Reframe', description: 'Follows the action automatically', ... }
]

// ── Full Editable Clip State ──

export interface EditableClipSettings {
  clipId: string
  title: string
  startTime: number
  endTime: number
  captionsEnabled: boolean
  captionStyle: string         // CaptionStyle.id
  captionPosition: 'top' | 'center' | 'bottom'
  captionText: string | null   // raw SRT or plain text
  layoutMode: LayoutMode
  aspectRatio: '9:16' | '16:9'
  textOverlays: TextOverlay[]
  exportPreset: string         // ExportPreset.id
  // TODO(v2): zoomEffects: ZoomEffect[]
  // TODO(v2): hookSuggestion: { suggestedStart: number; reason: string } | null
  // TODO(v2): creatorTemplateId: string | null
  // TODO(v3): editVariants: EditVariant[]
  // TODO(v3): editScore: number
  // TODO(v3): editMode: 'safe' | 'aggressive'
}

// ── Creator Templates (Phase 2 scaffold) ──

// TODO(v2): implement saved creator templates
export interface CreatorTemplate {
  id: string
  name: string
  captionStyle: string
  layoutMode: LayoutMode
  exportPreset: string
  aspectRatio: string
  // TODO(v2): zoom presets, emphasis settings, platform-specific tweaks
}
