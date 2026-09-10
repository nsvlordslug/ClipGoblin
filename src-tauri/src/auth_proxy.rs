use serde::{Deserialize, Serialize};

pub(crate) const PROXY_BASE: &str = "https://clipgoblin-auth-proxy.lordslug.workers.dev";

#[cfg(debug_assertions)]
const SANDBOX_PROXY_ENV: &str = "CLIPGOBLIN_AUTH_PROXY_BASE";
#[cfg(debug_assertions)]
const SANDBOX_PROXY_BASE: &str = "http://127.0.0.1:8788";

#[cfg(debug_assertions)]
fn sandbox_proxy_override<'a>(path: &str, candidate: &'a str) -> Option<&'a str> {
    (path.starts_with("/auth/tiktok/") && candidate == SANDBOX_PROXY_BASE).then_some(candidate)
}

fn proxy_base(_path: &str) -> String {
    #[cfg(debug_assertions)]
    if let Ok(candidate) = std::env::var(SANDBOX_PROXY_ENV) {
        if let Some(proxy) = sandbox_proxy_override(_path, &candidate) {
            return proxy.to_string();
        }
        if _path.starts_with("/auth/tiktok/") {
            log::warn!("Ignoring invalid local auth proxy override");
        }
    }

    PROXY_BASE.to_string()
}

/// Generic token response returned by the auth proxy for all providers.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenResponse {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
    pub token_type: Option<String>,
    pub scope: Option<serde_json::Value>,
    /// TikTok-specific field
    pub open_id: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// Proxy client that forwards OAuth token requests to the Cloudflare Worker,
/// keeping client secrets out of the desktop binary in Steam builds.
pub struct AuthProxy {
    client: reqwest::Client,
}

impl AuthProxy {
    /// Create a client for the OAuth proxy. A desktop binary cannot keep a
    /// shared proxy credential secret, so the Worker validates routes, redirect
    /// URIs, payloads, and rate limits instead.
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .use_native_tls()
            .http1_only()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {e}"))?;
        Ok(Self { client })
    }

    // ── Twitch ──────────────────────────────────────────────

    pub async fn twitch_token_exchange(
        &self,
        code: &str,
        redirect_uri: &str,
    ) -> Result<TokenResponse, String> {
        self.post(
            "/auth/twitch/token",
            serde_json::json!({
                "code": code,
                "redirect_uri": redirect_uri,
            }),
        )
        .await
    }

    pub async fn twitch_refresh(&self, refresh_token: &str) -> Result<TokenResponse, String> {
        self.post(
            "/auth/twitch/refresh",
            serde_json::json!({
                "refresh_token": refresh_token,
            }),
        )
        .await
    }

    // ── YouTube ─────────────────────────────────────────────

    pub async fn youtube_token_exchange(
        &self,
        code: &str,
        redirect_uri: &str,
    ) -> Result<TokenResponse, String> {
        self.post(
            "/auth/youtube/token",
            serde_json::json!({
                "code": code,
                "redirect_uri": redirect_uri,
            }),
        )
        .await
    }

    pub async fn youtube_refresh(&self, refresh_token: &str) -> Result<TokenResponse, String> {
        self.post(
            "/auth/youtube/refresh",
            serde_json::json!({
                "refresh_token": refresh_token,
            }),
        )
        .await
    }

    // ── TikTok ──────────────────────────────────────────────

    pub async fn tiktok_token_exchange(
        &self,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
    ) -> Result<TokenResponse, String> {
        self.post(
            "/auth/tiktok/token",
            serde_json::json!({
                "code": code,
                "redirect_uri": redirect_uri,
                "code_verifier": code_verifier,
            }),
        )
        .await
    }

    pub async fn tiktok_refresh(&self, refresh_token: &str) -> Result<TokenResponse, String> {
        self.post(
            "/auth/tiktok/refresh",
            serde_json::json!({
                "refresh_token": refresh_token,
            }),
        )
        .await
    }

    // ── Internal ────────────────────────────────────────────

    async fn post(&self, path: &str, body: serde_json::Value) -> Result<TokenResponse, String> {
        let url = format!("{}{}", proxy_base(path), path);
        let body_str =
            serde_json::to_string(&body).map_err(|e| format!("Failed to serialize body: {e}"))?;

        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .body(body_str)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response body: {e}"))?;

        if !status.is_success() {
            return Err(format!("Proxy request failed ({}): {}", status, text));
        }

        serde_json::from_str::<TokenResponse>(&text)
            .map_err(|e| format!("Failed to parse proxy response: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_proxy_remains_the_default() {
        assert_eq!(
            PROXY_BASE,
            "https://clipgoblin-auth-proxy.lordslug.workers.dev"
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn sandbox_proxy_is_exact_loopback_only_for_tiktok() {
        assert_eq!(
            sandbox_proxy_override("/auth/tiktok/token", "http://127.0.0.1:8788"),
            Some("http://127.0.0.1:8788")
        );
        assert_eq!(
            sandbox_proxy_override("/auth/tiktok/refresh", "http://127.0.0.1:8788"),
            Some("http://127.0.0.1:8788")
        );
        assert_eq!(
            sandbox_proxy_override("/auth/tiktok/token", "http://localhost:8788"),
            None
        );
        assert_eq!(
            sandbox_proxy_override("/auth/tiktok/token", "http://127.0.0.1:8788/"),
            None
        );
        assert_eq!(
            sandbox_proxy_override("/auth/tiktok/token", "https://127.0.0.1:8788"),
            None
        );
        assert_eq!(
            sandbox_proxy_override("/auth/tiktok/token", "https://example.com"),
            None
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn non_tiktok_routes_never_use_the_sandbox_proxy() {
        for path in [
            "/auth/twitch/token",
            "/auth/twitch/refresh",
            "/auth/youtube/token",
            "/auth/youtube/refresh",
            "/reports/bug",
        ] {
            assert_eq!(sandbox_proxy_override(path, SANDBOX_PROXY_BASE), None);
        }
    }
}
