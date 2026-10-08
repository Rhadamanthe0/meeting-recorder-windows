# Portage Windows — état final V1

Base du fork `meeting-recorder-windows` (v0.1.0) copiée de l'amont
`omarchy-meeting-recorder` v1.4.0 (amont conservé dans `UPSTREAM_README.md`).
Objectif : 100 % local et gratuit (WASAPI micro + loopback, Whisper local,
diarisation Nemotron ONNX, résumé via LM Studio/Ollama OpenAI-compatible).

## Fait (V1)

- `src/platform.rs` : SEULE source de chemins (`config/data/state/cache/
  meetings/models/runtime_dir`) — XDG/glib sur Linux, `dirs` sur Windows.
- `Cargo.toml` : package renommé, `vulkan` hors défaut, deps Windows
  (`wasapi`, `rodio`, `dirs`, `interprocess`, `windows-sys` ; `cpal` est
  une dépendance transitive de `rodio`),
  `libc` sous `cfg(unix)`, `[[bin]]` dédié. Deps déjà déclarées : rien à ajouter.
- `src/audio/windows.rs` : capture WASAPI micro + loopback, s16le 48 kHz
  stéréo, chunks 20 ms, resampling linéaire simple, reconnexion après 1 s.
- `src/ipc.rs` : named pipe `\\.\pipe\meeting-recorder-windows`
  (`interprocess`), protocole NDJSON et commandes inchangés.
- `src/player.rs` : lecture `rodio` (un `Sink` par piste), `ffmpeg.exe -ss`
  préféré, repli décodeur intégré ; `ffmpeg.exe` via PATH + dossier exe
  (`src/export.rs`).
- `src/action_process.rs` : Job Objects pour les actions et les outils audio
  (lecture, import, export, décodage, sondes et formes d'onde), avec arrêt des
  descendants à la fermeture du job, y compris lors d'un crash du processus.
- `src/agent.rs` : LLM local OpenAI-compatible — LM Studio
  `http://localhost:1234/v1` puis Ollama `http://localhost:11434/v1`,
  config `llm_base_url` / `llm_model` (défaut `qwen3-4b-instruct`), sans outils.
- `hyprctl`/bar-widget neutralisés (`should_offer() == false` sur Windows).
- Packaging V1 : `README.md` (fork), MSI per-user WiX v3 (`packaging/windows/`,
  construit en CI, publié en release ; `MeetingRecorder-<version>.msi`,
  double-clic sans admin) + voie source avancée (Rust GNU, MSYS2 UCRT64,
  ffmpeg, `cargo build --release`), `.github/workflows/windows.yml`
  (CI `windows-latest`), `.gitignore` complété.

Écart de nommage connu (réel, cf. `src/main.rs:27`) : le binaire Cargo est
`meeting-recorder-windows`, mais `APP_NAME` vaut encore
`omarchy-meeting-recorder` — les dossiers `%APPDATA%\omarchy-meeting-recorder\`
et les messages d'aide gardent l'ancien nom. La doc suit le réel.

## Reste (hors V1)

1. Icône tray Windows.
2. Tests audio réels (matériel WASAPI) ; seuls les tests unitaires/logiques existent.
3. Build GTK validé par CI (runs Success du 2026-09-27).

## Limites connues

- Resampling WASAPI linéaire, pas de DSP dédié.
- `ffmpeg.exe` requis pour un seek précis et les formes d'onde.
- Transcription CPU longue ; Vulkan OPT-IN (`--features vulkan`).

## Limites connues V1 (relecture, non corrigées)

- A la fermeture du son, les pistes s'arrêtent avant le flux : l'ordre compte.
- Les modèles sont rangés sous Roaming, pas sous le dossier local attendu.

Les formats WASAPI non pris en charge produisent un diagnostic dans l'interface.
Les noms exportés conservent les accents ; les caractères interdits, de contrôle
et les noms réservés Windows sont filtrés. Une commande `start` dépassant la
limite IPC est refusée avec un message et un code de sortie non nul.
