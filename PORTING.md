# Portage Windows — état final V1

Base du fork `meeting-recorder-windows` (v0.1.0) copiée de l'amont
`omarchy-meeting-recorder` v1.4.0 (amont conservé dans `UPSTREAM_README.md`).
Objectif : 100 % local et gratuit (WASAPI micro + loopback, Whisper local,
diarisation Nemotron ONNX, résumé via LM Studio/Ollama OpenAI-compatible).

## Fait (V1)

- `src/platform.rs` : SEULE source de chemins (`config/data/state/cache/
  meetings/models/runtime_dir`) — XDG/glib sur Linux, `dirs` sur Windows.
- `Cargo.toml` : package renommé, `vulkan` hors défaut, deps Windows
  (`wasapi`, `cpal`, `rodio`, `dirs`, `interprocess`, `windows-sys`),
  `libc` sous `cfg(unix)`, `[[bin]]` dédié. Deps déjà déclarées : rien à ajouter.
- `src/audio/windows.rs` : capture WASAPI micro + loopback, s16le 48 kHz
  stéréo, chunks 20 ms, resampling linéaire simple, reconnexion après 1 s.
- `src/ipc.rs` : named pipe `\\.\pipe\meeting-recorder-windows`
  (`interprocess`), protocole NDJSON et commandes inchangés.
- `src/player.rs` : lecture `rodio` (un `Sink` par piste), `ffmpeg.exe -ss`
  préféré, repli décodeur intégré ; `ffmpeg.exe` via PATH + dossier exe
  (`src/export.rs`).
- `src/agent.rs` : LLM local OpenAI-compatible — LM Studio
  `http://localhost:1234/v1` puis Ollama `http://localhost:11434/v1`,
  config `llm_base_url` / `llm_model` (défaut `qwen3-4b-instruct`), sans outils.
- `hyprctl`/bar-widget neutralisés (`should_offer() == false` sur Windows).
- Packaging V1 : `README.md` (fork), `install.ps1` idempotent,
  `.github/workflows/windows.yml` (CI `windows-latest`), `.gitignore` complété.

Écart de nommage connu (réel, cf. `src/main.rs:27`) : le binaire Cargo est
`meeting-recorder-windows`, mais `APP_NAME` vaut encore
`omarchy-meeting-recorder` — les dossiers `%APPDATA%\omarchy-meeting-recorder\`
et les messages d'aide gardent l'ancien nom. Doc et script suivent le réel.

## Reste (hors V1)

1. MSI/WiX (aucun installeur ; binaire + `install.ps1` uniquement).
2. Icône tray Windows.
3. Job Object pour les enfants `ffmpeg.exe` (orphelins possibles sur crash).
4. Tests audio réels (matériel WASAPI) ; seuls les tests unitaires/logiques existent.
5. Build GTK validé par CI (workflow présent, jamais encore vert ici).

## Limites connues

- Resampling WASAPI linéaire, pas de DSP dédié.
- `ffmpeg.exe` requis pour un seek précis et les formes d'onde.
- Transcription CPU longue ; Vulkan OPT-IN (`--features vulkan`).
