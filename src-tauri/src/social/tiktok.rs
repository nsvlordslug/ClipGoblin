//! TikTok platform adapter — OAuth 2.0 (Login Kit) + Content Posting API.
//!
//! Implements the full `PlatformAdapter` trait for TikTok:
//! - TikTok OAuth 2.0 authorization code flow (Login Kit)
//! - User info fetch via /v2/user/info/
//! - Video upload via Content Posting API (init → upload → publish)
//! - Automatic token refresh via refresh_token grant
//!
//! Uses Sandbox mode by default — suitable for testing before production approval.

use crate::auth_proxy::AuthProxy;
use crate::db;
use crate::error::AppError;
use crate::social::{
    validate_export_file, ConnectedAccount, PlatformAdapter, UploadMeta, UploadResult,
    UploadResultStatus,
};
use once_cell::sync::Lazy;
use rusqlite::Connection;
use serde::Deserialize;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::{Duration, Instant};

// ═══════════════════════════════════════════════════════════════════
//  Constants
// ═══════════════════════════════════════════════════════════════════

const TIKTOK_AUTH_URL: &str = "https://www.tiktok.com/v2/auth/authorize/";
const TIKTOK_USERINFO_URL: &str = "https://open.tiktokapis.com/v2/user/info/";
const TIKTOK_PUBLISH_INIT_URL: &str =
    "https://open.tiktokapis.com/v2/post/publish/video/init/";
const TIKTOK_CREATOR_INFO_URL: &str =
    "https://open.tiktokapis.com/v2/post/publish/creator_info/query/";
const TIKTOK_PUBLISH_STATUS_URL: &str =
    "https://open.tiktokapis.com/v2/post/publish/status/fetch/";

const CALLBACK_PORT: u16 = 17387;
const REDIRECT_URI: &str = "https://nsvlordslug.github.io/ClipGoblin/callback/";

// Scopes for Login Kit + Content Posting
const SCOPES: &str = "user.info.basic,video.publish,video.upload";

const AUTH_TIMEOUT_SECS: u64 = 120;

/// 5 MB per chunk for uploads.
const UPLOAD_CHUNK_SIZE: usize = 10 * 1024 * 1024; // 10 MB per chunk for large files
const SINGLE_CHUNK_LIMIT: usize = 64 * 1024 * 1024; // Files under 64 MB → single chunk

static CLIENT_KEY: Lazy<String> = Lazy::new(|| {
    match std::env::var("TIKTOK_CLIENT_KEY") {
        Ok(val) => {
            let preview = if val.len() > 6 { &val[..6] } else { &val };
            log::info!("TikTok CLIENT_KEY loaded: '{}...' (len={})", preview, val.len());
            val
        }
        Err(_) => {
            log::warn!("TIKTOK_CLIENT_KEY environment variable is not set — TikTok OAuth will fail");
            String::new()
        }
    }
});

fn client_key() -> &'static str {
    &CLIENT_KEY
}

// ═══════════════════════════════════════════════════════════════════
//  OAuth CSRF state
// ═══════════════════════════════════════════════════════════════════

static OAUTH_STATE: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));

/// PKCE code_verifier — stored between start_auth and exchange_code.
static PKCE_VERIFIER: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));

// ═══════════════════════════════════════════════════════════════════
//  Internal response types
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: String,
    #[serde(default)]
    refresh_expires_in: u64,
    /// open_id is returned on initial auth but NOT on token refresh
    #[serde(default)]
    open_id: String,
    /// Returned by TikTok API but unused — kept to avoid deserialization errors.
    #[allow(dead_code)]
    token_type: Option<String>,
}

#[derive(Debug)]
struct TikTokUserInfo {
    open_id: String,
    display_name: String,
    /// The actual @handle — `Some` if user.info.profile scope is available, `None` otherwise.
    username: Option<String>,
}

/// Response from Content Posting init endpoint.
#[derive(Debug, Deserialize)]
struct PublishInitResponse {
    data: Option<PublishInitData>,
    error: Option<TikTokApiError>,
}

#[derive(Debug, Deserialize)]
struct PublishInitData {
    publish_id: String,
    upload_url: String,
}

#[derive(Debug, Deserialize)]
struct TikTokApiError {
    code: String,
    message: String,
}

// ═══════════════════════════════════════════════════════════════════
//  Public callback-server helpers (called from lib.rs commands)
// ═══════════════════════════════════════════════════════════════════

/// Bind the OAuth callback server on port 17387.
pub fn bind_callback_server() -> Result<TcpListener, AppError> {
    TcpListener::bind(format!("127.0.0.1:{}", CALLBACK_PORT))
        .or_else(|_| TcpListener::bind(format!("[::1]:{}", CALLBACK_PORT)))
        .map_err(|e| {
            AppError::Api(format!(
                "Failed to bind TikTok callback server on port {}: {}",
                CALLBACK_PORT, e
            ))
        })
}

/// Wait for the TikTok OAuth callback on an already-bound listener.
/// Times out after `AUTH_TIMEOUT_SECS` seconds.
pub fn wait_for_auth_code(listener: TcpListener) -> Result<String, AppError> {
    listener
        .set_nonblocking(true)
        .map_err(|e| AppError::Api(format!("Failed to configure callback server: {}", e)))?;

    let deadline = Instant::now() + Duration::from_secs(AUTH_TIMEOUT_SECS);

    loop {
        if Instant::now() > deadline {
            return Err(AppError::Api(
                "TikTok login timed out after 2 minutes. Please try again.".into(),
            ));
        }

        let (stream, _) = match listener.accept() {
            Ok(conn) => conn,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(e) => return Err(AppError::Api(format!("Callback server error: {}", e))),
        };

        stream.set_nonblocking(false).ok();

        let mut reader = BufReader::new(&stream);
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() {
            continue;
        }

        let path = request_line.split_whitespace().nth(1).unwrap_or("");

        // Skip non-callback requests (e.g. /favicon.ico)
        if !path.starts_with("/callback") {
            let resp =
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            if let Ok(mut w) = stream.try_clone() {
                w.write_all(resp.as_bytes()).ok();
                w.flush().ok();
            }
            continue;
        }

        // Parse query parameters from /callback/?code=xxx&state=yyy
        let query = path.split('?').nth(1).unwrap_or("");
        let params: Vec<(&str, &str)> = query
            .split('&')
            .filter_map(|p| {
                let mut kv = p.splitn(2, '=');
                Some((kv.next()?, kv.next().unwrap_or("")))
            })
            .collect();

        let find = |key: &str| params.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);

        // Handle TikTok error callback
        if let Some(error) = find("error") {
            let error_decoded = urlencoding::decode(error)
                .map(|s| s.into_owned())
                .unwrap_or_else(|_| error.to_string());
            let error_safe = html_escape(&error_decoded);
            send_html_response(
                &stream,
                false,
                &format!("Authorization failed: {}", error_safe),
            );
            return Err(AppError::Api(format!(
                "TikTok authorization denied: {}",
                error_decoded
            )));
        }

        // Extract auth code and validate CSRF state
        if let Some(code) = find("code") {
            let callback_state = find("state").unwrap_or("");
            let expected_state = OAUTH_STATE.lock().map(|g| g.clone()).unwrap_or_default();

            if callback_state.is_empty() || callback_state != expected_state {
                send_html_response(
                    &stream,
                    false,
                    "Invalid OAuth state. Please try logging in again.",
                );
                return Err(AppError::Api(
                    "OAuth state mismatch — possible CSRF. Please try again.".into(),
                ));
            }

            send_html_response(
                &stream,
                true,
                "TikTok connected! You can close this tab and return to ClipGoblin.",
            );
            // URL-decode the code so exchange_code doesn't double-encode it
            let decoded_code = urlencoding::decode(code)
                .map(|s| s.into_owned())
                .unwrap_or_else(|_| code.to_string());
            log::info!("TikTok auth code received (len={})", decoded_code.len());
            return Ok(decoded_code);
        }

        send_html_response(
            &stream,
            false,
            "No authorization code received from TikTok.",
        );
        return Err(AppError::Api(
            "TikTok callback did not contain an authorization code.".into(),
        ));
    }
}

// ═══════════════════════════════════════════════════════════════════
//  PlatformAdapter
// ═══════════════════════════════════════════════════════════════════

pub struct TikTokAdapter;

#[async_trait::async_trait(?Send)]
impl PlatformAdapter for TikTokAdapter {
    fn platform_id(&self) -> &'static str {
        "tiktok"
    }

    fn is_ready(&self, db: &Connection) -> Result<bool, AppError> {
        let has_access = db::get_setting(db, "tiktok_access_token")
            .map_err(|e| AppError::Database(e.to_string()))?
            .is_some();
        let has_refresh = db::get_setting(db, "tiktok_refresh_token")
            .map_err(|e| AppError::Database(e.to_string()))?
            .is_some();
        Ok(has_access && has_refresh)
    }

    async fn start_auth(&self) -> Result<String, AppError> {
        use sha2::{Digest, Sha256};

        // Generate CSRF state using UUID v4 (cryptographically random)
        let state = uuid::Uuid::new_v4().to_string().replace('-', "");

        if let Ok(mut guard) = OAUTH_STATE.lock() {
            *guard = state.clone();
        }

        // Generate PKCE code_verifier (43–128 chars, unreserved charset [A-Za-z0-9-._~])
        // Two UUID v4s concatenated without dashes = 64 hex chars (valid PKCE verifier)
        let code_verifier = format!(
            "{}{}",
            uuid::Uuid::new_v4().to_string().replace('-', ""),
            uuid::Uuid::new_v4().to_string().replace('-', ""),
        );

        // Store verifier for use in token exchange
        if let Ok(mut guard) = PKCE_VERIFIER.lock() {
            *guard = code_verifier.clone();
        }

        // TikTok's PKCE S256: code_challenge = HEX(SHA256(code_verifier))
        // NOTE: TikTok uses hex encoding, NOT the standard base64url encoding.
        // See https://developers.tiktok.com/doc/login-kit-desktop/
        let mut sha = Sha256::new();
        sha.update(code_verifier.as_bytes());
        let hash = sha.finalize();
        let code_challenge = hash
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>();

        log::info!(
            "TikTok PKCE: verifier_len={}, challenge_len={}",
            code_verifier.len(), code_challenge.len()
        );

        // TikTok Login Kit OAuth URL with PKCE
        let url = format!(
            "{}?client_key={}&redirect_uri={}&response_type=code&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
            TIKTOK_AUTH_URL,
            urlencoding::encode(client_key()),
            urlencoding::encode(REDIRECT_URI),
            urlencoding::encode(SCOPES),
            urlencoding::encode(&state),
            urlencoding::encode(&code_challenge),
        );

        log::info!("TikTok auth URL: {}", url);

        Ok(url)
    }

    async fn handle_callback(
        &self,
        db: &Connection,
        code: &str,
    ) -> Result<ConnectedAccount, AppError> {
        let code_owned = code.to_string();
        let (tokens, user_info) = do_handle_callback_net(&code_owned).await?;

        let expiry = chrono::Utc::now().timestamp() + tokens.expires_in as i64;
        let refresh_expiry =
            chrono::Utc::now().timestamp() + tokens.refresh_expires_in as i64;

        db::save_setting(db, "tiktok_access_token", &tokens.access_token)
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(db, "tiktok_refresh_token", &tokens.refresh_token)
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(db, "tiktok_token_expiry", &expiry.to_string())
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(db, "tiktok_refresh_expiry", &refresh_expiry.to_string())
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(db, "tiktok_open_id", &user_info.open_id)
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(db, "tiktok_display_name", &user_info.display_name)
            .map_err(|e| AppError::Database(e.to_string()))?;
        // Save the TikTok @handle. Priority:
        // 1. API-returned username (if user.info.profile scope is available)
        // 2. Existing manually-set handle (don't overwrite user's override)
        // 3. display_name as a last resort
        if let Some(ref handle) = user_info.username {
            // Got the real handle from the API — always use it
            db::save_setting(db, "tiktok_handle", handle)
                .map_err(|e| AppError::Database(e.to_string()))?;
        } else if db::get_setting(db, "tiktok_handle")
            .map_err(|e| AppError::Database(e.to_string()))?
            .is_none()
        {
            // No API handle and nothing saved — default to display_name
            db::save_setting(db, "tiktok_handle", &user_info.display_name)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }

        let now = chrono::Utc::now().to_rfc3339();
        Ok(ConnectedAccount {
            platform: "tiktok".into(),
            account_name: user_info.display_name,
            account_id: user_info.open_id,
            connected_at: now,
        })
    }

    async fn refresh_token(&self, db: &Connection) -> Result<(), AppError> {
        let refresh_tok = db::get_setting(db, "tiktok_refresh_token")
            .map_err(|e| AppError::Database(e.to_string()))?
            .ok_or_else(|| {
                AppError::Api("No TikTok refresh token found — please reconnect.".into())
            })?;

        let tokens = do_refresh_token_net(&refresh_tok).await?;

        let expiry = chrono::Utc::now().timestamp() + tokens.expires_in as i64;
        let refresh_expiry =
            chrono::Utc::now().timestamp() + tokens.refresh_expires_in as i64;

        db::save_setting(db, "tiktok_access_token", &tokens.access_token)
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(db, "tiktok_refresh_token", &tokens.refresh_token)
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(db, "tiktok_token_expiry", &expiry.to_string())
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(db, "tiktok_refresh_expiry", &refresh_expiry.to_string())
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    async fn upload_video(
        &self,
        db: &Connection,
        file_path: &str,
        meta: &UploadMeta,
    ) -> Result<UploadResult, AppError> {
        log::info!("TikTok upload_video called: file={}, clip_id={}", file_path, meta.clip_id);

        // 1. Validate the export file
        validate_export_file(Some(file_path))?;
        log::info!("TikTok upload: file validated OK");

        // 2. Duplicate check — skip if the stored URL is a stale fallback
        if !meta.force {
            if let Some(existing) = db::get_upload_for_clip(db, &meta.clip_id, "tiktok")
                .map_err(|e| AppError::Database(e.to_string()))?
            {
                let url = existing.video_url.unwrap_or_default();
                // Only treat as duplicate if the URL points to an actual video,
                // not a generic fallback like "tiktok.com" or a profile page.
                let has_real_url = url.contains("/video/") || url.contains("publish_id=");
                if has_real_url {
                    return Ok(UploadResult {
                        status: UploadResultStatus::Duplicate {
                            existing_url: url,
                        },
                        job_id: String::new(),
                    });
                }
                log::info!("TikTok: existing upload has stale URL '{}' — allowing re-upload", url);
            }
        }

        // 3. Ensure token is fresh
        let expiry_str = db::get_setting(db, "tiktok_token_expiry")
            .map_err(|e| AppError::Database(e.to_string()))?;

        let needs_refresh = match expiry_str {
            Some(s) => {
                let expiry: i64 = s.parse().unwrap_or(0);
                let now = chrono::Utc::now().timestamp();
                now >= expiry - 60
            }
            None => true,
        };

        if needs_refresh {
            let refresh_tok = db::get_setting(db, "tiktok_refresh_token")
                .map_err(|e| AppError::Database(e.to_string()))?
                .ok_or_else(|| {
                    AppError::Api("No TikTok refresh token found — please reconnect.".into())
                })?;

            let new_tokens = do_refresh_token_net(&refresh_tok).await?;

            let new_expiry = chrono::Utc::now().timestamp() + new_tokens.expires_in as i64;
            db::save_setting(db, "tiktok_access_token", &new_tokens.access_token)
                .map_err(|e| AppError::Database(e.to_string()))?;
            db::save_setting(db, "tiktok_token_expiry", &new_expiry.to_string())
                .map_err(|e| AppError::Database(e.to_string()))?;
            db::save_setting(db, "tiktok_refresh_token", &new_tokens.refresh_token)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }

        let access_token = db::get_setting(db, "tiktok_access_token")
            .map_err(|e| AppError::Database(e.to_string()))?
            .ok_or_else(|| {
                AppError::Api("No TikTok access token — please reconnect.".into())
            })?;

        log::info!("TikTok upload: access token present (len={})", access_token.len());

        // Read file bytes (sync)
        let file_bytes = std::fs::read(file_path)
            .map_err(|e| AppError::Unknown(format!("Failed to read export file: {}", e)))?;

        log::info!("TikTok upload: read {} bytes from {}", file_bytes.len(), file_path);

        let title = meta.title.clone();
        let description = meta.description.clone();
        let visibility = meta.visibility.clone();
        // Prefer the explicit handle setting; fall back to display_name
        let tiktok_handle = db::get_setting(db, "tiktok_handle")
            .map_err(|e| AppError::Database(e.to_string()))?
            .or_else(|| {
                db::get_setting(db, "tiktok_display_name").ok().flatten()
            })
            .unwrap_or_default();

        // 4. Upload via Content Posting API
        log::info!("TikTok upload: calling do_upload_net (title='{}', visibility='{}', handle='{}')", title, visibility, tiktok_handle);
        let (publish_id, video_url) =
            do_upload_net(&access_token, &title, &description, &visibility, file_bytes, &tiktok_handle).await?;

        // 5. Record in upload history
        db::upsert_upload(db, &meta.clip_id, "tiktok", &video_url)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(UploadResult {
            status: UploadResultStatus::Complete { video_url },
            job_id: publish_id,
        })
    }

    fn disconnect(&self, db: &Connection) -> Result<(), AppError> {
        db::delete_settings_for_platform(db, "tiktok")
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn get_account(&self, db: &Connection) -> Result<Option<ConnectedAccount>, AppError> {
        let display_name = db::get_setting(db, "tiktok_display_name")
            .map_err(|e| AppError::Database(e.to_string()))?;
        let open_id = db::get_setting(db, "tiktok_open_id")
            .map_err(|e| AppError::Database(e.to_string()))?;

        match (display_name, open_id) {
            (Some(name), Some(id)) => Ok(Some(ConnectedAccount {
                platform: "tiktok".into(),
                account_name: name,
                account_id: id,
                connected_at: String::new(),
            })),
            _ => Ok(None),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Async network helpers (no &Connection — safe for Send futures)
// ═══════════════════════════════════════════════════════════════════

async fn do_handle_callback_net(
    code: &str,
) -> Result<(TokenResponse, TikTokUserInfo), AppError> {
    let tokens = exchange_code(code).await?;
    let user_info = fetch_user_info(&tokens.access_token, &tokens.open_id).await?;
    Ok((tokens, user_info))
}

async fn do_refresh_token_net(refresh_tok: &str) -> Result<TokenResponse, AppError> {
    log::info!("TikTok token refresh via auth proxy");

    let proxy = AuthProxy::new()
        .map_err(|e| AppError::Api(format!("Auth proxy init failed: {}", e)))?;
    let proxy_resp = proxy.tiktok_refresh(refresh_tok).await
        .map_err(|e| AppError::Api(e))?;

    // Check for error in proxy response
    if let Some(err) = proxy_resp.error {
        let desc = proxy_resp.error_description.unwrap_or_default();
        return Err(AppError::Api(format!(
            "TikTok token refresh error: {} — {}",
            err, desc
        )));
    }

    let access_token = proxy_resp.access_token
        .ok_or_else(|| AppError::Api("Proxy response missing access_token".into()))?;
    let refresh_token = proxy_resp.refresh_token
        .ok_or_else(|| AppError::Api("Proxy response missing refresh_token".into()))?;

    log::info!("TikTok token refresh succeeded via proxy");

    Ok(TokenResponse {
        access_token,
        expires_in: proxy_resp.expires_in.unwrap_or(0),
        refresh_token,
        refresh_expires_in: 0,
        open_id: proxy_resp.open_id.unwrap_or_default(),
        token_type: proxy_resp.token_type,
    })
}

/// Query creator info — recommended by TikTok before initiating a Direct Post.
/// Returns the available privacy levels so we can pick a valid one.
/// Non-fatal: if this fails, we fall back to SELF_ONLY and still attempt the upload.
async fn query_creator_info(
    access_token: &str,
) -> Vec<String> {
    let client = reqwest::Client::new();

    // TikTok expects a POST with an empty JSON body (not bodyless)
    let resp = match client
        .post(TIKTOK_CREATOR_INFO_URL)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json; charset=UTF-8")
        .body("{}")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            log::warn!("TikTok creator_info request failed: {} — continuing with SELF_ONLY", e);
            return vec![];
        }
    };

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    log::info!("TikTok creator_info response ({})", status);
    log::debug!("TikTok creator_info body: {}", body);

    if !status.is_success() {
        log::warn!("TikTok creator_info returned {} — continuing with SELF_ONLY", status);
        return vec![];
    }

    // Parse privacy_level_options from response
    let parsed: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("Failed to parse creator_info JSON: {} — continuing with SELF_ONLY", e);
            return vec![];
        }
    };

    // Check for API-level error
    let error_code = parsed["error"]["code"].as_str().unwrap_or("ok");
    if error_code != "ok" {
        let error_msg = parsed["error"]["message"].as_str().unwrap_or("unknown");
        log::warn!(
            "TikTok creator_info API error: {} — {} — continuing with SELF_ONLY",
            error_code, error_msg
        );
        return vec![];
    }

    let options = parsed["data"]["privacy_level_options"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if options.is_empty() {
        log::warn!("TikTok creator_info returned no privacy_level_options");
    } else {
        log::info!("TikTok available privacy levels: {:?}", options);
    }

    options
}

/// Map frontend visibility value to TikTok Content Posting API privacy_level.
///   "public"  → PUBLIC_TO_EVERYONE
///   "friends" → MUTUAL_FOLLOW_FRIENDS
///   "private" → SELF_ONLY (draft — only you can see it)
fn map_visibility_to_privacy(visibility: &str) -> &'static str {
    match visibility {
        "public" => "PUBLIC_TO_EVERYONE",
        "friends" => "MUTUAL_FOLLOW_FRIENDS",
        _ => "SELF_ONLY",
    }
}

/// Upload video via TikTok Content Posting API.
/// Flow: creator_info query → init upload → PUT chunks → returns publish_id.
async fn do_upload_net(
    access_token: &str,
    title: &str,
    description: &str,
    visibility: &str,
    file_bytes: Vec<u8>,
    tiktok_handle: &str,
) -> Result<(String, String), AppError> {
    let client = reqwest::Client::new();
    let total_size = file_bytes.len();

    // 0. Query creator info (recommended by TikTok before Direct Post — non-fatal)
    let privacy_options = query_creator_info(access_token).await;

    // Map frontend visibility to TikTok API privacy level
    let requested = map_visibility_to_privacy(visibility);

    // Use the requested level if available, otherwise fall back safely
    let privacy_level = if privacy_options.contains(&requested.to_string()) {
        requested
    } else if privacy_options.contains(&"SELF_ONLY".to_string()) {
        log::warn!(
            "TikTok: requested privacy '{}' not available, falling back to SELF_ONLY. Available: {:?}",
            requested, privacy_options
        );
        "SELF_ONLY"
    } else if let Some(first) = privacy_options.first() {
        log::warn!(
            "TikTok: requested privacy '{}' not available, using first available: {}",
            requested, first
        );
        first.as_str()
    } else {
        "SELF_ONLY" // absolute fallback
    };

    log::info!(
        "TikTok upload: file_size={}, privacy_level={}, available_options={:?}",
        total_size, privacy_level, privacy_options
    );

    // 1. Determine chunk sizing
    //    TikTok Content Posting API rules:
    //    - Files ≤ 64 MB: upload as single chunk (chunk_size = video_size, count = 1)
    //    - Files > 64 MB: split into 10 MB chunks (each 5–64 MB allowed, final up to 128 MB)
    let (actual_chunk_size, chunk_count) = if total_size <= SINGLE_CHUNK_LIMIT {
        (total_size, 1usize)
    } else {
        let count = (total_size + UPLOAD_CHUNK_SIZE - 1) / UPLOAD_CHUNK_SIZE;
        (UPLOAD_CHUNK_SIZE, count)
    };

    // 2. Init upload — tells TikTok we want to upload a video
    // TikTok's "title" field is the full video caption (description + hashtags).
    // Use description if available (already has hashtags appended by frontend),
    // fall back to title if description is empty.
    let caption = if description.trim().is_empty() { title } else { description };

    let init_body = serde_json::json!({
        "post_info": {
            "title": caption,
            "privacy_level": privacy_level,
            "disable_duet": false,
            "disable_comment": false,
            "disable_stitch": false,
        },
        "source_info": {
            "source": "FILE_UPLOAD",
            "video_size": total_size,
            "chunk_size": actual_chunk_size,
            "total_chunk_count": chunk_count,
        }
    });

    log::debug!("TikTok upload init body: {}", init_body);

    let init_resp = client
        .post(TIKTOK_PUBLISH_INIT_URL)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json; charset=UTF-8")
        .json(&init_body)
        .send()
        .await?;

    let init_status = init_resp.status();
    let init_body_text = init_resp.text().await.unwrap_or_default();

    log::info!("TikTok upload init response ({})", init_status);
    log::debug!("TikTok upload init response body: {}", init_body_text);

    if !init_status.is_success() {
        return Err(AppError::Api(format!(
            "TikTok upload init failed ({}): {}",
            init_status, init_body_text
        )));
    }

    let init_result: PublishInitResponse = serde_json::from_str(&init_body_text)
        .map_err(|e| AppError::Api(format!(
            "Failed to parse TikTok init response: {} — body: {}",
            e, init_body_text
        )))?;

    if let Some(ref err) = init_result.error {
        if err.code != "ok" {
            return Err(AppError::Api(format!(
                "TikTok upload init error: {} — {}",
                err.code, err.message
            )));
        }
    }

    let init_data = init_result.data.ok_or_else(|| {
        AppError::Api(format!(
            "TikTok upload init returned no data. Full response: {}",
            init_body_text
        ))
    })?;

    let upload_url = init_data.upload_url;
    let publish_id = init_data.publish_id;

    log::info!("TikTok upload URL obtained, publish_id={}", publish_id);

    // 3. Upload file in chunks via PUT
    let mut offset: usize = 0;
    let mut chunk_idx: usize = 0;

    while offset < total_size {
        let end = std::cmp::min(offset + actual_chunk_size, total_size);
        let chunk = &file_bytes[offset..end];

        let content_range = format!("bytes {}-{}/{}", offset, end - 1, total_size);

        log::info!(
            "TikTok uploading chunk {}/{}: Content-Range: {}",
            chunk_idx + 1, chunk_count, content_range
        );

        let chunk_resp = client
            .put(&upload_url)
            .header("Content-Range", &content_range)
            .header("Content-Length", chunk.len().to_string())
            .header("Content-Type", "video/mp4")
            .body(chunk.to_vec())
            .send()
            .await?;

        let status = chunk_resp.status().as_u16();

        if status >= 200 && status < 300 {
            // Chunk accepted
            offset = end;
            chunk_idx += 1;
            log::info!(
                "TikTok upload chunk {}/{} complete (HTTP {})",
                chunk_idx, chunk_count, status
            );
            continue;
        }

        let body = chunk_resp.text().await.unwrap_or_default();
        log::error!(
            "TikTok chunk upload failed (HTTP {}): {}",
            status, body
        );
        return Err(AppError::Api(format!(
            "TikTok chunk upload failed ({}): {}",
            status, body
        )));
    }

    log::info!("TikTok upload complete — all {} chunks sent. Polling for video URL...", chunk_count);

    // Poll the publish status endpoint to get the real video URL.
    // TikTok needs time to process the upload before returning a post ID.
    let video_url = poll_publish_status(&client, access_token, &publish_id, tiktok_handle).await;

    Ok((publish_id, video_url))
}

/// Poll TikTok's publish status endpoint until we get a video URL or timeout.
/// Returns the best URL we can build: a direct video link if the post ID is
/// available, otherwise falls back to the user's profile page.
async fn poll_publish_status(
    client: &reqwest::Client,
    access_token: &str,
    publish_id: &str,
    tiktok_handle: &str,
) -> String {
    // Poll every 5s for up to 30 seconds. If the status endpoint doesn't
    // work (common in sandbox), we fall back quickly to the profile URL.
    const MAX_ATTEMPTS: u32 = 6;
    const POLL_INTERVAL: Duration = Duration::from_secs(5);

    log::info!("TikTok: starting publish status poll (handle='{}')", tiktok_handle);

    for attempt in 1..=MAX_ATTEMPTS {
        tokio::time::sleep(POLL_INTERVAL).await;

        log::info!(
            "TikTok publish status poll {}/{} for publish_id={}",
            attempt, MAX_ATTEMPTS, publish_id
        );

        let resp = client
            .post(TIKTOK_PUBLISH_STATUS_URL)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "publish_id": publish_id }))
            .send()
            .await;

        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                log::warn!("TikTok status poll request failed: {}", e);
                continue;
            }
        };

        let body_text = resp.text().await.unwrap_or_default();
        log::debug!("TikTok status poll response: {}", body_text);

        // Parse as raw JSON first so we can inspect whatever TikTok returns
        let json: serde_json::Value = match serde_json::from_str(&body_text) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("TikTok status poll parse error: {} — body_len={}", e, body_text.len());
                continue;
            }
        };

        // Check for API-level errors (e.g. scope issues, rate limits)
        if let Some(err) = json.get("error") {
            let code = err.get("code").and_then(|v| v.as_str()).unwrap_or("unknown");
            let msg = err.get("message").and_then(|v| v.as_str()).unwrap_or("");
            log::warn!("TikTok status poll API error: {} — {}", code, msg);
            if code == "ok" {
                // "ok" is not actually an error — TikTok includes error.code = "ok" on success
            } else {
                // Real error — stop polling
                break;
            }
        }

        let data = &json["data"];
        let status = data["status"].as_str().unwrap_or("");

        match status {
            "PUBLISH_COMPLETE" => {
                // Extract post ID — could be string, number, array, or nested
                let post_id_val = &data["publicaly_available_post_id"];
                let post_id = post_id_val.as_str().map(|s| s.to_string())
                    .or_else(|| post_id_val.as_u64().map(|n| n.to_string()))
                    .or_else(|| post_id_val.as_i64().map(|n| n.to_string()))
                    .or_else(|| {
                        // Might be an array
                        post_id_val.as_array().and_then(|arr| arr.first()).and_then(|v| {
                            v.as_str().map(|s| s.to_string())
                                .or_else(|| v.as_u64().map(|n| n.to_string()))
                        })
                    });

                if let Some(pid) = post_id.filter(|s| !s.is_empty() && s != "0") {
                    let url = format!("https://www.tiktok.com/video/{}", pid);
                    log::info!("TikTok video URL resolved: {}", url);
                    return url;
                }
                log::info!("TikTok PUBLISH_COMPLETE but no post ID yet (moderation pending)");
            }
            "FAILED" => {
                let reason = data["fail_reason"].as_str().unwrap_or("unknown");
                log::error!("TikTok publish failed: {}", reason);
                break;
            }
            "" => {
                log::info!("TikTok status poll: no status field in response");
            }
            other => {
                log::info!("TikTok publish status: {} — still processing", other);
            }
        }
    }

    // Fallback: link to user's TikTok profile page using their handle.
    if !tiktok_handle.is_empty() {
        log::info!("TikTok status poll done — falling back to profile @{}", tiktok_handle);
        format!("https://www.tiktok.com/@{}", tiktok_handle)
    } else {
        log::info!("TikTok status poll done — no handle set, using tiktok.com");
        "https://www.tiktok.com".to_string()
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Private helpers
// ═══════════════════════════════════════════════════════════════════

/// Exchange an authorization code for access + refresh tokens via auth proxy.
async fn exchange_code(code: &str) -> Result<TokenResponse, AppError> {
    // Retrieve the PKCE code_verifier generated during start_auth
    let code_verifier = PKCE_VERIFIER
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();

    if code_verifier.is_empty() {
        return Err(AppError::Api(
            "PKCE code_verifier missing — please restart the TikTok login flow.".into(),
        ));
    }

    log::info!(
        "TikTok token exchange via auth proxy (code={}..., verifier={} chars)",
        &code[..code.len().min(10)],
        code_verifier.len()
    );

    let proxy = AuthProxy::new()
        .map_err(|e| AppError::Api(format!("Auth proxy init failed: {}", e)))?;
    let proxy_resp = proxy
        .tiktok_token_exchange(code, REDIRECT_URI, &code_verifier)
        .await
        .map_err(|e| AppError::Api(e))?;

    // Check for error in proxy response
    if let Some(err) = proxy_resp.error {
        let desc = proxy_resp.error_description.unwrap_or_default();
        return Err(AppError::Api(format!(
            "TikTok OAuth error: {} — {}",
            err, desc
        )));
    }

    let access_token = proxy_resp.access_token
        .ok_or_else(|| AppError::Api("Proxy response missing access_token".into()))?;
    let refresh_token = proxy_resp.refresh_token
        .ok_or_else(|| AppError::Api("Proxy response missing refresh_token".into()))?;

    log::info!("TikTok token exchange succeeded via proxy");

    Ok(TokenResponse {
        access_token,
        expires_in: proxy_resp.expires_in.unwrap_or(0),
        refresh_token,
        refresh_expires_in: 0, // proxy doesn't return this; TikTok refresh tokens last ~365 days
        open_id: proxy_resp.open_id.unwrap_or_default(),
        token_type: proxy_resp.token_type,
    })
}

/// Fetch the authenticated user's TikTok display name and open_id.
async fn fetch_user_info(
    access_token: &str,
    open_id: &str,
) -> Result<TikTokUserInfo, AppError> {
    let client = reqwest::Client::new();
    let resp = client
        .get(TIKTOK_USERINFO_URL)
        .header("Authorization", format!("Bearer {}", access_token))
        .query(&[("fields", "open_id,display_name")])
        .send()
        .await?;

    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();

    log::info!("TikTok user info response ({})", status);
    log::debug!("TikTok user info body: {}", body_text);

    if !status.is_success() {
        return Err(AppError::Api(format!(
            "TikTok user info fetch failed ({}): {}",
            status, body_text
        )));
    }

    let body: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|e| AppError::Api(format!(
            "TikTok user info: failed to parse response: {} — body: {}",
            e, body_text
        )))?;
    let data = &body["data"]["user"];
    let display_name = data["display_name"]
        .as_str()
        .unwrap_or("TikTok User")
        .to_string();
    log::info!("TikTok user info: display_name={}", display_name);

    // Try to fetch the actual @handle (requires user.info.profile scope).
    // This is a separate call so it can fail gracefully in sandbox mode
    // where user.info.profile isn't approved.
    let username = fetch_username_best_effort(&client, access_token).await;
    log::info!("TikTok user info: display_name={}, username={:?}", display_name, username);

    Ok(TikTokUserInfo {
        open_id: open_id.to_string(),
        display_name,
        username,
    })
}

/// Try to fetch the TikTok @handle via user.info.profile scope.
/// Returns None if the scope isn't authorized — this is expected in sandbox.
async fn fetch_username_best_effort(
    client: &reqwest::Client,
    access_token: &str,
) -> Option<String> {
    let resp = client
        .get(TIKTOK_USERINFO_URL)
        .header("Authorization", format!("Bearer {}", access_token))
        .query(&[("fields", "username")])
        .send()
        .await
        .ok()?;

    let body_text = resp.text().await.ok()?;
    log::debug!("TikTok username fetch response: {}", body_text);

    let body: serde_json::Value = serde_json::from_str(&body_text).ok()?;

    // Check for API error (scope not authorized, etc.)
    if let Some(err) = body.get("error") {
        let code = err.get("code").and_then(|v| v.as_str()).unwrap_or("");
        if code != "ok" {
            log::info!("TikTok username fetch not available ({}), will use display_name", code);
            return None;
        }
    }

    body["data"]["user"]["username"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Send a styled HTML response through the TCP stream.
fn send_html_response(stream: &std::net::TcpStream, success: bool, message: &str) {
    let (icon, color) = if success {
        ("&#10004;", "#00f2ea") // checkmark, tiktok cyan
    } else {
        ("&#10008;", "#ef4444") // cross, red
    };

    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head><title>ClipGoblin</title></head>
<body style="background:#0f0a1a;color:#e2e8f0;font-family:system-ui;display:flex;align-items:center;justify-content:center;height:100vh;margin:0">
<div style="text-align:center">
<h1 style="color:{color}">{icon} {heading}</h1>
<p>{message}</p>
</div>
</body>
</html>"#,
        color = color,
        icon = icon,
        heading = if success { "Connected!" } else { "Error" },
        message = message,
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

/// Minimal HTML-escape to prevent XSS in error messages.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
