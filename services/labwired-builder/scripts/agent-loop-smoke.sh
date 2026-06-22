#!/usr/bin/env bash
# Replays the agent authoring loop against the live MCP gateway to prove the
# Part-A dead-ends are closed. Requires curl + python3. Override MCP_URL to test
# a non-prod gateway.
set -euo pipefail
MCP_URL="${MCP_URL:-https://mcp.proto.cat/mcp}"
call() { # $1=tool $2=json-args
  curl -s --max-time 90 -X POST "$MCP_URL" \
    -H 'content-type: application/json' \
    -H 'accept: application/json, text/event-stream' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"$1\",\"arguments\":$2}}"
}

echo "1) labwired_lookup of:manifest_schema must return a schema"
call labwired_lookup '{"of":"manifest_schema"}' | grep -q board_io && echo "  OK: schema returned" || { echo "  FAIL"; exit 1; }

echo "2) labwired_lookup of:chips must list chip ids"
call labwired_lookup '{"of":"chips"}' | grep -q '"chips"' && echo "  OK: chips returned" || { echo "  FAIL"; exit 1; }

echo "3) A bad manifest must now name the offending field (not the generic string)"
BAD='{"firmware_source":"#include <Arduino.h>\nvoid setup(){}\nvoid loop(){}\n","chip_id":"esp32","system_manifest":"chip: esp32\nperipherals:\n  - type: led\n    pin: 2\n"}'
OUT="$(call labwired_run_build "$BAD")"
echo "$OUT" | grep -qiE 'labwired_lookup|unknown field|chip id' \
  && echo "  OK: actionable error" \
  || { echo "  FAIL — still generic:"; echo "$OUT" | head -c 500; exit 1; }

echo "ALL SMOKE CHECKS PASSED"
