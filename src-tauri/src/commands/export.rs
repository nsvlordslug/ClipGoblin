//! Clip export and rendering commands.

use std::process::Stdio;
use tauri::{AppHandle, State};
use crate::db;
use crate::DbConn;
use crate::error::AppError;
use crate::hardware::HardwareInfo;
use crate::job_queue::JobQueue;
use crate::report_error;
use crate::vertical_crop;
use crate::commands::vod::{find_ffmpeg, generate_thumbnail, run_transcription, generate_srt_for_clip, TranscriptResult};

/// Generate captions for a clip by transcribing its audio segment.
#[tauri::command]
pub async fn generate_clip_captions(
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
pub fn set_clip_thumbnail(
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
pub async fn export_clip(
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
            // #FF6B2B — R=FF G=6B B=2B — ASS &H2B6BFF
            primary_colour: "&H2B6BFF", outline_colour: "&H000000",
            back_colour: "&H00000000", outline: 3, shadow: 1, border_style: 1,
            spacing: 1.2, glow_blur: 0, glow_colour: "", uppercase: true,
            dt_fontcolor: "#FF6B2B", dt_borderw: 4, dt_boxcolor: "",
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
pub(crate) fn build_caption_filter(clip: &db::ClipRow, target_width: i32, target_height: i32) -> Option<String> {
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
