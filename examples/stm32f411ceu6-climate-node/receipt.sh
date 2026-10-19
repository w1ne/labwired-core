#!/usr/bin/env bash
# Reproduce the climate-node run/verify receipts (receipt/run.json,
# receipt/verify.json) from THIS tree:
#
#   1. build the firmware (PlatformIO, offline once cached),
#   2. run the io-smoke through the local engine with the PA5 logic capture,
#   3. run it a second time and check the transcript is byte-identical,
#   4. run the same smoke cut at 12M cycles (before the 15M stimuli) as the
#      oracle's negative control,
#   5. evaluate receipt/oracle.json with the shared board-config evaluator
#      (receipt/local-oracle.mjs) and write the two receipts.
#
# The MCP stdio attempts are a separate, heavier harness (they need the
# monorepo toolchain); see receipt/README.md and `node receipt/mcp-attempt.mjs`.
set -euo pipefail
export PATH="$HOME/.nvm/versions/node/v22.23.2/bin:$PATH:$HOME/.local/bin"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
ENGINE="${LABWIRED_CLI:-$ROOT/core/target/release/labwired}"
SMOKE="$HERE/io-smoke.yaml"
ELF="$HERE/firmware/.pio/build/blackpill_f411ce/firmware.elf"
RUN2_DIR="${RUN2_DIR:-/tmp/opencode/stage3-receipt-run2}"
NEG_DIR="${NEG_DIR:-/tmp/opencode/stage3-receipt-neg}"

test -x "$ENGINE" || { echo "engine not found: $ENGINE (build core first: cargo build --release -p labwired-cli)" >&2; exit 1; }

echo "== 1/5 build firmware =="
( cd "$HERE/firmware" && pio run -e blackpill_f411ce )

echo "== 2/5 main run -> receipt/run =="
rm -rf "$HERE/receipt/run" && mkdir -p "$HERE/receipt/run"
"$ENGINE" test --script "$SMOKE" --firmware "$ELF" --system "$HERE/system.yaml" \
  --output-dir "$HERE/receipt/run" --run-manifest --watch-gpio gpioa:5

echo "== 3/5 second run -> $RUN2_DIR (determinism) =="
rm -rf "$RUN2_DIR" && mkdir -p "$RUN2_DIR"
"$ENGINE" test --script "$SMOKE" --firmware "$ELF" --system "$HERE/system.yaml" \
  --output-dir "$RUN2_DIR" --run-manifest --watch-gpio gpioa:5
if ! cmp -s "$HERE/receipt/run/uart.log" "$RUN2_DIR/uart.log"; then
  echo "DETERMINISM DEFECT: uart.log differs between runs" >&2
  diff "$HERE/receipt/run/uart.log" "$RUN2_DIR/uart.log" >&2 || true
  exit 1
fi
echo "uart.log byte-identical across runs"

echo "== 4/5 negative control (cut at 12.5M, before the 15M stimuli) =="
rm -rf "$NEG_DIR" && mkdir -p "$NEG_DIR"
"$ENGINE" test --script "$SMOKE" --firmware "$ELF" --system "$HERE/system.yaml" \
  --max-steps 12500000 --watch-gpio gpioa:5 --output-dir "$NEG_DIR" || true

echo "== 5/5 compose receipts (shared board-config evaluator) =="
RUN_DIR="$HERE/receipt/run" RUN2_DIR="$RUN2_DIR" NEG_DIR="$NEG_DIR" ENGINE_BIN="$ENGINE" \
  node "$HERE/receipt/local-oracle.mjs" compose

echo "== hashes =="
sha256sum "$ELF" "$HERE/system.yaml" "$HERE/io-smoke.yaml" "$HERE/receipt/run/uart.log"
