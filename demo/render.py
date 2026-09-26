#!/usr/bin/env python3
"""Renders demo/script.txt into two aligned tracks with piper.

    demo/render.py <voices-dir> <out-dir>

Writes <out-dir>/maya.wav (the microphone side) and <out-dir>/tom.wav (the
computer audio side), both 48 kHz stereo and the same length, so playing them
at the same moment gives a natural conversation. Nothing is played.

Voices: en_US-amy-medium (Maya) and en_US-ryan-medium (Tom) from
https://huggingface.co/rhasspy/piper-voices
"""

import array
import random
import subprocess
import sys
import tempfile
import wave
from pathlib import Path

RATE = 22050  # what the medium piper voices produce
VOICES = {"M": "en_US-amy-medium.onnx", "T": "en_US-ryan-medium.onnx"}


def synth(model: Path, text: str, out: Path) -> array.array:
    subprocess.run(
        ["piper-tts", "--model", str(model), "--output_file", str(out)],
        input=text.encode(),
        check=True,
        capture_output=True,
    )
    with wave.open(str(out)) as w:
        assert w.getframerate() == RATE and w.getnchannels() == 1
        samples = array.array("h")
        samples.frombytes(w.readframes(w.getnframes()))
    return samples


def main() -> None:
    voices, out = Path(sys.argv[1]), Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    script = Path(__file__).with_name("script.txt").read_text().splitlines()
    turns = [line.split("|", 1) for line in script if line and not line.startswith("#")]

    random.seed(7)
    tracks = {"M": array.array("h"), "T": array.array("h")}
    cursor = int(RATE * 0.8)
    previous = None
    with tempfile.TemporaryDirectory() as tmp:
        for i, (who, text) in enumerate(turns):
            speech = synth(voices / VOICES[who], text.strip(), Path(tmp) / f"{i}.wav")
            # A short breath between turns, a little longer when the speaker changes.
            gap = random.uniform(0.35, 0.6) if who == previous else random.uniform(0.45, 0.9)
            cursor += int(RATE * gap) if previous else 0
            for track in tracks.values():
                if len(track) < cursor + len(speech):
                    track.extend([0] * (cursor + len(speech) - len(track)))
            tracks[who][cursor : cursor + len(speech)] = speech
            cursor += len(speech)
            previous = who
        end = cursor + int(RATE * 1.5)
        for who, name in (("M", "maya"), ("T", "tom")):
            track = tracks[who]
            track.extend([0] * (end - len(track)))
            raw = Path(tmp) / f"{name}.raw"
            raw.write_bytes(track.tobytes())
            subprocess.run(
                ["ffmpeg", "-v", "error", "-y", "-f", "s16le", "-ar", str(RATE), "-ac", "1",
                 "-i", str(raw), "-ar", "48000", "-ac", "2", str(out / f"{name}.wav")],
                check=True,
            )
    print(f"{len(turns)} turns, {end / RATE:.1f} s")


if __name__ == "__main__":
    main()
