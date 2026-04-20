use serde::Deserialize;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};
use std::sync::Mutex;
use once_cell::sync::Lazy;
use crate::auth_proxy::AuthProxy;

/// Embedded Twitch Client ID — compiled into the binary.
/// Override with TWITCH_CLIENT_ID env var for development.
const DEFAULT_TWITCH_CLIENT_ID: &str = "i734ser5qcdf8grvlllx6yzprwegfr";

static CLIENT_ID: Lazy<String> = Lazy::new(|| {
    match std::env::var("TWITCH_CLIENT_ID") {
        Ok(val) if !val.is_empty() => {
            log::info!("Twitch CLIENT_ID loaded from env (len={})", val.len());
            val
        }
        _ => {
            log::info!("Using embedded Twitch CLIENT_ID");
            DEFAULT_TWITCH_CLIENT_ID.to_string()
        }
    }
});

/// Returns the Twitch Client ID (embedded default or .env override).
pub fn client_id() -> &'static str {
    &CLIENT_ID
}

/// Stores the expected OAuth state to verify on callback.
static OAUTH_STATE: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));

const TWITCH_API_URL: &str = "https://api.twitch.tv/helix";
const REDIRECT_URI: &str = "http://localhost:17385";

#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[allow(dead_code)]
    pub expires_in: u64,
    #[allow(dead_code)]
    pub token_type: String,
    pub refresh_token: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct TwitchUser {
    pub id: String,
    pub login: String,
    pub display_name: String,
    pub profile_image_url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct UsersResponse {
    data: Vec<TwitchUser>,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct TwitchVideo {
    pub id: String,
    pub user_id: String,
    pub title: String,
    pub duration: String,
    pub created_at: String,
    pub thumbnail_url: String,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Pagination {
    cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct VideosResponse {
    data: Vec<TwitchVideo>,
    pagination: Option<Pagination>,
}

/// Build the Twitch OAuth authorization URL with CSRF state.
/// Uses the embedded client_id (confidential client with client_secret).
pub fn get_auth_url() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    // Generate a random-ish state token for CSRF protection
    let mut hasher = DefaultHasher::new();
    Instant::now().hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    let state = format!("{:x}", hasher.finish());

    // Store state for verification on callback
    if let Ok(mut guard) = OAUTH_STATE.lock() {
        *guard = state.clone();
    }

    let url = format!(
        "https://id.twitch.tv/oauth2/authorize?client_id={}&redirect_uri={}&response_type=code&scope=user:read:email+user:read:broadcast&force_verify=true&state={}",
        client_id(),
        urlencoding::encode(REDIRECT_URI),
        state,
    );

    log::info!("[Twitch Auth] Auth URL built: client_id={}, redirect_uri={}", client_id(), REDIRECT_URI);

    url
}

/// Bind the callback server on port 17385 (matches http://localhost:17385 redirect URI).
pub fn bind_callback_server() -> Result<TcpListener, String> {
    TcpListener::bind("127.0.0.1:17385")
        .or_else(|_| TcpListener::bind("[::1]:17385"))
        .map_err(|e| format!("Failed to bind callback server on port 17385: {}", e))
}

/// Wait for the OAuth callback on an already-bound listener.
/// Times out after 2 minutes.
pub fn wait_for_auth_code(listener: TcpListener) -> Result<String, String> {
    // Use non-blocking mode so we can implement a timeout
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("Failed to configure server: {}", e))?;

    let deadline = Instant::now() + Duration::from_secs(120);

    loop {
        if Instant::now() > deadline {
            return Err("Login timed out after 2 minutes. Please try again.".to_string());
        }

        let (stream, _) = match listener.accept() {
            Ok(conn) => conn,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(e) => return Err(format!("Server error: {}", e)),
        };

        stream.set_nonblocking(false).ok();

        let mut reader = BufReader::new(&stream);
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() {
            continue;
        }

        let path = request_line.split_whitespace().nth(1).unwrap_or("");

        // Skip requests that aren't our callback (e.g. /favicon.ico)
        // With http://localhost redirect, Twitch redirects to /?code=XXX
        if !path.starts_with("/?") && path != "/" {
            let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            if let Ok(mut w) = stream.try_clone() {
                w.write_all(resp.as_bytes()).ok();
                w.flush().ok();
            }
            continue;
        }

        // Parse query parameters
        let query = path.split('?').nth(1).unwrap_or("");
        let params: Vec<(&str, &str)> = query
            .split('&')
            .filter_map(|p| {
                let mut kv = p.splitn(2, '=');
                Some((kv.next()?, kv.next().unwrap_or("")))
            })
            .collect();

        let find = |key: &str| params.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);

        // Handle Twitch error callback (e.g. user denied authorization)
        if let Some(_error) = find("error") {
            let desc = find("error_description").unwrap_or("Authorization+denied");
            let desc_decoded = urlencoding::decode(desc)
                .map(|s| s.into_owned())
                .unwrap_or_else(|_| desc.to_string());
            // HTML-escape to prevent XSS via crafted error_description
            let desc_safe = desc_decoded
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&#x27;");
            send_html_response(
                &stream,
                &format!(
                    "<h1 style=\"color:#ef4444\">&#10008; Login Failed</h1><p>{}</p>",
                    desc_safe
                ),
            );
            return Err(format!("Twitch authorization denied: {}", desc_decoded));
        }

        // Extract auth code — verify state parameter to prevent CSRF
        if let Some(code) = find("code") {
            let callback_state = find("state").unwrap_or("");
            let expected_state = OAUTH_STATE.lock().map(|g| g.clone()).unwrap_or_default();
            if callback_state.is_empty() || callback_state != expected_state {
                send_html_response(
                    &stream,
                    "<h1 style=\"color:#ef4444\">&#10008; Error</h1><p>Invalid OAuth state. Please try logging in again.</p>",
                );
                return Err("OAuth state mismatch — possible CSRF. Please try again.".to_string());
            }
            send_html_response(
                &stream,
                "<h1 style=\"color:#8b5cf6\">&#10004; Logged in!</h1><p>You can close this tab and return to ClipGoblin.</p>",
            );
            return Ok(code.to_string());
        }

        // Callback path but no code or error
        send_html_response(
            &stream,
            "<h1 style=\"color:#ef4444\">&#10008; Error</h1><p>No authorization code received.</p>",
        );
        return Err("Twitch callback did not contain an authorization code.".to_string());
    }
}

fn send_html_response(stream: &std::net::TcpStream, body_content: &str) {
    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head><title>ClipGoblin</title></head>
<body style="background:#0f0a1a;color:#e2e8f0;font-family:system-ui;display:flex;align-items:center;justify-content:center;height:100vh;margin:0">
<div style="text-align:center">{}</div>
</body>
</html>"#,
        body_content
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    if let Ok(mut w) = stream.try_clone() {
        w.write_all(response.as_bytes()).ok();
        w.flush().ok();
    }
}

/// Refresh an expired access token using the auth proxy.
pub async fn refresh_access_token(
    refresh_token: &str,
) -> Result<TokenResponse, String> {
    log::info!("[Twitch Refresh] Refreshing token via auth proxy");

    let proxy = AuthProxy::new()?;
    let proxy_resp = proxy.twitch_refresh(refresh_token).await?;

    // Check for error in proxy response
    if let Some(err) = proxy_resp.error {
        let desc = proxy_resp.error_description.unwrap_or_default();
        log::error!("[Twitch Refresh] Proxy returned error: {} — {}", err, desc);
        return Err(format!("Token refresh failed: {} — {}", err, desc));
    }

    let access_token = proxy_resp.access_token
        .ok_or_else(|| "Proxy response missing access_token".to_string())?;

    log::info!("[Twitch Refresh] Success — new token len={}", access_token.len());

    Ok(TokenResponse {
        access_token,
        expires_in: proxy_resp.expires_in.unwrap_or(0),
        token_type: proxy_resp.token_type.unwrap_or_else(|| "bearer".to_string()),
        refresh_token: proxy_resp.refresh_token,
    })
}

/// Exchange an authorization code for an access token via the auth proxy.
pub async fn exchange_code(
    code: &str,
) -> Result<TokenResponse, String> {
    log::info!("[Twitch Token] Exchanging code via auth proxy (code={}..., redirect_uri={})",
        &code[..code.len().min(8)], REDIRECT_URI);

    let proxy = AuthProxy::new()?;
    let proxy_resp = proxy.twitch_token_exchange(code, REDIRECT_URI).await?;

    // Check for error in proxy response
    if let Some(err) = proxy_resp.error {
        let desc = proxy_resp.error_description.unwrap_or_default();
        log::error!("[Twitch Token] Proxy returned error: {} — {}", err, desc);
        return Err(format!("Token exchange failed: {} — {}", err, desc));
    }

    let access_token = proxy_resp.access_token
        .ok_or_else(|| "Proxy response missing access_token".to_string())?;

    log::info!("[Twitch Token] Got access_token (len={}), refresh_token={}",
        access_token.len(),
        if proxy_resp.refresh_token.is_some() { "present" } else { "MISSING" });

    Ok(TokenResponse {
        access_token,
        expires_in: proxy_resp.expires_in.unwrap_or(0),
        token_type: proxy_resp.token_type.unwrap_or_else(|| "bearer".to_string()),
        refresh_token: proxy_resp.refresh_token,
    })
}

/// Get an app access token using the client credentials flow.
/// Note: client_credentials is for server-side app tokens (no user context).
/// This flow is not supported through the auth proxy — kept as a stub.
#[allow(dead_code)]
pub async fn get_app_access_token(_client_secret: &str) -> Result<String, String> {
    Err("App access tokens (client_credentials) are not supported via the auth proxy. Use exchange_code() for user tokens.".to_string())
}

/// Helper: run a curl GET request against the Twitch Helix API and return the body as a string.
pub async fn curl_twitch_get(url: &str, access_token: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .use_native_tls()
        .http1_only()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let resp = client
        .get(url)
        .header("Client-Id", client_id())
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch: {}", e))?;

    let status = resp.status();
    let text = resp.text().await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    if !status.is_success() {
        return Err(format!("Twitch API request failed ({}): {}", status, text));
    }

    Ok(text)
}

/// Get the authenticated user's info using their user access token.
pub async fn get_authenticated_user(
    access_token: &str,
) -> Result<TwitchUser, String> {
    log::info!("[Twitch User] Fetching user info with token (len={}), client_id={}", access_token.len(), client_id());

    let url = format!("{}/users", TWITCH_API_URL);
    let body = curl_twitch_get(&url, access_token).await.map_err(|e| {
        log::error!("[Twitch User] HTTP request failed: {}", e);
        format!("Failed to fetch user: {}", e)
    })?;

    log::info!("[Twitch User] Response status: 200 OK");

    let users_resp: UsersResponse = serde_json::from_str(&body)
        .map_err(|e| {
            log::error!("[Twitch User] JSON parse failed: {}", e);
            format!("Failed to parse user response: {}", e)
        })?;

    // Check for API error in response
    if let Ok(err_val) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(status) = err_val.get("status") {
            let msg = err_val.get("message").and_then(|m| m.as_str()).unwrap_or("");
            log::error!("[Twitch User] Fetch FAILED ({}): {}", status, msg);
            return Err(format!("User lookup failed ({}): {}", status, msg));
        }
    }

    match users_resp.data.into_iter().next() {
        Some(user) => {
            log::info!("[Twitch User] Success: id={}, login={}, display_name={}", user.id, user.login, user.display_name);
            Ok(user)
        }
        None => {
            log::error!("[Twitch User] API returned empty data array — no user found");
            Err("No user returned".to_string())
        }
    }
}

/// Get VODs (archive videos) for a user. Paginates to fetch all available VODs.
pub async fn get_vods(
    access_token: &str,
    user_id: &str,
) -> Result<Vec<TwitchVideo>, String> {
    let mut all_videos = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let mut url = format!(
            "{}/videos?user_id={}&type=archive&first=100",
            TWITCH_API_URL, user_id
        );
        if let Some(ref c) = cursor {
            url.push_str(&format!("&after={}", c));
        }

        let body = curl_twitch_get(&url, access_token).await?;

        // Check for API error
        if let Ok(err_val) = serde_json::from_str::<serde_json::Value>(&body) {
            if let Some(status) = err_val.get("status") {
                let msg = err_val.get("message").and_then(|m| m.as_str()).unwrap_or("");
                return Err(format!("VODs request failed ({}): {}", status, msg));
            }
        }

        let videos_resp: VideosResponse = serde_json::from_str(&body)
            .map_err(|e| format!("Failed to parse VODs response: {}", e))?;

        let page_empty = videos_resp.data.is_empty();
        all_videos.extend(videos_resp.data);

        // Check for next page
        match videos_resp.pagination.and_then(|p| p.cursor) {
            Some(c) if !page_empty => cursor = Some(c),
            _ => break,
        }
    }

    Ok(all_videos)
}

/// Response from Twitch "Get Channel Information" endpoint.
/// Currently unused — kept for potential future use (e.g. if Twitch adds per-VOD game data).
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ChannelInfo {
    pub broadcaster_id: String,
    pub game_name: String,
    pub game_id: String,
    pub broadcaster_name: String,
    #[serde(default)]
    pub title: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct ChannelInfoResponse {
    data: Vec<ChannelInfo>,
}

#[allow(dead_code)]
/// Fetch channel information including current game using the Helix "Get Channel Information" endpoint.
/// This reliably returns game_name/game_id (unlike /videos which does not).
pub async fn get_channel_info(
    access_token: &str,
    broadcaster_id: &str,
) -> Result<Option<ChannelInfo>, String> {
    let url = format!("{}/channels?broadcaster_id={}", TWITCH_API_URL, broadcaster_id);

    let body = curl_twitch_get(&url, access_token).await?;

    let info: ChannelInfoResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse channel info: {}", e))?;

    Ok(info.data.into_iter().next())
}

/// Parse a Twitch duration string like "3h25m12s" into total seconds.
pub fn parse_duration(duration: &str) -> i64 {
    let mut total: i64 = 0;
    let mut current_num = String::new();

    for ch in duration.chars() {
        if ch.is_ascii_digit() {
            current_num.push(ch);
        } else {
            let n: i64 = current_num.parse().unwrap_or(0);
            current_num.clear();
            match ch {
                'h' => total += n * 3600,
                'm' => total += n * 60,
                's' => total += n,
                _ => {}
            }
        }
    }

    total
}

// ── Community clips ────────────────────────────────────────────────────

/// Raw clip record from the `/helix/clips` endpoint. Only the fields we need.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TwitchCommunityClip {
    /// VOD the clip was cut from. Some clips aren't tied to a VOD (null).
    #[serde(default)]
    pub video_id: Option<String>,
    /// Seconds from the VOD start where the clip begins. Can be null for
    /// clips made from old VODs or during live streams before the VOD rendered.
    #[serde(default)]
    pub vod_offset: Option<i64>,
    /// Clip length in seconds (float — e.g. 27.6).
    #[serde(default)]
    pub duration: f64,
    /// How many times the clip was viewed — our strongest quality signal.
    #[serde(default)]
    pub view_count: i64,
    #[serde(default)]
    pub title: String,
}

/// Fetch every community-created clip cut from the given broadcaster between
/// `started_at` and `ended_at` (RFC3339 timestamps). Paginates up to ~600
/// clips (6 pages × 100) — more than enough for a single streaming session.
///
/// Returns only clips that are tied to a specific VOD (`video_id` set) and
/// that have a `vod_offset` — clips without offsets can't be mapped to a
/// timeline position.
///
/// Uses the user's existing access token — no additional scope required.
/// The `/helix/clips` endpoint is accessible with any authenticated call.
pub async fn fetch_community_clips(
    access_token: &str,
    broadcaster_id: &str,
    started_at: &str,
    ended_at: &str,
) -> Result<Vec<TwitchCommunityClip>, String> {
    let mut all: Vec<TwitchCommunityClip> = Vec::new();
    let mut cursor: Option<String> = None;
    let max_pages = 6;

    for _ in 0..max_pages {
        let mut url = format!(
            "https://api.twitch.tv/helix/clips?broadcaster_id={}&started_at={}&ended_at={}&first=100",
            broadcaster_id, started_at, ended_at
        );
        if let Some(c) = &cursor {
            url.push_str(&format!("&after={}", c));
        }

        let body = curl_twitch_get(&url, access_token).await
            .map_err(|e| format!("helix/clips: {}", e))?;

        let resp: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| format!("helix/clips parse: {}", e))?;

        if let Some(status) = resp.get("status") {
            let msg = resp.get("message").and_then(|m| m.as_str()).unwrap_or("");
            return Err(format!("Twitch API {}: {}", status, msg));
        }

        if let Some(arr) = resp["data"].as_array() {
            for v in arr {
                if let Ok(clip) = serde_json::from_value::<TwitchCommunityClip>(v.clone()) {
                    // Keep only VOD-anchored clips with a resolvable timeline position.
                    if clip.video_id.is_some() && clip.vod_offset.is_some() {
                        all.push(clip);
                    }
                }
            }
        }

        cursor = resp["pagination"]["cursor"].as_str().map(|s| s.to_string());
        if cursor.is_none() || cursor.as_deref() == Some("") { break; }
    }

    Ok(all)
}
