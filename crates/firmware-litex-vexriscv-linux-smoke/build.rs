// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let memory_map = if env::var_os("CARGO_FEATURE_NEXYS_VIDEO_SRAM").is_some() {
        include_bytes!("memory-nexys-video.x").as_slice()
    } else {
        include_bytes!("memory.x").as_slice()
    };
    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(memory_map)
        .unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rustc-link-arg=-Tmemory.x");
    println!("cargo:rustc-link-arg=-Tlink.x");
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=memory-nexys-video.x");
}
