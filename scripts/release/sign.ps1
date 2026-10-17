# Signs a file with the certificate in the WINDOWS_SIGNING_CERT_BASE64 secret (a base64 .pfx) and checks the signature.
# -AllowUntrusted accepts a signature that is valid but doesn't chain to a trusted root. It is for testing this script
# with a self-signed certificate. The release workflow never uses it.
param(
    [Parameter(Mandatory = $true)][string]$Path,
    [switch]$AllowUntrusted
)
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'check-signing.ps1')
# $LASTEXITCODE is $null when that script ended without an exit code, so test for a non-zero value, not for "not 0".
if ($LASTEXITCODE) { exit 1 }

$signtool = (Get-Command signtool.exe -ErrorAction SilentlyContinue).Source
if (-not $signtool) {
    $signtool = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin' -Filter signtool.exe -Recurse -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName.Contains([IO.Path]::DirectorySeparatorChar + 'x64' + [IO.Path]::DirectorySeparatorChar) } | Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $signtool) { Write-Host '::error::signtool.exe was not found.'; exit 1 }

$pfx = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName() + '.pfx')
try {
    [IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:WINDOWS_SIGNING_CERT_BASE64))
    & $signtool sign /f $pfx /p $env:WINDOWS_SIGNING_CERT_PASSWORD /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $Path
    if ($LASTEXITCODE -ne 0) { Write-Host "::error::signtool could not sign $Path."; exit 1 }
}
finally {
    Remove-Item $pfx -Force -ErrorAction SilentlyContinue
}

if ($AllowUntrusted) {
    $signature = Get-AuthenticodeSignature $Path
    if (-not $signature.SignerCertificate) { Write-Host "::error::$Path has no signature after signing."; exit 1 }
} else {
    & $signtool verify /pa $Path
    if ($LASTEXITCODE -ne 0) { Write-Host "::error::The signature on $Path did not verify."; exit 1 }
}
Write-Host "Signed $Path."
