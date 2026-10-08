param(
    [string]$Exe = "target/release/meeting-recorder-windows.exe",
    [string]$Output = "synthetic-audio-results"
)
$ErrorActionPreference = "Stop"
$Exe = (Resolve-Path -LiteralPath $Exe).Path
New-Item -ItemType Directory -Force -Path $Output | Out-Null
$Output = (Resolve-Path -LiteralPath $Output).Path
# GUI-subsystem executables need an explicit wait and redirected output.
$probe = Start-Process -FilePath $Exe -ArgumentList "--version" -PassThru -NoNewWindow `
    -RedirectStandardOutput (Join-Path $Output "version.log") `
    -RedirectStandardError (Join-Path $Output "version-stderr.log")
try {
    if (-not $probe.WaitForExit(10000)) {
        Stop-Process -Id $probe.Id -Force
        throw "Synthetic version check timed out; GUI launch refused"
    }
    $version = Get-Content -LiteralPath (Join-Path $Output "version.log")
    if ($probe.ExitCode -ne 0 -or ($version -join " ") -notmatch 'CI synthetic audio; hardware audio disabled on Windows') {
        throw "GUI refused: ci-audio binary required; hardware capture is forbidden"
    }
} finally { $probe.Dispose() }
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
$process = Start-Process -FilePath $Exe -PassThru -NoNewWindow `
    -RedirectStandardOutput (Join-Path $Output "stdout.log") `
    -RedirectStandardError (Join-Path $Output "stderr.log")
$pipe = $null
$reader = $null
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        $process.Refresh()
        if ($process.HasExited) { throw "Synthetic GUI exited with code $($process.ExitCode)" }
        if ($process.MainWindowHandle -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
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
