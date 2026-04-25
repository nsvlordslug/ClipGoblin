# Session Handoff — ClipGoblin

**Last session:** 2026-04-24 (Phase 12 caller migration + 6-pattern title framework rebuild + save-path Wave 3 + Phase 6.0 foundation)
**Written for:** next Claude Code session resuming work on ClipGoblin.

Read this first, then [docs/ROADMAP.md](../docs/ROADMAP.md), then [CLAUDE.md](../CLAUDE.md). **[docs/PHASE12_PROMPT_DIFF.md](../docs/PHASE12_PROMPT_DIFF.md) is historical reference now** — the 3-pattern framework it describes was scrapped this session. See the "Title framework rebuild" section below for what replaced it.

---

## tl;dr — where we are, what's next

**Shipped this session (commits `8349a84` → `d0d3fda`, 7 commits):**

1. **Caller migration** — `commands/captions.rs` rewired to call the new Wave 3 functions (`generate_llm_titles`, `generate_llm_caption`, `extract_money_quote_llm`, `caption_candidate_to_variant`). Old `generate_llm` / `generate_llm_title` still exist in `post_captions.rs` as dead pub functions (safe rollback path, no callers).

2. **Title framework scrapped and rebuilt.** The original 4-pattern framework (`StakeArrowOutcome` / `EmotionColonDetail` / `QuoteTwist` / `NaturalVoice`) was producing label-style titles ("boss fog → 5 seconds of dead air", "Humbled: stuck in animation") — shapes with arrow separators / colon-prefix emotion words appear in **0% of 180 real high-engagement gaming Shorts titles** I sampled (TenZ, OtzStreams, Jynxzi, Lilith Omen). Replaced with 6 research-backed patterns:
   - `QuietFlex` — 2-4 word understatement ("actually clean")
   - `AftermathConfession` — first-person past-tense self-deprecation ("gonna sleep on the couch again")
   - `Observational` — third-person narration ("Meg learnt to fly")
   - `TechCallout` — named mechanic ("don't blind Legion when vaulting")
   - `SpecificSuperlative` — "worst/fastest/most X" pinned to named thing ("the WORST hex in DBD")
   - `CuriosityQuestion` — specific unanswered question ("why has nobody left the bus?")

3. **Ranker rebalanced** against real-world data:
   - Removed emotion-word bonus (old list rewarded clichés like "Humbled" / "Speechless" that real top titles avoid)
   - Reduced number/stake bonus 0.25 → 0.10 (many top titles have no numbers)
   - Added `anchor_score` (proper noun count, up to 0.20) — concrete anchors are the single biggest differentiator in real data
   - Added `has_template_artifact` hard-reject — arrow separators (`->`/`→`), em-dash separators, "POV:" prefix (only 2 of 180 real titles used POV), and legacy `{TitleCaseWord}: description` colon-prefix shape
   - Expanded banlist: `hits different`, `goes hard`, `no cap`, `lowkey`, `elden ring moment`, `gaming moment`, etc.
   - Title length target raised 42/50 → 60 (old cap was starving the model on reaction/story content)

4. **Save-path Wave 3 wiring** — `analyze_vod` now upgrades heuristic titles with `generate_llm_titles` when BYOK + `Scope::Titles` is on. Previous behavior: every new VOD analysis produced `"Highlight at 5:32"` / `"Stream Moment Playing Elden Ring"` style titles until the user clicked Regenerate per clip. Now they land Wave 3-shaped immediately. Sequential awaits (per-clip), ~$0.10 per analyze on Sonnet. Per-clip failures gracefully degrade to heuristic.

5. **Heuristic save-path titles rewritten** — `save_path_heuristic_title` in `commands/captions.rs` tries three layers: QuietFlex from transcript phrase → AftermathConfession templated from event tags + game name → legacy `grounded_highlight_title` fallback. Each tag combo has 5+ variants, picked by `start_seconds % len`, so multiple clips sharing a tag don't all collide on the same title. Free users get meaningfully better titles too.

6. **Session regen anti-repeat** — `REGEN_TITLE_HISTORY: HashMap<clip_id, Vec<String>>` in `commands/captions.rs` keeps the last 10 generated titles per clip for the app's lifetime. Fixes the "hit Regenerate 3 times, get the same title twice" bug. Frontend `Editor.tsx` also passes `currentTitle` so the anti-repeat sees what's actually on screen, not stale DB state.

7. **SubtitleEditor scroll bug** — `scrollIntoView` on active segment was bubbling up through the entire editor panel during playback. Replaced with manual `scrollTo` scoped to the list container.

8. **Phase 6.0 foundation (3 of 4 pieces, 4th intentionally skipped)** — `ai_usage_log` table + migration, `ai_usage` module with per-provider cost calc + rolling summary, `get_ai_cost_summary` tauri command, Settings UI now displays `~$X per VOD analyze` + `$X spent in last 30 days`. All 3 LLM call sites (titles, captions, money-quote) now write usage rows. Pre-run confirmation modal (#4) skipped — Settings readout provides enough cost visibility without adding friction to every analyze.

9. **Scrapped things:** caption_candidate legacy single-text shape adapter (`caption_candidate_to_variant`) still exists as migration aid. `build_hashtags_pub` deleted — callers use `build_hashtags_v2(platform, niche, game)` directly. Old 4-pattern `TitlePattern` variants removed from the enum (breaking change, but the new 6 variants supersede them cleanly).

**Tests:** 388/388 green (up from 329 — +6 `ai_usage` tests, ranker tests rewritten for new hard-rejects/anchor scoring, etc.).

---

## Cost reality (revised from earlier sessions)

**Actual per-call cost on Claude Sonnet 4.6 (measured from real usage):** ~$0.005 per LLM call (~500 input + ~200 output tokens = $0.0015 + $0.003).

**Per VOD analyze with save-path Wave 3 on:** ~$0.10 for 7-10 clips (1 API call per clip).

**$5 BYOK key lifespan (revised):**
| Usage | Per month | $5 lasts |
|---|---|---|
| Heavy — daily VOD + 5 regens/day | ~$3.75 | ~40 days |
| Moderate — 2-3 VODs/week + regens | ~$1.50 | ~3.3 months |
| Light — weekly VOD | ~$0.60 | ~7 months |

**Earlier sessions' "a year on $5 with heavy usage" claim was wrong** — off by 3-10x. Toggling off `use_for_titles` in Settings saves ~$0.08 per analyze; Regenerate still uses LLM per-clip. That's the lever for cost control.

---

## Title framework — current state (replaces old 3-pattern design)

**The old `docs/PHASE12_PROMPT_DIFF.md` is historical only.** What actually ships:

### TitlePattern enum (post_captions.rs)

```rust
pub enum TitlePattern {
    QuietFlex,              // 2-4 word understatement, DEFAULT for clutch/skill
    AftermathConfession,    // first-person past-tense, DEFAULT for reaction/fail
    Observational,          // third-person narration
    TechCallout,            // named mechanic
    SpecificSuperlative,    // "worst/fastest/most X"
    CuriosityQuestion,      // unanswered question
}
```

### Ranker rules

- **Hard rejects (score = 0.0):** banlist hit, arrow separator, em-dash separator, "POV:" prefix, `{TitleCaseWord}: description` colon-prefix shape
- **Scored dimensions (max sum 0.80):**
  - Number/stake present: +0.10 (reduced from 0.25 — many top titles have no numbers)
  - Length appropriate: +0.20 (target 60 chars)
  - Concrete anchor (proper nouns): +0.10 / +0.20 for 1 / 2+ anchors
  - Specificity (no generic nouns): +0.10 / +0.20 depending on hits
  - History overlap (opening word + Jaccard): +0.10 if no overlap
- **Per-pattern tweaks applied in caller (generate_llm_titles):**
  - QuietFlex: +0.15 bonus if ≤25 chars, -0.10 penalty if >35

### Banned emotion words (prompt-level, not ranker)

Speechless, Stunned, Rattled, Shook, Floored, Shocked, Paralyzed, Frozen, Numb, Silenced, Gutted, Wrecked, Broken, Humbled, Mortified, Crushed, Devastated, Defeated.

### Banned phrases (ranker-level, substring-match)

`insane` / `crazy` / `epic` / `literally` / `wild` / `shocking` / `unbelievable` / `omg` / `you won't believe` / `mind-blowing` / `must see` / `check this` / `watch this` / `you need to see` / `hits different` / `goes hard` / `goes crazy` / `no cap` / `lowkey` / `elden ring moment` / `valorant moment` / `dbd moment` / `apex moment` / `warzone moment` / `gaming moment` / `clip moment` / `stream moment` / `classic clip` / `classic moment` / `classic gaming`

---

## Release state (unchanged since last session)

- **Current published version:** v1.2.3
- **Published release:** https://github.com/nsvlordslug/ClipGoblin/releases/tag/v1.2.3
- **Landing:** https://clipgoblin.mindrotstudios.com
- **Auto-updater:** polls `latest.json` on startup
- Nothing shipped to users this session — the local branch has a lot of improvements but user preference was "no release yet."

**Infrastructure unchanged** — domain/DNS, Cloudflare Worker, TikTok production, YouTube Testing mode with `lordslug@gmail.com` + `thegameingbros1143@gmail.com` test users. See prior handoff in git history if details needed.

---

## Architecture status

**Live detection pipeline** (unchanged this session):
- Entry: [`analyze_vod()` commands/vod.rs:919](../src-tauri/src/commands/vod.rs)
- Signal sources: audio RMS spikes, Whisper transcript keyword→emotion, chat replay message-rate peaks (fallback only), Twitch community clips (Helix, 48h window)
- 7-stage selector: fusion → scoring → boundary opt → rejection → dedup → min-gap → diversity

**Title/caption generation (heavily reworked this session):**
- BYOK regenerate flow: `commands/captions.rs::generate_ai_title` / `generate_post_captions` → `post_captions::generate_llm_titles` / `generate_llm_caption` with money-quote prelude
- BYOK save path: `commands/vod.rs` analyze loop writes heuristic titles (`save_path_heuristic_title`), then `commands/captions.rs::upgrade_titles_with_llm` replaces them with LLM titles when `Scope::Titles` resolves to LLM
- Free path save: `save_path_heuristic_title` directly (layered: transcript quote → tag templates → grounded fallback)
- Free path captions: `generate_from_parts` + matrix-first `synthesize_event` (Wave 3b, already wired)

**AI usage logging (Phase 6.0, new this session):**
- Every LLM call writes to `ai_usage_log` table via `ai_usage::log_usage`
- `ai_usage::estimate_cost` returns `CostSummary { avg_per_analyze_usd, total_30d_usd, vod_count }`
- `Settings.tsx` fetches + displays on mount

**Phase 5 cleanup still DEFERRED** — the "dead scaffolding" modules (`pipeline.rs`, `engine.rs`, `audio_signal.rs`, `scene_signal.rs`, `transcript_signal.rs`, `clip_fusion.rs`, `clip_ranker.rs`, `clip_labeler.rs`) are NOT actually dead; 100+ integration tests exercise them. Do not touch without a careful per-module plan.

---

## Key files (updated this session)

| Path | What changed |
|---|---|
| `src-tauri/src/post_captions.rs` | TitlePattern enum (6 variants), generate_llm_titles prompt (100+ lines rewritten), generate_llm_caption `avoid_caption` param, `TokenUsage` struct, usage-sink plumbing on all 3 LLM fns |
| `src-tauri/src/commands/captions.rs` | `save_path_heuristic_title`, `aftermath_from_tags`, `upgrade_titles_with_llm`, `REGEN_TITLE_HISTORY` session cache, `get_ai_cost_summary` wired via usage logger at all 4 call points |
| `src-tauri/src/commands/vod.rs` | analyze_vod loop uses `save_path_heuristic_title`; post-signal async pass calls `upgrade_titles_with_llm` |
| `src-tauri/src/detection/ranker.rs` | Rewritten `score_title` — removed emotion bonus, reduced number bonus, added `anchor_score`, added `has_template_artifact` with arrow / em-dash / POV / colon-prefix rejections, expanded `DEFAULT_BANNED_WORDS` |
| `src-tauri/src/detection/mod.rs` | `title_length_target` raised 42/50 → 60 for all platforms |
| `src-tauri/src/ai_usage.rs` | **NEW** — cost computation, token pricing table, `log_usage` + `estimate_cost` + `CostSummary` |
| `src-tauri/src/ai_provider.rs` | Added `Provider::as_str()` for foreign-key use in `ai_usage_log` |
| `src-tauri/src/db.rs` | `ai_usage_log` table + indexes migration |
| `src-tauri/src/commands/settings.rs` | `get_ai_cost_summary` tauri command |
| `src-tauri/src/lib.rs` | `mod ai_usage;` + `get_ai_cost_summary` in invoke_handler |
| `src/pages/Settings.tsx` | Cost summary display below BYOK toggles |
| `src/pages/Editor.tsx` | Passes `currentTitle` to `generate_ai_title` for anti-repeat |
| `src/components/SubtitleEditor.tsx` | Scroll-only-list fix |

---

## Critical rules (from CLAUDE.md — do not violate)

1. **Small surgical edits only.** Editor.tsx ~1900 lines, Clips.tsx ~900, Settings.tsx ~900 — never rewrite whole files. Targeted `old_string → new_string`.
2. **`generate_llm_titles` / `generate_llm_caption` prompts are editable** (post-Phase 12 rebuild) but changes still benefit from Slug review before merging.
3. **`powershell -ExecutionPolicy Bypass -file bump-version.ps1 <version>`** before every release commit. Syncs `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`. Docs-only commits can skip.
4. **Rust not available in Claude Code sandbox** — always ask Slug to run `cargo check` / `cargo tauri dev` in his terminal. Static analysis only on Claude's side.
5. **Cargo.toml is in `src-tauri/`** — not project root.
6. **CRLF warnings on git add** — expected on Windows, harmless.
7. **New terminals open in `C:\Windows\System32`** — always `cd` to project root first.
8. **Commit format:** HEREDOC-style with `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`.

---

## Recent commit history

```
d0d3fda  phase 6.0: cost estimator command + Settings display
4cf1258  phase 6.0: wire usage logging into all 3 LLM call sites
88fd2dc  phase 6.0: ai_usage_log table + cost computation foundation
660cce6  chore(captions): adopt build_hashtags_v2 in BYOK caption flow
3ca1dd0  fix(analyze): vary heuristic save-path titles when multiple clips share a tag
4ab7e85  feat(analyze): wire save-path titles through Wave 3 framework
c35c229  phase 12: caller migration + research-driven title framework rebuild
8349a84  fix(editor): scope subtitle auto-scroll to its container
19dc62c  phase 12 wave 3c: free_title() with community-clip passthrough
e33dfd5  phase 12 wave 3b: emotion x context caption template matrix
71505f2  phase 12 wave 3a: generate_llm_caption() + hook/body split
54a291a  docs: update SESSION_HANDOFF after v1.2.3 OAuth hotfix shipped
bd45864  v1.2.3: fix OAuth for release builds
```

---

## How to resume

1. **Read this file.** Skip `docs/PHASE12_PROMPT_DIFF.md` for the framework details — the 3-pattern design it documents no longer matches the code. Come here for the 6-pattern current state.
2. **Skim [docs/ROADMAP.md](../docs/ROADMAP.md)** — still the source of truth for what's beyond this session.
3. **Skim [CLAUDE.md](../CLAUDE.md)** — unchanging rules.
4. **Confirm repo state:** `git log --oneline -8` — top should be `d0d3fda phase 6.0: cost estimator command + Settings display`.
5. **Confirm tests green:** `cd src-tauri && cargo test --lib` — expect 388 pass, 1 ignored (`bin_manager::tests::download_real`).
6. **Running the app:** the published version (`AppData/Local/ClipGoblin/clipviral.exe`) is still v1.2.3 pre-caller-migration. The debug binary at `src-tauri/target/debug/clipviral.exe` is what has the session's work. Launch with `cargo build --manifest-path src-tauri/Cargo.toml` → start the exe. Vite dev server must be running (`npm run dev`) because the debug binary points at `http://localhost:5173`.

---

## Recommended next step: Phase 1 (chat + per-game configs + emote density)

Per ROADMAP, Phase 1 is the single largest detection-quality jump. Scope:
- Per-game TOML configs at `config/games/{game_id}.toml` with emotion keyword maps specific to each game (Valorant's "ace/clutch/1v5" vs DBD's "gen pop/hooked/iri" vs Elden Ring's "phase 2/fog gate/whiff"). Falls back to generic when game unknown.
- game_id auto-loaded from Twitch VOD metadata.
- Chat content analysis promoted from fallback to primary signal source (tokenize per 10s window, count event-keyword density).
- Emote density — count top 20 Twitch emotes per 10s window (KEKW/OMEGALUL/Pog/5Head/Sadge/MonkaS etc.) as a distinct signal. Huge quality lift, near-free to add.

**Why Phase 1 over alternatives:**
- Mostly non-LLM work → doesn't increase running cost
- Detection is the one quality axis we haven't touched this session
- Unblocks Phase 6 (vision) indirectly — better detection = fewer ambiguous clips that need vision

Estimated ~3-5 focused sessions. Probably not a "tack on at end of another session" size.

## Alternative next steps

- **Free-path money-quote plumbing** (~1-2h) — `extract_money_quote_free` already exists; needs RMS samples from `analyze_audio_intensity` threaded into `commands/captions.rs`. Low leverage since Slug uses BYOK.
- **Phase 6 BYOK vision** — frame extraction + Claude vision on ambiguous clips. Medium-large build + meaningful runtime cost (vision is 5-10x text tokens). Defer until Phase 1 ships.
- **Phase 6.0 pre-run modal** (~1-2h) — originally part of Phase 6.0 but skipped intentionally. Settings cost readout provides enough visibility; modal adds click-friction to every analyze without adding decision value.
- **Cut v1.2.4 release** — bump-version, tag, push. Auto-updater serves the new build to existing testers. User explicitly said "no release yet" this session; revisit when they're ready.

---

## Known follow-ups (non-blocking)

- Old `generate_llm` / `generate_llm_title` functions in `post_captions.rs` are unused but still pub. Safe to delete once Wave 3 has baked in for a few real-use sessions. Currently useful as rollback reference.
- `latest.json` updater manifest `notes` field still reads "See the changelog for details." (hardcoded in release.yml). Low priority.
- TikTok client secret was piped to Cloudflare Worker via `npx wrangler secret put` in an earlier session — rotate when convenient.
- v1.1.0 and v1.0.3 drafts in GitHub Releases are stale — safe to delete.
- Session regen cache (`REGEN_TITLE_HISTORY`) only lives in memory. Resets on app restart. If Slug wants persistence across restarts, add a table; otherwise the in-memory 10-entry ring is enough.
- `grounded_highlight_title` is kept as the deepest fallback in `save_path_heuristic_title`. When Phase 1 ships per-game configs, the heuristic can probably retire that fallback entirely.
