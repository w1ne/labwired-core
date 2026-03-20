#!/bin/bash
# LabWired Foundry API - Synthesis-as-a-Service Demo
# Showcases an external agent requesting a digital twin synthesis.

if [[ -x "${HOME}/opt/go1.24.0-bin/bin/go" ]]; then
  GO_BIN="${HOME}/opt/go1.24.0-bin/bin/go"
else
  GO_BIN="$(command -v go)" || { echo "go not found in PATH"; exit 1; }
fi

echo "🚀 [DEMO] Starting Foundry Synthesis-as-a-Service Loop..."

# 1. Setup DB and Server
cd $(dirname $0)
rm -f foundry_demo.db
PORT=8082 DB_PATH=foundry_demo.db "$GO_BIN" run cmd/server/main.go > /tmp/server_demo.log 2>&1 &
SERVER_PID=$!
sleep 2 # wait for server

# 2. Generate a fresh key
echo "🔑 [DEMO] Generating a fresh Developer API Key..."
KEY_OUTPUT=$("$GO_BIN" run ./cmd/addkey -workspace demo-workspace-2 -db foundry_demo.db)
API_KEY=$(echo "$KEY_OUTPUT" | grep 'Your API Key' | awk -F': ' '{print $2}' | tr -d '[:space:]')
echo "🔑 [DEMO] Setup Complete. Key: $API_KEY"
echo "------------------------------------------------------"

# 3. Simulate an Agent Requesting a Quote
echo "🤖 [AGENT] I need a digital twin for an ADXL345 accelerometer."
echo "🤖 [AGENT] Checking how much it will cost via POST /v1/estimate..."

curl -s -X POST -H "Authorization: Bearer $API_KEY" -H "Content-Type: application/json" -d '{
  "component_name": "ADXL345",
  "requirements": "I2C interface required. Register 0x00 should return Device ID 0xE5.",
  "datasheet_url": "https://www.analog.com/media/en/technical-documentation/data-sheets/ADXL345.pdf"
}' "http://localhost:8082/v1/estimate" | jq .

echo "------------------------------------------------------"
echo "🤖 [AGENT] Cost accepted. Proceeding with Synthesis..."
echo "🤖 [AGENT] Submitting payload to POST /v1/synthesize..."

curl -s -X POST -H "Authorization: Bearer $API_KEY" -H "Content-Type: application/json" -d '{
  "component_name": "ADXL345",
  "requirements": "I2C interface required. Register 0x00 should return Device ID 0xE5.",
  "datasheet_url": "https://www.analog.com/media/en/technical-documentation/data-sheets/ADXL345.pdf"
}' "http://localhost:8082/v1/synthesize" | jq .

echo "------------------------------------------------------"
echo "⚙️  [FOUNDRY] Job Enqueued. Internal Agents are writing YAML and proving assertions..."
echo "⚙️  [FOUNDRY] Consumed Dynamic Simulation Runs from quota."

# 4. Check Quota Usage to prove billing
echo "💰 [BILLING] Let's verify our Quota usage after the Synthesis Job:"
curl -s -H "Authorization: Bearer $API_KEY" "http://localhost:8082/v1/usage" | jq .

kill -9 $SERVER_PID
echo "🎉 [DEMO] Synthesis-as-a-Service Complete!"
