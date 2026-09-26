#!/usr/bin/env python3
"""Renders demo/import-script.txt into one mono MP3 with three voices.

    demo/render_import.py <voices-dir> <out.mp3>

Voices: en_US-amy-medium, en_US-hfc_male-medium and en_US-lessac-medium from
https://huggingface.co/rhasspy/piper-voices. Nothing is played.
"""

import random
import subprocess
import sys
import tempfile
from pathlib import Path

VOICES = {"A": "en_US-amy-medium.onnx", "R": "en_US-hfc_male-medium.onnx", "L": "en_US-lessac-medium.onnx"}
# Pitch factor per voice, 1.0 keeps a voice as piper makes it.
PITCH = {"A": 1.0, "R": 1.0, "L": 1.0}


def main() -> None:
    voices, out = Path(sys.argv[1]), Path(sys.argv[2])
    script = Path(__file__).with_name("import-script.txt").read_text().splitlines()
    turns = [line.split("|", 1) for line in script if line and not line.startswith("#")]
    random.seed(11)
    with tempfile.TemporaryDirectory() as tmp:
        parts = []
        for i, (who, text) in enumerate(turns):
            wav = Path(tmp) / f"{i}.wav"
            subprocess.run(["piper-tts", "--model", str(voices / VOICES[who]), "--output_file", str(wav)],
                           input=text.strip().encode(), check=True, capture_output=True)
            # Every voice at 22050 Hz mono, with a short pause after each turn.
            norm = Path(tmp) / f"{i}n.wav"
            pause = random.uniform(0.45, 0.85)
            subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(wav), "-af",
                            f"asetrate=22050*{PITCH[who]},aresample=22050,atempo={1 / PITCH[who]:.4f},"
                            f"apad=pad_dur={pause:.2f}", "-ac", "1", str(norm)], check=True)
            parts.append(norm)
        listing = Path(tmp) / "list.txt"
        listing.write_text("".join(f"file '{p}'\n" for p in parts))
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "concat", "-safe", "0", "-i", str(listing),
                        "-ac", "1", "-c:a", "libmp3lame", "-b:a", "96k", str(out)], check=True)
    print(f"{len(turns)} turns")


if __name__ == "__main__":
    main()
