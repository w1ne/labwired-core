// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CH32V003 TIM1 INPUT CAPTURE on the COMMITTED ch32fun-smoke chip profile:
//! a pad edge reaches the timer's TI1 through the CH32V003 pad map, and the
//! capture flag requests PFIC interrupt 37 (TIM1_CC) as a LEVEL that reading
//! CH1CVR drops.
//!
//! Before this the capture router only knew the STM32 tables, so on a
//! CH32V003 it put TIM1's inputs on PA8..PA11 (this package has no such pads)
//! and TIM2's CH2/CH3 inputs on PA1/PA2, which are TIM1 pads here. And the
//! timer's `irq:` reached the PFIC as a latched pend, so ch32fun's
//! input_capture ran its handler a second time after the flag was gone and
//! printed "badtrigger".

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::qingke_pfic::QingkePficState;
use labwired_core::Bus;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

const RCC_APB2PCENR: u64 = 0x4002_1018;
const AFIO_PCFR1: u64 = 0x4001_0004;
const TIM1: u64 = 0x4001_2C00;
const TIM1_CHCTLR1: u64 = TIM1 + 0x18;
const TIM1_CCER: u64 = TIM1 + 0x20;
const TIM1_DMAINTENR: u64 = TIM1 + 0x0C;
const TIM1_INTFR: u64 = TIM1 + 0x10;
const TIM1_CH1CVR: u64 = TIM1 + 0x34;
const TIM1_CTLR1: u64 = TIM1 + 0x00;
const TIM2: u64 = 0x4000_0000;
const TIM2_CHCTLR1: u64 = TIM2 + 0x18;
const TIM2_CCER: u64 = TIM2 + 0x20;
const TIM2_INTFR: u64 = TIM2 + 0x10;
const TIM1_CC_IRQ: u32 = 37;
const CC1IF: u32 = 1 << 1;
const CC2IF: u32 = 1 << 2;

fn bus_and_pfic() -> (SystemBus, Arc<QingkePficState>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip =
        ChipDescriptor::from_file(root.join("configs/chips/ch32v003f4p6-ch32fun-smoke.yaml"))
            .unwrap();
    let manifest: SystemManifest =
        serde_yaml::from_str("name: ch32\nchip: unused\nexternal_devices: []\n").unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let _cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
    let pfic = bus
        .peripherals
        .iter()
        .find_map(|p| p.dev.qingke_pfic_state())
        .unwrap();
    // AFIO, GPIOA, GPIOC, GPIOD and TIM1 on APB2; TIM2 on APB1. The GPIO
    // pads stay at their reset configuration, floating input.
    bus.write_u32(
        RCC_APB2PCENR,
        1 | (1 << 2) | (1 << 4) | (1 << 5) | (1 << 11),
    )
    .unwrap();
    bus.write_u32(0x4002_101C, 1).unwrap();
    (bus, pfic)
}

/// ch32fun examples/input_capture's TIM1 setup: CH1 captures TI1 rising,
/// CH2 captures TI1 falling (CC2S = 10, CC2P), both interrupts enabled.
fn input_capture_setup(bus: &mut SystemBus) {
    bus.write_u32(TIM1_CHCTLR1, 0x0201).unwrap();
    bus.write_u32(TIM1_CCER, 0x0031).unwrap();
    bus.write_u32(TIM1_CTLR1, 1).unwrap();
    bus.write_u32(TIM1_DMAINTENR, CC1IF | CC2IF).unwrap();
}

fn requested(pfic: &QingkePficState) -> bool {
    pfic.level[TIM1_CC_IRQ as usize / 32].load(Ordering::SeqCst) & (1 << (TIM1_CC_IRQ % 32)) != 0
}

fn drive(bus: &mut SystemBus, port: &str, pin: u8, level: bool) {
    let idx = bus.find_peripheral_index_by_name(port).unwrap();
    bus.set_peripheral_gpio_input(idx, pin, level);
}

fn intfr(bus: &SystemBus) -> u32 {
    bus.read_u32(TIM1_INTFR).unwrap() & (CC1IF | CC2IF)
}

#[test]
fn pd2_edges_capture_on_tim1_and_request_tim1_cc_as_a_level() {
    let (mut bus, pfic) = bus_and_pfic();
    input_capture_setup(&mut bus);
    assert!(!requested(&pfic));

    drive(&mut bus, "gpiod", 2, true);
    assert_eq!(
        intfr(&bus),
        CC1IF,
        "rising edge on T1CH1 (PD2) captures CH1"
    );
    assert!(requested(&pfic), "CC1IF & CC1IE requests PFIC 37");

    // The handler reads CH1CVR; that clears CC1IF and must drop the request
    // before the handler returns, or the core takes it again.
    let _ = bus.read_u16(TIM1_CH1CVR).unwrap();
    assert_eq!(intfr(&bus), 0);
    assert!(
        !requested(&pfic),
        "the read that clears CC1IF drops the level"
    );

    drive(&mut bus, "gpiod", 2, false);
    assert_eq!(intfr(&bus), CC2IF, "falling edge captures CH2 (TI1, CC2P)");
    assert!(requested(&pfic));
}

#[test]
fn tim1_inputs_follow_the_afio_tim1_remap() {
    let (mut bus, pfic) = bus_and_pfic();
    input_capture_setup(&mut bus);
    // PCFR1.TIM1_RM = 01: T1CH1 moves to PC6.
    bus.write_u32(AFIO_PCFR1, 0b01 << 6).unwrap();
    drive(&mut bus, "gpiod", 2, true);
    assert_eq!(intfr(&bus), 0, "PD2 is not T1CH1 under partial remap 1");
    assert!(!requested(&pfic));
    drive(&mut bus, "gpioc", 6, true);
    assert_eq!(intfr(&bus), CC1IF, "PC6 is");
}

#[test]
fn tim2_inputs_are_on_the_ch32v003_pads_not_the_stm32f1_ones() {
    let (mut bus, _pfic) = bus_and_pfic();
    // TIM2 CH2 capturing TI2 rising.
    bus.write_u32(TIM2_CHCTLR1, 0x0100).unwrap();
    bus.write_u32(TIM2_CCER, 0x0010).unwrap();
    bus.write_u32(TIM2, 1).unwrap();
    // PA1 is TIM2_CH2 on an STM32F103 and T1CH2 on the CH32V003.
    drive(&mut bus, "gpioa", 1, true);
    assert_eq!(
        bus.read_u32(TIM2_INTFR).unwrap() & CC2IF,
        0,
        "PA1 is not T2CH2"
    );
    drive(&mut bus, "gpiod", 3, true);
    assert_eq!(
        bus.read_u32(TIM2_INTFR).unwrap() & CC2IF,
        CC2IF,
        "PD3 is T2CH2"
    );
}
