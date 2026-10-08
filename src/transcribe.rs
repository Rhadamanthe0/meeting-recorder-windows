//! Transcription after the meeting, in-process with whisper.cpp (whisper-rs).
//!
//! Each recorded track is transcribed separately, then the sentences are
//! interleaved on the original timeline. Nemotron distinguishes voices on
//! each side; level, overlap and repeated-text checks suppress computer audio
//! leaking into the microphone. Imports use one transcription with speaker
//! turns from the same diarization model.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use gtk::glib;
use whisper_rs::{
    DtwMode, DtwParameters, FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
};

use crate::APP_NAME;
use crate::audio::{CHANNELS, RATE};

pub const WHISPER_RATE: usize = 16_000;

/// (code, label) in the order of the dropdown. "auto" lets whisper detect it.
pub const LANGUAGES: [(&str, &str); 8] = [
    ("auto", "Auto-detect"),
    ("en", "English"),
    ("nl", "Dutch"),
    ("de", "German"),
    ("fr", "French"),
    ("es", "Spanish"),
    ("it", "Italian"),
    ("pt", "Portuguese"),
];

/// What the transcription reports while it runs.
#[derive(Debug, Clone)]
pub enum Event {
    Stage(String),
    /// Overall progress over both tracks, 0.0 to 1.0.
    Progress(f64),
    /// A freshly transcribed line.
    Segment(String),
    /// Always the last event: reporting ends explicitly before the caller
    /// handles the final result.
    Finished,
}

pub type Events = async_channel::Sender<Event>;
/// Set to true to stop a running transcription.
pub type Abort = Arc<AtomicBool>;

pub const CANCELLED: &str = "transcription cancelled";

/// Only one heavy job (transcription, diarization) runs at a time: whisper
/// and the speaker model each take most of the CPU and memory.
static HEAVY_SLOT: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn acquire_heavy_slot(
    events: &Events,
    abort: &Abort,
) -> Result<std::sync::MutexGuard<'static, ()>, String> {
    emit(
        events,
        Event::Stage("Waiting for the transcription slot…".into()),
    );
    loop {
        if abort.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        match HEAVY_SLOT.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(std::sync::TryLockError::Poisoned(e)) => return Ok(e.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Segment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker: String,
    pub text: String,
}

pub struct Transcript {
    pub segments: Vec<Segment>,
    /// The language used or detected, as a whisper code.
    pub language: String,
    pub duration_secs: i64,
    /// Speaker separation failed, but the text is still usable; the UI must warn.
    pub diarization_failed: bool,
}

fn emit(events: &Events, event: Event) {
    let _ = events.send_blocking(event);
}

// ---------------------------------------------------------------------------
// Audio loading

/// Loads a track as 16 kHz mono f32. A `.raw` file is the app's own staging
/// format (s16le, 48 kHz, stereo); anything else is decoded by ffmpeg.
pub fn load_track(path: &Path) -> Result<Vec<f32>, String> {
    if path.extension().is_some_and(|e| e == "raw") {
        match decode_raw_with_ffmpeg(path) {
            Ok(track) => return Ok(track),
            Err(e) => eprintln!("{APP_NAME}: {e}; falling back to the built-in reader"),
        }
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let frame = 2 * CHANNELS as usize;
        let mono: Vec<f32> = bytes
            .chunks_exact(frame)
            .map(|f| {
                let sum: f32 = f
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|s| f32::from(i16::from_le_bytes(*s)))
                    .sum();
                sum / CHANNELS as f32 / 32768.0
            })
            .collect();
        Ok(downsample(&mono, RATE as usize / WHISPER_RATE))
    } else {
        decode_with_ffmpeg(path)
    }
}

fn decode_raw_with_ffmpeg(path: &Path) -> Result<Vec<f32>, String> {
    let output = crate::action_process::output(
        crate::platform::silent_command(crate::export::ffmpeg())
            .args([
                "-nostdin",
                "-loglevel",
                "error",
                "-f",
                "s16le",
                "-ar",
                &RATE.to_string(),
                "-ac",
                &CHANNELS.to_string(),
                "-i",
            ])
            .arg(path)
            .args([
                "-f",
                "f32le",
                "-ac",
                "1",
                "-ar",
                &WHISPER_RATE.to_string(),
                "-",
            ]),
    )
    .map_err(|e| format!("could not run ffmpeg: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "ffmpeg could not decode {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect())
}

fn decode_with_ffmpeg(path: &Path) -> Result<Vec<f32>, String> {
    let output = crate::action_process::output(
        crate::platform::silent_command(crate::export::ffmpeg())
            .args(["-nostdin", "-loglevel", "error", "-i"])
            .arg(path)
            .args([
                "-f",
                "f32le",
                "-ac",
                "1",
                "-ar",
                &WHISPER_RATE.to_string(),
                "-",
            ]),
    )
    .map_err(|e| format!("could not run ffmpeg: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "ffmpeg could not decode {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect())
}

/// Decimates by an integer `factor` behind a windowed-sinc low-pass filter.
fn downsample(input: &[f32], factor: usize) -> Vec<f32> {
    if factor <= 1 {
        return input.to_vec();
    }
    const TAPS: usize = 63;
    // Cut off a little below the new Nyquist frequency.
    let cutoff = 0.9 / (2.0 * factor as f64);
    let mid = (TAPS / 2) as f64;
    let mut kernel: Vec<f32> = (0..TAPS)
        .map(|i| {
            let x = i as f64 - mid;
            let sinc = if x == 0.0 {
                2.0 * cutoff
            } else {
                (2.0 * std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x)
            };
            let t = 2.0 * std::f64::consts::PI * i as f64 / (TAPS - 1) as f64;
            let blackman = 0.42 - 0.5 * t.cos() + 0.08 * (2.0 * t).cos();
            (sinc * blackman) as f32
        })
        .collect();
    let sum: f32 = kernel.iter().sum();
    kernel.iter_mut().for_each(|k| *k /= sum);

    let half = TAPS / 2;
    (0..input.len() / factor)
        .map(|n| {
            let center = n * factor;
            kernel
                .iter()
                .enumerate()
                .map(|(k, weight)| {
                    let i = center as isize + k as isize - half as isize;
                    if i < 0 || i as usize >= input.len() {
                        0.0
                    } else {
                        input[i as usize] * weight
                    }
                })
                .sum()
        })
        .collect()
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

fn ms_to_sample(ms: i64) -> usize {
    ms.max(0) as usize * WHISPER_RATE / 1000
}

fn sample_to_ms(sample: usize) -> i64 {
    (sample * 1000 / WHISPER_RATE) as i64
}

/// Below about -50 dBFS there is no speech to find, only hallucinations.
fn is_silent(samples: &[f32]) -> bool {
    samples.iter().fold(0.0f32, |m, s| m.max(s.abs())) < 0.003
}

/// How loud a track is while something is said: the 95th percentile of its
/// 30 ms frame levels, so pauses do not drag it down.
fn active_level(samples: &[f32]) -> f32 {
    let mut levels: Vec<f32> = samples.chunks(WHISPER_RATE * 30 / 1000).map(rms).collect();
    if levels.is_empty() {
        return 0.0;
    }
    levels.sort_by(f32::total_cmp);
    levels[levels.len() * 95 / 100]
}

/// Leaves everything below 0.8 alone and bends what is above it towards 1.0.
fn soft_clip(x: f32) -> f32 {
    const KNEE: f32 = 0.8;
    if x.abs() <= KNEE {
        x
    } else {
        x.signum() * (KNEE + (1.0 - KNEE) * ((x.abs() - KNEE) / (1.0 - KNEE)).tanh())
    }
}

/// Mixes the two tracks for whisper. Each is brought to a similar speaking
/// level first, so a quiet mic is not drowned by loud computer audio; a track
/// that is only noise is left as it is rather than boosted.
fn mix(mic: &[f32], computer: &[f32]) -> Vec<f32> {
    const TARGET: f32 = 0.1;
    let gain = |track: &[f32]| {
        let level = active_level(track);
        if is_silent(track) || level < 0.003 {
            1.0
        } else {
            (TARGET / level).clamp(0.25, 8.0)
        }
    };
    let (mic_gain, computer_gain) = (gain(mic), gain(computer));
    (0..mic.len().max(computer.len()))
        .map(|i| {
            let m = mic.get(i).copied().unwrap_or(0.0) * mic_gain;
            let c = computer.get(i).copied().unwrap_or(0.0) * computer_gain;
            soft_clip(m + c)
        })
        .collect()
}

/// Who speaks when: one side of a recording (its own track, so its own
/// speaker, split further when several voices share it), or the voices found
/// in a single imported file.
#[derive(Clone)]
enum Speakers {
    /// Everything on this track is `label`; with turns, `label 1`, `label 2`, ...
    Side(&'static str, Vec<crate::diarize::Turn>),
    Turns(Vec<crate::diarize::Turn>),
}

impl Speakers {
    fn speaker(&self, start_ms: i64, end_ms: i64) -> String {
        match self {
            Speakers::Side(label, turns) if turns.is_empty() => (*label).to_owned(),
            Speakers::Side(label, turns) => format!(
                "{label} {}",
                crate::diarize::speaker_at(turns, start_ms, end_ms) + 1
            ),
            Speakers::Turns(turns) => {
                format!(
                    "Speaker {}",
                    crate::diarize::speaker_at(turns, start_ms, end_ms) + 1
                )
            }
        }
    }

    /// Where `speaker` starts talking near `around_ms`, if that can be told
    /// more precisely than whisper's word times.
    fn takeover_ms(&self, speaker: &str, around_ms: i64) -> Option<i64> {
        let (turns, prefix) = match self {
            Speakers::Side(label, turns) => (turns, format!("{label} ")),
            Speakers::Turns(turns) => (turns, "Speaker ".to_owned()),
        };
        let index = speaker.strip_prefix(&prefix)?.parse::<usize>().ok()?;
        crate::diarize::turn_start_near(turns, index.checked_sub(1)?, around_ms)
    }

    /// With diarization a whisper segment can hold two voices without a
    /// sentence end between them, so a line is also cut at a pause where the
    /// speaker changes. Never inside a run of words: a sentence stays whole.
    fn cuts_at_pauses(&self) -> bool {
        match self {
            Speakers::Side(_, turns) => !turns.is_empty(),
            Speakers::Turns(_) => true,
        }
    }
}

/// A stretch with sound in it, in samples of the mix.
#[derive(Debug, Clone, Copy)]
struct Region {
    /// Where the region starts, including some padding before the sound.
    start: usize,
    /// Where the sound itself starts.
    onset: usize,
    end: usize,
}

const FRAME: usize = WHISPER_RATE * 30 / 1000;

/// Frames of `track` with sound in them: above four times its own noise floor.
fn active_frames(track: &[f32], frames: usize) -> Vec<bool> {
    let energies: Vec<f32> = track.chunks(FRAME).map(rms).collect();
    let mut active = vec![false; frames];
    if energies.is_empty() {
        return active;
    }
    let mut sorted = energies.clone();
    sorted.sort_by(f32::total_cmp);
    let floor = sorted[sorted.len() / 10];
    let threshold = (floor * 4.0).max(0.002);
    for (i, energy) in energies.iter().enumerate().take(frames) {
        if *energy >= threshold {
            active[i] = true;
        }
    }
    active
}

/// Finds the parts with sound in them, by frame energy against the noise floor.
/// Generous padding keeps word edges intact.
fn speech_regions(tracks: &[&[f32]], len: usize) -> Vec<Region> {
    let frames = len.div_ceil(FRAME);
    // Each track gets its own threshold: steady sound on one side (music, a
    // fan, a noisy line) must not hide the speech on the other side.
    let mut active = vec![false; frames];
    for track in tracks {
        for (a, on) in active.iter_mut().zip(active_frames(track, frames)) {
            *a |= on;
        }
    }
    regions_from(&active, len)
}

/// The parts of the (levelled) mic with your own voice in them. Through
/// speakers the other side leaks into the mic, delayed a little and always
/// quieter than on its own track. A mic frame only counts when it is at least
/// half as loud as the loudest computer audio around it (echo trails behind),
/// and only in runs of a few frames, so the gaps between their words do not
/// let the echo through either.
fn own_speech_regions(mic: &[f32], computer: &[f32]) -> Vec<Region> {
    const AROUND: usize = 3;
    const RUN: usize = 3;
    let frames = mic.len().div_ceil(FRAME);
    let level = |t: &[f32]| -> Vec<f32> {
        (0..frames)
            .map(|i| rms(&t[(i * FRAME).min(t.len())..((i + 1) * FRAME).min(t.len())]))
            .collect()
    };
    let (own, other) = (level(mic), level(computer));
    let mut active = active_frames(mic, frames);
    for (i, a) in active.iter_mut().enumerate() {
        let loudest = other[i.saturating_sub(AROUND)..(i + AROUND + 1).min(frames)]
            .iter()
            .fold(0.0f32, |m, v| m.max(*v));
        if own[i] * 2.0 < loudest {
            *a = false;
        }
    }
    // Drop runs shorter than RUN frames.
    let mut i = 0;
    while i < frames {
        if !active[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < frames && active[i] {
            i += 1;
        }
        if i - start < RUN {
            active[start..i].iter_mut().for_each(|a| *a = false);
        }
    }
    regions_from(&active, mic.len())
}

fn regions_from(active: &[bool], len: usize) -> Vec<Region> {
    const PAD: usize = WHISPER_RATE * 300 / 1000;
    const MERGE_GAP: usize = WHISPER_RATE * 800 / 1000;
    let mut regions: Vec<Region> = Vec::new();
    for (i, _) in active.iter().enumerate().filter(|(_, a)| **a) {
        let onset = i * FRAME;
        let start = onset.saturating_sub(PAD);
        let end = ((i + 1) * FRAME + PAD).min(len);
        match regions.last_mut() {
            Some(last) if start <= last.end + MERGE_GAP => last.end = last.end.max(end),
            _ => regions.push(Region { start, onset, end }),
        }
    }
    regions
}

/// The regions glued together with a pause in between, and where each one
/// starts in the glued buffer. Whisper invents text in long silences and
/// places timestamps badly after them, so it only gets the parts with sound;
/// the map puts every timestamp back on the real timeline.
struct Glued {
    samples: Vec<f32>,
    /// (offset in `samples`, region)
    map: Vec<(usize, Region)>,
}

impl Glued {
    fn new(mix: &[f32], regions: &[Region]) -> Self {
        // Long enough for whisper to start a new segment at every join.
        const GAP: usize = WHISPER_RATE * 700 / 1000;
        let mut samples = Vec::new();
        let mut map = Vec::new();
        for region in regions {
            if !samples.is_empty() {
                samples.extend(std::iter::repeat_n(0.0, GAP));
            }
            map.push((samples.len(), *region));
            samples.extend_from_slice(&mix[region.start..region.end]);
        }
        Glued { samples, map }
    }

    /// A time in the glued buffer as (ms on the real timeline, region index).
    fn locate(&self, glued_ms: i64) -> (i64, usize) {
        locate(&self.map, glued_ms)
    }
}

fn locate(map: &[(usize, Region)], glued_ms: i64) -> (i64, usize) {
    let at = ms_to_sample(glued_ms);
    let index = map
        .iter()
        .rposition(|(offset, _)| *offset <= at)
        .unwrap_or(0);
    let Some((offset, region)) = map.get(index).copied() else {
        return (glued_ms, 0);
    };
    let sample = region.start + (at - offset.min(at)).min(region.end - region.start);
    (sample_to_ms(sample), index)
}

// ---------------------------------------------------------------------------
// Model

/// Where downloaded models live: the app's `models` folder under the data dir.
pub fn models_dir() -> PathBuf {
    crate::platform::models_dir()
}

/// Downloads `url` to `target` through a `.part` file, reporting progress as
/// "`label` 42%". Refuses a result smaller than `min_bytes`.
pub fn download(
    url: &str,
    target: &Path,
    label: &str,
    min_bytes: u64,
    events: &Events,
    abort: &Abort,
) -> Result<(), String> {
    let valid = |p: &Path| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() >= min_bytes);
    if valid(target) {
        return Ok(());
    }
    let dir = target.parent().expect("model path has a parent");
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // Unique per attempt so two downloaders never share a temp file: process
    // id + nanos + a per-process counter, created exclusively.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".to_owned());
    let part = dir.join(format!(
        ".{name}.part-{}-{}-{}",
        std::process::id(),
        nanos,
        seq
    ));
    emit(events, Event::Stage(label.to_owned()));

    if abort.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    let response = ureq::get(url)
        .config()
        .timeout_connect(Some(std::time::Duration::from_secs(30)))
        .timeout_recv_response(Some(std::time::Duration::from_secs(30)))
        // Models can exceed a gigabyte; bound stalled transfers while
        // allowing a slow connection to finish a normal download.
        .timeout_global(Some(std::time::Duration::from_secs(2 * 3600)))
        .build()
        .call()
        .map_err(|e| format!("could not download {url}: {e}"))?;
    let total = response.body().content_length();
    let mut reader = response.into_body().into_reader();
    let file = File::options()
        .write(true)
        .create_new(true)
        .open(&part)
        .map_err(|e| e.to_string())?;
    // Cleanup also covers read/write/flush errors, after the file handle has
    // been closed (required on Windows).
    let result = (|| {
        let mut file = BufWriter::new(file);
        let mut buf = vec![0u8; 1 << 16];
        let (mut done, mut last_pct) = (0u64, u64::MAX);
        loop {
            if abort.load(Ordering::Relaxed) {
                return Err(CANCELLED.into());
            }
            let n = reader
                .read(&mut buf)
                .map_err(|e| format!("download interrupted: {e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            done += n as u64;
            if let Some(total) = total.filter(|t| *t > 0) {
                let pct = done * 100 / total;
                if pct != last_pct {
                    last_pct = pct;
                    emit(events, Event::Stage(format!("{label} {pct}%")));
                    emit(events, Event::Progress(done as f64 / total as f64));
                }
            }
        }
        file.flush().map_err(|e| e.to_string())?;
        file.get_ref().sync_data().map_err(|e| e.to_string())?;
        drop(file);
        if total.is_some_and(|t| t != done) || done < min_bytes {
            if valid(target) {
                // Another downloader filled the target in the meantime.
                return Ok(());
            }
            return Err(format!("the download of {url} was incomplete"));
        }
        match std::fs::rename(&part, target) {
            Ok(()) => Ok(()),
            Err(e) => {
                if valid(target) {
                    return Ok(());
                }
                Err(e.to_string())
            }
        }
    })();
    let _ = std::fs::remove_file(&part);
    result
}

// ---------------------------------------------------------------------------
// Transcription

/// Transcribes the meeting. `language` is a whisper code or "auto".
pub fn transcribe(
    mic: &[f32],
    computer: &[f32],
    language: &str,
    events: &Events,
    abort: &Abort,
) -> Result<Transcript, String> {
    let _slot = acquire_heavy_slot(events, abort)?;
    let duration_secs = (mic.len().max(computer.len()) / WHISPER_RATE) as i64;
    let empty = |language: &str| Transcript {
        segments: Vec::new(),
        language: if language == "auto" {
            "unknown".into()
        } else {
            language.to_owned()
        },
        duration_secs,
        diarization_failed: false,
    };
    if is_silent(mic) && is_silent(computer) {
        emit(events, Event::Progress(1.0));
        return Ok(empty(language));
    }

    // Each side goes through whisper on its own: whisper follows one voice at
    // a time, so two people talking at once, or a song under someone, would
    // otherwise lose the quieter one. The side of a line is then its track.
    let (mic, computer) = (mix(mic, &[]), mix(computer, &[]));
    let mic_regions = own_speech_regions(&mic, &computer);
    let computer_regions = speech_regions(&[&computer], computer.len());
    if mic_regions.is_empty() && computer_regions.is_empty() {
        emit(events, Event::Progress(1.0));
        return Ok(empty(language));
    }
    // Several voices on one side are told apart: people sharing your mic, or
    // several people on the other end of the call. On the mic only your own
    // stretches count, so the other side leaking in is not taken for a person
    // in the room. The speaker model is loaded once for both sides.
    let mut speaker_model: Option<crate::nemotron::Model> = None;
    let (local, local_failed) =
        voices(&only(&mic, &mic_regions), &mut speaker_model, events, abort)?;
    // Echo that got past the level check can still come out as a voice of
    // its own; its lines are dropped after the mic's pass.
    let (local, echo) = split_echo(
        local,
        &active_frames(&computer, computer.len().div_ceil(FRAME)),
    );
    let (remote, remote_failed) = voices(&computer, &mut speaker_model, events, abort)?;
    let diarization_failed = local_failed || remote_failed;
    let context = load_whisper(events, abort)?;

    let length = |regions: &[Region]| regions.iter().map(|r| r.end - r.start).sum::<usize>();
    let total = (length(&mic_regions) + length(&computer_regions)).max(1) as f64;
    let mut sides = [
        (&mic, &mic_regions, Speakers::Side("You", local), echo),
        (
            &computer,
            &computer_regions,
            Speakers::Side("Remote", remote),
            Vec::new(),
        ),
    ];
    // The side with the most sound first: with "auto" its language counts for both.
    sides.sort_by_key(|(_, regions, _, _)| std::cmp::Reverse(length(regions)));
    let mut language = language.to_owned();
    let mut detected = None;
    let mut segments = Vec::new();
    let mut done = 0.0;
    for (track, regions, speakers, echo) in &sides {
        if regions.is_empty() {
            continue;
        }
        let share = length(regions) as f64 / total;
        let (lines, found) = side_pass(
            &context,
            track,
            regions,
            speakers,
            &language,
            (done, done + share),
            false,
            events,
            abort,
        )?;
        if language == "auto"
            && let Some(found) = found
        {
            language = found.clone();
            detected = Some(found);
        }
        segments.extend(lines.into_iter().filter(|l| {
            echo.is_empty() || crate::diarize::speaker_at(echo, l.start_ms, l.end_ms) == 0
        }));
        done += share;
    }
    emit(events, Event::Progress(1.0));
    Ok(Transcript {
        segments: interleave(segments),
        language: if language == "auto" {
            detected.unwrap_or_else(|| "unknown".into())
        } else {
            language
        },
        duration_secs,
        diarization_failed,
    })
}

/// The sentences of both sides in the order they were said, joined into
/// paragraphs per speaker. A sentence of yours that repeats what the other
/// side said at the same moment is their voice leaking into your mic, and goes.
fn interleave(mut sentences: Vec<Segment>) -> Vec<Segment> {
    let is_local = |speaker: &str| {
        crate::meeting::side_of(speaker)
            .is_some_and(|(side, _)| side == crate::meeting::DEFAULT_YOU)
    };
    sentences.sort_by_key(|s| s.start_ms);
    let words = |text: &str| -> Vec<String> {
        text.split_whitespace()
            .map(|w| {
                w.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
            })
            .filter(|w| !w.is_empty())
            .collect()
    };
    let trigrams = |w: &[String]| -> Vec<String> { w.windows(3).map(|t| t.join(" ")).collect() };
    let is_echo = |mine: &Segment| {
        let own_words = words(&mine.text);
        let near: Vec<Vec<String>> = sentences
            .iter()
            .filter(|s| {
                !is_local(&s.speaker)
                    && s.start_ms < mine.end_ms + 2000
                    && mine.start_ms < s.end_ms + 2000
            })
            .map(|s| words(&s.text))
            .collect();
        let own = trigrams(&own_words);
        if own.is_empty() {
            // One or two words ("Oui.", "D'accord."): a short reply is not
            // provably an echo. Text similarity alone cannot tell "you
            // answering the same word" from "their voice leaking into your
            // mic", so keep the line instead of deleting a real answer.
            // Echo suppression only applies to longer phrases below.
            return false;
        }
        let theirs: std::collections::HashSet<String> =
            near.iter().flat_map(|w| trigrams(w)).collect();
        own.iter().filter(|t| theirs.contains(*t)).count() * 2 >= own.len()
    };
    let keep: Vec<bool> = sentences
        .iter()
        .map(|s| !is_local(&s.speaker) || !is_echo(s))
        .collect();
    let mut out: Vec<Segment> = Vec::new();
    for (sentence, keep) in sentences.into_iter().zip(keep) {
        if !keep {
            continue;
        }
        match out.last_mut() {
            Some(last)
                if last.speaker == sentence.speaker
                    && (sentence.start_ms - last.end_ms < PARAGRAPH_PAUSE_MS
                        || !ends_sentence(&last.text))
                    && sentence.end_ms - last.start_ms < PARAGRAPH_MAX_MS =>
            {
                last.text.push(' ');
                last.text.push_str(&sentence.text);
                last.end_ms = last.end_ms.max(sentence.end_ms);
            }
            _ => out.push(sentence),
        }
    }
    out
}

/// Splits the voices found on the mic into real ones and echo. Through
/// speakers the other side reaches the mic, and when it is loud enough to pass
/// `own_speech_regions` the diarization hears it as one more person in the
/// room. Such a voice only speaks while the computer audio does: a voice with
/// most of its speech (60% or more) over computer audio is echo, as long as
/// another voice on the mic mostly speaks on its own (under 40%), so that a
/// call over steady music never loses you. Returns the turns of the real
/// voices, renumbered (empty when one is left), and all turns with speaker 0
/// for a real voice and 1 for echo (empty when there is no echo).
fn split_echo(
    turns: Vec<crate::diarize::Turn>,
    computer_active: &[bool],
) -> (Vec<crate::diarize::Turn>, Vec<crate::diarize::Turn>) {
    let mut spoken = std::collections::BTreeMap::<usize, (i64, i64)>::new();
    for turn in &turns {
        let first = ms_to_sample(turn.start_ms) / FRAME;
        let last = ms_to_sample(turn.end_ms).div_ceil(FRAME);
        let (all, over) = spoken.entry(turn.speaker).or_default();
        for frame in first..last {
            *all += 1;
            *over += i64::from(computer_active.get(frame).copied().unwrap_or(false));
        }
    }
    let share = |speaker: &usize| {
        spoken
            .get(speaker)
            .map_or(0.0, |(all, over)| *over as f64 / (*all).max(1) as f64)
    };
    let is_echo = |t: &crate::diarize::Turn| share(&t.speaker) >= 0.6;
    if !spoken.keys().any(|s| share(s) < 0.4) || !turns.iter().any(is_echo) {
        return (turns, Vec::new());
    }
    let mask: Vec<crate::diarize::Turn> = turns
        .iter()
        .map(|t| crate::diarize::Turn {
            speaker: usize::from(is_echo(t)),
            ..t.clone()
        })
        .collect();
    let real: Vec<crate::diarize::Turn> = turns.into_iter().filter(|t| !is_echo(t)).collect();
    let mut order: Vec<usize> = Vec::new();
    let real: Vec<crate::diarize::Turn> = real
        .into_iter()
        .map(|t| {
            let speaker = order
                .iter()
                .position(|s| *s == t.speaker)
                .unwrap_or_else(|| {
                    order.push(t.speaker);
                    order.len() - 1
                });
            crate::diarize::Turn { speaker, ..t }
        })
        .collect();
    (if order.len() < 2 { Vec::new() } else { real }, mask)
}

/// `track` with everything outside `regions` silenced.
fn only(track: &[f32], regions: &[Region]) -> Vec<f32> {
    let mut out = vec![0.0; track.len()];
    for region in regions {
        out[region.start..region.end].copy_from_slice(&track[region.start..region.end]);
    }
    out
}

/// Who is who on one side of a recording: the turns when more than one voice
/// is heard there, nothing when it is one person. A missing speaker model is
/// no reason to fail the transcript; the side then stays one speaker and the
/// returned flag tells the transcript that the separation failed. The model
/// is loaded once and shared between both sides.
fn voices(
    track: &[f32],
    model: &mut Option<crate::nemotron::Model>,
    events: &Events,
    abort: &Abort,
) -> Result<(Vec<crate::diarize::Turn>, bool), String> {
    if is_silent(track) {
        return Ok((Vec::new(), false));
    }
    if model.is_none() {
        match crate::nemotron::ensure(events, abort)
            .and_then(|path| crate::nemotron::Model::load(&path))
        {
            Ok(loaded) => *model = Some(loaded),
            Err(e) if e == CANCELLED => return Err(e),
            Err(e) => {
                eprintln!("{}: telling voices apart: {e}", crate::APP_NAME);
                return Ok((Vec::new(), true));
            }
        }
    }
    // `diarize::turns_loaded` runs on the already loaded model, so the model
    // stays shared between both sides instead of reloading per side.
    let Some(loaded) = model.as_mut() else {
        eprintln!(
            "{}: telling voices apart: missing speaker model",
            crate::APP_NAME
        );
        return Ok((Vec::new(), true));
    };
    match crate::diarize::turns_loaded(loaded, track, None, events, abort) {
        Ok(turns) if turns.iter().any(|t| t.speaker > 0) => Ok((turns, false)),
        Ok(_) => Ok((Vec::new(), false)),
        Err(e) if e == CANCELLED => Err(e),
        Err(e) => {
            eprintln!("{}: telling voices apart: {e}", crate::APP_NAME);
            Ok((Vec::new(), true))
        }
    }
}

/// Transcribes one imported audio file (16 kHz mono) and tells the voices in
/// it apart. `speakers` fixes how many people speak; `None` lets the
/// clustering decide, `Some(1)` skips finding speakers altogether. The lines
/// are labelled "Speaker 1", "Speaker 2", ... in the order they first speak.
pub fn transcribe_single(
    track: &[f32],
    language: &str,
    speakers: Option<usize>,
    events: &Events,
    abort: &Abort,
) -> Result<Transcript, String> {
    let _slot = acquire_heavy_slot(events, abort)?;
    let duration_secs = (track.len() / WHISPER_RATE) as i64;
    let empty = || Transcript {
        segments: Vec::new(),
        language: if language == "auto" {
            "unknown".into()
        } else {
            language.to_owned()
        },
        duration_secs,
        diarization_failed: false,
    };
    if is_silent(track) {
        emit(events, Event::Progress(1.0));
        return Ok(empty());
    }
    let level = mix(track, &[]);
    let regions = speech_regions(&[track], level.len());
    if regions.is_empty() {
        emit(events, Event::Progress(1.0));
        return Ok(empty());
    }
    // Speakers first, so the live lines can already say who is talking.
    // A missing speaker model is no reason to fail the transcript (same
    // fallback as `voices` for recordings); the file then stays one speaker.
    let (turns, diarization_failed) = match speakers {
        Some(1) => (crate::diarize::single(track), false),
        _ => match crate::diarize::turns(track, speakers, events, abort) {
            Ok(turns) => (turns, false),
            Err(e) if e == CANCELLED => return Err(e),
            Err(e) => {
                eprintln!("{}: telling voices apart: {e}", crate::APP_NAME);
                (crate::diarize::single(track), true)
            }
        },
    };
    let speakers = Speakers::Turns(turns);
    whisper_pass(
        &level,
        &regions,
        &speakers,
        language,
        duration_secs,
        diarization_failed,
        (events, abort),
    )
}

/// The shared part: whisper over the stretches with sound, then the lines.
fn whisper_pass(
    mixed: &[f32],
    regions: &[Region],
    speakers: &Speakers,
    language: &str,
    duration_secs: i64,
    diarization_failed: bool,
    reporting: (&Events, &Abort),
) -> Result<Transcript, String> {
    let (events, abort) = reporting;
    let context = load_whisper(events, abort)?;
    let (segments, detected) = side_pass(
        &context,
        mixed,
        regions,
        speakers,
        language,
        (0.0, 1.0),
        true,
        events,
        abort,
    )?;
    emit(events, Event::Progress(1.0));
    Ok(Transcript {
        segments,
        language: if language == "auto" {
            detected.unwrap_or_else(|| "unknown".into())
        } else {
            language.to_owned()
        },
        duration_secs,
        diarization_failed,
    })
}

fn load_whisper(events: &Events, abort: &Abort) -> Result<WhisperContext, String> {
    let model = crate::models::ensure(events, abort)?;
    if abort.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    emit(events, Event::Stage("Loading model".into()));
    emit(events, Event::Progress(0.0));
    whisper_rs::install_logging_hooks();
    let mut context_params = WhisperContextParameters::default();
    context_params.use_gpu(cfg!(feature = "vulkan"));
    // Word times aligned on the attention heads (DTW): the plain token times
    // drift by up to a second, too much to tell where one speaker takes over.
    // A model file of unknown kind gets plain token times.
    if let Some(model_preset) = crate::models::dtw_preset() {
        context_params.dtw_parameters(DtwParameters {
            mode: DtwMode::ModelPreset { model_preset },
            ..Default::default()
        });
    }
    WhisperContext::new_with_params(&model, context_params)
        .map_err(|e| format!("could not load the model {}: {e}", model.display()))
}

/// Whisper over the stretches of `track` with sound in them, then the lines.
/// Progress runs from `progress.0` to `progress.1`.
#[allow(clippy::too_many_arguments)]
fn side_pass(
    context: &WhisperContext,
    track: &[f32],
    regions: &[Region],
    speakers: &Speakers,
    language: &str,
    progress: (f64, f64),
    paragraphs: bool,
    events: &Events,
    abort: &Abort,
) -> Result<(Vec<Segment>, Option<String>), String> {
    let glued = Glued::new(track, regions);
    emit(events, Event::Stage("Transcribing".into()));
    let (words, detected) =
        run_whisper(context, &glued, speakers, language, progress, events, abort)?;
    Ok((
        phrases(&words, &glued, speakers, track, paragraphs),
        detected,
    ))
}

/// A word with its times in the glued buffer, and how sure whisper was that
/// its segment held speech at all.
struct Word {
    text: String,
    start_ms: i64,
    end_ms: i64,
    no_speech: f32,
    /// Index of the whisper segment it came from.
    segment: usize,
}

/// Owned for one synchronous `WhisperState::full` call. The dependency's safe
/// callback setters leak their boxes; use its raw setters with this scoped
/// owner instead. Callbacks only read it, including the worker-thread abort.
struct WhisperCallbacks {
    events: Events,
    abort: Abort,
    progress: (f64, f64),
    map: Vec<(usize, Region)>,
    speakers: Speakers,
}

impl WhisperCallbacks {
    /// # Safety
    /// This owner must stay at its current address until the synchronous full
    /// call returns. Do not retain or reuse params after dropping the owner.
    unsafe fn install(&self, params: &mut FullParams<'_, '_>) {
        let data = self as *const Self as *mut std::ffi::c_void;
        // SAFETY: the caller retains the owner for the entire call; all three
        // trampolines use shared access and no callback mutates the owner.
        unsafe {
            params.set_progress_callback(Some(whisper_progress));
            params.set_progress_callback_user_data(data);
            params.set_new_segment_callback(Some(whisper_segment));
            params.set_new_segment_callback_user_data(data);
            params.set_abort_callback(Some(whisper_abort));
            params.set_abort_callback_user_data(data);
        }
    }
}

unsafe extern "C" fn whisper_progress(
    _: *mut whisper_rs::WhisperSysContext,
    _: *mut whisper_rs::WhisperSysState,
    pct: i32,
    data: *mut std::ffi::c_void,
) {
    // SAFETY: install passes the live, immutably accessed callback owner.
    let callbacks = unsafe { &*(data as *const WhisperCallbacks) };
    let pct = f64::from(pct.clamp(0, 100)) / 100.0;
    emit(
        &callbacks.events,
        Event::Progress(callbacks.progress.0 + (callbacks.progress.1 - callbacks.progress.0) * pct),
    );
}

unsafe extern "C" fn whisper_abort(data: *mut std::ffi::c_void) -> bool {
    // SAFETY: install passes the live owner; AtomicBool permits worker reads.
    unsafe { &*(data as *const WhisperCallbacks) }
        .abort
        .load(Ordering::Relaxed)
}

unsafe extern "C" fn whisper_segment(
    _: *mut whisper_rs::WhisperSysContext,
    state: *mut whisper_rs::WhisperSysState,
    added: i32,
    data: *mut std::ffi::c_void,
) {
    use whisper_rs::whisper_rs_sys as sys;
    // SAFETY: Whisper invokes this synchronously with its live state and the
    // callback owner installed above. Segment indices come from that state.
    let callbacks = unsafe { &*(data as *const WhisperCallbacks) };
    let count = unsafe { sys::whisper_full_n_segments_from_state(state) };
    for index in (count - added).max(0)..count {
        let text = unsafe { sys::whisper_full_get_segment_text_from_state(state, index) };
        if text.is_null() {
            continue;
        }
        // SAFETY: a segment's text is a NUL-terminated string owned by state.
        let Ok(text) = unsafe { std::ffi::CStr::from_ptr(text) }.to_str() else {
            continue;
        };
        let text = text.trim();
        if text.is_empty() || is_noise_marker(text) {
            continue;
        }
        let start = unsafe { sys::whisper_full_get_segment_t0_from_state(state, index) };
        let end = unsafe { sys::whisper_full_get_segment_t1_from_state(state, index) };
        let (start, _) = locate(&callbacks.map, start * 10);
        let (end, _) = locate(&callbacks.map, end * 10);
        let speaker = callbacks.speakers.speaker(start, end);
        emit(
            &callbacks.events,
            Event::Segment(format!("{speaker}: {text}")),
        );
    }
}

fn run_whisper(
    context: &WhisperContext,
    glued: &Glued,
    speakers: &Speakers,
    language: &str,
    progress: (f64, f64),
    events: &Events,
    abort: &Abort,
) -> Result<(Vec<Word>, Option<String>), String> {
    let mut state = context.create_state().map_err(|e| e.to_string())?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    params.set_n_threads(threads.min(16) as i32);
    params.set_language(Some(language));
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_suppress_blank(true);
    params.set_suppress_nst(true);
    params.set_no_speech_thold(0.6);
    // Word times, so a segment can be split where the speaker changes.
    params.set_token_timestamps(true);
    params.set_split_on_word(true);

    let callbacks = Box::new(WhisperCallbacks {
        events: events.clone(),
        abort: abort.clone(),
        progress,
        map: glued.map.clone(),
        speakers: speakers.clone(),
    });
    // SAFETY: the box has a stable address, full is synchronous, and params is
    // consumed by it. The owner remains alive until every callback finishes.
    unsafe { callbacks.install(&mut params) };
    let result = state.full(params, &glued.samples);
    drop(callbacks);
    if abort.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    result.map_err(|e| e.to_string())?;

    let detected = whisper_rs::get_lang_str(state.full_lang_id_from_state()).map(str::to_owned);
    let eot = context.token_eot();
    let mut words: Vec<Word> = Vec::new();
    for (index, segment) in state.as_iter().enumerate() {
        let no_speech = segment.no_speech_probability();
        // Bytes first: a character can be split over two tokens.
        let mut current: Option<(Vec<u8>, i64, i64)> = None;
        let flush = |current: &mut Option<(Vec<u8>, i64, i64)>, words: &mut Vec<Word>| {
            if let Some((bytes, start_ms, end_ms)) = current.take() {
                let text = String::from_utf8_lossy(&bytes).trim().to_owned();
                if !text.is_empty() {
                    words.push(Word {
                        text,
                        start_ms,
                        end_ms,
                        no_speech,
                        segment: index,
                    });
                }
            }
        };
        for i in 0..segment.n_tokens() {
            let Some(token) = segment.get_token(i) else {
                continue;
            };
            if token.token_id() >= eot {
                continue; // timestamps and other special tokens
            }
            let Ok(bytes) = token.to_bytes() else {
                continue;
            };
            let data = token.token_data();
            // DTW gives one moment per token; fall back to the plain times.
            let (t0, t1) = if data.t_dtw >= 0 {
                (data.t_dtw * 10, data.t_dtw * 10)
            } else {
                (data.t0 * 10, data.t1 * 10)
            };
            match current.as_mut() {
                Some((word, _, end)) if !bytes.starts_with(b" ") => {
                    word.extend_from_slice(bytes);
                    *end = t1.max(*end);
                }
                _ => {
                    flush(&mut current, &mut words);
                    current = Some((bytes.to_vec(), t0, t1));
                }
            }
        }
        flush(&mut current, &mut words);
    }
    Ok((words, detected))
}

/// A pause longer than this starts a new paragraph, even for the same speaker.
const PARAGRAPH_PAUSE_MS: i64 = 3000;
/// Very long turns are still split, so a line stays a useful place to jump to.
const PARAGRAPH_MAX_MS: i64 = 90_000;

/// Groups the words into the lines of the transcript: a new line where the
/// speaker changes, where a sentence ends on another speaker, and at every
/// stretch of silence. Timestamps are put back on the real timeline.
fn phrases(
    words: &[Word],
    glued: &Glued,
    speakers: &Speakers,
    mixed: &[f32],
    paragraphs: bool,
) -> Vec<Segment> {
    struct Phrase {
        words: Vec<String>,
        start_ms: i64,
        end_ms: i64,
        region: usize,
        no_speech: f32,
        segment: usize,
    }

    // A stock phrase is only suspicious when whisper heard nothing else around it.
    let mut per_segment = std::collections::HashMap::<usize, usize>::new();
    for word in words {
        *per_segment.entry(word.segment).or_default() += 1;
    }

    // First cut at sentence ends and region changes, so each piece has one voice.
    let mut pieces: Vec<Phrase> = Vec::new();
    for word in words {
        let (start, region) = glued.locate(word.start_ms);
        let (end, _) = glued.locate(word.end_ms.max(word.start_ms));
        let sentence_ended = pieces
            .last()
            .is_some_and(|p| p.words.last().is_some_and(|w| ends_sentence(w)));
        let turn_at_pause = speakers.cuts_at_pauses()
            && pieces.last().is_some_and(|p| {
                start - p.end_ms >= 250
                    && speakers.speaker(p.start_ms, p.end_ms)
                        != speakers.speaker(start, end.max(start))
            });
        match pieces.last_mut() {
            Some(p)
                if p.region == region
                    && p.segment == word.segment
                    && !sentence_ended
                    && !turn_at_pause =>
            {
                p.words.push(word.text.clone());
                p.end_ms = end.max(p.end_ms);
                p.no_speech = p.no_speech.max(word.no_speech);
            }
            _ => pieces.push(Phrase {
                words: vec![word.text.clone()],
                start_ms: start,
                end_ms: end.max(start),
                region,
                no_speech: word.no_speech,
                segment: word.segment,
            }),
        }
    }

    // The first words of a stretch of sound begin where the sound does; whisper
    // tends to put them at the start of the padding instead.
    let mut previous_region = usize::MAX;
    for piece in &mut pieces {
        if piece.region != previous_region
            && let Some((_, region)) = glued.map.get(piece.region)
        {
            let onset = sample_to_ms(region.onset);
            if (piece.start_ms - onset).abs() < 1500 {
                let shift = onset - piece.start_ms;
                piece.start_ms = onset;
                piece.end_ms += shift.max(0);
            }
        }
        previous_region = piece.region;
        // Token times can collapse to nothing; about 250 ms a word is a floor.
        let floor = piece.start_ms + 250 * piece.words.len() as i64;
        piece.end_ms = piece.end_ms.max(floor);
    }

    // A sentence is never split between speakers: a piece that stops without
    // a sentence end is joined to the next one when that follows within a
    // short pause, and the speaker is then chosen for the sentence as a whole.
    let mut sentences: Vec<Phrase> = Vec::new();
    for piece in pieces {
        match sentences.last_mut() {
            Some(previous)
                if !ends_sentence(previous.words.last().map_or("", String::as_str))
                    && piece.start_ms - previous.end_ms < 3000 =>
            {
                previous.words.extend(piece.words);
                previous.end_ms = piece.end_ms.max(previous.end_ms);
                previous.no_speech = previous.no_speech.max(piece.no_speech);
            }
            _ => sentences.push(piece),
        }
    }
    let pieces = sentences;

    // Who said it, then glue neighbours by the same speaker back together.
    let mut segments: Vec<Segment> = Vec::new();
    for piece in pieces {
        let text = piece.words.join(" ");
        let whole = per_segment.get(&piece.segment) == Some(&piece.words.len());
        if is_noise_marker(&text) || is_hallucination(&text, &piece, whole, mixed) {
            continue;
        }
        // The whole piece goes to the speaker it overlaps most.
        let speaker = speakers.speaker(piece.start_ms, piece.end_ms);
        // A new speaker starts where their voice takes over, not where whisper
        // guessed; keep the order of lines intact.
        let mut piece = piece;
        let changed = segments.last().is_none_or(|last| last.speaker != speaker);
        if changed && let Some(start) = speakers.takeover_ms(&speaker, piece.start_ms) {
            let floor = segments.last().map_or(0, |last| last.start_ms + 1);
            piece.start_ms = start.max(floor);
            piece.end_ms = piece.end_ms.max(piece.start_ms + 1);
        }
        // One paragraph per turn: short pauses ("So... Okay. Then...") stay
        // together, a long pause or a very long turn starts a new paragraph.
        match segments.last_mut() {
            Some(last)
                if paragraphs
                    && last.speaker == speaker
                    && (piece.start_ms - last.end_ms < PARAGRAPH_PAUSE_MS
                        || !ends_sentence(&last.text))
                    && piece.end_ms - last.start_ms < PARAGRAPH_MAX_MS =>
            {
                last.text.push(' ');
                last.text.push_str(&text);
                last.end_ms = piece.end_ms;
            }
            _ => segments.push(Segment {
                start_ms: piece.start_ms,
                end_ms: piece.end_ms,
                speaker,
                text,
            }),
        }
    }

    fn is_hallucination(text: &str, piece: &Phrase, whole: bool, mixed: &[f32]) -> bool {
        let span = &mixed[ms_to_sample(piece.start_ms).min(mixed.len())
            ..ms_to_sample(piece.end_ms).min(mixed.len())];
        let level = rms(span);
        if level < 0.004 || piece.no_speech > 0.85 {
            return true; // whisper talking over silence
        }
        // The things whisper says when it hears nothing, from its subtitle diet.
        whole && is_stock_phrase(text) && (piece.no_speech > 0.3 || level < 0.02)
    }

    segments
}

/// Whether `text` ends a sentence: a full stop, question or exclamation mark,
/// possibly followed by a closing quote or bracket.
fn ends_sentence(text: &str) -> bool {
    text.trim_end()
        .trim_end_matches(['"', '\'', ')', '\u{201d}', '\u{2019}'])
        .ends_with(['.', '?', '!', '\u{2026}'])
}

/// "Thank you.", "Bye.", "Thanks for watching!" and friends: what whisper
/// produces for silence or noise, learned from subtitles.
fn is_stock_phrase(text: &str) -> bool {
    const STOCK: &[&str] = &[
        "thank you",
        "thank you very much",
        "thanks",
        "thanks for watching",
        "thank you for watching",
        "bye",
        "bye bye",
        "you",
        "okay",
        "so",
        "subtitles by the amaraorg community",
        "subtitles by",
        "please subscribe",
        "dank je",
        "dank je wel",
        "bedankt",
        "bedankt voor het kijken",
        "ondertiteling",
        "ondertiteld door",
        "tot de volgende keer",
        "vielen dank",
        "untertitel im auftrag des zdf",
        "merci",
    ];
    let normalized: String = text
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    STOCK.contains(&normalized.as_str()) || normalized.starts_with("subtitles by")
}

/// "[BLANK_AUDIO]", "(music)", "*applause*" and friends.
fn is_noise_marker(text: &str) -> bool {
    let t = text.trim();
    let wrapped = |open: char, close: char| t.starts_with(open) && t.ends_with(close);
    wrapped('[', ']')
        || wrapped('(', ')')
        || wrapped('*', '*')
        || t.chars().all(|c| !c.is_alphanumeric())
}

// ---------------------------------------------------------------------------
// Output

fn clock(ms: i64) -> String {
    let secs = ms / 1000;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

fn language_name(code: &str) -> String {
    LANGUAGES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, label)| (*label).to_owned())
        .or_else(|| {
            whisper_rs::get_lang_id(code)
                .and_then(whisper_rs::get_lang_str_full)
                .map(|full| {
                    let mut chars = full.chars();
                    chars
                        .next()
                        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                        .unwrap_or_default()
                })
        })
        .unwrap_or_else(|| code.to_owned())
}

pub fn to_markdown(title: &str, date: &str, transcript: &Transcript) -> String {
    let mut out = format!("# {title}\n\n");
    out += &format!("- **Date:** {date}\n");
    out += &format!(
        "- **Duration:** {}\n",
        clock(transcript.duration_secs * 1000)
    );
    out += &format!(
        "- **Language:** {}\n\n",
        language_name(&transcript.language)
    );
    out += "## Transcript\n\n";
    if transcript.segments.is_empty() {
        out += "_No speech was recognized._\n";
    }
    for segment in &transcript.segments {
        out += &format!(
            "**[{}] {}:** {}\n\n",
            clock(segment.start_ms),
            segment.speaker,
            segment.text
        );
    }
    out
}

// ---------------------------------------------------------------------------
// CLI

/// `omarchy-meeting-recorder transcribe <mic> <computer> [--language xx]`
pub fn cli(args: &[String]) -> glib::ExitCode {
    let mut files = Vec::new();
    let mut language = "auto".to_owned();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--language" | "-l" => match iter.next() {
                Some(code) => language = code.clone(),
                None => return usage(),
            },
            "--model" | "-m" => match iter.next() {
                Some(name) => crate::models::set_override(name),
                None => return usage(),
            },
            _ => files.push(PathBuf::from(arg)),
        }
    }
    let [mic_path, computer_path] = files.as_slice() else {
        return usage();
    };
    run_cli(|events, abort| {
        let mic = load_track(mic_path)?;
        let computer = load_track(computer_path)?;
        transcribe(&mic, &computer, &language, events, abort)
    })
}

/// `omarchy-meeting-recorder transcribe-file <audio> [--speakers N] [--language xx]`
pub fn cli_file(args: &[String]) -> glib::ExitCode {
    let mut files = Vec::new();
    let mut language = "auto".to_owned();
    let mut speakers = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--language" | "-l" => match iter.next() {
                Some(code) => language = code.clone(),
                None => return usage(),
            },
            "--model" | "-m" => match iter.next() {
                Some(name) => crate::models::set_override(name),
                None => return usage(),
            },
            "--speakers" | "-s" => match iter.next().and_then(|n| n.parse::<usize>().ok()) {
                Some(n) if (1..=8).contains(&n) => speakers = Some(n),
                _ => return usage(),
            },
            _ => files.push(PathBuf::from(arg)),
        }
    }
    let [path] = files.as_slice() else {
        return usage();
    };
    run_cli(|events, abort| {
        let track = load_track(path)?;
        transcribe_single(&track, &language, speakers, events, abort)
    })
}

/// Runs a transcription for the command line: progress and live lines on
/// stderr, the Markdown on stdout.
fn run_cli(work: impl FnOnce(&Events, &Abort) -> Result<Transcript, String>) -> glib::ExitCode {
    let (tx, rx) = async_channel::unbounded();
    let started = Instant::now();
    let reporter = std::thread::spawn(move || {
        let mut last_stage = String::new();
        while let Ok(event) = rx.recv_blocking() {
            match event {
                Event::Stage(stage) if stage != last_stage => {
                    // Downloads report every percent; show every tenth.
                    let percent = stage
                        .rsplit_once(' ')
                        .and_then(|(_, p)| p.strip_suffix('%'))
                        .and_then(|p| p.parse::<u32>().ok());
                    if percent.is_none_or(|p| p % 10 == 0) {
                        eprintln!("[{:6.1}s] {stage}", started.elapsed().as_secs_f64());
                    }
                    last_stage = stage;
                }
                Event::Segment(text) => eprintln!("  {text}"),
                Event::Finished => break,
                _ => {}
            }
        }
    });

    let abort = Abort::default();
    let result = work(&tx, &abort);
    emit(&tx, Event::Finished);
    let _ = reporter.join();

    match result {
        Ok(transcript) => {
            let date = glib::DateTime::now_local()
                .and_then(|t| t.format("%Y-%m-%d %H:%M"))
                .map(|s| s.to_string())
                .unwrap_or_default();
            print!("{}", to_markdown("Transcript", &date, &transcript));
            eprintln!("Done in {:.1}s", started.elapsed().as_secs_f64());
            glib::ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{APP_NAME}: {message}");
            glib::ExitCode::FAILURE
        }
    }
}

fn usage() -> glib::ExitCode {
    eprintln!(
        "Usage: {APP_NAME} transcribe <mic> <computer> [--language auto|en|nl|...] [--model name]"
    );
    eprintln!(
        "       {APP_NAME} transcribe-file <audio> [--speakers N] [--language auto|en|nl|...] [--model name]"
    );
    glib::ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whisper_callbacks_release_their_data_and_keep_progress_and_abort_behavior() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<WhisperCallbacks>();
        for cancelled in [false, true] {
            for _ in 0..20 {
                let (events, receiver) = async_channel::unbounded();
                let abort = Arc::new(AtomicBool::new(cancelled));
                let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
                let callbacks = Box::new(WhisperCallbacks {
                    events: events.clone(),
                    abort: abort.clone(),
                    progress: (0.25, 0.75),
                    map: Vec::new(),
                    speakers: Speakers::Turns(Vec::new()),
                });
                // SAFETY: keep the stable owner alive for every invocation;
                // progress/abort do not use the null context/state arguments.
                unsafe {
                    callbacks.install(&mut params);
                    let data = &*callbacks as *const WhisperCallbacks as *mut std::ffi::c_void;
                    whisper_progress(std::ptr::null_mut(), std::ptr::null_mut(), 50, data);
                    assert_eq!(whisper_abort(data), cancelled);
                }
                assert!(matches!(receiver.try_recv().unwrap(), Event::Progress(p) if p == 0.5));
                drop(params);
                drop(callbacks);
                assert_eq!(Arc::strong_count(&abort), 1);
                drop(events);
                // A leaked callback retains a sender even after params drops.
                assert!(receiver.is_closed());
            }
        }
    }

    #[test]
    fn interrupted_download_removes_its_partial_file() {
        use std::net::TcpListener;
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/model", server.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut client, _) = server.accept().unwrap();
            let mut request = [0; 4096];
            let _ = client.read(&mut request);
            client
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\nConnection: close\r\n\r\npartial")
                .unwrap();
        });
        let dir =
            std::env::temp_dir().join(format!("mr-interrupted-download-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (events, _rx) = async_channel::unbounded();
        assert!(
            download(
                &url,
                &dir.join("model.bin"),
                "Test",
                100,
                &events,
                &Abort::default()
            )
            .is_err()
        );
        worker.join().unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir(&dir).unwrap();
    }

    fn line(start_ms: i64, speaker: &str, text: &str) -> Segment {
        Segment {
            start_ms,
            end_ms: start_ms + 2000,
            speaker: speaker.into(),
            text: text.into(),
        }
    }

    #[test]
    fn both_sides_come_back_in_the_order_they_spoke() {
        let out = interleave(vec![
            line(0, "Remote", "Thanks, I can start with the release."),
            line(5000, "Remote", "So the beta went out on Monday."),
            line(2500, "You", "Sure, go ahead."),
        ]);
        let order: Vec<&str> = out.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(
            order,
            [
                "Thanks, I can start with the release.",
                "Sure, go ahead.",
                "So the beta went out on Monday."
            ]
        );
    }

    fn turn(start_ms: i64, end_ms: i64, speaker: usize) -> crate::diarize::Turn {
        crate::diarize::Turn {
            start_ms,
            end_ms,
            speaker,
        }
    }

    /// Computer audio in the given ms ranges, as frames.
    fn computer_audio(ranges: &[(i64, i64)]) -> Vec<bool> {
        (0..1000)
            .map(|f| {
                let ms = sample_to_ms(f * FRAME);
                ranges.iter().any(|(s, e)| (*s..*e).contains(&ms))
            })
            .collect()
    }

    #[test]
    fn a_voice_on_the_mic_that_only_speaks_with_the_other_side_is_echo() {
        // You speak on your own; the second voice always with the other side.
        let turns = vec![
            turn(0, 2000, 0),
            turn(2000, 4000, 1),
            turn(4000, 6000, 0),
            turn(6000, 8000, 1),
        ];
        let active = computer_audio(&[(2000, 4000), (6000, 8000)]);
        let (real, mask) = split_echo(turns, &active);
        // One voice left: the side is just "You".
        assert!(real.is_empty());
        let echo: Vec<usize> = mask.iter().map(|t| t.speaker).collect();
        assert_eq!(echo, vec![0, 1, 0, 1]);
    }

    #[test]
    fn two_people_sharing_the_mic_stay_two() {
        let turns = vec![turn(0, 2000, 0), turn(2000, 4000, 1), turn(4000, 6000, 0)];
        let active = computer_audio(&[(6000, 9000)]);
        let (real, mask) = split_echo(turns.clone(), &active);
        assert_eq!(real, turns);
        assert!(mask.is_empty());
    }

    #[test]
    fn steady_music_under_the_call_drops_no_one() {
        let turns = vec![turn(0, 2000, 0), turn(2000, 4000, 1)];
        let active = computer_audio(&[(0, 9000)]);
        let (real, mask) = split_echo(turns.clone(), &active);
        assert_eq!(real, turns);
        assert!(mask.is_empty());
    }

    #[test]
    fn the_other_side_leaking_into_the_mic_is_dropped() {
        let out = interleave(vec![
            line(
                0,
                "Remote 1",
                "The review is still pending after four days.",
            ),
            line(300, "You", "review is still pending after four"),
            // The same words much later are yours.
            line(30_000, "You", "The review is still pending, I see."),
        ]);
        let yours: Vec<&str> = out
            .iter()
            .filter(|s| s.speaker == "You")
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(yours, ["The review is still pending, I see."]);
    }

    #[test]
    fn short_identical_replies_are_kept() {
        let out = interleave(vec![
            line(
                0,
                "Remote 1",
                "The review is still pending after four days.",
            ),
            line(300, "You", "review is still pending after four"),
        ]);
        assert!(out.iter().all(|s| s.speaker != "You"));
        let out = interleave(vec![
            line(0, "Remote", "Oui."),
            line(900, "You", "Oui."),
            line(20_000, "Remote", "D'accord."),
            line(20_900, "You", "D'accord."),
        ]);
        assert_eq!(out.len(), 4);
    }
}
