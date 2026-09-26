#!/usr/bin/env python3
"""Composes a close-up for X: a crop of a 4K capture as a rounded card with a
soft shadow on a dark field, with a caption underneath.

    demo/details.py <capture.png> <x> <y> <w> <h> <out.png> "<caption>" ["<subtitle>"]
      [--size 1600x900] [--radius 24] [--scale 1.0]

Crop coordinates are physical pixels of the 3840x2160 (scale 2) capture. The
card is never scaled up past --scale, so it stays sharp.
"""

import argparse

from PIL import Image, ImageDraw, ImageFilter, ImageFont

BOLD = "/usr/share/fonts/liberation/LiberationSans-Bold.ttf"
REGULAR = "/usr/share/fonts/liberation/LiberationSans-Regular.ttf"


def field(w, h):
    """Radial #1B1B27 to #111119, like the other social images."""
    inner, outer = (0x1B, 0x1B, 0x27), (0x11, 0x11, 0x19)
    small = Image.new("RGB", (160, 90))
    px = small.load()
    for y in range(90):
        for x in range(160):
            d = min(1.0, (((x - 80) / 80) ** 2 + ((y - 45) / 45) ** 2) ** 0.5)
            px[x, y] = tuple(int(i + (o - i) * d) for i, o in zip(inner, outer))
    return small.resize((w, h), Image.BICUBIC)


def wordmark(height, color):
    """The Omarchy wordmark from /usr/share/omarchy/logo.svg in one colour."""
    import subprocess
    import tempfile
    with tempfile.NamedTemporaryFile(suffix=".png") as tmp:
        subprocess.run(["magick", "-background", "none", "/usr/share/omarchy/logo.svg", "-resize",
                        f"x{height}", "-fill", color, "-colorize", "100", tmp.name], check=True)
        return Image.open(tmp.name).convert("RGBA")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("capture")
    ap.add_argument("x", type=int)
    ap.add_argument("y", type=int)
    ap.add_argument("w", type=int)
    ap.add_argument("h", type=int)
    ap.add_argument("out")
    ap.add_argument("caption")
    ap.add_argument("subtitle", nargs="?", default="")
    ap.add_argument("--size", default="1600x900")
    ap.add_argument("--radius", type=int, default=24)
    ap.add_argument("--scale", type=float, default=1.0)
    ap.add_argument("--bg", help="an image to use as the field instead of the dark gradient")
    ap.add_argument("--logo", action="store_true", help="add the Omarchy wordmark bottom right")
    ap.add_argument("--caption-color", default="white")
    ap.add_argument("--subtitle-color", default="#8B8B9E")
    a = ap.parse_args()

    W, H = (int(v) for v in a.size.split("x"))
    card = Image.open(a.capture).convert("RGB").crop((a.x, a.y, a.x + a.w, a.y + a.h))
    text_h = 150 if a.caption else 0
    box_w, box_h = W - 160, H - 120 - text_h
    s = min(box_w / card.width, box_h / card.height, a.scale)
    card = card.resize((round(card.width * s), round(card.height * s)), Image.LANCZOS)
    r = round(a.radius * s)

    mask = Image.new("L", card.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, card.width - 1, card.height - 1), r, fill=255)

    if a.bg:
        canvas = Image.open(a.bg).convert("RGBA").resize((W, H), Image.LANCZOS)
    else:
        canvas = field(W, H).convert("RGBA")
    cx = (W - card.width) // 2
    cy = 60 + (box_h - card.height) // 2
    shadow = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle(
        (cx, cy + 14, cx + card.width, cy + card.height + 14), r, fill=(0, 0, 0, 165))
    canvas.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(28)))
    canvas.paste(card, (cx, cy), mask)

    draw = ImageDraw.Draw(canvas)
    if a.caption:
        title = ImageFont.truetype(BOLD, 44)
        tw = draw.textlength(a.caption, font=title)
        ty = cy + card.height + 48
        draw.text(((W - tw) / 2, ty), a.caption, font=title, fill=a.caption_color)
        if a.subtitle:
            sub = ImageFont.truetype(REGULAR, 25)
            sw = draw.textlength(a.subtitle, font=sub)
            draw.text(((W - sw) / 2, ty + 62), a.subtitle, font=sub, fill=a.subtitle_color)
    if a.logo:
        logo = wordmark(34, a.subtitle_color)
        canvas.alpha_composite(logo, (W - logo.width - 48, H - logo.height - 40))
    canvas.convert("RGB").save(a.out, optimize=True)


if __name__ == "__main__":
    main()
