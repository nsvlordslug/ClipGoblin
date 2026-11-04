export type ReleaseChangeKind = 'new' | 'improved' | 'fixed' | 'under-the-hood'

export interface ReleaseChangeGroup {
  kind: ReleaseChangeKind
  items: string[]
}

export interface ReleaseNote {
  version: string
  date: string
  summary: string
  changes: ReleaseChangeGroup[]
}

export const RELEASE_NOTES_SEEN_KEY = 'clipgoblin.release-notes.seen-version'
export const PENDING_UPDATE_KEY = 'clipgoblin.update.pending-version'

export const RELEASE_NOTES: ReleaseNote[] = [
  {
    version: '1.7.16',
    date: '2026-11-04',
    summary: 'A clearer finish to every ClipGoblin update.',
    changes: [
      {
        kind: 'new',
        items: [
          'After an update, ClipGoblin now opens a one-time What’s New window with the changes in that release.',
        ],
      },
      {
        kind: 'improved',
        items: [
          'If you skip one or more versions, the window rolls the missed release notes into one readable update history.',
          'The update history has dedicated Dark, White, and Zombie styling with keyboard and reduced-motion support.',
        ],
      },
      {
        kind: 'under-the-hood',
        items: [
          'ClipGoblin records the installed-from version before restarting, then remembers dismissal locally so each release appears only once.',
        ],
      },
    ],
  },
  {
    version: '1.7.15',
    date: '2026-11-03',
    summary: 'A reliable updater overlay in every theme.',
    changes: [
      {
        kind: 'fixed',
        items: [
          'The update panel no longer reshapes the dashboard or breaks the Install & restart control in the Zombie theme.',
        ],
      },
    ],
  },
  {
    version: '1.7.14',
    date: '2026-11-03',
    summary: 'Production dashboard action sizing.',
    changes: [
      {
        kind: 'fixed',
        items: [
          'Build Montage now remains fully visible when the development-only Import VOD action is absent.',
        ],
      },
    ],
  },
  {
    version: '1.7.13',
    date: '2026-11-03',
    summary: 'TikTok analytics and an optional new look.',
    changes: [
      {
        kind: 'new',
        items: [
          'Added the optional Neon Zombie theme while preserving the existing Dark and White themes.',
          'Added TikTok analytics support for connected accounts with the approved video-list permission.',
        ],
      },
    ],
  },
]

export interface PendingUpdate {
  fromVersion: string
  toVersion: string
}

export interface ReleaseNotesPresentation {
  currentVersion: string
  previousVersion: string | null
  entries: ReleaseNote[]
}

function versionParts(version: string): number[] {
  return version
    .replace(/^v/i, '')
    .split(/[.-]/)
    .map(part => Number.parseInt(part, 10))
    .map(part => Number.isFinite(part) ? part : 0)
}

export function compareVersions(left: string, right: string): number {
  const leftParts = versionParts(left)
  const rightParts = versionParts(right)
  const length = Math.max(leftParts.length, rightParts.length)

  for (let index = 0; index < length; index += 1) {
    const difference = (leftParts[index] ?? 0) - (rightParts[index] ?? 0)
    if (difference !== 0) return difference
  }
  return 0
}

export function rememberPendingUpdate(fromVersion: string, toVersion: string): void {
  try {
    localStorage.setItem(PENDING_UPDATE_KEY, JSON.stringify({ fromVersion, toVersion }))
  } catch {
    // Release notes are helpful, but an unavailable webview store must never block an update.
  }
}

function readPendingUpdate(): PendingUpdate | null {
  try {
    const raw = localStorage.getItem(PENDING_UPDATE_KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw) as Partial<PendingUpdate>
    if (typeof parsed.fromVersion !== 'string' || typeof parsed.toVersion !== 'string') return null
    return { fromVersion: parsed.fromVersion, toVersion: parsed.toVersion }
  } catch {
    return null
  }
}

function readSeenVersion(): string | null {
  try {
    return localStorage.getItem(RELEASE_NOTES_SEEN_KEY)
  } catch {
    return null
  }
}

export function getReleaseNotesPresentation(currentVersion: string): ReleaseNotesPresentation | null {
  const seenVersion = readSeenVersion()
  if (seenVersion && compareVersions(seenVersion, currentVersion) >= 0) return null

  const pendingUpdate = readPendingUpdate()
  const previousVersion = pendingUpdate?.toVersion === currentVersion
    ? pendingUpdate.fromVersion
    : seenVersion

  const entries = RELEASE_NOTES
    .filter(note => compareVersions(note.version, currentVersion) <= 0)
    .filter(note => previousVersion ? compareVersions(note.version, previousVersion) > 0 : note.version === currentVersion)
    .sort((left, right) => compareVersions(right.version, left.version))

  if (entries.length === 0) return null
  return { currentVersion, previousVersion, entries }
}

export function markReleaseNotesSeen(currentVersion: string): void {
  try {
    localStorage.setItem(RELEASE_NOTES_SEEN_KEY, currentVersion)
    localStorage.removeItem(PENDING_UPDATE_KEY)
  } catch {
    // The modal can still close for this session if storage is unavailable.
  }
}
