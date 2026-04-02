import { useEffect, Component, type ReactNode } from 'react'
import { Routes, Route, NavLink } from 'react-router-dom'
import { usePlatformStore } from './stores/platformStore'
import { useAiStore } from './stores/aiStore'
import { useUiStore } from './stores/uiStore'
import { useTemplateStore } from './stores/templateStore'
import {
  LayoutDashboard,
  Video,
  Scissors,
  Settings,
  Film,
  Clock,
  Sun,
  Moon,
  HelpCircle,
  Bug,
} from 'lucide-react'
import Tooltip from './components/Tooltip'
import logoImg from './assets/logo.png'
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
import { useScheduleStore } from './stores/scheduleStore'

const navItems = [
  { to: '/', label: 'Dashboard', icon: LayoutDashboard },
  { to: '/vods', label: 'VODs', icon: Video },
  { to: '/clips', label: 'Clips', icon: Scissors },
  { to: '/scheduled', label: 'Scheduled', icon: Clock },
  { to: '/montage', label: 'Montage', icon: Film },
  { to: '/settings', label: 'Settings', icon: Settings },
  { to: '/bug-report', label: 'Report Bug', icon: Bug },
  { to: '/help', label: 'Help', icon: HelpCircle },
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
      return (
        <div className="flex items-center justify-center h-screen bg-surface-950">
          <div className="text-center max-w-md px-6">
            <div className="text-5xl mb-4">💀</div>
            <h1 className="text-xl font-bold text-white mb-2">Something went wrong</h1>
            <p className="text-sm text-slate-400 mb-6">
              {this.state.error?.message || 'An unexpected error occurred.'}
            </p>
            <button
              onClick={() => window.location.reload()}
              className="px-5 py-2.5 bg-violet-600 hover:bg-violet-500 text-white rounded-lg text-sm font-medium transition-colors cursor-pointer"
            >
              Reload App
            </button>
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
  useEffect(() => {
    loadPlatforms(); loadAi(); loadUi(); loadTemplates(); loadSchedules()
    const unlisten = startScheduleListening()
    return unlisten
  }, [loadPlatforms, loadAi, loadUi, loadTemplates, loadSchedules, startScheduleListening])

  const toggleTheme = () => updateUi({ theme: theme === 'dark' ? 'light' : 'dark' })

  return (
    <div className="flex h-screen overflow-hidden">
      {/* Sidebar */}
      <aside className="w-72 shrink-0 bg-surface-900 border-r border-surface-700 flex flex-col">
        {/* ── Branding block ── */}
        <div className="sidebar-brand">
          <div className="sidebar-logo">
            <img src={logoImg} alt="" style={{ width: '100%', height: '100%', objectFit: 'cover', display: 'block' }} />
          </div>
          <div className="sidebar-wordmark">
            <span className="sidebar-title">
              <span className="text-clip">Clip</span>
              <span className="text-goblin">Goblin</span>
            </span>
            <span className="sidebar-subtitle">Clip Engine</span>
          </div>
        </div>

        {/* ── Navigation ── */}
        <nav className="flex-1 px-3 py-5 space-y-1">
          {navItems.map(({ to, label, icon: Icon }) => (
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
              <Icon className="w-[18px] h-[18px]" />
              {label}
            </NavLink>
          ))}
        </nav>

        {/* ── Footer ── */}
        <div className="px-6 py-4 border-t border-surface-700">
          <p className="text-[10px] text-slate-600 font-medium tracking-wide">ClipGoblin <span className="text-slate-500">v1.0.0</span></p>
        </div>
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
          <div className="py-8 pl-8 pr-16 max-w-6xl">
            <Routes>
              <Route path="/" element={<Dashboard />} />
              <Route path="/vods" element={<Vods />} />
              <Route path="/clips" element={<Clips />} />
              <Route path="/editor/:clipId" element={<Editor />} />
              <Route path="/player/:vodId" element={<Player />} />
              <Route path="/scheduled" element={<ScheduledUploads />} />
              <Route path="/montage" element={<MontageBuilder />} />
              <Route path="/results/:vodId" element={<Results />} />
              <Route path="/settings" element={<SettingsPage />} />
              <Route path="/bug-report" element={<BugReport />} />
              <Route path="/help" element={<HelpGuide />} />
            </Routes>
          </div>
        </ErrorBoundary>
      </main>
    </div>
  )
}