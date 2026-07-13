//! Settings, utility, and system info commands.

use std::path::{Component, Path, PathBuf};
use tauri::{AppHandle, Manager, State};
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
    "theme",
    "auto_analyze",
    "tiktok_handle",
    "ui_settings",
    "clip_templates",
    "whisper_model",
    "detection_sensitivity",
    "use_twitch_community_clips",
    "ai_clip_detection_enabled",
];

pub(crate) fn app_data_root() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("clipviral")
}

fn is_plain_local_absolute(path: &Path) -> bool {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return false;
    }
    #[cfg(windows)]
    {
        use std::path::Prefix;
        return matches!(
            path.components().next(),
            Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))
        );
    }
    #[cfg(not(windows))]
    true
}

fn canonical_candidate(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        return path
            .canonicalize()
            .map_err(|e| format!("Failed to resolve path: {e}"));
    }

    let mut ancestor = path.parent();
    while let Some(current) = ancestor {
        if current.exists() {
            let canonical_ancestor = current
                .canonicalize()
                .map_err(|e| format!("Failed to resolve parent directory: {e}"))?;
            let suffix = path
                .strip_prefix(current)
                .map_err(|_| "Failed to normalize path".to_string())?;
            return Ok(canonical_ancestor.join(suffix));
        }
        ancestor = current.parent();
    }
    Err("Path has no existing local parent".to_string())
}

fn is_within_root(path: &Path, root: &Path) -> bool {
    let Ok(candidate) = canonical_candidate(path) else {
        return false;
    };
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    candidate.starts_with(root)
}

pub(crate) fn persist_download_directory(
    app: &AppHandle,
    db_conn: &DbConn,
    path: &Path,
) -> Result<String, String> {
    if !is_plain_local_absolute(path) || !path.is_dir() {
        return Err("Selected download folder must be an existing local directory".to_string());
    }
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("Failed to resolve selected folder: {e}"))?;
    app.asset_protocol_scope()
        .allow_directory(&canonical, true)
        .map_err(|e| format!("Failed to allow selected media folder: {e}"))?;
    let path_str = canonical.to_string_lossy().to_string();
    let conn = db_conn.lock().map_err(|e| format!("DB lock: {e}"))?;
    db::save_setting(&conn, "download_dir", &path_str).map_err(|e| format!("DB error: {e}"))?;
    Ok(path_str)
}

pub(crate) fn allow_configured_asset_directories(
    app: &AppHandle,
    db_conn: &DbConn,
) -> Result<(), String> {
    let base = app_data_root();
    std::fs::create_dir_all(&base)
        .map_err(|e| format!("Failed to create app data directory: {e}"))?;
    app.asset_protocol_scope()
        .allow_directory(&base, true)
        .map_err(|e| format!("Failed to allow app media directory: {e}"))?;

    let configured = {
        let conn = db_conn.lock().map_err(|e| format!("DB lock: {e}"))?;
        db::get_setting(&conn, "download_dir").map_err(|e| format!("DB error: {e}"))?
    };
    if let Some(path) = configured
        .map(PathBuf::from)
        .filter(|path| is_plain_local_absolute(path) && path.is_dir())
    {
        app.asset_protocol_scope()
            .allow_directory(path, true)
            .map_err(|e| format!("Failed to allow configured media folder: {e}"))?;
    }
    Ok(())
}

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
    // Only hand web URLs to the OS opener — never a local path, file://, or other
    // scheme that explorer/open/xdg-open would route to an arbitrary handler.
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Refusing to open a non-http(s) URL".to_string());
    }
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

/// Phase 6.0 — return rolling AI usage cost summary for Settings display.
/// Reads from `ai_usage_log` and computes:
///   - avg cost per VOD analyze across the last `lookback_vods` (default 10)
///   - 30-day total spend
///   - count of distinct VODs in the rolling window
/// Returns zeros across the board if the log is empty (e.g. user hasn't
/// run an analyze yet, or BYOK has been off).
#[tauri::command]
pub fn get_ai_cost_summary(
    lookback_vods: Option<u32>,
    db: State<'_, DbConn>,
) -> Result<crate::ai_usage::CostSummary, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    let lookback = lookback_vods.unwrap_or(10).clamp(1, 100);
    Ok(crate::ai_usage::estimate_cost(&conn, lookback))
}

/// Phase 1 (BYOK cost visibility) — estimate the BYOK cost to analyze a VOD
/// of `duration_secs` BEFORE the user kicks it off, so spend is never a
/// surprise. Rendered next to the Analyze action.
///
/// Returns 0.0 when no paid AI step will run for an analyze (clip-judge toggle
/// off or its provider Free, and LLM titles off too — mirrors the pipeline gates).
/// The projection prices the judge, optional final pass, and per-clip title
/// calls independently using their currently resolved models.
#[tauri::command]
pub fn estimate_analyze_cost(duration_secs: f64, db: State<'_, DbConn>) -> Result<f64, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;

    // Two independent BYOK costs can occur during an analyze, so mirror the exact
    // gates the pipeline uses:
    //  • the clip judge — only when the detection toggle is on AND the ClipJudge
    //    scope resolves to a real provider (see `run_ai_judge`);
    //  • best-effort LLM clip titles — only when the Titles scope resolves to a
    //    real provider (see `upgrade_titles_with_llm`).
    // If neither will run, there is no BYOK spend to estimate.
    let judge_enabled = crate::db::get_setting(&conn, "ai_clip_detection_enabled")
        .ok()
        .flatten()
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    let judge = crate::ai_provider::resolve(&conn, crate::ai_provider::Scope::ClipJudge);
    let judge_will_run = judge_enabled && judge.is_llm();
    let titles = crate::ai_provider::resolve(&conn, crate::ai_provider::Scope::Titles);
    let titles_will_run = titles.is_llm();
    if !judge_will_run && !titles_will_run {
        return Ok(0.0);
    }

    let judge_step = judge_will_run.then_some((judge.provider, judge.model.as_str()));
    let final_model = crate::ai_provider::final_pass_model();
    let final_step = (judge_will_run && judge.use_sonnet_final_pass)
        .then_some((crate::ai_provider::Provider::Claude, final_model.as_str()));
    let title_step = titles_will_run.then_some((titles.provider, titles.model.as_str()));

    Ok(crate::ai_usage::project_analyze_cost(
        duration_secs,
        judge_step,
        final_step,
        title_step,
    ))
}

/// Phase 1 (BYOK cost visibility) — total BYOK spend already logged for one
/// VOD across every AI call (clip judge + analysis-time titles/captions +
/// any later regens tagged with this VOD). Backs the post-analyze
/// "this analyze cost ~$Y" readout. Returns 0.0 for an unknown VOD.
#[tauri::command]
pub fn get_analysis_cost(
    vod_id: String,
    db: State<'_, DbConn>,
) -> Result<f64, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    Ok(crate::ai_usage::sum_cost_for_vod(&conn, &vod_id))
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
pub fn pick_download_folder(
    app: AppHandle,
    db: State<'_, DbConn>,
) -> Result<Option<String>, String> {
    let path = app
        .dialog()
        .file()
        .set_title("Select Download Folder")
        .blocking_pick_folder();

    match path {
        Some(file_path) => {
            let path = file_path
                .into_path()
                .map_err(|e| format!("Invalid selected folder: {e}"))?;
            let path_str = persist_download_directory(&app, &*db, &path)?;
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

/// Open an app-owned or explicitly selected folder in the system file manager.
#[tauri::command]
pub fn open_folder(path: String, app: AppHandle, db: State<'_, DbConn>) -> Result<(), String> {
    let dir = Path::new(&path);
    if !is_plain_local_absolute(dir) {
        return Err("Refusing to open a non-local filesystem path".to_string());
    }

    let base = app_data_root();
    std::fs::create_dir_all(&base)
        .map_err(|e| format!("Failed to create app data directory: {e}"))?;
    let configured = {
        let conn = db.lock().map_err(|e| format!("DB lock: {e}"))?;
        db::get_setting(&conn, "download_dir")
            .ok()
            .flatten()
            .map(PathBuf::from)
    };
    let allowed = is_within_root(dir, &base)
        || configured
            .as_deref()
            .filter(|root| is_plain_local_absolute(root) && root.is_dir())
            .is_some_and(|root| is_within_root(dir, root));
    if !allowed {
        return Err("Refusing to open a folder outside ClipGoblin storage".to_string());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("Failed to create directory: {e}"))?;
    app.asset_protocol_scope()
        .allow_directory(dir, true)
        .map_err(|e| format!("Failed to allow media folder: {e}"))?;

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
pub fn get_detection_stats(
    vod_id: String,
    db: State<'_, DbConn>,
) -> Result<Option<serde_json::Value>, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn local_path_validation_rejects_relative_unc_device_and_traversal_paths() {
        assert!(is_plain_local_absolute(Path::new(
            r"C:\Users\tester\Videos\ClipGoblin"
        )));
        assert!(!is_plain_local_absolute(Path::new(r"Videos\ClipGoblin")));
        assert!(!is_plain_local_absolute(Path::new(
            r"C:\Users\tester\..\Windows"
        )));
        assert!(!is_plain_local_absolute(Path::new(
            r"\\server\share\ClipGoblin"
        )));
        assert!(!is_plain_local_absolute(Path::new(
            r"\\?\C:\Users\tester\Videos"
        )));
    }

    #[test]
    fn root_containment_accepts_children_and_rejects_siblings() {
        let test_root =
            std::env::temp_dir().join(format!("clipviral-settings-test-{}", uuid::Uuid::new_v4()));
        let allowed_root = test_root.join("allowed");
        let sibling = test_root.join("outside");
        std::fs::create_dir_all(&allowed_root).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();

        assert!(is_within_root(
            &allowed_root.join("nested").join("future"),
            &allowed_root
        ));
        assert!(!is_within_root(&sibling, &allowed_root));

        std::fs::remove_dir_all(test_root).unwrap();
    }
}
