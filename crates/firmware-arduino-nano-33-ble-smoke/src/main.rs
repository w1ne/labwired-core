// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arduino Nano 33 BLE smoke firmware.
//!
//! The board carries an nRF52840 (Cortex-M4F): 1 MiB flash at 0x0, 256 KiB RAM
//! at 0x20000000. Renode builds it as `using "platforms/cpus/nrf52840.repl"`
//! plus the RGB LED, an LSM9DS1 on TWI0 and an ArduCAM on SPI2/TWI0. `Serial1`
//! is UARTE0 on D0/TX = P1.03 and D1/RX = P1.10; the board's USB CDC
//! (`Serial`) is not modelled. The RGB LED is **active low** (Arduino's own
//! documentation: "the RGB pins are active-low"), wired to P0.24 (red) / P0.16
//! (green) / P0.06 (blue); the built-in D13 LED is P0.13 and active high.
//!
//! Prints:
//! * a banner and the modelled-surface summary,
//! * the nRF52840 chip row's eight silicon blocks (shared with
//!   `firmware-nrf52840-demo`: FICR/NVMC, CLOCK, GPIO, GPIOTE tasks, TIMER0,
//!   RTC0, RNG, PPI — bits 0..7),
//! * the RGB sequence with the LED bits of GPIO0 OUT read back from the model
//!   (`rgb=red out=00012040` — red's bit cleared, the other channels and D13
//!   set) — bit 8,
//! * a heartbeat on D13.
//!
//! A summary word goes to RAM at 0x2003FF00. The LSM9DS1 IMU and the ArduCAM
//! have no engine model; nothing here touches TWI0 or SPI2.

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

#[path = "../../firmware-nrf52840-demo/src/renode_checks.rs"]
mod renode_checks;
use renode_checks::*;

const UART0_PSEL_TXD: u32 = 0x4000_250C;
const UART0_PSEL_RXD: u32 = 0x4000_2514;
const UART0_BAUDRATE: u32 = 0x4000_2524;
const BAUDRATE_115200: u32 = 0x01D7_E000;

const GPIO0_DIRSET: u32 = 0x5000_0518;

const LED_RED: u32 = 1 << 24;
const LED_GREEN: u32 = 1 << 16;
const LED_BLUE: u32 = 1 << 6;
const LED_BUILTIN: u32 = 1 << 13; // D13, active high
const LED_MASK: u32 = LED_RED | LED_GREEN | LED_BLUE | LED_BUILTIN;

fn leds() -> u32 {
    rd(GPIO_OUT) & LED_MASK
}

#[entry]
fn main() -> ! {
    wr(UART_ENABLE, 8);
    wr(UART0_PSEL_TXD, 32 + 3); // P1.03 = D0/TX
    wr(UART0_PSEL_RXD, 32 + 10); // P1.10 = D1/RX
    wr(UART0_BAUDRATE, BAUDRATE_115200);

    let mut l = Line::new();
    l.s(b"ARDUINO NANO 33 BLE SMOKE").send();
    l.s(b"cpu=nrf52840 uart=uarte0").send();

    let mut summary = chip_checks(&mut l);

    // RGB off (active low), built-in LED on (active high).
    wr(GPIO0_DIRSET, LED_MASK);
    wr(GPIO_OUTSET, LED_RED | LED_GREEN | LED_BLUE | LED_BUILTIN);
    let mut pass = true;
    for (name, bit) in [
        (&b"red"[..], LED_RED),
        (&b"green"[..], LED_GREEN),
        (&b"blue"[..], LED_BLUE),
    ] {
        wr(GPIO_OUTCLR, bit);
        let out = leds();
        pass &= out == LED_MASK & !bit && rd(GPIO_IN) & bit == 0;
        l.s(b"rgb=").s(name).s(b" out=").hex(out, 8).send();
        for _ in 0..2000u32 {
            let _ = rd(UART_ENABLE);
        }
        wr(GPIO_OUTSET, bit);
    }
    pass &= leds() == LED_MASK;
    l.s(b"rgb off out=").hex(leds(), 8).ok(pass).send();
    summary |= (pass as u32) << 8;

    wr(RESULT, 0x5284_0000 | summary);
    if summary == 0x1FF {
        l.s(b"NANO33BLE PASS").send();
    } else {
        l.s(b"NANO33BLE FAIL summary=").hex(summary, 3).send();
    }

    // Heartbeat on D13 for the gpio_edges check.
    loop {
        wr(GPIO_OUTCLR, LED_BUILTIN);
        for _ in 0..1000u32 {
            let _ = rd(UART_ENABLE);
        }
        wr(GPIO_OUTSET, LED_BUILTIN);
        for _ in 0..1000u32 {
            let _ = rd(UART_ENABLE);
        }
    }
}
