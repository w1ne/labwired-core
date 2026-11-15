#!/usr/bin/env bash
# Rebuild the committed XIAO ESP32C3 + BME280 fixture from the stock Arduino
# sketch, and record its digest. The partition table goes beside the ELF as
# <stem>.partitions.bin; the ESP-IDF app aborts at boot without it.
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
project_dir="${repo_root}/examples/xiao-esp32c3-bme280/firmware"
fixture="${repo_root}/tests/fixtures/tier1/seeed-xiao-esp32c3-bme280.elf"
partitions="${repo_root}/tests/fixtures/tier1/seeed-xiao-esp32c3-bme280.partitions.bin"
manifest="${repo_root}/tests/fixtures/tier1/MANIFEST.json"
build_dir="${project_dir}/.pio/build/xiao_esp32c3_bme280"

pio run --project-dir "${project_dir}" --environment xiao_esp32c3_bme280
mkdir -p "$(dirname "${fixture}")"
cp "${build_dir}/firmware.elf" "${fixture}"
cp "${build_dir}/partitions.bin" "${partitions}"

python3 - "${repo_root}" "${manifest}" "${fixture}" "${partitions}" <<'PY'
import hashlib
import json
import subprocess
import sys
from pathlib import Path

repo, manifest, *files = map(Path, sys.argv[1:])
data = json.loads(manifest.read_text(encoding="utf-8"))
source_rev = subprocess.check_output(
    ["git", "-C", str(repo), "rev-parse", "HEAD"], text=True
).strip()
for path in files:
    data[path.name] = {
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "source_rev": source_rev,
    }
    print(f"updated {path.name} digest in {manifest}")
manifest.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
PY
