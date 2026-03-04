#!/bin/bash
# LabWired Foundry VaaS: Agent Simulation Script
# This script demonstrates an autonomous agent navigating the Verification-as-a-Service API.

BASE_URL="http://localhost:8080/v1"
TOKEN="local-dev-token"

echo "🤖 [AGENT] Initializing Foundry VaaS connection..."

# 1. Task Ingestion
echo "📥 [AGENT] Polling for new verification tasks..."
task_resp=$(curl -s -H "Authorization: Bearer $TOKEN" "$BASE_URL/tasks/next")
task_id=$(echo $task_resp | jq -r .id)
task_name=$(echo $task_resp | jq -r .name)

if [[ "$task_id" == "null" || -z "$task_id" ]]; then
  echo "   > No tasks available."
  exit 0
fi

echo "   > Acquired Task: $task_name ($task_id)"

# 2. Context Retrieval
echo "📚 [AGENT] Requesting context and constraints for task $task_id..."
ctx_resp=$(curl -s -H "Authorization: Bearer $TOKEN" "$BASE_URL/tasks/$task_id/context")

echo "   > Retrieved Datasheet Excerpts:"
echo $ctx_resp | jq -r '.datasheet_excerpts[]' | while read -r line; do
  echo "     - $line"
done

# 3. Iterative Verification Loop
echo "⚙️ [AGENT] Drafting initial peripheral model and submitting for verification..."
verify_resp=$(curl -s -X POST "$BASE_URL/tasks/$task_id/verify" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "chip_yaml": "name: BME280\nregisters:\n  - name: id\n    addr: 0xD0\n    reset: 0x00"
  }')

pass=$(echo $verify_resp | jq -r .pass)

if [[ "$pass" == "true" ]]; then
  echo "✅ [AGENT] Verification Passed! Solid Proof achieved."
else
  echo "❌ [AGENT] Verification Failed. Analyzing compiler feedback..."
  compiler_logs=$(echo $verify_resp | jq -r .compiler_logs)
  vcd_url=$(echo $verify_resp | jq -r .vcd_url)
  echo "   > Compiler says: $compiler_logs"
  echo "   > VCD Trace available at: $vcd_url"
  echo "🔄 [AGENT] (In a real loop, the agent would now patch the YAML and resubmit)"
fi
