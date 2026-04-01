# ClipGoblin

AI-powered Twitch stream clip generator. Automatically detect highlights, edit clips, generate captions, and publish to social platforms — all from one desktop app.

Built with **Tauri 2 + React + TypeScript** (Rust backend, React frontend).

## Getting Started

1. **Connect your Twitch account** — Click "Connect Twitch" in Settings or on the My Channel page to link your Twitch account. No API credentials needed.

2. **Your channel loads automatically** — Once connected, ClipGoblin fetches your available VODs.

3. **Analyze a VOD** — Select a VOD and click Analyze. ClipGoblin scans the entire stream to detect highlight-worthy moments using local heuristic analysis (no AI needed).

4. **Review & edit clips** — Browse detected highlights, adjust start/end times, pick an aspect ratio (16:9, 9:16, 1:1), add captions, and preview everything in the Editor.

5. **Export & publish** — Export clips as video files, then upload directly to connected platforms like YouTube.

## How to Use

### Clip Detection

ClipGoblin analyzes VODs locally using audio peaks, chat density, and stream events — no AI or API key required. Adjust sensitivity in Settings > Detection Sensitivity: Low catches only the best moments, High finds more subtle clips.

### Editing Clips

The Editor lets you fine-tune each clip: adjust timing with frame-by-frame precision, choose aspect ratio, add captions with multiple style presets, and preview everything before export.

### Captions & Titles

Free mode generates titles and captions using pattern-based templates. For higher quality, connect an AI provider (OpenAI, Claude, or Gemini) in Settings > AI Provider. AI is only used for captions and titles, never for clip detection.

### Exporting

Export renders your clip to a video file using FFmpeg. Choose from presets optimized for different platforms. Exported files are saved to your Exports folder (configurable in Settings > Storage Locations).

### Batch Upload

Select multiple clips on the Clips page and upload them all at once. Unexported clips are automatically exported first. You can schedule uploads for a specific date and time.

### Montage Builder

Combine multiple highlights into a single montage video with automatic transitions.

## AI Provider Guide

AI providers are **optional** and only used for generating captions and titles. Clip detection always runs locally for speed and zero cost.

> **⚠️ Always save your API key when it's shown — most providers only display it once. If you lose it, you'll need to create a new one.**

### Free Mode

Pattern-based caption and title generation. No API key needed. Works offline. Good enough for most use cases.

### Claude (Anthropic)

Natural-sounding captions with strong context awareness.

1. Go to [console.anthropic.com](https://console.anthropic.com/) and create an account
2. Go to **Settings → Billing** and add a credit card
3. Click **"Buy credits"** — enter $5 (minimum amount, lasts months)
4. Go to **API Keys** in the sidebar
5. Click **"Create Key"**, name it "ClipGoblin"
6. Copy the key immediately and save it somewhere safe — you will never see it again after closing this dialog
7. Paste the key into ClipGoblin's **Settings → AI Provider → Claude**

### OpenAI (GPT)

High-quality captions with natural language understanding.

1. Go to [platform.openai.com](https://platform.openai.com/) and create an account
2. New accounts get $5 in free credits (no credit card needed) — these expire after 3 months
3. If you need more credits later, go to **Settings → Billing → Add payment method** and add funds
4. Go to **API Keys** in the sidebar
5. Click **"Create new secret key"**, name it "ClipGoblin"
6. Copy the key immediately and save it somewhere safe — you will never see it again after closing this dialog
7. Paste the key into ClipGoblin's **Settings → AI Provider → OpenAI**

### Google Gemini

Google's AI for caption generation.

1. Go to [aistudio.google.com](https://aistudio.google.com/) and sign in with your Google account
2. Click **"Get API key"** in the left sidebar
3. Click **"Create API key"** and select a Google Cloud project (or create one)
4. Copy the key immediately and save it somewhere safe
5. Gemini offers a free tier with rate limits — no credit card needed for basic use. For higher limits, set up billing in Google Cloud Console
6. Paste the key into ClipGoblin's **Settings → AI Provider → Gemini**

> Tip: Enable "Fall back to Free mode" in Settings so the app keeps working if the API is ever down.

## AI Model Cost Guide

Estimated costs per clip caption/title generation. Actual costs depend on clip length and transcript size.

### OpenAI Models

| Model | Input / 1M tokens | Output / 1M tokens | Est. per clip |
|-------|-------------------|--------------------:|-------------:|
| GPT-5.4 Nano | $0.20 | $1.25 | ~$0.0003 |
| GPT-5.4 Mini | $0.75 | $4.50 | ~$0.001 |
| GPT-5.4 | $2.50 | $15.00 | ~$0.004 |

### Anthropic (Claude) Models

| Model | Input / 1M tokens | Output / 1M tokens | Est. per clip |
|-------|-------------------|--------------------:|-------------:|
| Claude Haiku 3.5 | $0.80 | $4.00 | ~$0.001 |
| Claude Sonnet 4 | $3.00 | $15.00 | ~$0.004 |

### Google (Gemini) Models

| Model | Input / 1M tokens | Output / 1M tokens | Est. per clip |
|-------|-------------------|--------------------:|-------------:|
| Gemini 2.0 Flash | $0.10 | $0.40 | ~$0.0001 |
| Gemini 1.5 Pro | $1.25 | $5.00 | ~$0.002 |
| Gemini 2.5 Pro | $1.25 | $10.00 | ~$0.003 |

> Costs are approximate and based on ~500-1000 tokens per clip. A typical session analyzing 20 clips costs less than $0.10 with most models.

> **Disclaimer:** Pricing shown is approximate and was accurate as of April 2026. AI providers may change their pricing at any time. Check your provider's pricing page for the most current rates. Actual cost per generation depends on clip length and transcript size — shorter clips cost less, longer clips with more dialogue cost slightly more.

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Space` | Play / Pause video |
| `I` | Set clip start (In point) |
| `O` | Set clip end (Out point) |
| `Left` / `Right` | Seek backward / forward 5s |
| `Shift + Left` / `Right` | Seek backward / forward 1 frame |
| `Ctrl + Z` | Undo |
| `Ctrl + Shift + Z` | Redo |

## Troubleshooting

**"Cannot play video" on clips** — The source VOD file may have been moved or deleted. Make sure the original VOD file is still in its download location.

**FFmpeg not found** — ClipGoblin requires FFmpeg for exporting clips. Install it from [ffmpeg.org](https://ffmpeg.org/download.html) and make sure it's in your system PATH.

**AI captions not generating** — Check that your API key is correct using the "Test Connection" button. Make sure you've enabled "Caption generation" and/or "Title generation" in the checkboxes.

**YouTube upload fails** — Ensure your YouTube account is connected in Settings > Publishing Accounts. The clip must be exported first. If you get a token error, disconnect and reconnect your YouTube account.

**No highlights detected** — Try increasing Detection Sensitivity to High. Very short VODs or streams with low activity may yield fewer highlights. Make sure the VOD has audio — audio peaks are a key detection signal.

## Prerequisites

- [Node.js](https://nodejs.org/) (v18+)
- [Rust](https://rustup.rs/) (1.77.2+)
- [FFmpeg](https://ffmpeg.org/download.html) in your system PATH
- A Twitch account (no developer application needed — OAuth is built in)

## Development

```bash
# Install dependencies
npm install

# Run in development mode (starts both Vite + Tauri)
npm run tauri dev

# Build for production
npm run tauri build
```

## Stack

- **Backend:** Rust (Tauri 2) — file system, job queue, API calls, secure storage
- **Frontend:** React 19 + TypeScript (Vite)
- **State Management:** Zustand
- **Styling:** Tailwind CSS 4
- **Video Processing:** FFmpeg

## License

MIT
