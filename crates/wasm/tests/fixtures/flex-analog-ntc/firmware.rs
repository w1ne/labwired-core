// SPDX-License-Identifier: MIT
// Portable alarm behavior against the registered NTC analog-source model.
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

/// Read one NTC sample and assert the alarm above the project's 2000 mV limit.
#[no_mangle]
pub extern "C" fn run(
    temperature_port: u32,
    voltage_port: u32,
    alarm_drive_port: u32,
    temperature_c: f64,
) -> u32 {
    unsafe {
        write(temperature_port, temperature_c);
        advance_ns(1_000);
        let alarm = if read(voltage_port) >= 2_000.0 { 1.0 } else { 0.0 };
        write(alarm_drive_port, alarm);
        advance_ns(1_000);
        read(voltage_port) as u32
    }
}
