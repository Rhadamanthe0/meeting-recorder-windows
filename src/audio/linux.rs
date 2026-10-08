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
use std::time::Duration;

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
        Ok(())
    }

    pub fn set_paused(&self, paused: bool) {
        self.inner.lock().unwrap().paused = paused;
    }

    /// Flushes and syncs the file, then reports the first write failure if
    /// any (kept from the capture thread). The flush/sync run OUTSIDE the
    /// lock so the meters never block on the disk.
    pub fn stop_recording(&self) -> std::io::Result<()> {
        let (write_error, file) = {
            let mut inner = self.inner.lock().unwrap();
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
    let Ok(mut child) = Command::new("parec")
        .args([
            "--raw",
            "--format=s16le",
            &format!("--rate={RATE}"),
            &format!("--channels={CHANNELS}"),
            "--latency-msec=20",
            "-d",
            device,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return;
    };
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
            if chunks.is_multiple_of(50)
                && let Err(e) = file.flush()
            {
                note_write_error(&mut inner, "flush", e.to_string());
            }
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
}
