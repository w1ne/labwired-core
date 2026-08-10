use std::{env, fs, path::PathBuf};
fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::copy("memory.x", out.join("memory.x")).unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rustc-link-arg-bin=firmware-nrf52840-obd2-scanner=-Tlink.x");
    println!("cargo:rerun-if-changed=memory.x");
}
