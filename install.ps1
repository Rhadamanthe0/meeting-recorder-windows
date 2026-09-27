# Installation Windows (idempotent, ne supprime rien)
# Vérifie winget, installe Rust stable GNU (rustup) + cible,
# installe MSYS2/GTK4 (UCRT64) + PATH, installe ffmpeg (winget),
# crée les dossiers de config et réunions, puis affiche les prochaines étapes.
# Usage : powershell -ExecutionPolicy Bypass -File .\install.ps1

# --- Auto-mise à jour depuis GitHub (idempotent, ne supprime rien) ---
# Si lancé dans un clone git du dépôt, récupère les derniers commits.
# Sinon, affiche l'URL de clone et continue l'installation ici.
$RepoUrl = "https://github.com/Rhadamanthe0/omarchy-meeting-recorder-windows"
$git = Get-Command git -ErrorAction SilentlyContinue
if ($git -and (Test-Path -LiteralPath (Join-Path $PSScriptRoot ".git"))) {
    Write-Host "[install] Mise à jour du dépôt (git pull --ff-only)..."
    try {
        & git pull --ff-only
    }
    catch {
        Write-Warning "[install] git pull impossible (hors ligne ?). On continue avec la version locale."
    }
}
else {
    Write-Host "[install] Pour récupérer le dépôt depuis GitHub : git clone $RepoUrl"
}

$ErrorActionPreference = "Stop"

# --- Vérifie que winget est disponible ---
function Test-Winget {
    return [bool](Get-Command winget -ErrorAction SilentlyContinue)
}

if (-not (Test-Winget)) {
    Write-Error "winget introuvable. Installez 'App Installer' depuis le Microsoft Store puis relancez."
}

# --- Rust stable GNU via rustup (idempotent) ---
# Ce projet ne build qu'en GNU (hôte x86_64-pc-windows-gnu) : MSVC non supporté
# (link GTK MinGW impossible depuis MSVC).
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

# Toolchain stable GNU + cible GNU (sans réinstaller si déjà là).
& rustup toolchain install stable-x86_64-pc-windows-gnu 2>$null
& rustup default stable-x86_64-pc-windows-gnu 2>$null
& rustup target add x86_64-pc-windows-gnu
# Vérifie que l'hôte actif est bien GNU, sinon message d'erreur clair.
$rustcInfo = (& rustc -vV 2>$null) -join "`n"
if (-not $rustcInfo -or $rustcInfo -notmatch "gnu") {
    Write-Error "MSVC non supporté : ce projet build en GNU (stable-x86_64-pc-windows-gnu). Relancez rustup pour basculer sur la toolchain GNU."
}
Write-Host "[install] Rust : $(& rustc --version 2>$null)"

# --- MSYS2/GTK4 UCRT64 (idempotent) ---
# Requis au build (glib-sys via pkgconf/gcc) et à l'exécution (runtime GTK4/libadwaita).
$ucrtBin = "C:\msys64\ucrt64\bin"
$pkgconfLocal = Join-Path $ucrtBin "pkgconf.exe"
$pkgconfCmd = Get-Command pkgconf -ErrorAction SilentlyContinue
if (-not (Test-Path -LiteralPath $pkgconfLocal) -and -not $pkgconfCmd) {
    Write-Host "[install] MSYS2/GTK absent : installation de MSYS2.MSYS2 via winget..."
    winget install --exact --silent --accept-source-agreements --accept-package-agreements MSYS2.MSYS2
    Write-Host "[install] Installation des paquets GTK4/libadwaita UCRT64 via pacman..."
    & C:\msys64\msys2_shell.cmd -defterm -here -no-start -ucrt64 -c "pacman -Sy --noconfirm && pacman -S --needed --noconfirm mingw-w64-ucrt-x86_64-gtk4 mingw-w64-ucrt-x86_64-libadwaita mingw-w64-ucrt-x86_64-pkgconf mingw-w64-ucrt-x86_64-gcc mingw-w64-ucrt-x86_64-clang"
}
else {
    Write-Host "[install] MSYS2/GTK déjà présent : $ucrtBin"
}

# --- Ajoute UCRT64 au PATH utilisateur persistant + session (idempotent) ---
$userPath = [System.Environment]::GetEnvironmentVariable("PATH", "User")
if ($userPath -notlike "*$ucrtBin*") {
    [Environment]::SetEnvironmentVariable("PATH", "$userPath;$ucrtBin", "User")
    Write-Host "[install] PATH utilisateur : $ucrtBin ajouté (persistant)."
}
else {
    Write-Host "[install] PATH utilisateur OK : $ucrtBin déjà présent."
}
if ($env:PATH -notlike "*$ucrtBin*") {
    $env:PATH = "$env:PATH;$ucrtBin"
}

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

# --- Vérifications finales bloquantes (échouent explicitement si absentes) ---
$rustcFull = (& rustc -vV 2>$null) -join "`n"
if (-not $rustcFull -or $rustcFull -notmatch "gnu") {
    Write-Error "Vérification Rust GNU échouée : `rustc -vV` ne contient pas 'gnu'. MSVC non supporté : ce projet build en GNU."
}
Write-Host "[install] vérif rustc (GNU) : $(& rustc --version 2>$null)"
try {
    $pkgconfVer = (& pkgconf --version 2>$null) -join "`n"
    if (-not $pkgconfVer) { throw "pkgconf introuvable" }
    Write-Host "[install] vérif pkgconf : $pkgconfVer"
}
catch {
    Write-Error "Vérification pkgconf échouée : MSYS2 UCRT64 manquant ou C:\msys64\ucrt64\bin absent du PATH."
}
try {
    $ffmpegVer = (& ffmpeg -version 2>$null | Select-Object -First 1) -join "`n"
    if (-not $ffmpegVer) { throw "ffmpeg introuvable" }
    Write-Host "[install] vérif ffmpeg : $ffmpegVer"
}
catch {
    Write-Error "Vérification ffmpeg échouée : ffmpeg introuvable après install."
}
try {
    $gccVer = (& gcc --version 2>$null | Select-Object -First 1) -join "`n"
    if (-not $gccVer) { throw "gcc introuvable" }
    Write-Host "[install] vérif gcc : $gccVer"
}
catch {
    Write-Error "Vérification gcc échouée : toolchain GCC UCRT64 manquante (pacman mingw-w64-ucrt-x86_64-gcc)."
}

# --- Prochaines étapes ---
Write-Host ""
Write-Host "Prochaines étapes :"
Write-Host "  1. cargo build --release (GNU)"
Write-Host "  2. Lancez LM Studio (http://localhost:1234/v1) ou Ollama (http://localhost:11434/v1) et chargez qwen3-4b-instruct."
Write-Host "  3. .\target\release\meeting-recorder-windows.exe"
Write-Host "Config : $configDir\config.toml (model, llm_base_url, llm_model). Réunions : $meetingsDir."
