mod ai_provider;
mod audio_signal;
mod clip_fusion;
mod clip_labeler;
mod clip_output;
mod clip_ranker;
mod db;
mod engine;
mod integration_test;
mod clip_selector;
mod error;
mod hardware;
mod job_queue;
mod pipeline;
mod post_captions;
mod scene_signal;
mod transcript_signal;
mod twitch;
mod social;
mod vertical_crop;

use std::sync::Mutex;
use std::io::{BufRead, BufReader};
use std::process::Stdio;
use rusqlite::Connection;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_dialog::DialogExt;

type DbConn = Mutex<Connection>;

/// Re-export so commands can reference the type via `State<'_, HardwareInfo>`.
use hardware::HardwareInfo;
use error::AppError;
use job_queue::{Job, JobQueue};

/// Emit a structured `"job-error"` event to the frontend AND convert to String.
/// Use at Tauri command boundaries for errors that should notify the UI.
fn report_error(app: &AppHandle, err: AppError) -> String {
    use tauri::Emitter;
    log::error!("[{}] {}", err.category(), err.detail());
    let _ = app.emit("job-error", err.to_event());
    err.to_string()
}

#[derive(serde::Serialize)]
struct AppInfo {
    version: String,
    data_dir: String,
    db_path: String,
}


// ── Clip title generation ──
// Mirrors the TypeScript module at src/lib/clipNaming.ts.
// Generates context-aware titles from analysis signals.

/// Event vocabulary — maps tag substrings to readable action labels.
/// These describe WHAT HAPPENED.
const EVENTS: &[(&str, &str)] = &[
    ("kill", "Kill"), ("death", "Death"), ("clutch", "Clutch Play"), ("save", "Save"),
    ("escape", "Escape"), ("chase", "Chase"), ("fight", "Fight"), ("ambush", "Ambush"),
    ("snipe", "Snipe"), ("headshot", "Headshot"), ("combo", "Combo"), ("dodge", "Dodge"),
    ("block", "Block"), ("counter", "Counter"), ("gank", "Gank"), ("wipe", "Team Wipe"),
    ("ace", "Ace"), ("steal", "Steal"), ("grab", "Grab"), ("explosion", "Explosion"),
    ("jumpscare", "Jumpscare"), ("scare", "Scare"),
    ("generator", "Generator"), ("repair", "Repair"),
    ("hook", "Hook"), ("interrupt", "Interrupt"), ("down", "Down"),
    ("rescue", "Rescue"), ("loop", "Loop"), ("mindgame", "Mind Game"),
    ("juke", "Juke"), ("bait", "Bait"), ("outplay", "Outplay"),
    ("miss", "Missed Hit"), ("whiff", "Whiff"),
    ("encounter", "Encounter"), ("skirmish", "Skirmish"),
    ("scream", "Scream"),
];

fn parse_tags(tags: Option<&str>) -> Vec<String> {
    tags.map(|t| t.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default()
}

fn classify(tags: &[String], vocab: &[(&str, &str)]) -> Vec<String> {
    let mut found = Vec::new();
    for tag in tags {
        for &(key, label) in vocab {
            if tag.contains(key) && !found.contains(&label.to_string()) {
                found.push(label.to_string());
                break;
            }
        }
    }
    found
}

// ═══════════════════════════════════════════════════════════════════
//  Grounded title / confidence / explanation for the save path.
//  These replace the hype-title generator for newly saved highlights.
// ═══════════════════════════════════════════════════════════════════

/// 2-stage title: ground truth → hook formatting.
///
/// Same logic as clip_labeler but adapted for the save path which
/// receives raw tag strings instead of structured CandidateClip data.
fn grounded_highlight_title(
    transcript_snippet: Option<&str>,
    tags: Option<&str>,
    start_seconds: f64,
) -> String {
    let phrase = extract_title_phrase(transcript_snippet.unwrap_or(""));
    let event = primary_event_from_tags(tags);
    let idx = start_seconds as usize;

    // 1. Reaction + Context (transcript + event)
    if let (Some(ref p), Some(ev)) = (&phrase, event) {
        if !p.to_lowercase().contains(&ev.to_lowercase()) {
            let ctx = save_context_tag(ev);
            let words: Vec<&str> = p.split_whitespace().take(5).collect();
            let q = words.join(" ");
            let formatted = if p.split_whitespace().count() > 5 {
                format!("{}...", q)
            } else {
                save_punctuate(&q)
            };
            return format!("\"{}\" {}", formatted, ctx);
        }
    }

    // 2. Transcript alone (if specific enough)
    if let Some(ref p) = phrase {
        if !is_vague_phrase(p) {
            let words: Vec<&str> = p.split_whitespace().collect();
            if words.len() >= 7 {
                let short: Vec<&str> = words[..6].to_vec();
                return format!("\"{}...\"", short.join(" "));
            }
            return format!("\"{}\"", p);
        }
    }

    // 3. Outcome-based (compound tags)
    if let Some(tag_str) = tags {
        let tag_list = parse_tags(Some(tag_str));
        if let Some(title) = save_outcome_title(&tag_list) {
            return title;
        }
    }

    // 4. Event + Tension (verb-forward, timing-aware)
    if let Some(ev) = event {
        let phrases: &[&str] = match ev {
            "jumpscare" | "ambush" => &["Ambush comes out of nowhere", "Caught off guard instantly", "Jumpscare hits with no warning"],
            "fight" => &["Fight breaks out instantly", "Fight goes wrong fast", "Fight starts and it gets bad"],
            "explosion" => &["Explosion hits out of nowhere", "Blows up with no warning"],
            "panic" => &["Panic hits instantly", "Everything goes wrong at once", "Panic sets in right away"],
            "celebration" => &["Clutches it at the last second", "Barely survives then celebrates"],
            "frustration" => &["Nothing goes right", "Loses it after that play"],
            "shock" | "disbelief" => &["Didn't see that coming", "Shock hits out of nowhere"],
            "hype" => &["Hype hits out of nowhere", "Goes off at the perfect time"],
            "reaction" => &["Reaction says it all", "Reacts instantly"],
            _ => &["Happens out of nowhere"],
        };
        return phrases[idx % phrases.len()].to_string();
    }

    // 5. Tag summary fallback
    if let Some(tag_str) = tags {
        let tag_list = parse_tags(Some(tag_str));
        let events = classify(&tag_list, EVENTS);
        if !events.is_empty() {
            return events[..events.len().min(2)].join(" + ");
        }
    }

    // 6. Timestamp
    let mins = (start_seconds as u32) / 60;
    let secs = (start_seconds as u32) % 60;
    format!("Highlight at {}:{:02}", mins, secs)
}

fn save_punctuate(s: &str) -> String {
    let t = s.trim_end();
    if t.ends_with('.') || t.ends_with('!') || t.ends_with('?') { t.to_string() }
    else { format!("{}.", t) }
}

fn save_context_tag(event: &str) -> &'static str {
    match event {
        "jumpscare" | "ambush" => "caught off guard",
        "fight" => "mid-fight",
        "explosion" => "right before it blows up",
        "celebration" => "clutches it",
        "panic" => "instant panic",
        "frustration" => "loses it",
        "disbelief" => "didn't see that coming",
        "shock" => "instant reaction",
        "hype" => "peak hype",
        "reaction" => "the reaction",
        _ => "out of nowhere",
    }
}

fn save_outcome_title(tag_list: &[String]) -> Option<String> {
    let has = |t: &str| tag_list.iter().any(|x| x.contains(t));
    if has("fight") && has("celebration") { return Some("Fight breaks out and they clutch it".into()); }
    if has("fight") && has("frustration") { return Some("Fight goes wrong and they lose it".into()); }
    if has("fight") && has("panic") { return Some("Fight turns bad fast".into()); }
    if (has("ambush") || has("jumpscare")) && has("panic") { return Some("Ambush hits and panic sets in".into()); }
    if (has("ambush") || has("jumpscare")) && has("shock") { return Some("Ambush out of nowhere".into()); }
    if has("panic") && has("celebration") { return Some("Almost dies then clutches it".into()); }
    if has("hype") && has("celebration") { return Some("Clutch play at the last second".into()); }
    None
}

fn extract_title_phrase(excerpt: &str) -> Option<String> {
    let trimmed = excerpt.trim();
    if trimmed.len() < 3 { return None; }
    let filler = ["like", "so", "um", "uh", "okay", "ok", "well", "and", "but"];
    let words: Vec<&str> = trimmed.split_whitespace()
        .skip_while(|w| filler.iter().any(|f| w.to_lowercase() == *f))
        .take(8)
        .collect();
    if words.len() < 2 { return None; }
    Some(words.join(" "))
}

fn is_vague_phrase(s: &str) -> bool {
    let wc = s.split_whitespace().count();
    if wc < 4 { return true; }
    let lower = s.to_lowercase();
    let vague = ["oh my god", "oh my gosh", "what the hell", "what the fuck",
                  "no way dude", "are you serious", "holy shit"];
    wc <= 4 && vague.iter().any(|v| lower.contains(v))
}

fn primary_event_from_tags(tags: Option<&str>) -> Option<&'static str> {
    let tag_str = tags?;
    let tag_list = parse_tags(Some(tag_str));
    let lower: Vec<String> = tag_list.iter().map(|t| t.to_lowercase()).collect();
    if lower.iter().any(|t| t.contains("jumpscare") || t.contains("ambush")) { return Some("jumpscare"); }
    if lower.iter().any(|t| t.contains("fight"))     { return Some("fight"); }
    if lower.iter().any(|t| t.contains("explosion")) { return Some("explosion"); }
    if lower.iter().any(|t| t.contains("celebration")){ return Some("celebration"); }
    if lower.iter().any(|t| t.contains("panic"))     { return Some("panic"); }
    if lower.iter().any(|t| t.contains("frustration")){ return Some("frustration"); }
    if lower.iter().any(|t| t.contains("disbelief")) { return Some("disbelief"); }
    if lower.iter().any(|t| t.contains("shock"))     { return Some("shock"); }
    if lower.iter().any(|t| t.contains("hype"))      { return Some("hype"); }
    if lower.iter().any(|t| t.contains("reaction"))  { return Some("reaction"); }
    if lower.iter().any(|t| t.contains("rapid"))     { return Some("rapid cuts"); }
    None
}

/// Compress a raw analysis score to a calibrated confidence value.
///
/// Raw scores from different analysis modes (AI, signals, chat)
/// cluster 0.50–0.99 due to bonus stacking.  This de-inflates first,
/// then applies a piecewise curve matching the pipeline calibration.
///
/// Target distribution:
///   most clips: 55–80%   strong: 80–90%   exceptional: 90–95%
fn compute_confidence(raw_score: f64, signal_count: usize) -> f64 {
    // Step 1: de-inflate — strip bonus stacking headroom
    let normalized = (raw_score * 0.85 - 0.10).clamp(0.0, 0.99);

    // Step 2: piecewise curve (same shape as clip_ranker::rescale_confidence)
    const ANCHORS: [(f64, f64); 8] = [
        (0.00, 0.00),
        (0.25, 0.25),
        (0.40, 0.55),
        (0.50, 0.65),
        (0.60, 0.77),
        (0.70, 0.84),
        (0.80, 0.89),
        (0.90, 0.93),
    ];

    let base = if normalized >= 0.90 {
        (0.93 + (normalized - 0.90) * 0.20).min(0.95)
    } else {
        let mut out = 0.0;
        for i in 1..ANCHORS.len() {
            if normalized <= ANCHORS[i].0 {
                let (x0, y0) = ANCHORS[i - 1];
                let (x1, y1) = ANCHORS[i];
                let t = (normalized - x0) / (x1 - x0);
                out = y0 + t * (y1 - y0);
                break;
            }
        }
        out
    };

    // Minimal signal nudge
    let nudge = if signal_count >= 4 { 0.01 } else { 0.0 };
    (base + nudge).min(0.96)
}

/// Count how many of the score channels are meaningfully active.
fn count_active_signals(audio: f64, visual: f64, chat: f64, has_transcript: bool) -> usize {
    let mut n = 0;
    if audio > 0.1 { n += 1; }
    if visual > 0.1 { n += 1; }
    if chat > 0.1 { n += 1; }
    if has_transcript { n += 1; }
    n
}

/// Build a factual explanation: signal values + count.
fn build_highlight_explanation(audio: f64, visual: f64, chat: f64, has_transcript: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    if audio > 0.0 { parts.push(format!("audio {:.0}%", audio * 100.0)); }
    if visual > 0.0 { parts.push(format!("visual {:.0}%", visual * 100.0)); }
    if chat > 0.0 { parts.push(format!("chat {:.0}%", chat * 100.0)); }
    if has_transcript { parts.push("transcript match".into()); }

    let count = parts.len();
    if parts.is_empty() {
        "No signal data".into()
    } else {
        format!("{} signal{} — {}", count, if count != 1 { "s" } else { "" }, parts.join(", "))
    }
}

/// Per-second audio intensity data extracted from a VOD via ffmpeg.
#[derive(Debug, Clone)]
struct AudioProfile {
    /// RMS volume level per second (0.0 = silence, 1.0 = max)
    rms_per_second: Vec<f64>,
    /// Indices of detected volume spikes (>1.5x average)
    spike_seconds: Vec<usize>,
}

// ── Tauri commands ──

/// Start the Twitch OAuth login flow: opens browser, waits for callback,
/// exchanges code for token, fetches the authenticated user, and saves
/// their channel as the only channel.
#[tauri::command]
async fn twitch_login(app: AppHandle, db: State<'_, DbConn>) -> Result<db::ChannelRow, String> {
    log::info!("[twitch_login] === Starting Twitch login flow ===");

    // 1. Bind callback server BEFORE opening the browser (avoids race condition)
    let listener = twitch::bind_callback_server()?;
    log::info!("[twitch_login] Step 1: Callback server bound on port 17385");

    // 2. Open the auth URL in the user's browser (uses embedded client_id + PKCE)
    let auth_url = twitch::get_auth_url();
    log::info!("[twitch_login] Step 2: Opening browser for OAuth");
    app.opener().open_url(&auth_url, None::<&str>)
        .map_err(|e| format!("Failed to open browser: {}", e))?;

    // 3. Wait for the OAuth callback on the already-listening server
    log::info!("[twitch_login] Step 3: Waiting for OAuth callback...");
    let code = tokio::task::spawn_blocking(move || twitch::wait_for_auth_code(listener))
        .await
        .map_err(|e| format!("Task error: {}", e))??;
    log::info!("[twitch_login] Step 3: Auth code received (len={})", code.len());

    // Exchange the code for an access token (PKCE — no client_secret needed)
    log::info!("[twitch_login] Step 4: Exchanging code for token...");
    let token_resp = twitch::exchange_code(&code).await?;
    log::info!("[twitch_login] Step 4: Token exchange succeeded");

    // Fetch the authenticated user's identity
    log::info!("[twitch_login] Step 5: Fetching user info...");
    let user = twitch::get_authenticated_user(&token_resp.access_token).await?;
    log::info!("[twitch_login] Step 5: Got user: {} ({})", user.display_name, user.login);

    // Save the user token for future API calls
    log::info!("[twitch_login] Step 6: Saving tokens and user info to DB...");
    {
        let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
        db::save_setting(&conn, "twitch_user_access_token", &token_resp.access_token)
            .map_err(|e| format!("DB error: {}", e))?;
        if let Some(ref rt) = token_resp.refresh_token {
            db::save_setting(&conn, "twitch_refresh_token", rt)
                .map_err(|e| format!("DB error: {}", e))?;
        }
        db::save_setting(&conn, "twitch_user_id", &user.id)
            .map_err(|e| format!("DB error: {}", e))?;
        db::save_setting(&conn, "twitch_login", &user.login)
            .map_err(|e| format!("DB error: {}", e))?;
    }
    log::info!("[twitch_login] Step 6: Tokens saved to DB");

    // Clear any existing channels and add only this user's channel
    let channel_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let channel = db::ChannelRow {
        id: channel_id.clone(),
        twitch_user_id: user.id.clone(),
        twitch_login: user.login.clone(),
        display_name: user.display_name.clone(),
        profile_image_url: user.profile_image_url.clone(),
        created_at: now,
    };

    log::info!("[twitch_login] Step 7: Saving channel to DB...");
    {
        let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
        // Remove all existing channels — user can only have their own
        db::delete_all_channels(&conn).map_err(|e| format!("DB error: {}", e))?;
        db::insert_channel(
            &conn,
            &channel.id,
            &channel.twitch_user_id,
            &channel.twitch_login,
            &channel.display_name,
            &channel.profile_image_url,
        )
        .map_err(|e| format!("DB error: {}", e))?;
    }

    log::info!("[twitch_login] === Login complete: {} ({}) — returning ChannelRow to frontend ===",
        channel.display_name, channel.twitch_login);

    Ok(channel)
}

/// Check if the user is currently logged in (has a saved channel).
#[tauri::command]
fn get_logged_in_user(db: State<'_, DbConn>) -> Result<Option<db::ChannelRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    let channels = db::get_all_channels(&conn).map_err(|e| format!("DB error: {}", e))?;
    Ok(channels.into_iter().next())
}

/// Log out — clear saved tokens and channel.
#[tauri::command]
fn twitch_logout(db: State<'_, DbConn>) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::delete_all_channels(&conn).map_err(|e| format!("DB error: {}", e))?;
    db::save_setting(&conn, "twitch_user_access_token", "").map_err(|e| format!("DB error: {}", e))?;
    db::save_setting(&conn, "twitch_refresh_token", "").map_err(|e| format!("DB error: {}", e))?;
    db::save_setting(&conn, "twitch_user_id", "").map_err(|e| format!("DB error: {}", e))?;
    db::save_setting(&conn, "twitch_login", "").map_err(|e| format!("DB error: {}", e))?;
    Ok(())
}

// ── Tool finders ──

/// Find yt-dlp executable by checking common install locations and PATH.
fn find_ytdlp() -> Result<std::path::PathBuf, AppError> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();

    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        for ver in &["Python312", "Python313", "Python311", "Python310"] {
            candidates.push(
                std::path::PathBuf::from(&local)
                    .join("Programs")
                    .join("Python")
                    .join(ver)
                    .join("Scripts")
                    .join("yt-dlp.exe"),
            );
        }
    }

    if let Ok(appdata) = std::env::var("APPDATA") {
        for ver in &["Python312", "Python313", "Python311", "Python310"] {
            candidates.push(
                std::path::PathBuf::from(&appdata)
                    .join("Python")
                    .join(ver)
                    .join("Scripts")
                    .join("yt-dlp.exe"),
            );
        }
    }

    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        candidates.push(std::path::PathBuf::from(&userprofile).join(".local").join("bin").join("yt-dlp.exe"));
    }

    for path in &candidates {
        if path.exists() {
            return Ok(path.clone());
        }
    }

    // Last resort: check PATH
    if let Ok(output) = std::process::Command::new("yt-dlp").arg("--version").output() {
        if output.status.success() {
            return Ok(std::path::PathBuf::from("yt-dlp"));
        }
    }

    Err(AppError::Download(format!(
        "yt-dlp not found. Install it with: pip install yt-dlp\nSearched: {}",
        candidates.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>().join(", ")
    )))
}

/// Find ffmpeg executable by checking common install locations and PATH.
fn find_ffmpeg() -> Result<std::path::PathBuf, AppError> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();

    // winget installs to a tools directory
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        candidates.push(std::path::PathBuf::from(&local).join("Microsoft").join("WinGet").join("Links").join("ffmpeg.exe"));
    }

    // Common install locations
    candidates.push(std::path::PathBuf::from("C:\\ffmpeg\\bin\\ffmpeg.exe"));
    candidates.push(std::path::PathBuf::from("C:\\Program Files\\ffmpeg\\bin\\ffmpeg.exe"));

    // App data directory (bundled)
    if let Some(data) = dirs::data_dir() {
        candidates.push(data.join("clipviral").join("ffmpeg").join("ffmpeg.exe"));
    }

    for path in &candidates {
        if path.exists() {
            return Ok(path.clone());
        }
    }

    // Check PATH
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    if let Ok(status) = cmd.status() {
        if status.success() {
            return Ok(std::path::PathBuf::from("ffmpeg"));
        }
    }

    Err(AppError::Ffmpeg("Not found. Please install ffmpeg (winget install Gyan.FFmpeg).".into()))
}

// ── Download helpers ──

/// Parse yt-dlp progress output to extract download percentage.
fn parse_ytdlp_progress(line: &str) -> Option<u8> {
    if !line.contains("[download]") {
        return None;
    }
    let pct_pos = line.find('%')?;
    let before = &line[..pct_pos];
    let trimmed = before.trim_end();
    let num_start = trimmed.rfind(|c: char| !c.is_ascii_digit() && c != '.')? + 1;
    let num_str = &trimmed[num_start..];
    let val: f64 = num_str.parse().ok()?;
    Some(val.min(100.0).max(0.0) as u8)
}

/// Download a VOD using yt-dlp with real-time progress tracking.
#[tauri::command]
async fn download_vod(vod_id: String, app: AppHandle, db: State<'_, DbConn>) -> Result<(), String> {
    let ytdlp = find_ytdlp().map_err(|e| report_error(&app, e))?;

    // Atomic check-and-set: read status and update in a single lock scope
    let vod = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let vod = db::get_vod_by_id(&conn, &vod_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or_else(|| "VOD not found".to_string())?;

        if vod.download_status == "downloading" {
            return Err("This VOD is already downloading.".to_string());
        }

        db::update_vod_download_status(&conn, &vod_id, "downloading", None, None)
            .map_err(|e| format!("DB error: {}", e))?;
        db::update_vod_download_progress(&conn, &vod_id, 0)
            .map_err(|e| format!("DB error: {}", e))?;
        vod
    };

    // Get download directory from settings or use default
    let download_dir = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        match db::get_setting(&conn, "download_dir") {
            Ok(Some(dir)) if !dir.is_empty() => std::path::PathBuf::from(dir),
            _ => dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("clipviral")
                .join("downloads"),
        }
    };
    std::fs::create_dir_all(&download_dir).ok();

    let output_template = download_dir
        .join(format!("{}.%(ext)s", vod.twitch_video_id))
        .to_string_lossy()
        .to_string();

    let vod_url = vod.vod_url.clone();
    let twitch_video_id = vod.twitch_video_id.clone();
    let dl_dir = download_dir.clone();
    let vod_id_bg = vod_id.clone();
    let app_handle = app.clone();

    // Spawn background task — returns immediately so UI stays responsive
    tokio::task::spawn(async move {
        let vod_id_progress = vod_id_bg.clone();
        let vod_id_status = vod_id_bg;

        let result = tokio::task::spawn_blocking(move || {
            let progress_conn = db::db_path().ok().and_then(|p| rusqlite::Connection::open(p).ok());

            let mut cmd = std::process::Command::new(&ytdlp);
            cmd.arg("--force-overwrites")
                .arg("--newline")
                .arg("--no-color")
                .arg("--remux-video").arg("mp4")
                .arg("-o")
                .arg(&output_template)
                .arg(&vod_url)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            // Tell yt-dlp where ffmpeg is so it can remux MPEG-TS to proper MP4
            if let Ok(ffmpeg) = find_ffmpeg() {
                if let Some(ffmpeg_dir) = ffmpeg.parent() {
                    cmd.arg("--ffmpeg-location").arg(ffmpeg_dir);
                }
            }

            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }

            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => return Err(format!("Failed to start yt-dlp: {}", e)),
            };

            let stderr = child.stderr.take();
            let stderr_thread = std::thread::spawn(move || {
                if let Some(err) = stderr {
                    let reader = BufReader::new(err);
                    for _ in reader.lines() {}
                }
            });

            if let Some(stdout) = child.stdout.take() {
                let reader = BufReader::new(stdout);
                let mut last_reported: u8 = 0;
                for line in reader.lines().flatten() {
                    if let Some(pct) = parse_ytdlp_progress(&line) {
                        if pct != last_reported && (pct >= last_reported.saturating_add(2) || pct == 100) {
                            last_reported = pct;
                            if let Some(ref conn) = progress_conn {
                                db::update_vod_download_progress(conn, &vod_id_progress, pct as i64).ok();
                            }
                        }
                    }
                }
            }

            let _ = stderr_thread.join();
            let status = child.wait().map_err(|e| format!("yt-dlp error: {}", e))?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("yt-dlp exited with code: {:?}", status.code()))
            }
        })
        .await;

        let db: State<'_, DbConn> = app_handle.state();

        match result {
            Ok(Ok(())) => {
                let mut found_path: Option<std::path::PathBuf> = None;
                if let Ok(entries) = std::fs::read_dir(&dl_dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.starts_with(&twitch_video_id)
                            && !name.ends_with(".part")
                            && !name.ends_with(".ytdl")
                        {
                            found_path = Some(entry.path());
                            break;
                        }
                    }
                }
                let (path_str, file_size) = match &found_path {
                    Some(p) => (
                        Some(p.to_string_lossy().to_string()),
                        std::fs::metadata(p).ok().map(|m| m.len() as i64),
                    ),
                    None => (None, None),
                };
                if let Ok(conn) = db.lock() {
                    db::update_vod_download_status(
                        &conn,
                        &vod_id_status,
                        "downloaded",
                        path_str.as_deref(),
                        file_size,
                    )
                    .ok();
                    db::update_vod_download_progress(&conn, &vod_id_status, 100).ok();
                }
            }
            _ => {
                if let Ok(conn) = db.lock() {
                    db::update_vod_download_status(&conn, &vod_id_status, "failed", None, None).ok();
                }
            }
        }
    });

    Ok(())
}

/// Get cached VODs from DB only (no Twitch API call). Used for polling status.
#[tauri::command]
fn get_cached_vods(channel_id: String, db: State<'_, DbConn>) -> Result<Vec<db::VodRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_vods_by_channel(&conn, &channel_id).map_err(|e| format!("DB error: {}", e))
}

// ── AI Analysis ──

/// Extract per-second audio intensity from a video file using ffmpeg.
/// Returns an AudioProfile with RMS levels and detected spike positions.
fn analyze_audio_intensity(
    vod_path: &str,
    ffmpeg: &std::path::Path,
) -> Result<AudioProfile, AppError> {
    // Use ffmpeg's volumedetect + astats to get per-second RMS levels
    // We extract audio as raw PCM and analyze volume in 1-second windows
    let temp_file = std::env::temp_dir()
        .join("clipviral_audio")
        .join(format!("{}.txt", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(temp_file.parent().unwrap()).ok();

    // Escape the path for ffmpeg filter syntax — colons in Windows drive letters
    // (e.g. C:\...) conflict with ffmpeg's filter parameter separator (:)
    let escaped_path = temp_file.to_string_lossy()
        .replace('\\', "/")
        .replace(':', "\\:");

    let mut cmd = std::process::Command::new(ffmpeg);
    cmd.arg("-i").arg(vod_path)
       .arg("-af")
       .arg(format!(
           "astats=metadata=1:reset=1,ametadata=mode=print:file='{}'",
           escaped_path
       ))
       .arg("-vn")
       .arg("-f").arg("null")
       .arg("-")
       .stdout(Stdio::null())
       .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let status = cmd.status().map_err(|e| AppError::Ffmpeg(format!("Audio analysis launch failed: {e}")))?;
    if !status.success() {
        std::fs::remove_file(&temp_file).ok();
        return Err(AppError::Ffmpeg("Audio analysis exited with an error".into()));
    }

    // Parse the astats output file for RMS levels per frame
    let content = std::fs::read_to_string(&temp_file)
        .map_err(|e| AppError::Ffmpeg(format!("Read audio stats: {e}")))?;
    std::fs::remove_file(&temp_file).ok();

    let mut rms_values: Vec<f64> = Vec::new();
    let mut current_time: Option<f64> = None;
    let mut current_rms: Option<f64> = None;
    let mut last_second: i64 = -1;
    let mut second_rms_sum = 0.0_f64;
    let mut second_count = 0u32;

    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("lavfi.astats.Overall.RMS_level=") {
            if let Ok(val) = rest.trim().parse::<f64>() {
                current_rms = Some(val);
            }
        } else if line.starts_with("frame:") {
            // Each frame line contains pts_time
            if let Some(pts_pos) = line.find("pts_time:") {
                let pts_str = &line[pts_pos + 9..];
                if let Some(end) = pts_str.find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-') {
                    if let Ok(t) = pts_str[..end].parse::<f64>() {
                        current_time = Some(t);
                    }
                } else if let Ok(t) = pts_str.trim().parse::<f64>() {
                    current_time = Some(t);
                }
            }
        }

        // Accumulate RMS into per-second buckets
        if let (Some(t), Some(rms)) = (current_time, current_rms) {
            let sec = t as i64;
            if sec != last_second && last_second >= 0 && second_count > 0 {
                // Store average RMS for the previous second
                // RMS is in dB (negative), convert to linear 0..1 scale
                let avg_db = second_rms_sum / second_count as f64;
                // Clamp: -60dB = silence (0.0), 0dB = max (1.0)
                let linear = ((avg_db + 60.0) / 60.0).clamp(0.0, 1.0);
                // Fill any gaps
                while rms_values.len() < last_second as usize {
                    rms_values.push(0.0);
                }
                rms_values.push(linear);
                second_rms_sum = 0.0;
                second_count = 0;
            }
            last_second = sec;
            second_rms_sum += rms;
            second_count += 1;
            current_rms = None;
        }
    }
    // Push last second
    if second_count > 0 {
        let avg_db = second_rms_sum / second_count as f64;
        let linear = ((avg_db + 60.0) / 60.0).clamp(0.0, 1.0);
        while rms_values.len() < last_second as usize {
            rms_values.push(0.0);
        }
        rms_values.push(linear);
    }

    if rms_values.is_empty() {
        return Err(AppError::Ffmpeg("No audio data extracted".into()));
    }

    // Detect spikes: seconds where volume > 1.5x the rolling average
    let avg: f64 = rms_values.iter().sum::<f64>() / rms_values.len() as f64;
    let spike_threshold = (avg * 1.5).max(0.3); // At least 0.3 to avoid noise
    let spike_seconds: Vec<usize> = rms_values.iter().enumerate()
        .filter(|(_, &v)| v > spike_threshold)
        .map(|(i, _)| i)
        .collect();

    log::info!("Audio analysis: {} seconds, {} spikes detected (avg={:.3}, threshold={:.3})",
        rms_values.len(), spike_seconds.len(), avg, spike_threshold);

    Ok(AudioProfile { rms_per_second: rms_values, spike_seconds })
}

/// Generate a single thumbnail frame from a video at the given timestamp.
fn generate_thumbnail(
    ffmpeg: &std::path::Path,
    vod_path: &str,
    timestamp_secs: f64,
    output_path: &std::path::Path,
) -> Result<(), AppError> {
    let mut cmd = std::process::Command::new(ffmpeg);
    // Input-seeking (-ss before -i) is fast and accurate for MP4 files
    cmd.arg("-ss").arg(format!("{}", timestamp_secs))
       .arg("-i").arg(vod_path)
       .arg("-vframes").arg("1")
       .arg("-vf").arg("scale=640:-1")
       .arg("-q:v").arg("5")
       .arg("-y")
       .arg(output_path.to_string_lossy().as_ref())
       .stdout(Stdio::null())
       .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let status = cmd.status().map_err(|e| AppError::Ffmpeg(format!("Thumbnail launch failed: {e}")))?;
    // ffmpeg may return non-zero (e.g. 69 for MPEG-TS near end) but still write the file
    if output_path.exists() && std::fs::metadata(output_path).map(|m| m.len() > 0).unwrap_or(false) {
        Ok(())
    } else if status.success() {
        Ok(())
    } else {
        Err(AppError::Ffmpeg("Thumbnail generation failed".into()))
    }
}

// ── Speech-to-Text (faster-whisper) ──

/// Transcript data from faster-whisper
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TranscriptWord {
    word: String,
    start: f64,
    end: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TranscriptSegment {
    start: f64,
    end: f64,
    text: String,
    words: Vec<TranscriptWord>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TranscriptKeyword {
    pub keyword: String,
    pub timestamp: f64,
    pub end_timestamp: f64,
    pub context: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TranscriptResult {
    pub segments: Vec<TranscriptSegment>,
    pub full_text: String,
    pub language: String,
    pub keywords_found: Vec<TranscriptKeyword>,
}

/// Find Python executable path
fn find_python() -> Result<std::path::PathBuf, AppError> {
    // Check common Windows Python paths (user-independent)
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        for ver in &["Python312", "Python313", "Python311", "Python310"] {
            candidates.push(std::path::PathBuf::from(&local).join("Programs").join("Python").join(ver).join("python.exe"));
        }
    }
    candidates.push(std::path::PathBuf::from(r"C:\Python312\python.exe"));
    candidates.push(std::path::PathBuf::from(r"C:\Python311\python.exe"));
    for p in &candidates {
        if p.exists() {
            return Ok(p.clone());
        }
    }
    // Try PATH
    which::which("python").or_else(|_| which::which("python3"))
        .map_err(|_| AppError::Transcription("Python not found. Install Python 3.10+ to enable speech-to-text.".into()))
}

/// Run faster-whisper transcription on a video file.
/// Returns transcript JSON and saves to disk.
fn run_transcription(vod_path: &str, output_path: &str, hw: &HardwareInfo, vod_id: Option<&str>) -> Result<TranscriptResult, AppError> {
    let python = find_python()?;
    let device = if hw.use_cuda { "cuda" } else { "cpu" };

    // Locate transcribe.py
    let script = find_transcribe_script()?;

    log::info!("Transcription: python={} script={} device={}", python.display(), script.display(), device);

    // Quick diagnostic: check if faster-whisper is importable
    if let Ok(check) = std::process::Command::new(&python)
        .args(["-c", "import faster_whisper; print(faster_whisper.__version__)"])
        .env("CUDA_VISIBLE_DEVICES", "")
        .output()
    {
        if check.status.success() {
            let ver = String::from_utf8_lossy(&check.stdout);
            log::info!("faster-whisper version: {}", ver.trim());
        } else {
            let err = String::from_utf8_lossy(&check.stderr);
            log::warn!("faster-whisper import failed: {}", err.trim());
            return Err(AppError::Transcription(format!(
                "faster-whisper is not installed for {}. Run: {} -m pip install faster-whisper",
                python.display(), python.display()
            )));
        }
    }

    // Attempt transcription. If CUDA was requested and fails, retry on CPU.
    match run_transcription_with_script(&python, &script, vod_path, output_path, device, vod_id) {
        Ok(result) => Ok(result),
        Err(first_err) if device == "cuda" => {
            log::warn!("CUDA transcription failed ({}), retrying on CPU...", first_err.detail());
            run_transcription_with_script(&python, &script, vod_path, output_path, "cpu", vod_id)
                .map_err(|cpu_err| {
                    AppError::Transcription(format!(
                        "Failed on both CUDA and CPU. CUDA: {} | CPU: {}",
                        first_err.detail(), cpu_err.detail()
                    ))
                })
        }
        Err(e) => Err(e),
    }
}

/// Locate transcribe.py by searching project directories and AppData.
fn find_transcribe_script() -> Result<std::path::PathBuf, AppError> {
    let exe = std::env::current_exe().unwrap_or_default();
    let mut dir = exe.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();

    // Walk up from the executable directory (handles dev + release layouts)
    for _ in 0..5 {
        let candidate = dir.join("ai_engine").join("transcribe.py");
        if candidate.exists() {
            return Ok(candidate);
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => break,
        }
    }

    // Fallback: AppData directory
    let data_fallback = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clipviral")
        .join("ai_engine")
        .join("transcribe.py");
    if data_fallback.exists() {
        return Ok(data_fallback);
    }

    Err(AppError::Transcription(
        "transcribe.py not found — place it in ai_engine/ next to the executable or in AppData/clipviral/ai_engine/".into()
    ))
}

fn run_transcription_with_script(
    python: &std::path::Path,
    script: &std::path::Path,
    vod_path: &str,
    output_path: &str,
    device: &str,
    vod_id: Option<&str>,
) -> Result<TranscriptResult, AppError> {
    log::info!("Running transcription: {} {} --device {} --output {}", script.display(), vod_path, device, output_path);

    let mut cmd = std::process::Command::new(python);
    cmd.arg(script)
       .arg(vod_path)
       .arg("--model").arg("small")
       .arg("--device").arg(device)
       .arg("--output").arg(output_path)
       .stdout(Stdio::piped())
       .stderr(Stdio::piped());

    // When running in CPU mode, prevent CUDA library loading entirely.
    // faster-whisper (via CTranslate2) probes for cuBLAS at import time,
    // which crashes if CUDA DLLs are missing — even with --device cpu.
    // Blanking CUDA_VISIBLE_DEVICES forces the library to skip GPU init.
    if device == "cpu" {
        cmd.env("CUDA_VISIBLE_DEVICES", "");
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    // Spawn as a child process so we can read heartbeats from stderr
    // and enforce a timeout if the process truly hangs.
    let mut child = cmd.spawn()
        .map_err(|e| AppError::Transcription(format!("Failed to launch Python: {e}")))?;

    // Read stderr in a background thread to capture heartbeats + error output.
    // Heartbeats are JSON lines like {"heartbeat":true,"approx_pct":42,...}
    // emitted every ~15s by transcribe.py so we know it's still alive.
    let stderr_handle = child.stderr.take();
    let vod_id_for_thread = vod_id.map(|s| s.to_string());
    let (heartbeat_tx, heartbeat_rx) = std::sync::mpsc::channel::<()>();
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut err) = stderr_handle {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(&mut err);
            for line in reader.lines() {
                match line {
                    Ok(line) => {
                        // Try to parse heartbeat JSON
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                            if json.get("heartbeat").and_then(|v| v.as_bool()).unwrap_or(false) {
                                let pct = json.get("approx_pct").and_then(|v| v.as_i64()).unwrap_or(0);
                                let segs = json.get("segments_so_far").and_then(|v| v.as_i64()).unwrap_or(0);
                                log::info!("Transcription heartbeat: ~{}% done, {} segments", pct, segs);
                                // Signal the main thread that we're still alive
                                heartbeat_tx.send(()).ok();
                                // Update analysis progress (20-38% range maps to transcription)
                                if let Some(ref vid) = vod_id_for_thread {
                                    let mapped = 20 + (pct as i64 * 18 / 100).min(17);
                                    set_analysis_progress(vid, mapped);
                                }
                                continue;
                            }
                        }
                        // Not a heartbeat — collect as regular stderr
                        buf.extend_from_slice(line.as_bytes());
                        buf.push(b'\n');
                    }
                    Err(_) => break,
                }
            }
        }
        buf
    });

    // Wait for the process with a generous timeout.
    // The base timeout is 30 minutes, but resets on each heartbeat.
    // If we get no heartbeat AND no process exit for 5 minutes, assume hung.
    let no_heartbeat_timeout = std::time::Duration::from_secs(300); // 5 min with no heartbeat = stuck
    let mut last_activity = std::time::Instant::now();

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                // Drain any heartbeat signals (non-blocking)
                while heartbeat_rx.try_recv().is_ok() {
                    last_activity = std::time::Instant::now();
                }
                if last_activity.elapsed() > no_heartbeat_timeout {
                    log::error!("Transcription stalled — no heartbeat for {}s, killing process",
                        no_heartbeat_timeout.as_secs());
                    child.kill().ok();
                    child.wait().ok();
                    return Err(AppError::Transcription(
                        format!("Transcription stalled after {} minutes with no progress on {}. \
                            The process may have hung or run out of memory.",
                            no_heartbeat_timeout.as_secs() / 60, device)
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            Err(e) => {
                return Err(AppError::Transcription(format!("Failed to wait for transcription process: {e}")));
            }
        }
    };

    // Collect remaining output
    let stderr_buf = stderr_thread.join().unwrap_or_default();
    let mut stdout_buf = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        use std::io::Read;
        out.read_to_end(&mut stdout_buf).ok();
    }

    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr_buf);
        let stdout = String::from_utf8_lossy(&stdout_buf);
        log::error!("Transcription script failed (exit {}). stderr: {} stdout: {}",
            status.code().unwrap_or(-1), stderr.trim(), stdout.trim());

        // Parse structured error from stdout if the script managed to output JSON
        // (stdout may contain multiple JSON lines — check each)
        for line in stdout.lines() {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(line.trim()) {
                if let Some(err_msg) = json.get("error").and_then(|e| e.as_str()) {
                    return Err(AppError::Transcription(err_msg.to_string()));
                }
            }
        }

        // Fall back to stderr content
        let err_str = stderr.trim();
        if err_str.contains("unrecognized arguments") {
            return Err(AppError::Transcription(
                "Transcription script version mismatch. Update transcribe.py from ai_engine/.".into()
            ));
        }
        return Err(AppError::Transcription(format!("Script failed: {err_str}")));
    }

    let json_str = std::fs::read_to_string(output_path)
        .map_err(|e| AppError::Transcription(format!("Failed to read transcript output: {e}")))?;

    serde_json::from_str::<TranscriptResult>(&json_str)
        .map_err(|e| AppError::Transcription(format!("Invalid transcript JSON: {e}")))
}

/// Generate an SRT subtitle file from transcript segments for a specific clip time range.
fn generate_srt_for_clip(
    transcript: &TranscriptResult,
    clip_start: f64,
    clip_end: f64,
    output_path: &std::path::Path,
) -> Result<(), String> {
    let mut srt = String::new();
    let mut index = 1;

    for seg in &transcript.segments {
        // Only include segments that overlap with clip range
        if seg.end < clip_start || seg.start > clip_end {
            continue;
        }

        // Use word-level timestamps if available for better timing
        if !seg.words.is_empty() {
            // Group words into subtitle chunks (max ~8 words per subtitle)
            let mut chunk_words: Vec<&TranscriptWord> = Vec::new();
            for word in &seg.words {
                if word.end < clip_start || word.start > clip_end {
                    continue;
                }
                chunk_words.push(word);

                if chunk_words.len() >= 6 {
                    // Emit subtitle
                    let start_time = (chunk_words[0].start - clip_start).max(0.0);
                    let end_time = (chunk_words.last().unwrap().end - clip_start).max(0.0);
                    let text: Vec<&str> = chunk_words.iter().map(|w| w.word.as_str()).collect();

                    srt.push_str(&format!("{}\n", index));
                    srt.push_str(&format!("{} --> {}\n", format_srt_time(start_time), format_srt_time(end_time)));
                    srt.push_str(&format!("{}\n\n", text.join(" ")));
                    index += 1;
                    chunk_words.clear();
                }
            }
            // Emit remaining words
            if !chunk_words.is_empty() {
                let start_time = (chunk_words[0].start - clip_start).max(0.0);
                let end_time = (chunk_words.last().unwrap().end - clip_start).max(0.0);
                let text: Vec<&str> = chunk_words.iter().map(|w| w.word.as_str()).collect();

                srt.push_str(&format!("{}\n", index));
                srt.push_str(&format!("{} --> {}\n", format_srt_time(start_time), format_srt_time(end_time)));
                srt.push_str(&format!("{}\n\n", text.join(" ")));
            }
        } else {
            // Fall back to segment-level timing
            let start_time = (seg.start - clip_start).max(0.0);
            let end_time = (seg.end - clip_start).max(0.0);

            srt.push_str(&format!("{}\n", index));
            srt.push_str(&format!("{} --> {}\n", format_srt_time(start_time), format_srt_time(end_time)));
            srt.push_str(&format!("{}\n\n", seg.text));
            index += 1;
        }
    }

    std::fs::write(output_path, srt).map_err(|e| format!("Failed to write SRT: {}", e))
}

fn format_srt_time(seconds: f64) -> String {
    let h = (seconds / 3600.0) as u32;
    let m = ((seconds % 3600.0) / 60.0) as u32;
    let s = (seconds % 60.0) as u32;
    let ms = ((seconds % 1.0) * 1000.0) as u32;
    format!("{:02}:{:02}:{:02},{:03}", h, m, s, ms)
}

/// Extract the full dialogue text from transcript segments that overlap a clip's time range.
/// Concatenates all segment text into a single string — used to save a richer
/// `transcript_snippet` in the highlights table so Claude gets more context.
fn extract_transcript_for_range(transcript: &TranscriptResult, start: f64, end: f64) -> Option<String> {
    let texts: Vec<&str> = transcript.segments.iter()
        .filter(|seg| seg.end >= start && seg.start <= end)
        .map(|seg| seg.text.as_str())
        .collect();
    if texts.is_empty() {
        return None;
    }
    Some(texts.join(" ").trim().to_string())
}

/// Find keywords in transcript near a given timestamp range
fn keyword_boost_for_range(transcript: &TranscriptResult, start: f64, end: f64) -> f64 {
    let mut boost: f64 = 0.0;
    for kw in &transcript.keywords_found {
        if kw.timestamp >= start - 5.0 && kw.end_timestamp <= end + 5.0 {
            // Different boost based on keyword intensity
            let kw_lower = kw.keyword.to_lowercase();
            if kw_lower.contains("no way") || kw_lower.contains("oh my god") || kw_lower.contains("holy")
                || kw_lower.contains("what the") || kw_lower.contains("insane") {
                boost += 0.08; // High-intensity keywords
            } else if kw_lower.contains("let's go") || kw_lower.contains("clutch") || kw_lower.contains("destroyed")
                || kw_lower.contains("rage") || kw_lower.contains("noooo") || kw_lower.contains("yesss") {
                boost += 0.06; // Medium-intensity keywords
            } else {
                boost += 0.03; // Basic viral keywords
            }
        }
    }
    boost.min(0.15_f64) // Cap total keyword boost at 0.15
}

/// Analyze a VOD to find highlight-worthy moments.
/// Uses local signal analysis (audio + transcript + chat) when ffmpeg + downloaded VOD are available.
/// Falls back to position heuristics otherwise. No external API calls are made.
#[tauri::command]
async fn analyze_vod(vod_id: String, app: AppHandle, db: State<'_, DbConn>, hw: State<'_, HardwareInfo>) -> Result<(), String> {
    // Atomic check-and-set: read status, validate, and update in a single lock scope
    let (vod, has_ffmpeg) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let vod = db::get_vod_by_id(&conn, &vod_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or_else(|| "VOD not found".to_string())?;

        if vod.analysis_status == "analyzing" {
            return Err("Analysis is already in progress.".to_string());
        }

        db::update_vod_analysis_status(&conn, &vod_id, "analyzing")
            .map_err(|e| format!("DB error: {}", e))?;
        db::update_vod_analysis_progress(&conn, &vod_id, 0)
            .map_err(|e| format!("DB error: {}", e))?;

        let has_ffmpeg = find_ffmpeg().is_ok();
        (vod, has_ffmpeg)
    };

    // Read sensitivity setting before moving into background task
    let sensitivity = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::get_setting(&conn, "detection_sensitivity")
            .ok()
            .flatten()
            .unwrap_or_else(|| "medium".to_string())
    };

    let vod_id_bg = vod_id.clone();
    let vod_clone = vod.clone();
    let hw_info = hw.inner().clone();

    // Run analysis in background
    tokio::task::spawn(async move {
        let db: State<'_, DbConn> = app.state();

        // Progress updates: run_analysis_signals handles 5-82% internally
        // via direct DB connection. We handle 0-5% and 82-100% here.
        if let Ok(conn) = db.lock() {
            db::update_vod_analysis_progress(&conn, &vod_id_bg, 2).ok();
        }

        // Cascading analysis: signal-driven (local) → position heuristic.
        let has_local_file = vod_clone.local_path.is_some();

        let mut result: Result<Vec<db::HighlightRow>, String> = Err("No analysis method available".into());

        // Tier 1: Signal-driven (audio + transcript + chat) — fully local
        if has_ffmpeg && has_local_file {
            log::info!("Running signal-driven analysis for VOD {}", vod_id_bg);
            let vod_for_sync = vod_clone.clone();
            let hw_for_sync = hw_info.clone();
            let sens = sensitivity.clone();
            match tokio::task::spawn_blocking(move || run_analysis_signals(&vod_for_sync, &hw_for_sync, &sens)).await {
                Ok(Ok(highlights)) => { result = Ok(highlights); }
                Ok(Err(e)) => {
                    log::warn!("Signal analysis failed, falling back to position heuristic: {e}");
                }
                Err(e) => {
                    log::warn!("Signal analysis task panicked, falling back: {e}");
                }
            }
        }

        // Tier 2: Position heuristic (always available)
        if result.is_err() {
            log::info!("Running position fallback for VOD {} (ffmpeg={}, downloaded={})",
                vod_id_bg, has_ffmpeg, has_local_file);
            if let Ok(conn) = db.lock() {
                db::update_vod_analysis_progress(&conn, &vod_id_bg, 10).ok();
            }
            let vod_for_sync = vod_clone.clone();
            match tokio::task::spawn_blocking(move || run_analysis(&vod_for_sync)).await {
                Ok(r) => { result = r; }
                Err(e) => { result = Err(format!("Task error: {e}")); }
            }
        };

        // Creating clips from highlights (82-88%)
        if let Ok(conn) = db.lock() {
            db::update_vod_analysis_progress(&conn, &vod_id_bg, 83).ok();
        }

        match result {
            Ok(highlights) => {
                let mut clip_thumb_info: Vec<(String, f64)> = Vec::new();

                if let Ok(conn) = db.lock() {
                    // Clear previous analysis
                    db::delete_clips_for_vod(&conn, &vod_id_bg).ok();
                    db::delete_highlights_for_vod(&conn, &vod_id_bg).ok();

                    let now = chrono::Utc::now().to_rfc3339();

                    for h in &highlights {
                        db::insert_highlight(&conn, h).ok();

                        // Create a clip for each highlight
                        let clip_id = uuid::Uuid::new_v4().to_string();

                        // Check if auto-captions SRT exists for this highlight
                        let captions_dir = dirs::data_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join("clipviral")
                            .join("captions");
                        let srt_path = captions_dir.join(format!("{}.srt", h.id));
                        let auto_captions = if srt_path.exists() {
                            // Read SRT content to use as captions_text
                            std::fs::read_to_string(&srt_path).ok()
                        } else {
                            None
                        };

                        let clip = db::ClipRow {
                            id: clip_id.clone(),
                            highlight_id: h.id.clone(),
                            vod_id: h.vod_id.clone(),
                            title: h.description.clone().unwrap_or_else(|| "Highlight".to_string()),
                            start_seconds: h.start_seconds,
                            end_seconds: h.end_seconds,
                            aspect_ratio: "9:16".to_string(),
                            crop_x: None,
                            crop_y: None,
                            crop_width: None,
                            crop_height: None,
                            captions_enabled: 1,
                            captions_text: auto_captions,
                            captions_position: "bottom".to_string(),
                            caption_style: "clean".to_string(),
                            facecam_layout: "none".to_string(),
                            render_status: "pending".to_string(),
                            output_path: None,
                            thumbnail_path: None,
                            created_at: now.clone(),
                            game: vod_clone.game_name.clone(),
                            publish_description: None,
                            publish_hashtags: None,
                        };
                        db::insert_clip(&conn, &clip).ok();

                        // Save auto-captions path to clip
                        if srt_path.exists() {
                            db::update_clip_auto_captions(&conn, &clip_id, &srt_path.to_string_lossy()).ok();
                        }

                        clip_thumb_info.push((clip_id, h.start_seconds));
                    }

                    db::update_vod_analysis_progress(&conn, &vod_id_bg, 88).ok();
                }
                // conn lock dropped here

                // Generate thumbnails outside DB lock (88-98%)
                if let Ok(ffmpeg_path) = find_ffmpeg() {
                    if let Some(ref vod_path) = vod_clone.local_path {
                        let thumb_dir = dirs::data_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join("clipviral")
                            .join("thumbnails");
                        std::fs::create_dir_all(&thumb_dir).ok();

                        if let Ok(thumb_conn) = db::db_path().and_then(|p| rusqlite::Connection::open(p).map_err(|e| e.to_string())) {
                            let total_thumbs = clip_thumb_info.len();
                            for (idx, (clip_id, start_secs)) in clip_thumb_info.iter().enumerate() {
                                let thumb_path = thumb_dir.join(format!("{}.jpg", clip_id));
                                let dur = vod_clone.duration_seconds as f64;
                                let candidates = [
                                    (start_secs + 2.0).min(dur),
                                    (start_secs + 10.0).min(dur),
                                    (start_secs + 5.0).min(dur),
                                    (*start_secs).max(1.0),
                                ];
                                let min_thumb_size = 3000u64;
                                let mut saved = false;
                                for ts in &candidates {
                                    if generate_thumbnail(&ffmpeg_path, vod_path, *ts, &thumb_path).is_ok() {
                                        let sz = std::fs::metadata(&thumb_path).map(|m| m.len()).unwrap_or(0);
                                        if sz >= min_thumb_size {
                                            db::update_clip_thumbnail(
                                                &thumb_conn, clip_id,
                                                Some(&thumb_path.to_string_lossy()),
                                            ).ok();
                                            saved = true;
                                            break;
                                        }
                                    }
                                }
                                if !saved {
                                    if thumb_path.exists() {
                                        db::update_clip_thumbnail(
                                            &thumb_conn, clip_id,
                                            Some(&thumb_path.to_string_lossy()),
                                        ).ok();
                                    }
                                }
                                // Update progress per thumbnail
                                if total_thumbs > 0 {
                                    let thumb_progress = 88 + ((idx + 1) as i64 * 10 / total_thumbs as i64);
                                    db::update_vod_analysis_progress(&thumb_conn, &vod_id_bg, thumb_progress).ok();
                                }
                            }
                        }
                    }
                }

                // Mark complete
                if let Ok(conn) = db.lock() {
                    db::update_vod_analysis_status(&conn, &vod_id_bg, "completed").ok();
                    db::update_vod_analysis_progress(&conn, &vod_id_bg, 100).ok();
                }
            }
            Err(e) => {
                log::error!("Analysis failed: {}", e);
                if let Ok(conn) = db.lock() {
                    db::update_vod_analysis_status(&conn, &vod_id_bg, "failed").ok();
                }
            }
        }
    });

    Ok(())
}

/// Helper: update analysis progress directly (opens its own DB connection).
/// Used inside `spawn_blocking` where the Tauri State DB isn't available.
fn set_analysis_progress(vod_id: &str, progress: i64) {
    if let Ok(path) = db::db_path() {
        if let Ok(conn) = rusqlite::Connection::open(path) {
            db::update_vod_analysis_progress(&conn, vod_id, progress).ok();
        }
    }
}

/// Signal-driven analysis using the clip_selector module.
/// Finds clips via audio spikes, transcript keywords, and chat peaks.
fn run_analysis_signals(vod: &db::VodRow, hw: &HardwareInfo, sensitivity: &str) -> Result<Vec<db::HighlightRow>, String> {
    let ffmpeg = find_ffmpeg()?;
    let vod_path = vod.local_path.clone()
        .ok_or("VOD not downloaded")?;
    let duration = vod.duration_seconds as f64;
    let vod_id = &vod.id;
    let now = chrono::Utc::now().to_rfc3339();

    // ── Stage 1: Audio analysis (5-15%) ──
    log::info!("Signal analysis: extracting audio profile...");
    set_analysis_progress(vod_id, 5);
    let audio_profile = analyze_audio_intensity(&vod_path, &ffmpeg).ok();
    let audio_ctx = audio_profile.as_ref().map(|a| {
        clip_selector::AudioContext::new(a.rms_per_second.clone(), a.spike_seconds.clone())
    });
    set_analysis_progress(vod_id, 15);

    // ── Stage 2: Transcription (15-40%) ──
    log::info!("Signal analysis: attempting transcription...");
    set_analysis_progress(vod_id, 18);
    let transcript_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clipviral")
        .join("transcripts");
    std::fs::create_dir_all(&transcript_dir).ok();
    let transcript_path = transcript_dir.join(format!("{}.json", vod_id));
    let transcript: Option<TranscriptResult> = if transcript_path.exists() {
        log::info!("Signal analysis: loading cached transcript");
        set_analysis_progress(vod_id, 25);
        std::fs::read_to_string(&transcript_path).ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    } else if let Ok(_python) = find_python() {
        set_analysis_progress(vod_id, 20);
        let out = transcript_path.to_string_lossy().to_string();
        let result = run_transcription(&vod_path, &out, hw, Some(vod_id)).ok();
        set_analysis_progress(vod_id, 38);
        result
    } else {
        None
    };
    set_analysis_progress(vod_id, 40);

    // ── Stage 3: Chat analysis (40-50%) ──
    log::info!("Signal analysis: analyzing chat activity...");
    set_analysis_progress(vod_id, 42);
    let chat_peaks: Vec<db::HighlightRow> = analyze_via_chat(vod).unwrap_or_default();
    set_analysis_progress(vod_id, 50);

    // ── Stage 4: Clip selection pipeline (50-65%) ──
    log::info!("Signal analysis: running clip selector pipeline...");
    set_analysis_progress(vod_id, 52);
    let (selected, detection_stats) = clip_selector::select_clips(
        audio_ctx.as_ref(),
        transcript.as_ref(),
        &chat_peaks,
        duration,
        sensitivity,
    );
    set_analysis_progress(vod_id, 60);

    // Persist detection stats for the VOD page to display
    if let Ok(db_path) = db::db_path() {
        if let Ok(conn) = rusqlite::Connection::open(&db_path) {
            let stats_json = serde_json::to_string(&detection_stats).unwrap_or_default();
            db::save_setting(&conn, &format!("detection_stats_{}", vod_id), &stats_json).ok();
        }
    }

    if selected.is_empty() {
        log::warn!("Signal analysis: selector returned no clips, falling back to position heuristic");
        set_analysis_progress(vod_id, 55);
        return run_analysis(vod);
    }

    // ── Stage 5: Scoring and ranking (60-75%) ──
    log::info!("Signal analysis: scoring {} candidates...", selected.len());
    set_analysis_progress(vod_id, 62);
    let mut highlights: Vec<db::HighlightRow> = Vec::new();
    let total_candidates = selected.len();

    for (i, c) in selected.iter().enumerate() {
        let all_tags: Vec<String> = [&c.event_tags[..], &c.emotion_tags[..]].concat();
        let tag_str = if all_tags.is_empty() { "auto".to_string() } else { all_tags.join(",") };

        let title = grounded_highlight_title(
            c.transcript_excerpt.as_deref(),
            Some(&tag_str),
            c.start_time,
        );

        let kw_boost = if let Some(ref t) = transcript {
            keyword_boost_for_range(t, c.start_time, c.end_time)
        } else {
            0.0
        };

        let raw_score = (c.total_score + kw_boost).min(0.99);
        let audio = c.hook_strength;
        let visual = c.emotional_spike;
        let chat = if c.signal_sources.contains(&clip_selector::SignalSource::Chat) {
            c.event_reaction_alignment
        } else { 0.0 };
        let has_transcript = c.transcript_excerpt.is_some();
        let sig_count = count_active_signals(audio, visual, chat, has_transcript);

        let event_summary = crate::post_captions::generate_event_summary_from_parts(
            &all_tags,
            c.transcript_excerpt.as_deref(),
            audio, visual, 0.0, c.start_time,
        );

        // Use full transcript for the clip range if available; fall back to
        // the single-sentence excerpt from signal fusion.
        let full_range_transcript = transcript.as_ref()
            .and_then(|t| extract_transcript_for_range(t, c.start_time, c.end_time))
            .or_else(|| c.transcript_excerpt.clone());

        highlights.push(db::HighlightRow {
            id: uuid::Uuid::new_v4().to_string(),
            vod_id: vod_id.clone(),
            start_seconds: c.start_time,
            end_seconds: c.end_time,
            virality_score: raw_score,
            audio_score: audio,
            visual_score: visual,
            chat_score: chat,
            transcript_snippet: full_range_transcript,
            description: Some(title),
            tags: Some(tag_str),
            thumbnail_path: None,
            created_at: now.clone(),
            confidence_score: Some(compute_confidence(raw_score, sig_count)),
            explanation: Some(build_highlight_explanation(audio, visual, chat, has_transcript)),
            event_summary: Some(event_summary),
        });

        // Update progress within scoring loop
        let scoring_progress = 62 + ((i + 1) as i64 * 13 / total_candidates as i64);
        set_analysis_progress(vod_id, scoring_progress);
    }
    set_analysis_progress(vod_id, 75);

    // ── Stage 6: Generate captions (75-82%) ──
    log::info!("Signal analysis: generating captions...");
    set_analysis_progress(vod_id, 76);
    if let Some(ref t) = transcript {
        let captions_dir = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("clipviral")
            .join("captions");
        std::fs::create_dir_all(&captions_dir).ok();

        let mut srt_count = 0;
        for h in &highlights {
            let srt_path = captions_dir.join(format!("{}.srt", h.id));
            if generate_srt_for_clip(t, h.start_seconds, h.end_seconds, &srt_path).is_ok() {
                srt_count += 1;
            }
        }
        if srt_count > 0 {
            log::info!("Signal analysis: generated {} SRT caption files", srt_count);
        }
    }
    set_analysis_progress(vod_id, 82);

    log::info!("Signal analysis: produced {} final clips", highlights.len());
    Ok(highlights)
}

/// Position-based fallback — last resort when no signals are available.
/// Only used when VOD is not downloaded or ffmpeg is missing.
fn run_analysis(vod: &db::VodRow) -> Result<Vec<db::HighlightRow>, String> {
    let duration = vod.duration_seconds as f64;
    let vod_id = &vod.id;
    let now = chrono::Utc::now().to_rfc3339();

    // Try chat-based analysis first
    if let Ok(chat_highlights) = analyze_via_chat(vod) {
        if !chat_highlights.is_empty() {
            return Ok(chat_highlights);
        }
    }

    // Fallback: duration-based heuristic analysis
    let mut highlights = Vec::new();

    if duration <= 60.0 {
        highlights.push(db::HighlightRow {
            id: uuid::Uuid::new_v4().to_string(),
            vod_id: vod_id.clone(),
            start_seconds: 0.0,
            end_seconds: duration,
            virality_score: 0.75,
            audio_score: 0.7,
            visual_score: 0.7,
            chat_score: 0.5,
            transcript_snippet: None,
            description: Some(format!("Full clip at 0:00")),
            tags: Some("full,highlight".to_string()),
            thumbnail_path: None,
            created_at: now.clone(),
            confidence_score: Some(compute_confidence(0.75, 0)),
            explanation: Some("Position-based estimate, no signal analysis".into()),
            event_summary: None,
        });
    } else {
        let clip_duration = 30.0_f64.min(duration * 0.15);
        let positions: Vec<(f64, f64)> = if duration < 300.0 {
            vec![(0.05, 0.85), (0.45, 0.78), (0.80, 0.82)]
        } else {
            vec![(0.03, 0.80), (0.20, 0.75), (0.40, 0.82), (0.60, 0.78), (0.85, 0.88)]
        };

        for (frac, score) in positions {
            let start = (duration * frac).max(0.0);
            let end = (start + clip_duration).min(duration);
            if end - start < 5.0 { continue; }

            let mins = (start as u32) / 60;
            let secs = (start as u32) % 60;

            highlights.push(db::HighlightRow {
                id: uuid::Uuid::new_v4().to_string(),
                vod_id: vod_id.clone(),
                start_seconds: start,
                end_seconds: end,
                virality_score: score,
                audio_score: score * 0.9,
                visual_score: score * 0.95,
                chat_score: 0.5,
                transcript_snippet: None,
                description: Some(format!("Estimated highlight at {}:{:02}", mins, secs)),
                tags: Some("auto,estimated".to_string()),
                thumbnail_path: None,
                created_at: now.clone(),
                confidence_score: Some(compute_confidence(score, 0)),
                explanation: Some("Position-based estimate, no signal analysis".into()),
                event_summary: None,
            });
        }
    }

    Ok(highlights)
}

/// Try to analyze a VOD using Twitch chat replay (downloaded via yt-dlp).
fn analyze_via_chat(vod: &db::VodRow) -> Result<Vec<db::HighlightRow>, String> {
    let ytdlp = find_ytdlp()?;
    let temp_dir = std::env::temp_dir().join("clipviral_chat");
    std::fs::create_dir_all(&temp_dir).ok();

    let out_template = temp_dir.join(&vod.twitch_video_id).to_string_lossy().to_string();

    let mut cmd = std::process::Command::new(&ytdlp);
    cmd.arg("--write-subs")
        .arg("--sub-lang").arg("live_chat")
        .arg("--skip-download")
        .arg("--no-warnings")
        .arg("-o").arg(&out_template)
        .arg(&vod.vod_url)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let status = cmd.status().map_err(|e| format!("yt-dlp chat: {}", e))?;
    if !status.success() {
        return Err("Chat download failed".to_string());
    }

    let chat_path = temp_dir.join(format!("{}.live_chat.json", vod.twitch_video_id));
    if !chat_path.exists() {
        return Err("No chat file found".to_string());
    }

    let content = std::fs::read_to_string(&chat_path).map_err(|e| format!("Read chat: {}", e))?;
    let duration = vod.duration_seconds as f64;
    let window_size = 30.0_f64.max(duration * 0.05);

    let num_windows = ((duration / window_size).ceil() as usize).max(1);
    let mut window_counts = vec![0u32; num_windows];
    let mut total_messages = 0u32;

    for line in content.lines() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            let offset = val.get("time_in_seconds")
                .or_else(|| val.get("content_offset_seconds"))
                .and_then(|v| v.as_f64());
            if let Some(t) = offset {
                let idx = ((t / window_size) as usize).min(num_windows - 1);
                window_counts[idx] += 1;
                total_messages += 1;
            }
        }
    }

    std::fs::remove_file(&chat_path).ok();

    if total_messages < 5 {
        return Err("Not enough chat data".to_string());
    }

    let avg = total_messages as f64 / num_windows as f64;
    let mut peaks: Vec<(usize, u32)> = window_counts.iter().enumerate()
        .filter(|(_, &count)| count as f64 > avg * 1.3)
        .map(|(i, &count)| (i, count))
        .collect();
    peaks.sort_by(|a, b| b.1.cmp(&a.1));
    peaks.truncate(5);

    if peaks.is_empty() {
        return Err("No engagement peaks found".to_string());
    }

    let max_count = peaks[0].1 as f64;
    let now = chrono::Utc::now().to_rfc3339();
    let mut highlights = Vec::new();

    for (idx, count) in &peaks {
        let start = (*idx as f64 * window_size).max(0.0);
        let end = (start + window_size).min(duration);
        let chat_score = *count as f64 / max_count;
        let virality = 0.5 + chat_score * 0.45;

        let mins = (start as u32) / 60;
        let secs = (start as u32) % 60;
        highlights.push(db::HighlightRow {
            id: uuid::Uuid::new_v4().to_string(),
            vod_id: vod.id.clone(),
            start_seconds: start,
            end_seconds: end,
            virality_score: virality,
            audio_score: virality * 0.9,
            visual_score: virality * 0.85,
            chat_score,
            transcript_snippet: Some(format!("{} chat messages in this window", count)),
            description: Some(format!("Chat spike ({} msgs) at {}:{:02}", count, mins, secs)),
            tags: Some("chat-peak,reaction,auto".to_string()),
            thumbnail_path: None,
            created_at: now.clone(),
            confidence_score: Some(compute_confidence(virality, 1)),
            explanation: Some(format!("1 signal — chat {:.0}% ({} messages)", chat_score * 100.0, count)),
            event_summary: Some(format!("chat went off with {} messages", count)),
        });
    }

    Ok(highlights)
}

// ── Clip Export / Rendering ──

// NOTE: build_filter_graph and render_clip_with_ffmpeg have been replaced by
// vertical_crop::build_export_command + clip_to_export_request + build_caption_filter.
// The new pipeline is used by export_clip below.

// Legacy build_filter_graph — kept temporarily for reference during migration.
// TODO: Remove once vertical_crop integration is verified in production.
#[allow(dead_code)]
fn build_filter_graph(clip: &db::ClipRow) -> (String, bool) {
    let (tw, th) = match clip.aspect_ratio.as_str() {
        "9:16" => (1080, 1920),
        "1:1" => (1080, 1080),
        _ => (1920, 1080),
    };

    let captions_active = clip.captions_enabled == 1
        && clip.captions_text.as_ref().map_or(false, |t| !t.is_empty());

    let caption_filter = if captions_active {
        let text = clip.captions_text.as_ref().unwrap();

        // Check if captions_text looks like SRT format (has timestamps like "00:00:01,000 -->")
        let is_srt = text.contains("-->") && text.lines().count() > 2;

        if is_srt {
            // Write SRT to a temp file for ffmpeg subtitles filter
            let srt_temp = std::env::temp_dir().join(format!("clip_{}.srt", clip.id));
            std::fs::write(&srt_temp, text).ok();
            let srt_path = srt_temp.to_string_lossy().to_string()
                .replace('\\', "/")  // ffmpeg needs forward slashes
                .replace(':', "\\:");  // Escape colons for filter syntax

            let ypos = match clip.captions_position.as_str() {
                "top" => 30,
                "center" => th / 2 - 30,
                _ => th - 120,
            };

            Some(format!(
                "subtitles='{}':\
                 force_style='FontSize=24,FontName=Arial,PrimaryColour=&HFFFFFF,\
                 OutlineColour=&H000000,Outline=2,Shadow=1,\
                 Alignment=2,MarginV={}'",
                srt_path, ypos
            ))
        } else {
            // Static drawtext for manually entered captions
            // Escape ffmpeg special characters to prevent text expansion injection
            let esc = text
                .replace('\\', "\\\\")
                .replace('\'', "'\\''")
                .replace(':', "\\:")
                .replace('%', "%%")
                .replace('[', "\\[")
                .replace(']', "\\]")
                .replace(';', "\\;");
            let ypos = match clip.captions_position.as_str() {
                "top" => "h*0.08",
                "center" => "(h-text_h)/2",
                _ => "h*0.85",
            };
            Some(format!(
                "drawtext=text='{}':fontsize=48:fontcolor=white:borderw=3:bordercolor=black:x=(w-text_w)/2:y={}",
                esc, ypos
            ))
        }
    } else {
        None
    };

    match clip.facecam_layout.as_str() {
        "split" => {
            let th_top = (th as f64 * 0.6) as i32;
            let th_bot = th - th_top;
            let mut f = format!(
                "[0:v]split[a][b];\
                 [a]crop=iw:ih*0.6:0:0,scale={}:{}[top];\
                 [b]crop=iw*0.4:ih*0.4:0:ih*0.6,scale={}:{}[bottom];\
                 [top][bottom]vstack",
                tw, th_top, tw, th_bot
            );
            if let Some(cf) = caption_filter {
                f.push_str(&format!("[stacked];[stacked]{}[out]", cf));
            } else {
                f.push_str("[out]");
            }
            (f, true)
        }
        "pip" => {
            let ps = (tw as f64 * 0.3) as i32;
            let mut f = format!(
                "[0:v]split[bg][ps];\
                 [bg]scale={}:{}:force_original_aspect_ratio=increase,crop={}:{}[main];\
                 [ps]crop=iw*0.3:ih*0.3:0:ih*0.7,scale={}:{}[pip];\
                 [main][pip]overlay=W-w-20:H-h-20",
                tw, th, tw, th, ps, ps
            );
            if let Some(cf) = caption_filter {
                f.push_str(&format!("[overlaid];[overlaid]{}[out]", cf));
            } else {
                f.push_str("[out]");
            }
            (f, true)
        }
        _ => {
            // Use the vertical_crop module for quality-preserving
            // crop-first-then-scale logic.  Handles landscape, vertical,
            // and undersized inputs correctly.
            let target = vertical_crop::OutputSize { width: tw as u32, height: th as u32 };
            let base = vertical_crop::vertical_filter(target, vertical_crop::CropAnchor::Center);
            let mut parts = vec![base];
            if let Some(cf) = caption_filter {
                parts.push(cf);
            }
            (parts.join(","), false)
        }
    }
}

#[allow(dead_code)]
fn render_clip_with_ffmpeg(
    ffmpeg: &std::path::Path,
    vod_path: &str,
    clip: &db::ClipRow,
    output_path: &std::path::Path,
) -> Result<(), AppError> {
    let (filter, is_complex) = build_filter_graph(clip);

    let mut cmd = std::process::Command::new(ffmpeg);
    cmd.arg("-ss").arg(format!("{}", clip.start_seconds))
       .arg("-to").arg(format!("{}", clip.end_seconds))
       .arg("-i").arg(vod_path);

    if is_complex {
        cmd.arg("-filter_complex").arg(&filter)
           .arg("-map").arg("[out]")
           .arg("-map").arg("0:a?");
    } else {
        cmd.arg("-vf").arg(&filter);
    }

    cmd.arg("-c:v").arg("libx264")
       .arg("-preset").arg("medium")
       .arg("-crf").arg("23")
       .arg("-c:a").arg("aac")
       .arg("-b:a").arg("128k")
       .arg("-movflags").arg("+faststart")
       .arg("-y")
       .arg(output_path.to_string_lossy().as_ref())
       .stdout(Stdio::null())
       .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let status = cmd.status().map_err(|e| AppError::Ffmpeg(format!("Render launch failed: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(AppError::Ffmpeg("Clip rendering exited with an error".into()))
    }
}

/// Generate captions for a clip by running speech-to-text on the VOD audio.
/// Returns the SRT text if successful, or an error message.
#[tauri::command]
async fn generate_clip_captions(
    clip_id: String,
    db: State<'_, DbConn>,
    hw: State<'_, HardwareInfo>,
) -> Result<String, String> {
    let (clip, vod) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let clip = db::get_clip_by_id(&conn, &clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("Clip not found")?;
        let vod = db::get_vod_by_id(&conn, &clip.vod_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("VOD not found")?;
        (clip, vod)
    };

    let vod_path = vod.local_path.clone().ok_or("VOD not downloaded")?;

    // Check for cached transcript first
    let transcript_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clipviral")
        .join("transcripts");
    std::fs::create_dir_all(&transcript_dir).ok();
    let transcript_path = transcript_dir.join(format!("{}.json", vod.id));

    let transcript: TranscriptResult = if transcript_path.exists() {
        let json_str = std::fs::read_to_string(&transcript_path)
            .map_err(|e| format!("Read transcript: {}", e))?;
        serde_json::from_str(&json_str)
            .map_err(|e| format!("Parse transcript: {}", e))?
    } else {
        // Run speech-to-text
        let vp = vod_path.clone();
        let out = transcript_path.to_string_lossy().to_string();
        let hw_clone = hw.inner().clone();
        tokio::task::spawn_blocking(move || {
            run_transcription(&vp, &out, &hw_clone, None)
        }).await.map_err(|e| format!("Task error: {}", e))??
    };

    // Generate SRT for this clip's time range
    let captions_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clipviral")
        .join("captions");
    std::fs::create_dir_all(&captions_dir).ok();
    let srt_path = captions_dir.join(format!("{}.srt", clip.id));

    generate_srt_for_clip(&transcript, clip.start_seconds, clip.end_seconds, &srt_path)?;

    let srt_text = std::fs::read_to_string(&srt_path)
        .map_err(|e| format!("Read SRT: {}", e))?;

    if srt_text.trim().is_empty() {
        return Err("No speech detected in this clip's time range".to_string());
    }

    // Save to clip
    {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::save_setting(&conn, &format!("clip_{}_captions", clip_id), &srt_text).ok();
        // Update clip captions_text directly
        conn.execute(
            "UPDATE clips SET captions_text = ?1 WHERE id = ?2",
            rusqlite::params![srt_text, clip_id],
        ).map_err(|e| format!("DB error: {}", e))?;
    }

    Ok(srt_text)
}

/// Set a clip's thumbnail to a specific frame at the given absolute time.
#[tauri::command]
fn set_clip_thumbnail(
    clip_id: String,
    timestamp: f64,
    db: State<'_, DbConn>,
) -> Result<String, String> {
    let ffmpeg = find_ffmpeg()?;

    let vod_path = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let clip = db::get_clip_by_id(&conn, &clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("Clip not found")?;
        let vod = db::get_vod_by_id(&conn, &clip.vod_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("VOD not found")?;
        vod.local_path.ok_or("VOD not downloaded")?
    };

    let thumb_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clipviral")
        .join("thumbnails");
    std::fs::create_dir_all(&thumb_dir).ok();
    let thumb_path = thumb_dir.join(format!("{}.jpg", clip_id));

    generate_thumbnail(&ffmpeg, &vod_path, timestamp, &thumb_path)?;

    let path_str = thumb_path.to_string_lossy().to_string();
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::update_clip_thumbnail(&conn, &clip_id, Some(&path_str))
        .map_err(|e| format!("DB error: {}", e))?;

    Ok(path_str)
}

/// Export a clip — renders the clip segment with configured settings using ffmpeg.
#[tauri::command]
async fn export_clip(
    clip_id: String,
    app: AppHandle,
    db: State<'_, DbConn>,
    queue: State<'_, JobQueue>,
) -> Result<(), String> {
    let ffmpeg = find_ffmpeg().map_err(|e| report_error(&app, e))?;

    let (clip, vod_path) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let clip = db::get_clip_by_id(&conn, &clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("Clip not found")?;
        let vod = db::get_vod_by_id(&conn, &clip.vod_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("VOD not found")?;
        let path = vod.local_path.ok_or("VOD not downloaded — download it first to export clips")?;
        (clip, path)
    };

    let job_id = format!("export-{}", clip_id);
    let clip_id_bg = clip_id.clone();

    queue.add_job(job_id, move |handle| async move {
        // Mark rendering in DB inside the job, so status is only set once
        // the job is actually running (not stuck if app crashes before queuing).
        {
            let db_path = db::db_path().map_err(|e| format!("DB path error: {e}"))?;
            let conn = rusqlite::Connection::open(db_path)
                .map_err(|e| format!("DB error: {e}"))?;
            db::update_clip_render_status(&conn, &clip_id_bg, "rendering", None)
                .map_err(|e| format!("DB error: {}", e))?;
        }
        // ── Preparing ──
        handle.set_progress(5);

        let output_dir = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("clipviral")
            .join("exports");
        std::fs::create_dir_all(&output_dir)
            .map_err(|e| format!("Failed to create export directory: {e}"))?;
        let output_path = output_dir.join(format!("{}.mp4", clip_id_bg));

        // ── Building export request ──
        handle.set_progress(5);
        let request = clip_to_export_request(&clip, &vod_path, &output_path);

        // ── Running ffmpeg with real progress ──
        let output_ref = output_path.clone();
        let clip_id_ref = clip_id_bg.clone();
        let handle_ref = handle.clone();

        let result = tokio::task::spawn_blocking(move || {
            vertical_crop::run_export(&ffmpeg, &request, |pct| {
                handle_ref.set_progress(pct);
            })
        })
        .await
        .map_err(|e| format!("Export task panicked: {e}"))?;

        // ── Update DB with result ──
        let db_path = db::db_path().map_err(|e| format!("DB path error: {e}"))?;
        let conn = rusqlite::Connection::open(db_path)
            .map_err(|e| format!("DB error: {e}"))?;

        if result.success {
            db::update_clip_render_status(
                &conn, &clip_id_ref, "completed",
                Some(&output_ref.to_string_lossy()),
            ).ok();
            handle.set_progress(100);
            Ok(())
        } else {
            db::update_clip_render_status(&conn, &clip_id_ref, "failed", None).ok();
            let msg = if result.stderr_tail.is_empty() {
                "FFmpeg exited with an error".to_string()
            } else {
                format!("FFmpeg error: {}", result.stderr_tail)
            };
            Err(msg)
        }
    });

    Ok(())
}

/// Convert a DB ClipRow into an ExportRequest for the vertical_crop module.
fn clip_to_export_request(
    clip: &db::ClipRow,
    vod_path: &str,
    output_path: &std::path::Path,
) -> vertical_crop::ExportRequest {
    // Resolve platform from aspect ratio (future: store preset id in DB)
    let platform = vertical_crop::Platform::from_aspect_ratio(&clip.aspect_ratio);
    let target = platform.resolution();

    // Resolve layout from DB string
    let layout = vertical_crop::LayoutMode::from_db(&clip.facecam_layout);

    // Build caption filter if captions are enabled
    let caption_filter = build_caption_filter(clip, target.width as i32, target.height as i32);

    vertical_crop::ExportRequest {
        source_path: std::path::PathBuf::from(vod_path),
        output_path: output_path.to_path_buf(),
        start: clip.start_seconds,
        end: clip.end_seconds,
        platform,
        target,
        layout,
        caption_filter,
    }
}

/// Per-style parameters for FFmpeg subtitle rendering.
/// Maps the frontend CaptionStyle definitions in editTypes.ts to FFmpeg filter params.
/// `font_size` matches the editTypes.ts values (designed for 1080px-wide output).
struct SubStyle {
    font_name: &'static str,
    /// Font size in pixels at 1080px-wide reference (matches editTypes.ts fontSize).
    /// Used for both SRT (via original_size) and drawtext paths.
    font_size: i32,
    /// CSS font-weight (100–900).  Mapped to ASS Bold flag (-1 for ≥700, 0 otherwise)
    /// AND injected as `\b<weight>` override for sub-bold granularity (e.g. 800).
    font_weight: i32,
    /// ASS primary colour in &HBBGGRR format (text fill).
    primary_colour: &'static str,
    /// ASS outline colour.
    outline_colour: &'static str,
    /// ASS back colour in &HAABBGGRR (used when border_style=3 for opaque box).
    back_colour: &'static str,
    outline: i32,
    shadow: i32,
    /// 1 = outline + drop shadow, 3 = opaque background box.
    border_style: i32,
    /// Letter spacing in ASS units.
    spacing: f32,
    /// ASS \blur value for the glow layer — gaussian blur radius.
    /// Only used when glow_colour is set.  0 = no glow layer.
    glow_blur: i32,
    /// Glow colour in &HAABBGGRR ASS format.  When non-empty a second "Glow"
    /// ASS style is emitted: same text, larger outline in this colour, blurred,
    /// rendered on a lower layer beneath the crisp foreground.
    glow_colour: &'static str,
    uppercase: bool,
    /// Hex colour for drawtext fontcolor (CSS-order #RRGGBB or named).
    dt_fontcolor: &'static str,
    /// drawtext border width.
    dt_borderw: i32,
    /// Optional drawtext box=1 background colour (empty = no box).
    dt_boxcolor: &'static str,
}

fn get_sub_style(id: &str) -> SubStyle {
    match id {
        // font_size values match editTypes.ts fontSize (px at 1080px-wide reference)
        // font_weight values match editTypes.ts fontWeight
        "bold-white" => SubStyle {
            font_name: "Impact", font_size: 58, font_weight: 900,
            primary_colour: "&HFFFFFF", outline_colour: "&H000000",
            back_colour: "&H00000000", outline: 3, shadow: 1, border_style: 1,
            spacing: 1.5, glow_blur: 0, glow_colour: "", uppercase: true,
            dt_fontcolor: "white", dt_borderw: 4, dt_boxcolor: "",
        },
        "boxed" => SubStyle {
            font_name: "Arial", font_size: 46, font_weight: 600,
            primary_colour: "&HFFFFFF", outline_colour: "&H000000",
            back_colour: "&H38000000", outline: 0, shadow: 0, border_style: 3,
            spacing: 0.8, glow_blur: 0, glow_colour: "", uppercase: false,
            dt_fontcolor: "white", dt_borderw: 0, dt_boxcolor: "black@0.78",
        },
        "neon" => SubStyle {
            // Segoe UI is the frontend font on Windows; fall back to Arial
            font_name: "Segoe UI", font_size: 54, font_weight: 800,
            // #00FF88 → R=00 G=FF B=88 → ASS &HBBGGRR = &H88FF00
            primary_colour: "&H88FF00", outline_colour: "&H000000",
            back_colour: "&H00000000",
            // CSS uses 4 stacked black shadows → thick outline.  Outline=4 matches.
            outline: 4, shadow: 0, border_style: 1,
            spacing: 1.2,
            // Glow layer: bright green, gaussian-blurred behind text
            // CSS: '0 0 8px #00ff8880' (#80 hex ≈ 50% opacity)
            // ASS alpha: 00=opaque FF=transparent → &H80 = 50% transparent = 50% opaque
            glow_blur: 8, glow_colour: "&H8088FF00",
            uppercase: true,
            dt_fontcolor: "#00FF88", dt_borderw: 3, dt_boxcolor: "",
        },
        "minimal" => SubStyle {
            font_name: "Arial", font_size: 40, font_weight: 500,
            primary_colour: "&HFFFFFF", outline_colour: "&H000000",
            back_colour: "&H00000000", outline: 1, shadow: 1, border_style: 1,
            spacing: 0.8, glow_blur: 0, glow_colour: "", uppercase: false,
            dt_fontcolor: "white@0.92", dt_borderw: 1, dt_boxcolor: "",
        },
        "fire" => SubStyle {
            font_name: "Impact", font_size: 56, font_weight: 900,
            // #FF4444 → R=FF G=44 B=44 → ASS &H4444FF
            primary_colour: "&H4444FF", outline_colour: "&H000000",
            back_colour: "&H00000000", outline: 3, shadow: 1, border_style: 1,
            spacing: 1.2, glow_blur: 0, glow_colour: "", uppercase: true,
            dt_fontcolor: "#FF4444", dt_borderw: 4, dt_boxcolor: "",
        },
        // "clean" and any unknown style
        _ => SubStyle {
            font_name: "Arial", font_size: 52, font_weight: 700,
            primary_colour: "&HFFFFFF", outline_colour: "&H000000",
            back_colour: "&H00000000", outline: 2, shadow: 1, border_style: 1,
            spacing: 0.4, glow_blur: 0, glow_colour: "", uppercase: false,
            dt_fontcolor: "white", dt_borderw: 3, dt_boxcolor: "",
        },
    }
}

/// Convert SRT timestamp "HH:MM:SS,mmm" to ASS timestamp "H:MM:SS.cc".
fn srt_time_to_ass(srt: &str) -> String {
    // SRT: "00:01:23,456" → ASS: "0:01:23.46"
    let s = srt.replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() == 3 {
        let h: u32 = parts[0].parse().unwrap_or(0);
        // ASS uses centiseconds (2 digits), SRT uses milliseconds (3 digits)
        let sec_parts: Vec<&str> = parts[2].split('.').collect();
        let secs = sec_parts[0];
        let ms: u32 = sec_parts.get(1).unwrap_or(&"0").parse().unwrap_or(0);
        let cs = ms / 10; // milliseconds → centiseconds
        format!("{}:{}:{}.{:02}", h, parts[1], secs, cs)
    } else {
        "0:00:00.00".to_string()
    }
}

/// Build the caption filter string from clip settings.
/// Returns None if captions are disabled or empty.
fn build_caption_filter(clip: &db::ClipRow, target_width: i32, target_height: i32) -> Option<String> {
    if clip.captions_enabled != 1 {
        return None;
    }
    let text = clip.captions_text.as_ref()?;
    if text.is_empty() {
        return None;
    }

    let style = get_sub_style(&clip.caption_style);
    let is_srt = text.contains("-->") && text.lines().count() > 2;

    // MarginV = distance from the BOTTOM edge for Alignment=2 (bottom-center).
    // Bottom position: ~18% from bottom clears YouTube Shorts UI (likes/comments)
    // and regular player controls (progress bar).  Target: text baseline at ~82% height.
    let margin_v = match clip.captions_position.as_str() {
        "top" => target_height - (target_height * 18 / 100),
        "center" => target_height / 2 - 30,
        _ => target_height * 18 / 100, // ~346px on 1920-tall → bottom of text at 82%
    };

    if is_srt {
        // ── Convert SRT → ASS with explicit PlayRes ──
        // Writing a full ASS file with PlayResX/PlayResY matching the export
        // resolution gives us pixel-accurate FontSize control.  The default
        // SRT→ASS path in libass uses an unpredictable internal PlayRes which
        // causes wild font-size scaling.

        // ASS Bold field: -1 = bold (≥700), 0 = normal
        let bold_flag: i32 = if style.font_weight >= 700 { -1 } else { 0 };

        let has_glow = !style.glow_colour.is_empty();

        // ASS header — PlayRes matches export resolution so FontSize = pixels
        let mut ass = format!("\
[Script Info]\r\n\
ScriptType: v4.00+\r\n\
PlayResX: {tw}\r\n\
PlayResY: {th}\r\n\
WrapStyle: 0\r\n\
ScaledBorderAndShadow: yes\r\n\
\r\n\
[V4+ Styles]\r\n\
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\r\n\
Style: Default,{fn_},{fs},&H00{pc},&H00FFFFFF,&H00{oc},{bc},{bold},0,0,0,100,100,{sp:.1},0,{bs},{ol},{sh},2,10,10,{mv},1\r\n",
            tw = target_width,
            th = target_height,
            fn_ = style.font_name,
            fs = style.font_size,
            pc = &style.primary_colour[2..],  // strip "&H" prefix — ASS V4+ uses &HAABBGGRR
            oc = &style.outline_colour[2..],
            bc = style.back_colour,
            bold = bold_flag,
            sp = style.spacing,
            bs = style.border_style,
            ol = style.outline,
            sh = style.shadow,
            mv = margin_v,
        );

        // Optional glow layer style: creates a luminous halo behind the crisp text.
        // - PrimaryColour: fully opaque glow colour (bright centre)
        // - OutlineColour: semi-transparent glow colour (fading edges)
        // - Large outline (8px) provides the glow spread area
        // - The \blur override in each Dialogue line gaussian-blurs everything
        if has_glow {
            // Fully opaque version of glow colour (replace alpha byte with 00)
            let glow_opaque = format!("&H00{}", &style.glow_colour[4..]);
            ass.push_str(&format!("\
Style: Glow,{fn_},{fs},{go},{go},{gc},&H00000000,{bold},0,0,0,100,100,{sp:.1},0,1,8,0,2,10,10,{mv},1\r\n",
                fn_ = style.font_name,
                fs = style.font_size,
                go = glow_opaque,      // fully opaque green for primary/secondary
                gc = style.glow_colour, // semi-transparent green for outline
                bold = bold_flag,
                sp = style.spacing,
                mv = margin_v,
            ));
        }

        ass.push_str("\r\n\
[Events]\r\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\n");

        // Parse SRT cues and append as Dialogue lines
        // SRT format: index\n HH:MM:SS,mmm --> HH:MM:SS,mmm \n text \n\n
        let blocks: Vec<&str> = text.split("\n\n").filter(|b| !b.trim().is_empty()).collect();
        for block in &blocks {
            let lines: Vec<&str> = block.lines().collect();
            // Find the timing line (contains "-->")
            let timing_idx = lines.iter().position(|l| l.contains("-->"));
            if let Some(ti) = timing_idx {
                let timing = lines[ti];
                let parts: Vec<&str> = timing.split("-->").collect();
                if parts.len() == 2 {
                    let start_ass = srt_time_to_ass(parts[0].trim());
                    let end_ass = srt_time_to_ass(parts[1].trim());
                    // Remaining lines after timing are the subtitle text
                    let sub_text: String = lines[ti + 1..].iter()
                        .map(|l| l.trim())
                        .filter(|l| !l.is_empty())
                        .collect::<Vec<_>>()
                        .join("\\N"); // ASS line break
                    let sub_text = if style.uppercase { sub_text.to_uppercase() } else { sub_text };

                    // \b<weight> override for precise font weight (e.g. \b800 for extra-bold)
                    let weight_tag = format!("\\b{}", style.font_weight);

                    // If glow style exists, emit a blurred glow layer on Layer 0
                    if has_glow {
                        ass.push_str(&format!(
                            "Dialogue: 0,{},{},Glow,,0,0,0,,{{{wt}\\blur{blur}}}{txt}\r\n",
                            start_ass, end_ass,
                            wt = weight_tag, blur = style.glow_blur, txt = sub_text
                        ));
                    }
                    // Crisp foreground text on Layer 1 (renders on top of glow)
                    ass.push_str(&format!(
                        "Dialogue: 1,{},{},Default,,0,0,0,,{{{wt}}}{txt}\r\n",
                        start_ass, end_ass, wt = weight_tag, txt = sub_text
                    ));
                }
            }
        }

        let ass_temp = std::env::temp_dir().join(format!("clip_{}.ass", clip.id));
        if let Err(e) = std::fs::write(&ass_temp, &ass) {
            log::warn!("Failed to write temp ASS for subtitles filter: {}", e);
            return None;
        }
        let ass_path = ass_temp.to_string_lossy().to_string()
            .replace('\\', "/")
            .replace(':', "\\:");

        // Use the ass filter (not subtitles) to avoid any SRT re-parsing
        Some(format!("ass='{}'", ass_path))
    } else {
        let display_text = if style.uppercase { text.to_uppercase() } else { text.clone() };
        let esc = display_text
            .replace('\\', "\\\\")
            .replace('\'', "'\\''")
            .replace(':', "\\:")
            .replace('%', "%%")
            .replace('[', "\\[")
            .replace(']', "\\]")
            .replace(';', "\\;");
        let ypos = match clip.captions_position.as_str() {
            "top" => "h*0.08",
            "center" => "(h-text_h)/2",
            _ => "h*0.78",  // ~78% → clears YouTube Shorts/player UI at bottom
        };

        let mut filter = format!(
            "drawtext=text='{text}':fontsize={fs}:fontcolor={fc}:borderw={bw}:bordercolor=black:x=(w-text_w)/2:y={y}",
            text = esc, fs = style.font_size, fc = style.dt_fontcolor, bw = style.dt_borderw, y = ypos,
        );
        if !style.dt_boxcolor.is_empty() {
            filter.push_str(&format!(":box=1:boxcolor={}:boxborderw=8", style.dt_boxcolor));
        }
        Some(filter)
    }
}

/// Save editor settings for a clip.
#[tauri::command]
fn update_clip_settings(
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

/// Get a single clip's details by ID.
#[tauri::command]
fn get_clip_detail(clip_id: String, db: State<'_, DbConn>) -> Result<db::ClipRow, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::get_clip_by_id(&conn, &clip_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or_else(|| "Clip not found".to_string())
}

/// Generate TikTok-style post captions on demand from a clip's highlight data.
///
/// If a Claude API key is configured, uses the LLM for fresh generation.
/// Otherwise falls back to the pattern-based system.
#[tauri::command]
async fn generate_post_captions(
    clip_id: String,
    seed: Option<u32>,
    transcript_text: Option<String>,
    current_title: Option<String>,
    current_game: Option<String>,
    selected_mode: Option<String>,
    db: State<'_, DbConn>,
) -> Result<post_captions::PostCaptions, String> {
    let (clip, tags, transcript, highlight_scores, resolved) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;

        let clip = db::get_clip_by_id(&conn, &clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("Clip not found")?;

        let highlights = db::get_highlights_by_vod(&conn, &clip.vod_id)
            .map_err(|e| format!("DB error: {}", e))?;
        let highlight = highlights.iter().find(|h| h.id == clip.highlight_id);

        let tags: Vec<String> = highlight
            .and_then(|h| h.tags.as_ref())
            .map(|t| t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();

        // Prefer full subtitle transcript from frontend; fall back to highlight snippet
        let transcript = transcript_text
            .filter(|t| !t.trim().is_empty())
            .or_else(|| highlight.and_then(|h| h.transcript_snippet.clone()));
        let scores = (
            highlight.map(|h| h.audio_score).unwrap_or(0.0),
            highlight.map(|h| h.visual_score).unwrap_or(0.0),
            highlight.map(|h| h.chat_score).unwrap_or(0.0),
        );

        // Resolve provider for captions scope
        let resolved = ai_provider::resolve(&conn, ai_provider::Scope::Captions);

        (clip, tags, transcript, scores, resolved)
    };

    // Use frontend title if provided, otherwise fall back to clip title
    let title = current_title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| clip.title.clone());

    let (audio, visual, chat) = highlight_scores;

    // Default to direct_quote if no mode specified
    let mode = selected_mode.unwrap_or_else(|| "direct_quote".into());

    // ── Try LLM generation if provider is configured ──
    if resolved.is_llm() {
        let tone = post_captions::classify_tone_pub(
            &tags, transcript.as_deref(), audio, visual, chat,
        );
        let event = post_captions::primary_event_pub(&tags);
        let event_summary = post_captions::synthesize_event_pub(
            event, tone, &tags, seed.unwrap_or(0) as usize,
        );
        let tone_label = tone.label();
        let quote = post_captions::strong_quote_pub(transcript.as_deref());

        // Prefer live game value from frontend; fall back to DB value
        let game_name = current_game.as_deref()
            .filter(|s| !s.is_empty())
            .or(clip.game.as_deref());

        log::info!("Caption generation: using {:?} (model: {})", resolved.provider, resolved.model);
        log::info!("Caption generation: mode = {}, game = {:?}", mode, game_name);

        match post_captions::generate_llm(
            &resolved.api_key, &resolved.model, &mode,
            &event_summary, quote.as_deref(), tone_label,
            &tags, transcript.as_deref(), &title, game_name,
        ).await {
            Ok(llm_captions) => {
                log::info!("LLM generated {} caption(s) for clip {} (mode: {})", llm_captions.len(), clip_id, mode);
                let hashtags = post_captions::build_hashtags_pub(&tags, tone);
                let casual = llm_captions.first().map(|c| c.text.clone()).unwrap_or_default();
                let funny  = llm_captions.get(1).map(|c| c.text.clone()).unwrap_or_default();
                let hype   = llm_captions.get(2).map(|c| c.text.clone()).unwrap_or_default();
                return Ok(post_captions::PostCaptions {
                    captions: llm_captions,
                    hashtags,
                    source: "llm".into(),
                    casual, funny, hype,
                });
            }
            Err(e) => {
                log::warn!("LLM caption generation failed: {}", e);
                if !resolved.fallback_to_free {
                    return Err(format!("Caption generation failed: {}", e));
                }
                log::info!("Falling back to Free mode (pattern-based)");
            }
        }
    }

    // ── Fallback: pattern-based generation ──
    Ok(post_captions::generate_from_parts(
        &tags,
        transcript.as_deref(),
        &title,
        clip.start_seconds,
        audio, visual, chat,
        seed.unwrap_or(0),
    ))
}

/// Generate an AI-powered clip title.
///
/// Uses the configured BYOK provider (Titles scope) to generate a short,
/// punchy title for the clip.  Returns the local heuristic title as fallback.
#[tauri::command]
async fn generate_ai_title(
    clip_id: String,
    transcript_text: Option<String>,
    current_game: Option<String>,
    db: State<'_, DbConn>,
) -> Result<String, String> {
    let (clip, tags, transcript, highlight_scores, resolved) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;

        let clip = db::get_clip_by_id(&conn, &clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("Clip not found")?;

        let highlights = db::get_highlights_by_vod(&conn, &clip.vod_id)
            .map_err(|e| format!("DB error: {}", e))?;
        let highlight = highlights.iter().find(|h| h.id == clip.highlight_id);

        let tags: Vec<String> = highlight
            .and_then(|h| h.tags.as_ref())
            .map(|t| t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();

        let transcript = transcript_text
            .filter(|t| !t.trim().is_empty())
            .or_else(|| highlight.and_then(|h| h.transcript_snippet.clone()));
        let scores = (
            highlight.map(|h| h.audio_score).unwrap_or(0.0),
            highlight.map(|h| h.visual_score).unwrap_or(0.0),
            highlight.map(|h| h.chat_score).unwrap_or(0.0),
        );

        let resolved = ai_provider::resolve(&conn, ai_provider::Scope::Titles);

        (clip, tags, transcript, scores, resolved)
    };

    let (audio, visual, chat) = highlight_scores;

    if resolved.is_llm() {
        let tone = post_captions::classify_tone_pub(
            &tags, transcript.as_deref(), audio, visual, chat,
        );
        let event = post_captions::primary_event_pub(&tags);
        let event_summary = post_captions::synthesize_event_pub(
            event, tone, &tags, 0,
        );

        let game_name = current_game.as_deref()
            .filter(|s| !s.is_empty())
            .or(clip.game.as_deref());

        log::info!("AI title generation: using {:?} (model: {})", resolved.provider, resolved.model);

        match post_captions::generate_llm_title(
            &resolved.api_key, &resolved.model,
            &event_summary, transcript.as_deref(),
            &tags, game_name,
        ).await {
            Ok(title) => {
                log::info!("AI generated title for clip {}: {}", clip_id, title);
                return Ok(title);
            }
            Err(e) => {
                log::warn!("AI title generation failed: {}", e);
                if !resolved.fallback_to_free {
                    return Err(format!("Title generation failed: {}", e));
                }
            }
        }
    }

    // Fallback: return the existing clip title
    Ok(clip.title)
}

/// Test an AI provider connection with a minimal API call.
/// Returns a status string: "connected", or an error description.
#[tauri::command]
async fn test_ai_connection(
    provider: String,
    api_key: String,
    model: String,
) -> Result<String, String> {
    if api_key.is_empty() {
        return Err("No API key provided".into());
    }

    let client = reqwest::Client::new();

    match provider.as_str() {
        "claude" => {
            let body = serde_json::json!({
                "model": model,
                "max_tokens": 5,
                "messages": [{"role": "user", "content": "Say ok"}]
            });
            let resp = client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Connection failed: {}", e))?;

            match resp.status().as_u16() {
                200 => Ok("connected".into()),
                401 => Err("Invalid API key".into()),
                403 => Err("API key lacks permission".into()),
                404 => Err(format!("Model '{}' not available", model)),
                429 => Err("Rate limited — try again in a moment".into()),
                s   => {
                    let body = resp.text().await.unwrap_or_default();
                    Err(format!("HTTP {}: {}", s, &body[..body.len().min(100)]))
                }
            }
        }

        "openai" => {
            let body = serde_json::json!({
                "model": model,
                "max_tokens": 5,
                "messages": [{"role": "user", "content": "Say ok"}]
            });
            let resp = client
                .post("https://api.openai.com/v1/chat/completions")
                .header("Authorization", format!("Bearer {}", api_key))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Connection failed: {}", e))?;

            match resp.status().as_u16() {
                200 => Ok("connected".into()),
                401 => Err("Invalid API key".into()),
                403 => Err("API key lacks permission".into()),
                404 => Err(format!("Model '{}' not available", model)),
                429 => Err("Rate limited — try again in a moment".into()),
                s   => {
                    let body = resp.text().await.unwrap_or_default();
                    Err(format!("HTTP {}: {}", s, &body[..body.len().min(100)]))
                }
            }
        }

        "gemini" => {
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
                model
            );
            let body = serde_json::json!({
                "contents": [{"parts": [{"text": "Say ok"}]}],
                "generationConfig": {"maxOutputTokens": 5}
            });
            let resp = client
                .post(&url)
                .header("x-goog-api-key", api_key)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Connection failed: {}", e))?;

            match resp.status().as_u16() {
                200 => Ok("connected".into()),
                400 => Err("Invalid request — check API key".into()),
                403 => Err("API key invalid or lacks permission".into()),
                404 => Err(format!("Model '{}' not available", model)),
                429 => Err("Rate limited — try again in a moment".into()),
                s   => {
                    let body = resp.text().await.unwrap_or_default();
                    Err(format!("HTTP {}: {}", s, &body[..body.len().min(100)]))
                }
            }
        }

        _ => Err(format!("Unknown provider: {}", provider)),
    }
}

// ── Other commands ──

/// Open a VOD URL on Twitch in the browser.
#[tauri::command]
async fn open_vod(vod_id: String, app: AppHandle, db: State<'_, DbConn>) -> Result<(), String> {
    let vod = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::get_vod_by_id(&conn, &vod_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or_else(|| "VOD not found".to_string())?
    };

    // Validate URL before opening to prevent arbitrary URL injection
    if !vod.vod_url.starts_with("https://www.twitch.tv/")
        && !vod.vod_url.starts_with("https://twitch.tv/")
    {
        return Err(format!("Refusing to open non-Twitch URL: {}", vod.vod_url));
    }

    app.opener()
        .open_url(&vod.vod_url, None::<&str>)
        .map_err(|e| format!("Failed to open browser: {}", e))?;

    Ok(())
}

#[tauri::command]
fn get_channels(db: State<'_, DbConn>) -> Result<Vec<db::ChannelRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_all_channels(&conn).map_err(|e| format!("DB error: {}", e))
}

/// Try to refresh the Twitch user access token using the stored refresh token.
/// On success, saves the new tokens to the DB and returns the new access token.
async fn try_refresh_twitch_token(db: &std::sync::Mutex<rusqlite::Connection>) -> Result<String, String> {
    let refresh_token = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::get_setting(&conn, "twitch_refresh_token")
            .map_err(|e| format!("DB error: {}", e))?
            .unwrap_or_default()
    };

    if refresh_token.is_empty() {
        return Err("No refresh token available. Please log out and log in again.".into());
    }

    let token_resp = twitch::refresh_access_token(&refresh_token).await?;

    // Save the new tokens
    {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::save_setting(&conn, "twitch_user_access_token", &token_resp.access_token)
            .map_err(|e| format!("DB error: {}", e))?;
        if let Some(ref rt) = token_resp.refresh_token {
            db::save_setting(&conn, "twitch_refresh_token", rt)
                .map_err(|e| format!("DB error: {}", e))?;
        }
    }

    Ok(token_resp.access_token)
}

#[tauri::command]
async fn get_vods(
    channel_id: String,
    db: State<'_, DbConn>,
) -> Result<Vec<db::VodRow>, String> {
    let (twitch_user_id, mut access_token) = {
        let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
        let channels = db::get_all_channels(&conn).map_err(|e| format!("DB error: {}", e))?;
        let channel = channels
            .iter()
            .find(|c| c.id == channel_id)
            .ok_or_else(|| "Channel not found".to_string())?
            .clone();
        let token = db::get_setting(&conn, "twitch_user_access_token")
            .map_err(|e| format!("DB error: {}", e))?
            .unwrap_or_default();
        (channel.twitch_user_id, token)
    };

    if access_token.is_empty() {
        return Err("Not logged in. Please log in with Twitch first.".into());
    }

    // Try fetching VODs; if 401, refresh token and retry
    let videos = match twitch::get_vods(&access_token, &twitch_user_id).await {
        Ok(v) => v,
        Err(e) if e.contains("401") => {
            access_token = try_refresh_twitch_token(&db).await?;
            twitch::get_vods(&access_token, &twitch_user_id).await?
        }
        Err(e) => return Err(e),
    };

    // NOTE: The Twitch /videos endpoint does NOT return game_id/game_name, and the
    // /channels endpoint only returns the CURRENT game (not the game played during a specific VOD).
    // Game detection is handled at the clip level via subtitle keyword inference (detectGame()),
    // or manually by the user via the "Set game" button on VOD cards.

    let vod_rows: Vec<db::VodRow> = videos
        .iter()
        .map(|v| {
            let vod_id = uuid::Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            db::VodRow {
                id: vod_id,
                channel_id: channel_id.clone(),
                twitch_video_id: v.id.clone(),
                title: v.title.clone(),
                duration_seconds: twitch::parse_duration(&v.duration),
                stream_date: v.created_at.clone(),
                thumbnail_url: v.thumbnail_url
                    .replace("%{width}", "640")
                    .replace("%{height}", "360"),
                vod_url: v.url.clone(),
                download_status: "pending".to_string(),
                local_path: None,
                file_size_bytes: None,
                analysis_status: "pending".to_string(),
                created_at: now,
                download_progress: Some(0),
                analysis_progress: 0,
                game_name: None,
            }
        })
        .collect();

    {
        let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
        for vod in &vod_rows {
            db::upsert_vod(&conn, vod).map_err(|e| format!("DB error: {}", e))?;
        }
    }

    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_vods_by_channel(&conn, &channel_id).map_err(|e| format!("DB error: {}", e))
}

#[tauri::command]
fn get_highlights(
    vod_id: String,
    db: State<'_, DbConn>,
) -> Result<Vec<db::HighlightRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_highlights_by_vod(&conn, &vod_id).map_err(|e| format!("DB error: {}", e))
}

#[tauri::command]
fn get_all_highlights(db: State<'_, DbConn>) -> Result<Vec<db::HighlightRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_all_highlights(&conn).map_err(|e| format!("DB error: {}", e))
}

#[tauri::command]
fn get_clips(db: State<'_, DbConn>) -> Result<Vec<db::ClipRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_all_clips(&conn).map_err(|e| format!("DB error: {}", e))
}

#[tauri::command]
fn delete_clip(clip_id: String, db: State<'_, DbConn>) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;

    // Get the vod_id before deleting so we can check remaining clips
    let vod_id: Option<String> = conn.query_row(
        "SELECT vod_id FROM clips WHERE id = ?1", rusqlite::params![clip_id],
        |row| row.get(0),
    ).ok();

    db::delete_clip(&conn, &clip_id).map_err(|e| format!("DB error: {}", e))?;

    // If no clips remain for this VOD, reset analysis_status so user can re-analyze
    if let Some(vid) = vod_id {
        let remaining: i64 = conn.query_row(
            "SELECT COUNT(*) FROM clips WHERE vod_id = ?1", rusqlite::params![vid],
            |row| row.get(0),
        ).unwrap_or(0);

        if remaining == 0 {
            db::update_vod_analysis_status(&conn, &vid, "pending")
                .map_err(|e| format!("DB error: {}", e))?;
            log::info!("All clips deleted for VOD {} — reset analysis_status to pending", vid);
        }
    }

    Ok(())
}

/// Settings keys the frontend is allowed to read/write.
/// Secrets (tokens, API keys) are accessed only through dedicated commands.
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
];

#[tauri::command]
fn save_setting(
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
fn get_setting(
    key: String,
    db: State<'_, DbConn>,
) -> Result<Option<String>, String> {
    if !ALLOWED_SETTING_KEYS.contains(&key.as_str()) {
        return Err(format!("Setting '{}' is not readable from the frontend", key));
    }
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_setting(&conn, &key).map_err(|e| format!("DB error: {}", e))
}

/// Open a URL in the user's default system browser (with their logged-in profile).
/// Uses explorer.exe on Windows to avoid session/profile issues that cmd /c start causes.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
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
fn get_app_info() -> Result<AppInfo, String> {
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
fn get_hardware_info(hw: State<'_, HardwareInfo>) -> Result<HardwareInfo, String> {
    Ok(hw.inner().clone())
}

/// Return a snapshot of all background jobs.
#[tauri::command]
fn list_jobs(queue: State<'_, JobQueue>) -> Vec<Job> {
    queue.list()
}

/// Return a single job's current state.
#[tauri::command]
fn get_job(id: String, queue: State<'_, JobQueue>) -> Option<Job> {
    queue.get(&id)
}

/// Remove a finished job from the queue.
#[tauri::command]
fn remove_job(id: String, queue: State<'_, JobQueue>) -> bool {
    queue.remove(&id)
}

/// Open a folder picker dialog and save the selected path as the download directory.
#[tauri::command]
fn pick_download_folder(app: AppHandle, db: State<'_, DbConn>) -> Result<Option<String>, String> {
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

/// Save an already-exported clip to the configured download folder.
///
/// If a download folder is set in Settings, saves directly with no dialog.
/// If no folder is configured, opens a folder picker, saves the selection to
/// Settings for future use, then saves the file.
/// Returns the saved file path, or None if the user cancelled the picker.
#[tauri::command]
fn save_clip_to_disk(
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

/// Refresh VOD metadata from Twitch API (title, thumbnail, game) without re-downloading.
/// Also backfills game_name to existing clips that don't have one.
#[tauri::command]
async fn refresh_vod_metadata(
    vod_id: String,
    db: State<'_, DbConn>,
) -> Result<db::VodRow, String> {
    let (twitch_video_id, mut access_token) = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        let vod = db::get_vod_by_id(&conn, &vod_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("VOD not found")?;
        let token = db::get_setting(&conn, "twitch_user_access_token")
            .map_err(|e| format!("DB error: {}", e))?
            .unwrap_or_default();
        (vod.twitch_video_id, token)
    };

    if access_token.is_empty() {
        return Err("Not logged in. Please log in with Twitch first.".into());
    }

    // Fetch fresh video data from Twitch — retry with refreshed token on 401
    let client = reqwest::Client::new();
    let url = format!("https://api.twitch.tv/helix/videos?id={}", twitch_video_id);
    let resp = client
        .get(&url)
        .header("Client-Id", twitch::client_id())
        .header("Authorization", format!("Bearer {}", &access_token))
        .send()
        .await
        .map_err(|e| format!("Twitch API error: {}", e))?;

    let resp = if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        // Token expired — try refreshing
        access_token = try_refresh_twitch_token(&db).await?;
        client
            .get(&url)
            .header("Client-Id", twitch::client_id())
            .header("Authorization", format!("Bearer {}", &access_token))
            .send()
            .await
            .map_err(|e| format!("Twitch API error: {}", e))?
    } else {
        resp
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Twitch API {}: {}", status, body));
    }

    let resp_json: serde_json::Value = resp.json().await
        .map_err(|e| format!("Parse error: {}", e))?;

    let video = resp_json["data"].as_array()
        .and_then(|arr| arr.first())
        .ok_or("Video not found on Twitch")?;

    let title = video["title"].as_str().unwrap_or("").to_string();
    let thumbnail_url = video["thumbnail_url"].as_str().unwrap_or("")
        .replace("%{width}", "640")
        .replace("%{height}", "360");

    // Update VOD title and thumbnail in database.
    // Preserve game_name — it's user-set and should not be cleared on metadata refresh.
    {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        conn.execute(
            "UPDATE vods SET title = ?1, thumbnail_url = ?2 WHERE id = ?3",
            rusqlite::params![title, thumbnail_url, vod_id],
        ).map_err(|e| format!("DB error: {}", e))?;

        log::info!("[refresh_vod_metadata] Updated title/thumbnail for VOD {}", vod_id);
    }

    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::get_vod_by_id(&conn, &vod_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or_else(|| "VOD not found after update".to_string())
}

/// Set the game on a single clip (lightweight — used for auto-save after subtitle inference).
#[tauri::command]
fn set_clip_game(clip_id: String, game: Option<String>, db: State<'_, DbConn>) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let g = game.as_deref().filter(|s| !s.is_empty());
    log::info!("[set_clip_game] Setting clip {} game to: {:?}", clip_id, g);
    db::update_clip_game(&conn, &clip_id, g)
        .map_err(|e| format!("DB error: {}", e))
}

/// Set the title on a single clip (lightweight — used for auto-save on blur).
#[tauri::command]
fn set_clip_title(clip_id: String, title: Option<String>, db: State<'_, DbConn>) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let t = title.as_deref().filter(|s| !s.is_empty());
    log::info!("[set_clip_title] Setting clip {} title to: {:?}", clip_id, t);
    db::update_clip_title(&conn, &clip_id, t)
        .map_err(|e| format!("DB error: {}", e))
}

/// Save publish description and hashtags on a clip (auto-save on blur / after generation).
#[tauri::command]
fn set_clip_publish_meta(
    clip_id: String,
    description: Option<String>,
    hashtags: Option<String>,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let desc = description.as_deref().filter(|s| !s.is_empty());
    let tags = hashtags.as_deref().filter(|s| !s.is_empty());
    log::info!("[set_clip_publish_meta] clip {} desc_len={:?} tags={:?}", clip_id, desc.map(|d| d.len()), tags);
    if let Some(d) = desc {
        println!("[CLIPGOBLIN DEBUG] Publish description saved: \"{}\"", d);
    }
    db::update_clip_publish_meta(&conn, &clip_id, desc, tags)
        .map_err(|e| format!("DB error: {}", e))
}

/// Manually set the game name on a VOD and propagate to all its clips.
/// Used as a manual fallback when auto-detection doesn't work.
#[tauri::command]
fn set_vod_game(vod_id: String, game_name: Option<String>, db: State<'_, DbConn>) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let gn = game_name.as_deref().filter(|s| !s.is_empty());
    log::info!("[set_vod_game] Setting VOD {} game to: {:?}", vod_id, gn);
    db::update_vod_game_name(&conn, &vod_id, gn)
        .map_err(|e| format!("DB error: {}", e))?;
    // Propagate to all clips from this VOD (overwrite all, since user explicitly set it)
    if let Some(name) = gn {
        conn.execute(
            "UPDATE clips SET game = ?1 WHERE vod_id = ?2",
            rusqlite::params![name, vod_id],
        ).map_err(|e| format!("DB error: {}", e))?;
        log::info!("[set_vod_game] Propagated game to all clips for VOD {}", vod_id);
    } else {
        conn.execute(
            "UPDATE clips SET game = NULL WHERE vod_id = ?1",
            rusqlite::params![vod_id],
        ).map_err(|e| format!("DB error: {}", e))?;
    }
    Ok(())
}

/// Delete a VOD's video file only (keeps clips and metadata).
/// Returns how many bytes were freed.
#[tauri::command]
fn delete_vod_file(vod_id: String, db: State<'_, DbConn>) -> Result<u64, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let vod = db::get_vod_by_id(&conn, &vod_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or("VOD not found")?;

    let mut freed: u64 = 0;
    if let Some(ref path) = vod.local_path {
        let p = std::path::Path::new(path);
        if p.exists() {
            freed = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            std::fs::remove_file(p).map_err(|e| format!("Failed to delete file: {}", e))?;
        }
    }

    // Update VOD status back to pending
    conn.execute(
        "UPDATE vods SET download_status = 'pending', local_path = NULL, file_size_bytes = NULL, download_progress = 0 WHERE id = ?1",
        rusqlite::params![vod_id],
    ).map_err(|e| format!("DB error: {}", e))?;

    Ok(freed)
}

/// Delete a VOD and ALL its associated clips, highlights, and files.
/// Returns how many bytes were freed.
#[tauri::command]
fn delete_vod_and_clips(vod_id: String, db: State<'_, DbConn>) -> Result<u64, String> {
    println!("[delete_vod_and_clips] START vod_id={}", vod_id);
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let vod = db::get_vod_by_id(&conn, &vod_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or("VOD not found")?;
    println!("[delete_vod_and_clips] Found VOD: twitch_video_id={}", vod.twitch_video_id);

    let mut freed: u64 = 0;

    // Delete VOD video file
    if let Some(ref path) = vod.local_path {
        let p = std::path::Path::new(path);
        if p.exists() {
            freed += std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            std::fs::remove_file(p).ok();
        }
    }

    // Delete exported clip files
    let clips = db::get_clips_by_vod(&conn, &vod_id).unwrap_or_default();
    for clip in &clips {
        if let Some(ref path) = clip.output_path {
            let p = std::path::Path::new(path);
            if p.exists() {
                freed += std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
                std::fs::remove_file(p).ok();
            }
        }
        if let Some(ref path) = clip.thumbnail_path {
            let p = std::path::Path::new(path);
            if p.exists() {
                freed += std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
                std::fs::remove_file(p).ok();
            }
        }
    }

    // Delete DB records: clips, highlights, then vod
    db::delete_clips_for_vod(&conn, &vod_id).ok();
    conn.execute(
        "DELETE FROM highlights WHERE vod_id = ?1",
        rusqlite::params![vod_id],
    ).ok();
    db::delete_vod(&conn, &vod_id)
        .map_err(|e| format!("DB error deleting vod: {}", e))?;

    // Verify the VOD is gone and the twitch_video_id is in deleted_vods
    let still_exists = db::get_vod_by_id(&conn, &vod_id).ok().flatten().is_some();
    println!("[delete_vod_and_clips] DONE vod_id={} freed={} still_in_db={}", vod_id, freed, still_exists);

    Ok(freed)
}

/// Get VOD disk usage info (for delete confirmation dialog).
#[tauri::command]
fn get_vod_disk_usage(vod_id: String, db: State<'_, DbConn>) -> Result<serde_json::Value, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let vod = db::get_vod_by_id(&conn, &vod_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or("VOD not found")?;

    let vod_size: u64 = vod.local_path.as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .unwrap_or(0);

    let clips = db::get_clips_by_vod(&conn, &vod_id).unwrap_or_default();
    let clip_count = clips.len();
    let mut clips_size: u64 = 0;
    for clip in &clips {
        if let Some(ref p) = clip.output_path {
            clips_size += std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        }
        if let Some(ref p) = clip.thumbnail_path {
            clips_size += std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        }
    }

    Ok(serde_json::json!({
        "vod_size": vod_size,
        "clip_count": clip_count,
        "clips_size": clips_size,
        "total_size": vod_size + clips_size,
        "has_file": vod.local_path.is_some(),
    }))
}

/// Get a single VOD's details by ID.
#[tauri::command]
fn get_vod_detail(vod_id: String, db: State<'_, DbConn>) -> Result<db::VodRow, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::get_vod_by_id(&conn, &vod_id)
        .map_err(|e| format!("DB error: {}", e))?
        .ok_or_else(|| "VOD not found".to_string())
}

/// Set a VOD's analysis status (used by frontend to mark stale analyses as failed).
#[tauri::command]
fn set_vod_analysis_status(vod_id: String, status: String, db: State<'_, DbConn>) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::update_vod_analysis_status(&conn, &vod_id, &status)
        .map_err(|e| format!("DB error: {}", e))
}

/// Get the current download directory (from settings or default).
#[tauri::command]
fn get_download_dir(db: State<'_, DbConn>) -> Result<String, String> {
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

// ── Storage location commands ──

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoragePaths {
    exports_dir: String,
    downloads_dir: String,
    data_dir: String,
}

/// Return the three key storage directories, creating them if needed.
#[tauri::command]
fn get_storage_paths(db: State<'_, DbConn>) -> Result<StoragePaths, String> {
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
fn open_folder(path: String) -> Result<(), String> {
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

// ── Detection stats command ──

/// Get detection stats for a VOD (stored after analysis completes).
#[tauri::command]
fn get_detection_stats(vod_id: String, db: State<'_, DbConn>) -> Result<Option<serde_json::Value>, String> {
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

// ── Performance tracking commands ──

#[tauri::command]
fn save_clip_performance(
    clip_id: String,
    platform: String,
    views: i64,
    likes: i64,
    comments: i64,
    shares: i64,
    retention_rate: f64,
    first_3s_hold_rate: f64,
    completion_rate: f64,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::insert_clip_performance(
        &conn, &clip_id, &platform, views, likes, comments, shares,
        retention_rate, first_3s_hold_rate, completion_rate,
    ).map_err(|e| format!("DB error: {}", e))
}

#[tauri::command]
fn get_clip_performance(clip_id: String, db: State<'_, DbConn>) -> Result<Vec<db::ClipPerformanceRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::get_clip_performance(&conn, &clip_id).map_err(|e| format!("DB error: {}", e))
}

#[tauri::command]
fn get_creator_profile(db: State<'_, DbConn>) -> Result<db::CreatorProfileRow, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    db::get_or_create_creator_profile(&conn).map_err(|e| format!("DB error: {}", e))
}

/// Recalculate creator scoring weights based on actual clip performance data.
/// This is the feedback loop — learn what works for this creator.
#[tauri::command]
fn update_scoring_from_performance(db: State<'_, DbConn>) -> Result<String, String> {
    let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
    let mut profile = db::get_or_create_creator_profile(&conn)
        .map_err(|e| format!("DB error: {}", e))?;

    // Get all clips with performance data
    let mut stmt = conn.prepare(
        "SELECT c.id, h.virality_score, h.audio_score, h.visual_score, h.chat_score, h.tags,
                p.retention_rate, p.first_3s_hold_rate, p.completion_rate, p.views, p.shares
         FROM clips c
         JOIN highlights h ON h.id = c.highlight_id
         JOIN clip_performance p ON p.clip_id = c.id
         WHERE p.views > 0
         ORDER BY p.retention_rate DESC"
    ).map_err(|e| format!("DB error: {}", e))?;

    let perf_data: Vec<(f64, f64, f64, f64, String, f64, f64, f64, i64, i64)> = stmt.query_map([], |row| {
        Ok((
            row.get::<_, f64>(1)?,  // virality
            row.get::<_, f64>(2)?,  // audio
            row.get::<_, f64>(3)?,  // visual
            row.get::<_, f64>(4)?,  // chat
            row.get::<_, String>(5).unwrap_or_default(),  // tags
            row.get::<_, f64>(6)?,  // retention
            row.get::<_, f64>(7)?,  // 3s hold
            row.get::<_, f64>(8)?,  // completion
            row.get::<_, i64>(9)?,  // views
            row.get::<_, i64>(10)?, // shares
        ))
    }).map_err(|e| format!("DB error: {}", e))?
    .filter_map(|r| r.ok())
    .collect();

    if perf_data.len() < 3 {
        return Ok("Not enough performance data yet (need at least 3 clips with metrics). Keep creating and tracking clips!".to_string());
    }

    // Calculate which clips performed best (top quartile)
    let top_count = (perf_data.len() / 4).max(1);
    let top_clips = &perf_data[..top_count];

    // Analyze what scores the best performers had
    let avg_3s_hold: f64 = top_clips.iter().map(|d| d.6).sum::<f64>() / top_count as f64;

    // Adjust weights: if top clips had high 3s hold rate, increase hook weight
    if avg_3s_hold > 0.7 {
        profile.avg_hook_weight = (profile.avg_hook_weight + 0.02).min(0.40);
        profile.avg_context_weight = (profile.avg_context_weight - 0.01).max(0.05);
    }

    // If top clips had high completion, boost payoff weight
    let avg_completion: f64 = top_clips.iter().map(|d| d.7).sum::<f64>() / top_count as f64;
    if avg_completion > 0.6 {
        profile.avg_payoff_weight = (profile.avg_payoff_weight + 0.02).min(0.30);
        profile.avg_loop_weight = (profile.avg_loop_weight - 0.01).max(0.05);
    }

    // Collect top-performing tags
    let mut tag_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for clip in top_clips {
        for tag in clip.4.split(',') {
            let tag = tag.trim().to_string();
            if !tag.is_empty() {
                *tag_counts.entry(tag).or_insert(0) += 1;
            }
        }
    }
    let mut sorted_tags: Vec<(String, usize)> = tag_counts.into_iter().collect();
    sorted_tags.sort_by(|a, b| b.1.cmp(&a.1));
    let top_tags: Vec<String> = sorted_tags.iter().take(10).map(|(t, _)| t.clone()).collect();
    profile.top_performing_tags = Some(top_tags.join(","));

    profile.total_clips_tracked = perf_data.len() as i64;

    // Normalize weights to sum to 1.0
    let sum = profile.avg_hook_weight + profile.avg_emotional_weight + profile.avg_payoff_weight
        + profile.avg_loop_weight + profile.avg_context_weight;
    profile.avg_hook_weight /= sum;
    profile.avg_emotional_weight /= sum;
    profile.avg_payoff_weight /= sum;
    profile.avg_loop_weight /= sum;
    profile.avg_context_weight /= sum;

    db::update_creator_profile(&conn, &profile)
        .map_err(|e| format!("DB error: {}", e))?;

    Ok(format!(
        "Scoring weights updated from {} clips! Hook: {:.0}%, Emotional: {:.0}%, Payoff: {:.0}%, Loop: {:.0}%, Context: {:.0}%. Top tags: {}",
        perf_data.len(),
        profile.avg_hook_weight * 100.0,
        profile.avg_emotional_weight * 100.0,
        profile.avg_payoff_weight * 100.0,
        profile.avg_loop_weight * 100.0,
        profile.avg_context_weight * 100.0,
        profile.top_performing_tags.as_deref().unwrap_or("none yet"),
    ))
}

/// Get transcript for a VOD (run transcription if not cached)
#[tauri::command]
async fn get_transcript(vod_id: String, db: State<'_, DbConn>, hw: State<'_, HardwareInfo>) -> Result<serde_json::Value, String> {
    let vod = {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::get_vod_by_id(&conn, &vod_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or("VOD not found")?
    };

    let vod_path = vod.local_path.ok_or("VOD not downloaded")?;

    let transcript_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clipviral")
        .join("transcripts");
    std::fs::create_dir_all(&transcript_dir).ok();
    let output_path = transcript_dir.join(format!("{}.json", vod_id));

    // Return cached transcript if it exists
    if output_path.exists() {
        let json_str = std::fs::read_to_string(&output_path)
            .map_err(|e| format!("Read error: {}", e))?;
        let val: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| format!("Parse error: {}", e))?;
        return Ok(val);
    }

    // Run transcription
    let output_str = output_path.to_string_lossy().to_string();
    let vod_path_clone = vod_path.clone();
    let hw_clone = hw.inner().clone();
    let result = tokio::task::spawn_blocking(move || {
        run_transcription(&vod_path_clone, &output_str, &hw_clone, None)
    }).await.map_err(|e| format!("Task error: {}", e))??;

    // Save path to VOD record
    {
        let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
        db::update_vod_transcript_path(&conn, &vod_id, &output_path.to_string_lossy()).ok();
    }

    serde_json::to_value(&result).map_err(|e| format!("Serialize: {}", e))
}

// ── Social publishing commands ──

/// Connect a social platform (YouTube, TikTok, Instagram) via OAuth.
///
/// The `PlatformAdapter` trait is `#[async_trait(?Send)]` (because `rusqlite::Connection`
/// is `!Sync`), so its async methods return `!Send` futures.  Tauri commands need
/// `Send` futures, so we use `block_in_place` + `block_on` to run each `!Send`
/// call synchronously on the current worker thread.
#[tauri::command]
async fn connect_platform(
    platform: String,
    app: AppHandle,
    db: State<'_, DbConn>,
) -> Result<social::ConnectedAccount, String> {
    let adapter = social::get_adapter(&platform).map_err(|e| e.to_string())?;

    // 1. Get auth URL (no DB needed — start_auth just builds a URL string).
    //    Must use block_in_place because the trait future is !Send.
    let auth_url = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(adapter.start_auth())
    })
    .map_err(|e| e.to_string())?;

    // 2. Bind callback server (sync, before opening browser to avoid race)
    //    Each platform listens on its own port.
    let listener = match platform.as_str() {
        "youtube" => social::youtube::bind_callback_server().map_err(|e| e.to_string())?,
        "tiktok" => social::tiktok::bind_callback_server().map_err(|e| e.to_string())?,
        _ => return Err(format!("No callback server for platform: {}", platform)),
    };

    // 3. Open browser
    app.opener()
        .open_url(&auth_url, None::<&str>)
        .map_err(|e| format!("Failed to open browser: {}", e))?;

    // 4. Wait for OAuth callback (blocking — runs on a threadpool thread)
    let plat = platform.clone();
    let code = tokio::task::spawn_blocking(move || {
        match plat.as_str() {
            "youtube" => social::youtube::wait_for_auth_code(listener),
            "tiktok" => social::tiktok::wait_for_auth_code(listener),
            _ => Err(crate::error::AppError::NotSupported(format!("{} callback", plat))),
        }
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
    .map_err(|e| e.to_string())?;

    // 5. Exchange code for tokens + persist to DB.
    //    handle_callback does HTTP first, then writes to DB — all in one !Send future.
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    let account = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(adapter.handle_callback(&conn, &code))
    })
    .map_err(|e| e.to_string())?;

    Ok(account)
}

/// Disconnect a social platform (removes stored tokens/channel info).
#[tauri::command]
fn disconnect_platform(
    platform: String,
    db: State<'_, DbConn>,
) -> Result<(), String> {
    let adapter = social::get_adapter(&platform).map_err(|e| e.to_string())?;
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    adapter.disconnect(&conn).map_err(|e| e.to_string())?;
    Ok(())
}

/// Get the connected account for a specific platform (if any).
#[tauri::command]
fn get_connected_account(
    platform: String,
    db: State<'_, DbConn>,
) -> Result<Option<social::ConnectedAccount>, String> {
    let adapter = social::get_adapter(&platform).map_err(|e| e.to_string())?;
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    adapter.get_account(&conn).map_err(|e| e.to_string())
}

/// Get all connected social accounts across all platforms.
#[tauri::command]
fn get_all_connected_accounts(
    db: State<'_, DbConn>,
) -> Result<Vec<social::ConnectedAccount>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    social::get_all_accounts(&conn).map_err(|e| e.to_string())
}

/// Upload a clip to a social platform.
/// Reads the clip's output_path from DB, validates, then delegates to the adapter.
///
/// Uses `block_in_place` + `block_on` for the `!Send` adapter future (see
/// `connect_platform` for the full explanation of the `?Send` workaround).
#[tauri::command]
async fn upload_to_platform(
    platform: String,
    meta: social::UploadMeta,
    db: State<'_, DbConn>,
) -> Result<social::UploadResult, String> {
    let adapter = social::get_adapter(&platform).map_err(|e| e.to_string())?;

    // Read clip output_path from DB (sync), validate, then drop the lock
    let output_path = {
        let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
        let clip = db::get_clip_by_id(&conn, &meta.clip_id)
            .map_err(|e| format!("DB error: {}", e))?
            .ok_or_else(|| format!("Clip '{}' not found", meta.clip_id))?;
        social::validate_export_file(clip.output_path.as_deref())
            .map_err(|e| e.to_string())?
            .to_string()
    };

    // Upload: adapter.upload_video is async(?Send), needs &Connection for
    // duplicate checks, token refresh, and recording upload history.
    // Must use block_in_place because the trait future is !Send (same
    // pattern as connect_platform — see that command for details).
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    let result = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current()
            .block_on(adapter.upload_video(&conn, &output_path, &meta))
    })
    .map_err(|e| e.to_string())?;

    Ok(result)
}

/// Check if a clip has already been uploaded to a platform.
/// Returns the upload history row if found, None otherwise.
#[tauri::command]
fn get_upload_status(
    clip_id: String,
    platform: String,
    db: State<'_, DbConn>,
) -> Result<Option<db::UploadHistoryRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_upload_for_clip(&conn, &clip_id, &platform)
        .map_err(|e| format!("DB error: {}", e))
}

/// Get ALL upload history entries for a clip (all platforms).
#[tauri::command]
fn get_clip_upload_history(
    clip_id: String,
    db: State<'_, DbConn>,
) -> Result<Vec<db::UploadHistoryRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_uploads_for_clip(&conn, &clip_id)
        .map_err(|e| format!("DB error: {}", e))
}

/// Clear the deleted_vods table so Twitch API re-fetch can re-insert all VODs.
#[tauri::command]
fn restore_deleted_vods(db: State<'_, DbConn>) -> Result<u64, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    let count = conn.execute("DELETE FROM deleted_vods", [])
        .map_err(|e| format!("DB error: {}", e))?;
    println!("[restore_deleted_vods] Cleared {} entries from deleted_vods", count);
    Ok(count as u64)
}

// ── Scheduled upload commands ──

#[tauri::command]
fn schedule_upload(
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
    };
    db::insert_scheduled_upload(&conn, &row).map_err(|e| format!("DB error: {}", e))?;
    Ok(id)
}

#[tauri::command]
fn list_scheduled_uploads(db: State<'_, DbConn>) -> Result<Vec<db::ScheduledUploadRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_all_scheduled_uploads(&conn).map_err(|e| format!("DB error: {}", e))
}

#[tauri::command]
fn get_scheduled_uploads_for_clip(
    clip_id: String,
    db: State<'_, DbConn>,
) -> Result<Vec<db::ScheduledUploadRow>, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::get_scheduled_uploads_for_clip(&conn, &clip_id).map_err(|e| format!("DB error: {}", e))
}

#[tauri::command]
fn cancel_scheduled_upload(id: String, db: State<'_, DbConn>) -> Result<bool, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::cancel_scheduled_upload(&conn, &id).map_err(|e| format!("DB error: {}", e))
}

#[tauri::command]
fn reschedule_upload(id: String, new_time: String, db: State<'_, DbConn>) -> Result<bool, String> {
    let conn = db.lock().map_err(|e| format!("DB lock error: {}", e))?;
    db::reschedule_upload(&conn, &id, &new_time).map_err(|e| format!("DB error: {}", e))
}

// ── Background upload scheduler ──

/// Background scheduler: checks for due scheduled uploads every 60 seconds.
fn start_upload_scheduler(handle: tauri::AppHandle) {
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

fn process_due_uploads(handle: &tauri::AppHandle) -> Result<(), String> {
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

        // Get clip output path
        let output_path = {
            let conn = db.lock().map_err(|e| format!("DB lock: {}", e))?;
            let clip = match db::get_clip_by_id(&conn, &upload.clip_id) {
                Ok(Some(c)) => c,
                _ => {
                    db::update_scheduled_upload_status(&conn, &upload.id, "failed", Some("Clip not found"), None, None).ok();
                    continue;
                }
            };
            match social::validate_export_file(clip.output_path.as_deref()) {
                Ok(p) => p.to_string(),
                Err(e) => {
                    db::update_scheduled_upload_status(&conn, &upload.id, "failed", Some(&format!("Export file: {}", e)), None, None).ok();
                    continue;
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

fn handle_scheduled_failure(
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

// ── App entry point ──

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let conn = db::init_db().expect("Failed to initialize database");

    // Recover any clips that were stuck mid-render when the app last closed
    db::recover_stale_rendering(&conn).ok();

    let hw = hardware::detect_hardware();

    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().level(log::LevelFilter::Info).build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
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
