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

    println!("cargo:rustc-link-search={}", out.display());
    // cortex-m-rt's link.x consumes the memory.x written above. Passed here,
    // per-crate, so that
    //
    //     cargo build --release -p firmware-rp2040-pio-onboarding \
    //         --target thumbv6m-none-eabi
    //
    // produces a linked ELF on its own. It used to come from a RUSTFLAGS
    // environment variable that crates/core/tests/strict_onboarding.rs set
    // around its build, which meant the crate linked correctly ONLY when that
    // gate built it: a developer running the command above got an ELF with
    // entry point 0x0 that the simulator rejects at step 0.
    //
    // ⚠️ Exactly one place may pass this. When the gate's RUSTFLAGS and a
    // crate's build.rs both did, link.x was included twice and its memory.x
    // with it, and the build failed with "region 'FLASH' already defined".
    println!("cargo:rustc-link-arg=-Tlink.x");
    println!("cargo:rerun-if-changed=memory.x");
}
