#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../../.." && pwd)"

python3 - "${repo_root}/tests/fixtures/tier1/esp32s3-devkitc1-rgb.elf" \
  "${repo_root}/tests/fixtures/tier1/esp32s3-devkitc1-rgb-flash.bin" \
  "${repo_root}/tests/fixtures/tier1/MANIFEST.json" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

fixture = Path(sys.argv[1])
flash_fixture = Path(sys.argv[2])
manifest = json.loads(Path(sys.argv[3]).read_text(encoding="utf-8"))
for image in (fixture, flash_fixture):
    if not image.is_file():
        raise SystemExit(f"missing committed fixture: {image}")
    expected = manifest.get(image.name, {}).get("sha256")
    actual = hashlib.sha256(image.read_bytes()).hexdigest()
    if actual != expected:
        raise SystemExit(f"{image.name} digest mismatch: {actual} != {expected}")
    print(f"committed {image.name} digest verified: {actual}")
PY
