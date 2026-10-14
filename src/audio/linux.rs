#![cfg(target_os = "linux")]
//! Audio capture through `parec`: one process per source, kept running for the
//! whole life of the app so the meters work before and after a recording too.
//! Déplacé depuis `src/audio.rs` SANS changement de logique (bootstrap Windows).

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::{CHANNELS, HISTORY, RATE};

/// 20 ms of s16le audio.
const CHUNK_BYTES: usize = (RATE / 50 * 2 * CHANNELS) as usize;
const FLOOR_DB: f64 = -60.0;

struct Inner {
    levels: VecDeque<f32>,
    file: Option<BufWriter<File>>,
    /// Identifies the recording for sync operations completed off the lock.
    recording: u64,
    /// While paused the meters keep running but nothing is written.
    paused: bool,
    /// First write/flush/sync failure step (`None` while writing fine):
    /// kept, surfaced by `stop_recording`, and polled from the UI thread to
    /// warn instead of claiming a clean recording. Set on the capture thread.
    write_error: Option<String>,
    // Active recording time excludes pauses; frame positions are relative to it.
    clock: Option<(Instant, u64)>,
    frames: u64,
}

/// Keeps only the first write failure, so the root cause survives later noise.
fn note_write_error(inner: &mut Inner, context: &str, message: String) {
    if inner.write_error.is_none() {
        inner.write_error = Some(format!("{context}: {message}"));
    }
}

fn note_sync_error(inner: &mut Inner, recording: u64, message: String) {
    if inner.recording == recording && inner.file.is_some() {
        note_write_error(inner, "sync", message);
    }
}

/// Fill missing time at a source restart, or through stop/pause.
/// Contiguous packets use sample counts, never their possibly delayed read time.
/// A fixed-size zero buffer bounds memory even after a long source outage.
fn pad_to_clock(inner: &mut Inner, now: Instant) {
    let Some((started, base)) = inner.clock else {
        return;
    };
    let elapsed = now.saturating_duration_since(started).as_nanos();
    let target = base.saturating_add((elapsed * u128::from(RATE) / 1_000_000_000) as u64);
    let zeroes = [0u8; CHUNK_BYTES];
    while inner.frames < target {
        let frames = (target - inner.frames).min(u64::from(RATE / 50));
        let Some(file) = inner.file.as_mut() else {
            return;
        };
        if let Err(e) = file.write_all(&zeroes[..frames as usize * CHANNELS as usize * 2]) {
            note_write_error(inner, "silence", e.to_string());
            return;
        }
        inner.frames += frames;
    }
}

fn capture_failed(shared: &Mutex<Inner>, device: &str, message: &str) {
    let mut inner = shared.lock().unwrap();
    if inner.file.is_some() {
        note_write_error(&mut inner, "capture", format!("{device}: {message}"));
    }
}

#[derive(Clone)]
pub struct Source {
    inner: Arc<Mutex<Inner>>,
}

impl Source {
    /// Starts capturing `device`, a PulseAudio source name such as `@DEFAULT_MONITOR@`.
    pub fn spawn(device: &'static str) -> Self {
        let inner = Arc::new(Mutex::new(Inner {
            levels: VecDeque::from(vec![0.0; HISTORY]),
            file: None,
            recording: 0,
            paused: false,
            write_error: None,
            clock: None,
            frames: 0,
        }));
        let shared = inner.clone();
        thread::spawn(move || {
            loop {
                capture(device, &shared);
                // parec exits when the device goes away; try again.
                thread::sleep(Duration::from_secs(1));
            }
        });
        Source { inner }
    }

    /// Tees the raw stream (s16le, RATE, CHANNELS) into `path` from now on.
    pub fn start_recording(&self, path: &Path) -> std::io::Result<()> {
        let file = BufWriter::new(File::create(path)?);
        let mut inner = self.inner.lock().unwrap();
        inner.file = Some(file);
        inner.recording = inner.recording.wrapping_add(1);
        inner.paused = false;
        inner.write_error = None;
        inner.frames = 0;
        inner.clock = Some((Instant::now(), 0));
        Ok(())
    }

    pub fn set_paused(&self, paused: bool) {
        let mut inner = self.inner.lock().unwrap();
        if inner.paused == paused {
            return;
        }
        if paused {
            pad_to_clock(&mut inner, Instant::now());
            inner.clock = None;
        } else if inner.file.is_some() {
            inner.clock = Some((Instant::now(), inner.frames));
        }
        inner.paused = paused;
    }

    /// Flushes and syncs the file, then reports the first write failure if
    /// any (kept from the capture thread). The flush/sync run OUTSIDE the
    /// lock so the meters never block on the disk.
    pub fn stop_recording(&self) -> std::io::Result<()> {
        let (write_error, file) = {
            let mut inner = self.inner.lock().unwrap();
            pad_to_clock(&mut inner, Instant::now());
            inner.clock = None;
            (inner.write_error.take(), inner.file.take())
        };
        let mut error = write_error;
        if let Some(mut file) = file {
            if let Err(e) = file.flush() {
                error = error.or(Some(format!("flush: {e}")));
            }
            if let Err(e) = file.get_ref().sync_data() {
                error = error.or(Some(format!("sync: {e}")));
            }
        }
        match error {
            Some(message) => Err(std::io::Error::other(message)),
            None => Ok(()),
        }
    }

    /// The first write/flush/sync failure step, if the recording degraded.
    /// `None` while writing fine. Polled from the UI thread to warn instead
    /// of claiming a clean recording.
    pub fn write_error(&self) -> Option<String> {
        self.inner.lock().unwrap().write_error.clone()
    }

    pub fn levels(&self) -> Vec<f32> {
        self.inner.lock().unwrap().levels.iter().copied().collect()
    }

    /// The loudest of the last `n` peaks, so a short burst is not missed by a slower reader.
    pub fn recent_peak(&self, n: usize) -> f32 {
        let inner = self.inner.lock().unwrap();
        inner
            .levels
            .iter()
            .rev()
            .take(n)
            .copied()
            .fold(0.0, f32::max)
    }
}

fn capture(device: &str, shared: &Mutex<Inner>) {
    let mut command = Command::new("parec");
    command.args([
        "--raw",
        "--format=s16le",
        &format!("--rate={RATE}"),
        &format!("--channels={CHANNELS}"),
        "--latency-msec=20",
        "-d",
        device,
    ]);
    capture_command(device, shared, &mut command);
}

fn capture_command(device: &str, shared: &Mutex<Inner>, command: &mut Command) {
    // Timestamp the source transition, not the arrival of its first packet:
    // spawn/read scheduling and pipe buffering are not capture interruptions.
    let started = Instant::now();
    let child = command.stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            capture_failed(shared, device, &error.to_string());
            return;
        }
    };
    let Some(mut stdout) = child.stdout.take() else {
        capture_failed(shared, device, "missing capture stream");
        let _ = child.kill();
        let _ = child.wait();
        return;
    };
    pad_to_clock(&mut shared.lock().unwrap(), started);
    let mut buf = vec![0u8; CHUNK_BYTES];
    // So a crash loses at most a second: flush every second, and push it to
    // the disk itself every half minute in case the machine goes down too.
    let mut chunks: u64 = 0;
    while stdout.read_exact(&mut buf).is_ok() {
        chunks += 1;
        let peak = buf
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs())
            .max()
            .unwrap_or(0) as f32
            / 32768.0;
        let mut inner = shared.lock().unwrap();
        inner.levels.pop_front();
        inner.levels.push_back(peak);
        // Paused, not recording, or already failed: skip the write but keep
        // parec open and the meters alive (`continue`, NEVER `return`).
        if inner.paused || inner.file.is_none() {
            continue;
        }
        if let Some(file) = inner.file.as_mut() {
            if let Err(e) = file.write_all(&buf) {
                note_write_error(&mut inner, "write", e.to_string());
                continue;
            }
            inner.frames += u64::from(RATE / 50);
        }
        if let Some(file) = inner.file.as_mut()
            && chunks.is_multiple_of(50)
            && let Err(e) = file.flush()
        {
            note_write_error(&mut inner, "flush", e.to_string());
        }
        // Sync disque toutes les 30 s, HORS verrou via un clone du File.
        let sync_clone = if chunks.is_multiple_of(1500) {
            inner
                .file
                .as_ref()
                .and_then(|file| file.get_ref().try_clone().ok())
        } else {
            None
        };
        let recording = inner.recording;
        drop(inner);
        if let Some(file) = sync_clone
            && let Err(e) = file.sync_data()
        {
            note_sync_error(&mut shared.lock().unwrap(), recording, e.to_string());
        }
    }
    capture_failed(
        shared,
        device,
        "parec stream ended or could not be read; retrying",
    );
    let _ = child.kill();
    let _ = child.wait();
}

/// Maps a linear peak to 0..1 on a -60 dB..0 dB scale.
pub fn to_meter(peak: f32) -> f64 {
    if peak <= 0.0 {
        return 0.0;
    }
    (1.0 - 20.0 * f64::from(peak).log10() / FLOOR_DB).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_late_sync_failure_cannot_degrade_the_next_recording() {
        let source = Source {
            inner: Arc::new(Mutex::new(Inner {
                levels: VecDeque::new(),
                file: None,
                recording: 0,
                paused: false,
                write_error: None,
                clock: None,
                frames: 0,
            })),
        };
        let path = std::env::temp_dir().join(format!("mr-sync-generation-{}", std::process::id()));
        source.start_recording(&path).unwrap();
        let old = source.inner.lock().unwrap().recording;
        source.stop_recording().unwrap();
        source.start_recording(&path).unwrap();
        let current = source.inner.lock().unwrap().recording;
        note_sync_error(&mut source.inner.lock().unwrap(), old, "old failure".into());
        assert!(source.write_error().is_none());
        note_sync_error(
            &mut source.inner.lock().unwrap(),
            current,
            "current failure".into(),
        );
        assert!(
            source
                .stop_recording()
                .unwrap_err()
                .to_string()
                .contains("current failure")
        );
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn recovered_audio_follows_gap_and_pause_adds_no_time() {
        let dir = std::env::temp_dir().join(format!("capture-timeline-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("audio.raw");
        let start = Instant::now();
        let inner = Inner {
            levels: VecDeque::new(),
            file: Some(BufWriter::new(File::create(&path).unwrap())),
            recording: 0,
            paused: false,
            write_error: None,
            clock: Some((start, 0)),
            frames: 0,
        };
        let source = Source {
            inner: Arc::new(Mutex::new(inner)),
        };
        {
            let mut inner = source.inner.lock().unwrap();
            let packet = vec![1u8; CHUNK_BYTES];
            inner.file.as_mut().unwrap().write_all(&packet).unwrap();
            inner.frames += u64::from(RATE / 50);
            // The source resumes at 1.0 s, before its next 20 ms packet.
            pad_to_clock(&mut inner, start + Duration::from_secs(1));
            inner.file.as_mut().unwrap().write_all(&packet).unwrap();
            inner.frames += u64::from(RATE / 50);
            inner.clock = None; // A paused interval contributes no missing samples.
            pad_to_clock(&mut inner, start + Duration::from_secs(10));
        }
        capture_failed(&source.inner, "fixture", "disconnected");
        assert!(source.write_error().unwrap().contains("disconnected"));
        assert!(source.stop_recording().is_err());
        let data = std::fs::read(&path).unwrap();
        let second = RATE as usize * CHANNELS as usize * 2;
        assert_eq!(data.len(), second + CHUNK_BYTES);
        assert!(data[..CHUNK_BYTES].iter().all(|b| *b == 1));
        assert!(data[CHUNK_BYTES..second].iter().all(|b| *b == 0));
        assert!(data[second..].iter().all(|b| *b == 1));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn process_exit_and_restart_keep_audio_after_the_silent_gap() {
        let dir = std::env::temp_dir().join(format!("capture-process-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("audio.raw");
        let source = Source {
            inner: Arc::new(Mutex::new(Inner {
                levels: VecDeque::from(vec![0.0; HISTORY]),
                file: None,
                recording: 0,
                paused: false,
                write_error: None,
                clock: None,
                frames: 0,
            })),
        };
        source.start_recording(&path).unwrap();
        let mut command = Command::new("sh");
        command.args([
            "-c",
            &format!("head -c {CHUNK_BYTES} /dev/zero | tr '\\000' '\\001'"),
        ]);
        capture_command("fixture", &source.inner, &mut command);
        assert!(source.write_error().unwrap().contains("capture"));
        thread::sleep(Duration::from_millis(150));
        capture_command("fixture", &source.inner, &mut command);
        assert!(source.stop_recording().is_err());
        let data = std::fs::read(&path).unwrap();
        let last_audio = data.iter().rposition(|byte| *byte == 1).unwrap() + 1;
        assert!(last_audio >= (RATE as usize * CHANNELS as usize * 2 * 130 / 1000));
        let first_audio = data.iter().position(|byte| *byte == 1).unwrap();
        let gap_start = first_audio + CHUNK_BYTES;
        let second_audio = last_audio - CHUNK_BYTES;
        assert!(second_audio - gap_start >= RATE as usize * CHANNELS as usize * 2 * 100 / 1000);
        assert!(data[gap_start..second_audio].iter().all(|byte| *byte == 0));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn failed_spawn_stop_fills_terminal_gap_and_excludes_pause() {
        let dir = std::env::temp_dir().join(format!("capture-terminal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("audio.raw");
        let source = Source {
            inner: Arc::new(Mutex::new(Inner {
                levels: VecDeque::from(vec![0.0; HISTORY]),
                file: None,
                recording: 0,
                paused: false,
                write_error: None,
                clock: None,
                frames: 0,
            })),
        };
        source.start_recording(&path).unwrap();
        capture_command(
            "fixture",
            &source.inner,
            &mut Command::new(dir.join("absent-parec")),
        );
        assert!(source.write_error().unwrap().contains("capture"));
        thread::sleep(Duration::from_millis(30));
        source.set_paused(true);
        let paused_frames = source.inner.lock().unwrap().frames;
        thread::sleep(Duration::from_millis(40));
        source.set_paused(false);
        thread::sleep(Duration::from_millis(30));
        let resumed = source.inner.lock().unwrap().clock.unwrap().0;
        let before_stop_frames =
            (resumed.elapsed().as_nanos() * u128::from(RATE) / 1_000_000_000) as u64;
        assert!(source.stop_recording().is_err());
        let after_stop_frames =
            (resumed.elapsed().as_nanos() * u128::from(RATE) / 1_000_000_000) as u64;
        let written_frames = std::fs::metadata(&path).unwrap().len() / u64::from(CHANNELS * 2);
        assert!(written_frames >= paused_frames + u64::from(RATE) * 20 / 1000);
        // The paused interval must not be reinserted at stop.
        assert!(written_frames >= paused_frames + before_stop_frames);
        assert!(written_frames <= paused_frames + after_stop_frames);
        assert!(std::fs::read(&path).unwrap().iter().all(|byte| *byte == 0));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn buffered_packets_do_not_create_an_internal_silent_gap() {
        let dir = std::env::temp_dir().join(format!("capture-backlog-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("audio.raw");
        let source = Source {
            inner: Arc::new(Mutex::new(Inner {
                levels: VecDeque::from(vec![0.0; HISTORY]),
                file: None,
                recording: 0,
                paused: false,
                write_error: None,
                clock: None,
                frames: 0,
            })),
        };
        source.start_recording(&path).unwrap();
        let started = source.inner.lock().unwrap().clock.unwrap().0;
        let mut command = Command::new("sh");
        // Five contiguous 20 ms packets become readable together after 100 ms,
        // as they can when the reader or producer buffers an uninterrupted stream.
        command.args([
            "-c",
            &format!(
                "sleep 0.1; head -c {} /dev/zero | tr '\\000' '\\001'",
                CHUNK_BYTES * 5
            ),
        ]);
        capture_command("buffered-fixture", &source.inner, &mut command);
        let wall_frames = (started.elapsed().as_nanos() * u128::from(RATE) / 1_000_000_000) as u64;
        let frames = source.inner.lock().unwrap().frames;
        assert!(
            frames <= wall_frames + u64::from(RATE / 50),
            "buffered audio inflated the track: frames={frames}, elapsed_frames={wall_frames}"
        );
        assert!(source.stop_recording().is_err()); // Deliberate fixture EOF is reported.
        let data = std::fs::read(&path).unwrap();
        let audio_start = data.iter().position(|byte| *byte == 1).unwrap();
        assert!(
            data[audio_start..audio_start + CHUNK_BYTES * 5]
                .iter()
                .all(|byte| *byte == 1)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
