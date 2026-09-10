#!/usr/bin/env python3
"""Generate ClipGoblin's reusable Undead Legion image-glyph pack.

The approved full alphabet sheet supplies the visible glyph silhouettes and
paint treatment. Each available character is cropped, cleaned to transparency,
and stored as a reusable image glyph. Symbols absent from the sheet use the
existing ClipGoblin-authored brush fallback. No installed or bundled typeface
supplies the caption glyph skeleton.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import random
from dataclasses import dataclass
from collections import deque
from pathlib import Path

import numpy as np
from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont

from undead_legion_brush_paths import BRUSH_GLYPHS, BrushGlyph, BrushStroke


ROOT = Path(__file__).resolve().parents[1]
LABEL_FONT_PATH = ROOT / "public" / "fonts" / "Bangers-Regular.ttf"
OUTPUT_DIR = ROOT / "public" / "caption-glyphs" / "undead-legion"
DESIGN_DIR = ROOT / "docs" / "design-tests"

RENDERER_VERSION = "undead-legion-image-glyph-v4"
SOURCE_LAYOUT_SIZE = (1473, 768)
CELL_WIDTH = 360
CELL_HEIGHT = 424
CELL_ORIGIN_X = 54
FACE_TOP = 35
DESIGN_X = 272
DESIGN_Y = 272
BRUSH_WIDTH_SCALE = 278
BASELINE = 314
NOMINAL_FONT_SIZE = 272
LINE_HEIGHT = 352
ATLAS_COLUMNS = 9
PRINTABLE_GLYPHS = (
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
    "abcdefghijklmnopqrstuvwxyz"
    "0123456789"
    "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~"
)

LIME = (178, 255, 28, 255)
LIME_DARK = (76, 188, 31, 255)
MAGENTA = (255, 48, 205, 255)
MAGENTA_DARK = (165, 17, 117, 255)
BLACK = (8, 7, 12, 255)
CHARCOAL = (21, 15, 24, 255)


@dataclass(frozen=True)
class SourceGlyphSpec:
    box: tuple[int, int, int, int]
    baseline_y: int
    scale: float


def source_layout() -> dict[str, SourceGlyphSpec]:
    layout: dict[str, SourceGlyphSpec] = {}

    def add_row(
        characters: str,
        boxes: list[tuple[int, int]],
        y_range: tuple[int, int],
        baseline_y: int,
        scale: float,
    ) -> None:
        if len(characters) != len(boxes):
            raise ValueError(f"Source row mismatch for {characters}")
        for character, (left, right) in zip(characters, boxes):
            layout[character] = SourceGlyphSpec(
                (left, y_range[0], right, y_range[1]),
                baseline_y,
                scale,
            )

    add_row(
        "ABCDEFGHIJKLM",
        [(60, 163), (163, 269), (269, 362), (362, 475), (475, 588),
         (588, 692), (692, 790), (790, 903), (903, 971), (971, 1079),
         (1079, 1182), (1182, 1288), (1288, 1410)],
        (54, 198), 189, 1.62,
    )
    add_row(
        "NOPQRSTUVWXYZ",
        [(58, 164), (164, 269), (269, 374), (374, 476), (476, 581),
         (581, 677), (677, 789), (789, 889), (889, 992), (992, 1107),
         (1107, 1210), (1210, 1307), (1307, 1412)],
        (209, 363), 359, 1.60,
    )
    add_row(
        "abcdefghijklmnopqrstuvwxyz",
        [(59, 121), (121, 178), (178, 231), (231, 288), (288, 341),
         (341, 392), (392, 445), (445, 500), (500, 537), (537, 576),
         (576, 631), (631, 664), (664, 735), (735, 791), (791, 847),
         (847, 901), (901, 956), (956, 1001), (1001, 1048), (1048, 1095),
         (1095, 1150), (1150, 1204), (1204, 1268), (1268, 1326),
         (1326, 1379), (1379, 1435)],
        (416, 559), 534, 1.76,
    )
    add_row(
        "0123456789",
        [(44, 119), (119, 174), (174, 237), (237, 299), (299, 360),
         (360, 420), (420, 482), (482, 546), (546, 609), (609, 690)],
        (610, 766), 756, 1.52,
    )

    punctuation = {
        ",": (725, 771), ";": (771, 805), ":": (805, 836),
        "!": (836, 868), "?": (868, 914), '"': (914, 952),
        "(": (952, 987), ")": (987, 1020), "[": (1020, 1053),
        "]": (1053, 1086), "+": (1086, 1124), "-": (1124, 1164),
        "*": (1164, 1203), "/": (1203, 1240), "=": (1240, 1275),
        "#": (1275, 1316), "$": (1316, 1367), "%": (1367, 1451),
    }
    for character, (left, right) in punctuation.items():
        layout[character] = SourceGlyphSpec((left, 610, right, 766), 756, 1.52)
    return layout


SOURCE_LAYOUT = source_layout()


def stable_seed(value: str) -> int:
    return int.from_bytes(hashlib.sha256(value.encode("utf-8")).digest()[:8], "big")


def shifted(mask: Image.Image, x: int, y: int) -> Image.Image:
    result = Image.new("L", mask.size)
    result.paste(mask, (x, y))
    return result


def dilated(mask: Image.Image, radius: int) -> Image.Image:
    size = radius * 2 + 1
    return mask.filter(ImageFilter.MaxFilter(size))


def solid_layer(mask: Image.Image, color: tuple[int, int, int, int]) -> Image.Image:
    layer = Image.new("RGBA", mask.size, color)
    layer.putalpha(ImageChops.multiply(mask, Image.new("L", mask.size, color[3])))
    return layer


def alpha_over(canvas: Image.Image, mask: Image.Image, color: tuple[int, int, int, int]) -> None:
    canvas.alpha_composite(solid_layer(mask, color))


def resample_path(points: list[tuple[float, float]], spacing: float = 4.0) -> list[tuple[float, float]]:
    sampled: list[tuple[float, float]] = [points[0]]
    for start, end in zip(points, points[1:]):
        distance = math.hypot(end[0] - start[0], end[1] - start[1])
        steps = max(1, math.ceil(distance / spacing))
        for index in range(1, steps + 1):
            amount = index / steps
            sampled.append((
                start[0] + (end[0] - start[0]) * amount,
                start[1] + (end[1] - start[1]) * amount,
            ))
    return sampled


def transformed_points(
    stroke_spec: BrushStroke,
    character: str,
) -> list[tuple[float, float]]:
    rng = random.Random(stable_seed(f"shape:{character}"))
    slant = rng.uniform(-0.035, 0.075)
    transformed = []
    for x, y in stroke_spec.points:
        x_with_slant = x + slant * (0.55 - y)
        transformed.append((
            CELL_ORIGIN_X + x_with_slant * DESIGN_X,
            FACE_TOP + y * DESIGN_Y,
        ))
    return resample_path(transformed)


def draw_variable_brush_stroke(
    mask: Image.Image,
    character: str,
    stroke_index: int,
    stroke_spec: BrushStroke,
) -> None:
    points = transformed_points(stroke_spec, character)
    if len(points) < 2:
        return
    rng = random.Random(stable_seed(f"ribbon:{character}:{stroke_index}"))
    phase = rng.uniform(0, math.tau)
    base_width = stroke_spec.width * BRUSH_WIDTH_SCALE
    left_edge: list[tuple[float, float]] = []
    right_edge: list[tuple[float, float]] = []

    for index, point in enumerate(points):
        before = points[max(0, index - 1)]
        after = points[min(len(points) - 1, index + 1)]
        tangent_x = after[0] - before[0]
        tangent_y = after[1] - before[1]
        length = max(0.001, math.hypot(tangent_x, tangent_y))
        normal_x = -tangent_y / length
        normal_y = tangent_x / length
        rhythm = math.sin(index * 0.37 + phase) * 0.11
        width = base_width * (0.91 + rhythm + rng.uniform(-0.035, 0.035))
        if not stroke_spec.closed:
            terminal_distance = min(index, len(points) - 1 - index)
            if terminal_distance < 4:
                width *= 0.24 + terminal_distance * 0.19
        offset = width / 2.0
        jitter = rng.uniform(-0.55, 0.55)
        center_x = point[0] + normal_x * jitter
        center_y = point[1] + normal_y * jitter
        left_edge.append((center_x + normal_x * offset, center_y + normal_y * offset))
        right_edge.append((center_x - normal_x * offset, center_y - normal_y * offset))

    polygon = left_edge + list(reversed(right_edge))
    ImageDraw.Draw(mask).polygon(polygon, fill=255)


def draw_irregular_dot(
    mask: Image.Image,
    character: str,
    dot_index: int,
    dot: tuple[float, float, float],
) -> None:
    x, y, radius = dot
    center_x = CELL_ORIGIN_X + x * DESIGN_X
    center_y = FACE_TOP + y * DESIGN_Y
    radius_px = radius * DESIGN_X
    rng = random.Random(stable_seed(f"dot:{character}:{dot_index}"))
    points = []
    for index in range(9):
        angle = index / 9 * math.tau
        local_radius = radius_px * rng.uniform(0.80, 1.15)
        points.append((
            center_x + math.cos(angle) * local_radius,
            center_y + math.sin(angle) * local_radius,
        ))
    ImageDraw.Draw(mask).polygon(points, fill=255)


def render_base_mask(character: str) -> Image.Image:
    design = BRUSH_GLYPHS.get(character, BRUSH_GLYPHS["?"])
    mask = Image.new("L", (CELL_WIDTH, CELL_HEIGHT))
    for index, stroke_spec in enumerate(design.strokes):
        draw_variable_brush_stroke(mask, character, index, stroke_spec)
    for index, dot in enumerate(design.dots):
        draw_irregular_dot(mask, character, index, dot)
    return mask


def roughen_mask(mask: Image.Image, character: str) -> Image.Image:
    """Cut deterministic terminal notches and dry-brush bites into the authored mask."""
    bounds = mask.getbbox()
    if bounds is None:
        return mask
    rng = random.Random(stable_seed(f"rough:{character}"))
    left, top, right, bottom = bounds
    eraser = Image.new("L", mask.size)
    draw = ImageDraw.Draw(eraser)

    # Larger triangular end cuts stop the silhouette reading like a normal font.
    count = 7 if character.isalpha() else 4 if character.isdigit() else 2
    for _ in range(count):
        edge = rng.choice(("left", "right", "bottom"))
        radius = rng.randint(2, 6)
        if edge == "left":
            x = left + rng.randint(-1, 2)
            y = rng.randint(top + 6, max(top + 7, bottom - 6))
        elif edge == "right":
            x = right + rng.randint(-2, 1)
            y = rng.randint(top + 6, max(top + 7, bottom - 6))
        else:
            x = rng.randint(left + 5, max(left + 6, right - 5))
            y = bottom + rng.randint(-2, 1)
        if rng.random() < 0.62:
            draw.polygon(
                ((x - radius, y - radius), (x + radius * 2, y), (x - radius, y + radius)),
                fill=255,
            )
        else:
            draw.ellipse((x - radius, y - radius, x + radius, y + radius), fill=255)

    if character.isalnum() and right - left > 45:
        for _ in range(3):
            x = rng.randint(left + 10, right - 10)
            y = rng.randint(top + 18, bottom - 12)
            length = rng.randint(10, 24)
            draw.line((x, y, x + rng.randint(-8, 7), y + length), fill=rng.randint(80, 165), width=rng.choice((1, 2, 3)))

    return ImageChops.subtract(mask, eraser)


def keep_material_components(mask: np.ndarray, character: str) -> np.ndarray:
    height, width = mask.shape
    visited = np.zeros_like(mask, dtype=bool)
    components: list[list[tuple[int, int]]] = []
    for y, x in np.argwhere(mask):
        if visited[y, x]:
            continue
        queue: deque[tuple[int, int]] = deque([(int(y), int(x))])
        visited[y, x] = True
        component: list[tuple[int, int]] = []
        while queue:
            current_y, current_x = queue.popleft()
            component.append((current_y, current_x))
            for next_y in range(max(0, current_y - 1), min(height, current_y + 2)):
                for next_x in range(max(0, current_x - 1), min(width, current_x + 2)):
                    if mask[next_y, next_x] and not visited[next_y, next_x]:
                        visited[next_y, next_x] = True
                        queue.append((next_y, next_x))
        components.append(component)

    if not components:
        return mask
    largest = max(len(component) for component in components)
    minimum = max(8, round(largest * 0.012))
    expected_components = {
        ",": 2,
        ";": 3,
        ":": 2,
        "!": 2,
        "?": 2,
        '"': 2,
        "=": 2,
        "%": 3,
    }.get(character, 2 if character in "ij" else 1)
    center_x = width / 2.0
    viable: list[tuple[float, list[tuple[int, int]]]] = []
    for component in components:
        if len(component) < minimum:
            continue
        xs = [point[1] for point in component]
        component_center = (min(xs) + max(xs)) / 2.0
        center_weight = 1.0 - min(1.0, abs(component_center - center_x) / max(1.0, width / 2.0))
        score = len(component) * (0.42 + center_weight * 0.58)
        viable.append((score, component))
    selected = [
        component
        for _, component in sorted(viable, key=lambda item: item[0], reverse=True)[:expected_components]
    ]
    cleaned = np.zeros_like(mask, dtype=bool)
    for component in selected:
        ys, xs = zip(*component)
        cleaned[np.array(ys), np.array(xs)] = True
    return cleaned


def source_face_mask(crop: Image.Image, character: str) -> Image.Image:
    pixels = np.asarray(crop.convert("RGB"), dtype=np.int16)
    red = pixels[:, :, 0]
    green = pixels[:, :, 1]
    blue = pixels[:, :, 2]
    lime = (
        (green > 145)
        & (red > 85)
        & (blue < 190)
        & ((red + green) > 270)
        & ((green - blue) > 15)
    )
    magenta = (
        (red > 135)
        & (blue > 65)
        & ((red + blue) > 230)
        & ((red - green) > 20)
    )
    cleaned = keep_material_components(lime | magenta, character)
    return Image.fromarray((cleaned.astype(np.uint8) * 255), mode="L")


def clean_reference_material(crop: Image.Image, face: Image.Image) -> Image.Image:
    canvas = Image.new("RGBA", crop.size)
    depth = Image.new("L", face.size)
    for step in range(10, 2, -2):
        depth = ImageChops.lighter(depth, shifted(dilated(face, 2), -step // 3, step))
    glow_shape = dilated(depth, 4).filter(ImageFilter.GaussianBlur(3.5))
    alpha_over(canvas, glow_shape, (112, 255, 48, 82))

    neon_rim = ImageChops.subtract(dilated(depth, 4), dilated(depth, 2))
    alpha_over(canvas, neon_rim, (117, 255, 47, 205))
    alpha_over(canvas, depth, BLACK)
    alpha_over(canvas, dilated(face, 3), CHARCOAL)

    material = crop.convert("RGBA")
    material.putalpha(face)
    canvas.alpha_composite(material)
    return canvas


def render_reference_glyph(
    sheet: Image.Image,
    character: str,
    spec: SourceGlyphSpec,
) -> tuple[Image.Image, int, int]:
    scale_x = sheet.width / SOURCE_LAYOUT_SIZE[0]
    scale_y = sheet.height / SOURCE_LAYOUT_SIZE[1]
    source_box = (
        round(spec.box[0] * scale_x),
        round(spec.box[1] * scale_y),
        round(spec.box[2] * scale_x),
        round(spec.box[3] * scale_y),
    )
    source_baseline = round(spec.baseline_y * scale_y)
    crop = sheet.crop(source_box).convert("RGB")
    face = source_face_mask(crop, character)
    if face.getbbox() is None:
        raise ValueError(f"No source material found for {character!r} in {spec.box}")
    padding = 22
    padded_crop = Image.new("RGB", (crop.width + padding * 2, crop.height + padding * 2))
    padded_crop.paste(crop, (padding, padding))
    padded_face = Image.new("L", padded_crop.size)
    padded_face.paste(face, (padding, padding))
    material = clean_reference_material(padded_crop, padded_face)
    bounds = material.getbbox()
    if bounds is None:
        raise ValueError(f"No cleaned source glyph found for {character!r}")
    glyph = material.crop(bounds)

    scale = spec.scale / scale_y
    maximum_width = CELL_WIDTH - CELL_ORIGIN_X - 18
    maximum_height = CELL_HEIGHT - 28
    scale = min(
        scale,
        maximum_width / max(1, glyph.width),
        maximum_height / max(1, glyph.height),
    )
    resized = glyph.resize(
        (max(1, round(glyph.width * scale)), max(1, round(glyph.height * scale))),
        Image.Resampling.LANCZOS,
    ).filter(ImageFilter.UnsharpMask(radius=0.8, percent=115, threshold=2))

    baseline_in_glyph = (
        padding + source_baseline - source_box[1] - bounds[1]
    ) * scale
    destination_y = round(BASELINE - baseline_in_glyph)
    destination_y = max(0, min(CELL_HEIGHT - resized.height, destination_y))
    cell = Image.new("RGBA", (CELL_WIDTH, CELL_HEIGHT))
    cell.alpha_composite(resized, (CELL_ORIGIN_X, destination_y))
    advance = max(20, round(resized.width * (0.82 if character.isalpha() else 0.88) + 5))
    return cell, advance, BASELINE


def paint_mask_for_face(
    face: Image.Image,
    character: str,
    bounds: tuple[int, int, int, int],
) -> Image.Image:
    """Build irregular magenta paint clipped inside the lower glyph face."""
    rng = random.Random(stable_seed(f"paint:{character}"))
    left, top, right, bottom = bounds
    height = max(1, bottom - top)
    width = max(1, right - left)
    paint = Image.new("L", face.size)
    draw = ImageDraw.Draw(paint)

    points: list[tuple[int, int]] = [(left - 8, bottom + 8)]
    step = max(3, width // 12)
    phase = rng.random() * math.tau
    for x in range(left - 8, right + 9, step):
        wave = math.sin((x - left) / max(1, width) * math.tau * 1.7 + phase)
        jitter = rng.uniform(-0.035, 0.035)
        boundary = top + height * (0.68 + wave * 0.085 + jitter)
        points.append((x, int(boundary)))
    points.extend(((right + 8, bottom + 8), (left - 8, bottom + 8)))
    draw.polygon(points, fill=235)

    # Dry-brush tongues ensure the pink reads as paint inside the lime face.
    for _ in range(max(2, width // 48)):
        x = rng.randint(left, max(left, right - 1))
        tongue_top = int(top + height * rng.uniform(0.49, 0.66))
        tongue_bottom = int(top + height * rng.uniform(0.74, 0.91))
        tongue_width = rng.randint(2, max(3, min(8, width // 8)))
        draw.polygon(
            [
                (x - tongue_width, tongue_bottom),
                (x, tongue_top),
                (x + tongue_width, tongue_bottom),
            ],
            fill=rng.randint(110, 205),
        )

    # A few lime gaps remain in the lower paint like worn brush coverage.
    for _ in range(max(2, width // 55)):
        x = rng.randint(left, max(left, right - 1))
        y = rng.randint(int(top + height * 0.70), max(int(top + height * 0.71), bottom))
        draw.line((x, y, x + rng.randint(-4, 5), y + rng.randint(5, 14)), fill=35, width=1)

    paint = paint.filter(ImageFilter.GaussianBlur(2.8))
    return ImageChops.multiply(paint, face)


def face_gradient(size: tuple[int, int], bounds: tuple[int, int, int, int]) -> Image.Image:
    left, top, right, bottom = bounds
    height = max(1, bottom - top)
    gradient = Image.new("RGBA", size)
    pixels = gradient.load()
    for y in range(max(0, top), min(size[1], bottom + 1)):
        t = max(0.0, min(1.0, (y - top) / height))
        r = round(LIME[0] * (1 - t) + LIME_DARK[0] * t)
        g = round(LIME[1] * (1 - t) + LIME_DARK[1] * t)
        b = round(LIME[2] * (1 - t) + LIME_DARK[2] * t)
        for x in range(max(0, left), min(size[0], right + 1)):
            pixels[x, y] = (r, g, b, 255)
    return gradient


def add_face_texture(
    canvas: Image.Image,
    face: Image.Image,
    character: str,
    bounds: tuple[int, int, int, int],
) -> None:
    rng = random.Random(stable_seed(f"texture:{character}"))
    left, top, right, bottom = bounds
    texture_alpha = Image.new("L", face.size)
    draw = ImageDraw.Draw(texture_alpha)

    for _ in range(max(5, (right - left) // 18)):
        x = rng.randint(left, max(left, right - 1))
        y = rng.randint(top, max(top, bottom - 1))
        draw.line(
            (x, y, x + rng.randint(-7, 8), y + rng.randint(5, 18)),
            fill=rng.randint(45, 115),
            width=rng.choice((1, 1, 2)),
        )
    for _ in range(max(8, (right - left) // 12)):
        x = rng.randint(left, max(left, right - 1))
        y = rng.randint(top, max(top, bottom - 1))
        radius = rng.choice((1, 1, 2))
        draw.ellipse((x - radius, y - radius, x + radius, y + radius), fill=rng.randint(30, 90))

    texture_alpha = ImageChops.multiply(texture_alpha, face)
    alpha_over(canvas, texture_alpha, (5, 8, 6, 125))


def glyph_baseline(character: str) -> int:
    if not character.isalnum():
        return BASELINE
    return BASELINE + (stable_seed(f"baseline:{character}") % 15) - 7


def glyph_advance(character: str) -> int:
    design = BRUSH_GLYPHS[character]
    overlap = 0.78 if character.isalpha() else 0.84
    return max(20, round(design.width * DESIGN_X * overlap + 15))


def render_glyph(character: str) -> tuple[Image.Image, int, int]:
    face = roughen_mask(render_base_mask(character), character)
    bounds = face.getbbox()
    if bounds is None:
        return Image.new("RGBA", (CELL_WIDTH, CELL_HEIGHT)), 0, BASELINE

    canvas = Image.new("RGBA", (CELL_WIDTH, CELL_HEIGHT))

    glow = shifted(dilated(face, 10), -2, 4).filter(ImageFilter.GaussianBlur(8.0))
    alpha_over(canvas, glow, (112, 255, 48, 155))

    # The deepest directional layer reads as physical black extruded paint.
    depth = Image.new("L", face.size)
    for step in range(13, 2, -2):
        depth = ImageChops.lighter(depth, shifted(dilated(face, 4), -step // 2, step))
    alpha_over(canvas, depth, BLACK)

    # A magenta sidewall bridges the black extrusion and the painted face.
    sidewall = Image.new("L", face.size)
    for step in range(7, 2, -1):
        sidewall = ImageChops.lighter(sidewall, shifted(dilated(face, 2), -step // 4, step))
    sidewall = ImageChops.subtract(sidewall, dilated(face, 3))
    alpha_over(canvas, sidewall, MAGENTA_DARK)

    neon_rim = ImageChops.subtract(dilated(face, 10), dilated(face, 6))
    alpha_over(canvas, neon_rim, (117, 255, 47, 225))

    separator = dilated(face, 6)
    alpha_over(canvas, separator, CHARCOAL)

    lime_face = face_gradient(face.size, bounds)
    lime_face.putalpha(face)
    canvas.alpha_composite(lime_face)

    paint = paint_mask_for_face(face, character, bounds)
    pink_face = Image.new("RGBA", face.size, MAGENTA)
    pink_face.putalpha(paint)
    canvas.alpha_composite(pink_face)

    # Preserve a narrow top-left light catcher after the paint pass.
    edge = ImageChops.subtract(face, shifted(face, 2, 3))
    alpha_over(canvas, edge, (226, 255, 117, 155))
    add_face_texture(canvas, face, character, bounds)

    return canvas, glyph_advance(character), glyph_baseline(character)


def save_pack(output_dir: Path, source_sheet_path: Path) -> tuple[Path, Path, dict]:
    missing = set(PRINTABLE_GLYPHS) - set(SOURCE_LAYOUT) - set(BRUSH_GLYPHS)
    if missing:
        raise ValueError(f"Missing image glyphs and symbol fallbacks: {''.join(sorted(missing))}")
    source_bytes = source_sheet_path.read_bytes()
    source_sheet = Image.open(source_sheet_path).convert("RGB")
    if source_sheet.width < 1_000 or source_sheet.height < 500:
        raise ValueError(
            f"Undead Legion source sheet is too small for clean extraction: {source_sheet.size}"
        )
    output_dir.mkdir(parents=True, exist_ok=True)
    glyph_dir = output_dir / "glyphs"
    glyph_dir.mkdir(parents=True, exist_ok=True)

    rows = math.ceil(len(PRINTABLE_GLYPHS) / ATLAS_COLUMNS)
    atlas = Image.new("RGBA", (ATLAS_COLUMNS * CELL_WIDTH, rows * CELL_HEIGHT))
    glyphs: dict[str, dict] = {}
    extracted_count = 0
    fallback_count = 0

    for index, character in enumerate(PRINTABLE_GLYPHS):
        if character in SOURCE_LAYOUT:
            cell, advance, baseline = render_reference_glyph(
                source_sheet,
                character,
                SOURCE_LAYOUT[character],
            )
            source_kind = "primary-sheet-cleaned"
            extracted_count += 1
        else:
            cell, advance, baseline = render_glyph(character)
            source_kind = "authored-symbol-fallback"
            fallback_count += 1
        column = index % ATLAS_COLUMNS
        row = index // ATLAS_COLUMNS
        atlas_x = column * CELL_WIDTH
        atlas_y = row * CELL_HEIGHT
        atlas.alpha_composite(cell, (atlas_x, atlas_y))
        cell.save(glyph_dir / f"{ord(character):04X}.png", optimize=True)
        glyphs[character] = {
            "atlas": [atlas_x, atlas_y, CELL_WIDTH, CELL_HEIGHT],
            "advance": advance,
            "originX": CELL_ORIGIN_X,
            "baseline": baseline,
            "sourceKind": source_kind,
        }

    metadata = {
        "rendererVersion": RENDERER_VERSION,
        "name": "ClipGoblin Undead Legion",
        "sourceFont": None,
        "sourceFontLicense": "User-approved project alphabet artwork; no external font skeleton",
        "construction": "cleaned transparent image glyphs from the approved complete alphabet sheet, with authored fallbacks only for absent symbols",
        "originalMaterialArtwork": True,
        "sourceSheetSha256": hashlib.sha256(source_bytes).hexdigest(),
        "coverage": {
            "physicalGlyphs": len(PRINTABLE_GLYPHS),
            "primarySheetCleaned": extracted_count,
            "authoredSymbolFallback": fallback_count,
        },
        "cleanup": {
            "matte": "local color-key extraction with connected-component cleanup",
            "edge": "locally rebuilt charcoal separation, black depth, and green rim/glow",
            "sourceStored": False,
        },
        "atlas": {
            "file": "atlas.png",
            "width": atlas.width,
            "height": atlas.height,
            "columns": ATLAS_COLUMNS,
            "rows": rows,
        },
        "metrics": {
            "cellWidth": CELL_WIDTH,
            "cellHeight": CELL_HEIGHT,
            "nominalFontSize": NOMINAL_FONT_SIZE,
            "baseline": BASELINE,
            "lineHeight": LINE_HEIGHT,
            "spaceAdvance": 64,
            "letterSpacing": -10,
        },
        "normalization": {
            "uppercase": False,
            "smartQuotes": "ASCII quotes",
            "dashes": "hyphen",
            "ellipsis": "three periods",
            "unknown": "?",
        },
        "material": {
            "face": "lime paint with lower-face magenta paint blended inside the glyph",
            "edge": "thin charcoal separation with narrow magenta sidewall and green halo",
            "depth": "directional black paint depth down-left",
            "texture": "source-preserved jagged brush faces with pink paint inside the lower lime face",
        },
        "glyphs": glyphs,
    }

    atlas_path = output_dir / "atlas.png"
    metadata_path = output_dir / "metadata.json"
    atlas.save(atlas_path, optimize=True)
    metadata_path.write_text(json.dumps(metadata, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")
    return atlas_path, metadata_path, metadata


def normalize_text(text: str) -> str:
    replacements = {
        "\u2018": "'", "\u2019": "'", "\u201c": '\"', "\u201d": '\"',
        "\u2013": "-", "\u2014": "-", "\u2026": "...",
    }
    for source, target in replacements.items():
        text = text.replace(source, target)
    return text


def assemble_text(
    text: str,
    atlas: Image.Image,
    metadata: dict,
    target_width: int,
    font_size: int,
) -> Image.Image:
    """Reference compositor for deterministic generator fixtures."""
    scale = font_size / metadata["metrics"]["nominalFontSize"]
    glyphs = metadata["glyphs"]
    lines = normalize_text(text).splitlines() or [""]
    line_height = round(metadata["metrics"]["lineHeight"] * scale)
    measured: list[int] = []
    for line in lines:
        width = 0.0
        for character in line:
            if character == " ":
                width += metadata["metrics"]["spaceAdvance"] * scale
                continue
            entry = glyphs.get(character, glyphs["?"])
            if "alias" in entry:
                entry = glyphs[entry["alias"]]
            width += (entry["advance"] + metadata["metrics"]["letterSpacing"]) * scale
        measured.append(max(1, round(width)))

    canvas = Image.new("RGBA", (target_width, max(line_height, line_height * len(lines))))
    for line_index, line in enumerate(lines):
        x = (target_width - measured[line_index]) / 2
        line_baseline = round((line_index + 1) * line_height - (LINE_HEIGHT - BASELINE) * scale)
        for character in line:
            if character == " ":
                x += metadata["metrics"]["spaceAdvance"] * scale
                continue
            entry = glyphs.get(character, glyphs["?"])
            if "alias" in entry:
                entry = glyphs[entry["alias"]]
            atlas_x, atlas_y, width, height = entry["atlas"]
            cell = atlas.crop((atlas_x, atlas_y, atlas_x + width, atlas_y + height))
            cell = cell.resize(
                (max(1, round(width * scale)), max(1, round(height * scale))),
                Image.Resampling.LANCZOS,
            )
            paste_x = round(x - entry["originX"] * scale)
            paste_y = round(line_baseline - entry["baseline"] * scale)
            canvas.alpha_composite(cell, (paste_x, paste_y))
            x += (entry["advance"] + metadata["metrics"]["letterSpacing"]) * scale
    return canvas


def checkerboard(size: tuple[int, int], tile: int = 24) -> Image.Image:
    image = Image.new("RGB", size, (28, 28, 32))
    draw = ImageDraw.Draw(image)
    for y in range(0, size[1], tile):
        for x in range(0, size[0], tile):
            fill = (41, 41, 46) if (x // tile + y // tile) % 2 else (24, 24, 29)
            draw.rectangle((x, y, x + tile - 1, y + tile - 1), fill=fill)
    return image.convert("RGBA")


def make_shippable_review(atlas: Image.Image, metadata: dict, output_path: Path) -> None:
    board = Image.new("RGB", (1920, 1080), (10, 10, 13))
    draw = ImageDraw.Draw(board)
    title_font = ImageFont.truetype(str(LABEL_FONT_PATH), 52)
    label_font = ImageFont.truetype(str(LABEL_FONT_PATH), 30)
    draw.text((55, 40), "UNDEAD LEGION - CLEANED IMAGE-GLYPH PACK", font=title_font, fill=(235, 235, 241))
    draw.text((58, 105), "SOURCE SILHOUETTES + LIME/PINK FACE + BLACK DEPTH + GREEN GLOW", font=label_font, fill=(172, 255, 55))

    samples = [
        ("UNDEAD LEGION", 118),
        ("LEAVE ME ALONE!", 100),
        ("Aa Bb Gg Mm Rr Zz 0123456789", 75),
        ("WAIT... WHAT?! #2026", 78),
    ]
    y = 180
    backgrounds = [(17, 23, 28), (222, 225, 219), (38, 20, 45), (12, 35, 25)]
    for index, (text, size) in enumerate(samples):
        panel = checkerboard((1810, 188), 26) if index == 0 else Image.new("RGBA", (1810, 188), backgrounds[index])
        rendered = assemble_text(text, atlas, metadata, 1740, size)
        panel.alpha_composite(rendered, (35, max(0, (188 - rendered.height) // 2)))
        board.paste(panel.convert("RGB"), (55, y))
        y += 212

    output_path.parent.mkdir(parents=True, exist_ok=True)
    board.save(output_path, optimize=True)


def make_glyph_contact_sheet(atlas: Image.Image, metadata: dict, output_path: Path) -> None:
    columns = 13
    cell_width = 140
    cell_height = 164
    rows = math.ceil(len(PRINTABLE_GLYPHS) / columns)
    board = Image.new("RGB", (1920, 180 + rows * cell_height), (10, 10, 13))
    draw = ImageDraw.Draw(board)
    title_font = ImageFont.truetype(str(LABEL_FONT_PATH), 44)
    label_font = ImageFont.truetype(str(LABEL_FONT_PATH), 20)
    note_font = ImageFont.truetype(str(LABEL_FONT_PATH), 16)
    draw.text((45, 30), "UNDEAD LEGION - PER-GLYPH CLEANUP / COVERAGE", font=title_font, fill=(235, 235, 241))
    draw.text((48, 88), "GREEN = APPROVED SHEET CROP   PINK = AUTHORED FALLBACK FOR A MISSING SYMBOL", font=label_font, fill=(172, 255, 55))
    draw.text((48, 126), "Every cell below is a separate transparent glyph asset.", font=note_font, fill=(188, 188, 198))

    glyphs = metadata["glyphs"]
    for index, character in enumerate(PRINTABLE_GLYPHS):
        column = index % columns
        row = index // columns
        left = 45 + column * cell_width
        top = 170 + row * cell_height
        panel = checkerboard((cell_width - 8, cell_height - 8), 12)
        entry = glyphs[character]
        atlas_x, atlas_y, width, height = entry["atlas"]
        glyph = atlas.crop((atlas_x, atlas_y, atlas_x + width, atlas_y + height))
        bounds = glyph.getbbox()
        if bounds:
            glyph = glyph.crop(bounds)
            glyph.thumbnail((cell_width - 24, cell_height - 48), Image.Resampling.LANCZOS)
            panel.alpha_composite(
                glyph,
                ((panel.width - glyph.width) // 2, 22 + (panel.height - 44 - glyph.height) // 2),
            )
        panel_draw = ImageDraw.Draw(panel)
        display = {" ": "SPACE", "\\": "BACKSLASH", "`": "GRAVE"}.get(character, character)
        source_kind = entry["sourceKind"]
        marker = (172, 255, 55) if source_kind == "primary-sheet-cleaned" else (255, 80, 205)
        panel_draw.rectangle((0, 0, panel.width - 1, panel.height - 1), outline=marker, width=2)
        panel_draw.text((7, 3), display, font=label_font, fill=(238, 238, 243))
        panel_draw.text(
            (7, panel.height - 19),
            "SHEET" if source_kind == "primary-sheet-cleaned" else "FALLBACK",
            font=note_font,
            fill=marker,
        )
        board.paste(panel.convert("RGB"), (left, top))

    output_path.parent.mkdir(parents=True, exist_ok=True)
    board.save(output_path, optimize=True)


def make_private_comparison(
    reference_path: Path,
    atlas: Image.Image,
    metadata: dict,
    output_path: Path,
) -> None:
    reference = Image.open(reference_path).convert("RGB")
    target = reference.copy()
    target.thumbnail((860, 790), Image.Resampling.LANCZOS)

    board = Image.new("RGB", (1920, 1080), (11, 11, 14))
    draw = ImageDraw.Draw(board)
    title_font = ImageFont.truetype(str(LABEL_FONT_PATH), 44)
    label_font = ImageFont.truetype(str(LABEL_FONT_PATH), 30)
    draw.text((55, 35), "PRIVATE REFERENCE COMPARISON - DO NOT SHIP", font=title_font, fill=(255, 105, 214))
    draw.text((55, 100), "TARGET CONSTRUCTION", font=label_font, fill=(225, 225, 230))
    draw.text((1010, 100), "CLEANED IMAGE-GLYPH PACK", font=label_font, fill=(225, 225, 230))

    target_panel = checkerboard((860, 820), 26)
    target_panel.paste(target, ((860 - target.width) // 2, (820 - target.height) // 2))
    board.paste(target_panel.convert("RGB"), (55, 155))

    proof_panel = checkerboard((855, 820), 26)
    proof_samples = [
        ("UNDEAD LEGION", 106),
        ("ABCDEFGH IJKLM", 62),
        ("NOPQRSTUVWXYZ", 62),
        ("abcdefghijklm", 58),
        ("nopqrstuvwxyz", 58),
        ("0123456789 !@#$%", 58),
    ]
    proof_y = 80
    for text, size in proof_samples:
        proof = assemble_text(text, atlas, metadata, 820, size)
        proof_panel.alpha_composite(proof, ((855 - proof.width) // 2, proof_y))
        proof_y += proof.height + 24
    board.paste(proof_panel.convert("RGB"), (1010, 155))
    output_path.parent.mkdir(parents=True, exist_ok=True)
    board.save(output_path, optimize=True)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, default=OUTPUT_DIR)
    parser.add_argument("--review", type=Path, default=DESIGN_DIR / "undead-legion-glyph-pack-review-1080.png")
    parser.add_argument("--contact-sheet", type=Path, default=DESIGN_DIR / "undead-legion-glyph-contact-sheet.png")
    parser.add_argument("--source-sheet", type=Path, required=True)
    parser.add_argument("--reference", type=Path)
    parser.add_argument("--private-comparison", type=Path)
    args = parser.parse_args()

    atlas_path, metadata_path, metadata = save_pack(args.output, args.source_sheet)
    atlas = Image.open(atlas_path).convert("RGBA")
    make_shippable_review(atlas, metadata, args.review)
    make_glyph_contact_sheet(atlas, metadata, args.contact_sheet)
    if args.reference and args.private_comparison:
        make_private_comparison(args.reference, atlas, metadata, args.private_comparison)

    print(f"atlas={atlas_path}")
    print(f"metadata={metadata_path}")
    print(f"review={args.review}")
    print(f"contact_sheet={args.contact_sheet}")
    if args.private_comparison:
        print(f"private_comparison={args.private_comparison}")


if __name__ == "__main__":
    main()
