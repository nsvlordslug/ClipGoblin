# v1.3.14 — yt-dlp Freshness Cure + Download-Failure Visibility — Design

**Status:** Approved in brainstorming; pending written-spec review
**Date:** 2026-05-08
**Target release:** v1.3.14 (hotfix — unbreaks users who can't download VODs)

---

## 1. Background

A user reported "clips not playing." Investigation traced a four-bug cascade:

1. **Root cause:** The user's bundled `yt-dlp.exe` is **stale**. `bin_manager::ytdlp_path()` resolves bundled-first, system-PATH-second. A regular user has no system yt-dlp, so the first-run `BinariesSetup` downloads yt-dlp once into `%APPDATA%\clipviral\bin\` and **it is never updated again** — `ytdlp_path()` returns it forever via `if p.exists()`, with no version/staleness check anywhere in `bin_manager.rs`. Twitch changes its playback API roughly monthly and routinely breaks yt-dlp's Twitch extractor (yt-dlp ships ~weekly to keep up). A months-frozen yt-dlp → Twitch downloads fail. The developer (Slug) can't reproduce because his machine has no bundled binary and uses his own current system-PATH yt-dlp — a literally different binary.
2. **Bug C:** `download_vod` reads yt-dlp's stderr line-by-line and discards every line (`vod.rs:213-216`: `for _ in reader.lines() {}`). On failure the only error is `"yt-dlp exited with code: Some(1)"` — the actual reason is destroyed.
3. **Failure-reason swallow:** The post-download match arm (`vod.rs:305-309`) handles failure with `_ => { ... update_vod_download_status(..., "failed", None, None) }` — it correctly persists `download_status = "failed"` but **discards `result`'s error entirely**. Combined with Bug C, a failed VOD has a "failed" status but zero diagnostic "why" anywhere.
4. **Silent unplayable-clips trap (Bug D surface):** Analysis's Tier-2 position-heuristic fallback (`vod.rs:~1253`, "always available") still runs for a failed-download VOD, producing clips with `local_path = None` that can't play. The user experiences this as "clips won't play" rather than "download failed," which is why the real cause stayed hidden.

**Scope decision (brainstorming):** "Cure first, button next." v1.3.14 = the cure + minimal visibility. The one-click diagnostics button, the bug-reporter log-dir fix (Bug B), and the full Bug D treatment are deferred to v1.3.15's own design cycle.

## 2. Goals & success criteria

### Goals
- Users' bundled yt-dlp stays current automatically, so Twitch-extractor breakage self-heals within days.
- A user hitting the issue today has a one-click, no-terminal way to force-refresh yt-dlp.
- When a download fails, the failure reason is captured, logged, and visible — not destroyed.
- A failed-download VOD does not silently produce unplayable position-heuristic clips.

### Success criteria
- On startup, if the bundled `yt-dlp.exe` exists and its recorded last refresh is missing or >7 days old, it is re-downloaded in the background to the latest release. System-PATH-only users are unaffected (nothing bundled to refresh).
- On a failed-download VOD card, an "Update yt-dlp & Retry" action force-refreshes yt-dlp (bypassing the 7-day staleness gate) and then re-attempts the download, with progress/result feedback.
- A failed `download_vod` writes the captured yt-dlp stderr tail to the log (`log::error!`) and includes it in the error surfaced to the UI.
- Triggering analysis on a VOD whose `download_status == "failed"` does not generate position-heuristic clips; it surfaces the download failure instead.
- No regression: a healthy download + analyze + playback flow is unchanged.

### Out of scope (deferred to v1.3.15)
- One-click "Send Diagnostics" button (zero-typing → GitHub issue via existing pipeline, auto-titled + `diagnostic` label)
- Bug B — bug-reporter log directory fix (`bug_report.rs::log_dir()` uses `dirs::data_dir()` Roaming; tauri-plugin-log writes to `app_log_dir()` Local → "(no log directory found)")
- Full Bug D — rethinking when the position-heuristic fallback is ever appropriate; distinguishing "not downloaded by choice" vs "download failed"; a dedicated `download_error` DB column
- Frontend `console.error` → Rust log bridge
- yt-dlp self-update via `-U` (we deliberately chose app-controlled re-download instead)

## 3. Design

### 3.1 Staleness-gated background yt-dlp refresh (the cure)

Reuse the existing, tested `bin_manager::download_ytdlp()` + `YTDLP_URL` (already in `bin_manager.rs`; has the ignored integration test `download_real`). Do **not** use yt-dlp's own `-U` self-update — we control the refresh entirely and don't depend on the stale binary's update logic.

**Staleness record:** reuse the existing `settings` key/value store (`db::get_setting` / `db::save_setting`) with key `ytdlp_last_refresh` holding an RFC3339 timestamp. No DB migration.

**Logic (new function in `bin_manager.rs`, e.g. `refresh_ytdlp_if_stale`):**
- If no *bundled* `yt-dlp.exe` exists → return immediately (system-PATH users untouched; nothing to manage).
- Read `ytdlp_last_refresh`. If missing OR older than 7 days → call `download_ytdlp()`; on success write `ytdlp_last_refresh = now`. On failure, `log::warn!` and leave the existing binary in place (a stale yt-dlp still beats no yt-dlp; next startup retries).
- 7-day threshold: Twitch breaks ≈monthly, yt-dlp releases ≈weekly; weekly refresh stays ahead without churn.

**Trigger:** call `refresh_ytdlp_if_stale` from the app startup path (`lib.rs` run/setup) inside a non-blocking background task (`tokio::spawn`), so it never delays UI or first interaction. The DB handle / settings access must be available in that task (mirror how other startup background tasks acquire state).

**Concurrency note:** `download_ytdlp()` writes to a `.tmp` then renames into place — an in-progress download attempt elsewhere is unlikely on startup, but the rename-into-place pattern already makes this safe enough for the hotfix. No lock needed for v1.3.14.

### 3.2 "Update yt-dlp & Retry" action on the failed-download VOD card

The contextual cure lives exactly where the user gets stuck: on a VOD card whose `download_status == "failed"`. A single action — labeled "Update yt-dlp & Retry" — does, in order:

1. Force-run `download_ytdlp()` (bypassing the 7-day staleness gate — Twitch can break a yt-dlp that's only days old, so the gate is irrelevant at the point of an actual failure). On success, update `ytdlp_last_refresh = now`.
2. Re-invoke the existing `download_vod` for that VOD.

Progress/result feedback reuses the existing binaries download UX / `ProgressCb` pattern already present for first-run. A backend command (in `commands/binaries.rs`, alongside the existing binary-download commands) exposes the force-refresh; the frontend card action calls force-refresh then `download_vod`.

**Why one combined action, not two buttons:** simplicity (user's explicit call). The overwhelmingly common failure cause is stale yt-dlp, so "update then retry" is the right default and the explicit label teaches the user what the fix is. The ~20 MB refresh cost is acceptable because retries are infrequent (a stuck user clicks once, not in a loop).

**Double-click guard:** if a force-refresh already completed within this app session (or `ytdlp_last_refresh` is within the last hour), skip the re-download and go straight to retry — avoids a redundant 20 MB pull if the user clicks twice.

**No Settings button.** The startup auto-refresh (§3.1) covers the proactive case; this card action covers the reactive case. A standalone Settings updater is YAGNI for the hotfix (revisit in v1.3.15 if support demand shows otherwise).

### 3.3 Minimal Bug C — capture yt-dlp stderr

Replace the discard loop at `vod.rs:213-216`:

```rust
let stderr = child.stderr.take();
let stderr_thread = std::thread::spawn(move || {
    if let Some(err) = stderr {
        let reader = BufReader::new(err);
        for _ in reader.lines() {}            // ← discards everything
    }
});
```

with a **bounded** capture: the stderr thread collects lines into a ring/cap buffer holding at most the last ~80 lines or ~8 KB (whichever is hit first), and returns that tail when joined. On non-zero exit (`vod.rs:~236-241`), the error returned from the blocking task includes the captured stderr tail, e.g. `format!("yt-dlp exited with code {:?}\n--- yt-dlp stderr (tail) ---\n{}", code, stderr_tail)`. The bound prevents verbose yt-dlp output from ballooning memory.

### 3.4 Minimal Bug D — capture, log, surface the reason; guard analysis

**(a) Stop swallowing the reason.** At the post-download match (`vod.rs:305-309`), replace the bare `_ =>` arm with arms that extract the real error from `result` (now meaningful thanks to 3.3):
- `Ok(Err(msg))` → the yt-dlp failure message including the stderr tail
- `Err(join_err)` → the task-join error
Persist `download_status = "failed"` (unchanged) **and** `log::error!("[download_vod] failed for {vod_id}: {reason}")`, **and** surface the reason to the UI via the existing error/event channel used elsewhere in this file (e.g. the `report_error` / event-emit pattern already used at the top of `download_vod`). No new DB column for v1.3.14 (deferred to v1.3.15 full Bug D).

**(b) Guard analysis against failed-download VODs.** At the analysis entry point (the `analyze_vod` command path in `vod.rs`), before falling through to the Tier-2 position-heuristic fallback, check the VOD's `download_status`. If it is `"failed"`, do **not** produce position-heuristic clips — return/emit a clear error ("VOD download failed — retry the download before analyzing") so the user is pointed at the real problem instead of getting unplayable clips. Position-heuristic fallback remains valid for VODs that were never downloaded *by design* (status not `"failed"`); only the explicit-failure case is guarded.

**(c) VOD card surfacing.** `src/pages/Vods.tsx` must render a `download_status == "failed"` VOD as a clear failed state, showing the captured failure reason (from 3.4a — at minimum a short message; full stderr stays in logs), and the **"Update yt-dlp & Retry"** action from §3.2 is the card's primary recovery affordance for this state. The implementation plan verifies the current failed-state rendering: if a usable failed badge already exists, the change is converging its recovery action to "Update yt-dlp & Retry" and surfacing the reason; if no failed state is distinguished today, add a minimal failed badge + reason + the action.

## 4. File-level changes (logical map; plan pins exact lines)

- `src-tauri/src/bin_manager.rs` — add `refresh_ytdlp_if_stale` (staleness gate + reuse `download_ytdlp`) and a force-refresh entry usable by the card action.
- `src-tauri/src/lib.rs` — invoke `refresh_ytdlp_if_stale` in a non-blocking startup background task.
- `src-tauri/src/commands/binaries.rs` — expose a Tauri command for the force yt-dlp refresh (alongside existing binary-download commands), used by the card action.
- `src-tauri/src/commands/vod.rs` — 3.3 (bounded stderr capture, ~213-241), 3.4a (failure-reason capture/log/surface, ~305-309), 3.4b (analysis guard on `download_status == "failed"`).
- `src/pages/Vods.tsx` — failed-download VOD card: failed state + surfaced reason + "Update yt-dlp & Retry" action (force-refresh command, then `download_vod`); double-click/cooldown guard. No `Settings.tsx` change (no standalone updater button).
- Persistence: existing `settings` k/v key `ytdlp_last_refresh` (no migration).

## 5. Watchouts

- **Startup network failure:** `refresh_ytdlp_if_stale` must never block startup or hard-fail the app. Offline users keep their existing binary; `log::warn!` and move on; retry next startup.
- **System-PATH users:** the refresh must be strictly gated on a bundled binary existing. Never attempt to manage a PATH yt-dlp the user installed themselves.
- **Bandwidth:** ~20 MB re-download, but staleness-gated to ≤ ~weekly per machine — negligible.
- **Stale-binary still better than none:** if a refresh fails, do not delete the existing bundled yt-dlp; keep serving the old one (degraded but functional for VODs Twitch hasn't broken).
- **Analysis guard breadth:** only guard the explicit `"failed"` status. Do not break the legitimate position-heuristic path for genuinely-not-downloaded VODs — that broader rework is v1.3.15.
- **`bump-version.ps1` + commit discipline** per CLAUDE.md applies at ship time (v1.3.14).

## 6. Open questions

None — all resolved in brainstorming (mechanism = app-controlled re-download; destination of the future button = GitHub issue; sequence = cure first; Bug D = minimal guard included).
