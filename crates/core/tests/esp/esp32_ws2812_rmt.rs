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

// The vendor SVD (tests/fixtures/real_world/esp32.svd) marks STATUS/ADDR,
// INT_RAW and INT_ST fields read-only, INT_CLR write-only, and DATE read-write.
// Replacing the pre-841be9961 SystemStub removes 19 write-echo proxy credits:
// offsets 0x60..=0x9c (16 words), 0xa0, 0xa4 and 0xac. The DATE reset is
// 0x16022600 and must retain its credit. On b02de9b9121b533e99a706fd67d4ab8b68c94198,
// the same 2,997-register probe measured 2,713 with the old stub versus 2,693
// with the new model: DATE was one additional, unjustified loss. The baseline
// was 2,701, leaving 12 credits of headroom; its eight-point deficit therefore
// did not identify eight particular registers. These tests pin the actual
// access contracts; they do not claim implemented STATUS/ADDR progress fields.
fn runtime_rmt_bus() -> SystemBus {
    let mut bus = SystemBus::new();
    let _cpu = labwired_core::system::xtensa::configure_xtensa_esp32(&mut bus);
    bus
}

#[test]
fn classic_rmt_interrupt_registers_preserve_hardware_owned_state() {
    use labwired_core::peripherals::esp32::gpio::RMT_SIG_OUT4;

    let mut bus = runtime_rmt_bus();
    // STATUS and APB address words are read-only, currently reset-zero
    // placeholders. This checks access semantics without claiming TX progress.
    for offset in (0x60..0xa0).step_by(4) {
        assert_eq!(bus.read_u32(RMT_BASE + offset).unwrap(), 0);
        bus.write_u32(RMT_BASE + offset, u32::MAX).unwrap();
        assert_eq!(bus.read_u32(RMT_BASE + offset).unwrap(), 0);
    }

    // Drive real symbols on two routed channels. Their TX_END bits occupy
    // different bytes, so selective word and byte clears are both observable.
    let pending = (1 << 0) | (1 << 12);
    bus.write_u32(ENABLE_W1TS, (1 << 16) | (1 << 17)).unwrap();
    let mut cycle = 0;
    for (channel, pin, signal) in [(0_u64, 16_u64, RMT_SIG_OUT0), (4, 17, RMT_SIG_OUT4)] {
        bus.write_u32(GPIO_BASE + 0x530 + pin * 4, signal).unwrap();
        let memory = RMTMEM + channel * 64 * 4;
        bus.write_u32(memory, bit_symbol(false)).unwrap();
        bus.write_u32(memory + 4, 0).unwrap();
        bus.write_u32(RMT_BASE + 0x20 + channel * 8, 1 | (1 << 24))
            .unwrap();
        bus.write_u32(RMT_BASE + 0x24 + channel * 8, TX_START_BIT)
            .unwrap();
        for _ in 0..32 {
            cycle += 1;
            bus.set_current_cycle(cycle);
            bus.tick_peripherals();
        }
    }
    assert_eq!(
        bus.read_u32(RMT_BASE + 0xa0).unwrap(),
        pending,
        "both routed TX streams must complete"
    );
    assert_eq!(
        bus.read_u32(RMT_BASE + 0xa4).unwrap(),
        0,
        "disabled interrupts stay masked"
    );
    bus.write_u32(RMT_BASE + 0xa8, 1).unwrap();
    assert_eq!(bus.read_u32(RMT_BASE + 0xa4).unwrap(), 1);
    bus.write_u32(RMT_BASE + 0xa8, pending).unwrap();
    assert_eq!(bus.read_u32(RMT_BASE + 0xa4).unwrap(), pending);

    for offset in [0xa0, 0xa4] {
        for value in [u32::MAX, 0] {
            bus.write_u32(RMT_BASE + offset, value).unwrap();
            assert_eq!(
                bus.read_u32(RMT_BASE + 0xa0).unwrap(),
                pending,
                "read-only write must preserve RAW"
            );
            assert_eq!(
                bus.read_u32(RMT_BASE + 0xa4).unwrap(),
                pending,
                "read-only write must preserve ST"
            );
        }
        for byte in [0, 1] {
            bus.write_u8(RMT_BASE + offset + byte, 0xff).unwrap();
            assert_eq!(bus.read_u32(RMT_BASE + 0xa0).unwrap(), pending);
            assert_eq!(bus.read_u32(RMT_BASE + 0xa4).unwrap(), pending);
        }
    }

    bus.write_u32(RMT_BASE + 0xac, 0).unwrap();
    assert_eq!(
        bus.read_u32(RMT_BASE + 0xa0).unwrap(),
        pending,
        "zero cannot acknowledge interrupts"
    );
    bus.write_u32(RMT_BASE + 0xac, 1).unwrap();
    assert_eq!(bus.read_u32(RMT_BASE + 0xa0).unwrap(), 1 << 12);
    assert_eq!(bus.read_u32(RMT_BASE + 0xa4).unwrap(), 1 << 12);
    assert_eq!(
        bus.read_u32(RMT_BASE + 0xac).unwrap(),
        0,
        "INT_CLR is write-only"
    );
    bus.write_u8(RMT_BASE + 0xad, 1 << 4).unwrap();
    assert_eq!(bus.read_u32(RMT_BASE + 0xa0).unwrap(), 0);
    assert_eq!(bus.read_u32(RMT_BASE + 0xa4).unwrap(), 0);
}

#[test]
fn classic_rmt_date_matches_vendor_reset_and_read_write_access() {
    let mut bus = runtime_rmt_bus();
    let date = RMT_BASE + 0xfc;
    assert_eq!(bus.read_u32(date).unwrap(), 0x1602_2600);
    for value in [u32::MAX, 0, 0x0123_4567] {
        bus.write_u32(date, value).unwrap();
        assert_eq!(bus.read_u32(date).unwrap(), value);
    }
    bus.write_u8(date + 2, 0xab).unwrap();
    assert_eq!(bus.read_u32(date).unwrap(), 0x01ab_4567);
}
