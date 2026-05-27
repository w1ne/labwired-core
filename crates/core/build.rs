// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// #124 Phase 4: pre-compile the Xtensa JIT hot-block WAT into wasm bytes
// at crate build time. Both backends (native wasmtime + browser
// `js_sys::WebAssembly`) consume the identical byte stream so emit logic
// stays runtime-agnostic.
//
// The WAT source lives in `src/cpu/xtensa_jit/hot_bb.wat`. This script:
//   1. Reads the WAT text.
//   2. Parses it with the `wat` crate (build-dep only — keeps it out of
//      the browser bundle's runtime deps).
//   3. Writes the resulting binary module to
//      `$OUT_DIR/xtensa_jit_hot_bb.wasm`, which `wasm_bytes.rs` pulls in
//      via `include_bytes!`.

use std::env;
use std::fs;
use std::path::PathBuf;

const WAT_SRC: &str = "src/cpu/xtensa_jit/hot_bb.wat";

fn main() {
    println!("cargo:rerun-if-changed={WAT_SRC}");
    println!("cargo:rerun-if-changed=build.rs");

    let wat_text = fs::read_to_string(WAT_SRC).unwrap_or_else(|e| panic!("read {WAT_SRC}: {e}"));
    let bytes = wat::parse_str(&wat_text).unwrap_or_else(|e| panic!("parse {WAT_SRC} as WAT: {e}"));

    let out_dir = env::var_os("OUT_DIR").expect("OUT_DIR must be set by cargo");
    let dest = PathBuf::from(out_dir).join("xtensa_jit_hot_bb.wasm");
    fs::write(&dest, &bytes).unwrap_or_else(|e| panic!("write {}: {e}", dest.display()));
}
