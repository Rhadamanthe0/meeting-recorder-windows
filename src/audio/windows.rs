#![cfg(target_os = "windows")]
//! Capture WASAPI : micro (`@DEFAULT_SOURCE@`) + loopback système
//! (`@DEFAULT_MONITOR@`), via la crate `wasapi` 0.25 déclarée
//! (crate `windows` moderne, métadonnées embarquées, pas de winmd à fournir).
//!
//! Même contrat que `super::linux` : un thread par `Source`, chunks de
//! 20 ms en s16le 48 kHz stéréo, historique de 3 s de pics, écriture dans
//! un `BufWriter<File>` quand l'enregistrement est actif et non pausé.
//! En cas d'erreur ou de changement de périphérique, la boucle rouvre le
//! client après 1 s (comme parec qui quitte quand le device disparaît).

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::{CHANNELS, HISTORY, RATE};
use wasapi::{
    AudioCaptureClient, AudioClient, DeviceEnumerator, DeviceEventCallbacks, Direction, Handle,
    Role, SampleType, StreamMode, WaveFormat, initialize_mta,
};

/// 20 ms of s16le audio.
const CHUNK_BYTES: usize = (RATE / 50 * 2 * CHANNELS) as usize;
/// Seuil bas du vumètre, identique au backend Linux.
const FLOOR_DB: f64 = -60.0;
/// Période WASAPI de repli (en unités de 100 ns) si le device n'en donne pas.
const FALLBACK_PERIOD_HNS: i64 = 200_000; // 20 ms

struct Inner {
    levels: VecDeque<f32>,
    file: Option<BufWriter<File>>,
    /// Identifies the recording for sync operations completed off the lock.
    recording: u64,
    /// While paused the meters keep running but nothing is written.
    paused: bool,
    /// Last init failure step (`None` while capturing fine): polled by the UI
    /// to warn instead of recording silence. Set on the capture thread.
    error: Option<String>,
    /// First write/flush/sync failure (`None` while writing fine): kept,
    /// surfaced by `stop_recording`, and polled from the UI thread to warn
    /// instead of claiming a clean recording. Set on the capture thread.
    write_error: Option<String>,
    /// Session clock across reopens: when the last chunk was emitted. Used to
    /// fill the gap with silence so the timeline survives device changes.
    last_emit: Option<Instant>,
    /// The endpoint actually being captured (item 7): filled at open time.
    endpoint: Option<CapturedEndpoint>,
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

/// The endpoint actually captured (item 7), shown in the UI meter tooltips.
#[derive(Clone, Debug)]
pub struct CapturedEndpoint {
    pub id: String,
    pub friendlyname: String,
    pub direction: String,
    pub role: String,
    pub state: String,
}

/// Which endpoint a `Source` captures (item 7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Follows the WASAPI default for (`dir`, `role`): reopens when that
    /// default changes or when the currently captured endpoint moves.
    FollowDefault { dir: Direction, role: Role },
    /// Pinned endpoint (stable id from the settings): reopens only on it.
    Fixed { id: String },
}

/// The capture direction for a `Source::spawn` device name:
/// `@DEFAULT_MONITOR@` loops back the default render endpoint, anything else
/// (`@DEFAULT_SOURCE@` included) captures the default microphone.
fn expected_dir(device: &str) -> Direction {
    if device == "@DEFAULT_MONITOR@" {
        Direction::Render
    } else {
        Direction::Capture
    }
}

/// Pure helper: a pinned id (stable, never the friendly name) wins over the
/// default; an empty/missing pin follows the `Console` default.
fn target_for_device(device: &str, pinned: Option<&str>) -> Target {
    match pinned.filter(|id| !id.trim().is_empty()) {
        Some(id) => Target::Fixed { id: id.to_owned() },
        None => Target::FollowDefault {
            dir: expected_dir(device),
            role: Role::Console,
        },
    }
}

/// Endpoint notification relayed by `mpsc` from the WASAPI callbacks (which
/// must never call back into the `DeviceEnumerator`).
#[derive(Clone, Debug)]
enum EndpointNotice {
    DefaultChanged { dir: Direction, role: Role },
    DeviceChanged(String),
}

/// Pure helper: does this notice concern our target?
/// - `FollowDefault` reopens only on its own default (`dir` + `role`) or on
///   the currently captured endpoint id;
/// - `Fixed` reopens only on its pinned id.
fn should_reopen(target: &Target, notice: &EndpointNotice, current_id: Option<&str>) -> bool {
    match (target, notice) {
        (
            Target::FollowDefault { dir, role },
            EndpointNotice::DefaultChanged { dir: d, role: r },
        ) => dir == d && role == r,
        (Target::FollowDefault { .. }, EndpointNotice::DeviceChanged(id)) => {
            current_id == Some(id.as_str())
        }
        (Target::Fixed { id }, EndpointNotice::DeviceChanged(changed)) => id == changed,
        (Target::Fixed { .. }, EndpointNotice::DefaultChanged { .. }) => false,
    }
}

#[derive(Clone)]
pub struct Source {
    inner: Arc<Mutex<Inner>>,
}

impl Source {
    /// Starts capturing `device` :
    /// - `@DEFAULT_MONITOR@` : loopback du rendu par défaut (audio système),
    /// - tout autre nom (`@DEFAULT_SOURCE@` inclus) : capture micro par défaut.
    pub fn spawn(device: &'static str) -> Self {
        let inner = Arc::new(Mutex::new(Inner {
            levels: VecDeque::from(vec![0.0; HISTORY]),
            file: None,
            recording: 0,
            paused: false,
            error: None,
            write_error: None,
            last_emit: None,
            endpoint: None,
        }));
        let shared = inner.clone();
        thread::spawn(move || {
            if cfg!(feature = "ci-audio") {
                let weak = Arc::downgrade(&shared);
                drop(shared);
                synthetic_loop(device, weak);
            } else {
                endpoint_loop(device, &shared);
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

    pub fn stop_recording(&self) -> std::io::Result<()> {
        let (write_error, file) = {
            let mut inner = self.inner.lock().unwrap();
            (inner.write_error.take(), inner.file.take())
        };
        // Flush + sync HORS verrou (guards droppés) : les vumètres ne
        // bloquent jamais sur le disque.
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
    /// `None` while writing fine. Clone sous mutex, jamais de bloc.
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

    /// The last capture init failure step, if the device refuses to start
    /// (exotic mix format, no default device, …). `None` while capturing
    /// fine. Polled from the UI thread to warn instead of recording silence.
    pub fn init_error(&self) -> Option<String> {
        self.inner.lock().unwrap().error.clone()
    }

    /// The endpoint actually captured (item 7). `None` before the first open.
    /// Clone sous mutex, jamais de bloc.
    pub fn endpoint(&self) -> Option<CapturedEndpoint> {
        self.inner.lock().unwrap().endpoint.clone()
    }
}

/// A fake endpoint for CI. Its samples are generated here, never read from
/// WASAPI, a default device, saved settings, or an environment-supplied device.
fn synthetic_loop(device: &str, weak: std::sync::Weak<Mutex<Inner>>) {
    let frequency = if device == "@DEFAULT_MONITOR@" {
        880.0
    } else {
        440.0
    };
    let Some(shared) = weak.upgrade() else { return };
    shared.lock().unwrap().endpoint = Some(CapturedEndpoint {
        id: format!("synthetic:{}", tag(device)),
        friendlyname: format!("Synthetic {frequency} Hz"),
        direction: expected_dir(device).to_string(),
        role: "test".into(),
        state: "active".into(),
    });
    drop(shared);
    let mut converter = Converter {
        desc: MixDesc {
            rate: 44_100,
            channels: 1,
            channel_mask: 0x4, // SPEAKER_FRONT_CENTER (mono).
            kind: SampleKind::F32,
            blockalign: 4,
            valid: true,
        },
        pos: 0.0,
    };
    let mut frame = 0u64;
    let mut converted = Vec::new();
    let mut chunks = 0;
    while let Some(shared) = weak.upgrade() {
        let packet: Vec<u8> = (0..882)
            .flat_map(|_| {
                let sample = (std::f64::consts::TAU * frequency * frame as f64 / 44_100.0).sin()
                    as f32
                    * 0.25;
                frame += 1;
                sample.to_le_bytes()
            })
            .collect();
        converter.push_packet(&packet, &mut converted);
        let ready = converted.len() / CHUNK_BYTES * CHUNK_BYTES;
        for chunk in converted[..ready].as_chunks::<CHUNK_BYTES>().0 {
            push_chunk(&shared, chunk, &mut chunks);
        }
        converted.drain(..ready);
        drop(shared);
        thread::sleep(Duration::from_millis(20));
    }
}

/// Tag court pour le log fichier (`[mic|pc]`).
fn tag(device: &str) -> &'static str {
    if device == "@DEFAULT_MONITOR@" {
        "pc"
    } else {
        "mic"
    }
}

/// Heure HH:MM:SS (UTC via `SystemTime`, sans dépendance supplémentaire).
fn now_hms() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() % 86_400)
        .unwrap_or(0);
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Log fichier uniquement (`audio-debug.log` sous [`crate::platform::data_dir`]) ;
/// jamais de console. Toute erreur est ignorée pour ne pas changer le
/// comportement audio. Capé à ~1 Mo : au-delà, le fichier est tronqué avec
/// une ligne `log rotated` au lieu de grandir sans fin (stats toutes les 5 s).
const MAX_LOG_BYTES: u64 = 1024 * 1024;

fn debug_log(device: &str, msg: &str) {
    let line = format!("[{}] [{}] {}\n", now_hms(), tag(device), msg);
    let path = crate::platform::data_dir()
        .join(crate::APP_NAME)
        .join("audio-debug.log");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > MAX_LOG_BYTES {
        let _ = std::fs::write(&path, format!("[{}] log rotated\n", now_hms()));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Nom court du layout natif pour le log MixDesc.
fn kind_name(kind: SampleKind) -> &'static str {
    match kind {
        SampleKind::F32 => "f32",
        SampleKind::I16 => "i16",
        SampleKind::I24 => "i24",
        SampleKind::I32 => "i32",
    }
}

/// Échec d'init : une ligne avec l'étape + une ligne « reopen », puis retour
/// (la boucle `spawn` dort 1 s et rouvre, comportement inchangé). L'étape est
/// aussi mémorisée dans `Inner::error` pour que l'UI prévienne au lieu
/// d'enregistrer du silence ; effacée au prochain démarrage réussi.
fn init_failed(device: &str, step: &str, shared: &Mutex<Inner>) {
    debug_log(device, &format!("init failed step={step}"));
    debug_log(device, "reopen in 1s");
    shared.lock().unwrap().error = Some(step.to_owned());
}

/// Layout natif d'un échantillon du format mix du device.
#[derive(Clone, Copy)]
enum SampleKind {
    F32,
    I16,
    I24,
    I32,
}

/// Format mix WASAPI du device, vers lequel on convertit en logiciel.
#[derive(Clone, Copy)]
struct MixDesc {
    rate: u32,
    channels: usize,
    channel_mask: u32,
    kind: SampleKind,
    blockalign: usize,
    /// Faux pour un format exotique (ex. float 64 bits) : on émet des zéros.
    valid: bool,
}

impl MixDesc {
    fn from_mix(mix: &WaveFormat) -> Self {
        let rate = mix.get_samplespersec().max(1);
        let channels = (mix.get_nchannels() as usize).max(1);
        let mask = mix.get_dwchannelmask();
        let channel_mask = if mask.count_ones() as usize == channels {
            mask
        } else {
            wasapi::make_channelmasks(channels)
                .first()
                .copied()
                .unwrap_or(0)
        };
        let bits = mix.get_bitspersample();
        let mut blockalign = mix.get_blockalign() as usize;
        if blockalign == 0 {
            blockalign = channels * (bits as usize).max(8) / 8;
        }
        let (kind, valid) = match mix.get_subformat() {
            Ok(SampleType::Float) if bits == 32 => (SampleKind::F32, true),
            Ok(SampleType::Float) => (SampleKind::F32, false),
            Ok(SampleType::Int) => match bits {
                16 => (SampleKind::I16, true),
                24 => (SampleKind::I24, true),
                32 => (SampleKind::I32, true),
                _ => (SampleKind::I16, false),
            },
            // Sous-format inconnu : le mix partagé Windows est quasi
            // toujours du float32 ; à défaut on émet des zéros.
            Err(_) => match bits {
                16 => (SampleKind::I16, true),
                24 => (SampleKind::I24, true),
                _ => (SampleKind::F32, false),
            },
        };
        MixDesc {
            rate,
            channels,
            channel_mask,
            kind,
            blockalign,
            valid,
        }
    }
}

/// Lit le canal `channel` de la frame `frame` en f32 dans [-1.0, 1.0].
fn frame_channel_f32(raw: &[u8], frame: usize, channel: usize, desc: &MixDesc) -> f32 {
    let bps = match desc.kind {
        SampleKind::F32 | SampleKind::I32 => 4,
        SampleKind::I16 => 2,
        SampleKind::I24 => 3,
    };
    let off = frame.saturating_mul(desc.blockalign) + channel.saturating_mul(bps);
    let Some(s) = raw.get(off..off + bps) else {
        return 0.0;
    };
    match desc.kind {
        SampleKind::F32 => {
            let v = f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
            if v.is_finite() {
                v.clamp(-1.0, 1.0)
            } else {
                0.0
            }
        }
        SampleKind::I16 => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
        SampleKind::I24 => {
            let mut v = (s[0] as i32) | ((s[1] as i32) << 8) | ((s[2] as i32) << 16);
            if v & 0x80_00_00 != 0 {
                v |= 0xFF00_0000u32 as i32; // extension de signe
            }
            v as f32 / 8_388_608.0
        }
        SampleKind::I32 => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
    }
}

fn missed_chunks(elapsed: Duration) -> u64 {
    elapsed.as_millis() as u64 / 20
}

const MAX_PAD_BURST: u64 = 1500;

/// Émet `n` chunks de silence (zéros) : niveaux + horloge via `push_chunk`.
fn pad_silence(shared: &Mutex<Inner>, chunks: &mut u64, n: u64) {
    for _ in 0..n {
        push_chunk(shared, &[0u8; CHUNK_BYTES], chunks);
    }
}

/// Comble le gap depuis le dernier chunk émis (cappé à `MAX_PAD_BURST`) :
/// la timeline survit aux réveils tardifs et aux réouvertures. Initialise
/// l'horloge au premier appel (aucun chunk émis : aucun gap à combler).
fn pad_missed(device: &str, shared: &Mutex<Inner>, chunks: &mut u64) {
    // Copie hors verrou : le garde est droppé avant tout re-lock.
    let last = shared.lock().unwrap().last_emit;
    let elapsed = match last {
        Some(t) => t.elapsed(),
        None => {
            shared.lock().unwrap().last_emit = Some(Instant::now());
            Duration::ZERO
        }
    };
    let n = missed_chunks(elapsed).min(MAX_PAD_BURST);
    if n > 0 {
        debug_log(device, &format!("pad missed={n}"));
        pad_silence(shared, chunks, n);
    }
}

fn downmix_samples(channels: usize, mask: u32, sample: impl Fn(usize) -> f32) -> (f32, f32) {
    const CENTER: f32 = 0.707;
    let mut positions = (0..32).filter(|bit| mask & (1u32 << bit) != 0);
    let (mut left, mut right) = (0.0f32, 0.0f32);
    for channel in 0..channels {
        // WAVEFORMATEXTENSIBLE orders channels by increasing mask bit.
        // Missing/unnamed positions contribute to both sides rather than
        // silently losing a channel. Only an actual LFE is omitted.
        let (l, r) = match positions.next().unwrap_or(2) {
            0 => (1.0, 0.0), // Front left.
            1 => (0.0, 1.0), // Front right.
            3 => (0.0, 0.0), // LFE.
            4 | 6 | 9 | 12 | 15 => (CENTER, 0.0),
            5 | 7 | 10 | 14 | 17 => (0.0, CENTER),
            _ => (CENTER, CENTER), // Center/back-center/height/unknown.
        };
        let value = sample(channel);
        left += l * value;
        right += r * value;
    }
    let peak = left.abs().max(right.abs()).max(1.0);
    (
        (left / peak).clamp(-1.0, 1.0),
        (right / peak).clamp(-1.0, 1.0),
    )
}

/// Convertit des paquets natifs vers s16le 48 kHz stéréo (resample linéaire
/// simple, sans dépendance supplémentaire).
///
/// Mapping des canaux natifs selon le masque WAVEFORMATEXTENSIBLE :
/// - 1 canal : mono dupliqué sur L/R ;
/// - 2 canaux : stéréo direct (FL/FR) ;
/// - multicanal : downmix via [`downmix_samples`] (centre/surrounds à 0.707,
///   seul le LFE est ignoré, normalisation anti-saturation) ;
/// - masque absent/incohérent : layout standard WASAPI pour ce nombre de
///   canaux, puis contribution centrale pour les positions non nommées.
struct Converter {
    desc: MixDesc,
    /// Retard fractionnaire (en frames source) reporté au paquet suivant,
    /// pour ne pas dériver sur les enregistrements longs.
    pos: f64,
}

impl Converter {
    fn push_packet(&mut self, raw: &[u8], out: &mut Vec<u8>) {
        if !self.desc.valid || self.desc.blockalign == 0 {
            return;
        }
        let frames_in = raw.len() / self.desc.blockalign;
        if frames_in == 0 {
            return;
        }
        let step = self.desc.rate as f64 / RATE as f64;
        let channels = self.desc.channels;
        let mut n_out = 0usize;
        loop {
            let src = self.pos + n_out as f64 * step;
            let i0 = src.floor() as usize;
            if i0 >= frames_in {
                break;
            }
            let frac = (src - i0 as f64) as f32;
            let i1 = (i0 + 1).min(frames_in - 1);
            // Échantillon natif interpolé ; le masque identifie le LFE.
            let sample = |channel: usize| {
                let a = frame_channel_f32(raw, i0, channel, &self.desc);
                let b = frame_channel_f32(raw, i1, channel, &self.desc);
                (a + (b - a) * frac).clamp(-1.0, 1.0)
            };
            let (left, right) = if channels == 1 {
                // Mono dupliqué.
                let v = sample(0);
                (v, v)
            } else if channels == 2 {
                // Stéréo direct.
                (sample(0), sample(1))
            } else {
                downmix_samples(channels, self.desc.channel_mask, sample)
            };
            for v in [left, right] {
                let s = (v * 32767.0).round().clamp(-32768.0, 32767.0) as i16;
                out.extend_from_slice(&s.to_le_bytes());
            }
            n_out += 1;
            // Garde-fou : un paquet ne produit jamais plus de 10 s audio.
            if n_out > RATE as usize * 10 {
                break;
            }
        }
        self.pos = self.pos + n_out as f64 * step - frames_in as f64;
        if !self.pos.is_finite() || self.pos < 0.0 {
            self.pos = 0.0;
        }
    }
}

fn push_chunk(shared: &Mutex<Inner>, chunk: &[u8], chunks: &mut u64) {
    debug_assert_eq!(chunk.len(), CHUNK_BYTES);
    *chunks += 1;
    let peak = chunk
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs())
        .max()
        .unwrap_or(0) as f32
        / 32768.0;
    let now = Instant::now();
    // UN verrou court : niveaux + horloge session.
    let mut inner = shared.lock().unwrap();
    inner.levels.pop_front();
    inner.levels.push_back(peak);
    inner.last_emit = Some(now);
    // En pause : niveaux seuls, rien n'est écrit.
    if inner.paused || inner.file.is_none() {
        return;
    }
    if let Some(file) = inner.file.as_mut() {
        if let Err(e) = file.write_all(chunk) {
            note_write_error(&mut inner, "write", e.to_string());
            return;
        }
        // Comme sur Linux : au plus une seconde perdue en cas de crash,
        // sync disque toutes les 30 s.
        if chunks.is_multiple_of(50)
            && let Err(e) = file.flush()
        {
            note_write_error(&mut inner, "flush", e.to_string());
        }
    }
    // Sync disque HORS verrou via un clone du File.
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

/// Flux ouvert sur la cible : client WASAPI + capture + format.
struct OpenStream {
    client: AudioClient,
    capture: AudioCaptureClient,
    event: Option<Handle>,
    desc: MixDesc,
    current_id: Option<String>,
}

/// Un thread par `Source` (item 7) : UN `DeviceEnumerator` + UN
/// `register_notification_callback` vivants tant que le thread vit. Les
/// callbacks ne font que relayer par `mpsc` (JAMAIS d'appel enumerator
/// dedans : le système audio les exécute sur son propre thread) ; ce thread
/// re-résout la cible puis stoppe et ré-init le flux, avec debounce. Le gap
/// est conservé via `Inner::last_emit` (comblé en zéros à la réouverture).
fn endpoint_loop(device: &str, shared: &Mutex<Inner>) {
    // COM MTA pour ce thread (inutile mais inoffensif si déjà initialisé).
    let _ = initialize_mta();
    // La cible ne change pas pendant la vie du thread : pin stable en
    // settings (`Fixed`) ou suivi du default (`FollowDefault`).
    let slot = if device == "@DEFAULT_MONITOR@" {
        "system"
    } else {
        "mic"
    };
    let target = target_for_device(
        device,
        crate::settings::load_capture_device(slot).as_deref(),
    );
    let mut chunks: u64 = 0;
    // parec quitte quand le device disparaît ; on rouvre pareil :
    // l'enumerator est recréé si sa création échoue, sinon il vit pour tout
    // le thread.
    loop {
        let enumerator = match DeviceEnumerator::new() {
            Ok(enumerator) => enumerator,
            Err(_) => {
                init_failed(device, "enumerator", shared);
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        let (tx, rx) = mpsc::channel::<EndpointNotice>();
        let mut callbacks = DeviceEventCallbacks::new();
        let tx_default = tx.clone();
        callbacks.set_default_device_callback(move |dir, role, _| {
            let _ = tx_default.send(EndpointNotice::DefaultChanged { dir, role });
        });
        let tx_state = tx.clone();
        callbacks.set_device_state_callback(move |id, _| {
            let _ = tx_state.send(EndpointNotice::DeviceChanged(id));
        });
        let tx_added = tx.clone();
        callbacks.set_device_added_callback(move |id| {
            let _ = tx_added.send(EndpointNotice::DeviceChanged(id));
        });
        let tx_removed = tx;
        callbacks.set_device_removed_callback(move |id| {
            let _ = tx_removed.send(EndpointNotice::DeviceChanged(id));
        });
        // `_registration` garde les notifications actives jusqu'à la fin du
        // thread ; sans lui, plus de réouverture sur changement de device.
        let _registration = match enumerator.register_notification_callback(callbacks) {
            Ok(registration) => registration,
            Err(_) => {
                init_failed(device, "register-notifications", shared);
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        loop {
            match open_target(device, &enumerator, &target, shared, &mut chunks) {
                Some(open) => {
                    run_stream(device, &target, shared, &open, &rx, &mut chunks);
                    // Debounce : les notifications arrivent en rafales, on
                    // coalesce avant de re-résoudre.
                    thread::sleep(Duration::from_millis(250));
                    while rx.try_recv().is_ok() {}
                }
                None => {
                    // Échec de résolution (`init_failed` déjà renseigné ;
                    // `Fixed` supprimé → step explicite) : on garde la
                    // timeline via le gap `last_emit`, puis on réessaie.
                    pad_missed(device, shared, &mut chunks);
                    thread::sleep(Duration::from_secs(1));
                    while rx.try_recv().is_ok() {}
                }
            }
        }
    }
}

/// Résout la cible en endpoint WASAPI et ouvre le flux (item 7) :
/// `get_device(&id)` si pin, sinon `get_default_device_for_role` (`Console`
/// par défaut). Renseigne `Inner::endpoint` et comble le gap `last_emit`.
/// `None` après `init_failed` (`Fixed` supprimé → step explicite + pad).
fn open_target(
    device: &str,
    enumerator: &DeviceEnumerator,
    target: &Target,
    shared: &Mutex<Inner>,
    chunks: &mut u64,
) -> Option<OpenStream> {
    debug_log(device, &format!("start device={device}"));

    // NOTE wasapi 0.24 : plus de `get_default_device` libre ; on passe par
    // `DeviceEnumerator`.
    let (dev, direction, role) = match target {
        Target::Fixed { id } => match enumerator.get_device(id) {
            Ok(dev) => (dev, expected_dir(device).to_string(), "pinned".to_owned()),
            Err(_) => {
                init_failed(device, "device-removed", shared);
                return None;
            }
        },
        Target::FollowDefault { dir, role } => {
            match enumerator.get_default_device_for_role(dir, role) {
                Ok(dev) => (dev, dir.to_string(), role.to_string()),
                Err(_) => {
                    init_failed(device, "default-device", shared);
                    return None;
                }
            }
        }
    };
    // Nom convivial wasapi 0.24 (`get_friendlyname`), repli ID (`get_id`) ;
    // rien d'inventé : si les deux échouent, on logue « unknown ».
    match dev.get_friendlyname() {
        Ok(name) => debug_log(device, &format!("endpoint friendlyname={name}")),
        Err(_) => match dev.get_id() {
            Ok(id) => debug_log(device, &format!("endpoint id={id}")),
            Err(_) => debug_log(device, "endpoint unknown"),
        },
    }
    // Endpoint réellement capturé : id stable, nom convivial, direction,
    // rôle, état (`get_state`). Lu par l'UI pour les tooltips des vumètres.
    let id = dev.get_id().ok();
    let friendly = dev
        .get_friendlyname()
        .unwrap_or_else(|_| id.clone().unwrap_or_else(|| "unknown".to_owned()));
    let state = dev
        .get_state()
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "unknown".to_owned());
    let endpoint = CapturedEndpoint {
        id: id.clone().unwrap_or_default(),
        friendlyname: friendly,
        direction,
        role,
        state,
    };
    debug_log(
        device,
        &format!(
            "endpoint direction={} role={} state={}",
            endpoint.direction, endpoint.role, endpoint.state
        ),
    );
    shared.lock().unwrap().endpoint = Some(endpoint);
    let mut client = match dev.get_iaudioclient() {
        Ok(client) => client,
        Err(_) => {
            init_failed(device, "iaudioclient", shared);
            return None;
        }
    };
    // Format mix partagé : toujours accepté ; conversion logicielle derrière.
    let mix = match client.get_mixformat() {
        Ok(mix) => mix,
        Err(_) => {
            init_failed(device, "mixformat", shared);
            return None;
        }
    };
    let desc = MixDesc::from_mix(&mix);
    debug_log(
        device,
        &format!(
            "mix rate={} channels={} kind={} valid={} blockalign={}",
            desc.rate,
            desc.channels,
            kind_name(desc.kind),
            desc.valid,
            desc.blockalign
        ),
    );
    if !desc.valid || desc.blockalign == 0 || desc.channels == 0 {
        init_failed(device, "desc-invalid", shared);
        return None;
    }
    let period = match client.get_device_period() {
        Ok((def, _)) if def > 0 => def,
        _ => FALLBACK_PERIOD_HNS,
    };
    // Direction::Capture sur un device Render = loopback partagé
    // (AUDCLNT_STREAMFLAGS_LOOPBACK, cf. wasapi 0.24 `initialize_client`).
    // `autoconvert: true` garde le format mix accepté (SRC moteur audio).
    if client
        .initialize_client(
            &mix,
            &Direction::Capture,
            &StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: period,
            },
        )
        .is_err()
    {
        init_failed(device, "initialize-client", shared);
        return None;
    }
    let capture = match client.get_audiocaptureclient() {
        Ok(capture) => capture,
        Err(_) => {
            init_failed(device, "captureclient", shared);
            return None;
        }
    };
    let event = client.set_get_eventhandle().ok();
    if client.start_stream().is_err() {
        init_failed(device, "start-stream", shared);
        return None;
    }
    // Le flux tourne : un échec précédent est résorbé, l'UI n'a plus à prévenir.
    shared.lock().unwrap().error = None;
    // (Ré)ouverture : comble le gap depuis le dernier chunk émis (cappé) —
    // la timeline survit aux réouvertures.
    pad_missed(device, shared, chunks);

    Some(OpenStream {
        client,
        capture,
        event,
        desc,
        current_id: id,
    })
}

/// Boucle de lecture d'un flux ouvert : draine les paquets, convertit,
/// écrit par chunks de 20 ms et comble EXACTEMENT les chunks manqués.
/// Sortie (après `stop_stream`) sur erreur de lecture ou sur notification
/// d'endpoint qui concerne la cible : l'appelant re-résout et ré-init, le
/// gap `last_emit` étant conservé.
fn run_stream(
    device: &str,
    target: &Target,
    shared: &Mutex<Inner>,
    open: &OpenStream,
    notices: &mpsc::Receiver<EndpointNotice>,
    chunks: &mut u64,
) {
    let OpenStream {
        client,
        capture,
        event,
        desc,
        current_id,
    } = open;
    let mut converter = Converter {
        desc: *desc,
        pos: 0.0,
    };
    let mut pending: Vec<u8> = Vec::with_capacity(CHUNK_BYTES * 2);
    let mut raw = Vec::with_capacity(1 << 16);
    // Compteurs pour le log fichier toutes les 5 s (paquets lus, paquets
    // flag silent, discontinuités, converter inactif). Remis à zéro à chaque log.
    let mut last_stats = Instant::now();
    let mut pkts_since: u64 = 0;
    let mut silent_since: u64 = 0;
    let mut glitch_since: u64 = 0;
    let invalid = if desc.valid { 0 } else { 1 };

    loop {
        // Notifications d'endpoints (relais mpsc, jamais d'enumerator dans
        // le callback) : on ne rouvre que si notre cible est concernée.
        let mut reopen = false;
        while let Ok(notice) = notices.try_recv() {
            if should_reopen(target, &notice, current_id.as_deref()) {
                reopen = true;
            }
        }
        if reopen {
            debug_log(device, "endpoint changed, reopen");
            let _ = client.stop_stream();
            return;
        }
        // Draine tous les paquets disponibles (GetBuffer/ReleaseBuffer via
        // le wrapper wasapi 0.24).
        loop {
            let next = match capture.get_next_packet_size() {
                Ok(next) => next,
                Err(_) => {
                    let _ = client.stop_stream();
                    return;
                }
            };
            let frames = match next {
                Some(n) if n > 0 => n as usize,
                _ => break,
            };
            let need = frames.saturating_mul(desc.blockalign);
            if need == 0 {
                break;
            }
            if need > (1 << 24) {
                // Paquet énorme : le consommer quand même, sinon GetBuffer
                // n'est jamais libéré et on draine du silence en boucle.
                // `read_from_device` fait ReleaseBuffer même quand le tampon
                // est trop petit ; le contenu hors norme est tronqué.
                raw.resize(1 << 24, 0);
                if let Ok((_, info)) = capture.read_from_device(&mut raw) {
                    pkts_since += 1;
                    if info.flags.silent {
                        silent_since += 1;
                    }
                }
                continue;
            }
            raw.resize(need, 0);
            let (got, info) = match capture.read_from_device(&mut raw) {
                Ok((got, info)) => (got as usize, info),
                Err(_) => {
                    let _ = client.stop_stream();
                    return;
                }
            };
            if got == 0 {
                break;
            }
            pkts_since += 1;
            if info.flags.silent {
                silent_since += 1;
            }
            if info.flags.data_discontinuity || info.flags.timestamp_error {
                // Glitch de position/horloge : log + compteur (stats 5 s) +
                // pad du gap éventuel pour garder la timeline.
                glitch_since += 1;
                debug_log(device, "buffer discontinuity, padding the gap");
                pad_missed(device, shared, chunks);
            }
            // AUDCLNT_BUFFERFLAGS_SILENT : le moteur signale un paquet
            // silencieux (loopback sans son) ; le contenu est indéfini,
            // on le remplace par des zéros.
            // L'absence de paquet (rendu silencieux) est traitée en zéros
            // ci-dessous via la cadence à 20 ms.
            let len = got.saturating_mul(desc.blockalign).min(raw.len());
            if info.flags.silent {
                raw[..len].fill(0);
            }
            converter.push_packet(&raw[..len], &mut pending);
        }

        while pending.len() >= CHUNK_BYTES {
            let chunk: Vec<u8> = pending.drain(..CHUNK_BYTES).collect();
            push_chunk(shared, &chunk, chunks);
        }
        // Sous-utilisation ou silence : pad à zéro pour garder la cadence
        // 20 ms (vumètres vivants, fichier sans trou). Un réveil tardif
        // comble EXACTEMENT les chunks manqués : le premier complète le
        // `pending` partiel s'il existe, les suivants sont des zéros.
        let elapsed = shared
            .lock()
            .unwrap()
            .last_emit
            .map(|t| t.elapsed())
            .unwrap_or(Duration::ZERO);
        let mut remaining = missed_chunks(elapsed).min(MAX_PAD_BURST);
        if remaining > 0 {
            debug_log(device, &format!("pad missed={remaining}"));
            pending.resize(CHUNK_BYTES, 0);
            let chunk: Vec<u8> = pending.drain(..CHUNK_BYTES).collect();
            push_chunk(shared, &chunk, chunks);
            remaining -= 1;
            if remaining > 0 {
                pad_silence(shared, chunks, remaining);
            }
        }
        // Compteurs toutes les 5 s par thread, depuis le dernier log.
        if last_stats.elapsed() >= Duration::from_secs(5) {
            debug_log(
                device,
                &format!(
                    "pkts={pkts_since} silent={silent_since} glitch={glitch_since} invalid={invalid}"
                ),
            );
            pkts_since = 0;
            silent_since = 0;
            glitch_since = 0;
            last_stats = Instant::now();
        }

        match event {
            // Timeout = 20 ms : réveil aussi en cas de silence (loopback).
            Some(handle) => {
                let _ = handle.wait_for_event(20);
            }
            None => thread::sleep(Duration::from_millis(5)),
        }
    }
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

    #[cfg(feature = "ci-audio")]
    #[test]
    fn synthetic_endpoint_records_pauses_and_releases_without_wasapi() {
        let source = Source::spawn("@DEFAULT_SOURCE@");
        let weak = Arc::downgrade(&source.inner);
        let deadline = Instant::now() + Duration::from_secs(5);
        while source.recent_peak(HISTORY) < 0.2 {
            assert!(Instant::now() < deadline, "synthetic input did not start");
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(source.endpoint().unwrap().id, "synthetic:mic");
        assert!(source.init_error().is_none());
        let path = std::env::temp_dir().join(format!("mr-synthetic-{}.raw", std::process::id()));
        source.start_recording(&path).unwrap();
        let length = || {
            let inner = source.inner.lock().unwrap();
            let file = inner.file.as_ref().unwrap();
            file.get_ref().metadata().unwrap().len() + file.buffer().len() as u64
        };
        while length() < CHUNK_BYTES as u64 * 3 {
            assert!(
                Instant::now() < deadline,
                "synthetic recording did not advance"
            );
            thread::sleep(Duration::from_millis(10));
        }
        source.set_paused(true);
        let before = length();
        thread::sleep(Duration::from_millis(80));
        assert_eq!(length(), before, "paused input wrote samples");
        source.set_paused(false);
        while length() <= before {
            assert!(
                Instant::now() < deadline,
                "resumed recording did not advance"
            );
            thread::sleep(Duration::from_millis(10));
        }
        source.stop_recording().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len() % CHUNK_BYTES, 0);
        assert!(
            bytes
                .as_chunks::<2>()
                .0
                .iter()
                .any(|s| i16::from_le_bytes(*s).unsigned_abs() > 6000)
        );
        std::fs::remove_file(path).unwrap();
        drop(source);
        while weak.upgrade().is_some() {
            assert!(
                Instant::now() < deadline,
                "synthetic input retained the closed source"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn late_wakeups_pad_every_missed_chunk() {
        assert_eq!(missed_chunks(Duration::from_millis(0)), 0);
        assert_eq!(missed_chunks(Duration::from_millis(19)), 0);
        assert_eq!(missed_chunks(Duration::from_millis(20)), 1);
        assert_eq!(missed_chunks(Duration::from_millis(45)), 2);
        assert_eq!(missed_chunks(Duration::from_millis(1000)), 50);
        const { assert!(MAX_PAD_BURST == 1500) };
    }

    #[test]
    fn first_write_error_is_kept() {
        let mut inner = Inner {
            levels: VecDeque::from(vec![0.0; HISTORY]),
            file: None,
            recording: 0,
            paused: false,
            error: None,
            write_error: None,
            last_emit: None,
            endpoint: None,
        };
        note_write_error(&mut inner, "write", "disk full".to_owned());
        note_write_error(&mut inner, "flush", "later failure".to_owned());
        assert_eq!(inner.write_error.as_deref(), Some("write: disk full"));
        inner.write_error = None;
        inner.recording = 1;
        let path = std::env::temp_dir().join(format!("mr-windows-sync-{}", std::process::id()));
        inner.file = Some(BufWriter::new(File::create(&path).unwrap()));
        note_sync_error(&mut inner, 0, "old failure".into());
        assert!(inner.write_error.is_none());
        note_sync_error(&mut inner, 1, "current failure".into());
        assert_eq!(inner.write_error.as_deref(), Some("sync: current failure"));
        inner.file.take();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn center_voice_survives_the_downmix() {
        let (left, right) =
            downmix_samples(8, 0x63f, |i| [0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0][i]);
        assert!(left > 0.5 && right > 0.5);
        assert!((left - right).abs() < 0.01);
    }

    #[test]
    fn front_left_and_surround_reach_their_side() {
        let (left, right) =
            downmix_samples(8, 0x63f, |i| [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0][i]);
        assert!(left > 0.9 && right.abs() < 0.01);
        let (left, right) =
            downmix_samples(8, 0x63f, |i| [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0][i]);
        assert!(left > 0.5);
        assert!(left > right);
    }

    #[test]
    fn full_scale_on_every_channel_does_not_clip() {
        let (left, right) = downmix_samples(8, 0x63f, |_| -1.0);
        assert!(left.abs() <= 1.0 && right.abs() <= 1.0);
    }

    #[test]
    fn four_channel_layout_keeps_rear_audio_and_omits_only_lfe() {
        let raw: Vec<u8> = [0.0f32, 0.0, 0.0, 0.5]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        let convert = |mask| {
            let mut converter = Converter {
                desc: MixDesc {
                    rate: RATE,
                    channels: 4,
                    channel_mask: mask,
                    kind: SampleKind::F32,
                    blockalign: 16,
                    valid: true,
                },
                pos: 0.0,
            };
            let mut bytes = Vec::new();
            converter.push_packet(&raw, &mut bytes);
            assert_eq!(bytes.len(), 4);
            (
                i16::from_le_bytes([bytes[0], bytes[1]]),
                i16::from_le_bytes([bytes[2], bytes[3]]),
            )
        };
        let (left, right) = convert(0x33); // FL FR BL BR (quad).
        assert_eq!(left, 0);
        assert!(right > 10_000, "rear-right audio was lost");
        let (left, right) = convert(0x107); // FL FR FC BC (surround).
        assert!(left > 10_000);
        assert_eq!(left, right);
        assert_eq!(convert(0xf), (0, 0)); // FL FR FC LFE (3.1).
    }

    #[test]
    fn converter_keeps_center_through_resampling() {
        // 960 frames i16 5.1 à 48 kHz, centre seul (canal 2) à 440 Hz.
        let desc = MixDesc {
            rate: RATE,
            channels: 6,
            channel_mask: 0x3f, // 5.1 : FL FR FC LFE BL BR.
            kind: SampleKind::I16,
            blockalign: 12,
            valid: true,
        };
        let mut converter = Converter { desc, pos: 0.0 };
        let mut raw = vec![0u8; 960 * 12];
        for frame in 0..960 {
            let v = (f32::sin(2.0 * std::f32::consts::PI * 440.0 * frame as f32 / RATE as f32)
                * 30000.0) as i16;
            let off = frame * 12 + 2 * 2;
            raw[off..off + 2].copy_from_slice(&v.to_le_bytes());
        }
        let mut out = Vec::new();
        converter.push_packet(&raw, &mut out);
        assert_eq!(out.len(), CHUNK_BYTES);
        let peak = |channel: usize| {
            out.as_chunks::<2>()
                .0
                .iter()
                .skip(channel)
                .step_by(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs() as f32 / 32768.0)
                .fold(0.0, f32::max)
        };
        assert!(peak(0) > 0.2, "left peak {}", peak(0));
        assert!(peak(1) > 0.2, "right peak {}", peak(1));
    }

    #[test]
    fn default_targets_follow_console_defaults() {
        assert_eq!(
            target_for_device("@DEFAULT_SOURCE@", None),
            Target::FollowDefault {
                dir: Direction::Capture,
                role: Role::Console,
            }
        );
        assert_eq!(
            target_for_device("@DEFAULT_MONITOR@", None),
            Target::FollowDefault {
                dir: Direction::Render,
                role: Role::Console,
            }
        );
    }

    #[test]
    fn pinned_id_wins_over_default_but_empty_follows() {
        assert_eq!(
            target_for_device("@DEFAULT_SOURCE@", Some("pinned-id")),
            Target::Fixed {
                id: "pinned-id".to_owned(),
            }
        );
        assert!(matches!(
            target_for_device("@DEFAULT_SOURCE@", None),
            Target::FollowDefault { .. }
        ));
        assert!(matches!(
            target_for_device("@DEFAULT_SOURCE@", Some("")),
            Target::FollowDefault { .. }
        ));
    }

    #[test]
    fn follow_default_reopens_only_on_its_own_default_or_current_id() {
        let target = Target::FollowDefault {
            dir: Direction::Capture,
            role: Role::Console,
        };
        assert!(should_reopen(
            &target,
            &EndpointNotice::DefaultChanged {
                dir: Direction::Capture,
                role: Role::Console,
            },
            None,
        ));
        assert!(!should_reopen(
            &target,
            &EndpointNotice::DefaultChanged {
                dir: Direction::Render,
                role: Role::Console,
            },
            None,
        ));
        assert!(!should_reopen(
            &target,
            &EndpointNotice::DefaultChanged {
                dir: Direction::Capture,
                role: Role::Multimedia,
            },
            None,
        ));
        assert!(should_reopen(
            &target,
            &EndpointNotice::DeviceChanged("current".to_owned()),
            Some("current"),
        ));
        assert!(!should_reopen(
            &target,
            &EndpointNotice::DeviceChanged("other".to_owned()),
            Some("current"),
        ));
        assert!(!should_reopen(
            &target,
            &EndpointNotice::DeviceChanged("other".to_owned()),
            None,
        ));
    }

    #[test]
    fn fixed_reopens_only_on_its_pinned_id() {
        let target = Target::Fixed {
            id: "pinned".to_owned(),
        };
        assert!(should_reopen(
            &target,
            &EndpointNotice::DeviceChanged("pinned".to_owned()),
            Some("pinned"),
        ));
        assert!(!should_reopen(
            &target,
            &EndpointNotice::DeviceChanged("other".to_owned()),
            Some("pinned"),
        ));
        assert!(!should_reopen(
            &target,
            &EndpointNotice::DefaultChanged {
                dir: Direction::Capture,
                role: Role::Console,
            },
            Some("pinned"),
        ));
    }
}
