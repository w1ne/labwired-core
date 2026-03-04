#!/bin/bash
# LabWired Foundry API - Security & Quota Regression Test
# Runs an end-to-end check of Auth Middlewares and Quota Exhaustion

echo "🚀 [TEST] Starting API Regression Suite..."

# 1. Setup DB and Server
cd $(dirname $0)
rm -f foundry_test.db
PORT=8081 DB_PATH=foundry_test.db ~/opt/go1.22.0-bin/bin/go run cmd/server/main.go > /tmp/server_test.log 2>&1 &
SERVER_PID=$!
sleep 2 # wait for server

# 2. Generate a fresh key
echo "🔑 [TEST] Generating fresh Free Tier API Key..."
KEY_OUTPUT=$(~/opt/go1.22.0-bin/bin/go run ./cmd/addkey -workspace test-workspace -db foundry_test.db)
API_KEY=$(echo "$KEY_OUTPUT" | grep 'Your API Key' | awk -F': ' '{print $2}' | tr -d '[:space:]')
echo "🔑 [TEST] Key: $API_KEY"

# 3. Test Unauthorized Access
echo "🛡️ [TEST] Asserting 401 Unauthorized for missing key..."
STATUS_NO_AUTH=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:8081/v1/usage")
if [ "$STATUS_NO_AUTH" != "401" ]; then
    echo "❌ FAILED: Expected 401, got $STATUS_NO_AUTH"
    kill -9 $SERVER_PID
    exit 1
fi
echo "✅ Passed: Missing key blocked."

# 4. Test Authorized Access
echo "🛡️ [TEST] Asserting 200 OK for valid key..."
STATUS_AUTH=$(curl -s -o /dev/null -w "%{http_code}" -H "Authorization: Bearer $API_KEY" "http://localhost:8081/v1/usage")
if [ "$STATUS_AUTH" != "200" ]; then
    echo "❌ FAILED: Expected 200, got $STATUS_AUTH"
    kill -9 $SERVER_PID
    exit 1
fi
echo "✅ Passed: Valid key accepted."

# 5. Exhaust Quota (Free tier = 50 runs)
echo "💸 [TEST] Executing 50 Verification Runs to consume free quota..."
for i in {1..50}; do
  curl -s -X POST -o /dev/null -H "Authorization: Bearer $API_KEY" -d '{}' "http://localhost:8081/v1/systems/verify"
done

echo "💸 [TEST] Asserting 429 Too Many Requests on the 51st run..."
STATUS_EXHAUSTED=$(curl -s -X POST -o /dev/null -w "%{http_code}" -H "Authorization: Bearer $API_KEY" -d '{}' "http://localhost:8081/v1/systems/verify")

if [ "$STATUS_EXHAUSTED" != "429" ]; then
    echo "❌ FAILED: Expected 429 Quota Exceeded, got $STATUS_EXHAUSTED"
    kill -9 $SERVER_PID
    exit 1
fi
echo "✅ Passed: Free tier quota successfully capped at 50 runs!"

kill -9 $SERVER_PID
echo "🎉 All Regression Tests Passed!"
