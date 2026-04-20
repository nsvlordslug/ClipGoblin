import { useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { Check, Circle, X } from 'lucide-react'
import { useAppStore } from '../stores/appStore'
import { usePlatformStore } from '../stores/platformStore'
import { useScheduleStore } from '../stores/scheduleStore'

const DISMISS_KEY = 'tester_checklist_dismissed_v1'

function isDismissed(): boolean {
  try { return localStorage.getItem(DISMISS_KEY) === 'true' } catch { return false }
}

function markDismissed() {
  try { localStorage.setItem(DISMISS_KEY, 'true') } catch { /* ignore */ }
}

/**
 * Checklist shown on the Dashboard for testers, tracking the core onboarding
 * path from "fresh install" to "first clip shipped". Each row auto-checks as
 * the underlying state changes (e.g. connecting Twitch flips the first row).
 *
 * Persists dismissal in localStorage so testers who've finished aren't nagged.
 * Users can re-trigger via Help → "Show onboarding checklist".
 */
export default function TesterChecklist() {
  const navigate = useNavigate()
  const [dismissedLocal, setDismissedLocal] = useState(() => isDismissed())

  const { loggedInUser, vods, highlights, clips } = useAppStore()
  const { accounts } = usePlatformStore()
  const { uploads } = useScheduleStore()

  const steps = useMemo(() => {
    const anyPlatform = Object.values(accounts).some(a => !!a)
    const anyAnalyzed = vods.some(v => v.analysis_status === 'completed')
    const anyExported = clips.some(c => c.render_status === 'completed')
    const anyScheduled = uploads.length > 0

    return [
      {
        id: 'twitch',
        label: 'Connect Twitch',
        desc: 'Login to fetch your VODs',
        done: !!loggedInUser,
        action: () => navigate('/settings'),
      },
      {
        id: 'platform',
        label: 'Connect a publishing platform',
        desc: 'YouTube or TikTok so you can upload clips',
        done: anyPlatform,
        action: () => navigate('/settings'),
      },
      {
        id: 'analyze',
        label: 'Analyze a VOD',
        desc: 'Auto-Hunt from Dashboard or pick one from VODs',
        done: anyAnalyzed,
        action: () => navigate('/vods'),
      },
      {
        id: 'review',
        label: 'Review a detected clip',
        desc: `${highlights.length} highlights available`,
        done: highlights.length > 0,
        action: () => navigate('/clips'),
      },
      {
        id: 'export',
        label: 'Export a clip',
        desc: 'Open the Editor and click Export',
        done: anyExported,
        action: () => navigate('/clips'),
      },
      {
        id: 'schedule',
        label: 'Schedule or publish an upload',
        desc: 'Ship a clip manually or try Auto-ship',
        done: anyScheduled,
        action: () => navigate('/scheduled'),
      },
    ]
  }, [loggedInUser, vods, highlights, clips, accounts, uploads, navigate])

  const doneCount = steps.filter(s => s.done).length
  const totalCount = steps.length
  const allDone = doneCount === totalCount

  // Hide if dismissed OR everything done (quiet disappearance — user got here, job's over).
  if (dismissedLocal || allDone) return null

  const dismiss = () => {
    markDismissed()
    setDismissedLocal(true)
  }

  return (
    <section
      className="v4-panel relative"
      style={{padding: 16, background: 'linear-gradient(135deg, rgba(167,139,250,0.08), rgba(244,114,182,0.04))'}}
    >
      <button
        onClick={dismiss}
        className="absolute top-2 right-2 p-1 rounded text-slate-500 hover:text-white cursor-pointer"
        title="Dismiss (re-open via Help)"
        aria-label="Dismiss checklist"
      >
        <X className="w-3.5 h-3.5" />
      </button>

      <div className="flex items-center gap-2 mb-2">
        <span className="text-[11px] font-bold tracking-wider uppercase text-violet-300">
          🧪 Tester checklist
        </span>
        <span className="text-[11px] text-slate-500">
          {doneCount}/{totalCount} done
        </span>
      </div>

      <div className="text-xs text-slate-400 mb-3">
        New here? Run through these to exercise the full pipeline. Clicking a row jumps you to the right page.
      </div>

      <div className="space-y-1.5">
        {steps.map(s => (
          <button
            key={s.id}
            onClick={s.action}
            disabled={s.done}
            className={`w-full flex items-center gap-2.5 text-left px-2.5 py-1.5 rounded-md transition-colors ${
              s.done
                ? 'opacity-60 cursor-default'
                : 'hover:bg-surface-800 cursor-pointer'
            }`}
          >
            {s.done
              ? <Check className="w-3.5 h-3.5 text-emerald-400 shrink-0" />
              : <Circle className="w-3.5 h-3.5 text-slate-500 shrink-0" />}
            <div className="flex-1 min-w-0">
              <div className={`text-xs font-medium ${s.done ? 'text-slate-400 line-through' : 'text-white'}`}>
                {s.label}
              </div>
              <div className="text-[10px] text-slate-500 truncate">{s.desc}</div>
            </div>
          </button>
        ))}
      </div>
    </section>
  )
}

/**
 * Public helper so Help → "Show onboarding checklist" can reset the
 * dismissed flag and bring the panel back.
 */
export function resetTesterChecklistDismissal() {
  try { localStorage.removeItem(DISMISS_KEY) } catch { /* ignore */ }
}
