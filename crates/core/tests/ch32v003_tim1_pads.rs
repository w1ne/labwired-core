// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CH32V003 TIM1, the advanced-control timer, drives its channel outputs AND
//! its complementary CHxN outputs onto the CH32V003 reset-mapped pads, on the
//! COMMITTED ch32fun-smoke chip profile.
//!
//! PCFR1.TIM1_RM = 00 (CH32V003 DS pin table; ch32fun's
//! examples/tim1_pwm_complementary_outputs lists the same): T1CH1/CH1N
//! PD2/PD0, T1CH2/CH2N PA1/PA2, T1CH3/CH3N PC3/PD1, T1CH4 PC4. Before this,
//! the APB2PCENR TIM1 gate (bit 11) was masked off and the timer model did not
//! generate OCxN at all, so ch32fun's tim1_pwm left PD0 flat.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

const RCC_APB2PCENR: u64 = 0x4002_1018;
const GPIOA: u64 = 0x4001_0800;
const GPIOC: u64 = 0x4001_1000;
const GPIOD: u64 = 0x4001_1400;
const TIM1: u64 = 0x4001_2C00;

/// CFGLR nibble: 10 MHz output, alternate-function push-pull.
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

fn af_pp(pins: &[u8]) -> u32 {
    pins.iter().fold(0, |acc, &p| acc | (AF_PP << (4 * p)))
}

#[test]
fn ch32v003_tim1_channels_and_complements_route_to_their_reset_pads() {
    let mut bus = bus();
    // GPIOA/GPIOC/GPIOD and TIM1 (APB2 bit 11).
    bus.write_u32(RCC_APB2PCENR, (1 << 2) | (1 << 4) | (1 << 5) | (1 << 11))
        .unwrap();
    assert_eq!(
        bus.read_u32(RCC_APB2PCENR).unwrap() & (1 << 11),
        1 << 11,
        "TIM1EN kept"
    );
    bus.write_u32(GPIOA, af_pp(&[1, 2])).unwrap();
    bus.write_u32(GPIOC, af_pp(&[3, 4])).unwrap();
    bus.write_u32(GPIOD, af_pp(&[0, 1, 2])).unwrap();

    // PWM mode 1 on all four channels, every CCxE and CCxNE, MOE, CEN.
    bus.write_u32(TIM1 + 0x2C, 255).unwrap();
    bus.write_u32(TIM1 + 0x18, 0x6060).unwrap();
    bus.write_u32(TIM1 + 0x1C, 0x6060).unwrap();
    bus.write_u32(TIM1 + 0x34, 64).unwrap();
    bus.write_u32(TIM1 + 0x20, 0x1555).unwrap();
    bus.write_u32(TIM1 + 0x44, 1 << 15).unwrap();
    bus.write_u32(TIM1, 1).unwrap();

    assert_eq!(func(&bus, "gpiod", 2).as_deref(), Some("TIM1_CH1"));
    assert_eq!(func(&bus, "gpioa", 1).as_deref(), Some("TIM1_CH2"));
    assert_eq!(func(&bus, "gpioc", 3).as_deref(), Some("TIM1_CH3"));
    assert_eq!(func(&bus, "gpioc", 4).as_deref(), Some("TIM1_CH4"));
    assert_eq!(func(&bus, "gpiod", 0).as_deref(), Some("TIM1_CH1N"));
    assert_eq!(func(&bus, "gpioa", 2).as_deref(), Some("TIM1_CH2N"));
    assert_eq!(func(&bus, "gpiod", 1).as_deref(), Some("TIM1_CH3N"));
}

/// ch32fun's tim1_pwm enables CH1N alone (CC1NE|CC1NP, CC1E clear): PD0
/// takes the route and PD2 (CH1) is left to the GPIO.
#[test]
fn ch32v003_tim1_lone_ch1n_owns_pd0_only() {
    let mut bus = bus();
    bus.write_u32(RCC_APB2PCENR, (1 << 4) | (1 << 5) | (1 << 11))
        .unwrap();
    bus.write_u32(GPIOD, af_pp(&[0, 2])).unwrap();
    bus.write_u32(TIM1 + 0x2C, 255).unwrap();
    bus.write_u32(TIM1 + 0x18, 0x0060).unwrap();
    bus.write_u32(TIM1 + 0x20, 0x000C).unwrap();
    bus.write_u32(TIM1 + 0x44, 1 << 15).unwrap();
    bus.write_u32(TIM1, 1).unwrap();

    assert_eq!(func(&bus, "gpiod", 0).as_deref(), Some("TIM1_CH1N"));
    assert_ne!(func(&bus, "gpiod", 2).as_deref(), Some("TIM1_CH1"));
}
