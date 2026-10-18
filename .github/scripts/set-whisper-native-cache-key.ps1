param([string]$RustToolsSha256)
$ErrorActionPreference = 'Stop'

# Ne modifie aucun environnement de compilation : ce cache reste indépendant
# du cache Cargo. Tous les fichiers sont lus, jamais les credentials Cargo.
function Get-WhisperNativeCacheKey {
    param(
        [string]$RustToolsSha256,
        [string]$Workspace = $env:GITHUB_WORKSPACE,
        [System.Collections.IDictionary]$Environment = [Environment]::GetEnvironmentVariables()
    )
    if ($RustToolsSha256 -notmatch '^[a-fA-F0-9]{64}$' -or
        $Environment['RUST_CACHE_NATIVE_TOOLS'] -notmatch '^[a-fA-F0-9]{64}$') {
        throw 'Empreintes des outils Rust et natifs requises'
    }
    function Assert-NoLink([string]$Path) {
        $cursor = [IO.Path]::GetFullPath($Path)
        while ($cursor) {
            if (Test-Path -LiteralPath $cursor) {
                $item = Get-Item -LiteralPath $cursor -Force
                if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Lien inattendu dans les entrées du cache' }
            }
            $cursor = Split-Path -Parent $cursor
        }
    }
    if (-not $Workspace -or -not (Test-Path -LiteralPath $Workspace -PathType Container)) { throw 'Workspace invalide' }
    Assert-NoLink $Workspace
    $root = (Resolve-Path -LiteralPath $Workspace).ProviderPath
    $entries = [Collections.Generic.List[object]]::new()
    function Collect-Inputs([string]$Path, [string]$Name) {
        $item = Get-Item -LiteralPath $Path -Force
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Lien inattendu dans les sources du cache' }
        $entries.Add(@{ name = $Name; path = $item.FullName; directory = $item.PSIsContainer })
        if ($item.PSIsContainer) {
            foreach ($child in Get-ChildItem -LiteralPath $Path -Force) {
                Collect-Inputs $child.FullName ($Name + '/' + $child.Name)
            }
        }
    }
    foreach ($name in @('third-party', '.github', 'Cargo.toml', 'Cargo.lock')) {
        Collect-Inputs (Join-Path $root $name) $name
    }
    if (-not (Test-Path -LiteralPath (Join-Path $root '.github/workflows/windows.yml') -PathType Leaf)) {
        throw 'Workflow Windows absent'
    }
    $configDirs = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $cursor = $root
    while ($cursor) {
        [void]$configDirs.Add((Join-Path $cursor '.cargo'))
        $cursor = Split-Path -Parent $cursor
    }
    $cargoHome = $Environment['CARGO_HOME']
    if (-not $cargoHome) { $cargoHome = Join-Path ([Environment]::GetFolderPath('UserProfile')) '.cargo' }
    [void]$configDirs.Add([IO.Path]::GetFullPath($cargoHome))
    $cacheable = $true
    foreach ($directory in $configDirs) {
        Assert-NoLink $directory
        foreach ($name in @('config', 'config.toml')) {
            $path = Join-Path $directory $name
            if (Test-Path -LiteralPath $path) {
                if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw 'Configuration Cargo invalide' }
                Assert-NoLink $path
                # Les configurations peuvent inclure d'autres fichiers ou
                # sélectionner des outils externes. Compiler normalement, sans
                # ce cache facultatif, plutôt que réimplémenter Cargo/TOML.
                $cacheable = $false
                Collect-Inputs $path ('cargo-config:' + [IO.Path]::GetFullPath($path))
            }
        }
    }
    $hasher = [Security.Cryptography.IncrementalHash]::CreateHash([Security.Cryptography.HashAlgorithmName]::SHA256)
    function Add-Field([string]$Name, [byte[]]$Content) {
        $nameBytes = [Text.Encoding]::UTF8.GetBytes($Name)
        $hasher.AppendData([BitConverter]::GetBytes([long]$nameBytes.Length))
        $hasher.AppendData($nameBytes)
        $hasher.AppendData([BitConverter]::GetBytes([long]$Content.Length))
        $hasher.AppendData($Content)
    }
    try {
        Add-Field 'workspace' ([Text.Encoding]::UTF8.GetBytes($root))
        Add-Field 'rust-tools' ([Text.Encoding]::UTF8.GetBytes($RustToolsSha256.ToLowerInvariant()))
        [string[]]$names = @($Environment.Keys | Where-Object {
            $_ -match '^(CARGO|RUST|CC|CFLAGS|CPP|CXX|CMAKE|GGML|WHISPER|LIBCLANG|BINDGEN|PKG_CONFIG|ARFLAGS|AR($|_)|RANLIB($|_)|NM($|_)|LD($|_)|LDFLAGS|HOST_|TARGET_|MSYS|MINGW|SOURCE_DATE_EPOCH$)'
        })
        [Array]::Sort($names, [StringComparer]::Ordinal)
        foreach ($name in $names) { Add-Field ('env:' + $name) ([Text.Encoding]::UTF8.GetBytes([string]$Environment[$name])) }
        $byName = @{}
        foreach ($entry in $entries) { $byName[$entry.name] = $entry }
        [string[]]$paths = @($byName.Keys)
        [Array]::Sort($paths, [StringComparer]::Ordinal)
        foreach ($path in $paths) {
            $entry = $byName[$path]
            if ($entry.directory) { Add-Field ('directory:' + $path) ([byte[]]@()) }
            else { Add-Field ('file:' + $path) ([IO.File]::ReadAllBytes($entry.path)) }
        }
        return [PSCustomObject]@{
            Key = 'windows-gnu-whisper-out-v1-' + [Convert]::ToHexString($hasher.GetHashAndReset()).ToLowerInvariant()
            Cacheable = $cacheable
            Reason = if ($cacheable) { '' } else { 'cargo-configuration-present' }
        }
    } finally { $hasher.Dispose() }
}

if ($MyInvocation.InvocationName -ne '.') {
    if (-not $env:GITHUB_OUTPUT) { throw 'GITHUB_OUTPUT absent' }
    $result = Get-WhisperNativeCacheKey -RustToolsSha256 $RustToolsSha256
    Write-Output "Whisper native cache: cacheable=$($result.Cacheable.ToString().ToLowerInvariant()); reason=$($result.Reason)"
    @("key=$($result.Key)", "cacheable=$($result.Cacheable.ToString().ToLowerInvariant())", "reason=$($result.Reason)") |
        Out-File -LiteralPath $env:GITHUB_OUTPUT -Encoding utf8 -Append
}
