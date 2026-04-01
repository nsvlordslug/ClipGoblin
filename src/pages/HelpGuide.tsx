import { useState } from 'react'
import { BookOpen, ChevronDown, ChevronRight, Keyboard, AlertTriangle, Rocket, Wand2, Brain, DollarSign } from 'lucide-react'

/** Collapsible section for help guide */
function HelpSection({ icon: Icon, title, children }: { icon: React.ComponentType<{ className?: string }>, title: string, children: React.ReactNode }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="border border-surface-600 rounded-lg overflow-hidden">
      <button onClick={() => setOpen(!open)} className="flex items-center gap-3 w-full px-4 py-3 text-left hover:bg-surface-700/50 transition-colors cursor-pointer">
        <Icon className="w-4 h-4 text-violet-400 shrink-0" />
        <span className="text-sm font-medium text-white flex-1">{title}</span>
        {open ? <ChevronDown className="w-4 h-4 text-slate-500" /> : <ChevronRight className="w-4 h-4 text-slate-500" />}
      </button>
      {open && <div className="px-4 pb-4 space-y-3 text-sm text-slate-300 leading-relaxed">{children}</div>}
    </div>
  )
}

export default function HelpGuide() {
  return (
    <div className="space-y-8 max-w-2xl">
      <h1 className="text-2xl font-bold text-white">Help & Guide</h1>

      <section className="bg-surface-800 border border-surface-700 rounded-xl p-6">
        <div className="flex items-center gap-2 mb-1">
          <BookOpen className="w-5 h-5 text-violet-400" />
          <h2 className="text-lg font-semibold text-white">Learn ClipGoblin</h2>
        </div>
        <p className="text-sm text-slate-400 mb-5">
          Everything you need to know to get the most out of ClipGoblin.
        </p>
        <div className="space-y-2">

          <HelpSection icon={Rocket} title="Getting Started">
            <p><span className="text-white font-medium">1. Connect your Twitch account</span> — Click "Connect Twitch" in Settings or on the My Channel page to link your Twitch account. No API credentials needed.</p>
            <p><span className="text-white font-medium">2. Your channel loads automatically</span> — Once connected, ClipGoblin fetches your available VODs.</p>
            <p><span className="text-white font-medium">3. Analyze a VOD</span> — Select a VOD and click Analyze. ClipGoblin will scan the entire stream to detect highlight-worthy moments using local heuristic analysis (no AI needed).</p>
            <p><span className="text-white font-medium">4. Review & edit clips</span> — Browse detected highlights, adjust start/end times, pick an aspect ratio, add captions, and export.</p>
            <p><span className="text-white font-medium">5. Export & publish</span> — Export clips as video files, then upload directly to connected platforms like YouTube.</p>
          </HelpSection>

          <HelpSection icon={Wand2} title="How to Use">
            <div className="space-y-2">
              <p className="text-white font-medium">Clip Detection</p>
              <p>ClipGoblin analyzes VODs locally using audio peaks, chat density, and stream events — no AI or API key required. Adjust sensitivity in Settings &rarr; Detection Sensitivity: Low catches only the best moments, High finds more subtle clips.</p>
              <p className="text-white font-medium mt-3">Editing Clips</p>
              <p>The Editor lets you fine-tune each clip: adjust timing with frame-by-frame precision, choose aspect ratio (16:9, 9:16, 1:1), add captions with multiple style presets, and preview everything before export.</p>
              <p className="text-white font-medium mt-3">Captions & Titles</p>
              <p>Free mode generates titles and captions using pattern-based templates. For higher quality, connect an AI provider (OpenAI, Claude, or Gemini) in Settings &rarr; AI Provider — the AI is only used for captions and titles, never for clip detection.</p>
              <p className="text-white font-medium mt-3">Exporting</p>
              <p>Export renders your clip to a video file using FFmpeg. Choose from presets optimized for different platforms. Exported files are saved to your Exports folder (see Settings &rarr; Storage Locations).</p>
              <p className="text-white font-medium mt-3">Batch Upload</p>
              <p>Select multiple clips on the Clips page and upload them all at once. Unexported clips are automatically exported first. You can schedule uploads for a specific date and time.</p>
              <p className="text-white font-medium mt-3">Montage Builder</p>
              <p>Combine multiple highlights into a single montage video with automatic transitions.</p>
            </div>
          </HelpSection>

          <HelpSection icon={Brain} title="AI Provider Guide">
            <p>AI providers are <span className="text-white">optional</span> and only used for generating captions and titles. Clip detection always runs locally for speed and zero cost.</p>

            <div className="bg-amber-500/10 border border-amber-500/20 rounded-lg px-3 py-2 mt-2">
              <p className="text-xs text-amber-400/90">&#x26A0;&#xFE0F; Always save your API key when it's shown — most providers only display it once. If you lose it, you'll need to create a new one.</p>
            </div>

            <div className="bg-surface-900 rounded-lg p-3 mt-3">
              <p className="text-white font-medium">Free Mode</p>
              <p className="text-slate-400 text-xs mt-1">Pattern-based caption and title generation. No API key needed. Works offline. Good enough for most use cases.</p>
            </div>

            <div className="bg-surface-900 rounded-lg p-3 mt-3">
              <p className="text-white font-medium">Claude (Anthropic)</p>
              <p className="text-slate-400 text-xs mt-1">Natural-sounding captions with strong context awareness.</p>
              <ol className="list-decimal list-inside text-xs text-slate-300 mt-2 space-y-1.5">
                <li>Go to <a href="https://console.anthropic.com/" target="_blank" rel="noopener noreferrer" className="text-violet-400 hover:underline">console.anthropic.com</a> and create an account</li>
                <li>Go to <span className="text-white">Settings &rarr; Billing</span> and add a credit card</li>
                <li>Click <span className="text-white">"Buy credits"</span> — enter $5 (minimum amount, lasts months)</li>
                <li>Go to <span className="text-white">API Keys</span> in the sidebar</li>
                <li>Click <span className="text-white">"Create Key"</span>, name it "ClipGoblin"</li>
                <li>Copy the key immediately and save it somewhere safe — you will never see it again after closing this dialog</li>
                <li>Paste the key into ClipGoblin's <span className="text-white">Settings &rarr; AI Provider &rarr; Claude</span></li>
              </ol>
            </div>

            <div className="bg-surface-900 rounded-lg p-3 mt-3">
              <p className="text-white font-medium">OpenAI (GPT)</p>
              <p className="text-slate-400 text-xs mt-1">High-quality captions with natural language understanding.</p>
              <ol className="list-decimal list-inside text-xs text-slate-300 mt-2 space-y-1.5">
                <li>Go to <a href="https://platform.openai.com/" target="_blank" rel="noopener noreferrer" className="text-violet-400 hover:underline">platform.openai.com</a> and create an account</li>
                <li>New accounts get $5 in free credits (no credit card needed) — these expire after 3 months</li>
                <li>If you need more credits later, go to <span className="text-white">Settings &rarr; Billing &rarr; Add payment method</span> and add funds</li>
                <li>Go to <span className="text-white">API Keys</span> in the sidebar</li>
                <li>Click <span className="text-white">"Create new secret key"</span>, name it "ClipGoblin"</li>
                <li>Copy the key immediately and save it somewhere safe — you will never see it again after closing this dialog</li>
                <li>Paste the key into ClipGoblin's <span className="text-white">Settings &rarr; AI Provider &rarr; OpenAI</span></li>
              </ol>
            </div>

            <div className="bg-surface-900 rounded-lg p-3 mt-3">
              <p className="text-white font-medium">Google Gemini</p>
              <p className="text-slate-400 text-xs mt-1">Google's AI for caption generation.</p>
              <ol className="list-decimal list-inside text-xs text-slate-300 mt-2 space-y-1.5">
                <li>Go to <a href="https://aistudio.google.com/" target="_blank" rel="noopener noreferrer" className="text-violet-400 hover:underline">aistudio.google.com</a> and sign in with your Google account</li>
                <li>Click <span className="text-white">"Get API key"</span> in the left sidebar</li>
                <li>Click <span className="text-white">"Create API key"</span> and select a Google Cloud project (or create one)</li>
                <li>Copy the key immediately and save it somewhere safe</li>
                <li>Gemini offers a free tier with rate limits — no credit card needed for basic use. For higher limits, set up billing in Google Cloud Console</li>
                <li>Paste the key into ClipGoblin's <span className="text-white">Settings &rarr; AI Provider &rarr; Gemini</span></li>
              </ol>
            </div>

            <p className="text-xs text-slate-500 mt-3">Tip: Enable "Fall back to Free mode" in Settings so your app keeps working if the API is ever down.</p>
          </HelpSection>

          <HelpSection icon={DollarSign} title="AI Model Cost Guide">
            <p>Estimated costs per clip caption/title generation. Actual costs depend on clip length and transcript size.</p>

            <div className="mt-3">
              <p className="text-white font-medium text-xs mb-2">OpenAI Models</p>
              <div className="bg-surface-900 rounded-lg overflow-hidden">
                <table className="w-full text-xs">
                  <thead><tr className="border-b border-surface-700 text-slate-500">
                    <th className="text-left px-3 py-2 font-medium">Model</th>
                    <th className="text-right px-3 py-2 font-medium">Input / 1M tokens</th>
                    <th className="text-right px-3 py-2 font-medium">Output / 1M tokens</th>
                    <th className="text-right px-3 py-2 font-medium">Est. per clip</th>
                  </tr></thead>
                  <tbody className="text-slate-300">
                    <tr className="border-b border-surface-700/50"><td className="px-3 py-1.5">GPT-5.4 Nano</td><td className="text-right px-3">$0.20</td><td className="text-right px-3">$1.25</td><td className="text-right px-3">~$0.0003</td></tr>
                    <tr className="border-b border-surface-700/50"><td className="px-3 py-1.5">GPT-5.4 Mini</td><td className="text-right px-3">$0.75</td><td className="text-right px-3">$4.50</td><td className="text-right px-3">~$0.001</td></tr>
                    <tr><td className="px-3 py-1.5">GPT-5.4</td><td className="text-right px-3">$2.50</td><td className="text-right px-3">$15.00</td><td className="text-right px-3">~$0.004</td></tr>
                  </tbody>
                </table>
              </div>
            </div>

            <div className="mt-3">
              <p className="text-white font-medium text-xs mb-2">Anthropic (Claude) Models</p>
              <div className="bg-surface-900 rounded-lg overflow-hidden">
                <table className="w-full text-xs">
                  <thead><tr className="border-b border-surface-700 text-slate-500">
                    <th className="text-left px-3 py-2 font-medium">Model</th>
                    <th className="text-right px-3 py-2 font-medium">Input / 1M tokens</th>
                    <th className="text-right px-3 py-2 font-medium">Output / 1M tokens</th>
                    <th className="text-right px-3 py-2 font-medium">Est. per clip</th>
                  </tr></thead>
                  <tbody className="text-slate-300">
                    <tr className="border-b border-surface-700/50"><td className="px-3 py-1.5">Claude Haiku 3.5</td><td className="text-right px-3">$0.80</td><td className="text-right px-3">$4.00</td><td className="text-right px-3">~$0.001</td></tr>
                    <tr><td className="px-3 py-1.5">Claude Sonnet 4</td><td className="text-right px-3">$3.00</td><td className="text-right px-3">$15.00</td><td className="text-right px-3">~$0.004</td></tr>
                  </tbody>
                </table>
              </div>
            </div>

            <div className="mt-3">
              <p className="text-white font-medium text-xs mb-2">Google (Gemini) Models</p>
              <div className="bg-surface-900 rounded-lg overflow-hidden">
                <table className="w-full text-xs">
                  <thead><tr className="border-b border-surface-700 text-slate-500">
                    <th className="text-left px-3 py-2 font-medium">Model</th>
                    <th className="text-right px-3 py-2 font-medium">Input / 1M tokens</th>
                    <th className="text-right px-3 py-2 font-medium">Output / 1M tokens</th>
                    <th className="text-right px-3 py-2 font-medium">Est. per clip</th>
                  </tr></thead>
                  <tbody className="text-slate-300">
                    <tr className="border-b border-surface-700/50"><td className="px-3 py-1.5">Gemini 2.0 Flash</td><td className="text-right px-3">$0.10</td><td className="text-right px-3">$0.40</td><td className="text-right px-3">~$0.0001</td></tr>
                    <tr className="border-b border-surface-700/50"><td className="px-3 py-1.5">Gemini 1.5 Pro</td><td className="text-right px-3">$1.25</td><td className="text-right px-3">$5.00</td><td className="text-right px-3">~$0.002</td></tr>
                    <tr><td className="px-3 py-1.5">Gemini 2.5 Pro</td><td className="text-right px-3">$1.25</td><td className="text-right px-3">$10.00</td><td className="text-right px-3">~$0.003</td></tr>
                  </tbody>
                </table>
              </div>
            </div>

            <p className="text-xs text-slate-500 mt-2">Costs are approximate and based on ~500-1000 tokens per clip. A typical session analyzing 20 clips costs less than $0.10 with most models.</p>
            <div className="bg-amber-500/10 border border-amber-500/20 rounded-lg px-3 py-2 mt-3">
              <p className="text-xs text-amber-400/90">Pricing shown is approximate and was accurate as of April 2026. AI providers may change their pricing at any time. Check your provider's pricing page for the most current rates. Actual cost per generation depends on clip length and transcript size — shorter clips cost less, longer clips with more dialogue cost slightly more.</p>
            </div>
          </HelpSection>

          <HelpSection icon={Keyboard} title="Keyboard Shortcuts">
            <div className="bg-surface-900 rounded-lg overflow-hidden">
              <table className="w-full text-xs">
                <thead><tr className="border-b border-surface-700 text-slate-500">
                  <th className="text-left px-3 py-2 font-medium">Shortcut</th>
                  <th className="text-left px-3 py-2 font-medium">Action</th>
                </tr></thead>
                <tbody className="text-slate-300">
                  <tr className="border-b border-surface-700/50"><td className="px-3 py-1.5 font-mono text-violet-300">Space</td><td className="px-3">Play / Pause video</td></tr>
                  <tr className="border-b border-surface-700/50"><td className="px-3 py-1.5 font-mono text-violet-300">I</td><td className="px-3">Set clip start (In point)</td></tr>
                  <tr className="border-b border-surface-700/50"><td className="px-3 py-1.5 font-mono text-violet-300">O</td><td className="px-3">Set clip end (Out point)</td></tr>
                  <tr className="border-b border-surface-700/50"><td className="px-3 py-1.5 font-mono text-violet-300">&larr; / &rarr;</td><td className="px-3">Seek backward / forward 5s</td></tr>
                  <tr className="border-b border-surface-700/50"><td className="px-3 py-1.5 font-mono text-violet-300">Shift + &larr; / &rarr;</td><td className="px-3">Seek backward / forward 1 frame</td></tr>
                  <tr className="border-b border-surface-700/50"><td className="px-3 py-1.5 font-mono text-violet-300">Ctrl + Z</td><td className="px-3">Undo</td></tr>
                  <tr><td className="px-3 py-1.5 font-mono text-violet-300">Ctrl + Shift + Z</td><td className="px-3">Redo</td></tr>
                </tbody>
              </table>
            </div>
          </HelpSection>

          <HelpSection icon={AlertTriangle} title="Troubleshooting">
            <div className="space-y-3">
              <div>
                <p className="text-white font-medium">"Cannot play video" on clips</p>
                <p className="text-slate-400">The source VOD file may have been moved or deleted. Make sure the original VOD file is still in its download location. Check Settings &rarr; Storage Locations to find your files.</p>
              </div>
              <div>
                <p className="text-white font-medium">FFmpeg not found</p>
                <p className="text-slate-400">ClipGoblin requires FFmpeg for exporting clips. Install it from <a href="https://ffmpeg.org/download.html" target="_blank" rel="noopener noreferrer" className="text-violet-400 hover:underline">ffmpeg.org</a> and make sure it's in your system PATH.</p>
              </div>
              <div>
                <p className="text-white font-medium">AI captions not generating</p>
                <p className="text-slate-400">Check that your API key is correct using the "Test Connection" button in Settings &rarr; AI Provider. Make sure you've enabled "Caption generation" and/or "Title generation" in the checkboxes. If the API is down, enable "Fall back to Free mode."</p>
              </div>
              <div>
                <p className="text-white font-medium">YouTube upload fails</p>
                <p className="text-slate-400">Ensure your YouTube account is connected in Settings &rarr; Publishing Accounts. The clip must be exported first. If you get a token error, disconnect and reconnect your YouTube account.</p>
              </div>
              <div>
                <p className="text-white font-medium">No highlights detected</p>
                <p className="text-slate-400">Try increasing Detection Sensitivity to High in Settings. Very short VODs or streams with low activity may yield fewer highlights. Make sure the VOD has audio — audio peaks are a key detection signal.</p>
              </div>
            </div>
          </HelpSection>

        </div>
      </section>
    </div>
  )
}
