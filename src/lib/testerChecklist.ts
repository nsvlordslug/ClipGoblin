const DISMISS_KEY = 'tester_checklist_dismissed_v1'

export function isTesterChecklistDismissed(): boolean {
  try {
    return localStorage.getItem(DISMISS_KEY) === 'true'
  } catch {
    return false
  }
}

export function dismissTesterChecklist(): void {
  try {
    localStorage.setItem(DISMISS_KEY, 'true')
  } catch {
    // Storage can be unavailable in restricted webviews.
  }
}

export function resetTesterChecklistDismissal(): void {
  try {
    localStorage.removeItem(DISMISS_KEY)
  } catch {
    // Storage can be unavailable in restricted webviews.
  }
}
