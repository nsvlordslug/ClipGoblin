import { useEffect, useState } from 'react'
import { Save, FolderOpen, Info, Brain, Check, Loader2, X, Zap, Eye, Sun, Moon, Bookmark, Pencil, Trash2, HardDrive, ExternalLink, Gauge, Tv, LogOut, Download, Mic, Cpu } from 'lucide-react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import ConnectedAccounts from '../components/ConnectedAccounts'
import Tooltip from '../components/Tooltip'
import { useAiStore, PROVIDER_META, MODEL_OPTIONS, type AiProvider } from '../stores/aiStore'
import { useAppStore } from '../stores/appStore'
import { useUiStore } from '../stores/uiStore'
import { useTemplateStore } from '../stores/templateStore'
import { CAPTION_STYLES, EXPORT_PRESETS } from '../lib/editTypes'

const PROVIDERS: AiProvider[] = ['free', 'openai', 'claude', 'gemini']

/** Manage custom clip templates — rename and delete */
function TemplateManager() {
  const store = useTemplateStore()
  const [renamingId, setRenamingId] = useState<string | null>(null)
  const [renameValue, setRenameValue] = useState('')
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null)

  const customTemplates = store.templates.filter(t => !t.builtIn)
  const builtInTemplates = store.templates.filter(t => t.builtIn)

  const startRename = (id: string, currentName: string) => {
    setRenamingId(id)
    setRenameValue(currentName)
  }

  const commitRename = () => {
    if (renamingId && renameValue.trim()) {
      store.rename(renamingId, renameValue.trim())
    }
    setRenamingId(null)
  }

  const handleDelete = (id: string) => {
    store.remove(id)
    setConfirmDeleteId(null)
  }

  return (
    <section className="bg-surface-800 border border-surface-700 rounded-xl p-6">
      <div className="flex items-center gap-2 mb-1">
        <Bookmark className="w-5 h-5 text-violet-400" />
        <h2 className="text-lg font-semibold text-white">Clip Templates</h2>
      </div>
      <p className="text-sm text-slate-400 mb-5">
        Manage your saved editor presets. Built-in templates cannot be modified.
      </p>

      {/* Built-in templates (read-only list) */}
      {builtInTemplates.length > 0 && (
        <div className="mb-4">
          <p className="text-[10px] text-slate-500 uppercase tracking-wider font-semibold mb-2">Starter Templates</p>
          <div className="space-y-1">
            {builtInTemplates.map(tmpl => {
              const style = CAPTION_STYLES.find(s => s.id === tmpl.captionStyleId)
              const preset = EXPORT_PRESETS.find(p => p.id === tmpl.exportPresetId)
              return (
                <div key={tmpl.id} className="flex items-center justify-between px-3 py-2 bg-surface-900 rounded-lg">
                  <div>
                    <span className="text-sm text-slate-300">{tmpl.name}</span>
                    <span className="text-[10px] text-slate-500 ml-2">
                      {style?.name} &middot; {preset?.aspectRatio} &middot; {tmpl.hashtags.slice(0, 3).map(h => `#${h}`).join(' ')}
                    </span>
                  </div>
                  <span className="text-[9px] text-slate-600 italic">built-in</span>
                </div>
              )
            })}
          </div>
        </div>
      )}

      {/* Custom templates (editable) */}
      <p className="text-[10px] text-slate-500 uppercase tracking-wider font-semibold mb-2">My Templates</p>
      {customTemplates.length === 0 ? (
        <p className="text-xs text-slate-500 italic px-3 py-4">
          No custom templates yet. Save one from the Editor to get started.
        </p>
      ) : (
        <div className="space-y-1">
          {customTemplates.map(tmpl => {
            const style = CAPTION_STYLES.find(s => s.id === tmpl.captionStyleId)
            const preset = EXPORT_PRESETS.find(p => p.id === tmpl.exportPresetId)
            const isRenaming = renamingId === tmpl.id
            const isConfirmingDelete = confirmDeleteId === tmpl.id
            return (
              <div key={tmpl.id} className="flex items-center justify-between gap-2 px-3 py-2 bg-surface-900 rounded-lg">
                <div className="flex-1 min-w-0">
                  {isRenaming ? (
                    <input
                      type="text"
                      value={renameValue}
                      onChange={e => setRenameValue(e.target.value)}
                      onKeyDown={e => { if (e.key === 'Enter') commitRename(); if (e.key === 'Escape') setRenamingId(null) }}
                      onBlur={commitRename}
                      autoFocus
                      className="w-full px-2 py-0.5 bg-surface-800 border border-violet-500 rounded text-sm text-white focus:outline-none"
                    />
                  ) : (
                    <>
                      <span className="text-sm text-slate-300">{tmpl.name}</span>
                      <span className="text-[10px] text-slate-500 ml-2">
                        {style?.name} &middot; {preset?.aspectRatio} &middot; {tmpl.hashtags.slice(0, 3).map(h => `#${h}`).join(' ')}
                      </span>
                    </>
                  )}
                </div>
                <div className="flex items-center gap-1">
                  {isConfirmingDelete ? (
                    <>
                      <span className="text-[10px] text-red-400 mr-1">Delete?</span>
                      <button onClick={() => handleDelete(tmpl.id)}
                        className="px-2 py-0.5 rounded bg-red-600 text-white text-[10px] hover:bg-red-500 transition-colors cursor-pointer">
                        Yes
                      </button>
                      <button onClick={() => setConfirmDeleteId(null)}
                        className="px-2 py-0.5 rounded bg-surface-700 text-slate-400 text-[10px] hover:text-white transition-colors cursor-pointer">
                        No
                      </button>
                    </>
                  ) : (
                    <>
                      <Tooltip text="Rename template" position="left">
                        <button onClick={() => startRename(tmpl.id, tmpl.name)}
                          className="p-1 rounded text-slate-500 hover:text-white transition-colors cursor-pointer">
                          <Pencil className="w-3.5 h-3.5" />
                        </button>
                      </Tooltip>
                      <Tooltip text="Delete template" position="left">
                        <button onClick={() => setConfirmDeleteId(tmpl.id)}
                          className="p-1 rounded text-slate-500 hover:text-red-400 transition-colors cursor-pointer">
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      </Tooltip>
                    </>
                  )}
                </div>
              </div>
            )
          })}
        </div>
      )}
    </section>
  )
}

export default function SettingsPage() {
  const [dataDir, setDataDir] = useState('—')
  const [downloadDir, setDownloadDir] = useState('—')
  const [aiSaved, setAiSaved] = useState(false)
  const [testing, setTesting] = useState(false)
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null)
  const [storagePaths, setStoragePaths] = useState<{ exportsDir: string; downloadsDir: string; dataDir: string } | null>(null)
  const [openingFolder, setOpeningFolder] = useState<string | null>(null)
  const [sensitivity, setSensitivity] = useState<'low' | 'medium' | 'high'>('medium')
  const [sensitivitySaved, setSensitivitySaved] = useState(false)

  // Transcription model state
  const [modelStatus, setModelStatus] = useState<{ base: { downloaded: boolean }; medium: { downloaded: boolean } } | null>(null)
  const [activeModel, setActiveModel] = useState<'base' | 'medium'>('base')
  const [modelDownloading, setModelDownloading] = useState<string | null>(null)
  const [modelProgress, setModelProgress] = useState(0)
  const [modelDownloadedMb, setModelDownloadedMb] = useState(0)
  const [confirmDeleteModel, setConfirmDeleteModel] = useState<string | null>(null)

  const ai = useAiStore()
  const ui = useUiStore()
  const { loggedInUser, twitchLogin, twitchLogout, isLoading: twitchLoading } = useAppStore()
  const s = ai.settings
  const isByok = ai.isByok()
  const meta = PROVIDER_META[s.provider]
  const models = MODEL_OPTIONS[s.provider]

  // Active provider's key + model field names
  const keyField = `${s.provider}ApiKey` as keyof typeof s
  const modelField = `${s.provider}Model` as keyof typeof s

  useEffect(() => {
    const load = async () => {
      try {
        const info = await invoke<{ data_dir: string; db_path: string; version: string }>('get_app_info')
        setDataDir(info.data_dir)
        const dlDir = await invoke<string>('get_download_dir')
        setDownloadDir(dlDir)
        const paths = await invoke<{ exportsDir: string; downloadsDir: string; dataDir: string }>('get_storage_paths')
        setStoragePaths(paths)
        const sens = await invoke<string | null>('get_setting', { key: 'detection_sensitivity' })
        if (sens === 'low' || sens === 'high') setSensitivity(sens)
        // Load whisper model status
        const mStatus = await invoke<{ base: { downloaded: boolean }; medium: { downloaded: boolean } }>('check_model_status')
        setModelStatus(mStatus)
        const savedModel = await invoke<string | null>('get_setting', { key: 'whisper_model' })
        if (savedModel === 'base' || savedModel === 'medium') setActiveModel(savedModel)
      } catch { /* backend not ready */ }
      // Only load AI settings from DB if they haven't been loaded yet.
      // Re-loading on every Settings mount would overwrite in-memory changes
      // (e.g. keys the user just typed but auto-save hasn't flushed yet).
      if (!ai.loaded) {
        await ai.load()
      }
    }
    load()
  }, [])

  // Listen for model download progress
  useEffect(() => {
    if (!modelDownloading) return
    const unlisten = listen<{ model: string; percent: number; downloadedMb: number; totalMb: number }>(
      'model-download-progress',
      (event) => {
        setModelProgress(event.payload.percent)
        setModelDownloadedMb(Math.round(event.payload.downloadedMb))
        if (event.payload.percent >= 100) {
          setModelDownloading(null)
          setModelProgress(0)
          // Refresh status
          invoke<{ base: { downloaded: boolean }; medium: { downloaded: boolean } }>('check_model_status').then(setModelStatus).catch(() => {})
        }
      }
    )
    return () => { unlisten.then(fn => fn()) }
  }, [modelDownloading])

  const handleModelDownload = async (modelName: string) => {
    setModelDownloading(modelName)
    setModelProgress(0)
    setModelDownloadedMb(0)
    try {
      await invoke('download_model', { modelName })
      // Status refreshed via event listener
    } catch {
      setModelDownloading(null)
    }
  }

  const handleModelDelete = async (modelName: string) => {
    try {
      await invoke('delete_model', { modelName })
      const mStatus = await invoke<{ base: { downloaded: boolean }; medium: { downloaded: boolean } }>('check_model_status')
      setModelStatus(mStatus)
    } catch { /* best effort */ }
    setConfirmDeleteModel(null)
  }

  const handleModelSelect = async (modelName: 'base' | 'medium') => {
    setActiveModel(modelName)
    try {
      await invoke('save_setting', { key: 'whisper_model', value: modelName })
    } catch { /* best effort */ }
  }

  const handleSaveAi = async () => {
    try {
      await ai.save()
      setAiSaved(true)
      setTimeout(() => setAiSaved(false), 2000)
    } catch (err) { console.error('Failed to save AI settings:', err) }
  }

  const handleBrowseFolder = async () => {
    try {
      const path = await invoke<string | null>('pick_download_folder')
      if (path) setDownloadDir(path)
    } catch (err) { console.error('Failed to pick folder:', err) }
  }

  const handleOpenFolder = async (path: string) => {
    setOpeningFolder(path)
    try {
      await invoke('open_folder', { path })
    } catch (err) {
      console.error('Failed to open folder:', err)
    } finally {
      setTimeout(() => setOpeningFolder(null), 600)
    }
  }

  const inputCls = "w-full px-4 py-2.5 bg-surface-900 border border-surface-600 rounded-lg text-white text-sm placeholder-slate-500 focus:outline-none focus:border-violet-500 focus:ring-1 focus:ring-violet-500"
  const btnCls = "flex items-center gap-2 px-5 py-2.5 bg-violet-600 hover:bg-violet-500 disabled:opacity-50 disabled:cursor-not-allowed text-white text-sm font-medium rounded-lg transition-colors cursor-pointer"

  return (
    <div className="space-y-8 max-w-2xl">
      <h1 className="text-2xl font-bold text-white">Settings</h1>

      {/* Twitch Account */}
      <section className="bg-surface-800 border border-surface-700 rounded-xl p-6">
        <div className="flex items-center gap-2 mb-1">
          <Tv className="w-5 h-5 text-violet-400" />
          <h2 className="text-lg font-semibold text-white">Twitch Account</h2>
        </div>
        <p className="text-sm text-slate-400 mb-5">
          Connect your Twitch account to fetch VODs and analyze streams.
        </p>
        {loggedInUser ? (
          <div className="space-y-3">
            <div className="flex items-center gap-3 bg-emerald-500/10 border border-emerald-500/30 rounded-lg px-4 py-3">
              {loggedInUser.profile_image_url && (
                <img src={loggedInUser.profile_image_url} alt="" className="w-8 h-8 rounded-full" />
              )}
              <div className="flex-1">
                <p className="text-sm text-white font-medium">{loggedInUser.display_name}</p>
                <p className="text-xs text-emerald-400">Connected</p>
              </div>
              <button
                onClick={twitchLogout}
                className="flex items-center gap-1.5 px-3 py-1.5 bg-surface-900 border border-surface-600 rounded-lg text-xs text-slate-300 hover:text-red-400 hover:border-red-500/40 transition-colors cursor-pointer"
              >
                <LogOut className="w-3.5 h-3.5" />
                Disconnect
              </button>
            </div>
          </div>
        ) : (
          <button
            onClick={twitchLogin}
            disabled={twitchLoading}
            className={btnCls}
          >
            {twitchLoading ? <Loader2 className="w-4 h-4 animate-spin" /> : <Tv className="w-4 h-4" />}
            {twitchLoading ? 'Connecting...' : 'Connect Twitch'}
          </button>
        )}
      </section>

      {/* AI Provider */}
      <section className="bg-surface-800 border border-surface-700 rounded-xl p-6">
        <div className="flex items-center gap-2 mb-1">
          <Brain className="w-5 h-5 text-violet-400" />
          <h2 className="text-lg font-semibold text-white">AI Provider</h2>
        </div>
        <p className="text-sm text-slate-400 mb-5">
          Free mode is always available. BYOK only improves generation quality.
        </p>

        {/* Current status */}
        <div className={`flex items-center justify-between px-3 py-2 rounded-lg border mb-4 ${
          ai.isMisconfigured()
            ? 'bg-amber-500/10 border-amber-500/30'
            : ai.effectiveMode() !== 'free'
              ? 'bg-emerald-500/10 border-emerald-500/30'
              : 'bg-surface-900 border-surface-600'
        }`}>
          <div className="flex items-center gap-2">
            <div className={`w-2 h-2 rounded-full shrink-0 ${
              ai.isMisconfigured() ? 'bg-amber-400' : ai.effectiveMode() === 'free' ? 'bg-slate-500' : 'bg-emerald-400'
            }`} />
            <span className={`text-xs ${
              ai.isMisconfigured() ? 'text-amber-400' : ai.effectiveMode() !== 'free' ? 'text-emerald-400' : 'text-slate-400'
            }`}>{ai.statusText()}</span>
          </div>
          {ai.effectiveMode() !== 'free' && (
            <span className="text-[9px] text-slate-500">Usage billed through your API provider</span>
          )}
        </div>

        {/* Provider selector */}
        <div className="grid grid-cols-4 gap-2 mb-5">
          {PROVIDERS.map(id => {
            const m = PROVIDER_META[id]
            const hasKey = id !== 'free' && !!(s[`${id}ApiKey` as keyof typeof s])
            const tip = id === 'free' ? 'Pattern-based captions, no API key needed'
              : id === 'openai' ? 'Uses your OpenAI API key for higher quality captions'
              : id === 'claude' ? 'Uses your Anthropic API key for natural-sounding captions'
              : 'Uses your Google API key for caption generation'
            return (
              <Tooltip key={id} text={tip} position="bottom">
                <button onClick={() => { ai.update({ provider: id }); setTestResult(null) }}
                  className={`w-full px-3 py-3 rounded-lg text-center border transition-colors cursor-pointer relative ${
                    s.provider === id
                      ? 'bg-violet-600/20 border-violet-500/50 text-white'
                      : 'bg-surface-900 border-surface-600 text-slate-400 hover:text-white hover:border-surface-500'
                  }`}>
                  <div className="text-sm font-medium">{m.name}</div>
                  <div className="text-[10px] mt-0.5 opacity-60">{m.hint}</div>
                  {hasKey && s.provider !== id && (
                    <div className="absolute top-1.5 right-1.5 w-1.5 h-1.5 rounded-full bg-emerald-400" title="Key saved" />
                  )}
                </button>
              </Tooltip>
            )
          })}
        </div>

        {/* BYOK: API key + model selection */}
        {isByok && (
          <div className="space-y-4 mb-4">
            <div>
              <label className="block text-sm text-slate-300 mb-1.5">{meta.name} API Key</label>
              <input type="password"
                value={(s[keyField] as string) || ''}
                onChange={e => ai.update({ [keyField]: e.target.value })}
                placeholder={meta.keyPlaceholder}
                className={`${inputCls} font-mono`} />
            </div>

            {models.length > 0 && (
              <div>
                <label className="block text-sm text-slate-300 mb-1.5">Model</label>
                <select
                  value={(s[modelField] as string) || models[0].value}
                  onChange={e => ai.update({ [modelField]: e.target.value })}
                  className={inputCls}>
                  {models.map(m => (
                    <option key={m.value} value={m.value}>{m.label}</option>
                  ))}
                </select>
              </div>
            )}

            {/* Test connection + status */}
            {(s[keyField] as string) ? (
              <div className="flex items-center gap-3">
                <button
                  disabled={testing}
                  onClick={async () => {
                    setTesting(true)
                    setTestResult(null)
                    try {
                      await invoke<string>('test_ai_connection', {
                        provider: s.provider,
                        apiKey: s[keyField] as string,
                        model: (s[modelField] as string) || '',
                      })
                      setTestResult({ ok: true, message: 'Connected' })
                    } catch (err) {
                      setTestResult({ ok: false, message: String(err) })
                    } finally {
                      setTesting(false)
                    }
                  }}
                  className="flex items-center gap-1.5 px-3 py-1.5 bg-surface-900 border border-surface-600 rounded-lg text-xs text-slate-300 hover:text-white hover:border-violet-500/40 transition-colors cursor-pointer disabled:opacity-50">
                  {testing ? <Loader2 className="w-3 h-3 animate-spin" /> : <Zap className="w-3 h-3" />}
                  {testing ? 'Testing...' : 'Test Connection'}
                </button>

                {testResult && (
                  <div className={`flex items-center gap-1.5 text-xs ${testResult.ok ? 'text-emerald-400' : 'text-red-400'}`}>
                    {testResult.ok ? <Check className="w-3.5 h-3.5" /> : <X className="w-3.5 h-3.5" />}
                    <span>{testResult.message}</span>
                  </div>
                )}

                {!testResult && (
                  <span className="text-xs text-slate-500">Key saved — test to verify</span>
                )}
              </div>
            ) : (
              <div className="bg-amber-500/10 border border-amber-500/20 rounded-lg px-3 py-2 text-xs text-amber-400">
                No API key saved. The app will use Free mode until a key is added.
              </div>
            )}
          </div>
        )}

        {/* Free mode info */}
        {!isByok && (
          <div className="bg-surface-900 border border-surface-600 rounded-lg px-4 py-3 mb-4">
            <p className="text-sm text-slate-300">Free mode active</p>
            <p className="text-xs text-slate-500 mt-1">
              Clip detection, scoring, titles, and captions all work without an API key.
              BYOK providers improve AI analysis and caption generation quality.
            </p>
          </div>
        )}

        {/* Usage toggles + clip detection note */}
        {isByok && (
          <div className="space-y-3 mb-4">
            <p className="text-xs text-slate-500 mb-1">Use {meta.name} for:</p>

            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={s.useForCaptions}
                onChange={e => ai.update({ useForCaptions: e.target.checked })}
                className="w-4 h-4 rounded border-surface-600 bg-surface-900 text-violet-500 focus:ring-violet-500" />
              <span className="text-sm text-slate-300">Caption generation (TikTok copy)</span>
            </label>

            <label className="flex items-center gap-3 cursor-pointer">
              <input type="checkbox" checked={s.useForTitles}
                onChange={e => ai.update({ useForTitles: e.target.checked })}
                className="w-4 h-4 rounded border-surface-600 bg-surface-900 text-violet-500 focus:ring-violet-500" />
              <span className="text-sm text-slate-300">Title generation</span>
            </label>

            {/* Clip detection note */}
            <div className="flex items-center gap-2 text-xs text-slate-500 pt-2 border-t border-surface-700">
              <Info className="w-3.5 h-3.5 shrink-0" />
              <span>Clip detection always runs in Free mode for speed and zero cost.</span>
            </div>

            <label className="flex items-center gap-3 cursor-pointer mt-1">
              <input type="checkbox" checked={s.fallbackToFree}
                onChange={e => ai.update({ fallbackToFree: e.target.checked })}
                className="w-4 h-4 rounded border-surface-600 bg-surface-900 text-violet-500 focus:ring-violet-500" />
              <div>
                <span className="text-sm text-slate-300">Fall back to Free mode if API fails</span>
                <p className="text-[10px] text-slate-500">Recommended. Keeps the app working even if the API is down.</p>
              </div>
            </label>
          </div>
        )}

        <button onClick={handleSaveAi} className={btnCls}>
          <Save className="w-4 h-4" />
          {aiSaved ? 'Saved!' : 'Save AI Settings'}
        </button>
      </section>

      {/* UI Preferences */}
      <section className="bg-surface-800 border border-surface-700 rounded-xl p-6">
        <div className="flex items-center gap-2 mb-1">
          <Eye className="w-5 h-5 text-violet-400" />
          <h2 className="text-lg font-semibold text-white">UI Preferences</h2>
        </div>
        <p className="text-sm text-slate-400 mb-5">
          Customize how the interface looks and behaves.
        </p>
        <Tooltip text="Turn off hover tooltips once you're familiar with the app" position="right">
          <label className="flex items-center gap-3 cursor-pointer">
            <input
              type="checkbox"
              checked={ui.settings.showTooltips}
              onChange={e => ui.update({ showTooltips: e.target.checked })}
              className="w-4 h-4 rounded border-surface-600 bg-surface-900 text-violet-500 focus:ring-violet-500"
            />
            <div>
              <span className="text-sm text-slate-300">Show Tooltips</span>
              <p className="text-[10px] text-slate-500">Display helpful descriptions when hovering over buttons and controls</p>
            </div>
          </label>
        </Tooltip>

        {/* Theme toggle */}
        <div className="mt-5 pt-5 border-t border-surface-700">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              {ui.settings.theme === 'dark' ? <Moon className="w-4 h-4 text-violet-400" /> : <Sun className="w-4 h-4 text-amber-400" />}
              <div>
                <span className="text-sm text-slate-300">Theme</span>
                <p className="text-[10px] text-slate-500">Switch between dark and light color schemes</p>
              </div>
            </div>
            <Tooltip text={`Switch to ${ui.settings.theme === 'dark' ? 'light' : 'dark'} mode`} position="left">
              <button
                onClick={() => ui.update({ theme: ui.settings.theme === 'dark' ? 'light' : 'dark' })}
                className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-surface-900 border border-surface-600 text-sm text-slate-300 hover:text-white hover:border-surface-500 transition-colors cursor-pointer"
              >
                {ui.settings.theme === 'dark' ? (
                  <><Sun className="w-3.5 h-3.5" /> Light</>
                ) : (
                  <><Moon className="w-3.5 h-3.5" /> Dark</>
                )}
              </button>
            </Tooltip>
          </div>
        </div>
      </section>

      {/* Clip Templates */}
      <TemplateManager />

      {/* Clip Detection Sensitivity */}
      <section className="bg-surface-800 border border-surface-700 rounded-xl p-6">
        <div className="flex items-center gap-2 mb-1">
          <Gauge className="w-5 h-5 text-violet-400" />
          <h2 className="text-lg font-semibold text-white">Detection Sensitivity</h2>
        </div>
        <p className="text-sm text-slate-400 mb-5">
          Controls how many clips are found during VOD analysis. Higher sensitivity catches more subtle moments.
        </p>
        <div className="grid grid-cols-3 gap-2 mb-4">
          {([
            { id: 'low' as const, label: 'Low', desc: 'Fewer clips, only the best moments' },
            { id: 'medium' as const, label: 'Medium', desc: 'Balanced — recommended for most VODs' },
            { id: 'high' as const, label: 'High', desc: 'More clips, catches subtle moments' },
          ] as const).map(opt => (
            <button
              key={opt.id}
              onClick={async () => {
                setSensitivity(opt.id)
                try {
                  await invoke('save_setting', { key: 'detection_sensitivity', value: opt.id })
                  setSensitivitySaved(true)
                  setTimeout(() => setSensitivitySaved(false), 1500)
                } catch { /* best effort */ }
              }}
              className={`px-3 py-3 rounded-lg text-center border transition-colors cursor-pointer ${
                sensitivity === opt.id
                  ? 'bg-violet-600/20 border-violet-500/50 text-white'
                  : 'bg-surface-900 border-surface-600 text-slate-400 hover:text-white hover:border-surface-500'
              }`}
            >
              <div className="text-sm font-medium">{opt.label}</div>
              <div className="text-[10px] mt-0.5 opacity-60">{opt.desc}</div>
            </button>
          ))}
        </div>
        {sensitivitySaved && (
          <div className="flex items-center gap-1.5 text-xs text-emerald-400">
            <Check className="w-3.5 h-3.5" /> Saved — applies to next analysis
          </div>
        )}
      </section>

      {/* Transcription Model */}
      <section className="bg-surface-800 border border-surface-700 rounded-xl p-6">
        <div className="flex items-center gap-2 mb-1">
          <Mic className="w-5 h-5 text-violet-400" />
          <h2 className="text-lg font-semibold text-white">Transcription Model</h2>
        </div>
        <p className="text-sm text-slate-400 mb-5">
          Choose which AI model is used for speech recognition during VOD analysis.
        </p>

        <div className="grid grid-cols-2 gap-3 mb-4">
          {/* Base model card */}
          {([
            {
              id: 'base' as const,
              title: 'Base (Fast)',
              desc: 'Best for clear audio with a good microphone. Transcribes quickly \u2014 about 5\u201310 minutes per hour of video. Occasionally misses quiet words or mumbling.',
              size: '142 MB',
              sizeMb: 142,
              recommended: true,
            },
            {
              id: 'medium' as const,
              title: 'Medium (Accurate)',
              desc: 'Better at catching every word, even with background game audio. Takes 2\u20133x longer to transcribe. Choose this if the base model misses too many words.',
              size: '1.5 GB',
              sizeMb: 1500,
              recommended: false,
            },
          ]).map(model => {
            const downloaded = modelStatus?.[model.id]?.downloaded ?? false
            const isActive = activeModel === model.id
            const isDownloading = modelDownloading === model.id
            const isConfirmingDelete = confirmDeleteModel === model.id

            return (
              <div
                key={model.id}
                onClick={() => downloaded && handleModelSelect(model.id)}
                className={`relative rounded-xl border p-4 transition-colors ${
                  isActive
                    ? 'bg-violet-600/10 border-violet-500/50'
                    : 'bg-surface-900 border-surface-600 hover:border-surface-500'
                } ${downloaded ? 'cursor-pointer' : ''}`}
              >
                {/* Active indicator */}
                {isActive && downloaded && (
                  <div className="absolute top-3 right-3 w-3 h-3 rounded-full bg-violet-500 ring-2 ring-violet-500/30" />
                )}

                <div className="flex items-center gap-2 mb-2">
                  <span className="text-sm font-semibold text-white">{model.title}</span>
                  {model.recommended && (
                    <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-emerald-500/20 text-emerald-400 font-medium">
                      Recommended
                    </span>
                  )}
                </div>

                <p className="text-xs text-slate-400 mb-3 leading-relaxed">{model.desc}</p>

                <div className="flex items-center justify-between mb-3">
                  <span className="text-[10px] text-slate-500">Size: {model.size}</span>
                  {downloaded ? (
                    <span className="flex items-center gap-1 text-[10px] text-emerald-400">
                      <Check className="w-3 h-3" /> Downloaded
                    </span>
                  ) : (
                    <span className="text-[10px] text-slate-500">Not downloaded</span>
                  )}
                </div>

                {/* Download progress bar */}
                {isDownloading && (
                  <div className="mb-3">
                    <div className="w-full bg-surface-800 rounded-full h-2 border border-surface-700 overflow-hidden mb-1">
                      <div
                        className="h-full bg-gradient-to-r from-violet-600 to-violet-400 rounded-full transition-all duration-300"
                        style={{ width: `${Math.min(modelProgress, 100)}%` }}
                      />
                    </div>
                    <p className="text-[10px] text-slate-400">
                      {modelDownloadedMb} MB / {model.sizeMb} MB ({Math.min(modelProgress, 100)}%)
                    </p>
                  </div>
                )}

                {/* Action buttons */}
                <div onClick={e => e.stopPropagation()}>
                  {!downloaded && !isDownloading && (
                    <button
                      onClick={() => handleModelDownload(model.id)}
                      className="flex items-center gap-1.5 px-3 py-1.5 bg-violet-600 hover:bg-violet-500 text-white text-xs font-medium rounded-lg transition-colors cursor-pointer w-full justify-center"
                    >
                      <Download className="w-3 h-3" />
                      Download
                    </button>
                  )}
                  {isDownloading && (
                    <button
                      disabled
                      className="flex items-center gap-1.5 px-3 py-1.5 bg-surface-800 border border-surface-600 text-slate-400 text-xs rounded-lg w-full justify-center opacity-60"
                    >
                      <Loader2 className="w-3 h-3 animate-spin" />
                      Downloading...
                    </button>
                  )}
                  {downloaded && !isDownloading && (
                    <>
                      {isConfirmingDelete ? (
                        <div className="flex items-center gap-2">
                          <span className="text-[10px] text-red-400">Delete model?</span>
                          <button
                            onClick={() => handleModelDelete(model.id)}
                            className="px-2 py-0.5 rounded bg-red-600 text-white text-[10px] hover:bg-red-500 transition-colors cursor-pointer"
                          >
                            Yes
                          </button>
                          <button
                            onClick={() => setConfirmDeleteModel(null)}
                            className="px-2 py-0.5 rounded bg-surface-700 text-slate-400 text-[10px] hover:text-white transition-colors cursor-pointer"
                          >
                            No
                          </button>
                        </div>
                      ) : (
                        <button
                          onClick={() => setConfirmDeleteModel(model.id)}
                          className="flex items-center gap-1.5 px-3 py-1.5 bg-surface-800 border border-surface-600 text-slate-400 hover:text-red-400 hover:border-red-500/40 text-xs rounded-lg transition-colors cursor-pointer w-full justify-center"
                        >
                          <Trash2 className="w-3 h-3" />
                          Delete
                        </button>
                      )}
                    </>
                  )}
                </div>
              </div>
            )
          })}
        </div>

        {/* GPU note */}
        <div className="flex items-center gap-2 text-xs text-slate-500 pt-3 border-t border-surface-700">
          <Cpu className="w-3.5 h-3.5 shrink-0 text-violet-400" />
          <span>ClipGoblin automatically uses your NVIDIA GPU for faster transcription when available. No configuration needed.</span>
        </div>
      </section>

      {/* Connected Platforms */}
      <section className="bg-surface-800 border border-surface-700 rounded-xl p-6">
        <h2 className="text-lg font-semibold text-white mb-1">Publishing Accounts</h2>
        <p className="text-sm text-slate-400 mb-5">
          Connect your social accounts to publish clips directly from ClipGoblin.
        </p>
        <ConnectedAccounts />
      </section>

      {/* Download Location */}
      <section className="bg-surface-800 border border-surface-700 rounded-xl p-6">
        <h2 className="text-lg font-semibold text-white mb-1">Download Location</h2>
        <p className="text-sm text-slate-400 mb-5">Choose where downloaded VODs are saved.</p>
        <div className="flex items-center gap-3">
          <FolderOpen className="w-4 h-4 text-slate-400 shrink-0" />
          <span className="text-slate-400 truncate font-mono text-xs flex-1 min-w-0">{downloadDir}</span>
          <button onClick={handleBrowseFolder}
            className="flex items-center gap-2 px-4 py-2.5 bg-violet-600 hover:bg-violet-500 text-white text-sm font-medium rounded-lg transition-colors cursor-pointer shrink-0">
            Browse
          </button>
        </div>
      </section>

      {/* Storage Locations */}
      <section className="bg-surface-800 border border-surface-700 rounded-xl p-6">
        <div className="flex items-center gap-2 mb-1">
          <HardDrive className="w-5 h-5 text-violet-400" />
          <h2 className="text-lg font-semibold text-white">Storage Locations</h2>
        </div>
        <p className="text-sm text-slate-400 mb-5">
          Open the folders where ClipGoblin stores your files.
        </p>
        <div className="space-y-3">
          {([
            { label: 'Open Exports Folder', desc: 'Rendered clips ready to upload or share', path: storagePaths?.exportsDir },
            { label: 'Open Downloads Folder', desc: 'Downloaded Twitch VODs', path: storagePaths?.downloadsDir },
            { label: 'Open App Data Folder', desc: 'Database, thumbnails, transcripts, captions', path: storagePaths?.dataDir },
          ] as const).map(({ label, desc, path }) => (
            <div key={label} className="flex items-center gap-3 bg-surface-900 rounded-lg px-4 py-3">
              <div className="flex-1 min-w-0">
                <span className="text-sm text-slate-300">{label}</span>
                <p className="text-[10px] text-slate-500 mt-0.5">{desc}</p>
                {path && (
                  <p className="text-[10px] text-slate-600 font-mono mt-1 truncate" title={path}>{path}</p>
                )}
              </div>
              <button
                onClick={() => path && handleOpenFolder(path)}
                disabled={!path || openingFolder === path}
                className="flex items-center gap-1.5 px-3 py-2 bg-surface-800 border border-surface-600 rounded-lg text-xs text-slate-300 hover:text-white hover:border-violet-500/40 transition-colors cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed shrink-0"
              >
                {openingFolder === path ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <ExternalLink className="w-3.5 h-3.5" />}
                Open
              </button>
            </div>
          ))}
        </div>
      </section>

      {/* About */}
      <section className="bg-surface-800 border border-surface-700 rounded-xl p-6">
        <h2 className="text-lg font-semibold text-white mb-1">About</h2>
        <p className="text-sm text-slate-400 mb-4">
          ClipGoblin is a Twitch stream clip generator powered by AI analysis.
        </p>
        <div className="space-y-2 text-sm">
          <div className="flex gap-2">
            <span className="text-slate-300">Version:</span>
            <span className="text-slate-400">1.0.0</span>
          </div>
          <div className="flex gap-2">
            <span className="text-slate-300">Built with:</span>
            <span className="text-slate-400">Tauri 2 + React + TypeScript</span>
          </div>
        </div>
      </section>
    </div>
  )
}
