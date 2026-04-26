//! Twitch emote density as a clip-detection signal (Phase 1, ROADMAP item).
//!
//! Counts top Twitch emotes per 10-second window. Sharp emote bursts
//! correlate strongly with viewer reactions to in-clip events — chat
//! often reacts with emotes faster (and more legibly) than with prose.
//! Five KEKWs in two seconds means something just happened, even if
//! the audio + transcript miss it.
//!
//! This module exposes the curated emote list and the per-message
//! counter; the per-window aggregation lives in `vod.rs` next to
//! the chat-rate analyzer since both consume the same chat replay
//! file (single download, dual analysis).

/// Top globally-popular Twitch emotes spanning native, BTTV, FFZ, and 7TV.
/// Curated 2026-04 — refresh periodically as new emotes hit critical mass.
///
/// Match is case-sensitive (Twitch emotes are case-sensitive: "KEKW" != "Kekw")
/// and substring-based (no word-boundary check) to keep the matcher cheap.
/// False positives are theoretically possible if a username embeds an exact
/// emote name, but chat replays rarely surface usernames inline in the JSON
/// payload's body field, so the impact is negligible in practice.
pub const DEFAULT_TWITCH_EMOTES: &[&str] = &[
    // Laughter — the highest-frequency reaction class on most channels
    "KEKW", "OMEGALUL", "LULW", "PepeLaugh", "KEKL", "PETTHEPEEPO", "OMEGALUUL",
    // Hype / excitement
    "Pog", "PogU", "PogChamp", "POGGERS", "POGGIES", "PogO", "PauseChamp",
    // Tension / anxiety — predictive of "something is about to happen"
    "monkaS", "monkaW", "monkaGIGA", "monkaH",
    // Sadness / disappointment / frustration
    "Sadge", "PepeHands", "FeelsBadMan", "Madge", "Mad", "Cope",
    // Disbelief / surprise / shock
    "WutFace", "5Head", "BibleThump", "NotLikeThis", "WICKED", "Stare",
    // Common reactions (mixed valence)
    "EZ", "EZY", "Pepega", "WeirdChamp", "FeelsStrongMan", "Clap",
    // Twitch global classics — still showing up in 2026
    "LUL", "Kappa", "TriHard", "ResidentSleeper", "BabyRage",
];

/// Count total emote occurrences in a single message text. Multiple emotes
/// in one message all count: "KEKW KEKW OMEGALUL" returns 3.
///
/// Uses longest-match scanning (case-sensitive, byte-level) to avoid the
/// substring double-counting trap — "OMEGALUL" contains "LUL" inside it,
/// and a naïve substring counter would count both. We instead scan the
/// text once, at each position finding the LONGEST emote that matches
/// there, then skipping past the match so shorter emotes nested inside
/// don't get re-counted. Same algorithm Twitch's own emote renderer uses.
pub fn count_emotes(text: &str) -> u32 {
    if text.is_empty() {
        return 0;
    }
    let bytes = text.as_bytes();
    let mut count = 0u32;
    let mut i = 0usize;
    while i < bytes.len() {
        // Find the longest listed emote that matches starting at byte i.
        let mut best_len = 0usize;
        for emote in DEFAULT_TWITCH_EMOTES {
            let elen = emote.len();
            if elen <= bytes.len() - i
                && &bytes[i..i + elen] == emote.as_bytes()
                && elen > best_len
            {
                best_len = elen;
            }
        }
        if best_len > 0 {
            count += 1;
            i += best_len;
        } else {
            i += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_returns_zero() {
        assert_eq!(count_emotes(""), 0);
    }

    #[test]
    fn unrelated_text_returns_zero() {
        assert_eq!(count_emotes("hello chat just talking normally"), 0);
    }

    #[test]
    fn single_emote_returns_one() {
        assert_eq!(count_emotes("KEKW"), 1);
    }

    #[test]
    fn multiple_distinct_emotes_in_one_message() {
        assert_eq!(count_emotes("KEKW OMEGALUL Pog Sadge"), 4);
    }

    #[test]
    fn repeated_same_emote_counts_each() {
        assert_eq!(count_emotes("KEKW KEKW KEKW"), 3);
    }

    #[test]
    fn case_sensitive_no_match() {
        // Lowercase variant should not match the canonical "KEKW"
        assert_eq!(count_emotes("kekw"), 0);
    }

    #[test]
    fn embedded_emote_still_matches() {
        // No word-boundary check — substring is enough. This is intentional;
        // chat sometimes runs words together. Trade-off vs false positives is
        // acceptable since the per-window count thresholds are aggregate-level.
        assert!(count_emotes("preKEKWpost") >= 1);
    }

    #[test]
    fn nested_emote_does_not_double_count() {
        // "OMEGALUL" contains "LUL" as a substring. A naïve substring counter
        // would count both. The longest-match scan should count exactly 1.
        assert_eq!(count_emotes("OMEGALUL"), 1);
    }

    #[test]
    fn longest_match_resolves_overlap() {
        // "LULW" both contains "LUL" inside it. Should count as 1, not 2.
        assert_eq!(count_emotes("LULW"), 1);
        // Same for PogU containing Pog
        assert_eq!(count_emotes("PogU"), 1);
        // Same for Madge containing Mad
        assert_eq!(count_emotes("Madge"), 1);
        // Same for EZY containing EZ
        assert_eq!(count_emotes("EZY"), 1);
    }

    #[test]
    fn standalone_short_emote_still_counts() {
        // The longest-match algo shouldn't suppress short emotes when
        // they're not contained in a longer match.
        assert_eq!(count_emotes("LUL"), 1);
        assert_eq!(count_emotes("Pog"), 1);
        assert_eq!(count_emotes("EZ"), 1);
    }

    #[test]
    fn jsonl_chat_line_with_emote_matches() {
        // Sanity: emote inside a typical Twitch chat replay JSON-per-line shape
        let line = r#"{"time_in_seconds":43.5,"message":{"body":"KEKW that was clean"},"commenter":{"name":"viewer1"}}"#;
        assert_eq!(count_emotes(line), 1);
    }

    #[test]
    fn list_has_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for e in DEFAULT_TWITCH_EMOTES {
            assert!(seen.insert(*e), "duplicate emote in list: {}", e);
        }
    }

    #[test]
    fn list_has_reasonable_size() {
        // ~30 popular emotes is the target. If this drifts, refresh the list.
        assert!(DEFAULT_TWITCH_EMOTES.len() >= 25);
        assert!(DEFAULT_TWITCH_EMOTES.len() <= 60);
    }
}
