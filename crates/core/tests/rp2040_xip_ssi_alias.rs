// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RP2040 XIP_SSI (`0x1800_0000`) takes the atomic SET/CLR register aliases.
//!
//! Stock MicroPython (v1.29.0 RPI_PICO) clears SSIENR from RAM through the CLR
//! alias `0x1800_3008`. The bus only decoded aliases in the `0x4000_0000`
//! peripheral window, so that write was an unmapped BusFault before
//! MicroPython printed anything.

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::Bus;

const SSIENR: u64 = 0x1800_0008;
const SET: u64 = 0x2000;
const CLR: u64 = 0x3000;

#[test]
fn xip_ssi_ssienr_set_and_clr_aliases_reach_the_register() {
    let chip_path = root("configs/chips/rp2040.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("rp2040 chip");
    let mut manifest =
        SystemManifest::from_file(&root("configs/systems/rp2040-pico.yaml")).expect("pico");
    manifest.chip = chip_path.to_str().expect("utf8").to_string();
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("bus");

    bus.write_u32(SSIENR + SET, 1).expect("SET alias of SSIENR");
    assert_eq!(
        bus.read_u32(SSIENR).unwrap() & 1,
        1,
        "SET alias enables the SSI"
    );
    bus.write_u32(SSIENR + CLR, 1)
        .expect("CLR alias of SSIENR (MicroPython)");
    assert_eq!(
        bus.read_u32(SSIENR).unwrap() & 1,
        0,
        "CLR alias disables the SSI"
    );
}
