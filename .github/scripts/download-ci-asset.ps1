# Transport only: callers must verify the pinned length/hash before extraction.
function Test-CIDownloadTransient {
    param([Parameter(Mandatory = $true)][System.Management.Automation.ErrorRecord]$Failure)
    $chain = @()
    $exception = $Failure.Exception
    while ($null -ne $exception -and $chain.Count -lt 16) {
        $chain += $exception
        $exception = $exception.InnerException
    }
    foreach ($exception in $chain) {
        if ($exception -is [Security.Authentication.AuthenticationException]) { return $false }
        if ($exception -is [Net.WebException] -and
            $exception.Status -in @([Net.WebExceptionStatus]::TrustFailure, [Net.WebExceptionStatus]::SecureChannelFailure)) {
            return $false
        }
    }
    foreach ($exception in $chain) {
        $response = $exception.PSObject.Properties['Response']
        $status = $exception.PSObject.Properties['StatusCode']
        if ($null -ne $response -and $null -ne $response.Value) {
            $status = $response.Value.PSObject.Properties['StatusCode']
        }
        if ($null -ne $status -and $null -ne $status.Value) {
            $code = [int]$status.Value
            return ($code -in @(408, 429) -or ($code -ge 500 -and $code -le 599))
        }
        if ($exception -is [TimeoutException]) { return $true }
        if ($exception -is [Net.Sockets.SocketException] -and
            [string]$exception.SocketErrorCode -in @('TimedOut', 'ConnectionReset', 'ConnectionAborted',
                'ConnectionRefused', 'NetworkUnreachable', 'HostUnreachable', 'HostNotFound', 'TryAgain')) {
            return $true
        }
        if ($exception -is [Net.WebException] -and
            [string]$exception.Status -in @('Timeout', 'NameResolutionFailure', 'ConnectFailure',
                'ConnectionClosed', 'ReceiveFailure', 'SendFailure', 'KeepAliveFailure')) {
            return $true
        }
    }
    return $false
}

function Invoke-CIAssetDownload {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][uri]$Uri,
        [Parameter(Mandatory = $true)][string]$OutFile
    )
    if (-not $Uri.IsAbsoluteUri -or $Uri.Scheme -ne 'https' -or $Uri.UserInfo) {
        throw 'An absolute HTTPS asset URL without embedded credentials is required.'
    }
    $partial = $OutFile + '.partial-' + [guid]::NewGuid().ToString('N')
    try {
        for ($attempt = 1; $attempt -le 3; $attempt++) {
            try {
                # Bound connection waits and stalled reads. A progressing large
                # transfer can last longer; the existing job lease is unchanged.
                Invoke-WebRequest -Uri $Uri -OutFile $partial -ConnectionTimeoutSeconds 60 -OperationTimeoutSeconds 60 -ErrorAction Stop
            } catch {
                if (Test-Path -LiteralPath $partial) { Remove-Item -LiteralPath $partial -Force -ErrorAction Stop }
                if ($attempt -eq 3 -or -not (Test-CIDownloadTransient -Failure $_)) { throw }
                Write-Warning "[ci] Asset transport interrupted; retry $($attempt + 1)/3."
                Start-Sleep -Seconds (10 * $attempt)
                continue
            }
            Move-Item -LiteralPath $partial -Destination $OutFile -Force -ErrorAction Stop
            return
        }
    } finally {
        if (Test-Path -LiteralPath $partial) { Remove-Item -LiteralPath $partial -Force -ErrorAction Stop }
    }
}
