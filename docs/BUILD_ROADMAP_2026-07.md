# ClipGoblin Local-First Build Roadmap

**Started:** 2026-07-14
**Product constraint:** one-time purchase, no ClipGoblin-hosted video or AI bill, optional user-funded BYOK only.
**Execution rule:** complete and verify one packet at a time. Do not combine packets in a release merely to make the version look larger.

## Session protocol

At the start of each coding session:

1. Read this file and `git status --short --branch`.
2. Pick only the first packet marked `NEXT` or `IN PROGRESS`.
3. Inspect existing behavior before editing.
4. Keep schema changes backward compatible and local-only.
5. Run focused tests first, then the full Rust/frontend checks before calling a packet complete.
6. Update this file with what landed, exact verification, and the next packet.

This keeps each session small enough to survive usage limits without spending the next session reconstructing intent.

## Product quality gates

- Detection benchmark: median of at least 3 keeper clips in the top 6 across the reference VOD set.
- Pipeline completion: at least 95% of valid local VOD runs finish without manual database or file cleanup.
- Recovery: killing the app during download, analysis, export, or upload must lead to a truthful retry/resume state after restart.
- Local-first: media never passes through ClipGoblin infrastructure.
- Personalization safety: learned preferences may reorder plausible clips but never bypass structural noise/dead-air gates.
- Release safety: typecheck, lint, frontend tests/build, `cargo check`, and Rust tests must pass before versioning.

## Packet 0 - Durable explicit personalization and atomic analysis save

**Status:** COMPLETE

Scope:

- Preserve Good / Meh / Boring feedback independently of generated highlight rows.
- Backfill existing ratings into the durable feedback table.
- Learn bounded dimension, tag, and signal affinities locally.
- Prefer same-channel and same-game examples without requiring either.
- Apply a maximum +/-8 percentage-point ranking adjustment only after enough varied feedback.
- Save replacement analysis results in one SQLite transaction.
- Keep the previous successful clip set when replacement persistence fails.
- Surface whether a completed analysis actually used personalization.

Done when:

- Database migration and rollback tests pass.
- Personalization minimum-sample, direction, and safety-bound tests pass.
- Existing detector tests remain green.
- A VOD with sufficient ratings records nonzero `personalized_candidates` in detection stats.

Implementation record (2026-07-14):

- Added durable `detection_feedback` history that survives clip deletion and re-analysis.
- Added bounded local personalization with minimum sample/variety requirements, same-channel/game weighting, and a hard +/-8 percentage-point limit.
- Preserved structural detector gates, including the single-signal shock safety ceiling.
- Replaced destructive partial analysis writes with one SQLite transaction that preserves the previous successful clip set on failure.
- Added a completed-VOD indicator when candidates were actually personalized.
- Verified: frontend typecheck, lint, production build, and all 19 frontend tests passed.
- Verified: `cargo check` passed and the full Rust library suite passed (556 passed, 1 ignored, 0 failed).
- Manual validation passed on 2026-07-14: two analyses used 14 durable ratings at 70% confidence, personalized 34 and 30 candidates respectively, and produced 8 and 11 final clips without a persistence failure.
- Twitch token refresh also recovered successfully during the validation run. Community-clip requests completed normally and returned zero clips for the tested VOD windows.
- Existing follow-up notes: the Rust project has an established warning backlog, and Vite reports an established large-chunk warning; neither blocked this packet.

## Packet 1A - Explicit feedback UX and learning status

**Status:** COMPLETE

Scope:

- Promote rating controls from hidden developer tooling into an optional normal Detection setting.
- Show learning state: not enough variety, learning, active, and sample count.
- Keep moment preference separate from edit quality with structured reasons: Starts too late, Cuts off early, Too long, Wrong moment, and Duplicate.
- Preserve free-text notes for review, but do not interpret arbitrary note text as a preference signal.
- Keep raw scoring-data export behind the existing developer unlock.
- Make it explicit that moment ratings tune ranking while structured edit issues tune bounded boundary learning.

Done when:

- Users can understand whether personalization is active without reading logs.
- Multiple edit-quality reasons can be selected independently of Good / Meh / Boring.
- Re-analysis and clip deletion do not erase structured edit feedback.

Implementation record (2026-07-14):

- Moved personalized-detection feedback into the normal Detection settings and kept the preference disabled by default.
- Added visible empty, needs-more, needs-variety, learning, and active states with usable sample count and confidence.
- Added multi-select Starts too late, Cuts off early, Too long, Wrong moment, and Duplicate controls plus optional notes.
- Stored edit-quality feedback in a separate local table with original clip boundaries and no highlight foreign key, so generated-highlight replacement cannot erase it.
- Kept ranking feedback and edit-quality feedback independent; clearing one does not silently clear the other.
- Added strict issue validation, duplicate filtering, note-length limits, save-error feedback, and focused Rust/frontend tests.
- Verified: frontend typecheck, lint, production build, and all 22 frontend tests passed.
- Verified: `cargo check` passed and the full Rust library suite passed (560 passed, 1 ignored, 0 failed).
- Existing follow-up notes remain: the Rust warning backlog and Vite large-chunk warning are unchanged and did not block this packet.

## Packet 1B - Conservative behavior ledger and preference controls

**Status:** COMPLETE

Scope:

- Add an append-only local `clip_behavior_events` ledger for review, open-in-editor, meaningful trim, export, publish, and delete.
- Do not treat a single click as approval; define conservative evidence weights.
- Add reset/export controls for the local preference history.
- Record exact before/after trim boundaries as stronger evidence than issue buttons alone.
- Deduplicate repeated opens, saves, exports, and publishes so ordinary retries cannot overpower explicit ratings.

Done when:

- Re-analysis and clip deletion do not erase behavior history.
- Tests prove event deduplication and conservative weighting.
- Users can inspect, export, and reset learned history without deleting clips or media.

Implementation record (2026-07-14):

- Added an append-only local behavior ledger for review, first editor open, meaningful trim, export, publish, and delete actions.
- Deduplicated ordinary opens, repeated exports, retries, reviews, and publishes so passive actions cannot overpower explicit ratings.
- Added Copy history and guarded Reset learning controls; copied history excludes media and API keys.
- Added conservative boundary learning for Starts too late and Cuts off early. A side activates only after feedback from at least two distinct clips, same-channel evidence is isolated, same-game evidence is preferred, and real trims outweigh issue buttons.
- Kept Wrong moment, Duplicate, free-text notes, and ambiguous Too long reports out of automatic boundary direction changes.
- Preserved the 12-second quality floor, capped learned start/end shifts, and capped learned output windows at 60 seconds while preserving the ending payoff.
- Added per-VOD detection stats showing when learned timing was actually applied.
- Verified: frontend typecheck, lint, production build, and all 46 frontend tests passed.
- Verified: `cargo check` passed and the full Rust library suite passed (574 passed, 1 ignored, 0 failed).

## Packet 2 - Benchmark harness and profile calibration

**Status:** PLANNED

Scope:

- Create a versioned local benchmark manifest with expected keeper windows and known false positives.
- Record acceptable start/end ranges for boundary failures, including punchlines cut off during active speech.
- Cover FPS, horror, cozy/quiet, chat-heavy, variety, and low-viewer VODs.
- Report precision-at-6, keeper recall, duplicate rate, boundary error, and runtime.
- Compare base ranking versus personalized ranking on the same candidates.
- Tune confidence ramp, game/channel blending, and score ceiling from benchmark evidence.

Done when:

- One command produces a deterministic before/after report.
- Detection changes cannot ship when benchmark quality regresses beyond an agreed tolerance.

## Packet 3 - Story-aware free detection

**Status:** PLANNED

Scope:

- Add sparse exploratory transcript windows so quiet banter is not invisible to two-pass transcription.
- Detect setup/payoff pairs, sentence boundaries, speaker turns, repeated chat reactions, and reaction tails.
- Treat Twitch clips, streamer markers, chat, transcript, and audio as corroborating evidence rather than automatic duplicates.
- Infer clip-level game/category without a hosted service by combining confirmed VOD/clip values, Twitch community-clip game IDs, and title/transcript evidence. Store the confidence and source, and require confirmation before a low-confidence guess overwrites anything.
- Re-score after boundary optimization so the saved score describes the actual clip boundaries.

Done when:

- Quiet verbal moments improve without materially increasing false positives or transcription time.
- Clips begin before setup and end after payoff/sentence completion on the benchmark set.
- Game-switching VODs can label clips independently, while uncertain clips stay visibly unclassified instead of receiving a confident wrong label.

## Packet 3A - External clip libraries

**Status:** COMPLETE

Scope:

- Import existing videos through a native file picker into the normal Clip Library, editor, export, and publishing flow.
- Scan configured Medal, OBS, and Meld folders without requiring a ClipGoblin-hosted service.
- Offer opt-in auto-import for newly completed local recordings.
- Deduplicate by a content fingerprint rather than filename alone.
- Preserve originals and create cached MP4 preview proxies only when the WebView cannot reliably play the source container.

Implementation record (2026-07-14):

- Added manual multi-file import plus local Medal, OBS, and Meld folder scanners with selection and imported-state indicators.
- Medal accepts the parent capture folder, scans every nested game folder immediately after selection, and offers per-game-folder imports with readable labels such as `Dead by Daylight`.
- Large imports are automatically split into 50-file requests with progress reporting. The backend retains a 200-ID per-request safety ceiling, but it is no longer exposed as a normal user-facing limit.
- A damaged or unreadable video is reported and skipped without cancelling the rest of the folder or library import.
- New Medal clips inherit their game label from the containing folder; rescans backfill only blank labels so manual corrections are preserved.
- Added Clip Library source tabs for Twitch, Medal, OBS, Meld, and Local clips. Medal clips are grouped by game and large sets start collapsed to keep the library usable.
- Added opt-in 30-second auto-import monitoring with bounded scan depth/file count, symlink rejection, and a 15-second write-stability window.
- Added SHA-256 partial-content fingerprints so renamed or moved duplicates are not imported twice.
- Added standalone source metadata to clips and full edit, caption, crop, export, TikTok, and YouTube compatibility.
- Added on-demand MKV/FLV MP4 preview caching with remux-first and transcode fallback; imported originals are never changed.
- Restricted Tauri asset access to each prepared imported preview file instead of granting the frontend access to entire source folders.
- Added visible prepare, retry, empty, success, and import notification states.
- This is a local Medal library integration, not Medal account/cloud OAuth; clips must exist on the PC.

## Packet 4 - Durable analysis/download jobs and checkpoints

**Status:** PLANNED

Scope:

- Introduce durable job records with stage, attempt, timestamps, progress, last error, and resumability.
- Separate truthful terminal states from frontend polling guesses.
- Checkpoint download, audio extraction, transcription windows, candidate scoring, persistence, and thumbnails.
- Resume reusable stages after restart; retry only the failed stage where safe.
- Add cancellation with child-process termination and cleanup of owned temporary files.

Done when:

- Forced termination at each stage returns to a clear Resume or Retry state.
- No job remains permanently `analyzing`, `rendering`, or `uploading` without an owning process.

## Packet 5 - Media pipeline preflight and recovery

**Status:** PLANNED

Scope:

- Validate source path, free disk, ffmpeg/yt-dlp/Whisper availability, codecs, and output writability before expensive work.
- Use atomic temporary outputs followed by rename for downloads, exports, captions, and thumbnails.
- Validate output duration/size/decodability before marking complete.
- Add storage cleanup that distinguishes regenerable cache from user exports.
- Produce a local diagnostic bundle with scrubbed logs and stage history.

Done when:

- Failed operations do not leave corrupt files presented as completed media.
- Common failures have a specific recovery action in the UI.

## Packet 6 - Platform-ready publishing packs

**Status:** PLANNED

Scope:

- Turn one accepted moment into per-platform variants for TikTok, Shorts, Reels, and Threads.
- Centralize media preflight, safe areas, duration/size rules, caption limits, and privacy/disclosure requirements.
- Keep direct desktop-to-platform uploads; the Worker handles secret-dependent OAuth exchanges only.
- Add resumable/chunked uploads where supported and durable status polling.

Done when:

- One clip can retain separate platform copy/settings without overwriting another platform's version.
- Retries are idempotent and cannot accidentally double-post.

## Packet 7 - Meta approval and adapters

**Status:** PLANNED

Scope:

- Finish Instagram professional-account OAuth and Reels publishing first.
- Add Threads as a companion text/video destination after Reels is stable.
- Request only required scopes and prepare a reproducible review-video flow.
- Add token-expiry recovery, review-mode diagnostics, and policy-facing disclosure copy.

Done when:

- Sandbox/test accounts complete connect, publish, status, disconnect, and reconnect flows.
- The exact approval demo can be repeated from a clean install.

## Packet 8 - UI coherence and workflow audit

**Status:** IN PROGRESS

Scope:

- Inventory every primary workflow, setting, dialog, status, and dead end added since the original design.
- Reorganize controls around Fetch, Detect, Review, Edit, Export, and Publish rather than implementation modules.
- Apply progressive disclosure to advanced detection, BYOK, crop, caption, and publishing options.
- Standardize loading, empty, error, retry, destructive-action, and completion states.
- Audit keyboard access, focus order, contrast, text fit, and responsive behavior at common desktop window sizes.
- Add Playwright screenshot checks for the dashboard, VODs, clip library, editor, settings, and publishing dialogs.

Done when:

- A new tester can complete the main VOD-to-publish flow without developer guidance.
- Frequently used actions stay visible while advanced controls no longer crowd the default path.
- No supported window size has overlapping, clipped, or unreachable controls.

Implementation record (started 2026-07-14):

- Added `docs/UI_AUDIT_2026-07.md` with prioritized findings and independent implementation slices.
- Started with the Clip Library density problem reported at a 1280x840 app window.
- Replaced always-visible feedback forms with one-at-a-time progressive disclosure; manual validation passed on 2026-07-14 at the user's normal app window size.
- Reorganized Settings into Accounts, Detection, AI, Editing, Storage, and Appearance workspaces while keeping every control mounted on the same route.
- Kept BYOK cost guidance beside paid AI controls, moved per-clip camera overrides into Editing, and added a direct recovery path from disabled AI detection to AI configuration.
- Browser-validated Settings at 1280x800 and 1024x720 with no horizontal overflow; the user then manually validated the redesign in the native Tauri app on 2026-07-14.
- Reworked VOD cards around one lifecycle-aware primary action (Download, Analyze, View clips, or recovery), with secondary and destructive commands in one disclosure. Completed VODs deep-link to their exact Clip Library section.
- The user manually validated the VOD-card redesign in the native Tauri app on 2026-07-14.
- Split the clip editor into mounted Edit, Captions, and Publish workspaces, keeping preview and Save persistent while preserving child publishing state between tabs. Thumbnail, export feedback, and upload controls now live in Publish.
- The user manually validated the editor-workspace redesign in the native Tauri app at their normal window size on 2026-07-14.
- Replaced the Dashboard's duplicate five-tab workbench with a five-item priority inbox ordered by failed uploads, unrated low-confidence highlights, and export-ready clips.
- Added exact task routing: review items reveal the matching Clip Library feedback panel, ready items open the matching Editor Publish tab, and failed uploads open account recovery or the matching Schedule reschedule control.
- Dashboard attention counts now exclude rated highlights and clips that have already entered an upload workflow instead of leaving stale tasks behind.
- Refined the priority inbox visual hierarchy after user testing and added a compact OBS/Meld capture strip without restoring the old cockpit density.
- Added a dark first-load shell so a slow native startup no longer presents a blank white window.
- The user manually validated the Dashboard, VOD cards, Clip Library disclosure, and Editor tabs in the native app on 2026-07-14.
- Verified the current UI slices with frontend typecheck, lint, production build, all 46 frontend tests, and the full Rust library suite (574 passed, 1 ignored, 0 failed).
- Existing Rust warnings and the Vite large-chunk warning remain unchanged follow-up work.

## Packet 9 - Streamer capture signals

**Status:** IN PROGRESS

Scope:

- Add global hotkey, OBS/Meld integration, Stream Deck action, and configurable spoken marker phrase.
- Store markers locally and merge them as high-value evidence during VOD analysis.
- Keep automatic detection useful when no marker was used.

Done when:

- A marker reliably anchors the correct VOD timestamp and survives app restart.

Implementation record (2026-07-14):

- Added authenticated OBS WebSocket v5 connection testing, replay-buffer status, Save Replay Buffer, saved-path handling, and automatic library import.
- Added Meld local WebChannel discovery, status, `meld.recordClip`, completed-file discovery, and automatic library import.
- OBS passwords use the existing fail-closed DPAPI-sensitive settings path and can be explicitly forgotten.
- Added persistent local OBS/Meld Mark moment actions. Matching markers enter future Twitch VOD analysis as explicit creator provenance and are kept separate by channel and VOD time window.
- Added file-stability waits before importing recorder output and kept recorder/network calls off the UI thread.
- Remaining in this packet: global hotkey, Stream Deck action, configurable spoken phrase, and native end-to-end testing against the user's installed OBS and Meld versions.

## Packet 10 - Steam packaging and commercial-readiness

**Status:** PLANNED

Scope:

- Add a Steam build flavor that uses Steam depot updates rather than the standalone updater.
- Sync only small presets/settings/learned weights through Steam Cloud; never tokens or media.
- Audit redistribution licenses for ffmpeg, yt-dlp, Whisper models, fonts, and every bundled sidecar.
- Complete Steam AI disclosure for BYOK/live-generated text and document guardrails.
- Build a limited demo and a first-run hardware/storage estimate.

Done when:

- Clean-machine install/update/uninstall works for standalone and Steam flavors.
- Third-party notices and store claims exactly match shipped behavior.

## Packet 11 - Preset ecosystem and launch loop

**Status:** PLANNED

Scope:

- Use Steam Workshop for validated data-only detection profiles, caption presets, keyword packs, and safe zones.
- Reject arbitrary scripts and unlicensed font payloads.
- Add opt-in local benchmark/result export for testers without uploading media.
- Prepare the Coming Soon page, demo, trailer flow, tester cohort, and launch feedback cadence.

Done when:

- Community content is schema-validated, versioned, reversible, and cannot execute code.
- Launch metrics can distinguish acquisition, successful first VOD, keeper selection, export, and publish without collecting media.

## Explicitly avoided recurring-cost features

- Cloud video processing or storage.
- ClipGoblin-funded AI inference.
- Off-PC scheduled publishing that requires hosted media.
- Hosted team workspaces or review links.
- A custom hosted preset marketplace when Steam Workshop can provide it.

These may be reconsidered only with a separately funded service model; they do not belong in the one-time-purchase desktop promise.
