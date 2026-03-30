# Social Publishing Architecture — Design Spec

**Date:** 2026-03-27
**Scope:** Platform publishing with YouTube as first complete vertical slice; TikTok/Instagram scaffolded.

---

## 1. Overview

Allow users to connect their YouTube, TikTok, and Instagram accounts from ClipGoblin and upload exported clips directly. Uses app-owned OAuth credentials (not BYOK). Social publishing is architecturally separate from the AI provider system.

## 2. Backend Architecture

### 2.1 File Structure

```
src-tauri/src/
  social/
    mod.rs          — PlatformAdapter trait, shared types, dispatcher
    youtube.rs      — YouTube OAuth 2.0 + Data API v3 upload
    tiktok.rs       — Stub (returns "not yet supported")
    instagram.rs    — Stub (returns "not yet supported")
```

### 2.2 PlatformAdapter Trait

```rust
#[async_trait]
pub trait PlatformAdapter: Send + Sync {
    fn platform_id(&self) -> &'static str;
    fn is_ready(&self, db: &Connection) -> Result<bool, AppError>;
    async fn start_auth(&self, db: &Connection) -> Result<String, AppError>;
    async fn handle_callback(&self, db: &Connection, code: &str) -> Result<ConnectedAccount, AppError>;
    async fn refresh_token(&self, db: &Connection) -> Result<(), AppError>;
    async fn upload_video(&self, db: &Connection, file_path: &str, meta: &UploadMeta) -> Result<UploadResult, AppError>;
    async fn get_upload_status(&self, db: &Connection, job_id: &str) -> Result<UploadStatus, AppError>;
    fn disconnect(&self, db: &Connection) -> Result<(), AppError>;
    fn get_account(&self, db: &Connection) -> Result<Option<ConnectedAccount>, AppError>;
}
```

### 2.3 Shared Types

```rust
pub struct ConnectedAccount {
    pub platform: String,
    pub account_name: String,       // "MyChannel" or "@username"
    pub account_id: String,
    pub connected_at: String,
}

pub struct UploadMeta {
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    pub visibility: String,         // "public" | "unlisted" | "private"
    pub clip_id: String,            // For duplicate detection
    pub force: bool,                // true = bypass duplicate check
}

pub struct UploadResult {
    pub status: UploadResultStatus,
    pub job_id: String,
}

/// Frontend uses this to render the correct UI state.
pub enum UploadResultStatus {
    /// Upload accepted, file transfer in progress.
    Uploading { progress_pct: u8 },
    /// YouTube is processing the video after upload.
    Processing,
    /// Upload complete. video_url is always populated.
    Complete { video_url: String },
    /// Upload failed. error describes what went wrong.
    Failed { error: String },
    /// Duplicate detected. existing_url points to the prior upload.
    /// Frontend should show warning + "Upload again" option.
    Duplicate { existing_url: String },
}
```

### 2.4 Dispatcher

`social/mod.rs` exports `get_adapter(platform: &str) -> Result<Box<dyn PlatformAdapter>, AppError>` which returns the correct implementation. All Tauri commands route through this — no platform-specific commands.

### 2.5 Token Storage

Uses existing `settings` table with platform-prefixed keys:

| Key | Description |
|-----|-------------|
| `youtube_access_token` | OAuth access token |
| `youtube_refresh_token` | OAuth refresh token |
| `youtube_token_expiry` | Unix timestamp of access token expiry |
| `youtube_channel_name` | Display name ("MyChannel") |
| `youtube_channel_id` | YouTube channel ID |

Same pattern for future `tiktok_*`, `instagram_*` keys.

These keys are NOT in the frontend-writable `ALLOWED_SETTING_KEYS` whitelist. Only backend writes them.

### 2.6 YouTube OAuth Flow

1. `start_auth()` builds Google OAuth URL for installed-app flow.
2. Spawns local HTTP listener on a random port (same pattern as existing Twitch OAuth in `twitch.rs`).
3. Returns auth URL; frontend opens it in the user's browser.
4. Google redirects to `http://localhost:{port}` with authorization code.
5. Backend exchanges code for access + refresh tokens via Google's token endpoint.
6. Fetches channel info via YouTube Data API (`GET /youtube/v3/channels?part=snippet&mine=true`).
7. Stores tokens + channel name + channel ID in settings table.

**Required scopes:** `https://www.googleapis.com/auth/youtube.upload`, `https://www.googleapis.com/auth/youtube.readonly`

### 2.7 YouTube Upload

1. `is_ready()` checks: access token exists, refresh token exists, token not expired (or refreshable).
2. If token expired: auto-refresh via Google's token endpoint using refresh token.
3. File validation: path exists, file size > 0, extension is `.mp4`/`.webm`/`.mov`.
4. Duplicate check: query `upload_history` table for matching `(clip_id, platform)`. If found, return error with existing video URL.
5. Resumable upload via YouTube Data API v3 (`POST /upload/youtube/v3/videos?uploadType=resumable`).
6. Sends file in 5MB chunks with progress tracking.
7. On completion: extracts video ID, constructs URL `https://youtu.be/{video_id}`.
8. Stores upload record in `upload_history` table.
9. Returns `UploadResult { job_id, video_url }`.

### 2.8 Upload History Table

New table in `db.rs`:

```sql
CREATE TABLE IF NOT EXISTS upload_history (
    id TEXT PRIMARY KEY,
    clip_id TEXT NOT NULL,
    platform TEXT NOT NULL,
    video_url TEXT,
    uploaded_at TEXT,
    UNIQUE(clip_id, platform)
)
```

The `UNIQUE(clip_id, platform)` constraint prevents duplicate uploads. If a user wants to re-upload, they must explicitly acknowledge the duplicate.

### 2.9 is_ready Check

Before any upload, `is_ready(db)` validates:
- Access token exists in settings
- Refresh token exists in settings
- Token expiry is either in the future OR a refresh succeeds

Returns `false` (not an error) if any check fails. The frontend uses this to decide button state.

### 2.10 TikTok / Instagram Stubs

Both implement `PlatformAdapter` with all methods returning `AppError::NotSupported("TikTok publishing coming soon")`. `get_account()` returns `None`. `is_ready()` returns `false`.

## 3. Tauri Commands

Six new commands registered in `lib.rs`:

| Command | Args | Returns | Notes |
|---------|------|---------|-------|
| `connect_platform` | `platform: String` | `ConnectedAccount` | Async: opens browser, spawns localhost listener, awaits callback. Returns only after tokens are exchanged and account info is fetched. Frontend should show a loading spinner during this await. If the user closes the browser without completing auth, the command times out after 120s and returns an error. |
| `disconnect_platform` | `platform: String` | `()` | Deletes all settings keys for the platform (`{platform}_access_token`, `{platform}_refresh_token`, `{platform}_token_expiry`, `{platform}_channel_name`, `{platform}_channel_id`). Does NOT delete upload_history records. Frontend clears `accounts[platform]` from platformStore immediately on success. |
| `get_connected_account` | `platform: String` | `Option<ConnectedAccount>` | Read-only check |
| `get_all_connected_accounts` | — | `Vec<ConnectedAccount>` | For Settings page bulk load |
| `upload_to_platform` | `platform, clip_id, title, description, tags, visibility, force` | `UploadResult` | Validates file + tokens, starts upload. `force: true` bypasses duplicate check. |
| `get_upload_status` | `platform, job_id` | `UploadResultStatus` | Poll current state: Uploading/Processing/Complete/Failed |

## 4. Frontend — Connected Accounts in Settings

### 4.1 platformStore.ts Rewrite

Replace in-memory stubs with Tauri-backed state:

```typescript
interface PlatformState {
  accounts: Record<string, ConnectedAccount | null>  // keyed by platform
  loading: Record<string, boolean>
  load: () => Promise<void>
  connect: (platform: string) => Promise<ConnectedAccount>
  disconnect: (platform: string) => Promise<void>
  isConnected: (platform: string) => boolean
  isReady: (platform: string) => boolean
  getAccount: (platform: string) => ConnectedAccount | null
}
```

- `load()` calls `get_all_connected_accounts` on app init.
- `connect()` calls `connect_platform` Tauri command (async — blocks until OAuth completes).
- `disconnect()` calls `disconnect_platform`, then clears local state.
- `isConnected()` checks if `accounts[platform]` is non-null.

### 4.2 ConnectedAccounts.tsx Rewrite

Each platform row renders one of three states:

**Connected:**
```
● Connected as MyChannel                    [Disconnect]
```

**Not connected (YouTube — available):**
```
○ Not connected                        [Connect YouTube]
```

**Not connected (TikTok/Instagram — coming soon):**
```
○ Coming soon                               [disabled]
```

**Loading (during OAuth):**
```
◌ Connecting...                              [spinner]
```

Platform list: `["youtube", "tiktok", "instagram"]`. No Twitter in initial scope.

### 4.3 Settings.tsx

The Connected Accounts section already mounts `<ConnectedAccounts />`. No changes needed to Settings.tsx layout — only the component internals change.

## 5. Frontend — Upload Flow in Editor

### 5.1 Upload Button in ActionsBar

Located in `Editor.tsx`, rendered after export completes. Three states:

**Clip not exported:** Button hidden.

**Exported + YouTube connected:**
```
[▲ Upload to YouTube]
```
On click: calls `upload_to_platform` with publishMeta fields + clip_id.

**Exported + YouTube NOT connected:**
```
[▲ Connect YouTube]
```
On click: runs connect-then-upload as a single flow:
1. Calls `connect_platform("youtube")` — opens browser, waits for OAuth.
2. On success: immediately calls `upload_to_platform` with same metadata.
3. User experiences one click → browser auth → upload starts.

### 5.2 Upload Progress

Button text transitions:
```
[▲ Upload to YouTube]  →  [Uploading... 45%]  →  [Uploaded ✓]
```

- Progress: polls `get_upload_status` every 2 seconds, or listens for job events.
- Complete: button shows "Uploaded" with external link icon. Click opens video URL in browser.
- Failed: button shows "Upload failed" in red. Click retries upload. Error shown in tooltip.

### 5.3 Upload Metadata Source

All metadata comes from the existing `publishMeta` state in Editor:

| Field | Source | Fallback |
|-------|--------|----------|
| `title` | `publishMeta.title` | clip title from DB |
| `description` | `publishMeta.description` | empty string |
| `tags` | `publishMeta.hashtags` | empty array |
| `visibility` | `publishMeta.visibility` | "unlisted" (safe default — visible only via link) |

### 5.4 Duplicate Protection

Two-step UX:

1. **First attempt** (`force: false`, the default): Backend checks `upload_history` for existing `(clip_id, "youtube")`. If found, returns `UploadResultStatus::Duplicate { existing_url }` without uploading. Frontend shows:
   - "Already uploaded to YouTube" with clickable link to the existing video.
   - "Upload again" button that re-calls `upload_to_platform` with `force: true`.

2. **Re-upload** (`force: true`): Backend skips the duplicate check, uploads normally, and REPLACES the existing `upload_history` row with the new video URL.

The duplicate check is backend-enforced. The frontend never bypasses it silently — `force: true` requires an explicit user action.

### 5.5 File Validation

Backend validates the **exported** clip file before uploading (the rendered file at `clips.output_path`, NOT the original VOD source):

1. Reads `output_path` from the `clips` table for the given `clip_id`.
2. Checks the file exists on disk.
3. Checks file size > 0 bytes.
4. Checks extension is `.mp4`, `.webm`, or `.mov`.

If any check fails, returns a descriptive error:
- Missing path in DB: "Clip has not been exported yet"
- File not found on disk: "Export file not found — re-export the clip first"
- Empty file: "Export file is empty — re-export the clip"
- Wrong format: "Unsupported file format for YouTube upload"

## 6. What Does NOT Change

- AI provider system (BYOK, free mode, captions, titles) — completely untouched.
- Caption generation (post_captions.rs, PublishComposer.tsx) — works as-is, feeds metadata into upload.
- Export system (FFmpeg pipeline, job queue) — untouched. Upload is a separate step after export.
- Twitch login system — separate OAuth flow, not affected.

## 7. Implementation Order

YouTube as the first complete vertical slice:

```
Phase 1: Backend foundation
  1. social/mod.rs — trait, types, dispatcher, file validation, is_ready
  2. social/youtube.rs — OAuth flow (localhost callback), token exchange, channel fetch
  3. social/youtube.rs — upload via Data API v3 (resumable), progress, completion URL
  4. social/tiktok.rs — stub implementation
  5. social/instagram.rs — stub implementation
  6. db.rs — upload_history table migration
  7. lib.rs — register 6 new Tauri commands, wire to dispatcher

Phase 2: Frontend — Connected Accounts
  8. platformStore.ts — rewrite with Tauri command integration
  9. ConnectedAccounts.tsx — real connect/disconnect/status UI
  10. Settings.tsx — no layout changes needed, component handles itself

Phase 3: Frontend — Upload flow
  11. Editor.tsx — upload button with 3 states (hidden/connect/upload)
  12. Editor.tsx — connect-then-upload seamless flow
  13. Editor.tsx — upload progress + completion link + error handling
  14. Editor.tsx — duplicate upload warning

Phase 4: Polish
  15. Token refresh on app startup (background check)
  16. Error handling edge cases (network failures, quota limits, revoked tokens)
```

## 8. App-Owned OAuth Credentials

The YouTube OAuth Client ID and Client Secret are compiled into the binary. They are NOT user-configurable. The values will be read from environment variables at build time or from a config file:

```rust
const YOUTUBE_CLIENT_ID: &str = env!("YOUTUBE_CLIENT_ID");
const YOUTUBE_CLIENT_SECRET: &str = env!("YOUTUBE_CLIENT_SECRET");
```

For development: set via `.env` file (gitignored). For release builds: injected during CI.
