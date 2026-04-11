# ClipGoblin — Cowork Instructions

## Project Overview
ClipGoblin is a Tauri 2 + React + TypeScript desktop app (crate name: `clipviral`) that detects highlights from Twitch VODs, generates AI captions/titles, and publishes clips to YouTube/TikTok. Built by Slug (Salvador/Sal).

- **Project root:** `C:\Users\cereb\Desktop\Claude projects\clipviral`
- **Dev command:** `cargo tauri dev` (from project root)
- **Cargo check:** `cd src-tauri && cargo check`
- **Database:** `%APPDATA%\clipviral\clipviral.db`
- **GitHub repo:** `https://github.com/nsvlordslug/ClipGoblin`
- **Current version:** v1.0.0

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

### 5. Rust Not Available in Cowork VM
You cannot run `cargo check` in this VM. Always ask Slug to run it in his terminal. Do static analysis to catch obvious errors, but the real compiler check happens on his machine.

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
│   ├── tiktok.rs           # TikTok OAuth + Content Posting API (sandbox)
│   └── instagram.rs        # stub — TODO(v2)
├── db.rs                   # SQLite schema, migrations, all CRUD helpers
├── twitch.rs               # Twitch OAuth (Confidential client, embedded creds)
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
- TikTok: sandbox working, production app submitted 3 times (latest uses GitHub Pages callback)
- Scheduled uploads: background scheduler checks every 60s, auto-retry once on failure
- Batch upload: auto-exports un-exported clips before uploading

---

## Known Gotchas

1. **Twitch PKCE doesn't work** — Twitch rejects token exchange for Public clients with "Invalid client credentials". Use Confidential with embedded secret instead.
2. **TikTok redirect URI must match website domain** — Uses GitHub Pages callback page that forwards to localhost.
3. **VOD channel_id goes stale on reconnect** — upsert now includes `channel_id = excluded.channel_id`.
4. **restore_deleted_vods must run before fetchVods** — otherwise upsert skips previously deleted VODs.
5. **CUDA cublas64_12.dll missing** — user needs CUDA Toolkit 12 installed for GPU transcription.
6. **Asset protocol 403 on external drives** — scope set to `["**"]` in tauri.conf.json.
7. **lib.rs truncation** — was 4359 lines, now split into 8 modules. NEVER let it grow large again.
8. **Cargo.toml is in src-tauri/** — run `cargo check` from there, not project root.

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

## What's Next (in order)

### Phase 1: App Packaging + Auto-Update
1. Tauri auto-updater plugin
2. .msi installer via `cargo tauri build`
3. GitHub Actions CI/CD for auto-build + GitHub Releases
4. Code signing for Windows SmartScreen

### Phase 2: Bug Reporting + Automated Fix Pipeline
1. In-app bug reporter (structured form → GitHub Issues with logs/system info)
2. Claude Code triage (classify: bug/feature request/exploit attempt)
3. Auto-fix PRs to `fixes` branch (scope-locked: bugs only, no features/security changes)
4. Discord webhook notifications to private channel
5. Approval gate: only Slug's Discord user ID can approve (slash command or reaction)

### Phase 3: Platform Expansion
1. Meta Developer setup (one app for Instagram + Facebook + Threads)
2. Instagram Reels, Facebook video, Threads adapters
3. TikTok production approval (3rd submission pending)

### Phase 4: Analytics & Polish
1. Analytics dashboard (YouTube/TikTok view counts)
2. Clip performance badges on Clips page
3. Landing page screenshots + download link
