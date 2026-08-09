// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The H735's newly-wired peripherals must be MODELS, not labels.
//!
//! ## Why this test exists
//!
//! `packages/board-config` publishes an H735 alternate-function pin table
//! derived from ST's device database. It advertised USART2, UART4, USART6,
//! I2C3, SPI3, TIM4, TIM8 and FDCAN1/2 as pin functions while
//! `configs/chips/stm32h735.yaml` modelled none of them. `labwired_describe
//! stm32h735` therefore listed eleven peripherals of which ten did not exist:
//! wiring a part to PA2 produced a pin labelled `uart2.tx` attached to nothing,
//! and nothing anywhere failed. The chip is now declared with all of them.
//!
//! A declaration is cheap and a wrong one is invisible, so this test refuses to
//! take the YAML's word for it. `chip_conformance` counts the estate (29 -> 38)
//! and `svd_conformance` checks each base address and IRQ against the vendored
//! SVD; neither reads a single register. What is asserted here is that the
//! window RESPONDS, and responds like the IP it claims to be.
//!
//! ## The two-sided assertion
//!
//! Each instance is probed at a READ-ONLY status register whose silicon reset
//! value is NONZERO, and every instance is clock-gated, which turns that into a
//! two-sided check with no arbitrary constants of my own:
//!
//!   * BEFORE the RCC enable bit is set, the probe must read 0.
//!   * AFTER it is set, the probe must read the IP's documented reset value --
//!     USART `ISR` = 0xC0 (TXE|TC), I2C `ISR` = 0x1 (TXE), SPI `SR` = 0x1002,
//!     TIM `ARR` = 0xFFFF, FDCAN `ENDN` = 0x8765_4321.
//!
//! An unmapped hole reads 0 in both states and fails the second assertion. A
//! peripheral wired without its gate reads its reset value in both states and
//! fails the first.
//!
//! The probes are read-only status registers ON PURPOSE. An earlier version of
//! this test wrote a value to a writable register (BRR/TIMINGR/PSC) and read it
//! back, which was WEAKER in both directions and vacuous in one: those
//! registers reset to 0, so "reads 0 while gated" held whether or not the gate
//! existed -- deleting usart2's `clock:` key left the test green. They also
//! turned out to retain writes while gated, so the write-back proved only that
//! some memory existed at the address. Reset values are the IP's own
//! fingerprint and cannot be produced by an accident of wiring.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};
use std::path::PathBuf;

/// RCC @ 0x5802_4400 (RM0468 §8.7). The H7 enable block sits at 0xD4..0xF4 --
/// nowhere near the F4/L4 0x30..0x44 the other STM32 models use.
const RCC: u64 = 0x5802_4400;
const RCC_AHB1ENR: u64 = RCC + 0xD8;
const RCC_APB1LENR: u64 = RCC + 0xE8;
const RCC_APB1HENR: u64 = RCC + 0xEC;
const RCC_APB2ENR: u64 = RCC + 0xF0;

/// FDCAN `ENDN` -- the endianness word, fixed at 0x8765_4321 on real silicon.
/// Probed rather than `CREL`, which this model answers even while clock-gated.
const FDCAN_ENDN: u64 = 0x004;

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Build the H735 the way every production entry point does: `from_config`
/// followed by `configure_cortex_m`.
fn machine_h735() -> Machine<impl Cpu> {
    let chip_path = root("configs/chips/stm32h735.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("h735 chip yaml");
    let manifest = SystemManifest {
        chip: chip_path.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build h735 bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    Machine::new(cpu, bus)
}

/// One clock-gated instance: where it lives, what turns it on, and the
/// read-only status register whose reset value identifies the IP.
struct Instance {
    id: &'static str,
    base: u64,
    enr: u64,
    bit: u32,
    /// (offset, value the IP reports once clocked). Must be nonzero, so that
    /// "reads 0" is unambiguously "not there / not clocked".
    probe: (u64, u32),
    probe_name: &'static str,
}

// Kept as an aligned table: the columns ARE the review surface here -- a wrong
// base or enable bit is spotted by scanning down, not by reading prose.
#[rustfmt::skip]
const NEW_INSTANCES: &[Instance] = &[
    // USART/UART -- stm32v2 IP. ISR resets to TXE|TC.
    Instance { id: "usart2", base: 0x4000_4400, enr: RCC_APB1LENR, bit: 17, probe: (0x1C, 0x0000_00C0), probe_name: "ISR" },
    Instance { id: "uart4",  base: 0x4000_4C00, enr: RCC_APB1LENR, bit: 19, probe: (0x1C, 0x0000_00C0), probe_name: "ISR" },
    Instance { id: "usart6", base: 0x4001_1400, enr: RCC_APB2ENR,  bit: 5,  probe: (0x1C, 0x0000_00C0), probe_name: "ISR" },
    // I2C v2 (h5 profile). ISR resets with TXE set.
    Instance { id: "i2c3",   base: 0x4000_5C00, enr: RCC_APB1LENR, bit: 23, probe: (0x18, 0x0000_0001), probe_name: "ISR" },
    // SPI v2 (stm32h5 profile). SR resets to 0x1002.
    Instance { id: "spi3",   base: 0x4000_3C00, enr: RCC_APB1LENR, bit: 15, probe: (0x14, 0x0000_1002), probe_name: "SR" },
    // Timers. ARR resets to all-ones (16-bit on both of these).
    Instance { id: "tim4",   base: 0x4000_0800, enr: RCC_APB1LENR, bit: 2,  probe: (0x2C, 0x0000_FFFF), probe_name: "ARR" },
    Instance { id: "tim8",   base: 0x4001_0400, enr: RCC_APB2ENR,  bit: 1,  probe: (0x2C, 0x0000_FFFF), probe_name: "ARR" },
    // FDCAN -- both instances behind the SINGLE shared APB1HENR.FDCANEN bit.
    Instance { id: "fdcan1", base: 0x4000_A000, enr: RCC_APB1HENR, bit: 8, probe: (FDCAN_ENDN, 0x8765_4321), probe_name: "ENDN" },
    Instance { id: "fdcan2", base: 0x4000_A400, enr: RCC_APB1HENR, bit: 8, probe: (FDCAN_ENDN, 0x8765_4321), probe_name: "ENDN" },
    // ADC1/ADC2 -- probed at HTR1, the analog-watchdog high threshold, which
    // resets to the 26-bit all-ones 0x03FF_FFFF. Deliberately chosen: an L4
    // layout answering at 0x24 returns TR2, which resets to 0, so this single
    // value distinguishes a real H7 block from the alias that used to stand in
    // for it. Both instances share RCC_AHB1ENR.ADC12EN.
    Instance { id: "adc1",   base: 0x4002_2000, enr: RCC_AHB1ENR,  bit: 5,  probe: (0x24, 0x03FF_FFFF), probe_name: "HTR1" },
    Instance { id: "adc2",   base: 0x4002_2100, enr: RCC_AHB1ENR,  bit: 5,  probe: (0x24, 0x03FF_FFFF), probe_name: "HTR1" },
];

fn enable(m: &mut Machine<impl Cpu>, enr: u64, bit: u32) {
    let cur = m.bus.read_u32(enr).expect("read RCC enable register");
    m.bus
        .write_u32(enr, cur | (1 << bit))
        .expect("write RCC enable register");
}

#[test]
fn new_instances_are_dark_until_their_rcc_bit_is_set() {
    let m = machine_h735();
    for inst in NEW_INSTANCES {
        let (off, _) = inst.probe;
        let addr = inst.base + off;
        let got = m.bus.read_u32(addr).unwrap_or(0);
        assert_eq!(
            got, 0,
            "{} {} at {:#010X} answered {:#010X} while still clock-gated -- its \
             `clock:` key is not being honoured, so the RCC enable bit is \
             decorative",
            inst.id, inst.probe_name, addr, got
        );
    }
}

#[test]
fn new_instances_report_their_ip_reset_value_once_clocked() {
    let mut m = machine_h735();
    for inst in NEW_INSTANCES {
        let (off, expected) = inst.probe;
        let addr = inst.base + off;
        enable(&mut m, inst.enr, inst.bit);
        let got = m.bus.read_u32(addr).unwrap_or(0);
        assert_eq!(
            got, expected,
            "{} {} at {:#010X} read {:#010X}, expected the IP reset value \
             {:#010X}. A read of 0 means the address is an unmapped hole and \
             the peripheral was never attached; any other value means the \
             wrong IP is wired here",
            inst.id, inst.probe_name, addr, got, expected
        );
    }
}

/// The two FDCAN instances are distinct devices, not one aliased twice. A
/// copy-paste that pointed both declarations at the same base would still pass
/// every assertion above.
#[test]
fn the_two_fdcan_instances_are_independent() {
    let mut m = machine_h735();
    enable(&mut m, RCC_APB1HENR, 8);

    // CCCR @ 0x18: set INIT (bit 0) on FDCAN1 only.
    const CCCR: u64 = 0x018;
    let f1 = 0x4000_A000 + CCCR;
    let f2 = 0x4000_A400 + CCCR;

    let before = m.bus.read_u32(f2).expect("fdcan2 CCCR");
    m.bus.write_u32(f1, 0x0000_0001).expect("fdcan1 CCCR write");
    let after = m.bus.read_u32(f2).expect("fdcan2 CCCR");

    assert_eq!(
        before, after,
        "writing FDCAN1 CCCR changed FDCAN2 CCCR ({before:#010X} -> {after:#010X}) -- \
         the two declarations resolve to ONE device"
    );
}
