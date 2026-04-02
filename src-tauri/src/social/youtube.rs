//! YouTube platform adapter — OAuth 2.0 + resumable video upload.
//!
//! Implements the full `PlatformAdapter` trait for YouTube:
//! - Google OAuth 2.0 authorization code flow (offline access)
//! - Channel info fetch via YouTube Data API v3
//! - Resumable (chunked) video upload with 5 MB chunks
//! - Automatic token refresh via refresh_token grant
//!
//! The trait uses `#[async_trait(?Send)]` because `rusqlite::Connection` is
//! `!Sync`, preventing `&Connection` from being held across `.await` points
//! in a `Send` future.

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

static CLIENT_ID: Lazy<String> = Lazy::new(|| {
    std::env::var("YOUTUBE_CLIENT_ID").unwrap_or_else(|_| {
        log::warn!("YOUTUBE_CLIENT_ID environment variable is not set — YouTube OAuth will fail");
        String::new()
    })
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
        db: &Connection,
        code: &str,
    ) -> Result<ConnectedAccount, AppError> {
        // --- async portion (no db reference used) ---
        let code_owned = code.to_string();
        let (tokens, channel) = do_handle_callback_net(&code_owned).await?;

        // --- sync: persist to db ---
        let expiry = chrono::Utc::now().timestamp() + tokens.expires_in as i64;

        db::save_setting(db, "youtube_access_token", &tokens.access_token)
            .map_err(|e| AppError::Database(e.to_string()))?;
        if let Some(ref rt) = tokens.refresh_token {
            db::save_setting(db, "youtube_refresh_token", rt)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }
        db::save_setting(db, "youtube_token_expiry", &expiry.to_string())
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(db, "youtube_channel_name", &channel.name)
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(db, "youtube_channel_id", &channel.id)
            .map_err(|e| AppError::Database(e.to_string()))?;

        let now = chrono::Utc::now().to_rfc3339();
        Ok(ConnectedAccount {
            platform: "youtube".into(),
            account_name: channel.name,
            account_id: channel.id,
            connected_at: now,
        })
    }

    async fn refresh_token(&self, db: &Connection) -> Result<(), AppError> {
        // --- sync: read refresh token from db ---
        let refresh_tok = db::get_setting(db, "youtube_refresh_token")
            .map_err(|e| AppError::Database(e.to_string()))?
            .ok_or_else(|| {
                AppError::Api("No YouTube refresh token found — please reconnect.".into())
            })?;

        // --- async: exchange with Google ---
        let tokens = do_refresh_token_net(&refresh_tok).await?;

        // --- sync: persist new tokens ---
        let expiry = chrono::Utc::now().timestamp() + tokens.expires_in as i64;
        db::save_setting(db, "youtube_access_token", &tokens.access_token)
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(db, "youtube_token_expiry", &expiry.to_string())
            .map_err(|e| AppError::Database(e.to_string()))?;

        // Google sometimes rotates refresh tokens
        if let Some(ref rt) = tokens.refresh_token {
            db::save_setting(db, "youtube_refresh_token", rt)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }

        Ok(())
    }

    async fn upload_video(
        &self,
        db: &Connection,
        file_path: &str,
        meta: &UploadMeta,
    ) -> Result<UploadResult, AppError> {
        // 1. Validate the export file (sync)
        validate_export_file(Some(file_path))?;

        // 2. Duplicate check (sync, unless force=true)
        if !meta.force {
            if let Some(existing) = db::get_upload_for_clip(db, &meta.clip_id, "youtube")
                .map_err(|e| AppError::Database(e.to_string()))?
            {
                return Ok(UploadResult {
                    status: UploadResultStatus::Duplicate {
                        existing_url: existing.video_url.unwrap_or_default(),
                    },
                    job_id: String::new(),
                });
            }
        }

        // 3. Ensure token is fresh — read expiry (sync), refresh if needed (async), read token (sync)
        let expiry_str = db::get_setting(db, "youtube_token_expiry")
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
            // Read refresh token (sync) then do network call (async) then save (sync)
            let refresh_tok = db::get_setting(db, "youtube_refresh_token")
                .map_err(|e| AppError::Database(e.to_string()))?
                .ok_or_else(|| {
                    AppError::Api("No YouTube refresh token found — please reconnect.".into())
                })?;

            let new_tokens = do_refresh_token_net(&refresh_tok).await?;

            let new_expiry = chrono::Utc::now().timestamp() + new_tokens.expires_in as i64;
            db::save_setting(db, "youtube_access_token", &new_tokens.access_token)
                .map_err(|e| AppError::Database(e.to_string()))?;
            db::save_setting(db, "youtube_token_expiry", &new_expiry.to_string())
                .map_err(|e| AppError::Database(e.to_string()))?;
            if let Some(ref rt) = new_tokens.refresh_token {
                db::save_setting(db, "youtube_refresh_token", rt)
                    .map_err(|e| AppError::Database(e.to_string()))?;
            }
        }

        let access_token = db::get_setting(db, "youtube_access_token")
            .map_err(|e| AppError::Database(e.to_string()))?
            .ok_or_else(|| AppError::Api("No YouTube access token — please reconnect.".into()))?;

        // Read file bytes (sync, before async upload)
        let file_bytes = std::fs::read(file_path)
            .map_err(|e| AppError::Unknown(format!("Failed to read export file: {}", e)))?;

        // Collect owned copies of metadata for the async portion
        let title = meta.title.clone();
        let description = meta.description.clone();
        let tags = meta.tags.clone();
        let visibility = match meta.visibility.as_str() {
            "public" | "private" | "unlisted" => meta.visibility.clone(),
            _ => "private".to_string(),
        };

        // 4-5. Initiate resumable upload + send chunks (async, no db ref)
        let (video_id, video_url) = do_upload_net(
            &access_token,
            &title,
            &description,
            &tags,
            &visibility,
            file_bytes,
        )
        .await?;

        // 6. Record in upload history (sync)
        db::upsert_upload(db, &meta.clip_id, "youtube", &video_url)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(UploadResult {
            status: UploadResultStatus::Complete { video_url },
            job_id: video_id,
        })
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
        .map_err(|e| AppError::Api(e))?;

    if let Some(err) = proxy_resp.error {
        let desc = proxy_resp.error_description.unwrap_or_default();
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

/// Initiate a resumable upload and send file bytes in chunks.
/// Returns `(video_id, video_url)`.
async fn do_upload_net(
    access_token: &str,
    title: &str,
    description: &str,
    tags: &[String],
    visibility: &str,
    file_bytes: Vec<u8>,
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
        .ok_or_else(|| {
            AppError::Api("YouTube did not return a resumable upload URL.".into())
        })?;

    // Send file in chunks
    let total = file_bytes.len();
    let mut offset: usize = 0;
    let mut video_id = String::new();

    while offset < total {
        let end = std::cmp::min(offset + UPLOAD_CHUNK_SIZE, total);
        let chunk = &file_bytes[offset..end];

        let content_range = format!("bytes {}-{}/{}", offset, end - 1, total);

        let chunk_resp = client
            .put(&upload_url)
            .header("Content-Range", &content_range)
            .header("Content-Length", chunk.len().to_string())
            .body(chunk.to_vec())
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
        .map_err(|e| AppError::Api(e))?;

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
