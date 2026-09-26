#!/usr/bin/env python3
"""Cuts videos from 4K (3840x2160, scale 2) VM screen recordings.

    demo/cut.py <spec.json> <out-dir>

The spec maps an output name to its formats and segments:

    {"framings": {"win": {"16:9": [w, h, x, y], "1:1": [...], "9:16": [...]}},
     "videos": {"transitions": {"formats": ["16:9", "1:1"],
                                "segments": [["take.mp4", start, end, "win", speed], ...]}}}

Crops are physical pixels of the capture. Segments are joined with a short
crossfade where the framing changes and a near cut where it stays. Output is
H.264 yuv420p with faststart: 1920x1080, 1080x1080 or 1080x1920.
"""

import json
import subprocess
import sys
import tempfile
from pathlib import Path

SIZES = {"16:9": (1920, 1080), "1:1": (1080, 1080), "9:16": (1080, 1920)}
SUFFIX = {"16:9": "", "1:1": "-square", "9:16": "-vertical"}
FADE = 0.3


def duration(path):
    out = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0",
                          str(path)], capture_output=True, text=True).stdout
    return float(out)


def render(name, video, framings, fmt, root, out):
    w, h = SIZES[fmt]
    with tempfile.TemporaryDirectory() as tmp:
        parts = []
        for i, (src, start, end, framing, speed) in enumerate(video["segments"]):
            cw, ch, cx, cy = framings[framing][fmt]
            part = Path(tmp) / f"{i}.mp4"
            vf = (f"crop={cw}:{ch}:{cx}:{cy},scale={w}:{h}:flags=lanczos,"
                  f"setpts=PTS/{speed},fps=30,format=yuv420p")
            subprocess.run(["ffmpeg", "-v", "error", "-y", "-ss", str(start), "-to", str(end),
                            "-i", str(root / src), "-vf", vf, "-an", "-c:v", "libx264", "-crf", "14",
                            "-preset", "fast", str(part)], check=True)
            parts.append((part, duration(part), framing))
        if len(parts) == 1:
            chain, last = "[0:v]null[v0]", "[v0]"
        else:
            chain, last, offset = "", "[0:v]", parts[0][1]
            for i in range(1, len(parts)):
                fade = FADE if parts[i][2] != parts[i - 1][2] else 0.04
                offset -= fade
                chain += (f"{last}[{i}:v]xfade=transition=fade:duration={fade}:"
                          f"offset={offset:.3f}[v{i}];")
                last, offset = f"[v{i}]", offset + parts[i][1]
            chain = chain.rstrip(";")
        inputs = [a for p in parts for a in ("-i", str(p[0]))]
        target = out / f"{name}{SUFFIX[fmt]}.mp4"
        subprocess.run(["ffmpeg", "-v", "error", "-y", *inputs, "-filter_complex", chain, "-map", last,
                        "-c:v", "libx264", "-preset", "slow", "-crf", "23", "-pix_fmt", "yuv420p",
                        "-movflags", "+faststart", str(target)], check=True)
        print(f"{target.name} {duration(target):.1f}s {target.stat().st_size / 1e6:.1f} MB")


def main():
    spec_path, out = Path(sys.argv[1]), Path(sys.argv[2])
    spec = json.loads(spec_path.read_text())
    root = Path(spec.get("root", spec_path.parent))
    out.mkdir(parents=True, exist_ok=True)
    only = set(sys.argv[3:])
    for name, video in spec["videos"].items():
        if only and name not in only:
            continue
        for fmt in video["formats"]:
            render(name, video, spec["framings"], fmt, root, out)


if __name__ == "__main__":
    main()
