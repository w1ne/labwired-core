// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    // cortex-m-rt's link.x consumes this memory.x for the FLASH/RAM regions.
    // The `-Tlink.x` link arg is supplied globally for thumbv6m targets by
    // the repo-level .cargo/config.toml (mirrors crates/firmware-l073-demo),
    // so it must NOT be emitted here a second time.
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::copy("memory.x", out_dir.join("memory.x")).unwrap();
    println!("cargo:rustc-link-search={}", out_dir.display());
    println!("cargo:rerun-if-changed=memory.x");
}
