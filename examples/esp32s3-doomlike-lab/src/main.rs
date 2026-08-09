//! ESP32-S3 Doom-like lab entry point (requires `--features hw`).
//!
//! Implemented in Task 4: GPIO/SPI startup, ILI9341 transport, fixed-step loop.

#![no_std]
#![no_main]

#[cfg(feature = "hw")]
use esp_backtrace as _;

#[cfg(feature = "hw")]
#[esp_hal::main]
fn main() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

// Provide a panic/entry placeholder when the binary is type-checked without `hw`.
#[cfg(not(feature = "hw"))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
