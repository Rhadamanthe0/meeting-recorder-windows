$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'set-whisper-native-cache-key.ps1')
$fixture = Join-Path ([IO.Path]::GetTempPath()) ('whisper-key-' + [Guid]::NewGuid().ToString('N'))
$originalNative = $env:RUST_CACHE_NATIVE_TOOLS
$originalOutput = $env:GITHUB_OUTPUT
$originalWorkspace = $env:GITHUB_WORKSPACE
$originalCargoHome = $env:CARGO_HOME
function Must-Fail([scriptblock]$Action) {
    $failed = $false
    try { & $Action | Out-Null } catch { $failed = $true }
    if (-not $failed) { throw 'Une entrée invalide a été acceptée' }
}
try {
    $workspace = Join-Path $fixture 'repo'
    $cargoHome = Join-Path $fixture 'cargo-home'
    New-Item -ItemType Directory -Force -Path (Join-Path $workspace 'third-party/whisper/src'),
        (Join-Path $workspace '.github/workflows'), (Join-Path $workspace '.github/scripts'),
        (Join-Path $workspace '.cargo'), (Join-Path $fixture '.cargo'), $cargoHome | Out-Null
    $source = Join-Path $workspace 'third-party/whisper/src/lib.rs'
    Set-Content -LiteralPath $source -Value 'source' -NoNewline
    foreach ($name in @('Cargo.toml', 'Cargo.lock', '.github/workflows/windows.yml', '.github/scripts/check.ps1')) {
        Set-Content -LiteralPath (Join-Path $workspace $name) -Value 'fixture' -NoNewline
    }
    $configPaths = @((Join-Path $workspace '.cargo/config.toml'),
        (Join-Path $fixture '.cargo/config'), (Join-Path $cargoHome 'config.toml'))
    $rust = 'a' * 64
    $options = @{ RUST_CACHE_NATIVE_TOOLS = ('b' * 64); CARGO_HOME = $cargoHome; GGML_NATIVE = 'OFF' }
    function Key { (Get-WhisperNativeCacheKey -RustToolsSha256 $rust -Workspace $workspace -Environment $options).Key }
    $baseline = Key
    if ($baseline -notmatch '^windows-gnu-whisper-out-v1-[a-f0-9]{64}$' -or (Key) -ne $baseline) { throw 'Clé invalide ou instable' }
    $reordered = @{ GGML_NATIVE = 'OFF'; CARGO_HOME = $cargoHome; RUST_CACHE_NATIVE_TOOLS = ('b' * 64) }
    if ((Get-WhisperNativeCacheKey $rust $workspace $reordered).Key -ne $baseline) { throw 'Ordre environnement significatif' }
    # Un config Cargo dans un ancêtre réel du dossier temporaire reste valide.
    $simpleInputs = Get-WhisperNativeCacheKey $rust $workspace $options
    if (-not $simpleInputs.Cacheable -and $simpleInputs.Reason -ne 'cargo-configuration-present') { throw 'Désactivation inattendue' }
    function Must-Change {
        if ((Key) -eq $baseline) { throw 'Modification ignorée' }
    }
    Set-Content -LiteralPath $source -Value 'changed' -NoNewline
    Must-Change
    Set-Content -LiteralPath $source -Value 'source' -NoNewline
    Rename-Item -LiteralPath $source -NewName 'renamed.rs'
    Must-Change
    Rename-Item -LiteralPath (Join-Path (Split-Path $source) 'renamed.rs') -NewName 'lib.rs'
    Remove-Item -LiteralPath $source
    Must-Change
    Set-Content -LiteralPath $source -Value 'source' -NoNewline
    foreach ($config in $configPaths) {
        Set-Content -LiteralPath $config -Value '[build]' -NoNewline
        Must-Change
        $unsupported = Get-WhisperNativeCacheKey $rust $workspace $options
        if ($unsupported.Cacheable -or $unsupported.Reason -ne 'cargo-configuration-present') { throw 'Configuration non prise en charge cachée' }
        $firstConfig = Key
        Set-Content -LiteralPath $config -Value '[build] # changed' -NoNewline
        if ((Key) -eq $firstConfig) { throw 'Contenu de configuration ignoré' }
        Set-Content -LiteralPath $config -Value '"\u0069nclude" = ["outside.toml"]' -NoNewline
        if ((Get-WhisperNativeCacheKey $rust $workspace $options).Cacheable) { throw 'Include TOML échappé accepté pour le cache' }
        Remove-Item -LiteralPath $config
    }
    foreach ($file in @('Cargo.toml', 'Cargo.lock', '.github/workflows/windows.yml', '.github/scripts/check.ps1')) {
        $path = Join-Path $workspace $file
        Set-Content -LiteralPath $path -Value 'changed' -NoNewline
        Must-Change
        Set-Content -LiteralPath $path -Value 'fixture' -NoNewline
    }
    $rust = 'c' * 64
    Must-Change
    $rust = 'a' * 64
    $options.RUST_CACHE_NATIVE_TOOLS = 'd' * 64
    Must-Change
    $options.RUST_CACHE_NATIVE_TOOLS = 'b' * 64
    foreach ($name in @('CARGO_ENCODED_RUSTFLAGS', 'RUSTFLAGS', 'CC', 'CFLAGS', 'CPPFLAGS', 'CXXFLAGS',
        'CMAKE_GENERATOR', 'WHISPER_TEST', 'LIBCLANG_PATH', 'BINDGEN_EXTRA_CLANG_ARGS', 'PKG_CONFIG_PATH',
        'AR', 'ARFLAGS', 'AR_x86_64_pc_windows_gnu', 'RANLIB', 'NM', 'LD', 'LDFLAGS', 'HOST_CC', 'TARGET_CXX',
        'MSYSTEM', 'MINGW_PREFIX', 'SOURCE_DATE_EPOCH', 'cxxflags')) {
        $options[$name] = 'changed'
        Must-Change
        $options.Remove($name)
    }
    $options.GGML_NATIVE = 'ON'
    Must-Change
    $options.GGML_NATIVE = 'OFF'
    $options.UNRELATED_SECRET = 'never logged'
    $options.ARTIFACT_SERVICE_URL = 'volatile, not a compiler option'
    if ((Key) -ne $baseline) { throw 'Option sans effet incluse' }
    $options.Remove('UNRELATED_SECRET')
    $options.Remove('ARTIFACT_SERVICE_URL')
    $other = Join-Path $fixture 'other-repo'
    Copy-Item -LiteralPath $workspace -Destination $other -Recurse
    if ((Get-WhisperNativeCacheKey $rust $other $options).Key -eq $baseline) { throw 'Workspace ignoré' }
    $link = Join-Path $workspace 'third-party/link'
    New-Item -ItemType SymbolicLink -Path $link -Target $cargoHome | Out-Null
    Must-Fail { Key }
    Remove-Item -LiteralPath $link
    New-Item -ItemType SymbolicLink -Path $configPaths[0] -Target $source | Out-Null
    Must-Fail { Key }
    Remove-Item -LiteralPath $configPaths[0]
    Set-Content -LiteralPath (Join-Path $cargoHome 'credentials.toml') -Value 'not an input'
    if ((Key) -ne $baseline) { throw 'Credentials inclus dans la clé' }
    Must-Fail { Get-WhisperNativeCacheKey -RustToolsSha256 '' -Workspace $workspace -Environment $options }
    Must-Fail { Get-WhisperNativeCacheKey -RustToolsSha256 'incorrect' -Workspace $workspace -Environment $options }
    $options.RUST_CACHE_NATIVE_TOOLS = ''
    Must-Fail { Key }
    $options.RUST_CACHE_NATIVE_TOOLS = 'b' * 64
    $env:GITHUB_OUTPUT = $null
    Must-Fail { & (Join-Path $PSScriptRoot 'set-whisper-native-cache-key.ps1') -RustToolsSha256 $rust }
    $env:GITHUB_OUTPUT = Join-Path $fixture 'output'
    $env:GITHUB_WORKSPACE = $workspace
    $env:RUST_CACHE_NATIVE_TOOLS = 'b' * 64
    $env:CARGO_HOME = $cargoHome
    & (Join-Path $PSScriptRoot 'set-whisper-native-cache-key.ps1') -RustToolsSha256 $rust
    if ($env:RUST_CACHE_NATIVE_TOOLS -ne ('b' * 64)) { throw 'Empreinte Cargo modifiée' }
    $output = @(Get-Content -LiteralPath $env:GITHUB_OUTPUT)
    if ($output.Count -ne 3 -or $output[0] -notmatch '^key=windows-gnu-whisper-out-v1-[a-f0-9]{64}$' -or
        $output[1] -ne ('cacheable=' + $simpleInputs.Cacheable.ToString().ToLowerInvariant()) -or
        $output[2] -ne ('reason=' + $simpleInputs.Reason)) { throw 'Export invalide' }
    Set-Content -LiteralPath $configPaths[0] -Value '"\u0069nclude" = ["outside.toml"]' -NoNewline
    $env:GITHUB_OUTPUT = Join-Path $fixture 'output-config'
    & (Join-Path $PSScriptRoot 'set-whisper-native-cache-key.ps1') -RustToolsSha256 $rust
    $output = @(Get-Content -LiteralPath $env:GITHUB_OUTPUT)
    if ($output.Count -ne 3 -or $output[1] -ne 'cacheable=false' -or
        $output[2] -ne 'reason=cargo-configuration-present') { throw 'Configuration non couverte non désactivée' }
    if ($env:RUST_CACHE_NATIVE_TOOLS -ne ('b' * 64)) { throw 'Empreinte Cargo modifiée après configuration' }
    Write-Output 'OK: clé Whisper stricte et déterministe ; sources/configurations/options couvertes ; liens et entrées invalides refusés'
} finally {
    $env:RUST_CACHE_NATIVE_TOOLS = $originalNative
    $env:GITHUB_OUTPUT = $originalOutput
    $env:GITHUB_WORKSPACE = $originalWorkspace
    $env:CARGO_HOME = $originalCargoHome
    Remove-Item -LiteralPath $fixture -Recurse -Force -ErrorAction SilentlyContinue
}
