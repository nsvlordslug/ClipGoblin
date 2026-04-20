import { useEffect, Component, type ReactNode } from 'react'
import { Routes, Route, NavLink } from 'react-router-dom'
import { usePlatformStore } from './stores/platformStore'
import { useAiStore } from './stores/aiStore'
import { useUiStore } from './stores/uiStore'
import { useTemplateStore } from './stores/templateStore'
import { Sun, Moon } from 'lucide-react'
import Tooltip from './components/Tooltip'
import FirstRunSetup from './components/FirstRunSetup'
import BinariesSetup from './components/BinariesSetup'
import UpdateChecker from './components/UpdateChecker'
import { useStreamStatus, formatViewerCount } from './hooks/useStreamStatus'
import logoImg from './assets/logo.png'
import { version } from '../package.json'
import Dashboard from './pages/Dashboard'
import Vods from './pages/Vods'
import Clips from './pages/Clips'
import SettingsPage from './pages/Settings'
import Player from './pages/Player'
import Editor from './pages/Editor'
import MontageBuilder from './pages/MontageBuilder'
import Results from './pages/Results'
import ScheduledUploads from './pages/ScheduledUploads'
import HelpGuide from './pages/HelpGuide'
import BugReport from './pages/BugReport'
import Analytics from './pages/Analytics'
import { useScheduleStore } from './stores/scheduleStore'
import { useAppStore } from './stores/appStore'

const mainNavItems = [
  { to: '/', label: 'Dashboard', icon: '⊞', badgeKind: 'none' as const },
  { to: '/vods', label: 'VODs', icon: '📺', badgeKind: 'live' as const },
  { to: '/clips', label: 'Clips', icon: '✂', badgeKind: 'new' as const },
  { to: '/scheduled', label: 'Scheduled', icon: '🕒', badgeKind: 'scheduled' as const },
  { to: '/montage', label: 'Montage', icon: '🎬', badgeKind: 'none' as const },
  { to: '/analytics', label: 'Analytics', icon: '📈', badgeKind: 'none' as const },
]

const accountNavItems = [
  { to: '/settings', label: 'Settings', icon: '⚙', badgeKind: 'none' as const },
  { to: '/bug-report', label: 'Report Bug', icon: '🐛', badgeKind: 'none' as const },
  { to: '/help', label: 'Help', icon: '?', badgeKind: 'none' as const },
]

// ── Error Boundary ──
interface EBProps { children: ReactNode }
interface EBState { hasError: boolean; error: Error | null }

class ErrorBoundary extends Component<EBProps, EBState> {
  state: EBState = { hasError: false, error: null }

  static getDerivedStateFromError(error: Error): EBState {
    return { hasError: true, error }
  }

  render() {
    if (this.state.hasError) {
      const msg = this.state.error?.message || 'An unexpected error occurred.'
      const stack = this.state.error?.stack || ''
      const buildTag = `v${version}${import.meta.env.DEV ? ' · dev' : ''}`
      const report = `Build: ${buildTag}\nError: ${msg}\n\n${stack}`
      return (
        <div className="flex items-center justify-center h-screen bg-surface-950">
          <div className="text-center max-w-xl px-6">
            <div className="text-5xl mb-4">💀</div>
            <h1 className="text-xl font-bold text-white mb-2">Something went wrong</h1>
            <p className="text-sm text-slate-400 mb-1">{msg}</p>
            <p className="text-[11px] text-slate-600 font-mono mb-6">Build {buildTag}</p>
            <div className="flex gap-2 justify-center">
              <button
                onClick={() => window.location.reload()}
                className="px-5 py-2.5 bg-violet-600 hover:bg-violet-500 text-white rounded-lg text-sm font-medium transition-colors cursor-pointer"
              >
                Reload App
              </button>
              <button
                onClick={() => {
                  navigator.clipboard.writeText(report).catch(() => {})
                }}
                className="px-5 py-2.5 bg-surface-800 hover:bg-surface-700 border border-surface-700 text-white rounded-lg text-sm font-medium transition-colors cursor-pointer"
                title="Copy error details for the Bug Report page"
              >
                Copy details
              </button>
              <button
                onClick={() => { window.location.href = '/bug-report' }}
                className="px-5 py-2.5 bg-surface-800 hover:bg-surface-700 border border-surface-700 text-white rounded-lg text-sm font-medium transition-colors cursor-pointer"
              >
                Report
              </button>
            </div>
          </div>
        </div>
      )
    }
    return this.props.children
  }
}

export default function App() {
  const loadPlatforms = usePlatformStore(s => s.load)
  const loadAi = useAiStore(s => s.load)
  const loadUi = useUiStore(s => s.load)
  const loadTemplates = useTemplateStore(s => s.load)
  const loadSchedules = useScheduleStore(s => s.load)
  const startScheduleListening = useScheduleStore(s => s.startListening)
  const theme = useUiStore(s => s.settings.theme)
  const updateUi = useUiStore(s => s.update)
  const vods = useAppStore(s => s.vods)
  const highlights = useAppStore(s => s.highlights)
  const loggedInUser = useAppStore(s => s.loggedInUser)
  const scheduledUploads = useScheduleStore(s => s.uploads)
  const liveVodCount = vods.filter(v => v.analysis_status === 'analyzing').length
  const reviewCount = highlights.filter(h => (h.confidence_score ?? h.virality_score) < 0.85).length
  const pendingScheduledCount = scheduledUploads.filter(u => u.status === 'pending' || u.status === 'uploading').length
  const streamStatus = useStreamStatus(loggedInUser?.id ?? null)
  const badgeFor = (kind: 'live' | 'new' | 'scheduled' | 'none'): {text: string, color: string} | null => {
    if (kind === 'live' && liveVodCount > 0) return {text: `${liveVodCount} LIVE`, color: 'text-red-300 bg-red-500/20'}
    if (kind === 'new' && reviewCount > 0) return {text: `${reviewCount} NEW`, color: 'text-pink-300 bg-pink-500/20'}
    if (kind === 'scheduled' && pendingScheduledCount > 0) return {text: String(pendingScheduledCount), color: 'text-amber-300 bg-amber-500/20'}
    return null
  }
  useEffect(() => {
    loadPlatforms(); loadAi(); loadUi(); loadTemplates(); loadSchedules()
    const unlisten = startScheduleListening()
    return unlisten
  }, [loadPlatforms, loadAi, loadUi, loadTemplates, loadSchedules, startScheduleListening])

  const toggleTheme = () => updateUi({ theme: theme === 'dark' ? 'light' : 'dark' })

  return (
    <BinariesSetup>
    <FirstRunSetup>
    <div className="flex h-screen overflow-hidden">
      {/* Sidebar */}
      <aside className="w-60 shrink-0 bg-surface-900 border-r border-surface-700 flex flex-col">
        {/* ── Branding block (v4 compact) ── */}
        <div className="v4-sidebar-brand">
          <div className="v4-sidebar-logo">
            <img src={logoImg} alt="ClipGoblin" />
          </div>
          <div>
            <div className="v4-sidebar-name">
              <span className="text-clip">Clip</span><span className="text-goblin">Goblin</span>
            </div>
            <div className="v4-sidebar-sub">v{version} · CLIP ENGINE</div>
          </div>
        </div>

        {/* ── Navigation ── */}
        <nav className="flex-1 px-3 py-4 space-y-1">
          {mainNavItems.map(({ to, label, icon, badgeKind }) => {
            const badge = badgeFor(badgeKind)
            return (
              <NavLink
                key={to}
                to={to}
                end={to === '/'}
                className={({ isActive }) =>
                  `flex items-center gap-3 px-4 py-2.5 rounded-lg text-sm font-medium transition-colors ${
                    isActive
                      ? 'bg-violet-600/20 text-violet-400'
                      : 'text-slate-400 hover:text-slate-200 hover:bg-surface-800'
                  }`
                }
              >
                <span className="v4-nav-icon">{icon}</span>
                <span className="flex-1">{label}</span>
                {badge && (
                  <span className={`text-[10px] font-bold px-2 py-0.5 rounded-full ${badge.color}`}>
                    {badge.text}
                  </span>
                )}
              </NavLink>
            )
          })}

          <div className="pt-4 pb-1 px-4 text-[10px] font-bold tracking-[0.15em] text-slate-600 uppercase">Account</div>

          {accountNavItems.map(({ to, label, icon, badgeKind }) => {
            const badge = badgeFor(badgeKind)
            return (
              <NavLink
                key={to}
                to={to}
                end={to === '/'}
                className={({ isActive }) =>
                  `flex items-center gap-3 px-4 py-2.5 rounded-lg text-sm font-medium transition-colors ${
                    isActive
                      ? 'bg-violet-600/20 text-violet-400'
                      : 'text-slate-400 hover:text-slate-200 hover:bg-surface-800'
                  }`
                }
              >
                <span className="v4-nav-icon">{icon}</span>
                <span className="flex-1">{label}</span>
                {badge && (
                  <span className={`text-[10px] font-bold px-2 py-0.5 rounded-full ${badge.color}`}>
                    {badge.text}
                  </span>
                )}
              </NavLink>
            )
          })}
        </nav>

        {/* ── Channel card (v4) ── */}
        {loggedInUser && (
          <div className="v4-sidebar-channel">
            <div className="avatar">
              {loggedInUser.profile_image_url
                ? <img src={loggedInUser.profile_image_url} alt={loggedInUser.display_name} />
                : null}
            </div>
            <div style={{flex: 1, minWidth: 0}}>
              <div className="text-[13px] font-bold text-white truncate">@{loggedInUser.twitch_login}</div>
              {streamStatus.is_live ? (
                <div className="flex items-center gap-1.5 text-[11px] text-red-400" title={streamStatus.title ?? undefined}>
                  <span
                    className="inline-block w-1.5 h-1.5 rounded-full bg-red-400"
                    style={{boxShadow: '0 0 6px #f87171', animation: 'pulse 1.5s infinite'}}
                  />
                  <span className="truncate">
                    LIVE · {formatViewerCount(streamStatus.viewer_count)} viewer{streamStatus.viewer_count === 1 ? '' : 's'}
                  </span>
                </div>
              ) : (
                <div className="text-[11px] text-slate-500 truncate">{loggedInUser.display_name}</div>
              )}
            </div>
          </div>
        )}
      </aside>

      {/* Main content */}
      <main className="flex-1 overflow-y-auto bg-surface-950 relative">
        {/* Quick theme toggle */}
        <Tooltip text={`Switch to ${theme === 'dark' ? 'light' : 'dark'} mode`} position="left">
          <button
            onClick={toggleTheme}
            className="fixed top-4 right-4 z-50 p-2.5 rounded-xl bg-surface-800 border border-surface-700 text-slate-400 hover:text-white hover:border-surface-500 shadow-lg transition-all duration-200 cursor-pointer"
            aria-label={`Switch to ${theme === 'dark' ? 'light' : 'dark'} mode`}
          >
            {theme === 'dark'
              ? <Sun className="w-[18px] h-[18px]" />
              : <Moon className="w-[18px] h-[18px]" />
            }
          </button>
        </Tooltip>
        <ErrorBoundary>
          <div className="py-6 px-8 max-w-[1400px]">
            <Routes>
              <Route path="/" element={<Dashboard />} />
              <Route path="/vods" element={<Vods />} />
              <Route path="/clips" element={<Clips />} />
              <Route path="/editor/:clipId" element={<Editor />} />
              <Route path="/player/:vodId" element={<Player />} />
              <Route path="/scheduled" element={<ScheduledUploads />} />
              <Route path="/montage" element={<MontageBuilder />} />
              <Route path="/analytics" element={<Analytics />} />
              <Route path="/results/:vodId" element={<Results />} />
              <Route path="/settings" element={<SettingsPage />} />
              <Route path="/bug-report" element={<BugReport />} />
              <Route path="/help" element={<HelpGuide />} />
            </Routes>
          </div>
        </ErrorBoundary>
      </main>
      <UpdateChecker />
    </div>
    </FirstRunSetup>
    </BinariesSetup>
  )
}