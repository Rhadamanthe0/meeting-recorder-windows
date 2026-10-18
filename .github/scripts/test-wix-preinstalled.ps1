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
} finally {
    if (Test-Path -LiteralPath $fixture) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
