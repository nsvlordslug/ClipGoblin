# Social Publishing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow users to connect their YouTube account and upload exported clips directly from ClipGoblin, with TikTok/Instagram scaffolded for future implementation.

**Architecture:** Platform adapter trait (`PlatformAdapter`) in a `social/` module. YouTube implements the trait with app-owned OAuth 2.0 (localhost callback) and Data API v3 resumable uploads. TikTok/Instagram are stubs. Six generic Tauri commands dispatch to the correct adapter. Frontend uses a Zustand store backed by Tauri commands.

**Tech Stack:** Rust (reqwest, tokio, serde, rusqlite), Tauri 2, React + Zustand, YouTube Data API v3, OAuth 2.0 installed-app flow.

**Spec:** `docs/superpowers/specs/2026-03-27-social-publishing-design.md`

---

## File Map

### New files (backend)

| File | Responsibility |
|------|---------------|
| `src-tauri/src/social/mod.rs` | `PlatformAdapter` trait, shared types (`ConnectedAccount`, `UploadMeta`, `UploadResultStatus`), `get_adapter()` dispatcher, file validation helper |
| `src-tauri/src/social/youtube.rs` | YouTube OAuth flow (localhost callback, token exchange, channel fetch), token refresh, resumable upload, upload status polling |
| `src-tauri/src/social/tiktok.rs` | Stub — all methods return `NotSupported` error |
| `src-tauri/src/social/instagram.rs` | Stub — all methods return `NotSupported` error |

### Modified files (backend)

| File | Change |
|------|--------|
| `src-tauri/src/lib.rs` | Add `mod social;`, register 6 new Tauri commands, add command implementations that call into `social::get_adapter()` |
| `src-tauri/src/db.rs` | Add `upload_history` table migration, `insert_upload`, `get_upload`, `delete_upload_for_reupload` helpers |
| `src-tauri/src/error.rs` | Add `NotSupported(String)` variant |

### Modified files (frontend)

| File | Change |
|------|--------|
| `src/stores/platformStore.ts` | Full rewrite: Tauri-backed state, `load()` / `connect()` / `disconnect()` calling real commands |
| `src/components/ConnectedAccounts.tsx` | Rewrite: loading states, "Connected as X" display, coming-soon for TikTok/Instagram |
| `src/pages/Editor.tsx` | Upload button in ActionsBar (3 states), connect-then-upload flow, upload progress, duplicate warning |

---

## Task 1: Error type — add NotSupported variant

**Files:**
- Modify: `src-tauri/src/error.rs`

- [ ] **Step 1: Add NotSupported variant to AppError**

In `src-tauri/src/error.rs`, add the variant to the enum and update the match arms:

```rust
// In the enum definition, add after Unknown(String):
    NotSupported(String),
```

```rust
// In category():
    Self::NotSupported(_) => "not_supported",
```

```rust
// In detail():
    Self::NotSupported(s) => s,
```

```rust
// In Display impl (the fmt function):
    Self::NotSupported(s) => write!(f, "Not supported: {}", s),
```

- [ ] **Step 2: Verify it compiles**

Run: `cd "C:\Users\cereb\Desktop\Claude projects\clipviral\src-tauri" && cargo check 2>&1 | grep "^error"`
Expected: No output (clean compile).

- [ ] **Step 3: Commit**

```bash
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
git add src-tauri/src/error.rs
git commit -m "feat(social): add NotSupported error variant"
```

---

## Task 2: Database — upload_history table and helpers

**Files:**
- Modify: `src-tauri/src/db.rs`

- [ ] **Step 1: Add upload_history table migration**

In `src-tauri/src/db.rs`, find the migrations section (after the existing `ALTER TABLE` statements near line 110). Add:

```rust
    // Social publishing: upload history for duplicate detection
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS upload_history (
            id TEXT PRIMARY KEY,
            clip_id TEXT NOT NULL,
            platform TEXT NOT NULL,
            video_url TEXT,
            uploaded_at TEXT,
            UNIQUE(clip_id, platform)
        )"
    )?;
```

- [ ] **Step 2: Add UploadHistoryRow struct**

After the existing row structs (near `HighlightRow`), add:

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct UploadHistoryRow {
    pub id: String,
    pub clip_id: String,
    pub platform: String,
    pub video_url: Option<String>,
    pub uploaded_at: String,
}
```

- [ ] **Step 3: Add helper functions**

```rust
pub fn get_upload_for_clip(conn: &Connection, clip_id: &str, platform: &str) -> SqliteResult<Option<UploadHistoryRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, clip_id, platform, video_url, uploaded_at FROM upload_history WHERE clip_id = ?1 AND platform = ?2"
    )?;
    let mut rows = stmt.query_map(params![clip_id, platform], |row| {
        Ok(UploadHistoryRow {
            id: row.get(0)?,
            clip_id: row.get(1)?,
            platform: row.get(2)?,
            video_url: row.get(3)?,
            uploaded_at: row.get(4)?,
        })
    })?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

pub fn upsert_upload(conn: &Connection, clip_id: &str, platform: &str, video_url: &str) -> SqliteResult<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO upload_history (id, clip_id, platform, video_url, uploaded_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(clip_id, platform) DO UPDATE SET video_url = excluded.video_url, uploaded_at = excluded.uploaded_at",
        params![id, clip_id, platform, video_url, now],
    )?;
    Ok(())
}

pub fn delete_settings_for_platform(conn: &Connection, platform: &str) -> SqliteResult<()> {
    let prefixes = [
        format!("{}_access_token", platform),
        format!("{}_refresh_token", platform),
        format!("{}_token_expiry", platform),
        format!("{}_channel_name", platform),
        format!("{}_channel_id", platform),
    ];
    for key in &prefixes {
        conn.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
    }
    Ok(())
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cd "C:\Users\cereb\Desktop\Claude projects\clipviral\src-tauri" && cargo check 2>&1 | grep "^error"`
Expected: No output.

- [ ] **Step 5: Commit**

```bash
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
git add src-tauri/src/db.rs
git commit -m "feat(social): add upload_history table and DB helpers"
```

---

## Task 3: social/mod.rs — trait, types, dispatcher

**Files:**
- Create: `src-tauri/src/social/mod.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod social;`)

- [ ] **Step 1: Add module declaration**

In `src-tauri/src/lib.rs`, add after line 19 (`mod twitch;`):

```rust
mod social;
```

- [ ] **Step 2: Create social/mod.rs with trait and types**

Create `src-tauri/src/social/mod.rs`:

```rust
//! Platform publishing adapters.
//!
//! Shared trait + dispatcher for YouTube, TikTok, Instagram.
//! YouTube is fully implemented; TikTok/Instagram are stubs.

pub mod youtube;
pub mod tiktok;
pub mod instagram;

use crate::db;
use crate::error::AppError;
use rusqlite::Connection;
use std::path::Path;

// ═══════════════════════════════════════════════════════════════════
//  Shared types (serialized to frontend)
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConnectedAccount {
    pub platform: String,
    pub account_name: String,
    pub account_id: String,
    pub connected_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UploadMeta {
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    pub visibility: String,
    pub clip_id: String,
    pub force: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status")]
pub enum UploadResultStatus {
    #[serde(rename = "uploading")]
    Uploading { progress_pct: u8 },
    #[serde(rename = "processing")]
    Processing,
    #[serde(rename = "complete")]
    Complete { video_url: String },
    #[serde(rename = "failed")]
    Failed { error: String },
    #[serde(rename = "duplicate")]
    Duplicate { existing_url: String },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UploadResult {
    pub status: UploadResultStatus,
    pub job_id: String,
}

// ═══════════════════════════════════════════════════════════════════
//  Platform adapter trait
// ═══════════════════════════════════════════════════════════════════

#[async_trait::async_trait]
pub trait PlatformAdapter: Send + Sync {
    fn platform_id(&self) -> &'static str;
    fn is_ready(&self, db: &Connection) -> Result<bool, AppError>;
    async fn start_auth(&self) -> Result<(String, tokio::sync::oneshot::Sender<String>), AppError>;
    async fn handle_callback(&self, db: &Connection, code: &str) -> Result<ConnectedAccount, AppError>;
    async fn refresh_token(&self, db: &Connection) -> Result<(), AppError>;
    async fn upload_video(&self, db: &Connection, file_path: &str, meta: &UploadMeta) -> Result<UploadResult, AppError>;
    fn disconnect(&self, db: &Connection) -> Result<(), AppError>;
    fn get_account(&self, db: &Connection) -> Result<Option<ConnectedAccount>, AppError>;
}

// ═══════════════════════════════════════════════════════════════════
//  Dispatcher
// ═══════════════════════════════════════════════════════════════════

pub fn get_adapter(platform: &str) -> Result<Box<dyn PlatformAdapter>, AppError> {
    match platform {
        "youtube" => Ok(Box::new(youtube::YouTubeAdapter)),
        "tiktok" => Ok(Box::new(tiktok::TikTokAdapter)),
        "instagram" => Ok(Box::new(instagram::InstagramAdapter)),
        _ => Err(AppError::NotSupported(format!("Unknown platform: {}", platform))),
    }
}

// ═══════════════════════════════════════════════════════════════════
//  File validation
// ═══════════════════════════════════════════════════════════════════

/// Validate an exported clip file before upload.
/// Returns the validated path or a descriptive error.
pub fn validate_export_file(output_path: Option<&str>) -> Result<&str, AppError> {
    let path_str = output_path
        .ok_or_else(|| AppError::NotFound("Clip has not been exported yet".into()))?;

    let path = Path::new(path_str);

    if !path.exists() {
        return Err(AppError::NotFound(
            "Export file not found — re-export the clip first".into(),
        ));
    }

    let metadata = std::fs::metadata(path)
        .map_err(|e| AppError::Unknown(format!("Cannot read export file: {}", e)))?;

    if metadata.len() == 0 {
        return Err(AppError::NotFound("Export file is empty — re-export the clip".into()));
    }

    let ext = path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if !["mp4", "webm", "mov"].contains(&ext.as_str()) {
        return Err(AppError::NotSupported(
            format!("Unsupported file format '.{}' for upload (expected .mp4, .webm, or .mov)", ext),
        ));
    }

    Ok(path_str)
}

// ═══════════════════════════════════════════════════════════════════
//  Helpers for all adapters
// ═══════════════════════════════════════════════════════════════════

/// Get all connected accounts across all platforms.
pub fn get_all_accounts(db: &Connection) -> Result<Vec<ConnectedAccount>, AppError> {
    let adapters: Vec<Box<dyn PlatformAdapter>> = vec![
        Box::new(youtube::YouTubeAdapter),
        Box::new(tiktok::TikTokAdapter),
        Box::new(instagram::InstagramAdapter),
    ];
    let mut accounts = Vec::new();
    for adapter in &adapters {
        if let Ok(Some(acct)) = adapter.get_account(db) {
            accounts.push(acct);
        }
    }
    Ok(accounts)
}
```

- [ ] **Step 3: Add async-trait dependency**

In `src-tauri/Cargo.toml`, add to `[dependencies]`:

```toml
async-trait = "0.1"
```

- [ ] **Step 4: Create stub files so the module compiles**

Create `src-tauri/src/social/youtube.rs`:

```rust
pub struct YouTubeAdapter;
```

Create `src-tauri/src/social/tiktok.rs`:

```rust
pub struct TikTokAdapter;
```

Create `src-tauri/src/social/instagram.rs`:

```rust
pub struct InstagramAdapter;
```

These are temporary — they'll be filled in subsequent tasks. The module won't compile yet because the structs don't implement `PlatformAdapter`. That's expected; we'll fix it in Tasks 4-6.

- [ ] **Step 5: Commit**

```bash
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
git add src-tauri/src/social/ src-tauri/Cargo.toml src-tauri/src/lib.rs
git commit -m "feat(social): add platform adapter trait, types, and dispatcher"
```

---

## Task 4: social/tiktok.rs — stub implementation

**Files:**
- Modify: `src-tauri/src/social/tiktok.rs`

- [ ] **Step 1: Implement the stub**

Replace `src-tauri/src/social/tiktok.rs` with:

```rust
use crate::error::AppError;
use crate::social::{ConnectedAccount, PlatformAdapter, UploadMeta, UploadResult};
use rusqlite::Connection;

pub struct TikTokAdapter;

#[async_trait::async_trait]
impl PlatformAdapter for TikTokAdapter {
    fn platform_id(&self) -> &'static str { "tiktok" }

    fn is_ready(&self, _db: &Connection) -> Result<bool, AppError> { Ok(false) }

    async fn start_auth(&self) -> Result<(String, tokio::sync::oneshot::Sender<String>), AppError> {
        Err(AppError::NotSupported("TikTok publishing coming soon".into()))
    }

    async fn handle_callback(&self, _db: &Connection, _code: &str) -> Result<ConnectedAccount, AppError> {
        Err(AppError::NotSupported("TikTok publishing coming soon".into()))
    }

    async fn refresh_token(&self, _db: &Connection) -> Result<(), AppError> {
        Err(AppError::NotSupported("TikTok publishing coming soon".into()))
    }

    async fn upload_video(&self, _db: &Connection, _file_path: &str, _meta: &UploadMeta) -> Result<UploadResult, AppError> {
        Err(AppError::NotSupported("TikTok publishing coming soon".into()))
    }

    fn disconnect(&self, _db: &Connection) -> Result<(), AppError> { Ok(()) }

    fn get_account(&self, _db: &Connection) -> Result<Option<ConnectedAccount>, AppError> { Ok(None) }
}
```

- [ ] **Step 2: Commit**

```bash
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
git add src-tauri/src/social/tiktok.rs
git commit -m "feat(social): add TikTok stub adapter"
```

---

## Task 5: social/instagram.rs — stub implementation

**Files:**
- Modify: `src-tauri/src/social/instagram.rs`

- [ ] **Step 1: Implement the stub**

Replace `src-tauri/src/social/instagram.rs` with:

```rust
use crate::error::AppError;
use crate::social::{ConnectedAccount, PlatformAdapter, UploadMeta, UploadResult};
use rusqlite::Connection;

pub struct InstagramAdapter;

#[async_trait::async_trait]
impl PlatformAdapter for InstagramAdapter {
    fn platform_id(&self) -> &'static str { "instagram" }

    fn is_ready(&self, _db: &Connection) -> Result<bool, AppError> { Ok(false) }

    async fn start_auth(&self) -> Result<(String, tokio::sync::oneshot::Sender<String>), AppError> {
        Err(AppError::NotSupported("Instagram publishing coming soon".into()))
    }

    async fn handle_callback(&self, _db: &Connection, _code: &str) -> Result<ConnectedAccount, AppError> {
        Err(AppError::NotSupported("Instagram publishing coming soon".into()))
    }

    async fn refresh_token(&self, _db: &Connection) -> Result<(), AppError> {
        Err(AppError::NotSupported("Instagram publishing coming soon".into()))
    }

    async fn upload_video(&self, _db: &Connection, _file_path: &str, _meta: &UploadMeta) -> Result<UploadResult, AppError> {
        Err(AppError::NotSupported("Instagram publishing coming soon".into()))
    }

    fn disconnect(&self, _db: &Connection) -> Result<(), AppError> { Ok(()) }

    fn get_account(&self, _db: &Connection) -> Result<Option<ConnectedAccount>, AppError> { Ok(None) }
}
```

- [ ] **Step 2: Commit**

```bash
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
git add src-tauri/src/social/instagram.rs
git commit -m "feat(social): add Instagram stub adapter"
```

---

## Task 6: social/youtube.rs — OAuth + upload

**Files:**
- Modify: `src-tauri/src/social/youtube.rs`

This is the largest task. It implements the full YouTube adapter: OAuth with localhost callback, token storage, token refresh, channel info fetch, resumable upload with progress, and upload completion URL.

- [ ] **Step 1: Write the YouTube adapter**

Replace `src-tauri/src/social/youtube.rs` with the full implementation:

```rust
//! YouTube OAuth 2.0 + Data API v3 upload adapter.
//!
//! OAuth flow:
//!   1. start_auth() → spawns localhost listener, returns auth URL
//!   2. User completes consent in browser → Google redirects to localhost
//!   3. handle_callback() → exchanges code for tokens, fetches channel info
//!
//! Upload flow:
//!   1. upload_video() → validates file, checks duplicates, refreshes token
//!   2. Initiates resumable upload via Data API v3
//!   3. Sends file in chunks, returns video URL on completion

use crate::db;
use crate::error::AppError;
use crate::social::{ConnectedAccount, PlatformAdapter, UploadMeta, UploadResult, UploadResultStatus, validate_export_file};
use rusqlite::Connection;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

// ═══════════════════════════════════════════════════════════════════
//  Constants
// ═══════════════════════════════════════════════════════════════════

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const YOUTUBE_API_URL: &str = "https://www.googleapis.com/youtube/v3";
const YOUTUBE_UPLOAD_URL: &str = "https://www.googleapis.com/upload/youtube/v3/videos";

const CALLBACK_PORT: u16 = 17386;
const REDIRECT_URI: &str = "http://localhost:17386";
const SCOPES: &str = "https://www.googleapis.com/auth/youtube.upload https://www.googleapis.com/auth/youtube.readonly";

// App-owned OAuth credentials — injected at build time.
// For development: set YOUTUBE_CLIENT_ID and YOUTUBE_CLIENT_SECRET env vars,
// or fall back to placeholder values that will fail at runtime with a clear error.
const CLIENT_ID: &str = option_env!("YOUTUBE_CLIENT_ID").unwrap_or("YOUTUBE_CLIENT_ID_NOT_SET");
const CLIENT_SECRET: &str = option_env!("YOUTUBE_CLIENT_SECRET").unwrap_or("YOUTUBE_CLIENT_SECRET_NOT_SET");

const AUTH_TIMEOUT_SECS: u64 = 120;
const UPLOAD_CHUNK_SIZE: usize = 5 * 1024 * 1024; // 5 MB

// ═══════════════════════════════════════════════════════════════════
//  Adapter
// ═══════════════════════════════════════════════════════════════════

pub struct YouTubeAdapter;

#[async_trait::async_trait]
impl PlatformAdapter for YouTubeAdapter {
    fn platform_id(&self) -> &'static str { "youtube" }

    fn is_ready(&self, db: &Connection) -> Result<bool, AppError> {
        let access = db::get_setting(db, "youtube_access_token")
            .map_err(|e| AppError::Database(e.to_string()))?;
        let refresh = db::get_setting(db, "youtube_refresh_token")
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(access.is_some() && refresh.is_some())
    }

    async fn start_auth(&self) -> Result<(String, tokio::sync::oneshot::Sender<String>), AppError> {
        if CLIENT_ID == "YOUTUBE_CLIENT_ID_NOT_SET" {
            return Err(AppError::Api("YouTube OAuth credentials not configured. Set YOUTUBE_CLIENT_ID and YOUTUBE_CLIENT_SECRET environment variables.".into()));
        }

        // Generate CSRF state
        let state = format!("{:x}", {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            std::time::Instant::now().hash(&mut h);
            std::process::id().hash(&mut h);
            h.finish()
        });

        let auth_url = format!(
            "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&access_type=offline&prompt=consent",
            GOOGLE_AUTH_URL,
            urlencoding::encode(CLIENT_ID),
            urlencoding::encode(REDIRECT_URI),
            urlencoding::encode(SCOPES),
            urlencoding::encode(&state),
        );

        // Spawn callback listener in background
        let (tx, _rx) = tokio::sync::oneshot::channel::<String>();

        // We return the URL; the caller (Tauri command) handles opening browser + waiting.
        // The actual callback is handled by connect_platform in lib.rs.
        Ok((auth_url, tx))
    }

    async fn handle_callback(&self, db: &Connection, code: &str) -> Result<ConnectedAccount, AppError> {
        // Exchange code for tokens
        let tokens = exchange_code(code).await?;

        // Store tokens
        db::save_setting(db, "youtube_access_token", &tokens.access_token)
            .map_err(|e| AppError::Database(e.to_string()))?;
        if let Some(ref rt) = tokens.refresh_token {
            db::save_setting(db, "youtube_refresh_token", rt)
                .map_err(|e| AppError::Database(e.to_string()))?;
        }
        let expiry = chrono::Utc::now().timestamp() + tokens.expires_in as i64;
        db::save_setting(db, "youtube_token_expiry", &expiry.to_string())
            .map_err(|e| AppError::Database(e.to_string()))?;

        // Fetch channel info
        let channel = fetch_channel_info(&tokens.access_token).await?;

        db::save_setting(db, "youtube_channel_name", &channel.name)
            .map_err(|e| AppError::Database(e.to_string()))?;
        db::save_setting(db, "youtube_channel_id", &channel.id)
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(ConnectedAccount {
            platform: "youtube".into(),
            account_name: channel.name,
            account_id: channel.id,
            connected_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    async fn refresh_token(&self, db: &Connection) -> Result<(), AppError> {
        let refresh_token = db::get_setting(db, "youtube_refresh_token")
            .map_err(|e| AppError::Database(e.to_string()))?
            .ok_or_else(|| AppError::Api("No YouTube refresh token stored".into()))?;

        let client = reqwest::Client::new();
        let resp = client.post(GOOGLE_TOKEN_URL)
            .form(&[
                ("client_id", CLIENT_ID),
                ("client_secret", CLIENT_SECRET),
                ("refresh_token", &refresh_token),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await
            .map_err(|e| AppError::Api(format!("Token refresh request failed: {}", e)))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Api(format!("Token refresh failed: {}", &body[..body.len().min(300)])));
        }

        let token_resp: TokenResponse = resp.json().await
            .map_err(|e| AppError::Api(format!("Failed to parse refresh response: {}", e)))?;

        db::save_setting(db, "youtube_access_token", &token_resp.access_token)
            .map_err(|e| AppError::Database(e.to_string()))?;
        let expiry = chrono::Utc::now().timestamp() + token_resp.expires_in as i64;
        db::save_setting(db, "youtube_token_expiry", &expiry.to_string())
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    async fn upload_video(&self, db: &Connection, file_path: &str, meta: &UploadMeta) -> Result<UploadResult, AppError> {
        // 1. Validate file
        validate_export_file(Some(file_path))?;

        // 2. Check duplicate
        if !meta.force {
            if let Ok(Some(existing)) = db::get_upload_for_clip(db, &meta.clip_id, "youtube") {
                let url = existing.video_url.unwrap_or_default();
                return Ok(UploadResult {
                    status: UploadResultStatus::Duplicate { existing_url: url },
                    job_id: existing.id,
                });
            }
        }

        // 3. Ensure token is fresh
        ensure_token_fresh(db, self).await?;

        let access_token = db::get_setting(db, "youtube_access_token")
            .map_err(|e| AppError::Database(e.to_string()))?
            .ok_or_else(|| AppError::Api("No YouTube access token".into()))?;

        // 4. Initiate resumable upload
        let video_metadata = serde_json::json!({
            "snippet": {
                "title": meta.title,
                "description": meta.description,
                "tags": meta.tags,
                "categoryId": "20" // Gaming
            },
            "status": {
                "privacyStatus": meta.visibility,
                "selfDeclaredMadeForKids": false
            }
        });

        let client = reqwest::Client::new();
        let init_resp = client.post(format!("{}?uploadType=resumable&part=snippet,status", YOUTUBE_UPLOAD_URL))
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .header("X-Upload-Content-Type", "video/*")
            .json(&video_metadata)
            .send()
            .await
            .map_err(|e| AppError::Api(format!("Upload init failed: {}", e)))?;

        if !init_resp.status().is_success() {
            let body = init_resp.text().await.unwrap_or_default();
            return Err(AppError::Api(format!("YouTube upload init failed: {}", &body[..body.len().min(500)])));
        }

        let upload_url = init_resp.headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::Api("No upload URL in YouTube response".into()))?
            .to_string();

        // 5. Upload file in chunks
        let file_bytes = std::fs::read(file_path)
            .map_err(|e| AppError::Unknown(format!("Cannot read export file: {}", e)))?;
        let total_size = file_bytes.len();

        let mut offset = 0;
        while offset < total_size {
            let end = (offset + UPLOAD_CHUNK_SIZE).min(total_size);
            let chunk = &file_bytes[offset..end];

            let resp = client.put(&upload_url)
                .header("Content-Length", chunk.len().to_string())
                .header("Content-Range", format!("bytes {}-{}/{}", offset, end - 1, total_size))
                .body(chunk.to_vec())
                .send()
                .await
                .map_err(|e| AppError::Api(format!("Chunk upload failed: {}", e)))?;

            let status = resp.status().as_u16();
            if status == 200 || status == 201 {
                // Upload complete — extract video ID
                let body: serde_json::Value = resp.json().await
                    .map_err(|e| AppError::Api(format!("Failed to parse upload response: {}", e)))?;
                let video_id = body["id"].as_str().unwrap_or("unknown");
                let video_url = format!("https://youtu.be/{}", video_id);

                // Record in upload history
                db::upsert_upload(db, &meta.clip_id, "youtube", &video_url)
                    .map_err(|e| AppError::Database(e.to_string()))?;

                return Ok(UploadResult {
                    status: UploadResultStatus::Complete { video_url },
                    job_id: video_id.to_string(),
                });
            } else if status == 308 {
                // Chunk accepted, continue
                offset = end;
            } else {
                let body = resp.text().await.unwrap_or_default();
                return Err(AppError::Api(format!("Upload chunk failed ({}): {}", status, &body[..body.len().min(300)])));
            }
        }

        Err(AppError::Api("Upload completed without confirmation from YouTube".into()))
    }

    fn disconnect(&self, db: &Connection) -> Result<(), AppError> {
        db::delete_settings_for_platform(db, "youtube")
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn get_account(&self, db: &Connection) -> Result<Option<ConnectedAccount>, AppError> {
        let name = db::get_setting(db, "youtube_channel_name")
            .map_err(|e| AppError::Database(e.to_string()))?;
        let id = db::get_setting(db, "youtube_channel_id")
            .map_err(|e| AppError::Database(e.to_string()))?;

        match (name, id) {
            (Some(name), Some(id)) => Ok(Some(ConnectedAccount {
                platform: "youtube".into(),
                account_name: name,
                account_id: id,
                connected_at: String::new(), // Not tracked separately
            })),
            _ => Ok(None),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  OAuth helpers
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    #[serde(default)]
    refresh_token: Option<String>,
}

struct ChannelInfo {
    id: String,
    name: String,
}

/// Bind the localhost callback listener for OAuth.
pub fn bind_callback_server() -> Result<TcpListener, AppError> {
    TcpListener::bind(format!("127.0.0.1:{}", CALLBACK_PORT))
        .or_else(|_| TcpListener::bind(format!("[::1]:{}", CALLBACK_PORT)))
        .map_err(|e| AppError::Api(format!("Cannot bind YouTube OAuth callback port {}: {}", CALLBACK_PORT, e)))
}

/// Wait for the OAuth callback on the listener. Returns the authorization code.
/// Times out after AUTH_TIMEOUT_SECS.
pub fn wait_for_auth_code(listener: TcpListener) -> Result<String, AppError> {
    listener.set_nonblocking(true)
        .map_err(|e| AppError::Api(format!("Cannot set non-blocking: {}", e)))?;

    let deadline = Instant::now() + Duration::from_secs(AUTH_TIMEOUT_SECS);

    loop {
        if Instant::now() > deadline {
            return Err(AppError::Api("YouTube login timed out (2 minutes). Please try again.".into()));
        }

        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);

                // Parse first line: GET /?code=XXX&state=YYY HTTP/1.1
                if let Some(line) = request.lines().next() {
                    if let Some(query_start) = line.find('?') {
                        let query_end = line.find(" HTTP").unwrap_or(line.len());
                        let query = &line[query_start + 1..query_end];

                        let params: std::collections::HashMap<&str, &str> = query
                            .split('&')
                            .filter_map(|p| p.split_once('='))
                            .collect();

                        if let Some(&code) = params.get("code") {
                            send_html_response(&mut stream, true, "Connected to YouTube!");
                            return Ok(urlencoding::decode(code).unwrap_or_default().to_string());
                        }

                        if let Some(&error) = params.get("error") {
                            let desc = params.get("error_description")
                                .map(|d| urlencoding::decode(d).unwrap_or_default().to_string())
                                .unwrap_or_else(|| error.to_string());
                            send_html_response(&mut stream, false, &format!("Login failed: {}", desc));
                            return Err(AppError::Api(format!("YouTube auth denied: {}", desc)));
                        }
                    }
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

fn send_html_response(stream: &mut impl Write, success: bool, message: &str) {
    let (color, icon) = if success { ("#22c55e", "&#10004;") } else { ("#ef4444", "&#10008;") };
    let body = format!(
        r#"<html><body style="background:#0f0f0f;color:#fff;font-family:system-ui;display:flex;justify-content:center;align-items:center;height:100vh;margin:0">
        <div style="text-align:center"><span style="font-size:48px;color:{}">{}</span><p style="font-size:18px;margin-top:16px">{}</p><p style="color:#888;font-size:13px">You can close this tab.</p></div>
        </body></html>"#,
        color, icon, message
    );
    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

async fn exchange_code(code: &str) -> Result<TokenResponse, AppError> {
    let client = reqwest::Client::new();
    let resp = client.post(GOOGLE_TOKEN_URL)
        .form(&[
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("code", code),
            ("grant_type", "authorization_code"),
            ("redirect_uri", REDIRECT_URI),
        ])
        .send()
        .await
        .map_err(|e| AppError::Api(format!("Token exchange failed: {}", e)))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Api(format!("Google token exchange failed: {}", &body[..body.len().min(300)])));
    }

    resp.json::<TokenResponse>().await
        .map_err(|e| AppError::Api(format!("Failed to parse token response: {}", e)))
}

async fn fetch_channel_info(access_token: &str) -> Result<ChannelInfo, AppError> {
    let client = reqwest::Client::new();
    let resp = client.get(format!("{}/channels?part=snippet&mine=true", YOUTUBE_API_URL))
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| AppError::Api(format!("Channel fetch failed: {}", e)))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Api(format!("YouTube channel fetch failed: {}", &body[..body.len().min(300)])));
    }

    let body: serde_json::Value = resp.json().await
        .map_err(|e| AppError::Api(format!("Failed to parse channel response: {}", e)))?;

    let items = body["items"].as_array()
        .ok_or_else(|| AppError::Api("No YouTube channel found for this account".into()))?;

    let channel = items.first()
        .ok_or_else(|| AppError::Api("No YouTube channel found for this account".into()))?;

    Ok(ChannelInfo {
        id: channel["id"].as_str().unwrap_or("").to_string(),
        name: channel["snippet"]["title"].as_str().unwrap_or("YouTube Channel").to_string(),
    })
}

/// Ensure the access token is fresh. Refreshes if expired.
async fn ensure_token_fresh(db: &Connection, adapter: &YouTubeAdapter) -> Result<(), AppError> {
    let expiry_str = db::get_setting(db, "youtube_token_expiry")
        .map_err(|e| AppError::Database(e.to_string()))?
        .unwrap_or_else(|| "0".into());

    let expiry: i64 = expiry_str.parse().unwrap_or(0);
    let now = chrono::Utc::now().timestamp();

    // Refresh if token expires within 60 seconds
    if now >= expiry - 60 {
        log::info!("YouTube token expired or expiring soon, refreshing...");
        adapter.refresh_token(db).await?;
    }

    Ok(())
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cd "C:\Users\cereb\Desktop\Claude projects\clipviral\src-tauri" && cargo check 2>&1 | grep "^error"`
Expected: No output.

- [ ] **Step 3: Commit**

```bash
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
git add src-tauri/src/social/youtube.rs
git commit -m "feat(social): implement YouTube OAuth + resumable upload adapter"
```

---

## Task 7: lib.rs — register Tauri commands

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add the 6 new command functions**

Add before the `invoke_handler` block (around line 2830):

```rust
// ═══════════════════════════════════════════════════════════════════
//  Social publishing commands
// ═══════════════════════════════════════════════════════════════════

#[tauri::command]
async fn connect_platform(
    platform: String,
    app: AppHandle,
    db: State<'_, DbConn>,
) -> Result<social::ConnectedAccount, String> {
    let adapter = social::get_adapter(&platform).map_err(|e| e.to_string())?;

    // 1. Bind callback server before opening browser
    let listener = social::youtube::bind_callback_server().map_err(|e| e.to_string())?;

    // 2. Get auth URL
    let (auth_url, _tx) = adapter.start_auth().await.map_err(|e| e.to_string())?;

    // 3. Open browser
    app.opener().open_url(&auth_url, None::<&str>).map_err(|e| e.to_string())?;

    // 4. Wait for callback (blocking in spawned task)
    let code = tokio::task::spawn_blocking(move || {
        social::youtube::wait_for_auth_code(listener)
    }).await.map_err(|e| format!("Auth task error: {}", e))?.map_err(|e| e.to_string())?;

    // 5. Exchange code + fetch account info
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    let account = adapter.handle_callback(&conn, &code).await.map_err(|e| e.to_string())?;

    Ok(account)
}

#[tauri::command]
async fn disconnect_platform(
    platform: String,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let adapter = social::get_adapter(&platform).map_err(|e| e.to_string())?;
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    adapter.disconnect(&conn).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn get_connected_account(
    platform: String,
    db: State<'_, DbConn>,
) -> Result<Option<social::ConnectedAccount>, String> {
    let adapter = social::get_adapter(&platform).map_err(|e| e.to_string())?;
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    adapter.get_account(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_all_connected_accounts(
    db: State<'_, DbConn>,
) -> Result<Vec<social::ConnectedAccount>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    social::get_all_accounts(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
async fn upload_to_platform(
    platform: String,
    clip_id: String,
    title: String,
    description: String,
    tags: Vec<String>,
    visibility: Option<String>,
    force: Option<bool>,
    db: State<'_, DbConn>,
) -> Result<social::UploadResult, String> {
    let adapter = social::get_adapter(&platform).map_err(|e| e.to_string())?;
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;

    // Check platform readiness
    if !adapter.is_ready(&conn).map_err(|e| e.to_string())? {
        return Err(format!("Not connected to {}. Please connect your account first.", platform));
    }

    // Get the exported file path from the clips table
    let clip = db::get_clip(&conn, &clip_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or_else(|| format!("Clip {} not found", clip_id))?;

    let output_path = clip.output_path
        .as_deref()
        .ok_or_else(|| "Clip has not been exported yet".to_string())?;

    let meta = social::UploadMeta {
        title,
        description,
        tags,
        visibility: visibility.unwrap_or_else(|| "unlisted".into()),
        clip_id,
        force: force.unwrap_or(false),
    };

    adapter.upload_video(&conn, output_path, &meta).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_upload_status(
    platform: String,
    clip_id: String,
    db: State<'_, DbConn>,
) -> Result<Option<social::UploadResultStatus>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    match db::get_upload_for_clip(&conn, &clip_id, &platform) {
        Ok(Some(row)) => {
            let url = row.video_url.unwrap_or_default();
            if url.is_empty() {
                Ok(Some(social::UploadResultStatus::Processing))
            } else {
                Ok(Some(social::UploadResultStatus::Complete { video_url: url }))
            }
        }
        Ok(None) => Ok(None),
        Err(e) => Err(format!("DB error: {}", e)),
    }
}
```

- [ ] **Step 2: Check if db::get_clip exists, add if missing**

Search for `get_clip` in `db.rs`. If it doesn't exist, add this helper near the other clip functions:

```rust
pub fn get_clip(conn: &Connection, clip_id: &str) -> SqliteResult<Option<ClipRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, highlight_id, vod_id, title, start_seconds, end_seconds, aspect_ratio,
                crop_x, crop_y, crop_width, crop_height, captions_enabled, render_status, output_path, created_at
         FROM clips WHERE id = ?1"
    )?;
    let mut rows = stmt.query_map(params![clip_id], |row| {
        Ok(ClipRow {
            id: row.get(0)?,
            highlight_id: row.get(1)?,
            vod_id: row.get(2)?,
            title: row.get(3)?,
            start_seconds: row.get(4)?,
            end_seconds: row.get(5)?,
            aspect_ratio: row.get(6)?,
            crop_x: row.get(7)?,
            crop_y: row.get(8)?,
            crop_width: row.get(9)?,
            crop_height: row.get(10)?,
            captions_enabled: row.get(11)?,
            render_status: row.get(12)?,
            output_path: row.get(13)?,
            created_at: row.get(14)?,
        })
    })?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}
```

Note: The exact `ClipRow` fields depend on what's already defined in `db.rs`. Check the existing struct and adjust column indices accordingly. The critical field is `output_path`.

- [ ] **Step 3: Register commands in invoke_handler**

In the `invoke_handler` block (line ~2835), add after `get_transcript,`:

```rust
            connect_platform,
            disconnect_platform,
            get_connected_account,
            get_all_connected_accounts,
            upload_to_platform,
            get_upload_status,
```

- [ ] **Step 4: Verify it compiles**

Run: `cd "C:\Users\cereb\Desktop\Claude projects\clipviral\src-tauri" && cargo check 2>&1 | grep "^error"`
Expected: No output. Fix any type mismatches based on the actual `ClipRow` struct in `db.rs`.

- [ ] **Step 5: Commit**

```bash
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
git add src-tauri/src/lib.rs src-tauri/src/db.rs
git commit -m "feat(social): register 6 Tauri commands for social publishing"
```

---

## Task 8: platformStore.ts — Tauri-backed rewrite

**Files:**
- Modify: `src/stores/platformStore.ts`

- [ ] **Step 1: Rewrite the store**

Replace the entire contents of `src/stores/platformStore.ts`:

```typescript
import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

// ── Types (match Rust social::ConnectedAccount) ──

export interface ConnectedAccount {
  platform: string
  account_name: string
  account_id: string
  connected_at: string
}

export interface UploadResult {
  status: UploadResultStatus
  job_id: string
}

export type UploadResultStatus =
  | { status: 'uploading'; progress_pct: number }
  | { status: 'processing' }
  | { status: 'complete'; video_url: string }
  | { status: 'failed'; error: string }
  | { status: 'duplicate'; existing_url: string }

// ── Platform metadata ──

export const PLATFORM_INFO: Record<string, { name: string; color: string; icon: string; available: boolean }> = {
  youtube:   { name: 'YouTube',   color: '#ff0000', icon: 'YT', available: true },
  tiktok:    { name: 'TikTok',    color: '#00f2ea', icon: 'TT', available: false },
  instagram: { name: 'Instagram', color: '#e1306c', icon: 'IG', available: false },
}

// ── Store ──

interface PlatformState {
  accounts: Record<string, ConnectedAccount | null>
  loading: Record<string, boolean>
  load: () => Promise<void>
  connect: (platform: string) => Promise<ConnectedAccount>
  disconnect: (platform: string) => Promise<void>
  isConnected: (platform: string) => boolean
  getAccount: (platform: string) => ConnectedAccount | null
}

export const usePlatformStore = create<PlatformState>((set, get) => ({
  accounts: {},
  loading: {},

  load: async () => {
    try {
      const accounts = await invoke<ConnectedAccount[]>('get_all_connected_accounts')
      const map: Record<string, ConnectedAccount | null> = {}
      for (const acct of accounts) {
        map[acct.platform] = acct
      }
      set({ accounts: map })
    } catch (e) {
      console.error('Failed to load connected accounts:', e)
    }
  },

  connect: async (platform: string) => {
    set(s => ({ loading: { ...s.loading, [platform]: true } }))
    try {
      const account = await invoke<ConnectedAccount>('connect_platform', { platform })
      set(s => ({
        accounts: { ...s.accounts, [platform]: account },
        loading: { ...s.loading, [platform]: false },
      }))
      return account
    } catch (e) {
      set(s => ({ loading: { ...s.loading, [platform]: false } }))
      throw e
    }
  },

  disconnect: async (platform: string) => {
    set(s => ({ loading: { ...s.loading, [platform]: true } }))
    try {
      await invoke('disconnect_platform', { platform })
      set(s => ({
        accounts: { ...s.accounts, [platform]: null },
        loading: { ...s.loading, [platform]: false },
      }))
    } catch (e) {
      set(s => ({ loading: { ...s.loading, [platform]: false } }))
      throw e
    }
  },

  isConnected: (platform: string) => {
    return get().accounts[platform] != null
  },

  getAccount: (platform: string) => {
    return get().accounts[platform] ?? null
  },
}))
```

- [ ] **Step 2: Commit**

```bash
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
git add src/stores/platformStore.ts
git commit -m "feat(social): rewrite platformStore with Tauri command integration"
```

---

## Task 9: ConnectedAccounts.tsx — real UI

**Files:**
- Modify: `src/components/ConnectedAccounts.tsx`

- [ ] **Step 1: Rewrite the component**

Replace the entire contents of `src/components/ConnectedAccounts.tsx`:

```tsx
import { useEffect } from 'react'
import { usePlatformStore, PLATFORM_INFO } from '../stores/platformStore'
import { Link2, Unlink, Loader2 } from 'lucide-react'

export default function ConnectedAccounts() {
  const { accounts, loading, load, connect, disconnect } = usePlatformStore()

  useEffect(() => { load() }, [load])

  const platforms = Object.keys(PLATFORM_INFO) as string[]

  return (
    <div className="space-y-3">
      {platforms.map(key => {
        const info = PLATFORM_INFO[key]
        const account = accounts[key]
        const isLoading = loading[key] ?? false

        return (
          <div key={key} className="flex items-center gap-3 p-3 bg-surface-900 border border-surface-600 rounded-lg">
            {/* Platform icon */}
            <div className="w-8 h-8 rounded-lg flex items-center justify-center text-xs font-bold shrink-0"
              style={{ background: `${info.color}20`, color: info.color, border: `1px solid ${info.color}40` }}>
              {info.icon}
            </div>

            <div className="flex-1 min-w-0">
              <p className="text-sm text-white font-medium">{info.name}</p>
              {isLoading ? (
                <p className="text-[10px] text-slate-400">Connecting...</p>
              ) : account ? (
                <p className="text-[10px] text-emerald-400 truncate">
                  Connected as {account.account_name}
                </p>
              ) : info.available ? (
                <p className="text-[10px] text-slate-500">Not connected</p>
              ) : (
                <p className="text-[10px] text-slate-600">Coming soon</p>
              )}
            </div>

            {isLoading ? (
              <Loader2 className="w-4 h-4 text-slate-400 animate-spin" />
            ) : account ? (
              <button onClick={() => disconnect(key)}
                className="flex items-center gap-1 px-2 py-1 text-xs text-red-400 bg-red-500/10 border border-red-500/30 rounded hover:bg-red-500/20 transition-colors cursor-pointer">
                <Unlink className="w-3 h-3" />
                Disconnect
              </button>
            ) : info.available ? (
              <button onClick={() => connect(key).catch(() => {})}
                className="flex items-center gap-1 px-2 py-1 text-xs text-slate-300 bg-surface-800 border border-surface-500 rounded hover:text-white hover:border-violet-500/40 transition-colors cursor-pointer">
                <Link2 className="w-3 h-3" />
                Connect
              </button>
            ) : (
              <span className="px-2 py-1 text-xs text-slate-600">—</span>
            )}
          </div>
        )
      })}
    </div>
  )
}
```

- [ ] **Step 2: Commit**

```bash
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
git add src/components/ConnectedAccounts.tsx
git commit -m "feat(social): rewrite ConnectedAccounts with real connect/disconnect/status"
```

---

## Task 10: Editor.tsx — upload button and connect-then-upload flow

**Files:**
- Modify: `src/pages/Editor.tsx`

- [ ] **Step 1: Add upload state and imports**

At the top of `Editor.tsx`, ensure these imports exist (add what's missing):

```typescript
import { usePlatformStore, PLATFORM_INFO } from '../stores/platformStore'
import type { UploadResult } from '../stores/platformStore'
import { invoke } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'
```

- [ ] **Step 2: Add upload state to ActionsBar**

Add new props and state to the `ActionsBar` component. Find the existing `ActionsBar` function signature and add the new props:

```typescript
function ActionsBar({
  clipId, clip, saving, saved, exporting, exportProgress, exportDone, exportError, vodPath,
  exportPreset, onSave, onExport,
  // New props for upload
  publishMeta,
}: {
  // ... existing props ...
  publishMeta?: { title: string; description: string; hashtags: string[]; visibility: string }
}) {
  const { connect, isConnected } = usePlatformStore()
  const { projects, addClip, createProject } = useMontageStore()
  const navigate = useNavigate()

  // Upload state
  const [uploading, setUploading] = useState(false)
  const [uploadDone, setUploadDone] = useState(false)
  const [uploadError, setUploadError] = useState<string | null>(null)
  const [uploadUrl, setUploadUrl] = useState<string | null>(null)
  const [duplicateUrl, setDuplicateUrl] = useState<string | null>(null)
```

- [ ] **Step 3: Add the upload handler function**

Inside ActionsBar, add:

```typescript
  const handleUpload = async (force = false) => {
    if (!clipId || !platformKey) return
    setUploading(true)
    setUploadError(null)
    setUploadDone(false)
    setDuplicateUrl(null)

    try {
      // If not connected, connect first (seamless connect-then-upload)
      if (!isConnected(platformKey)) {
        await connect(platformKey)
      }

      const result = await invoke<UploadResult>('upload_to_platform', {
        platform: platformKey,
        clipId,
        title: publishMeta?.title || clip?.title || 'Untitled Clip',
        description: publishMeta?.description || '',
        tags: publishMeta?.hashtags || [],
        visibility: publishMeta?.visibility || 'unlisted',
        force,
      })

      if (result.status.status === 'complete') {
        setUploadDone(true)
        setUploadUrl(result.status.video_url)
      } else if (result.status.status === 'duplicate') {
        setDuplicateUrl(result.status.existing_url)
      } else if (result.status.status === 'failed') {
        setUploadError(result.status.error)
      }
    } catch (e: any) {
      setUploadError(typeof e === 'string' ? e : e.message || 'Upload failed')
    } finally {
      setUploading(false)
    }
  }
```

- [ ] **Step 4: Replace the existing publish button**

Find the existing publish button block in the JSX (the `{platformKey && info && (` section). Replace the entire block with:

```tsx
        {/* Upload / Connect button */}
        {platformKey && info && exportDone && (
          <>
            {duplicateUrl ? (
              <div className="flex-1 flex flex-col gap-1">
                <p className="text-[10px] text-amber-400 px-1">Already uploaded to {info.name}</p>
                <div className="flex gap-1">
                  <button onClick={() => openUrl(duplicateUrl)}
                    className="flex-1 flex items-center justify-center gap-1 px-2 py-1.5 text-xs text-slate-300 bg-surface-800 border border-surface-600 rounded hover:text-white transition-colors cursor-pointer">
                    View existing
                  </button>
                  <button onClick={() => { setDuplicateUrl(null); handleUpload(true) }}
                    className="flex-1 flex items-center justify-center gap-1 px-2 py-1.5 text-xs text-amber-400 bg-amber-500/10 border border-amber-500/30 rounded hover:bg-amber-500/20 transition-colors cursor-pointer">
                    Upload again
                  </button>
                </div>
              </div>
            ) : uploadDone && uploadUrl ? (
              <button onClick={() => openUrl(uploadUrl)}
                className="flex-1 flex items-center justify-center gap-2 px-3 py-2 text-xs font-medium rounded-lg bg-green-600/20 text-green-400 border border-green-500/30 hover:bg-green-600/30 transition-colors cursor-pointer">
                <Check className="w-3.5 h-3.5" />
                Uploaded — View on {info.name}
              </button>
            ) : (
              <button
                onClick={() => handleUpload(false)}
                disabled={uploading}
                className={`flex-1 flex items-center justify-center gap-2 px-3 py-2 text-xs font-medium rounded-lg transition-colors cursor-pointer border ${
                  uploadError
                    ? 'bg-red-600/10 text-red-400 border-red-500/30 hover:bg-red-600/20'
                    : isConnected(platformKey)
                    ? 'text-white border-transparent hover:opacity-90'
                    : 'bg-surface-800 text-slate-400 border-surface-600 hover:text-white'
                }`}
                style={isConnected(platformKey) && !uploadError ? { backgroundColor: `${info.color}cc` } : undefined}
                title={uploadError || undefined}
              >
                {uploading ? (
                  <><Loader2 className="w-3.5 h-3.5 animate-spin" /> Uploading...</>
                ) : uploadError ? (
                  <><Upload className="w-3.5 h-3.5" /> Retry Upload</>
                ) : isConnected(platformKey) ? (
                  <><Upload className="w-3.5 h-3.5" /> Upload to {info.name}</>
                ) : (
                  <><Link2 className="w-3.5 h-3.5" /> Connect {info.name}</>
                )}
              </button>
            )}
          </>
        )}
```

- [ ] **Step 5: Pass publishMeta to ActionsBar**

In the parent Editor component, find where `<ActionsBar` is rendered and add the `publishMeta` prop:

```tsx
<ActionsBar
  // ... existing props ...
  publishMeta={publishMeta}
/>
```

- [ ] **Step 6: Add missing imports**

Make sure `Link2`, `Loader2`, `Upload`, `Check` are imported from `lucide-react` at the top of the file. Check the existing import line and add any missing ones.

- [ ] **Step 7: Verify the frontend compiles**

Run: `cd "C:\Users\cereb\Desktop\Claude projects\clipviral" && npx tsc --noEmit 2>&1 | head -20`
Expected: No errors (or only pre-existing ones).

- [ ] **Step 8: Commit**

```bash
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
git add src/pages/Editor.tsx
git commit -m "feat(social): add upload button with connect-then-upload flow in editor"
```

---

## Task 11: Load connected accounts on app init

**Files:**
- Modify: `src/App.tsx` (or wherever the app root/layout is)

- [ ] **Step 1: Find the app root component**

Look for `App.tsx` or the main layout component. Add account loading on mount:

```typescript
import { usePlatformStore } from './stores/platformStore'

// Inside the root component:
const loadAccounts = usePlatformStore(s => s.load)
useEffect(() => { loadAccounts() }, [loadAccounts])
```

If there's already a similar pattern for other stores (like `aiStore.load()`), follow that pattern exactly.

- [ ] **Step 2: Commit**

```bash
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
git add src/App.tsx
git commit -m "feat(social): load connected accounts on app startup"
```

---

## Task 12: Full integration test

- [ ] **Step 1: Verify backend compiles clean**

Run: `cd "C:\Users\cereb\Desktop\Claude projects\clipviral\src-tauri" && cargo check 2>&1 | grep "^error"`
Expected: No output.

- [ ] **Step 2: Run existing backend tests**

Run: `cd "C:\Users\cereb\Desktop\Claude projects\clipviral\src-tauri" && cargo test 2>&1 | grep "^test result:"`
Expected: All pass, 0 failed.

- [ ] **Step 3: Verify frontend compiles**

Run: `cd "C:\Users\cereb\Desktop\Claude projects\clipviral" && npx tsc --noEmit 2>&1 | head -20`
Expected: No new errors.

- [ ] **Step 4: Launch the app**

Run: `cd "C:\Users\cereb\Desktop\Claude projects\clipviral" && cargo tauri dev`
Expected: App launches. Settings page shows YouTube with "Connect" button. TikTok/Instagram show "Coming soon".

- [ ] **Step 5: Final commit**

```bash
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
git add -A
git commit -m "feat(social): complete YouTube publishing vertical slice"
```
