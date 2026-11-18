#!/bin/sh
# Reproduce the portable firmware module; needs rustc's wasm32 target.
set -eu
cd "$(dirname "$0")"
rustc --edition 2021 --target wasm32-unknown-unknown --crate-type cdylib \
  -C panic=abort -C opt-level=s -C strip=symbols -C codegen-units=1 \
  -o firmware.wasm firmware.rs
