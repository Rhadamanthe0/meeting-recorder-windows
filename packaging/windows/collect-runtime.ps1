# Rassemble dans packaging/windows/stage/ tout ce que l'exe exige.
# Idempotent : le dossier stage est reconstruit à chaque appel (rien d'autre
# n'est modifié sur le système, hors téléchargement ffmpeg en cache TEMP).
# Usage : powershell -ExecutionPolicy Bypass -File packaging/windows/collect-runtime.ps1
# ÉCHEC EXPLICITE (Write-Error) si un fichier requis est introuvable.

param(
    # Dossier de sortie (recréé à chaque appel).
    [string]$Stage = (Join-Path $PSScriptRoot "stage"),
    # Build GNU attendu : target/release/meeting-recorder-windows.exe.
    [string]$TargetRelease = (Join-Path (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent) "target\release"),
    # Runtime GTK4/libadwaita : $env:UCRT_BIN (CI) sinon C:\msys64\ucrt64\bin.
    [string]$UcrtBin = $env:UCRT_BIN,
    # Build FFmpeg pinné si aucun ffmpeg local : asset immuable GyanD/codexffmpeg 9.0.2.
    # Hash source : api.github.com GyanD/codexffmpeg releases/tags/9.0.2 (vérifié le 2026-09-30),
    # digest officiel de l'asset ffmpeg-9.0.2-essentials_build.zip. Bump via PR relue.
    [string]$FfmpegZipUrl = "https://github.com/GyanD/codexffmpeg/releases/download/9.0.2/ffmpeg-9.0.2-essentials_build.zip",
    # SHA-256 attendu du ZIP (même source que ci-dessus).
    [string]$FfmpegZipSha256 = "60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba",
    # Taille attendue du ZIP en octets (même source que ci-dessus).
    [long]$FfmpegZipSize = 114768076
)

$ErrorActionPreference = "Stop"

# --- Résout UCRT_BIN (binaires + DLLs MinGW/GTK) ---
if (-not $UcrtBin) { $UcrtBin = "C:\msys64\ucrt64\bin" }
if (-not (Test-Path -LiteralPath $UcrtBin)) {
    Write-Error "UCRT_BIN introuvable : '$UcrtBin' (ni `$env:UCRT_BIN ni C:\msys64\ucrt64\bin). Installez MSYS2 UCRT64 + GTK4/libadwaita."
}
$UcrtRoot = Split-Path $UcrtBin -Parent  # ...\ucrt64 (pour share/ et lib/)
$objdump = Join-Path $UcrtBin "objdump.exe"
if (-not (Test-Path -LiteralPath $objdump)) {
    Write-Error "objdump.exe introuvable dans '$UcrtBin' (paquet mingw-w64-ucrt-x86_64-binutils manquant ?)."
}
$schemaCompiler = Join-Path $UcrtBin "glib-compile-schemas.exe"
if (-not (Test-Path -LiteralPath $schemaCompiler)) {
    Write-Error "glib-compile-schemas.exe introuvable dans '$UcrtBin' (paquet mingw-w64-ucrt-x86_64-glib2 manquant ?)."
}

# --- DLLs système Windows : jamais embarquées (fournies par l'OS) ---
# Liste explicite (insensible à la casse) + préfixes api-ms-*/ext-ms-*.
# combase.dll : inbox Win10+, COM de base (importée via crate windows/WASAPI).
# avicap32/avifil32/msvfw32 : VFW inbox, importées par ffmpeg/ffprobe Gyan.
$SystemDlls = New-Object System.Collections.Generic.HashSet[string]([System.StringComparer]::OrdinalIgnoreCase)
@(
    "advapi32.dll", "avicap32.dll", "avifil32.dll", "avrt.dll", "bcrypt.dll", "cfgmgr32.dll", "combase.dll", "comctl32.dll",
    "comdlg32.dll", "crypt32.dll", "cryptbase.dll", "d2d1.dll", "d3d11.dll",
    "dcomp.dll", "dhcpcsvc.dll", "dnsapi.dll", "dwmapi.dll", "dxgi.dll",
    "fwpuclnt.dll", "gdi32.dll", "gdiplus.dll", "hid.dll", "imm32.dll",
    "iphlpapi.dll", "kernel32.dll", "kernelbase.dll", "ksuser.dll",
    "mmdevapi.dll", "msimg32.dll", "msvcrt.dll", "msvfw32.dll", "mswsock.dll", "netapi32.dll",
    "nsi.dll", "ntdll.dll", "ole32.dll", "oleaut32.dll", "opengl32.dll",
    "powrprof.dll", "propsys.dll", "psapi.dll", "rpcrt4.dll", "sechost.dll",
    "setupapi.dll", "shcore.dll", "shell32.dll", "shlwapi.dll", "ucrtbase.dll",
    "user32.dll", "userenv.dll", "usp10.dll", "uxtheme.dll", "version.dll",
    "winmm.dll", "winspool.drv", "ws2_32.dll", "wtsapi32.dll", "dwrite.dll",
    "windowscodecs.dll", "winhttp.dll", "wininet.dll", "urlmon.dll",
    "dbghelp.dll", "ncrypt.dll", "profapi.dll", "secur32.dll",
    "d3d12.dll", "wldap32.dll", "bcryptprimitives.dll"
) | ForEach-Object { [void]$SystemDlls.Add($_) }

function Test-SystemDll([string]$name) {
    if ($SystemDlls.Contains($name)) { return $true }
    $lower = $name.ToLowerInvariant()
    return $lower.StartsWith("api-ms-") -or $lower.StartsWith("ext-ms-")
}

# Imports directs d'un binaire via objdump (noms de DLL uniques).
function Get-DllImports([string]$file) {
    $out = & $objdump -p $file 2>$null
    if ($LASTEXITCODE -ne 0) {
        Write-Error "objdump -p a échoué sur '$file' (exit $LASTEXITCODE)."
    }
    $found = @()
    foreach ($line in $out) {
        if ($line -match "DLL Name:\s*(\S+)") { $found += $Matches[1] }
    }
    $unique = $found | Sort-Object -Unique
    if (-not $unique) {
        Write-Warning "Aucun import 'DLL Name:' trouvé dans '$file'."
        return @()
    }
    Write-Verbose "[stage] imports $($unique.Count) : $(Split-Path $file -Leaf)"
    return $unique
}

# --- (Re)crée un stage vide ---
if (Test-Path -LiteralPath $Stage) {
    Remove-Item -LiteralPath $Stage -Recurse -Force
}
New-Item -ItemType Directory -Path $Stage | Out-Null

function Copy-ToStage([string]$source, [string]$destName) {
    $dest = Join-Path $Stage $destName
    if (Test-Path -LiteralPath $dest) { Remove-Item -LiteralPath $dest -Force }
    Copy-Item -LiteralPath $source -Destination $dest -Force
    Write-Output "[stage] + $destName"
}

# --- 1. Exécutable (build GNU) ---
$exe = Join-Path $TargetRelease "meeting-recorder-windows.exe"
if (-not (Test-Path -LiteralPath $exe)) {
    Write-Error "Exécutable introuvable : '$exe'. Lancez d'abord 'cargo build --release' (toolchain GNU x86_64-pc-windows-gnu)."
}
Copy-ToStage $exe "meeting-recorder-windows.exe"

# --- 2. onnxruntime (déjà copiées vers target/release en CI) ---
$ortDlls = Get-ChildItem -LiteralPath $TargetRelease -Filter "onnxruntime*.dll" -ErrorAction SilentlyContinue
if (-not $ortDlls) {
    Write-Error "onnxruntime*.dll introuvables dans '$TargetRelease' (étape CI 'Copie DLL ORT vers target/release' manquante ?)."
}
foreach ($dll in $ortDlls) { Copy-ToStage $dll.FullName $dll.Name }

# --- 3. Runtime MinGW (target/release d'abord, UCRT_BIN en repli) ---
foreach ($dll in @("libstdc++-6.dll", "libgcc_s_seh-1.dll", "libwinpthread-1.dll")) {
    $fromRelease = Join-Path $TargetRelease $dll
    $fromUcrt = Join-Path $UcrtBin $dll
    if (Test-Path -LiteralPath $fromRelease) { Copy-ToStage $fromRelease $dll }
    elseif (Test-Path -LiteralPath $fromUcrt) { Copy-ToStage $fromUcrt $dll }
    else { Write-Error "Runtime MinGW introuvable : '$dll' (ni dans '$TargetRelease' ni dans '$UcrtBin')." }
}

# --- 4. Closure GTK4/libadwaita : dépendances transitives via objdump ---
# Parcours en largeur : chaque DLL copiée depuis UCRT_BIN est à son tour analysée.
# CRT Visual C++ exigé par onnxruntime officiel (MSVCP140_1.dll importée par
# onnxruntime.dll) : absent de ucrt64/bin et non bundlable via MSYS2, présent dans
# %SystemRoot%\System32 sur les runners CI (VS installé). Repli limité à ces DLL
# CRT : déploiement app-local autorisé par la licence VC++ Redist.
$CrtSystemFallback = New-Object System.Collections.Generic.HashSet[string]([System.StringComparer]::OrdinalIgnoreCase)
@("msvcp140.dll", "msvcp140_1.dll", "msvcp140_2.dll", "vcruntime140.dll", "vcruntime140_1.dll") | ForEach-Object { [void]$CrtSystemFallback.Add($_) }
$copied = New-Object System.Collections.Generic.HashSet[string]([System.StringComparer]::OrdinalIgnoreCase)
Get-ChildItem -LiteralPath $Stage -Filter "*.dll" | ForEach-Object { [void]$copied.Add($_.Name) }
$queue = New-Object System.Collections.Generic.Queue[string]
Get-ChildItem -LiteralPath $Stage -Filter "*.dll" | ForEach-Object { $queue.Enqueue($_.FullName) }
$queue.Enqueue((Join-Path $Stage "meeting-recorder-windows.exe"))
function Expand-StageQueue {
    while ($queue.Count -gt 0) {
        $file = $queue.Dequeue()
        foreach ($dep in (Get-DllImports $file)) {
            if (Test-SystemDll $dep) { continue }          # fournie par Windows
            if ($copied.Contains($dep)) { continue }       # déjà au stage
            $src = Join-Path $UcrtBin $dep
            if (-not (Test-Path -LiteralPath $src)) {
                if ($CrtSystemFallback.Contains($dep)) {
                    $sysSrc = Join-Path (Join-Path $env:SystemRoot "System32") $dep
                    if (Test-Path -LiteralPath $sysSrc) {
                        $dest = Join-Path $Stage $dep
                        if (Test-Path -LiteralPath $dest) { Remove-Item -LiteralPath $dest -Force }
                        Copy-Item -LiteralPath $sysSrc -Destination $dest -Force
                        Write-Output "[stage] + $dep (CRT systeme)"
                        [void]$copied.Add($dep)
                        $queue.Enqueue((Join-Path $Stage $dep))
                        continue
                    }
                }
                Write-Error "DLL requise introuvable : '$dep' (importée par '$(Split-Path $file -Leaf)', absente de '$UcrtBin'). Installez le paquet MSYS2 UCRT64 correspondant."
            }
            Copy-ToStage $src $dep
            [void]$copied.Add($dep)
            $queue.Enqueue((Join-Path $Stage $dep))
        }
    }
}
Expand-StageQueue

# --- 5. Données GTK indispensables ---
# Schémas GSettings : sources XML copiées puis recompilées (gschemas.compiled).
$srcSchemas = Join-Path $UcrtRoot "share\glib-2.0\schemas"
if (-not (Test-Path -LiteralPath $srcSchemas)) {
    Write-Error "Schémas GSettings introuvables : '$srcSchemas' (paquet glib2 manquant ?)."
}
$dstSchemas = Join-Path $Stage "share\glib-2.0\schemas"
New-Item -ItemType Directory -Path $dstSchemas -Force | Out-Null
$xml = Get-ChildItem -LiteralPath $srcSchemas -Filter "*.gschema.xml" -ErrorAction SilentlyContinue
if (-not $xml) { Write-Error "Aucun *.gschema.xml dans '$srcSchemas'." }
foreach ($f in $xml) { Copy-Item -LiteralPath $f.FullName -Destination $dstSchemas -Force }
& $schemaCompiler $dstSchemas
if (-not (Test-Path -LiteralPath (Join-Path $dstSchemas "gschemas.compiled"))) {
    Write-Error "glib-compile-schemas n'a pas produit gschemas.compiled dans '$dstSchemas'."
}
Write-Output "[stage] + share/glib-2.0/schemas (recompilés)"

# Chargeurs d'images gdk-pixbuf (PNG/SVG des icônes Adwaita/GTK).
$loaderDirs = Get-ChildItem -LiteralPath (Join-Path $UcrtRoot "lib\gdk-pixbuf-2.0") -Directory -ErrorAction SilentlyContinue |
    ForEach-Object { Join-Path $_.FullName "loaders" } | Where-Object { Test-Path -LiteralPath $_ }
if (-not $loaderDirs) {
    Write-Error "Chargeurs gdk-pixbuf introuvables sous '$(Join-Path $UcrtRoot 'lib\gdk-pixbuf-2.0')' (paquet gdk-pixbuf2 manquant ?)."
}
foreach ($dir in $loaderDirs) {
    $versionDir = Split-Path (Split-Path $dir -Parent) -Leaf  # ex. 2.10.0
    $dst = Join-Path $Stage "lib\gdk-pixbuf-2.0\$versionDir\loaders"
    New-Item -ItemType Directory -Path $dst -Force | Out-Null
    Get-ChildItem -LiteralPath $dir -Filter "*.dll" | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination $dst -Force
    }
    Write-Output "[stage] + lib/gdk-pixbuf-2.0/$versionDir/loaders"
}

# Icônes minimales (hicolor + Adwaita, utilisées par GTK/libadwaita).
$srcIcons = Join-Path $UcrtRoot "share\icons"
if (-not (Test-Path -LiteralPath $srcIcons)) {
    Write-Error "Icônes GTK introuvables : '$srcIcons' (paquet adwaita-icon-theme manquant ?)."
}
Copy-Item -LiteralPath $srcIcons -Destination (Join-Path $Stage "share\icons") -Recurse -Force
Write-Output "[stage] + share/icons"

# --- 5b. Icône des raccourcis MSI (stage/app.ico, multi-tailles) ---
# L'exe n'embarque aucune icône (pas de winres) : Product.wxs pointe son
# <Icon Id="AppIcon"> vers ce .ico. Icône maison committée
# (packaging/windows/app.ico, 16/24/32/48/256) copiée telle quelle :
# déterministe, zéro dépendance CI (remplace le scraping Adwaita, dont le
# paquet MSYS2 ne fournit aucun PNG micro non-symbolic). ÉCHEC EXPLICITE
# si le fichier est absent.
$iconSrc = Join-Path $PSScriptRoot "app.ico"
if (-not (Test-Path -LiteralPath $iconSrc)) {
    Write-Error "Icône introuvable : '$iconSrc' (fichier committé manquant ?)."
}
Copy-ToStage $iconSrc "app.ico"

# --- 6. ffmpeg + ffprobe (src/export.rs : à côté de l'exe ou au PATH) ---
function Find-LocalTool([string]$name) {
    $hits = @()
    $inRelease = Join-Path $TargetRelease $name
    if (Test-Path -LiteralPath $inRelease) { $hits += $inRelease }
    try {
        $onPath = (Get-Command $name -ErrorAction Stop).Source
        if ($onPath -and (Test-Path -LiteralPath $onPath)) { $hits += $onPath }
    } catch {}
    $inUcrt = Join-Path $UcrtBin $name
    if (Test-Path -LiteralPath $inUcrt) { $hits += $inUcrt }
    return $hits | Select-Object -Unique
}
# Mode CI strict : si $env:MR_FFMPEG_PINNED_ONLY -eq "1", ignore Find-LocalTool
# (pas de copie depuis le PATH/TargetRelease) et télécharge toujours le ZIP pinné vérifié.
# Comportement dev local (variable absente ou différente de "1") : repli PATH conservé,
# mais le téléchargement reste le ZIP pinné vérifié.
# Note : Authenticode n'est pas vérifié car l'amont GyanD ne signe pas ses binaires ;
# le pin de hash SHA-256 + taille ci-dessous est le contrôle (bump via PR relue).
# Le ZIP est téléchargé+vérifié+extrait UNE fois (mémorisé dans $ffmpegExtracted)
# : la boucle ne fait plus que Copy-ToStage depuis l'arbre extrait.
$ffmpegExtracted = $null
foreach ($tool in @("ffmpeg.exe", "ffprobe.exe")) {
    if (Test-Path -LiteralPath (Join-Path $Stage $tool)) { continue }
    if ($env:MR_FFMPEG_PINNED_ONLY -ne "1") {
        $local = Find-LocalTool $tool | Select-Object -First 1
        if ($local) {
            Copy-ToStage $local $tool
            continue
        }
    } else {
        Write-Output "[stage] MR_FFMPEG_PINNED_ONLY=1 : $tool local ignoré, téléchargement du ZIP pinné vérifié."
    }
    # Secours : build GyanD FFmpeg essentials pinné (contient bin/ffmpeg.exe + bin/ffprobe.exe).
    if ($ffmpegExtracted -and (Test-Path -LiteralPath (Join-Path $ffmpegExtracted $tool))) {
        Copy-ToStage (Join-Path $ffmpegExtracted $tool) $tool
        continue
    }
    Write-Output "[stage] $tool absent localement : téléchargement $FfmpegZipUrl ..."
    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) "meeting-recorder-ffmpeg"
    New-Item -ItemType Directory -Path $tmp -Force | Out-Null
    $zip = Join-Path $tmp "ffmpeg-9.0.2-essentials_build.zip"
    try {
        Invoke-WebRequest -Uri $FfmpegZipUrl -OutFile $zip
    } catch {
        Write-Error "Téléchargement FFmpeg impossible ($FfmpegZipUrl) et $tool introuvable localement. Détail : $($_.Exception.Message)"
    }
    # Fail-closed AVANT Expand-Archive : taille + SHA-256, throw bloquant si différent.
    $actualSize = (Get-Item -LiteralPath $zip).Length
    if ($actualSize -ne $FfmpegZipSize) {
        throw "ZIP FFmpeg rejeté : taille $actualSize o (attendu $FfmpegZipSize o). Supprimez le cache TEMP/meeting-recorder-ffmpeg et relancez."
    }
    $actualHash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $FfmpegZipSha256.ToLowerInvariant()) {
        throw "ZIP FFmpeg rejeté : SHA-256 $actualHash (attendu $($FfmpegZipSha256.ToLowerInvariant())). Supprimez le cache TEMP/meeting-recorder-ffmpeg et relancez."
    }
    $unzipped = Join-Path $tmp "ffmpeg-9.0.2-essentials_build"
    if (Test-Path -LiteralPath $unzipped) { Remove-Item -LiteralPath $unzipped -Recurse -Force }
    Expand-Archive -LiteralPath $zip -DestinationPath $unzipped -Force
    $bin = Get-ChildItem -LiteralPath $unzipped -Filter $tool -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $bin) {
        Write-Error "$tool absent de l'archive FFmpeg ($FfmpegZipUrl)."
    }
    $ffmpegExtracted = Split-Path $bin.FullName -Parent
    Copy-ToStage $bin.FullName $tool
}
# Smoke test APRÈS extraction du ZIP vérifié (ordre : download → hash → extract → stage → smoke) :
# le ffmpeg embarqué doit démarrer et afficher sa version.
$ffVersion = (& (Join-Path $Stage "ffmpeg.exe") -version 2>$null | Select-Object -First 1)
if (-not $ffVersion) { Write-Error "Le ffmpeg du stage ne démarre pas (binaire corrompu ?)." }
Write-Output "[stage] ffmpeg : $ffVersion"
$fpVersion = (& (Join-Path $Stage "ffprobe.exe") -version 2>$null | Select-Object -First 1)
if (-not $fpVersion) { Write-Error "Le ffprobe du stage ne démarre pas (binaire corrompu ?)." }
Write-Output "[stage] ffprobe : $fpVersion"

# --- 6b. Seconde passe : binaires arrivés après la BFS principale ---
# ffmpeg/ffprobe (et toute DLL copiée sous stage/ après coup) doivent voir
# leurs imports résolus avec la même règle système et la même erreur explicite.
# -Include non fiable sous Windows PowerShell 5.1 avec -LiteralPath sur un dossier : filtre par extension.
Get-ChildItem -LiteralPath $Stage -Recurse -File |
    Where-Object { ($_.Extension -eq '.exe' -or $_.Extension -eq '.dll') -and -not $copied.Contains($_.Name) -and $_.Name -ne "meeting-recorder-windows.exe" } |
    ForEach-Object { [void]$copied.Add($_.Name); $queue.Enqueue($_.FullName) }
Expand-StageQueue

# --- 7. Vérification finale explicite ---
$required = @("meeting-recorder-windows.exe", "onnxruntime.dll",
    "libstdc++-6.dll", "libgcc_s_seh-1.dll", "libwinpthread-1.dll",
    "ffmpeg.exe", "ffprobe.exe")
foreach ($f in $required) {
    if (-not (Test-Path -LiteralPath (Join-Path $Stage $f))) {
        Write-Error "Stage incomplet : '$f' manquant (ne devrait jamais arriver ici)."
    }
}
if (-not (Test-Path -LiteralPath (Join-Path $Stage "share\glib-2.0\schemas\gschemas.compiled"))) {
    Write-Error "Stage incomplet : gschemas.compiled manquant."
}
$nDll = (Get-ChildItem -LiteralPath $Stage -Filter "*.dll" | Measure-Object).Count
Write-Output "[stage] OK : $Stage ($nDll DLL, exe + ORT + MinGW + closure GTK + ffmpeg/ffprobe + data GTK)"
