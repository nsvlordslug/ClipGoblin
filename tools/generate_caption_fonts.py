#!/usr/bin/env python3
"""Build ClipGoblin's material caption fonts from bundled OFL sources.

The base faces carry the custom silhouettes. Companion fonts share the exact
same metrics and contain only material details, so the editor and libass can
stack them without text drifting between layers.

Build dependency: fonttools and skia-pathops.
"""

from __future__ import annotations

import argparse
import hashlib
import math
from pathlib import Path
from typing import Callable

import pathops
from fontTools.pens.cu2quPen import Cu2QuPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont


ROOT = Path(__file__).resolve().parents[1]
FONT_DIR = ROOT / "public" / "fonts"


def stable_int(value: str) -> int:
    return int.from_bytes(hashlib.sha256(value.encode("utf-8")).digest()[:8], "big")


def polygon(path: pathops.Path, points: list[tuple[float, float]]) -> None:
    pen = path.getPen()
    pen.moveTo(points[0])
    for point in points[1:]:
        pen.lineTo(point)
    pen.closePath()


def rectangle(
    path: pathops.Path,
    left: float,
    bottom: float,
    right: float,
    top: float,
    skew: float = 0.0,
) -> None:
    polygon(
        path,
        [
            (left + skew, bottom),
            (right + skew, bottom),
            (right - skew, top),
            (left - skew, top),
        ],
    )


def ellipse(path: pathops.Path, cx: float, cy: float, rx: float, ry: float) -> None:
    # Four cubic arcs are accepted by Skia PathOps and converted back to TrueType
    # quadratics when the generated glyph is written.
    k = 0.5522847498307936
    pen = path.getPen()
    pen.moveTo((cx + rx, cy))
    pen.curveTo((cx + rx, cy + ry * k), (cx + rx * k, cy + ry), (cx, cy + ry))
    pen.curveTo((cx - rx * k, cy + ry), (cx - rx, cy + ry * k), (cx - rx, cy))
    pen.curveTo((cx - rx, cy - ry * k), (cx - rx * k, cy - ry), (cx, cy - ry))
    pen.curveTo((cx + rx * k, cy - ry), (cx + rx, cy - ry * k), (cx + rx, cy))
    pen.closePath()


def glyph_path(font: TTFont, glyph_name: str) -> pathops.Path:
    glyph_set = font.getGlyphSet()
    path = pathops.Path()
    glyph_set[glyph_name].draw(path.getPen(glyph_set))
    return path


def boolean_path(one: pathops.Path, two: pathops.Path, operation: pathops.PathOp) -> pathops.Path:
    if not tuple(two.verbs):
        return pathops.Path(one) if operation == pathops.PathOp.DIFFERENCE else pathops.Path()
    return pathops.op(
        one,
        two,
        operation,
        fix_winding=True,
        keep_starting_points=False,
    )


def path_to_glyph(path: pathops.Path):
    pen = TTGlyphPen(None)
    quadratic_pen = Cu2QuPen(pen, max_err=1.0, reverse_direction=True)
    path.draw(quadratic_pen)
    return pen.glyph()


def clear_hinting(font: TTFont) -> None:
    font["glyf"].removeHinting()
    for table in ("cvt ", "fpgm", "prep", "DSIG", "FFTM"):
        if table in font:
            del font[table]


def rename_font(font: TTFont, family: str, copyright_note: str) -> None:
    postscript_name = "".join(character for character in family if character.isalnum())
    names = {
        0: copyright_note,
        1: family,
        2: "Regular",
        3: f"ClipGoblin:{postscript_name}:2026",
        4: family,
        5: "Version 1.000",
        6: postscript_name,
        16: family,
        17: "Regular",
    }
    name_table = font["name"]
    for name_id, value in names.items():
        name_table.setName(value, name_id, 3, 1, 0x409)
        name_table.setName(value, name_id, 1, 0, 0)
    font["head"].fontRevision = 1.0
    font.recalcTimestamp = False


def glyph_character_map(font: TTFont) -> dict[str, str]:
    result: dict[str, str] = {}
    for codepoint, glyph_name in sorted((font.getBestCmap() or {}).items()):
        result.setdefault(glyph_name, chr(codepoint))
    return result


def tape_cutouts(bounds: tuple[float, float, float, float], seed: int) -> pathops.Path:
    left, bottom, right, top = bounds
    width = right - left
    height = top - bottom
    cuts = pathops.Path()
    tooth = max(7.0, min(width, height) * 0.025)

    # Torn tape ends: shallow irregular bites on the top and bottom edges.
    count = max(2, min(6, int(width / 145)))
    for index in range(count):
        x = left + width * (index + 0.55) / count
        jitter = ((seed >> (index * 4)) & 0xF) / 15.0 - 0.5
        x += jitter * tooth * 2.2
        depth = tooth * (0.7 + ((seed >> (index * 5 + 3)) & 0x7) / 10.0)
        if (seed + index) % 2:
            polygon(cuts, [(x - tooth, bottom - 2), (x, bottom + depth), (x + tooth, bottom - 2)])
        else:
            polygon(cuts, [(x - tooth, top + 2), (x, top - depth), (x + tooth, top + 2)])

    # Occasional clipped corner makes every glyph feel assembled, not typeset.
    corner = max(10.0, min(width, height) * 0.045)
    if seed % 2:
        polygon(cuts, [(right - corner, top + 2), (right + 2, top + 2), (right + 2, top - corner)])
    else:
        polygon(cuts, [(left - 2, bottom - 2), (left + corner, bottom - 2), (left - 2, bottom + corner)])
    return cuts


def paper_cutouts(bounds: tuple[float, float, float, float], seed: int) -> pathops.Path:
    left, bottom, right, top = bounds
    width = right - left
    height = top - bottom
    cuts = pathops.Path()
    radius = max(20.0, min(36.0, min(width, height) * 0.055))
    count = max(8, min(20, int((width + height) / 100)))

    # Many tiny, shallow scallops give the silhouette a torn-paper edge while
    # leaving the large playful source shapes intact and readable.
    for index in range(count):
        fraction = (index + 0.35 + ((seed >> (index % 24)) & 0x3) * 0.08) / count
        edge = (seed + index) % 4
        r = radius * (0.65 + ((seed >> ((index * 3) % 40)) & 0x7) / 10.0)
        if edge == 0:
            ellipse(cuts, left + width * fraction, top + r * 0.30, r, r * 0.72)
        elif edge == 1:
            ellipse(cuts, right + r * 0.30, bottom + height * fraction, r * 0.72, r)
        elif edge == 2:
            ellipse(cuts, left + width * fraction, bottom - r * 0.30, r, r * 0.72)
        else:
            ellipse(cuts, left - r * 0.30, bottom + height * fraction, r * 0.72, r)
    return cuts


def goblin_cutouts(bounds: tuple[float, float, float, float], seed: int) -> pathops.Path:
    left, bottom, right, top = bounds
    width = right - left
    height = top - bottom
    cuts = pathops.Path()
    radius = max(48.0, min(108.0, width * 0.26))
    bite_count = 2 + seed % 2

    # Deliberate clusters of three overlapping circles read as teeth marks,
    # unlike a generic distressed font's random missing pixels.
    for bite in range(bite_count):
        side_right = (seed + bite) % 3 != 0
        y = bottom + height * (0.23 + 0.52 * ((bite + 1) / (bite_count + 1)))
        y += (((seed >> (bite * 5)) & 0x1F) / 31.0 - 0.5) * height * 0.12
        edge_x = right - radius * 0.08 if side_right else left + radius * 0.08
        for tooth_index, y_offset in enumerate((-0.72, 0.0, 0.72)):
            tooth_radius = radius * (0.58 if tooth_index == 1 else 0.48)
            ellipse(cuts, edge_x, y + y_offset * radius, tooth_radius, tooth_radius)

    # A chipped corner reinforces the hand-cut horror-poster silhouette.
    chip = max(18.0, radius * 0.55)
    polygon(cuts, [(right - chip, top + 2), (right + 2, top + 2), (right + 2, top - chip * 0.62)])
    polygon(cuts, [(left - 2, bottom - 2), (left + chip * 0.8, bottom - 2), (left - 2, bottom + chip * 0.55)])
    return cuts


CutoutBuilder = Callable[[tuple[float, float, float, float], int], pathops.Path]


def build_base_font(
    source: Path,
    output: Path,
    family: str,
    copyright_note: str,
    cutout_builder: CutoutBuilder,
) -> None:
    font = TTFont(source, recalcTimestamp=False)
    clear_hinting(font)
    characters = glyph_character_map(font)
    glyph_table = font["glyf"]

    for glyph_name in font.getGlyphOrder():
        character = characters.get(glyph_name, glyph_name)
        base = glyph_path(font, glyph_name)
        if not tuple(base.verbs) or base.bounds is None:
            continue
        seed = stable_int(f"{family}:{character}")
        cuts = cutout_builder(base.bounds, seed)
        shaped = boolean_path(base, cuts, pathops.PathOp.DIFFERENCE)
        glyph_table[glyph_name] = path_to_glyph(shaped)

    rename_font(font, family, copyright_note)
    output.parent.mkdir(parents=True, exist_ok=True)
    font.save(output, reorderTables=True)


DetailBuilder = Callable[[tuple[float, float, float, float], int], pathops.Path]


def build_detail_font(
    base_font_path: Path,
    output: Path,
    family: str,
    copyright_note: str,
    detail_builder: DetailBuilder,
    *,
    clip_to_base: bool = True,
) -> None:
    font = TTFont(base_font_path, recalcTimestamp=False)
    characters = glyph_character_map(font)
    glyph_table = font["glyf"]

    for glyph_name in font.getGlyphOrder():
        character = characters.get(glyph_name, glyph_name)
        base = glyph_path(font, glyph_name)
        if not tuple(base.verbs) or base.bounds is None:
            glyph_table[glyph_name] = path_to_glyph(pathops.Path())
            continue
        seed = stable_int(f"{family}:{character}")
        detail = detail_builder(base.bounds, seed)
        material = (
            boolean_path(base, detail, pathops.PathOp.INTERSECTION)
            if clip_to_base
            else detail
        )
        glyph_table[glyph_name] = path_to_glyph(material)

    rename_font(font, family, copyright_note)
    font.save(output, reorderTables=True)


def tape_seams(bounds: tuple[float, float, float, float], seed: int) -> pathops.Path:
    left, bottom, right, top = bounds
    width = right - left
    height = top - bottom
    details = pathops.Path()
    seam = max(5.0, height * 0.012)
    for index, fraction in enumerate((0.28, 0.54, 0.78)):
        y = bottom + height * fraction
        skew = width * (0.05 if (seed + index) % 2 else -0.04)
        polygon(
            details,
            [
                (left - 5, y - seam),
                (right + 5, y - seam + skew),
                (right + 5, y + seam + skew),
                (left - 5, y + seam),
            ],
        )
    # Short diagonal joins suggest individual strips laid over each other.
    x = left + width * (0.36 + (seed % 17) / 70.0)
    polygon(details, [(x - seam, bottom), (x + seam, bottom), (x + width * 0.16, top), (x + width * 0.16 - seam * 2, top)])
    return details


def tape_patches(bounds: tuple[float, float, float, float], seed: int) -> pathops.Path:
    details = pathops.Path()
    if seed % 3:
        return details
    left, bottom, right, top = bounds
    width = right - left
    height = top - bottom
    patch_width = min(width * 0.34, height * 0.22)
    patch_height = min(height * 0.20, patch_width * 0.78)
    x = left + width * (0.18 + ((seed >> 8) & 0xFF) / 255.0 * 0.48)
    y = bottom + height * (0.18 + ((seed >> 16) & 0xFF) / 255.0 * 0.52)
    rectangle(details, x, y, x + patch_width, y + patch_height, skew=patch_width * 0.07)
    return details


def paper_fibers(bounds: tuple[float, float, float, float], seed: int) -> pathops.Path:
    left, bottom, right, top = bounds
    width = right - left
    height = top - bottom
    details = pathops.Path()
    count = max(12, min(32, int(width * height / 18000)))
    for index in range(count):
        local = stable_int(f"paper-fiber:{seed}:{index}")
        x = left + width * (((local >> 4) & 0xFFF) / 4095.0)
        y = bottom + height * (((local >> 16) & 0xFFF) / 4095.0)
        rx = max(2.2, width * (0.004 + ((local >> 28) & 0x7) / 1300.0))
        ry = max(1.6, height * (0.002 + ((local >> 32) & 0x7) / 1800.0))
        ellipse(details, x, y, rx, ry)
    return details


def paper_tabs(bounds: tuple[float, float, float, float], seed: int) -> pathops.Path:
    details = pathops.Path()
    if seed % 4 not in (0, 1):
        return details
    left, bottom, right, top = bounds
    width = right - left
    height = top - bottom
    tab_width = min(width * 0.32, height * 0.19)
    tab_height = tab_width * 0.56
    x = left + width * (0.10 if seed % 2 else 0.63)
    y = bottom + height * (0.69 if (seed >> 4) % 2 else 0.18)
    rectangle(details, x, y, x + tab_width, y + tab_height, skew=tab_width * 0.06)
    return details


def goblin_distress(bounds: tuple[float, float, float, float], seed: int) -> pathops.Path:
    left, bottom, right, top = bounds
    width = right - left
    height = top - bottom
    details = pathops.Path()
    count = max(10, min(28, int(width * height / 15000)))
    for index in range(count):
        local = stable_int(f"goblin-distress:{seed}:{index}")
        x = left + width * (0.08 + (((local >> 5) & 0xFFF) / 4095.0) * 0.84)
        y = bottom + height * (0.05 + (((local >> 17) & 0xFFF) / 4095.0) * 0.90)
        if index % 4 == 0:
            slash_width = max(4.0, width * 0.025)
            slash_height = max(16.0, height * 0.055)
            polygon(details, [(x, y), (x + slash_width, y), (x - slash_width, y + slash_height), (x - slash_width * 2, y + slash_height)])
        else:
            radius = max(2.5, min(width, height) * (0.008 + (local & 0x7) / 800.0))
            ellipse(details, x, y, radius, radius * 0.7)
    return details


def build_all() -> None:
    copyright_note = (
        "ClipGoblin material derivative font, 2026. Original font copyright retained; "
        "licensed under the SIL Open Font License 1.1."
    )
    specs = [
        (
            "RussoOne-Regular.ttf",
            "ClipGoblinTapeRiot-Regular.ttf",
            "ClipGoblin Tape Riot",
            tape_cutouts,
            [
                ("ClipGoblinTapeRiotSeams-Regular.ttf", "ClipGoblin Tape Riot Seams", tape_seams, True),
                ("ClipGoblinTapeRiotPatches-Regular.ttf", "ClipGoblin Tape Riot Patches", tape_patches, False),
            ],
        ),
        (
            "TitanOne-Regular.ttf",
            "ClipGoblinPaperMischief-Regular.ttf",
            "ClipGoblin Paper Mischief",
            paper_cutouts,
            [
                ("ClipGoblinPaperMischiefFiber-Regular.ttf", "ClipGoblin Paper Mischief Fiber", paper_fibers, True),
                ("ClipGoblinPaperMischiefTabs-Regular.ttf", "ClipGoblin Paper Mischief Tabs", paper_tabs, False),
            ],
        ),
        (
            "Anton-Regular.ttf",
            "ClipGoblinGoblinBite-Regular.ttf",
            "ClipGoblin Goblin Bite",
            goblin_cutouts,
            [
                ("ClipGoblinGoblinBiteDistress-Regular.ttf", "ClipGoblin Goblin Bite Distress", goblin_distress, True),
            ],
        ),
    ]

    for source_name, base_name, family, cutouts, details in specs:
        base_path = FONT_DIR / base_name
        build_base_font(FONT_DIR / source_name, base_path, family, copyright_note, cutouts)
        for detail_name, detail_family, detail_builder, clip_to_base in details:
            build_detail_font(
                base_path,
                FONT_DIR / detail_name,
                detail_family,
                copyright_note,
                detail_builder,
                clip_to_base=clip_to_base,
            )
        print(f"Built {family}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.parse_args()
    build_all()


if __name__ == "__main__":
    main()
