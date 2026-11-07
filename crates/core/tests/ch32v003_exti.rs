// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CH32V003 EXTI on the COMMITTED ch32fun-smoke chip profile: a pad edge
//! reaches PFIC interrupt 20 (EXTI7_0) through the CH32V003 `AFIO_EXTICR`,
//! as a level that the handler's INTFR write-1-clear drops.
//!
//! The CH32V003's EXTI is the F1 register file, but its port select is not:
//! one `AFIO_EXTICR` at 0x08 with a 2-bit field per line (ch32fun's
//! `AFIO_EXTICR_EXTI3_PD` = 0xC0), where the F103 has four EXTICRs of 4-bit
//! fields. Read the F103 way, 0xC0 selects port 12 for line 1 and port A for
//! line 3, so ch32fun's exti_pin_change_isr never saw its PD3 edge.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::qingke_pfic::QingkePficState;
use labwired_core::Bus;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

const RCC_APB2PCENR: u64 = 0x4002_1018;
const AFIO_EXTICR: u64 = 0x4001_0008;
const EXTI: u64 = 0x4001_0400;
const INTENR: u64 = EXTI;
const RTENR: u64 = EXTI + 0x08;
const FTENR: u64 = EXTI + 0x0C;
const SWIEVR: u64 = EXTI + 0x10;
const INTFR: u64 = EXTI + 0x14;
const EXTI7_0_IRQ: u32 = 20;

fn bus_and_pfic() -> (SystemBus, Arc<QingkePficState>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip =
        ChipDescriptor::from_file(root.join("configs/chips/ch32v003f4p6-ch32fun-smoke.yaml"))
            .unwrap();
    let manifest: SystemManifest =
        serde_yaml::from_str("name: ch32\nchip: unused\nexternal_devices: []\n").unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    // Shares the PFIC with the core and every PFIC level source.
    let _cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
    let pfic = bus
        .peripherals
        .iter()
        .find_map(|p| p.dev.qingke_pfic_state())
        .unwrap();
    // exti_pin_change_isr: GPIOC, GPIOD and AFIO clocks.
    bus.write_u32(RCC_APB2PCENR, (1 << 5) | (1 << 4) | 1)
        .unwrap();
    (bus, pfic)
}

fn requested(pfic: &QingkePficState) -> bool {
    pfic.level[0].load(Ordering::SeqCst) & (1 << EXTI7_0_IRQ) != 0
}

fn drive(bus: &mut SystemBus, port: &str, pin: u8, level: bool) {
    let idx = bus.find_peripheral_index_by_name(port).unwrap();
    bus.set_peripheral_gpio_input(idx, pin, level);
}

#[test]
fn pd3_rising_edge_requests_exti7_0_until_intfr_is_cleared() {
    let (mut bus, pfic) = bus_and_pfic();
    // The example's configuration, store for store.
    bus.write_u32(AFIO_EXTICR, 0xC0).unwrap(); // AFIO_EXTICR_EXTI3_PD
    bus.write_u32(INTENR, 1 << 3).unwrap();
    bus.write_u32(RTENR, 1 << 3).unwrap();
    assert!(!requested(&pfic));

    // The same line number on the wrong port does nothing.
    drive(&mut bus, "gpioc", 3, true);
    assert_eq!(bus.read_u32(INTFR).unwrap(), 0, "PC3 is not selected");

    drive(&mut bus, "gpiod", 3, true);
    assert_eq!(bus.read_u32(INTFR).unwrap(), 1 << 3);
    assert!(requested(&pfic), "INTFR & INTENR drives interrupt 20");

    // The handler's `EXTI->INTFR = EXTI_Line3` drops the request at once.
    bus.write_u32(INTFR, 1 << 3).unwrap();
    assert_eq!(bus.read_u32(INTFR).unwrap(), 0);
    assert!(!requested(&pfic));

    // Release: a falling edge, and FTENR is clear.
    drive(&mut bus, "gpiod", 3, false);
    assert_eq!(bus.read_u32(INTFR).unwrap(), 0);
    assert!(!requested(&pfic));
}

#[test]
fn intenr_gates_the_request_not_the_flag() {
    let (mut bus, pfic) = bus_and_pfic();
    // Line 6 on PC6, falling edge, interrupt masked.
    bus.write_u32(AFIO_EXTICR, 0b10 << 12).unwrap();
    bus.write_u32(FTENR, 1 << 6).unwrap();
    drive(&mut bus, "gpioc", 6, true);
    drive(&mut bus, "gpioc", 6, false);
    assert_eq!(bus.read_u32(INTFR).unwrap(), 1 << 6, "flag latches masked");
    assert!(!requested(&pfic));
    bus.write_u32(INTENR, 1 << 6).unwrap();
    assert!(requested(&pfic), "unmasking a pending line requests it");
}

#[test]
fn swievr_and_the_ten_implemented_lines() {
    let (mut bus, pfic) = bus_and_pfic();
    bus.write_u32(INTENR, 0xFFFF_FFFF).unwrap();
    assert_eq!(bus.read_u32(INTENR).unwrap(), 0x3FF, "lines 0..9");
    // Line 9 (AWU) latches but is not a line of EXTI7_0.
    bus.write_u32(SWIEVR, 1 << 9).unwrap();
    assert_eq!(bus.read_u32(INTFR).unwrap(), 1 << 9);
    assert!(!requested(&pfic));
    bus.write_u32(SWIEVR, (1 << 9) | 1).unwrap();
    assert!(requested(&pfic), "software event on line 0");
    bus.write_u32(INTFR, 0x3FF).unwrap();
    assert_eq!(bus.read_u32(INTFR).unwrap(), 0);
    assert!(!requested(&pfic));
}
