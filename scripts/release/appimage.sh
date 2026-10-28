#!/usr/bin/env bash
# Builds the Linux AppImage from the release binary: one file that runs on most Linux desktops without installing. Run
# from the repository root, after `cargo build --release --locked --bin looji`. The file goes to ./dist.
#
# The AppImage holds the program, its desktop entry and its icon. It uses the libraries already on the computer, such as
# Vulkan, fontconfig, and Wayland or X11. appimagetool is downloaded at a pinned version and checked against its published
# SHA-256 before it runs.
set -euo pipefail

APPIMAGETOOL_VERSION=1.9.1
APPIMAGETOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0

version=$(awk '/^\[workspace\.package\]/{p=1} p && /^version *=/{gsub(/[" ]/,"",$3); print $3; exit}' Cargo.toml)
if [[ -z "$version" ]]; then
    echo "::error::Could not read the version from Cargo.toml" >&2
    exit 1
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

tool="$work/appimagetool"
curl -fsSL -o "$tool" \
    "https://github.com/AppImage/appimagetool/releases/download/${APPIMAGETOOL_VERSION}/appimagetool-x86_64.AppImage"
echo "${APPIMAGETOOL_SHA256}  ${tool}" | sha256sum -c -
chmod +x "$tool"

appdir="$work/AppDir"
install -Dm755 target/release/looji "$appdir/usr/bin/looji"
install -Dm644 packaging/linux/looji.desktop "$appdir/looji.desktop"
install -Dm644 assets/branding/app-icon-512.png "$appdir/looji.png"
ln -s looji.png "$appdir/.DirIcon"
cat > "$appdir/AppRun" <<'APPRUN'
#!/bin/sh
here=$(dirname "$(readlink -f "$0")")
exec "$here/usr/bin/looji" "$@"
APPRUN
chmod +x "$appdir/AppRun"

mkdir -p dist
output="dist/looji-${version}-linux-x64.AppImage"
# The extract-and-run mode lets appimagetool run on a machine without FUSE, such as a CI runner.
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$tool" -n "$appdir" "$output"
chmod +x "$output"

# Check the file: it unpacks, it holds the program and the desktop entry, and the program finds every library it links to.
check="$work/check"
mkdir "$check"
built="$(pwd)/$output"
(cd "$check" && "$built" --appimage-extract >/dev/null)
test -x "$check/squashfs-root/usr/bin/looji"
test -f "$check/squashfs-root/looji.desktop"
if ldd "$check/squashfs-root/usr/bin/looji" | grep -q "not found"; then
    echo "::error::The program needs a library that is not on this computer:" >&2
    ldd "$check/squashfs-root/usr/bin/looji" | grep "not found" >&2
    exit 1
fi

ls -l dist
