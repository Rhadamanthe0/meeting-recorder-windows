# Fixtures locales : aucun réseau ni programme natif n'est lancé.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'set-rust-cache-native-tools.ps1')
$fixture = Join-Path ([IO.Path]::GetTempPath()) ('native-cache-' + [Guid]::NewGuid().ToString('N'))
$saved = @{}
foreach ($name in @('GITHUB_ENV', 'UCRT_BIN', 'GGML_NATIVE', 'LIBCLANG_PATH', 'PKG_CONFIG_PATH')) {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name)
}
function Read-PacmanInventory([string]$Pacman) {
    if ($script:inventoryError) { throw 'pacman a échoué' }
    if ($Pacman -ne (Join-Path $fixture 'usr/bin/pacman.exe')) { throw 'Mauvaise racine pacman' }
    return $script:inventoryFixture
}
function Fingerprint {
    Remove-Item -LiteralPath $env:GITHUB_ENV -ErrorAction SilentlyContinue
    Set-RustCacheNativeTools -ToolsProofPath $proofPath | Out-Null
    $line = Get-Content -LiteralPath $env:GITHUB_ENV
    if ($line -notmatch '^RUST_CACHE_NATIVE_TOOLS=[a-f0-9]{64}$') { throw 'Export invalide' }
    return $line
}
function Must-Fail([scriptblock]$Action) {
    $failed = $false
    try { & $Action | Out-Null } catch { $failed = $true }
    if (-not $failed) { throw 'Un état invalide a été accepté' }
    if (Test-Path -LiteralPath $env:GITHUB_ENV) { throw 'Empreinte publiée après erreur' }
}
try {
    $env:UCRT_BIN = Join-Path $fixture 'ucrt64/bin'
    New-Item -ItemType Directory -Path $env:UCRT_BIN, (Join-Path $fixture 'usr/bin') -Force | Out-Null
    New-Item -ItemType File -Path (Join-Path $fixture 'usr/bin/pacman.exe') | Out-Null
    $env:GITHUB_ENV = Join-Path $fixture 'github-env'
    $env:GGML_NATIVE = 'OFF'
    $env:LIBCLANG_PATH = $env:UCRT_BIN
    $env:PKG_CONFIG_PATH = Join-Path $fixture 'ucrt64/lib/pkgconfig'
    $proofPath = Join-Path $fixture 'tools.json'
    $validProof = '{"toolManifestSha256":"' + ('a' * 64) + '","gtk4":"4.24.1"}'
    Set-Content -LiteralPath $proofPath -Value $validProof -NoNewline
    $script:inventoryFixture = @('mingw-w64-ucrt-x86_64-gcc 16.2.0-1', 'bash 5.3.3-1')
    $baseline = Fingerprint
    if ((Fingerprint) -ne $baseline) { throw 'Empreinte non déterministe' }
    [Array]::Reverse($script:inventoryFixture)
    if ((Fingerprint) -ne $baseline) { throw 'Ordre pacman significatif' }
    $script:inventoryFixture[0] = 'bash 5.3.4-1'
    if ((Fingerprint) -eq $baseline) { throw 'Version ignorée' }
    $script:inventoryFixture[0] = 'bash 5.3.3-1'
    foreach ($name in @('GGML_NATIVE', 'LIBCLANG_PATH', 'PKG_CONFIG_PATH')) {
        $previous = [Environment]::GetEnvironmentVariable($name)
        [Environment]::SetEnvironmentVariable($name, $previous + '-changed')
        if ((Fingerprint) -eq $baseline) { throw "Option ignorée : $name" }
        [Environment]::SetEnvironmentVariable($name, $previous)
    }
    Set-Content -LiteralPath $proofPath -Value ($validProof.Replace('4.24.1', '4.24.2')) -NoNewline
    if ((Fingerprint) -eq $baseline) { throw 'Preuve ignorée' }
    Remove-Item -LiteralPath $proofPath
    $legacy = Fingerprint
    if ($legacy -eq $baseline -or (Fingerprint) -ne $legacy) { throw 'Absence de preuve non explicite' }
    foreach ($invalid in @('', '{', '{}', '[]', '{"toolManifestSha256":"incorrect"}',
        ('{"toolManifestSha256":["' + ('a' * 64) + '"]}'))) {
        Set-Content -LiteralPath $proofPath -Value $invalid -NoNewline
        Must-Fail { Fingerprint }
    }
    Set-Content -LiteralPath $proofPath -Value $validProof -NoNewline
    function Get-Content { throw 'Preuve illisible' }
    try { Must-Fail { Fingerprint } } finally { Remove-Item Function:\Get-Content }
    foreach ($invalidInventory in @(@(), @(''), @('malformed'), @('bash 1', 'bash 2'))) {
        $script:inventoryFixture = $invalidInventory
        Must-Fail { Fingerprint }
    }
    $script:inventoryError = $true
    Must-Fail { Fingerprint }
    $script:inventoryError = $false
    Remove-Item -LiteralPath (Join-Path $fixture 'usr/bin/pacman.exe')
    Must-Fail { Fingerprint }
    Write-Output 'OK: clé native déterministe, versions/options/preuve couvertes, erreurs refusées'
} finally {
    foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
    Remove-Item -LiteralPath $fixture -Recurse -Force -ErrorAction SilentlyContinue
}
