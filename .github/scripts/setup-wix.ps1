# WiX 3.14.1 portable : aucun Chocolatey, installateur global ou privilège admin.
$ErrorActionPreference = "Stop"
$wixDir = Join-Path $env:RUNNER_TEMP ("meeting-recorder-wix-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $wixDir | Out-Null
$zip = Join-Path $wixDir "wix314-binaries.zip"
Invoke-WebRequest -Uri "https://github.com/wixtoolset/wix3/releases/download/wix3141rtm/wix314-binaries.zip" -OutFile $zip
# Archive officielle wix3141rtm téléchargée et vérifiée le 8 octobre 2026.
if ((Get-Item -LiteralPath $zip).Length -ne 41297555) { throw "Taille WiX inattendue" }
$expected = "6ac824e1642d6f7277d0ed7ea09411a508f6116ba6fae0aa5f2c7daa2ff43d31"
if ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) {
    throw "SHA-256 WiX inattendu : archive refusée"
}
Expand-Archive -LiteralPath $zip -DestinationPath $wixDir
foreach ($name in @("candle.exe", "light.exe", "heat.exe")) {
    $tool = Join-Path $wixDir $name
    if (-not (Test-Path -LiteralPath $tool)) { throw "WiX incomplet : $name absent" }
    $help = & $tool -?
    if ($LASTEXITCODE -ne 0) { throw "$name ne démarre pas (exit $LASTEXITCODE)" }
    $help | Select-Object -First 2
}
$wixDir >> $env:GITHUB_PATH
Write-Output "[ci] WiX 3.14.1 vérifié : $wixDir"
