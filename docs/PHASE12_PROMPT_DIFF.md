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

**Wave 1 status:** landed in commit `ae0dc1a`. 70/70 tests green.

---

## 11. Wave 2 — concrete Rust diff for review

**Status:** design reviewed + approved. This section is the code-level plan Slug approves **before** any Rust edit against `post_captions.rs`.

**Scope:**
1. Add `TitleCandidate` + `TitlePattern` types
2. Add `generate_llm_titles()` (new function — 3-pattern structured prompt, JSON 5-candidate output, ranker-scored)
3. Add `extract_money_quote_llm()` (BYOK — separate tiny API call)
4. Add `extract_money_quote_free()` (pure heuristic — no API)
5. Add non-async unit tests (JSON parse, pattern regex, Free heuristic)

**Out of scope for Wave 2** (explicit — keeps review tight):
- Modifying the existing `generate_llm_title()` body or its prompt. It stays as-is; the new function lives alongside. Caller migration happens in a follow-up once the new function is proven.
- Any caller changes in `commands/captions.rs`. Zero touch.
- Wave 3 items (caption rewrite, Free-path emotion × context matrix).

### 11a. New types (append to `post_captions.rs` near the LLM section)

```rust
// ═══════════════════════════════════════════════════════════════════
//  Title candidate types (Phase 12 Wave 2)
// ═══════════════════════════════════════════════════════════════════

/// One of three structural patterns a title candidate must match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TitlePattern {
    /// `{stake} → {outcome}` — e.g. "down 0-12 → 1v5 ACE"
    StakeArrowOutcome,
    /// `{Emotion}: {specific detail}` — e.g. "Speechless: one-tap through smoke"
    EmotionColonDetail,
    /// `"{money quote}" {twist}` — e.g. `"I can't miss" — misses next shot`
    QuoteTwist,
}

/// One title candidate with its structural pattern and post-hoc score.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TitleCandidate {
    pub text: String,
    pub pattern: TitlePattern,
    /// Score from `detection::ranker::score_title`. 0.0 = banlist hit (reject).
    #[serde(default)]
    pub score: f32,
}
```

### 11b. New function: `generate_llm_titles()`

Signature (note the new `money_quote` and `streamer_history` params):

```rust
pub async fn generate_llm_titles(
    api_key: &str,
    model: &str,
    event_summary: &str,
    money_quote: Option<&str>,
    full_transcript: Option<&str>,
    tags: &[String],
    game_name: Option<&str>,
    streamer_history: Option<&[String]>,
) -> Result<Vec<TitleCandidate>, AppError>;
```

Body outline (pseudocode — full prompt text in section 1d of this doc):

```rust
pub async fn generate_llm_titles(...) -> Result<Vec<TitleCandidate>, AppError> {
    let transcript_section = /* identical to existing generate_llm_title logic */;
    let money_quote_line = money_quote
        .map(|q| format!("MONEY QUOTE (if any): \"{}\"\n", q))
        .unwrap_or_default();
    let tag_line = /* identical */;
    let game_line = /* identical */;
    let history_line = match streamer_history {
        Some(h) if !h.is_empty() => format!("STREAMER RECENT TITLES:\n{}\n", h.join("\n")),
        _ => "STREAMER RECENT TITLES: none yet\n".into(),
    };

    let prompt = format!(
        r#"Generate 5 SHORT title candidates for a gaming clip. Return JSON only.

{game}WHAT HAPPENED: {event}
{money_quote}{transcript}{tags}
{history}
STRUCTURE — every candidate MUST follow exactly one of these three patterns:
... (full prompt text from section 1d)

OUTPUT — JSON only, no prose:
{{"candidates": [{{"pattern": "...", "text": "..."}}, ...5 total]}}"#,
        game = game_line,
        event = event_summary,
        money_quote = money_quote_line,
        transcript = transcript_section,
        tags = tag_line,
        history = history_line,
    );

    // Same request infrastructure as existing generate_llm_title
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 400,  // 5 × ~40 chars + JSON overhead
        "system": LLM_SYSTEM_PROMPT,
        "messages": [{"role": "user", "content": prompt}],
    });

    let resp = client.post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body).send().await
        .map_err(|e| AppError::Api(format!("Claude titles request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Api(format!("Claude API {}: {}", status, &body[..body.len().min(200)])));
    }

    let resp_json: serde_json::Value = resp.json().await
        .map_err(|e| AppError::Api(format!("Failed to parse Claude response: {e}")))?;

    let text = resp_json["content"][0]["text"].as_str()
        .ok_or_else(|| AppError::Api("No text in Claude titles response".into()))?;

    // Parse the JSON output
    let parsed: TitlesResponse = serde_json::from_str(text)
        .or_else(|_| extract_json_from_markdown(text).and_then(|s| serde_json::from_str(&s)))
        .map_err(|e| AppError::Api(format!("Malformed titles JSON: {e}")))?;

    // Score each candidate via the ranker
    use crate::detection::{Platform, ranker};
    let ctx = ranker::RankerContext {
        streamer_history: streamer_history.unwrap_or(&[]),
        banned_words: ranker::DEFAULT_BANNED_WORDS,
        target_platform: Platform::TikTok,  // default; caller can re-score for other platforms
        has_money_quote: money_quote.is_some(),
    };

    let mut scored: Vec<TitleCandidate> = parsed.candidates.into_iter()
        .map(|c| TitleCandidate {
            score: ranker::score_title(&c.text, &ctx),
            text: c.text,
            pattern: c.pattern,
        })
        .collect();

    // Sort descending by score — caller picks candidates[0]
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    if scored.is_empty() {
        return Err(AppError::Api("Claude returned zero title candidates".into()));
    }

    Ok(scored)
}

#[derive(serde::Deserialize)]
struct TitlesResponse {
    candidates: Vec<TitleCandidate>,
}

/// If the model wraps the JSON in a markdown code fence, strip it.
fn extract_json_from_markdown(text: &str) -> Result<String, AppError> {
    let trimmed = text.trim();
    if let Some(stripped) = trimmed.strip_prefix("```json").and_then(|s| s.strip_suffix("```")) {
        Ok(stripped.trim().to_string())
    } else if let Some(stripped) = trimmed.strip_prefix("```").and_then(|s| s.strip_suffix("```")) {
        Ok(stripped.trim().to_string())
    } else {
        // Try to find the first '{' and last '}' and slice
        let start = trimmed.find('{').ok_or_else(|| AppError::Api("No JSON in response".into()))?;
        let end = trimmed.rfind('}').ok_or_else(|| AppError::Api("No JSON end in response".into()))?;
        Ok(trimmed[start..=end].to_string())
    }
}
```

**Design notes:**
- `generate_llm_titles` does NOT client-side enforce the 40-char limit — that's the ranker's job (length scoring). Over-40-char candidates get score penalties but aren't rejected. Caller decides.
- Scoring happens inside the function so the returned `Vec<TitleCandidate>` is pre-sorted.
- Platform defaults to `TikTok` for scoring; for multi-platform use the caller can re-score via `ranker::score_title` with a different platform.

### 11c. Keeping old `generate_llm_title()` alive (no changes)

The existing `pub async fn generate_llm_title(...) -> Result<String, AppError>` at [post_captions.rs:1360](../src-tauri/src/post_captions.rs) is **not modified** in Wave 2. It stays byte-identical. This keeps `commands/captions.rs:446` working with zero changes and gives us a safe rollback path.

Caller migration happens as a follow-up after Wave 2 is proven in integration: either a tiny "Wave 2.5" PR or bundled with Wave 3. That migration will:
1. Change `commands/captions.rs` to call `generate_llm_titles(..., money_quote=None, streamer_history=None)` and take `.text` of the first candidate.
2. Mark old `generate_llm_title()` `#[deprecated]` then delete.

### 11d. Money-quote extraction

**BYOK path — new function in `post_captions.rs`:**

```rust
pub async fn extract_money_quote_llm(
    api_key: &str,
    model: &str,
    event_summary: &str,
    full_transcript: &str,
    tags: &[String],
) -> Result<Option<String>, AppError> {
    let tag_line = if tags.is_empty() {
        String::new()
    } else {
        format!("SIGNALS: {}\n", tags.join(", "))
    };

    let prompt = format!(
        r#"Extract the single best "money quote" from this gaming clip transcript.
A money quote is a 2–6 word phrase worth prominently displaying on the clip
(title, caption, or video overlay).

WHAT HAPPENED: {event}
TRANSCRIPT: "{transcript}"
{tags}
CRITERIA:
- Must be 2 to 6 words total (strict)
- Must be something the streamer ACTUALLY said (no fabrication)
- Should be self-contained (makes sense out of context)
- Priority: memorable phrasing > on-topic > emotional reaction
- If nothing in transcript qualifies, return null

OUTPUT — JSON only:
{{"quote": "...", "confidence": 0.0-1.0}}
or
{{"quote": null, "confidence": 0.0}}"#,
        event = event_summary,
        transcript = full_transcript.chars().take(800).collect::<String>(),
        tags = tag_line,
    );

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 60,
        "system": LLM_SYSTEM_PROMPT,
        "messages": [{"role": "user", "content": prompt}],
    });

    let resp = client.post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body).send().await
        .map_err(|e| AppError::Api(format!("Money-quote request failed: {e}")))?;

    if !resp.status().is_success() {
        // Non-fatal — caller treats as None
        return Ok(None);
    }

    let resp_json: serde_json::Value = resp.json().await
        .map_err(|e| AppError::Api(format!("Malformed money-quote response: {e}")))?;

    let text = resp_json["content"][0]["text"].as_str()
        .ok_or_else(|| AppError::Api("No text in money-quote response".into()))?;

    #[derive(serde::Deserialize)]
    struct MoneyQuoteResponse {
        quote: Option<String>,
        #[serde(default)]
        confidence: f32,
    }

    let parsed: MoneyQuoteResponse = serde_json::from_str(text.trim())
        .or_else(|_| extract_json_from_markdown(text).and_then(|s| serde_json::from_str(&s)))
        .map_err(|e| AppError::Api(format!("Malformed money-quote JSON: {e}")))?;

    // Confidence threshold: return None below 0.6
    if parsed.confidence < 0.6 {
        return Ok(None);
    }

    Ok(parsed.quote.filter(|q| {
        let wc = q.split_whitespace().count();
        wc >= 2 && wc <= 6  // client-side 2-6 word enforcement
    }))
}
```

**Free path — pure function, no API, unit-testable:**

```rust
pub fn extract_money_quote_free(
    transcript_segments: &[(f64, f64, String)],  // (start_sec, end_sec, text)
    rms_samples: Option<&[(f64, f32)]>,           // (timestamp_sec, rms)
) -> Option<String> {
    const EMOTIONAL_KEYWORDS: &[&str] = &[
        "insane", "crazy", "clutch", "wow", "nope", "yes", "no way",
        "what", "damn", "lord", "god", "jesus", "holy", "dude", "bro",
        "kidding", "serious", "actually", "literally", "honestly",
        // Include the emotional words from the ranker list too
    ];

    let mut best: Option<(f32, String)> = None;

    for (start, end, text) in transcript_segments {
        // Slide a 2-6 word window over the segment
        let words: Vec<&str> = text.split_whitespace().collect();
        for window_size in 2..=6 {
            for start_idx in 0..words.len().saturating_sub(window_size - 1) {
                let phrase_words = &words[start_idx..start_idx + window_size];
                let phrase = phrase_words.join(" ");

                let keyword_hit = phrase_words.iter().any(|w| {
                    let clean = w.trim_matches(|c: char| !c.is_alphabetic()).to_lowercase();
                    EMOTIONAL_KEYWORDS.iter().any(|k| clean == *k)
                });

                let rms_during = rms_samples
                    .map(|samples| samples.iter()
                        .filter(|(t, _)| t >= start && t <= end)
                        .map(|(_, r)| *r)
                        .fold(0.0_f32, f32::max))
                    .unwrap_or(0.5);

                let brevity_bonus = (6 - window_size) as f32 / 4.0 * 0.2;

                let score = (if keyword_hit { 0.4 } else { 0.0 })
                    + (rms_during * 0.4)
                    + brevity_bonus;

                if score > 0.5 && best.as_ref().map_or(true, |(s, _)| score > *s) {
                    best = Some((score, phrase));
                }
            }
        }
    }

    best.map(|(_, phrase)| phrase)
}
```

### 11e. Tests (non-async, unit-testable)

Add to the existing `#[cfg(test)] mod tests` block:

```rust
// ── TitleCandidate / TitlePattern ─────────────────────────────

#[test]
fn title_pattern_serialize_screaming_snake() {
    let p = TitlePattern::StakeArrowOutcome;
    let s = serde_json::to_string(&p).unwrap();
    assert_eq!(s, "\"STAKE_ARROW_OUTCOME\"");
}

#[test]
fn title_pattern_deserialize_from_llm_shape() {
    let json = r#"{"text": "1v5 ACE", "pattern": "STAKE_ARROW_OUTCOME"}"#;
    let c: TitleCandidate = serde_json::from_str(json).unwrap();
    assert_eq!(c.text, "1v5 ACE");
    assert_eq!(c.pattern, TitlePattern::StakeArrowOutcome);
}

#[test]
fn extract_json_strips_markdown_fence() {
    assert_eq!(extract_json_from_markdown("```json\n{\"x\": 1}\n```").unwrap(), "{\"x\": 1}");
    assert_eq!(extract_json_from_markdown("{\"x\": 1}").unwrap(), "{\"x\": 1}");
    assert_eq!(
        extract_json_from_markdown("here is your answer:\n{\"x\": 1}\nhope it helps").unwrap(),
        "{\"x\": 1}",
    );
}

// ── extract_money_quote_free ──────────────────────────────────

#[test]
fn money_quote_free_picks_emotional_short_phrase() {
    let segments = vec![
        (10.0, 12.0, "I have no idea what just happened".to_string()),
        (12.0, 14.0, "that was actually insane dude".to_string()),
    ];
    let rms = vec![(11.0, 0.3), (13.0, 0.9)];
    let q = extract_money_quote_free(&segments, Some(&rms));
    assert!(q.is_some());
    let q = q.unwrap();
    let wc = q.split_whitespace().count();
    assert!(wc >= 2 && wc <= 6, "got {} words: {}", wc, q);
}

#[test]
fn money_quote_free_returns_none_on_empty() {
    assert!(extract_money_quote_free(&[], None).is_none());
}

#[test]
fn money_quote_free_works_without_rms() {
    let segments = vec![(0.0, 2.0, "actually insane play".to_string())];
    // Without RMS, we fall back to 0.5 default. Keyword hit + brevity should still score.
    let q = extract_money_quote_free(&segments, None);
    assert!(q.is_some());
}
```

### 11f. Resolved Wave 2 decisions

| # | Decision | Resolution | Why |
|---|---|---|---|
| 1 | Markdown fence extraction | 3-layer fallback: ```` ```json ````, generic ``` ``` ```, then first-`{` to last-`}` slice | Handles 95% of real failure modes. Structured-output mode is a later migration. |
| 2 | Money-quote confidence threshold | 0.6 + `log::debug!` actual confidence per call | Natural split point between real and marginal phrases. Log gives us data to tune after ~20 real clips. |
| 3 | Default platform for title scoring | `target_platform: Option<Platform>` param, `None` → TikTok | Caller during analysis has no platform; caller during publish does. Honest defaulting beats silent assumption. |
| 4 | Free-path emotional keywords | **Reuse `ranker::DEFAULT_EMOTIONAL_WORDS`** (30 curated words from Wave 1) | DRY. Expanding vocab without a bigger corpus is risky; Wave 3 template matrix will share the list. |
| 5 | Money-quote return type | `Result<Option<String>>` — both layers kept | Preserves API-failure vs genuine-no-quote distinction for telemetry + cost accounting. |

### 11g. Implementation order (cargo-test checkpoints)

1. **Types** — `TitleCandidate` + `TitlePattern` (~20 lines, no deps, no tests needed)
2. **JSON utility + tests** — `extract_json_from_markdown` (pure, unit-testable in sandbox) → ✋ checkpoint
3. **Free money-quote + tests** — `extract_money_quote_free` (pure, reuses ranker constants) → ✋ checkpoint
4. **BYOK money-quote** — `extract_money_quote_llm` (async, serde JSON parse only is unit-testable)
5. **Title generator** — `generate_llm_titles` (async) → ✋ final checkpoint

Steps 1–3 fully unit-testable. Steps 4–5 depend on Slug running the app for real LLM validation.
