#!/usr/bin/env bash
# Rebuild the committed Tier-1 fixture blobs from source and refresh MANIFEST.json.
# Needs the espressif Rust toolchain (`source ~/export-esp.sh`) + espflash.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/tests/fixtures/tier1"
mkdir -p "$OUT"

build_s3() {
  local src="$ROOT/examples/tier1-fixture/esp32s3"
  (cd "$src" && cargo build --release)
  local elf="$src/target/xtensa-esp32s3-none-elf/release/tier1-fixture-esp32s3"
  cp "$elf" "$OUT/esp32s3.elf"
  espflash save-image --chip esp32s3 --merge "$elf" "$OUT/esp32s3-flash.bin"
}

build_s3

(cd "$OUT" && python3 - <<'EOF'
import hashlib, json, pathlib, subprocess
rev = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
manifest = {}
for f in sorted(pathlib.Path(".").iterdir()):
    if f.suffix in (".elf", ".bin"):
        manifest[f.name] = {
            "sha256": hashlib.sha256(f.read_bytes()).hexdigest(),
            "source_rev": rev,
        }
pathlib.Path("MANIFEST.json").write_text(json.dumps(manifest, indent=2) + "\n")
print("MANIFEST.json refreshed")
EOF
)
