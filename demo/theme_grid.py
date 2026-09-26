#!/usr/bin/env python3
"""The same screen in several Omarchy themes, as one image.

    demo/theme_grid.py <out.png> <cols> <tile-w> "<caption>" <shot.png:x,y,w,h:Label> ...

x y w h are the window's logical geometry in a 3840x2160 (scale 2) shot, as
`hyprctl clients -j` reports it. Tiles get rounded corners, a soft shadow and
the theme's name underneath, on the dark field the other images use.
"""

import sys

from PIL import Image, ImageDraw, ImageFilter, ImageFont

BOLD = "/usr/share/fonts/liberation/LiberationSans-Bold.ttf"
REGULAR = "/usr/share/fonts/liberation/LiberationSans-Regular.ttf"
W, H = 1600, 900


def field():
    inner, outer = (0x1B, 0x1B, 0x27), (0x11, 0x11, 0x19)
    small = Image.new("RGB", (160, 90))
    px = small.load()
    for y in range(90):
        for x in range(160):
            d = min(1.0, (((x - 80) / 80) ** 2 + ((y - 45) / 45) ** 2) ** 0.5)
            px[x, y] = tuple(int(i + (o - i) * d) for i, o in zip(inner, outer))
    return small.resize((W, H), Image.BICUBIC).convert("RGBA")


def main():
    out, cols, tile_w, caption = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
    tiles = []
    for spec in sys.argv[5:]:
        path, geo, label = spec.split(":", 2)
        x, y, w, h = (int(v) * 2 for v in geo.split(","))
        im = Image.open(path).convert("RGB").crop((x, y, x + w, y + h))
        im = im.resize((tile_w, round(tile_w * h / w)), Image.LANCZOS)
        tiles.append((im, label))
    rows = (len(tiles) + cols - 1) // cols
    th = tiles[0][0].height
    gap, label_h = 26, 34
    grid_w = cols * tile_w + (cols - 1) * gap
    grid_h = rows * (th + label_h) + (rows - 1) * gap
    top = (H - 90 - grid_h) // 2 + 10
    left = (W - grid_w) // 2

    canvas = field()
    shadow = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    sd = ImageDraw.Draw(shadow)
    positions = []
    for i, (im, label) in enumerate(tiles):
        cx = left + (i % cols) * (tile_w + gap)
        cy = top + (i // cols) * (th + label_h + gap)
        positions.append((cx, cy))
        sd.rounded_rectangle((cx, cy + 8, cx + tile_w, cy + th + 8), 10, fill=(0, 0, 0, 150))
    canvas.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(16)))
    draw = ImageDraw.Draw(canvas)
    small = ImageFont.truetype(REGULAR, 20)
    for (im, label), (cx, cy) in zip(tiles, positions):
        mask = Image.new("L", im.size, 0)
        ImageDraw.Draw(mask).rounded_rectangle((0, 0, im.width - 1, im.height - 1), 10, fill=255)
        canvas.paste(im, (cx, cy), mask)
        lw = draw.textlength(label, font=small)
        draw.text((cx + (tile_w - lw) / 2, cy + th + 8), label, font=small, fill="#8B8B9E")
    title = ImageFont.truetype(BOLD, 40)
    tw = draw.textlength(caption, font=title)
    draw.text(((W - tw) / 2, H - 78), caption, font=title, fill="white")
    canvas.convert("RGB").save(out, optimize=True)


if __name__ == "__main__":
    main()
