// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::rp2350_xip::{Rp2350Qmi, Rp2350XipMaintenance};
use labwired_core::{Bus, Peripheral, SimulationError};

fn bus(chip: &str) -> SystemBus {
    let path = root(&format!("configs/chips/{chip}.yaml"));
    let chip = ChipDescriptor::from_file(&path).unwrap();
    let mut system = SystemManifest::from_file(root("configs/systems/rp2350-zero.yaml")).unwrap();
    system.chip = path.to_str().unwrap().into();
    SystemBus::from_config(&chip, &system).unwrap()
}

#[test]
fn maintenance_accepts_all_clean_and_invalidate_operations_but_not_pinning() {
    let mut m = Rp2350XipMaintenance;
    for line in [0, 0x1000, 0x3000, 0x03ff_c000, 0x03ff_fff8] {
        for op in 0..=3 {
            m.write(line + op, 0xab).unwrap();
        }
        assert!(matches!(
            m.write(line + 7, 0),
            Err(SimulationError::NotImplemented(_))
        ));
        for op in 4..=6 {
            assert!(m.write(line + op, 0).is_err());
        }
    }
    assert!(m.write(0x0400_0000, 0).is_err());
    assert!(m.read(0).is_err());
}

#[test]
fn sdk_whole_cache_clean_and_invalidate_preserve_flash_bytes_on_all_variants() {
    for chip in ["rp2350", "rp2350-feather", "rp2354b"] {
        let mut b = bus(chip);
        b.flash.data[0x1234] = 0xa5;
        for line in (0x1bff_c000..0x1c00_0000).step_by(8) {
            b.write_u8(line + 1, 0).unwrap();
            b.write_u8(line, 0).unwrap();
        }
        assert_eq!(b.read_u8(0x1000_1234).unwrap(), 0xa5);
        // A word maintenance write is one operation. The low address region
        // must not be folded into RP2040 XIP_SSI's SET/CLR/XOR alias.
        b.write_u32(0x1800_3000, 0xffff_ffff).unwrap();
        assert!(b.atomic_alias_redirect(0x1800_3000).is_none());
        assert!(b.write_u8(0x1800_0007, 0).is_err());
        assert!(b.write_u8(0x1c00_0000, 0).is_err());
    }
}

#[test]
fn rp2040_ssi_atomic_alias_is_retained() {
    let path = root("configs/chips/rp2040.yaml");
    let chip = ChipDescriptor::from_file(&path).unwrap();
    let mut system = SystemManifest::from_file(root("configs/systems/rp2040-pico.yaml")).unwrap();
    system.chip = path.to_str().unwrap().into();
    let mut b = SystemBus::from_config(&chip, &system).unwrap();
    b.write_u32(0x1800_0008, 1).unwrap();
    b.write_u32(0x1800_3008, 1).unwrap();
    assert_eq!(b.read_u32(0x1800_0008).unwrap(), 0);
}

#[test]
fn qmi_cs1_save_restore_and_reset_follow_native_register_map() {
    let mut q = Rp2350Qmi::new();
    assert_eq!(q.read_u32(0x20).unwrap(), 0x4000_0004);
    assert_eq!(q.read_u32(0x24).unwrap(), 0x1000);
    assert_eq!(q.read_u32(0x28).unwrap(), 0xa003);
    for (reg, value) in [(0x20, 0x4000_0103), (0x24, 0x0000_33aa), (0x28, 0xa0eb)] {
        q.write_u32(reg, value).unwrap();
        let saved = q.read_u32(reg).unwrap();
        q.write_u32(reg, 0).unwrap();
        q.write_u32(reg, saved).unwrap();
        assert_eq!(q.read_u32(reg).unwrap(), saved);
    }
    assert_eq!(
        q.read_u32(0x0c).unwrap(),
        0x4000_0004,
        "CS0 remains separate"
    );
    q.on_system_reset();
    assert_eq!(q.read_u32(0x20).unwrap(), 0x4000_0004);
}

#[test]
fn qmi_rejects_direct_transfers_and_nonidentity_translation_without_state_change() {
    let mut q = Rp2350Qmi::new();
    let csr = q.read_u32(0).unwrap();
    for write in [(0, 1), (0, 4), (0, 0x80), (4, 0x9f)] {
        assert!(matches!(
            q.write_u32(write.0, write.1),
            Err(SimulationError::NotImplemented(_))
        ));
    }
    assert_eq!(q.read_u32(0).unwrap(), csr);
    for i in 0..8 {
        let reg = 0x34 + i * 4;
        let identity = 0x0400_0000 | ((i as u32 & 3) << 10);
        assert_eq!(q.read_u32(reg).unwrap(), identity);
        q.write_u32(reg, identity).unwrap();
        assert!(q.write_u32(reg, identity ^ 1).is_err());
        assert_eq!(q.read_u32(reg).unwrap(), identity);
    }
    assert!(q.read_u32(8).is_err());
    assert!(q.read_u32(0x54).is_err());
}

#[test]
fn chip_qmi_window_keeps_cs1_registers_and_does_not_alias_bootram() {
    for chip in ["rp2350", "rp2354b"] {
        let mut b = bus(chip);
        b.write_u32(0x400d_0020, 0x4000_0302).unwrap();
        assert_eq!(b.read_u32(0x400d_0020).unwrap(), 0x4000_0302);
        assert_eq!(b.read_u16(0x400e_0000).unwrap(), 0x4770);
    }
}
