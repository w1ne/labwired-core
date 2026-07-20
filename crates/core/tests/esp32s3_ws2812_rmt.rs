// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end proof of the ESP32-S3 NeoPixel path: RMT → GPIO pad → observer →
//! WS2812 decode. Builds an S3 GPIO + RMT on a bus, routes GPIO48 (the onboard
//! NeoPixel pin) to the RMT channel-0 output signal through the GPIO matrix,
//! attaches a `Ws2812` decoder as a GPIO observer on that pad, loads RMTMEM with
//! the symbols for a known 3-pixel frame (red, green, blue), starts the RMT, and
//! ticks the bus — asserting the decoder recovered exactly those pixels.
//!
//! This is the whole Stage-1→3 chain exercised at once: the RMT timed playback
//! (Stage 2) drives real bit-level edges onto the matrix-routed pad (Stage 1 +
//! FUNC_OUT_SEL routing), and the WS2812 decoder (Stage 3) turns the observed
//! edge timing back into colors. Named `esp32s3_*` so a board-coverage ratchet
//! discovers it.

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::components::ws2812::Ws2812;
use labwired_core::peripherals::esp32s3::gpio::{Esp32s3Gpio, GpioObserver, RMT_SIG_OUT0};
use labwired_core::peripherals::esp32s3::rmt::{Esp32s3Rmt, TX_START_BIT};
use labwired_core::Bus;
use std::sync::Arc;

const GPIO_BASE: u64 = 0x6000_4000;
const RMT_BASE: u64 = 0x6001_6000;
/// GPIO48's FUNC_OUT_SEL_CFG register (array base 0x554, stride 4).
const FUNC_OUT_SEL48: u64 = GPIO_BASE + 0x554 + 48 * 4;
/// RMTMEM direct aperture inside the RMT window.
const RMTMEM: u64 = RMT_BASE + 0x400;
const DATA_PIN: u8 = 48;

/// 8 MHz decode clock → WS2812 HIGH threshold = 4 sim cycles, reset = 320.
const CPU_HZ: u64 = 8_000_000;
/// One RMT symbol per WS2812 bit: HIGH = 2 (bit 0) or 6 (bit 1) cycles, both
/// straddling the 4-cycle threshold; LOW = 2 cycles (well under the reset gap).
const T0H: u32 = 2;
const T1H: u32 = 6;
const TLOW: u32 = 2;

/// Encode one WS2812 bit as an RMT symbol word: HIGH pulse then LOW pulse.
/// Entry layout: `dur0[14:0] | level0<<15 | dur1[30:16] | level1<<31`.
fn bit_symbol(bit: bool) -> u32 {
    let high = if bit { T1H } else { T0H };
    high | (1 << 15) | (TLOW << 16) // level0 = high (1), level1 = low (0)
}

/// Append the 24 symbols (MSB-first, GRB) of one pixel to `symbols`.
fn push_pixel(symbols: &mut Vec<u32>, grb: u32) {
    for i in (0..24).rev() {
        symbols.push(bit_symbol((grb >> i) & 1 != 0));
    }
}

#[test]
fn rmt_drives_ws2812_frame_decoded_end_to_end() {
    let mut bus = SystemBus::new();
    bus.add_peripheral(
        "gpio",
        GPIO_BASE,
        0x1000,
        None,
        Box::new(Esp32s3Gpio::new()),
    );
    bus.add_peripheral(
        "rmt",
        RMT_BASE,
        0x1000,
        Some(40),
        Box::new(Esp32s3Rmt::new(40)),
    );

    // Attach the WS2812 decoder as a GPIO observer on the data pin.
    let strip = Arc::new(Ws2812::new(DATA_PIN, 3, CPU_HZ));
    {
        let idx = bus.find_peripheral_index_by_name("gpio").unwrap();
        let gpio = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Esp32s3Gpio>()
            .unwrap();
        gpio.add_observer(strip.clone() as Arc<dyn GpioObserver>);
    }

    // Route GPIO48 to the RMT channel-0 output signal (what firmware's GPIO
    // matrix setup does).
    bus.write_u32(FUNC_OUT_SEL48, RMT_SIG_OUT0).unwrap();

    // Build the symbol stream for a red, green, blue frame. WS2812 wire order is
    // GRB, so: red = GRB 0x00FF00, green = 0xFF0000, blue = 0x0000FF.
    let mut symbols = Vec::new();
    push_pixel(&mut symbols, 0x00_FF_00); // red   → decoded [G,R,B] = [0x00,0xFF,0x00]
    push_pixel(&mut symbols, 0xFF_00_00); // green → [0xFF,0x00,0x00]
    push_pixel(&mut symbols, 0x00_00_FF); // blue  → [0x00,0x00,0xFF]
    symbols.push(0); // END marker

    for (i, w) in symbols.iter().enumerate() {
        bus.write_u32(RMTMEM + (i as u64) * 4, *w).unwrap();
    }

    // Configure CH0: DIV_CNT=1, MEM_SIZE=2 (72 symbols need >48 words), then
    // start. The config write precedes TX_START so the arm captures DIV/MEM.
    bus.write_u32(RMT_BASE + 0x20, (1 << 8) | (2 << 16))
        .unwrap();
    bus.write_u32(RMT_BASE + 0x20, TX_START_BIT | (1 << 8) | (2 << 16))
        .unwrap();

    // Play the whole waveform out (one bus tick per sim cycle). 72 bits × ~8
    // cycles/bit ≈ 600; tick generously.
    for _ in 0..1200 {
        bus.tick_peripherals();
    }

    assert_eq!(
        strip.pixels(),
        vec![[0x00, 0xFF, 0x00], [0xFF, 0x00, 0x00], [0x00, 0x00, 0xFF]],
        "RMT-driven WS2812 frame must decode to red, green, blue (GRB wire order)"
    );
}
