#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"

python3 - "${repo_root}/tests/fixtures/tier1/esp32s3-devkitc1-n16r8-bme280.elf" \
  "${repo_root}/tests/fixtures/tier1/MANIFEST.json" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

fixture = Path(sys.argv[1])
manifest = json.loads(Path(sys.argv[2]).read_text(encoding="utf-8"))
if not fixture.is_file():
    raise SystemExit(f"missing committed fixture: {fixture}")
expected = manifest.get(fixture.name, {}).get("sha256")
actual = hashlib.sha256(fixture.read_bytes()).hexdigest()
if actual != expected:
    raise SystemExit(f"{fixture.name} digest mismatch: {actual} != {expected}")
print(f"committed {fixture.name} digest verified: {actual}")
PY
