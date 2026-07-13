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
    caption_font_scale: f64,
    facecam_layout: String,
    game: Option<String>,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::update_clip_settings(
        &conn, &clip_id, &title, start_seconds, end_seconds,
        &aspect_ratio, captions_enabled, captions_text.as_deref(),
        &captions_position, &caption_style, caption_font_scale, &facecam_layout,
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
        let path = clip
            .output_path
            .ok_or("No export file found for this clip")?;
        (path, clip.title, clip.aspect_ratio)
    };

    let src = std::path::Path::new(&output_path);
    if !src.exists() || std::fs::metadata(src).map(|m| m.len() == 0).unwrap_or(true) {
        return Err("Export file is missing or empty — re-export the clip".into());
    }

    // Build descriptive filename: [title]_[format].mp4
    let safe_title: String = clip_title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe_title = safe_title.trim().to_string();
    let format_tag = aspect_ratio.replace(':', "x"); // "9:16" → "9x16"
    let filename = if safe_title.is_empty() {
        format!("{}_{}.mp4", clip_id, format_tag)
    } else {
        format!("{}_{}.mp4", safe_title, format_tag)
    };

    // Resolve destination folder: use saved setting, or prompt user to pick one
    let saved_folder = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::get_setting(&conn, "download_dir").ok().flatten()
    };
    let dest_folder =
        if let Some(dir) = saved_folder.filter(|dir| std::path::Path::new(dir).is_dir()) {
            crate::commands::settings::persist_download_directory(
                &app,
                &*db,
                std::path::Path::new(&dir),
            )?
        } else {
            let picked = app
                .dialog()
                .file()
                .set_title("Choose a folder to save clips to")
                .blocking_pick_folder();
            match picked {
                Some(folder) => {
                    let path = folder
                        .into_path()
                        .map_err(|e| format!("Invalid selected folder: {e}"))?;
                    let folder_str =
                        crate::commands::settings::persist_download_directory(&app, &*db, &path)?;
                    log::info!("[save_clip_to_disk] Saved download folder: {}", folder_str);
                    folder_str
                }
                None => return Ok(None),
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

    std::fs::copy(src, &dest_path).map_err(|e| format!("Failed to save clip: {}", e))?;

    let dest_str = dest_path.to_string_lossy().to_string();
    log::info!(
        "[save_clip_to_disk] Saved clip {} to: {}",
        clip_id,
        dest_str
    );
    Ok(Some(dest_str))
}

/// Save a user-supplied review (rating + note) for a single highlight row.
/// Used by the dev-only Review UI behind the "Show clip review tools"
/// Settings toggle. Rating must be one of "good", "meh", "boring", or
/// `None` to clear.
#[tauri::command]
pub fn save_clip_review(
    highlight_id: String,
    rating: Option<String>,
    note: Option<String>,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    if let Some(ref r) = rating {
        if r != "good" && r != "meh" && r != "boring" {
            return Err(format!(
                "Invalid review rating '{}'. Expected 'good', 'meh', or 'boring'.",
                r
            ));
        }
    }

    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;

    db::set_clip_review(&conn, &highlight_id, rating.as_deref(), note.as_deref())
        .map_err(|e| format!("DB error saving review: {}", e))
}

/// Build a single JSON blob containing everything needed for offline analysis
/// of a VOD's clip-scoring quality: VOD metadata, the resolved detection
/// config (re-resolved at export time using the VOD's game_name + the
/// current sensitivity setting), and per-clip data including dimension
/// breakdown, signal sources, and any user reviews.
///
/// Frontend consumes this via `navigator.clipboard.writeText(...)`.
/// Used by the dev-only Review UI behind the "Show clip review tools"
/// Settings toggle.
///
/// **Caveat — config staleness:** the exported `config_resolved` reflects the
/// CURRENT sensitivity setting plus the per-game TOMLs as they exist now, NOT
/// a snapshot of what was active when the clips were originally scored. If the
/// user changes the sensitivity setting between analysis and export, the
/// dimension scores in `clips[]` were computed under the previous config.
/// The export annotates this in `config_note` for downstream consumers.
#[tauri::command]
pub fn export_review_data_for_vod(
    vod_id: String,
    db: State<'_, DbConn>,
) -> Result<String, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;

    // ── VOD metadata ──
    let vod = db::get_vod_by_id(&conn, &vod_id)
        .map_err(|e| format!("DB error fetching VOD: {}", e))?
        .ok_or_else(|| format!("VOD '{}' not found", vod_id))?;

    // ── Resolved config ──
    let sensitivity_str = db::get_setting(&conn, "detection_sensitivity")
        .ok().flatten()
        .unwrap_or_else(|| "medium".to_string());
    let sensitivity = crate::game_config::Sensitivity::from_str_or_default(&sensitivity_str);
    let resolved = crate::game_config::ResolvedConfig::resolve(
        vod.game_name.as_deref(),
        sensitivity,
    );

    let resolved_json = serde_json::json!({
        "audio_spike_threshold": resolved.audio.spike_threshold,
        "chat_emote_burst_threshold": resolved.chat.emote_burst_threshold,
        "chat_rate_min_msgs_per_window": resolved.chat.rate_min_msgs_per_window,
        "transcript_weight": resolved.transcript.weight,
        "selector_min_clip_duration": resolved.selector.min_clip_duration,
        "selector_max_clip_duration": resolved.selector.max_clip_duration,
        "selector_min_gap_between_clips": resolved.selector.min_gap_between_clips,
        "titles_preferred": resolved.titles.preferred_categories,
        "titles_disabled": resolved.titles.disabled_categories,
        "sensitivity": sensitivity_str,
    });

    // ── Per-clip data ──
    let highlights = db::get_highlights_by_vod(&conn, &vod_id)
        .map_err(|e| format!("DB error fetching highlights: {}", e))?;

    let clips_json: Vec<serde_json::Value> = highlights.iter().map(|h| {
        // Parse the stored JSON columns back into structured values so the
        // export reads as nested JSON, not as escaped strings. Log a warning
        // on parse failure so corrupted-JSON rows are distinguishable from
        // pre-Phase-C rows (both surface as `null` in the export).
        let dimensions: Option<serde_json::Value> = h.scoring_dimensions
            .as_deref()
            .and_then(|s| match serde_json::from_str(s) {
                Ok(v) => Some(v),
                Err(e) => {
                    log::warn!(
                        "[export] highlight {} has malformed scoring_dimensions JSON: {} (raw: {:?})",
                        h.id, e, s
                    );
                    None
                }
            });
        let sources: Option<serde_json::Value> = h.signal_sources
            .as_deref()
            .and_then(|s| match serde_json::from_str(s) {
                Ok(v) => Some(v),
                Err(e) => {
                    log::warn!(
                        "[export] highlight {} has malformed signal_sources JSON: {} (raw: {:?})",
                        h.id, e, s
                    );
                    None
                }
            });

        serde_json::json!({
            "highlight_id": h.id,
            "start_seconds": h.start_seconds,
            "end_seconds": h.end_seconds,
            "duration_seconds": h.end_seconds - h.start_seconds,
            "total_score": h.virality_score,
            "confidence_score": h.confidence_score,
            "dimensions": dimensions,
            "signal_sources": sources,
            "tags": h.tags,
            "transcript_snippet": h.transcript_snippet,
            "event_summary": h.event_summary,
            "review_rating": h.review_rating,
            "review_note": h.review_note,
        })
    }).collect();

    let payload = serde_json::json!({
        "vod": {
            "id": vod.id,
            "title": vod.title,
            "game_name": vod.game_name,
            "duration_seconds": vod.duration_seconds,
        },
        "config_resolved": resolved_json,
        "config_note": "Re-resolved at export time. Reflects the current sensitivity \
            setting and the per-game TOML files as they exist now. NOT a snapshot of \
            the config active when the clips in this export were originally scored.",
        "clips": clips_json,
        "exported_at": chrono::Utc::now().to_rfc3339(),
    });

    serde_json::to_string_pretty(&payload)
        .map_err(|e| format!("JSON serialization error: {}", e))
}
