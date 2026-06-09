use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    // Place memory.x where the linker can find it. The `-Tlink.x` link-arg is
    // supplied once by the workspace `.cargo/config.toml` for thumbv6m, so we
    // do NOT re-emit it here (doing so includes memory.x twice -> linker error
    // "region 'FLASH' already defined").
    let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(include_bytes!("memory.x"))
        .unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");
}
