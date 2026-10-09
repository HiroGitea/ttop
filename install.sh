#!/bin/sh
# ttop installer: downloads a prebuilt static binary from GitHub Releases.
#
#   curl -fsSL https://raw.githubusercontent.com/HiroGitea/ttop/main/install.sh | sh
#
# Environment variables:
#   TTOP_VERSION      release tag to install, e.g. v0.1.0 (default: latest)
#   TTOP_INSTALL_DIR  installation directory (default: ~/.local/bin, which lets
#                     ttop update itself without sudo)
set -eu

REPO="HiroGitea/ttop"

say() { printf 'ttop-install: %s\n' "$*" >&2; }
die() { say "error: $*"; exit 1; }

# A path is writable if it exists and is writable, or its nearest existing parent is.
writable() {
    if [ -e "$1" ]; then [ -w "$1" ]; else writable "$(dirname "$1")"; fi
}

case "$(uname -s)" in
    Linux) ;;
    *) die "unsupported OS: $(uname -s) (ttop only runs on Linux)" ;;
esac

case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) die "unsupported architecture: $(uname -m)" ;;
esac
asset="ttop-linux-$arch"

version="${TTOP_VERSION:-latest}"
if [ "$version" = latest ]; then
    base="https://github.com/$REPO/releases/latest/download"
else
    base="https://github.com/$REPO/releases/download/$version"
fi

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL --retry 3 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q -O "$2" "$1"; }
else
    die "curl or wget is required"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
trap 'exit 130' INT TERM

say "downloading $asset ($version)"
fetch "$base/$asset" "$tmp/ttop" || die "download failed: $base/$asset"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || die "download failed: $base/SHA256SUMS"

expected="$(awk -v f="$asset" '$2 == f || $2 == "*" f { print $1 }' "$tmp/SHA256SUMS")"
[ -n "$expected" ] || die "no checksum for $asset in SHA256SUMS"
if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$tmp/ttop" | awk '{ print $1 }')"
elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "$tmp/ttop" | awk '{ print $1 }')"
else
    die "sha256sum or shasum is required to verify the download"
fi
[ "$expected" = "$actual" ] || die "checksum mismatch for $asset"
chmod 755 "$tmp/ttop"

dir="${TTOP_INSTALL_DIR:-$HOME/.local/bin}"
sudo=""
if ! writable "$dir"; then
    command -v sudo >/dev/null 2>&1 || die "$dir is not writable and sudo is not available"
    sudo="sudo"
    say "$dir is not writable, using sudo (automatic updates will need: sudo ttop --update)"
fi

$sudo mkdir -p "$dir"
$sudo cp "$tmp/ttop" "$dir/ttop.new"
$sudo mv -f "$dir/ttop.new" "$dir/ttop"

say "installed $("$dir/ttop" --version) to $dir/ttop"
case ":$PATH:" in
    *":$dir:"*) ;;
    *) say "note: $dir is not in PATH; to run ttop by name, add this line to your shell profile:"
       say "  export PATH=\"$dir:\$PATH\"" ;;
esac
