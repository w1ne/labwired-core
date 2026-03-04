#!/bin/bash
# LabWired Foundry - E2E API Validation Script
# This script exercises all 6 core user flows to verify API correctness,
# dynamic pricing logic, and quota enforcement.

set -e

echo "🚀 [E2E] Starting Full API Validation Suite..."

# 1. SETUP
cd $(dirname $0)
fuser -k 8080/tcp || true
rm -f foundry_e2e.db
rm -rf /tmp/foundry/artifacts_e2e
mkdir -p /tmp/foundry/artifacts_e2e

# Start Server
PORT=8080 DB_PATH=foundry_e2e.db ARTIFACTS_DIR=/tmp/foundry/artifacts_e2e ~/opt/go1.24.0-bin/bin/go run cmd/server/main.go > /tmp/server_e2e.log 2>&1 &
SERVER_PID=$!

# Cleanup on exit
trap "kill -9 $SERVER_PID" EXIT

sleep 3 # Wait for server and DB migrations

# 2. GENERATE KEY
echo "🔑 [E2E] Generating first-party Builder Key..."
KEY_OUTPUT=$(~/opt/go1.24.0-bin/bin/go run cmd/addkey/main.go -workspace labwired-team -db foundry_e2e.db)
API_KEY=$(echo "$KEY_OUTPUT" | grep 'Your API Key' | awk -F': ' '{print $2}' | tr -d '[:space:]')
echo "🔑 [E2E] Key generated: $API_KEY"

# ------------------------------------------------------------------------------
# FLOW A: Public Discovery (No Auth)
# ------------------------------------------------------------------------------
echo "🌍 [A] Public Discovery..."
STATUS_INFO=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:8080/v1/info")
if [ "$STATUS_INFO" != "200" ]; then echo "❌ Flow A (/info) failed: $STATUS_INFO"; exit 1; fi

STATUS_CAT=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:8080/v1/catalog")
if [ "$STATUS_CAT" != "200" ]; then echo "❌ Flow A (/catalog) failed: $STATUS_CAT"; exit 1; fi
echo "✅ Flow A Passed."

# ------------------------------------------------------------------------------
# FLOW B: Initial Quota Check
# ------------------------------------------------------------------------------
echo "💰 [B] Usage Check..."
USAGE_RESP=$(curl -s -H "Authorization: Bearer $API_KEY" "http://localhost:8080/v1/usage")
USED=$(echo "$USAGE_RESP" | jq .runs_used_this_month)
QUOTA=$(echo "$USAGE_RESP" | jq .quota)

if [ "$USED" != "0" ]; then echo "❌ Flow B (used) failed: $USED"; exit 1; fi
if [ "$QUOTA" != "1000" ]; then echo "❌ Flow B (quota) failed: $QUOTA"; exit 1; fi
echo "✅ Flow B Passed (0/1000 used)."

# ------------------------------------------------------------------------------
# FLOW C: Price Estimate (Free)
# ------------------------------------------------------------------------------
echo "🏷️ [C] Cost Estimation..."
EST_RESP=$(curl -s -X POST -H "Authorization: Bearer $API_KEY" -H "Content-Type: application/json" -d '{
  "component_name": "BME280",
  "requirements": "I2C interface required."
}' "http://localhost:8080/v1/estimate")
COST=$(echo "$EST_RESP" | jq .estimated_cost_runs)

if [ "$COST" != "15" ]; then echo "❌ Flow C (estimate) failed: $COST"; exit 1; fi

# Assert it didn't consume quota
USED_AFTER=$(curl -s -H "Authorization: Bearer $API_KEY" "http://localhost:8080/v1/usage" | jq .runs_used_this_month)
if [ "$USED_AFTER" != "0" ]; then echo "❌ Flow C consumed quota!"; exit 1; fi
echo "✅ Flow C Passed (Estimate: 15 runs, Balance: 0 used)."

# ------------------------------------------------------------------------------
# FLOW D: Dynamic Synthesis (Priced)
# ------------------------------------------------------------------------------
echo "🛠️ [D] Autonomous Synthesis..."
SYNTH_RESP=$(curl -s -X POST -H "Authorization: Bearer $API_KEY" -H "Content-Type: application/json" -d '{
  "component_name": "BME280",
  "requirements": "I2C interface required."
}' "http://localhost:8080/v1/synthesize")
echo "DEBUG Flow D Resp: $SYNTH_RESP"

STATUS=$(echo "$SYNTH_RESP" | jq -r .status)
if [ "$STATUS" != "processing" ]; then echo "❌ Flow D (status) failed: $STATUS"; exit 1; fi

# Assert it consumed exactly the estimated cost
USED_AFTER_SYNTH=$(curl -s -H "Authorization: Bearer $API_KEY" "http://localhost:8080/v1/usage" | jq .runs_used_this_month)
if [ "$USED_AFTER_SYNTH" != "15" ]; then echo "❌ Flow D quota mismatch: $USED_AFTER_SYNTH"; exit 1; fi
echo "✅ Flow D Passed (Deducted 15 runs)."

# ------------------------------------------------------------------------------
# FLOW E: Stateless Verification (Standard Price: 1)
# ------------------------------------------------------------------------------
echo "🔍 [E] Model Verification..."
VERIFY_RESP=$(curl -s -X POST -H "Authorization: Bearer $API_KEY" -H "Content-Type: application/json" -d '{
  "chip_yaml": "registers: []"
}' "http://localhost:8080/v1/models/verify")
echo "DEBUG Flow E Resp: $VERIFY_RESP"

RUN_ID=$(echo "$VERIFY_RESP" | jq -r .run_id)
if [[ "$RUN_ID" != run-model-* ]]; then echo "❌ Flow E (run_id) failed: $RUN_ID"; exit 1; fi

# Assert it consumed 1 run
USED_AFTER_VERIFY=$(curl -s -H "Authorization: Bearer $API_KEY" "http://localhost:8080/v1/usage" | jq .runs_used_this_month)
if [ "$USED_AFTER_VERIFY" != "16" ]; then echo "❌ Flow E quota mismatch: $USED_AFTER_VERIFY"; exit 1; fi
echo "✅ Flow E Passed (Deducted 1 run)."

# ------------------------------------------------------------------------------
# FLOW F: Quota Exhaustion
# ------------------------------------------------------------------------------
echo "🚫 [F] Quota Exhaustion (Bombarding server sequentially for stability)..."
# We are currently at 16 runs. We need 984 more to hit 1000.
# Sequential curl to localhost is fast enough (< 1 minute).
for i in {1..984}; do
  curl -s -X POST -o /dev/null -H "Authorization: Bearer $API_KEY" -H "Content-Type: application/json" -d '{"chip_yaml": "bombard"}' "http://localhost:8080/v1/models/verify"
  if (( $i % 100 == 0 )); then
    echo "   ...sent $i requests..."
  fi
done
echo "⏳ Waiting for DB to settle..."
sleep 2

# The 1001th run should fail
STATUS_429=$(curl -s -X POST -o /dev/null -w "%{http_code}" -H "Authorization: Bearer $API_KEY" -H "Content-Type: application/json" -d '{"chip_yaml": ""}' "http://localhost:8080/v1/models/verify")

if [ "$STATUS_429" != "429" ]; then
    echo "❌ Flow F (exclusion) failed: Expected 429, got $STATUS_429"
    exit 1
fi

USED_FINAL=$(curl -s -H "Authorization: Bearer $API_KEY" "http://localhost:8080/v1/usage" | jq .runs_used_this_month)
echo "✅ Flow F Passed. Quota exhausted at $USED_FINAL/1000 runs. Next request correctly blocked."

echo "------------------------------------------------------"
echo "🎉 [E2E] ALL FLOWS VALIDATED SUCCESSFULLY!"
echo "------------------------------------------------------"
