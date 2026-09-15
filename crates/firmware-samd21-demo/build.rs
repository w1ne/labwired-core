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
    // Link against cortex-m-rt's script. WITHOUT this the crate still builds,
    // and links with the default layout: entry 0x0, sections at 0x10000, no
    // vector table. `labwired run` then faults at 0xfffffffe and the failure
    // looks like a broken chip model rather than a broken link. It belongs
    // here and NOT in the workspace `.cargo/config.toml`, where a global
    // thumbv6m `-Tlink.x` would apply the script twice and duplicate the
    // MEMORY regions from memory.x.
    println!("cargo:rustc-link-arg=-Tlink.x");
    println!("cargo:rerun-if-changed=memory.x");
}
