import { useEffect } from 'react'
import { Routes, Route, NavLink } from 'react-router-dom'
import { usePlatformStore } from './stores/platformStore'
import {
  LayoutDashboard,
  Tv,
  Video,
  Scissors,
  Settings,
  Film,
} from 'lucide-react'
import logoImg from './assets/logo.png'
import Dashboard from './pages/Dashboard'
import Channels from './pages/Channels'
import Vods from './pages/Vods'
import Clips from './pages/Clips'
import SettingsPage from './pages/Settings'
import Player from './pages/Player'
import Editor from './pages/Editor'
import MontageBuilder from './pages/MontageBuilder'
import Results from './pages/Results'

const navItems = [
  { to: '/', label: 'Dashboard', icon: LayoutDashboard },
  { to: '/channels', label: 'My Channel', icon: Tv },
  { to: '/vods', label: 'VODs', icon: Video },
  { to: '/clips', label: 'Clips', icon: Scissors },
  { to: '/montage', label: 'Montage', icon: Film },
  { to: '/settings', label: 'Settings', icon: Settings },
]

export default function App() {
  const loadPlatforms = usePlatformStore(s => s.load)
  useEffect(() => { loadPlatforms() }, [loadPlatforms])

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
          <p className="text-[10px] text-slate-600 font-medium tracking-wide">ClipGoblin <span className="text-slate-500">v0.1.0</span></p>
        </div>
      </aside>

      {/* Main content */}
      <main className="flex-1 overflow-y-auto bg-surface-950">
        <div className="p-8 max-w-6xl">
          <Routes>
            <Route path="/" element={<Dashboard />} />
            <Route path="/channels" element={<Channels />} />
            <Route path="/vods" element={<Vods />} />
            <Route path="/clips" element={<Clips />} />
            <Route path="/settings" element={<SettingsPage />} />
            <Route path="/player/:vodId" element={<Player />} />
            <Route path="/editor/:clipId" element={<Editor />} />
            <Route path="/montage" element={<MontageBuilder />} />
            <Route path="/results" element={<Results />} />
          </Routes>
        </div>
      </main>
    </div>
  )
}
