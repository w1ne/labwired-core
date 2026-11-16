use std::{env, fs, path::Path};
fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let target = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}
fn main() {
    let root = env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = Path::new(&root);
    let source = root.join("../../configs");
    println!("cargo:rerun-if-changed={}", source.display());
    let dest = root.join("python/labwired/configs");
    // An sdist already carries this tree inside the Python package. Its
    // relocated Rust workspace may no longer contain the repository catalog.
    if !source.is_dir() {
        assert!(
            dest.join("chips").is_dir()
                || root.join("../../python/labwired/configs/chips").is_dir(),
            "config catalog is absent: build a wheel from the repository before creating an sdist"
        );
        return;
    }
    // A clean copy prevents removed descriptors leaking into later wheels.
    if dest.exists() {
        fs::remove_dir_all(&dest).unwrap();
    }
    copy_tree(&source, &dest);
    // Maturin relocates path dependencies under archive-root/crates. The
    // config crate's build script needs archive-root/configs to generate its
    // built-in registry. Stage this second tree for sdist only.
    let sdist_configs = root.join("configs");
    if sdist_configs.exists() {
        fs::remove_dir_all(&sdist_configs).unwrap();
    }
    copy_tree(&source, &sdist_configs);

    // Core includes these JSON files at compile time. Stage them under the
    // same archive-root paths before packing an sdist; rebuilding an archive
    // must not depend on this build script racing core's compilation.
    let sdist_examples = root.join("examples/luckfox-pico-mini-a-linux");
    if sdist_examples.exists() {
        fs::remove_dir_all(&sdist_examples).unwrap();
    }
    for relative in [
        "examples/luckfox-pico-mini-a-linux/basket/trusted-prepared.json",
        "examples/luckfox-pico-mini-a-linux/basket/trusted-newc.json",
        "examples/luckfox-pico-mini-a-linux/basket-native-runtime/system.json",
        "examples/luckfox-pico-mini-a-linux/basket-native-runtime/receipt.json",
    ] {
        let source = root.join("../..").join(relative);
        println!("cargo:rerun-if-changed={}", source.display());
        let dest = root.join(relative);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::copy(&source, &dest).unwrap();
    }
}
