import { useState } from 'react'
import { X, Check } from 'lucide-react'
import { LAYOUT_OPTIONS } from '../lib/editTypes'
import type { LayoutMode, LayoutOption } from '../lib/editTypes'

interface Props {
  current: LayoutMode
  /** Current export aspect ratio — cards match this shape */
  aspectRatio: '9:16' | '16:9'
  /** Platform name for the format badge */
  platformName?: string
  onSelect: (layout: LayoutMode) => void
  onClose: () => void
}

/** Mini composition preview card matching the export format shape. */
function LayoutCard({ option, aspectRatio, selected, hovered }: {
  option: LayoutOption; aspectRatio: string; selected: boolean; hovered: boolean
}) {
  const border = selected ? option.accent : hovered ? 'rgba(255,255,255,0.3)' : 'rgba(255,255,255,0.1)'

  // Card shape matches the export format
  const aspectClass = aspectRatio === '9:16' ? 'aspect-[9/16]' : 'aspect-video'

  return (
    <div className={`relative w-full ${aspectClass} rounded overflow-hidden`}
      style={{ border: `2px solid ${border}`, background: '#0a0a14' }}>

      {/* Composition regions */}
      {option.regions.map((r, i) => (
        <div key={i} className="absolute flex items-center justify-center transition-all duration-200"
          style={{
            left: `${r.x}%`, top: `${r.y}%`, width: `${r.w}%`, height: `${r.h}%`,
            background: r.fill,
            borderRadius: r.w < 50 ? '3px' : undefined,
          }}>
          <span className="text-[6px] font-mono text-white/40 select-none uppercase tracking-wider">{r.label}</span>
        </div>
      ))}

      {/* Simulated subtitle bar at bottom */}
      <div className="absolute bottom-[8%] left-[10%] right-[10%] flex justify-center">
        <div className="h-[3px] bg-white/20 rounded-full" style={{ width: '60%' }} />
      </div>

      {/* Selected checkmark */}
      {selected && (
        <div className="absolute top-1 right-1 w-4 h-4 rounded-full flex items-center justify-center"
          style={{ background: option.accent }}>
          <Check className="w-2.5 h-2.5 text-white" />
        </div>
      )}
    </div>
  )
}

export default function LayoutPicker({ current, aspectRatio, platformName, onSelect, onClose }: Props) {
  const [hoveredId, setHoveredId] = useState<LayoutMode | null>(null)
  const previewOption = LAYOUT_OPTIONS.find(o => o.id === (hoveredId || current)) || LAYOUT_OPTIONS[0]

  const formatLabel = aspectRatio === '9:16' ? 'Vertical' : 'Landscape'

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onClick={e => { if (e.target === e.currentTarget) onClose() }}>
      <div className="bg-surface-800 border border-surface-600 rounded-2xl shadow-2xl w-full max-w-lg overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-surface-700">
          <div>
            <h2 className="text-base font-semibold text-white">Choose Layout</h2>
            <p className="text-[10px] text-slate-500 mt-0.5">
              Previewing for {formatLabel} {platformName ? `(${platformName})` : ''} — {aspectRatio}
            </p>
          </div>
          <button onClick={onClose} className="p-1 rounded-lg text-slate-400 hover:text-white hover:bg-surface-700 transition-colors cursor-pointer">
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* Layout grid */}
        <div className="p-5">
          <div className="grid grid-cols-3 gap-4">
            {LAYOUT_OPTIONS.map(option => {
              const isSelected = option.id === current
              const isHovered = option.id === hoveredId

              return (
                <button
                  key={option.id}
                  onClick={() => { onSelect(option.id); onClose() }}
                  onMouseEnter={() => setHoveredId(option.id)}
                  onMouseLeave={() => setHoveredId(null)}
                  className={`flex flex-col items-center gap-2 p-3 rounded-xl border transition-all cursor-pointer ${
                    isSelected
                      ? 'border-violet-500/50 bg-violet-600/10 shadow-lg shadow-violet-500/10'
                      : isHovered
                        ? 'border-surface-500 bg-surface-700/50'
                        : 'border-surface-600 bg-surface-900 hover:bg-surface-800'
                  }`}
                >
                  <LayoutCard option={option} aspectRatio={aspectRatio} selected={isSelected} hovered={isHovered} />

                  <span className={`text-xs font-medium ${isSelected ? 'text-violet-400' : 'text-slate-300'}`}>
                    {option.name}
                  </span>

                  {option.tag && (
                    <span className="text-[9px] px-1.5 py-0.5 rounded-full"
                      style={{ color: option.accent, background: `${option.accent}20`, border: `1px solid ${option.accent}30` }}>
                      {option.tag}
                    </span>
                  )}
                </button>
              )
            })}
          </div>

          <div className="mt-4 px-1">
            <p className="text-xs text-slate-400">{previewOption.description}</p>
          </div>
        </div>
      </div>
    </div>
  )
}
