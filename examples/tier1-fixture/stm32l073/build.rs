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
    // The repo-level .cargo/config.toml no longer passes -Tlink.x globally for
    // thumbv6m (it duplicated MEMORY regions), so the fixture supplies it here.
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::copy("memory.x", out_dir.join("memory.x")).unwrap();
    println!("cargo:rustc-link-search={}", out_dir.display());
    println!("cargo:rustc-link-arg=-Tlink.x");
    println!("cargo:rerun-if-changed=memory.x");
}
