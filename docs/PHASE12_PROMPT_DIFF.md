# Phase 12 — Prompt Rewrite Review

**Status:** design approved 2026-04-20. Decisions resolved (see section 9). Implementation in 3 waves.
**Target file:** [`src-tauri/src/post_captions.rs`](../src-tauri/src/post_captions.rs) (1885 lines)
**Scope:** `generate_llm_title()`, `generate_llm()`, new money-quote helper, Free-path template matrix, shared ranker.
**Roadmap reference:** [Phase 12 in ROADMAP.md](ROADMAP.md#phase-12--title--caption-quality-loop-week-34)

## Rollout waves

| Wave | Scope | Touches |
|---|---|---|
| **1** (current) | Shared ranker module + expanded `build_hashtags()` with `Platform` + niche support | New `detection/ranker.rs`, new `detection/mod.rs`, `lib.rs`, `post_captions.rs` (hashtag fn only) |
| **2** | Title rewrite (`generate_llm_titles()`) + money-quote extraction (BYOK + Free) | `post_captions.rs` prompt rewrite, new money-quote functions |
| **3** | Caption rewrite (`generate_llm_caption()`) + Free-path matrix | `post_captions.rs` prompt rewrite, new `config/caption_templates.toml` |

Subtitle rendering (Phase 12 item 10) is a separate diff against `commands/export.rs`, post-Phase-12.

---

## 0. What's changing and why

Today's prompts produce captions/titles that are good but occasionally lean generic. Phase 12 tightens them by:
1. **Forcing structural constraints** on titles (three allowed patterns) instead of leaving shape to the model
2. **Emitting multiple candidates + ranking** instead of trusting the single output
3. **Splitting captions into hook + body** so the first ~50 chars is scroll-stopper material
4. **Adding a money-quote extraction step** so the most memorable phrase can drive titles/captions/overlay
5. **Replacing Free-path's mode × gametype templates with an emotion × context matrix** with game-specific vocab slots
6. **Sharing a ranker between BYOK and Free** — same scoring function picks winner from N candidates either way

Everything below is additive to the existing pipeline. Event-summary generation, GameType detection, and the 5-mode system are preserved. Phase 6.0 toggle framework remains a prerequisite — this doc assumes `ai_for_titles` / `ai_for_captions` toggles exist.

---

## 1. `generate_llm_title()` — current vs proposed

### 1a. Current state ([post_captions.rs:1286–1447](../src-tauri/src/post_captions.rs))

- Picks ONE random "angle" from 6 options (REACTION/GAMEPLAY/QUOTE/ABSURDITY/HOOK/SELF-DEPRECATING)
- Picks ONE random transcript word to ban per call
- Sends a single prompt with a 60-char limit
- Returns **one string**
- Client-side enforces 60 chars, truncates at last space if over

### 1b. Problems

- **Single-output model.** No ranker — whatever the model returns is what ships. If the model whiffs, reroll costs another API call.
- **No structural constraint.** Model is free to produce any shape. Results drift toward "emotion-word sentence fragment."
- **60-char limit is permissive.** TikTok Shorts mobile UI cuts at ~42 chars. 40-char cap per ROADMAP.
- **Angle rotation is diversity-only.** Doesn't tie the title structure to the clip's dominant mode/content.
- **Single banned word is a hack.** Real banlist (insane, crazy, epic, literally…) never gets enforced.

### 1c. Proposed signature

```rust
pub async fn generate_llm_titles(
    api_key: &str,
    model: &str,
    event_summary: &str,
    money_quote: Option<&str>,         // NEW — from extract_money_quote
    full_transcript: Option<&str>,
    tags: &[String],
    game_name: Option<&str>,
    streamer_history: Option<&[String]>, // NEW — stub; Phase 11 populates; pass None for now
) -> Result<Vec<TitleCandidate>, AppError>;

pub struct TitleCandidate {
    pub text: String,
    pub pattern: TitlePattern,          // which of the 3 patterns
    pub score: f32,                     // from shared ranker
}

pub enum TitlePattern {
    StakeArrowOutcome,   // "down 0-12 → 1v5 ACE"
    EmotionColonDetail,  // "SPEECHLESS: one-tap through smoke"
    QuoteTwist,          // "\"I can't miss\" — misses next shot"
}
```

Existing `generate_llm_title()` → thin wrapper that calls the new function and returns `best_candidate.text` for backward compat until callers migrate.

### 1d. Proposed prompt (literal text to send)

```text
Generate 5 SHORT title candidates for a gaming clip. Return JSON only.

GAME: {game_name or "general"}
WHAT HAPPENED: {event_summary}
MONEY QUOTE (if any): "{money_quote or omit line entirely}"
TRANSCRIPT (first 800 chars):
"{transcript}"
SIGNAL TAGS: {tags joined by ", "}

STRUCTURE — every candidate MUST follow exactly one of these three patterns:

1. STAKE_ARROW_OUTCOME — format: "{stake} → {outcome}"
   Examples: "down 0-12 → 1v5 ACE", "120hp → one-shot", "last alive → 4k"

2. EMOTION_COLON_DETAIL — format: "{EMOTION_WORD}: {specific visual or action}"
   Examples: "SPEECHLESS: one-tap through smoke", "RATTLED: smoke into headshot",
   "FLOORED: three kills, zero ammo left"

3. QUOTE_TWIST — format: "\"{money quote}\" {twist or contradiction}"
   Examples: "\"I can't miss\" — misses next shot", "\"easy game\" 0-5 next round",
   "\"trust me\" — should not have"

REQUIRED ELEMENTS — every candidate must include AT LEAST ONE:
- A number or stake (1v5, 0hp, 12s, 3 kills, round 12)
- An emotional word (SPEECHLESS, RATTLED, FLOORED — all-caps emotion fine here)
- A specific visual or action detail (through smoke, through the wall, with one bullet)

HARD LIMITS:
- Maximum 40 characters per title (TikTok Shorts mobile cuts at ~42)
- Lowercase is fine EXCEPT for emotion words in pattern 2 (those are ALL CAPS)
- No hashtags. No emojis. No period at end.

BANNED WORDS (reject any candidate containing these):
insane, crazy, epic, literally, wild, shocking, unbelievable, omg,
you won't believe, mind-blowing, must see, check this, watch this

STREAMER'S RECENT TITLES (avoid token overlap >50% with these):
{streamer_history joined by newline, or "none yet"}

TONE INHERITANCE:
- If the clip has a strong money quote, at least 2 of 5 candidates MUST use pattern 3
- If the event is visual (ace, kill, death) with no quote, favor patterns 1 and 2
- Candidates should span all 3 patterns (not all one shape)

OUTPUT — JSON only, no prose:
{
  "candidates": [
    {"pattern": "STAKE_ARROW_OUTCOME", "text": "..."},
    {"pattern": "EMOTION_COLON_DETAIL", "text": "..."},
    {"pattern": "QUOTE_TWIST", "text": "..."},
    {"pattern": "STAKE_ARROW_OUTCOME", "text": "..."},
    {"pattern": "EMOTION_COLON_DETAIL", "text": "..."}
  ]
}
```

### 1e. Open decisions for Slug

- **Keep the 6-angle rotation anywhere?** Proposal: drop it entirely. The 3-pattern structure subsumes it and is enforceable.
- **Streamer-history parameter stubbed as `None` until Phase 11 ships?** Proposal: yes. Keeps signature stable.
- **Emotion words in ALL CAPS for pattern 2** — is that okay? Current `LLM_SYSTEM_PROMPT` says "lowercase is fine". Might read as shouty. Alternative: title-case the emotion word.
- **Few-shot examples inline in the prompt, or moved to a constant?** Proposal: constant in Rust, rendered into prompt — easier to tune without editing the prompt template.

---

## 2. `generate_llm()` — current vs proposed

### 2a. Current state ([post_captions.rs:1114–1280](../src-tauri/src/post_captions.rs))

- Generates ONE caption for ONE selected mode per call
- Mode instructions via `tone_instruction()` (10 modes: 5 core + punchy/clean/funny/hype/search)
- GameType (FPS/Horror/Cozy/Social/Generic) injected as "STYLE GUIDE" block
- Prompt already has a decent anti-generic banlist
- 280-char hard limit, client-side truncation at sentence/space boundary
- Returns `Vec<CaptionVariant>` with one element

### 2b. Problems

- **Single-part output.** No scroll-stopper hook. The model sometimes buries the lede.
- **Hashtag strategy undefined.** System prompt says "never use hashtags" but Phase 12 says "3 evergreen + 2 niche, platform-aware". These conflict. Decision needed.
- **No money-quote priority.** If the clip has a killer line, there's no mechanism to ensure the caption uses it.
- **No ranking.** Same single-output problem as titles.
- **10 modes might be too many.** The 5 legacy modes (punchy/clean/funny/hype/search) predate the current 5-mode system and overlap with it. Trim?

### 2c. Proposed signature

```rust
pub async fn generate_llm_caption(
    api_key: &str,
    model: &str,
    selected_mode: &str,
    platform: Platform,                 // NEW — TikTok | YouTubeShorts | InstagramReels
    event_summary: &str,
    money_quote: Option<&str>,          // NEW
    transcript_quote: Option<&str>,
    tone_label: &str,
    tags: &[String],
    full_transcript: Option<&str>,
    clip_title: &str,
    game_name: Option<&str>,
    streamer_niche_tags: &[String],     // NEW — e.g. ["valorant", "radiant"]
) -> Result<Vec<CaptionCandidate>, AppError>;

pub struct CaptionCandidate {
    pub hook_line: String,              // first ~50 chars, scroll-stopper
    pub body: String,                   // remainder
    pub hashtags: Vec<String>,          // 5 total: 3 evergreen + 2 niche
    pub uses_money_quote: bool,
    pub score: f32,
}

pub enum Platform { TikTok, YouTubeShorts, InstagramReels }
```

`generate_llm()` → wrapper that returns `Vec<CaptionVariant>` (1 element, best scored) for backward compat.

### 2d. Proposed prompt skeleton

```text
Write 3 caption candidates for my gaming clip. Return JSON only.

PLATFORM: {TikTok | YouTube Shorts | Instagram Reels}
GAME: {game_name}
GENRE: {tone_label}
CLIP TITLE: {clip_title}
WHAT HAPPENED: {event_summary}
MONEY QUOTE (use if present): "{money_quote}"
TRANSCRIPT (800 chars): "{transcript}"
SIGNAL TAGS: {tags}
STREAMER NICHE: {niche tags or "none"}

MODE: {mode} — {tone_instruction text}

STRUCTURE — every candidate is two parts:
- hook_line: the first ~50 characters. Active voice. Emotional driver. Scroll-stopper.
  Must be complete on its own when truncated at 50 chars.
- body: the rest (under 230 chars). Specifics. Context. Reaction.

HASHTAG STRATEGY — return exactly 5:
- 3 evergreen platform hashtags (appropriate for {platform})
- 2 streamer-niche hashtags (drawn from: {streamer_niche_tags}, game name, game genre)
- No more than 5 total. No spaces inside hashtags.

MONEY-QUOTE PRIORITY:
- If a money quote is provided and fits the mode, 2 of 3 candidates SHOULD use it verbatim or close-paraphrase in the hook_line.
- Never fabricate quotes. Only use the provided quote or transcript fragments.

HARD LIMITS:
- hook_line: under 50 characters (hard — truncate if model oversteps)
- body: under 230 characters
- hook_line + body: under 280 characters combined (ignoring hashtags)

BANNED PHRASES (reject candidate):
this happened, caught on stream, this was crazy, just happened,
watch this, you need to see this, you won't believe, check this out,
insane clip, literally, epic

OUTPUT JSON only:
{
  "candidates": [
    {
      "hook_line": "...",
      "body": "...",
      "hashtags": ["...", "...", "...", "...", "..."],
      "uses_money_quote": true | false
    },
    {...},
    {...}
  ]
}
```

### 2e. Open decisions for Slug

- **Hashtag policy conflict.** `LLM_SYSTEM_PROMPT` (line 1073) literally says "You never use hashtags." Phase 12 says generate 5. Options:
  1. Change the system prompt — drop "never use hashtags" and let the caption prompt control this
  2. Keep system prompt as-is, have a separate hashtag prompt
  3. Drop hashtag generation from LLM entirely, keep client-side `build_hashtags()` Rust function
  Proposal: option 1, since the model needs context (platform + niche + game) that client-side can't easily match.
- **Trim the 10 modes to 5?** Current `tone_instruction()` has 10 modes; only 5 are used in the Free path's mode enum. The "legacy 5" (punchy/clean/funny/hype/search) appear in older paths. Proposal: trim to the core 5; deprecate the rest.
- **Platform-aware hashtags — where does `Platform` come from?** New field on the command? Inferred from export target? Flag as open.
- **3 candidates or 5?** Titles get 5. Captions are longer and more expensive. Proposal: 3 for captions.

---

## 3. New function — `extract_money_quote()`

Per ROADMAP: "pick the single best 2–6 word phrase worth prominently displaying."

### 3a. BYOK path

```rust
pub async fn extract_money_quote_llm(
    api_key: &str,
    model: &str,
    event_summary: &str,
    full_transcript: &str,
    tags: &[String],
) -> Result<Option<String>, AppError>;
```

Prompt:

```text
Extract the single best "money quote" from this gaming clip transcript.
A money quote is a 2–6 word phrase worth prominently displaying on the clip
(title, caption, or video overlay).

WHAT HAPPENED: {event_summary}
TRANSCRIPT: "{full_transcript}"
SIGNALS: {tags}

CRITERIA:
- Must be 2 to 6 words total (strict)
- Must be something the streamer ACTUALLY said (no fabrication)
- Should be self-contained (makes sense out of context)
- Priority: memorable phrasing > on-topic > emotional reaction
- If nothing in transcript qualifies, return null

OUTPUT — JSON only:
{"quote": "...", "confidence": 0.0–1.0}
or
{"quote": null, "confidence": 0.0}
```

Confidence threshold: return `Some(quote)` only if `confidence >= 0.6`.

### 3b. Free path heuristic

```rust
pub fn extract_money_quote_free(
    transcript_segments: &[(f64, f64, String)],  // (start, end, text)
    rms_samples: &[(f64, f32)],                   // (timestamp, rms)
) -> Option<String>;
```

Algorithm:
1. Split transcript into phrases (2–6 word windows)
2. Score each phrase: `emotional_keyword_hit * 0.4 + peak_rms_during_phrase * 0.4 + brevity_bonus * 0.2`
3. Emotional keywords: reuse the keyword→emotion map from `clip_selector.rs:255`
4. Return top-scoring phrase if score ≥ threshold (calibrate on test clips, start at 0.5)

### 3c. Open decisions

- **Is BYOK money-quote a separate API call, or piggybacked onto title/caption?** Separate = cleaner pipeline, cost estimator is accurate. Piggybacked = cheaper but conflates concerns. Proposal: separate call, gated by `ai_for_captions` toggle.
- **Free-path RMS source.** We already compute RMS samples in `analyze_audio_intensity()` ([vod.rs:239](../src-tauri/src/commands/vod.rs)). Need to plumb through to post_captions. Small refactor — acceptable?

---

## 4. Free-path template matrix (emotion × context)

Replaces the current `synthesize_event()` hardcoded compound/single lookup ([post_captions.rs:436–559](../src-tauri/src/post_captions.rs)).

### 4a. Storage

`config/caption_templates.toml` (new file). Hot-reloadable in dev, compiled-in via `include_str!` for release.

### 4b. Shape

```toml
# emotion rows: shock, hype, funny, rage, panic
# context cols: ace, death, clutch, fail, reaction, chase, explosion

[[templates]]
emotion = "shock"
context = "ace"
text = "{streamer} went from zero to {kill_count} in {time}s"
game_vocab = ["valorant", "cs2", "apex"]  # optional — limits to these games

[[templates]]
emotion = "shock"
context = "ace"
text = "clean {kill_count}-for-{kill_count} through {smoke_type}"
slots = ["kill_count", "smoke_type"]  # declares fillable slots

[[templates]]
emotion = "shock"
context = "death"
text = "one tick of damage away. still dies."

# ... 50–80 templates total, 3–5 per (emotion, context) cell
```

### 4c. Slot-filling

Slots filled from:
- **Per-game TOML configs** (Phase 1 deliverable) — `config/games/{game_id}.toml` provides `kill_count`, `smoke_type`, game-specific nouns
- **Clip signals** — `{tags}`, `{event_summary}`, `{game_name}`
- **Fallback** — generic token ("the enemy", "the fight") if no game config matches

### 4d. Integration with existing free-path modes

The mode × gametype system stays. What changes: `synthesize_event()` first consults the emotion × context matrix. If there's a match, it returns that. Otherwise falls back to the current hardcoded templates.

Migration plan:
1. Ship matrix alongside existing `synthesize_event()` — matrix-first, existing fallback
2. Over time, delete rows from the hardcoded tables as coverage improves
3. Eventually the hardcoded tables are empty and deleted

This preserves "small surgical edits" rule — no big-bang rewrite.

### 4e. Community-clip title passthrough (Free path only)

When a Twitch community clip covers the moment ([twitch.rs:519](../src-tauri/src/twitch.rs)), its title is used verbatim as the clip title, subject to:
- Profanity/slur filter (reuse existing filter if one exists; otherwise minimal banlist)
- Length ≤ 60 chars (truncate at word boundary if over)
- Fallback to matrix-generated title if filter rejects

**Where:** in the Free path only — BYOK still calls `generate_llm_titles()` because the model can synthesize better with community clip as extra context.

---

## 5. Shared ranker ([NEW module])

`src-tauri/src/detection/ranker.rs` (new file, small).

### 5a. Scoring function

```rust
pub fn score_title(
    title: &str,
    context: &RankerContext,
) -> f32;

pub struct RankerContext<'a> {
    pub streamer_history: &'a [String],   // past 50 titles — used for overlap check
    pub banned_words: &'a [&'a str],
    pub target_platform: Platform,
    pub has_money_quote: bool,
}
```

Scoring (sum, 0.0–1.0):

| Signal | Weight | Rule |
|---|---|---|
| Contains number/stake | +0.25 | regex `\b\d+\b` or `1v\d`, `0hp`, etc. |
| Length-appropriate | +0.20 | ≤ 42 chars for TikTok mobile; linear penalty above |
| Emotional word | +0.15 | any of ~30 curated words |
| Specific vs generic | +0.20 | deducted if ≥2 generic nouns ("play", "moment", "thing") |
| No banlist hit | +0.10 (hard reject) | 0.0 score if any banned word |
| No history overlap | +0.10 | token-overlap Jaccard < 0.5 with past 50 titles |

### 5b. Same ranker applies to captions

For captions, score `hook_line` only (the critical part) with the same function. Body gets a separate, lighter scoring pass (length + specificity).

### 5c. Where it slots in

Both `generate_llm_titles()` and the Free-path matrix generator produce N candidates. Both pass them through `score_title()`. The highest-scoring candidate wins. In BYOK mode, the ranker re-orders the LLM's candidates (doesn't discard them); in Free mode, it picks from matrix output.

---

## 6. Subtitle rendering (Phase 12 items 10)

Tracked in ROADMAP as part of Phase 12 but is an **export-side** concern, not a prompt concern. Scope split:
- This diff: prompt changes + money-quote extraction + ranker (the "what to show" work)
- Separate diff later: keyword emphasis, money-quote styling, emoji injection, word-level timing (the "how to show" work in `commands/export.rs`)

Proposal: land section 1–5 first, then tackle subtitle rendering separately. Keeps review scope manageable.

---

## 7. Test plan (for when we implement)

| Test | Verifies |
|---|---|
| 10 real VODs, side-by-side (old prompt vs new) | New wins ≥ 7/10 per ROADMAP success criteria |
| Title length: every output ≤ 40 chars | Hard limit holds |
| Banlist: no output contains banned word | Client-side filter works as backstop |
| Pattern validation: every title matches one of 3 regexes | Structural constraint holds |
| Money-quote present → pattern 3 used ≥ 2/5 times | Tone inheritance works |
| Free matrix coverage: every (emotion, context) cell has ≥ 3 templates | No gaps on common combos |
| Ranker: injected "insane epic moment" scores 0 | Banlist rejection works |
| Backward compat: `generate_llm_title()` (single string) still works | Existing callers don't break |

---

## 8. Rollout order

1. Slug reviews + approves this diff (THIS STEP)
2. Phase 5 cleanup (delete dead scaffolding per ROADMAP Day 1)
3. Phase 6.0 toggle framework (prerequisite)
4. Implement section 5 (ranker) — standalone module, unit-testable without LLM calls
5. Implement section 3 (money-quote — BYOK + Free)
6. Implement section 1 (title rewrite)
7. Implement section 2 (caption rewrite)
8. Implement section 4 (Free-path matrix, in parallel with Phase 1 game configs)
9. A/B test on 10 VODs, measure, iterate
10. Ship in the release where Phase 6.0 + Phase 12 both land

---

## 9. Resolved decisions

All 11 open decisions from the first draft resolved 2026-04-20. Recorded here for traceability.

| # | Decision | Resolution | Why |
|---|---|---|---|
| 1 | 6-angle rotation | **Dropped** | 3 structural patterns subsume it and are regex-enforceable. |
| 2 | Streamer-history param | **Keep as `Option<&[String]>`, pass `None` for now** | Cheap to add now, breaking to add later. Phase 11 populates. |
| 3 | All-caps emotion words (pattern 2) | **Title-case** (e.g. "Speechless: one-tap through smoke") | All-caps triggers TikTok/IG spam heuristics and contradicts `LLM_SYSTEM_PROMPT`'s "lowercase is fine". A/B later if desired. |
| 4 | Hashtag policy conflict | **Drop LLM hashtag generation entirely — client-side `build_hashtags()` handles it, expanded with `Platform` + streamer niche + game name** | Hashtags are a deterministic mapping, not creative text. Saves output tokens, keeps system prompt intact, easier to tune. |
| 5 | Trim 10 modes → 5? | **KEEP 10** | Frontend audit: `CopyTone` type in [publishCopyGenerator.ts:24](../src/lib/publishCopyGenerator.ts) + [PublishComposer.tsx:204–207](../src/components/PublishComposer.tsx) exposes all 10 as user-facing tone buttons. Trimming is UX regression. Tune redundant ones' `tone_instruction()` text to be more distinct instead. |
| 6 | `Platform` parameter source | **New command field, default TikTok** | Explicit > implicit. Users edit one clip for multiple platforms; inferring from export target is fragile. TikTok is primary per landing page. |
| 7 | 3 vs 5 caption candidates | **3** | Output-token cost is linear. 5×280 vs 3×280 ≈ 40% more cost per call. 3 structured candidates give enough diversity. |
| 8 | Money-quote: separate vs piggyback | **Separate tiny API call** | Toggles are independent (titles ON, captions OFF is valid). Haiku marginal cost ≈ $0.0005/clip. Gated by `ai_for_titles OR ai_for_captions`. Result feeds both. |
| 9 | Plumb RMS into post_captions | **Yes, as `Option<&[(f64, f32)]>`** | Additive, not a refactor. `None` falls back to keyword-only scoring. |
| 10 | Matrix-first vs big-bang replace | **Matrix-first with hardcoded fallback** | Respects CLAUDE.md "small surgical edits" rule. Ship cells incrementally. |
| 11 | Subtitle rendering split | **Yes, separate diff** | Orthogonal to prompts, touches `commands/export.rs` (683 lines), benefits from own review. |

## 10. Wave 1 deliverables (what's landing first)

1. `src-tauri/src/detection/mod.rs` — new module namespace. Hosts `Platform` enum (`TikTok | YouTubeShorts | InstagramReels | Generic`).
2. `src-tauri/src/detection/ranker.rs` — `RankerContext` + `score_title()` scoring function + banlist + emotional-word list + tests. Standalone, no LLM calls.
3. `src-tauri/src/lib.rs` — add `mod detection;`.
4. `src-tauri/src/post_captions.rs` — expand `build_hashtags()` to accept `Platform` + `streamer_niche_tags` + `game_name`. Old signature preserved as thin wrapper so existing callers don't break.

**Does NOT touch** (saved for Wave 2+):
- `generate_llm()` prompt body
- `generate_llm_title()` prompt body
- `tone_instruction()`
- `synthesize_event()` / Free-path templates
- `commands/captions.rs` or any Tauri command

**Verification plan:**
- Static analysis (Rust unavailable in sandbox — Slug runs `cd src-tauri && cargo check`)
- Unit tests in `ranker.rs` (can run via `cargo test`)
- Regression: existing `build_hashtags` callers still produce same output for the no-platform / no-niche case.
