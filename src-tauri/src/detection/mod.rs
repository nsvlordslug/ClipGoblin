//! Detection pipeline — shared types and utilities.
//!
//! This module will grow as Phase 12 / Phase 2 / Phase 3 land. For now it hosts:
//! - [`Platform`] — target social platform (drives hashtag strategy + title length).
//! - [`ranker`] — scoring function for title/caption candidates (BYOK and Free paths).
//!
//! Intentionally NOT touched by Wave 1:
//! - `audio_fingerprint` (Phase 2)
//! - `color_signal` (Phase 3)
//! - Any prompt-emitting code
//!
//! See [`docs/PHASE12_PROMPT_DIFF.md`](../../../docs/PHASE12_PROMPT_DIFF.md) for rollout plan.

pub mod ranker;

// ───────────────────────────────────────────────────────────────────
// Platform — target social platform
// ───────────────────────────────────────────────────────────────────

/// Target social platform for a generated title/caption.
///
/// Drives:
/// - **Hashtag strategy** — different evergreen tags per platform.
/// - **Length scoring** — TikTok/Shorts mobile cuts titles at ~42 chars.
/// - **Tone hints** (future) — Reels tolerates slightly more text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    /// TikTok (primary target).
    TikTok,
    /// YouTube Shorts.
    YouTubeShorts,
    /// Instagram Reels.
    InstagramReels,
    /// Unknown / platform-agnostic fallback.
    Generic,
}

impl Platform {
    /// Preferred title char count. Titles at or under this get full length score;
    /// longer titles get a linear penalty (ranker::length_score).
    ///
    /// Originally set to the feed-view truncation points (~42 for TikTok/Shorts,
    /// ~50 for Reels) but that starved the model on reaction/story clips where
    /// real context requires a full sentence. Raised to 60 across the board:
    /// enough room for a complete sentence, still short enough that the clip-page
    /// view renders cleanly. The feed-view truncation is a separate visual concern
    /// that matters less than whether the title communicates anything at all.
    pub fn title_length_target(self) -> usize {
        match self {
            Platform::TikTok         => 60,
            Platform::YouTubeShorts  => 60,
            Platform::InstagramReels => 60,
            Platform::Generic        => 60,
        }
    }

    /// Evergreen hashtags for this platform — always safe to include.
    /// Returns exactly 3 tags. Caller combines with 2 niche tags (game + content)
    /// to produce the final 5-tag hashtag set.
    pub fn evergreen_hashtags(self) -> &'static [&'static str] {
        match self {
            Platform::TikTok         => &["gaming", "fyp", "gamingtiktok"],
            Platform::YouTubeShorts  => &["gaming", "shorts", "gamingshorts"],
            Platform::InstagramReels => &["gaming", "reels", "gamingreels"],
            Platform::Generic        => &["gaming", "clips", "fyp"],
        }
    }

    /// Human-readable display name for prompt interpolation + UI.
    pub fn display_name(self) -> &'static str {
        match self {
            Platform::TikTok         => "TikTok",
            Platform::YouTubeShorts  => "YouTube Shorts",
            Platform::InstagramReels => "Instagram Reels",
            Platform::Generic        => "a social platform",
        }
    }

    /// Parse from a string representation (case-insensitive).
    /// Returns [`Platform::Generic`] for unknown inputs.
    pub fn from_str_or_generic(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "tiktok"          => Platform::TikTok,
            "youtube_shorts" | "youtubeshorts" | "shorts" => Platform::YouTubeShorts,
            "instagram_reels" | "instagramreels" | "reels" => Platform::InstagramReels,
            _                 => Platform::Generic,
        }
    }
}

impl Default for Platform {
    fn default() -> Self {
        Platform::Generic
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_length_targets() {
        assert_eq!(Platform::TikTok.title_length_target(), 60);
        assert_eq!(Platform::InstagramReels.title_length_target(), 60);
        assert_eq!(Platform::Generic.title_length_target(), 60);
    }

    #[test]
    fn evergreen_hashtags_platform_specific() {
        assert!(Platform::TikTok.evergreen_hashtags().contains(&"fyp"));
        assert!(Platform::YouTubeShorts.evergreen_hashtags().contains(&"shorts"));
        assert!(Platform::InstagramReels.evergreen_hashtags().contains(&"reels"));
    }

    #[test]
    fn evergreen_hashtags_always_include_gaming() {
        for p in [
            Platform::TikTok,
            Platform::YouTubeShorts,
            Platform::InstagramReels,
            Platform::Generic,
        ] {
            assert!(
                p.evergreen_hashtags().contains(&"gaming"),
                "gaming must be evergreen on {:?}",
                p,
            );
        }
    }

    #[test]
    fn evergreen_hashtags_return_exactly_three() {
        for p in [
            Platform::TikTok,
            Platform::YouTubeShorts,
            Platform::InstagramReels,
            Platform::Generic,
        ] {
            assert_eq!(
                p.evergreen_hashtags().len(),
                3,
                "expected 3 evergreen tags for {:?}",
                p,
            );
        }
    }

    #[test]
    fn platform_from_str_case_insensitive() {
        assert_eq!(Platform::from_str_or_generic("TikTok"), Platform::TikTok);
        assert_eq!(Platform::from_str_or_generic("tiktok"), Platform::TikTok);
        assert_eq!(Platform::from_str_or_generic("shorts"), Platform::YouTubeShorts);
        assert_eq!(Platform::from_str_or_generic("reels"), Platform::InstagramReels);
        assert_eq!(Platform::from_str_or_generic("unknown"), Platform::Generic);
    }

    #[test]
    fn platform_default_is_generic() {
        assert_eq!(Platform::default(), Platform::Generic);
    }

    #[test]
    fn platform_display_names_are_human_readable() {
        assert_eq!(Platform::TikTok.display_name(), "TikTok");
        assert_eq!(Platform::YouTubeShorts.display_name(), "YouTube Shorts");
        assert_eq!(Platform::InstagramReels.display_name(), "Instagram Reels");
        assert_eq!(Platform::Generic.display_name(), "a social platform");
    }
}
