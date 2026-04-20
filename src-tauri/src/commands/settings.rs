//! Settings, utility, and system info commands.

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::db;
use crate::DbConn;
use crate::hardware::HardwareInfo;
use crate::job_queue::{Job, JobQueue};

#[derive(serde::Serialize)]
pub struct AppInfo {
    version: String,
    data_dir: String,
    db_path: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoragePaths {
    exports_dir: String,
    downloads_dir: String,
    data_dir: String,
}

const ALLOWED_SETTING_KEYS: &[&str] = &[
    "claude_api_key",
    "openai_api_key",
    "gemini_api_key",
    "ai_provider",
    "ai_settings",
    "download_dir",
    "theme",
    "auto_analyze",
    "tiktok_handle",
    "ui_settings",
    "clip_templates",
    "whisper_model",
    "detection_sensitivity",
    "use_twitch_community_clips",
];

#[tauri::command]
pub fn save_setting(
    key: String,
    value: String,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    if !ALLOWED_SETTING_KEYS.contains(&key.as_str()) {
        return Err(format!("Setting '{}' is not writable from the frontend", key));
    }
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::save_setting(&conn, &key, &value).map_err(|e| format!("DB error: {}", e))
}

#[tauri::command]
pub fn get_setting(
    key: String,
    db: State<'_, DbConn>,
) -> Result<Option<String>, String> {
    if !ALLOWED_SETTING_KEYS.contains(&key.as_str()) {
        return Err(format!("Setting '{}' is not readable from the frontend", key));
    }
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_setting(&conn, &key).map_err(|e| format!("DB error: {}", e))
}

#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        // explorer.exe properly delegates to the default browser using the
        // user's existing session — doesn't create new profiles or log them out.
        std::process::Command::new("explorer")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("Failed to open URL: {}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("Failed to open URL: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("Failed to open URL: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
pub fn get_app_info() -> Result<AppInfo, String> {
    let db_path = db::db_path().map_err(|e| format!("Data dir error: {e}"))?;
    let data_dir = db_path
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir,
        db_path: db_path.to_string_lossy().to_string(),
    })
}

/// Return the hardware profile detected at startup.
#[tauri::command]
pub fn get_hardware_info(hw: State<'_, HardwareInfo>) -> Result<HardwareInfo, String> {
    Ok(hw.inner().clone())
}

#[tauri::command]
pub fn list_jobs(queue: State<'_, JobQueue>) -> Vec<Job> {
    queue.list()
}

/// Return a single job's current state.
#[tauri::command]
pub fn get_job(id: String, queue: State<'_, JobQueue>) -> Option<Job> {
    queue.get(&id)
}

/// Remove a finished job from the queue.
#[tauri::command]
pub fn remove_job(id: String, queue: State<'_, JobQueue>) -> bool {
    queue.remove(&id)
}

/// Open a folder picker dialog and save the selected path as the download directory.
#[tauri::command]
pub fn pick_download_folder(app: AppHandle, db: State<'_, DbConn>) -> Result<Option<String>, String> {
    let path = app.dialog()
        .file()
        .set_title("Select Download Folder")
        .blocking_pick_folder();

    match path {
        Some(file_path) => {
            let path_str = file_path.to_string();
            let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
            db::save_setting(&conn, "download_dir", &path_str)
                .map_err(|e| format!("DB error: {}", e))?;
            Ok(Some(path_str))
        }
        None => Ok(None),
    }
}

/// Get the current download directory (from settings or default).
#[tauri::command]
pub fn get_download_dir(db: State<'_, DbConn>) -> Result<String, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    match db::get_setting(&conn, "download_dir") {
        Ok(Some(dir)) if !dir.is_empty() => Ok(dir),
        _ => {
            let default = dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("clipviral")
                .join("downloads");
            Ok(default.to_string_lossy().to_string())
        }
    }
}

/// Return the three key storage directories, creating them if needed.
#[tauri::command]
pub fn get_storage_paths(db: State<'_, DbConn>) -> Result<StoragePaths, String> {
    let base = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clipviral");

    let exports_dir = base.join("exports");
    let data_dir = base.clone();

    // Downloads dir may be user-configured
    let downloads_dir = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        match db::get_setting(&conn, "download_dir") {
            Ok(Some(dir)) if !dir.is_empty() => std::path::PathBuf::from(dir),
            _ => base.join("downloads"),
        }
    };

    Ok(StoragePaths {
        exports_dir: exports_dir.to_string_lossy().to_string(),
        downloads_dir: downloads_dir.to_string_lossy().to_string(),
        data_dir: data_dir.to_string_lossy().to_string(),
    })
}

/// Open a folder in the system file manager, creating it first if it doesn't exist.
#[tauri::command]
pub fn open_folder(path: String) -> Result<(), String> {
    let dir = std::path::Path::new(&path);
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("Failed to create directory: {e}"))?;

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(dir)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {e}"))?;
    }
    Ok(())
}

/// Get detection stats for a VOD (stored after analysis completes).
#[tauri::command]
pub fn get_detection_stats(vod_id: String, db: State<'_, DbConn>) -> Result<Option<serde_json::Value>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let key = format!("detection_stats_{}", vod_id);
    match db::get_setting(&conn, &key) {
        Ok(Some(json_str)) => {
            let val: serde_json::Value = serde_json::from_str(&json_str)
                .map_err(|e| format!("Failed to parse detection stats: {e}"))?;
            Ok(Some(val))
        }
        _ => Ok(None),
    }
}
