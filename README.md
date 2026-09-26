# meeting-recorder-windows

Fork Windows de [omarchy-meeting-recorder](https://github.com/jankeesvw/omarchy-meeting-recorder) (licence MIT conservée, voir `LICENSE`). L'amont est conservé tel quel dans `UPSTREAM_README.md`.

Enregistreur de réunions 100 % local et gratuit : micro + son système, transcription Whisper sur la machine, diarisation Nemotron, chapitres et actions via LLM local. Pas de bot, rien ne quitte le PC (sauf le texte du transcript envoyé au LLM local que vous avez choisi, pour les chapitres).

## Fonctionnalités

| Fonction | Windows (ce fork) | Amont Linux |
|---|---|---|
| Capture micro + loopback | WASAPI via `wasapi` : micro par défaut + loopback du rendu par défaut, s16le 48 kHz stéréo, chunks 20 ms (`src/audio/windows.rs`) | `parec` (`@DEFAULT_SOURCE@` / `@DEFAULT_MONITOR@`) |
| Transcription | Whisper local via `whisper-rs`, défaut `large-v3-turbo` | identique |
| Diarisation import mono-fichier | Nemotron 3 ONNX local, jusqu'à 8 speakers | identique |
| Chapitres / `ask` / actions LLM | LLM local OpenAI-compatible : LM Studio `http://localhost:1234/v1` puis Ollama `http://localhost:11434/v1`, modèle défaut `qwen3-4b-instruct` (`src/agent.rs`, sans outils) | `omarchy-default-agent` (Claude/Codex/…) sandboxé sans outils |
| Lecture réunion | `rodio`, un `Sink` par piste ; décodeur préféré `ffmpeg.exe -ss` sinon décodeur intégré (`src/player.rs`) ; `ffmpeg.exe` résolu via PATH puis dossier de l'exécutable (`src/export.rs`) | `ffmpeg` décodant dans `pacat` |
| IPC `watch` / `start` / `stop` / … | Named pipe `\\.\pipe\meeting-recorder-windows` (crate `interprocess`), protocole NDJSON inchangé (`src/ipc.rs`) | socket Unix `$XDG_RUNTIME_DIR/omarchy-meeting-recorder.sock` |
| Chemins | `src/platform.rs` : config `%APPDATA%\omarchy-meeting-recorder\`, réunions `Documents\Meetings`, modèles `…\omarchy-meeting-recorder\models\` | XDG/glib |
| Bar-widget / `hyprctl` | Neutralisés (`should_offer() == false` sur Windows, pas de barre Omarchy) | widget Quattro + règles Hyprland |

Note de cohérence : le binaire Cargo s'appelle `meeting-recorder-windows` (`target\release\meeting-recorder-windows.exe`), mais `APP_NAME` vaut encore `omarchy-meeting-recorder` (`src/main.rs`) : les messages d'aide et les dossiers `%APPDATA%\omarchy-meeting-recorder\` gardent l'ancien nom. C'est l'état réel du code, pas un choix de doc.

## Prérequis

- Windows 10/11 x64.
- Rust stable MSVC (via `install.ps1`, qui installe `rustup`).
- `ffmpeg.exe` recommandé (seek précis, formes d'onde, import de tout format) :
  ```powershell
  winget install Gyan.FFmpeg
  ```
  Résolution : `ffmpeg.exe` sur PATH, sinon à côté de l'exécutable, sinon nom simple laissé à la recherche PATH (`src/export.rs`). Sans ffmpeg, la lecture repose sur le décodeur intégré de `rodio` (seek approximatif via `skip_duration`).
- LLM local, l'un des deux (pour chapitres / `ask` / actions) :
  - LM Studio, serveur local sur `http://localhost:1234/v1`, ou
  - Ollama, endpoint OpenAI sur `http://localhost:11434/v1`.
- Runtime GTK4 (fenêtre) : le binaire lie `gtk4` + `libadwaita` (voir `Cargo.toml`).
  - Option retenue pour ce fork et la CI : **MSYS2 UCRT64** :
    1. Installer MSYS2, ouvrir un shell **UCRT64**.
    2. `pacman -Syu` puis `pacman -S mingw-w64-ucrt-x86_64-gtk4 mingw-w64-ucrt-x86_64-libadwaita mingw-w64-ucrt-x86_64-pkgconf mingw-w64-ucrt-x86_64-gcc`.
    3. Ajouter `C:\msys64\ucrt64\bin` au `PATH` avant `cargo build` (DLL + `pkg-config`).
  - Alternatives non retenues ici : `gvsbuild` (build GTK complet, lourd) ou `vcpkg` (possible mais variables `PKG_CONFIG_PATH`/`LIB` à câbler à la main). Voir `.github/workflows/windows.yml`, qui installe MSYS2 UCRT64.

## Installation

```powershell
# Depuis la racine du dépôt, en PowerShell :
.\install.ps1
cargo build --release
.\target\release\meeting-recorder-windows.exe
```

`install.ps1` est idempotent et ne supprime rien : il vérifie `winget`, installe `rustup` (toolchain stable + cible `x86_64-pc-windows-msvc`) et `ffmpeg`, crée `%APPDATA%\omarchy-meeting-recorder\` et `Documents\Meetings`, puis affiche les étapes suivantes (build, LM Studio). Build CPU par défaut (Vulkan OFF) ; pour Vulkan : `cargo build --release --features vulkan` (headers Vulkan + `glslc` requis).

## Configuration

Fichier `%APPDATA%\omarchy-meeting-recorder\config.toml` (créé par `install.ps1`, lu par `src/models.rs` et `src/agent.rs`) :

```toml
model = "large-v3-turbo"          # tiny, base, small, medium, large-v3, large-v3-turbo, ou chemin vers un .bin
llm_base_url = "http://localhost:1234/v1"   # sinon essai LM Studio puis Ollama
llm_model = "qwen3-4b-instruct"   # modèle chargé dans LM Studio / Ollama
```

- Modèles Whisper cherchés dans `platform::models_dir()` (`%APPDATA%\omarchy-meeting-recorder\models\ggml-<nom>.bin`), sinon `voxtype/models`, sinon téléchargés depuis Hugging Face (`ggerganov/whisper.cpp`).
- Réunions : `Documents\Meetings\<YYYYMMDDHHMM> <nom>\`.
- Réglages UI : état dans `platform::state_dir()` + `omarchy-meeting-recorder\settings.json`.

## Usage CLI

Binaire `meeting-recorder-windows` (l'aide affiche encore `omarchy-meeting-recorder`, cf. note plus haut) :

| Commande | Effet |
|---|---|
| `(sans args)` | Ouvre l'enregistreur, prêt à enregistrer |
| `<dossier ou .meeting-recorder>` | Ouvre une réunion sauvée (page done) |
| `start` / `stop` / `pause` | Pilote la fenêtre ouverte (via le named pipe) |
| `watch` | Relaye l'état NDJSON (`{"state":"off"}` si l'app est fermée) |
| `transcribe <mic> <computer> [--language xx] [--model nom]` | Transcrit deux pistes, Markdown sur stdout |
| `transcribe-file <audio> [--speakers N] [--language xx] [--model nom]` | Transcrit un fichier, voix séparées (Nemotron) |
| `diarize <audio> [--speakers N]` | Diarisation seule |
| `ask "<prompt>" < texte` / `ask --agent` | Prompt via LLM local (sans outils) / affiche l'agent |
| `action "<nom>" <dossier>` / `action` | Lance une action / liste les actions |
| `compact`, `new-window` | Bascule compacte / ouvre une autre fenêtre |

Exemples :

```powershell
.\target\release\meeting-recorder-windows.exe transcribe mic.ogg computer.ogg --language fr > transcript.md
.\target\release\meeting-recorder-windows.exe transcribe-file interview.mp3 --speakers 2 > transcript.md
Get-Content transcript.md | .\target\release\meeting-recorder-windows.exe ask "Résume en 5 points"
.\target\release\meeting-recorder-windows.exe watch
```

## Limites V1 connues

- Resampling WASAPI **linéaire** simple vers 48 kHz (`src/audio/windows.rs::Converter`), sans dépendance DSP.
- `ffmpeg.exe` requis pour un seek précis et les formes d'onde ; repli `rodio` sinon (seek approximatif).
- Pas de Job Object : un crash entre spawn et `Drop` peut laisser un `ffmpeg.exe` orphelin (`src/player.rs`).
- Pas d'icône tray, pas de widget de barre (neutralisé), pas de règles `hyprctl`.
- Pas d'installeur MSI/WiX (binaire + `install.ps1` uniquement).
- Transcription longue sur CPU possible ; Vulkan OPT-IN (`--features vulkan`).

## Licence

MIT, comme l'amont. Voir `LICENSE`.
