import { useEffect, useState, useRef, type KeyboardEvent } from 'react'
import { Save, FolderOpen, FolderInput, Info, Brain, Check, Loader2, X, Zap, Sun, Moon, Bookmark, Pencil, Trash2, HardDrive, ExternalLink, Gauge, Tv, LogOut, Download, Mic, Cpu, AlertTriangle, ClipboardCopy, RotateCcw } from 'lucide-react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useLocation } from 'react-router-dom'
import ConnectedAccounts from '../components/ConnectedAccounts'
import ExternalSourcesPanel from '../components/ExternalSourcesPanel'
import Tooltip from '../components/Tooltip'
import { useAiStore, PROVIDER_META, MODEL_OPTIONS, type AiProvider } from '../stores/aiStore'
import { useAppStore } from '../stores/appStore'
import { useUiStore, tryAdvanceTapCounter, type TapCounterState } from '../stores/uiStore'
import { useTemplateStore } from '../stores/templateStore'
import { CAPTION_STYLES, EXPORT_PRESETS } from '../lib/editTypes'
import { requiresHighDetectionCostConsent, type DetectionSensitivity } from '../lib/detectionCostConsent'
import {
  getNextSettingsSection,
  resolveSettingsSection,
  type SettingsNavigationKey,
  type SettingsSectionId,
} from '../lib/settingsNavigation'
import { getPersonalizationStatusCopy, type PersonalizationStatus } from '../types/clipReview'
import { version as appVersion } from '../../package.json'

const PROVIDERS: AiProvider[] = ['free', 'openai', 'claude', 'gemini']
type CostConsentRequest = 'high-detection' | 'sonnet-final-pass'

const SETTINGS_SECTIONS: Array<{
  id: SettingsSectionId
  label: string
  description: string
  icon: typeof Tv
}> = [
  { id: 'account', label: 'Accounts', description: 'Twitch and publishing connections', icon: Tv },
  { id: 'sources', label: 'Clip Sources', description: 'Medal, OBS, Meld, and local video', icon: FolderInput },
  { id: 'detection', label: 'Detection', description: 'Clip finding and transcription', icon: Gauge },
  { id: 'ai', label: 'AI', description: 'BYOK provider and paid features', icon: Brain },
  { id: 'editing', label: 'Editing', description: 'Templates and editor behavior', icon: Pencil },
  { id: 'storage', label: 'Storage', description: 'Folders and local app data', icon: HardDrive },
  { id: 'appearance', label: 'Appearance', description: 'Theme, tooltips, and app info', icon: Sun },
]

/** Manage custom clip templates — rename and delete */
function TemplateManager({ hidden = false }: { hidden?: boolean }) {
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
    <section className="v4-section" hidden={hidden}>
      <h3 className="v4-section-label">
        <Bookmark className="w-3.5 h-3.5 inline-block mr-1.5 text-violet-400" style={{verticalAlign: -2}} />
        Clip Templates
      </h3>
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
  const location = useLocation()
  const [downloadDir, setDownloadDir] = useState('—')
  const [aiSaved, setAiSaved] = useState(false)
  const [testing, setTesting] = useState(false)
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null)
  const [storagePaths, setStoragePaths] = useState<{ exportsDir: string; downloadsDir: string; dataDir: string } | null>(null)
  const [openingFolder, setOpeningFolder] = useState<string | null>(null)
  const [sensitivity, setSensitivity] = useState<DetectionSensitivity>('medium')
  const [sensitivitySaved, setSensitivitySaved] = useState(false)
  const [costConsentRequest, setCostConsentRequest] = useState<CostConsentRequest | null>(null)
  const [useCommunityClips, setUseCommunityClips] = useState(true)
  const [aiClipDetection, setAiClipDetection] = useState(false)
  const [allowPerClipCamOverride, setAllowPerClipCamOverride] = useState(false)
  const [personalizationStatus, setPersonalizationStatus] = useState<PersonalizationStatus | null>(null)
  const [personalizationAction, setPersonalizationAction] = useState<'copy' | 'reset' | null>(null)
  const [personalizationNotice, setPersonalizationNotice] = useState<string | null>(null)
  const [confirmResetPersonalization, setConfirmResetPersonalization] = useState(false)
  const [activeSettingsSection, setActiveSettingsSection] = useState<SettingsSectionId>(() => (
    resolveSettingsSection((location.state as { settingsSection?: unknown } | null)?.settingsSection)
  ))

  useEffect(() => {
    setActiveSettingsSection(
      resolveSettingsSection((location.state as { settingsSection?: unknown } | null)?.settingsSection),
    )
  }, [location.state])

  // Transcription model state
  const [modelStatus, setModelStatus] = useState<{ base: { downloaded: boolean }; medium: { downloaded: boolean } } | null>(null)
  const [activeModel, setActiveModel] = useState<'base' | 'medium'>('base')
  const [modelDownloading, setModelDownloading] = useState<string | null>(null)
  const [modelProgress, setModelProgress] = useState(0)
  const [modelDownloadedMb, setModelDownloadedMb] = useState(0)
  const [confirmDeleteModel, setConfirmDeleteModel] = useState<string | null>(null)

  // Phase 6.0 — AI usage cost summary (populated on mount + after every analyze/regen)
  const [costSummary, setCostSummary] = useState<{ avgPerAnalyzeUsd: number; total30dUsd: number; vodCount: number } | null>(null)

  const ai = useAiStore()
  const ui = useUiStore()
  const tapStateRef = useRef<TapCounterState>({ count: 0, lastTap: 0 })
  const settingsContentRef = useRef<HTMLDivElement>(null)
  const settingsNavButtonRefs = useRef<Partial<Record<SettingsSectionId, HTMLButtonElement | null>>>({})

  const selectSettingsSection = (section: SettingsSectionId) => {
    setActiveSettingsSection(section)
    requestAnimationFrame(() => {
      settingsContentRef.current?.closest('main')?.scrollTo({ top: 0, left: 0 })
    })
  }

  const handleSettingsNavKeyDown = (
    event: KeyboardEvent<HTMLButtonElement>,
    current: SettingsSectionId,
  ) => {
    const supportedKeys: SettingsNavigationKey[] = [
      'ArrowDown',
      'ArrowLeft',
      'ArrowRight',
      'ArrowUp',
      'End',
      'Home',
    ]
    if (!supportedKeys.includes(event.key as SettingsNavigationKey)) return

    event.preventDefault()
    const next = getNextSettingsSection(current, event.key as SettingsNavigationKey)
    selectSettingsSection(next)
    requestAnimationFrame(() => settingsNavButtonRefs.current[next]?.focus())
  }

  const handleVersionTap = () => {
    const result = tryAdvanceTapCounter(
      tapStateRef.current,
      Date.now(),
      ui.settings.developerModeUnlocked,
    )
    tapStateRef.current = result.next
    if (result.shouldUnlock) {
      ui.update({ developerModeUnlocked: true })
    }
  }

  const { loggedInUser, twitchLogin, twitchLogout, isLoading: twitchLoading } = useAppStore()
  const s = ai.settings
  const isByok = ai.isByok()
  const meta = PROVIDER_META[s.provider]
  const models = MODEL_OPTIONS[s.provider]

  // Active provider's key + model field names
  const keyField = `${s.provider}ApiKey` as keyof typeof s
  const modelField = `${s.provider}Model` as keyof typeof s
  const detectionModel = s.provider === 'claude'
    ? s.claudeJudgeModel
    : ((s[modelField] as string) || 'Default model')
  const personalizationCopy = personalizationStatus
    ? getPersonalizationStatusCopy(personalizationStatus)
    : null

  useEffect(() => {
    const load = async () => {
      try {
        const dlDir = await invoke<string>('get_download_dir')
        setDownloadDir(dlDir)
        const paths = await invoke<{ exportsDir: string; downloadsDir: string; dataDir: string }>('get_storage_paths')
        setStoragePaths(paths)
        const sens = await invoke<string | null>('get_setting', { key: 'detection_sensitivity' })
        if (sens === 'low' || sens === 'high') setSensitivity(sens)
        const communityRaw = await invoke<string | null>('get_setting', { key: 'use_twitch_community_clips' })
        if (communityRaw === 'false') setUseCommunityClips(false)
        const aiClipRaw = await invoke<string | null>('get_setting', { key: 'ai_clip_detection_enabled' })
        if (aiClipRaw === 'true') setAiClipDetection(true)
      } catch (error) { console.error('Settings load failed:', error) }
      try {
        const allow = await invoke<boolean>('get_allow_per_clip_override')
        setAllowPerClipCamOverride(allow)
      } catch { /* default false */ }
      try {
        const status = await invoke<PersonalizationStatus>('get_personalization_status')
        setPersonalizationStatus(status)
      } catch (error) { console.error('get_personalization_status failed:', error) }
      // Load whisper model status (separate try/catch so earlier failures don't block it)
      try {
        console.log('About to call check_model_status')
        const mStatus = await invoke<{ base: { downloaded: boolean }; medium: { downloaded: boolean } }>('check_model_status')
        console.log('Model status response:', JSON.stringify(mStatus))
        setModelStatus(mStatus)
        const savedModel = await invoke<string | null>('get_setting', { key: 'whisper_model' })
        if (savedModel === 'base' || savedModel === 'medium') setActiveModel(savedModel)
      } catch (error) { console.error('check_model_status failed:', error) }
      // Only load AI settings from DB if they haven't been loaded yet.
      // Re-loading on every Settings mount would overwrite in-memory changes
      // (e.g. keys the user just typed but auto-save hasn't flushed yet).
      const aiState = useAiStore.getState()
      if (!aiState.loaded) {
        await aiState.load()
      }
      // Phase 6.0 — fetch rolling cost summary for Settings display.
      try {
        const summary = await invoke<{ avgPerAnalyzeUsd: number; total30dUsd: number; vodCount: number }>(
          'get_ai_cost_summary',
          { lookbackVods: 10 },
        )
        setCostSummary(summary)
      } catch (error) { console.error('get_ai_cost_summary failed:', error) }
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

  const applySensitivity = async (next: DetectionSensitivity) => {
    const previous = sensitivity
    setSensitivity(next)
    try {
      await invoke('save_setting', { key: 'detection_sensitivity', value: next })
      setSensitivitySaved(true)
      setTimeout(() => setSensitivitySaved(false), 1500)
    } catch (error) {
      setSensitivity(previous)
      console.error('Failed to save detection sensitivity:', error)
    }
  }

  const handleSensitivitySelect = (next: DetectionSensitivity) => {
    if (requiresHighDetectionCostConsent({
      currentSensitivity: sensitivity,
      nextSensitivity: next,
      byokProviderSelected: isByok,
    })) {
      setCostConsentRequest('high-detection')
      return
    }
    void applySensitivity(next)
  }

  const handleCopyPersonalizationHistory = async () => {
    setPersonalizationAction('copy')
    setPersonalizationNotice(null)
    try {
      const historyJson = await invoke<string>('export_personalization_history')
      await navigator.clipboard.writeText(historyJson)
      setPersonalizationNotice('Learning history copied. It contains no media or API keys.')
    } catch (error) {
      setPersonalizationNotice(`Could not copy learning history: ${String(error)}`)
    } finally {
      setPersonalizationAction(null)
    }
  }

  const handleResetPersonalizationHistory = async () => {
    setPersonalizationAction('reset')
    setPersonalizationNotice(null)
    try {
      await invoke('reset_personalization_history')
      const status = await invoke<PersonalizationStatus>('get_personalization_status')
      setPersonalizationStatus(status)
      setConfirmResetPersonalization(false)
      setPersonalizationNotice('Personalized detection history reset.')
    } catch (error) {
      setPersonalizationNotice(`Could not reset learning history: ${String(error)}`)
    } finally {
      setPersonalizationAction(null)
    }
  }

  const handleCostConsentAccept = () => {
    const request = costConsentRequest
    setCostConsentRequest(null)
    if (request === 'high-detection') {
      void applySensitivity('high')
    } else if (request === 'sonnet-final-pass') {
      ai.update({ useSonnetFinalPass: true })
    }
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

  const inputCls = "v4-input"
  const btnCls = "flex items-center gap-2 px-5 py-2.5 bg-violet-600 hover:bg-violet-500 disabled:opacity-50 disabled:cursor-not-allowed text-white text-sm font-medium rounded-lg transition-colors cursor-pointer"

  return (
    <div className="v4-settings-page">
      <div className="v4-page-header">
        <div>
          <div className="v4-page-title">Settings</div>
          <div className="v4-page-sub">Accounts, detection, editing, storage, and appearance</div>
        </div>
      </div>

      <div className="v4-settings-layout">
        <nav className="v4-settings-nav" aria-label="Settings sections">
          {SETTINGS_SECTIONS.map(section => {
            const SectionIcon = section.icon
            const isActive = activeSettingsSection === section.id
            return (
              <button
                key={section.id}
                ref={node => { settingsNavButtonRefs.current[section.id] = node }}
                type="button"
                className={`v4-settings-nav-button ${isActive ? 'active' : ''}`}
                aria-current={isActive ? 'page' : undefined}
                onClick={() => selectSettingsSection(section.id)}
                onKeyDown={event => handleSettingsNavKeyDown(event, section.id)}
              >
                <SectionIcon className="v4-settings-nav-icon" aria-hidden="true" />
                <span className="min-w-0">
                  <span className="v4-settings-nav-title">{section.label}</span>
                  <span className="v4-settings-nav-description">{section.description}</span>
                </span>
              </button>
            )
          })}
        </nav>

        <div ref={settingsContentRef} className="v4-settings-content">

      {/* Twitch Account */}
      <section className="v4-section" hidden={activeSettingsSection !== 'account'}>
        <h3 className="v4-section-label">Connected accounts</h3>

        {/* Twitch row */}
        <div className="v4-setting-row">
          <div className="v4-setting-info flex items-center gap-2.5">
            <span
              className="w-6 h-6 rounded-md flex items-center justify-center text-[11px] font-bold shrink-0"
              style={{background: 'rgba(145,70,255,0.15)', color: '#9146FF', border: '1px solid rgba(145,70,255,0.4)'}}
            >
              {loggedInUser?.profile_image_url ? (
                <img src={loggedInUser.profile_image_url} alt="" className="w-full h-full rounded-md object-cover" />
              ) : '🟣'}
            </span>
            <div className="min-w-0">
              <div className="v4-setting-name flex items-center gap-2">
                <Tv className="w-3.5 h-3.5 text-violet-400" />
                Twitch
              </div>
              <div className="v4-setting-desc">
                {loggedInUser
                  ? `@${loggedInUser.twitch_login} · VODs fetched automatically`
                  : 'Connect to fetch VODs and analyze streams'}
              </div>
            </div>
          </div>
          {loggedInUser ? (
            <div className="flex items-center gap-2">
              <span className="v4-connected-pill">● CONNECTED</span>
              <Tooltip text="Disconnect your Twitch account" position="left">
                <button
                  onClick={twitchLogout}
                  className="v4-btn ghost"
                  style={{padding: '6px 12px', fontSize: 12}}
                >
                  <LogOut className="w-3 h-3" />
                  Disconnect
                </button>
              </Tooltip>
            </div>
          ) : (
            <button
              onClick={twitchLogin}
              disabled={twitchLoading}
              className="v4-btn primary"
              style={{padding: '6px 12px', fontSize: 12}}
            >
              {twitchLoading ? <Loader2 className="w-3 h-3 animate-spin" /> : <Tv className="w-3 h-3" />}
              {twitchLoading ? 'Connecting...' : 'Connect'}
            </button>
          )}
        </div>

        {/* YouTube / TikTok / Instagram rows */}
        <ConnectedAccounts />
      </section>

      <ExternalSourcesPanel hidden={activeSettingsSection !== 'sources'} />

      {/* AI Provider */}
      <section className="v4-section" hidden={activeSettingsSection !== 'ai'}>
        <h3 className="v4-section-label">
          <Brain className="w-3.5 h-3.5 inline-block mr-1.5 text-violet-400" style={{verticalAlign: -2}} />
          AI Provider (BYOK)
        </h3>
        <div className="v4-settings-cost-note">
          <Info className="w-4 h-4 shrink-0" />
          <span>Free mode has no API charges. BYOK usage is billed directly by your provider, and higher-cost options require confirmation.</span>
        </div>
        <div className="v4-setting-row">
          <div className="v4-setting-info">
            <div className="v4-setting-name flex items-center gap-2">
              <Brain className="w-4 h-4 text-violet-400" />
              {PROVIDER_META[s.provider].name}
            </div>
            <div className="v4-setting-desc">
              {ai.statusText()}
              {ai.effectiveMode() !== 'free' && ' · Usage billed through your API provider'}
            </div>
          </div>
          <span
            className={`v4-connected-pill ${
              ai.isMisconfigured() ? 'offline' : ai.effectiveMode() === 'free' ? 'idle' : ''
            }`}
          >
            ● {ai.isMisconfigured() ? 'MISCONFIGURED' : ai.effectiveMode() === 'free' ? 'IDLE' : 'ACTIVE'}
          </span>
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

            {/* Claude-only: clip-detection (judge) model + Sonnet final pass.
                Separate from the model dropdown above, which drives titles/captions. */}
            {s.provider === 'claude' && (
              <>
                <div>
                  <label className="block text-sm text-slate-300 mb-1.5">Clip-detection model</label>
                  <select
                    value={s.claudeJudgeModel}
                    onChange={e => ai.update({ claudeJudgeModel: e.target.value })}
                    className={inputCls}>
                    <option value="claude-sonnet-4-6">Claude Sonnet 4.6 (recommended — best clip quality)</option>
                    <option value="claude-haiku-4-5-20251001">Claude Haiku 4.5 (economy — cheaper, best for gameplay-heavy VODs)</option>
                  </select>
                </div>

                <label className="flex items-center gap-3 cursor-pointer">
                  <input type="checkbox" checked={s.useSonnetFinalPass}
                    onChange={e => {
                      if (e.target.checked) {
                        setCostConsentRequest('sonnet-final-pass')
                      } else {
                        ai.update({ useSonnetFinalPass: false })
                      }
                    }}
                    className="w-4 h-4 rounded border-surface-600 bg-surface-900 text-violet-500 focus:ring-violet-500" />
                  <div>
                    <span className="text-sm text-slate-300">Sonnet final pass</span>
                    <p className="text-[10px] text-slate-500">Optional paid final review. Enabling it requires cost consent.</p>
                  </div>
                </label>
              </>
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

            {/* Phase 6.0 — rolling cost summary from ai_usage_log */}
            {costSummary && costSummary.vodCount > 0 && (
              <div className="flex items-start gap-2 text-xs text-slate-400 pt-2 border-t border-surface-700">
                <Gauge className="w-3.5 h-3.5 shrink-0 mt-0.5 text-slate-500" />
                <div>
                  <div>
                    <span className="text-slate-300">~${costSummary.avgPerAnalyzeUsd.toFixed(3)}</span> per VOD analyze
                    <span className="text-slate-500"> (avg of last {costSummary.vodCount})</span>
                  </div>
                  <div className="text-slate-500">
                    ${costSummary.total30dUsd.toFixed(2)} spent in the last 30 days
                  </div>
                </div>
              </div>
            )}
            {costSummary && costSummary.vodCount === 0 && (
              <div className="flex items-center gap-2 text-xs text-slate-500 pt-2 border-t border-surface-700">
                <Gauge className="w-3.5 h-3.5 shrink-0" />
                <span>Cost estimate will appear after your first BYOK analyze.</span>
              </div>
            )}

            {/* Clip detection note */}
            <div className="flex items-center gap-2 text-xs text-slate-500 pt-2 border-t border-surface-700">
              <Info className="w-3.5 h-3.5 shrink-0" />
              <span>Core clip detection runs free and locally. The optional AI clip judge below (off by default) uses your own AI provider and may cost a few cents per VOD.</span>
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

      {/* Detection */}
      <section className="v4-section" hidden={activeSettingsSection !== 'detection'}>
        <h3 className="v4-section-label">
          <Gauge className="w-3.5 h-3.5 inline-block mr-1.5 text-violet-400" style={{verticalAlign: -2}} />
          Detection
        </h3>
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
              onClick={() => handleSensitivitySelect(opt.id)}
              className={`px-3 py-3 rounded-lg text-center border transition-colors cursor-pointer ${
                sensitivity === opt.id
                  ? 'bg-violet-600/20 border-violet-500/50 text-white'
                  : 'bg-surface-900 border-surface-600 text-slate-400 hover:text-white hover:border-surface-500'
              }`}
            >
              <div className="text-sm font-medium">{opt.label}</div>
              <div className="text-[10px] mt-0.5 opacity-60">{opt.desc}</div>
              {opt.id === 'high' && isByok && (
                <div className="text-[10px] mt-1.5 text-amber-300">Higher BYOK usage</div>
              )}
            </button>
          ))}
        </div>
        {sensitivitySaved && (
          <div className="flex items-center gap-1.5 text-xs text-emerald-400 mb-2">
            <Check className="w-3.5 h-3.5" /> Saved — applies to next analysis
          </div>
        )}

        {/* Use Twitch community clips as a detection signal */}
        <div className="v4-setting-row">
          <div className="v4-setting-info">
            <div className="v4-setting-name">Use Twitch community clips</div>
            <div className="v4-setting-desc">
              Boost highlights where viewers already made a Twitch clip. Human-curated signal · no extra scope needed.
            </div>
          </div>
          <button
            type="button"
            onClick={async () => {
              const next = !useCommunityClips
              setUseCommunityClips(next)
              try {
                await invoke('save_setting', { key: 'use_twitch_community_clips', value: next ? 'true' : 'false' })
              } catch { /* best effort */ }
            }}
            className={`v4-toggle ${useCommunityClips ? 'on' : ''}`}
            aria-label="Toggle Twitch community clip signal"
            aria-pressed={useCommunityClips}
          />
        </div>

        <div className="v4-setting-row">
          <div className="v4-setting-info">
            <div className="v4-setting-name">Personalized detection feedback</div>
            <div className="v4-setting-desc">
              Ratings and edit issues tune future ranking and clip boundaries. Opens, trims, exports, publishes, and deletes add lighter local evidence. Normal quality gates always stay in charge.
              {personalizationCopy && (
                <span className={`block mt-1 ${
                  personalizationCopy.tone === 'active'
                    ? 'text-emerald-400'
                    : personalizationCopy.tone === 'learning'
                      ? 'text-violet-300'
                      : personalizationCopy.tone === 'attention'
                        ? 'text-amber-300'
                        : 'text-slate-500'
                }`}>
                  {personalizationCopy.label}: {personalizationCopy.detail}
                </span>
              )}
              {ui.settings.showReviewTools && (
                <span className="mt-2 flex flex-wrap items-center gap-2">
                  <button
                    type="button"
                    className="v4-btn ghost"
                    onClick={() => void handleCopyPersonalizationHistory()}
                    disabled={personalizationAction !== null}
                  >
                    {personalizationAction === 'copy' ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <ClipboardCopy className="w-3.5 h-3.5" />}
                    Copy history
                  </button>
                  {!confirmResetPersonalization ? (
                    <button
                      type="button"
                      className="v4-btn ghost"
                      onClick={() => {
                        setConfirmResetPersonalization(true)
                        setPersonalizationNotice(null)
                      }}
                      disabled={personalizationAction !== null}
                    >
                      <RotateCcw className="w-3.5 h-3.5" />
                      Reset learning
                    </button>
                  ) : (
                    <>
                      <button
                        type="button"
                        className="v4-btn danger"
                        onClick={() => void handleResetPersonalizationHistory()}
                        disabled={personalizationAction !== null}
                      >
                        {personalizationAction === 'reset' ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Trash2 className="w-3.5 h-3.5" />}
                        Confirm reset
                      </button>
                      <button
                        type="button"
                        className="v4-btn ghost"
                        onClick={() => setConfirmResetPersonalization(false)}
                        disabled={personalizationAction !== null}
                      >
                        Keep history
                      </button>
                    </>
                  )}
                  {personalizationNotice && (
                    <span role="status" className="basis-full text-[11px] text-slate-400">
                      {personalizationNotice}
                    </span>
                  )}
                </span>
              )}
            </div>
          </div>
          <button
            type="button"
            onClick={() => ui.update({ showReviewTools: !ui.settings.showReviewTools })}
            className={`v4-toggle ${ui.settings.showReviewTools ? 'on' : ''}`}
            aria-label="Toggle personalized detection feedback"
            aria-pressed={ui.settings.showReviewTools}
          />
        </div>

        {/* AI clip detection (opt-in, BYOK) */}
        <div className="v4-setting-row">
          <div className="v4-setting-info">
            <div className="v4-setting-name">AI clip detection</div>
            <div className="v4-setting-desc">
              Reads your transcript with your AI provider to surface banter/plays/scares the signals miss and skip loud-but-empty moments. Off by default · uses your configured provider.
              {aiClipDetection && ai.isByok() && (
                <span className="block mt-1 text-amber-400/80">
                  ≈ $0.01–0.10 per VOD depending on your model — cheaper models (Haiku, Gemini Flash, GPT-4o-mini) are pennies.
                </span>
              )}
              {!ai.isByok() && (
                <span className="block mt-1 text-slate-500">
                  Requires an AI provider.
                  <button
                    type="button"
                    onClick={() => selectSettingsSection('ai')}
                    className="ml-1 font-semibold text-violet-300 hover:text-violet-200 cursor-pointer"
                  >
                    Open AI settings
                  </button>
                </span>
              )}
            </div>
          </div>
          <button
            type="button"
            disabled={!ai.isByok()}
            onClick={async () => {
              const next = !aiClipDetection
              setAiClipDetection(next)
              try {
                await invoke('save_setting', { key: 'ai_clip_detection_enabled', value: next ? 'true' : 'false' })
              } catch { /* best effort */ }
            }}
            className={`v4-toggle ${aiClipDetection ? 'on' : ''}`}
            style={{ opacity: ai.isByok() ? 1 : 0.4 }}
            aria-label="Toggle AI clip detection"
            aria-pressed={aiClipDetection}
          />
        </div>

        {/* Auto-ship high-confidence */}
        <div className="v4-setting-row">
          <div className="v4-setting-info">
            <div className="v4-setting-name">Auto-ship high-confidence</div>
            <div className="v4-setting-desc">Ship clips scoring 90%+ without review</div>
          </div>
          <button
            type="button"
            onClick={() => ui.update({ autoShipHighConfidence: !ui.settings.autoShipHighConfidence })}
            className={`v4-toggle ${ui.settings.autoShipHighConfidence ? 'on' : ''}`}
            aria-label="Toggle auto-ship high-confidence clips"
            aria-pressed={ui.settings.autoShipHighConfidence}
          />
        </div>

        {/* Use GPU (CUDA) */}
        <div className="v4-setting-row">
          <div className="v4-setting-info">
            <div className="v4-setting-name">Use GPU (CUDA)</div>
            <div className="v4-setting-desc">Faster transcription · requires CUDA 12</div>
          </div>
          <button
            type="button"
            onClick={() => ui.update({ useGpu: !ui.settings.useGpu })}
            className={`v4-toggle ${ui.settings.useGpu ? 'on' : ''}`}
            aria-label="Toggle GPU acceleration"
            aria-pressed={ui.settings.useGpu}
          />
        </div>

      </section>

      {/* Transcription Model */}
      <section className="v4-section" hidden={activeSettingsSection !== 'detection'}>
        <h3 className="v4-section-label">
          <Mic className="w-3.5 h-3.5 inline-block mr-1.5 text-violet-400" style={{verticalAlign: -2}} />
          Transcription Model
        </h3>
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
                className={`relative rounded-xl border p-4 transition-colors ${
                  isActive && downloaded
                    ? 'bg-emerald-500/5 border-emerald-500/40'
                    : downloaded
                      ? 'bg-surface-900 border-surface-600'
                      : 'bg-surface-900 border-surface-600'
                }`}
              >
                {/* Header: title + badges */}
                <div className="flex items-center gap-2 mb-2">
                  <span className="text-sm font-semibold text-white">{model.title}</span>
                  {model.recommended && (
                    <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-emerald-500/20 text-emerald-400 font-medium">
                      Recommended
                    </span>
                  )}
                  {isActive && downloaded && (
                    <span className="flex items-center gap-1 text-[9px] px-1.5 py-0.5 rounded-full bg-emerald-500/20 text-emerald-400 font-medium">
                      <Check className="w-2.5 h-2.5" /> Active
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

                {/* State 3: Not downloaded — Download button */}
                {!downloaded && !isDownloading && (
                  <button
                    onClick={() => handleModelDownload(model.id)}
                    className="flex items-center gap-1.5 px-3 py-1.5 bg-violet-600 hover:bg-violet-500 text-white text-xs font-medium rounded-lg transition-colors cursor-pointer w-full justify-center"
                  >
                    <Download className="w-3 h-3" />
                    Download
                  </button>
                )}

                {/* Downloading state */}
                {isDownloading && (
                  <button
                    disabled
                    className="flex items-center gap-1.5 px-3 py-1.5 bg-surface-800 border border-surface-600 text-slate-400 text-xs rounded-lg w-full justify-center opacity-60"
                  >
                    <Loader2 className="w-3 h-3 animate-spin" />
                    Downloading...
                  </button>
                )}

                {/* State 2: Downloaded but not active — Use + Delete */}
                {downloaded && !isDownloading && !isActive && (
                  <div className="flex items-center gap-2">
                    <button
                      onClick={() => handleModelSelect(model.id)}
                      className="flex-1 flex items-center gap-1.5 px-3 py-1.5 bg-violet-600 hover:bg-violet-500 text-white text-xs font-medium rounded-lg transition-colors cursor-pointer justify-center"
                    >
                      Use This Model
                    </button>
                    {isConfirmingDelete ? (
                      <div className="flex items-center gap-1">
                        <button
                          onClick={() => handleModelDelete(model.id)}
                          className="px-2 py-1 rounded bg-red-600 text-white text-[10px] hover:bg-red-500 transition-colors cursor-pointer"
                        >
                          Yes
                        </button>
                        <button
                          onClick={() => setConfirmDeleteModel(null)}
                          className="px-2 py-1 rounded bg-surface-700 text-slate-400 text-[10px] hover:text-white transition-colors cursor-pointer"
                        >
                          No
                        </button>
                      </div>
                    ) : (
                      <button
                        onClick={() => setConfirmDeleteModel(model.id)}
                        className="px-2 py-1.5 text-slate-500 hover:text-red-400 text-[10px] transition-colors cursor-pointer"
                      >
                        Delete
                      </button>
                    )}
                  </div>
                )}

                {/* State 1: Active + Downloaded — subtle Delete only */}
                {downloaded && !isDownloading && isActive && (
                  <div className="flex items-center justify-end">
                    {isConfirmingDelete ? (
                      <div className="flex items-center gap-2">
                        <span className="text-[10px] text-red-400">Delete active model?</span>
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
                        className="px-2 py-1 text-slate-500 hover:text-red-400 text-[10px] transition-colors cursor-pointer"
                      >
                        Delete
                      </button>
                    )}
                  </div>
                )}
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

      {/* Storage */}
      <section className="v4-section" hidden={activeSettingsSection !== 'storage'}>
        <h3 className="v4-section-label">
          <HardDrive className="w-3.5 h-3.5 inline-block mr-1.5 text-violet-400" style={{verticalAlign: -2}} />
          Storage
        </h3>

        {/* VOD download path */}
        <div className="v4-setting-row">
          <div className="v4-setting-info">
            <div className="v4-setting-name">VOD download path</div>
            <div className="v4-setting-desc font-mono truncate" title={downloadDir}>
              {downloadDir || '—'}
            </div>
          </div>
          <button onClick={handleBrowseFolder}
            className="v4-btn"
            style={{padding: '6px 12px', fontSize: 12}}
          >
            <FolderOpen className="w-3 h-3" />
            Change
          </button>
        </div>

        {/* Storage locations — Exports, Downloads, App Data */}
        {([
          { label: 'Exports folder', desc: 'Rendered clips ready to upload or share', path: storagePaths?.exportsDir },
          { label: 'Downloads folder', desc: 'Downloaded Twitch VODs', path: storagePaths?.downloadsDir },
          { label: 'App data folder', desc: 'Database, thumbnails, transcripts, captions', path: storagePaths?.dataDir },
        ] as const).map(({ label, desc, path }) => (
          <div key={label} className="v4-setting-row">
            <div className="v4-setting-info">
              <div className="v4-setting-name">{label}</div>
              <div className="v4-setting-desc">
                {desc}
                {path && <span className="font-mono block text-slate-600 mt-0.5 truncate" title={path}>{path}</span>}
              </div>
            </div>
            <button
              onClick={() => path && handleOpenFolder(path)}
              disabled={!path || openingFolder === path}
              className="v4-btn"
              style={{padding: '6px 12px', fontSize: 12}}
            >
              {openingFolder === path ? <Loader2 className="w-3 h-3 animate-spin" /> : <ExternalLink className="w-3 h-3" />}
              Open
            </button>
          </div>
        ))}
      </section>

      {/* UI Preferences */}
      <section className="v4-section" hidden={activeSettingsSection !== 'appearance'}>
        <h3 className="v4-section-label">
          <Sun className="w-3.5 h-3.5 inline-block mr-1.5 text-violet-400" style={{verticalAlign: -2}} />
          Appearance
        </h3>
        <div className="v4-setting-row">
          <div className="v4-setting-info">
            <div className="v4-setting-name">Show Tooltips</div>
            <div className="v4-setting-desc">Display helpful descriptions when hovering over buttons and controls</div>
          </div>
          <button
            type="button"
            onClick={() => ui.update({ showTooltips: !ui.settings.showTooltips })}
            className={`v4-toggle ${ui.settings.showTooltips ? 'on' : ''}`}
            aria-label="Toggle tooltips"
          />
        </div>
        <div className="v4-setting-row">
          <div className="v4-setting-info flex items-center gap-3">
            {ui.settings.theme === 'dark'
              ? <Moon className="w-4 h-4 text-violet-400 shrink-0" />
              : <Sun className="w-4 h-4 text-amber-400 shrink-0" />}
            <div>
              <div className="v4-setting-name">Theme</div>
              <div className="v4-setting-desc">Switch between dark and light color schemes</div>
            </div>
          </div>
          <Tooltip text={`Switch to ${ui.settings.theme === 'dark' ? 'light' : 'dark'} mode`} position="left">
            <button
              onClick={() => ui.update({ theme: ui.settings.theme === 'dark' ? 'light' : 'dark' })}
              className="v4-btn"
              style={{padding: '6px 12px', fontSize: 12}}
            >
              {ui.settings.theme === 'dark'
                ? <><Sun className="w-3 h-3" /> Light</>
                : <><Moon className="w-3 h-3" /> Dark</>}
            </button>
          </Tooltip>
        </div>
      </section>

      {/* Editor behavior */}
      <section className="v4-section" hidden={activeSettingsSection !== 'editing'}>
        <h3 className="v4-section-label">
          <Pencil className="w-3.5 h-3.5 inline-block mr-1.5 text-violet-400" style={{verticalAlign: -2}} />
          Editor Behavior
        </h3>
        <div className="v4-setting-row">
          <div className="v4-setting-info">
            <div className="v4-setting-name">Per-clip cam region overrides</div>
            <div className="v4-setting-desc">
              Let individual clips override the VOD's camera region. Existing overrides stay saved while this is off and are ignored during export.
            </div>
          </div>
          <button
            type="button"
            onClick={async () => {
              const next = !allowPerClipCamOverride
              setAllowPerClipCamOverride(next)
              try { await invoke('set_allow_per_clip_override', { enabled: next }) }
              catch (err) {
                console.error('[Settings] set_allow_per_clip_override failed', err)
                setAllowPerClipCamOverride(!next)
              }
            }}
            className={`v4-toggle ${allowPerClipCamOverride ? 'on' : ''}`}
            aria-label="Toggle per-clip cam region overrides"
            aria-pressed={allowPerClipCamOverride}
          />
        </div>
      </section>

      {/* Clip Templates */}
      <TemplateManager hidden={activeSettingsSection !== 'editing'} />

      {/* About */}
      <section className="v4-section" hidden={activeSettingsSection !== 'appearance'}>
        <h3 className="v4-section-label">
          <Info className="w-3.5 h-3.5 inline-block mr-1.5 text-violet-400" style={{verticalAlign: -2}} />
          About
        </h3>
        <p className="text-sm text-slate-400 mb-4">
          ClipGoblin is a local-first Twitch clip generator with optional bring-your-own-key AI.
        </p>
        <div className="space-y-2 text-sm">
          <div className="flex gap-2">
            <span className="text-slate-300">Version:</span>
            {/* Deliberately non-accessible: hidden developer-mode unlock gesture (7 taps within 2s). No role, aria-label, or keyboard handler — discoverability is the anti-goal. */}
            <span
              className="text-slate-400 cursor-default select-none"
              onClick={handleVersionTap}
            >
              {appVersion}
            </span>
          </div>
          <div className="flex gap-2">
            <span className="text-slate-300">Built with:</span>
            <span className="text-slate-400">Tauri 2 + React + TypeScript</span>
          </div>
        </div>
      </section>

        </div>
      </div>

      {costConsentRequest && (
        <div className="fixed inset-0 z-[80] flex items-center justify-center bg-black/70 p-4 backdrop-blur-sm">
          <div
            role="dialog"
            aria-modal="true"
            aria-labelledby="byok-cost-consent-title"
            aria-describedby="byok-cost-consent-description"
            className="w-full max-w-lg rounded-lg border border-amber-400/30 bg-surface-900 shadow-2xl"
          >
            <div className="flex items-start gap-3 border-b border-surface-700 px-5 py-4">
              <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-amber-400/10">
                <AlertTriangle className="h-5 w-5 text-amber-300" />
              </div>
              <div>
                <h2 id="byok-cost-consent-title" className="text-base font-semibold text-white">
                  {costConsentRequest === 'high-detection'
                    ? 'High detection may increase BYOK cost'
                    : 'Sonnet final pass adds paid AI usage'}
                </h2>
                <p id="byok-cost-consent-description" className="mt-1 text-xs leading-5 text-slate-400">
                  Review the cost impact before changing this setting. Nothing changes unless you accept.
                </p>
              </div>
            </div>

            <div className="space-y-4 px-5 py-4 text-sm text-slate-300">
              {costConsentRequest === 'high-detection' ? (
                <>
                  <p className="leading-6">
                    High lowers the quality threshold and allows roughly 75% more output clips than Medium. More clips can mean more paid title-generation calls when BYOK titles are enabled.
                  </p>
                  <div className="rounded-lg border border-surface-700 bg-surface-950 px-4 py-3">
                    <div className="flex justify-between gap-4 text-xs">
                      <span className="text-slate-500">AI provider</span>
                      <span className="text-right text-slate-200">{meta.name}</span>
                    </div>
                    <div className="mt-2 flex justify-between gap-4 text-xs">
                      <span className="text-slate-500">Detection model</span>
                      <span className="max-w-[65%] text-right text-slate-200">{detectionModel}</span>
                    </div>
                    <div className="mt-2 flex justify-between gap-4 text-xs">
                      <span className="text-slate-500">AI titles</span>
                      <span className="text-right text-slate-200">{s.useForTitles ? 'Enabled' : 'Disabled'}</span>
                    </div>
                  </div>
                  <p className="text-xs leading-5 text-slate-400">
                    High does not silently replace your selected model. The transcript judge still runs once; the main increase comes from allowing more clips and any per-clip AI work they trigger.
                  </p>
                </>
              ) : (
                <>
                  <p className="leading-6">
                    This option can add one Claude Sonnet final-review request after the first clip judge. It reviews the strongest candidate snippets to sharpen the final choices.
                  </p>
                  <p className="text-xs leading-5 text-slate-400">
                    It only runs when AI clip detection is enabled, the first judge uses a cheaper Claude model, and at least two moments are found. Provider pricing and token usage determine the charge.
                  </p>
                </>
              )}

              {costSummary && costSummary.vodCount > 0 && (
                <div className="rounded-lg border border-amber-400/20 bg-amber-400/5 px-4 py-3 text-xs leading-5 text-amber-100">
                  Your measured BYOK average is ${costSummary.avgPerAnalyzeUsd.toFixed(3)} per analyzed VOD across the last {costSummary.vodCount}. This is historical usage, not a guaranteed price for the next VOD.
                </div>
              )}

              <p className="text-xs leading-5 text-slate-500">
                ClipGoblin does not charge for this setting. Any usage is billed directly by {meta.name} through your API key, and actual cost varies with VOD length, model, and clip count.
              </p>
            </div>

            <div className="flex justify-end gap-2 border-t border-surface-700 px-5 py-4">
              <button
                type="button"
                autoFocus
                onClick={() => setCostConsentRequest(null)}
                className="rounded-lg border border-surface-600 bg-surface-800 px-4 py-2 text-sm font-medium text-slate-200 transition-colors hover:bg-surface-700"
              >
                Deny
              </button>
              <button
                type="button"
                onClick={handleCostConsentAccept}
                className="rounded-lg bg-amber-400 px-4 py-2 text-sm font-semibold text-slate-950 transition-colors hover:bg-amber-300"
              >
                Accept
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
