fn main() {
    // embassy-stm32's `memory-x` feature provides MEMORY{}; cortex-m-rt's
    // link.x consumes it. --nmagic avoids page-aligning sections into flash.
    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
}
