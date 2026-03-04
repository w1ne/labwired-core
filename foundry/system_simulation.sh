#!/bin/bash
# LabWired Foundry VaaS: System Synthesis Simulation Script
# Demonstrates an autonomous agent submitting a multi-component system for verification.

BASE_URL="http://localhost:8080/v1"
TOKEN="local-dev-token"

echo "🤖 [AGENT] Initializing System-Level Synthesis..."

echo "⚙️ [AGENT] Drafting system.yaml linking MCU, I2C Bus, and 2x BME280 Sensors..."
echo "🚀 [AGENT] Submitting System Netlist for Verification..."

verify_resp=$(curl -s -X POST "$BASE_URL/systems/verify" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "system_yaml": "core: STM32F4\nbuses:\n  - i2c1\nperipherals:\n  - name: BME280_1\n    bus: i2c1\n    addr: 0x76\n  - name: BME280_2\n    bus: i2c1\n    addr: 0x76"
  }')

pass=$(echo $verify_resp | jq -r .pass)
compiler_logs=$(echo $verify_resp | jq -r .compiler_logs)
vcd_url=$(echo $verify_resp | jq -r .vcd_url)

echo "📊 [AGENT] System Verification Results:"
echo "   > Pass Status: $pass"
echo "   > Compiler Output: $compiler_logs"
echo "   > VCD Trace: $vcd_url"

if [[ "$pass" == "false" ]]; then
  echo "❌ [AGENT] Integration failed. The agent will now parse the compiler output to resolve the address collision and resubmit."
else
  echo "✅ [AGENT] System successfully integrated and proven!"
fi
