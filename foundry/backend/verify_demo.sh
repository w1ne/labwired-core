#!/bin/bash
# LabWired Foundry API - Stateless "Compiler" Verification Demo
# Showcases an external agent verifying custom YAML directly against the cloud orchestrator.

echo "🚀 [DEMO] Starting Foundry Verification-as-a-Service Loop..."

# 1. Setup DB and Server
cd $(dirname $0)
rm -f foundry_demo_verify.db
PORT=8083 DB_PATH=foundry_demo_verify.db ~/opt/go1.22.0-bin/bin/go run cmd/server/main.go > /tmp/server_demo_verify.log 2>&1 &
SERVER_PID=$!
sleep 2 # wait for server

# 2. Generate a fresh key
echo "🔑 [DEMO] Generating a fresh Developer API Key..."
KEY_OUTPUT=$(~/opt/go1.22.0-bin/bin/go run ./cmd/addkey -workspace demo-workspace-3 -db foundry_demo_verify.db)
API_KEY=$(echo "$KEY_OUTPUT" | grep 'Your API Key' | awk -F': ' '{print $2}' | tr -d '[:space:]')
echo "🔑 [DEMO] Setup Complete. Key: $API_KEY"
echo "------------------------------------------------------"

# 3. Simulate an Agent Sending Custom YAML for Verification
echo "🤖 [AGENT] I drafted some tricky I2C YAML. Let me send it to the Foundry to verify."
echo "🤖 [AGENT] Submitting payload to POST /v1/models/verify..."

curl -s -X POST -H "Authorization: Bearer $API_KEY" -H "Content-Type: application/json" -d '{
  "chip_yaml": "registers:\n  - name: ID\n    address: 0xD0\n    reset: 0x60"
}' "http://localhost:8083/v1/models/verify" | jq .

echo "------------------------------------------------------"
echo "⚙️  [FOUNDRY] Job completed. Returned VCD trace, Compiler Logs, and Pass/Fail status."

# 4. Check Quota Usage
echo "💰 [BILLING] Let's verify our Quota usage after 1 stateless Verification Job:"
curl -s -H "Authorization: Bearer $API_KEY" "http://localhost:8083/v1/usage" | jq .

kill -9 $SERVER_PID
echo "🎉 [DEMO] Stateless Verification Complete!"
