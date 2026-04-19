//! Tauri commands for checking and downloading the bundled external binaries
//! (ffmpeg, ffprobe, yt-dlp). Called from the first-run setup UI.

use serde::Serialize;
use tauri::{Emitter, Window};

use crate::bin_manager::{self, BinaryStatus, ProgressCb};

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
