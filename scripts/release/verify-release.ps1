# Checks a release folder before it is published: every file matches SHA256SUMS.txt, and, with -RequireSigned, every
# .exe carries a valid signature. Exits 1 on the first problem, so nothing wrong is published.
param(
    [Parameter(Mandatory = $true)][string]$Dir,
    [switch]$RequireSigned
)
$Dir = (Resolve-Path $Dir).Path
$sums = Join-Path $Dir 'SHA256SUMS.txt'
if (-not (Test-Path $sums)) { Write-Host '::error::SHA256SUMS.txt is missing.'; exit 1 }
$expected = @{}
foreach ($line in Get-Content $sums) {
    if ($line -match '^([0-9a-fA-F]{64})\s+\*?(.+)$') { $expected[$Matches[2].Trim()] = $Matches[1].ToLower() }
}
$files = Get-ChildItem $Dir -File | Where-Object { $_.Name -ne 'SHA256SUMS.txt' }
if ($files.Count -eq 0) { Write-Host '::error::The release folder has no files.'; exit 1 }
foreach ($file in $files) {
    if (-not $expected.ContainsKey($file.Name)) { Write-Host "::error::$($file.Name) has no entry in SHA256SUMS.txt."; exit 1 }
    $actual = (Get-FileHash $file.FullName -Algorithm SHA256).Hash.ToLower()
    if ($actual -ne $expected[$file.Name]) { Write-Host "::error::$($file.Name) does not match its checksum."; exit 1 }
}
if ($RequireSigned) {
    foreach ($file in $files | Where-Object { $_.Extension -eq '.exe' }) {
        $signature = Get-AuthenticodeSignature $file.FullName
        if ($signature.Status -ne 'Valid') { Write-Host "::error::$($file.Name) is not validly signed (status: $($signature.Status)). It was not published."; exit 1 }
    }
}
Write-Host "Verified $($files.Count) files."
