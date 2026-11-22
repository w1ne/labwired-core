// SPDX-License-Identifier: MIT
// Portable two-sensor workload using the same registered source model twice.
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

/// Sample two independent sensors, drive their indicators, and pack both ADC readings.
#[no_mangle]
pub extern "C" fn run(
    left_distance_port: u32,
    left_voltage_port: u32,
    left_indicator_drive_port: u32,
    right_distance_port: u32,
    right_voltage_port: u32,
    right_indicator_drive_port: u32,
    left_distance_mm: u32,
    right_distance_mm: u32,
) -> u32 {
    unsafe {
        write(left_distance_port, f64::from(left_distance_mm));
        write(right_distance_port, f64::from(right_distance_mm));
        advance_ns(1_000);
        let left_mv = read(left_voltage_port) as u32;
        let right_mv = read(right_voltage_port) as u32;
        write(left_indicator_drive_port, f64::from(left_mv >= 1_500));
        write(right_indicator_drive_port, f64::from(right_mv >= 1_500));
        advance_ns(1_000);
        left_mv * 10_000 + right_mv
    }
}
