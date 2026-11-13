#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
python3 - "${repo_root}" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

repo = Path(sys.argv[1])
manifest = json.loads(
    (repo / "tests/fixtures/tier1/MANIFEST.json").read_text(encoding="utf-8")
)
fixture = repo / "tests/fixtures/tier1/adafruit-feather-esp32-v2-bme280.elf"
if not fixture.is_file():
    raise SystemExit(f"missing committed fixture: {fixture}")
expected = manifest.get(fixture.name, {}).get("sha256")
actual = hashlib.sha256(fixture.read_bytes()).hexdigest()
if actual != expected:
    raise SystemExit(f"{fixture.name} digest mismatch: {actual} != {expected}")
print(f"committed {fixture.name} digest verified: {actual}")
PY
