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
- Traitement audio local, sans abonnement. Les modèles sont téléchargés au
  premier usage. Les données envoyées par une action ou un serveur LLM distant
  dépendent de votre configuration.

## Installation

Voie 1 — RECOMMANDÉE (MSI) : téléchargez `MeetingRecorder-<version>.msi`
depuis https://github.com/Rhadamanthe0/meeting-recorder-windows/releases
et exécutez-le (double-clic, sans admin : installation per-user).
Tout est embarqué (GTK, ffmpeg, runtime).
Prérequis restant : LM Studio ou Ollama lancé en local (résumé et actions).

Voie 2 — source (avancé) : prérequis manuels Rust GNU (hôte
x86_64-pc-windows-gnu), MSYS2 UCRT64 (GTK4/libadwaita, pkgconf, GCC et
Clang), ffmpeg et le SDK ONNX Runtime Windows x64 1.28.
Le téléchargement vérifié du SDK est décrit dans l'étape « ORT dynamique
Microsoft 1.28 » de [la configuration CI](.github/workflows/windows.yml).
Dans le même terminal PowerShell, adaptez les deux chemins ci-dessous à
votre installation MSYS2 et au dossier `lib` du SDK extrait :

```powershell
$mrUcrtRoot = "C:\msys64\ucrt64"
$env:PATH = "$mrUcrtRoot\bin;$env:PATH"
$env:PKG_CONFIG = "$mrUcrtRoot\bin\pkgconf.exe"
$env:PKG_CONFIG_PATH = "$mrUcrtRoot\lib\pkgconfig;$mrUcrtRoot\share\pkgconfig"
$env:LIBCLANG_PATH = "$mrUcrtRoot\bin"
$env:ORT_LIB_LOCATION = "C:\outils\onnxruntime-win-x64-1.28.0\lib"
$env:ORT_PREFER_DYNAMIC_LINK = "1"
```

Puis :

```powershell
git clone https://github.com/Rhadamanthe0/meeting-recorder-windows
cd meeting-recorder-windows
cargo build --release --locked
Copy-Item "$env:ORT_LIB_LOCATION\onnxruntime*.dll" .\target\release\
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

Les tests du runner Windows utilisent des entrées audio simulées et une sortie
silencieuse : voir [les contrôles CI sans périphériques réels](docs/ci-audio-windows.md).

- Transcription longue sur CPU possible.
- Modèle défaut large-v3-turbo (~1,6 Go téléchargés, lent sur CPU : prévoir large pour une longue réunion).
- Pour tester vite : `model = "small"` dans config.toml.

D'après [omarchy-meeting-recorder](https://github.com/jankeesvw/omarchy-meeting-recorder). Licence MIT, voir `LICENSE`.
