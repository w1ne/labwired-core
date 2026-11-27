// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF54L15 FICR serves the SVD register file, not a zero-filled window.
//!
//! Source: Nordic `nrf54l15.svd` (vendored at
//! `tests/fixtures/real_world/nrf54l15.svd`), peripheral `GLOBAL_FICR_NS` at
//! 0x00FF_C000. The SVD gives every FICR word the reset value 0xFFFFFFFF
//! ("Unspecified") and enumerates the values for this part: INFO.PART =
//! N54L15, INFO.RAM = K256, INFO.RRAM = K1524. The test reads those values
//! from the SVD text, so a descriptor that drifts from the vendor file fails.
//!
//! The erased words matter to the boot path: nrfx `SystemInit`
//! (`system_nrf54l.c`) copies `TRIMCNF[n].DATA` to `*TRIMCNF[n].ADDR` until
//! an ADDR reads 0xFFFFFFFF or 0, so an erased table stops the copy at
//! entry 0.

use labwired_config::ChipDescriptor;
use labwired_core::bus::SystemBus;
use labwired_core::Bus;

const FICR: u64 = 0x00FF_C000;
// SVD cluster INFO at 0x300; register offsets inside the cluster.
const INFO_DEVICEID0: u64 = 0x304;
const INFO_DEVICEID1: u64 = 0x308;
const INFO_PART: u64 = 0x31C;
const INFO_VARIANT: u64 = 0x320;
const INFO_RAM: u64 = 0x328;
const INFO_RRAM: u64 = 0x32C;
const TRIMCNF0_ADDR: u64 = 0x400;

fn root(rel: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Value of the SVD `<enumeratedValue>` named `name` (unique in this SVD).
fn svd_enum(svd: &str, name: &str) -> u32 {
    let at = svd
        .find(&format!("<name>{name}</name>"))
        .unwrap_or_else(|| panic!("SVD has no enumerated value {name}"));
    let rest = &svd[at..];
    let v0 = rest.find("<value>").expect("value tag") + "<value>".len();
    let v1 = rest[v0..].find("</value>").expect("value close") + v0;
    let text = rest[v0..v1].trim();
    u32::from_str_radix(text.trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("{name}: bad value {text}: {e}"))
}

fn nrf54l15_bus() -> (ChipDescriptor, SystemBus) {
    let path = root("configs/chips/nrf54l15.yaml");
    let chip = ChipDescriptor::from_file(&path).expect("load chip");
    let manifest = labwired_config::SystemManifest {
        max_peripheral_tick_interval: None,
        parts: Vec::new(),
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "nrf54l15-ficr".to_string(),
        chip: path.to_string_lossy().to_string(),
        cpu_hz: None,
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    let bus = SystemBus::from_config(&chip, &manifest).expect("assemble nrf54l15 bus");
    (chip, bus)
}

#[test]
fn ficr_info_words_match_the_svd_and_the_memory_map() {
    let svd = std::fs::read_to_string(root("tests/fixtures/real_world/nrf54l15.svd"))
        .expect("read nrf54l15.svd");
    assert!(
        svd.contains("<name>GLOBAL_FICR_NS</name>") && svd.contains("<baseAddress>0x00FFC000"),
        "SVD FICR base moved"
    );
    let (chip, bus) = nrf54l15_bus();
    let read = |off: u64| bus.read_u32(FICR + off).expect("FICR read");

    assert_eq!(read(INFO_PART), svd_enum(&svd, "N54L15"), "INFO.PART");
    assert_eq!(read(INFO_RAM), svd_enum(&svd, "K256"), "INFO.RAM");
    assert_eq!(read(INFO_RRAM), svd_enum(&svd, "K1524"), "INFO.RRAM");

    // INFO.RAM / INFO.RRAM count KiB; they must agree with the chip memory map.
    assert_eq!(u64::from(read(INFO_RAM)) * 1024, chip.ram.size, "RAM size");
    assert_eq!(
        u64::from(read(INFO_RRAM)) * 1024,
        chip.flash.size,
        "RRAM size"
    );
}

#[test]
fn ficr_unspecified_words_read_erased_and_stop_the_trim_copy() {
    let (_, bus) = nrf54l15_bus();
    let read = |off: u64| bus.read_u32(FICR + off).expect("FICR read");

    // No per-device identity or variant is invented.
    for (off, name) in [
        (INFO_DEVICEID0, "DEVICEID[0]"),
        (INFO_DEVICEID1, "DEVICEID[1]"),
        (INFO_VARIANT, "VARIANT"),
    ] {
        assert_eq!(read(off), 0xFFFF_FFFF, "{name} must read erased");
    }
    // nrfx SystemInit stops the TRIMCNF copy loop on ADDR == 0xFFFFFFFF.
    assert_eq!(read(TRIMCNF0_ADDR), 0xFFFF_FFFF, "TRIMCNF[0].ADDR");
}

#[test]
fn ficr_info_part_is_read_only() {
    let (_, mut bus) = nrf54l15_bus();
    let part = bus.read_u32(FICR + INFO_PART).expect("read");
    let _ = bus.write_u32(FICR + INFO_PART, 0xA5A5_A5A5);
    assert_eq!(bus.read_u32(FICR + INFO_PART).expect("read"), part);
}
