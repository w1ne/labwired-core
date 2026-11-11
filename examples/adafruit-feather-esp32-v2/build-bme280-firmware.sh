#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
project_dir="${repo_root}/examples/adafruit-feather-esp32-v2"
fixture="${repo_root}/tests/fixtures/tier1/adafruit-feather-esp32-v2-bme280.elf"
manifest="${repo_root}/tests/fixtures/tier1/MANIFEST.json"
build_dir="${project_dir}/.pio/build/feather_esp32_v2_bme280"

pio run --project-dir "${project_dir}" --environment feather_esp32_v2_bme280
mkdir -p "$(dirname "${fixture}")"
cp "${build_dir}/firmware.elf" "${fixture}"
entry_addr="$(pio pkg exec -p toolchain-xtensa-esp-elf -- \
  xtensa-esp32-elf-nm -n --defined-only "${fixture}" \
  | awk '$3 == "labwired_entry" {print $1; exit}')"
if [[ -z "${entry_addr}" ]]; then
  echo "labwired_entry symbol missing from ${fixture}" >&2
  exit 1
fi
python3 - "${fixture}" "${entry_addr}" <<'PY'
import struct
import sys
from pathlib import Path

fixture = Path(sys.argv[1])
entry = int(sys.argv[2], 16)
data = bytearray(fixture.read_bytes())
if data[:4] != b"\x7fELF" or data[4] != 1 or data[5] != 1:
    raise SystemExit(f"expected a 32-bit little-endian ELF: {fixture}")
struct.pack_into("<I", data, 24, entry)
fixture.write_bytes(data)
print(f"set fast-boot fixture entry to labwired_entry at 0x{entry:08x}")
PY

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
