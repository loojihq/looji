# Writes SHA256SUMS.txt for every file in a folder: a lowercase hash, two spaces, and the file name, one per line. The
# app reads this file to check a downloaded update.
param([Parameter(Mandatory = $true)][string]$Dir)
$Dir = (Resolve-Path $Dir).Path
$lines = Get-ChildItem $Dir -File | Where-Object { $_.Name -ne 'SHA256SUMS.txt' } | Sort-Object Name | ForEach-Object {
    '{0}  {1}' -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name
}
[IO.File]::WriteAllText((Join-Path $Dir 'SHA256SUMS.txt'), (($lines -join "`n") + "`n"))
Write-Host "Wrote $($lines.Count) checksums."
