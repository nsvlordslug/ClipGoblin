import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { usePlatformStore, PLATFORM_INFO } from '../stores/platformStore'
import { Link2, Unlink, Loader2 } from 'lucide-react'
import Tooltip from './Tooltip'

export default function ConnectedAccounts() {
  const { accounts, loading, load, connect, disconnect } = usePlatformStore()
  const [tiktokHandle, setTiktokHandle] = useState('')
  const [handleSaved, setHandleSaved] = useState(false)
  const savedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

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
      if (savedTimerRef.current) clearTimeout(savedTimerRef.current)
      savedTimerRef.current = setTimeout(() => setHandleSaved(false), 2000)
    } catch (e) {
      console.error('Failed to save TikTok handle:', e)
    }
  }

  useEffect(() => { load() }, [load])

  // Clean up pending timer on unmount
  useEffect(() => () => { if (savedTimerRef.current) clearTimeout(savedTimerRef.current) }, [])

  const platforms = Object.keys(PLATFORM_INFO) as string[]

  return (
    <>
      {platforms.map(key => {
        const info = PLATFORM_INFO[key]
        const account = accounts[key]
        const isLoading = loading[key] ?? false

        return (
          <div key={key} className="v4-setting-row">
            <div className="v4-setting-info flex items-center gap-2.5">
              <span className="w-6 h-6 rounded-md flex items-center justify-center text-[11px] font-bold shrink-0"
                style={{ background: `${info.color}20`, color: info.color, border: `1px solid ${info.color}40` }}>
                {info.icon}
              </span>
              <div className="min-w-0">
                <div className="v4-setting-name">{info.name}</div>
                <div className="v4-setting-desc">
                  {isLoading
                    ? 'Connecting...'
                    : account
                      ? <>@{account.account_name}{key === 'tiktok' ? ' · Sandbox mode' : ''}</>
                      : info.available
                        ? 'Not connected'
                        : 'Coming soon · planned v2'}
                </div>
                {key === 'tiktok' && account && (
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
            </div>

            {isLoading ? (
              <Loader2 className="w-4 h-4 text-slate-400 animate-spin" />
            ) : account ? (
              <div className="flex items-center gap-2">
                <span className="v4-connected-pill">● CONNECTED</span>
                <Tooltip text={`Disconnect your ${info.name} account`} position="left">
                  <button onClick={() => disconnect(key)}
                    className="v4-btn ghost"
                    style={{padding: '6px 12px', fontSize: 12}}
                  >
                    <Unlink className="w-3 h-3" />
                    Disconnect
                  </button>
                </Tooltip>
              </div>
            ) : info.available ? (
              <Tooltip text={`Connect your ${info.name} account`} position="left">
                <button onClick={() => connect(key).catch(() => {})}
                  className="v4-btn"
                  style={{padding: '6px 12px', fontSize: 12}}
                >
                  <Link2 className="w-3 h-3" />
                  Connect
                </button>
              </Tooltip>
            ) : (
              <span className="v4-connected-pill idle">COMING SOON</span>
            )}
          </div>
        )
      })}
    </>
  )
}
