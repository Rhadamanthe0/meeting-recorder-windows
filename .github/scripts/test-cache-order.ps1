# Offline workflow contract: dependencies may change cache archive compatibility.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$lines = [IO.File]::ReadAllLines((Join-Path $root '.github/workflows/windows.yml'))
function Get-StepPosition([string]$Name) {
    $hits = @()
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -ceq ('      - name: ' + $Name)) { $hits += $i }
    }
    if ($hits.Count -ne 1) { throw "Expected one step: $Name" }
    return $hits[0]
}
$cache = Get-StepPosition 'Cache cargo'
foreach ($name in @('Install GTK4 (MSYS2 UCRT64)', 'Met à jour MSYS2 et installe GTK sans arrêt global de processus', 'Expose GTK au build GNU')) {
    if ((Get-StepPosition $name) -gt $cache) { throw 'Prepare MSYS2 and its compressor before restoring Cargo cache.' }
}
if ((Get-StepPosition 'Vérifie GNU après restauration du cache') -lt $cache) { throw 'The GNU check must follow cache restoration.' }
if ((Get-StepPosition 'Check') -lt $cache) { throw 'Cargo compilation must follow cache restoration.' }
Write-Output 'OK: cache restore and export share the prepared tool environment'
