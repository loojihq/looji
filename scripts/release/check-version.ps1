# Fails unless the tag is "v" plus the version in Cargo.toml, so a release can't be published under the wrong number.
param([Parameter(Mandatory = $true)][string]$Tag)
$version = & (Join-Path $PSScriptRoot 'version.ps1')
$expected = "v$version"
if ($Tag -ne $expected -and -not $Tag.StartsWith("$expected-")) {
    Write-Host "::error title=Tag and version differ::The tag is $Tag, but Cargo.toml says the version is $version. Tag the release as $expected, or change the version first."
    exit 1
}
Write-Host "Tag $Tag matches version $version."
