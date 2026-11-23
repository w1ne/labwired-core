// SPDX-License-Identifier: MIT
// Portable functional firmware for a registered formula-backed analog source.
// This is compiled to Wasm; it does not execute a target MCU binary.
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

/// Apply one temperature sample and return the catalog model's millivolts.
#[no_mangle]
pub extern "C" fn run(temperature_port: u32, voltage_port: u32, temperature_c: f64) -> u32 {
    unsafe {
        write(temperature_port, temperature_c);
        advance_ns(1_000);
        read(voltage_port) as u32
    }
}
