# Installation Windows (idempotent, ne supprime rien)
# Vérifie winget, installe Rust stable MSVC (rustup) + cible,
# installe ffmpeg (winget), crée les dossiers de config et réunions,
# puis affiche les prochaines étapes.
# Usage : powershell -ExecutionPolicy Bypass -File .\install.ps1

$ErrorActionPreference = "Stop"

# --- Vérifie que winget est disponible ---
function Test-Winget {
    return [bool](Get-Command winget -ErrorAction SilentlyContinue)
}

if (-not (Test-Winget)) {
    Write-Error "winget introuvable. Installez 'App Installer' depuis le Microsoft Store puis relancez."
}

# --- Rust stable MSVC via rustup (idempotent) ---
$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if (-not $cargo) {
    Write-Host "[install] Rust absent : installation de Rustlang.Rustup via winget..."
    winget install --exact --silent --accept-source-agreements --accept-package-agreements Rustlang.Rustup
    # Recharge le PATH de la session pour trouver cargo juste après l'install.
    $env:PATH = [System.Environment]::GetEnvironmentVariable("PATH", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("PATH", "User")
}
else {
    Write-Host "[install] cargo déjà présent : $($cargo.Source)"
}

# Toolchain stable + cible MSVC (sans réinstaller si déjà là).
& rustup toolchain install stable 2>$null
& rustup default stable 2>$null
& rustup target add x86_64-pc-windows-msvc
Write-Host "[install] Rust : $(& rustc --version 2>$null)"

# --- ffmpeg recommandé (seek précis, formes d'onde, imports) ---
$ffmpeg = Get-Command ffmpeg -ErrorAction SilentlyContinue
if (-not $ffmpeg) {
    Write-Host "[install] ffmpeg absent : installation via winget (Gyan.FFmpeg)..."
    winget install --exact --silent --accept-source-agreements --accept-package-agreements Gyan.FFmpeg
    $env:PATH = [System.Environment]::GetEnvironmentVariable("PATH", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("PATH", "User")
    $ffmpeg = Get-Command ffmpeg -ErrorAction SilentlyContinue
}
if ($ffmpeg) {
    Write-Host "[install] ffmpeg : $($ffmpeg.Source)"
}
else {
    Write-Warning "[install] ffmpeg toujours introuvable après install. Ajoutez son dossier au PATH ou posez ffmpeg.exe à côté de l'exécutable (voir src/export.rs)."
}

# --- Dossiers de config et réunions (lus par src/platform.rs) ---
# NOTE : APP_NAME vaut encore "omarchy-meeting-recorder" (src/main.rs) :
# le dossier réel est %APPDATA%\omarchy-meeting-recorder, pas "meeting-recorder-windows".
$configDir = Join-Path $env:APPDATA "omarchy-meeting-recorder"
$meetingsDir = Join-Path ([Environment]::GetFolderPath("MyDocuments")) "Meetings"
foreach ($dir in @($configDir, $meetingsDir)) {
    if (-not (Test-Path -LiteralPath $dir)) {
        New-Item -ItemType Directory -Path $dir | Out-Null
        Write-Host "[install] dossier créé : $dir"
    }
    else {
        Write-Host "[install] dossier OK : $dir"
    }
}

# --- Prochaines étapes ---
Write-Host ""
Write-Host "Prochaines étapes :"
Write-Host "  1. cargo build --release"
Write-Host "  2. Lancez LM Studio (http://localhost:1234/v1) ou Ollama (http://localhost:11434/v1) et chargez qwen3-4b-instruct."
Write-Host "  3. .\target\release\meeting-recorder-windows.exe"
Write-Host "Config : $configDir\config.toml (model, llm_base_url, llm_model). Réunions : $meetingsDir."
