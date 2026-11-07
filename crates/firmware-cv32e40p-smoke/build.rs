use std::path::PathBuf;

fn main() {
    let linker_script =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("memory.x");
    println!("cargo:rerun-if-changed={}", linker_script.display());
    println!(
        "cargo:rustc-link-arg-bin=firmware-cv32e40p-smoke=-T{}",
        linker_script.display()
    );
}
