import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { ChevronDown, ChevronRight } from 'lucide-react'
import { resetTesterChecklistDismissal } from '../components/TesterChecklist'
import { version as appVersion } from '../../package.json'

/** Collapsible deep-dive section */
function HelpSection({ icon, title, children }: { icon: string, title: string, children: React.ReactNode }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="v4-faq" style={{padding: 0, overflow: 'hidden'}}>
      <button
        onClick={() => setOpen(!open)}
        className="flex items-center gap-3 w-full px-4 py-3 text-left hover:bg-[rgba(167,139,250,0.08)] transition-colors cursor-pointer"
      >
        <span className="text-base shrink-0">{icon}</span>
        <span className="v4-faq-q flex-1" style={{margin: 0}}>{title}</span>
        {open ? <ChevronDown className="w-4 h-4 text-slate-500" /> : <ChevronRight className="w-4 h-4 text-slate-500" />}
      </button>
      {open && <div className="v4-faq-a px-4 pb-4 space-y-3">{children}</div>}
    </div>
  )
}

/** Flat FAQ item (question + answer visible, matching mockup) */
function FaqItem({ q, children }: { q: string; children: React.ReactNode }) {
  return (
    <div className="v4-faq">
      <div className="v4-faq-q">{q}</div>
      <div className="v4-faq-a">{children}</div>
    </div>
  )
}

/** Featured topic card (opens its corresponding deep-dive) */
function FeaturedCard({ icon, title, desc, onClick }: { icon: string; title: string; desc: string; onClick?: () => void }) {
  return (
    <div className="v4-faq" onClick={onClick} style={{cursor: 'pointer'}}>
      <div className="v4-faq-q">{icon} {title}</div>
      <div className="v4-faq-a">{desc}</div>
    </div>
  )
}

export default function HelpGuide() {
  const [search, setSearch] = useState('')
  const [deepDiveOpen, setDeepDiveOpen] = useState<string | null>(null)
  const navigate = useNavigate()

  return (
    <div className="space-y-4 max-w-[1100px]">
      {/* Page header with search input on right */}
      <div className="v4-page-header">
        <div>
          <div className="v4-page-title">Help &amp; Guide ?</div>
          <div className="v4-page-sub">FAQs, setup walkthroughs, and troubleshooting</div>
        </div>
        <input
          className="v4-input"
          placeholder="🔎 Search help articles..."
          value={search}
          onChange={e => setSearch(e.target.value)}
          style={{width: 260}}
        />
      </div>

      {/* Featured topics — 3-column grid */}
      <div
        className="grid gap-3.5"
        style={{gridTemplateColumns: 'repeat(3, 1fr)'}}
      >
        <FeaturedCard
          icon="🎯"
          title="Getting started"
          desc="Connect Twitch, import your first VOD, and ship your first clip in 5 minutes."
          onClick={() => setDeepDiveOpen('getting-started')}
        />
        <FeaturedCard
          icon="🧠"
          title="AI provider setup (BYOK)"
          desc="Set up Claude, OpenAI, or Gemini keys · cost breakdown per 1k clips."
          onClick={() => setDeepDiveOpen('ai-provider')}
        />
        <FeaturedCard
          icon="📤"
          title="Publishing walkthrough"
          desc="YouTube + TikTok auth, scheduling tips, batch upload workflows."
          onClick={() => setDeepDiveOpen('publishing')}
        />
      </div>

      {/* For testers */}
      <div>
        <h3 className="v4-section-label" style={{marginTop: 8, marginBottom: 10}}>🧪 For testers</h3>
        <div className="v4-panel" style={{padding: 16}}>
          <p className="text-sm text-slate-300 mb-3">
            You're running <b>ClipGoblin v{appVersion}</b> — a pre-release build. Here's what would help
            most with feedback:
          </p>
          <ul className="text-xs text-slate-400 space-y-1.5 mb-4 ml-4 list-disc">
            <li><b className="text-white">Smoke-test the whole pipeline:</b> connect Twitch → Auto-Hunt → review a clip → export → ship to YouTube / TikTok.</li>
            <li><b className="text-white">Try edge cases:</b> very short VODs (&lt; 30 min), very long VODs (3h+), VODs with little chat, VODs while you're offline.</li>
            <li><b className="text-white">Toggle detection settings:</b> flip sensitivity Low/Med/High, try Auto-ship high-confidence, disable Twitch community clips.</li>
            <li><b className="text-white">When something breaks,</b> click <a onClick={() => navigate('/bug-report')} className="text-violet-300 underline cursor-pointer">Report a Bug</a> — the form attaches logs + system info automatically.</li>
          </ul>
          <div className="flex flex-wrap gap-2">
            <button
              onClick={() => { resetTesterChecklistDismissal(); navigate('/') }}
              className="v4-btn"
              style={{padding: '6px 12px', fontSize: 12}}
            >
              Show onboarding checklist
            </button>
            <button
              onClick={() => navigate('/bug-report')}
              className="v4-btn primary"
              style={{padding: '6px 12px', fontSize: 12}}
            >
              🐛 Report a bug
            </button>
          </div>
          <div className="mt-4 pt-3 border-t border-surface-700">
            <p className="text-[11px] font-bold text-slate-500 uppercase tracking-wider mb-1.5">
              Known limitations (pre-release)
            </p>
            <ul className="text-[11px] text-slate-500 space-y-1 ml-4 list-disc">
              <li>Instagram publishing not implemented (Meta app not started yet).</li>
              <li>TikTok view counts need a reconnect — disconnect + reconnect in Settings once.</li>
              <li>Waveform thumbnails on clip rows are decorative placeholders (not real audio shape yet).</li>
              <li>AI Insights on Dashboard are generic counts — pattern detection needs more data first.</li>
              <li>Not code-signed yet — Windows SmartScreen will show "Unknown publisher". Click <b>More info → Run anyway</b> once.</li>
            </ul>
          </div>
        </div>
      </div>

      {/* Frequently asked */}
      <div>
        <h3 className="v4-section-label" style={{marginTop: 8, marginBottom: 10}}>Frequently asked</h3>
        <div className="space-y-2">
          <FaqItem q='Why did my VOD analysis fail with "cublas64_12.dll missing"?'>
            You need CUDA Toolkit 12 installed for GPU transcription. Download it from NVIDIA,
            or switch to CPU mode in Settings → Detection → GPU off.
          </FaqItem>
          <FaqItem q="How does clip detection decide confidence?">
            Confidence combines four signals: audio spike, chat velocity, transcript emphasis
            (caps/exclamations), and scene change. Weights are tunable in Settings → Detection.
          </FaqItem>
          <FaqItem q="Can I re-analyze a VOD with different sensitivity?">
            Yes — open the VOD card, click "Re-analyze". Previous highlights are preserved;
            new ones are appended.
          </FaqItem>
          <FaqItem q="Why won't my TikTok uploads publish to production?">
            ClipGoblin currently ships as a sandbox TikTok app pending production approval.
            Sandbox uploads go to your drafts.
          </FaqItem>
          <FaqItem q='What does "Ship rate" on the dashboard mean?'>
            % of detected highlights you actually published, vs deleted or left as drafts.
            Lower ship rates usually mean sensitivity is too high.
          </FaqItem>
          <FaqItem q="How do I get real view counts on the Analytics page?">
            Open Analytics and click <b>Refresh stats</b>. The app polls YouTube Data API
            (uses your existing scope — no re-auth) and TikTok Display API (needs the
            <code className="text-violet-300"> video.list</code> scope — disconnect
            and reconnect TikTok once in Settings to grant it). Instagram isn't wired up yet.
          </FaqItem>
          <FaqItem q='What is "Auto-ship high-confidence"?'>
            When enabled in Settings → Detection, any clip scoring 90%+ confidence
            after analysis is automatically queued to ship in 5 minutes to every
            connected platform. You'll see a banner on the Dashboard and the uploads
            appear on the Scheduled page — cancel any of them before the timer expires.
            The scheduler renders any missing exports on the fly, so you don't need to
            click Export manually before the timer fires.
          </FaqItem>
          <FaqItem q='What is "Use Twitch community clips"?'>
            When enabled in Settings → Detection, the analysis pipeline boosts
            moments where viewers already made their own Twitch clip. Clips with
            more views weigh more. No extra Twitch permission needed — uses the
            existing connected scope.
          </FaqItem>
        </div>
      </div>

      {/* Deep dives — expanded only when the matching featured card is clicked */}
      {deepDiveOpen && (
        <div>
          <h3 className="v4-section-label" style={{marginTop: 8, marginBottom: 10}}>
            Deep dives
            <button
              onClick={() => setDeepDiveOpen(null)}
              className="ml-3 text-[10px] text-violet-400 hover:text-violet-300 cursor-pointer normal-case tracking-normal"
              style={{letterSpacing: 0}}
            >
              Close ×
            </button>
          </h3>
          <div className="space-y-2">
            {deepDiveOpen === 'getting-started' && (
              <HelpSection icon="🎯" title="Getting Started">
                <p><span className="text-white font-medium">1. Connect your Twitch account</span> — Click "Connect Twitch" in Settings or on the My Channel page to link your Twitch account. No API credentials needed.</p>
                <p><span className="text-white font-medium">2. Your channel loads automatically</span> — Once connected, ClipGoblin fetches your available VODs.</p>
                <p><span className="text-white font-medium">3. Analyze a VOD</span> — Select a VOD and click Analyze. ClipGoblin will scan the entire stream to detect highlight-worthy moments using local heuristic analysis (no AI needed).</p>
                <p><span className="text-white font-medium">4. Review &amp; edit clips</span> — Browse detected highlights, adjust start/end times, pick an aspect ratio, add captions, and export.</p>
                <p><span className="text-white font-medium">5. Export &amp; publish</span> — Export clips as video files, then upload directly to connected platforms like YouTube.</p>
              </HelpSection>
            )}
            {deepDiveOpen === 'ai-provider' && (
              <HelpSection icon="🧠" title="AI Provider Guide">
                <p>AI providers are <span className="text-white">optional</span> and only used for generating captions and titles. Clip detection always runs locally for speed and zero cost.</p>
                <div className="bg-amber-500/10 border border-amber-500/20 rounded-lg px-3 py-2 mt-2">
                  <p className="text-xs text-amber-400/90">⚠️ Always save your API key when it's shown — most providers only display it once. If you lose it, you'll need to create a new one.</p>
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
                    <li>Go to <span className="text-white">Settings → Billing</span> and add a credit card</li>
                    <li>Click <span className="text-white">"Buy credits"</span> — enter $5 (minimum amount, lasts months)</li>
                    <li>Go to <span className="text-white">API Keys</span> in the sidebar</li>
                    <li>Click <span className="text-white">"Create Key"</span>, name it "ClipGoblin"</li>
                    <li>Copy the key immediately and save it somewhere safe — you will never see it again after closing this dialog</li>
                    <li>Paste the key into ClipGoblin's <span className="text-white">Settings → AI Provider → Claude</span></li>
                  </ol>
                </div>
                <div className="bg-surface-900 rounded-lg p-3 mt-3">
                  <p className="text-white font-medium">OpenAI (GPT)</p>
                  <p className="text-slate-400 text-xs mt-1">High-quality captions with natural language understanding.</p>
                  <ol className="list-decimal list-inside text-xs text-slate-300 mt-2 space-y-1.5">
                    <li>Go to <a href="https://platform.openai.com/" target="_blank" rel="noopener noreferrer" className="text-violet-400 hover:underline">platform.openai.com</a> and create an account</li>
                    <li>New accounts get $5 in free credits — these expire after 3 months</li>
                    <li>Go to <span className="text-white">API Keys</span> in the sidebar</li>
                    <li>Click <span className="text-white">"Create new secret key"</span>, name it "ClipGoblin"</li>
                    <li>Copy the key and paste it into <span className="text-white">Settings → AI Provider → OpenAI</span></li>
                  </ol>
                </div>
                <div className="bg-surface-900 rounded-lg p-3 mt-3">
                  <p className="text-white font-medium">Google Gemini</p>
                  <p className="text-slate-400 text-xs mt-1">Google's AI for caption generation.</p>
                  <ol className="list-decimal list-inside text-xs text-slate-300 mt-2 space-y-1.5">
                    <li>Go to <a href="https://aistudio.google.com/" target="_blank" rel="noopener noreferrer" className="text-violet-400 hover:underline">aistudio.google.com</a> and sign in with your Google account</li>
                    <li>Click <span className="text-white">"Get API key"</span> in the left sidebar</li>
                    <li>Click <span className="text-white">"Create API key"</span> and select a Google Cloud project</li>
                    <li>Copy the key into <span className="text-white">Settings → AI Provider → Gemini</span></li>
                  </ol>
                </div>
                <div className="mt-3 bg-surface-900 rounded-lg p-3">
                  <p className="text-white font-medium text-xs mb-2">Typical cost per clip</p>
                  <table className="w-full text-xs">
                    <thead><tr className="border-b border-surface-700 text-slate-500">
                      <th className="text-left px-3 py-2 font-medium">Model</th>
                      <th className="text-right px-3 py-2 font-medium">Per clip</th>
                    </tr></thead>
                    <tbody className="text-slate-300">
                      <tr className="border-b border-surface-700/50"><td className="px-3 py-1.5">GPT-5.4 Nano</td><td className="text-right px-3">~$0.0003</td></tr>
                      <tr className="border-b border-surface-700/50"><td className="px-3 py-1.5">Claude Haiku 3.5</td><td className="text-right px-3">~$0.001</td></tr>
                      <tr className="border-b border-surface-700/50"><td className="px-3 py-1.5">Claude Sonnet 4</td><td className="text-right px-3">~$0.004</td></tr>
                      <tr><td className="px-3 py-1.5">Gemini 2.0 Flash</td><td className="text-right px-3">~$0.0001</td></tr>
                    </tbody>
                  </table>
                </div>
              </HelpSection>
            )}
            {deepDiveOpen === 'publishing' && (
              <HelpSection icon="📤" title="Publishing walkthrough">
                <p className="text-white font-medium">Export first</p>
                <p>Clips must be rendered to a video file before uploading. Go to the Editor, adjust trim/captions/layout, then click Export.</p>
                <p className="text-white font-medium mt-3">YouTube</p>
                <p>Connect in Settings → Publishing accounts. Uses YouTube Data API v3 with resumable upload. Title, description, hashtags, and visibility are editable in the PublishComposer.</p>
                <p className="text-white font-medium mt-3">TikTok</p>
                <p>Connect in Settings → Publishing accounts. Currently sandbox mode — uploads go to drafts. Production approval pending.</p>
                <p className="text-white font-medium mt-3">Batch + scheduled</p>
                <p>Select multiple clips on the Clips page and upload them all at once. Unexported clips are auto-exported first. Schedule for a specific date/time and the background scheduler posts them when due (requires the app to be running).</p>
              </HelpSection>
            )}
          </div>
        </div>
      )}

      {/* Keyboard shortcuts */}
      <div>
        <h3 className="v4-section-label" style={{marginTop: 8, marginBottom: 10}}>Keyboard shortcuts</h3>
        <div className="v4-panel" style={{padding: 0, overflow: 'hidden'}}>
          <table className="w-full text-xs">
            <thead><tr className="border-b border-surface-700 text-slate-500">
              <th className="text-left px-4 py-2.5 font-medium">Shortcut</th>
              <th className="text-left px-4 py-2.5 font-medium">Action</th>
            </tr></thead>
            <tbody className="text-slate-300">
              <tr className="border-b border-surface-700/50"><td className="px-4 py-2 font-mono text-violet-300">Space</td><td className="px-4">Play / Pause video</td></tr>
              <tr className="border-b border-surface-700/50"><td className="px-4 py-2 font-mono text-violet-300">I</td><td className="px-4">Set clip start (In point)</td></tr>
              <tr className="border-b border-surface-700/50"><td className="px-4 py-2 font-mono text-violet-300">O</td><td className="px-4">Set clip end (Out point)</td></tr>
              <tr className="border-b border-surface-700/50"><td className="px-4 py-2 font-mono text-violet-300">← / →</td><td className="px-4">Seek backward / forward 5s</td></tr>
              <tr className="border-b border-surface-700/50"><td className="px-4 py-2 font-mono text-violet-300">Shift + ← / →</td><td className="px-4">Seek backward / forward 1 frame</td></tr>
              <tr className="border-b border-surface-700/50"><td className="px-4 py-2 font-mono text-violet-300">Ctrl + Z</td><td className="px-4">Undo</td></tr>
              <tr><td className="px-4 py-2 font-mono text-violet-300">Ctrl + Shift + Z</td><td className="px-4">Redo</td></tr>
            </tbody>
          </table>
        </div>
      </div>
    </div>
  )
}
