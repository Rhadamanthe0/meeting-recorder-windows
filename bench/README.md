# Bench

A small test suite for the transcription and the speakers: it runs the app's own command line over a set of recordings and scores the result. Use it to check a change, or to compare another speech or speaker model against what the app does now.

```bash
cargo build --release
bench/run.py                          # the six cases in fixtures/, duration depends on CPU and model
bench/run.py --ami                    # plus the first 5 minutes of a real AMI meeting (downloads about 170 MB once)
bench/run.py --ami --ami-minutes 0    # the whole 17 minute meeting
bench/run.py --ami --check            # fail when a case scores below thresholds.json
bench/run.py --bin /usr/bin/omarchy-meeting-recorder --json old.json    # any other build
bench/run.py --case room music        # only some cases
```

Only Python's standard library is needed, plus ffmpeg for the AMI download. The speech and speaker models are the app's own, downloaded on first use as usual.

## The cases

| Case | What it tests |
|---|---|
| `call` | You on a headset, three people on the other side who interrupt each other and say "yeah" in between |
| `call-speakers` | The same call through speakers: the other side leaks into your mic, 40 ms late |
| `room` | Two people share your mic, two people on the other side |
| `music` | The other side talks with music playing in the background |
| `import` | The `call` as one mixed file, as if dropped on the app |
| `silence` | Twenty seconds of room noise: the transcript must be empty |
| `ami-ES2004a-import` | A real four-person meeting as one mixed file (with `--ami`, the first 5 minutes by default) |
| `ami-ES2004a-call` | The same meeting as a call: one person's headset is your mic, the other three are the computer audio |

Each case in `fixtures/` has its audio as Opus (like the app's own recordings) and a `truth.json` with every line: who said it, on which side, when, and the words.

## The columns

- **found**: words of the script that are in the transcript, near the right moment
- **side**: of those, on the right side of the call (you or the other side)
- **person**: of those, with the right person (labels are matched to people one to one)
- **wer**: word error rate of the transcript in time order against the script (1.0 when one side is empty)
- **cer**: character error rate, same
- **leaked**: lines of yours that are really the other side coming through your speakers
- **lines**: lines in the transcript, only for `silence`
- **speakers**: voices told apart, out of the voices in the case
- **speaker error**: share of speech that `diarize` gives to the wrong speaker (imports only, 1.0 when nothing is scored)
- **seconds**: how long the transcription took

For AMI there is no script, so side and person are measured by who was speaking during each line, weighted by its words.

## In CI

The automated checks run `bench/tests.py` in CI via `.github/workflows/windows.yml`. There is no `bench.yml`: the full audio bench above is manual and needs the built binary, ffmpeg, the models and processing time. It reads the committed fixtures and does not require a microphone or speakers. Run it locally before changing transcription, diarization or scoring, and keep `thresholds.json` honest with what you measured.

## Making new cases

The fixtures are generated from the scripts in `scripts/`, meetings of a small team working on Omarchy, with [piper](https://github.com/OHF-Voice/piper1-gpl) voices that are in the public domain or CC0 (Joe, John, Kristin, Norman and Cori from [piper-voices](https://huggingface.co/rhasspy/piper-voices)):

```bash
bench/generate.py ~/path/to/piper-voices
```

A script line is `speaker|gap|text`, where the gap is the seconds after the previous line ends; a negative gap makes them talk at the same time. The generated files are committed, so running the bench does not need piper.

Reference intervals describe speech, rather than the entire Piper WAV: the
generator excludes only leading/trailing near-silence, using 10 ms RMS blocks
60 dB below each clip's peak (never below one 16-bit PCM step), with 100 ms
padding. Internal pauses, short replies, audio samples and clip placement are
preserved. The padding also protects quiet phoneme endings. A silent or
unrepresentable synthesized line stops generation instead of becoming a speech
reference.

The shared `call`, `call-speakers` and `import` references were corrected on
8 October 2026, using their existing clean mixed `import/audio.ogg`, decoded
by FFmpeg to mono 16 kHz float PCM, and the same boundary rule within each
original interval. Overlapping speech is retained conservatively. Audio files,
words, speakers, scoring and thresholds were not changed. Independently,
Wav2Vec2 CTC confirmed the final words precede the near-silent tails; its output
was diagnostic only and was not used to generate the reference intervals.
The short `Ben: Yeah.` remains in the reference and contributes to errors when
missed. See [audit evidence](../docs/AUDIT.md) for the model revision and results.

## Licenses

The generated fixtures are CC0. AMI is © the AMI Consortium, [CC BY 4.0](https://groups.inf.ed.ac.uk/ami/corpus/license.shtml); its speaker annotations come from [pyannote/AMI-diarization-setup](https://github.com/pyannote/AMI-diarization-setup). Neither is stored in this repository: `--ami` downloads them to `bench/.cache/`.
