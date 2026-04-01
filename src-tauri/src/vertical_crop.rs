//! Vertical crop/scale logic for converting landscape footage to 9:16.
//!
//! Handles three input cases:
//!   1. Landscape (16:9, 4:3, etc.) → crop center, scale to target
//!   2. Already vertical (9:16, 3:4)  → scale to fit, no crop
//!   3. Smaller than target            → scale up to fill, pad if needed
//!
//! The key rule: **crop first, scale second**.  Cropping from the
//! full-resolution source preserves more detail than scaling first.

// ═══════════════════════════════════════════════════════════════════
//  Types
// ═══════════════════════════════════════════════════════════════════

/// Target output dimensions.
#[derive(Debug, Clone, Copy)]
pub struct OutputSize {
    pub width: u32,
    pub height: u32,
}

impl OutputSize {
    pub const VERTICAL_1080: Self = Self { width: 1080, height: 1920 };
    pub const VERTICAL_720: Self = Self { width: 720, height: 1280 };

    pub fn aspect_ratio(&self) -> f64 {
        self.width as f64 / self.height as f64
    }
}

/// Export preset targeting a specific short-form platform.
///
/// For MVP all three vertical presets share the same technical settings
/// (1080x1920, H.264, CRF 23).  The enum exists so the backend knows
/// which platform is targeted and can diverge settings later (bitrate
/// caps, audio normalization, max duration enforcement, etc.).
///
/// String values match the frontend `ExportPreset.id` in `editTypes.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    TikTok,
    Reels,
    Shorts,
    YouTube,
    Square,
}

impl Platform {
    /// Human-readable label for filenames and UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::TikTok  => "TikTok",
            Self::Reels   => "Instagram Reels",
            Self::Shorts  => "YouTube Shorts",
            Self::YouTube => "YouTube",
            Self::Square  => "Square",
        }
    }

    /// Short tag appended to export filenames.
    pub fn file_tag(self) -> &'static str {
        match self {
            Self::TikTok  => "tiktok",
            Self::Reels   => "reels",
            Self::Shorts  => "shorts",
            Self::YouTube => "youtube",
            Self::Square  => "square",
        }
    }

    /// Target output resolution for this platform.
    pub fn resolution(self) -> OutputSize {
        match self {
            Self::TikTok | Self::Reels | Self::Shorts =>
                OutputSize { width: 1080, height: 1920 },
            Self::YouTube =>
                OutputSize { width: 1920, height: 1080 },
            Self::Square =>
                OutputSize { width: 1080, height: 1080 },
        }
    }

    /// Maximum clip duration in seconds (platform limit).
    pub fn max_duration(self) -> f64 {
        match self {
            Self::TikTok  => 60.0,
            Self::Reels   => 90.0,
            Self::Shorts  => 60.0,
            Self::YouTube => 600.0,
            Self::Square  => 140.0,
        }
    }

    /// Platform-specific encode settings.
    ///
    /// For MVP these all return the same defaults.  As platforms
    /// diverge (e.g. Reels prefers higher bitrate, Shorts wants
    /// specific audio loudness), override here.
    pub fn encode_settings(self) -> EncodeSettings {
        match self {
            // All share the same settings for now.
            // Future: TikTok may want -b:v 4M cap, Reels -ar 44100, etc.
            _ => EncodeSettings::default(),
        }
    }

    /// Parse from the frontend preset id string.
    pub fn from_preset_id(id: &str) -> Option<Self> {
        match id {
            "tiktok" => Some(Self::TikTok),
            "reels"  => Some(Self::Reels),
            "shorts" => Some(Self::Shorts),
            "youtube" => Some(Self::YouTube),
            "square" => Some(Self::Square),
            _ => None,
        }
    }

    /// Parse from the DB aspect_ratio string (fallback when no preset id is stored).
    pub fn from_aspect_ratio(ar: &str) -> Self {
        match ar {
            "9:16" => Self::TikTok,   // default vertical platform
            "1:1"  => Self::Square,
            _      => Self::YouTube,
        }
    }
}

/// Source video dimensions (may not be known at filter-build time).
#[derive(Debug, Clone, Copy)]
pub struct InputSize {
    pub width: u32,
    pub height: u32,
}

impl InputSize {
    pub fn aspect_ratio(&self) -> f64 {
        self.width as f64 / self.height as f64
    }

    pub fn is_landscape(&self) -> bool {
        self.width > self.height
    }

    pub fn is_vertical(&self) -> bool {
        self.height > self.width
    }

    pub fn is_smaller_than(&self, target: OutputSize) -> bool {
        self.width < target.width && self.height < target.height
    }
}

/// Export layout mode.
///
/// Determines how the source video is arranged in the vertical frame.
/// The string values match the `facecam_layout` column in the DB
/// and the `LayoutMode` type in the frontend.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutMode {
    /// Center crop — full frame dedicated to gameplay.
    /// Best for: gameplay clips, no facecam.
    /// ffmpeg: simple `-vf` scale+crop.
    #[serde(alias = "none")]
    GameplayFocus,

    /// Game on top (60%), facecam on bottom (40%).
    /// Best for: streamers with webcam, reaction content.
    /// ffmpeg: `-filter_complex` with split+vstack.
    Split {
        /// Fraction of frame height for game (0.0–1.0). Default 0.6.
        #[serde(default = "default_split_ratio")]
        ratio: f64,
    },

    /// Facecam overlay in corner on top of gameplay.
    /// Best for: cinematic feel, reactions.
    /// ffmpeg: `-filter_complex` with split+overlay.
    Pip {
        /// Normalized position (0.0–1.0). Default: bottom-right.
        #[serde(default = "default_pip_x")]
        x: f64,
        #[serde(default = "default_pip_y")]
        y: f64,
        /// Fraction of frame width (0.15–0.45). Default 0.3.
        #[serde(default = "default_pip_size")]
        size: f64,
    },
}

fn default_split_ratio() -> f64 { 0.6 }
fn default_pip_x() -> f64 { 0.93 }
fn default_pip_y() -> f64 { 0.93 }
fn default_pip_size() -> f64 { 0.3 }

impl LayoutMode {
    /// Parse from the DB string value.  Unknown values fall back to
    /// GameplayFocus (the smart fallback).
    pub fn from_db(s: &str) -> Self {
        match s {
            "split" => Self::Split { ratio: 0.6 },
            "pip" => Self::Pip { x: 0.93, y: 0.93, size: 0.3 },
            _ => Self::GameplayFocus,
        }
    }

    /// Whether this layout requires `-filter_complex` (vs simple `-vf`).
    pub fn is_complex(&self) -> bool {
        !matches!(self, Self::GameplayFocus)
    }
}

/// Where to crop from when the source is wider than the target.
#[derive(Debug, Clone, Copy)]
pub enum CropAnchor {
    /// Crop from the horizontal center (default for gameplay).
    Center,
    /// Crop from a specific x-offset (0.0 = left, 1.0 = right).
    /// Useful for future facecam-aware cropping.
    Offset(f64),
}

impl Default for CropAnchor {
    fn default() -> Self { Self::Center }
}

// ═══════════════════════════════════════════════════════════════════
//  Filter generation
// ═══════════════════════════════════════════════════════════════════

/// Build an ffmpeg `-vf` filter string that converts any input to
/// the target vertical resolution using center crop.
///
/// This produces a **simple filter** (not filter_complex) suitable
/// for the `FullFrame` / "none" layout mode.
///
/// # Strategy by input type
///
/// **Landscape (wider than target ratio):**
/// ```text
///   ┌──────────────────────────────┐
///   │     ┌──────────┐             │  1. Scale height to target
///   │     │  KEEP    │  crop       │  2. Crop width to target
///   │     │  CENTER  │  sides      │
///   │     └──────────┘             │
///   └──────────────────────────────┘
/// ```
/// Filter: `scale=-1:{th}:flags=lanczos,crop={tw}:{th}`
///
/// **Already vertical (taller than target ratio):**
/// ```text
///   ┌────┐
///   │    │    1. Scale width to target
///   │    │    2. Crop height to target (from top)
///   │    │
///   │    │
///   └────┘
/// ```
/// Filter: `scale={tw}:-1:flags=lanczos,crop={tw}:{th}:0:0`
///
/// **Smaller than target:**
/// ```text
///   ┌──────────┐
///   │  small   │    1. Scale up to fill target (may distort)
///   │  source  │    2. Crop overflow
///   └──────────┘    3. Black padding if still short
/// ```
/// Filter: `scale={tw}:{th}:force_original_aspect_ratio=increase,crop={tw}:{th},
///          pad={tw}:{th}:(ow-iw)/2:(oh-ih)/2:black`
///
pub fn vertical_filter(target: OutputSize, anchor: CropAnchor) -> String {
    let tw = target.width;
    let th = target.height;

    // We don't know the input resolution at filter-build time (ffmpeg
    // resolves `iw`/`ih` at runtime), so we use ffmpeg expressions.
    //
    // The universal approach:
    //   1. Scale so the SMALLER dimension matches target → fills the frame
    //   2. Crop the overflow from the LARGER dimension
    //   3. Pad with black if source is truly tiny (both dimensions smaller)
    //
    // For landscape input (most common): scale height to 1920, crop width to 1080
    // For vertical input: scale width to 1080, crop height to 1920
    // For tiny input: scale up to fill, pad remainder

    let crop_x = match anchor {
        CropAnchor::Center => "(iw-ow)/2".to_string(),
        CropAnchor::Offset(pct) => {
            let p = pct.clamp(0.0, 1.0);
            format!("(iw-ow)*{:.2}", p)
        }
    };

    // Step 1: Scale to fill (at least one dimension matches target)
    // Step 2: Crop to exact target
    // Step 3: Pad if source was too small in both dimensions
    format!(
        "scale={tw}:{th}:force_original_aspect_ratio=increase:flags=lanczos,\
         crop={tw}:{th}:{crop_x}:0,\
         pad={tw}:{th}:(ow-iw)/2:(oh-ih)/2:black"
    )
}

/// Build a filter for a known input size.  This produces a more
/// efficient filter by choosing the optimal strategy upfront.
pub fn vertical_filter_known(
    input: InputSize,
    target: OutputSize,
    anchor: CropAnchor,
) -> String {
    let tw = target.width;
    let th = target.height;
    let target_ar = target.aspect_ratio();
    let input_ar = input.aspect_ratio();

    if input.is_smaller_than(target) {
        // Tiny source: scale up to fill, crop overflow, pad remainder
        return format!(
            "scale={tw}:{th}:force_original_aspect_ratio=increase:flags=lanczos,\
             crop={tw}:{th},\
             pad={tw}:{th}:(ow-iw)/2:(oh-ih)/2:black"
        );
    }

    let crop_x = match anchor {
        CropAnchor::Center => "(iw-ow)/2".to_string(),
        CropAnchor::Offset(pct) => format!("(iw-ow)*{:.2}", pct.clamp(0.0, 1.0)),
    };

    if input_ar > target_ar {
        // Landscape → vertical: scale height to match, crop width
        format!(
            "scale=-1:{th}:flags=lanczos,crop={tw}:{th}:{crop_x}:0"
        )
    } else {
        // Already vertical or square: scale width to match, crop height from top
        format!(
            "scale={tw}:-1:flags=lanczos,crop={tw}:{th}:0:0"
        )
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════
//  Layout-aware filter builder
// ═══════════════════════════════════════════════════════════════════

/// Build a complete ffmpeg filter graph for a layout mode + target size.
///
/// Returns `(filter_string, is_complex)`.
///   - `is_complex=false` → use `-vf`
///   - `is_complex=true`  → use `-filter_complex` + `-map "[out]" -map "0:a?"`
///
/// The `caption_filter` is an optional extra filter appended at the end
/// (e.g. `subtitles=...` or `drawtext=...`).
pub fn layout_filter(
    mode: &LayoutMode,
    target: OutputSize,
    caption_filter: Option<&str>,
) -> (String, bool) {
    let tw = target.width;
    let th = target.height;

    match mode {
        LayoutMode::GameplayFocus => {
            let base = vertical_filter(target, CropAnchor::Center);
            let filter = match caption_filter {
                Some(cf) => format!("{},{}", base, cf),
                None => base,
            };
            (filter, false)
        }

        LayoutMode::Split { ratio } => {
            let r = ratio.clamp(0.3, 0.8);
            let th_top = (th as f64 * r) as u32;
            let th_bot = th - th_top;
            let cam_crop = 1.0 - r; // facecam region in source

            let mut f = format!(
                "[0:v]split[a][b];\
                 [a]crop=iw:ih*{r:.2}:0:0,scale={tw}:{th_top}:flags=lanczos[top];\
                 [b]crop=iw*{cam_crop:.2}:ih*{cam_crop:.2}:0:ih*{r:.2},\
                 scale={tw}:{th_bot}:flags=lanczos[bottom];\
                 [top][bottom]vstack"
            );

            if let Some(cf) = caption_filter {
                f.push_str(&format!("[stacked];[stacked]{}[out]", cf));
            } else {
                f.push_str("[out]");
            }
            (f, true)
        }

        LayoutMode::Pip { x, y, size } => {
            let ps = (tw as f64 * size.clamp(0.15, 0.45)) as u32;
            let ox = ((tw as f64 - ps as f64) * x.clamp(0.0, 1.0)) as u32;
            let oy = ((th as f64 - ps as f64) * y.clamp(0.0, 1.0)) as u32;

            // Source crop region for PiP: bottom-right corner of source
            let cam_frac = size.clamp(0.15, 0.45);

            let mut f = format!(
                "[0:v]split[bg][ps];\
                 [bg]scale={tw}:{th}:force_original_aspect_ratio=increase:flags=lanczos,\
                 crop={tw}:{th}[main];\
                 [ps]crop=iw*{cam_frac:.2}:ih*{cam_frac:.2}:0:ih*{top:.2},\
                 scale={ps}:{ps}:flags=lanczos[pip];\
                 [main][pip]overlay={ox}:{oy}",
                top = 1.0 - cam_frac,
            );

            if let Some(cf) = caption_filter {
                f.push_str(&format!("[overlaid];[overlaid]{}[out]", cf));
            } else {
                f.push_str("[out]");
            }
            (f, true)
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Export request + ffmpeg command builder
// ═══════════════════════════════════════════════════════════════════

use std::path::{Path, PathBuf};
use std::process::Command;

/// Everything needed to export a single clip.
///
/// Decoupled from the database — callers construct this from
/// whatever data source they have (ClipRow, CandidateClip, etc.).
#[derive(Debug, Clone)]
pub struct ExportRequest {
    /// Path to the source video (VOD).
    pub source_path: PathBuf,
    /// Where to write the exported MP4.
    pub output_path: PathBuf,
    /// Clip start time in seconds.
    pub start: f64,
    /// Clip end time in seconds.
    pub end: f64,
    /// Target platform (drives resolution + encode settings).
    pub platform: Platform,
    /// Target output resolution (usually from `platform.resolution()`).
    pub target: OutputSize,
    /// Layout mode (gameplay focus, split, pip).
    pub layout: LayoutMode,
    /// Optional caption/subtitle filter string.
    pub caption_filter: Option<String>,
}

/// Codec and quality settings for the export.
///
/// MVP defaults are sane for TikTok/Reels/Shorts.
/// Kept separate so hardware acceleration can swap in later.
#[derive(Debug, Clone)]
pub struct EncodeSettings {
    /// Video codec.  Default: `libx264`.
    pub video_codec: String,
    /// Encoder preset.  Default: `medium`.
    pub preset: String,
    /// Constant Rate Factor (quality).  Default: `23`.
    /// Lower = better quality, larger file.  18–28 is the useful range.
    pub crf: u32,
    /// Audio codec.  Default: `aac`.
    pub audio_codec: String,
    /// Audio bitrate.  Default: `128k`.
    pub audio_bitrate: String,
}

impl Default for EncodeSettings {
    fn default() -> Self {
        Self {
            video_codec: "libx264".into(),
            preset: "medium".into(),
            crf: 23,
            audio_codec: "aac".into(),
            audio_bitrate: "128k".into(),
        }
    }
}

/// Build a `std::process::Command` for ffmpeg that exports the clip.
///
/// The returned command is ready to `.status()` or `.output()`.
/// The caller is responsible for:
///   - suppressing the console window on Windows
///   - checking the exit status
///   - updating render status in the DB
///
/// # FFmpeg flag rationale
///
/// ```text
/// -ss / -to        Seek BEFORE -i (input seeking) for speed.
///                   FFmpeg seeks to the nearest keyframe, then
///                   decodes forward.  Much faster than output seeking.
///
/// -i               Input file.
///
/// -vf / -filter_complex
///                   Simple filter for GameplayFocus (single chain).
///                   Complex filter for Split/Pip (multiple streams).
///
/// -map "[out]"     Select the labeled output from filter_complex.
/// -map "0:a?"      Include audio if present (? = don't fail if missing).
///
/// -c:v libx264     H.264 — universal playback, TikTok/Reels/Shorts
///                   all accept it.  Future: swap for h264_nvenc.
/// -preset medium   Balanced speed/quality.  "fast" for previews,
///                   "slow" for final exports.
/// -crf 23          Visually transparent quality.  22 for higher
///                   quality at ~30% larger files.
///
/// -c:a aac         AAC audio — universal.
/// -b:a 128k        128 kbps stereo.  Sufficient for speech/game audio.
///
/// -movflags +faststart
///                   Moves the MP4 moov atom to the front so the file
///                   can start playing before fully downloaded.
///
/// -y               Overwrite output without prompting.
/// ```
pub fn build_ffmpeg_command(
    ffmpeg_path: &Path,
    request: &ExportRequest,
    encode: &EncodeSettings,
) -> Command {
    // Build the filter graph
    let caption = request.caption_filter.as_deref();
    let (filter, is_complex) = layout_filter(&request.layout, request.target, caption);

    let mut cmd = Command::new(ffmpeg_path);

    // ── Input seeking (fast — seeks to nearest keyframe) ──
    cmd.arg("-ss").arg(format!("{:.3}", request.start))
       .arg("-to").arg(format!("{:.3}", request.end))
       .arg("-i").arg(&request.source_path);

    // ── Filter graph ──
    if is_complex {
        // Complex: multiple streams need -filter_complex + explicit mapping
        cmd.arg("-filter_complex").arg(&filter)
           .arg("-map").arg("[out]")
           .arg("-map").arg("0:a?");
    } else {
        // Simple: single chain, ffmpeg auto-maps audio
        cmd.arg("-vf").arg(&filter);
    }

    // ── Video codec ──
    cmd.arg("-c:v").arg(&encode.video_codec)
       .arg("-preset").arg(&encode.preset)
       .arg("-crf").arg(encode.crf.to_string());

    // ── Audio codec ──
    cmd.arg("-c:a").arg(&encode.audio_codec)
       .arg("-b:a").arg(&encode.audio_bitrate);

    // ── Container ──
    cmd.arg("-movflags").arg("+faststart");

    // ── Output ──
    cmd.arg("-y")
       .arg(&request.output_path);

    // Suppress stdout/stderr (caller can override)
    cmd.stdout(std::process::Stdio::null())
       .stderr(std::process::Stdio::null());

    cmd
}

/// Convenience: build command with the platform's encode settings.
pub fn build_export_command(ffmpeg_path: &Path, request: &ExportRequest) -> Command {
    build_ffmpeg_command(ffmpeg_path, request, &request.platform.encode_settings())
}

/// Result of running an export.
pub struct ExportResult {
    /// Whether ffmpeg exited successfully.
    pub success: bool,
    /// Last N lines of ffmpeg stderr (for error diagnostics).
    /// Empty on success.
    pub stderr_tail: String,
}

/// Run the export and capture stderr for error reporting.
///
/// Uses `-progress pipe:1` to get machine-readable progress on stdout.
/// Calls `on_progress(0..100)` as encoding advances.  The callback is
/// invoked from the calling thread (this is a blocking function).
///
/// # Progress mapping
///
/// ffmpeg emits `out_time_us=<microseconds>` lines on stdout.
/// We divide by the clip duration to get a percentage.
///
/// ```text
///   0%  → command launched
///  1-90% → encoding (mapped from out_time_us / duration)
///  95%  → ffmpeg exited, writing moov atom
/// 100%  → done
/// ```
pub fn run_export(
    ffmpeg_path: &Path,
    request: &ExportRequest,
    on_progress: impl Fn(u8),
) -> ExportResult {
    let encode = request.platform.encode_settings();
    let caption = request.caption_filter.as_deref();
    let (filter, is_complex) = layout_filter(&request.layout, request.target, caption);

    let duration_us = ((request.end - request.start) * 1_000_000.0) as u64;

    let mut cmd = Command::new(ffmpeg_path);

    // Input seeking
    cmd.arg("-ss").arg(format!("{:.3}", request.start))
       .arg("-to").arg(format!("{:.3}", request.end))
       .arg("-i").arg(&request.source_path);

    // Filter graph
    if is_complex {
        cmd.arg("-filter_complex").arg(&filter)
           .arg("-map").arg("[out]")
           .arg("-map").arg("0:a?");
    } else {
        cmd.arg("-vf").arg(&filter);
    }

    // Encoding
    cmd.arg("-c:v").arg(&encode.video_codec)
       .arg("-preset").arg(&encode.preset)
       .arg("-crf").arg(encode.crf.to_string())
       .arg("-c:a").arg(&encode.audio_codec)
       .arg("-b:a").arg(&encode.audio_bitrate)
       .arg("-movflags").arg("+faststart");

    // Progress output on stdout (machine-readable)
    cmd.arg("-progress").arg("pipe:1");

    // Output
    cmd.arg("-y")
       .arg(&request.output_path);

    // Pipe stdout for progress, pipe stderr for errors
    cmd.stdout(std::process::Stdio::piped())
       .stderr(std::process::Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    on_progress(0);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return ExportResult {
                success: false,
                stderr_tail: format!("Failed to start ffmpeg: {e}"),
            };
        }
    };

    // Take ownership of pipes before waiting
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    // Read stderr in a background thread so it doesn't block
    let stderr_thread = std::thread::spawn(move || {
        let Some(mut pipe) = stderr_pipe else { return String::new() };
        use std::io::Read;
        let mut buf = String::new();
        pipe.read_to_string(&mut buf).ok();
        let lines: Vec<&str> = buf.lines().collect();
        let start = lines.len().saturating_sub(5);
        lines[start..].join("\n")
    });

    // Parse progress from stdout (blocks until ffmpeg closes stdout)
    if let Some(pipe) = stdout_pipe {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(pipe);
        let mut last_pct: u8 = 0;

        for line in reader.lines() {
            let Ok(line) = line else { break };

            // ffmpeg -progress emits: out_time_us=<microseconds>
            if let Some(us_str) = line.strip_prefix("out_time_us=") {
                if let Ok(us) = us_str.trim().parse::<u64>() {
                    if duration_us > 0 {
                        let raw_pct = ((us as f64 / duration_us as f64) * 90.0) as u8;
                        let pct = raw_pct.clamp(1, 90);
                        if pct > last_pct {
                            last_pct = pct;
                            on_progress(pct);
                        }
                    }
                }
            }
        }
    }

    // Wait for ffmpeg to finish
    on_progress(92);
    let exit_status = child.wait();
    on_progress(95);

    let stderr_tail = stderr_thread.join().unwrap_or_default();

    let success = match exit_status {
        Ok(s) => s.success(),
        Err(_) => false,
    };

    ExportResult {
        success,
        stderr_tail: if success { String::new() } else { stderr_tail },
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── Generic filter (unknown input) ──

    #[test]
    fn generic_filter_produces_scale_crop_pad() {
        let f = vertical_filter(OutputSize::VERTICAL_1080, CropAnchor::Center);
        assert!(f.contains("scale=1080:1920"), "filter: {}", f);
        assert!(f.contains("crop=1080:1920"), "filter: {}", f);
        assert!(f.contains("pad=1080:1920"), "filter: {}", f);
        assert!(f.contains("lanczos"), "should use lanczos: {}", f);
    }

    #[test]
    fn generic_filter_720p() {
        let f = vertical_filter(OutputSize::VERTICAL_720, CropAnchor::Center);
        assert!(f.contains("scale=720:1280"), "filter: {}", f);
        assert!(f.contains("crop=720:1280"), "filter: {}", f);
    }

    #[test]
    fn offset_anchor_uses_custom_x() {
        let f = vertical_filter(OutputSize::VERTICAL_1080, CropAnchor::Offset(0.3));
        assert!(f.contains("(iw-ow)*0.30"), "filter: {}", f);
    }

    // ── Known-input filter ──

    #[test]
    fn landscape_1080p_crops_width() {
        let input = InputSize { width: 1920, height: 1080 };
        let f = vertical_filter_known(input, OutputSize::VERTICAL_1080, CropAnchor::Center);
        // Should scale height to 1920, then crop width
        assert!(f.contains("scale=-1:1920"), "filter: {}", f);
        assert!(f.contains("crop=1080:1920"), "filter: {}", f);
        // No pad needed
        assert!(!f.contains("pad"), "no pad needed: {}", f);
    }

    #[test]
    fn landscape_1440p_crops_width() {
        let input = InputSize { width: 2560, height: 1440 };
        let f = vertical_filter_known(input, OutputSize::VERTICAL_1080, CropAnchor::Center);
        assert!(f.contains("scale=-1:1920"), "filter: {}", f);
        assert!(f.contains("crop=1080:1920"), "filter: {}", f);
    }

    #[test]
    fn already_vertical_scales_width() {
        let input = InputSize { width: 1080, height: 1920 };
        let f = vertical_filter_known(input, OutputSize::VERTICAL_1080, CropAnchor::Center);
        // Already the right ratio — scale width, crop height
        assert!(f.contains("scale=1080:-1"), "filter: {}", f);
        assert!(f.contains("crop=1080:1920:0:0"), "filter: {}", f);
    }

    #[test]
    fn small_input_scales_up_and_pads() {
        let input = InputSize { width: 640, height: 360 };
        let f = vertical_filter_known(input, OutputSize::VERTICAL_1080, CropAnchor::Center);
        assert!(f.contains("force_original_aspect_ratio=increase"), "filter: {}", f);
        assert!(f.contains("pad=1080:1920"), "filter: {}", f);
    }

    #[test]
    fn square_input_scales_width() {
        let input = InputSize { width: 1080, height: 1080 };
        let f = vertical_filter_known(input, OutputSize::VERTICAL_1080, CropAnchor::Center);
        // Square is "wider" than 9:16 target ratio → landscape path
        assert!(f.contains("scale=-1:1920"), "filter: {}", f);
        assert!(f.contains("crop=1080:1920"), "filter: {}", f);
    }

    #[test]
    fn wide_but_short_uses_landscape_path() {
        // 21:9 ultrawide
        let input = InputSize { width: 2560, height: 1080 };
        let f = vertical_filter_known(input, OutputSize::VERTICAL_1080, CropAnchor::Center);
        assert!(f.contains("scale=-1:1920"), "filter: {}", f);
    }

    #[test]
    fn offset_anchor_works_with_known_input() {
        let input = InputSize { width: 1920, height: 1080 };
        let f = vertical_filter_known(input, OutputSize::VERTICAL_1080, CropAnchor::Offset(0.7));
        assert!(f.contains("(iw-ow)*0.70"), "filter: {}", f);
    }

    // ── Layout filter ──

    #[test]
    fn gameplay_focus_is_simple_filter() {
        let (f, complex) = layout_filter(
            &LayoutMode::GameplayFocus,
            OutputSize::VERTICAL_1080,
            None,
        );
        assert!(!complex, "gameplay focus should be simple filter");
        assert!(f.contains("scale=1080:1920"), "filter: {}", f);
        assert!(f.contains("crop=1080:1920"), "filter: {}", f);
    }

    #[test]
    fn split_is_complex_filter() {
        let (f, complex) = layout_filter(
            &LayoutMode::Split { ratio: 0.6 },
            OutputSize::VERTICAL_1080,
            None,
        );
        assert!(complex, "split should be complex filter");
        assert!(f.contains("vstack"), "should vstack: {}", f);
        assert!(f.contains("[out]"), "should end with [out]: {}", f);
    }

    #[test]
    fn pip_is_complex_filter() {
        let (f, complex) = layout_filter(
            &LayoutMode::Pip { x: 0.93, y: 0.93, size: 0.3 },
            OutputSize::VERTICAL_1080,
            None,
        );
        assert!(complex, "pip should be complex filter");
        assert!(f.contains("overlay"), "should overlay: {}", f);
        assert!(f.contains("[out]"), "should end with [out]: {}", f);
    }

    #[test]
    fn caption_appended_to_gameplay_focus() {
        let (f, _) = layout_filter(
            &LayoutMode::GameplayFocus,
            OutputSize::VERTICAL_1080,
            Some("drawtext=text='test'"),
        );
        assert!(f.contains("drawtext"), "should include caption: {}", f);
    }

    #[test]
    fn caption_appended_to_split() {
        let (f, _) = layout_filter(
            &LayoutMode::Split { ratio: 0.6 },
            OutputSize::VERTICAL_1080,
            Some("drawtext=text='test'"),
        );
        assert!(f.contains("[stacked]"), "should chain after stack: {}", f);
        assert!(f.contains("drawtext"), "should include caption: {}", f);
    }

    #[test]
    fn from_db_unknown_falls_back() {
        assert!(matches!(LayoutMode::from_db("none"), LayoutMode::GameplayFocus));
        assert!(matches!(LayoutMode::from_db("split"), LayoutMode::Split { .. }));
        assert!(matches!(LayoutMode::from_db("pip"), LayoutMode::Pip { .. }));
        assert!(matches!(LayoutMode::from_db("invalid"), LayoutMode::GameplayFocus));
        assert!(matches!(LayoutMode::from_db(""), LayoutMode::GameplayFocus));
    }

    #[test]
    fn split_ratio_clamped() {
        let (f, _) = layout_filter(
            &LayoutMode::Split { ratio: 0.95 },  // exceeds max
            OutputSize::VERTICAL_1080,
            None,
        );
        // Should clamp to 0.80, not 0.95
        assert!(f.contains("ih*0.80"), "should clamp ratio: {}", f);
    }

    // ── Command builder ──

    fn sample_request() -> ExportRequest {
        ExportRequest {
            source_path: PathBuf::from("/tmp/vod.mp4"),
            output_path: PathBuf::from("/tmp/out.mp4"),
            start: 60.5,
            end: 90.0,
            platform: Platform::TikTok,
            target: OutputSize::VERTICAL_1080,
            layout: LayoutMode::GameplayFocus,
            caption_filter: None,
        }
    }

    /// Helper: collect Command args as strings for assertion.
    fn cmd_args(cmd: &Command) -> Vec<String> {
        let debug = format!("{:?}", cmd);
        // Parse the debug repr — not ideal but works for testing
        // arg positions in the Command struct.
        // Instead, just verify the command builds without panic.
        vec![debug]
    }

    #[test]
    fn command_builds_without_panic() {
        let req = sample_request();
        let _cmd = build_export_command(Path::new("ffmpeg"), &req);
        // Just verifying it doesn't panic
    }

    #[test]
    fn command_with_split_builds() {
        let mut req = sample_request();
        req.layout = LayoutMode::Split { ratio: 0.6 };
        let _cmd = build_export_command(Path::new("ffmpeg"), &req);
    }

    #[test]
    fn command_with_pip_builds() {
        let mut req = sample_request();
        req.layout = LayoutMode::Pip { x: 0.93, y: 0.93, size: 0.3 };
        let _cmd = build_export_command(Path::new("ffmpeg"), &req);
    }

    #[test]
    fn command_with_captions_builds() {
        let mut req = sample_request();
        req.caption_filter = Some("drawtext=text='hello':fontsize=48".into());
        let _cmd = build_export_command(Path::new("ffmpeg"), &req);
    }

    #[test]
    fn command_720p_builds() {
        let mut req = sample_request();
        req.target = OutputSize::VERTICAL_720;
        let _cmd = build_export_command(Path::new("ffmpeg"), &req);
    }

    #[test]
    fn custom_encode_settings() {
        let req = sample_request();
        let settings = EncodeSettings {
            video_codec: "h264_nvenc".into(),
            preset: "fast".into(),
            crf: 20,
            ..Default::default()
        };
        let _cmd = build_ffmpeg_command(Path::new("ffmpeg"), &req, &settings);
    }

    // ── Platform presets ──

    #[test]
    fn vertical_platforms_share_resolution() {
        assert_eq!(Platform::TikTok.resolution().width, 1080);
        assert_eq!(Platform::TikTok.resolution().height, 1920);
        assert_eq!(Platform::Reels.resolution().width, 1080);
        assert_eq!(Platform::Shorts.resolution().height, 1920);
    }

    #[test]
    fn youtube_is_landscape() {
        let r = Platform::YouTube.resolution();
        assert_eq!(r.width, 1920);
        assert_eq!(r.height, 1080);
    }

    #[test]
    fn square_is_square() {
        let r = Platform::Square.resolution();
        assert_eq!(r.width, r.height);
    }

    #[test]
    fn max_duration_per_platform() {
        assert!((Platform::TikTok.max_duration() - 60.0).abs() < 0.1);
        assert!((Platform::Reels.max_duration() - 90.0).abs() < 0.1);
        assert!((Platform::Shorts.max_duration() - 60.0).abs() < 0.1);
        assert!(Platform::YouTube.max_duration() > 300.0);
    }

    #[test]
    fn from_preset_id_round_trips() {
        assert_eq!(Platform::from_preset_id("tiktok"), Some(Platform::TikTok));
        assert_eq!(Platform::from_preset_id("reels"), Some(Platform::Reels));
        assert_eq!(Platform::from_preset_id("shorts"), Some(Platform::Shorts));
        assert_eq!(Platform::from_preset_id("youtube"), Some(Platform::YouTube));
        assert_eq!(Platform::from_preset_id("square"), Some(Platform::Square));
        assert_eq!(Platform::from_preset_id("invalid"), None);
    }

    #[test]
    fn from_aspect_ratio_defaults() {
        assert_eq!(Platform::from_aspect_ratio("9:16"), Platform::TikTok);
        assert_eq!(Platform::from_aspect_ratio("1:1"), Platform::Square);
        assert_eq!(Platform::from_aspect_ratio("16:9"), Platform::YouTube);
        assert_eq!(Platform::from_aspect_ratio("unknown"), Platform::YouTube);
    }

    #[test]
    fn file_tag_is_lowercase() {
        for p in [Platform::TikTok, Platform::Reels, Platform::Shorts, Platform::YouTube, Platform::Square] {
            assert_eq!(p.file_tag(), p.file_tag().to_lowercase(), "file_tag should be lowercase: {}", p.file_tag());
        }
    }

    #[test]
    fn platform_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Platform::TikTok).unwrap(), "\"tiktok\"");
        assert_eq!(serde_json::to_string(&Platform::Reels).unwrap(), "\"reels\"");
        assert_eq!(serde_json::to_string(&Platform::Shorts).unwrap(), "\"shorts\"");
    }
}
