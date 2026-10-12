use std::{env, path::PathBuf, process::Command};
fn run(c: &mut Command) {
    assert!(
        c.status().expect("native CMake command").success(),
        "native CMake failed"
    );
}
fn main() {
    println!("cargo:rerun-if-changed=native");
    println!("cargo:rerun-if-changed=../../third_party/tlib");
    if env::var_os("CARGO_FEATURE_NATIVE").is_none() {
        return;
    }
    assert_eq!(
        env::var("TARGET").unwrap(),
        "x86_64-unknown-linux-gnu",
        "native ARM backend supports x86_64 Linux GNU only"
    );
    assert_eq!(
        env::var("HOST").unwrap(),
        "x86_64-unknown-linux-gnu",
        "native backend requires matching host"
    );
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let src = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    run(Command::new("cmake")
        .arg("-S")
        .arg(src.join("native"))
        .arg("-B")
        .arg(&out)
        .args([
            "-DHOST_ARCH=i386",
            "-DHOST_WORD_SIZE=64",
            "-DTARGET_ARCH=arm",
            "-DTARGET_WORD_SIZE=32",
            "-DCMAKE_BUILD_TYPE=Release",
            "-DTLIB_COMMIT_SHA=167decf9129758829762e582939128c0694e90d6",
        ]));
    run(Command::new("cmake")
        .arg("--build")
        .arg(&out)
        .args(["--parallel", "1"]));
    println!(
        "cargo:rustc-env=LABWIRED_ARM_SYSTEM_LIBRARY={}",
        out.join("tlib/libtlib.so").display()
    );
}
