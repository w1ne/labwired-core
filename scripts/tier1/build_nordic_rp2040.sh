#!/usr/bin/env bash
# Rebuild the nRF52832, nRF52833, nRF52840, and RP2040 Tier-1 fixture ELFs
# from source and refresh the MANIFEST.json entry for each blob.
#
# nrf52832/nrf52840/rp2040 are workspace members, so their ELFs land in the
# workspace-root target/. nrf52833 is a STANDALONE crate (own [workspace]) and
# builds in its own target/ dir, like the stm32 fixtures.
#
# Usage: scripts/tier1/build_nordic_rp2040.sh [--refresh-manifest|--nordic-only]
#   --nordic-only rebuild only the three Nordic blobs and update only their
#                 manifest entries; preserve unrelated source provenance.
#   --refresh-manifest  (default) recompute sha256 for all blobs in
#                       tests/fixtures/tier1/MANIFEST.json, not just the
#                       four produced here.
#
# Prerequisites:
#   - stable Rust toolchain with thumbv7em-none-eabi + thumbv6m-none-eabi
#     targets installed (`rustup target add thumbv7em-none-eabi thumbv6m-none-eabi`)
#   - run from the workspace root or any subdirectory (the script resolves ROOT)
set -euo pipefail

NORDIC_ONLY=0
case "${1:-}" in
  --nordic-only) NORDIC_ONLY=1 ;;
  ''|--refresh-manifest) ;;
  *) echo "Unknown argument: $1" >&2; exit 2 ;;
esac

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/tests/fixtures/tier1"
mkdir -p "$OUT"

echo "==> Building nRF52832 fixture (thumbv7em-none-eabi)..."
(cd "$ROOT/examples/tier1-fixture/nrf52832" \
  && cargo build --release --target thumbv7em-none-eabi)
cp "$ROOT/target/thumbv7em-none-eabi/release/tier1-fixture-nrf52832" \
   "$OUT/nrf52832.elf"
echo "    nrf52832.elf -> $OUT/nrf52832.elf"

echo "==> Building nRF52833 fixture (thumbv7em-none-eabi, standalone crate)..."
(cd "$ROOT/examples/tier1-fixture/nrf52833" \
  && cargo build --release --target thumbv7em-none-eabi)
cp "$ROOT/examples/tier1-fixture/nrf52833/target/thumbv7em-none-eabi/release/tier1-fixture-nrf52833" \
   "$OUT/nrf52833.elf"
echo "    nrf52833.elf -> $OUT/nrf52833.elf"

echo "==> Building nRF52840 fixture (thumbv7em-none-eabi)..."
(cd "$ROOT/examples/tier1-fixture/nrf52840" \
  && cargo build --release --target thumbv7em-none-eabi)
cp "$ROOT/target/thumbv7em-none-eabi/release/tier1-fixture-nrf52840" \
   "$OUT/nrf52840.elf"
echo "    nrf52840.elf -> $OUT/nrf52840.elf"

if [ "$NORDIC_ONLY" -eq 0 ]; then
echo "==> Building RP2040 fixture (thumbv6m-none-eabi)..."
(cd "$ROOT/examples/tier1-fixture/rp2040" \
  && cargo build --release --target thumbv6m-none-eabi)
cp "$ROOT/target/thumbv6m-none-eabi/release/tier1-fixture-rp2040" \
   "$OUT/rp2040.elf"
echo "    rp2040.elf -> $OUT/rp2040.elf"
fi

echo "==> Refreshing MANIFEST.json..."
(cd "$OUT" && python3 - "$NORDIC_ONLY" <<'EOF'
import hashlib, json, pathlib, subprocess, sys

rev = subprocess.run(
    ["git", "rev-parse", "HEAD"],
    capture_output=True, text=True, check=True
).stdout.strip()

manifest_path = pathlib.Path("MANIFEST.json")
if manifest_path.exists():
    manifest = json.loads(manifest_path.read_text())
else:
    manifest = {}

for f in sorted(pathlib.Path(".").iterdir()):
    if sys.argv[1] == "1" and f.name not in ("nrf52832.elf", "nrf52833.elf", "nrf52840.elf"):
        continue
    if f.suffix in (".elf", ".bin") and f.is_file():
        manifest[f.name] = {
            "sha256": hashlib.sha256(f.read_bytes()).hexdigest(),
            "source_rev": rev,
        }

manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
print(f"MANIFEST.json refreshed ({len(manifest)} entries, rev={rev[:12]})")
EOF
)

echo ""
echo "Done. Verify with:"
echo "  ./target/release/labwired run --chip configs/chips/nrf52832.yaml \\"
echo "    --firmware tests/fixtures/tier1/nrf52832.elf --max-steps 8000000 | grep TIER1"
echo "  ./target/release/labwired run --chip configs/chips/nrf52833.yaml \\"
echo "    --firmware tests/fixtures/tier1/nrf52833.elf --max-steps 8000000 | grep TIER1"
echo "  ./target/release/labwired run --chip configs/chips/nrf52840.yaml \\"
echo "    --firmware tests/fixtures/tier1/nrf52840.elf --max-steps 8000000 | grep TIER1"
echo "  ./target/release/labwired run --chip configs/chips/rp2040.yaml \\"
echo "    --firmware tests/fixtures/tier1/rp2040.elf --max-steps 8000000 | grep TIER1"
