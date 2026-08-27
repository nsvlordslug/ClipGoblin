//! Native, deterministic compositor for the Undead Legion caption glyph pack.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufWriter, Cursor};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tauri::{AppHandle, Manager};

pub(crate) const RENDERER_VERSION: &str = "undead-legion-image-glyph-v4";
const VISUAL_SIZE_SCALE: f32 = 1.80;

const ATLAS_BYTES: &[u8] = include_bytes!("../../public/caption-glyphs/undead-legion/atlas.png");
const METADATA_JSON: &str = include_str!("../../public/caption-glyphs/undead-legion/metadata.json");

static GLYPH_PACK: OnceLock<Result<GlyphPack, String>> = OnceLock::new();

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionRequest {
    pub text: String,
    pub target_width: u32,
    pub target_height: u32,
    pub font_size: u32,
    pub anchor_y: i32,
    pub alignment: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionAsset {
    pub path: String,
    pub renderer_version: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackMetadata {
    renderer_version: String,
    atlas: AtlasMetadata,
    metrics: PackMetrics,
    glyphs: HashMap<String, GlyphMetadata>,
}

#[derive(Debug, Deserialize)]
struct AtlasMetadata {
    width: u32,
    height: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackMetrics {
    nominal_font_size: f32,
    baseline: f32,
    line_height: f32,
    space_advance: f32,
    letter_spacing: f32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GlyphMetadata {
    #[serde(default)]
    atlas: Option<[u32; 4]>,
    #[serde(default)]
    advance: Option<f32>,
    #[serde(default)]
    origin_x: Option<f32>,
    #[serde(default)]
    baseline: Option<f32>,
    #[serde(default)]
    alias: Option<String>,
    #[cfg_attr(not(test), allow(dead_code))]
    #[serde(default)]
    source_kind: Option<String>,
}

struct GlyphPack {
    metadata: PackMetadata,
    pixels: Vec<u8>,
}

fn load_glyph_pack() -> Result<GlyphPack, String> {
    let metadata: PackMetadata = serde_json::from_str(METADATA_JSON)
        .map_err(|error| format!("Undead Legion glyph metadata is invalid: {error}"))?;
    if metadata.renderer_version != RENDERER_VERSION {
        return Err("Undead Legion glyph metadata version is out of date".to_string());
    }

    let decoder = png::Decoder::new(Cursor::new(ATLAS_BYTES));
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("Undead Legion atlas could not be opened: {error}"))?;
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut pixels)
        .map_err(|error| format!("Undead Legion atlas could not be decoded: {error}"))?;
    pixels.truncate(info.buffer_size());
    if info.width != metadata.atlas.width || info.height != metadata.atlas.height {
        return Err("Undead Legion atlas dimensions do not match its metadata".to_string());
    }
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err("Undead Legion atlas must be an 8-bit RGBA image".to_string());
    }

    Ok(GlyphPack { metadata, pixels })
}

fn glyph_pack() -> Result<&'static GlyphPack, String> {
    match GLYPH_PACK.get_or_init(load_glyph_pack) {
        Ok(pack) => Ok(pack),
        Err(error) => Err(error.clone()),
    }
}

fn glyph_entry<'a>(pack: &'a GlyphPack, character: char) -> Option<&'a GlyphMetadata> {
    let key = character.to_string();
    let mut entry = pack.metadata.glyphs.get(&key)?;
    if let Some(alias) = entry.alias.as_deref() {
        entry = pack.metadata.glyphs.get(alias)?;
    }
    entry.atlas.map(|_| entry)
}

fn normalize_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\u{2018}' | '\u{2019}' => output.push('\''),
            '\u{201C}' | '\u{201D}' => output.push('"'),
            '\u{2013}' | '\u{2014}' => output.push('-'),
            '\u{2026}' => output.push_str("..."),
            '\r' => {}
            '\n' => output.push('\n'),
            '\t' => output.push(' '),
            value if value.is_ascii() => output.push(value),
            _ => output.push('?'),
        }
    }
    output
}

fn validate_request(request: &CaptionRequest) -> Result<(), String> {
    if request.text.trim().is_empty() || !request.text.chars().any(char::is_alphanumeric) {
        return Err("Undead Legion needs spoken caption text".to_string());
    }
    if request.text.chars().count() > 1_000 || request.text.contains('\0') {
        return Err("Undead Legion caption text is too long".to_string());
    }
    if !(320..=3_840).contains(&request.target_width)
        || !(320..=3_840).contains(&request.target_height)
    {
        return Err("Undead Legion output dimensions are unsupported".to_string());
    }
    if !(8..=256).contains(&request.font_size) {
        return Err("Undead Legion font size is unsupported".to_string());
    }
    if request.anchor_y < 0 || request.anchor_y > request.target_height as i32 {
        return Err("Undead Legion caption anchor is outside the frame".to_string());
    }
    if !matches!(request.alignment, 2 | 5 | 8) {
        return Err("Undead Legion caption alignment is unsupported".to_string());
    }
    Ok(())
}

fn design_advance(pack: &GlyphPack, character: char) -> f32 {
    if character == ' ' {
        return pack.metadata.metrics.space_advance;
    }
    glyph_entry(pack, character)
        .and_then(|entry| entry.advance)
        .unwrap_or_else(|| {
            glyph_entry(pack, '?')
                .and_then(|entry| entry.advance)
                .unwrap_or(pack.metadata.metrics.space_advance)
        })
        + pack.metadata.metrics.letter_spacing
}

fn design_line_width(pack: &GlyphPack, text: &str) -> f32 {
    let mut width: f32 = text
        .chars()
        .map(|character| design_advance(pack, character))
        .sum();
    if text
        .chars()
        .last()
        .is_some_and(|character| character != ' ')
    {
        width -= pack.metadata.metrics.letter_spacing;
    }
    width.max(0.0)
}

fn fitted_font_size(pack: &GlyphPack, text: &str, request: &CaptionRequest) -> f32 {
    let safe_width = request.target_width as f32 * 0.78;
    let longest_word = text
        .split_whitespace()
        .map(|word| design_line_width(pack, word))
        .fold(0.0_f32, f32::max);
    // The atlas includes room for glow and aggressive brush overhangs. Scale
    // the visible face up while preserving the existing safe-width clamp.
    let requested = request.font_size as f32 * VISUAL_SIZE_SCALE;
    if longest_word <= 0.0 {
        return requested;
    }
    requested
        .min(safe_width * pack.metadata.metrics.nominal_font_size / longest_word)
        .max(8.0)
}

fn wrap_lines(pack: &GlyphPack, text: &str, scale: f32, safe_width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for source_line in text.lines() {
        let mut current = String::new();
        for word in source_line.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if !current.is_empty() && design_line_width(pack, &candidate) * scale > safe_width {
                lines.push(current);
                current = word.to_string();
            } else {
                current = candidate;
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    if lines.is_empty() {
        lines.push(text.trim().to_string());
    }
    lines
}

fn atlas_pixel(pack: &GlyphPack, x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * pack.metadata.atlas.width + x) * 4) as usize;
    [
        pack.pixels[offset],
        pack.pixels[offset + 1],
        pack.pixels[offset + 2],
        pack.pixels[offset + 3],
    ]
}

fn bilinear_sample(pack: &GlyphPack, rect: [u32; 4], x: f32, y: f32) -> [f32; 4] {
    let local_x = x.clamp(0.0, rect[2].saturating_sub(1) as f32);
    let local_y = y.clamp(0.0, rect[3].saturating_sub(1) as f32);
    let x0 = local_x.floor() as u32;
    let y0 = local_y.floor() as u32;
    let x1 = (x0 + 1).min(rect[2].saturating_sub(1));
    let y1 = (y0 + 1).min(rect[3].saturating_sub(1));
    let tx = local_x - x0 as f32;
    let ty = local_y - y0 as f32;
    let samples = [
        (
            atlas_pixel(pack, rect[0] + x0, rect[1] + y0),
            (1.0 - tx) * (1.0 - ty),
        ),
        (
            atlas_pixel(pack, rect[0] + x1, rect[1] + y0),
            tx * (1.0 - ty),
        ),
        (
            atlas_pixel(pack, rect[0] + x0, rect[1] + y1),
            (1.0 - tx) * ty,
        ),
        (atlas_pixel(pack, rect[0] + x1, rect[1] + y1), tx * ty),
    ];
    let mut premultiplied = [0.0_f32; 4];
    for (pixel, weight) in samples {
        let alpha = pixel[3] as f32 / 255.0;
        premultiplied[0] += pixel[0] as f32 / 255.0 * alpha * weight;
        premultiplied[1] += pixel[1] as f32 / 255.0 * alpha * weight;
        premultiplied[2] += pixel[2] as f32 / 255.0 * alpha * weight;
        premultiplied[3] += alpha * weight;
    }
    premultiplied
}

fn composite_pixel(destination: &mut [u8], source: [f32; 4]) {
    let source_alpha = source[3].clamp(0.0, 1.0);
    if source_alpha <= 0.000_1 {
        return;
    }
    let destination_alpha = destination[3] as f32 / 255.0;
    let output_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
    let destination_scale = destination_alpha * (1.0 - source_alpha);
    for channel in 0..3 {
        let destination_premultiplied = destination[channel] as f32 / 255.0 * destination_scale;
        let value = if output_alpha > 0.0 {
            (source[channel] + destination_premultiplied) / output_alpha
        } else {
            0.0
        };
        destination[channel] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    destination[3] = (output_alpha * 255.0).round() as u8;
}

fn draw_glyph(
    pack: &GlyphPack,
    output: &mut [u8],
    output_width: u32,
    output_height: u32,
    entry: &GlyphMetadata,
    scale: f32,
    destination_x: f32,
    destination_y: f32,
) {
    let Some(rect) = entry.atlas else { return };
    let scaled_width = rect[2] as f32 * scale;
    let scaled_height = rect[3] as f32 * scale;
    let left = destination_x.floor().max(0.0) as u32;
    let top = destination_y.floor().max(0.0) as u32;
    let right = (destination_x + scaled_width)
        .ceil()
        .clamp(0.0, output_width as f32) as u32;
    let bottom = (destination_y + scaled_height)
        .ceil()
        .clamp(0.0, output_height as f32) as u32;

    for y in top..bottom {
        for x in left..right {
            let source_x = ((x as f32 + 0.5 - destination_x) / scale) - 0.5;
            let source_y = ((y as f32 + 0.5 - destination_y) / scale) - 0.5;
            let source = bilinear_sample(pack, rect, source_x, source_y);
            let offset = ((y * output_width + x) * 4) as usize;
            composite_pixel(&mut output[offset..offset + 4], source);
        }
    }
}

fn compose_caption(pack: &GlyphPack, request: &CaptionRequest) -> Result<Vec<u8>, String> {
    validate_request(request)?;
    let text = normalize_text(&request.text);
    let font_size = fitted_font_size(pack, &text, request);
    let scale = font_size / pack.metadata.metrics.nominal_font_size;
    let safe_width = request.target_width as f32 * 0.78;
    let lines = wrap_lines(pack, &text, scale, safe_width);
    let line_height = pack.metadata.metrics.line_height * scale;
    let block_height = line_height * lines.len() as f32;
    let block_top = match request.alignment {
        8 => request.anchor_y as f32,
        5 => request.anchor_y as f32 - block_height / 2.0,
        _ => request.anchor_y as f32 - block_height,
    };
    let mut output = vec![0_u8; (request.target_width * request.target_height * 4) as usize];

    for (line_index, line) in lines.iter().enumerate() {
        let line_width = design_line_width(pack, line) * scale;
        let mut cursor_x = (request.target_width as f32 - line_width) / 2.0;
        let line_baseline =
            block_top + line_index as f32 * line_height + pack.metadata.metrics.line_height * scale
                - (pack.metadata.metrics.line_height - pack.metadata.metrics.baseline) * scale;

        for character in line.chars() {
            if character == ' ' {
                cursor_x += pack.metadata.metrics.space_advance * scale;
                continue;
            }
            let entry = glyph_entry(pack, character)
                .or_else(|| glyph_entry(pack, '?'))
                .ok_or_else(|| "Undead Legion fallback glyph is missing".to_string())?;
            let origin_x = entry.origin_x.unwrap_or(0.0);
            let baseline = entry.baseline.unwrap_or(pack.metadata.metrics.baseline);
            draw_glyph(
                pack,
                &mut output,
                request.target_width,
                request.target_height,
                entry,
                scale,
                cursor_x - origin_x * scale,
                line_baseline - baseline * scale,
            );
            cursor_x += design_advance(pack, character) * scale;
        }
    }
    Ok(output)
}

pub(crate) fn cache_dir() -> Result<PathBuf, String> {
    let path = std::env::temp_dir()
        .join("clipgoblin-undead-legion")
        .join(RENDERER_VERSION);
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("Could not create the Undead Legion cache: {error}"))?;
    Ok(path)
}

fn artifact_file_is_ready(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
}

fn write_png(path: &Path, width: u32, height: u32, pixels: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path)
        .map_err(|error| format!("Could not create Undead Legion frame: {error}"))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder
        .write_header()
        .map_err(|error| format!("Could not start Undead Legion PNG: {error}"))?;
    writer
        .write_image_data(pixels)
        .map_err(|error| format!("Could not write Undead Legion PNG: {error}"))
}

pub(crate) fn render_caption_frame(request: &CaptionRequest) -> Result<PathBuf, String> {
    validate_request(request)?;
    let pack = glyph_pack()?;
    let mut hasher = Sha256::new();
    hasher.update(RENDERER_VERSION.as_bytes());
    hasher.update(serde_json::to_vec(request).map_err(|error| error.to_string())?);
    let key = format!("{:x}", hasher.finalize());
    let cache = cache_dir()?;
    let output_path = cache.join(format!("cue-{key}.png"));
    if artifact_file_is_ready(&output_path) {
        return Ok(output_path);
    }

    let pixels = compose_caption(pack, request)?;
    let temp_path = cache.join(format!(".cue-{key}-{}.png", uuid::Uuid::new_v4().simple()));
    write_png(
        &temp_path,
        request.target_width,
        request.target_height,
        &pixels,
    )?;
    match std::fs::rename(&temp_path, &output_path) {
        Ok(()) => {}
        Err(_) if artifact_file_is_ready(&output_path) => {
            let _ = std::fs::remove_file(&temp_path);
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temp_path);
            return Err(format!("Could not cache Undead Legion caption: {error}"));
        }
    }
    Ok(output_path)
}

#[tauri::command]
pub async fn render_undead_legion_caption(
    app: AppHandle,
    request: CaptionRequest,
) -> Result<CaptionAsset, String> {
    let path = tokio::task::spawn_blocking(move || render_caption_frame(&request))
        .await
        .map_err(|error| format!("Undead Legion renderer stopped unexpectedly: {error}"))??;
    app.asset_protocol_scope()
        .allow_file(&path)
        .map_err(|error| format!("Could not allow Undead Legion preview: {error}"))?;
    Ok(CaptionAsset {
        path: path.to_string_lossy().to_string(),
        renderer_version: RENDERER_VERSION.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        compose_caption, design_line_width, fitted_font_size, glyph_entry, glyph_pack,
        render_caption_frame, wrap_lines, CaptionRequest, RENDERER_VERSION,
    };

    fn alpha_bounds(pixels: &[u8], width: u32, height: u32) -> (u32, u32, u32, u32) {
        let mut left = width;
        let mut top = height;
        let mut right = 0;
        let mut bottom = 0;
        for (index, pixel) in pixels.chunks_exact(4).enumerate() {
            if pixel[3] == 0 {
                continue;
            }
            let x = index as u32 % width;
            let y = index as u32 / width;
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
        }
        assert!(
            right > left && bottom > top,
            "caption should paint visible pixels"
        );
        (left, top, right, bottom)
    }

    fn request(text: &str) -> CaptionRequest {
        CaptionRequest {
            text: text.to_string(),
            target_width: 1080,
            target_height: 1920,
            font_size: 82,
            anchor_y: 960,
            alignment: 5,
        }
    }

    #[test]
    fn pack_covers_caption_ascii_and_has_matching_version() {
        let pack = glyph_pack().expect("glyph pack");
        assert_eq!(pack.metadata.renderer_version, RENDERER_VERSION);
        for character in 'A'..='Z' {
            assert!(
                glyph_entry(pack, character).is_some(),
                "missing {character}"
            );
        }
        for character in 'a'..='z' {
            assert!(
                glyph_entry(pack, character).is_some(),
                "missing {character}"
            );
        }
        for character in '0'..='9' {
            assert!(
                glyph_entry(pack, character).is_some(),
                "missing {character}"
            );
        }
        for character in "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~".chars() {
            assert!(
                glyph_entry(pack, character).is_some(),
                "missing {character}"
            );
        }
    }

    #[test]
    fn pack_has_physical_lowercase_and_provenance() {
        let pack = glyph_pack().expect("glyph pack");
        let lowercase = glyph_entry(pack, 'a').expect("lowercase a");
        assert!(
            lowercase.alias.is_none(),
            "lowercase must be a physical glyph"
        );
        assert_eq!(
            lowercase.source_kind.as_deref(),
            Some("primary-sheet-cleaned")
        );
        assert_eq!(
            glyph_entry(pack, '&').and_then(|entry| entry.source_kind.as_deref()),
            Some("authored-symbol-fallback")
        );
    }

    #[test]
    fn representative_caption_is_transparent_and_contains_lime_magenta_and_black() {
        let pack = glyph_pack().expect("glyph pack");
        let pixels =
            compose_caption(pack, &request("WAIT... WHAT?! 2026")).expect("representative caption");
        assert_eq!(&pixels[..4], &[0, 0, 0, 0]);
        assert_eq!(&pixels[pixels.len() - 4..], &[0, 0, 0, 0]);

        let mut lime = 0;
        let mut magenta = 0;
        let mut black = 0;
        for pixel in pixels.chunks_exact(4).filter(|pixel| pixel[3] > 100) {
            if pixel[1] > 170 && pixel[0] > 70 && pixel[2] < 100 {
                lime += 1;
            }
            if pixel[0] > 145 && pixel[2] > 70 && pixel[0] as i16 - pixel[1] as i16 > 20 {
                magenta += 1;
            }
            if pixel[0] < 45 && pixel[1] < 45 && pixel[2] < 50 {
                black += 1;
            }
        }
        assert!(lime > 1_000, "lime face missing");
        assert!(magenta > 500, "pink lower-face paint missing");
        assert!(black > 1_000, "black directional depth missing");
    }

    #[test]
    fn renderer_is_deterministic_and_cached() {
        let first = render_caption_frame(&request("UNDEAD LEGION")).expect("first frame");
        let second = render_caption_frame(&request("UNDEAD LEGION")).expect("second frame");
        assert_eq!(first, second);
        assert!(std::fs::metadata(first).is_ok());
    }

    #[test]
    fn calibrated_size_is_readable_and_stays_inside_safe_margins() {
        let pack = glyph_pack().expect("glyph pack");
        let mut heights = Vec::new();
        for size_scale in [1.0_f32, 1.25, 1.5] {
            let mut sized_request = request("RUN!");
            sized_request.font_size = (66.0 * size_scale).round() as u32;
            let pixels = compose_caption(pack, &sized_request).expect("calibrated caption");
            let (left, top, right, bottom) = alpha_bounds(
                &pixels,
                sized_request.target_width,
                sized_request.target_height,
            );
            assert!(left >= 54, "Undead Legion crossed the left 5% safe margin");
            assert!(
                right <= 1_026,
                "Undead Legion crossed the right 5% safe margin"
            );
            assert!(top > 0 && bottom < sized_request.target_height);
            heights.push(bottom - top);
        }
        assert!(
            heights[0] >= 105,
            "Undead Legion remains too small at 100%: {}px",
            heights[0]
        );
        assert!(
            heights[1] > heights[0],
            "Undead Legion did not grow at 125%"
        );
        assert!(
            heights[1] >= 140,
            "Undead Legion remains too small at 125%: {}px",
            heights[1]
        );
        assert!(
            heights[2] > heights[1],
            "Undead Legion did not grow at 150%"
        );

        let mut long_request = request("UNCHARACTERISTICALLY");
        long_request.font_size = 99;
        let pixels = compose_caption(pack, &long_request).expect("long caption");
        let (left, _, right, _) = alpha_bounds(
            &pixels,
            long_request.target_width,
            long_request.target_height,
        );
        assert!(
            left >= 54,
            "Undead Legion long word crossed the left safe margin"
        );
        assert!(
            right <= 1_026,
            "Undead Legion long word crossed the right safe margin"
        );

        let wrap_text = "NEVER TURN AROUND THE MONSTER IS RIGHT BEHIND YOU";
        let mut wrap_request = request(wrap_text);
        wrap_request.font_size = (66.0_f32 * 1.25).round() as u32;
        let fitted = fitted_font_size(pack, wrap_text, &wrap_request);
        let scale = fitted / pack.metadata.metrics.nominal_font_size;
        let safe_width = wrap_request.target_width as f32 * 0.78;
        let lines = wrap_lines(pack, wrap_text, scale, safe_width);
        assert!(lines.len() > 1, "Undead Legion did not wrap a long caption");
        assert!(lines
            .iter()
            .all(|line| design_line_width(pack, line) * scale <= safe_width + 1.0));
    }
}
