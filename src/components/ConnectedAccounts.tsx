import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { usePlatformStore, PLATFORM_INFO } from '../stores/platformStore'
import { Link2, Unlink, Loader2 } from 'lucide-react'
import Tooltip from './Tooltip'

export default function ConnectedAccounts() {
  const { accounts, loading, load, connect, disconnect } = usePlatformStore()
  const [tiktokHandle, setTiktokHandle] = useState('')
  const [handleSaved, setHandleSaved] = useState(false)

  // Load the stored TikTok handle
  useEffect(() => {
    invoke<string | null>('get_setting', { key: 'tiktok_handle' }).then(v => {
      if (v) setTiktokHandle(v)
    })
  }, [accounts])

  const saveTiktokHandle = async (val: string) => {
    const clean = val.replace(/^@/, '')
    setTiktokHandle(clean)
    try {
      await invoke('save_setting', { key: 'tiktok_handle', value: clean })
      setHandleSaved(true)
      setTimeout(() => setHandleSaved(false), 2000)
    } catch (e) {
      console.error('Failed to save TikTok handle:', e)
    }
  }

  useEffect(() => { load() }, [load])

  const platforms = Object.keys(PLATFORM_INFO) as string[]

  return (
    <div className="space-y-3">
      {platforms.map(key => {
        const info = PLATFORM_INFO[key]
        const account = accounts[key]
        const isLoading = loading[key] ?? false

        return (
          <div key={key} className="flex items-center gap-3 p-3 bg-surface-900 border border-surface-600 rounded-lg">
            {/* Platform icon */}
            <div className="w-8 h-8 rounded-lg flex items-center justify-center text-xs font-bold shrink-0"
              style={{ background: `${info.color}20`, color: info.color, border: `1px solid ${info.color}40` }}>
              {info.icon}
            </div>

            <div className="flex-1 min-w-0">
              <p className="text-sm text-white font-medium">{info.name}</p>
              {isLoading ? (
                <p className="text-[10px] text-slate-400">Connecting...</p>
              ) : account ? (
                <div>
                  <p className="text-[10px] text-emerald-400 truncate">
                    Connected as {account.account_name}
                  </p>
                  {key === 'tiktok' && (
                    <div className="flex items-center gap-1 mt-1">
                      <Tooltip text="Your TikTok username — used for View on TikTok links" position="right">
                        <span className="text-[10px] text-slate-500">@</span>
                      </Tooltip>
                      <input
                        type="text"
                        value={tiktokHandle}
                        onChange={e => setTiktokHandle(e.target.value.replace(/^@/, ''))}
                        onBlur={e => saveTiktokHandle(e.target.value)}
                        onKeyDown={e => e.key === 'Enter' && saveTiktokHandle((e.target as HTMLInputElement).value)}
                        placeholder="your_handle"
                        className="bg-transparent text-[10px] text-slate-300 border-b border-surface-600 focus:border-violet-500 outline-none w-24 py-0.5"
                      />
                      {handleSaved && <span className="text-[9px] text-emerald-400">saved</span>}
                    </div>
                  )}
                </div>
              ) : info.available ? (
                <p className="text-[10px] text-slate-500">Not connected</p>
              ) : (
                <p className="text-[10px] text-slate-600">Coming soon</p>
              )}
            </div>

            {isLoading ? (
              <Loader2 className="w-4 h-4 text-slate-400 animate-spin" />
            ) : account ? (
              <Tooltip text={`Disconnect your ${info.name} account`} position="left">
                <button onClick={() => disconnect(key)}
                  className="flex items-center gap-1 px-2 py-1 text-xs text-red-400 bg-red-500/10 border border-red-500/30 rounded hover:bg-red-500/20 transition-colors cursor-pointer">
                  <Unlink className="w-3 h-3" />
                  Disconnect
                </button>
              </Tooltip>
            ) : info.available ? (
              <Tooltip text={`Connect your ${info.name} account`} position="left">
                <button onClick={() => connect(key).catch(() => {})}
                  className="flex items-center gap-1 px-2 py-1 text-xs text-slate-300 bg-surface-800 border border-surface-500 rounded hover:text-white hover:border-violet-500/40 transition-colors cursor-pointer">
                  <Link2 className="w-3 h-3" />
                  Connect
                </button>
              </Tooltip>
            ) : (
              <span className="px-2 py-1 text-xs text-slate-600">—</span>
            )}
          </div>
        )
      })}
    </div>
  )
}
