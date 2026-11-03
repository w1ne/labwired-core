use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    File::create(out.join("minimal.ld"))
        .expect("create linker script")
        .write_all(include_bytes!("minimal.ld"))
        .expect("write linker script");
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rustc-link-arg=-Tminimal.ld");
    println!("cargo:rerun-if-changed=minimal.ld");
}
