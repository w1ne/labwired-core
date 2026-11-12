#!/usr/bin/env bash
# CI-style evidence: correct D/C paints three squares; wrong D/C remains blank.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CORE="$(cd "$ROOT/../.." && pwd)"
BIN="${LABWIRED_BIN:-labwired}"
BUILD="$ROOT/.pio/build/crowpanel_advance_28"
cd "$CORE"
pio run -d "$ROOT" >/dev/null
bash "$ROOT/build_flash.sh" >/dev/null
IMAGE="$BUILD/flash.bin"
export LABWIRED_ESP32S3_FLASH="$IMAGE"
run() {
  local manifest="$1" image="$2" logfile="$3"
  RUST_LOG=warn "$BIN" run \
    --chip configs/chips/esp32s3.yaml \
    --system "$manifest" --rom-boot --firmware "$IMAGE" \
    --max-steps 180000000 --stop-on CROWPANEL_DISPLAY_TRANSFER_OK \
    --display-out "$image" > "$logfile" 2>&1
  grep -q CROWPANEL_DISPLAY_TRANSFER_OK "$logfile"
}
run "$ROOT/system.yaml" "$BUILD/positive.ppm" "$BUILD/positive.log"
python3 "$ROOT/check_display.py" positive "$BUILD/positive.ppm"
NEG="$ROOT/.negative-test-system.yaml"
trap 'rm -f "$NEG"' EXIT
sed 's/dc_pin: "GPIO41"/dc_pin: "GPIO4"/' "$ROOT/system.yaml" > "$NEG"
run "$NEG" "$BUILD/negative.ppm" "$BUILD/negative.log"
python3 "$ROOT/check_display.py" negative "$BUILD/negative.ppm"
echo 'PASS: CrowPanel ESP32-S3 + ST7789 smoke and D/C negative control'
