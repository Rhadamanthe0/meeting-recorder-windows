# Tests hors réseau : aucune exécution de candle.exe, light.exe ni heat.exe.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'verify-wix-preinstalled.ps1')
$fixture = Join-Path ([IO.Path]::GetTempPath()) ('wix-integrity-' + [guid]::NewGuid().ToString('N'))
$names = @('candle.exe', 'light.exe', 'heat.exe')
$expected = @{}
try {
    [IO.Directory]::CreateDirectory($fixture) | Out-Null
    foreach ($name in $names) {
        $tool = Join-Path $fixture $name
        [IO.File]::WriteAllText($tool, "trusted-fixture-$name")
        $expected[$name] = (Get-FileHash -LiteralPath $tool -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    if (-not (Test-CIWiXPreinstalled -Directory $fixture -ExpectedSha256 $expected)) {
        throw 'WiX sain refusé'
    }
    Write-Output 'PASS: les trois fichiers intacts sont acceptés'

    # Un faux exécutable qui affiche la bonne version ne doit pas passer.
    [IO.File]::WriteAllText((Join-Path $fixture 'candle.exe'), 'Windows Installer XML Toolset Compiler version 3.14.1.8722')
    if (Test-CIWiXPreinstalled -Directory $fixture -ExpectedSha256 $expected) {
        throw 'Usurpation de version WiX acceptée'
    }
    Write-Output 'PASS: une version 3.14.1.8722 falsifiée est refusée avant exécution'

    [IO.File]::WriteAllText((Join-Path $fixture 'candle.exe'), 'trusted-fixture-candle.exe')
    [IO.File]::WriteAllText((Join-Path $fixture 'light.exe'), 'modified-light.exe')
    if (Test-CIWiXPreinstalled -Directory $fixture -ExpectedSha256 $expected) {
        throw 'light.exe altéré accepté'
    }
    Write-Output 'PASS: light.exe altéré est refusé'

    [IO.File]::WriteAllText((Join-Path $fixture 'light.exe'), 'trusted-fixture-light.exe')
    [IO.File]::WriteAllText((Join-Path $fixture 'heat.exe'), 'modified-heat.exe')
    if (Test-CIWiXPreinstalled -Directory $fixture -ExpectedSha256 $expected) {
        throw 'heat.exe altéré accepté'
    }
    Write-Output 'PASS: heat.exe altéré est refusé'

    Remove-Item -LiteralPath (Join-Path $fixture 'heat.exe') -Force
    if (Test-CIWiXPreinstalled -Directory $fixture -ExpectedSha256 $expected) {
        throw 'Dossier WiX incomplet accepté'
    }
    if (Test-CIWiXPreinstalled -Directory (Join-Path $fixture 'missing') -ExpectedSha256 $expected) {
        throw 'Dossier WiX absent accepté'
    }
    Write-Output 'PASS: fichier ou dossier absent est refusé'

    foreach ($name in $names) {
        [IO.File]::WriteAllText((Join-Path $fixture $name), "trusted-fixture-$name")
    }
    # Le contrôle de production ne doit pas accepter des binaires synthétiques.
    if (Test-CIWiXPreinstalled -Directory $fixture) {
        throw 'Les empreintes de production ont été remplacées par les fixtures'
    }
    Write-Output 'PASS: les SHA-256 de production sont bien épinglés'

    # API GitHub : digest officiel si disponible et concordant (sans réseau).
    $script:wixApiMode = 'match'
    function Invoke-RestMethod {
        [CmdletBinding()]
        param([uri]$Uri, [string]$Method, [hashtable]$Headers, [int]$TimeoutSec)
        if ($Uri.AbsoluteUri -cne 'https://api.github.com/repos/wixtoolset/wix3/releases/tags/wix3141rtm') {
            throw 'URL GitHub non épinglée.'
        }
        if ($script:wixApiMode -eq 'offline') { throw 'Fixture API offline' }
        $digest = switch ($script:wixApiMode) {
            'match' { 'sha256:6ac824e1642d6f7277d0ed7ea09411a508f6116ba6fae0aa5f2c7daa2ff43d31' }
            'changed' { 'sha256:' + ('f' * 64) }
            'missing' { $null }
            'malformed' { 'sha512:abcd' }
            default { throw 'Fixture API mode unexpected' }
        }
        [pscustomobject]@{
            tag_name = 'wix3141rtm'
            assets = @([pscustomobject]@{
                name = 'wix314-binaries.zip'
                id = 158158052
                size = 41297555
                digest = $digest
            })
        }
    }
    $pinnedArchive = '6ac824e1642d6f7277d0ed7ea09411a508f6116ba6fae0aa5f2c7daa2ff43d31'
    foreach ($mode in @('match', 'missing', 'malformed', 'offline')) {
        $script:wixApiMode = $mode
        if ((Get-CIWiXTrustedArchiveSha256) -cne $pinnedArchive) {
            throw "Empreinte inattendue pour métadonnées $mode"
        }
        Write-Output "PASS: métadonnées en ligne $mode"
    }
    $script:wixApiMode = 'changed'
    $changedWasRejected = $false
    try { $null = Get-CIWiXTrustedArchiveSha256 } catch { $changedWasRejected = $true }
    if (-not $changedWasRejected) { throw 'Une nouvelle empreinte non approuvée a été acceptée.' }
    Write-Output 'PASS: digest officiel différent refusé (pas de mise à jour automatique)'

    # Cache sain, puis réparation après corruption ; jamais exécuter un ZIP non vérifié.
    $script:wixDownloads = 0
    $script:wixDownloadedPayload = 'trusted sample archive'
    function Invoke-CIAssetDownload {
        param([uri]$Uri, [string]$OutFile)
        if ($Uri.AbsoluteUri -cne 'https://github.com/wixtoolset/wix3/releases/download/wix3141rtm/wix314-binaries.zip') {
            throw 'URL WiX inattendue'
        }
        $script:wixDownloads++
        [IO.File]::WriteAllText($OutFile, $script:wixDownloadedPayload)
    }
    $sample = [Text.Encoding]::UTF8.GetBytes('trusted sample archive')
    $sampleSha256 = [BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($sample)).Replace('-', '').ToLowerInvariant()
    $cacheDirectory = Join-Path $fixture 'archive-cache'
    $uri = 'https://github.com/wixtoolset/wix3/releases/download/wix3141rtm/wix314-binaries.zip'
    $settings = @{CacheDirectory=$cacheDirectory;ExpectedSha256=$sampleSha256;ExpectedLength=[long]$sample.Length;Uri=[uri]$uri}
    $first = Restore-CIWiXVerifiedArchive @settings
    if (-not $first.Refreshed -or $script:wixDownloads -ne 1) { throw 'Premier téléchargement/cache échoué' }
    Write-Output 'PASS: cache absent téléchargé et enregistré'
    $second = Restore-CIWiXVerifiedArchive @settings
    if ($second.Refreshed -or $script:wixDownloads -ne 1) { throw 'Cache sain retéléchargé à tort' }
    Write-Output 'PASS: cache valide réutilisé sans réseau'
    [IO.File]::WriteAllText($first.Path, 'malicious cached archive')
    $third = Restore-CIWiXVerifiedArchive @settings
    if (-not $third.Refreshed -or $script:wixDownloads -ne 2 -or
        -not (Test-CIWiXArchive -Path $third.Path -ExpectedSha256 $sampleSha256 -ExpectedLength $sample.Length)) {
        throw 'Cache corrompu non réparé'
    }
    Write-Output 'PASS: cache altéré remplacé par téléchargement vérifié'
    [IO.File]::WriteAllText($first.Path, 'malicious cached archive')
    $script:wixDownloadedPayload = 'invalid remote archive'
    $badDownloadRejected = $false
    try { $null = Restore-CIWiXVerifiedArchive @settings } catch { $badDownloadRejected = $true }
    if (-not $badDownloadRejected -or $script:wixDownloads -ne 3) {
        throw 'Téléchargement incorrect accepté'
    }
    if (Test-CIWiXArchive -Path $first.Path -ExpectedSha256 $sampleSha256 -ExpectedLength $sample.Length) {
        throw 'Téléchargement incorrect enregistré dans le cache'
    }
    Write-Output 'PASS: téléchargement corrompu jamais promu dans le cache'
} finally {
    if (Test-Path -LiteralPath $fixture) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
