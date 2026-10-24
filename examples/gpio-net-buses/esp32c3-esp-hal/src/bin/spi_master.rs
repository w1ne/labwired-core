//! esp-hal 1.1 SPI2 master on gpio nets (ESP32-C3): the exchange of
//! `src/esp32c_spi_master.c` through esp-hal's blocking driver, against the
//! G071 slave (`env-spi-esp32c3.yaml` with this firmware). Mode 0, 100 kHz,
//! GPIO4 SCK, GPIO5 MOSI, GPIO3 MISO, GPIO1 CS0 (hardware chip select).
//!
//! Console (USB-Serial-JTAG): `C3 spi true 81 82 83 84`, `C3 SPI DONE`.

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    main,
    spi::{
        master::{Config, Spi},
        Mode,
    },
    time::Rate,
};
use esp_println::println;

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default());
    let mut spi = Spi::new(
        p.SPI2,
        Config::default()
            .with_frequency(Rate::from_khz(100))
            .with_mode(Mode::_0),
    )
    .unwrap()
    .with_sck(p.GPIO4)
    .with_mosi(p.GPIO5)
    .with_miso(p.GPIO3)
    .with_cs(p.GPIO1);
    println!("C3 SPI BOOT");
    let mut buf = [0xA5u8, 0x3C, 0x5A, 0xC3];
    let r = spi.transfer(&mut buf);
    println!(
        "C3 spi {:?} {:02X} {:02X} {:02X} {:02X}",
        r.is_ok(),
        buf[0],
        buf[1],
        buf[2],
        buf[3]
    );
    println!("C3 SPI DONE");
    loop {}
}
