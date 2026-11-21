//! ESP32-C2 (ESP8684H4X) esp-hal smoke: `esp_hal::init` (BBPLL through the
//! ROM regi2c helpers, RTC power-up), a UART0 console, GPIO8 toggles and
//! `Delay::delay_millis(2)` timed by SYSTIMER. Built image:
//! tests/fixtures/esp32c2-esp-hal-delay.elf (see its .md for the build).
#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
    main,
    time::Instant,
};
use esp_println::println;


#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default());
    println!("LW-C2 boot");
    let mut led = Output::new(p.GPIO8, Level::Low, OutputConfig::default());
    let delay = Delay::new();
    for i in 0..3u32 {
        let t0 = Instant::now();
        led.toggle();
        delay.delay_millis(2);
        println!("LW-C2 tick {} {}us", i, t0.elapsed().as_micros());
    }
    println!("LW-C2 done");
    loop {}
}
