// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF5340 FICR serves its vendor SVD reset values instead of a zero window.
//!
//! Source: Nordic `nrf5340.svd` (vendored at
//! `tests/fixtures/real_world/nrf5340.svd`), peripheral `FICR_S` at
//! 0x00FF0000. The SVD identifies INFO.PART as N5340 and supplies the flash
//! page geometry. Per-device ID, RAM/FLASH variant and trim values remain
//! unspecified in the SVD and are not invented here.

use labwired_config::ChipDescriptor;
use labwired_core::bus::SystemBus;
use labwired_core::Bus;

const FICR: u64 = 0x00FF_0000;
const INFO_DEVICEID0: u64 = 0x204;
const INFO_DEVICEID1: u64 = 0x208;
const INFO_PART: u64 = 0x20C;
const INFO_VARIANT: u64 = 0x210;
const INFO_PACKAGE: u64 = 0x214;
const INFO_RAM: u64 = 0x218;
const INFO_FLASH: u64 = 0x21C;
const INFO_CODEPAGESIZE: u64 = 0x220;
const INFO_CODESIZE: u64 = 0x224;
const INFO_DEVICETYPE: u64 = 0x228;
const TRIMCNF0_ADDR: u64 = 0x300;

fn root(rel: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn nrf5340_bus() -> (ChipDescriptor, SystemBus) {
    let path = root("configs/chips/nrf5340.yaml");
    let chip = ChipDescriptor::from_file(&path).expect("load nrf5340 chip");
    let manifest = labwired_config::SystemManifest {
        max_peripheral_tick_interval: None,
        parts: Vec::new(),
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "nrf5340-ficr".to_string(),
        chip: path.to_string_lossy().to_string(),
        cpu_hz: None,
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    let bus = SystemBus::from_config(&chip, &manifest).expect("assemble nrf5340 bus");
    (chip, bus)
}

fn svd_peripheral<'a>(svd: &'a str, name: &str) -> &'a str {
    let marker = format!("<name>{name}</name>");
    let name_at = svd
        .find(&marker)
        .unwrap_or_else(|| panic!("SVD has no peripheral {name}"));
    let start = svd[..name_at]
        .rfind("<peripheral>")
        .expect("peripheral start");
    let tail = &svd[name_at..];
    let end = tail.find("</peripheral>").expect("peripheral close") + name_at;
    &svd[start..end + "</peripheral>".len()]
}

fn svd_register_reset(peripheral: &str, name: &str) -> u32 {
    let marker = format!("<name>{name}</name>");
    let at = peripheral
        .find(&marker)
        .unwrap_or_else(|| panic!("FICR SVD has no register {name}"));
    let rest = &peripheral[at + marker.len()..];
    let start = rest.find("<resetValue>").expect("register reset value") + "<resetValue>".len();
    let end = rest[start..].find("</resetValue>").expect("reset close") + start;
    let value = rest[start..end].trim();
    u32::from_str_radix(value.trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("bad {name} reset value {value}: {e}"))
}

fn svd_cluster_offset(peripheral: &str, name: &str) -> u64 {
    let marker = format!("<name>{name}</name>");
    let at = peripheral
        .find(&marker)
        .unwrap_or_else(|| panic!("FICR SVD has no cluster {name}"));
    let rest = &peripheral[at + marker.len()..];
    let start = rest
        .find("<addressOffset>")
        .expect("cluster address offset")
        + "<addressOffset>".len();
    let end = rest[start..]
        .find("</addressOffset>")
        .expect("offset close")
        + start;
    let value = rest[start..end].trim();
    u64::from_str_radix(value.trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("bad {name} cluster offset {value}: {e}"))
}

fn svd_register_offset(peripheral: &str, name: &str) -> u64 {
    let marker = format!("<name>{name}</name>");
    let at = peripheral
        .find(&marker)
        .unwrap_or_else(|| panic!("FICR SVD has no register {name}"));
    let rest = &peripheral[at + marker.len()..];
    let start = rest
        .find("<addressOffset>")
        .expect("register address offset")
        + "<addressOffset>".len();
    let end = rest[start..]
        .find("</addressOffset>")
        .expect("offset close")
        + start;
    let value = rest[start..end].trim();
    u64::from_str_radix(value.trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("bad {name} register offset {value}: {e}"))
}

fn svd_register_dim_increment(peripheral: &str, name: &str) -> u64 {
    let marker = format!("<name>{name}</name>");
    let name_at = peripheral
        .find(&marker)
        .unwrap_or_else(|| panic!("FICR SVD has no register {name}"));
    let register_start = peripheral[..name_at]
        .rfind("<register>")
        .expect("register start");
    let rest = &peripheral[register_start..];
    let start = rest
        .find("<dimIncrement>")
        .expect("register dimension increment")
        + "<dimIncrement>".len();
    let end = rest[start..]
        .find("</dimIncrement>")
        .expect("dimension increment close")
        + start;
    let value = rest[start..end].trim();
    u64::from_str_radix(value.trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("bad {name} dimension increment {value}: {e}"))
}

fn svd_enum(svd: &str, name: &str) -> u32 {
    let at = svd
        .find(&format!("<name>{name}</name>"))
        .unwrap_or_else(|| panic!("SVD has no enumerated value {name}"));
    let rest = &svd[at..];
    let start = rest.find("<value>").expect("enum value") + "<value>".len();
    let end = rest[start..].find("</value>").expect("enum close") + start;
    let value = rest[start..end].trim();
    u32::from_str_radix(value.trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("bad {name} enum value {value}: {e}"))
}

#[test]
fn ficr_device_fields_match_the_vendored_svd() {
    let svd = std::fs::read_to_string(root("tests/fixtures/real_world/nrf5340.svd"))
        .expect("read nrf5340.svd");
    let ficr_svd = svd_peripheral(&svd, "FICR_S");
    assert!(ficr_svd.contains("<baseAddress>0x00FF0000</baseAddress>"));

    let (chip, bus) = nrf5340_bus();
    let read = |offset: u64| bus.read_u32(FICR + offset).expect("FICR read");
    let info_base = svd_cluster_offset(ficr_svd, "INFO");
    for (offset, register) in [
        (INFO_PART, "PART"),
        (INFO_CODEPAGESIZE, "CODEPAGESIZE"),
        (INFO_CODESIZE, "CODESIZE"),
        (INFO_DEVICETYPE, "DEVICETYPE"),
    ] {
        assert_eq!(
            offset,
            info_base + svd_register_offset(ficr_svd, register),
            "{register} SVD offset"
        );
        assert_eq!(
            read(offset),
            svd_register_reset(ficr_svd, register),
            "{register}"
        );
    }
    assert_eq!(read(INFO_PART), svd_enum(&svd, "N5340"), "INFO.PART enum");

    // The 256 4-KiB pages described by the SVD match this chip profile's flash.
    assert_eq!(
        u64::from(read(INFO_CODEPAGESIZE)) * u64::from(read(INFO_CODESIZE)),
        chip.flash.size,
        "INFO flash geometry"
    );
}

#[test]
fn ficr_unspecified_identity_variants_and_trim_table_remain_erased() {
    let svd = std::fs::read_to_string(root("tests/fixtures/real_world/nrf5340.svd"))
        .expect("read nrf5340.svd");
    let ficr_svd = svd_peripheral(&svd, "FICR_S");
    let info_base = svd_cluster_offset(ficr_svd, "INFO");
    let (_, bus) = nrf5340_bus();
    let read = |offset: u64| bus.read_u32(FICR + offset).expect("FICR read");

    for (offset, register) in [
        (INFO_VARIANT, "VARIANT"),
        (INFO_PACKAGE, "PACKAGE"),
        (INFO_RAM, "RAM"),
        (INFO_FLASH, "FLASH"),
    ] {
        assert_eq!(
            offset,
            info_base + svd_register_offset(ficr_svd, register),
            "{register} SVD offset"
        );
        assert_eq!(
            read(offset),
            svd_register_reset(ficr_svd, register),
            "{register} stays at the SVD unspecified value"
        );
    }
    let device_id_reset = svd_register_reset(ficr_svd, "DEVICEID[%s]");
    let device_id_base = info_base + svd_register_offset(ficr_svd, "DEVICEID[%s]");
    let device_id_increment = svd_register_dim_increment(ficr_svd, "DEVICEID[%s]");
    assert_eq!(INFO_DEVICEID0, device_id_base, "DEVICEID[0] SVD offset");
    assert_eq!(
        INFO_DEVICEID1,
        device_id_base + device_id_increment,
        "DEVICEID[1] SVD offset"
    );
    assert_eq!(read(INFO_DEVICEID0), device_id_reset, "DEVICEID[0]");
    assert_eq!(read(INFO_DEVICEID1), device_id_reset, "DEVICEID[1]");

    // The SVD models TRIMCNF as a 32-entry cluster with ADDR at +0 and DATA at
    // +4. Its first address is erased, so the simulator does not invent trims.
    assert!(ficr_svd.contains("<name>TRIMCNF[%s]</name>"));
    let trim_base = svd_cluster_offset(ficr_svd, "TRIMCNF[%s]");
    assert_eq!(
        TRIMCNF0_ADDR,
        trim_base + svd_register_offset(ficr_svd, "ADDR")
    );
    assert_eq!(
        read(TRIMCNF0_ADDR),
        svd_register_reset(ficr_svd, "ADDR"),
        "TRIMCNF[0].ADDR"
    );
}

#[test]
fn ficr_part_is_read_only() {
    let (_, mut bus) = nrf5340_bus();
    let part = bus.read_u32(FICR + INFO_PART).expect("read INFO.PART");
    let _ = bus.write_u32(FICR + INFO_PART, 0xA5A5_A5A5);
    assert_eq!(
        bus.read_u32(FICR + INFO_PART).expect("read INFO.PART"),
        part
    );
}
