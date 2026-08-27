//! Native, deterministic compositor for the persistent Cardboard placard.

use crate::commands::vod::find_ffmpeg;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use tauri::{AppHandle, Manager};

pub(crate) const RENDERER_VERSION: &str = "cardboard-reference-v6";

pub(crate) const MIN_CARD_SCALE: f64 = 0.5;
pub(crate) const MAX_CARD_SCALE: f64 = 1.0;
pub(crate) const DEFAULT_CARD_SCALE: f64 = 0.75;

const MATERIAL_WIDTH: f64 = 1536.0;
const MATERIAL_HEIGHT: f64 = 1098.0;

const MATERIAL_BYTES: &[u8] =
    include_bytes!("../../public/caption-materials/cardboard-placard-v1.png");
const FONT_BYTES: &[u8] = include_bytes!("../../public/fonts/Anton-Regular.ttf");

static MATERIAL_PATH: OnceLock<Result<PathBuf, String>> = OnceLock::new();
static FONT_PATH: OnceLock<Result<PathBuf, String>> = OnceLock::new();

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionRequest {
    pub text: String,
    pub target_width: u32,
    pub target_height: u32,
    pub font_size: u32,
    #[serde(default = "default_card_scale")]
    pub card_scale: f64,
    pub anchor_y: i32,
    pub alignment: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionAsset {
    pub path: String,
    pub renderer_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlacardLayout {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    text_left: i32,
    text_top: i32,
    text_width: i32,
    text_height: i32,
}

fn stage_embedded_file(filename: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let directory = std::env::temp_dir()
        .join("clipgoblin-cardboard")
        .join(RENDERER_VERSION);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create the Cardboard asset directory: {error}"))?;
    let path = directory.join(filename);
    let current = std::fs::read(&path)
        .map(|existing| existing == bytes)
        .unwrap_or(false);
    if !current {
        std::fs::write(&path, bytes)
            .map_err(|error| format!("Could not stage the Cardboard asset: {error}"))?;
    }
    Ok(path)
}

fn material_path() -> Result<PathBuf, String> {
    MATERIAL_PATH
        .get_or_init(|| stage_embedded_file("cardboard-placard.png", MATERIAL_BYTES))
        .clone()
}

fn font_path() -> Result<PathBuf, String> {
    FONT_PATH
        .get_or_init(|| stage_embedded_file("Anton-Regular.ttf", FONT_BYTES))
        .clone()
}

pub(crate) fn cache_dir() -> Result<PathBuf, String> {
    let path = std::env::temp_dir()
        .join("clipgoblin-cardboard")
        .join(RENDERER_VERSION)
        .join("frames");
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("Could not create the Cardboard frame cache: {error}"))?;
    Ok(path)
}

fn artifact_file_is_ready(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
}

fn ffmpeg_filter_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .replace(':', "\\:")
        .replace('\'', "\\'")
}

fn ffmpeg_error_tail(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ")
}

fn normalize_text(text: &str) -> String {
    text.chars()
        .filter_map(|character| match character {
            '\u{2018}' | '\u{2019}' => Some('\''),
            '\u{201C}' | '\u{201D}' => Some('"'),
            '\u{2013}' | '\u{2014}' => Some('-'),
            '\r' => None,
            '\t' => Some(' '),
            value if value.is_ascii() || value == '\n' => Some(value),
            _ => Some('?'),
        })
        .collect::<String>()
        .to_uppercase()
}

fn default_card_scale() -> f64 {
    DEFAULT_CARD_SCALE
}

pub(crate) fn normalize_card_scale(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(MIN_CARD_SCALE, MAX_CARD_SCALE)
    } else {
        DEFAULT_CARD_SCALE
    }
}

fn validate_request(request: &CaptionRequest) -> Result<(), String> {
    if request.text.chars().count() > 1_000 || request.text.contains('\0') {
        return Err("Cardboard caption text is too long".to_string());
    }
    if !(320..=3_840).contains(&request.target_width)
        || !(320..=3_840).contains(&request.target_height)
    {
        return Err("Cardboard output dimensions are unsupported".to_string());
    }
    if !(8..=256).contains(&request.font_size) {
        return Err("Cardboard font size is unsupported".to_string());
    }
    if !request.card_scale.is_finite()
        || !(MIN_CARD_SCALE..=MAX_CARD_SCALE).contains(&request.card_scale)
    {
        return Err("Cardboard card size is unsupported".to_string());
    }
    if request.anchor_y < 0 || request.anchor_y > request.target_height as i32 {
        return Err("Cardboard caption anchor is outside the frame".to_string());
    }
    if !matches!(request.alignment, 2 | 5 | 8) {
        return Err("Cardboard caption alignment is unsupported".to_string());
    }
    Ok(())
}

fn placard_layout(request: &CaptionRequest) -> PlacardLayout {
    let vertical = request.target_height > request.target_width;
    let width_ratio = if vertical { 0.86 } else { 0.62 };
    let width =
        (request.target_width as f64 * width_ratio * normalize_card_scale(request.card_scale))
            .round() as i32;
    let height = (width as f64 * MATERIAL_HEIGHT / MATERIAL_WIDTH).round() as i32;
    let left = (request.target_width as i32 - width) / 2;
    let desired_top = match request.alignment {
        8 => request.anchor_y,
        5 => request.anchor_y - height / 2,
        _ => request.anchor_y - height,
    };
    let safe_y = ((request.target_height as f64 * 0.025).round() as i32).max(8);
    let top = desired_top.clamp(
        safe_y,
        (request.target_height as i32 - height - safe_y).max(safe_y),
    );
    let text_left = left + width * 12 / 100;
    let text_top = top + height * 18 / 100;
    PlacardLayout {
        left,
        top,
        width,
        height,
        text_left,
        text_top,
        text_width: width * 76 / 100,
        text_height: height * 64 / 100,
    }
}

fn wrap_text(text: &str, max_characters: usize) -> String {
    let max_characters = max_characters.max(5);
    let mut lines = Vec::new();
    for source_line in text.lines() {
        let mut current = String::new();
        for word in source_line.split_whitespace() {
            let candidate_len =
                current.chars().count() + usize::from(!current.is_empty()) + word.chars().count();
            if !current.is_empty() && candidate_len > max_characters {
                lines.push(current);
                current = word.to_string();
            } else {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(word);
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines.join("\n")
}

fn requested_display_font_size(request: &CaptionRequest) -> u32 {
    ((request.font_size as f64 * 1.45).round() as u32).clamp(8, 256)
}

fn fitted_caption(request: &CaptionRequest, layout: PlacardLayout) -> (String, u32) {
    let normalized = normalize_text(&request.text);
    let trimmed = normalized.trim();
    let longest_word = trimmed
        .split_whitespace()
        .map(|word| word.chars().count())
        .max()
        .unwrap_or(1)
        .max(1);
    let requested = requested_display_font_size(request);
    for font_size in (8..=requested).rev() {
        if longest_word as f64 * font_size as f64 * 0.62 > layout.text_width as f64 {
            continue;
        }
        let max_characters = (layout.text_width as f64 / (font_size as f64 * 0.62))
            .floor()
            .max(5.0) as usize;
        let wrapped = wrap_text(trimmed, max_characters);
        let line_count = wrapped.lines().count().max(1) as f64;
        let block_height = font_size as f64 * (line_count + (line_count - 1.0) * 0.08);
        if block_height <= layout.text_height as f64 {
            return (wrapped, font_size);
        }
    }

    (wrap_text(trimmed, 5), 8)
}

fn renderer_filter(
    request: &CaptionRequest,
    layout: PlacardLayout,
    text_paths: &[PathBuf],
) -> Result<String, String> {
    validate_request(request)?;
    let mut filter = format!(
        "[1:v]format=rgba,scale={}:{}[card];[0:v][card]overlay={}:{}:format=auto",
        layout.width, layout.height, layout.left, layout.top,
    );
    if text_paths.is_empty() {
        return Ok(format!("{filter},format=rgba[out]"));
    }
    let font = ffmpeg_filter_path(&font_path()?);
    let (_, font_size) = fitted_caption(request, layout);
    let line_spacing = (font_size as f64 * 0.08).round() as i32;
    let line_step = font_size as i32 + line_spacing;
    let block_height =
        font_size as i32 * text_paths.len() as i32 + line_spacing * (text_paths.len() as i32 - 1);
    let first_y = layout.text_top + (layout.text_height - block_height) / 2;
    for (index, text_path) in text_paths.iter().enumerate() {
        let text = ffmpeg_filter_path(text_path);
        let y = first_y + index as i32 * line_step;
        filter.push_str(&format!(
            ",drawtext=fontfile='{font}':textfile='{text}':reload=0:fontsize={}:fontcolor=#17100A@0.98:x={}+({}-text_w)/2:y={y}",
            font_size, layout.text_left, layout.text_width,
        ));
    }
    Ok(format!("{filter},format=rgba[out]"))
}

pub(crate) fn render_caption_frame(request: &CaptionRequest) -> Result<PathBuf, String> {
    validate_request(request)?;
    let layout = placard_layout(request);
    let (wrapped, _) = fitted_caption(request, layout);
    let mut hasher = Sha256::new();
    hasher.update(RENDERER_VERSION.as_bytes());
    hasher.update(serde_json::to_vec(request).map_err(|error| error.to_string())?);
    hasher.update(wrapped.as_bytes());
    let key = format!("{:x}", hasher.finalize());
    let cache = cache_dir()?;
    let output_path = cache.join(format!("cue-{key}.png"));
    if artifact_file_is_ready(&output_path) {
        return Ok(output_path);
    }

    let mut text_paths = Vec::new();
    for (index, line) in wrapped.lines().enumerate() {
        let path = cache.join(format!("cue-{key}-line-{index}.txt"));
        std::fs::write(&path, line)
            .map_err(|error| format!("Could not stage Cardboard caption text: {error}"))?;
        text_paths.push(path);
    }
    let filter = renderer_filter(request, layout, &text_paths)?;
    let material = material_path()?;
    let temp_path = cache.join(format!(".cue-{key}-{}.png", uuid::Uuid::new_v4().simple()));
    let ffmpeg = find_ffmpeg().map_err(|error| error.to_string())?;
    let mut command = std::process::Command::new(ffmpeg);
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg(format!(
            "color=c=black@0.0:s={}x{}:r=30,format=rgba",
            request.target_width, request.target_height,
        ))
        .arg("-i")
        .arg(material)
        .arg("-filter_complex")
        .arg(filter)
        .arg("-map")
        .arg("[out]")
        .arg("-frames:v")
        .arg("1")
        .arg("-c:v")
        .arg("png")
        .arg("-pix_fmt")
        .arg("rgba")
        .arg(&temp_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command
        .output()
        .map_err(|error| format!("Could not start Cardboard rendering: {error}"))?;
    if !output.status.success() || !artifact_file_is_ready(&temp_path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!(
            "Cardboard rendering failed: {}",
            ffmpeg_error_tail(&output.stderr)
        ));
    }
    match std::fs::rename(&temp_path, &output_path) {
        Ok(()) => {}
        Err(_) if artifact_file_is_ready(&output_path) => {
            let _ = std::fs::remove_file(&temp_path);
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temp_path);
            return Err(format!("Could not cache Cardboard caption: {error}"));
        }
    }
    Ok(output_path)
}

#[tauri::command]
pub async fn render_cardboard_caption(
    app: AppHandle,
    request: CaptionRequest,
) -> Result<CaptionAsset, String> {
    let path = tokio::task::spawn_blocking(move || render_caption_frame(&request))
        .await
        .map_err(|error| format!("Cardboard renderer stopped unexpectedly: {error}"))??;
    app.asset_protocol_scope()
        .allow_file(&path)
        .map_err(|error| format!("Could not allow Cardboard preview: {error}"))?;
    Ok(CaptionAsset {
        path: path.to_string_lossy().to_string(),
        renderer_version: RENDERER_VERSION.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        artifact_file_is_ready, fitted_caption, placard_layout, render_caption_frame,
        renderer_filter, wrap_text, CaptionRequest, DEFAULT_CARD_SCALE, MATERIAL_BYTES,
        MATERIAL_HEIGHT, MATERIAL_WIDTH, MAX_CARD_SCALE, MIN_CARD_SCALE,
    };
    use std::io::Cursor;
    use std::path::Path;

    fn request(text: &str) -> CaptionRequest {
        CaptionRequest {
            text: text.to_string(),
            target_width: 1080,
            target_height: 1920,
            font_size: 65,
            card_scale: DEFAULT_CARD_SCALE,
            anchor_y: 1862,
            alignment: 2,
        }
    }

    fn alpha_bounds(path: &Path) -> (u32, u32, u32, u32) {
        let decoder = png::Decoder::new(std::fs::File::open(path).expect("caption fixture"));
        let mut reader = decoder.read_info().expect("caption PNG metadata");
        let mut pixels = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut pixels).expect("caption PNG pixels");
        let mut left = info.width;
        let mut top = info.height;
        let mut right = 0;
        let mut bottom = 0;
        for y in 0..info.height {
            for x in 0..info.width {
                let alpha = pixels[((y * info.width + x) * 4 + 3) as usize];
                if alpha > 0 {
                    left = left.min(x);
                    top = top.min(y);
                    right = right.max(x + 1);
                    bottom = bottom.max(y + 1);
                }
            }
        }
        (left, top, right, bottom)
    }

    #[test]
    fn placard_geometry_is_stable_across_caption_changes() {
        let short = placard_layout(&request("WAIT"));
        let long = placard_layout(&request("THAT WAS NOT THE PLAN"));
        assert_eq!(short, long);
        let rendered_ratio = short.width as f64 / short.height as f64;
        let material_ratio = super::MATERIAL_WIDTH / super::MATERIAL_HEIGHT;
        assert!((rendered_ratio - material_ratio).abs() < 0.01);
    }

    #[test]
    fn card_size_scales_the_whole_placard_and_preserves_safe_margins() {
        let mut smallest = request("WAIT");
        smallest.card_scale = MIN_CARD_SCALE;
        let mut default = request("WAIT");
        default.card_scale = DEFAULT_CARD_SCALE;
        let mut largest = request("WAIT");
        largest.card_scale = MAX_CARD_SCALE;

        let small = placard_layout(&smallest);
        let standard = placard_layout(&default);
        let large = placard_layout(&largest);
        assert!(small.width < standard.width && standard.width < large.width);
        assert_eq!(
            standard.width,
            (1080.0 * 0.86 * DEFAULT_CARD_SCALE).round() as i32
        );
        for layout in [small, standard, large] {
            assert!(layout.left >= 0);
            assert!(layout.left + layout.width <= 1080);
            assert!(layout.top >= 0);
            assert!(layout.top + layout.height <= 1920);
        }
    }

    #[test]
    fn offset_extrema_move_the_entire_card_within_safe_margins() {
        let mut upper = request("WAIT");
        upper.anchor_y = (1920.0 * 0.77) as i32;
        let mut lower = request("WAIT");
        lower.anchor_y = (1920.0 * 0.97) as i32;
        let upper_layout = placard_layout(&upper);
        let lower_layout = placard_layout(&lower);
        assert!(lower_layout.top - upper_layout.top > 300);
        for layout in [upper_layout, lower_layout] {
            assert!(layout.top >= 8);
            assert!(layout.top + layout.height <= 1912);
        }
    }

    #[test]
    fn increasing_text_size_never_makes_the_fitted_text_smaller() {
        let mut previous = 0;
        for font_size in [49, 65, 81] {
            let mut variant = request("THAT WAS NOT THE PLAN");
            variant.font_size = font_size;
            let layout = placard_layout(&variant);
            let (_, fitted) = fitted_caption(&variant, layout);
            assert!(fitted >= previous);
            previous = fitted;
        }
    }

    #[test]
    fn text_wrap_preserves_words() {
        assert_eq!(
            wrap_text("THAT WAS NOT THE PLAN", 12),
            "THAT WAS NOT\nTHE PLAN"
        );
    }

    #[test]
    fn material_keeps_reference_aspect_and_transparent_corners() {
        let decoder = png::Decoder::new(Cursor::new(MATERIAL_BYTES));
        let mut reader = decoder.read_info().expect("Cardboard material PNG");
        let mut pixels = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut pixels).expect("Cardboard pixels");
        assert_eq!(info.width as f64, MATERIAL_WIDTH);
        assert_eq!(info.height as f64, MATERIAL_HEIGHT);
        assert_eq!(info.color_type, png::ColorType::Rgba);
        let stride = info.width as usize * 4;
        let corner_alphas = [
            pixels[3],
            pixels[stride - 1],
            pixels[(info.height as usize - 1) * stride + 3],
            pixels[pixels.len() - 1],
        ];
        assert_eq!(corner_alphas, [0, 0, 0, 0]);
        let center = (info.height as usize / 2) * stride + (info.width as usize / 2) * 4;
        assert_eq!(pixels[center + 3], 255);
    }

    #[test]
    fn card_only_and_caption_filters_share_the_same_material_overlay() {
        let request = request("WAIT WHAT");
        let layout = placard_layout(&request);
        let card_only = renderer_filter(&request, layout, &[]).expect("card-only filter");
        let caption = renderer_filter(
            &request,
            layout,
            &[std::path::PathBuf::from("C:\\Temp\\cardboard-caption.txt")],
        )
        .expect("caption filter");
        let overlay = format!(
            "scale={}:{}[card];[0:v][card]overlay={}:{}",
            layout.width, layout.height, layout.left, layout.top,
        );
        assert!(card_only.contains(&overlay));
        assert!(caption.contains(&overlay));
        assert!(caption.contains("fontcolor=#17100A"));
        assert!(caption.contains("textfile='"));
    }

    #[test]
    fn renderer_executes_locally_when_ffmpeg_is_available() {
        if crate::commands::vod::find_ffmpeg().is_err() {
            return;
        }
        let caption = render_caption_frame(&request("WAIT WHAT"))
            .expect("Cardboard caption frame should render");
        let longer_caption = render_caption_frame(&request("THAT WAS NOT THE PLAN"))
            .expect("Longer Cardboard caption frame should render");
        let card_only =
            render_caption_frame(&request("")).expect("Cardboard card-only frame should render");
        assert!(artifact_file_is_ready(&caption));
        assert!(artifact_file_is_ready(&longer_caption));
        assert!(artifact_file_is_ready(&card_only));
        assert_ne!(caption, longer_caption);

        for (label, card_scale, font_size, anchor_y) in [
            ("card-75-text-75", 0.75, 49, 1862),
            ("card-75-text-100", 0.75, 65, 1862),
            ("card-75-text-125", 0.75, 81, 1862),
            ("card-50-text-100", 0.50, 65, 1862),
            ("card-100-text-100", 1.00, 65, 1862),
            ("card-75-offset-minus-20", 0.75, 65, 1478),
        ] {
            let mut fixture = request("THAT WAS NOT THE PLAN");
            fixture.card_scale = card_scale;
            fixture.font_size = font_size;
            fixture.anchor_y = anchor_y;
            let path = render_caption_frame(&fixture).expect("Cardboard fixture should render");
            println!("CARD_BOARD_FIXTURE {label}={}", path.display());
            assert!(artifact_file_is_ready(&path));
            let (left, top, right, bottom) = alpha_bounds(&path);
            assert!(left >= 54 && right <= 1026, "{label} left safe width");
            assert!(top >= 48 && bottom <= 1872, "{label} vertical safe margin");
        }
    }
}
