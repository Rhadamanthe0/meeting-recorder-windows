param([Parameter(Mandatory = $true)][string]$Root)
$ErrorActionPreference = "Stop"
$bash = Join-Path $Root "usr\bin\bash.exe"
if (-not (Test-Path -LiteralPath $bash)) { throw "MSYS2 bash absent : $bash" }
$env:MSYSTEM = "UCRT64"
$env:CHERE_INVOKING = "1"
function Invoke-Msys([string]$Command, [switch]$RetryDownload) {
    # Traiter nous-mêmes le code natif, même si l'appelant a activé cette option.
    $PSNativeCommandUseErrorActionPreference = $false
    $maxAttempts = if ($RetryDownload) { 3 } else { 1 }
    for ($attempt = 1; $attempt -le $maxAttempts; $attempt++) {
        $output = [System.Collections.Generic.List[string]]::new()
        # Locale stable pour reconnaître les diagnostics, tout en gardant UTF-8.
        & $bash --login -eo pipefail -c "export LC_ALL=C.UTF-8; $Command" 2>&1 | ForEach-Object {
            $line = [string]$_
            $output.Add($line)
            Write-Host $line
        }
        $exitCode = $LASTEXITCODE
        if ($exitCode -eq 0) { return }

        # Ne rejouer que les échecs de transport reconnus. Une signature invalide,
        # un certificat TLS refusé, un conflit ou toute autre erreur reste bloquant.
        $networkError = '^error: failed retrieving file .+ from .+ : (?:Operation too slow\.|(?:Operation|Connection) timed out|Could not resolve host|Failed to connect|Recv failure:|Send failure:|SSL connection timeout|The requested URL returned error: (?:408|429|5[0-9]{2})(?:\s|$))'
        $summaryError = '^error: (?:failed to commit transaction \(unexpected error\)|failed to synchronize all databases \(failed to retrieve some files\))$'
        $errors = @($output | Where-Object { $_ -match '^error:' })
        $downloads = @($errors | Where-Object { $_ -match $networkError })
        $otherErrors = @($errors | Where-Object { $_ -notmatch $networkError -and $_ -notmatch $summaryError })
        if (-not $RetryDownload -or $attempt -eq $maxAttempts -or $downloads.Count -eq 0 -or $otherErrors.Count -ne 0) {
            throw "Commande MSYS2 en échec (exit $exitCode, tentative $attempt/$maxAttempts) : $Command"
        }
        # Conserver le cache des paquets déjà reçus et les délais réseau pacman.
        # Chaque tentative ouvre une nouvelle session ; aucun processus tiers arrêté.
        $delay = 10 * $attempt
        Write-Warning "[ci] Téléchargement MSYS2 interrompu ; tentative $($attempt + 1)/$maxAttempts dans $delay secondes."
        Start-Sleep -Seconds $delay
    }
}
# Le keyring est celui du MSYS2 temporaire créé par setup-msys2. Fermer son
# agent avant la mise à jour permet aux processus de quitter normalement,
# sans taskkill global sur les terminaux ou les agents GPG du propriétaire.
Invoke-Msys 'gpgconf --homedir /etc/pacman.d/gnupg --kill all'
Invoke-Msys 'exec pacman --noconfirm -Syuu' -RetryDownload
# Nouvelle session après une éventuelle mise à jour du runtime MSYS2.
# Chaque erreur, y compris une signature invalide, reste bloquante.
Invoke-Msys 'exec pacman --noconfirm -Syuu' -RetryDownload
Invoke-Msys 'exec pacman --noconfirm -S --needed mingw-w64-ucrt-x86_64-gtk4 mingw-w64-ucrt-x86_64-libadwaita mingw-w64-ucrt-x86_64-pkgconf mingw-w64-ucrt-x86_64-gcc mingw-w64-ucrt-x86_64-clang mingw-w64-ucrt-x86_64-make mingw-w64-ucrt-x86_64-cmake' -RetryDownload
