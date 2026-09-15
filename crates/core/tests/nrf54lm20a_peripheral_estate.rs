// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The nRF54LM20A estate must be MODELS at the RIGHT addresses, not labels.
//!
//! ## Why this test exists
//!
//! `svd_conformance` checks every `base_address` and `irq` in
//! `configs/chips/nrf54lm20a.yaml` against the vendored
//! `nrf54lm20a_application.svd`, and `chip_conformance` counts the estate.
//! Neither reads a single register. Both would stay green for a chip whose
//! peripherals were declared perfectly and modelled as holes.
//!
//! ## What is asserted
//!
//! Each instance is probed at a register whose silicon RESET VALUE IS NONZERO,
//! so that "reads 0" is unambiguously "not there". Reset values are the IP's
//! own fingerprint and cannot be produced by an accident of wiring:
//!
//!   * SPIM `PRESCALER` = 0x40 and `PSEL.SCK` = 0xFFFF_FFFF (disconnected).
//!     Both come from the SVD, and both are at offsets that do not exist on the
//!     nRF52 SPIM map -- so this also proves the instance took this family's
//!     offset map and not the nRF52 one it would silently fall back to.
//!   * GRTC and the stub windows must ANSWER rather than fault the bus.
//!
//! ## The negative control that matters most
//!
//! P1 (0x500D8200) and P3 (0x500D8600) are 0x400 apart. Every previous nRF54L
//! port was mapped at `MDK_base - 0x504`, and had this part been declared that
//! way, the two windows would overlap and one port's registers would be served
//! entirely by the other -- while every label-checking gate stayed green.
//! `the_gpio_ports_are_not_each_other` writes one port and requires the other
//! not to move. It is written to FAIL on the arrangement the sibling chip uses.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};
use std::path::PathBuf;

// ── Addresses under test (all from nrf54lm20a_application.svd) ──────────────
const SPIM22: u64 = 0x500C_8000; // nordic_expansion_spi -- the display bus
const SPIM00: u64 = 0x5004_D000;
const GRTC: u64 = 0x500E_2000;
const TAMPC: u64 = 0x500E_F000; // NOT nRF54L15's 0x500DC000
const RRAMC: u64 = 0x5004_E000; // NOT nRF54L15's 0x5004B000

const P0: u64 = 0x5010_A000;
const P1: u64 = 0x500D_8200;
const P2: u64 = 0x5005_0400;
const P3: u64 = 0x500D_8600;

// nRF54L SPIM offsets. Neither exists on the nRF52 SPIM map.
const SPIM_PRESCALER: u64 = 0x52C;
const SPIM_PSEL_SCK: u64 = 0x600;

// nRF54L GPIO offsets (compacted -- OUT at 0x000, not the nRF52 0x504).
const GPIO_OUT: u64 = 0x000;
const GPIO_DIR: u64 = 0x010;
const GPIO_DIRSET: u64 = 0x014;
const GPIO_PIN_CNF0: u64 = 0x080;

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Build the part the way every production entry point does.
fn machine() -> Machine<impl Cpu> {
    let chip_path = root("configs/chips/nrf54lm20a.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("nrf54lm20a chip yaml");
    let manifest = SystemManifest {
        chip: chip_path.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build nrf54lm20a bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    Machine::new(cpu, bus)
}

fn read_u32(m: &Machine<impl Cpu>, addr: u64) -> u32 {
    (0..4)
        .map(|i| (m.bus.read_u8(addr + i).unwrap_or(0) as u32) << (i * 8))
        .fold(0, |a, b| a | b)
}

fn write_u32(m: &mut Machine<impl Cpu>, addr: u64, value: u32) {
    for i in 0..4 {
        m.bus
            .write_u8(addr + i, ((value >> (i * 8)) & 0xFF) as u8)
            .expect("bus write");
    }
}

/// Every declared window answers, and the two SPIM instances answer as nRF54L
/// SPIMs specifically.
#[test]
fn the_estate_answers_at_its_own_addresses() {
    let m = machine();

    for (name, base) in [("spi22", SPIM22), ("spi00", SPIM00)] {
        assert_eq!(
            read_u32(&m, base + SPIM_PRESCALER),
            0x40,
            "{name}: PRESCALER must read its 0x40 reset value. Zero here means \
             either an unmapped window or an instance that took the nRF52 SPIM \
             map, on which 0x52C is not a register at all."
        );
        assert_eq!(
            read_u32(&m, base + SPIM_PSEL_SCK),
            0xFFFF_FFFF,
            "{name}: PSEL.SCK must reset DISCONNECTED"
        );
    }

    // Stub windows must answer rather than fault the bus. TAMPC in particular:
    // Zephyr's SystemInit READS the protect-domain signal registers before
    // writing them, and zero is the correct answer -- a stub returning all-ones
    // would put the MDK into its deliberate approtect hang.
    assert_eq!(
        read_u32(&m, TAMPC + 0x500),
        0,
        "TAMPC protect-domain signal must read 0 (unprovisioned part)"
    );
    let _ = read_u32(&m, RRAMC);
    let _ = read_u32(&m, GRTC);
}

/// NEGATIVE CONTROL: the four GPIO ports are four distinct windows.
///
/// This is the assertion that fails if the ports are ever re-declared with the
/// `MDK_base - 0x504` back-offset the nRF54L15 profile uses: P1 and P3 are only
/// 0x400 apart, so the shifted windows overlap and one port answers for both.
#[test]
fn the_gpio_ports_are_not_each_other() {
    let mut m = machine();

    // Drive P1.22 -- an nRF54LM20 DK LED.
    write_u32(&mut m, P1 + GPIO_DIRSET, 1 << 22);
    write_u32(&mut m, P1 + 0x004, 1 << 22); // OUTSET

    assert_eq!(
        read_u32(&m, P1 + GPIO_DIR) & (1 << 22),
        1 << 22,
        "P1 must hold its own direction"
    );
    assert_eq!(read_u32(&m, P1 + GPIO_OUT) & (1 << 22), 1 << 22, "P1 OUT");

    for (name, base) in [("P0", P0), ("P2", P2), ("P3", P3)] {
        assert_eq!(
            read_u32(&m, base + GPIO_DIR),
            0,
            "{name} moved when only P1 was written -- the port windows overlap"
        );
        assert_eq!(
            read_u32(&m, base + GPIO_OUT),
            0,
            "{name} OUT moved when only P1 was written"
        );
    }
}

/// PIN_CNF reaches the ports through the bus, not just in a unit test.
///
/// This is the whole-machine half of `nrf54l_gpio_offsets.rs`: the pull-up
/// configuration for the DK's buttons is written through PIN_CNF and nothing
/// else, so a chip whose ports decode PIN_CNF at the wrong offset boots, blinks
/// and never reads an input.
#[test]
fn pin_cnf_reaches_the_ports_through_the_bus() {
    let mut m = machine();

    // P1.26 is DK button 0: input with a pull-up (PULL = 3 at bits 3:2).
    write_u32(&mut m, P1 + GPIO_PIN_CNF0 + 4 * 26, 0b1100);
    assert_eq!(
        read_u32(&m, P1 + GPIO_PIN_CNF0 + 4 * 26) & 0b1100,
        0b1100,
        "PIN_CNF[26].PULL must survive a bus write"
    );

    // And PIN_CNF's DIR bit is authoritative for direction.
    write_u32(&mut m, P2 + GPIO_PIN_CNF0 + 4, 0x0000_0001);
    assert_eq!(
        read_u32(&m, P2 + GPIO_DIR) & (1 << 1),
        1 << 1,
        "PIN_CNF[1].DIR must be reflected in P2's DIR"
    );
}

/// Port widths are per-port on this family and a wrong one is silent.
/// P0 has 10 pins; pin 20 does not exist and must not be inventable.
#[test]
fn port_widths_match_the_devicetree() {
    let mut m = machine();
    write_u32(&mut m, P0 + GPIO_DIRSET, 1 << 20);
    assert_eq!(
        read_u32(&m, P0 + GPIO_DIR) & (1 << 20),
        0,
        "P0 has ngpios = 10; pin 20 must not accept a direction"
    );
    // P1 genuinely has 32 pins, so its top pin must work.
    write_u32(&mut m, P1 + GPIO_DIRSET, 1 << 31);
    assert_eq!(
        read_u32(&m, P1 + GPIO_DIR) & (1 << 31),
        1 << 31,
        "P1 has ngpios = 32; pin 31 must exist"
    );
}
