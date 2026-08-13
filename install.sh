#!/bin/sh
#
# Agentic Software Factory installer (Linux / macOS).
#
# Downloads the latest prebuilt factory binary into $HOME/.local/bin.
# No Rust, Node or administrator rights required.
#
# Usage:
#
#   curl -fsSL https://raw.githubusercontent.com/OmegaMc1331/agentic-software-factory/main/install.sh | sh
#
# Optional environment overrides (mainly for pinning a version or testing):
#   FACTORY_VERSION       install a specific release tag, e.g. v0.1.0
#   FACTORY_BASE_URL      download the archive and checksum from this base URL
#   FACTORY_INSTALL_DIR   install into this directory (default: $HOME/.local/bin;
#                         the PATH check below is skipped for a custom directory)
#   FACTORY_DRY_RUN=1     resolve everything and print what would happen, install nothing

set -eu

REPO="${FACTORY_REPOSITORY:-OmegaMc1331/agentic-software-factory}"
VERSION="${FACTORY_VERSION:-}"
BASE_URL="${FACTORY_BASE_URL:-}"
INSTALL_DIR="${FACTORY_INSTALL_DIR:-$HOME/.local/bin}"
INSTALL_BIN="$INSTALL_DIR/factory"

# --- 1. detect OS and architecture -----------------------------------------
os="$(uname -s)"
hw="$(uname -m)"

case "$os" in
  Linux)  os="linux" ;;
  Darwin) os="macos" ;;
  *) printf 'Unsupported operating system: %s\n' "$os" >&2; exit 1 ;;
esac

case "$hw" in
  x86_64|amd64)  arch="x86_64" ;;
  aarch64|arm64) arch="aarch64" ;;
  *) printf 'Unsupported architecture: %s\n' "$hw" >&2; exit 1 ;;
esac

case "$os-$arch" in
  linux-x86_64)  ASSET="factory-linux-x86_64.tar.gz" ;;
  macos-aarch64) ASSET="factory-macos-aarch64.tar.gz" ;;
  macos-x86_64)  ASSET="factory-macos-x86_64.tar.gz" ;;
  *)
    printf 'Unsupported platform: %s %s (published builds: Linux x86_64, macOS Apple Silicon, macOS Intel)\n' "$os" "$hw" >&2
    exit 1
    ;;
esac

# --- 2. resolve the release tag (a pinned version needs no network lookup) ---
if [ -n "$VERSION" ]; then
  tag="$VERSION"
else
  api_url="https://api.github.com/repos/$REPO/releases/latest"
  tag="$(
    curl -fsSL -H 'User-Agent: factory-installer' "$api_url" |
      grep '"tag_name"' |
      head -n 1 |
      sed -E 's/.*"tag_name"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/'
  )"

  if [ -z "$tag" ]; then
    printf 'Unable to find a recent release. Check the repository and your network connection.\n' >&2
    exit 1
  fi
fi

base="${BASE_URL:-https://github.com/$REPO/releases/download/$tag}"

if [ "${FACTORY_DRY_RUN:-0}" = "1" ]; then
  printf 'dry-run: platform=%s-%s\n' "$os" "$arch"
  printf '  download  %s/%s\n' "$base" "$ASSET"
  printf '  install   %s\n' "$INSTALL_BIN"
  exit 0
fi

command -v curl >/dev/null 2>&1 || { printf 'curl is required to install factory.\n' >&2; exit 1; }

# --- 3. download the archive and its published checksum ----------------------
tmp="$(mktemp -d 2>/dev/null || mktemp -d -t factory-install)"
trap 'rm -rf "$tmp"' EXIT
archive="$tmp/$ASSET"
sha_file="$tmp/$ASSET.sha256"

printf 'Downloading %s (%s)...\n' "$ASSET" "$tag"
curl -fsSL -o "$archive" "$base/$ASSET"
curl -fsSL -o "$sha_file" "$base/$ASSET.sha256"
if [ ! -f "$sha_file" ]; then
  printf 'No published checksum for this release. Refusing to install an unverified binary.\n' >&2
  exit 1
fi

# --- 4. verify the SHA-256 checksum ------------------------------------------
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$archive" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  actual="$(shasum -a 256 "$archive" | awk '{print $1}')"
elif command -v openssl >/dev/null 2>&1; then
  actual="$(openssl dgst -sha256 "$archive" | sed -E 's/.*= *//')"
else
  printf 'No SHA-256 tool found (need sha256sum, shasum or openssl).\n' >&2
  exit 1
fi
expected="$(awk '{print $1}' "$sha_file")"
if [ -z "$expected" ] || [ "$actual" != "$expected" ]; then
  printf 'Checksum mismatch for %s. Aborting.\n' "$ASSET" >&2
  exit 1
fi
printf 'Checksum OK.\n'

# --- 5. extract and install ---------------------------------------------------
tar -xzf "$archive" -C "$tmp" factory
if [ ! -f "$tmp/factory" ]; then
  printf 'The archive does not contain a factory binary.\n' >&2
  exit 1
fi
chmod +x "$tmp/factory"
mkdir -p "$INSTALL_DIR"
mv -f "$tmp/factory" "$INSTALL_BIN"

# --- 6. done ------------------------------------------------------------------
printf '\nInstalled factory %s to %s\n' "$tag" "$INSTALL_BIN"
if [ -z "${FACTORY_INSTALL_DIR:-}" ]; then
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) printf 'Run it with: factory --version\n' ;;
    *)
      printf '%s is not on your PATH. Add this line to your shell profile (~/.bashrc or ~/.zshrc), then run: factory --version\n' "$INSTALL_DIR"
      printf '  export PATH="%s:$PATH"\n' "$INSTALL_DIR"
      ;;
  esac
else
  printf 'Add %s to your PATH, then run: factory --version\n' "$INSTALL_DIR"
fi