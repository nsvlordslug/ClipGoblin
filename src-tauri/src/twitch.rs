use serde::Deserialize;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};
use std::sync::Mutex;
use once_cell::sync::Lazy;

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

/// Embedded Twitch Client Secret — compiled into the binary.
/// Override with TWITCH_CLIENT_SECRET env var for development.
const DEFAULT_TWITCH_CLIENT_SECRET: &str = "9vkblt95slzrfinmdplo1y1lmu54i8";

static CLIENT_SECRET: Lazy<String> = Lazy::new(|| {
    match std::env::var("TWITCH_CLIENT_SECRET") {
        Ok(val) if !val.is_empty() => {
            log::info!("Twitch CLIENT_SECRET loaded from env (len={})", val.len());
            val
        }
        _ => {
            log::info!("Using embedded Twitch CLIENT_SECRET");
            DEFAULT_TWITCH_CLIENT_SECRET.to_string()
        }
    }
});

/// Returns the Twitch Client Secret (embedded default or .env override).
pub fn client_secret() -> &'static str {
    &CLIENT_SECRET
}

/// Stores the expected OAuth state to verify on callback.
static OAUTH_STATE: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));

const TWITCH_TOKEN_URL: &str = "https://id.twitch.tv/oauth2/token";
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

/// Refresh an expired access token using a refresh token.
/// Confidential clients send client_secret along with client_id + refresh_token.
pub async fn refresh_access_token(
    refresh_token: &str,
) -> Result<TokenResponse, String> {
    log::info!("[Twitch Refresh] Refreshing token with client_id={}", client_id());

    let client = reqwest::Client::new();
    let resp = client
        .post(TWITCH_TOKEN_URL)
        .form(&[
            ("client_id", client_id()),
            ("client_secret", client_secret()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| {
            log::error!("[Twitch Refresh] HTTP request failed: {}", e);
            format!("Failed to refresh token: {}", e)
        })?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        log::error!("[Twitch Refresh] FAILED ({}): {}", status, body);
        return Err(format!("Token refresh failed ({}): {}", status, body));
    }

    let token = resp.json::<TokenResponse>()
        .await
        .map_err(|e| {
            log::error!("[Twitch Refresh] JSON parse failed: {}", e);
            format!("Failed to parse refreshed token: {}", e)
        })?;

    log::info!("[Twitch Refresh] Success — new token len={}", token.access_token.len());
    Ok(token)
}

/// Exchange an authorization code for an access token (Confidential Client flow).
/// Uses client_secret instead of PKCE code_verifier.
pub async fn exchange_code(
    code: &str,
) -> Result<TokenResponse, String> {
    log::info!("[Twitch Token] POST {} — body: client_id={}&client_secret=***&code={}...&grant_type=authorization_code&redirect_uri={}",
        TWITCH_TOKEN_URL, client_id(), &code[..code.len().min(8)], REDIRECT_URI);

    let client = reqwest::Client::new();
    let resp = client
        .post(TWITCH_TOKEN_URL)
        .form(&[
            ("client_id", client_id()),
            ("client_secret", client_secret()),
            ("code", code),
            ("grant_type", "authorization_code"),
            ("redirect_uri", REDIRECT_URI),
        ])
        .send()
        .await
        .map_err(|e| {
            log::error!("[Twitch Token] HTTP request failed: {}", e);
            format!("Failed to exchange code: {}", e)
        })?;

    let status = resp.status();
    log::info!("[Twitch Token] Response status: {}", status);

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        log::error!("[Twitch Token] Exchange FAILED ({}): {}", status, body);
        return Err(format!("Token exchange failed ({}): {}", status, body));
    }

    let body_text = resp.text().await.map_err(|e| {
        log::error!("[Twitch Token] Failed to read response body: {}", e);
        format!("Failed to read token response: {}", e)
    })?;

    log::info!("[Twitch Token] Exchange succeeded, parsing JSON (len={})", body_text.len());

    let token: TokenResponse = serde_json::from_str(&body_text).map_err(|e| {
        log::error!("[Twitch Token] JSON parse failed: {} — body: {}", e, &body_text[..body_text.len().min(500)]);
        format!("Failed to parse token: {}", e)
    })?;

    log::info!("[Twitch Token] Got access_token (len={}), refresh_token={}",
        token.access_token.len(),
        if token.refresh_token.is_some() { "present" } else { "MISSING" });

    Ok(token)
}

/// Get an app access token using the client credentials flow.
/// Note: client_credentials is for server-side app tokens (no user context).
/// User tokens are obtained via exchange_code() instead.
#[allow(dead_code)]
pub async fn get_app_access_token(client_secret: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(TWITCH_TOKEN_URL)
        .form(&[
            ("client_id", client_id()),
            ("client_secret", client_secret),
            ("grant_type", "client_credentials"),
        ])
        .send()
        .await
        .map_err(|e| format!("Failed to request token: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Token request failed ({}): {}", status, body));
    }

    let token_resp: TokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {}", e))?;

    Ok(token_resp.access_token)
}

/// Get the authenticated user's info using their user access token.
pub async fn get_authenticated_user(
    access_token: &str,
) -> Result<TwitchUser, String> {
    log::info!("[Twitch User] Fetching user info with token (len={}), client_id={}", access_token.len(), client_id());

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/users", TWITCH_API_URL))
        .header("Client-Id", client_id())
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| {
            log::error!("[Twitch User] HTTP request failed: {}", e);
            format!("Failed to fetch user: {}", e)
        })?;

    let status = resp.status();
    log::info!("[Twitch User] Response status: {}", status);

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        log::error!("[Twitch User] Fetch FAILED ({}): {}", status, body);
        return Err(format!("User lookup failed ({}): {}", status, body));
    }

    let users_resp: UsersResponse = resp
        .json()
        .await
        .map_err(|e| {
            log::error!("[Twitch User] JSON parse failed: {}", e);
            format!("Failed to parse user response: {}", e)
        })?;

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
    let client = reqwest::Client::new();
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

        let resp = client
            .get(&url)
            .header("Client-Id", client_id())
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
            .map_err(|e| format!("Failed to fetch VODs: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("VODs request failed ({}): {}", status, body));
        }

        let videos_resp: VideosResponse = resp
            .json()
            .await
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
    let client = reqwest::Client::new();
    let url = format!("{}/channels?broadcaster_id={}", TWITCH_API_URL, broadcaster_id);

    let resp = client
        .get(&url)
        .header("Client-Id", client_id())
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch channel info: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Channel info request failed ({}): {}", status, body));
    }

    let info: ChannelInfoResponse = resp
        .json()
        .await
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
