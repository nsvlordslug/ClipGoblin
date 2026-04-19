mod bin_manager;
mod ai_provider;
mod auth_proxy;
mod crypto;
mod audio_signal;
mod clip_fusion;
mod clip_labeler;
mod clip_output;
mod clip_ranker;
mod db;
mod engine;
mod integration_test;
mod clip_selector;
mod commands;
mod error;
mod hardware;
mod job_queue;
mod pipeline;
mod post_captions;
mod scene_signal;
mod transcript_signal;
mod twitch;
mod log_scrubber;
mod social;
mod vertical_crop;
mod whisper;

use std::sync::Mutex;
use rusqlite::Connection;
use tauri::{AppHandle, Manager, State};
use error::AppError;
use job_queue::JobQueue;

/// Database connection type shared across commands.
pub(crate) type DbConn = Mutex<Connection>;

/// Emit a structured `"job-error"` event to the frontend AND convert to String.
/// Use at Tauri command boundaries for errors that should notify the UI.
pub(crate) fn report_error(app: &AppHandle, err: AppError) -> String {
    use tauri::Emitter;
    log::error!("[{}] {}", err.category(), err.detail());
    let _ = app.emit("job-error", err.to_event());
    err.to_string()
}

// ── Re-export command functions for the invoke_handler ──
// Tauri's generate_handler![] macro requires unqualified names, so we
// pull every command into this module's namespace via `use`.

use commands::auth::{twitch_login, twitch_logout, get_logged_in_user, get_channels};
use commands::bug_report::submit_bug_report;
use commands::captions::{generate_post_captions, generate_ai_title, test_ai_connection};
use commands::clip::{update_clip_settings, get_clip_detail, save_clip_to_disk};
use commands::export::{export_clip, set_clip_thumbnail, generate_clip_captions};
use commands::model::{check_model_status, download_model, delete_model};
use commands::binaries::{check_binary_status, download_binaries};
use commands::scheduled::{
    schedule_upload, list_scheduled_uploads, get_scheduled_uploads_for_clip,
    cancel_scheduled_upload, reschedule_upload, start_upload_scheduler,
};
use commands::settings::{
    save_setting, get_setting, open_url, get_app_info, get_hardware_info,
    list_jobs, get_job, remove_job, pick_download_folder, get_download_dir,
    get_storage_paths, open_folder, get_detection_stats,
};
use commands::social::{
    connect_platform, disconnect_platform, get_connected_account,
    get_all_connected_accounts, upload_to_platform, get_upload_status,
    get_clip_upload_history, restore_deleted_vods,
};
use commands::vod::{
    download_vod, get_cached_vods, analyze_vod, open_vod, get_vods,
    get_highlights, get_all_highlights, get_clips, delete_clip,
    refresh_vod_metadata, set_clip_game, set_clip_title, set_clip_publish_meta,
    set_vod_game, delete_vod_file, delete_vod_and_clips, get_vod_disk_usage,
    get_vod_detail, set_vod_analysis_status, save_clip_performance,
    get_clip_performance, get_creator_profile, update_scoring_from_performance,
    get_transcript,
};

// ── Steam init (only compiled with `steam` feature) ──

#[cfg(feature = "steam")]
fn init_steam() -> Result<(), String> {
    let (client, _single) = steamworks::Client::init_app(480)
        .map_err(|e| format!("Steam init failed: {}", e))?;
    log::info!("Steamworks SDK initialized");
    Ok(())
}

// ── App entry point ──

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = dotenvy::dotenv();

    let conn = db::init_db().expect("Failed to initialize database");

    // Recover any clips that were stuck mid-render when the app last closed
    db::recover_stale_rendering(&conn).ok();

    let hw = hardware::detect_hardware();

    #[cfg(feature = "steam")]
    {
        if let Err(e) = init_steam() {
            log::warn!("Steam not available: {}", e);
        }
    }

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().level(if cfg!(debug_assertions) {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        }).build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init());

    #[cfg(feature = "standalone")]
    {
        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    }

    builder
        .manage(Mutex::new(conn))
        .manage(hw)
        .manage(JobQueue::new())
        .invoke_handler(tauri::generate_handler![
            twitch_login,
            twitch_logout,
            get_logged_in_user,
            get_channels,
            get_vods,
            get_highlights,
            get_clips,
            delete_clip,
            save_setting,
            get_setting,
            get_app_info,
            get_hardware_info,
            list_jobs,
            get_job,
            remove_job,
            download_vod,
            analyze_vod,
            open_vod,
            get_cached_vods,
            pick_download_folder,
            get_download_dir,
            get_vod_detail,
            export_clip,
            set_clip_thumbnail,
            generate_clip_captions,
            update_clip_settings,
            get_clip_detail,
            get_all_highlights,
            generate_post_captions,
            generate_ai_title,
            test_ai_connection,
            save_clip_performance,
            get_clip_performance,
            get_creator_profile,
            update_scoring_from_performance,
            get_transcript,
            connect_platform,
            disconnect_platform,
            get_connected_account,
            get_all_connected_accounts,
            upload_to_platform,
            get_upload_status,
            get_clip_upload_history,
            restore_deleted_vods,
            schedule_upload,
            list_scheduled_uploads,
            get_scheduled_uploads_for_clip,
            cancel_scheduled_upload,
            reschedule_upload,
            open_url,
            save_clip_to_disk,
            refresh_vod_metadata,
            set_clip_game,
            set_clip_title,
            set_clip_publish_meta,
            set_vod_game,
            delete_vod_file,
            delete_vod_and_clips,
            get_vod_disk_usage,
            set_vod_analysis_status,
            get_storage_paths,
            open_folder,
            get_detection_stats,
            submit_bug_report,
            check_model_status,
            download_model,
            delete_model,
            check_binary_status,
            download_binaries,
        ])
        .setup(|app| {
            // Wire job queue events into Tauri's frontend event system.
            let queue: State<'_, JobQueue> = app.state();
            let handle = app.handle().clone();
            queue.on_progress(move |event| {
                use tauri::Emitter;
                let _ = handle.emit("job-progress", &event);
            });

            // Start background upload scheduler
            let scheduler_handle = app.handle().clone();
            std::thread::spawn(move || {
                start_upload_scheduler(scheduler_handle);
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
