//! ESP32-C3 hello-world for the LabWired simulator.
//!
//! The RISC-V analogue of `examples/esp32s3-hello-world`. Prints "Hello
//! world!" via esp-println (USB_SERIAL_JTAG path) once per second,
//! indefinitely. Runs identically on the simulator and on a connected
//! ESP32-C3 board.

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{delay::Delay, main};
use esp_println::println;

#[main]
fn main() -> ! {
    let _peripherals = esp_hal::init(esp_hal::Config::default());
    let delay = Delay::new();
    loop {
        println!("Hello world!");
        delay.delay_millis(1000);
    }
}
