//! Clip editing and management commands.

use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use rusqlite::OptionalExtension;
use crate::db;
use crate::personalization::PersonalizationStatus;
use crate::DbConn;

const MAX_BRANDING_ASSET_BYTES: u64 = 50 * 1024 * 1024;

fn branding_dir() -> Result<std::path::PathBuf, String> {
    let dir = dirs::data_dir()
        .ok_or_else(|| "Could not locate ClipGoblin app storage".to_string())?
        .join("clipviral")
        .join("branding");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("Could not create branding storage: {error}"))?;
    dir.canonicalize()
        .map_err(|error| format!("Could not open branding storage: {error}"))
}

fn supported_branding_extension(path: &std::path::Path) -> Option<String> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif")
        .then_some(extension)
}

fn validate_staged_branding_path(path: Option<String>) -> Result<Option<String>, String> {
    let Some(path) = path.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    let candidate = std::path::PathBuf::from(path)
        .canonicalize()
        .map_err(|_| "The selected branding asset is missing. Choose it again.".to_string())?;
    let root = branding_dir()?;
    if !candidate.is_file()
        || !candidate.starts_with(&root)
        || supported_branding_extension(&candidate).is_none()
    {
        return Err("Choose a branding image or GIF through ClipGoblin".to_string());
    }
    Ok(Some(candidate.to_string_lossy().to_string()))
}

#[tauri::command]
pub fn pick_context_branding_asset(app: AppHandle) -> Result<Option<String>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("Choose a branding image or GIF")
        .add_filter("Branding image or GIF", &["png", "jpg", "jpeg", "webp", "gif"])
        .blocking_pick_file();
    let Some(picked) = picked else {
        return Ok(None);
    };
    let source = picked
        .into_path()
        .map_err(|error| format!("Invalid selected branding file: {error}"))?
        .canonicalize()
        .map_err(|error| format!("Could not open selected branding file: {error}"))?;
    let extension = supported_branding_extension(&source)
        .ok_or_else(|| "Choose a PNG, JPG, WebP, or GIF file".to_string())?;
    let metadata = std::fs::metadata(&source)
        .map_err(|error| format!("Could not inspect branding file: {error}"))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("The selected branding file is empty".to_string());
    }
    if metadata.len() > MAX_BRANDING_ASSET_BYTES {
        return Err("Branding files must be 50 MB or smaller".to_string());
    }

    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("branding")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .take(48)
        .collect::<String>();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let destination = branding_dir()?.join(format!("{stem}-{}.{}", &id[..8], extension));
    std::fs::copy(&source, &destination)
        .map_err(|error| format!("Could not copy branding into ClipGoblin: {error}"))?;
    app.asset_protocol_scope()
        .allow_file(&destination)
        .map_err(|error| format!("Could not allow branding preview: {error}"))?;
    Ok(Some(destination.to_string_lossy().to_string()))
}

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
    caption_y_offset: f64,
    facecam_layout: String,
    facecam_settings: Option<String>,
    context_background_path: Option<String>,
    context_background_mode: String,
    context_blur_strength: f64,
    context_video_y: f64,
    game: Option<String>,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let before = db::get_clip_by_id(&conn, &clip_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or_else(|| "Clip not found".to_string())?;
    let requested_branding = context_background_mode == "branding";
    let context_background_path = match validate_staged_branding_path(context_background_path) {
        Ok(path) => path,
        Err(_) if !requested_branding => None,
        Err(error) => return Err(error),
    };
    let context_background_mode = if requested_branding
        && context_background_path.is_some()
    {
        "branding"
    } else {
        "blur"
    };
    let facecam_settings = match facecam_settings.filter(|value| !value.trim().is_empty()) {
        Some(value) => {
            if value.len() > 4_096 {
                return Err("Layout settings are too large".to_string());
            }
            let settings = crate::vertical_crop::EditorLayoutSettings::parse_json(&value)
                .map_err(|_| "Invalid Split or Picture in Picture settings".to_string())?;
            Some(
                serde_json::to_string(&settings)
                    .map_err(|error| format!("Could not save layout settings: {error}"))?,
            )
        }
        None => None,
    };
    db::update_clip_settings(
        &conn, &clip_id, &title, start_seconds, end_seconds,
        &aspect_ratio, captions_enabled, captions_text.as_deref(),
        &captions_position, &caption_style, caption_font_scale, caption_y_offset,
        &facecam_layout,
        facecam_settings.as_deref(),
        context_background_path.as_deref(), context_background_mode,
        context_blur_strength, context_video_y,
        game.as_deref(),
    ).map_err(|e| format!("DB error: {}", e))?;

    if (before.start_seconds - start_seconds).abs() >= 0.75
        || (before.end_seconds - end_seconds).abs() >= 0.75
    {
        let dedupe_key = format!(
            "trim:{}:{:.1}:{:.1}:{:.1}:{:.1}",
            clip_id,
            before.start_seconds,
            before.end_seconds,
            start_seconds,
            end_seconds,
        );
        let metadata = serde_json::json!({
            "startDelta": start_seconds - before.start_seconds,
            "endDelta": end_seconds - before.end_seconds,
        })
        .to_string();
        db::record_clip_behavior(
            &conn,
            &clip_id,
            "trim",
            Some(0.72),
            0.35,
            Some(before.start_seconds),
            Some(before.end_seconds),
            Some(start_seconds),
            Some(end_seconds),
            Some(&metadata),
            &dedupe_key,
        )
        .map_err(|e| format!("DB error recording trim evidence: {}", e))?;
    }
    Ok(())
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

/// Save a moment rating plus independent edit-quality feedback.
#[tauri::command]
pub fn save_clip_review(
    highlight_id: String,
    rating: Option<String>,
    note: Option<String>,
    issues: Option<Vec<String>>,
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

    let note = note
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if note.as_ref().is_some_and(|value| value.chars().count() > 2_000) {
        return Err("Review notes must be 2,000 characters or fewer.".to_string());
    }

    const VALID_ISSUES: [&str; 5] = [
        "starts_too_late",
        "cuts_off_early",
        "too_long",
        "wrong_moment",
        "duplicate",
    ];
    let mut normalized_issues = Vec::new();
    for issue in issues.unwrap_or_default() {
        if !VALID_ISSUES.contains(&issue.as_str()) {
            return Err(format!("Invalid clip edit issue '{}'.", issue));
        }
        if !normalized_issues.contains(&issue) {
            normalized_issues.push(issue);
        }
    }
    let issues_json = if normalized_issues.is_empty() {
        None
    } else {
        Some(
            serde_json::to_string(&normalized_issues)
                .map_err(|e| format!("Could not encode clip edit issues: {}", e))?,
        )
    };

    let mut conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;

    db::set_clip_review(
        &mut conn,
        &highlight_id,
        rating.as_deref(),
        note.as_deref(),
        issues_json.as_deref(),
    )
    .map_err(|e| format!("DB error saving review: {}", e))?;

    let clip_id = conn
        .query_row(
            "SELECT id FROM clips WHERE highlight_id = ?1 LIMIT 1",
            [&highlight_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("DB error finding reviewed clip: {}", e))?;
    if let Some(clip_id) = clip_id {
        let dedupe_key = format!(
            "review:{}:{}:{}",
            clip_id,
            rating.as_deref().unwrap_or("none"),
            issues_json.as_deref().unwrap_or("[]"),
        );
        let metadata = serde_json::json!({
            "rating": rating,
            "issues": normalized_issues,
            "hasNote": note.is_some(),
        })
        .to_string();
        let _ = db::record_clip_behavior(
            &conn,
            &clip_id,
            "review",
            None,
            0.0,
            None,
            None,
            None,
            None,
            Some(&metadata),
            &dedupe_key,
        );
    }
    Ok(())
}

#[tauri::command]
pub fn record_clip_opened(clip_id: String, db: State<'_, DbConn>) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::record_clip_behavior(
        &conn,
        &clip_id,
        "open",
        Some(0.62),
        0.12,
        None,
        None,
        None,
        None,
        None,
        &format!("open:{clip_id}"),
    )
    .map(|_| ())
    .map_err(|e| format!("DB error recording editor open: {}", e))
}

#[tauri::command]
pub fn get_personalization_status(
    db: State<'_, DbConn>,
) -> Result<PersonalizationStatus, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let feedback = db::get_detection_feedback(&conn)
        .map_err(|e| format!("DB error loading personalization status: {}", e))?;
    let behavior = db::get_clip_behavior_events(&conn)
        .map_err(|e| format!("DB error loading behavior history: {}", e))?;
    let edit_feedback = db::get_clip_edit_feedback(&conn)
        .map_err(|e| format!("DB error loading boundary feedback: {}", e))?;
    Ok(PersonalizationStatus::from_all_evidence(
        &feedback,
        &behavior,
        &edit_feedback,
    ))
}

#[tauri::command]
pub fn export_personalization_history(db: State<'_, DbConn>) -> Result<String, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let behavior = db::get_clip_behavior_events(&conn)
        .map_err(|e| format!("DB error loading behavior history: {}", e))?;
    let feedback = db::get_detection_feedback(&conn)
        .map_err(|e| format!("DB error loading rating history: {}", e))?;
    let edit_feedback = db::get_clip_edit_feedback(&conn)
        .map_err(|e| format!("DB error loading boundary feedback: {}", e))?;
    serde_json::to_string_pretty(&serde_json::json!({
        "ratings": feedback.iter().map(|row| serde_json::json!({
            "channelId": row.channel_id,
            "gameName": row.game_name,
            "rating": row.rating,
            "dimensions": row.scoring_dimensions,
            "signalSources": row.signal_sources,
            "tags": row.tags,
        })).collect::<Vec<_>>(),
        "boundaryFeedback": edit_feedback,
        "behavior": behavior,
        "exportedAt": chrono::Utc::now().to_rfc3339(),
        "privacy": "Local ClipGoblin personalization history. No media or API keys are included."
    }))
    .map_err(|e| format!("Could not serialize personalization history: {}", e))
}

#[tauri::command]
pub fn reset_personalization_history(db: State<'_, DbConn>) -> Result<(), String> {
    let mut conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::reset_personalization_history(&mut conn)
        .map_err(|e| format!("DB error resetting personalization history: {}", e))
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
            "review_issues": h.review_issues.as_deref().and_then(|value| {
                serde_json::from_str::<Vec<String>>(value).ok()
            }),
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
