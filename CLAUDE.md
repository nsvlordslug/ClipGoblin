# ClipGoblin — Cowork Instructions

## Project Overview
ClipGoblin is a Tauri 2 + React + TypeScript desktop app (crate name: `clipviral`) that detects highlights from Twitch VODs, generates AI captions/titles, and publishes clips to YouTube/TikTok. Built by Slug (Salvador/Sal).

- **Project root:** `C:\Users\cereb\Desktop\Claude projects\clipviral`
- **Dev command:** `cargo tauri dev` (from project root)
- **Cargo check:** `cd src-tauri && cargo check`
- **Database:** `%APPDATA%\clipviral\clipviral.db`
- **GitHub repo:** `https://github.com/nsvlordslug/ClipGoblin`
- **Current version:** v1.4.5 (package.json is the source of truth — synced by `bump-version.ps1`)

---

## CRITICAL RULES — READ FIRST

### 1. Small Surgical Edits Only
**NEVER rewrite large sections of any file.** Use targeted find-and-replace. If you need to add code, append or insert at specific line numbers. Never re-output entire files or large blocks. This rule exists because large file writes have repeatedly caused truncation that broke the build.

### 2. If a File Gets Truncated
Restore from git IMMEDIATELY:
```
git checkout HEAD -- <file>
```
Then re-apply only your changes on top of the restored file.

### 3. Finalized Prompts — DO NOT MODIFY
The AI caption/title prompts in `post_captions.rs` are **FINALIZED** after extensive testing:
- `generate_llm()` — caption generation prompt
- `generate_llm_title()` — title generation prompt
Do NOT modify these functions unless Slug explicitly asks you to.

### 4. Always Commit After Successful Builds
After any successful `cargo check` or `cargo tauri dev`, commit and push:
```
cd "C:\Users\cereb\Desktop\Claude projects\clipviral" && git add -A && git commit -m "description" && git push origin main
```

### 5. Rust IS Available Here — Verify Before Commit
`cargo check` / `cargo test` run fine in this environment (verified across the v1.4.x releases; full lib suite ~479 tests). Run them from `src-tauri/` before every commit. Frontend typecheck: `node node_modules\typescript\bin\tsc -b` from the project root — plain `npx tsc` can resolve a bogus `tsc` package when the shell starts in the parent folder.

### 6. Version Bump Before Every Commit
IMPORTANT: Before every git commit, run `powershell -file bump-version.ps1 <new_version>` to sync the version across package.json, src-tauri/Cargo.toml, and src-tauri/tauri.conf.json.

---

## Codebase Structure

```
src-tauri/src/
├── lib.rs                  # 176 lines — module declarations + run() ONLY
├── commands/
│   ├── mod.rs              # re-exports all command modules
│   ├── auth.rs     (147)   # Twitch OAuth (login, logout, refresh)
│   ├── captions.rs (572)   # AI caption/title generation, clip naming helpers
│   ├── clip.rs     (123)   # clip CRUD (update settings, get detail, save to disk)
│   ├── export.rs   (683)   # video export, subtitle rendering, caption styling
│   ├── scheduled.rs(236)   # scheduled uploads, background scheduler, retry logic
│   ├── settings.rs (237)   # app settings, utilities, storage paths, detection stats
│   ├── social.rs   (169)   # platform upload/connect/disconnect commands
│   └── vod.rs      (2079)  # VOD management, analysis pipeline, transcription
├── social/
│   ├── mod.rs              # PlatformAdapter trait, UploadMeta, UploadResult types
│   ├── youtube.rs          # YouTube Data API v3 OAuth + resumable upload
│   ├── tiktok.rs           # TikTok OAuth + Content Posting API (production; audit in progress)
│   └── instagram.rs        # stub — TODO(v2)
├── db.rs                   # SQLite schema, migrations, all CRUD helpers
├── twitch.rs               # Twitch OAuth (token exchange/refresh via the Cloudflare auth proxy)
├── auth_proxy.rs           # client for the clipgoblin-auth-proxy Worker (no embedded/shared client secret)
├── crypto.rs               # DPAPI encryption for tokens at rest ("dpapi:" prefix, per-Windows-user)
├── post_captions.rs        # AI + pattern-based caption/title generation (FINALIZED)
├── ai_provider.rs          # BYOK provider resolution (Claude/OpenAI/Gemini/Free)
├── clip_selector.rs        # clip detection, scoring, sensitivity scaling
├── error.rs                # AppError enum
├── hardware.rs             # GPU/CPU detection for CUDA
├── job_queue.rs            # background job management
├── vertical_crop.rs        # ffmpeg crop/export helpers
├── pipeline.rs             # signal types, scoring structs (future use)
├── engine.rs               # analysis engine (future use)
├── audio_signal.rs         # audio analysis (future use)
├── scene_signal.rs         # scene change detection (future use)
├── transcript_signal.rs    # transcript analysis (future use)
├── clip_fusion.rs          # multi-signal fusion (future use)
├── clip_ranker.rs          # clip ranking (future use)
├── clip_labeler.rs         # auto-labeling (future use)
└── clip_output.rs          # thumbnail extraction (future use)

src/ (React frontend)
├── App.tsx                 # routes + sidebar navigation
├── pages/
│   ├── Dashboard.tsx
│   ├── Vods.tsx
│   ├── Clips.tsx
│   ├── ClipEditor.tsx
│   ├── Scheduled.tsx
│   ├── Montage.tsx
│   ├── Settings.tsx
│   └── HelpGuide.tsx
├── components/             # UI components (ClipPlayer, TrimTimeline, CaptionPreview, etc.)
├── stores/                 # Zustand stores (appStore, editorStore, etc.)
└── types/                  # TypeScript types (editTypes.ts, etc.)

docs/ (GitHub Pages)
├── index.html              # landing page
├── terms.html              # terms of service
├── privacy.html            # privacy policy
└── callback/index.html     # TikTok OAuth redirect (forwards to localhost:17387)
```

---

## Key Credentials

Credentials are managed via Cloudflare Worker proxy (worker/ directory).
Client IDs are in wrangler.toml vars, secrets are in Cloudflare environment variables.
See worker/wrangler.toml for the proxy configuration.

---

## Key Design Decisions

### Twitch OAuth
- Uses **Confidential** client type (not Public/PKCE — Twitch rejects PKCE token exchange)
- Client ID and Secret are embedded as compile-time constants with `.env` override for dev
- Users just click "Connect Twitch" — no developer credentials needed

### AI Captions (BYOK)
- In BYOK mode, each tone button triggers an independent on-demand AI call (no template mixing)
- In Free mode, pattern-based templates are used (no API calls)
- Full SRT transcript is passed to AI, not just a snippet
- 280 char hard limit on captions, 60 char on titles, with smart truncation
- Title generation uses 6-angle system + random avoid-word to prevent repetition

### Clip Detection
- Dynamic clip count: `max(6, min(35, duration_minutes / 6))` with sensitivity multiplier
- Sensitivity control (Low/Medium/High) exposed in Settings
- 30-second minimum gap between clips, 50% overlap dedup
- Detection stats shown on VOD card

### VOD Management
- `upsert_vod()` updates `channel_id` on conflict (fixes stale IDs after Twitch reconnect)
- `restore_deleted_vods` runs BEFORE `fetchVods` on initial page load (prevents upsert skip)
- Stale analysis detection: timeout resets stuck analyses on startup

### Publishing
- YouTube: resumable upload with snippet.tags + description hashtags
- TikTok: production client live (GitHub Pages callback). Content Posting API audit: round 1 NOT approved 2026-06-09 (ref 20260606214146) — TikTok support confirmed the issue is the DEMO VIDEO (must start with login+scopes, show privacy switching, show options interacting), not the app. Resubmission prep tracked in `C:\Users\cereb\Pictures\cg-demo-frames\TikTok-UX-Mockups-Blueprint.md`.
- YouTube: Google OAuth brand verification APPROVED (2026-06-08); sensitive-scope review (youtube.upload + youtube.readonly) submitted 2026-06-09, under review. Until it passes, users click through the unverified-app warning (Advanced → Continue).
- Scheduled uploads: background scheduler checks every 60s, auto-retry once on failure
- Batch upload: auto-exports un-exported clips before uploading

---

## Known Gotchas

1. **Twitch desktop token exchange needs a Confidential client** — the client secret belongs only in the Cloudflare Worker; never embed it in the desktop app.
2. **TikTok redirect URI must match website domain** — Uses GitHub Pages callback page that forwards to localhost.
3. **VOD channel_id goes stale on reconnect** — upsert now includes `channel_id = excluded.channel_id`.
4. **restore_deleted_vods must run before fetchVods** — otherwise upsert skips previously deleted VODs.
5. **CUDA cublas64_12.dll missing** — user needs CUDA Toolkit 12 installed for GPU transcription.
6. **Asset protocol scope is runtime-only** — `tauri.conf.json` starts with an empty scope. The app allows its data root and a user-selected download folder after canonical path validation; do not restore `["**"]`.
7. **lib.rs truncation** — was 4359 lines, now split into 8 modules. NEVER let it grow large again.
8. **Cargo.toml is in src-tauri/** — run `cargo check` from there, not project root.
9. **Tokens are DPAPI-encrypted at rest** (crypto.rs, `dpapi:` prefix in the settings table). Any tooling that reads tokens outside the app must CryptUnprotectData first — raw values are ciphertext.
10. **Two clip ledgers:** direct uploads write `upload_history`; the Analytics pipeline reads `scheduled_uploads`. Direct uploads are mirrored through `db::record_direct_upload_state_for_analytics`, including processing/private posts that do not yet have a public URL.
11. **OAuth proxy has no desktop shared key** — provider client secrets stay in Cloudflare Worker secrets. The Worker enforces exact routes, redirect allowlists, PKCE where applicable, body limits, and rate limits. Deploy Worker changes separately; never add `PROXY_API_KEY` or `X-Proxy-Key` back to the app.
12. **Unaudited TikTok client = SELF_ONLY posting.** Until the Content Posting API audit passes, posts only succeed with the TikTok account set to Private (`unaudited_client_can_only_post_to_private_accounts`).
13. **A running `clipviral.exe` locks its own binary — rebuilds SILENTLY fail.** On Windows, while the app is open, `cargo build` / `cargo tauri dev` cannot overwrite `src-tauri/target/debug/clipviral.exe` (error: `failed to remove file … Access is denied. (os error 5)`), so the rebuild no-ops and the OLD binary keeps running — code edits never take effect. Symptom: "I rebuilt but nothing changed." Fix: fully CLOSE the app (or `Stop-Process -Name clipviral -Force`) BEFORE rebuilding. Verify the build actually landed with `(Get-Item src-tauri\target\debug\clipviral.exe).LastWriteTime` — if it isn't fresh, you're still on stale code.
14. **Frontend (.tsx) edits don't appear even after rebuilding → stale WebView2 cache.** The dev app loads the frontend from the Vite dev server (`devUrl` = localhost:5173), but Chromium/WebView2 caches compiled JS under `%LOCALAPPDATA%\com.clipgoblin.desktop\EBWebView\Default\{Cache, Code Cache}` and serves it across BOTH app restarts and Vite restarts. Symptom: a new Settings toggle / UI fix never shows no matter how many times you close + `cargo tauri dev` (cost a full debugging session — both a Settings toggle and a scroll fix "didn't work" because they never loaded; the cache had grown to ~550 MB). Confirm by writing a diagnostic to the DB from the effect and checking it never appears. Fix: close the app, delete `EBWebView\Default\Cache` + `Code Cache` (KEEP `Local Storage` so AI keys/settings survive), then restart. A hard-reload (Ctrl+Shift+R) in the webview can also bust it. **Now auto-handled in dev:** `lib.rs run()` wipes `Cache` + `Code Cache` on every debug-build startup (before the webview loads), so a fresh `cargo tauri dev` always serves current code — the manual steps above are just the fallback. (Header tricks like a Vite `Cache-Control: no-store` middleware do NOT work: Vite overrides module responses with `no-cache`, which WebView2 then ignores.)
15. **Release CPU-whisper crash (`0xc000001d` STATUS_ILLEGAL_INSTRUCTION) on non-AVX-512 CPUs.** whisper-rs-sys builds whisper.cpp via cmake with `GGML_NATIVE` defaulting **ON**, so it compiles for the BUILD host's CPU. GitHub's `windows-latest` runners are AVX-512-capable Xeons → the static whisper baked in AVX-512 → it crashes the instant transcription starts on consumer CPUs that lack it (e.g. Intel 12th-gen Alder Lake / i7-12700KF — has AVX2, not AVX-512). Bit v1.5.0; fixed v1.5.1. Tell-tale: dev/GPU (`cuda`) builds work (compiled locally on the user's own CPU) — only the RELEASE CPU build crashes, right at whisper kickoff (no tester report needed to confirm: the dev build of the SAME code transcribes fine). Fix = pin a portable AVX2 baseline in CI. whisper-rs-sys 0.15.0's `build.rs` forwards any `GGML_*` env var as a `-DGGML_*` cmake define, so the `Build Tauri app` step in `release.yml` sets `GGML_NATIVE=OFF` + `GGML_AVX2=ON` + `GGML_AVX512=OFF` (AVX2/FMA/F16C are on every x86 CPU since ~2013; no Cargo.toml change or fork needed). Verify the SHIPPED binary before publishing the draft: extract the exe (`msiexec /a <msi> /qn TARGETDIR=<dir>` → `…\PFiles\ClipGoblin\clipviral.exe`) and `llvm-objdump -d clipviral.exe` (install via `rustup component add llvm-tools-preview`) — **`zmm` register count MUST be 0**, `ymm` (AVX2) present (v1.5.1 shipped 0 zmm / 21,522 ymm). Cache caveat: `GGML_*` is NOT in whisper-rs-sys's fingerprint, so a warm `Swatinem/rust-cache` could in theory re-ship the old AVX-512 lib without recompiling; the Cargo.lock version bump invalidated it for v1.5.1, but if a future release's objdump shows `zmm`>0 despite the env vars, add `cargo clean -p whisper-rs-sys` before the build to force the recompile.

---

## What's Done (v1.0.0)

- ✅ Full Twitch OAuth + VOD fetching (23 VODs loading)
- ✅ VOD download + analysis with CUDA GPU acceleration + CPU fallback
- ✅ Clip detection with sensitivity scaling
- ✅ Full clip editor (trim, subtitles, crop, templates, undo/redo)
- ✅ BYOK AI captions (10 tones) + titles (6 angles) with Claude/OpenAI/Gemini
- ✅ Free pattern-based fallback (no API needed)
- ✅ YouTube upload with title, description, hashtags, visibility
- ✅ TikTok upload (sandbox mode)
- ✅ Batch upload + scheduled uploads with background scheduler
- ✅ Help & Guide page with AI cost breakdown + provider setup walkthroughs
- ✅ Settings page (Twitch connect, AI provider, storage locations, sensitivity)
- ✅ 46/46 code review items resolved (11 critical, 19 important, 16 minor)
- ✅ lib.rs split into 8 command modules (prevents truncation)
- ✅ Zero GPL dependencies
- ✅ GitHub repo + Pages landing page

---

## What's Done Since v1.0 (highlights, as of v1.4.4 / 2026-06-12)

- ✅ Phase 1 complete: auto-updater + signed installers + CI on tag push (`release.yml`, Node-24-ready actions) + manual draft publish
- ✅ Phase 2 complete: in-app bug reporter → GitHub Issues; `triage.yml` + `autofix.yml` pipelines; Discord webhooks
- ✅ Analytics dashboard (views + likes, per-platform, Refresh stats) — incl. the v1.4.3 direct-upload ledger fix
- ✅ TikTok Content Posting API compliance UI (creator_info-driven privacy/interactions/disclosure/consent) + v1.4.4 polish (processing notice, max-duration gate, exact guideline strings)
- ✅ YouTube auto-reauth on invalid_grant; human-readable TikTok upload errors
- ✅ Cam-region vertical-crop editor (v1.4.0); clip-trim reversibility fix
- ✅ OAuth secrets moved out of the binary → Cloudflare auth proxy; tokens DPAPI-encrypted at rest
- ✅ Landing/download/terms/privacy at clipgoblin.mindrotstudios.com (Pages, HTTPS enforced; apex 301→landing via `mindrot-apex-redirect` worker)

## What's Next (in order)

### 1. TikTok audit resubmission (active)
Re-record the demo on the current build per the storyboard in `cg-demo-frames\TikTok-UX-Mockups-Blueprint.md` (login+scopes first, privacy switching, options interacting, branded×private block) → assemble the UX-mockup PDF from fresh frames → resubmit with the own-content use-case framing.

### 2. Meta platform expansion (start after resubmission)
One Meta developer app covers Instagram + Facebook + Threads. Business Verification is the long pole — start it early. IG/Threads need a PUBLIC video URL (host clips temporarily, e.g. R2); Facebook takes direct upload. Build a Mindrot Studios landing at the apex before/with Business Verification.

### 3. Twitch clips importer / detection fusion (validated, speced after resubmission)
Helix Get Clips (`vod_offset`+`view_count`, no extra scope) as audience-signal seeds fused into detection; re-cut from the VOD (escapes the 60s clip cap); instant-import onboarding without full VOD analyze. Twitch's Auto Clips alpha multiplies the supply for free.

### 4. Steam release prep
One-time purchase ≈ one month of Eklipse (~$24.99), no subscription. Store page leans on the structural moat: local processing, no queues/caps/expiry, BYOK. Free keys for Discord beta testers at launch.
