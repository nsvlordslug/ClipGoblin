//! Clip editing and management commands.

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use crate::db;
use crate::DbConn;

#[tauri::command]
pub fn update_clip_settings(
    clip_id: String,
    title: String,
    start_seconds: f64,
    end_seconds: f64,
    aspect_ratio: String,
    captions_enabled: i32,
    captions_text: Option<String>,
    captions_position: String,
    caption_style: String,
    facecam_layout: String,
    game: Option<String>,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::update_clip_settings(
        &conn, &clip_id, &title, start_seconds, end_seconds,
        &aspect_ratio, captions_enabled, captions_text.as_deref(),
        &captions_position, &caption_style, &facecam_layout,
        game.as_deref(),
    ).map_err(|e| format!("DB error: {}", e))
}

#[tauri::command]
pub fn get_clip_detail(clip_id: String, db: State<'_, DbConn>) -> Result<db::ClipRow, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::get_clip_by_id(&conn, &clip_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or_else(|| "Clip not found".to_string())
}

#[tauri::command]
pub fn save_clip_to_disk(
    clip_id: String,
    app: AppHandle,
    db: State<'_, DbConn>,
) -> Result<Option<String>, String> {
    let (output_path, clip_title, aspect_ratio) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let clip = db::get_clip_by_id(&conn, &clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("Clip not found")?;
        if clip.render_status != "completed" {
            return Err("Clip has not been exported yet — export it first".into());
        }
        let path = clip.output_path.ok_or("No export file found for this clip")?;
        (path, clip.title, clip.aspect_ratio)
    };

    let src = std::path::Path::new(&output_path);
    if !src.exists() || std::fs::metadata(src).map(|m| m.len() == 0).unwrap_or(true) {
        return Err("Export file is missing or empty — re-export the clip".into());
    }

    // Build descriptive filename: [title]_[format].mp4
    let safe_title: String = clip_title.chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let safe_title = safe_title.trim().to_string();
    let format_tag = aspect_ratio.replace(':', "x"); // "9:16" → "9x16"
    let filename = if safe_title.is_empty() {
        format!("{}_{}.mp4", clip_id, format_tag)
    } else {
        format!("{}_{}.mp4", safe_title, format_tag)
    };

    // Resolve destination folder: use saved setting, or prompt user to pick one
    let dest_folder = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        match db::get_setting(&conn, "download_dir") {
            Ok(Some(dir)) if !dir.is_empty() && std::path::Path::new(&dir).is_dir() => dir,
            _ => {
                // No folder configured — open picker
                drop(conn); // release lock before blocking dialog
                let picked = app.dialog()
                    .file()
                    .set_title("Choose a folder to save clips to")
                    .blocking_pick_folder();
                match picked {
                    Some(folder) => {
                        let folder_str = folder.to_string();
                        // Save for future use
                        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
                        db::save_setting(&conn, "download_dir", &folder_str)
                            .map_err(|e| format!("DB error: {}", e))?;
                        log::info!("[save_clip_to_disk] Saved download folder: {}", folder_str);
                        folder_str
                    }
                    None => return Ok(None), // User cancelled
                }
            }
        }
    };

    // Ensure folder exists
    std::fs::create_dir_all(&dest_folder)
        .map_err(|e| format!("Failed to create download folder: {}", e))?;

    // Avoid overwriting — append (2), (3), etc. if file exists
    let dest_dir = std::path::Path::new(&dest_folder);
    let stem = filename.trim_end_matches(".mp4");
    let mut dest_path = dest_dir.join(&filename);
    let mut counter = 2u32;
    while dest_path.exists() {
        dest_path = dest_dir.join(format!("{} ({}).mp4", stem, counter));
        counter += 1;
    }

    std::fs::copy(src, &dest_path)
        .map_err(|e| format!("Failed to save clip: {}", e))?;

    let dest_str = dest_path.to_string_lossy().to_string();
    log::info!("[save_clip_to_disk] Saved clip {} to: {}", clip_id, dest_str);
    Ok(Some(dest_str))
}
