//! Scheduled upload commands and background scheduler.

use tauri::State;
use crate::db;
use crate::social;
use crate::DbConn;

#[tauri::command]
pub fn schedule_upload(
    clip_id: String,
    platform: String,
    scheduled_time: String,
    meta_json: String,
    db: State<'_, DbConn>,
) -> Result<String, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let row = db::ScheduledUploadRow {
        id: id.clone(),
        clip_id,
        platform,
        scheduled_time,
        status: "pending".to_string(),
        retry_count: 0,
        error_message: None,
        video_url: None,
        upload_meta_json: Some(meta_json),
        created_at: now,
        view_count: None,
        like_count: None,
        ctr_percent: None,
        stats_updated_at: None,
    };
    db::insert_scheduled_upload(&conn, &row).map_err(|e| format!("DB error: {}", e))?;
    Ok(id)
}

#[tauri::command]
pub fn list_scheduled_uploads(db: State<'_, DbConn>) -> Result<Vec<db::ScheduledUploadRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_all_scheduled_uploads(&conn).map_err(|e| format!("DB error: {}", e))
}

#[tauri::command]
pub fn get_scheduled_uploads_for_clip(
    clip_id: String,
    db: State<'_, DbConn>,
) -> Result<Vec<db::ScheduledUploadRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_scheduled_uploads_for_clip(&conn, &clip_id).map_err(|e| format!("DB error: {}", e))
}

#[tauri::command]
pub fn cancel_scheduled_upload(id: String, db: State<'_, DbConn>) -> Result<bool, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::cancel_scheduled_upload(&conn, &id).map_err(|e| format!("DB error: {}", e))
}

#[tauri::command]
pub fn reschedule_upload(id: String, new_time: String, db: State<'_, DbConn>) -> Result<bool, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::reschedule_upload(&conn, &id, &new_time).map_err(|e| format!("DB error: {}", e))
}

// ── Background upload scheduler ──

/// Background scheduler: checks for due scheduled uploads every 60 seconds.
pub(crate) fn start_upload_scheduler(handle: tauri::AppHandle) {
    use std::time::Duration;

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create scheduler runtime");
        rt.block_on(async move {
            // Wait 10 seconds after startup before first check
            tokio::time::sleep(Duration::from_secs(10)).await;

            loop {
                // Process due uploads
                if let Err(e) = process_due_uploads(&handle) {
                    log::error!("[Scheduler] Error processing scheduled uploads: {}", e);
                }

                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        });
    });
}

pub(crate) fn process_due_uploads(handle: &tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    use tauri::Emitter;

    let db: tauri::State<'_, DbConn> = handle.state();
    let now = chrono::Utc::now().to_rfc3339();

    let due_uploads = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::get_due_scheduled_uploads(&conn, &now).map_err(|e| format!("DB error: {}", e))?
    };

    if due_uploads.is_empty() {
        return Ok(());
    }

    log::info!("[Scheduler] Found {} due scheduled upload(s)", due_uploads.len());

    for upload in due_uploads {
        // Mark as uploading
        {
            let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
            db::update_scheduled_upload_status(&conn, &upload.id, "uploading", None, None, None)
                .map_err(|e| format!("DB error: {}", e))?;
        }

        let _ = handle.emit("scheduled-upload-status", serde_json::json!({
            "id": upload.id, "status": "uploading", "clip_id": upload.clip_id, "platform": upload.platform,
        }));

        // Parse upload meta from stored JSON
        let meta: social::UploadMeta = match &upload.upload_meta_json {
            Some(json) => match serde_json::from_str(json) {
                Ok(m) => m,
                Err(e) => {
                    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
                    db::update_scheduled_upload_status(&conn, &upload.id, "failed", Some(&format!("Invalid meta: {}", e)), None, None).ok();
                    continue;
                }
            },
            None => {
                let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
                db::update_scheduled_upload_status(&conn, &upload.id, "failed", Some("Missing upload metadata"), None, None).ok();
                continue;
            }
        };

        // Get clip output path — auto-export if missing/invalid so the
        // scheduler can process Auto-ship uploads without a manual export step.
        let output_path = {
            let clip = {
                let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
                match db::get_clip_by_id(&conn, &upload.clip_id) {
                    Ok(Some(c)) => c,
                    _ => {
                        db::update_scheduled_upload_status(&conn, &upload.id, "failed", Some("Clip not found"), None, None).ok();
                        continue;
                    }
                }
            };
            match social::validate_export_file(clip.output_path.as_deref()) {
                Ok(p) => p.to_string(),
                Err(_missing) => {
                    // No existing export — try to render one now. This is the
                    // critical path for Auto-ship: user hasn't clicked Export,
                    // but the scheduled upload is due and we have a clip row.
                    log::info!(
                        "[Scheduler] Clip {} has no ready export — auto-exporting before upload",
                        upload.clip_id,
                    );
                    let _ = handle.emit("scheduled-upload-status", serde_json::json!({
                        "id": upload.id, "status": "exporting", "clip_id": upload.clip_id, "platform": upload.platform,
                    }));
                    match tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(
                            crate::commands::export::render_clip_by_id(&upload.clip_id)
                        )
                    }) {
                        Ok(path) => path.to_string_lossy().to_string(),
                        Err(e) => {
                            let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
                            db::update_scheduled_upload_status(
                                &conn, &upload.id, "failed",
                                Some(&format!("Auto-export failed: {}", e)),
                                None, None,
                            ).ok();
                            log::error!("[Scheduler] Auto-export failed for {}: {}", upload.clip_id, e);
                            continue;
                        }
                    }
                }
            }
        };

        // Perform the upload (synchronous, same pattern as upload_to_platform command)
        let adapter = match social::get_adapter(&upload.platform) {
            Ok(a) => a,
            Err(e) => {
                let conn = db.lock().map_err(|e2| format!("DB lock: {}", e2))?;
                db::update_scheduled_upload_status(&conn, &upload.id, "failed", Some(&format!("No adapter: {}", e)), None, None).ok();
                continue;
            }
        };

        let result = {
            let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(adapter.upload_video(&conn, &output_path, &meta))
            })
        };

        match result {
            Ok(ref upload_result) => {
                match &upload_result.status {
                    social::UploadResultStatus::Complete { video_url } => {
                        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
                        db::update_scheduled_upload_status(&conn, &upload.id, "completed", None, Some(video_url), None).ok();
                        db::upsert_upload(&conn, &upload.clip_id, &upload.platform, video_url).ok();
                        log::info!("[Scheduler] Upload completed: {} -> {}", upload.id, video_url);
                        let _ = handle.emit("scheduled-upload-status", serde_json::json!({
                            "id": upload.id, "status": "completed", "clip_id": upload.clip_id,
                            "platform": upload.platform, "video_url": video_url,
                        }));
                    }
                    social::UploadResultStatus::Duplicate { existing_url } => {
                        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
                        db::update_scheduled_upload_status(&conn, &upload.id, "completed", None, Some(existing_url), None).ok();
                        let _ = handle.emit("scheduled-upload-status", serde_json::json!({
                            "id": upload.id, "status": "completed", "clip_id": upload.clip_id,
                            "platform": upload.platform, "video_url": existing_url,
                        }));
                    }
                    social::UploadResultStatus::Failed { error } => {
                        handle_scheduled_failure(handle, &db, &upload, error);
                    }
                    _ => {
                        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
                        db::update_scheduled_upload_status(&conn, &upload.id, "completed", None, None, None).ok();
                    }
                }
            }
            Err(e) => {
                handle_scheduled_failure(handle, &db, &upload, &e.to_string());
            }
        }
    }

    Ok(())
}

pub(crate) fn handle_scheduled_failure(
    handle: &tauri::AppHandle,
    db: &std::sync::Mutex<rusqlite::Connection>,
    upload: &db::ScheduledUploadRow,
    error: &str,
) {
    use tauri::Emitter;
    let conn = match db.lock() {
        Ok(c) => c,
        Err(_) => return,
    };

    if upload.retry_count < 1 {
        log::warn!("[Scheduler] Upload {} failed (will retry): {}", upload.id, error);
        db::update_scheduled_upload_status(&conn, &upload.id, "pending", Some(error), None, Some(upload.retry_count + 1)).ok();
        let _ = handle.emit("scheduled-upload-status", serde_json::json!({
            "id": upload.id, "status": "retrying", "clip_id": upload.clip_id,
            "platform": upload.platform, "error": error,
        }));
    } else {
        log::error!("[Scheduler] Upload {} permanently failed: {}", upload.id, error);
        db::update_scheduled_upload_status(&conn, &upload.id, "failed", Some(error), None, None).ok();
        let _ = handle.emit("scheduled-upload-status", serde_json::json!({
            "id": upload.id, "status": "failed", "clip_id": upload.clip_id,
            "platform": upload.platform, "error": error,
        }));
    }
}
