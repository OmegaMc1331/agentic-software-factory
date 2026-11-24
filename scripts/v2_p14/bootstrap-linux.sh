#!/bin/sh
set -eu
tool_root=${ASF_P14_TOOL_ROOT:-${XDG_DATA_HOME:-$HOME/.local/share}/asf-p14-tools}
mkdir -p "$tool_root/downloads"
export CARGO_HOME="$tool_root/cargo"
export RUSTUP_HOME="$tool_root/rustup"
if [ ! -x "$CARGO_HOME/bin/rustup" ]; then
  curl -fLsS https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init -o "$tool_root/downloads/rustup-init"
  curl -fLsS https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init.sha256 -o "$tool_root/downloads/rustup-init.sha256"
  expected=$(awk '{print $1}' "$tool_root/downloads/rustup-init.sha256")
  printf '%s  %s\n' "$expected" "$tool_root/downloads/rustup-init" | sha256sum -c -
  chmod 755 "$tool_root/downloads/rustup-init"
  "$tool_root/downloads/rustup-init" -y --no-modify-path --profile minimal --default-toolchain 1.99.1 --component rustfmt --component clippy
fi
if [ ! -x "$tool_root/node-v24.22.1-linux-x64/bin/node" ]; then
  curl -fLsS https://nodejs.org/dist/v24.22.1/node-v24.22.1-linux-x64.tar.xz -o "$tool_root/downloads/node-v24.22.1-linux-x64.tar.xz"
  curl -fLsS https://nodejs.org/dist/v24.22.1/SHASUMS256.txt -o "$tool_root/downloads/node-SHASUMS256.txt"
  (cd "$tool_root/downloads" && grep '  node-v24.22.1-linux-x64.tar.xz$' node-SHASUMS256.txt | sha256sum -c -)
  tar -xJf "$tool_root/downloads/node-v24.22.1-linux-x64.tar.xz" -C "$tool_root"
fi
export PATH="$CARGO_HOME/bin:$tool_root/node-v24.22.1-linux-x64/bin:$PATH"
if [ ! -x "$tool_root/npm/bin/npm" ]; then
  npm install --global --prefix "$tool_root/npm" --ignore-scripts npm@12.0.2
fi
export PATH="$tool_root/npm/bin:$PATH"
rustc -Vv
cargo -V
node --version
npm --version
command -v cc
if ! command -v pkg-config; then printf 'pkg-config is unavailable; native build requirements must be checked before qualification.\n'; fi
