#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
tier1="${repo_root}/tests/fixtures/tier1"

python3 - "${tier1}/MANIFEST.json" \
  "${tier1}/seeed-xiao-esp32c3-bme280.elf" \
  "${tier1}/seeed-xiao-esp32c3-bme280.partitions.bin" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

manifest = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
for fixture in map(Path, sys.argv[2:]):
    if not fixture.is_file():
        raise SystemExit(f"missing committed fixture: {fixture}")
    expected = manifest.get(fixture.name, {}).get("sha256")
    actual = hashlib.sha256(fixture.read_bytes()).hexdigest()
    if actual != expected:
        raise SystemExit(f"{fixture.name} digest mismatch: {actual} != {expected}")
    print(f"committed {fixture.name} digest verified: {actual}")
PY
