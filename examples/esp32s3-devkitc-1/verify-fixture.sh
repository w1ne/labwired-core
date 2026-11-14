#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"

python3 - "${repo_root}/tests/fixtures/tier1/esp32s3.elf" "${repo_root}/tests/fixtures/tier1/MANIFEST.json" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

fixture = Path(sys.argv[1])
manifest = Path(sys.argv[2])
if not fixture.is_file():
    raise SystemExit(f"missing committed ESP32-S3 fixture: {fixture}")
entry = json.loads(manifest.read_text(encoding="utf-8"))[fixture.name]
actual = hashlib.sha256(fixture.read_bytes()).hexdigest()
if actual != entry["sha256"]:
    raise SystemExit(f"fixture digest mismatch: {actual} != {entry['sha256']}")
print(f"committed ESP32-S3 fixture digest verified: {actual}")
PY
