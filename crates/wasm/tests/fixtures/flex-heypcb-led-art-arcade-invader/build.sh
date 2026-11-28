#!/bin/sh
# Rebuild the portable Flex behavior module with rustc's wasm32 target.
set -eu
cd "$(dirname "$0")"
rustc --edition 2021 --target wasm32-unknown-unknown --crate-type cdylib \
  -C panic=abort -C opt-level=s -C strip=symbols -C codegen-units=1 \
  -o firmware.wasm firmware.rs
