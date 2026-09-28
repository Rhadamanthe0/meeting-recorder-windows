#![cfg(target_os = "windows")]
//! Capture WASAPI : micro (`@DEFAULT_SOURCE@`) + loopback système
//! (`@DEFAULT_MONITOR@`), via la crate `wasapi` 0.24 déclarée
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
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::{CHANNELS, HISTORY, RATE};
use wasapi::{
    DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat, initialize_mta,
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
    /// While paused the meters keep running but nothing is written.
    paused: bool,
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
            paused: false,
        }));
        let shared = inner.clone();
        thread::spawn(move || {
            loop {
                capture(device, &shared);
                // parec quitte quand le device disparaît ; on rouvre pareil.
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
        inner.paused = false;
        Ok(())
    }

    pub fn set_paused(&self, paused: bool) {
        self.inner.lock().unwrap().paused = paused;
    }

    pub fn stop_recording(&self) {
        if let Some(mut file) = self.inner.lock().unwrap().file.take() {
            let _ = file.flush();
        }
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
/// (la boucle `spawn` dort 1 s et rouvre, comportement inchangé).
fn init_failed(device: &str, step: &str) {
    debug_log(device, &format!("init failed step={step}"));
    debug_log(device, "reopen in 1s");
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
    kind: SampleKind,
    blockalign: usize,
    /// Faux pour un format exotique (ex. float 64 bits) : on émet des zéros.
    valid: bool,
}

impl MixDesc {
    fn from_mix(mix: &WaveFormat) -> Self {
        let rate = mix.get_samplespersec().max(1);
        let channels = (mix.get_nchannels() as usize).max(1);
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
                _ => (SampleKind::F32, bits == 32),
            },
        };
        MixDesc {
            rate,
            channels,
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
            if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 }
        }
        SampleKind::I16 => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
        SampleKind::I24 => {
            let mut v = (s[0] as i32) | ((s[1] as i32) << 8) | ((s[2] as i32) << 16);
            if v & 0x80_00_00 != 0 {
                v |= 0xFF00_0000u32 as i32; // extension de signe
            }
            v as f32 / 8_388_608.0
        }
        SampleKind::I32 => {
            i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0
        }
    }
}

/// Convertit des paquets natifs vers s16le 48 kHz stéréo (resample linéaire
/// simple, sans dépendance supplémentaire). Mono dupliqué ; au-delà de
/// 2 canaux, on garde les deux premiers (FL/FR).
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
            for out_ch in 0..2 {
                let in_ch = if channels == 1 {
                    0
                } else {
                    out_ch.min(channels - 1)
                };
                let a = frame_channel_f32(raw, i0, in_ch, &self.desc);
                let b = frame_channel_f32(raw, i1, in_ch, &self.desc);
                let v = (a + (b - a) * frac).clamp(-1.0, 1.0);
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
    let mut inner = shared.lock().unwrap();
    inner.levels.pop_front();
    inner.levels.push_back(peak);
    if !inner.paused
        && let Some(file) = inner.file.as_mut()
    {
        let _ = file.write_all(chunk);
        // Comme sur Linux : au plus une seconde perdue en cas de crash,
        // sync disque toutes les 30 s.
        if chunks.is_multiple_of(50) {
            let _ = file.flush();
        }
        if chunks.is_multiple_of(1500) {
            let _ = file.get_ref().sync_data();
        }
    }
}

fn capture(device: &str, shared: &Mutex<Inner>) {
    // COM MTA pour ce thread (inutile mais inoffensif si déjà initialisé).
    let _ = initialize_mta();
    debug_log(device, &format!("start device={device}"));

    // `@DEFAULT_MONITOR@` = loopback du rendu par défaut ; tout le reste
    // (`@DEFAULT_SOURCE@` inclus) = capture micro par défaut.
    // NOTE wasapi 0.24 : plus de `get_default_device` libre ; on passe par
    // `DeviceEnumerator`. Un nom explicite retombe sur le micro par défaut.
    let default_dir = if device == "@DEFAULT_MONITOR@" {
        Direction::Render
    } else {
        Direction::Capture
    };
    let enumerator = match DeviceEnumerator::new() {
        Ok(enumerator) => enumerator,
        Err(_) => {
            init_failed(device, "enumerator");
            return;
        }
    };
    let dev = match enumerator.get_default_device(&default_dir) {
        Ok(dev) => dev,
        Err(_) => {
            init_failed(device, "default-device");
            return;
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
    let mut client = match dev.get_iaudioclient() {
        Ok(client) => client,
        Err(_) => {
            init_failed(device, "iaudioclient");
            return;
        }
    };
    // Format mix partagé : toujours accepté ; conversion logicielle derrière.
    let mix = match client.get_mixformat() {
        Ok(mix) => mix,
        Err(_) => {
            init_failed(device, "mixformat");
            return;
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
        init_failed(device, "desc-invalid");
        return;
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
        init_failed(device, "initialize-client");
        return;
    }
    let capture = match client.get_audiocaptureclient() {
        Ok(capture) => capture,
        Err(_) => {
            init_failed(device, "captureclient");
            return;
        }
    };
    let event = client.set_get_eventhandle().ok();
    if client.start_stream().is_err() {
        init_failed(device, "start-stream");
        return;
    }

    let mut converter = Converter { desc, pos: 0.0 };
    let mut pending: Vec<u8> = Vec::with_capacity(CHUNK_BYTES * 2);
    let mut raw = Vec::with_capacity(1 << 16);
    let mut chunks: u64 = 0;
    let mut last_emit = Instant::now();
    // Compteurs pour le log fichier toutes les 5 s (paquets lus, paquets
    // flag silent, converter inactif). Remis à zéro à chaque log.
    let mut last_stats = Instant::now();
    let mut pkts_since: u64 = 0;
    let mut silent_since: u64 = 0;
    let invalid = if desc.valid { 0 } else { 1 };

    loop {
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
            push_chunk(shared, &chunk, &mut chunks);
            last_emit = Instant::now();
        }
        // Sous-utilisation ou silence : pad à zéro pour garder la cadence
        // 20 ms (vumètres vivants, fichier sans trou).
        if pending.len() < CHUNK_BYTES && last_emit.elapsed() >= Duration::from_millis(20) {
            pending.resize(CHUNK_BYTES, 0);
            let chunk: Vec<u8> = pending.drain(..CHUNK_BYTES).collect();
            push_chunk(shared, &chunk, &mut chunks);
            last_emit = Instant::now();
        }
        // Compteurs toutes les 5 s par thread, depuis le dernier log.
        if last_stats.elapsed() >= Duration::from_secs(5) {
            debug_log(
                device,
                &format!("pkts={pkts_since} silent={silent_since} invalid={invalid}"),
            );
            pkts_since = 0;
            silent_since = 0;
            last_stats = Instant::now();
        }

        match &event {
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
