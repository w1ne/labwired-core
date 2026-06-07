#!/usr/bin/env bash
# Rebuild the ESP32-classic Tier-1 fixture ELF and refresh MANIFEST.json.
#
# Requires:
#   - Xtensa toolchain:  source ~/export-esp.sh
#   - espflash:          ~/.local/bin/espflash  (or espflash in PATH)
#
# Usage:
#   source ~/export-esp.sh
#   ./scripts/tier1/build_esp32.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SRC="$ROOT/examples/tier1-fixture/esp32"
OUT="$ROOT/tests/fixtures/tier1"
mkdir -p "$OUT"

echo "[tier1/esp32] Building fixture from $SRC ..."
(cd "$SRC" && cargo build --release)

ELF="$SRC/target/xtensa-esp32-none-elf/release/tier1-fixture-esp32"
if [ ! -f "$ELF" ]; then
    echo "ERROR: ELF not found at $ELF" >&2
    exit 1
fi

cp "$ELF" "$OUT/esp32.elf"
echo "[tier1/esp32] ELF copied to $OUT/esp32.elf"

# Refresh MANIFEST.json to include all *.elf and *.bin blobs.
(cd "$OUT" && python3 - <<'EOF'
import hashlib, json, pathlib, subprocess
rev = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
manifest_path = pathlib.Path("MANIFEST.json")
manifest = {}
if manifest_path.exists():
    try:
        manifest = json.loads(manifest_path.read_text())
    except Exception:
        manifest = {}
for f in sorted(pathlib.Path(".").iterdir()):
    if f.suffix in (".elf", ".bin"):
        manifest[f.name] = {
            "sha256": hashlib.sha256(f.read_bytes()).hexdigest(),
            "source_rev": rev,
        }
manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
print("MANIFEST.json refreshed")
EOF
)

echo "[tier1/esp32] Done."
