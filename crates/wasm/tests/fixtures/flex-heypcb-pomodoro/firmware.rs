// SPDX-License-Identifier: MIT
// Portable Flex behavior demonstration for the source-mapped D1/D2 LED pair.
// This compiles to WebAssembly and contains no CH32V003 instructions or MMIO.
#![no_std]

#[link(wasm_import_module = "labwired")]
extern "C" {
    fn read(port: u32) -> f64;
    fn write(port: u32, value: f64);
    fn advance_ns(delta: u32);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}

/// Positive mode pulses the imported U1.PD6 signal and checks both modeled
/// LED logic outputs. Negative mode keeps the drive low and requires both to
/// remain inactive. Return 1 for the positive observation, 0 for the negative
/// control, and 2 if either behavior differs from the portable contract.
#[no_mangle]
pub extern "C" fn run(drive_port: u32, d1_port: u32, d2_port: u32, mode: u32) -> u32 {
    unsafe {
        if mode == 0 {
            write(drive_port, 1.0);
            advance_ns(1_000);
            let asserted = read(d1_port) == 1.0 && read(d2_port) == 1.0;
            write(drive_port, 0.0);
            advance_ns(1_000);
            let cleared = read(d1_port) == 0.0 && read(d2_port) == 0.0;
            return if asserted && cleared { 1 } else { 2 };
        }

        write(drive_port, 0.0);
        advance_ns(2_000);
        if read(d1_port) == 0.0 && read(d2_port) == 0.0 {
            0
        } else {
            2
        }
    }
}
