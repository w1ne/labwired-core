// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end proof of classic ESP32 NeoPixel path: RMT → GPIO pad → WS2812
//! decode. Mirrors `esp32s3_ws2812_rmt.rs` against the classic register map
//! (RMTMEM at +0x800, TX_START in CONF1, RMT_SIG_OUT0 = 87, GPIO16).

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::declarative_led_strip::ws2812;
use labwired_core::peripherals::esp32::gpio::{Esp32Gpio, RMT_SIG_OUT0};
use labwired_core::peripherals::esp32::rmt::{Esp32Rmt, TX_START_BIT};
use labwired_core::Bus;
use std::sync::Arc;

const GPIO_BASE: u64 = 0x3FF4_4000;
const RMT_BASE: u64 = 0x3FF5_6000;
/// GPIO16 FUNC_OUT_SEL_CFG (array base 0x530, stride 4).
const FUNC_OUT_SEL16: u64 = GPIO_BASE + 0x530 + 16 * 4;
/// GPIO_ENABLE_W1TS — set output driver for GPIO0..31.
const ENABLE_W1TS: u64 = GPIO_BASE + 0x24;
const RMTMEM: u64 = RMT_BASE + 0x800;
const DATA_PIN: u8 = 16;

const CPU_HZ: u64 = 8_000_000;
const T0H: u32 = 2;
const T1H: u32 = 6;
const TLOW: u32 = 2;

fn bit_symbol(bit: bool) -> u32 {
    let high = if bit { T1H } else { T0H };
    high | (1 << 15) | (TLOW << 16)
}

fn push_pixel(symbols: &mut Vec<u32>, grb: u32) {
    for i in (0..24).rev() {
        symbols.push(bit_symbol((grb >> i) & 1 != 0));
    }
}

#[test]
fn classic_rmt_drives_ws2812_frame_decoded_end_to_end() {
    let mut bus = SystemBus::new();
    bus.add_peripheral("gpio", GPIO_BASE, 0x1000, None, Box::new(Esp32Gpio::new()));
    bus.add_peripheral(
        "rmt",
        RMT_BASE,
        0x1000,
        Some(47),
        Box::new(Esp32Rmt::new(47)),
    );

    let strip = Arc::new(ws2812(DATA_PIN, 3, CPU_HZ));
    assert!(
        SystemBus::install_gpio_observer(&mut bus, strip.clone()),
        "classic ESP32 GPIO must accept a WS2812 observer"
    );

    // pinMode(OUTPUT) + gpio_matrix_out — what Adafruit_NeoPixel / IDF do.
    bus.write_u32(ENABLE_W1TS, 1u32 << DATA_PIN).unwrap();
    bus.write_u32(FUNC_OUT_SEL16, RMT_SIG_OUT0).unwrap();

    let mut symbols = Vec::new();
    push_pixel(&mut symbols, 0x00_FF_00); // red
    push_pixel(&mut symbols, 0xFF_00_00); // green
    push_pixel(&mut symbols, 0x00_00_FF); // blue
    symbols.push(0); // END

    for (i, w) in symbols.iter().enumerate() {
        bus.write_u32(RMTMEM + (i as u64) * 4, *w).unwrap();
    }

    // CONF0: div_cnt=1, mem_size=2 (128 words > 73 symbols)
    bus.write_u32(RMT_BASE + 0x20, 1 | (2 << 24)).unwrap();
    // CONF1: TX_START
    bus.write_u32(RMT_BASE + 0x24, TX_START_BIT).unwrap();

    for c in 0..2000u64 {
        bus.set_current_cycle(c);
        bus.tick_peripherals();
    }

    assert_eq!(
        strip.pixels().iter().map(|p| p.wire).collect::<Vec<_>>(),
        vec![[0x00, 0xFF, 0x00], [0xFF, 0x00, 0x00], [0x00, 0x00, 0xFF]],
        "classic RMT-driven WS2812 frame must decode to red, green, blue"
    );
}
