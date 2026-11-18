#!/usr/bin/env bash
set -euo pipefail

cargo build -p firmware-stm32u575-demo \
  --bin nucleo-u5a5-saes-smoke \
  --release \
  --target thumbv8m.main-none-eabi
