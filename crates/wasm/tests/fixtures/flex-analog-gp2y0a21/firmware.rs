// SPDX-License-Identifier: MIT
// Portable functional behavior for a catalog-backed Sharp GP2Y0A21 source.
// This module is compiled to Wasm and does not execute a target MCU binary.
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

/// Sample one distance and drive the near indicator from the modeled voltage.
#[no_mangle]
pub extern "C" fn run(
    distance_port: u32,
    voltage_port: u32,
    indicator_drive_port: u32,
    distance_mm: u32,
) -> u32 {
    unsafe {
        write(distance_port, f64::from(distance_mm));
        advance_ns(1_000);
        let voltage_mv = read(voltage_port);
        write(indicator_drive_port, f64::from(voltage_mv >= 1_500.0));
        advance_ns(1_000);
        voltage_mv as u32
    }
}
