# Aucun téléchargement ni installation : vrais processus factices et fichiers temporaires.
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$installer = Join-Path $PSScriptRoot "install-msys2-packages.ps1"
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($installer, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count -ne 0) { throw ($parseErrors | Out-String) }
$invoke = $ast.Find({
    param($node)
    $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq "Invoke-Msys"
}, $false)
if (-not $invoke) { throw "Fonction Invoke-Msys absente" }
# Charger uniquement la fonction réelle, sans exécuter l'installation.
. ([scriptblock]::Create($invoke.Extent.Text))

$script:delays = [System.Collections.Generic.List[int]]::new()
function Start-Sleep([int]$Seconds) { $script:delays.Add($Seconds) }
$root = Join-Path ([System.IO.Path]::GetTempPath()) ("msys2 retry test " + [guid]::NewGuid().ToString("N"))
$previousState = $env:MR_MSYS2_TEST_STATE
# La fonction doit gérer explicitement les codes natifs, même avec cette option.
$PSNativeCommandUseErrorActionPreference = $true
$script:passed = 0

function Test-Scenario {
    param(
        [string]$Name,
        [string[]]$Diagnostics,
        [int[]]$ExitCodes,
        [int]$ExpectedAttempts,
        [bool]$ShouldFail = $false,
        [bool]$RetryDownload = $true
    )
    $state = Join-Path $root $Name
    New-Item -ItemType Directory -Path $state | Out-Null
    $env:MR_MSYS2_TEST_STATE = $state
    Set-Content -LiteralPath (Join-Path $state "count") -Value "0" -Encoding ascii
    for ($i = 0; $i -lt $ExitCodes.Count; $i++) {
        Set-Content -LiteralPath (Join-Path $state "stderr-$($i + 1)") -Value $Diagnostics[$i] -Encoding utf8NoBOM
        Set-Content -LiteralPath (Join-Path $state "exit-$($i + 1)") -Value $ExitCodes[$i] -Encoding ascii
    }
    $script:delays.Clear()
    $failure = $null
    try {
        Invoke-Msys 'exec pacman --noconfirm -S --needed example' -RetryDownload:$RetryDownload 3> (Join-Path $state "warnings") 6> (Join-Path $state "output")
    } catch { $failure = $_ }
    $attempts = [int](Get-Content -LiteralPath (Join-Path $state "count") -Raw)
    if ($attempts -ne $ExpectedAttempts) { throw "$Name : $attempts tentatives, attendu $ExpectedAttempts" }
    if (($null -ne $failure) -ne $ShouldFail) { throw "$Name : échec inattendu ou non détecté : $failure" }
    if ($ShouldFail -and $failure.Exception.Message -notmatch 'Commande MSYS2 en échec \(exit 1, tentative') {
        throw "$Name : mauvaise erreur : $failure"
    }
    if ($script:delays.Count -ne ($ExpectedAttempts - 1)) { throw "$Name : nombre de délais incorrect" }
    for ($i = 0; $i -lt $script:delays.Count; $i++) {
        if ($script:delays[$i] -ne (10 * ($i + 1))) { throw "$Name : délai incorrect" }
    }
    $script:passed++
    Write-Output "[test] $Name OK"
}

try {
    New-Item -ItemType Directory -Path $root | Out-Null
    if ($IsWindows) {
        $bash = Join-Path $root "fake bash.cmd"
        @'
@echo off
setlocal EnableExtensions DisableDelayedExpansion
set /p attempt=<"%MR_MSYS2_TEST_STATE%\count"
set /a attempt+=1 >nul
>"%MR_MSYS2_TEST_STATE%\count" echo %attempt%
echo fake pacman attempt %attempt%
type "%MR_MSYS2_TEST_STATE%\stderr-%attempt%" 1>&2
set /p code=<"%MR_MSYS2_TEST_STATE%\exit-%attempt%"
exit /b %code%
'@ | Set-Content -LiteralPath $bash -Encoding ascii
    } else {
        $bash = Join-Path $root "fake bash"
        @'
#!/bin/sh
set -eu
state=$MR_MSYS2_TEST_STATE
attempt=$(cat "$state/count")
attempt=$((attempt + 1))
printf '%s\n' "$attempt" > "$state/count"
printf 'fake pacman attempt %s\n' "$attempt"
cat "$state/stderr-$attempt" >&2
exit "$(cat "$state/exit-$attempt")"
'@ | Set-Content -LiteralPath $bash -Encoding utf8NoBOM
        & chmod +x $bash
        if ($LASTEXITCODE -ne 0) { throw "chmod du processus factice en échec" }
    }

    $timeout = "error: failed retrieving file 'tcl.pkg.tar.zst' from repo.msys2.org : Operation too slow. Less than 1 bytes/sec transferred the last 10 seconds"
    $summary = "error: failed to commit transaction (unexpected error)"
    $signature = 'error: tcl: signature from "MSYS2 Maintainer" is unknown trust'
    $networkFailure = "$timeout`n$summary"
    Test-Scenario -Name "success" -Diagnostics @('') -ExitCodes @(0) -ExpectedAttempts 1
    Test-Scenario -Name "timeout-then-success" -Diagnostics @($networkFailure, '') -ExitCodes @(1, 0) -ExpectedAttempts 2
    Test-Scenario -Name "third-attempt-success" -Diagnostics @($networkFailure, $networkFailure, '') -ExitCodes @(1, 1, 0) -ExpectedAttempts 3
    Test-Scenario -Name "retry-exhausted" -Diagnostics @($networkFailure, $networkFailure, $networkFailure) -ExitCodes @(1, 1, 1) -ExpectedAttempts 3 -ShouldFail $true
    Test-Scenario -Name "signature-refused" -Diagnostics @($signature) -ExitCodes @(1) -ExpectedAttempts 1 -ShouldFail $true
    Test-Scenario -Name "mixed-signature-network" -Diagnostics @("$timeout`n$signature`n$summary") -ExitCodes @(1) -ExpectedAttempts 1 -ShouldFail $true
    Test-Scenario -Name "missing-package" -Diagnostics @('error: target not found: example') -ExitCodes @(1) -ExpectedAttempts 1 -ShouldFail $true
    Test-Scenario -Name "unknown-error" -Diagnostics @($summary) -ExitCodes @(1) -ExpectedAttempts 1 -ShouldFail $true
    Test-Scenario -Name "tls-certificate" -Diagnostics @("error: failed retrieving file 'tcl.pkg.tar.zst' from repo.msys2.org : SSL certificate problem: certificate has expired`n$summary") -ExitCodes @(1) -ExpectedAttempts 1 -ShouldFail $true
    Test-Scenario -Name "http-404" -Diagnostics @("error: failed retrieving file 'tcl.pkg.tar.zst' from repo.msys2.org : The requested URL returned error: 404`n$summary") -ExitCodes @(1) -ExpectedAttempts 1 -ShouldFail $true
    Test-Scenario -Name "http-503" -Diagnostics @("error: failed retrieving file 'tcl.pkg.tar.zst' from repo.msys2.org : The requested URL returned error: 503`n$summary", '') -ExitCodes @(1, 0) -ExpectedAttempts 2
    Test-Scenario -Name "database-dns" -Diagnostics @("error: failed retrieving file 'msys.db' from repo.msys2.org : Could not resolve host: repo.msys2.org`nerror: failed to synchronize all databases (failed to retrieve some files)", '') -ExitCodes @(1, 0) -ExpectedAttempts 2
    Test-Scenario -Name "retry-not-enabled" -Diagnostics @($networkFailure) -ExitCodes @(1) -ExpectedAttempts 1 -ShouldFail $true -RetryDownload $false
    Write-Output "[test] $script:passed scénarios MSYS2 réussis"
} finally {
    $env:MR_MSYS2_TEST_STATE = $previousState
    if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force }
}
# Le dernier scénario attend un code natif non nul ; ne pas le propager au runner.
$global:LASTEXITCODE = 0
