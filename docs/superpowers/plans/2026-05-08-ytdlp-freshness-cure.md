# v1.3.14 — yt-dlp Freshness Cure + Download-Failure Visibility — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep the bundled yt-dlp current automatically (and via a one-click card action) so Twitch-extractor breakage self-heals, and make download failures visible instead of silently producing unplayable position-heuristic clips.

**Architecture:** Reuse the existing tested `bin_manager::download_ytdlp()`. Add a staleness-gated background refresh on startup (7-day gate, timestamp in the `settings` k/v store — no DB migration), a `force_refresh_ytdlp` Tauri command exposed via a failed-VOD-card "Update yt-dlp & Retry" action, bounded yt-dlp stderr capture, failure-reason logging/emission, and an analysis guard that refuses position-heuristic fallback on a `download_status == "failed"` VOD.

**Tech Stack:** Rust + Tauri 2 backend, React + TypeScript frontend (Zustand), existing `chrono` (RFC3339), existing `tauri-plugin-log`.

---

## Spec reference

Implements `docs/superpowers/specs/2026-05-08-ytdlp-freshness-cure-design.md`. Single cohesive subsystem — one plan.

## VM / build constraint (read first)

Per project CLAUDE.md rule #5: **`cargo` cannot run in this VM.** Therefore:

- **Rust tasks:** the implementer writes code + unit tests, does careful static review, and commits. The implementer does **NOT** run `cargo`. TDD "verify fail / verify pass" gates for Rust are deferred to Task 8 (Slug runs the real suite).
- **Frontend task (Task 7):** `npm run build` **does** run in this VM (used successfully in v1.3.13) — the implementer runs it.
- **Task 8** is Slug-side: real `cargo check` + `cargo test` + `cargo tauri dev` smoke + version bump + tag + push.

Working on `main` is the established pattern for this project (prior three releases shipped that way). Do **not** bump the version until Task 8.

## Pre-flight: confirm these symbols (line numbers may drift)

- `src-tauri/src/bin_manager.rs`: `pub fn ytdlp_path()`, `fn bundled_path(name)`, `pub fn bin_dir()`, `pub async fn download_ytdlp(progress: &ProgressCb)`, `const YTDLP_URL`, `pub type ProgressCb`.
- `src-tauri/src/db.rs`: `pub fn get_setting(conn, key) -> SqliteResult<Option<String>>` (~402), `pub fn save_setting(conn, key, value) -> SqliteResult<()>` (~392), `pub fn get_vod_by_id(conn, id) -> SqliteResult<Option<VodRow>>` (~559), `pub struct VodRow { ... pub download_status: String, pub local_path: Option<String> }` (~261).
- `src-tauri/src/lib.rs`: `pub(crate) type DbConn = Mutex<Connection>` (~40), `pub(crate) fn report_error(app: &AppHandle, err: AppError) -> String` (~44), `.invoke_handler(tauri::generate_handler![ ... download_binaries, ])` (~160-236), `.setup(|app| { ... Ok(()) })` (~237-253).
- `src-tauri/src/commands/binaries.rs`: `download_binaries(window: Window)` + `Progress`/`Phase` structs + `"download-progress"` event (whole file, 77 lines).
- `src-tauri/src/commands/vod.rs`: stderr discard loop (~213-216), exit handling (~236-241), post-download match `_ =>` arm (~305-309), `pub async fn analyze_vod(...)` (~1159), `has_local_file` (~1204), `// Tier 2: Position heuristic` (~1253-1257).
- `src/pages/Vods.tsx`: `function v4StatusClass` (~81), `function v4StatusLabel` (~89), `const handleDownload` (~190), action-row Download button (~637-645).

If anything moved, locate by symbol search before editing.

---

## Task 1: `bin_manager` — staleness predicate + gated/forced refresh (TDD on the pure part)

**Files:**
- Modify: `src-tauri/src/bin_manager.rs`

- [ ] **Step 1.1: Write the failing unit test for the staleness predicate**

Append inside the existing `#[cfg(test)] mod tests { ... }` block at the bottom of `src-tauri/src/bin_manager.rs` (it already exists — the `download_real` ignored test is there):

```rust
    #[test]
    fn ytdlp_is_stale_logic() {
        use chrono::{Duration, Utc};
        // No prior refresh recorded → stale.
        assert!(ytdlp_is_stale(None, 7));
        // Refreshed just now → fresh.
        let now = Utc::now().to_rfc3339();
        assert!(!ytdlp_is_stale(Some(&now), 7));
        // Refreshed 3 days ago, 7-day threshold → fresh.
        let three_days = (Utc::now() - Duration::days(3)).to_rfc3339();
        assert!(!ytdlp_is_stale(Some(&three_days), 7));
        // Refreshed 8 days ago, 7-day threshold → stale.
        let eight_days = (Utc::now() - Duration::days(8)).to_rfc3339();
        assert!(ytdlp_is_stale(Some(&eight_days), 7));
        // Unparseable timestamp → treat as stale (safe default).
        assert!(ytdlp_is_stale(Some("not-a-date"), 7));
    }
```

- [ ] **Step 1.2: (Slug-deferred) note expected failure**

Implementer does NOT run cargo. Record expected: COMPILE ERROR `cannot find function 'ytdlp_is_stale'`. Task 8 confirms.

- [ ] **Step 1.3: Implement the pure predicate**

In `src-tauri/src/bin_manager.rs`, add near the top of the `// ── Paths ──` section (after `fn bundled_path`), this pure function:

```rust
/// Returns `true` if the bundled yt-dlp should be refreshed: no prior
/// refresh recorded, an unparseable timestamp, or the last refresh is
/// older than `threshold_days`. Pure — caller supplies the stored value.
pub fn ytdlp_is_stale(last_refresh_rfc3339: Option<&str>, threshold_days: i64) -> bool {
    match last_refresh_rfc3339 {
        None => true,
        Some(s) => match chrono::DateTime::parse_from_rfc3339(s) {
            Ok(ts) => {
                let age = chrono::Utc::now().signed_duration_since(ts.with_timezone(&chrono::Utc));
                age > chrono::Duration::days(threshold_days)
            }
            Err(_) => true,
        },
    }
}
```

(`chrono` is already a workspace dependency — `bug_report.rs` uses `chrono::Local::now()` and `vod.rs` uses `chrono::Utc::now().to_rfc3339()`. If `bin_manager.rs` lacks a `chrono` import, fully-qualified `chrono::` paths as written above need no `use`.)

- [ ] **Step 1.4: Add the staleness-gated and forced refresh orchestrators**

Still in `src-tauri/src/bin_manager.rs`, add after `pub async fn download_ytdlp(...)` (so it's co-located with the download it wraps):

```rust
/// Settings key holding the RFC3339 timestamp of the last successful
/// bundled-yt-dlp refresh.
pub const YTDLP_LAST_REFRESH_KEY: &str = "ytdlp_last_refresh";

/// Force a yt-dlp refresh regardless of staleness. Downloads the latest
/// release over the bundled binary and returns Ok on success. Caller is
/// responsible for recording the refresh timestamp (it owns the DB conn).
pub async fn force_refresh_ytdlp(progress: &ProgressCb) -> Result<(), AppError> {
    log::info!("[bin_manager] force-refreshing yt-dlp");
    download_ytdlp(progress).await
}

/// Background startup refresh: only acts when a *bundled* yt-dlp exists
/// AND the recorded last refresh is missing/older than 7 days. Never
/// errors hard — a stale binary still beats no binary. Returns `true`
/// if a refresh was performed (so the caller can record the timestamp).
pub async fn refresh_ytdlp_if_stale(last_refresh_rfc3339: Option<String>) -> bool {
    // Strictly gate on a bundled binary — never touch a user's PATH yt-dlp.
    if bundled_path("yt-dlp.exe").is_none() {
        log::info!("[bin_manager] no bundled yt-dlp; skipping staleness refresh (system-PATH user)");
        return false;
    }
    if !ytdlp_is_stale(last_refresh_rfc3339.as_deref(), 7) {
        log::info!("[bin_manager] bundled yt-dlp is fresh; no refresh needed");
        return false;
    }
    log::info!("[bin_manager] bundled yt-dlp is stale; refreshing in background");
    let noop: ProgressCb = Box::new(|_, _| {});
    match download_ytdlp(&noop).await {
        Ok(()) => {
            log::info!("[bin_manager] background yt-dlp refresh complete");
            true
        }
        Err(e) => {
            log::warn!("[bin_manager] background yt-dlp refresh failed (keeping existing binary): {}", e);
            false
        }
    }
}
```

- [ ] **Step 1.5: Static self-review + commit**

Verify: `ytdlp_is_stale` is pure (no IO); `refresh_ytdlp_if_stale` returns early when no bundled binary; failure path logs `warn!` and returns `false` (does not delete the existing binary); `YTDLP_LAST_REFRESH_KEY` is `pub`.

```bash
git add src-tauri/src/bin_manager.rs
git commit -m "feat(bin_manager): yt-dlp staleness predicate + gated/forced refresh"
```

---

## Task 2: Startup background refresh hook in `lib.rs`

**Files:**
- Modify: `src-tauri/src/lib.rs` — inside the existing `.setup(|app| { ... })` closure (~237-253)

- [ ] **Step 2.1: Add the background refresh task**

Find the existing `.setup(|app| {` closure. It currently ends with the upload scheduler `std::thread::spawn` then `Ok(())`. Insert this block immediately **before** the final `Ok(())`:

```rust
            // Background: keep the bundled yt-dlp fresh so Twitch-extractor
            // breakage self-heals. Non-blocking; gated on a bundled binary
            // existing and >7 days since last refresh. Never blocks startup.
            // See bin_manager::refresh_ytdlp_if_stale.
            let ytdlp_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Read the stored timestamp under a short-lived lock, then
                // release it BEFORE the (network, ~20MB) download.
                let last_refresh: Option<String> = {
                    let state = ytdlp_handle.state::<crate::DbConn>();
                    match state.lock() {
                        Ok(conn) => crate::db::get_setting(&conn, crate::bin_manager::YTDLP_LAST_REFRESH_KEY)
                            .ok()
                            .flatten(),
                        Err(_) => None,
                    }
                };
                let refreshed = crate::bin_manager::refresh_ytdlp_if_stale(last_refresh).await;
                if refreshed {
                    let now = chrono::Utc::now().to_rfc3339();
                    let state = ytdlp_handle.state::<crate::DbConn>();
                    if let Ok(conn) = state.lock() {
                        let _ = crate::db::save_setting(
                            &conn,
                            crate::bin_manager::YTDLP_LAST_REFRESH_KEY,
                            &now,
                        );
                    }
                }
            });

```

(`State`/`Manager` are already imported in `lib.rs` — `use tauri::{AppHandle, Manager, State};` at ~35. `app.handle()` and `.state::<T>()` are available. `tauri::async_runtime::spawn` is the Tauri 2 runtime spawn — `download_ytdlp` is `async`. The DB lock is acquired twice for short windows and never held across the `.await`.)

- [ ] **Step 2.2: Static self-review + commit**

Verify: the block is inside the `.setup` closure, before `Ok(())`; the DB lock is **not** held across `.await`; failures are swallowed (best-effort); nothing blocks the synchronous setup path.

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(startup): background staleness-gated yt-dlp refresh"
```

---

## Task 3: `force_refresh_ytdlp` command + handler registration

**Files:**
- Modify: `src-tauri/src/commands/binaries.rs`
- Modify: `src-tauri/src/lib.rs` (register in `generate_handler![]`)

- [ ] **Step 3.1: Add the command**

In `src-tauri/src/commands/binaries.rs`, add at the end of the file (after `download_binaries`). Note the added imports at top.

Change the top imports from:

```rust
use serde::Serialize;
use tauri::{Emitter, Window};

use crate::bin_manager::{self, BinaryStatus, ProgressCb};
```

to:

```rust
use serde::Serialize;
use tauri::{Emitter, State, Window};

use crate::bin_manager::{self, BinaryStatus, ProgressCb};
use crate::DbConn;
```

Append this command at the end of the file:

```rust
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
```

- [ ] **Step 3.2: Register the command**

In `src-tauri/src/lib.rs`, find the `.invoke_handler(tauri::generate_handler![ ... ])` list. The line `download_binaries,` exists near the end (~235). Add `force_refresh_ytdlp,` immediately after it:

```rust
            check_binary_status,
            download_binaries,
            force_refresh_ytdlp,
        ])
```

The commands module re-exports via `commands/mod.rs` (pattern: `download_binaries` is already unqualified in the handler list, so `force_refresh_ytdlp` must be re-exported the same way). Confirm `commands/mod.rs` re-exports binaries' public items (it does for `download_binaries`); if it uses an explicit list, add `force_refresh_ytdlp` there too.

- [ ] **Step 3.3: Static self-review + commit**

Verify: imports added (`State`, `DbConn`); command emits `download-progress` with `binary: "yt-dlp"`; timestamp written on success only; registered in the handler list AND re-exported in `commands/mod.rs` if that file uses an explicit re-export list.

```bash
git add src-tauri/src/commands/binaries.rs src-tauri/src/lib.rs src-tauri/src/commands/mod.rs
git commit -m "feat(commands): force_refresh_ytdlp command + handler registration"
```

---

## Task 4: Bug C — bounded yt-dlp stderr capture (TDD on the buffer)

**Files:**
- Modify: `src-tauri/src/commands/vod.rs`

- [ ] **Step 4.1: Write the failing unit test for the bounded buffer**

Locate (or create, at the very bottom of the file) the `#[cfg(test)] mod tests { use super::*; ... }` block in `src-tauri/src/commands/vod.rs` (it exists — the Phase A hallucination tests live there). Append:

```rust
    #[test]
    fn stderr_tail_keeps_only_last_n_lines() {
        let mut t = StderrTail::new(3);
        for i in 0..10 {
            t.push(format!("line {i}"));
        }
        // Only the last 3 lines retained, in order.
        assert_eq!(t.joined(), "line 7\nline 8\nline 9");
    }

    #[test]
    fn stderr_tail_handles_fewer_than_cap() {
        let mut t = StderrTail::new(5);
        t.push("only".to_string());
        assert_eq!(t.joined(), "only");
    }

    #[test]
    fn stderr_tail_empty_is_empty_string() {
        let t = StderrTail::new(4);
        assert_eq!(t.joined(), "");
    }
```

- [ ] **Step 4.2: (Slug-deferred) note expected failure**

Expected: COMPILE ERROR `cannot find type 'StderrTail'`. Task 8 confirms.

- [ ] **Step 4.3: Implement the bounded buffer**

In `src-tauri/src/commands/vod.rs`, add near the top of the file (after the `use` block, before the first function — co-located with the download code that uses it):

```rust
/// A fixed-capacity ring of the most recent stderr lines. Used to keep a
/// bounded tail of yt-dlp's diagnostic output without buffering its full
/// (potentially large, verbose) stderr stream in memory.
struct StderrTail {
    cap: usize,
    lines: std::collections::VecDeque<String>,
}

impl StderrTail {
    fn new(cap: usize) -> Self {
        Self { cap, lines: std::collections::VecDeque::with_capacity(cap) }
    }
    fn push(&mut self, line: String) {
        if self.lines.len() == self.cap {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }
    fn joined(&self) -> String {
        self.lines.iter().cloned().collect::<Vec<_>>().join("\n")
    }
}
```

- [ ] **Step 4.4: Wire capture into the stderr thread + surface in the error**

Find the discard loop (~213-216):

```rust
            let stderr = child.stderr.take();
            let stderr_thread = std::thread::spawn(move || {
                if let Some(err) = stderr {
                    let reader = BufReader::new(err);
                    for _ in reader.lines() {}
                }
            });
```

Replace with:

```rust
            // Phase v1.3.14 (Bug C): keep a bounded tail of yt-dlp stderr so
            // download failures are diagnosable instead of swallowed.
            let stderr = child.stderr.take();
            let stderr_thread = std::thread::spawn(move || {
                let mut tail = StderrTail::new(80);
                if let Some(err) = stderr {
                    let reader = BufReader::new(err);
                    for line in reader.lines().map_while(Result::ok) {
                        tail.push(line);
                    }
                }
                tail.joined()
            });
```

Then find where the thread is joined and the exit status handled (~235-241):

```rust
            let _ = stderr_thread.join();
            let status = child.wait().map_err(|e| format!("yt-dlp error: {}", e))?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("yt-dlp exited with code: {:?}", status.code()))
            }
```

Replace with:

```rust
            let stderr_tail = stderr_thread.join().unwrap_or_default();
            let status = child.wait().map_err(|e| format!("yt-dlp error: {}", e))?;
            if status.success() {
                Ok(())
            } else {
                Err(format!(
                    "yt-dlp exited with code {:?}\n--- yt-dlp stderr (tail) ---\n{}",
                    status.code(),
                    stderr_tail
                ))
            }
```

- [ ] **Step 4.5: Static self-review + commit**

Verify: `StderrTail` cap is 80; `map_while(Result::ok)` drops only unreadable lines; the joined tail is included in the non-zero-exit `Err`; no unbounded buffering.

```bash
git add src-tauri/src/commands/vod.rs
git commit -m "feat(vod): capture bounded yt-dlp stderr tail (Bug C)"
```

---

## Task 5: Bug D-A — capture, log, emit the download-failure reason

**Files:**
- Modify: `src-tauri/src/commands/vod.rs`

- [ ] **Step 5.1: Replace the reason-swallowing match arm**

Find the post-download match (~247-310). The failure arm is currently:

```rust
            _ => {
                if let Ok(conn) = db.lock() {
                    db::update_vod_download_status(&conn, &vod_id_status, "failed", None, None).ok();
                }
            }
```

Replace **only that `_ => { ... }` arm** with:

```rust
            other => {
                // Phase v1.3.14 (Bug D-A): extract the real reason (now
                // meaningful thanks to Bug C's stderr capture), log it, and
                // emit it to the frontend instead of silently swallowing it.
                let reason = match other {
                    Ok(Err(msg)) => msg,
                    Err(join_err) => format!("download task panicked/join error: {join_err}"),
                    Ok(Ok(())) => unreachable!("Ok(Ok(())) handled by the success arm above"),
                };
                log::error!("[download_vod] failed for {}: {}", vod_id_status, reason);
                if let Ok(conn) = db.lock() {
                    db::update_vod_download_status(&conn, &vod_id_status, "failed", None, None).ok();
                }
                use tauri::Emitter;
                let _ = app_handle.emit(
                    "vod-download-failed",
                    serde_json::json!({ "vodId": vod_id_status, "reason": reason }),
                );
            }
```

(`app_handle` is in scope in this spawned task — confirmed at the line `let db: State<'_, DbConn> = app_handle.state();` just above the match. `serde_json` is a workspace dep used throughout `vod.rs`. The success arm `Ok(Ok(())) => { ... }` remains unchanged above this arm; replacing `_ =>` with `other =>` is exhaustive because `result` is `Result<Result<(), String>, JoinError>` and `Ok(Ok(()))` is the only other variant — it is handled by the existing success arm, so `unreachable!` there is correct.)

- [ ] **Step 5.2: Static self-review + commit**

Verify: the success arm above is untouched; `other` matches `Ok(Err(_))` and `Err(_)`; `log::error!` includes vod id + reason; `download_status` still set to `"failed"`; event name `"vod-download-failed"` with `vodId` + `reason`.

```bash
git add src-tauri/src/commands/vod.rs
git commit -m "feat(vod): capture/log/emit download-failure reason (Bug D-A)"
```

---

## Task 6: Bug D-B — guard analysis against failed-download VODs

**Files:**
- Modify: `src-tauri/src/commands/vod.rs`

- [ ] **Step 6.1: Write the failing unit test for the guard predicate**

Append to the `#[cfg(test)] mod tests` block in `src-tauri/src/commands/vod.rs`:

```rust
    #[test]
    fn blocks_position_fallback_only_for_failed_download() {
        assert!(should_block_position_fallback("failed"));
        assert!(!should_block_position_fallback("pending"));
        assert!(!should_block_position_fallback("downloaded"));
        assert!(!should_block_position_fallback("downloading"));
        assert!(!should_block_position_fallback(""));
    }
```

- [ ] **Step 6.2: (Slug-deferred) note expected failure**

Expected: COMPILE ERROR `cannot find function 'should_block_position_fallback'`. Task 8 confirms.

- [ ] **Step 6.3: Implement the predicate**

In `src-tauri/src/commands/vod.rs`, add near `StderrTail` (top of file, helper region):

```rust
/// A VOD whose download explicitly failed must not silently fall through
/// to position-heuristic clips (which can't play — no source file). Only
/// the explicit "failed" status is blocked; genuinely-not-downloaded VODs
/// (pending/etc.) keep the legitimate position-heuristic fallback.
fn should_block_position_fallback(download_status: &str) -> bool {
    download_status == "failed"
}
```

- [ ] **Step 6.4: Apply the guard before the Tier-2 fallback**

Find the Tier-2 block in `analyze_vod` (~1253-1257):

```rust
        // Tier 2: Position heuristic (always available)
        if result.is_err() {
            log::info!("Running position fallback for VOD {} (ffmpeg={}, downloaded={})",
                vod_id_bg, has_ffmpeg, has_local_file);
            if let Ok(conn) = db.lock() {
```

Insert the guard immediately **before** `// Tier 2: Position heuristic` (i.e., after Tier 1 finishes, before the Tier-2 `if result.is_err()`):

```rust
        // Phase v1.3.14 (Bug D-B): if the download explicitly failed, do
        // NOT produce position-heuristic clips (they have no source video
        // and can't play). Surface the real problem instead.
        if result.is_err() && should_block_position_fallback(&vod_clone.download_status) {
            log::error!(
                "[analyze_vod] refusing position fallback for {} — download_status=failed",
                vod_id_bg
            );
            if let Ok(conn) = db.lock() {
                db::update_vod_analysis_status(&conn, &vod_id_bg, "failed").ok();
            }
            use tauri::Emitter;
            let _ = app.emit(
                "vod-analysis-failed",
                serde_json::json!({
                    "vodId": vod_id_bg,
                    "reason": "VOD download failed — use 'Update yt-dlp & Retry' on the VOD before analyzing."
                }),
            );
            return;
        }

        // Tier 2: Position heuristic (always available)
```

Notes for the implementer:
- `vod_clone` is the `db::VodRow` already cloned for the background task (used at `vod_clone.local_path` ~1204/1388); it has `.download_status: String`.
- `app` is the `AppHandle` parameter of `analyze_vod` (`pub async fn analyze_vod(vod_id, app, db, hw)` ~1159) and is used elsewhere in this task (e.g. `app.emit("auto-ship-queued", ...)` ~1378).
- `db::update_vod_analysis_status(conn, id, status)` is the existing helper used right below in the Tier-2 block — confirm its exact signature there and match it. If the analysis-status setter has a different name/arity at the Tier-2 site, use whatever that site uses.
- The early `return;` exits the spawned analysis closure (the surrounding code is the same `tokio`/spawn closure that runs Tier 1/Tier 2 — confirm it returns `()`; if it returns a `Result`, use the matching early-return form already used elsewhere in the closure).

- [ ] **Step 6.5: Static self-review + commit**

Verify: guard fires only when `result.is_err()` AND `download_status == "failed"`; sets analysis status failed; emits `vod-analysis-failed`; early-returns before Tier-2; legit fallback for non-failed VODs is untouched.

```bash
git add src-tauri/src/commands/vod.rs
git commit -m "feat(vod): guard analysis against failed-download VODs (Bug D-B)"
```

---

## Task 7: Frontend — failed-download card state + "Update yt-dlp & Retry" action

**Files:**
- Modify: `src/pages/Vods.tsx`

- [ ] **Step 7.1: Add the failed-download status mappings**

In `src/pages/Vods.tsx`, find `function v4StatusClass` (~81):

```typescript
function v4StatusClass(vod: { analysis_status: string; download_status: string }): string {
  if (vod.analysis_status === 'completed') return 'done'
  if (vod.analysis_status === 'analyzing') return 'analyzing'
  if (vod.analysis_status === 'failed') return 'failed'
  if (vod.download_status === 'downloading' || vod.download_status === 'downloaded') return 'queued'
  return 'queued'
}
```

Add a `download_status === 'failed'` case (before the downloading/downloaded line):

```typescript
function v4StatusClass(vod: { analysis_status: string; download_status: string }): string {
  if (vod.analysis_status === 'completed') return 'done'
  if (vod.analysis_status === 'analyzing') return 'analyzing'
  if (vod.analysis_status === 'failed') return 'failed'
  if (vod.download_status === 'failed') return 'failed'
  if (vod.download_status === 'downloading' || vod.download_status === 'downloaded') return 'queued'
  return 'queued'
}
```

Then find `function v4StatusLabel` (~89):

```typescript
function v4StatusLabel(vod: { analysis_status: string; analysis_progress?: number; download_status: string; download_progress?: number }): string {
  if (vod.analysis_status === 'completed') return 'COMPLETE'
  if (vod.analysis_status === 'analyzing') return `ANALYZING · ${vod.analysis_progress ?? 0}%`
  if (vod.analysis_status === 'failed') return 'FAILED · RETRY'
  if (vod.download_status === 'downloading') return `DOWNLOADING · ${vod.download_progress ?? 0}%`
  if (vod.download_status === 'downloaded') return 'READY TO ANALYZE'
  return 'PENDING'
}
```

Add the failed-download label (before the downloading line):

```typescript
function v4StatusLabel(vod: { analysis_status: string; analysis_progress?: number; download_status: string; download_progress?: number }): string {
  if (vod.analysis_status === 'completed') return 'COMPLETE'
  if (vod.analysis_status === 'analyzing') return `ANALYZING · ${vod.analysis_progress ?? 0}%`
  if (vod.analysis_status === 'failed') return 'FAILED · RETRY'
  if (vod.download_status === 'failed') return 'DOWNLOAD FAILED · RETRY'
  if (vod.download_status === 'downloading') return `DOWNLOADING · ${vod.download_progress ?? 0}%`
  if (vod.download_status === 'downloaded') return 'READY TO ANALYZE'
  return 'PENDING'
}
```

- [ ] **Step 7.2: Add the combined handler**

Find `const handleDownload = async (vodId: string) => { ... }` (~190). Immediately after that function, add:

```typescript
  const [updatingYtdlpVodId, setUpdatingYtdlpVodId] = useState<string | null>(null)

  const handleUpdateYtdlpAndRetry = async (vodId: string) => {
    if (updatingYtdlpVodId) return // double-click guard
    setUpdatingYtdlpVodId(vodId)
    try {
      await invoke('force_refresh_ytdlp')
      await invoke('download_vod', { vodId })
    } catch (err) {
      alert(`Update & retry failed: ${err}`)
    } finally {
      setUpdatingYtdlpVodId(null)
      if (loggedInUser) refreshVods(loggedInUser.id)
    }
  }
```

(`useState` is already imported in `Vods.tsx` — `handleDownload` uses `loggedInUser`/`refreshVods` so they're in scope here too. Verify `useState` is in the existing React import; it is — other state hooks exist in this component.)

- [ ] **Step 7.3: Render the action on the failed-download card**

Find the action-row Download button (~637-645):

```typescript
                  {vod.download_status === 'downloaded' ? (
                    <button
                      onClick={() => navigate(`/player/${vod.id}`)}
                      className="flex-1 flex items-center justify-center gap-1.5 px-3 py-2 text-xs rounded-lg transition-colors cursor-pointer bg-emerald-500/20 text-emerald-400 border border-emerald-500/30 hover:bg-emerald-500/30"
                    >
```

…and its `) : (` Download `<button>` branch (the `else` that calls `handleDownload`). Change that two-branch conditional into three branches by inserting a `download_status === 'failed'` branch FIRST. Replace:

```typescript
                  {vod.download_status === 'downloaded' ? (
```

with:

```typescript
                  {vod.download_status === 'failed' ? (
                    <button
                      onClick={() => handleUpdateYtdlpAndRetry(vod.id)}
                      disabled={updatingYtdlpVodId === vod.id}
                      className="flex-1 flex items-center justify-center gap-1.5 px-3 py-2 text-xs rounded-lg transition-colors cursor-pointer bg-amber-500/20 text-amber-400 border border-amber-500/30 hover:bg-amber-500/30 disabled:opacity-40"
                      title="Twitch download failed — usually a stale yt-dlp. This updates yt-dlp and retries."
                    >
                      {updatingYtdlpVodId === vod.id ? 'Updating yt-dlp…' : 'Update yt-dlp & Retry'}
                    </button>
                  ) : vod.download_status === 'downloaded' ? (
```

(This converts the existing `A ? X : Y` into `failed ? NEW : downloaded ? X : Y`. Leave the existing `downloaded` `<button>` and the trailing `) : ( <download button> )}` exactly as they are — you are only inserting the new first branch and changing `{vod.download_status === 'downloaded' ? (` to `) : vod.download_status === 'downloaded' ? (` at the seam. Read the full existing ternary first and preserve its closing `)}`.)

- [ ] **Step 7.4: Verify the build**

Run from the project root:

```
npm run build
```

Expected: `tsc -b && vite build` completes with no errors. (This step DOES run in this VM.)

- [ ] **Step 7.5: Commit**

```bash
git add src/pages/Vods.tsx
git commit -m "feat(ui): failed-download card state + Update yt-dlp & Retry action"
```

---

## Task 8: Slug verification + ship v1.3.14

**Files:** version-bump files only (package.json, src-tauri/Cargo.toml, src-tauri/Cargo.lock, src-tauri/tauri.conf.json).

This task is performed by Slug (cargo + live app required; not available in the VM).

- [ ] **Step 8.1: Compile + unit tests**

```
cd src-tauri
cargo check
cargo test ytdlp_is_stale stderr_tail blocks_position_fallback -- --nocapture
cargo test 2>&1 | tail -15
```

Expected: `cargo check` finished, same pre-existing warning count as v1.3.13, 0 errors. The three new unit tests pass. Full suite: previous total + 7 new tests (1 `ytdlp_is_stale` + 3 `stderr_tail_*` + 1 `blocks_position_fallback` + the two extra `stderr_tail` cases), 0 failed.

If any new test fails, STOP — return the failure to the controller (do not bump version).

- [ ] **Step 8.2: Live smoke — staleness refresh path**

```
cd ..
cargo tauri dev
```

In a second terminal, force the staleness gate open by removing the timestamp:

```powershell
sqlite3 "$env:APPDATA\clipviral\clipviral.db" "DELETE FROM settings WHERE key='ytdlp_last_refresh';"
```

Restart the app. In the log (terminal stdout), expect within seconds of startup either:
- `[bin_manager] no bundled yt-dlp; skipping staleness refresh (system-PATH user)` — **if you (Slug) have no bundled yt-dlp** (your dev machine uses system PATH). This is correct behavior; to exercise the real path, see 8.3.
- or `[bin_manager] bundled yt-dlp is stale; refreshing in background` → `[bin_manager] background yt-dlp refresh complete`.

App must remain responsive throughout (refresh is non-blocking).

- [ ] **Step 8.3: Live smoke — card "Update yt-dlp & Retry" path**

To exercise the card action without a real failing VOD, simulate a failed download:

```powershell
# pick any VOD id from the library
sqlite3 "$env:APPDATA\clipviral\clipviral.db" "UPDATE vods SET download_status='failed', local_path=NULL WHERE id=(SELECT id FROM vods LIMIT 1);"
```

Reload the Vods page. Expected: that VOD card shows status **"DOWNLOAD FAILED · RETRY"** with a `failed` style, and the action row shows an amber **"Update yt-dlp & Retry"** button. Click it: button shows "Updating yt-dlp…", a yt-dlp download runs (progress via the existing `download-progress` event path), then `download_vod` is invoked and the card returns to a normal downloading/downloaded flow. Clicking twice while running is a no-op (disabled).

- [ ] **Step 8.4: Live smoke — analysis guard**

With a VOD still at `download_status='failed'` (re-set it via the SQL in 8.3 if needed), trigger analyze on it. Expected: it does **not** produce clips; the analysis ends failed with the reason surfaced ("VOD download failed — use 'Update yt-dlp & Retry'…") via the `vod-analysis-failed` event / FAILED state, and the log shows `[analyze_vod] refusing position fallback for <id> — download_status=failed`.

- [ ] **Step 8.5: Regression check**

A healthy VOD: download → analyze → clips play. Unchanged. The position-heuristic fallback still works for a `pending` (never-downloaded) VOD (do NOT block that path).

- [ ] **Step 8.6: Version bump + ship**

```powershell
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
powershell -File bump-version.ps1 1.3.14
git add package.json src-tauri/Cargo.lock src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -m "chore: bump to v1.3.14 (yt-dlp freshness cure + download-failure visibility)"
git tag -a v1.3.14 -m "v1.3.14 -- yt-dlp freshness cure + download-failure visibility"
git push origin main
git push origin v1.3.14
```

GitHub Actions builds the draft release. Publish with user-facing notes (this is a real user-facing fix — unlike v1.3.13): summarize "Twitch downloads now self-heal (auto-updating yt-dlp); failed downloads show a clear 'Update yt-dlp & Retry' action instead of producing unplayable clips."

---

## Self-review

(Plan author, fresh eyes against the spec.)

### Spec coverage

- **§3.1 staleness-gated background refresh** → Task 1 (`ytdlp_is_stale`, `refresh_ytdlp_if_stale`, `YTDLP_LAST_REFRESH_KEY`) + Task 2 (startup hook, lock-not-held-across-await, timestamp write). ✓
- **§3.2 card "Update yt-dlp & Retry" (force, bypass gate, double-click guard, no Settings button)** → Task 1 (`force_refresh_ytdlp` in bin_manager) + Task 3 (command) + Task 7 (card branch + `updatingYtdlpVodId` guard + handler force→download). ✓
- **§3.3 minimal Bug C bounded stderr** → Task 4 (`StderrTail`, cap 80, included in Err). ✓
- **§3.4a capture/log/emit reason** → Task 5 (`other =>` arm, `log::error!`, `vod-download-failed` emit, status still failed). ✓
- **§3.4b analysis guard (only explicit failed)** → Task 6 (`should_block_position_fallback`, guard before Tier-2, early return, legit fallback preserved). ✓
- **§3.4c card surfacing (failed state + action; reason in logs, best-effort emit)** → Task 7 (`v4StatusClass`/`v4StatusLabel` failed mappings + action). Reason persistence intentionally NOT added (spec defers DB column to v1.3.15); reason reaches logs (Task 5) + `vod-download-failed`/`vod-analysis-failed` events. ✓
- **§4 file map** → Tasks touch exactly: bin_manager.rs, lib.rs, commands/binaries.rs (+commands/mod.rs if explicit re-export), commands/vod.rs, src/pages/Vods.tsx, settings k/v key (no migration). ✓
- **§5 watchouts** → startup never blocks (Task 2 spawn, swallow errors); system-PATH gated (Task 1 `bundled_path` check); stale-still-better-than-none (Task 1 failure keeps binary); guard breadth = only "failed" (Task 6). ✓
- **§2 out-of-scope** → no Bug B, no diagnostics button, no console bridge, no DB error column, no `-U`. None appear in any task. ✓

No spec gaps.

### Placeholder scan

No "TBD/TODO/implement later". Every Rust/TS step has the literal code. The two soft references — Task 6 "match whatever the analysis-status setter is named at the Tier-2 site" and "if the closure returns Result, use the matching early-return" — are deliberate: the exact helper name/closure-return at line ~1257 must be confirmed against live code, and the plan tells the implementer precisely how to resolve it rather than guessing a possibly-wrong symbol. Task 3's "if commands/mod.rs uses an explicit re-export list" is likewise a precise conditional instruction, not a placeholder. Task 7's ternary seam instruction repeats the exact before/after strings.

### Type / name consistency

- `ytdlp_is_stale(Option<&str>, i64) -> bool` — defined Task 1.3, used Task 1.4 (`as_deref()`) consistently.
- `refresh_ytdlp_if_stale(Option<String>) -> bool` — defined Task 1.4, called Task 2.1 with `Option<String>` from `get_setting(...).ok().flatten()`, return drives the timestamp write. ✓
- `force_refresh_ytdlp(&ProgressCb) -> Result<(), AppError>` (bin_manager, Task 1.4) vs `force_refresh_ytdlp(window, db) -> Result<(),String>` (command, Task 3.1) — same name, different module (`bin_manager::` vs command). Command calls `bin_manager::force_refresh_ytdlp(&cb)`. Consistent, intentional, namespaced.
- `YTDLP_LAST_REFRESH_KEY` — defined Task 1.4, used Task 2 + Task 3 via `crate::bin_manager::YTDLP_LAST_REFRESH_KEY` / `bin_manager::YTDLP_LAST_REFRESH_KEY`. ✓
- `StderrTail::new/push/joined` — defined Task 4.3, used Task 4.1 tests + Task 4.4 wiring. ✓
- `should_block_position_fallback(&str) -> bool` — defined Task 6.3, used Task 6.1 tests + Task 6.4 guard. ✓
- Event names: `vod-download-failed` (Task 5), `vod-analysis-failed` (Task 6) — distinct, intentional. `download-progress` reused from existing binaries pattern (Task 3). ✓
- `updatingYtdlpVodId` / `handleUpdateYtdlpAndRetry` — defined Task 7.2, used Task 7.3. ✓

No drift detected.
