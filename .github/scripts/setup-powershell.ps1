# Compatible Windows PowerShell 5.1 : le service peut avoir un PATH différent
# de la session interactive. Aucun installateur global ni privilège admin.
$ErrorActionPreference = "Stop"
$pwsh = Get-Command pwsh.exe -ErrorAction SilentlyContinue
$exe = if ($pwsh) { $pwsh.Source } else { Join-Path $env:ProgramFiles "PowerShell\7\pwsh.exe" }
if (-not (Test-Path -LiteralPath $exe)) {
    $dir = Join-Path $env:RUNNER_TEMP ("meeting-recorder-pwsh-" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $dir | Out-Null
    $zip = Join-Path $dir "PowerShell-7.6.6-win-x64.zip"
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -UseBasicParsing -Uri "https://github.com/PowerShell/PowerShell/releases/download/v7.6.6/PowerShell-7.6.6-win-x64.zip" -OutFile $zip
    # Taille et digest officiels de l'asset GitHub, vérifiés le 8 octobre 2026.
    if ((Get-Item -LiteralPath $zip).Length -ne 106328873) { throw "Taille PowerShell inattendue" }
    $expected = "02fe458be20493fbdf43f61ea20610b811ee6c738ab1676c61b9cfcd1a33c860"
    if ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) {
        throw "SHA-256 PowerShell inattendu : archive refusée"
    }
    Expand-Archive -LiteralPath $zip -DestinationPath $dir
    $exe = Join-Path $dir "pwsh.exe"
}
& $exe -NoProfile -Command '$PSVersionTable.PSVersion.ToString()'
if ($LASTEXITCODE -ne 0) { throw "PowerShell ne démarre pas (exit $LASTEXITCODE)" }
[IO.File]::AppendAllText($env:GITHUB_PATH, (Split-Path $exe -Parent) + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
Write-Output "[ci] PowerShell prêt pour le service : $exe"
