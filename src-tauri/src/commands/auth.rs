//! Twitch authentication commands.

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::db;
use crate::twitch;
use crate::DbConn;

#[tauri::command]
pub async fn twitch_login(app: AppHandle, db: State<'_, DbConn>) -> Result<db::ChannelRow, String> {
    log::info!("[twitch_login] === Starting Twitch login flow ===");

    // 1. Bind callback server BEFORE opening the browser (avoids race condition)
    let listener = twitch::bind_callback_server()?;
    log::info!("[twitch_login] Step 1: Callback server bound on port 17385");

    // 2. Open the auth URL in the user's browser (uses embedded client_id + PKCE)
    let auth_url = twitch::get_auth_url();
    log::info!("[twitch_login] Step 2: Opening browser for OAuth");
    app.opener().open_url(&auth_url, None::<&str>)
        .map_err(|e| format!("Failed to open browser: {}", e))?;

    // 3. Wait for the OAuth callback on the already-listening server
    log::info!("[twitch_login] Step 3: Waiting for OAuth callback...");
    let code = tokio::task::spawn_blocking(move || twitch::wait_for_auth_code(listener))
        .await
        .map_err(|e| format!("Task error: {}", e))??;
    log::info!("[twitch_login] Step 3: Auth code received (len={})", code.len());

    // Exchange the code for an access token (PKCE — no client_secret needed)
    log::info!("[twitch_login] Step 4: Exchanging code for token...");
    let token_resp = twitch::exchange_code(&code).await?;
    log::info!("[twitch_login] Step 4: Token exchange succeeded");

    // Fetch the authenticated user's identity
    log::info!("[twitch_login] Step 5: Fetching user info...");
    let user = twitch::get_authenticated_user(&token_resp.access_token).await?;
    log::info!("[twitch_login] Step 5: Got user: {} ({})", user.display_name, user.login);

    // Save the user token for future API calls
    log::info!("[twitch_login] Step 6: Saving tokens and user info to DB...");
    {
        let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
        db::save_setting(&conn, "twitch_user_access_token", &token_resp.access_token)
            .map_err(|e| format!("DB error: {}", e))?;
        if let Some(ref rt) = token_resp.refresh_token {
            db::save_setting(&conn, "twitch_refresh_token", rt)
                .map_err(|e| format!("DB error: {}", e))?;
        }
        db::save_setting(&conn, "twitch_user_id", &user.id)
            .map_err(|e| format!("DB error: {}", e))?;
        db::save_setting(&conn, "twitch_login", &user.login)
            .map_err(|e| format!("DB error: {}", e))?;
    }
    log::info!("[twitch_login] Step 6: Tokens saved to DB");

    // Clear any existing channels and add only this user's channel
    let channel_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let channel = db::ChannelRow {
        id: channel_id.clone(),
        twitch_user_id: user.id.clone(),
        twitch_login: user.login.clone(),
        display_name: user.display_name.clone(),
        profile_image_url: user.profile_image_url.clone(),
        created_at: now,
    };

    log::info!("[twitch_login] Step 7: Saving channel to DB...");
    {
        let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
        // Remove all existing channels — user can only have their own
        db::delete_all_channels(&conn).map_err(|e| format!("DB error: {}", e))?;
        db::insert_channel(
            &conn,
            &channel.id,
            &channel.twitch_user_id,
            &channel.twitch_login,
            &channel.display_name,
            &channel.profile_image_url,
        )
        .map_err(|e| format!("DB error: {}", e))?;
    }

    log::info!("[twitch_login] === Login complete: {} ({}) — returning ChannelRow to frontend ===",
        channel.display_name, channel.twitch_login);

    Ok(channel)
}

/// Check if the user is currently logged in.
///
/// Anchors identity to the `twitch_user_id` setting (which is only written
/// during the OAuth login flow and cleared on logout) rather than to channel
/// row recency. Picking "most recent channel" is wrong because dev-only
/// `import_vod_by_url` creates stub channels for foreign streamers, and those
/// stubs would otherwise hijack the displayed-as-logged-in identity.
///
/// Behavior:
/// - Setting empty / missing → not logged in (None)
/// - Setting present and matches a channel row → return that channel
/// - Setting present but no matching channel → not logged in (treated as
///   logged out; the channel was deleted but the setting was orphaned).
#[tauri::command]
pub fn get_logged_in_user(db: State<'_, DbConn>) -> Result<Option<db::ChannelRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;

    let logged_in_twitch_user_id = db::get_setting(&conn, "twitch_user_id")
        .map_err(|e| format!("DB error: {}", e))?
        .unwrap_or_default();

    if logged_in_twitch_user_id.is_empty() {
        return Ok(None);
    }

    let channels = db::get_all_channels(&conn).map_err(|e| format!("DB error: {}", e))?;
    Ok(channels.into_iter().find(|c| c.twitch_user_id == logged_in_twitch_user_id))
}

/// Log out — clear saved tokens and channel.
#[tauri::command]
pub fn twitch_logout(db: State<'_, DbConn>) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::delete_all_channels(&conn).map_err(|e| format!("DB error: {}", e))?;
    db::save_setting(&conn, "twitch_user_access_token", "").map_err(|e| format!("DB error: {}", e))?;
    db::save_setting(&conn, "twitch_refresh_token", "").map_err(|e| format!("DB error: {}", e))?;
    db::save_setting(&conn, "twitch_user_id", "").map_err(|e| format!("DB error: {}", e))?;
    db::save_setting(&conn, "twitch_login", "").map_err(|e| format!("DB error: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn get_channels(db: State<'_, DbConn>) -> Result<Vec<db::ChannelRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_all_channels(&conn).map_err(|e| format!("DB error: {}", e))
}

/// Try to refresh the Twitch user access token using the stored refresh token.
/// On success, saves the new tokens to the DB and returns the new access token.
pub(crate) async fn try_refresh_twitch_token(db: &std::sync::Mutex<rusqlite::Connection>) -> Result<String, String> {
    let refresh_token = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::get_setting(&conn, "twitch_refresh_token")
            .map_err(|e| format!("DB error: {}", e))?
            .unwrap_or_default()
    };

    if refresh_token.is_empty() {
        return Err("No refresh token available. Please log out and log in again.".into());
    }

    let token_resp = twitch::refresh_access_token(&refresh_token).await?;

    // Save the new tokens
    {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::save_setting(&conn, "twitch_user_access_token", &token_resp.access_token)
            .map_err(|e| format!("DB error: {}", e))?;
        if let Some(ref rt) = token_resp.refresh_token {
            db::save_setting(&conn, "twitch_refresh_token", rt)
                .map_err(|e| format!("DB error: {}", e))?;
        }
    }

    Ok(token_resp.access_token)
}
