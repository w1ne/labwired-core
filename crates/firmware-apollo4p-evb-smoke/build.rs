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
    println!("cargo:rustc-link-arg=-Tlink.x");
    // FLASH starts at 0x18000, which is not 64-KiB page aligned, so the
    // default page size would map the ELF headers into a PT_LOAD segment at
    // 0x10000 — inside the secure bootloader's MRAM. --nmagic turns off page
    // alignment so nothing loads below the application origin.
    println!("cargo:rustc-link-arg=--nmagic");
    println!("cargo:rerun-if-changed=memory.x");
}
