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
}
