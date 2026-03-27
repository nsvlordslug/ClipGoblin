use rusqlite::{Connection, Result as SqliteResult, params};
use std::path::PathBuf;

/// Get the path to the database file.
pub fn db_path() -> PathBuf {
    let data_dir = dirs::data_dir()
        .expect("Failed to get data directory")
        .join("clipviral");
    std::fs::create_dir_all(&data_dir).expect("Failed to create data directory");
    data_dir.join("clipviral.db")
}

/// Initialize the database, creating tables if they don't exist.
pub fn init_db() -> SqliteResult<Connection> {
    let path = db_path();
    let conn = Connection::open(&path)?;

    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;

    // Auto-repair corrupted indexes on startup
    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        conn.execute_batch("REINDEX;")?;
    }

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT
        );

        CREATE TABLE IF NOT EXISTS twitch_channels (
            id TEXT PRIMARY KEY,
            twitch_user_id TEXT,
            twitch_login TEXT,
            display_name TEXT,
            profile_image_url TEXT,
            created_at TEXT
        );

        CREATE TABLE IF NOT EXISTS vods (
            id TEXT PRIMARY KEY,
            channel_id TEXT,
            twitch_video_id TEXT UNIQUE,
            title TEXT,
            duration_seconds INTEGER,
            stream_date TEXT,
            thumbnail_url TEXT,
            vod_url TEXT,
            download_status TEXT DEFAULT 'pending',
            local_path TEXT,
            file_size_bytes INTEGER,
            analysis_status TEXT DEFAULT 'pending',
            created_at TEXT
        );

        CREATE TABLE IF NOT EXISTS highlights (
            id TEXT PRIMARY KEY,
            vod_id TEXT,
            start_seconds REAL,
            end_seconds REAL,
            virality_score REAL,
            audio_score REAL,
            visual_score REAL,
            chat_score REAL,
            transcript_snippet TEXT,
            description TEXT,
            tags TEXT,
            thumbnail_path TEXT,
            created_at TEXT
        );

        CREATE TABLE IF NOT EXISTS clips (
            id TEXT PRIMARY KEY,
            highlight_id TEXT,
            vod_id TEXT,
            title TEXT,
            start_seconds REAL,
            end_seconds REAL,
            aspect_ratio TEXT DEFAULT '9:16',
            crop_x INTEGER,
            crop_y INTEGER,
            crop_width INTEGER,
            crop_height INTEGER,
            captions_enabled INTEGER DEFAULT 1,
            render_status TEXT DEFAULT 'pending',
            output_path TEXT,
            created_at TEXT
        );"
    )?;

    // Migrations: add columns that may not exist yet
    conn.execute("ALTER TABLE vods ADD COLUMN download_progress INTEGER DEFAULT 0", []).ok();
    conn.execute("ALTER TABLE clips ADD COLUMN captions_text TEXT", []).ok();
    conn.execute("ALTER TABLE clips ADD COLUMN captions_position TEXT DEFAULT 'bottom'", []).ok();
    conn.execute("ALTER TABLE clips ADD COLUMN facecam_layout TEXT DEFAULT 'none'", []).ok();
    conn.execute("ALTER TABLE clips ADD COLUMN thumbnail_path TEXT", []).ok();
    conn.execute("ALTER TABLE vods ADD COLUMN analysis_progress INTEGER DEFAULT 0", []).ok();

    // Session 4+: Transcript and performance tracking
    conn.execute("ALTER TABLE vods ADD COLUMN transcript_path TEXT", []).ok();
    conn.execute("ALTER TABLE clips ADD COLUMN auto_captions_path TEXT", []).ok();
    conn.execute("ALTER TABLE clips ADD COLUMN keyword_boost REAL DEFAULT 0.0", []).ok();

    // Grounded scoring: replace legacy virality_score display with calibrated confidence
    conn.execute("ALTER TABLE highlights ADD COLUMN confidence_score REAL", []).ok();
    conn.execute("ALTER TABLE highlights ADD COLUMN explanation TEXT", []).ok();

    // Event summary: one-sentence description of what happened
    conn.execute("ALTER TABLE highlights ADD COLUMN event_summary TEXT", []).ok();

    // Performance tracking for feedback loop
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS clip_performance (
            id TEXT PRIMARY KEY,
            clip_id TEXT NOT NULL,
            platform TEXT,
            views INTEGER DEFAULT 0,
            likes INTEGER DEFAULT 0,
            comments INTEGER DEFAULT 0,
            shares INTEGER DEFAULT 0,
            retention_rate REAL DEFAULT 0.0,
            first_3s_hold_rate REAL DEFAULT 0.0,
            completion_rate REAL DEFAULT 0.0,
            recorded_at TEXT,
            FOREIGN KEY (clip_id) REFERENCES clips(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS creator_profile (
            id TEXT PRIMARY KEY,
            content_style TEXT DEFAULT 'mixed',
            avg_hook_weight REAL DEFAULT 0.30,
            avg_emotional_weight REAL DEFAULT 0.25,
            avg_payoff_weight REAL DEFAULT 0.20,
            avg_loop_weight REAL DEFAULT 0.15,
            avg_context_weight REAL DEFAULT 0.10,
            total_clips_tracked INTEGER DEFAULT 0,
            top_performing_tags TEXT,
            updated_at TEXT
        );"
    ).ok();

    // Social publishing: upload history for duplicate detection
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS upload_history (
            id TEXT PRIMARY KEY,
            clip_id TEXT NOT NULL,
            platform TEXT NOT NULL,
            video_url TEXT,
            uploaded_at TEXT,
            UNIQUE(clip_id, platform)
        )"
    )?;

    Ok(conn)
}

// ── Settings helpers ──

pub fn save_setting(conn: &Connection, key: &str, value: &str) -> SqliteResult<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn get_setting(conn: &Connection, key: &str) -> SqliteResult<Option<String>> {
    let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
    let mut rows = stmt.query_map(params![key], |row| row.get::<_, String>(0))?;
    match rows.next() {
        Some(val) => Ok(Some(val?)),
        None => Ok(None),
    }
}

// ── Channel helpers ──

pub fn insert_channel(
    conn: &Connection,
    id: &str,
    twitch_user_id: &str,
    twitch_login: &str,
    display_name: &str,
    profile_image_url: &str,
) -> SqliteResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR REPLACE INTO twitch_channels (id, twitch_user_id, twitch_login, display_name, profile_image_url, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, twitch_user_id, twitch_login, display_name, profile_image_url, now],
    )?;
    Ok(())
}

pub fn get_all_channels(conn: &Connection) -> SqliteResult<Vec<ChannelRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, twitch_user_id, twitch_login, display_name, profile_image_url, created_at FROM twitch_channels ORDER BY created_at DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ChannelRow {
            id: row.get(0)?,
            twitch_user_id: row.get(1)?,
            twitch_login: row.get(2)?,
            display_name: row.get(3)?,
            profile_image_url: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;
    rows.collect()
}

pub fn delete_channel(conn: &Connection, id: &str) -> SqliteResult<()> {
    conn.execute("DELETE FROM twitch_channels WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn delete_all_channels(conn: &Connection) -> SqliteResult<()> {
    conn.execute("DELETE FROM twitch_channels", [])?;
    Ok(())
}

// ── VOD helpers ──

pub fn upsert_vod(conn: &Connection, vod: &VodRow) -> SqliteResult<()> {
    conn.execute(
        "INSERT INTO vods (id, channel_id, twitch_video_id, title, duration_seconds, stream_date, thumbnail_url, vod_url, download_status, local_path, file_size_bytes, analysis_status, created_at, download_progress, analysis_progress)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT(twitch_video_id) DO UPDATE SET
            title = excluded.title,
            duration_seconds = excluded.duration_seconds,
            thumbnail_url = excluded.thumbnail_url,
            vod_url = excluded.vod_url",
        params![
            vod.id, vod.channel_id, vod.twitch_video_id, vod.title,
            vod.duration_seconds, vod.stream_date, vod.thumbnail_url, vod.vod_url,
            vod.download_status, vod.local_path, vod.file_size_bytes, vod.analysis_status,
            vod.created_at, vod.download_progress, vod.analysis_progress,
        ],
    )?;
    Ok(())
}

pub fn get_vods_by_channel(conn: &Connection, channel_id: &str) -> SqliteResult<Vec<VodRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, channel_id, twitch_video_id, title, duration_seconds, stream_date, thumbnail_url, vod_url, download_status, local_path, file_size_bytes, analysis_status, created_at, download_progress, analysis_progress
         FROM vods WHERE channel_id = ?1 ORDER BY stream_date DESC"
    )?;
    let rows = stmt.query_map(params![channel_id], |row| {
        Ok(VodRow {
            id: row.get(0)?,
            channel_id: row.get(1)?,
            twitch_video_id: row.get(2)?,
            title: row.get(3)?,
            duration_seconds: row.get(4)?,
            stream_date: row.get(5)?,
            thumbnail_url: row.get(6)?,
            vod_url: row.get(7)?,
            download_status: row.get(8)?,
            local_path: row.get(9)?,
            file_size_bytes: row.get(10)?,
            analysis_status: row.get(11)?,
            created_at: row.get(12)?,
            download_progress: row.get(13)?,
            analysis_progress: row.get::<_, Option<i64>>(14)?.unwrap_or(0),
        })
    })?;
    rows.collect()
}

pub fn get_vod_by_id(conn: &Connection, id: &str) -> SqliteResult<Option<VodRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, channel_id, twitch_video_id, title, duration_seconds, stream_date, thumbnail_url, vod_url, download_status, local_path, file_size_bytes, analysis_status, created_at, download_progress, analysis_progress
         FROM vods WHERE id = ?1"
    )?;
    let mut rows = stmt.query_map(params![id], |row| {
        Ok(VodRow {
            id: row.get(0)?,
            channel_id: row.get(1)?,
            twitch_video_id: row.get(2)?,
            title: row.get(3)?,
            duration_seconds: row.get(4)?,
            stream_date: row.get(5)?,
            thumbnail_url: row.get(6)?,
            vod_url: row.get(7)?,
            download_status: row.get(8)?,
            local_path: row.get(9)?,
            file_size_bytes: row.get(10)?,
            analysis_status: row.get(11)?,
            created_at: row.get(12)?,
            download_progress: row.get(13)?,
            analysis_progress: row.get::<_, Option<i64>>(14)?.unwrap_or(0),
        })
    })?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

pub fn update_vod_download_status(
    conn: &Connection,
    id: &str,
    status: &str,
    local_path: Option<&str>,
    file_size: Option<i64>,
) -> SqliteResult<()> {
    conn.execute(
        "UPDATE vods SET download_status = ?1, local_path = ?2, file_size_bytes = ?3 WHERE id = ?4",
        params![status, local_path, file_size, id],
    )?;
    Ok(())
}

pub fn update_vod_download_progress(conn: &Connection, id: &str, progress: i64) -> SqliteResult<()> {
    conn.execute(
        "UPDATE vods SET download_progress = ?1 WHERE id = ?2",
        params![progress, id],
    )?;
    Ok(())
}

pub fn update_vod_analysis_status(conn: &Connection, id: &str, status: &str) -> SqliteResult<()> {
    conn.execute(
        "UPDATE vods SET analysis_status = ?1 WHERE id = ?2",
        params![status, id],
    )?;
    Ok(())
}

pub fn update_vod_analysis_progress(conn: &Connection, id: &str, progress: i64) -> SqliteResult<()> {
    conn.execute(
        "UPDATE vods SET analysis_progress = ?1 WHERE id = ?2",
        params![progress, id],
    )?;
    Ok(())
}

// ── Highlight helpers ──

pub fn get_highlights_by_vod(conn: &Connection, vod_id: &str) -> SqliteResult<Vec<HighlightRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, vod_id, start_seconds, end_seconds, virality_score, audio_score, visual_score, chat_score, transcript_snippet, description, tags, thumbnail_path, created_at, confidence_score, explanation, event_summary
         FROM highlights WHERE vod_id = ?1 ORDER BY COALESCE(confidence_score, virality_score * 0.75 + 0.05) DESC"
    )?;
    let rows = stmt.query_map(params![vod_id], |row| {
        Ok(HighlightRow {
            id: row.get(0)?,
            vod_id: row.get(1)?,
            start_seconds: row.get(2)?,
            end_seconds: row.get(3)?,
            virality_score: row.get(4)?,
            audio_score: row.get(5)?,
            visual_score: row.get(6)?,
            chat_score: row.get(7)?,
            transcript_snippet: row.get(8)?,
            description: row.get(9)?,
            tags: row.get(10)?,
            thumbnail_path: row.get(11)?,
            created_at: row.get(12)?,
            confidence_score: row.get(13)?,
            explanation: row.get(14)?,
            event_summary: row.get(15)?,
        })
    })?;
    rows.collect()
}

pub fn insert_highlight(conn: &Connection, h: &HighlightRow) -> SqliteResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO highlights (id, vod_id, start_seconds, end_seconds, virality_score, audio_score, visual_score, chat_score, transcript_snippet, description, tags, thumbnail_path, created_at, confidence_score, explanation, event_summary)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![h.id, h.vod_id, h.start_seconds, h.end_seconds, h.virality_score, h.audio_score, h.visual_score, h.chat_score, h.transcript_snippet, h.description, h.tags, h.thumbnail_path, h.created_at, h.confidence_score, h.explanation, h.event_summary],
    )?;
    Ok(())
}

pub fn delete_highlights_for_vod(conn: &Connection, vod_id: &str) -> SqliteResult<()> {
    conn.execute("DELETE FROM highlights WHERE vod_id = ?1", params![vod_id])?;
    Ok(())
}

// ── Clip helpers ──

pub fn insert_clip(conn: &Connection, c: &ClipRow) -> SqliteResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO clips (id, highlight_id, vod_id, title, start_seconds, end_seconds, aspect_ratio, crop_x, crop_y, crop_width, crop_height, captions_enabled, captions_text, captions_position, facecam_layout, render_status, output_path, thumbnail_path, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        params![c.id, c.highlight_id, c.vod_id, c.title, c.start_seconds, c.end_seconds, c.aspect_ratio, c.crop_x, c.crop_y, c.crop_width, c.crop_height, c.captions_enabled, c.captions_text, c.captions_position, c.facecam_layout, c.render_status, c.output_path, c.thumbnail_path, c.created_at],
    )?;
    Ok(())
}

pub fn delete_clips_for_vod(conn: &Connection, vod_id: &str) -> SqliteResult<()> {
    conn.execute("DELETE FROM clips WHERE vod_id = ?1", params![vod_id])?;
    Ok(())
}

pub fn delete_clip(conn: &Connection, clip_id: &str) -> SqliteResult<()> {
    // Delete the associated highlight too
    conn.execute(
        "DELETE FROM highlights WHERE id IN (SELECT highlight_id FROM clips WHERE id = ?1)",
        params![clip_id],
    )?;
    conn.execute("DELETE FROM clips WHERE id = ?1", params![clip_id])?;
    Ok(())
}

fn read_clip_row(row: &rusqlite::Row) -> rusqlite::Result<ClipRow> {
    Ok(ClipRow {
        id: row.get(0)?,
        highlight_id: row.get(1)?,
        vod_id: row.get(2)?,
        title: row.get(3)?,
        start_seconds: row.get(4)?,
        end_seconds: row.get(5)?,
        aspect_ratio: row.get(6)?,
        crop_x: row.get(7)?,
        crop_y: row.get(8)?,
        crop_width: row.get(9)?,
        crop_height: row.get(10)?,
        captions_enabled: row.get(11)?,
        captions_text: row.get(12)?,
        captions_position: row.get::<_, Option<String>>(13)?.unwrap_or_else(|| "bottom".to_string()),
        facecam_layout: row.get::<_, Option<String>>(14)?.unwrap_or_else(|| "none".to_string()),
        render_status: row.get(15)?,
        output_path: row.get(16)?,
        thumbnail_path: row.get(17)?,
        created_at: row.get(18)?,
    })
}

const CLIP_SELECT: &str = "SELECT id, highlight_id, vod_id, title, start_seconds, end_seconds, aspect_ratio, crop_x, crop_y, crop_width, crop_height, captions_enabled, captions_text, captions_position, facecam_layout, render_status, output_path, thumbnail_path, created_at FROM clips";

pub fn get_all_clips(conn: &Connection) -> SqliteResult<Vec<ClipRow>> {
    let mut stmt = conn.prepare(&format!("{} ORDER BY vod_id, start_seconds ASC", CLIP_SELECT))?;
    let rows = stmt.query_map([], |row| read_clip_row(row))?;
    rows.collect()
}

pub fn get_clip_by_id(conn: &Connection, clip_id: &str) -> SqliteResult<Option<ClipRow>> {
    let mut stmt = conn.prepare(&format!("{} WHERE id = ?1", CLIP_SELECT))?;
    let mut rows = stmt.query_map(params![clip_id], |row| read_clip_row(row))?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

pub fn update_clip_settings(
    conn: &Connection,
    clip_id: &str,
    title: &str,
    start_seconds: f64,
    end_seconds: f64,
    aspect_ratio: &str,
    captions_enabled: i32,
    captions_text: Option<&str>,
    captions_position: &str,
    facecam_layout: &str,
) -> SqliteResult<()> {
    conn.execute(
        "UPDATE clips SET title = ?1, start_seconds = ?2, end_seconds = ?3, aspect_ratio = ?4, captions_enabled = ?5, captions_text = ?6, captions_position = ?7, facecam_layout = ?8, render_status = 'pending' WHERE id = ?9",
        params![title, start_seconds, end_seconds, aspect_ratio, captions_enabled, captions_text, captions_position, facecam_layout, clip_id],
    )?;
    Ok(())
}

pub fn update_clip_render_status(
    conn: &Connection,
    clip_id: &str,
    status: &str,
    output_path: Option<&str>,
) -> SqliteResult<()> {
    conn.execute(
        "UPDATE clips SET render_status = ?1, output_path = ?2 WHERE id = ?3",
        params![status, output_path, clip_id],
    )?;
    Ok(())
}

pub fn update_clip_thumbnail(conn: &Connection, clip_id: &str, thumbnail_path: Option<&str>) -> SqliteResult<()> {
    conn.execute(
        "UPDATE clips SET thumbnail_path = ?1 WHERE id = ?2",
        params![thumbnail_path, clip_id],
    )?;
    Ok(())
}

// ── Transcript helpers ──

pub fn update_vod_transcript_path(conn: &Connection, id: &str, path: &str) -> SqliteResult<()> {
    conn.execute(
        "UPDATE vods SET transcript_path = ?1 WHERE id = ?2",
        params![path, id],
    )?;
    Ok(())
}

pub fn update_clip_auto_captions(conn: &Connection, clip_id: &str, path: &str) -> SqliteResult<()> {
    conn.execute(
        "UPDATE clips SET auto_captions_path = ?1 WHERE id = ?2",
        params![path, clip_id],
    )?;
    Ok(())
}

pub fn update_clip_keyword_boost(conn: &Connection, clip_id: &str, boost: f64) -> SqliteResult<()> {
    conn.execute(
        "UPDATE clips SET keyword_boost = ?1 WHERE id = ?2",
        params![boost, clip_id],
    )?;
    Ok(())
}

// ── Performance tracking helpers ──

pub fn insert_clip_performance(conn: &Connection, clip_id: &str, platform: &str,
    views: i64, likes: i64, comments: i64, shares: i64,
    retention: f64, hold_3s: f64, completion: f64) -> SqliteResult<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO clip_performance (id, clip_id, platform, views, likes, comments, shares, retention_rate, first_3s_hold_rate, completion_rate, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![id, clip_id, platform, views, likes, comments, shares, retention, hold_3s, completion, now],
    )?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ClipPerformanceRow {
    pub id: String,
    pub clip_id: String,
    pub platform: String,
    pub views: i64,
    pub likes: i64,
    pub comments: i64,
    pub shares: i64,
    pub retention_rate: f64,
    pub first_3s_hold_rate: f64,
    pub completion_rate: f64,
    pub recorded_at: String,
}

pub fn get_clip_performance(conn: &Connection, clip_id: &str) -> SqliteResult<Vec<ClipPerformanceRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, clip_id, platform, views, likes, comments, shares, retention_rate, first_3s_hold_rate, completion_rate, recorded_at
         FROM clip_performance WHERE clip_id = ?1 ORDER BY recorded_at DESC"
    )?;
    let rows = stmt.query_map(params![clip_id], |row| {
        Ok(ClipPerformanceRow {
            id: row.get(0)?,
            clip_id: row.get(1)?,
            platform: row.get(2)?,
            views: row.get(3)?,
            likes: row.get(4)?,
            comments: row.get(5)?,
            shares: row.get(6)?,
            retention_rate: row.get(7)?,
            first_3s_hold_rate: row.get(8)?,
            completion_rate: row.get(9)?,
            recorded_at: row.get(10)?,
        })
    })?;
    rows.collect()
}

/// Get average performance metrics across all clips (for scoring weight adjustment)
pub fn get_avg_performance_by_tags(conn: &Connection) -> SqliteResult<Vec<(String, f64)>> {
    let mut stmt = conn.prepare(
        "SELECT h.tags, AVG(p.retention_rate) as avg_retention
         FROM clip_performance p
         JOIN clips c ON c.id = p.clip_id
         JOIN highlights h ON h.id = c.highlight_id
         WHERE p.views > 0
         GROUP BY h.tags
         ORDER BY avg_retention DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
    })?;
    rows.collect()
}

// ── Creator profile helpers ──

pub fn get_or_create_creator_profile(conn: &Connection) -> SqliteResult<CreatorProfileRow> {
    let mut stmt = conn.prepare("SELECT id, content_style, avg_hook_weight, avg_emotional_weight, avg_payoff_weight, avg_loop_weight, avg_context_weight, total_clips_tracked, top_performing_tags, updated_at FROM creator_profile LIMIT 1")?;
    let mut rows = stmt.query_map([], |row| {
        Ok(CreatorProfileRow {
            id: row.get(0)?,
            content_style: row.get(1)?,
            avg_hook_weight: row.get(2)?,
            avg_emotional_weight: row.get(3)?,
            avg_payoff_weight: row.get(4)?,
            avg_loop_weight: row.get(5)?,
            avg_context_weight: row.get(6)?,
            total_clips_tracked: row.get(7)?,
            top_performing_tags: row.get(8)?,
            updated_at: row.get(9)?,
        })
    })?;
    match rows.next() {
        Some(row) => Ok(row?),
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO creator_profile (id, updated_at) VALUES (?1, ?2)",
                params![id, now],
            )?;
            Ok(CreatorProfileRow {
                id,
                content_style: "mixed".to_string(),
                avg_hook_weight: 0.30,
                avg_emotional_weight: 0.25,
                avg_payoff_weight: 0.20,
                avg_loop_weight: 0.15,
                avg_context_weight: 0.10,
                total_clips_tracked: 0,
                top_performing_tags: None,
                updated_at: now,
            })
        }
    }
}

pub fn update_creator_profile(conn: &Connection, profile: &CreatorProfileRow) -> SqliteResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE creator_profile SET content_style = ?1, avg_hook_weight = ?2, avg_emotional_weight = ?3, avg_payoff_weight = ?4, avg_loop_weight = ?5, avg_context_weight = ?6, total_clips_tracked = ?7, top_performing_tags = ?8, updated_at = ?9 WHERE id = ?10",
        params![profile.content_style, profile.avg_hook_weight, profile.avg_emotional_weight, profile.avg_payoff_weight, profile.avg_loop_weight, profile.avg_context_weight, profile.total_clips_tracked, profile.top_performing_tags, now, profile.id],
    )?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CreatorProfileRow {
    pub id: String,
    pub content_style: String,
    pub avg_hook_weight: f64,
    pub avg_emotional_weight: f64,
    pub avg_payoff_weight: f64,
    pub avg_loop_weight: f64,
    pub avg_context_weight: f64,
    pub total_clips_tracked: i64,
    pub top_performing_tags: Option<String>,
    pub updated_at: String,
}

pub fn get_all_highlights(conn: &Connection) -> SqliteResult<Vec<HighlightRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, vod_id, start_seconds, end_seconds, virality_score, audio_score, visual_score, chat_score, transcript_snippet, description, tags, thumbnail_path, created_at, confidence_score, explanation, event_summary
         FROM highlights ORDER BY vod_id, COALESCE(confidence_score, virality_score * 0.75 + 0.05) DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(HighlightRow {
            id: row.get(0)?,
            vod_id: row.get(1)?,
            start_seconds: row.get(2)?,
            end_seconds: row.get(3)?,
            virality_score: row.get(4)?,
            audio_score: row.get(5)?,
            visual_score: row.get(6)?,
            chat_score: row.get(7)?,
            transcript_snippet: row.get(8)?,
            description: row.get(9)?,
            tags: row.get(10)?,
            thumbnail_path: row.get(11)?,
            created_at: row.get(12)?,
            confidence_score: row.get(13)?,
            explanation: row.get(14)?,
            event_summary: row.get(15)?,
        })
    })?;
    rows.collect()
}

// ── Upload history helpers ──

pub fn get_upload_for_clip(conn: &Connection, clip_id: &str, platform: &str) -> SqliteResult<Option<UploadHistoryRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, clip_id, platform, video_url, uploaded_at FROM upload_history WHERE clip_id = ?1 AND platform = ?2"
    )?;
    let mut rows = stmt.query_map(params![clip_id, platform], |row| {
        Ok(UploadHistoryRow {
            id: row.get(0)?,
            clip_id: row.get(1)?,
            platform: row.get(2)?,
            video_url: row.get(3)?,
            uploaded_at: row.get(4)?,
        })
    })?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

pub fn upsert_upload(conn: &Connection, clip_id: &str, platform: &str, video_url: &str) -> SqliteResult<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO upload_history (id, clip_id, platform, video_url, uploaded_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(clip_id, platform) DO UPDATE SET video_url = excluded.video_url, uploaded_at = excluded.uploaded_at",
        params![id, clip_id, platform, video_url, now],
    )?;
    Ok(())
}

pub fn delete_settings_for_platform(conn: &Connection, platform: &str) -> SqliteResult<()> {
    let prefixes = [
        format!("{}_access_token", platform),
        format!("{}_refresh_token", platform),
        format!("{}_token_expiry", platform),
        format!("{}_channel_name", platform),
        format!("{}_channel_id", platform),
    ];
    for key in &prefixes {
        conn.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
    }
    Ok(())
}

// ── Row structs ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChannelRow {
    pub id: String,
    pub twitch_user_id: String,
    pub twitch_login: String,
    pub display_name: String,
    pub profile_image_url: String,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VodRow {
    pub id: String,
    pub channel_id: String,
    pub twitch_video_id: String,
    pub title: String,
    pub duration_seconds: i64,
    pub stream_date: String,
    pub thumbnail_url: String,
    pub vod_url: String,
    pub download_status: String,
    pub local_path: Option<String>,
    pub file_size_bytes: Option<i64>,
    pub analysis_status: String,
    pub created_at: String,
    pub download_progress: i64,
    pub analysis_progress: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HighlightRow {
    pub id: String,
    pub vod_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub virality_score: f64,
    pub audio_score: f64,
    pub visual_score: f64,
    pub chat_score: f64,
    pub transcript_snippet: Option<String>,
    pub description: Option<String>,
    pub tags: Option<String>,
    pub thumbnail_path: Option<String>,
    pub created_at: String,
    /// Calibrated user-facing confidence (0.0–0.98).  NULL for pre-migration rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_score: Option<f64>,
    /// Factual explanation of why this highlight was selected.  NULL for pre-migration rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
    /// One-sentence event summary (what happened).  NULL for pre-migration rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_summary: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ClipRow {
    pub id: String,
    pub highlight_id: String,
    pub vod_id: String,
    pub title: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub aspect_ratio: String,
    pub crop_x: Option<i32>,
    pub crop_y: Option<i32>,
    pub crop_width: Option<i32>,
    pub crop_height: Option<i32>,
    pub captions_enabled: i32,
    pub captions_text: Option<String>,
    pub captions_position: String,
    pub facecam_layout: String,
    pub render_status: String,
    pub output_path: Option<String>,
    pub thumbnail_path: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UploadHistoryRow {
    pub id: String,
    pub clip_id: String,
    pub platform: String,
    pub video_url: Option<String>,
    pub uploaded_at: String,
}
