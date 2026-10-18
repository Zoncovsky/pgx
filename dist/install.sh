#!/bin/sh
# PGX installer. Downloads the release tarball for this OS and CPU, verifies
# its SHA-256, and installs it under $PGX_INSTALL_DIR, linking `pgx` into
# $PGX_BIN_DIR. No root, no system PostgreSQL.
#
#   curl -fsSL https://pgx.dev/install | sh
#   PGX_VERSION=0.1.0 PGX_BIN_DIR=~/.local/bin sh install.sh
#
# Environment:
#   PGX_VERSION       version to install (default: latest release)
#   PGX_INSTALL_DIR   where the distribution tree goes (default: ~/.pgx/dist/<version>)
#   PGX_BIN_DIR       where the `pgx` symlink goes     (default: ~/.local/bin)
#   PGX_BASE_URL      release download base            (default: the GitHub releases URL)
set -eu

REPO="pgrundev/pgx"
BASE_URL="${PGX_BASE_URL:-https://github.com/$REPO/releases/download}"
BIN_DIR="${PGX_BIN_DIR:-$HOME/.local/bin}"

err() { echo "pgx installer: $*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

# Platform.
os="$(uname -s)"; arch="$(uname -m)"
case "$os" in
  Linux) os=linux ;;
  Darwin) os=darwin ;;
  *) err "unsupported operating system: $os (PGX supports Linux and macOS)" ;;
esac
case "$arch" in
  x86_64|amd64) arch=amd64 ;;
  aarch64|arm64) arch=arm64 ;;
  *) err "unsupported CPU architecture: $arch (PGX supports amd64 and arm64)" ;;
esac
if [ "$os" = darwin ] && [ "$arch" = amd64 ]; then
  err "macOS on Intel is not a supported target; supported: darwin-arm64, linux-amd64, linux-arm64"
fi

# Downloader.
if have curl; then dl() { curl -fsSL "$1" -o "$2"; }; dlout() { curl -fsSL "$1"; }
elif have wget; then dl() { wget -qO "$2" "$1"; }; dlout() { wget -qO- "$1"; }
else err "need curl or wget"; fi

version="${PGX_VERSION:-}"
if [ -z "$version" ]; then
  version="$(dlout "https://api.github.com/repos/$REPO/releases/latest" | sed -n 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/p' | head -1)"
  [ -n "$version" ] || err "could not determine the latest version; set PGX_VERSION"
fi
version="${version#v}"
install_dir="${PGX_INSTALL_DIR:-$HOME/.pgx/dist/$version}"

name="pgx-$version-$os-$arch"
url="$BASE_URL/v$version/$name.tar.gz"
sum_url="$url.sha256"

tmp="$(mktemp -d "${TMPDIR:-/tmp}/pgx-install.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT INT TERM
echo "pgx installer: downloading $name ..."
dl "$url" "$tmp/$name.tar.gz" || err "download failed: $url"
dl "$sum_url" "$tmp/$name.tar.gz.sha256" || err "checksum download failed: $sum_url"

echo "pgx installer: verifying checksum ..."
expected="$(awk '{print $1}' "$tmp/$name.tar.gz.sha256")"
if have sha256sum; then actual="$(sha256sum "$tmp/$name.tar.gz" | awk '{print $1}')"
elif have shasum; then actual="$(shasum -a 256 "$tmp/$name.tar.gz" | awk '{print $1}')"
else err "need sha256sum or shasum to verify the download"; fi
[ -n "$expected" ] || err "empty checksum file"
[ "$expected" = "$actual" ] || err "checksum mismatch (expected $expected, got $actual); refusing to install"

echo "pgx installer: extracting ..."
tar -xzf "$tmp/$name.tar.gz" -C "$tmp" || err "extract failed"
[ -x "$tmp/$name/bin/pgx" ] || err "archive does not contain bin/pgx"

# Install atomically: into <dir>.partial then rename.
mkdir -p "$(dirname "$install_dir")"
rm -rf "$install_dir.partial"
cp -R "$tmp/$name" "$install_dir.partial"
rm -rf "$install_dir"
mv "$install_dir.partial" "$install_dir"

mkdir -p "$BIN_DIR"
ln -sf "$install_dir/bin/pgx" "$BIN_DIR/pgx"

# Verify.
out="$("$install_dir/bin/pgx" version 2>&1)" || err "installed, but 'pgx version' failed: $out"
echo "pgx installer: installed $out"
echo "pgx installer: linked $BIN_DIR/pgx -> $install_dir/bin/pgx"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "pgx installer: add $BIN_DIR to your PATH:  export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac
echo "pgx installer: next: pgx init && pgx start"
