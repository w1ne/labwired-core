// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    // cortex-m-rt's link.x consumes this memory.x for the FLASH/RAM regions
    // and the initial-SP / reset-vector layout.
    //
    // This comment used to say the `-Tlink.x` arg was "supplied globally for
    // thumbv6m by core/.cargo/config.toml". It is not, and that file says so in
    // as many words — it explicitly declines to add a global thumbv6m
    // `-Tlink.x` because passing the script twice duplicates memory.x's MEMORY
    // regions. Nothing supplied it, so the documented build command
    //
    //     cargo build --release -p firmware-l073-demo --target thumbv6m-none-eabi
    //
    // linked WITHOUT the script and produced an ELF with entry point 0x0, which
    // the simulator rejects at step 0 with a memory violation at 0xfffffffe.
    // The committed fixture could not be reproduced from its own instructions.
    //
    // Pass it here, per-crate, which is what every other firmware crate in this
    // workspace already does (firmware-f401-demo, firmware-h563-io-demo, …).
    let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(include_bytes!("memory.x"))
        .unwrap();

    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rustc-link-arg=-Tlink.x");
    println!("cargo:rerun-if-changed=memory.x");
}
