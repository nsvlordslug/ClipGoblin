import { useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import { Tv, ExternalLink, LogOut, LogIn } from 'lucide-react'
import { useAppStore } from '../stores/appStore'

export default function Channels() {
  const { loggedInUser, checkLogin, twitchLogin, twitchLogout, isLoading, error, clearError } =
    useAppStore()
  const navigate = useNavigate()

  useEffect(() => {
    checkLogin()
  }, [checkLogin])

  const handleLogin = async () => {
    clearError()
    try {
      await twitchLogin()
    } catch {
      // error is set in store
    }
  }

  const handleLogout = async () => {
    await twitchLogout()
  }

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold text-white">My Channel</h1>

      {!loggedInUser ? (
        /* Not logged in — show login prompt */
        <div className="bg-surface-800 border border-surface-700 rounded-xl p-12 text-center">
          <div className="flex items-center justify-center w-16 h-16 rounded-2xl bg-violet-600/20 mx-auto mb-5">
            <Tv className="w-8 h-8 text-violet-400" />
          </div>
          <h3 className="text-xl font-semibold text-white mb-2">
            Connect Your Twitch Account
          </h3>
          <p className="text-slate-400 text-sm mb-6 max-w-md mx-auto">
            Log in with your Twitch account to access your VODs and start finding highlights in your streams.
          </p>
          {error && (
            <p className="text-red-400 text-sm mb-4 max-w-md mx-auto">{error}</p>
          )}
          <button
            onClick={handleLogin}
            disabled={isLoading}
            className="inline-flex items-center gap-2.5 px-6 py-3 bg-[#9146FF] hover:bg-[#7c3aed] disabled:opacity-50 disabled:cursor-not-allowed text-white font-semibold rounded-lg transition-colors cursor-pointer"
          >
            <LogIn className="w-5 h-5" />
            {isLoading ? 'Waiting for Twitch...' : 'Log in with Twitch'}
          </button>
          <p className="text-xs text-slate-500 mt-4">
            This will open your browser for secure Twitch authentication.
          </p>
        </div>
      ) : (
        /* Logged in — show channel card */
        <div className="bg-surface-800 border border-surface-700 rounded-xl p-6">
          <div className="flex items-center gap-4">
            {loggedInUser.profile_image_url ? (
              <img
                src={loggedInUser.profile_image_url}
                alt={loggedInUser.display_name}
                className="w-16 h-16 rounded-full bg-surface-700"
              />
            ) : (
              <div className="w-16 h-16 rounded-full bg-surface-700 flex items-center justify-center">
                <Tv className="w-7 h-7 text-slate-500" />
              </div>
            )}
            <div className="flex-1 min-w-0">
              <p className="text-xl font-bold text-white truncate">
                {loggedInUser.display_name}
              </p>
              <p className="text-sm text-slate-400">
                @{loggedInUser.twitch_login}
              </p>
              <div className="flex items-center gap-1.5 mt-1">
                <span className="inline-block w-2 h-2 rounded-full bg-emerald-500" />
                <span className="text-xs text-emerald-400">Connected</span>
              </div>
            </div>
            <div className="flex gap-2">
              <button
                onClick={() => navigate('/vods')}
                className="flex items-center gap-2 px-4 py-2.5 bg-violet-600 hover:bg-violet-500 text-white text-sm font-medium rounded-lg transition-colors cursor-pointer"
              >
                <ExternalLink className="w-4 h-4" />
                View VODs
              </button>
              <button
                onClick={handleLogout}
                className="flex items-center gap-2 px-4 py-2.5 bg-surface-700 hover:bg-red-600/20 text-slate-300 hover:text-red-400 text-sm font-medium rounded-lg transition-colors cursor-pointer"
              >
                <LogOut className="w-4 h-4" />
                Disconnect
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
