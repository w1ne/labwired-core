// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF5340 UICR reads as an unprogrammed (erased) device, not a zero window.
//!
//! Source: Nordic `nrf5340.svd` (vendored at
//! `tests/fixtures/real_world/nrf5340.svd`), peripheral `UICR_S` at
//! 0x00FF8000. UICR is NVM; an erased word reads 0xFFFFFFFF. The SVD gives
//! APPROTECT, SECUREAPPROTECT and ERASEPROTECT a reset value of 0, which its
//! own enums decode as "Protected", so the chip profile overrides those three
//! to the erased value.

use labwired_config::ChipDescriptor;
use labwired_core::bus::SystemBus;
use labwired_core::Bus;

const UICR: u64 = 0x00FF_8000;
const ERASED: u32 = 0xFFFF_FFFF;

fn root(rel: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn manifest(chip: &std::path::Path) -> labwired_config::SystemManifest {
    labwired_config::SystemManifest {
        max_peripheral_tick_interval: None,
        parts: Vec::new(),
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "nrf5340-uicr".to_string(),
        chip: chip.to_string_lossy().to_string(),
        cpu_hz: None,
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    }
}

fn chip() -> (std::path::PathBuf, ChipDescriptor) {
    let path = root("configs/chips/nrf5340.yaml");
    let chip = ChipDescriptor::from_file(&path).expect("load nrf5340 chip");
    (path, chip)
}

fn nrf5340_bus() -> SystemBus {
    let (path, chip) = chip();
    SystemBus::from_config(&chip, &manifest(&path)).expect("assemble nrf5340 bus")
}

fn uicr_svd() -> String {
    let svd = std::fs::read_to_string(root("tests/fixtures/real_world/nrf5340.svd"))
        .expect("read nrf5340.svd");
    let name_at = svd.find("<name>UICR_S</name>").expect("SVD has UICR_S");
    let start = svd[..name_at]
        .rfind("<peripheral>")
        .expect("peripheral start");
    let end = svd[name_at..]
        .find("</peripheral>")
        .expect("peripheral close")
        + name_at;
    svd[start..end].to_string()
}

/// Text of the first `<tag>` after the `<name>{register}</name>` marker.
fn svd_field(peripheral: &str, register: &str, tag: &str) -> u32 {
    let marker = format!("<name>{register}</name>");
    let at = peripheral
        .find(&marker)
        .unwrap_or_else(|| panic!("UICR SVD has no {register}"));
    let rest = &peripheral[at + marker.len()..];
    let open = format!("<{tag}>");
    let start = rest.find(&open).expect("tag") + open.len();
    let end = rest[start..].find(&format!("</{tag}>")).expect("tag close") + start;
    let value = rest[start..end].trim();
    u32::from_str_radix(value.trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("bad {register} {tag} {value}: {e}"))
}

/// The `<value>` of enum `name` inside register `register`.
fn svd_enum(peripheral: &str, register: &str, name: &str) -> u32 {
    let at = peripheral
        .find(&format!("<name>{register}</name>"))
        .unwrap_or_else(|| panic!("UICR SVD has no {register}"));
    let rest = &peripheral[at..];
    let end = rest.find("</register>").expect("register close");
    svd_field(&rest[..end], name, "value")
}

#[test]
fn uicr_protect_words_read_erased_not_the_svd_zero() {
    let svd = uicr_svd();
    assert!(svd.contains("<baseAddress>0x00FF8000</baseAddress>"));
    let bus = nrf5340_bus();
    for register in ["APPROTECT", "SECUREAPPROTECT", "ERASEPROTECT"] {
        let offset = u64::from(svd_field(&svd, register, "addressOffset"));
        // The SVD reset value is 0 ("Protected"); erased NVM is all ones.
        assert_eq!(
            svd_field(&svd, register, "resetValue"),
            0,
            "{register} SVD reset"
        );
        assert_eq!(
            svd_enum(&svd, register, "Protected"),
            0,
            "{register} Protected"
        );
        assert_eq!(
            bus.read_u32(UICR + offset).expect("UICR read"),
            ERASED,
            "{register} reads erased"
        );
    }
    // Decode against the SVD enums: an erased part has the debugger access
    // ports protected (anything but 0x50FA50FA) and ERASEALL unprotected.
    let read = |r: &str| {
        let offset = u64::from(svd_field(&svd, r, "addressOffset"));
        bus.read_u32(UICR + offset).expect("UICR read")
    };
    assert_ne!(
        read("APPROTECT"),
        svd_enum(&svd, "APPROTECT", "Unprotected")
    );
    assert_ne!(
        read("SECUREAPPROTECT"),
        svd_enum(&svd, "SECUREAPPROTECT", "Unprotected")
    );
    assert_eq!(
        read("ERASEPROTECT"),
        svd_enum(&svd, "ERASEPROTECT", "Unprotected")
    );
}

#[test]
fn uicr_config_otp_and_keyslot_words_read_erased() {
    let svd = uicr_svd();
    let bus = nrf5340_bus();
    for register in ["VREGHVOUT", "HFXOCNT", "TINSTANCE", "NFCPINS"] {
        let offset = u64::from(svd_field(&svd, register, "addressOffset"));
        assert_eq!(
            svd_field(&svd, register, "resetValue"),
            ERASED,
            "{register} SVD"
        );
        assert_eq!(
            bus.read_u32(UICR + offset).expect("UICR read"),
            ERASED,
            "{register}"
        );
    }
    // OTP[0..192] at 0x100 and the KEYSLOT block 0x400..0x1000 (128 CONFIG
    // pairs + 128 four-word keys): first and last words of each.
    let otp = u64::from(svd_field(&svd, "OTP[%s]", "addressOffset"));
    assert_eq!(otp, 0x100);
    let keyslot = u64::from(svd_field(&svd, "KEYSLOT", "addressOffset"));
    assert_eq!(keyslot, 0x400);
    for offset in [otp, otp + 191 * 4, keyslot, 0x7FC, 0x800, 0xFFC] {
        assert_eq!(
            bus.read_u32(UICR + offset).expect("UICR read"),
            ERASED,
            "UICR +{offset:#05x}"
        );
    }
}

#[test]
fn declarative_reset_override_rejects_an_unknown_register() {
    let (path, mut chip) = chip();
    let uicr = chip
        .peripherals
        .iter_mut()
        .find(|p| p.id == "uicr")
        .expect("uicr peripheral");
    let overrides = uicr
        .config
        .get_mut("reset_overrides")
        .and_then(|v| v.as_mapping_mut())
        .expect("uicr reset_overrides");
    overrides.insert("NO_SUCH_REGISTER".into(), 0u64.into());
    let err = match SystemBus::from_config(&chip, &manifest(&path)) {
        Ok(_) => panic!("unknown register id must fail"),
        Err(err) => format!("{err:#}"),
    };
    assert!(err.contains("NO_SUCH_REGISTER"), "{err}");
}
