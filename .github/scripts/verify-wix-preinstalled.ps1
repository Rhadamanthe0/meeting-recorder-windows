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
