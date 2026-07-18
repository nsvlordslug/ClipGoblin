import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { ChevronDown, ChevronRight, Search } from 'lucide-react'
import { resetTesterChecklistDismissal } from '../lib/testerChecklist'
import { version as appVersion } from '../../package.json'

type DeepDiveTopic =
  | 'getting-started'
  | 'clip-sources'
  | 'personalization'
  | 'editor'
  | 'montage'
  | 'ai-provider'
  | 'publishing'

const FEATURED_TOPICS: Array<{
  id: DeepDiveTopic
  icon: string
  title: string
  desc: string
  searchTerms: string
}> = [
  {
    id: 'getting-started',
    icon: '🎯',
    title: 'Getting started',
    desc: 'Connect Twitch, analyze a VOD, review clips, and publish your first result.',
    searchTerms: 'setup twitch vod analyze first clip game',
  },
  {
    id: 'clip-sources',
    icon: '🎥',
    title: 'Medal, OBS, Meld, and local clips',
    desc: 'Import recording folders, save replay buffers, and keep large libraries organized.',
    searchTerms: 'source import folder medal obs meld replay buffer local video auto import',
  },
  {
    id: 'personalization',
    icon: '🎯',
    title: 'Teach ClipGoblin your taste',
    desc: 'Use ratings and edit-issue feedback to improve future ranking and clip boundaries.',
    searchTerms: 'personalized detection learn rating good meh boring starts late cuts off early feedback',
  },
  {
    id: 'editor',
    icon: '🎨',
    title: 'Editor, captions, and branding',
    desc: 'Use Context Fit, Split, PiP, timed subtitles, branding media, and safe positioning.',
    searchTerms: 'edit captions subtitles context fit blur black bars letterbox branding split picture in picture pip layout size',
  },
  {
    id: 'montage',
    icon: '🎬',
    title: 'Build a montage',
    desc: 'Arrange saved clips, choose cuts or cross dissolves, then export long-form or vertical.',
    searchTerms: 'montage compilation combine join clips sequence transition dissolve crossfade youtube shorts export',
  },
  {
    id: 'ai-provider',
    icon: '🧠',
    title: 'Free mode and optional BYOK',
    desc: 'Understand local detection, AI clip judging, provider setup, and cost warnings.',
    searchTerms: 'ai provider byok free claude openai gemini cost high detection sonnet judge key',
  },
  {
    id: 'publishing',
    icon: '📤',
    title: 'YouTube and TikTok publishing',
    desc: 'Connect accounts, publish or schedule clips, and understand delayed platform status.',
    searchTerms: 'publish upload youtube tiktok accepted private visibility schedule batch delayed processing',
  },
]

/** Collapsible deep-dive section */
function HelpSection({ icon, title, children, defaultOpen = false }: {
  icon: string
  title: string
  children: React.ReactNode
  defaultOpen?: boolean
}) {
  const [open, setOpen] = useState(defaultOpen)
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
function FeaturedCard({ icon, title, desc, onClick }: { icon: string; title: string; desc: string; onClick: () => void }) {
  return (
    <button type="button" className="v4-faq w-full text-left" onClick={onClick} style={{cursor: 'pointer'}}>
      <div className="v4-faq-q">{icon} {title}</div>
      <div className="v4-faq-a">{desc}</div>
    </button>
  )
}

export default function HelpGuide() {
  const [search, setSearch] = useState('')
  const [deepDiveOpen, setDeepDiveOpen] = useState<DeepDiveTopic | null>(null)
  const navigate = useNavigate()
  const normalizedSearch = search.trim().toLocaleLowerCase()
  const matchesSearch = (...values: string[]) => (
    !normalizedSearch
    || values.some(value => value.toLocaleLowerCase().includes(normalizedSearch))
  )
  const visibleTopics = FEATURED_TOPICS.filter(topic => matchesSearch(
    topic.title,
    topic.desc,
    topic.searchTerms,
  ))

  const openDeepDive = (topic: DeepDiveTopic) => {
    setDeepDiveOpen(topic)
    requestAnimationFrame(() => {
      document.getElementById('help-deep-dive')?.scrollIntoView({ behavior: 'smooth', block: 'start' })
    })
  }

  return (
    <div className="space-y-4 max-w-[1100px]">
      {/* Page header with search input on right */}
      <div className="v4-page-header">
        <div>
          <div className="v4-page-title">Help &amp; Guide ?</div>
          <div className="v4-page-sub">FAQs, setup walkthroughs, and troubleshooting</div>
        </div>
        <div className="relative">
          <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-slate-500" />
          <input
            className="v4-input"
            aria-label="Search help articles"
            placeholder="Search help articles..."
            value={search}
            onChange={e => setSearch(e.target.value)}
            style={{width: 260, paddingLeft: 34}}
          />
        </div>
      </div>

      {/* Featured topics */}
      <div
        className="grid gap-3.5"
        style={{gridTemplateColumns: 'repeat(auto-fit, minmax(250px, 1fr))'}}
      >
        {visibleTopics.map(topic => (
          <FeaturedCard
            key={topic.id}
            icon={topic.icon}
            title={topic.title}
            desc={topic.desc}
            onClick={() => openDeepDive(topic.id)}
          />
        ))}
      </div>
      {visibleTopics.length === 0 && (
        <div className="v4-panel px-4 py-5 text-sm text-slate-400">
          No walkthrough matched that search. Matching FAQs, if any, appear below.
        </div>
      )}

      {/* For testers */}
      <div>
        <h3 className="v4-section-label" style={{marginTop: 8, marginBottom: 10}}>🧪 For testers</h3>
        <div className="v4-panel" style={{padding: 16}}>
          <p className="text-sm text-slate-300 mb-3">
            You're running <b>ClipGoblin v{appVersion}</b> — a pre-release build. Here's what would help
            most with feedback:
          </p>
          <ul className="text-xs text-slate-400 space-y-1.5 mb-4 ml-4 list-disc">
            <li><b className="text-white">Smoke-test the whole pipeline:</b> connect Twitch or import a local clip → review → edit → export → ship to YouTube / TikTok.</li>
            <li><b className="text-white">Try edge cases:</b> very short VODs (&lt; 30 min), very long VODs (3h+), VODs with little chat, VODs while you're offline.</li>
            <li><b className="text-white">Teach detection:</b> rate varied clips Good, Meh, and Boring, then mark timing issues such as Starts too late or Cuts off early.</li>
            <li><b className="text-white">Exercise imported media:</b> scan nested Medal game folders, save an OBS/Meld replay, generate subtitles, and try Context Fit with blur, black bars, or branding.</li>
            <li><b className="text-white">Build a montage:</b> mix clips from more than one source, reorder them, export both formats, and replay the finished MP4.</li>
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
              <li>TikTok view counts are unavailable until the optional video.list scope is approved.</li>
              <li>TikTok may limit posts to private while Direct Post approval is pending. Accepted posts can take several minutes to appear.</li>
              <li>Waveform thumbnails on clip rows are decorative placeholders (not real audio shape yet).</li>
              <li>Not code-signed yet — Windows SmartScreen will show "Unknown publisher". Click <b>More info → Run anyway</b> once.</li>
            </ul>
          </div>
        </div>
      </div>

      {/* Frequently asked */}
      <div>
        <h3 className="v4-section-label" style={{marginTop: 8, marginBottom: 10}}>Frequently asked</h3>
        <div className="space-y-2">
          {matchesSearch('cublas cuda gpu transcription missing dll') && <FaqItem q='Why did my VOD analysis fail with "cublas64_12.dll missing"?'>
            You need CUDA Toolkit 12 installed for GPU transcription. Download it from NVIDIA,
            or switch to CPU mode in Settings → Detection → GPU off.
          </FaqItem>}
          {matchesSearch('confidence signals audio chat transcript scene twitch community ai judge') && <FaqItem q="How does clip detection decide confidence?">
            Confidence fuses audio, scene, transcript, chat, Twitch community clips, and optional AI-judge evidence when those signals are available. Detection still works locally without BYOK.
          </FaqItem>}
          {matchesSearch('personalized detection learn ratings good meh boring future local') && <FaqItem q="How does personalized detection learn from me?">
            Turn on Personalized detection feedback in Settings → Detection. Good, Meh, and Boring choices teach moment ranking; structured edit issues teach clip boundaries. The profile stays on this PC and affects future analyses whether you use Free mode or BYOK.
          </FaqItem>}
          {matchesSearch('starts too late cuts off early choose multiple both issues') && <FaqItem q="Can I choose more than one edit issue?">
            Yes. Choose every issue that applies. For example, select both <b>Starts too late</b> and <b>Cuts off early</b> when a joke loses its setup and punchline. Free-form notes are saved for your reference; the structured buttons are what boundary learning reads.
          </FaqItem>}
          {matchesSearch('reanalyze vod sensitivity same vod personalization') && <FaqItem q="Can I re-analyze a VOD with different sensitivity or new feedback?">
            Yes — open the VOD card menu and click Re-analyze VOD. The new run uses the current sensitivity and your latest local personalization profile. A successful re-analysis replaces that VOD's current generated clips, while its saved learning evidence remains, so finish or export edits you need before starting it.
          </FaqItem>}
          {matchesSearch('medal obs meld import previews subtitles source folder') && <FaqItem q="How do imported Medal, OBS, or Meld clips work?">
            Open Settings → Clip Sources, choose the recording folder, scan it, and import a folder or selected files. Medal scans nested game folders. Imported clips appear under source tabs on the Clips page and use the same editor, local subtitle generation, exports, and publishing flow.
          </FaqItem>}
          {matchesSearch('game detect automatic set game twitch medal folder hashtags titles') && <FaqItem q="How is the game name detected?">
            Medal imports use their game-folder label. Publishing copy can also infer known games from saved metadata, titles, tags, and transcript clues. Twitch does not provide reliable historical game data for every VOD, so click <b>Set game</b> on a VOD when it is missing or wrong; that correction applies to its clips.
          </FaqItem>}
          {matchesSearch('montage compilation combine clips export sequence') && <FaqItem q="How do I combine clips into a montage?">
            Open Montage, create or select a project, then click clips in the library to add them. The preview automatically continues through the ordered sequence. Use the arrows to reorder it, choose YouTube 16:9 or Shorts 9:16, select Straight cut or Cross dissolve, and click Export Montage. ClipGoblin applies each clip's saved editor settings before joining them and stores the finished MP4 in the export folder.
          </FaqItem>}
          {matchesSearch('tiktok accepted private delayed missing lock tab processing') && <FaqItem q='TikTok says "Accepted," but where is my clip?'>
            Accepted means TikTok received the upload; it does not mean processing is finished. Private posts do not return a public link and may take several minutes to appear. Check Profile → Private (the lock tab) in the TikTok mobile app, and do not upload another copy while the app still shows Accepted.
          </FaqItem>}
          {matchesSearch('ship rate dashboard analytics') && <FaqItem q='What does "Ship rate" on the dashboard mean?'>
            % of detected highlights you actually published, vs deleted or left as drafts.
            Lower ship rates usually mean sensitivity is too high.
          </FaqItem>}
          {matchesSearch('analytics view counts youtube tiktok video list instagram') && <FaqItem q="How do I get real view counts on the Analytics page?">
            Open Analytics and click <b>Refresh stats</b>. The app currently polls the
            YouTube Data API using your existing scope. TikTok view counts will become
            available after the optional <code className="text-violet-300">video.list</code>
            scope is approved. Instagram isn't wired up yet.
          </FaqItem>}
          {matchesSearch('auto ship high confidence schedule') && <FaqItem q='What is "Auto-ship high-confidence"?'>
            When enabled in Settings → Detection, any clip scoring 90%+ confidence
            after analysis is automatically queued to ship in 5 minutes to every
            connected platform. You'll see a banner on the Dashboard and the uploads
            appear on the Scheduled page — cancel any of them before the timer expires.
            The scheduler renders any missing exports on the fly, so you don't need to
            click Export manually before the timer fires.
          </FaqItem>}
          {matchesSearch('twitch community clips viewer streamer boost signal') && <FaqItem q='What is "Use Twitch community clips"?'>
            When enabled in Settings → Detection, the analysis pipeline boosts
            moments where viewers already made their own Twitch clip. Clips with
            more views weigh more. No extra Twitch permission needed — uses the
            existing connected scope.
          </FaqItem>}
        </div>
      </div>

      {/* Deep dives — expanded only when the matching featured card is clicked */}
      {deepDiveOpen && (
        <div id="help-deep-dive" style={{scrollMarginTop: 16}}>
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
              <HelpSection key="getting-started" icon="🎯" title="Getting started" defaultOpen>
                <p><span className="text-white font-medium">1. Connect accounts</span> — Open Settings → Accounts. Connect Twitch to fetch VODs, then connect YouTube or TikTok when you are ready to publish.</p>
                <p><span className="text-white font-medium">2. Choose a source</span> — Use the VODs page for Twitch streams, or Settings → Clip Sources for Medal, OBS, Meld, and local recordings.</p>
                <p><span className="text-white font-medium">3. Check the game</span> — Medal uses the game-folder name. For Twitch, click Set game on the VOD when the historical category is missing or wrong; it improves game-aware detection and publishing copy.</p>
                <p><span className="text-white font-medium">4. Analyze a Twitch VOD</span> — Download the VOD if needed, then click Analyze. Local audio, scene, transcript, chat, and available Twitch clip signals work without an API key.</p>
                <p><span className="text-white font-medium">5. Review clips</span> — Open Clips, use the source tabs to find the right library, play the preview, and open Clip feedback to rate the moment or report bad boundaries.</p>
                <p><span className="text-white font-medium">6. Edit</span> — The Editor is split into Edit, Captions, and Publish tabs. Set trim and layout first, generate or edit timed subtitles, then prepare platform copy.</p>
                <p><span className="text-white font-medium">7. Export and publish</span> — Save the clip, choose a platform in Publish, confirm its visibility settings, and export/upload.</p>
              </HelpSection>
            )}
            {deepDiveOpen === 'clip-sources' && (
              <HelpSection key="clip-sources" icon="🎥" title="Medal, OBS, Meld, and local clips" defaultOpen>
                <p className="text-white font-medium">Medal libraries</p>
                <p>Open Settings → Clip Sources → Medal clips and choose the parent capture folder that contains all game folders. Click Scan all games, then use Import folder for one game, Import selected for hand-picked clips, or Import all. Large imports are processed in safe batches automatically.</p>
                <p className="text-white font-medium mt-3">OBS and Meld</p>
                <p>Choose each recorder's replay/clip folder. Turn on Auto-import new clips if you want ClipGoblin to pick up new files while it is running. For OBS, enter the WebSocket port and password, save, and click Test. Test confirms recorder access; Mark moment saves a local highlight marker; Save replay asks the recorder to create a clip and imports the new file.</p>
                <p className="text-white font-medium mt-3">Organization and editing</p>
                <p>Imported clips appear under Medal, OBS, Meld, or Local tabs on the Clips page. Medal clips are grouped by readable game-folder name. Open any imported clip in the same Editor used by Twitch clips, then generate local speech-to-text subtitles from the Captions tab.</p>
                <p className="text-xs text-slate-500">ClipGoblin does not upload or alter source recordings during import. Keep the original file in place so previews, subtitle generation, and exports can read it.</p>
              </HelpSection>
            )}
            {deepDiveOpen === 'personalization' && (
              <HelpSection key="personalization" icon="🎯" title="Teach ClipGoblin your taste" defaultOpen>
                <p><span className="text-white font-medium">Turn it on</span> — Open Settings → Detection and enable Personalized detection feedback. The status beneath it shows whether ranking and boundary learning have enough varied evidence yet.</p>
                <p><span className="text-white font-medium">Rate moment quality</span> — On the Clips page, open the checklist icon on a clip and choose Good, Meh, or Boring. Use at least two different ratings so the app can learn a contrast instead of assuming every moment is the same.</p>
                <p><span className="text-white font-medium">Report edit issues</span> — Select every chip that applies: Starts too late, Cuts off early, Too long, Wrong moment, or Duplicate. Both timing chips can be selected together when a clip loses context at both ends.</p>
                <p><span className="text-white font-medium">What changes</span> — Future analyses gently reorder candidates toward your taste, while boundary feedback adjusts how much setup and payoff to preserve. Re-analyzing the same VOD also uses the current profile. Normal quality gates remain in charge.</p>
                <p><span className="text-white font-medium">What stays local</span> — Ratings, edit behavior, and the learned profile stay in ClipGoblin's local database. They improve Free mode and BYOK-assisted detection alike. Free-form notes are saved for your reference but are not interpreted as training instructions.</p>
                <p className="text-xs text-slate-500">Settings → Detection also lets you copy the learning history for troubleshooting or reset the profile and start over.</p>
              </HelpSection>
            )}
            {deepDiveOpen === 'editor' && (
              <HelpSection key="editor" icon="🎨" title="Editor, captions, and branding" defaultOpen>
                <p><span className="text-white font-medium">Edit tab</span> — Adjust trim, aspect ratio, export preset, and layout. Full Frame center-crops. Context Fit keeps the entire scene visible and lets you choose a soft video blur, clean black bars, or branding behind it. Its Video placement slider moves the main frame up or down. Split and PiP keep gameplay and a second panel visible together.</p>
                <p><span className="text-white font-medium">Branding media</span> — Context Fit, Split, and PiP can replace the blur or facecam panel with your PNG, JPG, WebP, or animated GIF. Use Change → choose a supported layout → Branding, then choose the asset. Split ratio and PiP position/size controls also apply to branding.</p>
                <p><span className="text-white font-medium">Captions tab</span> — Turn Subtitles on and click Generate Subtitles (Speech-to-Text). It uses the bundled local Whisper model, including for imported clips. Existing subtitles can be regenerated with the refresh icon or edited one segment at a time.</p>
                <p><span className="text-white font-medium">Timing and placement</span> — Timed captions show words as they are spoken and leave real pauses blank. Drag captions on the preview or use Position and Offset. Safe-zone and facecam warnings appear before text is likely to be clipped or covered.</p>
                <p><span className="text-white font-medium">Styles and size</span> — Pick a caption style and use the 75–125% Size slider. Tape Riot uses ClipGoblin green and purple, Paper Mischief uses stacked paper-like depth, and Goblin Bite adds a distressed horror treatment. Very long words can shrink automatically to stay inside a vertical TikTok/Shorts frame.</p>
                <p><span className="text-white font-medium">Publish tab</span> — Generate or edit platform copy, pick visibility and platform options, then save, download, schedule, or upload.</p>
              </HelpSection>
            )}
            {deepDiveOpen === 'montage' && (
              <HelpSection key="montage" icon="🎬" title="Build a montage" defaultOpen>
                <p><span className="text-white font-medium">1. Save clip edits</span> — Finish each clip's trim, layout, branding, and captions in the Editor before adding it. Montage export uses the latest saved settings.</p>
                <p><span className="text-white font-medium">2. Create a project</span> — Open Montage and use New when you want a separate compilation. Projects and their clip order are saved on this PC.</p>
                <p><span className="text-white font-medium">3. Find clips</span> — Search by title or game and filter by Twitch, Medal, OBS, Meld, or Local. Click a clip to add it. Playing the preview continues through every following clip; select a timeline or sequence item to start from that point.</p>
                <p><span className="text-white font-medium">4. Set the sequence</span> — Use the up/down arrows to reorder clips. Edit opens the source clip without removing it from the project.</p>
                <p><span className="text-white font-medium">5. Choose a format</span> — YouTube 16:9 creates a 1920×1080 compilation. Shorts 9:16 switches the preview to a vertical canvas and creates a 1080×1920 export capped at YouTube's three-minute Shorts limit. Mixed source sizes are fitted safely instead of stretching.</p>
                <p><span className="text-white font-medium">6. Choose transitions</span> — Straight cut keeps clips back to back. Cross dissolve overlaps the outgoing and incoming clips while blending both picture and sound. The choice is saved with the project.</p>
                <p><span className="text-white font-medium">7. Export</span> — Click Export Montage and leave ClipGoblin open while it renders. The progress display advances clip by clip, then joins the sequence. When it finishes, replay the full output or open its folder.</p>
              </HelpSection>
            )}
            {deepDiveOpen === 'ai-provider' && (
              <HelpSection key="ai-provider" icon="🧠" title="Free mode and optional BYOK" defaultOpen>
                <p>Free mode includes local clip detection, scoring, personalization, titles, publishing copy, and Whisper subtitles with no API charges. A provider key is optional and is billed directly by that provider, not by ClipGoblin.</p>
                <div className="bg-amber-500/10 border border-amber-500/20 rounded-lg px-3 py-2 mt-2">
                  <p className="text-xs text-amber-400/90">Always save your API key when it is shown. Most providers only display a secret once; if you lose it, create a replacement and revoke the old key.</p>
                </div>
                <div className="bg-surface-900 rounded-lg p-3 mt-3">
                  <p className="text-white font-medium">Free Mode</p>
                  <p className="text-slate-400 text-xs mt-1">No key required. Local signals and your personal feedback profile find clips; pattern-based tools create titles and post copy; local Whisper creates subtitles.</p>
                </div>
                <div className="bg-surface-900 rounded-lg p-3 mt-3">
                  <p className="text-white font-medium">What BYOK can add</p>
                  <p className="text-slate-400 text-xs mt-1">Enable paid caption-copy or title generation under Settings → AI. AI clip detection is a separate, opt-in toggle under Settings → Detection; it reads transcript context to find moments local signals may miss and filter loud-but-empty moments.</p>
                  <p className="text-slate-400 text-xs mt-2">Claude users can choose the clip-detection model and optionally enable a Sonnet final pass. High detection and Sonnet final pass show an Accept/Deny cost warning before they change. After your first BYOK analysis, Settings shows your measured average per VOD and 30-day total.</p>
                </div>
                <div className="bg-surface-900 rounded-lg p-3 mt-3">
                  <p className="text-white font-medium">Claude (Anthropic)</p>
                  <p className="text-slate-400 text-xs mt-1">Supports titles, publishing copy, and the optional transcript clip judge.</p>
                  <ol className="list-decimal list-inside text-xs text-slate-300 mt-2 space-y-1.5">
                    <li>Go to <a href="https://console.anthropic.com/" target="_blank" rel="noopener noreferrer" className="text-violet-400 hover:underline">console.anthropic.com</a> and create an account</li>
                    <li>Set up API billing using the provider's current instructions</li>
                    <li>Go to <span className="text-white">API Keys</span> in the sidebar</li>
                    <li>Click <span className="text-white">"Create Key"</span>, name it "ClipGoblin"</li>
                    <li>Copy the key immediately and save it somewhere safe — you will never see it again after closing this dialog</li>
                    <li>Paste it into <span className="text-white">Settings → AI → Claude</span>, choose models, and click Test Connection</li>
                  </ol>
                </div>
                <div className="bg-surface-900 rounded-lg p-3 mt-3">
                  <p className="text-white font-medium">OpenAI (GPT)</p>
                  <p className="text-slate-400 text-xs mt-1">Supports titles, publishing copy, and the optional transcript clip judge.</p>
                  <ol className="list-decimal list-inside text-xs text-slate-300 mt-2 space-y-1.5">
                    <li>Go to <a href="https://platform.openai.com/" target="_blank" rel="noopener noreferrer" className="text-violet-400 hover:underline">platform.openai.com</a> and create an account</li>
                    <li>Set up API billing using the provider's current instructions</li>
                    <li>Go to <span className="text-white">API Keys</span> in the sidebar</li>
                    <li>Click <span className="text-white">"Create new secret key"</span>, name it "ClipGoblin"</li>
                    <li>Paste it into <span className="text-white">Settings → AI → OpenAI</span> and click Test Connection</li>
                  </ol>
                </div>
                <div className="bg-surface-900 rounded-lg p-3 mt-3">
                  <p className="text-white font-medium">Google Gemini</p>
                  <p className="text-slate-400 text-xs mt-1">Supports titles, publishing copy, and the optional transcript clip judge.</p>
                  <ol className="list-decimal list-inside text-xs text-slate-300 mt-2 space-y-1.5">
                    <li>Go to <a href="https://aistudio.google.com/" target="_blank" rel="noopener noreferrer" className="text-violet-400 hover:underline">aistudio.google.com</a> and sign in with your Google account</li>
                    <li>Click <span className="text-white">"Get API key"</span> in the left sidebar</li>
                    <li>Click <span className="text-white">"Create API key"</span> and select a Google Cloud project</li>
                    <li>Copy it into <span className="text-white">Settings → AI → Gemini</span> and click Test Connection</li>
                  </ol>
                </div>
                <p className="text-xs text-slate-500 mt-3">Provider prices and model names change. Treat the in-app measured cost as historical guidance, review the warning before higher-usage options, and use your provider console for authoritative billing.</p>
              </HelpSection>
            )}
            {deepDiveOpen === 'publishing' && (
              <HelpSection key="publishing" icon="📤" title="YouTube and TikTok publishing" defaultOpen>
                <p className="text-white font-medium">Prepare the clip</p>
                <p>Use the Editor's Edit and Captions tabs first, then open Publish. Select YouTube and/or TikTok, review the generated copy and platform settings, and click the export/upload command. ClipGoblin renders a fresh file when the saved edit needs one.</p>
                <p className="text-white font-medium mt-3">YouTube</p>
                <p>Connect under Settings → Accounts. Set title, description, hashtags, and visibility in Publish. A completed public or unlisted upload returns a View on YouTube link.</p>
                <p className="text-white font-medium mt-3">TikTok</p>
                <p><span className="text-white font-medium">Post directly</span> uses the privacy, interaction, and disclosure settings TikTok returns for your account. While Direct Post review is pending, TikTok may allow only Only me (private). Accepted means the upload was received and is still processing; private uploads have no public link and can take several minutes to appear under Profile → Private (lock tab) in the mobile app.</p>
                <p><span className="text-white font-medium">Send to drafts</span> transfers the rendered video to a TikTok inbox notification without publishing it. Open that notification in TikTok to edit, add the caption and audience, and publish. TikTok's draft API transfers the video only, so ClipGoblin captions and hashtags must be added again in TikTok.</p>
                <p className="text-xs text-amber-300/90">Do not immediately upload another copy while TikTok shows Accepted. Retry only after a real failure or when you intentionally choose Upload another copy.</p>
                <p className="text-white font-medium mt-3">Batch + scheduled</p>
                <p>Use Select on the Clips page for batch upload. Unexported clips are rendered first. Schedule for a specific date/time from Publish or the batch dialog; the app must be running when the scheduled time arrives. Ambiguous TikTok processing states remain visible instead of being treated as a confirmed public post.</p>
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
