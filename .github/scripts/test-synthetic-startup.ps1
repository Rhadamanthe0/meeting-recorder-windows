# Offline process/clock fixtures: no executable, GUI, audio endpoint or network.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$source = Join-Path $PSScriptRoot 'windows-synthetic-gui.ps1'
$tokens = $null; $errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($source, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
foreach ($name in @('Get-SyntheticStartupRemaining', 'Assert-SyntheticBinary')) {
    $fn = $ast.Find({ param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $name }, $false)
    if (-not $fn) { throw "Missing function $name" }
    . ([scriptblock]::Create($fn.Extent.Text))
}
$root = Join-Path ([IO.Path]::GetTempPath()) ('synthetic-startup-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
function Start-Process {
    param($FilePath, $ArgumentList, [switch]$PassThru, [switch]$NoNewWindow, $RedirectStandardOutput, $RedirectStandardError)
    if ($ArgumentList -ne '--version') { throw 'A fixture attempted a GUI/hardware launch' }
    $script:starts++
    $script:clock.ElapsedMilliseconds += $script:startCost
    $script:stdout = $RedirectStandardOutput
    [IO.File]::WriteAllText($RedirectStandardOutput, '')
    [IO.File]::WriteAllText($RedirectStandardError, '')
    $process = [pscustomobject]@{ Id = 12345; ExitCode = $script:exitCode }
    $process | Add-Member ScriptMethod WaitForExit {
        param([int]$Milliseconds)
        $script:wait = $Milliseconds
        $script:clock.ElapsedMilliseconds += [Math]::Min($Milliseconds, $script:latency)
        if ($Milliseconds -lt $script:latency) { return $false }
        [IO.File]::WriteAllText($script:stdout, $script:version)
        return $true
    }
    $process | Add-Member ScriptMethod Dispose { $script:disposed++ }
    return $process
}
function Stop-Process {
    param($Id, [switch]$Force)
    if ($Id -ne 12345 -or -not $Force) { throw 'Unexpected process stop' }
    $script:stops++
}
function Test-Scenario($Name, [int]$Latency, [string]$Version, [int]$ExitCode, [bool]$Fails, [int]$StartCost = 0, [int]$AlreadyElapsed = 0) {
    $script:clock = [pscustomobject]@{ ElapsedMilliseconds = $AlreadyElapsed }
    $script:latency = $Latency; $script:version = $Version; $script:exitCode = $ExitCode; $script:startCost = $StartCost
    $script:starts = 0; $script:stops = 0; $script:disposed = 0; $script:wait = 0
    $output = Join-Path $root $Name
    New-Item -ItemType Directory -Path $output | Out-Null
    $failure = $null
    try { Assert-SyntheticBinary 'fixture only.exe' $output $script:clock } catch { $failure = $_ }
    if (($null -ne $failure) -ne $Fails) { throw "$Name unexpected outcome: $failure" }
    if ($script:starts -gt 1 -or $script:disposed -ne $script:starts) { throw "$Name leaked or relaunched a process" }
    if ($script:wait -gt [Math]::Max(0, (40000 - $AlreadyElapsed - $StartCost))) { throw "$Name renewed its startup budget" }
    if ($AlreadyElapsed -ge 40000 -and $script:starts -ne 0) { throw "$Name launched after deadline" }
    if ($script:starts) {
        $record = Get-Content -LiteralPath (Join-Path $output 'version-check.json') -Raw | ConvertFrom-Json
        if ($record.elapsedMilliseconds -ne $script:clock.ElapsedMilliseconds) { throw "$Name missing duration evidence" }
        if ($record.startupBudgetMilliseconds -ne 40000) { throw "$Name changed the budget" }
        if (($Latency -gt $script:wait) -and ($script:stops -ne 1 -or $record.completed)) { throw "$Name accepted a hung process" }
    }
    if (-not $Fails -and (Get-SyntheticStartupRemaining $script:clock) -ne (40000 - $AlreadyElapsed - $StartCost - $Latency)) { throw "$Name granted a fresh GUI budget" }
    Write-Output "OK: $Name"
}
try {
    $synthetic = 'fixture 1.0 CI synthetic audio; hardware audio disabled on Windows'
    Test-Scenario 'cold-valid-12s' 12000 $synthetic 0 $false 5000
    Test-Scenario 'fast-valid' 100 $synthetic 0 $false
    Test-Scenario 'normal-binary-forbidden' 100 'production recorder 1.0' 0 $true
    Test-Scenario 'nonzero-exit' 100 $synthetic 7 $true
    Test-Scenario 'missing-output' 100 '' 0 $true
    Test-Scenario 'hung-version-killed' 50000 $synthetic 0 $true
    Test-Scenario 'spent-during-launch' 100 $synthetic 0 $true 41000
    Test-Scenario 'partly-spent-budget' 6000 $synthetic 0 $true 0 35000
    Test-Scenario 'expired-before-launch' 100 $synthetic 0 $true 0 40000
} finally {
    if ((Split-Path $root -Leaf) -notlike 'synthetic-startup-*') { throw 'Unexpected fixture directory' }
    Remove-Item -LiteralPath $root -Recurse -Force
}
