//! Title / caption candidate ranker.
//!
//! Both BYOK (LLM-generated) and Free (template-generated) paths emit N
//! candidates. This module scores each candidate 0.0–1.0 and the caller
//! picks the winner.
//!
//! See [`docs/PHASE12_PROMPT_DIFF.md`](../../../../docs/PHASE12_PROMPT_DIFF.md#5-shared-ranker-new-module).
//!
//! ## Scoring dimensions
//!
//! | Signal              | Max  | Rule                                             |
//! |---------------------|------|--------------------------------------------------|
//! | Contains number/stake | +0.25 | Digit anywhere in title                        |
//! | Length-appropriate  | +0.20 | ≤ platform target; linear penalty above          |
//! | Emotional word      | +0.15 | Any word in the curated emotional word list      |
//! | Specific (not generic) | +0.20 | Base; deducted if ≥2 generic nouns present    |
//! | No history overlap  | +0.10 | Jaccard < 0.5 with recent streamer titles        |
//! | No banlist hit      | hard reject | Score = 0.0 if any banned word present     |
//!
//! Total possible: 1.00. Ban-list hit returns 0.0 regardless.

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
    // Hard reject on banlist hit
    if contains_banned(title, ctx.banned_words) {
        return 0.0;
    }

    let mut score = 0.0_f32;

    if contains_number_or_stake(title) {
        score += 0.25;
    }

    score += length_score(title, ctx.target_platform);

    if contains_emotional_word(title) {
        score += 0.15;
    }

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

fn contains_emotional_word(title: &str) -> bool {
    // Inline the token split rather than going through `tokens()` — an iterator
    // chain returned as the trailing expression would outlive `lower` due to
    // `impl Trait` drop-order semantics. Binding or inlining avoids E0597.
    let lower = title.to_lowercase();
    lower
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|t| t.len() >= 3)
        .any(|t| DEFAULT_EMOTIONAL_WORDS.contains(&t))
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
        // 42+5 = 47, over by 5 → 0.20 * (1 - 0.5) = 0.10
        let over = "a".repeat(47);
        let at_limit = "a".repeat(42);
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

    // ── Emotional words ──────────────────────────────────────────

    #[test]
    fn emotional_word_boosts() {
        let ctx = ctx_tiktok();
        let with_emo = score_title("speechless: one tap through smoke", &ctx);
        let no_emo   = score_title("one tap through smoke", &ctx);
        assert!(with_emo > no_emo);
    }

    #[test]
    fn emotional_word_requires_word_boundary() {
        // "speechlessly" should still tokenize to "speechlessly", not match "speechless"
        let ctx = ctx_tiktok();
        let compound = score_title("speechlessly watched the play", &ctx);
        let match_  = score_title("speechless: watched the play", &ctx);
        // Compound word should NOT get the emotional-word bonus
        assert!(match_ >= compound);
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
        // Number + short + emotional + specific + no history
        let score = score_title("speechless: 1v5 through triple smoke", &ctx);
        assert!(score > 0.5, "expected > 0.5, got {}", score);
    }

    #[test]
    fn banned_beats_everything() {
        let ctx = ctx_tiktok();
        // Would score very high on all other dimensions but banlist kills it
        assert_eq!(score_title("insane speechless 1v5 ace", &ctx), 0.0);
    }

    #[test]
    fn scores_clamped_to_unit() {
        let ctx = ctx_tiktok();
        let score = score_title("speechless 1v5 0hp ace", &ctx);
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
    fn reels_tolerates_longer_than_tiktok() {
        let title_45 = "a".repeat(45);
        let tiktok_score = length_score(&title_45, Platform::TikTok);
        let reels_score = length_score(&title_45, Platform::InstagramReels);
        // 45 chars: over TikTok's 42 limit (penalty), under Reels' 50 (no penalty)
        assert!(reels_score > tiktok_score);
        assert_eq!(reels_score, 0.20);
    }
}
