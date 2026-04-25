//! Title / caption candidate ranker.
//!
//! Both BYOK (LLM-generated) and Free (template-generated) paths emit N
//! candidates. This module scores each candidate 0.0–1.0 and the caller
//! picks the winner.
//!
//! See [`docs/PHASE12_PROMPT_DIFF.md`](../../../../docs/PHASE12_PROMPT_DIFF.md#5-shared-ranker-new-module).
//!
//! ## Scoring dimensions (revised post-research 2026-04-24)
//!
//! Weighting was rebalanced after analysis of ~180 real high-engagement gaming
//! Shorts/TikTok titles (TenZ, OtzStreams, Jynxzi, Lilith Omen). Key findings that
//! drove the rebalance: (1) top-performing titles frequently have NO numbers
//! (e.g. "actually clean" — 1.3M views), so number weight was reduced from 0.25
//! to 0.10. (2) The old emotion-word bonus rewarded cliché labels ("Humbled",
//! "Speechless") that real top titles avoid, so it was removed entirely. (3)
//! Concrete anchors (proper nouns naming the boss/mechanic/player) are the single
//! biggest differentiator in real data, so an anchor bonus was added (up to 0.20).
//! (4) Template artifacts (arrow separators, em-dash separators, "POV:" prefix)
//! appear in ~0% of top titles and scream AI-generated, so those are hard-rejected.
//!
//! | Signal                  | Max  | Rule                                       |
//! |-------------------------|------|--------------------------------------------|
//! | Contains number/stake   | +0.10 | Digit anywhere in title (weak signal now) |
//! | Length-appropriate      | +0.20 | ≤ platform target; linear penalty above   |
//! | Concrete anchor         | +0.20 | 0/1/2+ proper nouns → 0.0/0.10/0.20       |
//! | Specific (not generic)  | +0.20 | Base; deducted if ≥2 generic nouns        |
//! | No history overlap      | +0.10 | Opening-word and Jaccard check             |
//! | No banlist hit          | hard reject | Score = 0.0                         |
//! | No template artifact    | hard reject | Arrow / em-dash sep / "POV:" prefix |
//!
//! Total possible: 0.80. Hard rejects return 0.0 regardless.

use super::Platform;
use std::collections::HashSet;

// ───────────────────────────────────────────────────────────────────
// Default word lists
// ───────────────────────────────────────────────────────────────────

/// Anti-cliché banlist. Any candidate containing one of these is hard-rejected.
///
/// Pulled from Phase 12 spec in ROADMAP.md. Curated for gaming-content-creator
/// voice — these are overused and read as bot-written or clickbait.
pub const DEFAULT_BANNED_WORDS: &[&str] = &[
    "insane",
    "crazy",
    "epic",
    "literally",
    "wild",
    "shocking",
    "unbelievable",
    "omg",
    "mind-blowing",
    "mindblowing",
    "you won't believe",
    "you wont believe",
    "must see",
    "check this",
    "watch this",
    "you need to see",
    // Added 2026-04-24 after research: phrases that tank real engagement.
    "hits different",
    "goes hard",
    "goes crazy",
    "no cap",
    "lowkey",
    // Generic "gaming moment" padding that keeps leaking through the specificity
    // penalty (one generic word alone isn't enough of a penalty to reject).
    "gaming moment",
    "clip moment",
    "stream moment",
    "classic clip",
    "classic moment",
    "classic gaming",
    // Game-name + moment is the exact shape the prompt bans. Include common games.
    "elden ring moment",
    "valorant moment",
    "dbd moment",
    "apex moment",
    "warzone moment",
];

/// Emotional words that score. Used to reward pattern-2-style titles
/// ("{Emotion}: {detail}") and to detect reaction-driven captions.
///
/// Curated for gaming contexts — adjectives/states a streamer would
/// actually use, not generic feelings ("happy", "sad").
pub const DEFAULT_EMOTIONAL_WORDS: &[&str] = &[
    "speechless", "rattled", "floored", "frozen", "gutted",
    "stunned", "dazed", "shook", "wrecked", "panicked",
    "heartbroken", "relieved", "hyped", "ecstatic",
    "clutched", "choked", "crushed", "dominated", "humbled",
    "numb", "livid", "bewildered", "dumbstruck", "paralyzed",
    "elated", "devastated", "overwhelmed", "terrified",
    "tilted", "vindicated", "broken", "dead", "done",
];

/// Generic nouns. 2+ hits on a title = specificity penalty.
///
/// These are the "mush" words that make a title feel like it could apply
/// to any random clip. Punishing them pushes candidates toward concrete
/// detail (kills, smokes, round numbers).
pub const DEFAULT_GENERIC_WORDS: &[&str] = &[
    "play", "moment", "thing", "stuff", "time",
    "situation", "clip", "video", "happened",
];

// ───────────────────────────────────────────────────────────────────
// Scoring context
// ───────────────────────────────────────────────────────────────────

/// Inputs to [`score_title`].
///
/// `streamer_history` is a `Vec<String>` of recent titles for this streamer.
/// Pass an empty slice if unavailable (Phase 11 will populate; Wave 1 passes `&[]`).
pub struct RankerContext<'a> {
    /// Streamer's last ~50 titles. Used for token-overlap check.
    pub streamer_history: &'a [String],
    /// Banned substrings. If any is found in the candidate (case-insensitive),
    /// the score is hard-rejected to 0.0.
    pub banned_words: &'a [&'a str],
    /// Target platform (drives length scoring).
    pub target_platform: Platform,
    /// Whether the clip has a money quote. Currently unused in scoring;
    /// kept in the context for future pattern-inheritance weighting.
    pub has_money_quote: bool,
}

impl<'a> RankerContext<'a> {
    /// Builder-style default: no history, default banlist, generic platform.
    pub fn default_for(platform: Platform) -> Self {
        RankerContext {
            streamer_history: &[],
            banned_words: DEFAULT_BANNED_WORDS,
            target_platform: platform,
            has_money_quote: false,
        }
    }
}

// ───────────────────────────────────────────────────────────────────
// Score function
// ───────────────────────────────────────────────────────────────────

/// Score a title candidate 0.0–1.0.
///
/// Banlist hit → 0.0 (hard reject). Otherwise sums weighted signals.
///
/// # Example
///
/// ```ignore
/// use clipviral::detection::{Platform, ranker::{score_title, RankerContext}};
/// let ctx = RankerContext::default_for(Platform::TikTok);
/// let good = score_title("down 0-12 → 1v5 ACE", &ctx);
/// let bad  = score_title("insane crazy epic gaming moment", &ctx);
/// assert!(good > 0.5);
/// assert_eq!(bad, 0.0);
/// ```
pub fn score_title(title: &str, ctx: &RankerContext) -> f32 {
    // Hard reject: banlist hit (cliché words) or template artifact (arrow, em-dash
    // separator, "POV:" prefix). Either reads as AI-generated in real-world feeds.
    if contains_banned(title, ctx.banned_words) {
        return 0.0;
    }
    if has_template_artifact(title) {
        return 0.0;
    }

    let mut score = 0.0_f32;

    // Number/stake — weak signal now. Many top-performing titles ("actually clean",
    // "dialed in") have zero numbers. Reduced 0.25 → 0.10.
    if contains_number_or_stake(title) {
        score += 0.10;
    }

    score += length_score(title, ctx.target_platform);

    // Concrete anchor: proper nouns (non-sentence-start capitalized words) signal
    // that the title names something specific from the clip. Real top titles
    // consistently include at least one anchor ("Meg learnt to fly", "don't blind
    // Legion when vaulting", "destroyed C9 but lost a friend").
    score += anchor_score(title);

    score += specificity_score(title);

    score += history_overlap_score(title, ctx.streamer_history);

    score.clamp(0.0, 1.0)
}

/// Pick the highest-scoring candidate from a list. Returns None for empty input.
pub fn pick_best<'a>(
    candidates: &'a [String],
    ctx: &RankerContext,
) -> Option<(&'a str, f32)> {
    candidates
        .iter()
        .map(|c| (c.as_str(), score_title(c, ctx)))
        .fold(None, |acc, (text, score)| match acc {
            None => Some((text, score)),
            Some((_, best)) if score > best => Some((text, score)),
            other => other,
        })
}

// ───────────────────────────────────────────────────────────────────
// Internal scoring helpers
// ───────────────────────────────────────────────────────────────────

fn contains_banned(title: &str, banned: &[&str]) -> bool {
    let lower = title.to_lowercase();
    banned.iter().any(|w| lower.contains(w))
}

fn contains_number_or_stake(title: &str) -> bool {
    // Any ASCII digit anywhere. Captures "1v5", "0hp", "12s", "round 12".
    title.chars().any(|c| c.is_ascii_digit())
}

fn length_score(title: &str, platform: Platform) -> f32 {
    let len = title.chars().count();
    let target = platform.title_length_target();

    if len == 0 {
        return 0.0;
    }
    if len <= target {
        return 0.20;
    }
    // Linear penalty: 0.20 at target, 0.0 at target+10
    let over = len.saturating_sub(target);
    if over >= 10 {
        return 0.0;
    }
    0.20 * (1.0 - (over as f32 / 10.0))
}

/// Hard-reject template artifacts that scream "AI-generated" in real feeds.
/// Based on 2026-04-24 research: ~0 of 180 top-performing gaming short-form
/// titles use arrow separators, em-dash separators, or "POV:" prefixes.
fn has_template_artifact(title: &str) -> bool {
    // Arrow separators: literal unicode, ASCII "->", and ">>"
    if title.contains('→') || title.contains("->") {
        return true;
    }
    // Em-dash or en-dash as a separator (surrounded by spaces).
    if title.contains(" — ") || title.contains(" – ") {
        return true;
    }
    // "POV:" prefix — only 2 of 180 real top titles used it; reads as TikTok
    // pastiche rather than creator voice.
    let trimmed_lower_start: String = title
        .chars()
        .skip_while(|c| c.is_whitespace())
        .take(6)
        .collect();
    let trimmed_lower = trimmed_lower_start.to_lowercase();
    if trimmed_lower.starts_with("pov:") || trimmed_lower.starts_with("pov :") {
        return true;
    }
    // Colon-prefix emotion shape — the legacy `{TitleCaseWord}: description` pattern
    // ("Gutted: ...", "Humbled: ..."). Real top-performing titles use this ~5% of
    // the time and always sparingly; when it comes out of an LLM it reads as
    // tabloid-AI. Reject when a short Title-case prefix sits before an early colon.
    if let Some(colon_pos) = title.find(':') {
        if colon_pos < 30 && colon_pos > 0 {
            let before_colon = &title[..colon_pos];
            let word_count = before_colon.split_whitespace().count();
            // 1-2 words before the colon, no quote chars (lets speech quotes pass),
            // starting with an ASCII uppercase letter → legacy Pattern 2 shape.
            if word_count >= 1
                && word_count <= 2
                && !before_colon.contains('"')
                && !before_colon.contains('\'')
            {
                let first_char = title.chars().next();
                if matches!(first_char, Some(c) if c.is_ascii_uppercase()) {
                    return true;
                }
            }
        }
    }
    false
}

/// Count "concrete anchors" — proper nouns signalled by capitalization after
/// the first word. The sentence-start is excluded so normal capitalization
/// doesn't count. Multi-letter caps-only tokens ("DBD", "HP") always count
/// even at position 0 since they're unambiguously acronyms.
fn anchor_score(title: &str) -> f32 {
    let words: Vec<&str> = title.split_whitespace().collect();
    let mut anchors = 0;
    for (i, w) in words.iter().enumerate() {
        // Strip punctuation from edges to judge the word itself.
        let stripped: String = w
            .chars()
            .filter(|c| c.is_ascii_alphabetic())
            .collect();
        if stripped.len() < 2 {
            continue;
        }
        let first = stripped.chars().next().unwrap();
        let is_all_caps = stripped.chars().all(|c| c.is_ascii_uppercase());
        let starts_capital = first.is_ascii_uppercase();

        // ALL-CAPS acronym anywhere counts. Otherwise only non-first-position
        // capitalized words count (to avoid rewarding ordinary sentence starts).
        if is_all_caps && stripped.len() >= 2 {
            anchors += 1;
        } else if starts_capital && i > 0 {
            anchors += 1;
        }
    }
    match anchors {
        0 => 0.0,
        1 => 0.10,
        _ => 0.20,
    }
}

fn specificity_score(title: &str) -> f32 {
    let lower = title.to_lowercase();
    let generic_hits = tokens(&lower)
        .filter(|t| DEFAULT_GENERIC_WORDS.contains(t))
        .count();

    match generic_hits {
        0 => 0.20,
        1 => 0.10,
        _ => 0.0, // 2+ generic words = penalty
    }
}

fn history_overlap_score(title: &str, history: &[String]) -> f32 {
    if history.is_empty() {
        // No history = neutral. Award half so new streamers aren't punished.
        return 0.10;
    }

    let title_tokens = token_set(title);
    if title_tokens.is_empty() {
        return 0.0;
    }

    let max_overlap = history
        .iter()
        .map(|past| jaccard(&title_tokens, &token_set(past)))
        .fold(0.0_f32, f32::max);

    if max_overlap < 0.5 {
        0.10
    } else if max_overlap >= 1.0 {
        0.0
    } else {
        // Linear falloff from 0.10 at 0.5 overlap to 0.0 at 1.0 overlap
        0.10 * (1.0 - (max_overlap - 0.5) * 2.0)
    }
}

// ───────────────────────────────────────────────────────────────────
// Token helpers
// ───────────────────────────────────────────────────────────────────

/// Iterator over word tokens (lowercase-alphabetic only, 3+ chars).
///
/// Stripping non-alphabetic characters means "1v5" yields "v" (too short,
/// filtered). That's fine — numbers are scored separately, tokens are for
/// word matching.
fn tokens(lower: &str) -> impl Iterator<Item = &str> + '_ {
    lower
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|t| t.len() >= 3)
}

/// Collect tokens as a set of owned strings for Jaccard comparison.
fn token_set(s: &str) -> HashSet<String> {
    let lower = s.to_lowercase();
    tokens(&lower).map(|t| t.to_string()).collect()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count() as f32;
    let union = a.union(b).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

// ═══════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_tiktok() -> RankerContext<'static> {
        RankerContext::default_for(Platform::TikTok)
    }

    // ── Banlist ──────────────────────────────────────────────────

    #[test]
    fn banned_word_hard_rejects() {
        let ctx = ctx_tiktok();
        assert_eq!(score_title("insane 1v5 ace", &ctx), 0.0);
        assert_eq!(score_title("crazy play from the boys", &ctx), 0.0);
        assert_eq!(score_title("you won't believe this", &ctx), 0.0);
    }

    #[test]
    fn banlist_is_case_insensitive() {
        let ctx = ctx_tiktok();
        assert_eq!(score_title("INSANE moment", &ctx), 0.0);
        assert_eq!(score_title("EPIC win", &ctx), 0.0);
    }

    // ── Number/stake ─────────────────────────────────────────────

    #[test]
    fn number_boosts_score() {
        let ctx = ctx_tiktok();
        let with_num = score_title("1v5 clutch through smoke", &ctx);
        let no_num   = score_title("clutch through smoke", &ctx);
        assert!(with_num > no_num, "number should boost: {} vs {}", with_num, no_num);
    }

    #[test]
    fn stake_patterns_detected() {
        let ctx = ctx_tiktok();
        assert!(score_title("0hp win", &ctx) > 0.25);
        assert!(score_title("12 second ace", &ctx) > 0.25);
        assert!(score_title("round 12 heroics", &ctx) > 0.25);
    }

    // ── Length ───────────────────────────────────────────────────

    #[test]
    fn short_title_scores_max_length() {
        assert_eq!(length_score("tiny", Platform::TikTok), 0.20);
    }

    #[test]
    fn long_title_penalized() {
        // Target is 60 after the Wave 3 bump. 65 chars = over by 5 → 0.20 * (1 - 0.5) = 0.10.
        let over = "a".repeat(65);
        let at_limit = "a".repeat(60);
        assert!(length_score(&over, Platform::Generic) < length_score(&at_limit, Platform::Generic));
    }

    #[test]
    fn very_long_title_zero_length_score() {
        assert_eq!(length_score(&"x".repeat(100), Platform::TikTok), 0.0);
    }

    #[test]
    fn empty_title_zero_length_score() {
        assert_eq!(length_score("", Platform::TikTok), 0.0);
    }

    // ── Concrete anchor (proper nouns) ───────────────────────────

    #[test]
    fn anchor_rewards_proper_nouns() {
        assert_eq!(anchor_score("meg learnt to fly"), 0.0);
        assert_eq!(anchor_score("Meg learnt to fly"), 0.0); // sentence-start, not counted
        assert_eq!(anchor_score("look what Meg did"), 0.10); // 1 proper noun mid-sentence
        assert_eq!(anchor_score("Legion chased Meg around"), 0.10); // Legion = sentence-start, Meg = anchor
        assert_eq!(anchor_score("DBD is ridiculous"), 0.10); // DBD = all-caps acronym counts
        assert_eq!(anchor_score("don't blind Legion when vaulting near Meg"), 0.20); // 2+ anchors
    }

    // ── Template artifacts (hard reject) ─────────────────────────

    #[test]
    fn arrow_separator_is_rejected() {
        let ctx = ctx_tiktok();
        assert_eq!(score_title("down 0-12 -> 1v5 ACE", &ctx), 0.0);
        assert_eq!(score_title("boss fog → five seconds of silence", &ctx), 0.0);
    }

    #[test]
    fn pov_prefix_is_rejected() {
        let ctx = ctx_tiktok();
        assert_eq!(score_title("POV: you walked into Malenia", &ctx), 0.0);
        assert_eq!(score_title("pov: tried the fog gate", &ctx), 0.0);
    }

    #[test]
    fn em_dash_separator_is_rejected() {
        let ctx = ctx_tiktok();
        assert_eq!(score_title("easy boss — not so easy", &ctx), 0.0);
    }

    #[test]
    fn pov_inside_title_not_rejected() {
        // "pov" inside the title (not as prefix) is fine
        let ctx = ctx_tiktok();
        assert!(score_title("my pov mid fight was chaos", &ctx) > 0.0);
    }

    #[test]
    fn colon_prefix_emotion_shape_is_rejected() {
        // Legacy Pattern 2 shape the new framework retired.
        let ctx = ctx_tiktok();
        assert_eq!(score_title("Gutted: stuck in the animation", &ctx), 0.0);
        assert_eq!(score_title("Humbled: boss won before i blinked", &ctx), 0.0);
        assert_eq!(score_title("Paralyzed: still standing there", &ctx), 0.0);
    }

    #[test]
    fn quoted_colon_passes() {
        // Speech quotes with colons inside shouldn't trigger the emotion-shape reject.
        let ctx = ctx_tiktok();
        assert!(score_title("\"easy boss\": 0-1 next round", &ctx) > 0.0);
    }

    // ── Specificity ──────────────────────────────────────────────

    #[test]
    fn two_generic_words_zero_specificity() {
        // "moment" + "thing" both generic
        assert_eq!(specificity_score("that moment was a thing"), 0.0);
    }

    #[test]
    fn one_generic_word_half_specificity() {
        assert_eq!(specificity_score("watching the clip unfold"), 0.10);
    }

    #[test]
    fn no_generic_words_full_specificity() {
        assert_eq!(specificity_score("ace through triple smoke"), 0.20);
    }

    // ── History overlap ──────────────────────────────────────────

    #[test]
    fn empty_history_awards_neutral() {
        assert_eq!(history_overlap_score("any title here", &[]), 0.10);
    }

    #[test]
    fn high_overlap_penalizes() {
        let history = vec!["ace through triple smoke".to_string()];
        // Exact-match = max overlap
        let exact = history_overlap_score("ace through triple smoke", &history);
        let diff  = history_overlap_score("down 0-12 1v5 victory", &history);
        assert!(diff > exact);
    }

    #[test]
    fn low_overlap_full_history_score() {
        let history = vec!["ace through triple smoke".to_string()];
        // Jaccard < 0.5 → full 0.10
        assert_eq!(history_overlap_score("down 0-12 1v5 victory", &history), 0.10);
    }

    // ── End-to-end ───────────────────────────────────────────────

    #[test]
    fn good_title_scores_above_half() {
        let ctx = ctx_tiktok();
        // Real top-performer shape: short, anchor, specific, no template artifact
        let score = score_title("don't blind Legion when vaulting", &ctx);
        assert!(score > 0.5, "expected > 0.5, got {}", score);
    }

    #[test]
    fn banned_beats_everything() {
        let ctx = ctx_tiktok();
        // Would score well on other dimensions but banlist kills it
        assert_eq!(score_title("insane 1v5 ace through smoke", &ctx), 0.0);
    }

    #[test]
    fn scores_clamped_to_unit() {
        let ctx = ctx_tiktok();
        let score = score_title("clean Margit parry at 1hp", &ctx);
        assert!(score <= 1.0);
        assert!(score >= 0.0);
    }

    // ── Pick best ────────────────────────────────────────────────

    #[test]
    fn pick_best_returns_winner() {
        let ctx = ctx_tiktok();
        let candidates = vec![
            "insane moment".to_string(),           // 0.0 (banned)
            "clutch play thing".to_string(),       // low
            "speechless: 1v5 through smoke".to_string(), // high
        ];
        let (best, score) = pick_best(&candidates, &ctx).expect("should pick one");
        assert_eq!(best, "speechless: 1v5 through smoke");
        assert!(score > 0.5);
    }

    #[test]
    fn pick_best_empty_returns_none() {
        let ctx = ctx_tiktok();
        let candidates: Vec<String> = vec![];
        assert!(pick_best(&candidates, &ctx).is_none());
    }

    #[test]
    fn pick_best_all_zero_still_returns_something() {
        // When everything is banned, pick_best still returns the first (score 0.0)
        // — the CALLER decides whether to fall back to Free path.
        let ctx = ctx_tiktok();
        let candidates = vec![
            "insane one".to_string(),
            "crazy two".to_string(),
        ];
        let result = pick_best(&candidates, &ctx);
        assert!(result.is_some());
        let (_, score) = result.unwrap();
        assert_eq!(score, 0.0);
    }

    // ── Platform differences ─────────────────────────────────────

    #[test]
    fn all_platforms_share_length_target() {
        // All platforms now use a 60-char target (see Platform::title_length_target).
        // Previously Reels tolerated longer; that was removed because the difference
        // wasn't pulling its weight and made cross-platform regeneration inconsistent.
        let title_45 = "a".repeat(45);
        let tiktok_score = length_score(&title_45, Platform::TikTok);
        let reels_score = length_score(&title_45, Platform::InstagramReels);
        assert_eq!(tiktok_score, reels_score);
        assert_eq!(reels_score, 0.20);
    }
}
