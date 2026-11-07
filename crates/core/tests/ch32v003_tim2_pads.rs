// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CH32V003 TIM2 channel outputs land on the CH32V003 reset-mapped pads, not
//! the STM32F103 ones, on the COMMITTED ch32fun-smoke chip profile.
//!
//! The CH32V003's TIM2 is the F1 general-purpose timer register block, so the
//! engine reuses the F1 timer and its F1 pad router. The pads differ: WCH's
//! CH32V003RM TIM2 remap table (PCFR1.TIM2_RM = 00) and ch32fun's
//! examples/tim2_pwm put T2CH1..T2CH4 on PD4, PD3, PC0, PD7, where the F103
//! has PA0..PA3. Before the CH32V003 table, the APB1PCENR TIM2 gate was also
//! masked off, so ch32fun's tim2_pwm left every pad flat.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

const RCC_APB2PCENR: u64 = 0x4002_1018;
const RCC_APB1PCENR: u64 = 0x4002_101C;
const GPIOA: u64 = 0x4001_0800;
const GPIOC: u64 = 0x4001_1000;
const GPIOD: u64 = 0x4001_1400;
const TIM2: u64 = 0x4000_0000;
const AFIO_PCFR1: u64 = 0x4001_0004;

/// CFGLR nibble: 10 MHz output, alternate-function push-pull (ch32fun's
/// `GPIO_Speed_10MHz | GPIO_CNF_OUT_PP_AF`).
const AF_PP: u32 = 0b1001;

fn bus() -> SystemBus {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip =
        ChipDescriptor::from_file(root.join("configs/chips/ch32v003f4p6-ch32fun-smoke.yaml"))
            .unwrap();
    let manifest: SystemManifest =
        serde_yaml::from_str("name: ch32\nchip: unused\nexternal_devices: []\n").unwrap();
    SystemBus::from_config(&chip, &manifest).unwrap()
}

fn func(bus: &SystemBus, port: &str, pin: u8) -> Option<String> {
    let idx = bus.find_peripheral_index_by_name(port).unwrap();
    bus.peripherals[idx].dev.gpio_routing(pin).unwrap().func
}

#[test]
fn ch32v003_tim2_channels_route_to_pd4_pd3_pc0_pd7() {
    let mut bus = bus();
    // GPIOA/GPIOC/GPIOD on APB2, TIM2 on APB1 bit 0.
    bus.write_u32(RCC_APB2PCENR, (1 << 2) | (1 << 4) | (1 << 5))
        .unwrap();
    bus.write_u32(RCC_APB1PCENR, 1).unwrap();
    assert_eq!(bus.read_u32(RCC_APB1PCENR).unwrap() & 1, 1, "TIM2EN kept");

    // Every candidate pad, CH32 and F103 alike, in AF push-pull.
    bus.write_u32(GPIOD, (AF_PP << 28) | (AF_PP << 16) | (AF_PP << 12))
        .unwrap();
    bus.write_u32(GPIOC, AF_PP).unwrap();
    bus.write_u32(GPIOA, (AF_PP << 8) | (AF_PP << 4)).unwrap();

    // ch32fun tim2_pwm: ATRLR 255, PWM mode 1 on all four channels, CCxE.
    bus.write_u32(TIM2 + 0x2C, 255).unwrap();
    bus.write_u32(TIM2 + 0x18, 0x6868).unwrap();
    bus.write_u32(TIM2 + 0x1C, 0x6868).unwrap();
    bus.write_u32(TIM2 + 0x20, 0x1111).unwrap();
    bus.write_u32(TIM2, 1).unwrap();

    assert_eq!(func(&bus, "gpiod", 4).as_deref(), Some("TIM2_CH1"));
    assert_eq!(func(&bus, "gpiod", 3).as_deref(), Some("TIM2_CH2"));
    assert_eq!(func(&bus, "gpioc", 0).as_deref(), Some("TIM2_CH3"));
    assert_eq!(func(&bus, "gpiod", 7).as_deref(), Some("TIM2_CH4"));
    // The F103 TIM2 pads (PA1/PA2 exist on a CH32V003F4P6) carry nothing.
    for pin in [1, 2] {
        assert_ne!(
            func(&bus, "gpioa", pin)
                .as_deref()
                .map(|f| f.starts_with("TIM2")),
            Some(true),
            "PA{pin} must not take an F103 TIM2 route"
        );
    }
}

/// CFGLR word with `AF_PP` on every listed pin.
fn af_pp(pins: &[u8]) -> u32 {
    pins.iter().fold(0, |w, &p| w | (AF_PP << (4 * p)))
}

/// TIM2 with all four channels in PWM mode 1, outputs enabled, under
/// PCFR1.TIM2_RM = `rm`, every CH32V003 TIM2 candidate pad in AF push-pull.
fn tim2_remapped(rm: u32) -> SystemBus {
    let mut bus = bus();
    // AFIO, GPIOC, GPIOD on APB2; TIM2 on APB1.
    bus.write_u32(RCC_APB2PCENR, 1 | (1 << 4) | (1 << 5))
        .unwrap();
    bus.write_u32(RCC_APB1PCENR, 1).unwrap();
    bus.write_u32(AFIO_PCFR1, rm << 8).unwrap();
    bus.write_u32(GPIOC, af_pp(&[0, 1, 2, 5, 7])).unwrap();
    bus.write_u32(GPIOD, af_pp(&[2, 3, 4, 5, 6, 7])).unwrap();
    bus.write_u32(TIM2 + 0x2C, 255).unwrap();
    bus.write_u32(TIM2 + 0x18, 0x6868).unwrap();
    bus.write_u32(TIM2 + 0x1C, 0x6868).unwrap();
    bus.write_u32(TIM2 + 0x20, 0x1111).unwrap();
    bus.write_u32(TIM2, 1).unwrap();
    bus
}

fn tim2_pads(bus: &SystemBus) -> Vec<(&'static str, u8)> {
    let mut on = Vec::new();
    for (port, pins) in [
        ("gpioc", &[0u8, 1, 2, 5, 7][..]),
        ("gpiod", &[2, 3, 4, 5, 6, 7]),
    ] {
        for &pin in pins {
            if func(bus, port, pin)
                .as_deref()
                .is_some_and(|f| f.starts_with("TIM2"))
            {
                on.push((port, pin));
            }
        }
    }
    on
}

/// The TIM2_RM remaps from the CH32V003 reference manual's field text
/// (quoted in ch32-rs/ch32-data), which ch32fun's tim2_pwm_remap and
/// tim2_encoder tables repeat.
#[test]
fn ch32v003_tim2_remaps_move_the_channels() {
    let cases: [(u32, [(&str, u8); 4]); 3] = [
        (
            0b01,
            [("gpioc", 5), ("gpioc", 2), ("gpiod", 2), ("gpioc", 1)],
        ),
        (
            0b10,
            [("gpioc", 1), ("gpiod", 3), ("gpioc", 0), ("gpiod", 7)],
        ),
        (
            0b11,
            [("gpioc", 1), ("gpioc", 7), ("gpiod", 6), ("gpiod", 5)],
        ),
    ];
    for (rm, pads) in cases {
        let bus = tim2_remapped(rm);
        for (ch, (port, pin)) in pads.iter().enumerate() {
            assert_eq!(
                func(&bus, port, *pin).as_deref(),
                Some(format!("TIM2_CH{}", ch + 1).as_str()),
                "TIM2_RM={rm:02b}: CH{} on {port} {pin}",
                ch + 1
            );
        }
        let mut want: Vec<(&str, u8)> = pads.to_vec();
        want.sort();
        let mut got = tim2_pads(&bus);
        got.sort();
        assert_eq!(got, want, "TIM2_RM={rm:02b}: no other pad carries TIM2");
    }
}
