# WiX 3.14.1.8722 : empreintes extraites de wix314-binaries.zip officiel,
# vérifié au préalable : 41 297 555 octets et SHA-256
# 6ac824e1642d6f7277d0ed7ea09411a508f6116ba6fae0aa5f2c7daa2ff43d31.
# La sonde de version des exécutables n'est PAS un contrôle d'intégrité.
function Test-CIWiXPreinstalled {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$Directory,
        # Injection autorisée uniquement pour les fixtures de test hors production.
        [hashtable]$ExpectedSha256 = @{
            'candle.exe' = 'a464676e2a2191d58e6ca643e89505bfb9628d3e721873a14994d3f4667987e5'
            'light.exe'  = '8ee3d7fa5592511152a4d1fc605780eae1b27ac7b41d75d518c958515de39529'
            'heat.exe'   = '49119d5454a95a3936ca569f287508beeb3fa9f7bc59fda25f418f2cd234b642'
        }
    )
    # Aucun exécutable WiX n'est lancé dans cette fonction : fail closed.
    foreach ($name in @('candle.exe', 'light.exe', 'heat.exe')) {
        $tool = Join-Path $Directory $name
        if (-not (Test-Path -LiteralPath $tool -PathType Leaf)) {
            Write-Warning "WiX préinstallé : $name absent"
            return $false
        }
        try {
            $actual = (Get-FileHash -LiteralPath $tool -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
        } catch {
            Write-Warning "WiX préinstallé : lecture SHA-256 impossible pour $name"
            return $false
        }
        if ($actual -cne $ExpectedSha256[$name]) {
            Write-Warning "WiX préinstallé : SHA-256 non conforme pour $name"
            return $false
        }
    }
    return $true
}

# WiX v3.14.1: GitHub did not publish a digest for its historical asset.
# Prefer the official GitHub asset digest when present, but NEVER silently adopt
# a changed checksum. The repository-pinned archive SHA-256 remains the trust root.
function Get-CIWiXTrustedArchiveSha256 {
    [CmdletBinding()]
    param()
    $pinned = '6ac824e1642d6f7277d0ed7ea09411a508f6116ba6fae0aa5f2c7daa2ff43d31'
    $url = 'https://api.github.com/repos/wixtoolset/wix3/releases/tags/wix3141rtm'
    try {
        $release = Invoke-RestMethod -Uri $url -Method Get -TimeoutSec 12 -ErrorAction Stop -Headers @{
            Accept = 'application/vnd.github+json'
            'User-Agent' = 'meeting-recorder-wix-integrity'
            'X-GitHub-Api-Version' = '2022-11-28'
        }
    } catch {
        Write-Warning '[ci] Empreinte WiX officielle en ligne indisponible : utilisation du SHA-256 épinglé dans le dépôt.'
        return $pinned
    }
    if ($null -eq $release -or $release.tag_name -cne 'wix3141rtm') {
        throw 'Métadonnées WiX officielles incohérentes : tag inattendu.'
    }
    # Pin the historic asset identity as well as its name and byte length.
    $assets = @($release.assets | Where-Object {
        $_.name -ceq 'wix314-binaries.zip' -and [long]$_.id -eq 158158052
    })
    if ($assets.Count -ne 1 -or [long]$assets[0].size -ne 41297555) {
        throw 'Métadonnées WiX officielles incohérentes : archive inattendue.'
    }
    $digest = [string]$assets[0].digest
    if ([string]::IsNullOrWhiteSpace($digest)) {
        Write-Host '[ci] API GitHub : aucun SHA-256 publié pour WiX 3.14.1 ; SHA-256 épinglé utilisé.'
        return $pinned
    }
    if ($digest -cnotmatch '^sha256:[0-9a-fA-F]{64}$') {
        Write-Warning '[ci] Format du digest WiX officiel non exploitable : SHA-256 épinglé utilisé.'
        return $pinned
    }
    $online = $digest.Substring(7).ToLowerInvariant()
    if ($online -cne $pinned) {
        throw 'Le SHA-256 officiel WiX a changé : refus de mise à jour automatique non approuvée.'
    }
    Write-Host '[ci] SHA-256 WiX officiel récupéré via GitHub et confirmé par le SHA épinglé.'
    return $online
}

function Test-CIWiXArchive {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$ExpectedSha256,
        [long]$ExpectedLength = 41297555
    )
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $false }
    try {
        if ((Get-Item -LiteralPath $Path -ErrorAction Stop).Length -ne $ExpectedLength) {
            Write-Warning '[ci] Cache WiX : taille inattendue.'
            return $false
        }
        $hash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256 -ErrorAction Stop).Hash.ToLowerInvariant()
    } catch {
        Write-Warning '[ci] Cache WiX : lecture ou calcul SHA-256 impossible.'
        return $false
    }
    if ($hash -cne $ExpectedSha256) {
        Write-Warning '[ci] Cache WiX : empreinte SHA-256 incorrecte.'
        return $false
    }
    return $true
}

# Only the pinned ZIP is saved across runs. A GitHub Actions cache cannot be
# overwritten: the workflow saves under a fresh key only after verification.
function Restore-CIWiXVerifiedArchive {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$CacheDirectory,
        [Parameter(Mandatory = $true)][string]$ExpectedSha256,
        [Parameter(Mandatory = $true)][uri]$Uri,
        [long]$ExpectedLength = 41297555
    )
    $null = New-Item -ItemType Directory -Path $CacheDirectory -Force
    $zip = Join-Path $CacheDirectory 'wix314-binaries.zip'
    if (Test-CIWiXArchive -Path $zip -ExpectedSha256 $ExpectedSha256 -ExpectedLength $ExpectedLength) {
        Write-Host '[ci] Archive WiX récupérée depuis le cache et vérifiée.'
        return [pscustomobject]@{ Path = $zip; Refreshed = $false }
    }
    Write-Warning '[ci] Archive WiX absente ou invalide : téléchargement et reconstruction du cache.'
    $download = Join-Path $CacheDirectory ('wix314-download-' + [guid]::NewGuid().ToString('N') + '.zip')
    try {
        Invoke-CIAssetDownload -Uri $Uri -OutFile $download
        if (-not (Test-CIWiXArchive -Path $download -ExpectedSha256 $ExpectedSha256 -ExpectedLength $ExpectedLength)) {
            throw 'Archive WiX téléchargée invalide (taille/SHA-256) : cache non renouvelé.'
        }
        Move-Item -LiteralPath $download -Destination $zip -Force -ErrorAction Stop
    } finally {
        if (Test-Path -LiteralPath $download) {
            Remove-Item -LiteralPath $download -Force
        }
    }
    Write-Host '[ci] Archive WiX officielle vérifiée et cache local renouvelé.'
    return [pscustomobject]@{ Path = $zip; Refreshed = $true }
}
