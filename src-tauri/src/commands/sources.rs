//! Native-picker and configured-folder commands for local clip sources.

use std::path::PathBuf;

use chrono::Utc;
use rusqlite::Connection;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::db;
use crate::external_sources::{self, ExternalMediaCandidate, ExternalSourceConfig, ImportedClip};
use crate::DbConn;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderConnectionSettings {
    pub obs_port: u16,
    pub obs_password_set: bool,
}

fn obs_connection(conn: &Connection) -> Result<(u16, String), String> {
    let port = db::get_setting(conn, "obs_websocket_port")
        .map_err(|error| format!("Database error: {error}"))?
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(4455);
    let password = db::get_setting(conn, "obs_websocket_password")
        .map_err(|error| format!("Database error: {error}"))?
        .unwrap_or_default();
    Ok((port, password))
}

#[tauri::command]
pub fn get_external_source_configs(
    db: State<'_, DbConn>,
) -> Result<Vec<ExternalSourceConfig>, String> {
    let conn = db.lock().map_err(|error| format!("Database lock: {error}"))?;
    external_sources::SOURCE_KINDS
        .iter()
        .map(|kind| {
            let directory = db::get_setting(&conn, &format!("source_{kind}_dir"))
                .map_err(|error| format!("Database error: {error}"))?;
            let auto_import = db::get_setting(&conn, &format!("source_{kind}_auto_import"))
                .map_err(|error| format!("Database error: {error}"))?
                .as_deref()
                == Some("true");
            Ok(ExternalSourceConfig {
                kind: (*kind).to_string(),
                directory,
                auto_import,
            })
        })
        .collect()
}

#[tauri::command]
pub fn pick_external_source_folder(
    kind: String,
    app: AppHandle,
    db: State<'_, DbConn>,
) -> Result<Option<String>, String> {
    if !external_sources::SOURCE_KINDS.contains(&kind.as_str()) {
        return Err(format!("Unsupported clip source '{kind}'"));
    }
    let picked = app
        .dialog()
        .file()
        .set_title(format!("Choose the {kind} clips folder"))
        .blocking_pick_folder();
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|error| format!("Invalid selected folder: {error}"))?
        .canonicalize()
        .map_err(|error| format!("Could not open selected folder: {error}"))?;
    if !path.is_absolute() || !path.is_dir() {
        return Err("Choose a local folder".to_string());
    }
    let path_string = path.to_string_lossy().to_string();
    let conn = db.lock().map_err(|error| format!("Database lock: {error}"))?;
    db::save_setting(&conn, &format!("source_{kind}_dir"), &path_string)
        .map_err(|error| format!("Database error: {error}"))?;
    db::save_setting(
        &conn,
        &format!("source_{kind}_watch_after"),
        &Utc::now().to_rfc3339(),
    )
    .map_err(|error| format!("Database error: {error}"))?;
    Ok(Some(path_string))
}

#[tauri::command]
pub fn set_external_source_auto_import(
    kind: String,
    enabled: bool,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    if !external_sources::SOURCE_KINDS.contains(&kind.as_str()) {
        return Err(format!("Unsupported clip source '{kind}'"));
    }
    let conn = db.lock().map_err(|error| format!("Database lock: {error}"))?;
    if enabled {
        let directory = db::get_setting(&conn, &format!("source_{kind}_dir"))
            .map_err(|error| format!("Database error: {error}"))?;
        if directory.as_deref().map(str::trim).filter(|value| !value.is_empty()).is_none() {
            return Err(format!("Choose the {kind} clips folder before enabling auto-import"));
        }
        let was_enabled = db::get_setting(&conn, &format!("source_{kind}_auto_import"))
            .map_err(|error| format!("Database error: {error}"))?
            .as_deref()
            == Some("true");
        if !was_enabled {
            db::save_setting(
                &conn,
                &format!("source_{kind}_watch_after"),
                &Utc::now().to_rfc3339(),
            )
            .map_err(|error| format!("Database error: {error}"))?;
        }
    }
    db::save_setting(
        &conn,
        &format!("source_{kind}_auto_import"),
        if enabled { "true" } else { "false" },
    )
    .map_err(|error| format!("Database error: {error}"))
}

#[tauri::command]
pub fn scan_external_source(
    kind: String,
    db: State<'_, DbConn>,
) -> Result<Vec<ExternalMediaCandidate>, String> {
    let conn = db.lock().map_err(|error| format!("Database lock: {error}"))?;
    external_sources::scan_configured_source(&conn, &kind)
}

#[tauri::command]
pub async fn import_external_candidates(
    kind: String,
    candidate_ids: Vec<String>,
    app: AppHandle,
) -> Result<Vec<ImportedClip>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = db::db_path().map_err(|error| format!("Database path: {error}"))?;
        let mut conn = Connection::open(path)
            .map_err(|error| format!("Database open: {error}"))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| format!("Database timeout setup: {error}"))?;
        external_sources::import_candidate_ids(&app, &mut conn, &kind, &candidate_ids)
    })
    .await
    .map_err(|error| format!("Import task stopped unexpectedly: {error}"))?
}

#[tauri::command]
pub fn pick_and_import_media(
    app: AppHandle,
    db: State<'_, DbConn>,
) -> Result<Vec<ImportedClip>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("Import videos into ClipGoblin")
        .add_filter("Video", &["mp4", "mov", "m4v", "webm", "mkv", "flv"])
        .blocking_pick_files();
    let Some(files) = picked else {
        return Ok(Vec::new());
    };
    if files.len() > 200 {
        return Err("Import at most 200 clips at a time".to_string());
    }

    let paths: Result<Vec<_>, _> = files
        .into_iter()
        .map(|file| file.into_path().map_err(|error| format!("Invalid selected file: {error}")))
        .collect();
    let mut conn = db.lock().map_err(|error| format!("Database lock: {error}"))?;
    let mut imported = Vec::new();
    for path in paths? {
        let clip = external_sources::import_media_path(&app, &mut conn, &path, "manual")?;
        imported.push(clip);
    }
    Ok(imported)
}

fn preview_needs_mp4_proxy(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| matches!(extension.to_ascii_lowercase().as_str(), "mkv" | "flv"))
        .unwrap_or(false)
}

fn run_preview_conversion(
    ffmpeg: &std::path::Path,
    source: &std::path::Path,
    partial: &std::path::Path,
    transcode: bool,
) -> Result<(), String> {
    let mut command = std::process::Command::new(ffmpeg);
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(source)
        .arg("-map")
        .arg("0:v:0")
        .arg("-map")
        .arg("0:a:0?");
    if transcode {
        command
            .arg("-c:v")
            .arg("libx264")
            .arg("-preset")
            .arg("veryfast")
            .arg("-crf")
            .arg("21")
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg("-c:a")
            .arg("aac")
            .arg("-b:a")
            .arg("160k");
    } else {
        command.arg("-c").arg("copy");
    }
    command.arg("-movflags").arg("+faststart").arg(partial);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command
        .output()
        .map_err(|error| format!("Could not start ffmpeg: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if detail.is_empty() {
            "ffmpeg could not prepare this video for preview".to_string()
        } else {
            detail
        })
    }
}

fn allow_preview_file(app: &AppHandle, path: &std::path::Path) -> Result<String, String> {
    app.asset_protocol_scope()
        .allow_file(path)
        .map_err(|error| format!("Could not allow this imported video preview: {error}"))?;
    Ok(path.to_string_lossy().to_string())
}

async fn wait_for_stable_media_file(path: &std::path::Path) -> Result<(), String> {
    let mut previous = None;
    let mut stable_observations = 0_u8;
    for _ in 0..30 {
        let signature = std::fs::metadata(path)
            .ok()
            .filter(|metadata| metadata.is_file() && metadata.len() > 0)
            .map(|metadata| (metadata.len(), metadata.modified().ok()));
        if signature.is_some() && signature == previous {
            stable_observations += 1;
            if stable_observations >= 3 {
                return Ok(());
            }
        } else {
            stable_observations = 0;
        }
        previous = signature;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err("The recorder created a clip file, but it did not finish writing in time".to_string())
}

/// Return a WebView-compatible preview source for a trusted clip row.
/// MKV/FLV files are cached as MP4 without altering the imported original.
#[tauri::command]
pub async fn prepare_clip_preview_source(
    clip_id: String,
    app: AppHandle,
    db: State<'_, DbConn>,
) -> Result<String, String> {
    let (source, cache_key) = {
        let conn = db.lock().map_err(|error| format!("Database lock: {error}"))?;
        let clip = db::get_clip_by_id(&conn, &clip_id)
            .map_err(|error| format!("Database error: {error}"))?
            .ok_or_else(|| "Clip not found".to_string())?;
        let source = clip
            .source_media_path
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "This clip does not use imported media".to_string())?;
        let cache_key = clip
            .source_fingerprint
            .filter(|value| value.chars().all(|character| character.is_ascii_hexdigit()))
            .unwrap_or_else(|| {
                clip.id
                    .chars()
                    .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
                    .collect()
            });
        (PathBuf::from(source), cache_key)
    };

    let source = source
        .canonicalize()
        .map_err(|error| format!("The imported source video is unavailable: {error}"))?;
    if !source.is_file() {
        return Err("The imported source video is unavailable".to_string());
    }
    if !preview_needs_mp4_proxy(&source) {
        return allow_preview_file(&app, &source);
    }

    let preview_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Could not locate the app data folder: {error}"))?
        .join("clip-previews");
    std::fs::create_dir_all(&preview_dir)
        .map_err(|error| format!("Could not create the preview cache: {error}"))?;
    let output = preview_dir.join(format!("{cache_key}.mp4"));
    let cache_is_fresh = output.is_file()
        && output.metadata().map(|metadata| metadata.len() > 0).unwrap_or(false)
        && match (source.metadata().and_then(|metadata| metadata.modified()), output.metadata().and_then(|metadata| metadata.modified())) {
            (Ok(source_modified), Ok(output_modified)) => output_modified >= source_modified,
            _ => true,
        };
    if cache_is_fresh {
        return allow_preview_file(&app, &output);
    }

    let partial = preview_dir.join(format!("{cache_key}.partial.mp4"));
    let ffmpeg = crate::commands::vod::find_ffmpeg().map_err(|error| error.to_string())?;
    let source_for_task = source.clone();
    let output_for_task = output.clone();
    let partial_for_task = partial.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = std::fs::remove_file(&partial_for_task);
        if run_preview_conversion(&ffmpeg, &source_for_task, &partial_for_task, false).is_err() {
            let _ = std::fs::remove_file(&partial_for_task);
            run_preview_conversion(&ffmpeg, &source_for_task, &partial_for_task, true)?;
        }
        if output_for_task.exists() {
            std::fs::remove_file(&output_for_task)
                .map_err(|error| format!("Could not replace the cached preview: {error}"))?;
        }
        std::fs::rename(&partial_for_task, &output_for_task)
            .map_err(|error| format!("Could not finish the cached preview: {error}"))
    })
    .await
    .map_err(|error| format!("Preview preparation stopped unexpectedly: {error}"))??;

    allow_preview_file(&app, &output)
}

#[tauri::command]
pub fn get_recorder_connection_settings(
    db: State<'_, DbConn>,
) -> Result<RecorderConnectionSettings, String> {
    let conn = db.lock().map_err(|error| format!("Database lock: {error}"))?;
    let (obs_port, password) = obs_connection(&conn)?;
    Ok(RecorderConnectionSettings {
        obs_port,
        obs_password_set: !password.is_empty(),
    })
}

#[tauri::command]
pub fn save_obs_connection_settings(
    port: u16,
    password: Option<String>,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    if port < 1024 {
        return Err("Use the OBS WebSocket port shown in OBS settings (normally 4455)".to_string());
    }
    let conn = db.lock().map_err(|error| format!("Database lock: {error}"))?;
    db::save_setting(&conn, "obs_websocket_port", &port.to_string())
        .map_err(|error| format!("Database error: {error}"))?;
    if let Some(password) = password {
        db::save_setting(&conn, "obs_websocket_password", password.trim())
            .map_err(|error| format!("Could not save the OBS password securely: {error}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn test_recorder_connection(
    kind: String,
    db: State<'_, DbConn>,
) -> Result<crate::recorders::RecorderStatus, String> {
    match kind.as_str() {
        "obs" => {
            let (port, password) = {
                let conn = db.lock().map_err(|error| format!("Database lock: {error}"))?;
                obs_connection(&conn)?
            };
            crate::recorders::obs_status(port, &password).await
        }
        "meld" => crate::recorders::meld_status().await,
        _ => Err(format!("Unsupported recorder '{kind}'")),
    }
}

#[tauri::command]
pub async fn save_replay_and_import(
    kind: String,
    app: AppHandle,
    db: State<'_, DbConn>,
) -> Result<ImportedClip, String> {
    match kind.as_str() {
        "obs" => {
            let (port, password) = {
                let conn = db.lock().map_err(|error| format!("Database lock: {error}"))?;
                obs_connection(&conn)?
            };
            let path = crate::recorders::obs_save_replay(port, &password).await?;
            wait_for_stable_media_file(&path).await?;
            let mut conn = db.lock().map_err(|error| format!("Database lock: {error}"))?;
            external_sources::import_media_path(&app, &mut conn, &path, "obs")
        }
        "meld" => {
            let (before, started_at) = {
                let conn = db.lock().map_err(|error| format!("Database lock: {error}"))?;
                let before = external_sources::scan_configured_source(&conn, "meld")?
                    .into_iter()
                    .map(|candidate| candidate.id)
                    .collect::<std::collections::HashSet<_>>();
                (before, Utc::now())
            };
            crate::recorders::meld_record_clip().await?;

            let mut saved_path = None;
            for _ in 0..40 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let candidates = {
                    let conn = db.lock().map_err(|error| format!("Database lock: {error}"))?;
                    external_sources::scan_configured_source(&conn, "meld")?
                };
                saved_path = candidates
                    .into_iter()
                    .filter(|candidate| !before.contains(&candidate.id))
                    .filter(|candidate| {
                        chrono::DateTime::parse_from_rfc3339(&candidate.recorded_at)
                            .map(|value| value.with_timezone(&Utc) >= started_at - chrono::Duration::seconds(2))
                            .unwrap_or(false)
                    })
                    .map(|candidate| PathBuf::from(candidate.path))
                    .next();
                if saved_path.is_some() {
                    break;
                }
            }
            let path = saved_path.ok_or_else(|| {
                "Meld accepted the clip command, but no new file appeared in its configured clips folder"
                    .to_string()
            })?;
            wait_for_stable_media_file(&path).await?;
            let mut conn = db.lock().map_err(|error| format!("Database lock: {error}"))?;
            external_sources::import_media_path(&app, &mut conn, &path, "meld")
        }
        _ => Err(format!("Unsupported recorder '{kind}'")),
    }
}

#[tauri::command]
pub fn create_stream_marker(
    recorder_kind: String,
    label: Option<String>,
    db: State<'_, DbConn>,
) -> Result<db::StreamMarkerRow, String> {
    if recorder_kind != "obs" && recorder_kind != "meld" && recorder_kind != "manual" {
        return Err(format!("Unsupported recorder '{recorder_kind}'"));
    }
    let label = label
        .map(|value| value.trim().chars().take(120).collect::<String>())
        .filter(|value| !value.is_empty());
    let conn = db.lock().map_err(|error| format!("Database lock: {error}"))?;
    let channel_id = db::get_all_channels(&conn)
        .map_err(|error| format!("Database error: {error}"))?
        .into_iter()
        .next()
        .map(|channel| channel.id);
    db::insert_stream_marker(
        &conn,
        &recorder_kind,
        channel_id.as_deref(),
        label.as_deref(),
    )
    .map_err(|error| format!("Could not save stream marker: {error}"))
}

#[tauri::command]
pub fn list_recent_stream_markers(
    db: State<'_, DbConn>,
) -> Result<Vec<db::StreamMarkerRow>, String> {
    let conn = db.lock().map_err(|error| format!("Database lock: {error}"))?;
    db::get_recent_stream_markers(&conn, 20)
        .map_err(|error| format!("Database error: {error}"))
}
