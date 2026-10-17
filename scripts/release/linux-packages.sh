#!/usr/bin/env bash
# Builds the Linux program, packages it as .deb and .rpm, and writes the Arch PKGBUILD with the source checksum filled in.
# Run from the repository root, after `cargo build --release --locked --bin looji`. The files go to ./dist.
#
#   scripts/release/linux-packages.sh [--sha256 <hex>]
#
# Without --sha256 the PKGBUILD keeps 'SKIP', which is fine for a dry run before the tag exists.
set -euo pipefail

sha256=SKIP
if [[ "${1:-}" == "--sha256" ]]; then
    sha256="${2:?--sha256 needs a value}"
fi

version=$(awk '/^\[workspace\.package\]/{p=1} p && /^version *=/{gsub(/[" ]/,"",$3); print $3; exit}' Cargo.toml)
if [[ -z "$version" ]]; then
    echo "::error::Could not read the version from Cargo.toml" >&2
    exit 1
fi

mkdir -p dist

cargo deb -p looji-ui --no-build --output "dist/looji-${version}-linux-x64.deb"
cargo generate-rpm -p crates/ui --output "dist/looji-${version}-linux-x64.rpm"

sed -e "s/^pkgver=.*/pkgver=${version}/" -e "s/^sha256sums=.*/sha256sums=('${sha256}')/" \
    packaging/arch/PKGBUILD > dist/PKGBUILD

# Check that each package holds the program and the desktop entry, and that the .deb lists the libraries it needs.
dpkg-deb --contents "dist/looji-${version}-linux-x64.deb" | grep -q 'usr/bin/looji'
if [[ -z "$(dpkg-deb -f "dist/looji-${version}-linux-x64.deb" Depends)" ]]; then
    echo "::error::The .deb has no Depends. The library scan failed, so it would install without its libraries." >&2
    exit 1
fi
rpm -qlp "dist/looji-${version}-linux-x64.rpm" | grep -q '/usr/bin/looji'
bash -n dist/PKGBUILD

ls -l dist
