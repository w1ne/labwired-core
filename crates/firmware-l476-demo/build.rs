// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(include_bytes!("memory.x"))
        .unwrap();
    File::create(out.join("minimal.ld"))
        .unwrap()
        .write_all(include_bytes!("minimal.ld"))
        .unwrap();
    File::create(out.join("cubemx.ld"))
        .unwrap()
        .write_all(include_bytes!("cubemx.ld"))
        .unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    // Per-binary linker scripts: minimal.ld for the original two demos
    // (their vector tables are embedded in the linker script via LONG()),
    // cubemx.ld for the HAL-style firmware which provides its own
    // .isr_vector section and needs .data/.bss layout.
    println!("cargo:rustc-link-arg-bin=firmware-l476-demo=-Tminimal.ld");
    println!("cargo:rustc-link-arg-bin=firmware-l476-l4periphs2=-Tminimal.ld");
    println!("cargo:rustc-link-arg-bin=firmware-l476-cubemx-hal=-Tcubemx.ld");
    println!("cargo:rustc-link-arg-bin=firmware-l476-tim1-advanced=-Tminimal.ld");
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=minimal.ld");
    println!("cargo:rerun-if-changed=cubemx.ld");
}
