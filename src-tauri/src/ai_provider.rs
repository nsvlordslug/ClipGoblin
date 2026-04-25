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
    };

    if !scope_enabled {
        return ResolvedProvider::free();
    }

    // Get the key + model for the selected provider
    let (api_key, model) = match provider {
        Provider::OpenAI => (settings.openai_api_key.clone(), settings.openai_model.clone()),
        Provider::Claude => (settings.claude_api_key.clone(), settings.claude_model.clone()),
        Provider::Gemini => (settings.gemini_api_key.clone(), settings.gemini_model.clone()),
        Provider::Free   => unreachable!(),
    };

    // If no key is configured, resolve to Free (don't hard fail)
    if api_key.is_empty() {
        log::info!("AI provider {:?} selected but no key configured — using Free mode for {:?}", provider, scope);
        return ResolvedProvider::free();
    }

    ResolvedProvider {
        provider,
        api_key,
        model,
        fallback_to_free: settings.fallback_to_free,
    }
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
}
