// SPDX-License-Identifier: MIT
// Portable functional firmware for the HeyPCB ESP32-S3 LDR source slice.
// This is compiled to WebAssembly; it is not ESP32 firmware or CPU emulation.
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

/// Sample the modeled LDR, convert its millivolts to a portable 12-bit count,
/// and drive the board's IO48 user LED above half scale. Return the ADC count,
/// or u32::MAX if the LED did not follow the computed threshold.
#[no_mangle]
pub extern "C" fn run(
    lux_port: u32,
    sensor_mv_port: u32,
    user_led_drive_port: u32,
    user_led_state_port: u32,
    lux: f64,
) -> u32 {
    unsafe {
        write(lux_port, lux);
        // The first step evaluates the data-driven LDR kit at this lux level.
        advance_ns(1_000);
        let millivolts = read(sensor_mv_port);
        let adc = (millivolts * 4095.0 / 3300.0) as u32;
        let should_light = adc >= 2048;

        write(user_led_drive_port, if should_light { 1.0 } else { 0.0 });
        advance_ns(1_000);
        let asserted = read(user_led_state_port) == if should_light { 1.0 } else { 0.0 };

        if should_light {
            write(user_led_drive_port, 0.0);
            advance_ns(1_000);
            let cleared = read(user_led_state_port) == 0.0;
            return if asserted && cleared { adc } else { u32::MAX };
        }

        if asserted { adc } else { u32::MAX }
    }
}
