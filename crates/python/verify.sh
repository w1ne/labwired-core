#!/bin/bash
set -e

# LabWired Python Bindings Verification Script
# Usage: ./verify.sh [firmware_path]

FIRMWARE=${1:-"../../examples/firmware-stm32f103-blinky/target/thumbv7em-none-eabihf/debug/firmware-stm32f103-blinky"}

echo "=== LabWired Python Bindings Verification ==="
echo "Firmware under test: $FIRMWARE"

if [ ! -f "$FIRMWARE" ]; then
    echo "Warning: Firmware not found at $FIRMWARE"
    echo "Please build the firmware-stm32f103-blinky example first:"
    echo "  cargo build -p firmware-stm32f103-blinky --target thumbv7em-none-eabihf"
    # We don't exit here to allow running if the user provides another path later or expects failure
fi

# 1. Setup Virtual Environment
if [ ! -d "venv" ]; then
    echo "[Setup] Creating virtual environment..."
    python3 -m venv venv
fi

source venv/bin/activate

# 2. Install Build Dependencies
echo "[Setup] Installing maturin and pytest..."
pip install maturin pytest

# 3. Build & Install in Release Mode
echo "[Build] Building labwired-python (Release Mode)..."
maturin develop --release

# 4. Run Tests
echo "[Test] Running pytest..."
export LABWIRED_FIRMWARE="$FIRMWARE"
pytest -v -s tests/test_bindings.py

echo "=== Verification Complete ==="
