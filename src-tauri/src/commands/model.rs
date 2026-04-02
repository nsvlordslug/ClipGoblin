//! Whisper model management — check status, download, delete.

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{Emitter, Window};
use tokio::io::AsyncWriteExt;

use crate::whisper::{self, WhisperModel};

// ── Types ──

#[derive(Serialize)]
pub struct ModelInfo {
    downloaded: bool,
    size_mb: u64,
    label: &'static str,
}

#[derive(Serialize)]
pub struct ModelStatus {
    base: ModelInfo,
    medium: ModelInfo,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    percent: u32,
    downloaded_mb: f64,
    total_mb: f64,
    model: String,
}

// ── Commands ──

#[tauri::command]
pub fn check_model_status() -> Result<ModelStatus, String> {
    Ok(ModelStatus {
        base: ModelInfo {
            downloaded: whisper::is_model_downloaded(WhisperModel::Base),
            size_mb: WhisperModel::Base.size_bytes() / 1_000_000,
            label: WhisperModel::Base.label(),
        },
        medium: ModelInfo {
            downloaded: whisper::is_model_downloaded(WhisperModel::Medium),
            size_mb: WhisperModel::Medium.size_bytes() / 1_000_000,
            label: WhisperModel::Medium.label(),
        },
    })
}

#[tauri::command]
pub async fn download_model(model_name: String, window: Window) -> Result<(), String> {
    let model = match model_name.as_str() {
        "base" => WhisperModel::Base,
        "medium" => WhisperModel::Medium,
        _ => return Err(format!("Unknown model: {}. Use 'base' or 'medium'.", model_name)),
    };

    // Skip if already downloaded
    if whisper::is_model_downloaded(model) {
        log::info!("[Model] {} already downloaded, skipping", model.label());
        return Ok(());
    }

    let url = model.download_url();
    let final_path = whisper::model_path(model)?;
    let tmp_path = final_path.with_extension("bin.tmp");

    log::info!("[Model] Downloading {} from {}", model.label(), url);

    // Stream download with progress
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Download failed: HTTP {}", resp.status()));
    }

    let total_bytes = resp.content_length().unwrap_or(model.size_bytes());
    let total_mb = total_bytes as f64 / 1_000_000.0;

    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .map_err(|e| format!("Failed to create temp file: {}", e))?;

    let mut downloaded: u64 = 0;
    let mut last_percent: u32 = 0;
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download stream error: {}", e))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Write error: {}", e))?;

        downloaded += chunk.len() as u64;
        let percent = if total_bytes > 0 {
            ((downloaded as f64 / total_bytes as f64) * 100.0) as u32
        } else {
            0
        };

        // Emit progress at most every 1% change to avoid flooding
        if percent != last_percent {
            last_percent = percent;
            let _ = window.emit(
                "model-download-progress",
                DownloadProgress {
                    percent,
                    downloaded_mb: downloaded as f64 / 1_000_000.0,
                    total_mb,
                    model: model_name.clone(),
                },
            );
        }
    }

    file.flush()
        .await
        .map_err(|e| format!("Flush error: {}", e))?;
    drop(file);

    // Rename temp to final
    tokio::fs::rename(&tmp_path, &final_path)
        .await
        .map_err(|e| format!("Failed to rename temp file: {}", e))?;

    log::info!(
        "[Model] {} download complete ({:.1} MB)",
        model.label(),
        downloaded as f64 / 1_000_000.0
    );

    // Emit 100% completion
    let _ = window.emit(
        "model-download-progress",
        DownloadProgress {
            percent: 100,
            downloaded_mb: total_mb,
            total_mb,
            model: model_name,
        },
    );

    Ok(())
}

#[tauri::command]
pub async fn delete_model(model_name: String) -> Result<(), String> {
    let model = match model_name.as_str() {
        "base" => WhisperModel::Base,
        "medium" => WhisperModel::Medium,
        _ => return Err(format!("Unknown model: {}. Use 'base' or 'medium'.", model_name)),
    };

    let path = whisper::model_path(model)?;
    if !path.exists() {
        return Ok(()); // already gone
    }

    tokio::fs::remove_file(&path)
        .await
        .map_err(|e| format!("Failed to delete model: {}", e))?;

    log::info!("[Model] Deleted {}", model.label());
    Ok(())
}
