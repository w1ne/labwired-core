use std::path::PathBuf;

fn main() {
    let linker_script =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("memory.x");
    println!("cargo:rerun-if-changed={}", linker_script.display());
    println!(
        "cargo:rustc-link-arg-bin=firmware-tock-veer-el2-smoke=-T{}",
        linker_script.display()
    );
}
