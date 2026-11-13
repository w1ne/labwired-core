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
    // arm32 hosts Cortex-A7/R5F/ARM7TDMI; the AArch32 Cortex-R52 exists only
    // in tlib's arm64 target, which must be built with a 64-bit word.
    for (dir, arch, word, env_name) in [
        ("arm", "arm", "32", "LABWIRED_ARM_SYSTEM_LIBRARY"),
        ("arm64", "arm64", "64", "LABWIRED_ARM64_SYSTEM_LIBRARY"),
    ] {
        let build = out.join(dir);
        run(Command::new("cmake")
            .arg("-S")
            .arg(src.join("native"))
            .arg("-B")
            .arg(&build)
            .args([
                "-DHOST_ARCH=i386",
                "-DHOST_WORD_SIZE=64",
                &format!("-DTARGET_ARCH={arch}"),
                &format!("-DTARGET_WORD_SIZE={word}"),
                "-DCMAKE_BUILD_TYPE=Release",
                "-DTLIB_COMMIT_SHA=167decf9129758829762e582939128c0694e90d6",
            ]));
        run(Command::new("cmake")
            .arg("--build")
            .arg(&build)
            .args(["--parallel", "1"]));
        println!(
            "cargo:rustc-env={env_name}={}",
            build.join("tlib/libtlib.so").display()
        );
    }
}
