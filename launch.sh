#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
if [[ -x target/release/meshtastic-gtk ]]; then
    exec target/release/meshtastic-gtk "$@"
fi
exec cargo run --locked -- "$@"
