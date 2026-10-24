# Preuve d'outils scellés : optionnelle. Les runners hébergés n'en fournissent
# aucune ; l'empreinte utilise alors le marqueur explicite
# '<absent:legacy-reference>' (échec si le fichier présent est invalide).
param([string]$ToolsProofPath = 'C:\RhadaCI\tools.json')
$ErrorActionPreference = 'Stop'

function Read-PacmanInventory([string]$Pacman) {
    $lines = @(& $Pacman -Q)
    if ($LASTEXITCODE -ne 0) { throw "Inventaire pacman impossible : $LASTEXITCODE" }
    return $lines
}

function Set-RustCacheNativeTools([string]$ToolsProofPath = 'C:\RhadaCI\tools.json') {
    if (-not $env:GITHUB_ENV) { throw 'GITHUB_ENV absent' }
    if (-not $env:UCRT_BIN -or -not (Test-Path -LiteralPath $env:UCRT_BIN -PathType Container)) {
        throw 'UCRT_BIN absent ou invalide'
    }
    $bin = (Resolve-Path -LiteralPath $env:UCRT_BIN).ProviderPath
    $ucrt = Split-Path -Parent $bin
    if ((Split-Path -Leaf $bin) -ne 'bin' -or (Split-Path -Leaf $ucrt) -ne 'ucrt64') {
        throw 'UCRT_BIN doit désigner la pile MSYS2 UCRT64 effective'
    }
    $pacman = Join-Path (Split-Path -Parent $ucrt) 'usr/bin/pacman.exe'
    if (-not (Test-Path -LiteralPath $pacman -PathType Leaf)) { throw 'pacman effectif absent' }
    # Lecture seule : aucune synchronisation ni installation de paquets.
    $packages = @{}
    foreach ($line in @(Read-PacmanInventory $pacman)) {
        if ($line -notmatch '^([A-Za-z0-9@._+\-]+)\s+(\S+)$' -or $packages.ContainsKey($Matches[1])) {
            throw "Inventaire pacman invalide : $line"
        }
        $packages[$Matches[1]] = $Matches[2]
    }
    if ($packages.Count -eq 0) { throw 'Inventaire pacman vide' }
    [string[]]$inventory = @($packages.Keys | ForEach-Object { "$_ $($packages[$_])" })
    [Array]::Sort($inventory, [StringComparer]::Ordinal)

    $proof = '<absent:legacy-reference>'
    if (Test-Path -LiteralPath $ToolsProofPath) {
        if (-not (Test-Path -LiteralPath $ToolsProofPath -PathType Leaf)) { throw 'Preuve des outils invalide' }
        $proof = Get-Content -LiteralPath $ToolsProofPath -Raw -Encoding utf8
        $parsed = ConvertFrom-Json -InputObject $proof -ErrorAction Stop
        if ($parsed -isnot [PSCustomObject] -or $parsed.toolManifestSha256 -isnot [string] -or
            $parsed.toolManifestSha256 -notmatch '^[a-fA-F0-9]{64}$') {
            throw 'Preuve des outils scellés invalide'
        }
    }
    $options = [ordered]@{}
    foreach ($name in @('GGML_NATIVE', 'LIBCLANG_PATH', 'PKG_CONFIG_PATH')) {
        $value = [Environment]::GetEnvironmentVariable($name)
        $options[$name] = if ($null -eq $value) { '<unset>' } else { $value }
    }
    $payload = [ordered]@{ schema = 1; ucrtBin = $bin; packages = $inventory; toolsProof = $proof; options = $options }
    $bytes = [Text.Encoding]::UTF8.GetBytes(($payload | ConvertTo-Json -Depth 4 -Compress))
    $digest = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant()
    "RUST_CACHE_NATIVE_TOOLS=$digest" | Out-File -LiteralPath $env:GITHUB_ENV -Encoding utf8 -Append
    Write-Output "[ci] Cache natif : $($packages.Count) paquets ; preuve scellée=$($proof -ne '<absent:legacy-reference>') ; SHA256=$digest"
}

if ($MyInvocation.InvocationName -ne '.') { Set-RustCacheNativeTools -ToolsProofPath $ToolsProofPath }
