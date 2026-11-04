import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Check, Gauge, Plus, Sparkles, Wrench, X } from 'lucide-react'
import { version as appVersion } from '../../package.json'
import {
  getReleaseNotesPresentation,
  markReleaseNotesSeen,
  type ReleaseChangeKind,
} from '../data/releaseNotes'

const CHANGE_LABELS: Record<ReleaseChangeKind, string> = {
  new: 'New',
  improved: 'Improved',
  fixed: 'Fixed',
  'under-the-hood': 'Under the hood',
}

const CHANGE_ICONS = {
  new: Plus,
  improved: Gauge,
  fixed: Check,
  'under-the-hood': Wrench,
} satisfies Record<ReleaseChangeKind, typeof Plus>

export default function WhatsNewModal() {
  const presentation = useMemo(() => getReleaseNotesPresentation(appVersion), [])
  const [open, setOpen] = useState(Boolean(presentation))
  const modalRef = useRef<HTMLDivElement>(null)
  const dismissButtonRef = useRef<HTMLButtonElement>(null)

  const dismiss = useCallback(() => {
    if (presentation) markReleaseNotesSeen(presentation.currentVersion)
    setOpen(false)
  }, [presentation])

  useEffect(() => {
    if (!open) return
    const previousActiveElement = document.activeElement as HTMLElement | null
    const previousOverflow = document.body.style.overflow
    document.body.style.overflow = 'hidden'
    dismissButtonRef.current?.focus()

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault()
        dismiss()
        return
      }
      if (event.key !== 'Tab' || !modalRef.current) return

      const focusable = Array.from(
        modalRef.current.querySelectorAll<HTMLElement>('button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])'),
      ).filter(element => !element.hasAttribute('disabled'))
      if (focusable.length === 0) return

      const first = focusable[0]
      const last = focusable[focusable.length - 1]
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault()
        last.focus()
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault()
        first.focus()
      }
    }

    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('keydown', handleKeyDown)
      document.body.style.overflow = previousOverflow
      previousActiveElement?.focus()
    }
  }, [dismiss, open])

  if (!open || !presentation) return null

  return (
    <div className="cg-whats-new-backdrop" onMouseDown={(event) => {
      if (event.target === event.currentTarget) dismiss()
    }}>
      <div
        ref={modalRef}
        className="cg-whats-new-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="cg-whats-new-title"
        aria-describedby="cg-whats-new-description"
      >
        <button type="button" className="cg-whats-new-close" onClick={dismiss} aria-label="Close What’s New">
          <X aria-hidden="true" />
        </button>

        <header className="cg-whats-new-header">
          <span className="cg-whats-new-icon" aria-hidden="true"><Sparkles /></span>
          <div>
            <h2 id="cg-whats-new-title">
              {presentation.previousVersion ? `Updated to ${presentation.currentVersion}` : `What’s new in ${presentation.currentVersion}`}
            </h2>
            <p id="cg-whats-new-description">
              {presentation.previousVersion
                ? `Everything that changed since version ${presentation.previousVersion}.`
                : 'Here’s what changed in this release.'}
            </p>
          </div>
        </header>

        <div className="cg-whats-new-scroll">
          {presentation.entries.map(entry => (
            <article className="cg-whats-new-release" key={entry.version}>
              <div className="cg-whats-new-release-heading">
                <h3>Version {entry.version}</h3>
                <span className="cg-whats-new-new-pill">New</span>
                <time dateTime={entry.date}>{new Date(`${entry.date}T12:00:00`).toLocaleDateString()}</time>
              </div>
              <p className="cg-whats-new-summary">{entry.summary}</p>

              {entry.changes.map(group => {
                const Icon = CHANGE_ICONS[group.kind]
                return (
                  <section className="cg-whats-new-group" data-kind={group.kind} key={group.kind}>
                    <h4><Icon aria-hidden="true" />{CHANGE_LABELS[group.kind]}</h4>
                    <ul>
                      {group.items.map(item => <li key={item}>{item}</li>)}
                    </ul>
                  </section>
                )
              })}
            </article>
          ))}
        </div>

        <footer className="cg-whats-new-footer">
          <button ref={dismissButtonRef} type="button" className="v4-btn primary cg-whats-new-dismiss" onClick={dismiss}>
            Got it
          </button>
        </footer>
      </div>
    </div>
  )
}
