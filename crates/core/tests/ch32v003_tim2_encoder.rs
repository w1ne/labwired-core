// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CH32V003 TIM2 ENCODER MODE on the COMMITTED ch32fun-smoke chip profile,
//! set up the way ch32fun's examples/tim2_encoder does it: PCFR1.TIM2_RM = 01
//! (A on PC5 = T2CH1, B on PC2 = T2CH2), both pads pulled up through
//! CFGLR/OUTDR, SMCFGR.SMS = 011, CNT preset to 0x8FFF, CEN.
//!
//! Before this the timer ignored SMCFGR and counted the 48 MHz clock, and the
//! GPIO port only reported a timer input that changed under an IDR write or
//! an outside driver. An input that was already high behind its pull-up when
//! the route went live never reached the timer, so the first detent counted
//! two of its four edges.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

const RCC_APB2PCENR: u64 = 0x4002_1018;
const RCC_APB1PCENR: u64 = 0x4002_101C;
const AFIO_PCFR1: u64 = 0x4001_0004;
const GPIOC_CFGLR: u64 = 0x4001_1000;
const GPIOC_OUTDR: u64 = 0x4001_100C;
const TIM2: u64 = 0x4000_0000;
const TIM2_SMCFGR: u64 = TIM2 + 0x08;
const TIM2_CNT: u64 = TIM2 + 0x24;
const TIM2_CTLR1: u64 = TIM2 + 0x00;
const START: u32 = 0x8FFF;
const A: u8 = 5;
const B: u8 = 2;

fn bus() -> SystemBus {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip =
        ChipDescriptor::from_file(root.join("configs/chips/ch32v003f4p6-ch32fun-smoke.yaml"))
            .unwrap();
    let manifest: SystemManifest =
        serde_yaml::from_str("name: ch32\nchip: unused\nexternal_devices: []\n").unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let _cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
    bus
}

/// ch32fun's t2encoder_init(), register for register.
fn t2encoder_init(bus: &mut SystemBus) {
    bus.write_u32(RCC_APB2PCENR, 1 | (1 << 4)).unwrap();
    bus.write_u32(RCC_APB1PCENR, 1).unwrap();
    bus.write_u32(AFIO_PCFR1, 0b01 << 8).unwrap();
    for pin in [B, A] {
        let cfg = bus.read_u32(GPIOC_CFGLR).unwrap() & !(0xF << (4 * pin));
        bus.write_u32(GPIOC_CFGLR, cfg | (0b1000 << (4 * pin)))
            .unwrap();
        let out = bus.read_u32(GPIOC_OUTDR).unwrap();
        bus.write_u32(GPIOC_OUTDR, out | (1 << pin)).unwrap();
    }
    bus.write_u32(TIM2_SMCFGR, 0b011).unwrap();
    bus.write_u32(TIM2 + 0x14, 1).unwrap();
    bus.write_u32(TIM2_CNT, START).unwrap();
    bus.write_u32(TIM2_CTLR1, 1).unwrap();
}

fn drive(bus: &mut SystemBus, pin: u8, level: bool) {
    let idx = bus.find_peripheral_index_by_name("gpioc").unwrap();
    bus.set_peripheral_gpio_input(idx, pin, level);
}

fn cnt(bus: &SystemBus) -> u32 {
    bus.read_u32(TIM2_CNT).unwrap() & 0xFFFF
}

/// One detent of a switch encoder idling with both contacts open (high):
/// A closes first going forward, B first going back.
fn detent(bus: &mut SystemBus, forward: bool) {
    let (first, second) = if forward { (A, B) } else { (B, A) };
    drive(bus, first, false);
    drive(bus, second, false);
    drive(bus, first, true);
    drive(bus, second, true);
}

#[test]
fn ch32fun_tim2_encoder_counts_four_per_detent_from_pulled_up_pads() {
    let mut bus = bus();
    t2encoder_init(&mut bus);
    assert_eq!(cnt(&bus), START);
    detent(&mut bus, true);
    assert_eq!(
        cnt(&bus),
        START + 4,
        "the first detent counts all four edges"
    );
    detent(&mut bus, true);
    detent(&mut bus, true);
    assert_eq!(cnt(&bus), START + 12);
    for _ in 0..5 {
        detent(&mut bus, false);
    }
    assert_eq!(cnt(&bus), START - 8);
    assert_ne!(
        bus.read_u32(TIM2_CTLR1).unwrap() & (1 << 4),
        0,
        "CTLR1.DIR shows the last count went down"
    );
}

/// The board holds both pads high (open contacts on a board with its own
/// pull-ups) before firmware runs: the routes go live on pads that are
/// already high, which is still the level the timer must start from.
#[test]
fn pads_held_high_before_the_route_goes_live_still_reach_the_timer() {
    let mut bus = bus();
    drive(&mut bus, A, true);
    drive(&mut bus, B, true);
    t2encoder_init(&mut bus);
    detent(&mut bus, true);
    assert_eq!(cnt(&bus), START + 4);
}

/// Encoder mode counts edges only: the 48 MHz clock does not move CNT.
#[test]
fn the_cpu_clock_does_not_count_in_encoder_mode() {
    let mut bus = bus();
    t2encoder_init(&mut bus);
    // Both clocks: the walk (no cycle clock attached) and the lazy
    // scheduler-mode replay against the published cycle.
    for _ in 0..10_000 {
        bus.tick_peripherals_fully();
    }
    let now = bus.current_cycle;
    bus.set_current_cycle(now + 10_000_000);
    assert_eq!(cnt(&bus), START);
}

/// The remap written AFTER the pads were pulled up: no GPIO write follows,
/// so only the AFIO store can tell the timer that PC5/PC2 are its inputs now
/// and already high.
#[test]
fn a_remap_after_the_pad_setup_hands_the_timer_the_pad_levels() {
    let mut bus = bus();
    t2encoder_init(&mut bus);
    bus.write_u32(AFIO_PCFR1, 0).unwrap();
    // Re-arm from mid-scale with TIM2_RM = 00: PC5/PC2 are not TIM2 pads.
    bus.write_u32(TIM2_CNT, START).unwrap();
    detent(&mut bus, true);
    assert_eq!(cnt(&bus), START, "PC5/PC2 do not feed TIM2 at TIM2_RM = 00");
    bus.write_u32(AFIO_PCFR1, 0b01 << 8).unwrap();
    detent(&mut bus, true);
    assert_eq!(cnt(&bus), START + 4);
}
