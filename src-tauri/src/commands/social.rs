//! Social platform connection and upload commands.

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::db;
use crate::DbConn;
use crate::social::{self, ConnectedAccount, UploadMeta, UploadResult};

/// Connect a social platform (YouTube, TikTok, Instagram) via OAuth.
///
/// The `PlatformAdapter` trait is `#[async_trait(?Send)]` (because `rusqlite::Connection`
/// is `!Sync`), so its async methods return `!Send` futures.  Tauri commands need
/// `Send` futures, so we use `block_in_place` + `block_on` to run each `!Send`
/// call synchronously on the current worker thread.
#[tauri::command]
pub async fn connect_platform(
    platform: String,
    app: AppHandle,
    db: State<'_, DbConn>,
) -> Result<ConnectedAccount, String> {
    let adapter = social::get_adapter(&platform).map_err(|e| e.to_string())?;

    // 1. Get auth URL (no DB needed — start_auth just builds a URL string).
    //    Must use block_in_place because the trait future is !Send.
    let auth_url = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(adapter.start_auth())
    })
    .map_err(|e| e.to_string())?;

    // 2. Bind callback server (sync, before opening browser to avoid race)
    //    Each platform listens on its own port.
    let listener = match platform.as_str() {
        "youtube" => social::youtube::bind_callback_server().map_err(|e| e.to_string())?,
        "tiktok" => social::tiktok::bind_callback_server().map_err(|e| e.to_string())?,
        _ => return Err(format!("No callback server for platform: {}", platform)),
    };

    // 3. Open browser
    app.opener()
        .open_url(&auth_url, None::<&str>)
        .map_err(|e| format!("Failed to open browser: {}", e))?;

    // 4. Wait for OAuth callback (blocking — runs on a threadpool thread)
    let plat = platform.clone();
    let code = tokio::task::spawn_blocking(move || {
        match plat.as_str() {
            "youtube" => social::youtube::wait_for_auth_code(listener),
            "tiktok" => social::tiktok::wait_for_auth_code(listener),
            _ => Err(crate::error::AppError::NotSupported(format!("{} callback", plat))),
        }
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
    .map_err(|e| e.to_string())?;

    // 5. Exchange code for tokens + persist to DB.
    //    handle_callback does HTTP first, then writes to DB — all in one !Send future.
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    let account = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(adapter.handle_callback(&conn, &code))
    })
    .map_err(|e| e.to_string())?;

    Ok(account)
}

/// Disconnect a social platform (removes stored tokens/channel info).
#[tauri::command]
pub fn disconnect_platform(
    platform: String,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let adapter = social::get_adapter(&platform).map_err(|e| e.to_string())?;
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    adapter.disconnect(&conn).map_err(|e| e.to_string())?;
    Ok(())
}

/// Get the connected account for a specific platform (if any).
#[tauri::command]
pub fn get_connected_account(
    platform: String,
    db: State<'_, DbConn>,
) -> Result<Option<ConnectedAccount>, String> {
    let adapter = social::get_adapter(&platform).map_err(|e| e.to_string())?;
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    adapter.get_account(&conn).map_err(|e| e.to_string())
}

/// Get all connected social accounts across all platforms.
#[tauri::command]
pub fn get_all_connected_accounts(
    db: State<'_, DbConn>,
) -> Result<Vec<ConnectedAccount>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    social::get_all_accounts(&conn).map_err(|e| e.to_string())
}

/// Upload a clip to a social platform.
/// Reads the clip's output_path from DB, validates, then delegates to the adapter.
///
/// Uses `block_in_place` + `block_on` for the `!Send` adapter future (see
/// `connect_platform` for the full explanation of the `?Send` workaround).
#[tauri::command]
pub async fn upload_to_platform(
    platform: String,
    meta: UploadMeta,
    db: State<'_, DbConn>,
) -> Result<UploadResult, String> {
    let adapter = social::get_adapter(&platform).map_err(|e| e.to_string())?;

    // Read clip output_path from DB (sync), validate, then drop the lock
    let output_path = {
        let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
        let clip = db::get_clip_by_id(&conn, &meta.clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or_else(|| format!("Clip '{}' not found", meta.clip_id))?;
        social::validate_export_file(clip.output_path.as_deref())
            .map_err(|e| e.to_string())?
            .to_string()
    };

    // Upload: adapter.upload_video is async(?Send), needs &Connection for
    // duplicate checks, token refresh, and recording upload history.
    // Must use block_in_place because the trait future is !Send (same
    // pattern as connect_platform — see that command for details).
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    let result = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current()
            .block_on(adapter.upload_video(&conn, &output_path, &meta))
    })
    .map_err(|e| {
        log::error!("[Upload] {} upload failed: {}", platform, e);
        e.to_string()
    })?;
    log::info!("[Upload] {} upload complete: {:?}", platform, result.status);

    Ok(result)
}

/// Check if a clip has already been uploaded to a platform.
/// Returns the upload history row if found, None otherwise.
#[tauri::command]
pub fn get_upload_status(
    clip_id: String,
    platform: String,
    db: State<'_, DbConn>,
) -> Result<Option<db::UploadHistoryRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_upload_for_clip(&conn, &clip_id, &platform)
        .map_err(|e| format!("DB error: {}", e))
}

/// Get ALL upload history entries for a clip (all platforms).
#[tauri::command]
pub fn get_clip_upload_history(
    clip_id: String,
    db: State<'_, DbConn>,
) -> Result<Vec<db::UploadHistoryRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_uploads_for_clip(&conn, &clip_id)
        .map_err(|e| format!("DB error: {}", e))
}

/// Clear the deleted_vods table so Twitch API re-fetch can re-insert all VODs.
#[tauri::command]
pub fn restore_deleted_vods(db: State<'_, DbConn>) -> Result<u64, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    let count = conn.execute("DELETE FROM deleted_vods", [])
        .map_err(|e| format!("DB error: {}", e))?;
    log::info!("[restore_deleted_vods] Cleared {} entries from deleted_vods table", count);
    Ok(count as u64)
}
