#!/usr/bin/env python3
"""Render a quick visual QA sheet for ClipGoblin's material caption fonts."""

from __future__ import annotations

import argparse
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


ROOT = Path(__file__).resolve().parents[1]
FONT_DIR = ROOT / "public" / "fonts"


def fitted_font(path: Path, text: str, max_width: int, target_size: int) -> ImageFont.FreeTypeFont:
    size = target_size
    while size > 18:
        font = ImageFont.truetype(str(path), size)
        width = font.getbbox(text, stroke_width=4)[2]
        if width <= max_width:
            return font
        size -= 2
    return ImageFont.truetype(str(path), size)


def center_x(draw: ImageDraw.ImageDraw, text: str, font: ImageFont.FreeTypeFont, width: int) -> int:
    box = draw.textbbox((0, 0), text, font=font, stroke_width=4)
    return int((width - (box[2] - box[0])) / 2 - box[0])


def draw_text_layer(
    draw: ImageDraw.ImageDraw,
    position: tuple[int, int],
    text: str,
    font: ImageFont.FreeTypeFont,
    fill: str,
    stroke_fill: str | None = None,
    stroke_width: int = 0,
) -> None:
    draw.text(
        position,
        text,
        font=font,
        fill=fill,
        stroke_width=stroke_width,
        stroke_fill=stroke_fill,
    )


def draw_tape_faces(
    draw: ImageDraw.ImageDraw,
    position: tuple[int, int],
    text: str,
    font: ImageFont.FreeTypeFont,
) -> None:
    x, y = position
    colour_index = 0
    for glyph in text:
        colour = "#adff1f" if colour_index % 2 == 0 else "#7f31ef"
        draw.text((x, y), glyph, font=font, fill=colour, stroke_width=3, stroke_fill="#09090b")
        x += int(draw.textlength(glyph, font=font))
        if glyph.isalnum():
            colour_index += 1


def render(output: Path) -> None:
    width, row_height = 1680, 360
    image = Image.new("RGB", (width, row_height * 3), "#09090c")
    draw = ImageDraw.Draw(image)
    rows = [
        {
            "label": "TAPE RIOT",
            "text": "THAT WAS NOT THE PLAN",
            "base": "ClipGoblinTapeRiot-Regular.ttf",
            "details": [
                ("ClipGoblinTapeRiotSeams-Regular.ttf", "#271335"),
                ("ClipGoblinTapeRiotPatches-Regular.ttf", "#ffd326"),
            ],
            "depth": ("#14091d", "#422066", "#09070b"),
            "face": "tape",
        },
        {
            "label": "PAPER MISCHIEF",
            "text": "I THOUGHT THAT WOULD WORK",
            "base": "ClipGoblinPaperMischief-Regular.ttf",
            "details": [
                ("ClipGoblinPaperMischiefFiber-Regular.ttf", "#817b78"),
                ("ClipGoblinPaperMischiefTabs-Regular.ttf", "#aaff24"),
            ],
            "depth": ("#2a113b", "#7a39a2", "#6d686e"),
            "face": "#f1eee7",
        },
        {
            "label": "GOBLIN BITE",
            "text": "IT HEARD ME BREATHE",
            "base": "ClipGoblinGoblinBite-Regular.ttf",
            "details": [("ClipGoblinGoblinBiteDistress-Regular.ttf", "#4c6912")],
            "depth": ("#220c32", "#7a28b1", "#1b151e"),
            "face": "#dfff20",
        },
    ]

    for row_index, row in enumerate(rows):
        row_top = row_index * row_height
        text = row["text"]
        base_path = FONT_DIR / row["base"]
        font = fitted_font(base_path, text, width - 110, 178 if row_index != 2 else 205)
        x = center_x(draw, text, font, width)
        y = row_top + 90
        deep, middle, edge = row["depth"]
        draw_text_layer(draw, (x + 22, y + 24), text, font, deep, "#050507", 5)
        draw_text_layer(draw, (x + 14, y + 15), text, font, middle, "#12081d", 4)
        draw_text_layer(draw, (x + 6, y + 7), text, font, edge, "#050507", 4)
        if row["face"] == "tape":
            draw_tape_faces(draw, (x, y), text, font)
        else:
            draw_text_layer(draw, (x, y), text, font, row["face"], "#0c0a0f", 4)
        for detail_file, colour in row["details"]:
            detail_font = ImageFont.truetype(str(FONT_DIR / detail_file), font.size)
            draw_text_layer(draw, (x, y), text, detail_font, colour)
        draw.text((38, row_top + 25), row["label"], font=ImageFont.truetype(str(base_path), 34), fill="#c4b5fd")

    output.parent.mkdir(parents=True, exist_ok=True)
    image.save(output)
    print(output)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    render(args.output)


if __name__ == "__main__":
    main()
