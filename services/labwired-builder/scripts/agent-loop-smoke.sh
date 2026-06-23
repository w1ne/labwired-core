#!/usr/bin/env bash
# Replays the agent authoring loop against the live MCP gateway to prove the
# Part-A dead-ends are closed. Requires curl + python3. Override MCP_URL to test
# a non-prod gateway.
#
# Parsing note: tools/call returns a JSON-RPC envelope whose tool payload is a
# JSON *string* in result.content[0].text (so its quotes are backslash-escaped).
# Grepping the raw envelope for '"chips"' is a false-negative trap — we decode
# the envelope and the inner payload with python3 and assert on the real values.
set -euo pipefail
MCP_URL="${MCP_URL:-https://mcp.proto.cat/mcp}"

# $1=tool $2=json-args ; prints the decoded inner tool payload (the text field).
call() {
  curl -s --max-time 90 -X POST "$MCP_URL" \
    -H 'content-type: application/json' \
    -H 'accept: application/json, text/event-stream' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"$1\",\"arguments\":$2}}" \
  | python3 -c '
import json,sys
env=json.load(sys.stdin)
res=env.get("result") or {}
parts=res.get("content") or []
text="".join(p.get("text","") for p in parts if p.get("type")=="text")
sys.stdout.write(text)
'
}

echo "1) labwired_lookup of:manifest_schema must return a schema with board_io"
call labwired_lookup '{"of":"manifest_schema"}' \
  | python3 -c 'import sys; t=sys.stdin.read(); sys.exit(0 if "board_io" in t else 1)' \
  && echo "  OK: schema returned" || { echo "  FAIL"; exit 1; }

echo "2) labwired_lookup of:chips must list a non-empty chip-id catalog"
call labwired_lookup '{"of":"chips"}' \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); ids=[c["id"] for c in (d.get("chips") or [])]; assert ids, "empty"; print("  OK: chips ->", ", ".join(ids))' \
  || { echo "  FAIL"; exit 1; }

echo "3) A bad manifest must return an actionable error naming the field + next step"
BAD='{"firmware_source":"#include <Arduino.h>\nvoid setup(){}\nvoid loop(){}\n","chip_id":"esp32","system_manifest":"chip: esp32\nperipherals:\n  - type: led\n    pin: 2\n"}'
call labwired_run_build "$BAD" \
  | python3 -c '
import json,sys
t=sys.stdin.read()
try:
    msg=json.loads(t).get("error","")
except Exception:
    msg=t
low=msg.lower()
# Must name the offending value AND point at the recovery tool — not a bare string.
ok=("esp32" in low) and ("known chip ids" in low or "unknown chip" in low) and ("labwired_lookup" in low or "of:" in low)
print("  error:", msg[:160])
sys.exit(0 if ok else 1)
' && echo "  OK: actionable error" || { echo "  FAIL — not actionable"; exit 1; }

echo "ALL SMOKE CHECKS PASSED"
