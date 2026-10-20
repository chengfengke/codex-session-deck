#!/bin/sh
set -eu
deck_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$deck_dir"
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --release --locked
cargo test --release --locked large_history_render_benchmark -- --ignored --nocapture
cargo build --release --locked
python3 tests/responsiveness.py target/release/codex-deck
python3 tests/manager_pty.py target/release/codex-deck
python3 tests/model_picker_pty.py target/release/codex-deck
python3 tests/search_integration.py target/release/codex-deck
python3 tests/workspace_pty.py target/release/codex-deck
python3 tests/activity_pty.py target/release/codex-deck
python3 tests/rename_compat.py target/release/codex-deck "${CODEX_DECK_LEGACY_BINARY:-target/release/codex-deck}"
python3 tests/integration.py target/release/codex-deck
python3 tests/pty_smoke.py target/release/codex-deck
python3 tests/pty_history_perf.py target/release/codex-deck
