//! YouTube platform adapter — OAuth 2.0 + resumable video upload.
//!
//! Implements the full `PlatformAdapter` trait for YouTube:
//! - Google OAuth 2.0 authorization code flow (offline access)
//! - Channel info fetch via YouTube Data API v3
//! - Resumable (chunked) video upload with 5 MB chunks
//! - Automatic token refresh via refresh_token grant
//!
//! Database guards are kept out of network awaits so OAuth and uploads do not
//! block unrelated app state reads.

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

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const YOUTUBE_API_URL: &str = "https://www.googleapis.com/youtube/v3";
const YOUTUBE_UPLOAD_URL: &str = "https://www.googleapis.com/upload/youtube/v3/videos";

const CALLBACK_PORT: u16 = 17386;
const REDIRECT_URI: &str = "http://localhost:17386";
const SCOPES: &str =
    "https://www.googleapis.com/auth/youtube.upload https://www.googleapis.com/auth/youtube.readonly";

const AUTH_TIMEOUT_SECS: u64 = 120;

/// 5 MB per chunk for resumable uploads.
const UPLOAD_CHUNK_SIZE: usize = 5 * 1024 * 1024;

static YOUTUBE_REFRESH_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn youtube_proxy_error(error: String) -> AppError {
    if error.contains("invalid_client") {
        AppError::Api(
            "ClipGoblin's YouTube OAuth configuration is invalid. Update the auth Worker's YOUTUBE_CLIENT_SECRET for the configured Google client ID, deploy the Worker, then reconnect YouTube in Settings."
                .into(),
        )
    } else {
        AppError::Api(error)
    }
}

/// Embedded YouTube OAuth client ID — safe to ship in the binary since OAuth
/// client IDs are public identifiers (the actual client *secret* stays in the
/// Cloudflare Worker). Same value already lives in `worker/wrangler.toml`.
/// Override with `YOUTUBE_CLIENT_ID` env var for development.
const DEFAULT_YOUTUBE_CLIENT_ID: &str =
    "963785158873-iuutl54610isuch1mcaqnbsoc90acrnb.apps.googleusercontent.com";

static CLIENT_ID: Lazy<String> = Lazy::new(|| {
    match std::env::var("YOUTUBE_CLIENT_ID") {
        Ok(val) if !val.is_empty() => {
            log::info!("YouTube CLIENT_ID loaded from env (len={})", val.len());
            val
        }
        _ => {
            log::info!("Using embedded YouTube CLIENT_ID");
            DEFAULT_YOUTUBE_CLIENT_ID.to_string()
        }
    }
});

fn client_id() -> &'static str {
    &CLIENT_ID
}

// ═══════════════════════════════════════════════════════════════════
//  OAuth CSRF state
// ═══════════════════════════════════════════════════════════════════

static OAUTH_STATE: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));

// ═══════════════════════════════════════════════════════════════════
//  Internal response types
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
}

struct ChannelInfo {
    id: String,
    name: String,
}

// ═══════════════════════════════════════════════════════════════════
//  Public callback-server helpers (called from lib.rs commands)
// ═══════════════════════════════════════════════════════════════════

/// Bind the OAuth callback server on port 17386.
pub fn bind_callback_server() -> Result<TcpListener, AppError> {
    TcpListener::bind(format!("127.0.0.1:{}", CALLBACK_PORT))
        .or_else(|_| TcpListener::bind(format!("[::1]:{}", CALLBACK_PORT)))
        .map_err(|e| {
            AppError::Api(format!(
                "Failed to bind YouTube callback server on port {}: {}",
                CALLBACK_PORT, e
            ))
        })
}

/// Wait for the Google OAuth callback on an already-bound listener.
/// Times out after `AUTH_TIMEOUT_SECS` seconds.
pub fn wait_for_auth_code(listener: TcpListener) -> Result<String, AppError> {
    listener
        .set_nonblocking(true)
        .map_err(|e| AppError::Api(format!("Failed to configure callback server: {}", e)))?;

    let deadline = Instant::now() + Duration::from_secs(AUTH_TIMEOUT_SECS);

    loop {
        if Instant::now() > deadline {
            return Err(AppError::Api(
                "YouTube login timed out after 2 minutes. Please try again.".into(),
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
        if !path.starts_with("/?") && path != "/" {
            let resp =
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
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

        // Handle Google error callback
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
                "Google authorization denied: {}",
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
                "YouTube connected! You can close this tab and return to ClipGoblin.",
            );
            return Ok(code.to_string());
        }

        // Callback path but no code or error
        send_html_response(
            &stream,
            false,
            "No authorization code received from Google.",
        );
        return Err(AppError::Api(
            "YouTube callback did not contain an authorization code.".into(),
        ));
    }
}

// ═══════════════════════════════════════════════════════════════════
//  PlatformAdapter
// ═══════════════════════════════════════════════════════════════════

pub struct YouTubeAdapter;

#[async_trait::async_trait(?Send)]
impl PlatformAdapter for YouTubeAdapter {
    fn platform_id(&self) -> &'static str {
        "youtube"
    }

    fn is_ready(&self, db: &Connection) -> Result<bool, AppError> {
        let has_access = db::get_setting(db, "youtube_access_token")
            .map_err(|e| AppError::Database(e.to_string()))?
            .is_some();
        let has_refresh = db::get_setting(db, "youtube_refresh_token")
            .map_err(|e| AppError::Database(e.to_string()))?
            .is_some();
        Ok(has_access && has_refresh)
    }

    async fn start_auth(&self) -> Result<String, AppError> {
        // Generate cryptographically random CSRF state via UUID v4
        let state = uuid::Uuid::new_v4().to_string();

        if let Ok(mut guard) = OAUTH_STATE.lock() {
            *guard = state.clone();
        }

        let url = format!(
            "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent&state={}",
            GOOGLE_AUTH_URL,
            urlencoding::encode(client_id()),
            urlencoding::encode(REDIRECT_URI),
            urlencoding::encode(SCOPES),
            urlencoding::encode(&state),
        );

        Ok(url)
    }

    async fn handle_callback(
        &self,
        db: &crate::DbConn,
        code: &str,
    ) -> Result<ConnectedAccount, AppError> {
        let code_owned = code.to_string();
        let (tokens, channel) = do_handle_callback_net(&code_owned).await?;
        let expiry = chrono::Utc::now().timestamp() + tokens.expires_in as i64;

        let conn = db
            .lock()
            .map_err(|e| AppError::Database(format!("DB lock: {}", e)))?;
        db::save_setting(&conn, "youtube_access_token", &tokens.access_token)
            .map_err(|e| AppError::Database(e.to_string()))?;
        if let Some(ref rt) = tokens.refresh_token {
            db::save_setting(&conn, "youtube_refresh_token", rt)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }
        db::save_setting(&conn, "youtube_token_expiry", &expiry.to_string())
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(&conn, "youtube_channel_name", &channel.name)
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(&conn, "youtube_channel_id", &channel.id)
            .map_err(|e| AppError::Database(e.to_string()))?;

        let now = chrono::Utc::now().to_rfc3339();
        Ok(ConnectedAccount {
            platform: "youtube".into(),
            account_name: channel.name,
            account_id: channel.id,
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
        validate_export_file(Some(file_path))?;

        let claim = {
            let conn = db
                .lock()
                .map_err(|e| AppError::Database(format!("DB lock: {}", e)))?;
            db::begin_upload(&conn, &meta.clip_id, "youtube", meta.force)
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
        let tags = meta.tags.clone();
        let visibility = match meta.visibility.as_str() {
            "public" | "private" | "unlisted" => meta.visibility.clone(),
            _ => "private".to_string(),
        };

        let upload_result = async {
            let access_token = ensure_fresh_access_token(db).await?;
            do_upload_net(
                &access_token,
                &title,
                &description,
                &tags,
                &visibility,
                file_path,
            )
            .await
        }
        .await;

        match upload_result {
            Ok((video_id, video_url)) => {
                let conn = db
                    .lock()
                    .map_err(|e| AppError::Database(format!("DB lock: {}", e)))?;
                db::mark_upload_complete(
                    &conn,
                    &meta.clip_id,
                    "youtube",
                    Some(&video_url),
                    Some(&video_id),
                    Some(&video_id),
                )
                .map_err(|e| AppError::Database(e.to_string()))?;
                Ok(UploadResult {
                    status: UploadResultStatus::Complete {
                        video_url: Some(video_url),
                        platform_video_id: Some(video_id.clone()),
                    },
                    job_id: video_id,
                })
            }
            Err(error) => {
                if let Ok(conn) = db.lock() {
                    let _ =
                        db::mark_upload_failed(&conn, &meta.clip_id, "youtube", &error.to_string());
                }
                Err(error)
            }
        }
    }

    fn disconnect(&self, db: &Connection) -> Result<(), AppError> {
        db::delete_settings_for_platform(db, "youtube")
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn get_account(&self, db: &Connection) -> Result<Option<ConnectedAccount>, AppError> {
        let channel_name = db::get_setting(db, "youtube_channel_name")
            .map_err(|e| AppError::Database(e.to_string()))?;
        let channel_id = db::get_setting(db, "youtube_channel_id")
            .map_err(|e| AppError::Database(e.to_string()))?;

        match (channel_name, channel_id) {
            (Some(name), Some(id)) => Ok(Some(ConnectedAccount {
                platform: "youtube".into(),
                account_name: name,
                account_id: id,
                connected_at: String::new(), // Not stored separately for retrieval
            })),
            _ => Ok(None),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Async network helpers (no &Connection — safe for Send futures)
// ═══════════════════════════════════════════════════════════════════

/// Exchange authorization code for tokens + fetch channel info.
/// Returns `(TokenResponse, ChannelInfo)`.
async fn do_handle_callback_net(
    code: &str,
) -> Result<(TokenResponse, ChannelInfo), AppError> {
    let tokens = exchange_code(code).await?;
    let channel = fetch_channel_info(&tokens.access_token).await?;
    Ok((tokens, channel))
}

/// Refresh YouTube tokens via auth proxy.
async fn do_refresh_token_net(refresh_tok: &str) -> Result<TokenResponse, AppError> {
    log::info!("[YouTube Refresh] Refreshing token via auth proxy");

    let proxy = AuthProxy::new()
        .map_err(|e| AppError::Api(format!("Auth proxy init failed: {}", e)))?;
    let proxy_resp = proxy.youtube_refresh(refresh_tok).await
        .map_err(youtube_proxy_error)?;

    if let Some(err) = proxy_resp.error {
        let desc = proxy_resp.error_description.unwrap_or_default();
        // invalid_grant = the refresh token itself is revoked/expired; it can't
        // be refreshed silently (OAuth requires re-consent). Surface a distinct
        // AuthExpired error so callers wipe the dead tokens and the UI can guide
        // a clean reconnect instead of showing a cryptic failure.
        if err == "invalid_grant" {
            return Err(AppError::AuthExpired(
                "Your YouTube session has expired. Please reconnect your YouTube account in Settings.".into(),
            ));
        }
        return Err(AppError::Api(format!(
            "YouTube token refresh failed: {} — {}",
            err, desc
        )));
    }

    let access_token = proxy_resp.access_token
        .ok_or_else(|| AppError::Api("Proxy response missing access_token".into()))?;

    Ok(TokenResponse {
        access_token,
        expires_in: proxy_resp.expires_in.unwrap_or(0),
        refresh_token: proxy_resp.refresh_token,
    })
}

async fn youtube_refresh_or_clear(
    db: &crate::DbConn,
    refresh_tok: &str,
) -> Result<TokenResponse, AppError> {
    match do_refresh_token_net(refresh_tok).await {
        Err(AppError::AuthExpired(msg)) => {
            log::warn!(
                "[YouTube] refresh token rejected (invalid_grant); clearing stale connection"
            );
            if let Ok(conn) = db.lock() {
                let _ = db::delete_settings_for_platform(&conn, "youtube");
            }
            Err(AppError::AuthExpired(msg))
        }
        other => other,
    }
}

/// Initiate a resumable upload and stream the file in bounded chunks.
/// Returns `(video_id, video_url)`.
async fn do_upload_net(
    access_token: &str,
    title: &str,
    description: &str,
    tags: &[String],
    visibility: &str,
    file_path: &str,
) -> Result<(String, String), AppError> {
    let snippet = serde_json::json!({
        "snippet": {
            "title": title,
            "description": description,
            "tags": tags,
            "categoryId": "20"
        },
        "status": {
            "privacyStatus": visibility,
            "selfDeclaredMadeForKids": false
        }
    });

    let client = reqwest::Client::new();

    // Initiate resumable upload
    let init_resp = client
        .post(format!(
            "{}?uploadType=resumable&part=snippet,status",
            YOUTUBE_UPLOAD_URL
        ))
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json; charset=UTF-8")
        .header("X-Upload-Content-Type", "video/*")
        .json(&snippet)
        .send()
        .await?;

    if !init_resp.status().is_success() {
        let status = init_resp.status();
        let body = init_resp.text().await.unwrap_or_default();
        return Err(AppError::Api(format!(
            "YouTube upload init failed ({}): {}",
            status, body
        )));
    }

    let upload_url = init_resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::Api("YouTube did not return a resumable upload URL.".into()))?;

    let mut file = tokio::fs::File::open(file_path)
        .await
        .map_err(|e| AppError::Unknown(format!("Failed to open export file: {}", e)))?;
    let total = file
        .metadata()
        .await
        .map_err(|e| AppError::Unknown(format!("Failed to inspect export file: {}", e)))?
        .len();
    if total == 0 {
        return Err(AppError::NotFound(
            "Export file is empty; re-export the clip".into(),
        ));
    }

    let mut offset: u64 = 0;
    let mut video_id = String::new();

    while offset < total {
        let chunk_len = std::cmp::min(UPLOAD_CHUNK_SIZE as u64, total - offset) as usize;
        let mut chunk = vec![0_u8; chunk_len];
        file.read_exact(&mut chunk)
            .await
            .map_err(|e| AppError::Unknown(format!("Failed to read export file: {}", e)))?;
        let end = offset + chunk_len as u64;

        let content_range = format!("bytes {}-{}/{}", offset, end - 1, total);

        let chunk_resp = client
            .put(&upload_url)
            .header("Content-Range", &content_range)
            .header("Content-Length", chunk_len.to_string())
            .body(chunk)
            .send()
            .await?;

        let status = chunk_resp.status().as_u16();

        if status == 308 {
            // Chunk accepted, continue
            offset = end;
            continue;
        }

        if status == 200 || status == 201 {
            // Upload complete — extract video ID
            let body: serde_json::Value = chunk_resp.json().await?;
            video_id = body["id"].as_str().unwrap_or("").to_string();
            break;
        }

        // Unexpected status
        let body = chunk_resp.text().await.unwrap_or_default();
        return Err(AppError::Api(format!(
            "YouTube chunk upload failed ({}): {}",
            status, body
        )));
    }

    if video_id.is_empty() {
        return Err(AppError::Api(
            "YouTube upload completed but no video ID was returned.".into(),
        ));
    }

    let video_url = format!("https://youtu.be/{}", video_id);
    Ok((video_id, video_url))
}

// ═══════════════════════════════════════════════════════════════════
//  Private helpers
// ═══════════════════════════════════════════════════════════════════

/// Exchange an authorization code for access + refresh tokens via auth proxy.
async fn exchange_code(code: &str) -> Result<TokenResponse, AppError> {
    log::info!("[YouTube Token] Exchanging code via auth proxy");

    let proxy = AuthProxy::new()
        .map_err(|e| AppError::Api(format!("Auth proxy init failed: {}", e)))?;
    let proxy_resp = proxy.youtube_token_exchange(code, REDIRECT_URI).await
        .map_err(youtube_proxy_error)?;

    if let Some(err) = proxy_resp.error {
        let desc = proxy_resp.error_description.unwrap_or_default();
        return Err(AppError::Api(format!(
            "YouTube token exchange failed: {} — {}",
            err, desc
        )));
    }

    let access_token = proxy_resp.access_token
        .ok_or_else(|| AppError::Api("Proxy response missing access_token".into()))?;

    Ok(TokenResponse {
        access_token,
        expires_in: proxy_resp.expires_in.unwrap_or(0),
        refresh_token: proxy_resp.refresh_token,
    })
}

/// Fetch the authenticated user's YouTube channel name and ID.
async fn fetch_channel_info(access_token: &str) -> Result<ChannelInfo, AppError> {
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/channels?part=snippet&mine=true", YOUTUBE_API_URL))
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Api(format!(
            "YouTube channel info fetch failed ({}): {}",
            status, body
        )));
    }

    let body: serde_json::Value = resp.json().await?;
    let items = body["items"]
        .as_array()
        .ok_or_else(|| AppError::Api("No YouTube channels found for this account.".into()))?;

    let channel = items
        .first()
        .ok_or_else(|| AppError::Api("No YouTube channels found for this account.".into()))?;

    let id = channel["id"].as_str().unwrap_or("").to_string();
    let name = channel["snippet"]["title"]
        .as_str()
        .unwrap_or("Unknown Channel")
        .to_string();

    if id.is_empty() {
        return Err(AppError::Api("YouTube channel ID was empty.".into()));
    }

    Ok(ChannelInfo { id, name })
}

/// Send a styled HTML response through the TCP stream, matching the Twitch callback style.
fn send_html_response(stream: &std::net::TcpStream, success: bool, message: &str) {
    let (icon, color) = if success {
        ("&#10004;", "#8b5cf6") // checkmark, purple
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

/// Minimal HTML-escape to prevent XSS in error messages rendered in the callback page.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

// ── Stats ────────────────────────────────────────────────────────────────

/// View + like counts for a single uploaded video.
#[derive(Debug, Clone)]
pub struct VideoStats {
    pub view_count: Option<i64>,
    pub like_count: Option<i64>,
}

/// Extract a YouTube video ID from the upload's `video_url` column.
/// Handles:
///   https://youtu.be/abc123
///   https://www.youtube.com/watch?v=abc123
///   https://www.youtube.com/shorts/abc123
///   https://youtube.com/shorts/abc123?feature=...
pub fn extract_video_id(url: &str) -> Option<String> {
    if let Some(rest) = url.strip_prefix("https://youtu.be/") {
        let id: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        if !id.is_empty() { return Some(id) }
    }
    let lower = url.to_lowercase();
    if let Some(idx) = lower.find("/shorts/") {
        let rest = &url[idx + "/shorts/".len()..];
        let id: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        if !id.is_empty() { return Some(id) }
    }
    if let Some(idx) = lower.find("v=") {
        let rest = &url[idx + 2..];
        let id: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        if !id.is_empty() { return Some(id) }
    }
    None
}

fn valid_access_token(conn: &Connection) -> Result<Option<String>, AppError> {
    let expiry = db::get_setting(conn, "youtube_token_expiry")
        .map_err(|e| AppError::Database(e.to_string()))?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    if chrono::Utc::now().timestamp() >= expiry - 60 {
        return Ok(None);
    }
    db::get_setting(conn, "youtube_access_token").map_err(|e| AppError::Database(e.to_string()))
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

    let _refresh_guard = YOUTUBE_REFRESH_MUTEX.lock().await;
    let refresh_tok = {
        let conn = db_conn
            .lock()
            .map_err(|e| AppError::Database(format!("DB lock: {}", e)))?;
        if !force {
            if let Some(token) = valid_access_token(&conn)? {
                return Ok(token);
            }
        }
        db::get_setting(&conn, "youtube_refresh_token")
            .map_err(|e| AppError::Database(e.to_string()))?
            .ok_or_else(|| AppError::Api("No YouTube refresh token; please reconnect.".into()))?
    };

    let new_tokens = youtube_refresh_or_clear(db_conn, &refresh_tok).await?;
    let new_expiry = chrono::Utc::now().timestamp() + new_tokens.expires_in as i64;
    let conn = db_conn
        .lock()
        .map_err(|e| AppError::Database(format!("DB lock: {}", e)))?;
    db::save_setting(&conn, "youtube_access_token", &new_tokens.access_token)
        .map_err(|e| AppError::Database(e.to_string()))?;
    db::save_setting(&conn, "youtube_token_expiry", &new_expiry.to_string())
        .map_err(|e| AppError::Database(e.to_string()))?;
    if let Some(ref rt) = new_tokens.refresh_token {
        db::save_setting(&conn, "youtube_refresh_token", rt)
            .map_err(|e| AppError::Database(e.to_string()))?;
    }
    Ok(new_tokens.access_token)
}

/// Return a valid YouTube bearer token, serializing refreshes per platform.
pub async fn ensure_fresh_access_token(db_conn: &crate::DbConn) -> Result<String, AppError> {
    refresh_access_token(db_conn, false).await
}

/// Fetch view + like counts for a single YouTube video via the Data API.
/// Requires `youtube.readonly` scope (already in SCOPES).
/// Returns `Ok(VideoStats)` with `None` fields when the API returns an empty
/// result (deleted video) — only errors on network/auth failures.
pub async fn fetch_video_stats(access_token: &str, video_id: &str) -> Result<VideoStats, AppError> {
    let url = format!(
        "{}/videos?part=statistics&id={}",
        YOUTUBE_API_URL, video_id
    );
    let resp = reqwest::Client::new()
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| AppError::Api(format!("YouTube stats network: {}", e)))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Api(format!("YouTube stats {}: {}", status, body)));
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Api(format!("YouTube stats parse: {}", e)))?;
    let items = json["items"].as_array();
    let stats = items
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("statistics"));
    match stats {
        None => Ok(VideoStats { view_count: None, like_count: None }),
        Some(s) => {
            let parse_u64 = |v: &serde_json::Value| -> Option<i64> {
                v.as_str().and_then(|s| s.parse::<i64>().ok())
            };
            Ok(VideoStats {
                view_count: s.get("viewCount").and_then(parse_u64),
                like_count: s.get("likeCount").and_then(parse_u64),
            })
        }
    }
}
