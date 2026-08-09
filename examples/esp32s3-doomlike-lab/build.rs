fn main() {
    // No additional linker args needed beyond .cargo/config.toml's
    // `-Tlinkall.x` (provided by esp-hal's xtensa-lx-rt link search path).
    //
    // If defmt is added later as a dependency, append:
    //   println!("cargo:rustc-link-arg=-Tdefmt.x");
}
