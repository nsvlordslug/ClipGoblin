import { useState } from 'react'
import { Bug, Send, Loader2, CheckCircle2, AlertCircle, ExternalLink } from 'lucide-react'
import { invoke } from '@tauri-apps/api/core'

const PAGES = ['VODs', 'Clips', 'Editor', 'Publishing', 'Settings', 'Other'] as const
const SEVERITIES = ['Crash', 'Broken Feature', 'Cosmetic'] as const

interface BugReportResult {
  success: boolean
  issueUrl: string | null
  error: string | null
}

export default function BugReport() {
  const [title, setTitle] = useState('')
  const [description, setDescription] = useState('')
  const [steps, setSteps] = useState('')
  const [expected, setExpected] = useState('')
  const [page, setPage] = useState<string>(PAGES[0])
  const [severity, setSeverity] = useState<string>(SEVERITIES[1])
  const [submitting, setSubmitting] = useState(false)
  const [result, setResult] = useState<BugReportResult | null>(null)

  const canSubmit = title.trim().length > 0 && description.trim().length > 0 && steps.trim().length > 0

  const handleSubmit = async () => {
    if (!canSubmit || submitting) return
    setSubmitting(true)
    setResult(null)
    try {
      const res = await invoke<BugReportResult>('submit_bug_report', {
        report: { title, description, steps, expected, page, severity },
      })
      setResult(res)
      if (res.success) {
        setTitle('')
        setDescription('')
        setSteps('')
        setExpected('')
        setPage(PAGES[0])
        setSeverity(SEVERITIES[1])
      }
    } catch (err) {
      setResult({ success: false, issueUrl: null, error: String(err) })
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <div className="space-y-6 max-w-2xl">
      <div className="flex items-center gap-3">
        <div className="flex items-center justify-center w-10 h-10 rounded-xl bg-red-500/20">
          <Bug className="w-5 h-5 text-red-400" />
        </div>
        <div>
          <h1 className="text-2xl font-bold text-white">Report a Bug</h1>
          <p className="text-sm text-slate-400">Help improve ClipGoblin by reporting issues you find.</p>
        </div>
      </div>

      {/* Success banner */}
      {result?.success && (
        <div className="flex items-start gap-3 p-4 bg-emerald-500/10 border border-emerald-500/30 rounded-xl">
          <CheckCircle2 className="w-5 h-5 text-emerald-400 mt-0.5 shrink-0" />
          <div>
            <p className="text-sm font-medium text-emerald-300">Bug report submitted!</p>
            {result.issueUrl && (
              <a
                href={result.issueUrl}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-1 text-xs text-emerald-400 hover:text-emerald-300 mt-1"
              >
                View on GitHub <ExternalLink className="w-3 h-3" />
              </a>
            )}
          </div>
        </div>
      )}

      {/* Error banner */}
      {result && !result.success && result.error && (
        <div className="flex items-start gap-3 p-4 bg-red-500/10 border border-red-500/30 rounded-xl">
          <AlertCircle className="w-5 h-5 text-red-400 mt-0.5 shrink-0" />
          <p className="text-sm text-red-300">{result.error}</p>
        </div>
      )}

      <div className="bg-surface-800 border border-surface-700 rounded-xl p-6 space-y-5">
        {/* Title */}
        <div className="space-y-1.5">
          <label className="text-sm font-medium text-slate-300">
            Title <span className="text-red-400">*</span>
          </label>
          <input
            type="text"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="Short summary of the issue"
            className="w-full bg-surface-900 border border-surface-600 text-white text-sm px-3 py-2 rounded-lg outline-none focus:border-violet-500 transition-colors placeholder:text-slate-600"
          />
        </div>

        {/* Description */}
        <div className="space-y-1.5">
          <label className="text-sm font-medium text-slate-300">
            Description <span className="text-red-400">*</span>
          </label>
          <textarea
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="What happened? Be as specific as possible."
            rows={3}
            className="w-full bg-surface-900 border border-surface-600 text-white text-sm px-3 py-2 rounded-lg outline-none focus:border-violet-500 transition-colors placeholder:text-slate-600 resize-y"
          />
        </div>

        {/* Steps to Reproduce */}
        <div className="space-y-1.5">
          <label className="text-sm font-medium text-slate-300">
            Steps to Reproduce <span className="text-red-400">*</span>
          </label>
          <textarea
            value={steps}
            onChange={(e) => setSteps(e.target.value)}
            placeholder={"1. Go to...\n2. Click on...\n3. See error"}
            rows={3}
            className="w-full bg-surface-900 border border-surface-600 text-white text-sm px-3 py-2 rounded-lg outline-none focus:border-violet-500 transition-colors placeholder:text-slate-600 resize-y"
          />
        </div>

        {/* Expected Behavior */}
        <div className="space-y-1.5">
          <label className="text-sm font-medium text-slate-300">Expected Behavior</label>
          <textarea
            value={expected}
            onChange={(e) => setExpected(e.target.value)}
            placeholder="What did you expect to happen?"
            rows={2}
            className="w-full bg-surface-900 border border-surface-600 text-white text-sm px-3 py-2 rounded-lg outline-none focus:border-violet-500 transition-colors placeholder:text-slate-600 resize-y"
          />
        </div>

        {/* Page + Severity row */}
        <div className="grid grid-cols-2 gap-5">
          {/* Page / Feature */}
          <div className="space-y-1.5">
            <label className="text-sm font-medium text-slate-300">Page / Feature</label>
            <select
              value={page}
              onChange={(e) => setPage(e.target.value)}
              className="w-full bg-surface-900 border border-surface-600 text-white text-sm px-3 py-2 rounded-lg outline-none focus:border-violet-500 transition-colors cursor-pointer"
            >
              {PAGES.map((p) => (
                <option key={p} value={p}>{p}</option>
              ))}
            </select>
          </div>

          {/* Severity */}
          <div className="space-y-1.5">
            <label className="text-sm font-medium text-slate-300">Severity</label>
            <div className="flex gap-2">
              {SEVERITIES.map((s) => (
                <button
                  key={s}
                  onClick={() => setSeverity(s)}
                  className={`flex-1 px-2 py-2 text-xs rounded-lg border transition-colors cursor-pointer ${
                    severity === s
                      ? s === 'Crash'
                        ? 'bg-red-500/20 text-red-400 border-red-500/50'
                        : s === 'Broken Feature'
                          ? 'bg-amber-500/20 text-amber-400 border-amber-500/50'
                          : 'bg-blue-500/20 text-blue-400 border-blue-500/50'
                      : 'bg-surface-900 text-slate-400 border-surface-600 hover:border-surface-500'
                  }`}
                >
                  {s}
                </button>
              ))}
            </div>
          </div>
        </div>

        {/* Info note */}
        <p className="text-[11px] text-slate-500">
          System info and recent logs will be automatically attached (scrubbed of sensitive data). Limited to 5 reports per day.
        </p>

        {/* Submit */}
        <button
          onClick={handleSubmit}
          disabled={!canSubmit || submitting}
          className="flex items-center justify-center gap-2 w-full px-4 py-2.5 bg-violet-600 hover:bg-violet-500 disabled:opacity-40 disabled:hover:bg-violet-600 text-white text-sm font-semibold rounded-lg transition-colors cursor-pointer"
        >
          {submitting ? (
            <><Loader2 className="w-4 h-4 animate-spin" /> Submitting...</>
          ) : (
            <><Send className="w-4 h-4" /> Submit Bug Report</>
          )}
        </button>
      </div>
    </div>
  )
}
