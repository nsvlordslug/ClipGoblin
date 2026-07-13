//! AI provider abstraction.
//!
//! Reads the user's AI settings from the DB and routes generation
//! requests to the appropriate backend: Free (local/offline),
//! OpenAI, Claude, or Gemini.
//!
//! This module does NOT make API calls itself — it provides the
//! configuration that callers need to make the right call.

use crate::db;

// ═══════════════════════════════════════════════════════════════════
//  Provider enum
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Free,
    OpenAI,
    Claude,
    Gemini,
}

impl Provider {
    fn from_str(s: &str) -> Self {
        match s {
            "openai" => Self::OpenAI,
            "claude" => Self::Claude,
            "gemini" => Self::Gemini,
            _ => Self::Free,
        }
    }

    /// Stable lowercase identifier used as a foreign-key value in the
    /// `ai_usage_log` table.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::OpenAI => "openai",
            Self::Claude => "claude",
            Self::Gemini => "gemini",
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Generation scope
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy)]
pub enum Scope {
    /// Title generation for clips.
    Titles,
    /// TikTok caption generation.
    Captions,
    /// AI clip-worthiness judge (detection). Enablement is gated by the separate
    /// `ai_clip_detection_enabled` setting, so this scope always resolves the
    /// configured provider when a key exists.
    ClipJudge,
}

// ═══════════════════════════════════════════════════════════════════
//  Resolved provider config — what a caller needs to make the call
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct ResolvedProvider {
    /// Which provider to use for this scope.
    pub provider: Provider,
    /// API key (empty for Free).
    pub api_key: String,
    /// Model name (empty for Free).
    pub model: String,
    /// Whether to fall back to Free if the API call fails.
    pub fallback_to_free: bool,
    /// Whether the clip judge should run a Sonnet final-pass over the top
    /// Haiku-ranked moments (Claude only). Carries the `useSonnetFinalPass`
    /// setting so the `ClipJudge` caller doesn't re-read the DB. Always `false`
    /// for non-`ClipJudge` scopes and for Free.
    pub use_sonnet_final_pass: bool,
}

impl ResolvedProvider {
    /// Whether this resolution points to an LLM (not Free).
    pub fn is_llm(&self) -> bool {
        self.provider != Provider::Free && !self.api_key.is_empty()
    }

    /// Free mode — no API calls.
    pub fn free() -> Self {
        Self {
            provider: Provider::Free,
            api_key: String::new(),
            model: String::new(),
            fallback_to_free: true,
            use_sonnet_final_pass: false,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Settings structure (matches frontend AiSettings JSON)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AiSettings {
    #[serde(default = "default_provider")]
    provider: String,

    #[serde(default)]
    openai_api_key: String,
    #[serde(default = "default_openai_model")]
    openai_model: String,

    #[serde(default)]
    claude_api_key: String,
    #[serde(default = "default_claude_model")]
    claude_model: String,
    /// Optional override for the clip-judge model (Claude). When unset, the
    /// `ClipJudge` scope defaults to Sonnet (`default_claude_judge_model`) for
    /// quality — independent of `claude_model`, which still drives titles/captions.
    /// Cost-conscious users can opt into Haiku here (economy).
    #[serde(default)]
    claude_judge_model: Option<String>,
    /// Run a single Sonnet final-pass over the top Haiku-ranked moments to
    /// pick/re-order the final clip set (Claude judge only). Paid and opt-in.
    #[serde(default)]
    use_sonnet_final_pass: bool,

    #[serde(default)]
    gemini_api_key: String,
    #[serde(default = "default_gemini_model")]
    gemini_model: String,

    /// Ignored — analysis always runs Free. Kept for backward compat with old DB values.
    #[serde(default = "default_true")]
    #[allow(dead_code)]
    use_for_analysis: bool,
    #[serde(default = "default_true")]
    use_for_titles: bool,
    #[serde(default = "default_true")]
    use_for_captions: bool,
    #[serde(default = "default_true")]
    fallback_to_free: bool,
}

fn default_provider() -> String { "free".into() }
fn default_openai_model() -> String { "gpt-4o-mini".into() }
fn default_claude_model() -> String { "claude-sonnet-4-6".into() }
/// Default model for the clip-worthiness JUDGE — Sonnet, the quality default.
/// Haiku measurably hurt clip quality on banter/comedy content, so Sonnet is the
/// out-of-the-box judge; Haiku stays available as an opt-in "economy" choice.
/// Titles/captions still use `claude_model`.
fn default_claude_judge_model() -> String { "claude-sonnet-4-6".into() }
/// Model for the (optional) judge final-pass — a single Sonnet call over only
/// the top survivors for taste. Kept separate from the bulk judge model.
fn default_claude_final_pass_model() -> String { "claude-sonnet-4-6".into() }
fn default_gemini_model() -> String { "gemini-2.5-flash".into() }
fn default_true() -> bool { true }

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            openai_api_key: String::new(),
            openai_model: default_openai_model(),
            claude_api_key: String::new(),
            claude_model: default_claude_model(),
            claude_judge_model: None,
            use_sonnet_final_pass: false,
            gemini_api_key: String::new(),
            gemini_model: default_gemini_model(),
            use_for_analysis: true,
            use_for_titles: true,
            use_for_captions: true,
            fallback_to_free: true,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Public API: resolve which provider to use for a given scope
// ═══════════════════════════════════════════════════════════════════

/// Read AI settings from the DB and resolve the provider for a scope.
///
/// Returns `ResolvedProvider::free()` if:
///   - provider is "free"
///   - the scope is disabled for the selected provider
///   - the settings JSON is missing/corrupt
///   - the selected provider has no API key
///
/// The caller should check `resolved.is_llm()` before making API calls.
pub fn resolve(conn: &rusqlite::Connection, scope: Scope) -> ResolvedProvider {
    let settings = load_settings(conn);
    let provider = Provider::from_str(&settings.provider);

    // Free mode — no further checks needed
    if provider == Provider::Free {
        return ResolvedProvider::free();
    }

    // Check if the scope is enabled for this provider
    let scope_enabled = match scope {
        Scope::Titles   => settings.use_for_titles,
        Scope::Captions => settings.use_for_captions,
        Scope::ClipJudge => true, // gated by `ai_clip_detection_enabled`, not a per-provider toggle
    };

    if !scope_enabled {
        return ResolvedProvider::free();
    }

    // Get the key + model for the selected provider.
    // The JUDGE scope on Claude defaults to Sonnet (quality) and honors
    // an optional `claudeJudgeModel` override — titles/captions keep following
    // `claude_model`.
    let (api_key, model) = match provider {
        Provider::OpenAI => (settings.openai_api_key.clone(), settings.openai_model.clone()),
        Provider::Claude => {
            let model = match scope {
                Scope::ClipJudge => settings
                    .claude_judge_model
                    .clone()
                    .filter(|m| !m.trim().is_empty())
                    .unwrap_or_else(default_claude_judge_model),
                _ => settings.claude_model.clone(),
            };
            (settings.claude_api_key.clone(), model)
        }
        Provider::Gemini => (settings.gemini_api_key.clone(), settings.gemini_model.clone()),
        Provider::Free   => unreachable!(),
    };

    // If no key is configured, resolve to Free (don't hard fail)
    if api_key.is_empty() {
        log::info!("AI provider {:?} selected but no key configured — using Free mode for {:?}", provider, scope);
        return ResolvedProvider::free();
    }

    // The Sonnet final-pass only applies to the Claude clip judge. It exists to
    // add Sonnet taste on top of a CHEAPER bulk judge (e.g. Haiku); when the judge
    // model is ALREADY the final-pass model (Sonnet), the extra pass would just run
    // Sonnet twice for no benefit — so disable it in that case.
    let judge_is_final_pass_model =
        model.trim() == default_claude_final_pass_model().trim();
    let use_sonnet_final_pass = matches!(scope, Scope::ClipJudge)
        && provider == Provider::Claude
        && settings.use_sonnet_final_pass
        && !judge_is_final_pass_model;

    ResolvedProvider {
        provider,
        api_key,
        model,
        fallback_to_free: settings.fallback_to_free,
        use_sonnet_final_pass,
    }
}

/// The model used for the judge's Sonnet final-pass. Exposed so the caller
/// (vod.rs) logs the right model for the `clip_judge_final` usage row.
pub fn final_pass_model() -> String {
    default_claude_final_pass_model()
}

/// Load settings from the DB.  Returns defaults on any error.
fn load_settings(conn: &rusqlite::Connection) -> AiSettings {
    // Try new ai_settings JSON blob first
    if let Ok(Some(json)) = db::get_setting(conn, "ai_settings") {
        if let Ok(s) = serde_json::from_str::<AiSettings>(&json) {
            return s;
        }
    }

    // Fallback: check legacy claude_api_key
    if let Ok(Some(key)) = db::get_setting(conn, "claude_api_key") {
        if !key.is_empty() {
            return AiSettings {
                provider: "claude".into(),
                claude_api_key: key,
                ..AiSettings::default()
            };
        }
    }

    AiSettings::default()
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_from_str_defaults_to_free() {
        assert_eq!(Provider::from_str("free"), Provider::Free);
        assert_eq!(Provider::from_str(""), Provider::Free);
        assert_eq!(Provider::from_str("invalid"), Provider::Free);
    }

    #[test]
    fn provider_from_str_parses_known() {
        assert_eq!(Provider::from_str("openai"), Provider::OpenAI);
        assert_eq!(Provider::from_str("claude"), Provider::Claude);
        assert_eq!(Provider::from_str("gemini"), Provider::Gemini);
    }

    #[test]
    fn resolved_free_is_not_llm() {
        assert!(!ResolvedProvider::free().is_llm());
    }

    #[test]
    fn resolved_with_key_is_llm() {
        let r = ResolvedProvider {
            provider: Provider::Claude,
            api_key: "sk-test".into(),
            model: "claude-sonnet-4-6".into(),
            fallback_to_free: true,
            use_sonnet_final_pass: false,
        };
        assert!(r.is_llm());
    }

    #[test]
    fn resolved_without_key_is_not_llm() {
        let r = ResolvedProvider {
            provider: Provider::Claude,
            api_key: String::new(),
            model: "claude-sonnet-4-6".into(),
            fallback_to_free: true,
            use_sonnet_final_pass: false,
        };
        assert!(!r.is_llm());
    }

    #[test]
    fn default_settings_are_free() {
        let s = AiSettings::default();
        assert_eq!(s.provider, "free");
        assert!(s.use_for_analysis);
        assert!(s.use_for_titles);
        assert!(s.use_for_captions);
        assert!(s.fallback_to_free);
    }

    #[test]
    fn settings_deserialize_from_json() {
        let json = r#"{"provider":"claude","claudeApiKey":"sk-ant-test","claudeModel":"claude-sonnet-4-6","useForAnalysis":true,"useForTitles":false,"useForCaptions":true,"fallbackToFree":true}"#;
        let s: AiSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.provider, "claude");
        assert_eq!(s.claude_api_key, "sk-ant-test");
        assert!(s.use_for_analysis);
        assert!(!s.use_for_titles);
        assert!(s.use_for_captions);
    }

    #[test]
    fn settings_deserialize_partial_uses_defaults() {
        let json = r#"{"provider":"openai","openaiApiKey":"sk-test"}"#;
        let s: AiSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.provider, "openai");
        assert_eq!(s.openai_api_key, "sk-test");
        assert_eq!(s.openai_model, "gpt-4o-mini"); // default
        assert!(s.use_for_analysis); // default
        assert!(s.fallback_to_free); // default
    }

    #[test]
    fn judge_model_defaults_to_sonnet_when_unset() {
        // No claudeJudgeModel set → judge defaults to Sonnet (quality), independent of claudeModel.
        let json = r#"{"provider":"claude","claudeApiKey":"sk-ant","claudeModel":"claude-sonnet-4-6"}"#;
        let s: AiSettings = serde_json::from_str(json).unwrap();
        assert!(s.claude_judge_model.is_none());
        assert_eq!(s.claude_model, "claude-sonnet-4-6"); // titles/captions unchanged
        let judge = s
            .claude_judge_model
            .clone()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(default_claude_judge_model);
        assert_eq!(judge, "claude-sonnet-4-6");
        assert!(!s.use_sonnet_final_pass); // paid final pass is opt-in
    }

    #[test]
    fn judge_model_override_is_honored() {
        let json = r#"{"provider":"claude","claudeApiKey":"sk-ant","claudeJudgeModel":"claude-sonnet-4-6","useSonnetFinalPass":false}"#;
        let s: AiSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.claude_judge_model.as_deref(), Some("claude-sonnet-4-6"));
        assert!(!s.use_sonnet_final_pass);
    }

    #[test]
    fn final_pass_model_is_sonnet() {
        assert!(final_pass_model().contains("sonnet"));
    }
}
