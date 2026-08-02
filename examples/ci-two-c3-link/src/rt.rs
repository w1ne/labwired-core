//! Minimal RISC-V runtime: point the stack somewhere real, then call main.
//!
//! No `riscv-rt`, no ESP-IDF. The whole point of this lab is that what runs is
//! the firmware you can read on one screen.

#[macro_export]
macro_rules! bare_metal_entry {
    ($main:path) => {
        core::arch::global_asm!(
            ".section .text._start",
            ".global _start",
            "_start:",
            "  la sp, _stack_top",
            "  j  __rust_entry",
        );

        #[no_mangle]
        pub extern "C" fn __rust_entry() -> ! {
            $main()
        }

        #[panic_handler]
        fn panic(_: &core::panic::PanicInfo) -> ! {
            loop {}
        }
    };
}
