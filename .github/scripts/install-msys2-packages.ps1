param([Parameter(Mandatory = $true)][string]$Root)
$ErrorActionPreference = "Stop"
$bash = Join-Path $Root "usr\bin\bash.exe"
if (-not (Test-Path -LiteralPath $bash)) { throw "MSYS2 bash absent : $bash" }
$env:MSYSTEM = "UCRT64"
$env:CHERE_INVOKING = "1"
function Invoke-Msys([string]$Command) {
    & $bash --login -eo pipefail -c $Command
    if ($LASTEXITCODE -ne 0) { throw "Commande MSYS2 en échec (exit $LASTEXITCODE) : $Command" }
}
# Le keyring est celui du MSYS2 temporaire créé par setup-msys2. Fermer son
# agent avant la mise à jour permet aux processus de quitter normalement,
# sans taskkill global sur les terminaux ou les agents GPG du propriétaire.
Invoke-Msys 'gpgconf --homedir /etc/pacman.d/gnupg --kill all'
Invoke-Msys 'exec pacman --noconfirm -Syuu'
# Nouvelle session après une éventuelle mise à jour du runtime MSYS2.
# Chaque erreur, y compris une signature invalide, reste bloquante.
Invoke-Msys 'exec pacman --noconfirm -Syuu'
Invoke-Msys 'exec pacman --noconfirm -S --needed mingw-w64-ucrt-x86_64-gtk4 mingw-w64-ucrt-x86_64-libadwaita mingw-w64-ucrt-x86_64-pkgconf mingw-w64-ucrt-x86_64-gcc mingw-w64-ucrt-x86_64-clang mingw-w64-ucrt-x86_64-make mingw-w64-ucrt-x86_64-cmake'
