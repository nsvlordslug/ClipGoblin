# ClipGoblin Detection Pipeline Expansion — Roadmap

**Status:** approved 2026-04-20. Execution starts with Phase 5 cleanup after a prompt-diff review pass on `post_captions.rs`.
**Current release:** v1.2.2 ([published](https://github.com/nsvlordslug/ClipGoblin/releases/latest))
**Landing:** https://clipgoblin.mindrotstudios.com
**Goal:** match or exceed StreamLadder / Eklipse.gg detection quality while staying 100% local, BYOK-friendly, and maintainable without per-game-patch treadmills.

---

## Current pipeline (as of v1.2.2)

`analyze_vod()` in [commands/vod.rs:919](../src-tauri/src/commands/vod.rs) is the entrypoint. Four signal sources feed the 7-stage selector:

1. **Audio intensity** — RMS spikes via ffmpeg `astats` ([vod.rs:239](../src-tauri/src/commands/vod.rs), [clip_selector.rs:205](../src-tauri/src/clip_selector.rs))
2. **Whisper transcript** — local whisper-rs, CPU/CUDA, keyword→emotion mapping ([vod.rs:479](../src-tauri/src/commands/vod.rs), [clip_selector.rs:255](../src-tauri/src/clip_selector.rs))
3. **Chat replay** — message-rate peaks (fallback only, [vod.rs:1641](../src-tauri/src/commands/vod.rs))
4. **Twitch community clips** — Helix API, 48hr window ([twitch.rs:519](../src-tauri/src/twitch.rs), [clip_selector.rs:307](../src-tauri/src/clip_selector.rs))

Signals fuse into `FusedMoment` within a 10s window → scoring (6 viral dimensions) → boundary optimization → rejection → dedup → min-gap → diversity selection → final clips.

User controls today: `detection_sensitivity` (Low/Medium/High) and `use_twitch_community_clips` toggle in Settings.

---

## Architectural principles

1. **Every new detection method is a signal generator feeding the existing selector.** Do not modify fusion/scoring/selection logic unless explicitly required — the existing pipeline handles multi-signal moments correctly and gives them score bonuses. New generators slot in alongside `generate_audio_candidates()`, `generate_community_candidates()`, etc.

2. **Vision = new signal source, not a post-hoc score multiplier.** When Phase 6 (BYOK vision) ships, the vision model's output becomes a fused signal (gets the existing multi-source bonus for free). Keeps the pipeline honest and doesn't break principle #1.

3. **Same ranker bridges BYOK and Free paths.** For titles/captions, generate N candidates (from LLM or templates), score with the same criteria, pick the winner. Everything is reusable.

4. **Zero ongoing cost to end users in base app.** BYOK is optional. Free path must stay good.

5. **One-time-purchase friendly, no subscription required.**

---

## Phase plan (ordered)

### Phase 5 — Scaffolding Cleanup (Day 1, BLOCKS ALL OTHERS)

**Effort:** ~1 day

Delete or complete these stub modules before adding new signals. Leaving them means new code might start using them, creating two parallel abstractions.

- [pipeline.rs](../src-tauri/src/pipeline.rs) — type defs only, never imported from commands/
- [engine.rs](../src-tauri/src/engine.rs) — stub functions, no callers
- [audio_signal.rs](../src-tauri/src/audio_signal.rs) — duplicate of work actually done in vod.rs:239
- [scene_signal.rs](../src-tauri/src/scene_signal.rs) — scene-cut detection stubbed (revived in Phase 3)
- [transcript_signal.rs](../src-tauri/src/transcript_signal.rs) — stubbed
- [clip_fusion.rs](../src-tauri/src/clip_fusion.rs) — actual fusion is in clip_selector.rs:327
- [clip_ranker.rs](../src-tauri/src/clip_ranker.rs) — actual ranking in clip_selector.rs:891
- [clip_labeler.rs](../src-tauri/src/clip_labeler.rs) — labeling inline in clip_selector.rs:937

**Plus:** audit v4 UI copy for features not in live code (scene-change mentions, etc.) — remove or gate behind the phase that implements them.

**Success criteria:** no dead modules in `src-tauri/src/`, no UI copy promising unshipped features.

---

### Phase 6.0 — BYOK Feature Toggle Framework (Days 2–3, BLOCKS 6 + 12)

**Effort:** ~2 days

Prerequisite for Phase 6 (detection) and Phase 12 (titles/captions). Gives users granular control over which AI steps burn their API budget.

**Deliverables:**

1. **Extended `ai_settings` JSON** — three new bools in the existing settings blob: `ai_for_detection`, `ai_for_titles`, `ai_for_captions`. No DB migration (ai_settings is already JSON per [commands/settings.rs](../src-tauri/src/commands/settings.rs)).
2. **Settings UI panel** — three independent checkboxes with live cost estimates.
3. **Cost estimator module** — reads `ai_usage_log` table, rolling average from last 10 VODs, formats USD, updates on toggle.
4. **Pre-run cost preview modal** — "Analyze" button shows estimate before firing.
5. **Usage logger** — wraps every AI call in post_captions.rs and future vision_signal.rs. New table:
   ```sql
   ai_usage_log(timestamp, feature, provider, model, tokens_in, tokens_out, cost_usd)
   ```
6. **Fallback routing** — when a toggle is OFF, that feature routes to Free path automatically.

**Defaults:**
| State | Detection | Titles | Captions |
|---|---|---|---|
| No BYOK key | off | off | off |
| Just set a key | **off** (expensive) | on | on |

**Design:**
```
AI Features (BYOK)
─────────────────────────────────────────
Provider: [Anthropic ▾]
Model:    [Claude Haiku 4.5 ▾]   <-- single model for all toggles
API key:  [•••••••••] ✓ valid

Use AI for:
[ ] Highlight detection (vision)     ~$0.05 per VOD
[✓] Titles                           ~$0.02 per VOD
[✓] Captions (social + money quote)  ~$0.05 per VOD

Estimated cost per VOD: $0.07
(based on your last 10 analyses)
```

**Success criteria:** user can toggle any of the three independently, costs update live, OFF toggles silently route to Free path.

---

### Phase 1 — Chat Content + Game Config + Emote Density (Week 1–2)

**Effort:** ~2.5 days

**Deliverables:**

1. **Per-game keyword configuration.** TOML files at `config/games/{game_id}.toml` keyed by Twitch `game_id`. Each contains:
   - Emotion keyword maps specific to that game (e.g., "ace", "clutch", "1v5" for Valorant; "gen pop", "hooked", "iri" for DBD)
   - Signal weight overrides
   - Per-game thresholds
2. **game_id auto-loading.** Pull `game_id` from VOD metadata via Twitch API, load matching config, merge with base defaults. Fall back to generic config if game unknown.
3. **Chat content analysis (promoted from fallback to primary).** Refactor [analyze_via_chat() in vod.rs:1641](../src-tauri/src/commands/vod.rs) so it:
   - Always runs, not just as fallback
   - Produces new signal type: chat content keywords (not just message rate)
   - Tokenizes chat per 10-second window, counts game-event keywords
   - Emits candidates where event-keyword density exceeds threshold
4. **Expanded transcript keyword map.** Support per-game keyword overrides on top of universal reaction vocab in [clip_selector.rs:255](../src-tauri/src/clip_selector.rs).
5. **Emote density signal (bolt-on, ~0.5 day).** Count occurrences of top 20 Twitch emotes per 10-second window: `KEKW`, `OMEGALUL`, `Pog`, `5Head`, `Sadge`, `MonkaS`, etc. Emote density = much stronger reaction signal than message rate. Source emote list from BTTV/FFZ/7TV public APIs or periodic manual update.

**Success criteria:**
- Valorant-specific keywords load for Valorant VODs
- Chat content shows up as distinct signal source in `FusedMoment::signal_sources`
- Emote bursts emit their own candidates
- Existing test VODs produce same-or-better selection quality than current baseline

---

### Phase 6 — BYOK Vision Analysis (Week 2–3)

**Effort:** ~6–8 days

Optional frame-level vision analysis using the user's BYOK API key. Plugs into the Phase 6.0 toggle framework.

**Deliverables:**

1. **Frame extraction pipeline.** For candidate moments above a configurable score threshold:
   - Extract 3–5 keyframes via ffmpeg at moment center, ±2s, ±4s
   - Downscale to 1024px long edge (token cost control)
   - Base64 JPEG encoding
2. **Vision prompt module.** Reuses existing BYOK plumbing (Anthropic, OpenAI, Google via [ai_provider.rs](../src-tauri/src/ai_provider.rs)):
   - Prompt: identify game event, rate clip-worthiness 0–10, return JSON
   - Support for user-selected model (Haiku/Sonnet/Opus, etc.)
3. **Signal integration.** New `generate_vision_candidates()`:
   - **Treats vision as a new signal source** — fuses normally via the existing 10s window, gets the multi-source bonus for free. Does NOT post-hoc mutate existing scores.
   - Only analyzes candidates above threshold (cost control)
   - Adds event tags from vision response
4. **Cost control.** Already built in Phase 6.0 toggle framework.

**Success criteria:**
- Vision enriches titles/tags for candidate moments
- Cost shown to user before analysis matches actual API bill within 10%
- User can disable entirely without breaking rest of pipeline

---

### Phase 12 — Title & Caption Quality Loop (Week 3–4)

**Effort:** ~8–10 days

**Applies to both BYOK and Free paths.** Reopens the previously-finalized prompts in [post_captions.rs](../src-tauri/src/post_captions.rs) — requires explicit Slug sign-off on the diff before merging.

**Deliverables:**

1. **Read and audit current post_captions.rs.** Produce a concrete prompt diff for review before writing any code.
2. **Context enrichment plumbing.** Plumb into the existing `generate_llm*` input structs:
   - Emote density summary ("chat had 87 KEKWs + 34 OMEGALULs")
   - Community-clip title (the viewer's own title — free wit from fans)
   - Game-event tags (from Phase 2)
   - Facecam reaction tag (from Phase 4.5)
   - Stakes/context from transcript
3. **Ranker/scorer module.** Shared by both paths. Scores a title higher if:
   - Contains a number/stake (`"1v5"`, `"0hp"`, `"12s"`)
   - Short (<50 chars for TikTok, <42 for YouTube Shorts mobile)
   - Has an emotional word
   - Specific not generic (demotes "nice play", "crazy moment")
   - No repeat of recent uploads (token overlap <50%)
   - No banned words (anti-cliché filter)
4. **Prompt rewrite (BYOK)** — requires Slug review:
   - **Hard structure constraint** — one of three patterns:
     - `{stake} → {outcome}` ("down 0-12 → 1v5 ACE")
     - `{emotion_word}: {specific_detail}` ("SPEECHLESS: one-tap through smoke")
     - `"{money_quote}" {twist}` (`"I can't miss" — misses next shot`)
   - **Anti-cliché banlist** — `insane, crazy, epic, literally, you won't believe, OMG, SHOCKING, unbelievable, wild`
   - **Required elements** — at least one of: number/stake, emotional word, or specific visual/action detail
   - **Strict char limit** — 40 for TikTok shorts
   - **Few-shot examples** — 6–10 exemplary titles, streamer's own history once available
   - **Output format** — JSON array of 5 candidates
   - **Tone inheritance** — strict: if tone is `quote`, MUST include transcript phrase; if `observation`, MUST be third-person
5. **Caption prompt rewrite (BYOK)** — requires Slug review:
   - Two-part output: `hook_line` (first ~50 chars scroll-stopper) + `body`
   - Hook line: active voice + emotional driver
   - Hashtag strategy: 3 platform-evergreen + 2 streamer-niche, platform-aware
   - Anti-generic banlist
   - Money-quote priority if transcript has strong line
6. **Money-quote extraction.**
   - BYOK path: new prompt — "pick the single best 2–6 word phrase worth prominently displaying"
   - Free path: heuristic — longest transcript phrase with emotional keyword AND high RMS during playback AND <6 words
7. **Free path template matrix.** Replace flat template list with **(emotion × context) grid**:
   - Emotion tags (rows): shock, hype, funny, rage, panic
   - Context tags (columns): ace, death, clutch, fail, reaction, etc.
   - Each cell: 3–5 templates
   - Game-specific vocab slot-filled from Phase 1 configs
   - 50–80 templates initial, stored as TOML for easy additions
8. **Community-clip title passthrough (Free path).** When Twitch community clip covers the moment, use its title verbatim (with profanity/slur filter). Fans already wrote the wit. Huge quality lift for Free users, zero LLM cost.
9. **Auto-tone selection.** Pick tone from emotion tags — user can override.
10. **Subtitle rendering improvements:**
    - Keyword emphasis — scale up + color the 1–2 impact words per line
    - Money-quote styling — bigger font, gradient fill
    - Emoji injection — contextual at peak RMS moment
    - Word-level timing — TikTok-style word-highlight animation (Whisper already gives word timings)
11. **Multi-candidate ranker** — both paths emit 5 candidates, same ranker picks winner.

**Where Free path still loses to BYOK (be honest in marketing):**
- Natural language fluency on complex/novel moments
- Streamer voice calibration (needs LLM)
- Cross-signal reasoning ("chat laughing but he looked confused")

**Success criteria:**
- A/B test: new prompt vs old prompt on 10 recent clips, new wins in ≥7
- Free path output measurably sharper (less generic) than pre-Phase 12 baseline
- Both paths respect the toggle framework

---

### Phase 2 — Game Audio Fingerprinting (Week 4–5)

**Effort:** ~5–7 days engine + 1–2 days per game

Catches silent game events (aces, headshots, round wins, level-ups) via deterministic audio cue matching.

**Deliverables:**

1. **Audio fingerprinting engine.** New module `src-tauri/src/detection/audio_fingerprint.rs`:
   - Constellation-style algorithm (frequency peaks over time)
   - Evaluate Rust crates: `rustfft` for FFT, possibly `chromaprint` bindings
   - API: `match_fingerprints(vod_audio: &[f32], references: &[Fingerprint]) -> Vec<FingerprintMatch>`
   - `FingerprintMatch { timestamp: f64, reference_id: String, confidence: f32 }`
2. **Reference sound library.**
   - `assets/audio_fingerprints/{game_id}/{event_name}.wav`
   - `assets/audio_fingerprints/{game_id}/manifest.toml`
   - Initial coverage: Valorant (ace sting, round win, headshot ding), CS2 (round win, bomb plant/defuse), Apex (kill, down), Fortnite (elimination), plus Sal's personal games (DBD, Elden Ring where applicable)
3. **Signal generator.** `generate_audio_fingerprint_candidates()` in clip_selector:
   - Emits candidates tagged `["game-event", "ace", "valorant"]`
   - Intensity from match confidence
4. **Integration.** Add fingerprint stage in [run_analysis_signals() vod.rs:1381](../src-tauri/src/commands/vod.rs) between transcription and chat.

**Parallel track:** Reference library collection (non-coding work) starts in parallel with Phase 1 so it's ready when engine lands.

**Success criteria:**
- Silent ace in test Valorant VOD detected + scored above rejection threshold
- FP rate on non-event audio below 5% on 1-hour test VOD
- New game = drop sounds into `assets/audio_fingerprints/{game_id}/`, no code changes

---

### Phase 3 — Audio Envelope Patterns + Scene + Color (Week 5)

**Effort:** ~4–5 days (ship envelope first, measure scene+color before full commit)

**Deliverables:**

1. **Audio envelope pattern detection.** Extend [analyze_audio_intensity() vod.rs:239](../src-tauri/src/commands/vod.rs):
   - Silence-drop patterns (sustained action → sudden quiet; revive/death)
   - Rapid burst patterns (multiple transients close together; multi-kills)
   - Sudden-onset patterns (quiet → loud transient; jumpscare, explosion)
   - Emit as `generate_envelope_candidates()` signal
2. **Scene-cut detection.** Complete (or reimplement in Phase 5 cleanup) scene_signal.rs:
   - `ffmpeg -vf "select=gt(scene,THRESHOLD)"` pipe
   - Configurable threshold in `CurationConfig`
3. **Color histogram analysis.** New module `detection/color_signal.rs`:
   - Sample 1 frame per second via ffmpeg
   - Red-dominant frames (damage flash, "YOU DIED" screens)
   - White-flash frames (explosions, flashbangs)
   - Blue-tint frames (DBD knockout, some death screens)
   - Emit as `generate_color_candidates()` signal

**Risk:** Color histograms can false-positive on stream overlays (donation alerts, subs).

**Success criteria:**
- Silent death screens in Elden Ring test VOD trigger color signal
- Multi-kill bursts without streamer reaction trigger envelope pattern
- Scene cuts at round transitions in Valorant trigger scene-cut signal

---

### Phase 6.5 — Hook Optimization (Week 5, ~1 day)

Small extension to the existing boundary optimizer in [clip_selector.rs:446](../src-tauri/src/clip_selector.rs). Currently snaps start to first above-average-RMS second. Enhance: prefer starts where audio-onset AND facecam-reaction-peak (from Phase 4.5) coincide within ±2s. Lands the "hook frame" in first 1–2s of the clip.

**Success criteria:** measurable TikTok watch-time win in A/B comparison.

---

### Phase 4.5 — Facecam Reaction Detection (Week 6)

**Effort:** ~4–5 days

Eklipse's marquee feature. Detects streamer's expression changes as highlight signal — catches the beat before the "WHAT?!" exclamation.

**Deliverables:**

1. **Face-landmark detection.** Rust crate evaluation: `dlib-face-recognition` (C++ bindings, ~50MB binary cost) vs tiny onnx model + `ort` crate (~10MB). Evaluate on binary-size budget.
2. **Reaction signal extraction.** Detect sharp expression changes:
   - Smile-widening
   - Shock/jaw-drop
   - Laugh (mouth + cheek motion)
   - Facepalm / head-shake
3. **Signal generator.** `generate_facecam_candidates()` — emits `["facecam-reaction", "{expression}"]` tags.

**Success criteria:**
- Silent jaw-drop moments detected in test VODs
- Integrates with Phase 6.5 hook optimization

---

### Phase 8 — Chat Overlay Burn-In (Week 6–7)

**Effort:** ~3–4 days

Export feature, not detection. StreamLadder's signature TikTok feature.

**Deliverables:**
- During ffmpeg render, overlay a semi-transparent chat message stream on the clip's edge
- Shows real chat messages from the moment
- Graceful skip when chat replay unavailable (sub-only, deleted messages, creator-disabled)

**Success criteria:** chat-overlaid clips look polished, match StreamLadder aesthetic.

---

### Phase 10 — Preset Style Templates (Week 7)

**Effort:** ~3 days

Pre-configured export profiles:
- **Gaming Hype** — fast captions, big font, red accent
- **Funny Moment** — purple accent, emote reaction overlay
- **Rage** — red shake, bold caption
- **Chill** — minimal captions, muted accent

One-click apply at export. Reduces decision fatigue for new users.

---

### Phase 9 — Auto-Compilation (Week 8)

**Effort:** ~2 days

End of analysis: if ≥5 viral-tier clips found, offer "Build today's montage (90s)" button. Auto-generates in existing Montage Builder with default transitions. One click → ready-to-upload highlight reel. Low effort since Montage Builder already exists.

---

### Phase 7 — Launch Prep (Whenever feature set is ready)

**Deliverables:**
1. System requirements documentation (min + recommended specs, NVIDIA GPU recommended for Whisper CUDA, BYOK vision optional)
2. Steam store copy audit — every feature claim maps to live tested code
3. Bump-version workflow reminder — `bump-version.ps1` before every release commit
4. Smoke test matrix — end-to-end tests on VODs from each supported game
5. Changelog + release notes

**Critical:** any release build requires `TAURI_SIGNING_PRIVATE_KEY` set + signing key at `.tauri\clipgoblin-v2.key`.

---

### Phase 11 — Analytics Feedback Loop (Post-Launch)

**Effort:** ~5–7 days

After upload, poll YouTube/TikTok analytics. Build correlation table: which of our 6 scoring dimensions actually predicts real-world performance for this streamer?

**New table:** `detection_feedback`:
- Tracks community clips ClipGoblin missed or deprioritized
- Tracks clips ClipGoblin flagged but no community clip exists
- Surfaces agreement rate as UI stat (only when N ≥ 20 VODs — else show `learning…`)

This is the "gets smarter the more you use it" differentiator. Eklipse/StreamLadder don't close this loop per-creator.

---

## Deferred

### Phase 4 — HUD Heartbeat Detection (re-evaluate after Phase 6)

**Why deferred:** Phase 6 (BYOK vision) covers similar ground (detecting game events from pixels) with **no per-game maintenance**. HUD heartbeat requires ongoing calibration that rots with game patches and has an expected >10% FP rate.

**Revisit trigger:** after Phase 6 ships, measure: does vision reliably catch silent aces and killfeed events? If yes → cut Phase 4. If gaps remain → revisit with ~5 days of work + per-game calibration.

---

## Competitive positioning

| Feature | ClipGoblin | StreamLadder | Eklipse |
|---|---|---|---|
| 100% local | ✅ | Cloud | Cloud |
| No recurring fee | ✅ (one-time) | $9–24/mo | $10–24/mo |
| BYOK AI | ✅ | No | No |
| Desktop app | ✅ | Web | Web |
| No upload caps | ✅ | Plan-limited | Plan-limited |

**Marketing framing:** they rent compute, ClipGoblin uses yours. Own the local advantage.

---

## Future considerations (post-launch)

- **Thumbnail intelligence** — auto-pick best keyframe + style with title overlay
- **Cross-VOD clustering** — "you've clipped this type of moment 12 times; auto-montage?"
- **Royalty-free music auto-selection** — licensing complexity is the blocker
- **Per-toggle AI model selection** — current plan is one model applies to all toggles; power users might want Haiku for detection + Opus for titles

---

## Non-goals (do not build)

- Full killfeed OCR (Tesseract/PaddleOCR) — replaced by HUD heartbeat + BYOK vision
- Local multimodal vision models (LLaVA, Qwen-VL) — hardware requirements conflict with audience
- Per-game trained YOLO models — maintenance burden not justified
- Live-stream mode — VOD-only for v1
- Background/auto-scheduled VOD scanning — manual "Analyze" button remains the trigger
- Browser extension for live Twitch — conflicts with VOD-only non-goal

---

## Revised execution order

```
Day 1:        Phase 5 cleanup (delete/complete stubs)
Day 2-3:      Phase 6.0 Toggle framework (prerequisite for 6 + 12)
Week 1-2:     Phase 1 chat + game config + emote density
Week 2-3:     Phase 6 BYOK vision (plugs into toggle framework)
Week 3-4:     Phase 12 titles/captions (both paths, with ranker)
              [requires Sal's review of prompt diff before starting]
Week 4-5:     Phase 2 audio fingerprinting (ref library collection in parallel)
Week 5:       Phase 3 envelope + 6.5 hook optimization
Week 6:       Phase 4.5 facecam reactions
Week 6-7:     Phase 8 chat overlay
Week 7:       Phase 10 preset templates
Week 8:       Phase 9 auto-compilation
Whenever:     Phase 7 launch prep
Post-launch:  Phase 11 analytics feedback loop
DEFER:        Phase 4 HUD heartbeat (evaluate post Phase 6)
```

Total: ~6–8 calendar weeks focused pace, 10–12 at part-time.

---

## Dev environment reference

- **Project root:** `C:\Users\cereb\Desktop\Claude projects\clipviral`
- **GitHub:** https://github.com/nsvlordslug/ClipGoblin
- **Required env for signed builds:** `PROXY_API_KEY`, `TAURI_SIGNING_PRIVATE_KEY`
- **Signing key:** `.tauri\clipgoblin-v2.key` (no password)
- **Transcript cache:** `%APPDATA%/clipviral/transcripts/{vod_id}.json`
- **Export output:** `%APPDATA%/clipviral/exports/{clip_id}.mp4`
- **Captions:** `%APPDATA%/clipviral/captions/{highlight_id}.srt`
- **New terminals open in `C:\Windows\System32`** — always `cd` to project root first.
- **Version bump before every release:** `powershell -file bump-version.ps1 <version>` syncs `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`.
- **Rust unavailable in Claude Code sandbox** — Sal runs `cargo check` / `cargo tauri dev` in his terminal.
