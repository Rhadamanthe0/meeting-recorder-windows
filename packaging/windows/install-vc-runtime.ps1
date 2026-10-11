# Prérequis explicite des binaires ORT Microsoft, dans la VM jetable du job.
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$archive = Join-Path $env:RUNNER_TEMP 'vc_redist.x64.exe'
$url = 'https://download.visualstudio.microsoft.com/download/pr/ebdab8e5-1d7b-4d9f-a11b-cbb1720c3b12/843068991DAAA1F73AD9F6239BCE4D0F6A07A51F18C37EA2A867E9BECA71295C/VC_redist.x64.exe'
$expected = '843068991daaa1f73ad9f6239bce4d0f6a07a51f18c37ea2a867e9beca71295c'
Invoke-WebRequest -Uri $url -OutFile $archive -TimeoutSec 180
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) {
    throw 'SHA-256 Visual C++ inattendu'
}
$signature = Get-AuthenticodeSignature -LiteralPath $archive
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation(?:,|$)') {
    throw 'Signature Microsoft Visual C++ invalide'
}
$process = Start-Process -FilePath $archive -ArgumentList '/install', '/quiet', '/norestart' -Wait -PassThru
# 3010 : redémarrage conseillé ; 1638 : une version plus récente est présente.
# Le chargement réel de la DLL ci-dessous reste obligatoire dans ces deux cas.
if ($process.ExitCode -notin @(0, 3010, 1638)) { throw "Installation Visual C++ échouée : $($process.ExitCode)" }
foreach ($dll in @('msvcp140.dll', 'msvcp140_1.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')) {
    if (!(Test-Path -LiteralPath (Join-Path $env:SystemRoot "System32/$dll"))) { throw "CRT absente : $dll" }
}
# Échouer avant la compilation si ORT ou une dépendance transitive ne se charge pas.
Add-Type 'using System; using System.Runtime.InteropServices; public static class RhadaOrtProbe { [DllImport("kernel32", CharSet=CharSet.Unicode, SetLastError=true)] public static extern IntPtr LoadLibrary(string path); [DllImport("kernel32")] public static extern bool FreeLibrary(IntPtr module); }'
$ort = Join-Path $env:ORT_LIB_LOCATION 'onnxruntime.dll'
$library = [RhadaOrtProbe]::LoadLibrary($ort)
if ($library -eq [IntPtr]::Zero) { throw "ORT ne se charge pas : erreur $([Runtime.InteropServices.Marshal]::GetLastWin32Error())" }
[void][RhadaOrtProbe]::FreeLibrary($library)
Write-Output '[ci] CRT Visual C++ et chargement ORT vérifiés'
