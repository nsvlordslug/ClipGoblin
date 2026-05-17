//! Tauri commands for checking and downloading the bundled external binaries
//! (ffmpeg, ffprobe, yt-dlp). Called from the first-run setup UI.

use serde::Serialize;
use tauri::{Emitter, State, Window};

use crate::bin_manager::{self, BinaryStatus, ProgressCb};
use crate::DbConn;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum Phase { Downloading, Extracting, Done }

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    binary: String,
    downloaded: u64,
    total: u64,
    phase: Phase,
}

#[tauri::command]
pub async fn check_binary_status() -> Result<BinaryStatus, String> {
    Ok(bin_manager::check_binaries())
}

#[tauri::command]
pub async fn download_binaries(window: Window) -> Result<(), String> {
    let status = bin_manager::check_binaries();

    if !status.ytdlp_available {
        let w = window.clone();
        let cb: ProgressCb = Box::new(move |d, t| {
            let _ = w.emit("download-progress", Progress {
                binary: "yt-dlp".into(),
                downloaded: d,
                total: t,
                phase: Phase::Downloading,
            });
        });
        bin_manager::download_ytdlp(&cb).await.map_err(|e| e.to_string())?;
        let _ = window.emit("download-progress", Progress {
            binary: "yt-dlp".into(),
            downloaded: 0,
            total: 0,
            phase: Phase::Done,
        });
    }

    if !status.ffmpeg_available || !status.ffprobe_available {
        let w = window.clone();
        let cb: ProgressCb = Box::new(move |d, t| {
            let _ = w.emit("download-progress", Progress {
                binary: "ffmpeg".into(),
                downloaded: d,
                total: t,
                phase: Phase::Downloading,
            });
        });
        bin_manager::download_ffmpeg(&cb).await.map_err(|e| e.to_string())?;
        let _ = window.emit("download-progress", Progress {
            binary: "ffmpeg".into(),
            downloaded: 0,
            total: 0,
            phase: Phase::Extracting,
        });
        let _ = window.emit("download-progress", Progress {
            binary: "ffmpeg".into(),
            downloaded: 0,
            total: 0,
            phase: Phase::Done,
        });
    }

    Ok(())
}

/// Force-refresh the bundled yt-dlp (bypasses the staleness gate). Used by
/// the failed-VOD-card "Update yt-dlp & Retry" action. Emits the same
/// `download-progress` events as `download_binaries` so the existing
/// progress UI can be reused. Records the refresh timestamp on success.
#[tauri::command]
pub async fn force_refresh_ytdlp(window: Window, db: State<'_, DbConn>) -> Result<(), String> {
    let w = window.clone();
    let cb: ProgressCb = Box::new(move |d, t| {
        let _ = w.emit("download-progress", Progress {
            binary: "yt-dlp".into(),
            downloaded: d,
            total: t,
            phase: Phase::Downloading,
        });
    });
    bin_manager::force_refresh_ytdlp(&cb).await.map_err(|e| e.to_string())?;
    let _ = window.emit("download-progress", Progress {
        binary: "yt-dlp".into(),
        downloaded: 0,
        total: 0,
        phase: Phase::Done,
    });
    let now = chrono::Utc::now().to_rfc3339();
    if let Ok(conn) = db.lock() {
        let _ = crate::db::save_setting(&conn, bin_manager::YTDLP_LAST_REFRESH_KEY, &now);
    }
    Ok(())
}
