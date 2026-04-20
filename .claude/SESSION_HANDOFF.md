# Session Handoff — ClipGoblin

**Last session:** 2026-04-20 (Wave 1 complete)
**Written for:** next Claude Code session resuming work on ClipGoblin.

Read this first, then [docs/ROADMAP.md](../docs/ROADMAP.md), then [docs/PHASE12_PROMPT_DIFF.md](../docs/PHASE12_PROMPT_DIFF.md), then [CLAUDE.md](../CLAUDE.md) for unchanging project rules.

---

## tl;dr — where we are, what's next

**Shipped in v1.2.2 session (prior work):**
- v1.2.2 released and published to GitHub
- Custom domain `clipgoblin.mindrotstudios.com` live on Cloudflare + GitHub Pages
- Landing page refreshed to match v4 app UI, `/download.html` redirect shim, dashboard screenshot
- [docs/ROADMAP.md](../docs/ROADMAP.md) — full 7-week expansion plan approved

**Shipped this session (Phase 12 Wave 1, commit `ae0dc1a`):**
- New `detection` module (`src-tauri/src/detection/`) with:
  - `Platform` enum — TikTok / YouTubeShorts / InstagramReels / Generic
  - `ranker` submodule — `score_title()`, `pick_best()`, banlist / emotional-word / generic-word constants
- `build_hashtags_v2(tags, tone, platform, streamer_niche_tags, game_name)` in `post_captions.rs` (3 evergreen + 2 niche strategy). Old `build_hashtags()` preserved as thin wrapper.
- Pre-existing `captions_are_short` test flake fixed (shortened `gen_internal_thought` quote-contrast templates).
- [docs/PHASE12_PROMPT_DIFF.md](../docs/PHASE12_PROMPT_DIFF.md) — resolved all 11 open design decisions + Wave 1/2/3 rollout plan.
- **Tests:** 70/70 green (41 post_captions + 29 detection).

**Immediate next step (Wave 2 — requires Slug review of Rust diff before landing):**
1. Rewrite `generate_llm_title()` → new `generate_llm_titles()` returning `Vec<TitleCandidate>` with 3 hard-structure patterns (stake→outcome / Emotion:detail / quote+twist), 5 candidates, 40-char limit, enforced banlist, JSON output. Old `generate_llm_title()` becomes a thin wrapper that picks the best candidate via `detection::ranker::pick_best` and returns `String`.
2. New `extract_money_quote_llm()` (BYOK, separate tiny API call) + `extract_money_quote_free()` (RMS × emotional-keyword heuristic).
3. Wire money-quote into `generate_llm_titles()` input.

**Do NOT** edit `generate_llm_title()` or the prompt body until Slug reviews the Wave 2 Rust diff. (CLAUDE.md rule #3 applies; Phase 12 reopened it but review-first still holds.)

Wave 3 (after Wave 2 ships): caption rewrite + Free-path emotion × context template matrix.

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
5. **Confirm repo state:** `git log --oneline -3` — top should be `ae0dc1a phase 12 wave 1: ranker module + platform-aware hashtags`.
6. **Confirm tests still green:** `cd src-tauri && cargo test --lib detection:: && cargo test --lib post_captions::` — expect 29 + 41 = 70 pass.
7. **Immediate next step:** produce the Wave 2 Rust diff for Slug to review:
   - `generate_llm_titles()` new function with JSON 5-candidate output + 3-pattern structure + banlist
   - `generate_llm_title()` becomes thin wrapper delegating to new fn + `detection::ranker::pick_best`
   - `extract_money_quote_llm()` (BYOK) + `extract_money_quote_free()` (heuristic)
   - `TitleCandidate` + `TitlePattern` structs in post_captions.rs
8. **Check in with Slug** before editing `generate_llm_title()` body or the `LLM_SYSTEM_PROMPT`.

---

## Known follow-ups (not blocking)

- `latest.json` updater manifest `notes` field currently reads "See the changelog for details." (hardcoded in [.github/workflows/release.yml](../.github/workflows/release.yml)). Users see the GitHub release body in the in-app prompt, so low priority. If polishing: swap `releaseBody` for a templated extraction.
- TikTok client secret was piped to Cloudflare Worker via `npx wrangler secret put TIKTOK_CLIENT_SECRET` in this session. User should rotate it eventually since the value appeared in transcripts.
- v1.1.0 and v1.0.3 drafts in GitHub Releases are stale — can be deleted (Slug has been informed).
