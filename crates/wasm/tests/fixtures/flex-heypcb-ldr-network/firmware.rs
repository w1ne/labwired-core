// SPDX-License-Identifier: MIT
// Portable functional firmware for the HeyPCB ESP32-S3 LDR sensor network.
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

/// Apply an illuminance, wait for SNS_AN1 to settle, convert its volts to a
/// portable 12-bit count, and drive the board's IO48 user LED above half
/// scale. Return the ADC count, or u32::MAX if the LED did not follow the
/// computed threshold.
#[no_mangle]
pub extern "C" fn run(
    lux_port: u32,
    sensor_v_port: u32,
    user_led_drive_port: u32,
    user_led_state_port: u32,
    lux: f64,
) -> u32 {
    unsafe {
        write(lux_port, lux);
        // C7 charges through R6 in parallel with U4: under 1 ms per time
        // constant inside the modeled 1..100 lx range.
        advance_ns(4_000_000);
        let volts = read(sensor_v_port);
        let adc = (volts * 4095.0 / 3.3) as u32;
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
