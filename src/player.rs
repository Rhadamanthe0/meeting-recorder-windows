//! The player on the done page: a two-lane waveform of the meeting (you above
//! the line, the other side below it) with a playhead, click or drag to seek.
//!
//! Playback is `ffmpeg` decoding into `pacat`, not GStreamer: a stock Omarchy
//! has no GStreamer audio sink, while `pacat` comes with the same package as
//! the `parec` the recorder already records with. Pausing stops the pipeline
//! and playing starts it again at the position; a meeting saved as separate
//! files is mixed on the fly.
//!
//! On Windows the same play/pause/seek flow drives `rodio` instead: each
//! track plays through its own `Sink` on one shared `OutputStream` (mixed by
//! the OS, the `amix` equivalent), decoded by `ffmpeg.exe -ss` when present
//! and by rodio's built-in decoder otherwise.

use std::cell::{Cell, RefCell};
#[cfg(target_os = "linux")]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
#[cfg(target_os = "linux")]
use std::process::{Child, Command};
use std::rc::Rc;
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
use rodio::Source as RodioSource;
#[cfg(target_os = "windows")]
use rodio::{Decoder, OutputStream, Sink};
#[cfg(target_os = "windows")]
use std::fs::File;
#[cfg(target_os = "windows")]
use std::io::{BufReader, Read};
#[cfg(target_os = "windows")]
use std::process::ChildStdout;

use gtk::prelude::*;
use gtk::{gio, glib};

#[cfg(target_os = "windows")]
use crate::action_process::ActionProcess;
use crate::export;
use crate::platform::silent_command;

const BINS: usize = 1000;
const MIC_COLOR: (f64, f64, f64) = (0.21, 0.52, 0.89);
const SYSTEM_COLOR: (f64, f64, f64) = (0.90, 0.38, 0.0);

type PositionCallback = Rc<RefCell<Option<Box<dyn Fn(i64)>>>>;
type ErrorCallback = Rc<RefCell<Option<Box<dyn Fn()>>>>;

/// A running `ffmpeg | pacat` pipeline, stopped when dropped.
#[cfg(target_os = "linux")]
struct Playback {
    ffmpeg: Child,
    pacat: Child,
    started: Instant,
    from_us: i64,
}

#[cfg(target_os = "linux")]
impl Playback {
    fn start(files: &[PathBuf], from_us: i64) -> Option<Playback> {
        let at = format!("{:.3}", from_us as f64 / 1_000_000.0);
        let mut ffmpeg = Command::new(export::ffmpeg());
        ffmpeg.args(["-v", "error", "-nostdin"]);
        for file in files {
            ffmpeg.args(["-ss", &at, "-i"]).arg(file);
        }
        if files.len() > 1 {
            ffmpeg.args([
                "-filter_complex",
                &format!("amix=inputs={}:normalize=0", files.len()),
            ]);
        }
        ffmpeg
            .args(["-f", "s16le", "-ar", "48000", "-ac", "2", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut ffmpeg = die_with_parent(&mut ffmpeg).spawn().ok()?;
        let audio = ffmpeg.stdout.take()?;
        let pacat = die_with_parent(
            Command::new("pacat")
                .args([
                    "--playback",
                    "--raw",
                    "--format=s16le",
                    "--rate=48000",
                    "--channels=2",
                    "--latency-msec=80",
                    "--client-name=Meeting Recorder",
                ])
                .stdin(Stdio::from(audio))
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        )
        .spawn();
        match pacat {
            Ok(pacat) => Some(Playback {
                ffmpeg,
                pacat,
                started: Instant::now(),
                from_us,
            }),
            Err(_) => {
                let _ = ffmpeg.kill();
                let _ = ffmpeg.wait();
                None
            }
        }
    }

    fn position_us(&self) -> i64 {
        self.from_us + self.started.elapsed().as_micros() as i64
    }

    fn ended(&mut self) -> bool {
        matches!(self.pacat.try_wait(), Ok(Some(_)))
    }
}

#[cfg(target_os = "linux")]
impl Drop for Playback {
    fn drop(&mut self) {
        let _ = self.ffmpeg.kill();
        let _ = self.pacat.kill();
        let _ = self.ffmpeg.wait();
        let _ = self.pacat.wait();
    }
}

/// Makes the child get SIGTERM when the app goes away, even after a crash, so
/// the meeting never keeps playing on its own.
#[cfg(target_os = "linux")]
fn die_with_parent(command: &mut Command) -> &mut Command {
    // SAFETY: prctl is async-signal-safe and touches only the child's own state.
    unsafe {
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        })
    }
}

/// Length of an audio file in microseconds, from ffprobe.
#[cfg(target_os = "linux")]
fn probe_duration_us(path: &Path) -> i64 {
    crate::action_process::output(
        Command::new(export::ffprobe())
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "csv=p=0",
            ])
            .arg(path),
    )
    .ok()
    .and_then(|out| String::from_utf8(out.stdout).ok())
    .and_then(|text| text.trim().parse::<f64>().ok())
    .map(|secs| (secs * 1_000_000.0) as i64)
    .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Windows playback: rodio instead of `ffmpeg | pacat`.
// ---------------------------------------------------------------------------

/// The live sound on Windows: one `Sink` per track on a single shared
/// `OutputStream`, mixed by the OS (the `amix` equivalent). Dropping the
/// stream closes the device and stops every sink; position still comes from
/// `started`, like the Linux pipeline.
#[cfg(target_os = "windows")]
struct Playback {
    /// Kept alive for the whole playback; dropping it ends the sound.
    _stream: Option<OutputStream>,
    /// The CI sink advances through decoded samples without opening an endpoint.
    silent_output: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    sinks: Vec<Sink>,
    started: Instant,
    from_us: i64,
}

#[cfg(target_os = "windows")]
impl Playback {
    /// Opens the output device and plays already prepared `sources`.
    /// Runs on the UI thread: only fast calls remain here.
    fn assemble(sources: Vec<TrackSource>, from_us: i64) -> Option<Playback> {
        if sources.is_empty() {
            return None;
        }
        if cfg!(feature = "ci-audio") {
            let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
            let mut sinks = Vec::with_capacity(sources.len());
            for source in sources {
                let (sink, mut output) = Sink::new_idle();
                match source {
                    TrackSource::Ffmpeg(source) => sink.append(source),
                    TrackSource::File(source) => sink.append(source),
                }
                let active = running.clone();
                std::thread::spawn(move || {
                    while active.load(std::sync::atomic::Ordering::Relaxed) {
                        let count = output.sample_rate() as usize * output.channels() as usize / 50;
                        for _ in 0..count {
                            if output.next().is_none() {
                                return;
                            }
                        }
                        std::thread::sleep(Duration::from_millis(20));
                    }
                });
                sinks.push(sink);
            }
            return Some(Playback {
                _stream: None,
                silent_output: Some(running),
                sinks,
                started: Instant::now(),
                from_us,
            });
        }
        let (_stream, handle) = OutputStream::try_default().ok()?;
        let mut sinks = Vec::with_capacity(sources.len());
        for source in sources {
            let Ok(sink) = Sink::try_new(&handle) else {
                continue;
            };
            match source {
                TrackSource::Ffmpeg(source) => sink.append(source),
                TrackSource::File(source) => sink.append(source),
            }
            sinks.push(sink);
        }
        if sinks.is_empty() {
            return None;
        }
        Some(Playback {
            _stream: Some(_stream),
            silent_output: None,
            sinks,
            started: Instant::now(),
            from_us,
        })
    }

    fn position_us(&self) -> i64 {
        self.from_us + self.started.elapsed().as_micros() as i64
    }

    fn ended(&mut self) -> bool {
        self.sinks.iter().all(|sink| sink.empty())
    }
}

#[cfg(target_os = "windows")]
impl Drop for Playback {
    fn drop(&mut self) {
        if let Some(running) = &self.silent_output {
            running.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        // Stop the sinks first so the ffmpeg children owned by their sources
        // are reaped promptly then; dropping `_stream` closes the device.
        // Each ffmpeg source also owns a kill-on-close Job Object, so a
        // process crash cannot leave playback decoders running.
        for sink in &self.sinks {
            sink.stop();
        }
    }
}

/// One track ready to play, built off the UI thread.
///
/// `rodio::OutputStream` wraps a `cpal::Stream` which is explicitly `!Send`,
/// so the stream itself cannot cross threads. Everything before it can: the
/// `ffmpeg -version` probe, the `ffmpeg` decoders and the file decoders are
/// all `Send`. They are prepared in the background (`prepare_sources`), the
/// UI thread then only opens the device and appends them (`assemble`).
#[cfg(target_os = "windows")]
enum TrackSource {
    Ffmpeg(FfmpegPcm),
    File(rodio::source::SkipDuration<Decoder<BufReader<File>>>),
}

/// Builds every playable track's source: the blocking half of starting
/// playback (the `ffmpeg -version` probe, spawning the `ffmpeg` decoders,
/// opening and probing the files). Runs on a background thread; the result
/// is `Send` so it can travel back to the UI thread for `assemble`.
#[cfg(target_os = "windows")]
fn prepare_sources(files: &[PathBuf], from_us: i64) -> Vec<TrackSource> {
    let mut sources = Vec::with_capacity(files.len());
    if have_ffmpeg() {
        for file in files {
            if let Some(source) = FfmpegPcm::spawn(file, from_us) {
                sources.push(TrackSource::Ffmpeg(source));
            }
        }
    } else {
        for file in files {
            // One unreadable track must not silence the other.
            let Ok(opened) = File::open(file) else {
                continue;
            };
            let Ok(decoder) = Decoder::new(BufReader::new(opened)) else {
                continue;
            };
            sources.push(TrackSource::File(
                decoder.skip_duration(Duration::from_micros(from_us.max(0) as u64)),
            ));
        }
    }
    sources
}

/// Whether `ffmpeg` resolves on PATH (`ffmpeg.exe` on Windows).
/// When present it is the preferred decoder (covers Opus-in-Ogg and seeks
/// with `-ss`); otherwise rodio's built-in (symphonia) decoder reads the
/// files directly.
#[cfg(target_os = "windows")]
fn have_ffmpeg() -> bool {
    crate::action_process::status(
        silent_command(export::ffmpeg())
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    )
    .map(|status| status.success())
    .unwrap_or(false)
}

/// Raw s16le 48 kHz stereo frames streaming from
/// `ffmpeg -ss <at> -i <file> -f s16le -ar 48000 -ac 2 -`, so seeking stays
/// accurate without loading the meeting into memory. The child dies with the
/// source (end of playback or seek).
#[cfg(target_os = "windows")]
struct FfmpegPcm {
    pipe: BufReader<ChildStdout>,
    process: ActionProcess,
}

#[cfg(target_os = "windows")]
impl FfmpegPcm {
    fn spawn(file: &Path, from_us: i64) -> Option<Self> {
        let at = format!("{:.3}", from_us.max(0) as f64 / 1_000_000.0);
        let mut command = silent_command(export::ffmpeg());
        command
            .args(["-v", "error", "-nostdin", "-ss", &at, "-i"])
            .arg(file)
            .args(["-f", "s16le", "-ar", "48000", "-ac", "2", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut process = ActionProcess::spawn(&mut command).ok()?;
        let out = process.child.stdout.take()?;
        Some(FfmpegPcm {
            pipe: BufReader::new(out),
            process,
        })
    }
}

#[cfg(target_os = "windows")]
impl Iterator for FfmpegPcm {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        let mut frame = [0u8; 2];
        self.pipe.read_exact(&mut frame).ok()?;
        Some(i16::from_le_bytes(frame))
    }
}

#[cfg(target_os = "windows")]
impl rodio::Source for FfmpegPcm {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        2
    }

    fn sample_rate(&self) -> u32 {
        48_000
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(target_os = "windows")]
impl Drop for FfmpegPcm {
    fn drop(&mut self) {
        self.process.terminate();
    }
}

/// Length of an audio file in microseconds: `ffprobe`, then the `Duration:`
/// line of `ffmpeg -i`, then a size-based estimate.
#[cfg(target_os = "windows")]
fn probe_duration_us(path: &Path) -> i64 {
    if let Some(us) = probe_with_ffprobe(path) {
        return us;
    }
    if let Some(us) = probe_with_ffmpeg(path) {
        return us;
    }
    estimate_duration_us(path)
}

#[cfg(target_os = "windows")]
fn probe_with_ffprobe(path: &Path) -> Option<i64> {
    crate::action_process::output(
        silent_command(export::ffprobe())
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "csv=p=0",
            ])
            .arg(path),
    )
    .ok()
    .and_then(|out| String::from_utf8(out.stdout).ok())
    .and_then(|text| text.trim().parse::<f64>().ok())
    .map(|secs| (secs * 1_000_000.0) as i64)
}

#[cfg(target_os = "windows")]
fn probe_with_ffmpeg(path: &Path) -> Option<i64> {
    let out =
        crate::action_process::output(silent_command(export::ffmpeg()).arg("-i").arg(path)).ok()?;
    parse_ffmpeg_duration(&out.stderr)
}

/// Reads `Duration: 00:05:12.34` from `ffmpeg -i`'s stderr.
/// Returns `None` for `Duration: N/A` or unparsable output.
#[cfg(target_os = "windows")]
fn parse_ffmpeg_duration(stderr: &[u8]) -> Option<i64> {
    const PREFIX: &str = "Duration: ";
    let text = String::from_utf8_lossy(stderr);
    let rest = text.find(PREFIX).map(|at| &text[at + PREFIX.len()..])?;
    let token = rest.split([',', ' ']).next()?;
    let mut parts = token.split(':');
    let hours: f64 = parts.next()?.parse().ok()?;
    let minutes: f64 = parts.next()?.parse().ok()?;
    let secs: f64 = parts.next()?.parse().ok()?;
    Some(((hours * 3600.0 + minutes * 60.0 + secs) * 1_000_000.0) as i64)
}

/// Rough duration from the file size when neither `ffprobe` nor `ffmpeg`
/// answers. Voice Opus as recorded here lands around 32 kbit/s; only the
/// waveform scale and seeking use this, the probes above give exact lengths.
#[cfg(target_os = "windows")]
fn estimate_duration_us(path: &Path) -> i64 {
    const NOMINAL_BPS: u64 = 32_000;
    match std::fs::metadata(path) {
        Ok(meta) => (meta.len().saturating_mul(8_000_000) / NOMINAL_BPS) as i64,
        Err(_) => 0,
    }
}

#[derive(Default)]
struct State {
    files: Vec<PathBuf>,
    duration_us: i64,
    /// Where playback starts from next, while paused.
    paused_at_us: i64,
    playback: Option<Playback>,
    /// A background `prepare_sources` in flight (Windows only). Bumped on
    /// every play/pause/seek/unload so a late result is dropped instead of
    /// playing from a stale position.
    #[cfg(target_os = "windows")]
    starting: bool,
    #[cfg(target_os = "windows")]
    start_seq: u64,
    /// Peaks per bin for the mic and the computer track, 0..1.
    peaks: Option<(Vec<f32>, Vec<f32>)>,
    /// Bumped on every load, so a slow waveform for an old meeting is dropped.
    generation: u64,
    /// Chapter starts in ms with their titles, drawn as markers.
    chapters: Vec<(i64, String)>,
}

#[derive(Clone)]
pub struct Player {
    root: gtk::Box,
    button: gtk::Button,
    wave: gtk::DrawingArea,
    time: gtk::Label,
    state: Rc<RefCell<State>>,
    /// Called with the position in ms while playing, for the transcript highlight.
    on_position: PositionCallback,
    /// Called once when playback fails to start (no device, no decodable
    /// track), so the UI can say so instead of silently flipping back to Play.
    on_error: ErrorCallback,
    ticking: Rc<Cell<bool>>,
}

impl Player {
    pub fn new() -> Self {
        let button = gtk::Button::builder()
            .icon_name("media-playback-start-symbolic")
            .tooltip_text("Play")
            .valign(gtk::Align::Center)
            .css_classes(["circular", "flat"])
            .build();
        let wave = gtk::DrawingArea::builder()
            .content_height(56)
            .hexpand(true)
            .build();
        wave.set_cursor_from_name(Some("pointer"));
        let time = gtk::Label::builder()
            .label("00:00")
            .css_classes(["numeric", "caption", "dim-label"])
            .valign(gtk::Align::Center)
            .build();
        let root = gtk::Box::builder()
            .spacing(10)
            .css_classes(["card", "player"])
            .build();

        root.append(&button);
        root.append(&wave);
        root.append(&time);

        let player = Player {
            root,
            button,
            wave,
            time,
            state: Rc::default(),
            on_position: Rc::default(),
            on_error: Rc::default(),
            ticking: Rc::default(),
        };

        let weak = player.downgrade();
        player.wave.set_draw_func(move |_, cr, width, height| {
            if let Some(this) = weak() {
                this.draw(cr, f64::from(width), f64::from(height));
            }
        });

        let weak = player.downgrade();
        player.button.connect_clicked(move |_| {
            if let Some(this) = weak() {
                this.toggle();
            }
        });

        // Click or drag anywhere on the waveform to seek.
        let drag = gtk::GestureDrag::new();
        let weak = player.downgrade();
        drag.connect_drag_begin(move |_, x, _| {
            if let Some(this) = weak() {
                this.seek_to_x(x);
            }
        });
        let weak = player.downgrade();
        drag.connect_drag_update(move |gesture, dx, _| {
            if let Some(this) = weak()
                && let Some((x, _)) = gesture.start_point()
            {
                this.seek_to_x(x + dx);
            }
        });
        player.wave.add_controller(drag);

        // Hovering a chapter marker shows its title.
        player.wave.set_has_tooltip(true);
        let weak = player.downgrade();
        player
            .wave
            .connect_query_tooltip(move |_, x, _, _, tooltip| {
                match weak().and_then(|this| this.chapter_near(f64::from(x))) {
                    Some(title) => {
                        tooltip.set_text(Some(&title));
                        true
                    }
                    None => false,
                }
            });

        player
    }

    // Widget callbacks must not own the player containing those widgets:
    // that cycle used to retain every closed meeting and its audio sources.
    fn downgrade(&self) -> impl Fn() -> Option<Self> + Clone + use<> {
        let (root, button, wave, time) = (
            self.root.downgrade(),
            self.button.downgrade(),
            self.wave.downgrade(),
            self.time.downgrade(),
        );
        let (state, on_position, on_error, ticking) = (
            Rc::downgrade(&self.state),
            Rc::downgrade(&self.on_position),
            Rc::downgrade(&self.on_error),
            Rc::downgrade(&self.ticking),
        );
        move || {
            Some(Self {
                root: root.upgrade()?,
                button: button.upgrade()?,
                wave: wave.upgrade()?,
                time: time.upgrade()?,
                state: state.upgrade()?,
                on_position: on_position.upgrade()?,
                on_error: on_error.upgrade()?,
                ticking: ticking.upgrade()?,
            })
        }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn connect_position(&self, callback: impl Fn(i64) + 'static) {
        *self.on_position.borrow_mut() = Some(Box::new(callback));
    }

    /// Registers the callback run when playback fails to start, on the UI thread.
    pub fn connect_error(&self, callback: impl Fn() + 'static) {
        *self.on_error.borrow_mut() = Some(Box::new(callback));
    }

    fn playback_failed(&self) {
        if let Some(callback) = self.on_error.borrow().as_ref() {
            callback();
        }
    }

    /// Loads the meeting in `dir`: its audio for playback, its tracks for the waveform.
    pub fn load(&self, dir: &Path) {
        self.unload();
        let playable: Vec<PathBuf> = if dir.join("audio.ogg").is_file() {
            vec![dir.join("audio.ogg")]
        } else {
            ["mic.ogg", "computer.ogg"]
                .iter()
                .map(|f| dir.join(f))
                .filter(|p| p.is_file())
                .collect()
        };
        let generation = {
            let mut state = self.state.borrow_mut();
            // Probed below in `spawn_blocking`: `ffprobe`/`ffmpeg` can take
            // seconds on a cold disk, and this runs on the UI thread.
            state.duration_us = 0;
            state.files = playable.clone();
            state.paused_at_us = 0;
            state.generation += 1;
            state.generation
        };
        self.root.set_visible(!playable.is_empty());

        // The kept tracks give each side its own lane; without them, one lane.
        let (mic, computer) = export::tracks(dir);
        let sources = if mic.is_file() && computer.is_file() {
            (mic, Some(computer))
        } else if let Some(first) = playable.first() {
            (first.clone(), None)
        } else {
            return;
        };
        let this = self.clone();
        glib::spawn_future_local(async move {
            let probed = gio::spawn_blocking(move || {
                let duration = playable
                    .iter()
                    .map(|p| probe_duration_us(p))
                    .max()
                    .unwrap_or(0);
                let mic = peaks(&sources.0);
                let computer = sources.1.as_deref().map(peaks);
                (duration, mic, computer)
            })
            .await;
            if let Ok((duration, mic, computer)) = probed {
                let mut state = this.state.borrow_mut();
                if state.generation == generation {
                    state.duration_us = duration;
                    if let Some(mic) = mic {
                        let computer = computer.flatten().unwrap_or_else(|| vec![0.0; BINS]);
                        state.peaks = Some((mic, computer));
                    }
                }
            }
            this.refresh();
            this.wave.queue_draw();
        });
        self.refresh();
    }

    /// Shows chapter markers on the waveform; empty removes them.
    pub fn set_chapters(&self, chapters: Vec<(i64, String)>) {
        self.state.borrow_mut().chapters = chapters;
        self.wave.queue_draw();
    }

    fn chapter_near(&self, x: f64) -> Option<String> {
        let duration = self.duration_us();
        if duration <= 0 {
            return None;
        }
        let width = f64::from(self.wave.width()).max(1.0);
        self.state
            .borrow()
            .chapters
            .iter()
            .map(|(ms, title)| (*ms as f64 * 1000.0 / duration as f64 * width, title))
            .filter(|(at, _)| (at - x).abs() <= 6.0)
            .min_by(|a, b| (a.0 - x).abs().total_cmp(&(b.0 - x).abs()))
            .map(|(_, title)| title.clone())
    }

    /// Stops playback and forgets the meeting.
    pub fn unload(&self) {
        let mut state = self.state.borrow_mut();
        // Invalidates a `load` probe still in flight so its late duration or
        // peaks cannot land on the now-empty player.
        state.generation += 1;
        state.playback = None;
        #[cfg(target_os = "windows")]
        {
            state.start_seq += 1;
            state.starting = false;
        }
        state.files.clear();
        state.duration_us = 0;
        state.paused_at_us = 0;
        state.peaks = None;
        state.chapters.clear();
        drop(state);
        #[cfg(target_os = "windows")]
        {
            self.button.set_sensitive(true);
        }
        self.set_playing_icon(false);
        self.wave.queue_draw();
    }

    fn duration_us(&self) -> i64 {
        self.state.borrow().duration_us
    }

    fn position_us(&self) -> i64 {
        let state = self.state.borrow();
        match &state.playback {
            Some(playback) => playback.position_us().min(state.duration_us),
            None => state.paused_at_us,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.state.borrow().playback.is_some()
    }

    fn toggle(&self) {
        if self.is_playing() {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn play(&self) {
        #[cfg(target_os = "windows")]
        {
            // Tout le travail bloquant (sonde `ffmpeg -version`, spawn des
            // décodeurs `ffmpeg`, ouverture et sondage des fichiers) part en
            // fond : le clic rend la main immédiatement, même si un spawn
            // rame. Seul l'assemblage (`OutputStream` + sinks, `!Send`,
            // donc non transférable) revient sur le thread UI.
            let (files, from_us, seq) = {
                let mut state = self.state.borrow_mut();
                if state.files.is_empty() {
                    return;
                }
                if state.paused_at_us >= state.duration_us {
                    state.paused_at_us = 0;
                }
                state.playback = None;
                state.start_seq += 1;
                state.starting = true;
                (state.files.clone(), state.paused_at_us, state.start_seq)
            };
            self.button.set_sensitive(false);
            self.button.set_tooltip_text(Some("Loading…"));
            let this = self.clone();
            glib::spawn_future_local(async move {
                let sources = gio::spawn_blocking(move || prepare_sources(&files, from_us))
                    .await
                    .unwrap_or_else(|_| Vec::new());
                this.finish_start(seq, from_us, sources);
            });
        }
        #[cfg(not(target_os = "windows"))]
        {
            let started = {
                let mut state = self.state.borrow_mut();
                if state.files.is_empty() {
                    return;
                }
                if state.paused_at_us >= state.duration_us {
                    state.paused_at_us = 0;
                }
                state.playback = None;
                state.playback = Playback::start(&state.files, state.paused_at_us);
                state.playback.is_some()
            };
            self.set_playing_icon(started);
            if started {
                self.start_ticking();
            } else {
                self.playback_failed();
            }
        }
    }

    /// Back on the UI thread with the prepared sources: open the device and
    /// play, unless a pause/seek/unload superseded this start meanwhile (its
    /// sources are then dropped off the UI thread, killing just-spawned
    /// `ffmpeg` at once: `FfmpegPcm::drop` does `kill` + a blocking `wait`).
    #[cfg(target_os = "windows")]
    fn finish_start(&self, seq: u64, from_us: i64, sources: Vec<TrackSource>) {
        {
            let state = self.state.borrow();
            if state.start_seq != seq {
                drop(state);
                // Stale start: reap off the UI thread, never block GTK.
                gio::spawn_blocking(move || drop(sources));
                return;
            }
        }
        let started = {
            let mut state = self.state.borrow_mut();
            state.starting = false;
            state.playback = Playback::assemble(sources, from_us);
            state.playback.is_some()
        };
        self.button.set_sensitive(true);
        self.set_playing_icon(started);
        if started {
            self.start_ticking();
        } else {
            self.refresh();
            self.playback_failed();
        }
    }

    pub fn pause(&self) {
        let position = self.position_us();
        {
            let mut state = self.state.borrow_mut();
            state.playback = None;
            state.paused_at_us = position;
            #[cfg(target_os = "windows")]
            {
                // Drops any late background start instead of playing it.
                state.start_seq += 1;
                state.starting = false;
            }
        }
        #[cfg(target_os = "windows")]
        {
            self.button.set_sensitive(true);
        }
        self.set_playing_icon(false);
        self.refresh();
    }

    /// Jumps to `ms` and plays from there, for a click on the transcript.
    pub fn play_from(&self, ms: i64) {
        self.state.borrow_mut().paused_at_us = ms.saturating_mul(1000).max(0);
        self.play();
    }

    fn seek(&self, us: i64) {
        let us = us.clamp(0, self.duration_us().max(0));
        #[cfg(target_os = "windows")]
        let playing = self.is_playing() || self.state.borrow().starting;
        #[cfg(not(target_os = "windows"))]
        let playing = self.is_playing();
        self.state.borrow_mut().paused_at_us = us;
        if playing {
            self.play();
        }
        self.refresh();
    }

    fn seek_to_x(&self, x: f64) {
        let width = f64::from(self.wave.width()).max(1.0);
        let fraction = (x / width).clamp(0.0, 1.0);
        self.seek((self.duration_us() as f64 * fraction) as i64);
    }

    fn set_playing_icon(&self, playing: bool) {
        self.button.set_icon_name(if playing {
            "media-playback-pause-symbolic"
        } else {
            "media-playback-start-symbolic"
        });
        self.button
            .set_tooltip_text(Some(if playing { "Pause" } else { "Play" }));
    }

    fn start_ticking(&self) {
        if self.ticking.replace(true) {
            return;
        }
        let this = self.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let ended = this
                .state
                .borrow_mut()
                .playback
                .as_mut()
                .is_some_and(Playback::ended);
            if ended {
                this.pause();
                this.state.borrow_mut().paused_at_us = 0;
            }
            this.refresh();
            if this.is_playing() {
                glib::ControlFlow::Continue
            } else {
                this.ticking.set(false);
                glib::ControlFlow::Break
            }
        });
    }

    fn refresh(&self) {
        let (position, duration) = (self.position_us(), self.duration_us());
        self.time.set_label(&format!(
            "{} / {}",
            clock(position / 1_000_000),
            clock(duration / 1_000_000)
        ));
        self.wave.queue_draw();
        if let Some(callback) = self.on_position.borrow().as_ref() {
            callback(position / 1000);
        }
    }

    fn draw(&self, cr: &gtk::cairo::Context, width: f64, height: f64) {
        let state = self.state.borrow();
        let duration = self.duration_us();
        let played = if duration > 0 {
            self.position_us() as f64 / duration as f64
        } else {
            0.0
        };
        let mid = height / 2.0;
        let step = width / BINS as f64;
        let bar = (step * 0.8).max(1.0);
        let head = played * width;

        match state.peaks.as_ref() {
            Some((mic, computer)) => {
                for i in 0..BINS {
                    let x = i as f64 * step;
                    let alpha = if x <= head { 1.0 } else { 0.38 };
                    let up = (f64::from(mic[i]) * (mid - 2.0)).max(0.5);
                    let down = (f64::from(computer[i]) * (mid - 2.0)).max(0.5);
                    let (r, g, b) = crate::theme::color("blue", MIC_COLOR);
                    cr.set_source_rgba(r, g, b, alpha);
                    cr.rectangle(x, mid - up, bar, up);
                    let _ = cr.fill();
                    let (r, g, b) = crate::theme::color("orange", SYSTEM_COLOR);
                    cr.set_source_rgba(r, g, b, alpha);
                    cr.rectangle(x, mid, bar, down);
                    let _ = cr.fill();
                }
            }
            None => {
                cr.set_source_rgba(0.5, 0.5, 0.5, 0.3);
                cr.rectangle(0.0, mid - 0.5, width, 1.0);
                let _ = cr.fill();
            }
        }
        // Markers and playhead in the text colour, so they show on light themes too.
        let ink = self.wave.color();
        let (ir, ig, ib) = (
            f64::from(ink.red()),
            f64::from(ink.green()),
            f64::from(ink.blue()),
        );
        if duration > 0 {
            // Chapter markers: a thin line with a small notch at the top.
            for (ms, _) in &state.chapters {
                let x = (*ms as f64 * 1000.0 / duration as f64 * width).round() + 0.5;
                cr.set_source_rgba(ir, ig, ib, 0.35);
                cr.rectangle(x - 0.5, 0.0, 1.0, height);
                let _ = cr.fill();
                cr.set_source_rgba(ir, ig, ib, 0.85);
                cr.move_to(x - 4.0, 0.0);
                cr.line_to(x + 4.0, 0.0);
                cr.line_to(x, 5.0);
                cr.close_path();
                let _ = cr.fill();
            }
            cr.set_source_rgba(ir, ig, ib, 0.9);
            cr.rectangle(head.round() - 0.5, 0.0, 1.5, height);
            let _ = cr.fill();
        }
    }
}

/// Decodes `path` at a low rate and keeps the loudest sample per bin, scaled
/// to 0..1 with a gentle curve so quiet speech still shows.
fn peaks(path: &Path) -> Option<Vec<f32>> {
    let output = crate::action_process::output(
        silent_command(export::ffmpeg())
            .args(["-v", "error", "-i"])
            .arg(path)
            .args(["-ac", "1", "-ar", "4000", "-f", "s16le", "-"]),
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let samples: Vec<f32> = output
        .stdout
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| f32::from(i16::from_le_bytes(*b)).abs() / 32768.0)
        .collect();
    if samples.is_empty() {
        return Some(vec![0.0; BINS]);
    }
    let per_bin = samples.len().div_ceil(BINS);
    let mut bins: Vec<f32> = samples
        .chunks(per_bin)
        .map(|chunk| chunk.iter().copied().fold(0.0, f32::max))
        .collect();
    bins.resize(BINS, 0.0);
    let loudest = bins.iter().copied().fold(0.0, f32::max).max(1e-4);
    Some(bins.into_iter().map(|v| (v / loudest).sqrt()).collect())
}

fn clock(secs: i64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

#[cfg(all(test, target_os = "windows"))]
mod windows_tests {
    use super::*;

    #[cfg(feature = "ci-audio")]
    #[test]
    fn synthetic_output_decodes_and_finishes_without_opening_hardware() {
        let dir = std::env::temp_dir().join(format!("mr-ci-playback-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");
        let result = crate::action_process::status(
            silent_command(export::ffmpeg())
                .args([
                    "-nostdin",
                    "-v",
                    "error",
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=660:duration=0.2",
                ])
                .arg(&path),
        )
        .unwrap();
        assert!(result.success());
        let sources = prepare_sources(&[path.clone(), path], 0);
        let mut playback = Playback::assemble(sources, 0).expect("synthetic output opens");
        assert!(
            playback._stream.is_none(),
            "a hardware output stream was opened"
        );
        assert_eq!(playback.sinks.len(), 2, "tracks must play concurrently");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !playback.ended() {
            assert!(
                Instant::now() < deadline,
                "synthetic playback did not finish"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(playback);
        while let Err(error) = std::fs::remove_dir_all(&dir) {
            assert!(
                Instant::now() < deadline,
                "decoder retained its input: {error}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn ffmpeg_duration_line_parses() {
        let stderr = b"Input #0, ogg, from 'mic.ogg':\n  Duration: 00:05:12.34, start: 0.000000, bitrate: 32 kb/s\n";
        let us = parse_ffmpeg_duration(stderr).expect("parses the Duration line");
        // 5 min 12.34 s, to the microsecond, within float rounding.
        assert!((us - 312_340_000).abs() < 1_000);
    }

    #[test]
    fn ffmpeg_unknown_duration_falls_through() {
        assert_eq!(
            parse_ffmpeg_duration(b"  Duration: N/A, bitrate: N/A\n"),
            None
        );
        assert_eq!(parse_ffmpeg_duration(b"no duration here\n"), None);
    }
}
