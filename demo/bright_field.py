#!/usr/bin/env python3
"""A hot pink 1600x900 field with a faint, tilted lattice of microphones, for
social images (details.py --bg).

    demo/bright_field.py <out.png>
"""

import math
import random
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image

ICON = "https://raw.githubusercontent.com/tailwindlabs/heroicons/master/optimized/24/outline/microphone.svg"


def main():
    with tempfile.TemporaryDirectory() as tmp:
        svg, png = Path(tmp) / "mic.svg", Path(tmp) / "mic.png"
        subprocess.run(["curl", "-sSL", "-o", str(svg), ICON], check=True)
        svg.write_text(svg.read_text().replace("currentColor", "#ffffff"))
        subprocess.run(["magick", "-background", "none", "-density", "600", str(svg), "-resize", "64x64",
                        "-alpha", "set", "-channel", "A", "-evaluate", "multiply", "0.16", "+channel",
                        str(png)], check=True)
        icon = Image.open(png).convert("RGBA")
    W, H, CELL, TILT = 1600, 900, 150, math.radians(17)
    field = Image.new("RGBA", (W, H), "#FF2D78")
    random.seed(3)
    reach = int(math.hypot(W, H) / CELL / 2) + 2
    ct, st = math.cos(TILT), math.sin(TILT)
    for j in range(-reach, reach + 1):
        for i in range(-reach, reach + 1):
            lx, ly = i * CELL, j * CELL
            cx, cy = W / 2 + lx * ct - ly * st, H / 2 + lx * st + ly * ct
            if -CELL < cx < W + CELL and -CELL < cy < H + CELL:
                mark = icon.rotate(random.uniform(-6, 6), resample=Image.BICUBIC, expand=True)
                field.alpha_composite(mark, (int(cx - mark.width / 2), int(cy - mark.height / 2)))
    field.convert("RGB").save(sys.argv[1])


if __name__ == "__main__":
    main()
