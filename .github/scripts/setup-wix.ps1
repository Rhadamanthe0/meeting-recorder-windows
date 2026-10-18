# WiX 3.14.1 portable : aucune installation globale, ni privilège admin.
$ErrorActionPreference = 'Stop'
# Vérifier le hash en ligne quand GitHub le publie ; à défaut, utiliser
# exclusivement le hash du ZIP officiel épinglé par le dépôt.
. (Join-Path $PSScriptRoot 'verify-wix-preinstalled.ps1')
. (Join-Path $PSScriptRoot 'download-ci-asset.ps1')
$expectedArchiveSha256 = Get-CIWiXTrustedArchiveSha256

$baseWix = 'C:\Program Files (x86)\WiX Toolset v3.14\bin'
if (Test-CIWiXPreinstalled -Directory $baseWix) {
    foreach ($name in @('candle.exe', 'light.exe', 'heat.exe')) {
        $help = & (Join-Path $baseWix $name) -?
        if ($LASTEXITCODE -ne 0 -or -not ($help -match '\b3\.14\.1\.8722\b')) {
            throw "WiX scellé invalide : $name (3.14.1.8722 attendu)"
        }
        $help | Select-Object -First 2
    }
    $baseWix >> $env:GITHUB_PATH
    Write-Output "[ci] WiX 3.14.1 préinstallé vérifié (SHA-256) : $baseWix"
    return
}

# VM éphémère : RUNNER_TEMP n'est pas persistant. La CI peut restaurer et
# sauvegarder le ZIP vérifié via GitHub Actions entre deux VM.
$cacheDirectory = Join-Path $env:RUNNER_TEMP 'meeting-recorder-wix-cache'
$archive = Restore-CIWiXVerifiedArchive -CacheDirectory $cacheDirectory -ExpectedSha256 $expectedArchiveSha256 -Uri 'https://github.com/wixtoolset/wix3/releases/download/wix3141rtm/wix314-binaries.zip'
if ($archive.Refreshed -and $env:GITHUB_OUTPUT) {
    "cache_refreshed=true" >> $env:GITHUB_OUTPUT
}
$wixDir = Join-Path $env:RUNNER_TEMP ('meeting-recorder-wix-verified-' + [guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $wixDir
Expand-Archive -LiteralPath $archive.Path -DestinationPath $wixDir
# Contrôle les exécutables extraits avant le premier lancement.
if (-not (Test-CIWiXPreinstalled -Directory $wixDir)) {
    throw 'Outils WiX extraits non conformes : aucun exécutable lancé.'
}
foreach ($name in @('candle.exe', 'light.exe', 'heat.exe')) {
    $tool = Join-Path $wixDir $name
    $help = & $tool -?
    if ($LASTEXITCODE -ne 0 -or -not ($help -match '\b3\.14\.1\.8722\b')) {
        throw "WiX extrait invalide : $name (3.14.1.8722 attendu)"
    }
    $help | Select-Object -First 2
}
$wixDir >> $env:GITHUB_PATH
Write-Output "[ci] WiX 3.14.1 vérifié depuis archive mise en cache : $wixDir"
