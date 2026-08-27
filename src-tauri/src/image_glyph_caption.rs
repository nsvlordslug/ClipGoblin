//! Native compositor for caption styles built from transparent image-glyph packs.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufWriter, Cursor};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tauri::{AppHandle, Manager};

pub(crate) const HELLFIRE_RENDERER_VERSION: &str = "hellfire-image-glyph-v3";
pub(crate) const HORROR_RENDERER_VERSION: &str = "horror-image-glyph-v4";
pub(crate) const SCARY_RENDERER_VERSION: &str = "scary-image-glyph-v3";
pub(crate) const GLOSSY_THUMBNAIL_RENDERER_VERSION: &str = "glossy-thumbnail-image-glyph-v1";

const HELLFIRE_ATLAS_BYTES: &[u8] =
    include_bytes!("../../public/caption-glyphs/hellfire/atlas.png");
const HELLFIRE_METADATA_JSON: &str =
    include_str!("../../public/caption-glyphs/hellfire/metadata.json");
const HORROR_ATLAS_BYTES: &[u8] = include_bytes!("../../public/caption-glyphs/horror/atlas.png");
const HORROR_METADATA_JSON: &str = include_str!("../../public/caption-glyphs/horror/metadata.json");
const SCARY_ATLAS_BYTES: &[u8] = include_bytes!("../../public/caption-glyphs/scary/atlas.png");
const SCARY_METADATA_JSON: &str = include_str!("../../public/caption-glyphs/scary/metadata.json");
const GLOSSY_THUMBNAIL_ATLAS_BYTES: &[u8] =
    include_bytes!("../../public/caption-glyphs/glossy-thumbnail/atlas.png");
const GLOSSY_THUMBNAIL_METADATA_JSON: &str =
    include_str!("../../public/caption-glyphs/glossy-thumbnail/metadata.json");
const GLOSSY_THUMBNAIL_CARD_BYTES: &[u8] =
    include_bytes!("../../public/caption-materials/glossy-thumbnail-burst-v1.png");

static HELLFIRE_PACK: OnceLock<Result<GlyphPack, String>> = OnceLock::new();
static HORROR_PACK: OnceLock<Result<GlyphPack, String>> = OnceLock::new();
static SCARY_PACK: OnceLock<Result<GlyphPack, String>> = OnceLock::new();
static GLOSSY_THUMBNAIL_PACK: OnceLock<Result<GlyphPack, String>> = OnceLock::new();

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionRequest {
    pub style_id: String,
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
    style_id: String,
    #[serde(default)]
    text_transform: Option<String>,
    atlas: AtlasMetadata,
    #[serde(default)]
    material_card: Option<AtlasMetadata>,
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

struct PackSource {
    style_id: &'static str,
    display_name: &'static str,
    renderer_version: &'static str,
    visual_size_scale: f32,
    atlas_bytes: &'static [u8],
    metadata_json: &'static str,
    material_card_bytes: Option<&'static [u8]>,
}

struct RgbaAsset {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

struct GlyphPack {
    source: PackSource,
    metadata: PackMetadata,
    pixels: Vec<u8>,
    material_card: Option<RgbaAsset>,
}

fn pack_source(style_id: &str) -> Option<PackSource> {
    match style_id {
        "minimal" => Some(PackSource {
            style_id: "minimal",
            display_name: "Glossy Thumbnail",
            renderer_version: GLOSSY_THUMBNAIL_RENDERER_VERSION,
            visual_size_scale: 1.95,
            atlas_bytes: GLOSSY_THUMBNAIL_ATLAS_BYTES,
            metadata_json: GLOSSY_THUMBNAIL_METADATA_JSON,
            material_card_bytes: Some(GLOSSY_THUMBNAIL_CARD_BYTES),
        }),
        "hellfire" => Some(PackSource {
            style_id: "hellfire",
            display_name: "Hellfire",
            renderer_version: HELLFIRE_RENDERER_VERSION,
            visual_size_scale: 2.36,
            atlas_bytes: HELLFIRE_ATLAS_BYTES,
            metadata_json: HELLFIRE_METADATA_JSON,
            material_card_bytes: None,
        }),
        "horror" => Some(PackSource {
            style_id: "horror",
            display_name: "Horror",
            renderer_version: HORROR_RENDERER_VERSION,
            visual_size_scale: 2.10,
            atlas_bytes: HORROR_ATLAS_BYTES,
            metadata_json: HORROR_METADATA_JSON,
            material_card_bytes: None,
        }),
        "scary" => Some(PackSource {
            style_id: "scary",
            display_name: "Scary",
            renderer_version: SCARY_RENDERER_VERSION,
            visual_size_scale: 1.80,
            atlas_bytes: SCARY_ATLAS_BYTES,
            metadata_json: SCARY_METADATA_JSON,
            material_card_bytes: None,
        }),
        _ => None,
    }
}

pub(crate) fn renderer_version(style_id: &str) -> Option<&'static str> {
    pack_source(style_id).map(|source| source.renderer_version)
}

pub(crate) fn display_name(style_id: &str) -> Option<&'static str> {
    pack_source(style_id).map(|source| source.display_name)
}

fn decode_rgba_asset(
    bytes: &[u8],
    display_name: &str,
    asset_name: &str,
) -> Result<RgbaAsset, String> {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("{display_name} {asset_name} could not be opened: {error}"))?;
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut pixels)
        .map_err(|error| format!("{display_name} {asset_name} could not be decoded: {error}"))?;
    pixels.truncate(info.buffer_size());
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err(format!(
            "{display_name} {asset_name} must be an 8-bit RGBA image"
        ));
    }
    Ok(RgbaAsset {
        width: info.width,
        height: info.height,
        pixels,
    })
}

fn load_glyph_pack(style_id: &str) -> Result<GlyphPack, String> {
    let source = pack_source(style_id)
        .ok_or_else(|| format!("Unknown image-glyph caption style: {style_id}"))?;
    let metadata: PackMetadata = serde_json::from_str(source.metadata_json)
        .map_err(|error| format!("{} glyph metadata is invalid: {error}", source.display_name))?;
    if metadata.renderer_version != source.renderer_version || metadata.style_id != source.style_id
    {
        return Err(format!(
            "{} glyph metadata is out of date",
            source.display_name
        ));
    }

    let atlas = decode_rgba_asset(source.atlas_bytes, source.display_name, "atlas")?;
    if atlas.width != metadata.atlas.width || atlas.height != metadata.atlas.height {
        return Err(format!(
            "{} atlas dimensions do not match its metadata",
            source.display_name
        ));
    }
    let material_card = match (source.material_card_bytes, metadata.material_card.as_ref()) {
        (Some(bytes), Some(expected)) => {
            let card = decode_rgba_asset(bytes, source.display_name, "material card")?;
            if card.width != expected.width || card.height != expected.height {
                return Err(format!(
                    "{} material card dimensions do not match its metadata",
                    source.display_name
                ));
            }
            Some(card)
        }
        (None, None) => None,
        _ => {
            return Err(format!(
                "{} material card metadata is incomplete",
                source.display_name
            ));
        }
    };

    Ok(GlyphPack {
        source,
        metadata,
        pixels: atlas.pixels,
        material_card,
    })
}

fn glyph_pack(style_id: &str) -> Result<&'static GlyphPack, String> {
    let slot = match style_id {
        "minimal" => &GLOSSY_THUMBNAIL_PACK,
        "hellfire" => &HELLFIRE_PACK,
        "horror" => &HORROR_PACK,
        "scary" => &SCARY_PACK,
        _ => return Err(format!("Unknown image-glyph caption style: {style_id}")),
    };
    match slot.get_or_init(|| load_glyph_pack(style_id)) {
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
    let source = pack_source(&request.style_id)
        .ok_or_else(|| format!("Unknown image-glyph caption style: {}", request.style_id))?;
    if request.text.trim().is_empty() || !request.text.chars().any(char::is_alphanumeric) {
        return Err(format!("{} needs spoken caption text", source.display_name));
    }
    if request.text.chars().count() > 1_000 || request.text.contains('\0') {
        return Err(format!("{} caption text is too long", source.display_name));
    }
    if !(320..=3_840).contains(&request.target_width)
        || !(320..=3_840).contains(&request.target_height)
    {
        return Err(format!(
            "{} output dimensions are unsupported",
            source.display_name
        ));
    }
    if !(8..=256).contains(&request.font_size) {
        return Err(format!("{} font size is unsupported", source.display_name));
    }
    if request.anchor_y < 0 || request.anchor_y > request.target_height as i32 {
        return Err(format!(
            "{} caption anchor is outside the frame",
            source.display_name
        ));
    }
    if !matches!(request.alignment, 2 | 5 | 8) {
        return Err(format!(
            "{} caption alignment is unsupported",
            source.display_name
        ));
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
    let safe_width_ratio = if pack.source.style_id == "minimal" {
        0.72
    } else {
        0.78
    };
    let safe_width = request.target_width as f32 * safe_width_ratio;
    let longest_word = text
        .split_whitespace()
        .map(|word| design_line_width(pack, word))
        .fold(0.0_f32, f32::max);
    // Image glyphs reserve substantial transparent room for glow, particles,
    // and sidewalls. Calibrate the painted face to the same perceived size as
    // ordinary caption fonts, then retain the existing safe-width fit.
    let requested = request.font_size as f32 * pack.source.visual_size_scale;
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

fn material_pixel(asset: &RgbaAsset, x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * asset.width + x) * 4) as usize;
    [
        asset.pixels[offset],
        asset.pixels[offset + 1],
        asset.pixels[offset + 2],
        asset.pixels[offset + 3],
    ]
}

fn bilinear_sample_material(asset: &RgbaAsset, x: f32, y: f32) -> [f32; 4] {
    let local_x = x.clamp(0.0, asset.width.saturating_sub(1) as f32);
    let local_y = y.clamp(0.0, asset.height.saturating_sub(1) as f32);
    let x0 = local_x.floor() as u32;
    let y0 = local_y.floor() as u32;
    let x1 = (x0 + 1).min(asset.width.saturating_sub(1));
    let y1 = (y0 + 1).min(asset.height.saturating_sub(1));
    let tx = local_x - x0 as f32;
    let ty = local_y - y0 as f32;
    let samples = [
        (material_pixel(asset, x0, y0), (1.0 - tx) * (1.0 - ty)),
        (material_pixel(asset, x1, y0), tx * (1.0 - ty)),
        (material_pixel(asset, x0, y1), (1.0 - tx) * ty),
        (material_pixel(asset, x1, y1), tx * ty),
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

#[allow(clippy::too_many_arguments)]
fn draw_material_card(
    asset: &RgbaAsset,
    output: &mut [u8],
    output_width: u32,
    output_height: u32,
    destination_x: f32,
    destination_y: f32,
    destination_width: f32,
    destination_height: f32,
) {
    let left = destination_x.floor().max(0.0) as u32;
    let top = destination_y.floor().max(0.0) as u32;
    let right = (destination_x + destination_width)
        .ceil()
        .clamp(0.0, output_width as f32) as u32;
    let bottom = (destination_y + destination_height)
        .ceil()
        .clamp(0.0, output_height as f32) as u32;
    for y in top..bottom {
        for x in left..right {
            let source_x =
                ((x as f32 + 0.5 - destination_x) / destination_width) * asset.width as f32 - 0.5;
            let source_y =
                ((y as f32 + 0.5 - destination_y) / destination_height) * asset.height as f32 - 0.5;
            let source = bilinear_sample_material(asset, source_x, source_y);
            let offset = ((y * output_width + x) * 4) as usize;
            composite_pixel(&mut output[offset..offset + 4], source);
        }
    }
}

struct GlossyCardLayout {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    text_shift_y: f32,
}

fn glossy_card_layout(
    pack: &GlyphPack,
    lines: &[String],
    scale: f32,
    block_top: f32,
    block_height: f32,
    request: &CaptionRequest,
) -> Option<GlossyCardLayout> {
    let card = pack.material_card.as_ref()?;
    let max_line_width = lines
        .iter()
        .map(|line| design_line_width(pack, line) * scale)
        .fold(0.0_f32, f32::max);
    let line_height = pack.metadata.metrics.line_height * scale;
    let padding_x = (line_height * 0.24).max(request.target_width as f32 * 0.025);
    let padding_y = (line_height * 0.18).max(request.target_height as f32 * 0.008);
    let aspect = card.width as f32 / card.height as f32;
    let minimum_height = block_height + padding_y * 2.0;
    let desired_width = (max_line_width + padding_x * 2.0).max(minimum_height * aspect);
    let width = desired_width.clamp(
        request.target_width as f32 * 0.38,
        request.target_width as f32 * 0.90,
    );
    let height = (width / aspect)
        .max(minimum_height)
        .min(request.target_height as f32 * 0.38);
    let desired_top = block_top - (height - block_height) / 2.0;
    let minimum_top = request.target_height as f32 * 0.04;
    let maximum_top = (request.target_height as f32 * 0.96 - height).max(minimum_top);
    let top = desired_top.clamp(minimum_top, maximum_top);
    Some(GlossyCardLayout {
        left: (request.target_width as f32 - width) / 2.0,
        top,
        width,
        height,
        text_shift_y: top - desired_top,
    })
}

#[allow(clippy::too_many_arguments)]
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
    let mut text = normalize_text(&request.text);
    if pack.metadata.text_transform.as_deref() == Some("uppercase") {
        text.make_ascii_uppercase();
    }
    let font_size = fitted_font_size(pack, &text, request);
    let scale = font_size / pack.metadata.metrics.nominal_font_size;
    let safe_width_ratio = if pack.source.style_id == "minimal" {
        0.72
    } else {
        0.78
    };
    let safe_width = request.target_width as f32 * safe_width_ratio;
    let lines = wrap_lines(pack, &text, scale, safe_width);
    let line_height = pack.metadata.metrics.line_height * scale;
    let block_height = line_height * lines.len() as f32;
    let mut block_top = match request.alignment {
        8 => request.anchor_y as f32,
        5 => request.anchor_y as f32 - block_height / 2.0,
        _ => request.anchor_y as f32 - block_height,
    };
    let mut output = vec![0_u8; (request.target_width * request.target_height * 4) as usize];
    if let Some(layout) = glossy_card_layout(pack, &lines, scale, block_top, block_height, request)
    {
        let card = pack
            .material_card
            .as_ref()
            .ok_or_else(|| format!("{} material card is missing", pack.source.display_name))?;
        draw_material_card(
            card,
            &mut output,
            request.target_width,
            request.target_height,
            layout.left,
            layout.top,
            layout.width,
            layout.height,
        );
        block_top += layout.text_shift_y;
    }

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
                .ok_or_else(|| format!("{} fallback glyph is missing", pack.source.display_name))?;
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

pub(crate) fn cache_dir(style_id: &str) -> Result<PathBuf, String> {
    let source = pack_source(style_id)
        .ok_or_else(|| format!("Unknown image-glyph caption style: {style_id}"))?;
    let path = std::env::temp_dir()
        .join("clipgoblin-image-glyph-captions")
        .join(source.style_id)
        .join(source.renderer_version);
    std::fs::create_dir_all(&path).map_err(|error| {
        format!(
            "Could not create the {} cache: {error}",
            source.display_name
        )
    })?;
    Ok(path)
}

fn artifact_file_is_ready(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
}

fn write_png(path: &Path, width: u32, height: u32, pixels: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path)
        .map_err(|error| format!("Could not create image-glyph caption frame: {error}"))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder
        .write_header()
        .map_err(|error| format!("Could not start image-glyph caption PNG: {error}"))?;
    writer
        .write_image_data(pixels)
        .map_err(|error| format!("Could not write image-glyph caption PNG: {error}"))
}

pub(crate) fn render_caption_frame(request: &CaptionRequest) -> Result<PathBuf, String> {
    validate_request(request)?;
    let pack = glyph_pack(&request.style_id)?;
    let mut hasher = Sha256::new();
    hasher.update(pack.source.renderer_version.as_bytes());
    hasher.update(serde_json::to_vec(request).map_err(|error| error.to_string())?);
    let key = format!("{:x}", hasher.finalize());
    let cache = cache_dir(&request.style_id)?;
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
            return Err(format!(
                "Could not cache {} caption: {error}",
                pack.source.display_name
            ));
        }
    }
    Ok(output_path)
}

#[tauri::command]
pub async fn render_image_glyph_caption(
    app: AppHandle,
    request: CaptionRequest,
) -> Result<CaptionAsset, String> {
    let style_id = request.style_id.clone();
    let path = tokio::task::spawn_blocking(move || render_caption_frame(&request))
        .await
        .map_err(|error| format!("Image-glyph caption renderer stopped unexpectedly: {error}"))??;
    app.asset_protocol_scope()
        .allow_file(&path)
        .map_err(|error| format!("Could not allow image-glyph caption preview: {error}"))?;
    let renderer_version = renderer_version(&style_id)
        .ok_or_else(|| format!("Unknown image-glyph caption style: {style_id}"))?;
    Ok(CaptionAsset {
        path: path.to_string_lossy().to_string(),
        renderer_version: renderer_version.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        compose_caption, design_line_width, fitted_font_size, glyph_entry, glyph_pack,
        render_caption_frame, wrap_lines, CaptionRequest, GLOSSY_THUMBNAIL_RENDERER_VERSION,
        HELLFIRE_RENDERER_VERSION, HORROR_RENDERER_VERSION, SCARY_RENDERER_VERSION,
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
            style_id: "hellfire".into(),
            text: text.to_string(),
            target_width: 1080,
            target_height: 1920,
            font_size: 72,
            anchor_y: 960,
            alignment: 5,
        }
    }

    #[test]
    fn hellfire_pack_covers_caption_ascii_and_matches_version() {
        let pack = glyph_pack("hellfire").expect("Hellfire glyph pack");
        assert_eq!(pack.metadata.renderer_version, HELLFIRE_RENDERER_VERSION);
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
    fn hellfire_pack_records_source_and_fallback_provenance() {
        let pack = glyph_pack("hellfire").expect("Hellfire glyph pack");
        assert_eq!(
            glyph_entry(pack, 'A').and_then(|entry| entry.source_kind.as_deref()),
            Some("primary-sheet-cleaned")
        );
        assert_eq!(
            glyph_entry(pack, 'n').and_then(|entry| entry.source_kind.as_deref()),
            Some("source-derived-case-fallback")
        );
        assert_eq!(
            glyph_entry(pack, '_').and_then(|entry| entry.source_kind.as_deref()),
            Some("authored-symbol-fallback")
        );
    }

    #[test]
    fn hellfire_caption_is_transparent_and_keeps_silver_and_red_material() {
        let pack = glyph_pack("hellfire").expect("Hellfire glyph pack");
        let pixels = compose_caption(pack, &request("NO ONE ESCAPES")).expect("caption");
        assert_eq!(&pixels[..4], &[0, 0, 0, 0]);
        assert_eq!(&pixels[pixels.len() - 4..], &[0, 0, 0, 0]);

        let mut silver = 0;
        let mut red = 0;
        let mut dark = 0;
        for pixel in pixels.chunks_exact(4).filter(|pixel| pixel[3] > 90) {
            if pixel[0] > 125
                && (pixel[0] as i16 - pixel[1] as i16).abs() < 35
                && (pixel[1] as i16 - pixel[2] as i16).abs() < 35
            {
                silver += 1;
            }
            if pixel[0] > 75 && pixel[0] as i16 - pixel[1] as i16 > 28 {
                red += 1;
            }
            if pixel[0] < 55 && pixel[1] < 45 && pixel[2] < 48 {
                dark += 1;
            }
        }
        assert!(silver > 1_000, "weathered silver face missing");
        assert!(red > 300, "deep red sidewall missing");
        assert!(dark > 300, "dark separator/contact shadow missing");
    }

    #[test]
    fn hellfire_renderer_is_deterministic_and_cached() {
        let first = render_caption_frame(&request("HELLFIRE")).expect("first frame");
        let second = render_caption_frame(&request("HELLFIRE")).expect("second frame");
        assert_eq!(first, second);
        assert!(std::fs::metadata(first).is_ok());
    }

    #[test]
    fn horror_pack_covers_caption_ascii_and_matches_version() {
        let pack = glyph_pack("horror").expect("Horror glyph pack");
        assert_eq!(pack.metadata.renderer_version, HORROR_RENDERER_VERSION);
        for character in (33_u8..=126).map(char::from) {
            assert!(
                glyph_entry(pack, character).is_some(),
                "missing {character}"
            );
        }
    }

    #[test]
    fn horror_caption_keeps_solid_face_and_granular_lower_breakup() {
        let pack = glyph_pack("horror").expect("Horror glyph pack");
        let mut horror_request = request("DO NOT LOOK BACK");
        horror_request.style_id = "horror".into();
        let pixels = compose_caption(pack, &horror_request).expect("caption");
        assert_eq!(&pixels[..4], &[0, 0, 0, 0]);
        assert_eq!(&pixels[pixels.len() - 4..], &[0, 0, 0, 0]);

        let mut bright = 0;
        let mut ash = 0;
        for pixel in pixels.chunks_exact(4).filter(|pixel| pixel[3] > 30) {
            if pixel[0] > 175 && pixel[1] > 175 && pixel[2] > 175 && pixel[3] > 180 {
                bright += 1;
            }
            if pixel[0] > 45 && pixel[0] < 225 && pixel[3] > 10 && pixel[3] < 220 {
                ash += 1;
            }
        }
        assert!(bright > 1_000, "solid distressed white face missing");
        assert!(ash > 100, "granular lower silhouette breakup missing");
    }

    #[test]
    fn horror_renderer_is_deterministic_and_cached() {
        let mut horror_request = request("HORROR");
        horror_request.style_id = "horror".into();
        let first = render_caption_frame(&horror_request).expect("first frame");
        let second = render_caption_frame(&horror_request).expect("second frame");
        assert_eq!(first, second);
        assert!(std::fs::metadata(first).is_ok());
    }

    #[test]
    fn scary_pack_covers_caption_ascii_and_uses_source_uppercase() {
        let pack = glyph_pack("scary").expect("Scary glyph pack");
        assert_eq!(pack.metadata.renderer_version, SCARY_RENDERER_VERSION);
        assert_eq!(pack.metadata.text_transform.as_deref(), Some("uppercase"));
        for character in (33_u8..=126).map(char::from) {
            assert!(
                glyph_entry(pack, character).is_some(),
                "missing {character}"
            );
        }
        assert_eq!(
            glyph_entry(pack, 'A').and_then(|entry| entry.source_kind.as_deref()),
            Some("primary-sheet-cleaned")
        );
        assert_eq!(
            glyph_entry(pack, 'a').and_then(|entry| entry.source_kind.as_deref()),
            Some("source-derived-case-fallback")
        );
    }

    #[test]
    fn scary_caption_keeps_red_brush_material_and_uppercase_transform() {
        let pack = glyph_pack("scary").expect("Scary glyph pack");
        let mut lowercase_request = request("do not look back");
        lowercase_request.style_id = "scary".into();
        let mut uppercase_request = lowercase_request.clone();
        uppercase_request.text = "DO NOT LOOK BACK".into();
        let lowercase = compose_caption(pack, &lowercase_request).expect("lowercase caption");
        let uppercase = compose_caption(pack, &uppercase_request).expect("uppercase caption");
        assert_eq!(lowercase, uppercase);
        assert_eq!(&lowercase[..4], &[0, 0, 0, 0]);
        assert_eq!(&lowercase[lowercase.len() - 4..], &[0, 0, 0, 0]);

        let mut red = 0;
        let mut dark = 0;
        for pixel in lowercase.chunks_exact(4).filter(|pixel| pixel[3] > 80) {
            if pixel[0] > 105
                && pixel[0] as i16 - pixel[1] as i16 > 65
                && pixel[0] as i16 - pixel[2] as i16 > 55
            {
                red += 1;
            }
            if pixel[0] < 55 && pixel[1] < 35 && pixel[2] < 38 {
                dark += 1;
            }
        }
        assert!(red > 700, "distressed red brush face missing");
        assert!(dark > 120, "dark separator/contact shadow missing");
    }

    #[test]
    fn scary_renderer_is_deterministic_and_cached() {
        let mut scary_request = request("SCARY");
        scary_request.style_id = "scary".into();
        let first = render_caption_frame(&scary_request).expect("first frame");
        let second = render_caption_frame(&scary_request).expect("second frame");
        assert_eq!(first, second);
        assert!(std::fs::metadata(first).is_ok());
    }

    #[test]
    fn glossy_thumbnail_replaces_minimal_with_complete_reusable_material_glyphs() {
        let pack = glyph_pack("minimal").expect("Glossy Thumbnail glyph pack");
        assert_eq!(
            pack.metadata.renderer_version,
            GLOSSY_THUMBNAIL_RENDERER_VERSION
        );
        assert_eq!(pack.metadata.text_transform.as_deref(), Some("uppercase"));
        assert!(pack.material_card.is_some(), "purple burst card missing");
        for character in (33_u8..=126).map(char::from) {
            assert!(
                glyph_entry(pack, character).is_some(),
                "missing {character}"
            );
        }
    }

    #[test]
    fn glossy_thumbnail_keeps_face_edge_depth_and_burst_card() {
        let pack = glyph_pack("minimal").expect("Glossy Thumbnail glyph pack");
        let mut glossy_request = request("THAT WAS WILD");
        glossy_request.style_id = "minimal".into();
        glossy_request.font_size = 66;
        let pixels = compose_caption(pack, &glossy_request).expect("caption");
        assert_eq!(&pixels[..4], &[0, 0, 0, 0]);
        assert_eq!(&pixels[pixels.len() - 4..], &[0, 0, 0, 0]);

        let mut orange = 0;
        let mut white = 0;
        let mut gold_depth = 0;
        let mut purple = 0;
        for pixel in pixels.chunks_exact(4).filter(|pixel| pixel[3] > 80) {
            if pixel[0] > 215 && pixel[1] > 60 && pixel[2] < 55 {
                orange += 1;
            }
            if pixel[0] > 235 && pixel[1] > 235 && pixel[2] > 215 {
                white += 1;
            }
            if pixel[0] > 65 && pixel[0] as i16 > pixel[1] as i16 * 2 && pixel[2] < 35 {
                gold_depth += 1;
            }
            if pixel[2] > 75 && pixel[0] > 35 && pixel[0] as i16 > pixel[1] as i16 * 2 {
                purple += 1;
            }
        }
        assert!(orange > 1_000, "orange-to-yellow face missing");
        assert!(white > 300, "clean white keyline missing");
        assert!(gold_depth > 300, "gold directional depth missing");
        assert!(purple > 2_000, "purple burst card missing");
    }

    #[test]
    fn glossy_thumbnail_renderer_is_deterministic_and_cached() {
        let mut glossy_request = request("GLOSSY THUMBNAIL");
        glossy_request.style_id = "minimal".into();
        glossy_request.font_size = 66;
        let first = render_caption_frame(&glossy_request).expect("first frame");
        let second = render_caption_frame(&glossy_request).expect("second frame");
        assert_eq!(first, second);
        assert!(std::fs::metadata(first).is_ok());
    }

    #[test]
    fn calibrated_glyph_styles_are_readable_and_stay_inside_safe_margins() {
        for (style_id, base_size) in [
            ("minimal", 66.0_f32),
            ("hellfire", 62.0),
            ("horror", 62.0),
            ("scary", 62.0),
        ] {
            let pack = glyph_pack(style_id).expect("glyph pack");
            let mut heights = Vec::new();
            for size_scale in [1.0_f32, 1.25, 1.5] {
                let mut sized_request = request("RUN!");
                sized_request.style_id = style_id.to_string();
                sized_request.font_size = (base_size * size_scale).round() as u32;
                let pixels = compose_caption(pack, &sized_request).expect("calibrated caption");
                let (left, top, right, bottom) = alpha_bounds(
                    &pixels,
                    sized_request.target_width,
                    sized_request.target_height,
                );
                assert!(left >= 54, "{style_id} crossed the left 5% safe margin");
                assert!(
                    right <= 1_026,
                    "{style_id} crossed the right 5% safe margin"
                );
                assert!(top > 0 && bottom < sized_request.target_height);
                heights.push(bottom - top);
            }
            let (minimum_100, minimum_125) = match style_id {
                "minimal" => (105, 130),
                "hellfire" => (110, 135),
                "horror" => (115, 145),
                "scary" => (75, 95),
                _ => unreachable!("tested style is registered"),
            };
            assert!(
                heights[0] >= minimum_100,
                "{style_id} remains too small at 100%: {}px",
                heights[0]
            );
            assert!(heights[1] > heights[0], "{style_id} did not grow at 125%");
            assert!(
                heights[1] >= minimum_125,
                "{style_id} remains too small at 125%: {}px",
                heights[1]
            );
            assert!(heights[2] > heights[1], "{style_id} did not grow at 150%");

            let mut long_request = request("UNCHARACTERISTICALLY");
            long_request.style_id = style_id.to_string();
            long_request.font_size = (base_size * 1.5).round() as u32;
            let pixels = compose_caption(pack, &long_request).expect("long caption");
            let (left, _, right, _) = alpha_bounds(
                &pixels,
                long_request.target_width,
                long_request.target_height,
            );
            assert!(
                left >= 54,
                "{style_id} long word crossed the left safe margin"
            );
            assert!(
                right <= 1_026,
                "{style_id} long word crossed the right safe margin"
            );

            let wrap_text = "NEVER TURN AROUND THE MONSTER IS RIGHT BEHIND YOU";
            let mut wrap_request = request(wrap_text);
            wrap_request.style_id = style_id.to_string();
            wrap_request.font_size = (base_size * 1.25).round() as u32;
            let fitted = fitted_font_size(pack, wrap_text, &wrap_request);
            let scale = fitted / pack.metadata.metrics.nominal_font_size;
            let safe_width = wrap_request.target_width as f32 * 0.78;
            let lines = wrap_lines(pack, wrap_text, scale, safe_width);
            assert!(lines.len() > 1, "{style_id} did not wrap a long caption");
            assert!(lines
                .iter()
                .all(|line| design_line_width(pack, line) * scale <= safe_width + 1.0));
        }
    }
}
