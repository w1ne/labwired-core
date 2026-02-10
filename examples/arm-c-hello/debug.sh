#!/bin/bash
# Quick start script for debugging with Cortex-Debug

set -e

echo "🔨 Building firmware..."
make -C "$(dirname "$0")"

echo "🚀 Starting LabWired DAP Server..."
echo "   (GDB Server will be available on localhost:3333)"
echo ""
echo "Now you can:"
echo "  1. Open this folder in VS Code"
echo "  2. Press F5 and select 'LabWired (Cortex-Debug)'"
echo "  3. Start debugging!"
echo ""

cd "$(dirname "$0")/../.."
cargo run -p labwired-dap -- \
    --program "$(dirname "$0")/target/firmware" \
    --system "$(dirname "$0")/system.yaml"
