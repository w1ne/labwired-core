// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Real STM32L4 HAL reads UID_BASE in ordinary board firmware. The factory
//! identity window must not BusFault, become writable, or map adjacent OTP.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

const UID: u64 = 0x1fff_7590;

fn bus() -> SystemBus {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32l476.yaml");
    let chip = ChipDescriptor::from_file(&path).unwrap();
    let manifest = SystemManifest {
        name: "stm32l476-uid-contract".into(),
        chip: path.to_string_lossy().into_owned(),
        ..Default::default()
    };
    SystemBus::from_config(&chip, &manifest).unwrap()
}

#[test]
fn uid_is_twelve_read_only_bytes_with_explicit_unspecified_identity() {
    let mut bus = bus();
    for offset in [0, 4, 8] {
        // A zero fixture is documented; it is not asserted as silicon data.
        assert_eq!(bus.read_u32(UID + offset).unwrap(), 0);
        bus.write_u32(UID + offset, 0x1234_5678).unwrap();
        assert_eq!(bus.read_u32(UID + offset).unwrap(), 0);
    }
    assert_eq!(bus.read_u8(UID + 11).unwrap(), 0);
    assert!(bus.read_u32(UID - 4).is_err(), "do not invent adjacent OTP");
    assert!(
        bus.read_u32(UID + 12).is_err(),
        "map only the 96-bit identifier"
    );
}
