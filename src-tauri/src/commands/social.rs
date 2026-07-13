//! Social platform connection and upload commands.

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::db;
use crate::DbConn;
use crate::social::{self, ConnectedAccount, UploadMeta, UploadResult};

/// Connect a social platform (YouTube, TikTok, Instagram) via OAuth.
///
/// Adapter futures are run with `block_in_place` because the shared trait is
/// `?Send`; adapters acquire the database only around brief synchronous work.
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
    let code = tokio::task::spawn_blocking(move || match plat.as_str() {
        "youtube" => social::youtube::wait_for_auth_code(listener),
        "tiktok" => social::tiktok::wait_for_auth_code(listener),
        _ => Err(crate::error::AppError::NotSupported(format!(
            "{} callback",
            plat
        ))),
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
    .map_err(|e| e.to_string())?;

    let account = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(adapter.handle_callback(&*db, &code))
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

    // Upload: adapter.upload_video takes the shared DbConn and locks internally
    // only for its DB reads/refresh + record — releasing the lock for the network
    // upload so it no longer blocks the whole app's DB. block_in_place because the
    // trait future is !Send (same pattern as connect_platform).
    let result = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(adapter.upload_video(&*db, &output_path, &meta))
    })
    .map_err(|e| {
        log::error!("[Upload] {} upload failed: {}", platform, e);
        e.to_string()
    })?;
    log::info!("[Upload] {} upload result: {:?}", platform, result.status);

    // Record this direct upload in the analytics ledger (scheduled_uploads) so it
    // appears in Analytics + the ScheduledUploads "Completed" section and gets
    // view-count refreshes. The scheduler creates its own row, so this only fires
    // for direct "Upload now" uploads — no duplicate rows. (Re-acquire the lock.)
    let analytics_state = match &result.status {
        social::UploadResultStatus::Complete {
            video_url,
            platform_video_id,
        } => Some((
            "completed",
            video_url.as_deref(),
            platform_video_id.as_deref(),
            None,
        )),
        social::UploadResultStatus::Processing if !result.job_id.is_empty() => {
            Some(("processing", None, None, None))
        }
        social::UploadResultStatus::Failed { error } => {
            Some(("failed", None, None, Some(error.as_str())))
        }
        _ => None,
    };
    if let Some((status, video_url, platform_video_id, error)) = analytics_state {
        let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
        if let Err(e) = db::record_direct_upload_state_for_analytics(
            &conn,
            &meta.clip_id,
            &platform,
            status,
            video_url,
            (!result.job_id.is_empty()).then_some(result.job_id.as_str()),
            platform_video_id,
            error,
        ) {
            log::warn!(
                "[Upload] failed to record analytics row for {}: {}",
                meta.clip_id,
                e
            );
        }
    }

    Ok(result)
}

/// Fetch TikTok creator info for the publish UI: allowed privacy levels,
/// interaction restrictions (comment/duet/stitch), and display name. TikTok's
/// Content Sharing Guidelines require the publish screen to reflect these, so
/// the compliance panel calls this on mount. Refreshes the access token first.
#[tauri::command]
pub async fn tiktok_get_creator_info(
    db: State<'_, DbConn>,
) -> Result<social::tiktok::TikTokCreatorInfo, String> {
    let rt = tokio::runtime::Handle::current();
    tokio::task::block_in_place(|| {
        rt.block_on(async {
            let token = social::tiktok::ensure_fresh_access_token(&*db)
                .await
                .map_err(|e| e.to_string())?;
            social::tiktok::fetch_creator_info(&token)
                .await
                .map_err(|e| e.to_string())
        })
    })
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

/// Summary returned by `refresh_upload_stats` — shown in a toast.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RefreshStatsSummary {
    pub updated: u32,
    pub skipped: u32,
    pub failed: u32,
}

/// Refresh view/like counts for every completed upload with a known video_url.
/// Per-upload failures are swallowed (logged + counted as `failed`) so one bad
/// row doesn't abort the whole sweep. Safe to call on-demand from the UI.
///
/// Uses the same `block_in_place`/`block_on` pattern as `connect_platform` because
/// rusqlite's `Connection` is `!Sync`, which prevents holding `&Connection` across
/// `.await` in a `Send` future. Running the inner future on the current worker
/// thread (via `block_on`) makes Send not required.
#[tauri::command]
pub async fn refresh_upload_stats(db: State<'_, DbConn>) -> Result<RefreshStatsSummary, String> {
    use crate::social::{tiktok, youtube};

    let rt = tokio::runtime::Handle::current();
    tokio::task::block_in_place(|| {
        rt.block_on(async {
            // 1. Snapshot the uploads that need refreshing (sync).
            let uploads = {
                let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
                db::get_completed_uploads_with_url(&conn).map_err(|e| format!("DB error: {}", e))?
            };

            let mut summary = RefreshStatsSummary {
                updated: 0,
                skipped: 0,
                failed: 0,
            };

            // 2. Cache each platform's access token once per refresh so we don't
            //    hammer the refresh endpoint per upload.
            let has_youtube = uploads.iter().any(|u| u.platform == "youtube");
            let has_tiktok = tiktok::video_stats_enabled()
                && uploads.iter().any(|u| u.platform == "tiktok");

            let yt_token: Option<String> = if has_youtube {
                match youtube::ensure_fresh_access_token(&*db).await {
                    Ok(t) => Some(t),
                    Err(e) => {
                        log::warn!("[refresh_upload_stats] YouTube token unavailable: {}", e);
                        None
                    }
                }
            } else {
                None
            };
            let tt_token: Option<String> = if has_tiktok {
                match tiktok::ensure_fresh_access_token(&*db).await {
                    Ok(t) => Some(t),
                    Err(e) => {
                        log::warn!("[refresh_upload_stats] TikTok token unavailable: {}", e);
                        None
                    }
                }
            } else {
                None
            };

            for u in uploads {
                match u.platform.as_str() {
                    "youtube" => {
                        let token = match &yt_token {
                            Some(t) => t,
                            None => {
                                summary.skipped += 1;
                                continue;
                            }
                        };
                        let video_id =
                            match u.platform_video_id.clone().or_else(|| {
                                u.video_url.as_deref().and_then(youtube::extract_video_id)
                            }) {
                                Some(id) => id,
                                None => {
                                    log::warn!(
                                        "[refresh_upload_stats] No YouTube video id for {}",
                                        u.id
                                    );
                                    summary.skipped += 1;
                                    continue;
                                }
                            };
                        match youtube::fetch_video_stats(token, &video_id).await {
                            Ok(stats) => {
                                let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
                                if let Err(e) = db::update_upload_stats(
                                    &conn,
                                    &u.id,
                                    stats.view_count,
                                    stats.like_count,
                                    None,
                                ) {
                                    log::error!(
                                        "[refresh_upload_stats] DB update failed for {}: {}",
                                        u.id,
                                        e
                                    );
                                    summary.failed += 1;
                                } else {
                                    summary.updated += 1;
                                }
                            }
                            Err(e) => {
                                log::warn!(
                                    "[refresh_upload_stats] YT stats failed for {}: {}",
                                    u.id,
                                    e
                                );
                                summary.failed += 1;
                            }
                        }
                    }
                    "tiktok" => {
                        let token = match &tt_token {
                            Some(t) => t,
                            None => {
                                summary.skipped += 1;
                                continue;
                            }
                        };
                        let video_id =
                            match u.platform_video_id.clone().or_else(|| {
                                u.video_url.as_deref().and_then(tiktok::extract_video_id)
                            }) {
                                Some(id) => id,
                                None => {
                                    log::warn!(
                                        "[refresh_upload_stats] No TikTok video id for {}",
                                        u.id
                                    );
                                    summary.skipped += 1;
                                    continue;
                                }
                            };
                        match tiktok::fetch_video_stats(token, &video_id).await {
                            Ok(stats) => {
                                let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
                                let stats_result = db::update_upload_stats(
                                    &conn,
                                    &u.id,
                                    stats.view_count,
                                    stats.like_count,
                                    None,
                                )
                                .and_then(|_| {
                                    db::update_upload_video_identity(
                                        &conn,
                                        &u.id,
                                        stats.share_url.as_deref(),
                                        Some(&video_id),
                                    )
                                });
                                if let Err(e) = stats_result {
                                    log::error!(
                                        "[refresh_upload_stats] DB update failed for {}: {}",
                                        u.id,
                                        e
                                    );
                                    summary.failed += 1;
                                } else {
                                    summary.updated += 1;
                                }
                            }
                            Err(e) => {
                                log::warn!(
                                    "[refresh_upload_stats] TT stats failed for {}: {}",
                                    u.id,
                                    e
                                );
                                summary.failed += 1;
                            }
                        }
                    }
                    _ => summary.skipped += 1,
                }
            }

            log::info!(
                "[refresh_upload_stats] updated={} skipped={} failed={}",
                summary.updated,
                summary.skipped,
                summary.failed,
            );
            Ok::<RefreshStatsSummary, String>(summary)
        })
    })
}
