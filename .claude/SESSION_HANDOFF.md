# Session Handoff — ClipGoblin

**Last session:** 2026-04-20 (Wave 2 complete)
**Written for:** next Claude Code session resuming work on ClipGoblin.

Read this first, then [docs/PHASE12_PROMPT_DIFF.md](../docs/PHASE12_PROMPT_DIFF.md), then [docs/ROADMAP.md](../docs/ROADMAP.md), then [CLAUDE.md](../CLAUDE.md) for unchanging project rules.

---

## tl;dr — where we are, what's next

**Shipped in v1.2.2 session (prior work):**
- v1.2.2 released and published to GitHub
- Custom domain `clipgoblin.mindrotstudios.com` live on Cloudflare + GitHub Pages
- Landing page refreshed to match v4 app UI, `/download.html` redirect shim, dashboard screenshot
- [docs/ROADMAP.md](../docs/ROADMAP.md) — full 7-week expansion plan approved

**Shipped this session — Phase 12 Waves 1+2 (commits `ae0dc1a`, `d3f92e6`, `94a5c25`, `b157c6a`):**

Wave 1 (infrastructure):
- New `detection` module with `Platform` enum + `ranker` submodule (`score_title`, `pick_best`, banlist / emotional-word / generic-word constants)
- `build_hashtags_v2(tags, tone, platform, streamer_niche_tags, game_name)` — old `build_hashtags()` preserved as thin wrapper
- Pre-existing `captions_are_short` test flake fixed

Wave 2 (title + money-quote pipeline, parallel to existing APIs):
- `TitlePattern` enum (StakeArrowOutcome / EmotionColonDetail / QuoteTwist) + `TitleCandidate` struct
- `generate_llm_titles()` — JSON 5-candidate structured prompt, ranker-scored + sorted
- `extract_money_quote_llm()` (BYOK) — confidence ≥ 0.6, 2–6 word validation, `Result<Option<String>>`
- `extract_money_quote_free()` — pure heuristic reusing `ranker::DEFAULT_EMOTIONAL_WORDS`
- `extract_json_from_markdown()` — 3-layer robustness for LLM JSON output

Existing `generate_llm_title()` + `generate_llm()` are byte-identical. `commands/captions.rs` was not touched. Caller migration is a separate tiny follow-up.

**Tests:** 329/329 green (28 new `w2_*` tests all passing).

**Immediate next step — pick one:**

1. **Caller migration (tiny)** — rewire `commands/captions.rs:446` to call `generate_llm_titles()`, take `.text` of the top candidate, and surface the money-quote pipeline. Requires Slug review of how money-quote wires into the existing analyze_vod flow (transcript + RMS samples plumbing).

2. **Wave 3 — caption rewrite + Free-path matrix:**
   - `generate_llm_caption()` — hook_line + body split, 3 candidates, money-quote priority, ranker-scored
   - `config/caption_templates.toml` + loader — emotion × context matrix replacing the hardcoded `synthesize_event()` compound/single lookup
   - Community-clip title passthrough (Free path)

3. **Cleanup deferred from Wave 1** — audit Phase 5 dead scaffolding (`pipeline.rs`, `engine.rs`, `audio_signal.rs`, `scene_signal.rs`, etc. — see ROADMAP Phase 5). Not blocking, but recommended before Wave 3 to prevent parallel abstractions.

Slug's call. Recommended order: 3 → 1 → 2 (cleanup first, then wire callers to new API, then tackle Wave 3).

**Do NOT** edit `generate_llm()` prompt body until Slug reviews the Wave 3 diff (section 11 not yet expanded for captions).

---

## Release state (verified working on 2026-04-20)

- **Current version:** v1.2.2
- **Published release:** https://github.com/nsvlordslug/ClipGoblin/releases/tag/v1.2.2
- **Landing page:** https://clipgoblin.mindrotstudios.com (live, HTTPS)
- **Download shim:** https://clipgoblin.mindrotstudios.com/download.html
- **Auto-updater:** polls `latest.json` on startup, will advertise v1.2.2 to existing 1.0.x / 1.1.x installs

**Infrastructure live:**
- Domain registered at Namecheap, DNS on Cloudflare (nameservers `katelyn.ns.cloudflare.com` / `sage.ns.cloudflare.com`)
- `clipgoblin.mindrotstudios.com` CNAME → `nsvlordslug.github.io` (DNS only, grey cloud — NOT proxied, or TLS will break)
- Cloudflare Worker `clipgoblin-auth-proxy` holding TikTok (production) + Twitch + YouTube OAuth creds
- TikTok is production, not sandbox, as of v1.2.1+

---

## Architecture status (verified via deep exploration this session)

**Live detection pipeline** (see [docs/ROADMAP.md#current-pipeline-as-of-v122](../docs/ROADMAP.md)):

- Entry: [`analyze_vod()` commands/vod.rs:919](../src-tauri/src/commands/vod.rs)
- Signal sources feeding [`clip_selector::select_clips()` clip_selector.rs:891](../src-tauri/src/clip_selector.rs):
  1. Audio RMS spikes (ffmpeg astats)
  2. Whisper transcript keyword→emotion mapping
  3. Chat replay message-rate peaks (fallback only currently — being promoted in Phase 1)
  4. Twitch community clips (Helix API, 48hr window)
- 7-stage selector: fusion → scoring (6 viral dims) → boundary opt → rejection → dedup → min-gap → diversity

**Dead scaffolding to delete in Phase 5** (compiles but not called):
- `src-tauri/src/pipeline.rs`
- `src-tauri/src/engine.rs`
- `src-tauri/src/audio_signal.rs`
- `src-tauri/src/scene_signal.rs`
- `src-tauri/src/transcript_signal.rs`
- `src-tauri/src/clip_fusion.rs`
- `src-tauri/src/clip_ranker.rs`
- `src-tauri/src/clip_labeler.rs`

Delete all of these Day 1 before adding new code, or we get parallel abstractions.

---

## Key files and paths

### Source code
| Path | Purpose |
|---|---|
| `src-tauri/src/commands/vod.rs` | VOD download + analysis entrypoint (2079 lines) |
| `src-tauri/src/clip_selector.rs` | 7-stage clip selection pipeline (1021 lines) |
| `src-tauri/src/twitch.rs` | Twitch OAuth + Helix clips fetch |
| `src-tauri/src/post_captions.rs` | **AI title/caption generation** (FINALIZED per CLAUDE.md but reopened in Phase 12) |
| `src-tauri/src/ai_provider.rs` | BYOK provider resolution (Claude/OpenAI/Gemini/Free) |
| `src-tauri/src/detection/mod.rs` | **NEW (Wave 1)** — `Platform` enum + evergreen hashtag lists + title-length targets |
| `src-tauri/src/detection/ranker.rs` | **NEW (Wave 1)** — `score_title()` / `pick_best()` / banlists for title+caption candidates |
| `src-tauri/src/commands/settings.rs` | Settings whitelist + get/save |
| `src-tauri/src/whisper.rs` | Whisper-rs integration (CUDA/CPU toggle) |
| `src-tauri/src/db.rs` | SQLite schema + migrations |

### Frontend
| Path | Purpose |
|---|---|
| `src/pages/Settings.tsx` | Settings UI — includes community clips toggle, sensitivity slider, AI provider picker |
| `src/pages/Vods.tsx` | VOD list + analyze button |
| `src/pages/Clips.tsx` | Clip library (919 lines) |
| `src/pages/Editor.tsx` | Full clip editor (1927 lines — never rewrite whole file) |
| `src/components/ImportVodDialog.tsx` | VOD-by-URL import (broke v1.2.0 build — now fixed) |

### Public surface
| Path | Purpose |
|---|---|
| `docs/index.html` | Landing page (refreshed this session) |
| `docs/download.html` | Redirect shim → latest .exe |
| `docs/app-dashboard.png` | Dashboard screenshot on landing |
| `docs/CNAME` | `clipgoblin.mindrotstudios.com` (GitHub Pages custom domain) |
| `docs/ROADMAP.md` | **Approved plan — read after this file** |
| `worker/wrangler.toml` | Cloudflare Worker config (TikTok prod client key) |

### Data paths (runtime)
- DB: `%APPDATA%/clipviral/clipviral.db`
- Transcripts: `%APPDATA%/clipviral/transcripts/{vod_id}.json`
- Exports: `%APPDATA%/clipviral/exports/{clip_id}.mp4`
- Captions: `%APPDATA%/clipviral/captions/{highlight_id}.srt`

---

## Critical rules (from CLAUDE.md — do not violate)

1. **Small surgical edits only.** Editor.tsx 1927 lines, Clips.tsx 919, Settings.tsx 882 — never rewrite whole files. Targeted `old_string → new_string`.
2. **`post_captions.rs` prompts** were finalized but **reopened for Phase 12** in this session. Still: produce a diff for Slug to review BEFORE editing.
3. **`powershell -ExecutionPolicy Bypass -file bump-version.ps1 <version>`** before every release commit. Syncs `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`. Docs-only commits can skip version bump.
4. **Rust not available in Claude Code sandbox** — always ask Slug to run `cargo check` / `cargo tauri dev` in his terminal. Static analysis only on Claude's side.
5. **Cargo.toml is in `src-tauri/`** — not project root.
6. **CRLF warnings on git add** — expected on Windows, harmless, git autoconverts.
7. **New terminals open in `C:\Windows\System32`** — always `cd` to project root first.
8. **Commit format:** HEREDOC-style message with `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`.

---

## Decisions made this session

1. **Reopened the "finalized" post_captions.rs prompts** for Phase 12 quality rewrite. Requires Slug review of concrete diff before merge.
2. **Phase 5 (cleanup) moved from parallel → Day 1 blocker.** Delete dead scaffolding before adding new code.
3. **Phase 6 BYOK vision moved BEFORE Phase 4 HUD heartbeat.** If vision is good enough, Phase 4 may be unnecessary.
4. **Phase 4 HUD heartbeat deferred** until post-Phase-6 evaluation.
5. **New Phase 6.0 (toggle framework)** added as prerequisite for Phase 6 + Phase 12. Three independent toggles (detection/titles/captions), single model applies to all, usage-logged cost estimator.
6. **New Phase 4.5 (facecam reaction detection)** added — Eklipse's marquee feature.
7. **New Phase 6.5 (hook optimization)** — small tweak to boundary optimizer.
8. **New Phase 8 (chat overlay burn-in)** — StreamLadder's marquee feature.
9. **New Phase 10 (preset style templates)** — Gaming Hype / Funny / Rage / Chill.
10. **New Phase 9 (auto-compilation)** — leverages existing Montage Builder.
11. **New Phase 11 (analytics feedback loop)** — post-launch, learns per-streamer.
12. **Phase 1 enhanced with emote density signal** — near-free add with huge quality lift.
13. **Both BYOK and Free paths improved in Phase 12** — same ranker bridges both, Free path gets community-clip title passthrough (free wit from fans).
14. **Architectural principle: vision = new signal source**, not post-hoc score multiplier. Preserves existing pipeline integrity.

---

## Recent commit history (for orientation)

```
b157c6a  phase 12 wave 2: title candidates + money-quote extraction
94a5c25  docs: add Wave 2 concrete Rust diff for review
d3f92e6  docs: update SESSION_HANDOFF after Wave 1 ship
ae0dc1a  phase 12 wave 1: ranker module + platform-aware hashtags
c05271c  docs: add expandable beta disclaimer + NDA to landing page
efa2541  docs: add ROADMAP + SESSION_HANDOFF for detection pipeline expansion
4891225  docs: center the first-run callout on the landing page
6b32d2c  docs: refresh landing page to match v1.2.x app
a020a75  docs: serve landing page at clipgoblin.mindrotstudios.com with clean /download link
56b07ff  v1.2.2 — Fix TS build error blocking CI release
d293939  v1.2.1 — TikTok production connection hotfix  (tag exists but broken build; never released)
18995a1  feat(v1.2.0): v4 UI port, Twitch community clips, Auto-ship, Analytics, onboarding
```

---

## How to resume

1. **Read this file.**
2. **Read [docs/PHASE12_PROMPT_DIFF.md](../docs/PHASE12_PROMPT_DIFF.md)** — Phase 12 design decisions + Wave 1/2/3 plan.
3. **Read [docs/ROADMAP.md](../docs/ROADMAP.md)** — full approved plan.
4. **Skim [CLAUDE.md](../CLAUDE.md)** — unchanging rules.
5. **Confirm repo state:** `git log --oneline -5` — top should be `b157c6a phase 12 wave 2: title candidates + money-quote extraction`.
6. **Confirm tests still green:** `cd src-tauri && cargo test --lib` — expect 329 pass, 1 ignored (`bin_manager::tests::download_real`).
7. **Immediate next step:** pick from the three options in the tl;dr above (Wave 3 prompt rewrite / caller migration / Phase 5 cleanup). Recommended order: cleanup → caller migration → Wave 3.
8. **Check in with Slug** before editing `generate_llm()` body or the `LLM_SYSTEM_PROMPT`.

---

## Known follow-ups (not blocking)

- `latest.json` updater manifest `notes` field currently reads "See the changelog for details." (hardcoded in [.github/workflows/release.yml](../.github/workflows/release.yml)). Users see the GitHub release body in the in-app prompt, so low priority. If polishing: swap `releaseBody` for a templated extraction.
- TikTok client secret was piped to Cloudflare Worker via `npx wrangler secret put TIKTOK_CLIENT_SECRET` in this session. User should rotate it eventually since the value appeared in transcripts.
- v1.1.0 and v1.0.3 drafts in GitHub Releases are stale — can be deleted (Slug has been informed).
