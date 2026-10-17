# Prints the version in Cargo.toml's [workspace.package]. The release is built from this, so the tag has to match it.
$cargo = Join-Path $PSScriptRoot '..\..\Cargo.toml'
$text = Get-Content $cargo -Raw
if ($text -notmatch '(?ms)\[workspace\.package\].*?^\s*version\s*=\s*"([^"]+)"') {
    Write-Error 'Could not find the workspace version in Cargo.toml.'
    exit 1
}
Write-Output $Matches[1]
