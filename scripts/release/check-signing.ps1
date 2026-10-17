# Fails, with a message that says what to do, unless the signing certificate and its password are available. This runs
# before anything is built, so a build without credentials stops here and no unsigned release is ever published.
$missing = @()
if ([string]::IsNullOrWhiteSpace($env:WINDOWS_SIGNING_CERT_BASE64)) { $missing += 'WINDOWS_SIGNING_CERT_BASE64' }
if ([string]::IsNullOrWhiteSpace($env:WINDOWS_SIGNING_CERT_PASSWORD)) { $missing += 'WINDOWS_SIGNING_CERT_PASSWORD' }
if ($missing.Count -gt 0) {
    $names = $missing -join ', '
    Write-Host "::error title=Signing credentials are missing::The release was not built because $names is not set. Add it under Settings > Secrets and variables > Actions. looji does not publish unsigned releases."
    exit 1
}
Write-Host 'Signing credentials are present.'
