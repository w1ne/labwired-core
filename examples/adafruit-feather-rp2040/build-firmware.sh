#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
project_dir="${repo_root}/examples/adafruit-feather-rp2040"
fixture="${repo_root}/tests/fixtures/tier1/feather-rp2040-bme280.elf"
manifest="${repo_root}/tests/fixtures/tier1/MANIFEST.json"

# Compile the customer-facing Arduino/Wire sketch for Adafruit's exact board
# target. Its Arduino startup currently reprograms RP2040 timer alarms faster
# than the simulator's no-cancel event queue can retire the old deadlines, so
# the simulator fixture below is a separate bare-metal smoke image.
pio run --project-dir "${project_dir}" --environment feather_rp2040
mkdir -p "$(dirname "${fixture}")"
pio pkg exec -p toolchain-rp2040-earlephilhower -- arm-none-eabi-gcc \
  -mcpu=cortex-m0plus -mthumb -Os -ffreestanding -fno-builtin \
  -ffunction-sections -fdata-sections -nostdlib \
  -Wl,--gc-sections -Wl,--build-id=none \
  -T "${project_dir}/rp2040-smoke.ld" \
  "${project_dir}/simulator/smoke.c" -o "${fixture}"
echo "built bare-metal RP2040 simulator smoke image: ${fixture}"

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
