use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::commands::export::{probe_media_duration, render_clip_by_id};
use crate::commands::vod::find_ffmpeg;
use crate::db;

const MAX_MONTAGE_CLIPS: usize = 200;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MontageExportProgress {
    project_id: String,
    progress: u8,
    stage: String,
    current_clip: usize,
    total_clips: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MontageExportResult {
    output_path: String,
    output_directory: String,
    duration_seconds: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MontageOutputSize {
    width: u32,
    height: u32,
}

impl MontageOutputSize {
    fn from_preset(preset: &str) -> Result<Self, String> {
        match preset {
            "youtube" => Ok(Self {
                width: 1920,
                height: 1080,
            }),
            "shorts" => Ok(Self {
                width: 1080,
                height: 1920,
            }),
            _ => Err("Choose either YouTube (16:9) or Shorts (9:16).".to_string()),
        }
    }
}

struct TempMontageDir(PathBuf);

impl Drop for TempMontageDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn emit_progress(
    app: &AppHandle,
    project_id: &str,
    progress: u8,
    stage: impl Into<String>,
    current_clip: usize,
    total_clips: usize,
) {
    let _ = app.emit(
        "montage-export-progress",
        MontageExportProgress {
            project_id: project_id.to_string(),
            progress,
            stage: stage.into(),
            current_clip,
            total_clips,
        },
    );
}

fn safe_filename_stem(title: &str) -> String {
    let mut safe = title
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, ' ' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    safe = safe.trim_matches([' ', '.', '_']).to_string();
    safe = safe.chars().take(80).collect();
    if safe.is_empty() {
        "ClipGoblin Montage".to_string()
    } else {
        safe
    }
}

fn unique_output_path(output_dir: &Path, title: &str) -> PathBuf {
    let stem = format!("{}_montage", safe_filename_stem(title));
    let mut candidate = output_dir.join(format!("{stem}.mp4"));
    let mut suffix = 2u32;
    while candidate.exists() {
        candidate = output_dir.join(format!("{stem} ({suffix}).mp4"));
        suffix += 1;
    }
    candidate
}

fn stderr_tail(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .rev()
        .take(18)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ")
}

fn has_audio_stream(ffmpeg: &Path, path: &Path) -> bool {
    let mut command = Command::new(ffmpeg);
    command
        .arg("-v")
        .arg("error")
        .arg("-i")
        .arg(path)
        .arg("-map")
        .arg("0:a:0")
        .arg("-t")
        .arg("0.01")
        .arg("-f")
        .arg("null")
        .arg("-")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.status().map(|status| status.success()).unwrap_or(true)
}

fn normalize_segment(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    size: MontageOutputSize,
) -> Result<(), String> {
    let audio_present = has_audio_stream(ffmpeg, input);
    let video_filter = format!(
        "scale={}:{}:force_original_aspect_ratio=decrease:flags=lanczos,pad={}:{}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,setpts=PTS-STARTPTS,fps=30,format=yuv420p",
        size.width, size.height, size.width, size.height
    );

    let mut command = Command::new(ffmpeg);
    command
        .arg("-y")
        .arg("-fflags")
        .arg("+genpts")
        .arg("-i")
        .arg(input);
    if !audio_present {
        command
            .arg("-f")
            .arg("lavfi")
            .arg("-i")
            .arg("anullsrc=channel_layout=stereo:sample_rate=48000");
    }
    command
        .arg("-map")
        .arg("0:v:0")
        .arg("-map")
        .arg(if audio_present { "0:a:0" } else { "1:a:0" })
        .arg("-vf")
        .arg(video_filter);
    if audio_present {
        command
            .arg("-af")
            .arg("aresample=48000:async=1:first_pts=0,asetpts=PTS-STARTPTS");
    } else {
        command.arg("-shortest");
    }
    command
        .arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("medium")
        .arg("-crf")
        .arg("20")
        .arg("-g")
        .arg("60")
        .arg("-keyint_min")
        .arg("60")
        .arg("-sc_threshold")
        .arg("0")
        .arg("-c:a")
        .arg("aac")
        .arg("-b:a")
        .arg("192k")
        .arg("-ar")
        .arg("48000")
        .arg("-ac")
        .arg("2")
        .arg("-video_track_timescale")
        .arg("90000")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    let result = command
        .output()
        .map_err(|error| format!("Could not start FFmpeg for a montage clip: {error}"))?;
    if result.status.success()
        && std::fs::metadata(output)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
    {
        Ok(())
    } else {
        let detail = stderr_tail(&result.stderr);
        Err(if detail.is_empty() {
            "FFmpeg could not prepare one of the montage clips.".to_string()
        } else {
            format!("FFmpeg could not prepare one of the montage clips: {detail}")
        })
    }
}

fn write_concat_manifest(temp_dir: &Path, count: usize) -> Result<PathBuf, String> {
    let manifest = temp_dir.join("segments.txt");
    let contents = (0..count)
        .map(|index| format!("file 'segment-{index:04}.mp4'\n"))
        .collect::<String>();
    std::fs::write(&manifest, contents)
        .map_err(|error| format!("Could not prepare the montage sequence: {error}"))?;
    Ok(manifest)
}

fn run_concat_command(
    ffmpeg: &Path,
    temp_dir: &Path,
    manifest: &Path,
    output: &Path,
    reencode: bool,
) -> Result<(), String> {
    let manifest_name = manifest
        .file_name()
        .ok_or_else(|| "Montage sequence file is invalid.".to_string())?;
    let mut command = Command::new(ffmpeg);
    command
        .current_dir(temp_dir)
        .arg("-y")
        .arg("-f")
        .arg("concat")
        .arg("-safe")
        .arg("0")
        .arg("-i")
        .arg(manifest_name);
    if reencode {
        command
            .arg("-c:v")
            .arg("libx264")
            .arg("-preset")
            .arg("medium")
            .arg("-crf")
            .arg("20")
            .arg("-c:a")
            .arg("aac")
            .arg("-b:a")
            .arg("192k");
    } else {
        command.arg("-c").arg("copy");
    }
    command
        .arg("-movflags")
        .arg("+faststart")
        .arg(output)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    let result = command
        .output()
        .map_err(|error| format!("Could not start FFmpeg to join the montage: {error}"))?;
    if result.status.success()
        && std::fs::metadata(output)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
    {
        Ok(())
    } else {
        let detail = stderr_tail(&result.stderr);
        Err(if detail.is_empty() {
            "FFmpeg could not join the montage clips.".to_string()
        } else {
            format!("FFmpeg could not join the montage clips: {detail}")
        })
    }
}

fn join_segments(
    ffmpeg: &Path,
    temp_dir: &Path,
    manifest: &Path,
    output: &Path,
) -> Result<(), String> {
    match run_concat_command(ffmpeg, temp_dir, manifest, output, false) {
        Ok(()) => Ok(()),
        Err(copy_error) => {
            log::warn!("[montage] stream-copy join failed; retrying with re-encode: {copy_error}");
            let _ = std::fs::remove_file(output);
            run_concat_command(ffmpeg, temp_dir, manifest, output, true)
        }
    }
}

fn reusable_rendered_clip(clip_id: &str) -> Result<Option<PathBuf>, String> {
    let db_path = db::db_path().map_err(|error| format!("DB path: {error}"))?;
    let connection = rusqlite::Connection::open(db_path)
        .map_err(|error| format!("DB open: {error}"))?;
    let clip = db::get_clip_by_id(&connection, clip_id)
        .map_err(|error| format!("DB error: {error}"))?
        .ok_or_else(|| format!("Clip {clip_id} no longer exists."))?;
    if clip.render_status == "rendering" {
        return Err(format!(
            "{} is already being exported. Wait for it to finish, then try the montage again.",
            if clip.title.trim().is_empty() {
                "One selected clip"
            } else {
                clip.title.trim()
            }
        ));
    }
    Ok(clip
        .output_path
        .filter(|_| clip.render_status == "completed")
        .map(PathBuf::from)
        .filter(|path| {
            std::fs::metadata(path)
                .map(|metadata| metadata.len() > 0)
                .unwrap_or(false)
        }))
}

#[tauri::command]
pub async fn export_montage(
    project_id: String,
    title: String,
    clip_ids: Vec<String>,
    preset: String,
    app: AppHandle,
) -> Result<MontageExportResult, String> {
    if clip_ids.is_empty() {
        return Err("Add at least one clip before exporting a montage.".to_string());
    }
    if clip_ids.len() > MAX_MONTAGE_CLIPS {
        return Err(format!(
            "A montage can contain at most {MAX_MONTAGE_CLIPS} clips."
        ));
    }
    if clip_ids.iter().any(|clip_id| clip_id.trim().is_empty()) {
        return Err("The montage contains an invalid clip.".to_string());
    }
    let unique = clip_ids.iter().collect::<HashSet<_>>();
    if unique.len() != clip_ids.len() {
        return Err("Remove duplicate clips before exporting the montage.".to_string());
    }

    let output_size = MontageOutputSize::from_preset(&preset)?;
    let ffmpeg = find_ffmpeg().map_err(|error| error.to_string())?;
    let output_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("clipviral")
        .join("exports");
    std::fs::create_dir_all(&output_dir)
        .map_err(|error| format!("Could not create the export folder: {error}"))?;
    let output_path = unique_output_path(&output_dir, &title);

    let temp_dir = std::env::temp_dir().join(format!(
        "clipgoblin-montage-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&temp_dir)
        .map_err(|error| format!("Could not prepare temporary montage files: {error}"))?;
    let _temp_guard = TempMontageDir(temp_dir.clone());

    let total = clip_ids.len();
    emit_progress(&app, &project_id, 1, "Preparing montage", 0, total);

    for (index, clip_id) in clip_ids.iter().enumerate() {
        let current = index + 1;
        let start_progress = 2 + ((index * 88) / total) as u8;
        emit_progress(
            &app,
            &project_id,
            start_progress,
            format!("Rendering clip {current} of {total}"),
            current,
            total,
        );

        let rendered_path = match reusable_rendered_clip(clip_id)? {
            Some(path) => path,
            None => render_clip_by_id(clip_id).await?,
        };
        let normalized_path = temp_dir.join(format!("segment-{index:04}.mp4"));
        let ffmpeg_for_task = ffmpeg.clone();
        let rendered_for_task = rendered_path.clone();
        let normalized_for_task = normalized_path.clone();
        tokio::task::spawn_blocking(move || {
            normalize_segment(
                &ffmpeg_for_task,
                &rendered_for_task,
                &normalized_for_task,
                output_size,
            )
        })
        .await
        .map_err(|error| format!("Montage preparation task failed: {error}"))??;

        let complete_progress = 2 + ((current * 88) / total) as u8;
        emit_progress(
            &app,
            &project_id,
            complete_progress,
            format!("Prepared clip {current} of {total}"),
            current,
            total,
        );
    }

    let manifest = write_concat_manifest(&temp_dir, total)?;
    emit_progress(&app, &project_id, 92, "Joining montage", total, total);
    let ffmpeg_for_task = ffmpeg.clone();
    let temp_for_task = temp_dir.clone();
    let manifest_for_task = manifest.clone();
    let output_for_task = output_path.clone();
    tokio::task::spawn_blocking(move || {
        join_segments(
            &ffmpeg_for_task,
            &temp_for_task,
            &manifest_for_task,
            &output_for_task,
        )
    })
    .await
    .map_err(|error| format!("Montage join task failed: {error}"))??;

    let duration_seconds = probe_media_duration(&output_path).unwrap_or(0.0);
    emit_progress(&app, &project_id, 100, "Montage ready", total, total);
    log::info!(
        "[montage] exported {} clips to {} ({:.2}s)",
        total,
        output_path.display(),
        duration_seconds
    );

    Ok(MontageExportResult {
        output_path: output_path.to_string_lossy().to_string(),
        output_directory: output_dir.to_string_lossy().to_string(),
        duration_seconds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_sizes_are_explicit_and_reject_unknown_values() {
        assert_eq!(
            MontageOutputSize::from_preset("youtube").unwrap(),
            MontageOutputSize {
                width: 1920,
                height: 1080,
            }
        );
        assert_eq!(
            MontageOutputSize::from_preset("shorts").unwrap(),
            MontageOutputSize {
                width: 1080,
                height: 1920,
            }
        );
        assert!(MontageOutputSize::from_preset("unknown").is_err());
    }

    #[test]
    fn filename_stem_removes_windows_unsafe_characters_and_bounds_length() {
        assert_eq!(safe_filename_stem(" Best: clips? <ever> "), "Best_ clips_ _ever");
        assert_eq!(safe_filename_stem("..."), "ClipGoblin Montage");
        assert!(safe_filename_stem(&"x".repeat(200)).len() <= 80);
        assert_eq!(safe_filename_stem(&"é".repeat(100)).chars().count(), 80);
    }

    #[test]
    fn concat_manifest_uses_only_generated_relative_names() {
        let dir = std::env::temp_dir().join(format!("montage-manifest-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = write_concat_manifest(&dir, 3).unwrap();
        let text = std::fs::read_to_string(manifest).unwrap();
        assert_eq!(
            text,
            "file 'segment-0000.mp4'\nfile 'segment-0001.mp4'\nfile 'segment-0002.mp4'\n"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ffmpeg_join_normalizes_mixed_geometry_and_adds_missing_audio() {
        let Ok(ffmpeg) = find_ffmpeg() else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("montage-ffmpeg-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let landscape = dir.join("landscape.mp4");
        let vertical = dir.join("vertical.mp4");

        let first = Command::new(&ffmpeg)
            .arg("-y")
            .arg("-f")
            .arg("lavfi")
            .arg("-i")
            .arg("color=c=red:s=320x180:d=0.6:r=30")
            .arg("-f")
            .arg("lavfi")
            .arg("-i")
            .arg("sine=frequency=440:duration=0.6:sample_rate=48000")
            .arg("-shortest")
            .arg("-c:v")
            .arg("libx264")
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg("-c:a")
            .arg("aac")
            .arg(&landscape)
            .output()
            .unwrap();
        assert!(first.status.success());

        let second = Command::new(&ffmpeg)
            .arg("-y")
            .arg("-f")
            .arg("lavfi")
            .arg("-i")
            .arg("color=c=blue:s=180x320:d=0.6:r=30")
            .arg("-c:v")
            .arg("libx264")
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg(&vertical)
            .output()
            .unwrap();
        assert!(second.status.success());

        let target = MontageOutputSize {
            width: 320,
            height: 180,
        };
        normalize_segment(&ffmpeg, &landscape, &dir.join("segment-0000.mp4"), target).unwrap();
        normalize_segment(&ffmpeg, &vertical, &dir.join("segment-0001.mp4"), target).unwrap();
        let manifest = write_concat_manifest(&dir, 2).unwrap();
        let output = dir.join("joined.mp4");
        join_segments(&ffmpeg, &dir, &manifest, &output).unwrap();

        assert!(has_audio_stream(&ffmpeg, &output));
        if let Some(duration) = probe_media_duration(&output) {
            assert!(duration > 1.0 && duration < 1.6, "unexpected duration: {duration}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
