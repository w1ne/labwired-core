// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! The RP2350's Cortex-M33 CoreDebug block (Armv8-M ARM, D1.2: DHCSR
//! 0xE000EDF0, DCRSR, DCRDR, DEMCR 0xE000EDFC) is part of the core and is
//! reachable from software with no debugger attached. Firmware that runs a
//! DWT cycle counter sets DEMCR.TRCENA first (Betaflight's
//! `cycleCounterInit`, CMSIS `CoreDebug->DEMCR |= TRCENA`); unmapped, that
//! store was a precise BusFault.
//!
//! Without a probe, software cannot enable halting debug: DHCSR.C_DEBUGEN is
//! set only by the debugger, and the other control bits act only while it is
//! set, so a software DHCSR write changes nothing.

use crate::common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;

const DHCSR: u64 = 0xE000_EDF0;
const DEMCR: u64 = 0xE000_EDFC;
const TRCENA: u32 = 1 << 24;

fn bus(chip_rel: &str, system_rel: &str) -> SystemBus {
    let chip_path = root(chip_rel);
    let chip = ChipDescriptor::from_file(&chip_path).expect("chip");
    let mut manifest = SystemManifest::from_file(root(system_rel)).expect("system");
    manifest.chip = chip_path.to_str().unwrap().to_string();
    let mut b = SystemBus::from_config(&chip, &manifest).expect("bus");
    let _ = configure_cortex_m(&mut b);
    b
}

#[test]
fn demcr_trcena_is_a_plain_store_without_a_probe() {
    let mut b = bus(
        "configs/chips/rp2350.yaml",
        "configs/systems/rp2350-zero.yaml",
    );
    let v = b.read_u32(DEMCR).expect("DEMCR is mapped");
    b.write_u32(DEMCR, v | TRCENA).expect("DEMCR write");
    assert_eq!(b.read_u32(DEMCR).unwrap() & TRCENA, TRCENA);
}

#[test]
fn software_cannot_enable_halting_debug() {
    let mut b = bus(
        "configs/chips/rp2350.yaml",
        "configs/systems/rp2350-zero.yaml",
    );
    // DBGKEY | C_HALT | C_DEBUGEN from software: ignored.
    b.write_u32(DHCSR, 0xA05F_0003).unwrap();
    assert_eq!(b.read_u32(DHCSR).unwrap(), 0x0001_0000, "S_REGRDY only");
}
