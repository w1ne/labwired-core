#!/usr/bin/env bash
set -euo pipefail

# Optional client-side smoke for the six RSP conformance scenarios: launch the
# real `labwired-cli --gdb` server around the CI UART fixture and drive it with
# one batch `gdb-multiarch` (or `arm-none-eabi-gdb`) session — `target remote`,
# `info registers pc`, `stepi`, a memory read, `detach`. The smoke passes only
# if GDB reports a PC, so a dead server or a hung resume fails the run.
#
# Skips with exit 0 when neither cross GDB is installed, so CI without one
# stays green. Rust-side counterpart (also ignored by default):
#   cargo test -p labwired-gdbstub --test rsp_conformance -- --ignored

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CORE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

if command -v gdb-multiarch >/dev/null 2>&1; then
  GDB="gdb-multiarch"
elif command -v arm-none-eabi-gdb >/dev/null 2>&1; then
  GDB="arm-none-eabi-gdb"
else
  echo "gdb-multiarch not installed — skipping"
  exit 0
fi

PORT="$(python3 - <<'PY'
import socket

with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
    probe.bind(("127.0.0.1", 0))
    print(probe.getsockname()[1])
PY
)"

SERVER_LOG="$(mktemp -t labwired-gdb-conformance.XXXXXX)"
GDB_LOG="$(mktemp -t labwired-gdb-conformance-gdb.XXXXXX)"
SERVER_PID=""
cleanup() {
  if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
    pkill -TERM -P "$SERVER_PID" 2>/dev/null || true
    kill -TERM "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  rm -f "$SERVER_LOG" "$GDB_LOG"
}
trap cleanup EXIT

cd "$CORE_DIR"

# `cargo test` holds the target-dir build lock for the whole run, so a prebuilt
# CLI (kept fresh by `cargo build -p labwired-cli`; the binary is `labwired`)
# is used when present; the `cargo run` fallback is for standalone invocations.
CLI_BIN="${LABWIRED_CLI_BIN:-$CORE_DIR/target/debug/labwired}"
if [[ -x "$CLI_BIN" ]]; then
  echo "server:   $CLI_BIN (prebuilt)"
  "$CLI_BIN" --gdb "$PORT" \
    --firmware tests/fixtures/uart-ok-thumbv7m.elf \
    --system configs/systems/ci-fixture-uart1.yaml >"$SERVER_LOG" 2>&1 &
else
  echo "server:   cargo run -q -p labwired-cli (no prebuilt binary)"
  cargo run -q -p labwired-cli -- --gdb "$PORT" \
    --firmware tests/fixtures/uart-ok-thumbv7m.elf \
    --system configs/systems/ci-fixture-uart1.yaml >"$SERVER_LOG" 2>&1 &
fi
SERVER_PID=$!

echo "gdb:      $GDB"
echo "port:     $PORT"

# Bounded wait for the server to listen. The probe binds instead of connecting:
# `GdbServer::run` accepts exactly one connection, so a connect probe would
# consume the session the real GDB needs.
if ! python3 - "$PORT" "$SERVER_PID" <<'PY'
import os
import socket
import sys
import time

port = int(sys.argv[1])
pid = int(sys.argv[2])
deadline = time.time() + 120
while time.time() < deadline:
    try:
        os.kill(pid, 0)
    except OSError:
        print("server exited before it listened", file=sys.stderr)
        sys.exit(1)
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        try:
            probe.bind(("127.0.0.1", port))
        except OSError:
            sys.exit(0)
    time.sleep(0.2)
print(f"server did not listen on port {port} within 120 s", file=sys.stderr)
sys.exit(1)
PY
then
  echo "server log ($SERVER_LOG):" >&2
  tail -n 40 "$SERVER_LOG" >&2 || true
  exit 1
fi

set +e
"$GDB" -batch \
  -ex "set confirm off" \
  -ex "target remote :$PORT" \
  -ex "info registers pc" \
  -ex "stepi" \
  -ex "x/4xb 0x400" \
  -ex "detach" \
  -ex "quit" 2>&1 | tee "$GDB_LOG"
GDB_STATUS="${PIPESTATUS[0]}"
set -e

if [[ "$GDB_STATUS" -ne 0 ]]; then
  echo "gdb exited with status $GDB_STATUS" >&2
  exit "$GDB_STATUS"
fi

if ! grep -Eq '^[[:space:]]*pc[[:space:]]+0x[0-9a-fA-F]+' "$GDB_LOG"; then
  echo "no PC line in gdb output (above)" >&2
  exit 1
fi

echo "PASS: $GDB batch smoke against labwired-cli --gdb"
exit 0
