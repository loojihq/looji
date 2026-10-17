#!/usr/bin/env bash
# Builds the Linux packages: a .deb and an .rpm from the release binary, plus a source tarball and an Arch PKGBUILD that
# builds from it. Run from the repository root, after `cargo build --release --locked --bin looji`. The files go to ./dist.
#
# The source tarball is made with git archive from the commit being released, and the PKGBUILD carries its checksum, so
# the PKGBUILD and the tarball are published together and match.
set -euo pipefail

version=$(awk '/^\[workspace\.package\]/{p=1} p && /^version *=/{gsub(/[" ]/,"",$3); print $3; exit}' Cargo.toml)
if [[ -z "$version" ]]; then
    echo "::error::Could not read the version from Cargo.toml" >&2
    exit 1
fi

mkdir -p dist

cargo deb -p looji-ui --no-build --output "dist/looji-${version}-linux-x64.deb"
cargo generate-rpm -p crates/ui --output "dist/looji-${version}-linux-x64.rpm"

git archive --format=tar.gz --prefix="looji-${version}/" -o "dist/looji-${version}.tar.gz" HEAD
source_sha=$(sha256sum "dist/looji-${version}.tar.gz" | cut -d' ' -f1)

sed -e "s/^pkgver=.*/pkgver=${version}/" -e "s/^sha256sums=.*/sha256sums=('${source_sha}')/" \
    packaging/arch/PKGBUILD > dist/PKGBUILD

# Check that the .deb and .rpm hold the program and the desktop entry, and that the .deb lists the libraries it needs.
dpkg-deb --contents "dist/looji-${version}-linux-x64.deb" | grep -q 'usr/bin/looji'
if [[ -z "$(dpkg-deb -f "dist/looji-${version}-linux-x64.deb" Depends)" ]]; then
    echo "::error::The .deb has no Depends. The library scan failed, so it would install without its libraries." >&2
    exit 1
fi
rpm -qlp "dist/looji-${version}-linux-x64.rpm" | grep -q '/usr/bin/looji'
bash -n dist/PKGBUILD

ls -l dist
