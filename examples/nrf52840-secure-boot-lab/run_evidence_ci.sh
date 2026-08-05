#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# CRA evidence CI driver: ephemeral OEM key → smoke → evidence pack.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
LAB="$(cd "$(dirname "$0")" && pwd)"
OUT_DIR="${OUT_DIR:-$ROOT/out/nrf52840-secure-boot-evidence}"
PACK_DIR="${PACK_DIR:-$OUT_DIR/cra-evidence-pack}"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/lw-cra-XXXXXX")"
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

echo "==> work dir: $WORK"
echo "==> out dir:  $OUT_DIR"
mkdir -p "$OUT_DIR" "$WORK/gen"

# 1. Firmware
echo "==> build firmware-nrf52840-secure-boot"
(
  cd "$ROOT"
  cargo build -p firmware-nrf52840-secure-boot --release --target thumbv7em-none-eabi
)
FW="$ROOT/target/thumbv7em-none-eabi/release/firmware-nrf52840-secure-boot"
test -f "$FW"

# 2. Ephemeral packages + pubkey
echo "==> make_packages.py --ephemeral"
python3 "$LAB/make_packages.py" --ephemeral --out-dir "$WORK/gen"
PUBHEX="$(tr -d ' \n' < "$WORK/gen/oem-verify-pubkey.hex")"
test "${#PUBHEX}" -eq 128

# 3. system.yaml with oem_pubkey_hex + absolute chip path
export LAB WORK
python3 <<'PY'
from pathlib import Path
import os
lab = Path(os.environ["LAB"])
work = Path(os.environ["WORK"])
pub = (work / "gen" / "oem-verify-pubkey.hex").read_text().strip()
src = (lab / "system.yaml").read_text()
chip = (lab.parent.parent / "configs" / "chips" / "nrf52840.yaml").resolve()
src = src.replace('chip: "../../configs/chips/nrf52840.yaml"', f'chip: "{chip}"')
old = """  - id: "se"
    type: "atecc608a"
    connection: "i2c0"
    config:
      i2c_address: 0x60
"""
new = f"""  - id: "se"
    type: "atecc608a"
    connection: "i2c0"
    config:
      i2c_address: 0x60
      oem_pubkey_hex: "{pub}"
"""
if old not in src:
    raise SystemExit("system.yaml se block not found for patch")
(work / "system.yaml").write_text(src.replace(old, new, 1))
print("wrote", work / "system.yaml")
PY

# 4. Assemble smoke
python3 "$LAB/assemble_smoke.py" \
  --packages "$WORK/gen/packages.yaml" \
  --digests "$WORK/gen/digests.json" \
  --system "$WORK/system.yaml" \
  --firmware "$FW" \
  --out "$WORK/secure-boot-smoke.yaml"

# 5. Run test
echo "==> labwired-cli test"
(
  cd "$ROOT"
  cargo run -q -p labwired-cli -- test \
    --script "$WORK/secure-boot-smoke.yaml" \
    --output-dir "$OUT_DIR" \
    --run-manifest \
    --junit "$OUT_DIR/junit.xml"
)

# 6. Evidence pack
echo "==> build_evidence_pack.py"
set +e
python3 "$LAB/build_evidence_pack.py" \
  --out-dir "$OUT_DIR" \
  --pack-dir "$PACK_DIR" \
  --claims-map "$LAB/claims-map.json" \
  --pubkey-hex "$WORK/gen/oem-verify-pubkey.hex"
PACK_RC=$?
set -e

if grep -R "BEGIN EC PRIVATE "KEY"" "$PACK_DIR" 2>/dev/null; then
  echo "error: private key leaked into evidence pack" >&2
  exit 3
fi

echo "==> pack status exit=$PACK_RC  dir=$PACK_DIR"
ls -la "$PACK_DIR"
exit "$PACK_RC"
