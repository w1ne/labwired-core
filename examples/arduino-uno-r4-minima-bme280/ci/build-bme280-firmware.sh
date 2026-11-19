#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
firmware_bin="${repo_root}/target/thumbv7em-none-eabi/release/firmware-ra4m1-bme280-smoke"
fixture="${repo_root}/tests/fixtures/tier1/arduino-uno-r4-minima-bme280.elf"
manifest="${repo_root}/tests/fixtures/tier1/MANIFEST.json"

cargo build --manifest-path "${repo_root}/Cargo.toml" \
  -p firmware-ra4m1-demo --bin firmware-ra4m1-bme280-smoke \
  --release --target thumbv7em-none-eabi
mkdir -p "$(dirname "${fixture}")"
cp "${firmware_bin}" "${fixture}"

python3 - "${repo_root}" "${fixture}" "${manifest}" <<'PY'
import hashlib
import json
import subprocess
import sys
from pathlib import Path

repo, fixture, manifest = map(Path, sys.argv[1:])
data = json.loads(manifest.read_text(encoding="utf-8"))
source_rev = subprocess.check_output(
    ["git", "-C", str(repo), "rev-parse", "HEAD"], text=True
).strip()
data[fixture.name] = {
    "sha256": hashlib.sha256(fixture.read_bytes()).hexdigest(),
    "source_rev": source_rev,
}
manifest.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
print(f"updated {fixture.name} fixture digest in {manifest}")
PY
