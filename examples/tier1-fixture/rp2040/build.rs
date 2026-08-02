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
    // Link against cortex-m-rt's script, exactly as the nRF fixtures' build.rs
    // does. Without this the crate links with the default layout: the ELF comes
    // out with entry 0x0 instead of 0x100000c1 and does not boot, yet the build
    // SUCCEEDS — so `scripts/tier1/build_nordic_rp2040.sh` silently produced a
    // dead fixture and every rp2040 tier-1 cell read `blocked`.
    //
    // It belongs here rather than in the workspace `.cargo/config.toml`: a
    // global thumbv6m `-Tlink.x` would apply the script twice for crates that
    // already request it and duplicate the MEMORY regions from memory.x.
    println!("cargo:rustc-link-arg=-Tlink.x");
    println!("cargo:rerun-if-changed=memory.x");
}
