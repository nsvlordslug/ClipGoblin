//! Local clip-source discovery and import.
//!
//! All paths enter through a native picker or a previously persisted folder.
//! Frontend-supplied candidate IDs are resolved against a fresh folder scan so
//! an invoke call cannot turn into arbitrary filesystem access.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::db::{self, ClipRow, HighlightRow};

pub(crate) const SOURCE_KINDS: &[&str] = &["medal", "obs", "meld"];
const MAX_SCAN_FILES: usize = 2_000;
const MAX_SCAN_DEPTH: usize = 8;
const FINGERPRINT_CHUNK: u64 = 256 * 1024;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalSourceConfig {
    pub kind: String,
    pub directory: Option<String>,
    pub auto_import: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalMediaCandidate {
    pub id: String,
    pub name: String,
    pub folder_label: String,
    pub path: String,
    pub size_bytes: u64,
    pub recorded_at: String,
    pub imported_clip_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedClip {
    pub clip_id: String,
    pub title: String,
    pub source_kind: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn validate_source_kind(kind: &str, include_manual: bool) -> Result<(), String> {
    if SOURCE_KINDS.contains(&kind) || (include_manual && kind == "manual") {
        Ok(())
    } else {
        Err(format!("Unsupported clip source '{kind}'"))
    }
}

fn dir_key(kind: &str) -> String {
    format!("source_{kind}_dir")
}

fn auto_key(kind: &str) -> String {
    format!("source_{kind}_auto_import")
}

fn watch_after_key(kind: &str) -> String {
    format!("source_{kind}_watch_after")
}

fn is_supported_media(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "mp4" | "mov" | "m4v" | "webm" | "mkv" | "flv"
            )
        })
        .unwrap_or(false)
}

fn source_folder_label(root: &Path, media_path: &Path) -> String {
    let nested_folder = media_path
        .parent()
        .and_then(|parent| parent.strip_prefix(root).ok())
        .and_then(|relative| relative.components().next())
        .and_then(|component| match component {
            Component::Normal(name) => name.to_str(),
            _ => None,
        });
    let raw = nested_folder
        .or_else(|| root.file_name().and_then(|name| name.to_str()))
        .unwrap_or("Other clips");
    let readable = raw
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if readable.is_empty() {
        "Other clips".to_string()
    } else {
        readable
    }
}

fn system_time_to_rfc3339(value: SystemTime) -> String {
    DateTime::<Utc>::from(value).to_rfc3339()
}

fn candidate_id(path: &Path, metadata: &fs::Metadata) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update(metadata.len().to_le_bytes());
    if let Ok(modified) = metadata.modified() {
        if let Ok(since_epoch) = modified.duration_since(SystemTime::UNIX_EPOCH) {
            hasher.update(since_epoch.as_nanos().to_le_bytes());
        }
    }
    format_digest(hasher.finalize().as_slice())
}

fn format_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn media_fingerprint(path: &Path) -> Result<String, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Could not inspect '{}': {error}", path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("The selected media file is empty or unavailable".to_string());
    }

    let mut file = File::open(path)
        .map_err(|error| format!("Could not read '{}': {error}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(metadata.len().to_le_bytes());

    let first_len = metadata.len().min(FINGERPRINT_CHUNK) as usize;
    let mut buffer = vec![0_u8; first_len];
    file.read_exact(&mut buffer)
        .map_err(|error| format!("Could not fingerprint '{}': {error}", path.display()))?;
    hasher.update(&buffer);

    if metadata.len() > FINGERPRINT_CHUNK {
        let last_len = metadata.len().min(FINGERPRINT_CHUNK) as usize;
        file.seek(SeekFrom::End(-(last_len as i64)))
            .map_err(|error| format!("Could not seek '{}': {error}", path.display()))?;
        buffer.resize(last_len, 0);
        file.read_exact(&mut buffer)
            .map_err(|error| format!("Could not finish fingerprinting '{}': {error}", path.display()))?;
        hasher.update(&buffer);
    }

    Ok(format_digest(hasher.finalize().as_slice()))
}

fn collect_media_files(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let root = directory
        .canonicalize()
        .map_err(|error| format!("Could not open '{}': {error}", directory.display()))?;
    if !root.is_dir() {
        return Err("The configured source folder no longer exists".to_string());
    }

    let mut found = Vec::new();
    let mut pending = vec![(root, 0_usize)];
    while let Some((folder, depth)) = pending.pop() {
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) => {
                log::warn!("[sources] Could not scan {}: {error}", folder.display());
                continue;
            }
        };
        for entry in entries.flatten() {
            if found.len() >= MAX_SCAN_FILES {
                break;
            }
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() && depth < MAX_SCAN_DEPTH {
                pending.push((path, depth + 1));
            } else if file_type.is_file() && is_supported_media(&path) {
                found.push(path);
            }
        }
        if found.len() >= MAX_SCAN_FILES {
            break;
        }
    }

    found.sort_by(|left, right| {
        let left_modified = left.metadata().and_then(|meta| meta.modified()).ok();
        let right_modified = right.metadata().and_then(|meta| meta.modified()).ok();
        right_modified.cmp(&left_modified)
    });
    Ok(found)
}

fn configured_directory(conn: &Connection, kind: &str) -> Result<PathBuf, String> {
    validate_source_kind(kind, false)?;
    let path = db::get_setting(conn, &dir_key(kind))
        .map_err(|error| format!("Database error: {error}"))?
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("Choose the {kind} clips folder first"))?;
    let canonical = PathBuf::from(path)
        .canonicalize()
        .map_err(|error| format!("The configured {kind} folder is unavailable: {error}"))?;
    if !canonical.is_absolute() || !canonical.is_dir() {
        return Err(format!("The configured {kind} folder is invalid"));
    }
    Ok(canonical)
}

pub(crate) fn scan_configured_source(
    conn: &Connection,
    kind: &str,
) -> Result<Vec<ExternalMediaCandidate>, String> {
    let directory = configured_directory(conn, kind)?;
    let (imported_by_path, missing_game_ids): (HashMap<String, String>, HashSet<String>) = {
        let mut stmt = conn
            .prepare(
                "SELECT source_media_path, id, game FROM clips
                 WHERE source_kind = ?1 AND source_media_path IS NOT NULL",
            )
            .map_err(|error| format!("Database error: {error}"))?;
        let rows = stmt
            .query_map([kind], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|error| format!("Database error: {error}"))?;
        let mut by_path = HashMap::new();
        let mut missing_games = HashSet::new();
        for row in rows.filter_map(Result::ok) {
            if row.2.as_deref().map(str::trim).unwrap_or_default().is_empty() {
                missing_games.insert(row.1.clone());
            }
            by_path.insert(row.0, row.1);
        }
        (by_path, missing_games)
    };

    let candidates = collect_media_files(&directory)?
        .into_iter()
        .filter_map(|path| {
            let canonical = path.canonicalize().ok()?;
            if !canonical.starts_with(&directory) {
                return None;
            }
            let metadata = canonical.metadata().ok()?;
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let path_string = canonical.to_string_lossy().to_string();
            Some(ExternalMediaCandidate {
                id: candidate_id(&canonical, &metadata),
                name: canonical
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Imported video")
                    .to_string(),
                folder_label: source_folder_label(&directory, &canonical),
                path: path_string.clone(),
                size_bytes: metadata.len(),
                recorded_at: system_time_to_rfc3339(modified),
                imported_clip_id: imported_by_path.get(&path_string).cloned(),
            })
        })
        .collect::<Vec<_>>();

    if kind == "medal" {
        for candidate in &candidates {
            let Some(clip_id) = candidate.imported_clip_id.as_deref() else {
                continue;
            };
            if !missing_game_ids.contains(clip_id) {
                continue;
            }
            if let Err(error) = conn.execute(
                "UPDATE clips SET game = ?1
                 WHERE id = ?2 AND (game IS NULL OR TRIM(game) = '')",
                rusqlite::params![candidate.folder_label, clip_id],
            ) {
                log::debug!("[sources] Could not backfill Medal game folder for {clip_id}: {error}");
            }
        }
    }

    Ok(candidates)
}

fn duplicate_clip(conn: &Connection, fingerprint: &str) -> Result<Option<ImportedClip>, String> {
    conn.query_row(
        "SELECT id, title, source_kind FROM clips WHERE source_fingerprint = ?1 LIMIT 1",
        [fingerprint],
        |row| {
            Ok(ImportedClip {
                clip_id: row.get(0)?,
                title: row.get(1)?,
                source_kind: row.get(2)?,
                status: "already_imported".to_string(),
                error: None,
            })
        },
    )
    .optional()
    .map_err(|error| format!("Database error: {error}"))
}

fn clean_title(path: &Path) -> String {
    let raw = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Imported clip");
    let title = raw.replace(['_', '-'], " ");
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        "Imported clip".to_string()
    } else {
        title
    }
}

pub(crate) fn import_media_path(
    app: &AppHandle,
    conn: &mut Connection,
    path: &Path,
    source_kind: &str,
) -> Result<ImportedClip, String> {
    validate_source_kind(source_kind, true)?;
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("Could not open '{}': {error}", path.display()))?;
    if !canonical.is_file() || !is_supported_media(&canonical) {
        return Err("Choose a supported video file (MP4, MOV, M4V, WebM, MKV, or FLV)".to_string());
    }

    let fingerprint = media_fingerprint(&canonical)?;
    if let Some(existing) = duplicate_clip(conn, &fingerprint)? {
        return Ok(existing);
    }
    let duration = crate::commands::export::probe_media_duration(&canonical)
        .filter(|duration| duration.is_finite() && *duration > 0.1)
        .ok_or_else(|| format!("Could not read the duration of '{}'", canonical.display()))?;

    let clip_id = uuid::Uuid::new_v4().to_string();
    let highlight_id = uuid::Uuid::new_v4().to_string();
    let vod_id = format!("external:{source_kind}");
    let title = clean_title(&canonical);
    let source_game = if source_kind == "medal" {
        configured_directory(conn, source_kind)
            .ok()
            .map(|root| source_folder_label(&root, &canonical))
    } else {
        None
    };
    let created_at = Utc::now().to_rfc3339();
    let recorded_at = canonical
        .metadata()
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .map(system_time_to_rfc3339)
        .unwrap_or_else(|| created_at.clone());

    let thumbnail_path = app
        .path()
        .app_data_dir()
        .ok()
        .map(|root| root.join("thumbnails").join(format!("{clip_id}.jpg")))
        .and_then(|thumbnail| {
            if let Some(parent) = thumbnail.parent() {
                if let Err(error) = fs::create_dir_all(parent) {
                    log::warn!("[sources] Could not create thumbnail directory: {error}");
                    return None;
                }
            }
            let ffmpeg = match crate::commands::vod::find_ffmpeg() {
                Ok(ffmpeg) => ffmpeg,
                Err(error) => {
                    log::warn!("[sources] Could not locate ffmpeg for thumbnail: {error}");
                    return None;
                }
            };
            let at = (duration * 0.15).clamp(0.0, 3.0);
            match crate::commands::vod::generate_thumbnail(
                &ffmpeg,
                &canonical.to_string_lossy(),
                at,
                &thumbnail,
            ) {
                Ok(()) => Some(thumbnail.to_string_lossy().to_string()),
                Err(error) => {
                    log::warn!("[sources] Thumbnail generation failed: {error}");
                    None
                }
            }
        });

    let tags = serde_json::to_string(&vec![source_kind, "imported"])
        .map_err(|error| format!("Could not encode source tags: {error}"))?;
    let highlight = HighlightRow {
        id: highlight_id.clone(),
        vod_id: vod_id.clone(),
        start_seconds: 0.0,
        end_seconds: duration,
        virality_score: 1.0,
        audio_score: 0.0,
        visual_score: 0.0,
        chat_score: 0.0,
        transcript_snippet: None,
        description: Some(title.clone()),
        tags: Some(tags),
        thumbnail_path: thumbnail_path.clone(),
        created_at: created_at.clone(),
        confidence_score: Some(0.99),
        explanation: Some(format!("Imported from {source_kind}; ready to edit")),
        event_summary: Some("Local clip import".to_string()),
        scoring_dimensions: None,
        signal_sources: Some(r#"["external"]"#.to_string()),
        review_rating: None,
        review_note: None,
        review_issues: None,
        community_clip_mp4_path: None,
    };
    let clip = ClipRow {
        id: clip_id.clone(),
        highlight_id: highlight_id.clone(),
        vod_id,
        title: title.clone(),
        start_seconds: 0.0,
        end_seconds: duration,
        aspect_ratio: "9:16".to_string(),
        crop_x: None,
        crop_y: None,
        crop_width: None,
        crop_height: None,
        captions_enabled: 0,
        captions_text: None,
        captions_position: "bottom".to_string(),
        caption_style: "clean".to_string(),
        caption_font_scale: 1.0,
        caption_y_offset: 0.0,
        captions_source_start: None,
        facecam_layout: "context_fit".to_string(),
        facecam_settings: None,
        context_background_path: None,
        context_background_mode: "blur".to_string(),
        context_blur_strength: 0.25,
        context_video_y: 0.5,
        render_status: "pending".to_string(),
        output_path: None,
        thumbnail_path,
        created_at,
        game: source_game,
        publish_description: None,
        publish_hashtags: None,
        cam_region_norm_override: None,
        cam_fit_mode: None,
        community_clip_mp4_path: None,
        source_kind: source_kind.to_string(),
        source_media_path: Some(canonical.to_string_lossy().to_string()),
        source_fingerprint: Some(fingerprint),
        source_recorded_at: Some(recorded_at),
    };

    let transaction = conn
        .transaction()
        .map_err(|error| format!("Database error: {error}"))?;
    db::insert_highlight(&transaction, &highlight)
        .map_err(|error| format!("Could not create imported highlight: {error}"))?;
    db::insert_clip(&transaction, &clip)
        .map_err(|error| format!("Could not create imported clip: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Could not finish importing clip: {error}"))?;

    Ok(ImportedClip {
        clip_id,
        title,
        source_kind: source_kind.to_string(),
        status: "imported".to_string(),
        error: None,
    })
}

pub(crate) fn import_candidate_ids(
    app: &AppHandle,
    conn: &mut Connection,
    kind: &str,
    candidate_ids: &[String],
) -> Result<Vec<ImportedClip>, String> {
    validate_source_kind(kind, false)?;
    if candidate_ids.len() > 200 {
        return Err("Import at most 200 clips at a time".to_string());
    }
    let selected: HashSet<&str> = candidate_ids.iter().map(String::as_str).collect();
    let candidates = scan_configured_source(conn, kind)?;
    let matching: Vec<_> = candidates
        .into_iter()
        .filter(|candidate| selected.contains(candidate.id.as_str()))
        .collect();
    if matching.len() != selected.len() {
        return Err("One or more selected clips changed or left the configured folder; scan again".to_string());
    }

    Ok(matching
        .iter()
        .map(|candidate| {
            import_media_path(app, conn, Path::new(&candidate.path), kind).unwrap_or_else(|error| {
                ImportedClip {
                    clip_id: String::new(),
                    title: candidate.name.clone(),
                    source_kind: kind.to_string(),
                    status: "failed".to_string(),
                    error: Some(error),
                }
            })
        })
        .collect())
}

fn auto_sync_once(app: &AppHandle, kind: &str) -> Result<Vec<ImportedClip>, String> {
    let path = db::db_path().map_err(|error| format!("Database path: {error}"))?;
    let mut conn = Connection::open(path).map_err(|error| format!("Database open: {error}"))?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|error| format!("Database timeout setup: {error}"))?;
    let enabled = db::get_setting(&conn, &auto_key(kind))
        .map_err(|error| format!("Database error: {error}"))?
        .as_deref()
        == Some("true");
    if !enabled {
        return Ok(Vec::new());
    }
    let watch_after = db::get_setting(&conn, &watch_after_key(kind))
        .map_err(|error| format!("Database error: {error}"))?
        .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);
    let candidates = scan_configured_source(&conn, kind)?;
    // A recorder may create the final filename before it finishes writing.
    // Waiting for a quiet modification window avoids importing a shortened
    // duration and fingerprinting an incomplete file.
    let stable_before = Utc::now() - chrono::Duration::seconds(15);
    let selected: Vec<_> = candidates
        .into_iter()
        .filter(|candidate| candidate.imported_clip_id.is_none())
        .filter(|candidate| {
            DateTime::parse_from_rfc3339(&candidate.recorded_at)
                .map(|value| {
                    let recorded_at = value.with_timezone(&Utc);
                    recorded_at >= watch_after && recorded_at <= stable_before
                })
                .unwrap_or(false)
        })
        .collect();

    let mut imported = Vec::new();
    for candidate in selected {
        match import_media_path(app, &mut conn, Path::new(&candidate.path), kind) {
            Ok(result) if result.status == "imported" => imported.push(result),
            Ok(_) => {}
            Err(error) => log::warn!("[sources] Auto-import failed for {}: {error}", candidate.path),
        }
    }
    Ok(imported)
}

pub(crate) fn start_source_monitor(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            for kind in SOURCE_KINDS {
                let handle = app.clone();
                let kind = (*kind).to_string();
                let monitored_kind = kind.clone();
                let result = tauri::async_runtime::spawn_blocking(move || {
                    auto_sync_once(&handle, &monitored_kind)
                })
                .await;
                match result {
                    Ok(Ok(imported)) if !imported.is_empty() => {
                        let _ = app.emit("external-clips-imported", imported);
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => log::debug!("[sources] {kind} monitor skipped: {error}"),
                    Err(error) => log::warn!("[sources] {kind} monitor task failed: {error}"),
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{clean_title, collect_media_files, is_supported_media, source_folder_label};
    use std::{fs, path::Path};

    #[test]
    fn source_extensions_are_case_insensitive() {
        assert!(is_supported_media(Path::new("clip.MP4")));
        assert!(is_supported_media(Path::new("replay.mkv")));
        assert!(!is_supported_media(Path::new("notes.txt")));
    }

    #[test]
    fn imported_titles_are_human_readable() {
        assert_eq!(clean_title(Path::new("clutch_play-2026.mp4")), "clutch play 2026");
    }

    #[test]
    fn parent_source_folder_includes_every_nested_game_folder() {
        let root = std::env::temp_dir().join(format!(
            "clipviral-external-source-{}",
            uuid::Uuid::new_v4()
        ));
        let first_game = root.join("Dead by Daylight");
        let second_game = root.join("Valorant").join("Clips");
        fs::create_dir_all(&first_game).expect("create first game folder");
        fs::create_dir_all(&second_game).expect("create second game folder");
        fs::write(first_game.join("escape.mp4"), b"first").expect("write first clip");
        fs::write(second_game.join("ace.MKV"), b"second").expect("write second clip");
        fs::write(root.join("metadata.json"), b"{}").expect("write ignored metadata");

        let found = collect_media_files(&root).expect("scan parent source folder");
        let mut names = found
            .iter()
            .filter_map(|path| path.file_name()?.to_str().map(str::to_string))
            .collect::<Vec<_>>();
        names.sort();

        assert_eq!(
            source_folder_label(&root, &first_game.join("escape.mp4")),
            "Dead by Daylight"
        );
        assert_eq!(
            source_folder_label(&root, &second_game.join("ace.MKV")),
            "Valorant"
        );

        fs::remove_dir_all(&root).expect("remove source test folder");
        assert_eq!(names, vec!["ace.MKV", "escape.mp4"]);
    }
}
