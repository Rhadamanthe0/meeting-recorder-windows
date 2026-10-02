# meeting-recorder-windows

Enregistreur de réunions Windows 100 % local et gratuit.

Dépôt : https://github.com/Rhadamanthe0/meeting-recorder-windows

## Ça fait quoi

- Micro + son du PC enregistrés ensemble, sans bot à inviter.
- Transcription Whisper en local, en français.
- Qui-a-dit-quoi avec Nemotron local, jusqu'à 8 voix ; l'écho du PC dans le
  micro n'est pas compté comme un locuteur, et deux locuteurs renommés à
  l'identique n'en font plus qu'un.
- Résumé et actions via LM Studio ou Ollama, en local.
- Historique des réunions + import d'un fichier audio.
- 0 €, rien ne sort du PC.

## Installation

Voie 1 — RECOMMANDÉE (MSI) : téléchargez `MeetingRecorder-<version>.msi`
depuis https://github.com/Rhadamanthe0/meeting-recorder-windows/releases
et exécutez-le (double-clic, sans admin : installation per-user).
Tout est embarqué (GTK, ffmpeg, runtime).
Prérequis restant : LM Studio ou Ollama lancé en local (résumé et actions).

Voie 2 — source (avancé) : prérequis manuels Rust GNU (hôte
x86_64-pc-windows-gnu), MSYS2 UCRT64 (GTK4/libadwaita) et ffmpeg, puis :

```powershell
git clone https://github.com/Rhadamanthe0/meeting-recorder-windows
cd meeting-recorder-windows
cargo build --release
.\target\release\meeting-recorder-windows.exe
```

Mise à jour (récupère les derniers commits) :

```powershell
git pull
```

## Utilisation

1. Lancez l'app (menu Démarrer : Meeting Recorder), enregistrez micro + PC.
2. Transcrivez, puis demandez un résumé.

```powershell
meeting-recorder-windows.exe start "Point hebdo"   # ouvre l'app si besoin et enregistre sous ce nom
meeting-recorder-windows.exe transcribe-file interview.mp3 --speakers 2 > transcript.md
Get-Content transcript.md | meeting-recorder-windows.exe ask "Résume en 5 points"
meeting-recorder-windows.exe watch
```

## Config

Fichier `%APPDATA%\omarchy-meeting-recorder\config.toml` :

```toml
model = "large-v3-turbo"
llm_base_url = "http://localhost:1234/v1"
llm_model = "qwen3-4b-instruct"
```

## Limites

- Transcription longue sur CPU possible.
- Modèle défaut large-v3-turbo (~1,6 Go téléchargés, lent sur CPU : prévoir large pour une longue réunion).
- Pour tester vite : `model = "small"` dans config.toml.

D'après [omarchy-meeting-recorder](https://github.com/jankeesvw/omarchy-meeting-recorder). Licence MIT, voir `LICENSE`.
