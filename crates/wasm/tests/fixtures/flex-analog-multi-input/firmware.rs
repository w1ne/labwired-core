// SPDX-License-Identifier: MIT
// Portable functional firmware for a reusable two-input analog source.
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

/// Set both declared model inputs, allow one catalog sample boundary, and
/// return the observed battery-divider voltage in integer millivolts.
#[no_mangle]
pub extern "C" fn run(
    soc_port: u32,
    usb_port: u32,
    battery_mv_port: u32,
    soc_pct: f64,
    usb_present: f64,
) -> u32 {
    unsafe {
        write(soc_port, soc_pct);
        write(usb_port, usb_present);
        advance_ns(1_000);
        read(battery_mv_port) as u32
    }
}
