#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
python3 -m venv "$project_dir/tools/esptool"
"$project_dir/tools/esptool/bin/python" -m pip install 'esptool==5.6.0'
