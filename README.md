# meeting-recorder-windows

Enregistreur de réunions Windows 100 % local et gratuit.

Dépôt : https://github.com/Rhadamanthe0/omarchy-meeting-recorder-windows

## Ça fait quoi

- Micro + son du PC enregistrés ensemble, sans bot à inviter.
- Transcription Whisper en local, en français.
- Qui-a-dit-quoi avec Nemotron local, jusqu'à 8 voix.
- Résumé et actions via LM Studio ou Ollama, en local.
- Historique des réunions + import d'un fichier audio.
- 0 €, rien ne sort du PC.

## Prérequis

- Windows 10/11 x64.
- LM Studio ou Ollama (pour résumé et actions).
- ffmpeg conseillé (`winget install Gyan.FFmpeg`).

## Installation depuis GitHub

```powershell
git clone https://github.com/Rhadamanthe0/omarchy-meeting-recorder-windows
cd omarchy-meeting-recorder-windows
.\install.ps1
cargo build --release
.\target\release\meeting-recorder-windows.exe
```

Besoin de GTK4/MSYS2 ? Suivez `install.ps1` et la CI (`.github/workflows/windows.yml`).

Mise à jour (récupère les derniers commits) :

```powershell
git pull
```

## Utilisation

1. Lancez l'exe, enregistrez micro + PC.
2. Transcrivez, puis demandez un résumé.

```powershell
.\target\release\meeting-recorder-windows.exe
.\target\release\meeting-recorder-windows.exe transcribe-file interview.mp3 --speakers 2 > transcript.md
Get-Content transcript.md | .\target\release\meeting-recorder-windows.exe ask "Résume en 5 points"
.\target\release\meeting-recorder-windows.exe watch
```

## Config

Fichier `%APPDATA%\omarchy-meeting-recorder\config.toml` :

```toml
model = "large-v3-turbo"
llm_base_url = "http://localhost:1234/v1"
llm_model = "qwen3-4b-instruct"
```

## Limites

- ffmpeg requis pour un seek précis et les formes d'onde.
- Transcription longue sur CPU possible.
- Pas d'installeur, binaire + `install.ps1` uniquement.

D'après [omarchy-meeting-recorder](https://github.com/jankeesvw/omarchy-meeting-recorder). Licence MIT, voir `LICENSE`.
