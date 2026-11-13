#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
project_dir="${repo_root}/examples/esp32s3-devkitc-1/rgb-smoke/firmware"
pio run --project-dir "${project_dir}" --environment devkitc1_rgb

fixture="${repo_root}/tests/fixtures/tier1/esp32s3-devkitc1-rgb.elf"
flash_fixture="${repo_root}/tests/fixtures/tier1/esp32s3-devkitc1-rgb-flash.bin"
build_dir="${project_dir}/.pio/build/devkitc1_rgb"
mkdir -p "$(dirname "${fixture}")"
cp "${build_dir}/firmware.elf" "${fixture}"
entry_addr="$(pio pkg exec -p toolchain-xtensa-esp-elf -- \
  xtensa-esp32s3-elf-nm -n --defined-only "${fixture}" \
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

platformio_core_dir="$(pio system info --json-output | python3 -c 'import json,sys; print(json.load(sys.stdin)["core_dir"]["value"])')"
pio pkg exec -p tool-esptool -- esptool --chip esp32s3 merge-bin \
  --output "${flash_fixture}" \
  --pad-to-size 4MB \
  --flash-mode keep --flash-freq keep --flash-size keep \
  0x0 "${build_dir}/bootloader.bin" \
  0x8000 "${build_dir}/partitions.bin" \
  0xe000 "${platformio_core_dir}/packages/framework-arduinoespressif32/tools/partitions/boot_app0.bin" \
  0x10000 "${build_dir}/firmware.bin"

python3 - "${repo_root}" "${fixture}" "${flash_fixture}" <<'PY'
import hashlib
import json
import subprocess
import sys
from pathlib import Path

repo = Path(sys.argv[1])
fixture = Path(sys.argv[2])
flash_fixture = Path(sys.argv[3])
manifest_path = repo / "tests/fixtures/tier1/MANIFEST.json"
manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
source_rev = subprocess.check_output(
    ["git", "-C", str(repo), "rev-parse", "HEAD"], text=True
).strip()
manifest[fixture.name] = {
    "sha256": hashlib.sha256(fixture.read_bytes()).hexdigest(),
    "source_rev": source_rev,
}
manifest[flash_fixture.name] = {
    "sha256": hashlib.sha256(flash_fixture.read_bytes()).hexdigest(),
    "source_rev": source_rev,
}
manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
print(f"updated {fixture.name} fixture digest in {manifest_path}")
print(f"updated {flash_fixture.name} fixture digest in {manifest_path}")
PY
