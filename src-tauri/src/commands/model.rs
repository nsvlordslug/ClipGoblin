//! Whisper model management — check status, download, delete.

use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;
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

fn validate_model_download(
    model: WhisperModel,
    downloaded_bytes: u64,
    actual_sha256: &str,
) -> Result<(), String> {
    if downloaded_bytes != model.size_bytes() {
        return Err(format!(
            "Downloaded model size did not match the official artifact (expected {} bytes, received {} bytes)",
            model.size_bytes(),
            downloaded_bytes
        ));
    }
    if !actual_sha256.eq_ignore_ascii_case(model.expected_sha256()) {
        return Err("Downloaded model failed its SHA-256 integrity check".to_string());
    }
    Ok(())
}

async fn remove_temp_download(path: &Path) {
    if let Err(error) = tokio::fs::remove_file(path).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            log::warn!(
                "[Model] Could not remove incomplete model download {}: {}",
                path.display(),
                error
            );
        }
    }
}

// ── Commands ──

#[tauri::command]
pub fn check_model_status() -> Result<ModelStatus, String> {
    let dir = whisper::models_dir().unwrap_or_default();
    let base_path = whisper::model_path(WhisperModel::Base).unwrap_or_default();
    let medium_path = whisper::model_path(WhisperModel::Medium).unwrap_or_default();
    let base_exists = base_path.exists();
    let medium_exists = medium_path.exists();
    let base_downloaded = whisper::is_model_downloaded(WhisperModel::Base);
    let medium_downloaded = whisper::is_model_downloaded(WhisperModel::Medium);
    log::info!(
        "[Model] check_model_status — dir={}, base={} (exists={}, valid={}), medium={} (exists={}, valid={})",
        dir.display(),
        base_path.display(),
        base_exists,
        base_downloaded,
        medium_path.display(),
        medium_exists,
        medium_downloaded,
    );
    Ok(ModelStatus {
        base: ModelInfo {
            downloaded: base_downloaded,
            size_mb: WhisperModel::Base.size_bytes() / 1_000_000,
            label: WhisperModel::Base.label(),
        },
        medium: ModelInfo {
            downloaded: medium_downloaded,
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
        _ => {
            return Err(format!(
                "Unknown model: {}. Use 'base' or 'medium'.",
                model_name
            ))
        }
    };

    // Skip if already downloaded
    if whisper::is_model_downloaded(model) {
        log::info!("[Model] {} already downloaded, skipping", model.label());
        return Ok(());
    }

    let url = model.download_url();
    let final_path = whisper::model_path(model)?;
    let tmp_path = final_path.with_extension("bin.tmp");
    remove_temp_download(&tmp_path).await;

    log::info!("[Model] Downloading {} from {}", model.label(), url);

    let expected_bytes = model.size_bytes();
    let total_mb = expected_bytes as f64 / 1_000_000.0;
    let download_result: Result<u64, String> = async {
        let client = reqwest::Client::new();
        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("Download request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Download failed: HTTP {}", resp.status()));
        }
        if let Some(content_length) = resp.content_length() {
            if content_length != expected_bytes {
                return Err(format!(
                    "Download server reported an unexpected model size (expected {} bytes, reported {} bytes)",
                    expected_bytes, content_length
                ));
            }
        }

        let mut file = tokio::fs::File::create(&tmp_path)
            .await
            .map_err(|e| format!("Failed to create temp file: {}", e))?;
        let mut hasher = Sha256::new();
        let mut downloaded: u64 = 0;
        let mut last_percent: u32 = 0;
        let mut stream = resp.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Download stream error: {}", e))?;
            let next_size = downloaded.saturating_add(chunk.len() as u64);
            if next_size > expected_bytes {
                return Err("Downloaded model exceeded the expected official size".to_string());
            }
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("Write error: {}", e))?;
            hasher.update(&chunk);
            downloaded = next_size;
            let percent = (((downloaded as f64 / expected_bytes as f64) * 100.0) as u32).min(99);

            // Emit progress at most every 1% change to avoid flooding.
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
        file.sync_all()
            .await
            .map_err(|e| format!("Model sync error: {}", e))?;
        drop(file);

        let actual_sha256 = format!("{:x}", hasher.finalize());
        validate_model_download(model, downloaded, &actual_sha256)?;
        Ok(downloaded)
    }
    .await;

    let downloaded = match download_result {
        Ok(downloaded) => downloaded,
        Err(error) => {
            remove_temp_download(&tmp_path).await;
            return Err(error);
        }
    };

    // Only replace an invalid old artifact after the new one is fully verified.
    if final_path.exists() {
        tokio::fs::remove_file(&final_path)
            .await
            .map_err(|e| format!("Failed to replace invalid local model: {}", e))?;
    }
    if let Err(error) = tokio::fs::rename(&tmp_path, &final_path).await {
        remove_temp_download(&tmp_path).await;
        return Err(format!("Failed to install verified model: {}", error));
    }

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
        _ => {
            return Err(format!(
                "Unknown model: {}. Use 'base' or 'medium'.",
                model_name
            ))
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_model_metadata_accepts_only_exact_size_and_hash() {
        let model = WhisperModel::Medium;
        assert!(
            validate_model_download(model, model.size_bytes(), model.expected_sha256(),).is_ok()
        );
        assert!(
            validate_model_download(model, model.size_bytes() - 1, model.expected_sha256(),)
                .unwrap_err()
                .contains("size")
        );
        assert!(
            validate_model_download(model, model.size_bytes(), &"0".repeat(64))
                .unwrap_err()
                .contains("SHA-256")
        );
    }
}
