#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
if [[ -x target/release/meshtastic-rust ]]; then
    exec target/release/meshtastic-rust "$@"
fi
exec cargo run --locked -- "$@"
