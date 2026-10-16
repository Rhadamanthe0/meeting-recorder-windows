param(
    [string]$Exe = "target/release/meeting-recorder-windows.exe",
    [string]$Output = "synthetic-audio-results"
)
$ErrorActionPreference = "Stop"
$Exe = (Resolve-Path -LiteralPath $Exe).Path
New-Item -ItemType Directory -Force -Path $Output | Out-Null
$Output = (Resolve-Path -LiteralPath $Output).Path
# Budget commun : les anciennes attentes successives etaient de 10 + 30 s.
# Le premier chargement dans une VM neuve a depasse les 10 s sans lancer le GUI.
function Get-SyntheticStartupRemaining($Clock) {
    return [int][Math]::Max(0, 40000 - $Clock.ElapsedMilliseconds)
}

function Assert-SyntheticBinary($Exe, $Output, $Clock) {
    if ((Get-SyntheticStartupRemaining $Clock) -le 0) {
        throw "Synthetic startup budget exhausted; GUI launch refused"
    }
    $probe = Start-Process -FilePath $Exe -ArgumentList "--version" -PassThru -NoNewWindow `
        -RedirectStandardOutput (Join-Path $Output "version.log") `
        -RedirectStandardError (Join-Path $Output "version-stderr.log")
    $completed = $false
    $exitCode = $null
    try {
        $remaining = Get-SyntheticStartupRemaining $Clock
        if ($remaining -le 0 -or -not $probe.WaitForExit($remaining)) {
            Stop-Process -Id $probe.Id -Force
            throw "Synthetic version check exhausted the shared startup budget; GUI launch refused"
        }
        $completed = $true
        $exitCode = $probe.ExitCode
        $version = Get-Content -LiteralPath (Join-Path $Output "version.log")
        if ($exitCode -ne 0 -or ($version -join " ") -notmatch 'CI synthetic audio; hardware audio disabled on Windows') {
            throw "GUI refused: ci-audio binary required; hardware capture is forbidden (version exit $exitCode)"
        }
    } finally {
        @{ elapsedMilliseconds = $Clock.ElapsedMilliseconds; startupBudgetMilliseconds = 40000;
            completed = $completed; exitCode = $exitCode } | ConvertTo-Json |
            Set-Content -LiteralPath (Join-Path $Output "version-check.json") -Encoding utf8
        $probe.Dispose()
    }
}
$startupClock = [Diagnostics.Stopwatch]::StartNew()
Assert-SyntheticBinary $Exe $Output $startupClock
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class SyntheticWindow {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam);
}
'@
# Software renderer makes a window-only capture possible without photographing
# the user's desktop. This script never sends keyboard/mouse events globally.
$env:GDK_BACKEND = "win32"
$env:GSK_RENDERER = "cairo"
if ((Get-SyntheticStartupRemaining $startupClock) -le 0) { throw "Synthetic startup budget exhausted; GUI launch refused" }
$process = Start-Process -FilePath $Exe -PassThru -NoNewWindow `
    -RedirectStandardOutput (Join-Path $Output "stdout.log") `
    -RedirectStandardError (Join-Path $Output "stderr.log")
$pipe = $null
$reader = $null
try {
    do {
        if ((Get-SyntheticStartupRemaining $startupClock) -le 0) { throw "Synthetic GUI startup budget exhausted" }
        $process.Refresh()
        if ($process.HasExited) { throw "Synthetic GUI exited with code $($process.ExitCode)" }
        if ($process.MainWindowHandle -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 100
    } while ((Get-SyntheticStartupRemaining $startupClock) -gt 0)
    if ($process.MainWindowHandle -eq [IntPtr]::Zero) { throw "Synthetic GUI did not create a window" }
    # The CI binary has a separate application ID, settings, and pipe. Never
    # connect to the production recorder, even if it is already running.
    $pipe = [System.IO.Pipes.NamedPipeClientStream]::new(".", "meeting-recorder-windows-ci-audio", [System.IO.Pipes.PipeDirection]::InOut)
    $pipe.Connect(5000)
    $reader = [System.IO.StreamReader]::new($pipe)
    $pending = $reader.ReadLineAsync()
    if (-not $pending.Wait(5000)) { throw "Synthetic IPC status timed out" }
    $status = $pending.Result | ConvertFrom-Json
    if ($status.state -ne "idle") { throw "Expected an idle synthetic recorder" }
    $pending.Result | Set-Content -LiteralPath (Join-Path $Output "status.json") -Encoding utf8
    Start-Sleep -Milliseconds 500
    $rect = [SyntheticWindow+Rect]::new()
    if (-not [SyntheticWindow]::GetWindowRect($process.MainWindowHandle, [ref]$rect)) { throw "Cannot read test window bounds" }
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    if ($width -le 100 -or $height -le 100) { throw "Test window has invalid bounds" }
    $bitmap = [System.Drawing.Bitmap]::new($width, $height)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $hdc = $graphics.GetHdc()
        try {
            if (-not [SyntheticWindow]::PrintWindow($process.MainWindowHandle, $hdc, 2)) { throw "Window-only rendering capture failed" }
        } finally { $graphics.ReleaseHdc($hdc) }
        $colors = [System.Collections.Generic.HashSet[int]]::new()
        for ($y = 0; $y -lt $height; $y += 10) {
            for ($x = 0; $x -lt $width; $x += 10) { [void]$colors.Add($bitmap.GetPixel($x, $y).ToArgb()) }
        }
        $bitmap.Save((Join-Path $Output "window.png"), [System.Drawing.Imaging.ImageFormat]::Png)
        if ($colors.Count -lt 10) { throw "Window capture is blank; render validation is incomplete" }
        # A bounded copy of this test window is also available through logs
        # when an audit environment cannot download the artifact redirect.
        $png = [IO.File]::ReadAllBytes((Join-Path $Output "window.png"))
        if ($png.Length -le 131072) {
            $encoded = [Convert]::ToBase64String($png)
            for ($offset = 0; $offset -lt $encoded.Length; $offset += 512) {
                $part = $encoded.Substring($offset, [Math]::Min(512, $encoded.Length - $offset))
                Write-Output "[ci] WINDOW_PNG_BASE64=$part"
            }
        } else { Write-Output "[ci] Window image exceeds inline limit; use synthetic-audio-results artifact" }
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
    Write-Output "Synthetic GUI and isolated IPC rendered successfully; no hardware audio endpoint opened"
} finally {
    if ($reader) { $reader.Dispose() }
    elseif ($pipe) { $pipe.Dispose() }
    $process.Refresh()
    if (-not $process.HasExited) {
        [void][SyntheticWindow]::PostMessage($process.MainWindowHandle, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
        if (-not $process.WaitForExit(5000)) { Stop-Process -Id $process.Id -Force }
    }
    $process.Dispose()
}
