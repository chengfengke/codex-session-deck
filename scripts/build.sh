#!/bin/sh
set -eu
app_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cargo build --release --locked --manifest-path "$app_dir/Cargo.toml"
case "$(uname -s)" in Linux) sd_os=linux;; Darwin) sd_os=macos;; *) echo 'Use scripts/build.ps1 on Windows.' >&2; exit 1;; esac
case "$(uname -m)" in arm64|aarch64) sd_arch=arm64;; x86_64|amd64) sd_arch=x64;; *) sd_arch=$(uname -m);; esac
mkdir -p "$app_dir/bin"
cp "$app_dir/target/release/codex-deck" "$app_dir/bin/codex-deck-$sd_os-$sd_arch"
cp "$app_dir/target/release/codex-deck" "$app_dir/bin/sessiondesk-$sd_os-$sd_arch"
printf 'Built %s\n' "$app_dir/bin/codex-deck-$sd_os-$sd_arch"
