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
use tokio::io::AsyncReadExt;

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

// Match the scopes approved for the production TikTok app. `video.list` is
// intentionally omitted until TikTok approves it in a future app revision.
const SCOPES: &str = "user.info.basic,video.publish,video.upload";

pub(crate) fn video_stats_enabled() -> bool {
    SCOPES.split(',').any(|scope| scope == "video.list")
}

const AUTH_TIMEOUT_SECS: u64 = 120;

/// 5 MB per chunk for uploads.
const UPLOAD_CHUNK_SIZE: usize = 10 * 1024 * 1024; // 10 MB per chunk for large files
const SINGLE_CHUNK_LIMIT: usize = 64 * 1024 * 1024; // Files under 64 MB → single chunk

/// Embedded TikTok OAuth client key — safe to ship in the binary since OAuth
/// client keys are public identifiers (the actual client *secret* stays in the
/// Cloudflare Worker). Same value already lives in `worker/wrangler.toml`.
/// Override with `TIKTOK_CLIENT_KEY` env var for development.
const DEFAULT_TIKTOK_CLIENT_KEY: &str = "awzco3f3mgjpwjam";

static CLIENT_KEY: Lazy<String> = Lazy::new(|| {
    match std::env::var("TIKTOK_CLIENT_KEY") {
        Ok(val) if !val.is_empty() => {
            let preview = if val.len() > 6 { &val[..6] } else { &val };
            log::info!("TikTok CLIENT_KEY loaded from env: '{}...' (len={})", preview, val.len());
            val
        }
        _ => {
            log::info!("Using embedded TikTok CLIENT_KEY");
            DEFAULT_TIKTOK_CLIENT_KEY.to_string()
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
static TIKTOK_REFRESH_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
            code_verifier.len(),
            code_challenge.len()
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
        db: &crate::DbConn,
        code: &str,
    ) -> Result<ConnectedAccount, AppError> {
        let code_owned = code.to_string();
        let (tokens, user_info) = do_handle_callback_net(&code_owned).await?;

        let expiry = chrono::Utc::now().timestamp() + tokens.expires_in as i64;
        let refresh_expiry = chrono::Utc::now().timestamp() + tokens.refresh_expires_in as i64;

        let conn = db
            .lock()
            .map_err(|e| AppError::Database(format!("DB lock: {}", e)))?;
        db::save_setting(&conn, "tiktok_access_token", &tokens.access_token)
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(&conn, "tiktok_refresh_token", &tokens.refresh_token)
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(&conn, "tiktok_token_expiry", &expiry.to_string())
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(&conn, "tiktok_refresh_expiry", &refresh_expiry.to_string())
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(&conn, "tiktok_open_id", &user_info.open_id)
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(&conn, "tiktok_display_name", &user_info.display_name)
            .map_err(|e| AppError::Database(e.to_string()))?;
        // Save the TikTok @handle. Priority:
        // 1. API-returned username (if user.info.profile scope is available)
        // 2. Existing manually-set handle (don't overwrite user's override)
        // 3. display_name as a last resort
        if let Some(ref handle) = user_info.username {
            // Got the real handle from the API — always use it
            db::save_setting(&conn, "tiktok_handle", handle)
                .map_err(|e| AppError::Database(e.to_string()))?;
        } else if db::get_setting(&conn, "tiktok_handle")
            .map_err(|e| AppError::Database(e.to_string()))?
            .is_none()
        {
            // No API handle and nothing saved — default to display_name
            db::save_setting(&conn, "tiktok_handle", &user_info.display_name)
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

    async fn refresh_token(&self, db: &crate::DbConn) -> Result<(), AppError> {
        refresh_access_token(db, true).await?;
        Ok(())
    }

    async fn upload_video(
        &self,
        db: &crate::DbConn,
        file_path: &str,
        meta: &UploadMeta,
    ) -> Result<UploadResult, AppError> {
        log::info!(
            "TikTok upload_video called: file={}, clip_id={}",
            file_path,
            meta.clip_id
        );
        validate_export_file(Some(file_path))?;

        let claim = {
            let conn = db
                .lock()
                .map_err(|e| AppError::Database(format!("DB lock: {}", e)))?;
            db::begin_upload(&conn, &meta.clip_id, "tiktok", meta.force)
                .map_err(|e| AppError::Database(e.to_string()))?
        };
        match claim {
            db::UploadClaim::Completed { video_url } => {
                return Ok(UploadResult {
                    status: UploadResultStatus::Duplicate {
                        existing_url: video_url,
                    },
                    job_id: String::new(),
                });
            }
            db::UploadClaim::InProgress { job_id } => {
                return Ok(UploadResult {
                    status: UploadResultStatus::Processing,
                    job_id: job_id.unwrap_or_default(),
                });
            }
            db::UploadClaim::Acquired => {}
        }

        let title = meta.title.clone();
        let description = meta.description.clone();
        let visibility = meta.visibility.clone();
        let clip_id = meta.clip_id.clone();

        let upload_result = async {
            let access_token = ensure_fresh_access_token(db).await?;
            do_upload_net(
                &access_token,
                &title,
                &description,
                &visibility,
                file_path,
                meta.disable_comment,
                meta.disable_duet,
                meta.disable_stitch,
                meta.brand_organic,
                meta.branded_content,
                &clip_id,
                |publish_id| {
                    let conn = db
                        .lock()
                        .map_err(|e| AppError::Database(format!("DB lock: {}", e)))?;
                    db::mark_upload_processing(&conn, &clip_id, "tiktok", publish_id)
                        .map_err(|e| AppError::Database(e.to_string()))
                },
            )
            .await
        }
        .await;

        match upload_result {
            Ok((
                publish_id,
                UploadResultStatus::Complete {
                    video_url,
                    platform_video_id,
                },
            )) => {
                let conn = db
                    .lock()
                    .map_err(|e| AppError::Database(format!("DB lock: {}", e)))?;
                db::mark_upload_complete(
                    &conn,
                    &meta.clip_id,
                    "tiktok",
                    video_url.as_deref(),
                    Some(&publish_id),
                    platform_video_id.as_deref(),
                )
                .map_err(|e| AppError::Database(e.to_string()))?;
                Ok(UploadResult {
                    status: UploadResultStatus::Complete {
                        video_url,
                        platform_video_id,
                    },
                    job_id: publish_id,
                })
            }
            Ok((publish_id, UploadResultStatus::Processing)) => Ok(UploadResult {
                status: UploadResultStatus::Processing,
                job_id: publish_id,
            }),
            Ok((publish_id, UploadResultStatus::Failed { error })) => {
                let conn = db
                    .lock()
                    .map_err(|e| AppError::Database(format!("DB lock: {}", e)))?;
                db::mark_upload_failed(&conn, &meta.clip_id, "tiktok", &error)
                    .map_err(|e| AppError::Database(e.to_string()))?;
                Ok(UploadResult {
                    status: UploadResultStatus::Failed { error },
                    job_id: publish_id,
                })
            }
            Ok((publish_id, status)) => Ok(UploadResult {
                status,
                job_id: publish_id,
            }),
            Err(error) => {
                if let Ok(conn) = db.lock() {
                    let _ =
                        db::mark_upload_failed(&conn, &meta.clip_id, "tiktok", &error.to_string());
                }
                Err(error)
            }
        }
    }

    fn disconnect(&self, db: &Connection) -> Result<(), AppError> {
        db::delete_settings_for_platform(db, "tiktok")
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn get_account(&self, db: &Connection) -> Result<Option<ConnectedAccount>, AppError> {
        let display_name = db::get_setting(db, "tiktok_display_name")
            .map_err(|e| AppError::Database(e.to_string()))?;
        let open_id =
            db::get_setting(db, "tiktok_open_id").map_err(|e| AppError::Database(e.to_string()))?;

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

    let proxy =
        AuthProxy::new().map_err(|e| AppError::Api(format!("Auth proxy init failed: {}", e)))?;
    let proxy_resp = proxy
        .tiktok_refresh(refresh_tok)
        .await
        .map_err(|e| AppError::Api(e))?;

    // Check for error in proxy response
    if let Some(err) = proxy_resp.error {
        let desc = proxy_resp.error_description.unwrap_or_default();
        if matches!(
            err.as_str(),
            "invalid_grant" | "access_token_invalid" | "refresh_token_invalid"
        ) {
            return Err(AppError::AuthExpired(
                "Your TikTok session has expired. Please reconnect your TikTok account in Settings.".into(),
            ));
        }
        return Err(AppError::Api(format!(
            "TikTok token refresh error: {}; {}",
            err, desc
        )));
    }

    let access_token = proxy_resp
        .access_token
        .ok_or_else(|| AppError::Api("Proxy response missing access_token".into()))?;
    let refresh_token = proxy_resp
        .refresh_token
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

/// Full creator info for the publish UI. TikTok's Content Sharing Guidelines
/// require the publish screen to reflect these (privacy options, interaction
/// restrictions, display name) rather than hardcoding them.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TikTokCreatorInfo {
    pub creator_nickname: String,
    pub creator_username: String,
    pub creator_avatar_url: String,
    pub privacy_level_options: Vec<String>,
    pub comment_disabled: bool,
    pub duet_disabled: bool,
    pub stitch_disabled: bool,
    pub max_video_post_duration_sec: u64,
}

/// Fetch the full creator_info for the compliance panel. Unlike
/// `query_creator_info` (upload path, only needs the privacy list and is
/// non-fatal), this surfaces errors to the UI so the publish screen can tell
/// the user to reconnect instead of silently posting with defaults.
pub async fn fetch_creator_info(access_token: &str) -> Result<TikTokCreatorInfo, AppError> {
    let client = reqwest::Client::new();

    let resp = client
        .post(TIKTOK_CREATOR_INFO_URL)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json; charset=UTF-8")
        .body("{}")
        .send()
        .await
        .map_err(|e| AppError::Api(format!("TikTok creator_info request failed: {}", e)))?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    log::debug!("TikTok fetch_creator_info ({}) body: {}", status, body);

    if !status.is_success() {
        return Err(AppError::Api(format!(
            "TikTok creator_info failed ({}): {}",
            status, body
        )));
    }

    let parsed: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| AppError::Api(format!("creator_info parse error: {}", e)))?;

    let error_code = parsed["error"]["code"].as_str().unwrap_or("ok");
    if error_code != "ok" {
        let msg = parsed["error"]["message"].as_str().unwrap_or("unknown");
        return Err(AppError::Api(format!(
            "TikTok creator_info error: {} — {}",
            error_code, msg
        )));
    }

    let data = &parsed["data"];
    let privacy_level_options = data["privacy_level_options"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(TikTokCreatorInfo {
        creator_nickname: data["creator_nickname"].as_str().unwrap_or("").to_string(),
        creator_username: data["creator_username"].as_str().unwrap_or("").to_string(),
        creator_avatar_url: data["creator_avatar_url"].as_str().unwrap_or("").to_string(),
        privacy_level_options,
        comment_disabled: data["comment_disabled"].as_bool().unwrap_or(false),
        duet_disabled: data["duet_disabled"].as_bool().unwrap_or(false),
        stitch_disabled: data["stitch_disabled"].as_bool().unwrap_or(false),
        max_video_post_duration_sec: data["max_video_post_duration_sec"].as_u64().unwrap_or(0),
    })
}

/// Map frontend visibility value to TikTok Content Posting API privacy_level.
///   "public"  → PUBLIC_TO_EVERYONE
///   "friends" → MUTUAL_FOLLOW_FRIENDS
///   "private" → SELF_ONLY (draft — only you can see it)
fn map_visibility_to_privacy(visibility: &str) -> &'static str {
    match visibility {
        // TikTok Content Posting API enums — sent directly by the compliance
        // panel (its dropdown is populated from creator_info.privacy_level_options).
        "PUBLIC_TO_EVERYONE" => "PUBLIC_TO_EVERYONE",
        "MUTUAL_FOLLOW_FRIENDS" => "MUTUAL_FOLLOW_FRIENDS",
        "FOLLOWER_OF_CREATOR" => "FOLLOWER_OF_CREATOR",
        "SELF_ONLY" => "SELF_ONLY",
        // Legacy / lowercase frontend values (batch dialog + older callers).
        "public" | "public_to_everyone" => "PUBLIC_TO_EVERYONE",
        "friends" | "mutual_follow_friends" => "MUTUAL_FOLLOW_FRIENDS",
        "follower_of_creator" => "FOLLOWER_OF_CREATOR",
        _ => "SELF_ONLY",
    }
}

/// Upload video via TikTok Content Posting API.
/// Flow: creator_info query → init upload → PUT chunks → returns publish_id.
/// Translate a TikTok Content Posting API error code into a human-readable,
/// actionable message. Unknown codes fall back to TikTok's own message + the
/// raw code so nothing is ever hidden from the user.
fn friendly_tiktok_error(code: &str, message: &str) -> String {
    let hint = match code {
        "unaudited_client_can_only_post_to_private_accounts" =>
            "TikTok has not approved ClipGoblin's Direct Post integration yet. Choose 'Only me (private)' while testing; wider audiences unlock after TikTok approves the app.",
        "access_token_invalid" | "access_token_expired" =>
            "Your TikTok session has expired. Reconnect TikTok in Settings and try again.",
        "scope_not_authorized" | "scope_permission_missed" =>
            "ClipGoblin doesn't have permission to post. Reconnect TikTok and approve all the requested permissions.",
        "spam_risk_too_many_posts" =>
            "TikTok is limiting this account for posting too often. Wait a while and try again.",
        "spam_risk_user_banned_from_posting" =>
            "TikTok has temporarily blocked this account from posting. Try again later.",
        "spam_risk" =>
            "TikTok flagged this as potential spam. Wait a while and try again.",
        "rate_limit_exceeded" =>
            "Too many requests to TikTok right now. Wait a minute and try again.",
        "file_format_check_failed" =>
            "TikTok rejected the video format. Try re-exporting the clip.",
        "video_pull_failed" | "video_pull_url_invalid" =>
            "TikTok couldn't fetch the video. Re-export the clip and try again.",
        "privacy_level_option_mismatch" =>
            "The selected privacy setting isn't available for your account. Pick a different audience.",
        _ => "",
    };
    if hint.is_empty() {
        if message.trim().is_empty() {
            format!("TikTok error: {}", code)
        } else {
            format!("{} (TikTok: {})", message, code)
        }
    } else {
        hint.to_string()
    }
}

/// Pull TikTok's `error.code`/`error.message` out of a raw response body and run
/// it through [`friendly_tiktok_error`]. Falls back to status + body when the
/// body isn't the expected error shape.
fn friendly_tiktok_error_from_body(body: &str, status: impl std::fmt::Display) -> String {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        let code = json["error"]["code"].as_str().unwrap_or("");
        if !code.is_empty() && code != "ok" {
            let message = json["error"]["message"].as_str().unwrap_or("");
            return friendly_tiktok_error(code, message);
        }
    }
    format!("TikTok request failed ({}): {}", status, body)
}

async fn do_upload_net<F>(
    access_token: &str,
    title: &str,
    description: &str,
    visibility: &str,
    file_path: &str,
    disable_comment: bool,
    disable_duet: bool,
    disable_stitch: bool,
    brand_organic: bool,
    branded_content: bool,
    clip_id: &str,
    on_initialized: F,
) -> Result<(String, UploadResultStatus), AppError>
where
    F: FnOnce(&str) -> Result<(), AppError>,
{
    let client = reqwest::Client::new();
    let mut file = tokio::fs::File::open(file_path)
        .await
        .map_err(|e| AppError::Unknown(format!("Failed to open export file: {}", e)))?;
    let total_size = file
        .metadata()
        .await
        .map_err(|e| AppError::Unknown(format!("Failed to inspect export file: {}", e)))?
        .len();
    if total_size == 0 {
        return Err(AppError::NotFound(
            "Export file is empty; re-export the clip".into(),
        ));
    }

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
            requested,
            first
        );
        first.as_str()
    } else {
        "SELF_ONLY" // absolute fallback
    };

    log::info!(
        "TikTok upload: file_size={}, privacy_level={}, available_options={:?}",
        total_size,
        privacy_level,
        privacy_options
    );

    // 1. Determine chunk sizing
    //    TikTok Content Posting API rules:
    //    - Files ≤ 64 MB: upload as single chunk (chunk_size = video_size, count = 1)
    //    - Files > 64 MB: split into 10 MB chunks (each 5–64 MB allowed, final up to 128 MB)
    let (actual_chunk_size, chunk_count) = if total_size <= SINGLE_CHUNK_LIMIT as u64 {
        (total_size, 1_u64)
    } else {
        let chunk_size = UPLOAD_CHUNK_SIZE as u64;
        let count = (total_size + chunk_size - 1) / chunk_size;
        (chunk_size, count)
    };

    // 2. Init upload — tells TikTok we want to upload a video
    // TikTok's "title" field is the full video caption (description + hashtags).
    // Use description if available (already has hashtags appended by frontend),
    // fall back to title if description is empty.
    let caption = if description.trim().is_empty() {
        title
    } else {
        description
    };

    let init_body = serde_json::json!({
        "post_info": {
            "title": caption,
            "privacy_level": privacy_level,
            "disable_duet": disable_duet,
            "disable_comment": disable_comment,
            "disable_stitch": disable_stitch,
            "brand_content_toggle": branded_content,
            "brand_organic_toggle": brand_organic,
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
        return Err(AppError::Api(friendly_tiktok_error_from_body(
            &init_body_text,
            init_status,
        )));
    }

    let init_result: PublishInitResponse = serde_json::from_str(&init_body_text).map_err(|e| {
        AppError::Api(format!(
            "Failed to parse TikTok init response: {} — body: {}",
            e, init_body_text
        ))
    })?;

    if let Some(ref err) = init_result.error {
        if err.code != "ok" {
            return Err(AppError::Api(friendly_tiktok_error(
                &err.code,
                &err.message,
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
    on_initialized(&publish_id)?;

    log::info!("TikTok upload URL obtained, publish_id={}", publish_id);

    // 3. Upload file in chunks via PUT
    let mut offset: u64 = 0;
    let mut chunk_idx: u64 = 0;

    while offset < total_size {
        let end = std::cmp::min(offset + actual_chunk_size, total_size);
        let chunk_len = (end - offset) as usize;
        let mut chunk = vec![0_u8; chunk_len];
        file.read_exact(&mut chunk)
            .await
            .map_err(|e| AppError::Unknown(format!("Failed to read export file: {}", e)))?;

        let content_range = format!("bytes {}-{}/{}", offset, end - 1, total_size);

        log::info!(
            "TikTok uploading chunk {}/{}: Content-Range: {}",
            chunk_idx + 1,
            chunk_count,
            content_range
        );

        let chunk_resp = client
            .put(&upload_url)
            .header("Content-Range", &content_range)
            .header("Content-Length", chunk_len.to_string())
            .header("Content-Type", "video/mp4")
            .body(chunk)
            .send()
            .await?;

        let status = chunk_resp.status().as_u16();

        if status >= 200 && status < 300 {
            // Chunk accepted
            offset = end;
            chunk_idx += 1;
            log::info!(
                "TikTok upload chunk {}/{} complete (HTTP {})",
                chunk_idx,
                chunk_count,
                status
            );
            let pct = ((chunk_idx * 100) / chunk_count.max(1)).min(100) as u8;
            crate::social::emit_upload_status("tiktok", clip_id, "uploading", Some(pct));
            continue;
        }

        let body = chunk_resp.text().await.unwrap_or_default();
        log::error!("TikTok chunk upload failed (HTTP {}): {}", status, body);
        return Err(AppError::Api(friendly_tiktok_error_from_body(
            &body, status,
        )));
    }

    log::info!(
        "TikTok upload complete — all {} chunks sent. Polling for video URL...",
        chunk_count
    );

    // Poll the publish status endpoint to resolve the real outcome.
    // TikTok needs time to process the upload before returning a post ID.
    crate::social::emit_upload_status("tiktok", clip_id, "processing", None);
    let status = poll_publish_status(&client, access_token, &publish_id).await;

    Ok((publish_id, status))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PublishPollResult {
    Processing,
    Complete {
        video_url: Option<String>,
        platform_video_id: Option<String>,
    },
    Failed {
        error: String,
    },
}

fn post_id_from_status_data(data: &serde_json::Value) -> Option<String> {
    let value = data
        .get("publicaly_available_post_id")
        .or_else(|| data.get("publicly_available_post_id"))?;
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.as_u64().map(|number| number.to_string()))
        .or_else(|| value.as_i64().map(|number| number.to_string()))
        .or_else(|| {
            value
                .as_array()
                .and_then(|items| items.first())
                .and_then(|item| {
                    item.as_str()
                        .map(str::to_string)
                        .or_else(|| item.as_u64().map(|number| number.to_string()))
                })
        })
        .filter(|id| !id.is_empty() && id != "0")
}

fn publish_poll_result_from_json(json: &serde_json::Value) -> Result<PublishPollResult, AppError> {
    let code = json["error"]["code"].as_str().unwrap_or("ok");
    if code != "ok" {
        let message = json["error"]["message"].as_str().unwrap_or("");
        return Err(AppError::Api(friendly_tiktok_error(code, message)));
    }

    let data = &json["data"];
    match data["status"].as_str().unwrap_or("") {
        "PUBLISH_COMPLETE" => {
            let platform_video_id = post_id_from_status_data(data);
            let video_url = platform_video_id
                .as_ref()
                .map(|id| format!("https://www.tiktok.com/video/{}", id));
            Ok(PublishPollResult::Complete {
                video_url,
                platform_video_id,
            })
        }
        "FAILED" => {
            let reason = data["fail_reason"].as_str().unwrap_or("unknown");
            Ok(PublishPollResult::Failed {
                error: format!("TikTok rejected the post: {}", reason),
            })
        }
        _ => Ok(PublishPollResult::Processing),
    }
}

async fn fetch_publish_status_with_client(
    client: &reqwest::Client,
    access_token: &str,
    publish_id: &str,
) -> Result<PublishPollResult, AppError> {
    let resp = client
        .post(TIKTOK_PUBLISH_STATUS_URL)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "publish_id": publish_id }))
        .send()
        .await
        .map_err(|e| AppError::Api(format!("TikTok publish status network error: {}", e)))?;

    let http_status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    if !http_status.is_success() {
        return Err(AppError::Api(friendly_tiktok_error_from_body(
            &body_text,
            http_status,
        )));
    }

    let json: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|e| AppError::Api(format!("TikTok publish status parse error: {}", e)))?;
    publish_poll_result_from_json(&json)
}

pub(crate) async fn fetch_publish_status(
    access_token: &str,
    publish_id: &str,
) -> Result<PublishPollResult, AppError> {
    fetch_publish_status_with_client(&reqwest::Client::new(), access_token, publish_id).await
}

/// Poll briefly for immediate feedback. A timeout remains `Processing`; the
/// scheduler reconciles the persisted publish ID in the background.
async fn poll_publish_status(
    client: &reqwest::Client,
    access_token: &str,
    publish_id: &str,
) -> UploadResultStatus {
    const MAX_ATTEMPTS: u32 = 6;
    const POLL_INTERVAL: Duration = Duration::from_secs(5);

    for attempt in 1..=MAX_ATTEMPTS {
        tokio::time::sleep(POLL_INTERVAL).await;
        match fetch_publish_status_with_client(client, access_token, publish_id).await {
            Ok(PublishPollResult::Complete {
                video_url,
                platform_video_id,
            }) => {
                return UploadResultStatus::Complete {
                    video_url,
                    platform_video_id,
                };
            }
            Ok(PublishPollResult::Failed { error }) => {
                return UploadResultStatus::Failed { error };
            }
            Ok(PublishPollResult::Processing) => {
                log::info!(
                    "TikTok publish status poll {}/{} is still processing",
                    attempt,
                    MAX_ATTEMPTS
                );
            }
            Err(error) => {
                log::warn!(
                    "TikTok publish status poll {}/{} failed: {}",
                    attempt,
                    MAX_ATTEMPTS,
                    error
                );
            }
        }
    }

    log::info!(
        "TikTok publish {} remains processing after the foreground poll",
        publish_id
    );
    UploadResultStatus::Processing
}

#[cfg(test)]
mod error_message_tests {
    use super::*;

    #[test]
    fn oauth_scopes_match_the_approved_live_app() {
        assert_eq!(SCOPES, "user.info.basic,video.publish,video.upload");
        assert!(!video_stats_enabled());
    }

    #[test]
    fn maps_audit_code_to_private_account_guidance() {
        let msg =
            friendly_tiktok_error("unaudited_client_can_only_post_to_private_accounts", "raw");
        assert!(msg.contains("Only me (private)"), "got: {msg}");
        assert!(msg.contains("approves"), "got: {msg}");
        assert!(
            !msg.contains("unaudited_client"),
            "should not leak raw code: {msg}"
        );
    }

    #[test]
    fn maps_expired_token_to_reconnect() {
        let msg = friendly_tiktok_error("access_token_invalid", "");
        assert!(msg.to_lowercase().contains("reconnect"), "got: {msg}");
    }

    #[test]
    fn unknown_code_keeps_message_and_code() {
        assert_eq!(
            friendly_tiktok_error("some_new_code", "Something broke"),
            "Something broke (TikTok: some_new_code)"
        );
    }

    #[test]
    fn unknown_code_empty_message_shows_code() {
        assert_eq!(
            friendly_tiktok_error("some_new_code", ""),
            "TikTok error: some_new_code"
        );
    }

    #[test]
    fn from_body_parses_tiktok_error_shape() {
        let body = r#"{"error":{"code":"access_token_invalid","message":"bad"}}"#;
        let msg = friendly_tiktok_error_from_body(body, 401);
        assert!(msg.to_lowercase().contains("reconnect"), "got: {msg}");
    }

    #[test]
    fn from_body_falls_back_on_non_error_body() {
        let msg = friendly_tiktok_error_from_body("not json", 500);
        assert!(msg.contains("500"), "got: {msg}");
        assert!(msg.contains("not json"), "got: {msg}");
    }

    #[test]
    fn completed_private_post_does_not_require_a_public_video_id() {
        let json = serde_json::json!({
            "data": { "status": "PUBLISH_COMPLETE" },
            "error": { "code": "ok", "message": "" }
        });

        assert_eq!(
            publish_poll_result_from_json(&json).unwrap(),
            PublishPollResult::Complete {
                video_url: None,
                platform_video_id: None,
            }
        );
    }

    #[test]
    fn completed_post_accepts_tiktok_post_id_spellings_and_shapes() {
        let corrected = serde_json::json!({
            "data": {
                "status": "PUBLISH_COMPLETE",
                "publicly_available_post_id": "12345"
            },
            "error": { "code": "ok" }
        });
        let legacy = serde_json::json!({
            "data": {
                "status": "PUBLISH_COMPLETE",
                "publicaly_available_post_id": [67890]
            },
            "error": { "code": "ok" }
        });

        assert_eq!(
            publish_poll_result_from_json(&corrected).unwrap(),
            PublishPollResult::Complete {
                video_url: Some("https://www.tiktok.com/video/12345".to_string()),
                platform_video_id: Some("12345".to_string()),
            }
        );
        assert_eq!(
            publish_poll_result_from_json(&legacy).unwrap(),
            PublishPollResult::Complete {
                video_url: Some("https://www.tiktok.com/video/67890".to_string()),
                platform_video_id: Some("67890".to_string()),
            }
        );
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

// ── Stats ────────────────────────────────────────────────────────────────

/// View + like counts for a single TikTok video.
#[derive(Debug, Clone)]
pub struct VideoStats {
    pub view_count: Option<i64>,
    pub like_count: Option<i64>,
    pub share_url: Option<String>,
}

/// Extract a TikTok video ID from the final `video_url` we stored.
/// Handles both canonical formats:
///   https://www.tiktok.com/@user/video/7296847283472837373
///   https://m.tiktok.com/v/7296847283472837373.html
pub fn extract_video_id(url: &str) -> Option<String> {
    if let Some(idx) = url.find("/video/") {
        let rest = &url[idx + "/video/".len()..];
        let id: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !id.is_empty() { return Some(id) }
    }
    if let Some(idx) = url.find("/v/") {
        let rest = &url[idx + "/v/".len()..];
        let id: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !id.is_empty() { return Some(id) }
    }
    None
}

fn valid_access_token(conn: &Connection) -> Result<Option<String>, AppError> {
    let expiry = db::get_setting(conn, "tiktok_token_expiry")
        .map_err(|e| AppError::Database(e.to_string()))?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    if chrono::Utc::now().timestamp() >= expiry - 60 {
        return Ok(None);
    }
    db::get_setting(conn, "tiktok_access_token").map_err(|e| AppError::Database(e.to_string()))
}

async fn refresh_access_token(db_conn: &crate::DbConn, force: bool) -> Result<String, AppError> {
    if !force {
        let conn = db_conn
            .lock()
            .map_err(|e| AppError::Database(format!("DB lock: {}", e)))?;
        if let Some(token) = valid_access_token(&conn)? {
            return Ok(token);
        }
    }

    let _refresh_guard = TIKTOK_REFRESH_MUTEX.lock().await;
    let refresh_tok = {
        let conn = db_conn
            .lock()
            .map_err(|e| AppError::Database(format!("DB lock: {}", e)))?;
        if !force {
            if let Some(token) = valid_access_token(&conn)? {
                return Ok(token);
            }
        }
        db::get_setting(&conn, "tiktok_refresh_token")
            .map_err(|e| AppError::Database(e.to_string()))?
            .ok_or_else(|| AppError::Api("No TikTok refresh token; please reconnect.".into()))?
    };

    let new_tokens = match do_refresh_token_net(&refresh_tok).await {
        Err(AppError::AuthExpired(message)) => {
            if let Ok(conn) = db_conn.lock() {
                let _ = db::delete_settings_for_platform(&conn, "tiktok");
            }
            return Err(AppError::AuthExpired(message));
        }
        other => other?,
    };
    let new_expiry = chrono::Utc::now().timestamp() + new_tokens.expires_in as i64;
    let new_refresh_expiry = chrono::Utc::now().timestamp() + new_tokens.refresh_expires_in as i64;
    let conn = db_conn
        .lock()
        .map_err(|e| AppError::Database(format!("DB lock: {}", e)))?;
    db::save_setting(&conn, "tiktok_access_token", &new_tokens.access_token)
        .map_err(|e| AppError::Database(e.to_string()))?;
    db::save_setting(&conn, "tiktok_token_expiry", &new_expiry.to_string())
        .map_err(|e| AppError::Database(e.to_string()))?;
    db::save_setting(&conn, "tiktok_refresh_token", &new_tokens.refresh_token)
        .map_err(|e| AppError::Database(e.to_string()))?;
    if new_tokens.refresh_expires_in > 0 {
        db::save_setting(
            &conn,
            "tiktok_refresh_expiry",
            &new_refresh_expiry.to_string(),
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
    }
    Ok(new_tokens.access_token)
}

/// Return a valid TikTok bearer token, serializing refreshes per platform.
pub async fn ensure_fresh_access_token(db_conn: &crate::DbConn) -> Result<String, AppError> {
    refresh_access_token(db_conn, false).await
}

/// Fetch a specific TikTok video via the Display API query endpoint.
pub async fn fetch_video_stats(access_token: &str, video_id: &str) -> Result<VideoStats, AppError> {
    let url =
        "https://open.tiktokapis.com/v2/video/query/?fields=id,view_count,like_count,share_url";
    let body = serde_json::json!({
        "filters": { "video_ids": [video_id] }
    });
    let resp = reqwest::Client::new()
        .post(url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Api(format!("TikTok stats network: {}", e)))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Api(format!("TikTok stats {}: {}", status, body)));
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Api(format!("TikTok stats parse: {}", e)))?;
    let video = json["data"]["videos"].as_array().and_then(|videos| {
        videos
            .iter()
            .find(|video| video["id"].as_str() == Some(video_id))
    });
    match video {
        None => Ok(VideoStats {
            view_count: None,
            like_count: None,
            share_url: None,
        }),
        Some(v) => Ok(VideoStats {
            view_count: v.get("view_count").and_then(|x| x.as_i64()),
            like_count: v.get("like_count").and_then(|x| x.as_i64()),
            share_url: v
                .get("share_url")
                .and_then(|x| x.as_str())
                .map(str::to_string),
        }),
    }
}
