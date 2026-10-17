#!/bin/sh
# Compatibility entry point for earlier server deployments.
set -eu
deck_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec "$deck_dir/codex-deck" "$@"
