#!/bin/sh
set -eu
sd_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ -x "$sd_dir/.runtime/cargo/bin/cargo" ]; then
  export RUSTUP_HOME="${RUSTUP_HOME:-$sd_dir/.runtime/rustup}"
  export CARGO_HOME="${CARGO_HOME:-$sd_dir/.runtime/cargo}"
  export PATH="$sd_dir/.runtime/bin:$sd_dir/.runtime/cargo/bin:$PATH"
  if [ -x "$sd_dir/.runtime/bin/cc" ]; then
    export CC="${CC:-$sd_dir/.runtime/bin/cc}"
    export AR="${AR:-$sd_dir/.runtime/bin/ar}"
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="${CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER:-$CC}"
  fi
fi
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
exec "$sd_dir/scripts/build.sh" "$@"
