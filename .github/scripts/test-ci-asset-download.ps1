# Offline: no production download, installer, service or process is invoked.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'download-ci-asset.ps1')
Add-Type @'
public sealed class CIDownloadTestResponse : System.Net.WebResponse {
    public System.Net.HttpStatusCode StatusCode { get; private set; }
    public CIDownloadTestResponse(int status) { StatusCode = (System.Net.HttpStatusCode)status; }
}
'@
$fixture = Join-Path ([IO.Path]::GetTempPath()) ('ci download test ' + [guid]::NewGuid().ToString('N'))
$script:attempts = 0
$script:delays = @()
$script:failures = @()
$script:partialPaths = @()
$script:assetLength = $null
function Start-Sleep([int]$Seconds) { $script:delays += $Seconds }
function Invoke-WebRequest {
    [CmdletBinding()]
    param([uri]$Uri, [string]$OutFile, [int]$ConnectionTimeoutSeconds, [int]$OperationTimeoutSeconds)
    if ($Uri.Scheme -ne 'https' -or $ConnectionTimeoutSeconds -ne 60 -or $OperationTimeoutSeconds -ne 60) { throw 'Unbounded or insecure fixture transfer.' }
    if (Test-Path -LiteralPath $OutFile) { throw 'A partial transfer was reused.' }
    $script:partialPaths += $OutFile
    $script:attempts++
    if ($script:attempts -le $script:failures.Count) {
        [IO.File]::WriteAllText($OutFile, 'partial')
        throw $script:failures[$script:attempts - 1]
    }
    if ($null -ne $script:assetLength) {
        $file = [IO.File]::Create($OutFile)
        try { $file.SetLength($script:assetLength) } finally { $file.Dispose() }
    } else {
        [IO.File]::WriteAllText($OutFile, 'complete transport; verification still required')
    }
}
function New-HttpFailure([int]$Code) {
    [Net.WebException]::new('fixture HTTP failure', $null, [Net.WebExceptionStatus]::ProtocolError,
        [CIDownloadTestResponse]::new($Code))
}
function Test-DownloadScenario {
    param([string]$Name, [object[]]$Failures, [int]$Attempts, [bool]$Fails)
    $script:attempts = 0; $script:delays = @(); $script:partialPaths = @(); $script:failures = $Failures
    $destination = Join-Path $fixture ($Name + '.zip')
    [IO.File]::WriteAllText($destination, 'preexisting')
    $failed = $false
    try { Invoke-CIAssetDownload -Uri 'https://example.invalid/pinned.zip' -OutFile $destination }
    catch { $failed = $true }
    if ($failed -ne $Fails -or $script:attempts -ne $Attempts) { throw "Unexpected result for $Name" }
    if ($script:delays.Count -ne ($Attempts - 1)) { throw "Unexpected retries for $Name" }
    for ($i = 0; $i -lt $script:delays.Count; $i++) {
        if ($script:delays[$i] -ne 10 * ($i + 1)) { throw 'Retry backoff changed.' }
    }
    $expected = if ($Fails) { 'preexisting' } else { 'complete transport; verification still required' }
    if ([IO.File]::ReadAllText($destination) -cne $expected) { throw "Partial file published for $Name" }
    foreach ($path in $script:partialPaths) {
        if (Test-Path -LiteralPath $path) { throw "Partial file retained for $Name" }
    }
    Write-Output "PASS: $Name"
}
try {
    New-Item -ItemType Directory -Path $fixture | Out-Null
    Test-DownloadScenario -Name 'success' -Failures @() -Attempts 1 -Fails $false
    $timeout = [Net.Sockets.SocketException]::new(10060)
    Test-DownloadScenario -Name 'observed-connect-timeout' -Failures @($timeout, $timeout) -Attempts 3 -Fails $false
    Test-DownloadScenario -Name 'exhausted-connect-timeout' -Failures @($timeout, $timeout, $timeout) -Attempts 3 -Fails $true
    Test-DownloadScenario -Name 'wrapped-connect-timeout' -Failures @([Net.Http.HttpRequestException]::new('fixture', $timeout)) -Attempts 2 -Fails $false
    Test-DownloadScenario -Name 'stalled-read-timeout' -Failures @([TimeoutException]::new('fixture stalled read')) -Attempts 2 -Fails $false
    foreach ($code in @(408, 429, 500, 503)) {
        Test-DownloadScenario -Name "http-$code" -Failures @((New-HttpFailure $code)) -Attempts 2 -Fails $false
    }
    foreach ($code in @(400, 401, 403, 404)) {
        Test-DownloadScenario -Name "http-$code" -Failures @((New-HttpFailure $code)) -Attempts 1 -Fails $true
    }
    Test-DownloadScenario -Name 'invalid-tls-certificate' -Failures @([Security.Authentication.AuthenticationException]::new('fixture')) -Attempts 1 -Fails $true
    Test-DownloadScenario -Name 'tls-with-nested-socket-error' -Failures @([Security.Authentication.AuthenticationException]::new('fixture', $timeout)) -Attempts 1 -Fails $true
    Test-DownloadScenario -Name 'unclassified-error' -Failures @([InvalidOperationException]::new('fixture')) -Attempts 1 -Fails $true
    $script:attempts = 0
    try { Invoke-CIAssetDownload -Uri 'http://example.invalid/file' -OutFile (Join-Path $fixture 'insecure.zip'); throw 'Insecure URL accepted.' }
    catch { if ($_.Exception.Message -eq 'Insecure URL accepted.') { throw } }
    if ($script:attempts -ne 0) { throw 'An insecure download was attempted.' }
    # Execute the actual workflow prefix against an invalid local asset. The
    # original size/SHA checks must still stop before any archive extraction.
    $repository = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
    $lines = [IO.File]::ReadAllLines((Join-Path $repository '.github/workflows/windows.yml'))
    $index = [Array]::IndexOf($lines, '      - name: Install ffmpeg (pinné 9.0.2 vérifié)')
    if ($index -lt 0 -or $lines[$index + 1] -ne '        run: |') { throw 'FFmpeg workflow step missing.' }
    $prefix = @()
    for ($i = $index + 2; $i -lt $lines.Count -and $lines[$i].StartsWith('          '); $i++) {
        $prefix += $lines[$i].Substring(10)
        if ($prefix[-1] -match '^Expand-Archive ') { break }
    }
    if ($prefix[-1] -notmatch '^Expand-Archive ') { throw 'Expected archive boundary missing.' }
    $script:extractions = 0
    function Expand-Archive {
        [CmdletBinding()]param([string]$LiteralPath, [string]$DestinationPath, [switch]$Force)
        $script:extractions++
        throw 'Extraction before integrity checks.'
    }
    $previousTemp = $env:RUNNER_TEMP
    Push-Location $repository
    try {
        $env:RUNNER_TEMP = $fixture
        foreach ($case in @(@{ Length = 1; Error = 'Taille ZIP' }, @{ Length = 114768076; Error = 'SHA-256' })) {
            $script:attempts = 0; $script:failures = @(); $script:delays = @(); $script:assetLength = $case.Length
            $failure = $null
            try { & ([scriptblock]::Create(($prefix -join "`n"))) } catch { $failure = $_ }
            if ($null -eq $failure -or $failure.Exception.Message -notmatch $case.Error) { throw 'Workflow integrity failure was lost.' }
            if ($script:attempts -ne 1 -or $script:delays.Count -ne 0 -or $script:extractions -ne 0) { throw 'An integrity failure was retried or extracted.' }
            Write-Output "PASS: workflow rejects $($case.Error) before extraction without retry"
        }
    } finally {
        Pop-Location
        $env:RUNNER_TEMP = $previousTemp
    }
    Write-Output 'PASS: 19 offline asset scenarios, including real workflow size/SHA rejection.'
} finally {
    if (Test-Path -LiteralPath $fixture) { Remove-Item -LiteralPath $fixture -Recurse -Force }
}
