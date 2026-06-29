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
use std::path::{Path, PathBuf};

const WAT_SRC: &str = "src/cpu/xtensa_jit/hot_bb.wat";

fn main() {
    println!("cargo:rerun-if-changed={WAT_SRC}");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=IOLINKI_MASTER_DIR");
    println!("cargo:rerun-if-env-changed=IOLINKI_DEVICE_DIR");

    let wat_text = fs::read_to_string(WAT_SRC).unwrap_or_else(|e| panic!("read {WAT_SRC}: {e}"));
    let bytes = wat::parse_str(&wat_text).unwrap_or_else(|e| panic!("parse {WAT_SRC} as WAT: {e}"));

    let out_dir = env::var_os("OUT_DIR").expect("OUT_DIR must be set by cargo");
    let dest = PathBuf::from(out_dir).join("xtensa_jit_hot_bb.wasm");
    fs::write(&dest, &bytes).unwrap_or_else(|e| panic!("write {}: {e}", dest.display()));

    if env::var_os("CARGO_FEATURE_IOLINK_NATIVE").is_some() {
        build_iolink_native_bridge();
    }
}

fn build_iolink_native_bridge() {
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set by cargo"),
    );
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("core crate must live under crates/core");
    let default_device_dir = repo_root.join("third_party/iolinki");
    let device_dir = env::var_os("IOLINKI_DEVICE_DIR")
        .map(PathBuf::from)
        .unwrap_or(default_device_dir);
    let master_dir = env::var_os("IOLINKI_MASTER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root.join("../iolinki-master"));

    if !master_dir.join("include/iolinki_master/master.h").exists() {
        panic!(
            "iolink-native requires IOLINKI_MASTER_DIR to point at the C iolinki-master repo; tried {}",
            master_dir.display()
        );
    }
    if !device_dir.join("include/iolinki/phy.h").exists() {
        panic!(
            "iolink-native requires IOLINKI_DEVICE_DIR to point at the iolinki device stack; tried {}",
            device_dir.display()
        );
    }

    let sources = [
        manifest_dir.join("native/iolink_master_bridge.c"),
        master_dir.join("src/master_controller.c"),
        master_dir.join("src/master_isdu.c"),
        master_dir.join("src/master_parameters.c"),
        master_dir.join("src/master_port.c"),
        master_dir.join("src/master_sio.c"),
        device_dir.join("src/crc.c"),
        device_dir.join("src/frame.c"),
    ];

    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("native/iolink_master_bridge.c").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        master_dir.join("include/iolinki_master/master.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        device_dir.join("include/iolinki/phy.h").display()
    );
    for source in &sources {
        println!("cargo:rerun-if-changed={}", source.display());
    }

    let mut build = cc::Build::new();
    build
        .std("c99")
        .warnings(false)
        .include(master_dir.join("include"))
        .include(device_dir.join("include"));
    for source in sources {
        build.file(source);
    }
    build.compile("labwired_iolinki_master_bridge");
}
