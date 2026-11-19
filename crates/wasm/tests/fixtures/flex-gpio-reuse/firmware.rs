// SPDX-License-Identifier: MIT
// Portable held-out GPIO workload. This is not AVR or CH32 target firmware.
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

/// Run a one-microsecond pulse followed by a two-microsecond pulse. Nonzero
/// mode is the negative control: it holds the drive low and requires no LED
/// response. `u32::MAX` marks a contract with no second indicator output.
#[no_mangle]
pub extern "C" fn run(
    drive_port: u32,
    led_port: u32,
    second_led_port: u32,
    mode: u32,
) -> u32 {
    unsafe {
        if mode != 0 {
            write(drive_port, 0.0);
            advance_ns(5_000);
            return if outputs_are(led_port, second_led_port, 0.0) {
                0
            } else {
                2
            };
        }

        write(drive_port, 1.0);
        advance_ns(1_000);
        if !outputs_are(led_port, second_led_port, 1.0) {
            return 2;
        }
        write(drive_port, 0.0);
        advance_ns(1_000);
        if !outputs_are(led_port, second_led_port, 0.0) {
            return 2;
        }
        write(drive_port, 1.0);
        advance_ns(2_000);
        if !outputs_are(led_port, second_led_port, 1.0) {
            return 2;
        }
        write(drive_port, 0.0);
        advance_ns(1_000);
        if !outputs_are(led_port, second_led_port, 0.0) {
            return 2;
        }
    }
    1
}

unsafe fn outputs_are(first: u32, second: u32, value: f64) -> bool {
    read(first) == value && (second == u32::MAX || read(second) == value)
}
