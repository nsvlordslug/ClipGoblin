import { useCallback, useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import {
  Check,
  CircleDot,
  FileVideo,
  FolderOpen,
  Import,
  Loader2,
  PlugZap,
  RefreshCw,
  Save,
  Video,
} from 'lucide-react'
import { useAppStore } from '../stores/appStore'
import {
  chunkCandidateIds,
  EXTERNAL_SOURCES,
  formatSourceBytes,
  groupCandidatesByFolder,
  selectableCandidates,
  toggleCandidate,
} from '../lib/externalSources'
import type {
  ExternalMediaCandidate,
  ExternalSourceConfig,
  ExternalSourceKind,
  ImportedClipResult,
  RecorderStatus,
} from '../lib/externalSources'

interface RecorderConnectionSettings {
  obsPort: number
  obsPasswordSet: boolean
}

const emptyCandidates: Record<ExternalSourceKind, ExternalMediaCandidate[]> = {
  medal: [],
  obs: [],
  meld: [],
}

const emptySelections: Record<ExternalSourceKind, Set<string>> = {
  medal: new Set(),
  obs: new Set(),
  meld: new Set(),
}

function sourceError(error: unknown): string {
  return String(error).replace(/^Error:\s*/i, '')
}

export default function ExternalSourcesPanel({ hidden = false }: { hidden?: boolean }) {
  const fetchClips = useAppStore(state => state.fetchClips)
  const fetchHighlights = useAppStore(state => state.fetchHighlights)
  const [configs, setConfigs] = useState<ExternalSourceConfig[]>([])
  const [candidates, setCandidates] = useState(emptyCandidates)
  const [selected, setSelected] = useState(emptySelections)
  const [busy, setBusy] = useState<string | null>('loading')
  const [notice, setNotice] = useState<{ ok: boolean; text: string } | null>(null)
  const [recorderSettings, setRecorderSettings] = useState<RecorderConnectionSettings>({
    obsPort: 4455,
    obsPasswordSet: false,
  })
  const [obsPort, setObsPort] = useState(4455)
  const [obsPassword, setObsPassword] = useState('')
  const [recorderStatus, setRecorderStatus] = useState<Partial<Record<'obs' | 'meld', RecorderStatus>>>({})

  const refreshLibrary = useCallback(async () => {
    await Promise.all([fetchClips(), fetchHighlights()])
  }, [fetchClips, fetchHighlights])

  const load = useCallback(async () => {
    try {
      const [sourceConfigs, connection] = await Promise.all([
        invoke<ExternalSourceConfig[]>('get_external_source_configs'),
        invoke<RecorderConnectionSettings>('get_recorder_connection_settings'),
      ])
      setConfigs(sourceConfigs)
      setRecorderSettings(connection)
      setObsPort(connection.obsPort)
    } catch (error) {
      setNotice({ ok: false, text: sourceError(error) })
    } finally {
      setBusy(null)
    }
  }, [])

  useEffect(() => { void load() }, [load])

  const configByKind = useMemo(
    () => Object.fromEntries(configs.map(config => [config.kind, config])) as Partial<Record<ExternalSourceKind, ExternalSourceConfig>>,
    [configs],
  )

  const pickFolder = async (kind: ExternalSourceKind) => {
    setBusy(`folder:${kind}`)
    setNotice(null)
    try {
      const path = await invoke<string | null>('pick_external_source_folder', { kind })
      if (path) {
        await load()
        setBusy(`folder:${kind}`)
        const rows = await invoke<ExternalMediaCandidate[]>('scan_external_source', { kind })
        setCandidates(current => ({ ...current, [kind]: rows }))
        setSelected(current => ({ ...current, [kind]: new Set() }))
        const sourceName = EXTERNAL_SOURCES.find(source => source.kind === kind)?.shortName
        const available = selectableCandidates(rows).length
        const scope = kind === 'medal' ? ' across all game folders' : ''
        const result = rows.length === 0
          ? `No clips found${scope}`
          : available === 0
            ? `${rows.length} clip${rows.length === 1 ? '' : 's'} found${scope}; all already imported`
            : `${available} clip${available === 1 ? '' : 's'} ready to import${scope}`
        setNotice({ ok: true, text: `${sourceName} folder saved. ${result}` })
      }
    } catch (error) {
      setNotice({ ok: false, text: sourceError(error) })
    } finally {
      setBusy(null)
    }
  }

  const scan = async (kind: ExternalSourceKind) => {
    setBusy(`scan:${kind}`)
    setNotice(null)
    try {
      const rows = await invoke<ExternalMediaCandidate[]>('scan_external_source', { kind })
      setCandidates(current => ({ ...current, [kind]: rows }))
      setSelected(current => ({ ...current, [kind]: new Set() }))
      const available = selectableCandidates(rows).length
      setNotice({
        ok: true,
        text: available === 0 ? 'No new clips found' : `${available} clip${available === 1 ? '' : 's'} ready to import`,
      })
    } catch (error) {
      setNotice({ ok: false, text: sourceError(error) })
    } finally {
      setBusy(null)
    }
  }

  const importCandidates = async (
    kind: ExternalSourceKind,
    candidateIds: string[],
    scopeLabel: string,
  ) => {
    const ids = [...new Set(candidateIds)]
    if (ids.length === 0) return
    setBusy(`import:${kind}`)
    let imported = 0
    let duplicates = 0
    let failed = 0
    let checked = 0
    let firstFailure: string | null = null
    try {
      const batches = chunkCandidateIds(ids)
      for (let index = 0; index < batches.length; index += 1) {
        setNotice({
          ok: true,
          text: `Importing ${scopeLabel}: batch ${index + 1} of ${batches.length} (${checked}/${ids.length} checked)`,
        })
        const result = await invoke<ImportedClipResult[]>('import_external_candidates', {
          kind,
          candidateIds: batches[index],
        })
        imported += result.filter(row => row.status === 'imported').length
        duplicates += result.filter(row => row.status === 'already_imported').length
        const failures = result.filter(row => row.status === 'failed')
        failed += failures.length
        firstFailure ??= failures.find(row => row.error)?.error ?? null
        checked += batches[index].length
      }
      await refreshLibrary()
      const rows = await invoke<ExternalMediaCandidate[]>('scan_external_source', { kind })
      setCandidates(current => ({ ...current, [kind]: rows }))
      setSelected(current => ({ ...current, [kind]: new Set() }))
      setNotice({
        ok: failed === 0,
        text: `${imported} clip${imported === 1 ? '' : 's'} imported${duplicates ? `, ${duplicates} already in the library` : ''}${failed ? `, ${failed} skipped${firstFailure ? ` (${firstFailure})` : ''}` : ''}`,
      })
    } catch (error) {
      await refreshLibrary().catch(() => undefined)
      try {
        const rows = await invoke<ExternalMediaCandidate[]>('scan_external_source', { kind })
        setCandidates(current => ({ ...current, [kind]: rows }))
      } catch {
        // Keep the last successful scan visible if recovery scanning also fails.
      }
      setNotice({
        ok: false,
        text: `${imported} imported before the batch stopped. ${sourceError(error)}`,
      })
    } finally {
      setBusy(null)
    }
  }

  const importSelected = async (kind: ExternalSourceKind) => {
    const ids = [...selected[kind]]
    await importCandidates(kind, ids, `${ids.length} selected clips`)
  }

  const importFiles = async () => {
    setBusy('manual-import')
    setNotice(null)
    try {
      const result = await invoke<ImportedClipResult[]>('pick_and_import_media')
      if (result.length > 0) {
        await refreshLibrary()
        const imported = result.filter(row => row.status === 'imported').length
        const duplicate = result.length - imported
        setNotice({
          ok: true,
          text: `${imported} imported${duplicate ? `, ${duplicate} already in the library` : ''}`,
        })
      }
    } catch (error) {
      setNotice({ ok: false, text: sourceError(error) })
    } finally {
      setBusy(null)
    }
  }

  const setAutoImport = async (kind: ExternalSourceKind, enabled: boolean) => {
    setBusy(`auto:${kind}`)
    setNotice(null)
    try {
      await invoke('set_external_source_auto_import', { kind, enabled })
      await load()
      setNotice({ ok: true, text: enabled ? 'Auto-import enabled for new clips' : 'Auto-import paused' })
    } catch (error) {
      setNotice({ ok: false, text: sourceError(error) })
    } finally {
      setBusy(null)
    }
  }

  const saveObs = async () => {
    setBusy('save:obs')
    setNotice(null)
    try {
      await invoke('save_obs_connection_settings', {
        port: obsPort,
        password: obsPassword.length > 0 ? obsPassword : null,
      })
      setObsPassword('')
      await load()
      setNotice({ ok: true, text: 'OBS connection saved securely' })
    } catch (error) {
      setNotice({ ok: false, text: sourceError(error) })
    } finally {
      setBusy(null)
    }
  }

  const clearObsPassword = async () => {
    setBusy('clear:obs')
    setNotice(null)
    try {
      await invoke('save_obs_connection_settings', { port: obsPort, password: '' })
      setObsPassword('')
      await load()
      setNotice({ ok: true, text: 'Saved OBS password removed' })
    } catch (error) {
      setNotice({ ok: false, text: sourceError(error) })
    } finally {
      setBusy(null)
    }
  }

  const testRecorder = async (kind: 'obs' | 'meld') => {
    setBusy(`test:${kind}`)
    setNotice(null)
    try {
      if (kind === 'obs' && (obsPort !== recorderSettings.obsPort || obsPassword.length > 0)) {
        const passwordChanged = obsPassword.length > 0
        await invoke('save_obs_connection_settings', {
          port: obsPort,
          password: passwordChanged ? obsPassword : null,
        })
        setRecorderSettings(current => ({
          obsPort,
          obsPasswordSet: passwordChanged ? true : current.obsPasswordSet,
        }))
        setObsPassword('')
      }
      const status = await invoke<RecorderStatus>('test_recorder_connection', { kind })
      setRecorderStatus(current => ({ ...current, [kind]: status }))
      setNotice({ ok: true, text: status.detail })
    } catch (error) {
      setRecorderStatus(current => ({ ...current, [kind]: undefined }))
      setNotice({ ok: false, text: sourceError(error) })
    } finally {
      setBusy(null)
    }
  }

  const markMoment = async (kind: 'obs' | 'meld') => {
    setBusy(`mark:${kind}`)
    setNotice(null)
    try {
      await invoke('create_stream_marker', { recorderKind: kind, label: null })
      setNotice({ ok: true, text: 'Moment marked for the matching Twitch VOD' })
    } catch (error) {
      setNotice({ ok: false, text: sourceError(error) })
    } finally {
      setBusy(null)
    }
  }

  const saveReplay = async (kind: 'obs' | 'meld') => {
    setBusy(`replay:${kind}`)
    setNotice(null)
    try {
      const result = await invoke<ImportedClipResult>('save_replay_and_import', { kind })
      await refreshLibrary()
      setNotice({
        ok: true,
        text: result.status === 'already_imported' ? 'Replay is already in the library' : `${result.title} imported`,
      })
    } catch (error) {
      setNotice({ ok: false, text: sourceError(error) })
    } finally {
      setBusy(null)
    }
  }

  return (
    <section className="v4-section" hidden={hidden}>
      <div className="flex items-start justify-between gap-4">
        <div>
          <h3 className="v4-section-label">
            <Import className="w-3.5 h-3.5 inline-block mr-1.5 text-violet-400" style={{ verticalAlign: -2 }} />
            Clip Sources
          </h3>
          <p className="text-sm text-slate-400">Local imports stay on this PC and use the same editor and publishing flow.</p>
        </div>
        <button
          type="button"
          onClick={() => void importFiles()}
          disabled={busy !== null}
          className="v4-btn primary shrink-0"
        >
          {busy === 'manual-import' ? <Loader2 className="w-4 h-4 animate-spin" /> : <FileVideo className="w-4 h-4" />}
          Import videos
        </button>
      </div>

      {notice && (
        <div role="status" className={`mt-4 rounded-md border px-3 py-2 text-xs ${notice.ok ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-300' : 'border-red-500/30 bg-red-500/10 text-red-300'}`}>
          {notice.text}
        </div>
      )}

      <div className="mt-5 divide-y divide-surface-700 border-y border-surface-700">
        {EXTERNAL_SOURCES.map(source => {
          const config = configByKind[source.kind]
          const rows = candidates[source.kind]
          const available = selectableCandidates(rows)
          const folderGroups = groupCandidatesByFolder(rows)
          const selectedCount = selected[source.kind].size
          const recorderKind = source.kind === 'obs' || source.kind === 'meld' ? source.kind : null
          const status = recorderKind ? recorderStatus[recorderKind] : undefined
          return (
            <div key={source.kind} className="py-5">
              <div className="flex items-start justify-between gap-4">
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="w-2 h-2 rounded-full" style={{ backgroundColor: source.accent }} />
                    <h4 className="text-sm font-semibold text-white">{source.name}</h4>
                    {status?.reachable && <Check className="w-3.5 h-3.5 text-emerald-400" aria-label="Connected" />}
                  </div>
                  <p className="mt-1 truncate text-xs text-slate-500" title={config?.directory || undefined}>
                    {config?.directory || 'No folder selected'}
                  </p>
                  {source.kind === 'medal' && (
                    <p className="mt-1 text-[11px] text-slate-500">
                      Choose the parent Medal capture folder; every game subfolder is included.
                    </p>
                  )}
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  <button
                    type="button"
                    className="v4-btn ghost"
                    onClick={() => void pickFolder(source.kind)}
                    disabled={busy !== null}
                    title={`Choose ${source.shortName} clips folder`}
                  >
                    {busy === `folder:${source.kind}` ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <FolderOpen className="w-3.5 h-3.5" />}
                    Folder
                  </button>
                  <button
                    type="button"
                    className="v4-btn ghost"
                    onClick={() => void scan(source.kind)}
                    disabled={busy !== null || !config?.directory}
                    title={`Scan ${source.shortName} folder`}
                    aria-label={`Scan ${source.shortName} folder`}
                  >
                    <RefreshCw className={`w-4 h-4 ${busy === `scan:${source.kind}` ? 'animate-spin' : ''}`} />
                    {source.kind === 'medal' ? 'Scan all games' : 'Scan folder'}
                  </button>
                </div>
              </div>

              {source.kind === 'obs' && (
                <div className="mt-4 grid grid-cols-[110px_minmax(180px,1fr)_auto] gap-2">
                  <label className="sr-only" htmlFor="obs-port">OBS WebSocket port</label>
                  <input
                    id="obs-port"
                    type="number"
                    min={1024}
                    max={65535}
                    value={obsPort}
                    onChange={event => setObsPort(Number(event.target.value))}
                    className="v4-input"
                    aria-label="OBS WebSocket port"
                  />
                  <label className="sr-only" htmlFor="obs-password">OBS WebSocket password</label>
                  <input
                    id="obs-password"
                    type="password"
                    value={obsPassword}
                    onChange={event => setObsPassword(event.target.value)}
                    placeholder={recorderSettings.obsPasswordSet ? 'Password saved' : 'WebSocket password'}
                    className="v4-input"
                  />
                  <button type="button" className="v4-btn ghost" onClick={() => void saveObs()} disabled={busy !== null}>
                    <Save className="w-3.5 h-3.5" /> Save
                  </button>
                  {recorderSettings.obsPasswordSet && (
                    <button
                      type="button"
                      className="col-start-2 justify-self-start text-[11px] text-slate-500 hover:text-red-300 disabled:opacity-50"
                      onClick={() => void clearObsPassword()}
                      disabled={busy !== null}
                    >
                      Forget saved password
                    </button>
                  )}
                </div>
              )}

              <div className="mt-4 flex flex-wrap items-center gap-2">
                <button
                  type="button"
                  role="switch"
                  aria-checked={config?.autoImport || false}
                  onClick={() => void setAutoImport(source.kind, !config?.autoImport)}
                  disabled={busy !== null || !config?.directory}
                  className={`v4-toggle ${config?.autoImport ? 'on' : ''}`}
                  aria-label={`Auto-import new ${source.shortName} clips`}
                  title={`Auto-import new ${source.shortName} clips`}
                />
                <span className="text-xs text-slate-400">Auto-import new clips</span>
                {recorderKind && (
                  <>
                    <span className="mx-1 h-4 w-px bg-surface-600" />
                    <button type="button" className="v4-btn ghost" onClick={() => void testRecorder(recorderKind)} disabled={busy !== null}>
                      {busy === `test:${recorderKind}` ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <PlugZap className="w-3.5 h-3.5" />}
                      Test
                    </button>
                    <button type="button" className="v4-btn ghost" onClick={() => void markMoment(recorderKind)} disabled={busy !== null}>
                      <CircleDot className="w-3.5 h-3.5" /> Mark moment
                    </button>
                    <button type="button" className="v4-btn primary" onClick={() => void saveReplay(recorderKind)} disabled={busy !== null}>
                      {busy === `replay:${recorderKind}` ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Video className="w-3.5 h-3.5" />}
                      Save replay
                    </button>
                  </>
                )}
              </div>

              {rows.length > 0 && (
                <div className="mt-4 overflow-hidden rounded-md border border-surface-700 bg-surface-900">
                  <div className="flex items-center justify-between border-b border-surface-700 px-3 py-2">
                    <label className="flex items-center gap-2 text-xs text-slate-300">
                      <input
                        type="checkbox"
                        checked={available.length > 0 && selectedCount === available.length}
                        onChange={event => setSelected(current => ({
                          ...current,
                          [source.kind]: event.target.checked
                            ? new Set(available.map(candidate => candidate.id))
                            : new Set(),
                        }))}
                      />
                      Select all {available.length} available
                    </label>
                    <button
                      type="button"
                      className="v4-btn primary"
                      disabled={available.length === 0 || busy !== null}
                      onClick={() => void (
                        selectedCount > 0
                          ? importSelected(source.kind)
                          : importCandidates(
                              source.kind,
                              available.map(candidate => candidate.id),
                              source.kind === 'medal' ? 'all game folders' : `all ${source.shortName} clips`,
                            )
                      )}
                    >
                      {busy === `import:${source.kind}` ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Import className="w-3.5 h-3.5" />}
                      {selectedCount > 0
                        ? selectedCount === available.length
                          ? `Import all (${available.length})`
                          : `Import selected (${selectedCount})`
                        : `Import all (${available.length})`}
                    </button>
                  </div>
                  <div className="max-h-52 overflow-y-auto">
                    {folderGroups.map(group => (
                      <div key={group.label} className="border-b border-surface-700 last:border-0">
                        <div className="sticky top-0 z-10 flex items-center justify-between gap-3 bg-surface-800 px-3 py-2">
                          <div className="flex min-w-0 items-center gap-2">
                            <FolderOpen className="h-3.5 w-3.5 shrink-0 text-amber-300" />
                            <span className="truncate text-xs font-semibold text-slate-200">{group.label}</span>
                            <span className="shrink-0 text-[10px] text-slate-500">
                              {group.available.length} new · {group.candidates.length} total
                            </span>
                          </div>
                          <button
                            type="button"
                            className="v4-btn ghost shrink-0"
                            disabled={group.available.length === 0 || busy !== null}
                            onClick={() => void importCandidates(
                              source.kind,
                              group.available.map(candidate => candidate.id),
                              group.label,
                            )}
                          >
                            <Import className="h-3.5 w-3.5" />
                            Import folder
                          </button>
                        </div>
                        {group.candidates.map(candidate => (
                          <label key={candidate.id} className={`flex items-center gap-3 border-t border-surface-800 px-3 py-2 ${candidate.importedClipId ? 'opacity-50' : 'hover:bg-surface-800/70'}`}>
                            <input
                              type="checkbox"
                              disabled={Boolean(candidate.importedClipId)}
                              checked={selected[source.kind].has(candidate.id)}
                              onChange={() => setSelected(current => ({
                                ...current,
                                [source.kind]: toggleCandidate(current[source.kind], candidate.id),
                              }))}
                            />
                            <FileVideo className="w-4 h-4 shrink-0 text-slate-500" />
                            <span className="min-w-0 flex-1 truncate text-xs text-slate-300" title={candidate.path}>{candidate.name}</span>
                            <span className="shrink-0 text-[10px] text-slate-500">{formatSourceBytes(candidate.sizeBytes)}</span>
                            {candidate.importedClipId && <span className="text-[10px] text-emerald-400">Imported</span>}
                          </label>
                        ))}
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )
        })}
      </div>
    </section>
  )
}
