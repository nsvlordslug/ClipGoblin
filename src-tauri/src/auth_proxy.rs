use serde::{Deserialize, Serialize};

const PROXY_BASE: &str = "https://clipgoblin-auth-proxy.lordslug.workers.dev";

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
    api_key: String,
}

impl AuthProxy {
    /// Create a new proxy client. Reads `PROXY_API_KEY` from the environment.
    pub fn new() -> Result<Self, String> {
        let api_key = option_env!("PROXY_API_KEY")
            .map(|s| s.to_string())
            .or_else(|| std::env::var("PROXY_API_KEY").ok())
            .ok_or_else(|| "PROXY_API_KEY not set at build or runtime".to_string())?;
        let client = reqwest::Client::builder()
            .use_native_tls()
            .http1_only()
            .timeout(std::time::Duration::from_secs(15))
            .build()            .map_err(|e| format!("Failed to build HTTP client: {e}"))?;
        Ok(Self { client, api_key })
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

    async fn post(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<TokenResponse, String> {
        let url = format!("{}{}", PROXY_BASE, path);
        let body_str = serde_json::to_string(&body)
            .map_err(|e| format!("Failed to serialize body: {e}"))?;

        let resp = self.client
            .post(&url)
            .header("X-Proxy-Key", &self.api_key)
            .header("Content-Type", "application/json")
            .body(body_str)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?;

        let status = resp.status();
        let text = resp.text().await
            .map_err(|e| format!("Failed to read response body: {e}"))?;

        if !status.is_success() {
            return Err(format!("Proxy request failed ({}): {}", status, text));
        }

        serde_json::from_str::<TokenResponse>(&text)
            .map_err(|e| format!("Failed to parse proxy response: {e}"))
    }
}
