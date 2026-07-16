export type ExternalSourceKind = 'medal' | 'obs' | 'meld'

export interface ExternalSourceConfig {
  kind: ExternalSourceKind
  directory: string | null
  autoImport: boolean
}

export interface ExternalMediaCandidate {
  id: string
  name: string
  folderLabel: string
  path: string
  sizeBytes: number
  recordedAt: string
  importedClipId: string | null
}

export interface ExternalCandidateGroup {
  label: string
  candidates: ExternalMediaCandidate[]
  available: ExternalMediaCandidate[]
}

export const EXTERNAL_IMPORT_BATCH_SIZE = 50

export interface ImportedClipResult {
  clipId: string
  title: string
  sourceKind: string
  status: 'imported' | 'already_imported' | 'failed'
  error?: string
}

export interface RecorderStatus {
  kind: 'obs' | 'meld'
  reachable: boolean
  recording: boolean
  replayBufferActive: boolean
  detail: string
}

export const EXTERNAL_SOURCES: Array<{
  kind: ExternalSourceKind
  name: string
  shortName: string
  accent: string
}> = [
  { kind: 'medal', name: 'Medal clips', shortName: 'Medal', accent: '#f4c84a' },
  { kind: 'obs', name: 'OBS replays', shortName: 'OBS', accent: '#8b9bb4' },
  { kind: 'meld', name: 'Meld clips', shortName: 'Meld', accent: '#fb4f9b' },
]

export function toggleCandidate(
  selected: ReadonlySet<string>,
  candidateId: string,
): Set<string> {
  const next = new Set(selected)
  if (next.has(candidateId)) next.delete(candidateId)
  else next.add(candidateId)
  return next
}

export function selectableCandidates(candidates: ExternalMediaCandidate[]): ExternalMediaCandidate[] {
  return candidates.filter(candidate => !candidate.importedClipId)
}

export function groupCandidatesByFolder(
  candidates: ExternalMediaCandidate[],
): ExternalCandidateGroup[] {
  const groups = new Map<string, ExternalMediaCandidate[]>()
  for (const candidate of candidates) {
    const label = candidate.folderLabel?.trim() || 'Other clips'
    const existing = groups.get(label)
    if (existing) existing.push(candidate)
    else groups.set(label, [candidate])
  }
  return [...groups.entries()]
    .sort(([left], [right]) => left.localeCompare(right, undefined, { sensitivity: 'base' }))
    .map(([label, grouped]) => ({
      label,
      candidates: grouped,
      available: selectableCandidates(grouped),
    }))
}

export function chunkCandidateIds(
  candidateIds: string[],
  batchSize = EXTERNAL_IMPORT_BATCH_SIZE,
): string[][] {
  const safeBatchSize = Math.max(1, Math.floor(batchSize))
  const batches: string[][] = []
  for (let offset = 0; offset < candidateIds.length; offset += safeBatchSize) {
    batches.push(candidateIds.slice(offset, offset + safeBatchSize))
  }
  return batches
}

export function formatSourceBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 MB'
  const mb = bytes / (1024 * 1024)
  return mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${Math.max(0.1, mb).toFixed(1)} MB`
}
