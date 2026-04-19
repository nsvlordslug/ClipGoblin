# Auto-Download ffmpeg + yt-dlp Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On first launch, automatically download `ffmpeg.exe`, `ffprobe.exe`, and `yt-dlp.exe` into `%APPDATA%/clipviral/bin/` so non-technical users don't have to install binaries manually.

**Architecture:**
- Centralize binary path resolution in a new `bin_manager.rs` module (priority: bundled bin dir → system PATH → trigger download).
- Add a new `commands/binaries.rs` Tauri module exposing `check_binary_status` and `download_binaries` commands.
- Refactor existing `find_ffmpeg()` / `find_ytdlp()` in `commands/vod.rs` and `whisper.rs` to delegate to `bin_manager`, so the ~10 existing call sites continue to work unchanged.
- Extend the existing `FirstRunSetup.tsx` gate to also detect + download missing binaries (reuse its modal/progress UX pattern).

**Tech Stack:** Rust (Tauri 2, `reqwest`, `tokio`, `futures-util`, `zip` — new), React + TypeScript (Tauri invoke + event listeners).

---

## Context — Existing State

A few things are ALREADY implemented and must not be broken:

- `find_ffmpeg()` at [src-tauri/src/commands/vod.rs:98](src-tauri/src/commands/vod.rs:98) — searches WinGet, `C:\ffmpeg`, app data, PATH.
- `find_ytdlp()` at [src-tauri/src/commands/vod.rs:39](src-tauri/src/commands/vod.rs:39) — searches Python Scripts dirs, PATH.
- Duplicate `find_ffmpeg()` at [src-tauri/src/whisper.rs:96](src-tauri/src/whisper.rs:96).
- Model download pattern in [src-tauri/src/commands/model.rs](src-tauri/src/commands/model.rs) — streams `reqwest` with per-1% progress events, temp-file-then-rename.
- [src/components/FirstRunSetup.tsx](src/components/FirstRunSetup.tsx) — existing gate that checks Whisper model status on mount.
- `AppError::Download` and `AppError::Ffmpeg` variants already exist in [src-tauri/src/error.rs](src-tauri/src/error.rs).
- Cargo deps already present: `reqwest` (stream, native-tls), `tokio`, `futures-util`, `dirs`, `which`. Missing: `zip`.

App data directory on Windows = `%APPDATA%\clipviral\` (already used for SQLite DB and Whisper models). New subdir = `%APPDATA%\clipviral\bin\`.

## File Structure

- **Create** `src-tauri/src/bin_manager.rs` — path resolution + download + extraction.
- **Create** `src-tauri/src/commands/binaries.rs` — Tauri commands wrapping `bin_manager`.
- **Modify** `src-tauri/Cargo.toml` — add `zip = "2"` (use stable 2.x which supports reading zip files; 0.6 is older; default-features off).
- **Modify** `src-tauri/src/lib.rs` — register module + commands.
- **Modify** `src-tauri/src/commands/mod.rs` — add `pub mod binaries;`.
- **Modify** `src-tauri/src/commands/vod.rs` — rewire `find_ytdlp()` and `find_ffmpeg()` to call `bin_manager`.
- **Modify** `src-tauri/src/whisper.rs` — replace local `find_ffmpeg()` with delegation to `bin_manager`.
- **Create** `src/components/BinariesSetup.tsx` — download gate for ffmpeg + yt-dlp.
- **Modify** `src/App.tsx` — wrap `<FirstRunSetup>` with `<BinariesSetup>`.

---

## Task 1: Add `zip` crate dependency

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add the `zip` dependency**

Edit `src-tauri/Cargo.toml`. Find this line:

```toml
futures-util = "0.3"
```

And add immediately after it:

```toml
zip = { version = "2", default-features = false, features = ["deflate"] }
```

Rationale: we only read `.zip` with DEFLATE-compressed entries from GitHub's FFmpeg-Builds release. Disabling default features avoids pulling in bzip2 / zstd / xz / aes native deps we don't need.

- [ ] **Step 2: Verify it compiles**

Run (from repo root):

```bash
cd src-tauri && cargo check
```

Expected: compiles clean (no warnings about unused dep since we'll use it in Task 3). If `zip v2` is not on crates.io yet, fall back to `zip = "0.6"` and keep the `default-features = false, features = ["deflate"]` shape — the API used in Task 3 (`ZipArchive::new`, `by_index`, `name()`, `is_file()`, reading into `Vec<u8>`) is stable across 0.6 and 2.x.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "deps: add zip crate for ffmpeg archive extraction"
```

---

## Task 2: Create `bin_manager.rs` — path resolution

**Files:**
- Create: `src-tauri/src/bin_manager.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod bin_manager;`)

- [ ] **Step 1: Create `bin_manager.rs` with types and path accessors**

Create `src-tauri/src/bin_manager.rs` with this exact content:

```rust
//! Manages bundled external binaries (ffmpeg, ffprobe, yt-dlp).
//!
//! Priority order when resolving a binary path:
//! 1. `%APPDATA%/clipviral/bin/<name>.exe` (bundled, auto-downloaded)
//! 2. System PATH (existing behaviour — users who already have them installed)
//! 3. Return error / trigger download UI
//!
//! Downloads land in the app data dir and are used by every call site via
//! [`ffmpeg_path`], [`ffprobe_path`], [`ytdlp_path`].

use std::path::PathBuf;
use std::process::Stdio;

use serde::Serialize;

use crate::error::AppError;

// ── Paths ──

/// `%APPDATA%/clipviral/bin/`, creating it if it doesn't exist.
pub fn bin_dir() -> Result<PathBuf, AppError> {
    let base = dirs::data_dir()
        .ok_or_else(|| AppError::Unknown("no APPDATA dir on this system".into()))?;
    let dir = base.join("clipviral").join("bin");
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::Unknown(format!("create bin dir: {e}")))?;
    Ok(dir)
}

fn bundled_path(name: &str) -> Option<PathBuf> {
    let p = bin_dir().ok()?.join(name);
    if p.exists() { Some(p) } else { None }
}

/// Returns `true` if `<name>` (without .exe) runs successfully with `-version`
/// or `--version` via the system PATH. Used as a fallback for users who already
/// have the tool installed system-wide.
fn in_path(name: &str, version_flag: &str) -> bool {
    let mut cmd = std::process::Command::new(name);
    cmd.arg(version_flag).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

/// Return a usable `ffmpeg` path: bundled first, then system PATH (bare name).
pub fn ffmpeg_path() -> Result<PathBuf, AppError> {
    if let Some(p) = bundled_path("ffmpeg.exe") {
        log::info!("[bin_manager] ffmpeg: bundled at {}", p.display());
        return Ok(p);
    }
    if in_path("ffmpeg", "-version") {
        log::info!("[bin_manager] ffmpeg: using system PATH");
        return Ok(PathBuf::from("ffmpeg"));
    }
    Err(AppError::Ffmpeg("ffmpeg not found (bundled or system PATH)".into()))
}

/// Return a usable `ffprobe` path: bundled first, then system PATH (bare name).
pub fn ffprobe_path() -> Result<PathBuf, AppError> {
    if let Some(p) = bundled_path("ffprobe.exe") {
        return Ok(p);
    }
    if in_path("ffprobe", "-version") {
        return Ok(PathBuf::from("ffprobe"));
    }
    Err(AppError::Ffmpeg("ffprobe not found (bundled or system PATH)".into()))
}

/// Return a usable `yt-dlp` path: bundled first, then system PATH (bare name).
pub fn ytdlp_path() -> Result<PathBuf, AppError> {
    if let Some(p) = bundled_path("yt-dlp.exe") {
        log::info!("[bin_manager] yt-dlp: bundled at {}", p.display());
        return Ok(p);
    }
    if in_path("yt-dlp", "--version") {
        log::info!("[bin_manager] yt-dlp: using system PATH");
        return Ok(PathBuf::from("yt-dlp"));
    }
    Err(AppError::Download("yt-dlp not found (bundled or system PATH)".into()))
}

// ── Status ──

/// Serialisable status of the three external binaries for the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryStatus {
    /// Either bundled ffmpeg.exe exists, or system PATH has ffmpeg.
    pub ffmpeg_available: bool,
    /// Either bundled ffprobe.exe exists, or system PATH has ffprobe.
    pub ffprobe_available: bool,
    /// Either bundled yt-dlp.exe exists, or system PATH has yt-dlp.
    pub ytdlp_available: bool,
    /// `true` when the bundled ffmpeg.exe file is present in the app bin dir.
    pub ffmpeg_bundled: bool,
    /// `true` when the bundled yt-dlp.exe file is present in the app bin dir.
    pub ytdlp_bundled: bool,
}

pub fn check_binaries() -> BinaryStatus {
    BinaryStatus {
        ffmpeg_available: ffmpeg_path().is_ok(),
        ffprobe_available: ffprobe_path().is_ok(),
        ytdlp_available: ytdlp_path().is_ok(),
        ffmpeg_bundled: bundled_path("ffmpeg.exe").is_some(),
        ytdlp_bundled: bundled_path("yt-dlp.exe").is_some(),
    }
}

// Download implementations live in Task 3 — see `download_ytdlp` and
// `download_ffmpeg` in this same module.
```

- [ ] **Step 2: Register the module**

Edit `src-tauri/src/lib.rs`. Find this block near the top:

```rust
mod ai_provider;
mod auth_proxy;
mod crypto;
mod audio_signal;
```

Add `mod bin_manager;` on its own line immediately above `mod ai_provider;`:

```rust
mod bin_manager;
mod ai_provider;
mod auth_proxy;
mod crypto;
mod audio_signal;
```

- [ ] **Step 3: Verify compilation**

```bash
cd src-tauri && cargo check
```

Expected: compiles clean.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/bin_manager.rs src-tauri/src/lib.rs
git commit -m "bin_manager: add module with path resolution and status struct"
```

---

## Task 3: Implement download helpers

**Files:**
- Modify: `src-tauri/src/bin_manager.rs`

- [ ] **Step 1: Add imports for streaming + zip**

At the top of `src-tauri/src/bin_manager.rs`, below the existing `use` lines, add:

```rust
use std::io::{Read, Write};
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;
```

Also add this constant block immediately under the imports:

```rust
// ── Download URLs ──

/// BtbN FFmpeg-Builds latest Win64 GPL release. Contains `bin/ffmpeg.exe` and
/// `bin/ffprobe.exe` inside a zip (~130 MB compressed).
const FFMPEG_ZIP_URL: &str =
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip";

/// yt-dlp's GitHub "latest" redirect to the most recent Windows exe (~20 MB).
const YTDLP_URL: &str =
    "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
```

- [ ] **Step 2: Append the `download_ytdlp` helper**

At the bottom of `src-tauri/src/bin_manager.rs`, append:

```rust
/// Progress callback: `(bytes_downloaded, total_bytes_or_zero_if_unknown)`.
pub type ProgressCb = Box<dyn Fn(u64, u64) + Send + Sync>;

pub async fn download_ytdlp(progress: &ProgressCb) -> Result<(), AppError> {
    let dir = bin_dir()?;
    let final_path = dir.join("yt-dlp.exe");
    let tmp_path = dir.join("yt-dlp.exe.tmp");

    log::info!("[bin_manager] downloading yt-dlp from {}", YTDLP_URL);

    let client = reqwest::Client::new();
    let resp = client.get(YTDLP_URL).send().await
        .map_err(|e| AppError::Download(format!("yt-dlp request: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Download(format!("yt-dlp HTTP {}", resp.status())));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(&tmp_path).await
        .map_err(|e| AppError::Download(format!("create tmp: {e}")))?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| AppError::Download(format!("stream: {e}")))?;
        file.write_all(&chunk).await
            .map_err(|e| AppError::Download(format!("write: {e}")))?;
        downloaded += chunk.len() as u64;
        progress(downloaded, total);
    }
    file.flush().await.map_err(|e| AppError::Download(format!("flush: {e}")))?;
    drop(file);

    // Atomic rename. If the target already exists (user re-downloading), remove first.
    let _ = tokio::fs::remove_file(&final_path).await;
    tokio::fs::rename(&tmp_path, &final_path).await
        .map_err(|e| AppError::Download(format!("rename: {e}")))?;

    log::info!("[bin_manager] yt-dlp installed to {}", final_path.display());
    Ok(())
}
```

- [ ] **Step 3: Append the `download_ffmpeg` helper**

At the bottom of `src-tauri/src/bin_manager.rs`, append:

```rust
/// Download the FFmpeg zip to a temp file, then extract `ffmpeg.exe` and
/// `ffprobe.exe` into `bin_dir()`.
///
/// Progress is reported during the *download* phase only (not extraction);
/// the zip is tiny to extract compared to downloading 130 MB.
pub async fn download_ffmpeg(progress: &ProgressCb) -> Result<(), AppError> {
    let dir = bin_dir()?;
    let zip_path = dir.join("ffmpeg-download.zip.tmp");

    log::info!("[bin_manager] downloading ffmpeg from {}", FFMPEG_ZIP_URL);

    // --- download phase ---
    let client = reqwest::Client::new();
    let resp = client.get(FFMPEG_ZIP_URL).send().await
        .map_err(|e| AppError::Download(format!("ffmpeg request: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Download(format!("ffmpeg HTTP {}", resp.status())));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(&zip_path).await
        .map_err(|e| AppError::Download(format!("create zip tmp: {e}")))?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            // Best-effort cleanup on mid-download failure.
            let _ = std::fs::remove_file(&zip_path);
            AppError::Download(format!("stream: {e}"))
        })?;
        file.write_all(&chunk).await
            .map_err(|e| AppError::Download(format!("write zip: {e}")))?;
        downloaded += chunk.len() as u64;
        progress(downloaded, total);
    }
    file.flush().await.map_err(|e| AppError::Download(format!("flush: {e}")))?;
    drop(file);

    // --- extraction phase (blocking; move to spawn_blocking) ---
    let zip_path_extract = zip_path.clone();
    let dir_extract = dir.clone();
    let extract_result = tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        extract_ffmpeg_bins(&zip_path_extract, &dir_extract)
    })
    .await
    .map_err(|e| AppError::Unknown(format!("extract join: {e}")))?;

    // Always remove the zip (success or failure).
    let _ = tokio::fs::remove_file(&zip_path).await;

    extract_result?;

    log::info!("[bin_manager] ffmpeg + ffprobe installed to {}", dir.display());
    Ok(())
}

/// Walk the zip, find entries whose filename is `ffmpeg.exe` or `ffprobe.exe`
/// (typically under `ffmpeg-.../bin/`), and write them into `dir`.
fn extract_ffmpeg_bins(zip_path: &std::path::Path, dir: &std::path::Path) -> Result<(), AppError> {
    let f = std::fs::File::open(zip_path)
        .map_err(|e| AppError::Download(format!("open zip: {e}")))?;
    let mut archive = zip::ZipArchive::new(f)
        .map_err(|e| AppError::Download(format!("read zip: {e}")))?;

    let targets = ["ffmpeg.exe", "ffprobe.exe"];
    let mut extracted = [false; 2];

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)
            .map_err(|e| AppError::Download(format!("zip entry {i}: {e}")))?;
        if !entry.is_file() { continue; }

        // The zip's internal path is like "ffmpeg-master-latest-win64-gpl/bin/ffmpeg.exe".
        // Match by the last path component.
        let name = entry.name().to_string();
        let leaf = std::path::Path::new(&name)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        for (idx, target) in targets.iter().enumerate() {
            if leaf.eq_ignore_ascii_case(target) && !extracted[idx] {
                let out = dir.join(target);
                let tmp = dir.join(format!("{target}.tmp"));
                let mut buf = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut buf)
                    .map_err(|e| AppError::Download(format!("read {target}: {e}")))?;
                let mut f = std::fs::File::create(&tmp)
                    .map_err(|e| AppError::Download(format!("create {target}.tmp: {e}")))?;
                f.write_all(&buf)
                    .map_err(|e| AppError::Download(format!("write {target}: {e}")))?;
                drop(f);
                let _ = std::fs::remove_file(&out);
                std::fs::rename(&tmp, &out)
                    .map_err(|e| AppError::Download(format!("rename {target}: {e}")))?;
                extracted[idx] = true;
                break;
            }
        }

        if extracted.iter().all(|b| *b) { break; }
    }

    if !extracted[0] {
        return Err(AppError::Download("ffmpeg.exe not found in zip".into()));
    }
    if !extracted[1] {
        return Err(AppError::Download("ffprobe.exe not found in zip".into()));
    }
    Ok(())
}
```

- [ ] **Step 4: Verify compilation**

```bash
cd src-tauri && cargo check
```

Expected: compiles clean. If `zip::ZipArchive` / `by_index` signatures differ in `zip v2` vs `0.6`, adjust the `extract_ffmpeg_bins` function accordingly (both versions accept `&mut self` for iteration and return an entry with `name()`, `is_file()`, `size()`, and `Read`; API is near-identical).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/bin_manager.rs
git commit -m "bin_manager: implement download + zip extraction for ffmpeg and yt-dlp"
```

---

## Task 4: Tauri commands (`commands/binaries.rs`)

**Files:**
- Create: `src-tauri/src/commands/binaries.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Create `commands/binaries.rs`**

Create `src-tauri/src/commands/binaries.rs` with this content:

```rust
//! Tauri commands for checking and downloading the bundled external binaries
//! (ffmpeg, ffprobe, yt-dlp). Called from the first-run setup UI.

use std::sync::Arc;
use serde::Serialize;
use tauri::{Emitter, Window};

use crate::bin_manager::{self, BinaryStatus, ProgressCb};

/// Phase of the per-binary download, as reported to the frontend.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum Phase { Downloading, Extracting, Done }

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    /// "ffmpeg" or "yt-dlp".
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

    // yt-dlp — only download if neither bundled nor on PATH.
    if !status.ytdlp_available {
        let w = window.clone();
        let cb: ProgressCb = Arc::new(move |downloaded, total| {
            let _ = w.emit("download-progress", Progress {
                binary: "yt-dlp".into(),
                downloaded,
                total,
                phase: Phase::Downloading,
            });
        }).into();
        // `ProgressCb` is `Box<dyn Fn(...)>` — construct it via Box::new.
        let cb: ProgressCb = Box::new(move |d, t| {
            let _ = window.emit("download-progress", Progress {
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

    // ffmpeg — only download if we're missing it system-wide.
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
```

**NOTE:** The `Arc`/first `cb` block above was an intermediate stray — delete it. The file after edits should have exactly one `let cb: ProgressCb = Box::new(...)` per binary, as shown by the second definition. Final file should have no `Arc` import. (This note exists because it's easy to leave the first attempt in.)

Final clean content of `src-tauri/src/commands/binaries.rs`:

```rust
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
            phase: Phase::Done,
        });
    }

    Ok(())
}
```

- [ ] **Step 2: Register submodule in `commands/mod.rs`**

Read `src-tauri/src/commands/mod.rs` first to see its exact format (a short file of `pub mod <name>;` lines), then append a new line:

```rust
pub mod binaries;
```

- [ ] **Step 3: Register commands in `lib.rs`**

In `src-tauri/src/lib.rs`:

Find the existing `use commands::model::...` line:

```rust
use commands::model::{check_model_status, download_model, delete_model};
```

Immediately after it, add:

```rust
use commands::binaries::{check_binary_status, download_binaries};
```

Then, inside the `tauri::generate_handler![...]` call (near line 195, right after `delete_model,`), add two lines:

```rust
            check_binary_status,
            download_binaries,
```

- [ ] **Step 4: Verify compilation**

```bash
cd src-tauri && cargo check
```

Expected: compiles clean. The `ProgressCb` type (`Box<dyn Fn(u64, u64) + Send + Sync>`) from Task 3 must be exactly this shape; if `cargo check` complains about `Fn` vs `FnMut` or `Send` bounds, add the missing bounds to the type alias in `bin_manager.rs`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/binaries.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git commit -m "commands: add check_binary_status and download_binaries"
```

---

## Task 5: Refactor existing `find_ffmpeg` / `find_ytdlp` to delegate

Goal: every existing call site of `find_ffmpeg()` and `find_ytdlp()` (there are ~10) keeps working unchanged, but now goes through `bin_manager` so the bundled dir takes priority.

**Files:**
- Modify: `src-tauri/src/commands/vod.rs`
- Modify: `src-tauri/src/whisper.rs`

- [ ] **Step 1: Rewrite `find_ytdlp` in `commands/vod.rs`**

Open `src-tauri/src/commands/vod.rs`. Replace the entire function body (lines 39–95, starting `fn find_ytdlp() -> Result<...>` through its closing `}`) with:

```rust
fn find_ytdlp() -> Result<std::path::PathBuf, AppError> {
    crate::bin_manager::ytdlp_path()
}
```

This preserves the signature (`Result<PathBuf, AppError>`) and visibility (private module fn). All existing callers — `find_ytdlp()?` and `if let Ok(ytdlp) = find_ytdlp()` — remain valid.

- [ ] **Step 2: Rewrite `find_ffmpeg` in `commands/vod.rs`**

Replace the entire function body (lines 98–138, from `pub(crate) fn find_ffmpeg()` through its closing `}`) with:

```rust
pub(crate) fn find_ffmpeg() -> Result<std::path::PathBuf, AppError> {
    crate::bin_manager::ffmpeg_path()
}
```

Preserves `pub(crate)` visibility (used by `engine.rs` and `commands/export.rs`).

- [ ] **Step 3: Rewrite `find_ffmpeg` in `whisper.rs`**

Open `src-tauri/src/whisper.rs`. Replace the entire function body (starting at line 96 `pub fn find_ffmpeg() -> Result<PathBuf, String>` through its closing `}` around line 144) with:

```rust
pub fn find_ffmpeg() -> Result<PathBuf, String> {
    crate::bin_manager::ffmpeg_path().map_err(|e| e.to_string())
}
```

Note the signature difference: `whisper.rs::find_ffmpeg` returns `Result<PathBuf, String>` (not `AppError`). The `.map_err(|e| e.to_string())` bridges the two.

- [ ] **Step 4: Verify compilation**

```bash
cd src-tauri && cargo check
```

Expected: compiles clean. Warnings about unused imports (`Stdio`, `std::process::Command`, etc.) in `vod.rs` / `whisper.rs` are fine — leave them for a later cleanup; the goal here is behavioural equivalence.

- [ ] **Step 5: Smoke-test at runtime (manual)**

Run the app with existing binaries in place:

```bash
cd "C:\Users\cereb\Desktop\Claude projects\clipviral" && cargo tauri dev
```

Load a VOD page. In the terminal watch for log lines like:

```
[bin_manager] ffmpeg: bundled at C:\Users\...\AppData\Roaming\clipviral\bin\ffmpeg.exe
[bin_manager] ffmpeg: using system PATH
[bin_manager] yt-dlp: using system PATH
```

One of these lines should appear per binary. If neither appears, the resolution returned an error — check the terminal for the `ffmpeg not found` / `yt-dlp not found` AppError.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands/vod.rs src-tauri/src/whisper.rs
git commit -m "refactor: delegate find_ffmpeg/find_ytdlp to bin_manager"
```

---

## Task 6: Frontend — `BinariesSetup` gate

**Files:**
- Create: `src/components/BinariesSetup.tsx`
- Modify: `src/App.tsx`

- [ ] **Step 1: Create the component**

Create `src/components/BinariesSetup.tsx` with this exact content:

```tsx
import { useEffect, useState, type ReactNode } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { Download, CheckCircle, AlertCircle, Loader2 } from 'lucide-react'
import logoImg from '../assets/logo.png'

interface BinaryStatus {
  ffmpegAvailable: boolean
  ffprobeAvailable: boolean
  ytdlpAvailable: boolean
  ffmpegBundled: boolean
  ytdlpBundled: boolean
}

type State = 'checking' | 'needed' | 'downloading' | 'done' | 'error' | 'ready'

interface ProgressEvent {
  binary: 'ffmpeg' | 'yt-dlp'
  downloaded: number
  total: number
  phase: 'downloading' | 'extracting' | 'done'
}

const FFMPEG_APPROX_MB = 130
const YTDLP_APPROX_MB = 20

export default function BinariesSetup({ children }: { children: ReactNode }) {
  const [state, setState] = useState<State>('checking')
  const [ffmpegProgress, setFfmpegProgress] = useState<{ downloaded: number; total: number; phase: string } | null>(null)
  const [ytdlpProgress, setYtdlpProgress] = useState<{ downloaded: number; total: number; phase: string } | null>(null)
  const [errorMsg, setErrorMsg] = useState('')
  const [needsFfmpeg, setNeedsFfmpeg] = useState(false)
  const [needsYtdlp, setNeedsYtdlp] = useState(false)

  useEffect(() => {
    let cancelled = false
    ;(async () => {
      try {
        const status = await invoke<BinaryStatus>('check_binary_status')
        if (cancelled) return
        const missingFfmpeg = !status.ffmpegAvailable || !status.ffprobeAvailable
        const missingYtdlp = !status.ytdlpAvailable
        setNeedsFfmpeg(missingFfmpeg)
        setNeedsYtdlp(missingYtdlp)
        setState(missingFfmpeg || missingYtdlp ? 'needed' : 'ready')
      } catch {
        // If the check itself fails, don't block the app — user can still try manually.
        if (!cancelled) setState('ready')
      }
    })()
    return () => { cancelled = true }
  }, [])

  useEffect(() => {
    if (state !== 'downloading') return
    const unlisten = listen<ProgressEvent>('download-progress', (ev) => {
      const { binary, downloaded, total, phase } = ev.payload
      if (binary === 'ffmpeg') setFfmpegProgress({ downloaded, total, phase })
      else if (binary === 'yt-dlp') setYtdlpProgress({ downloaded, total, phase })
    })
    return () => { unlisten.then(fn => fn()) }
  }, [state])

  const start = async () => {
    setState('downloading')
    setFfmpegProgress(null)
    setYtdlpProgress(null)
    setErrorMsg('')
    try {
      await invoke('download_binaries')
      setState('done')
    } catch (err) {
      setErrorMsg(String(err))
      setState('error')
    }
  }

  if (state === 'checking') {
    return (
      <div className="flex items-center justify-center h-screen bg-surface-950">
        <Loader2 className="w-8 h-8 text-violet-400 animate-spin" />
      </div>
    )
  }

  if (state === 'ready') return <>{children}</>

  const pct = (p: { downloaded: number; total: number } | null) => {
    if (!p || !p.total) return 0
    return Math.min(100, Math.round((p.downloaded / p.total) * 100))
  }

  const mb = (bytes: number) => Math.round(bytes / 1_000_000)

  return (
    <div className="flex items-center justify-center h-screen bg-surface-950">
      <div className="text-center max-w-md px-8">
        <div className="w-20 h-20 mx-auto mb-6 rounded-2xl overflow-hidden shadow-lg shadow-violet-500/20">
          <img src={logoImg} alt="" className="w-full h-full object-cover" />
        </div>

        <h1 className="text-2xl font-bold text-white mb-2">One-time setup</h1>

        {state === 'needed' && (
          <>
            <p className="text-sm text-slate-400 mb-2">
              ClipGoblin needs a couple of helper tools to download and process your VODs.
            </p>
            <ul className="text-xs text-slate-500 mb-6 space-y-1">
              {needsFfmpeg && <li>• ffmpeg (~{FFMPEG_APPROX_MB} MB) — video processing</li>}
              {needsYtdlp && <li>• yt-dlp (~{YTDLP_APPROX_MB} MB) — Twitch VOD downloads</li>}
            </ul>
            <button
              onClick={start}
              className="flex items-center gap-2 mx-auto px-6 py-3 bg-violet-600 hover:bg-violet-500 text-white text-sm font-medium rounded-xl transition-colors cursor-pointer shadow-lg shadow-violet-600/30"
            >
              <Download className="w-4 h-4" />
              Download &amp; Get Started
            </button>
          </>
        )}

        {state === 'downloading' && (
          <div className="space-y-5 text-left">
            {needsYtdlp && (
              <div>
                <div className="flex justify-between text-xs text-slate-400 mb-1">
                  <span>yt-dlp{ytdlpProgress?.phase === 'done' ? ' ✓' : ''}</span>
                  <span>
                    {ytdlpProgress ? `${mb(ytdlpProgress.downloaded)} / ${ytdlpProgress.total ? mb(ytdlpProgress.total) + ' MB' : '...'}` : 'waiting...'}
                  </span>
                </div>
                <div className="w-full bg-surface-800 rounded-full h-2 border border-surface-700 overflow-hidden">
                  <div className="h-full bg-gradient-to-r from-violet-600 to-violet-400 rounded-full transition-all duration-300" style={{ width: `${pct(ytdlpProgress)}%` }} />
                </div>
              </div>
            )}
            {needsFfmpeg && (
              <div>
                <div className="flex justify-between text-xs text-slate-400 mb-1">
                  <span>ffmpeg{ffmpegProgress?.phase === 'extracting' ? ' — extracting...' : ffmpegProgress?.phase === 'done' ? ' ✓' : ''}</span>
                  <span>
                    {ffmpegProgress ? `${mb(ffmpegProgress.downloaded)} / ${ffmpegProgress.total ? mb(ffmpegProgress.total) + ' MB' : '...'}` : 'waiting...'}
                  </span>
                </div>
                <div className="w-full bg-surface-800 rounded-full h-2 border border-surface-700 overflow-hidden">
                  <div className="h-full bg-gradient-to-r from-violet-600 to-violet-400 rounded-full transition-all duration-300" style={{ width: `${pct(ffmpegProgress)}%` }} />
                </div>
              </div>
            )}
          </div>
        )}

        {state === 'done' && (
          <>
            <div className="flex items-center justify-center gap-2 mb-4">
              <CheckCircle className="w-6 h-6 text-emerald-400" />
              <span className="text-lg text-emerald-400 font-medium">Setup complete!</span>
            </div>
            <button
              onClick={() => setState('ready')}
              className="flex items-center gap-2 mx-auto px-6 py-3 bg-violet-600 hover:bg-violet-500 text-white text-sm font-medium rounded-xl transition-colors cursor-pointer shadow-lg shadow-violet-600/30"
            >
              Continue
            </button>
          </>
        )}

        {state === 'error' && (
          <>
            <div className="flex items-center justify-center gap-2 mb-4">
              <AlertCircle className="w-6 h-6 text-red-400" />
              <span className="text-lg text-red-400 font-medium">Download failed</span>
            </div>
            <p className="text-xs text-red-400/80 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2 mb-6 break-words">
              {errorMsg}
            </p>
            <p className="text-[11px] text-slate-500 mb-6">
              If antivirus blocked the download, allow it and retry.
            </p>
            <div className="flex gap-3 justify-center">
              <button
                onClick={start}
                className="flex items-center gap-2 px-5 py-2.5 bg-violet-600 hover:bg-violet-500 text-white text-sm font-medium rounded-xl transition-colors cursor-pointer"
              >
                <Download className="w-4 h-4" />
                Retry
              </button>
              <button
                onClick={() => setState('ready')}
                className="px-5 py-2.5 bg-surface-800 border border-surface-700 text-slate-400 hover:text-white text-sm rounded-xl transition-colors cursor-pointer"
              >
                Skip for now
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
```

- [ ] **Step 2: Wrap the app in `BinariesSetup`**

Open `src/App.tsx`. Find the import line:

```tsx
import FirstRunSetup from './components/FirstRunSetup'
```

Add directly below it:

```tsx
import BinariesSetup from './components/BinariesSetup'
```

Then find the JSX opening:

```tsx
  return (
    <FirstRunSetup>
```

and the matching close:

```tsx
    </FirstRunSetup>
  )
}
```

Wrap the existing `FirstRunSetup` with `BinariesSetup`:

```tsx
  return (
    <BinariesSetup>
    <FirstRunSetup>
```

and

```tsx
    </FirstRunSetup>
    </BinariesSetup>
  )
}
```

Order matters: we want binaries first (they're needed to even *analyze* a VOD), then the Whisper model.

- [ ] **Step 3: Smoke-test with binaries already available**

```bash
cd "C:\Users\cereb\Desktop\Claude projects\clipviral" && cargo tauri dev
```

Expected: since ffmpeg/yt-dlp are likely on your system PATH already, the `check_binary_status` call returns `ytdlpAvailable: true, ffmpegAvailable: true, ffprobeAvailable: true`, and `BinariesSetup` renders children immediately — no UI change.

- [ ] **Step 4: Smoke-test the download flow**

Temporarily block the system PATH resolution so the gate triggers:

```bash
# One-time rename — remember to undo at end of task
ren "%LOCALAPPDATA%\Microsoft\WinGet\Links\ffmpeg.exe" "ffmpeg.exe.bak"
```

(Or whichever path is earliest on your PATH — check with `where ffmpeg` first.)

For yt-dlp, rename whatever location `where yt-dlp` returns.

Then run `cargo tauri dev`. Expected: "One-time setup" screen appears, listing both tools. Click "Download". Progress bars should update. When complete, app proceeds to `FirstRunSetup` (whisper model), then the main UI.

Verify the downloaded binaries exist:

```bash
dir "%APPDATA%\clipviral\bin"
```

Should show `ffmpeg.exe`, `ffprobe.exe`, `yt-dlp.exe`.

Load a VOD — it should work end-to-end using the bundled tools.

**UNDO** the rename:

```bash
ren "%LOCALAPPDATA%\Microsoft\WinGet\Links\ffmpeg.exe.bak" "ffmpeg.exe"
```

- [ ] **Step 5: Commit**

```bash
git add src/components/BinariesSetup.tsx src/App.tsx
git commit -m "ui: add BinariesSetup gate for first-run ffmpeg/yt-dlp download"
```

---

## Task 7: Version bump and final verification

**Files:**
- Modify: `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json` (all via `bump-version.ps1`)

- [ ] **Step 1: Bump version to 1.1.0**

From the project root (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -File bump-version.ps1 1.1.0
```

This is a user-visible new feature, not a bug fix, so bump minor (1.0.8 → 1.1.0).

- [ ] **Step 2: Full `cargo check`**

```bash
cd src-tauri && cargo check
```

Expected: clean compile.

- [ ] **Step 3: Full regression smoke test**

Run `cargo tauri dev` and exercise the acceptance criteria:

1. Fresh state (bundled binaries missing, no system PATH): download UI shows → downloads succeed → app works. ✓
2. Binaries already in bundled bin dir: no download UI → app works. ✓
3. Binaries on system PATH only: no download UI → app works using system copies. ✓
4. Intentionally disable network mid-download → error with Retry button → Retry works once network restored. ✓
5. Existing features unaffected: VOD download, analyze, clip export, caption generation. ✓

Watch terminal for `[bin_manager]` log lines confirming path resolution per binary.

- [ ] **Step 4: Commit everything**

```bash
git add -A
git commit -m "feat: auto-download ffmpeg and yt-dlp on first launch"
git push origin main
```

---

## Self-Review Notes

- **Spec coverage:** Steps 1–5 in the original spec map to Tasks 2–6 of this plan. Step 5 (error handling) is covered inline: retry button in Task 6 Step 1, partial-file cleanup via `remove_file` on stream error and always-cleanup of the zip in Task 3 Step 3.
- **Priority chain:** Implemented in `bin_manager.rs` at Task 2 Step 1 — bundled → PATH → error. Users with existing system installs are unaffected (verified in Task 5 Step 5 and Task 6 Step 3).
- **Type consistency:** `BinaryStatus` fields are camelCase'd via serde in both Rust struct and TS interface. `Phase` enum uses lowercase to match TS literal types.
- **No placeholders:** Every step either shows the exact code, the exact edit location, or the exact command.
- **`Fn` vs `FnMut`:** `ProgressCb` is `Box<dyn Fn(u64, u64) + Send + Sync>`. The closures capture `window.clone()` by move and only call `.emit` (which takes `&self`), so `Fn` is the correct bound.
- **Gotcha:** FirstRunSetup.tsx's `ModelStatus` interface uses `size_bytes` (snake_case) unlike the backend's likely camelCase serde default. That's a pre-existing quirk — don't touch it for this plan.
