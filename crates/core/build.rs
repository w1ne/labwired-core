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

    build_iolink_native_bridge();
    build_mlx90640_bridge();
}

// Compile the REAL Melexis MLX90640 driver + the LabWired I²C bridge only when
// the `mlx90640-decode-test` feature is enabled. The bridge's
// `MLX90640_I2CRead/Write` call back into Rust (the device model), so the
// unmodified vendor driver decodes frames the model serves over I²C.
fn build_mlx90640_bridge() {
    if std::env::var_os("CARGO_FEATURE_MLX90640_DECODE_TEST").is_none() {
        return;
    }

    let mlx_root = "../../third_party/mlx90640-library";
    cc::Build::new()
        .file("native/mlx90640_bridge.c")
        .file(format!("{mlx_root}/functions/MLX90640_API.c"))
        .include("native")
        .include(format!("{mlx_root}/headers"))
        .warnings(false) // vendor code is unmodified; don't fail on its warnings
        .compile("labwired_mlx90640_bridge");

    println!("cargo:rerun-if-changed=native/mlx90640_bridge.c");
    println!("cargo:rerun-if-changed=native/mlx90640_bridge.h");
    println!("cargo:rerun-if-changed={mlx_root}/functions/MLX90640_API.c");
    println!("cargo:rerun-if-changed={mlx_root}/headers/MLX90640_API.h");
}

// Compile the native IO-Link bridge (and, in later plan tasks, the real
// `iolinki-master` C stack) only when the `iolink-native` feature is enabled.
// Browser/wasm builds never set this feature, so they need no C toolchain and
// never link the GPL device-stack helpers.
fn build_iolink_native_bridge() {
    if std::env::var_os("CARGO_FEATURE_IOLINK_NATIVE").is_none() {
        return;
    }

    // Build scripts run with cwd = the package manifest dir (`crates/core`),
    // while the vendored stacks live at the workspace root (`core/third_party`).
    let master_root = "../../third_party/iolinki-master";
    let device_root = "../../third_party/iolinki";
    let mut build = cc::Build::new();
    build
        .file("native/iolink_master_bridge.c")
        .file("native/iolink_device_bridge.c")
        .file(format!("{master_root}/src/master_port.c"))
        .file(format!("{master_root}/src/master_controller.c"))
        .file(format!("{master_root}/src/master_isdu.c"))
        .file(format!("{master_root}/src/master_parameters.c"))
        .file(format!("{master_root}/src/master_sio.c"))
        // Device stack (singleton). frame.c/crc.c are shared with the master
        // helpers above; listing them once is enough.
        .file(format!("{device_root}/src/frame.c"))
        .file(format!("{device_root}/src/crc.c"))
        .file(format!("{device_root}/src/device.c"))
        .file(format!("{device_root}/src/dll.c"))
        .file(format!("{device_root}/src/isdu.c"))
        .file(format!("{device_root}/src/events.c"))
        .file(format!("{device_root}/src/params.c"))
        .file(format!("{device_root}/src/data_storage.c"))
        .file(format!("{device_root}/src/device_info.c"))
        .file(format!("{device_root}/src/phy_generic.c"))
        .file(format!("{device_root}/src/phy_virtual.c"))
        .file(format!("{device_root}/src/platform.c"))
        .file(format!("{device_root}/src/platform_stubs.c"))
        .file(format!("{device_root}/src/platform/linux/time_utils.c"))
        .file(format!("{device_root}/src/platform/linux/nvm_mock.c"))
        .include("native")
        .include(format!("{master_root}/include"))
        .include(format!("{device_root}/include"))
        .warnings(true)
        .compile("labwired_iolink_master_bridge");

    println!("cargo:rerun-if-changed=native/iolink_master_bridge.h");
    println!("cargo:rerun-if-changed=native/iolink_master_bridge.c");
    println!("cargo:rerun-if-changed=native/iolink_device_bridge.h");
    println!("cargo:rerun-if-changed=native/iolink_device_bridge.c");
    println!("cargo:rerun-if-changed={master_root}/src/master_port.c");
    println!("cargo:rerun-if-changed={master_root}/include/iolinki_master/master.h");
}
