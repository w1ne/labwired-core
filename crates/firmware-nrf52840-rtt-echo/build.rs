use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(include_bytes!("memory.x"))
        .unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rustc-link-arg=-Tlink.x");
    println!("cargo:rerun-if-changed=memory.x");

    // The vendor C is portable: the host gates (`cargo check`, clippy) compile
    // it with the host compiler, while the firmware build uses cc's
    // thumbv7em -> arm-none-eabi mapping.
    let target = env::var("TARGET").unwrap_or_default();
    let mut build = cc::Build::new();
    build
        .file("../../third_party/segger-rtt/SEGGER_RTT.c")
        .include("../../third_party/segger-rtt")
        .warnings(false);
    if target.starts_with("thumbv7em") {
        build.flag("-mcpu=cortex-m4");
    }
    build.compile("segger_rtt");
    println!("cargo:rerun-if-changed=../../third_party/segger-rtt/SEGGER_RTT.c");
    println!("cargo:rerun-if-changed=../../third_party/segger-rtt/SEGGER_RTT.h");
    println!("cargo:rerun-if-changed=../../third_party/segger-rtt/SEGGER_RTT_Conf.h");
}
