import { useState, useRef, useCallback, useEffect, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { useUiStore } from '../stores/uiStore'

interface TooltipProps {
  text: string
  children: ReactNode
  /** Position relative to the wrapped element. Default: 'top' */
  position?: 'top' | 'bottom' | 'left' | 'right'
  /** Delay in ms before showing. Default: 400 */
  delay?: number
}

export default function Tooltip({ text, children, position = 'top', delay = 400 }: TooltipProps) {
  const enabled = useUiStore(s => s.settings.showTooltips)
  const [visible, setVisible] = useState(false)
  const [coords, setCoords] = useState<{ top: number; left: number } | null>(null)
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const triggerRef = useRef<HTMLDivElement>(null)
  const tooltipRef = useRef<HTMLDivElement>(null)

  const show = useCallback(() => {
    if (!enabled) return
    timerRef.current = setTimeout(() => setVisible(true), delay)
  }, [enabled, delay])

  const hide = useCallback(() => {
    if (timerRef.current) clearTimeout(timerRef.current)
    timerRef.current = null
    setVisible(false)
    setCoords(null)
  }, [])

  // Calculate position once tooltip is visible and rendered
  useEffect(() => {
    if (!visible || !triggerRef.current) return
    const trigger = triggerRef.current.getBoundingClientRect()

    // Use requestAnimationFrame to measure tooltip after render
    const raf = requestAnimationFrame(() => {
      const tooltip = tooltipRef.current
      const tw = tooltip ? tooltip.offsetWidth : 0
      const th = tooltip ? tooltip.offsetHeight : 0
      const gap = 6

      let top = 0
      let left = 0

      switch (position) {
        case 'top':
          top = trigger.top - th - gap
          left = trigger.left + trigger.width / 2 - tw / 2
          break
        case 'bottom':
          top = trigger.bottom + gap
          left = trigger.left + trigger.width / 2 - tw / 2
          break
        case 'left':
          top = trigger.top + trigger.height / 2 - th / 2
          left = trigger.left - tw - gap
          break
        case 'right':
          top = trigger.top + trigger.height / 2 - th / 2
          left = trigger.right + gap
          break
      }

      // Keep tooltip within viewport bounds
      const vw = window.innerWidth
      const vh = window.innerHeight
      if (left < 4) left = 4
      if (left + tw > vw - 4) left = vw - tw - 4
      // If tooltip would go above viewport, flip to bottom
      if (top < 4 && position === 'top') {
        top = trigger.bottom + gap
      }
      // If tooltip would go below viewport, flip to top
      if (top + th > vh - 4 && position === 'bottom') {
        top = trigger.top - th - gap
      }

      setCoords({ top, left })
    })

    return () => cancelAnimationFrame(raf)
  }, [visible, position])

  return (
    <div ref={triggerRef} className="relative inline-flex" onMouseEnter={show} onMouseLeave={hide} onFocus={show} onBlur={hide}>
      {children}
      {visible && createPortal(
        <div
          ref={tooltipRef}
          className="fixed z-[9999] pointer-events-none"
          style={{
            top: coords ? coords.top : -9999,
            left: coords ? coords.left : -9999,
            opacity: coords ? 1 : 0,
            transition: 'opacity 0.1s',
          }}
        >
          <div className="px-2 py-1 bg-slate-900 border border-slate-700 rounded text-[10px] text-slate-200 shadow-lg max-w-[280px] text-center leading-snug"
            style={{ whiteSpace: text.length > 30 ? 'normal' : 'nowrap' }}>
            {text}
          </div>
        </div>,
        document.body,
      )}
    </div>
  )
}
