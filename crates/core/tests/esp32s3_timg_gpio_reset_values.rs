// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! The ESP32-S3 `TIMG0` and `GPIO` descriptors must carry the reset values the
//! **S3's own reference manual** states, and the sibling Espressif parts must
//! keep theirs.
//!
//! `configs/peripherals/esp32s3/timg0.yaml` and `.../gpio.yaml` say at the top
//! that they are "Based on ESP32-S3 Technical Reference Manual". Nine of their
//! reset values are not in that manual, and not in any Espressif part:
//!
//! | register | modelled | ESP32-S3 TRM v1.8 |
//! |---|---|---|
//! | `TIMG0.T0CONFIG`   | `0x60013000` | `0x60002000` (Reg 12.1, p.662) |
//! | `TIMG0.T1CONFIG`   | `0x60013000` | `0x60002000` (Reg 12.1, p.662) |
//! | `TIMG0.WDTCONFIG0` | `0x00048D00` | `0x0004C000` (Reg 12.10, p.665) |
//! | `TIMG0.WDTCONFIG2` | `0x00000000` | `0x018CBA80` (Reg 12.12, p.666) |
//! | `TIMG0.WDTCONFIG3` | `0x00000000` | `0x07FFFFFF` (Reg 12.13, p.666) |
//! | `TIMG0.WDTCONFIG4` | `0x00000000` | `0x000FFFFF` (Reg 12.14, p.666) |
//! | `TIMG0.WDTCONFIG5` | `0x00000000` | `0x000FFFFF` (Reg 12.15, p.667) |
//! | `TIMG0.RTCCALICFG` | `0x00000000` | `0x00013000` (Reg 12.18, p.668) |
//! | `GPIO.REG_DATE`    | `0x02108190` | `0x01907040` (Reg 6.34, p.512) |
//!
//! The TRM prints these per field, so they are reconstructed rather than
//! quoted: `T0CONFIG` is `INCREASE=1`(b30) `AUTORELOAD=1`(b29) `DIVIDER=0x01`
//! (b28:13) = `0x6000_2000`; `WDTCONFIG0` is `CPU_RESET_LENGTH=0x1`(b20:18)
//! `SYS_RESET_LENGTH=0x1`(b17:15) `FLASHBOOT_MOD_EN=1`(b14) = `0x0004_C000`;
//! `RTCCALICFG` is `CALI_MAX=0x01`(b30:16) `CALI_CLK_SEL=0x1`(b14:13)
//! `START_CYCLING=1`(b12) = `0x0001_3000`. The vendor SVD states the same
//! numbers independently, and this gate asserts against the SVD so the check is
//! machine-checkable and re-runnable.
//!
//! These nine are the `esp32s3` half of the 34 model/silicon conflicts that
//! `0036_part_knowledge_svd_promotion.sql` published: the corpus recorded that
//! our model and the vendor disagree, without deciding which was wrong. The TRM
//! decides it — the model was.
//!
//! Why a gate rather than a bare edit: a declarative descriptor's reset value
//! is what firmware reads before it writes anything, and nothing compared these
//! files to a vendor artifact before. `register_coverage` counts *responses*,
//! which a register holding the wrong reset value still produces, and
//! `svd_conformance` checks base addresses and IRQ numbers only.
//!
//! Three chips, deliberately. The ESP32-C3 and the classic ESP32 model the same
//! two peripherals, at register names that are character-for-character
//! identical, and both are correct today. `GPIO.REG_DATE` is the sharpest
//! control available: it is a silicon version stamp, so every part has a
//! *different* right answer — S3 `0x01907040`, C3 `0x02006130`. A test that only
//! proved the S3 now reads `0x01907040` could not tell a fixed S3 from a C3 that
//! had been dragged onto the S3's value.

use std::collections::HashMap;
use std::path::PathBuf;

use labwired_config::PeripheralDescriptor;
use labwired_core::peripherals::declarative::GenericPeripheral;
use labwired_core::Peripheral;

const S3_TIMG0: &str = "configs/peripherals/esp32s3/timg0.yaml";
const S3_GPIO: &str = "configs/peripherals/esp32s3/gpio.yaml";
const C3_TIMG0: &str = "configs/peripherals/esp32c3/timg0.yaml";
const C3_GPIO: &str = "configs/peripherals/esp32c3/gpio.yaml";
const ESP32_TIMG0: &str = "configs/peripherals/esp32/timg0.yaml";

const S3_SVD: &str = "tests/fixtures/svd/esp32s3.svd";
const C3_SVD: &str = "tests/fixtures/real_world/esp32c3.svd";
const ESP32_SVD: &str = "tests/fixtures/real_world/esp32.svd";

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn descriptor(rel: &str) -> PeripheralDescriptor {
    PeripheralDescriptor::from_file(root(rel)).unwrap_or_else(|e| panic!("load {rel}: {e}"))
}

/// One peripheral's register map as the vendor's SVD states it: name →
/// (offset, reset value). Parsed with the same `svd_ingestor` the CLI importer
/// and the coverage scan use, so this gate reads the SVD the way the rest of
/// the repo does — including `<dim>` array expansion, which is how the S3's
/// `T%sCONFIG` becomes `T0CONFIG` and `T1CONFIG`.
fn vendor_map(svd_rel: &str, peripheral: &str) -> HashMap<String, (u64, u32)> {
    let xml =
        std::fs::read_to_string(root(svd_rel)).unwrap_or_else(|e| panic!("read {svd_rel}: {e}"));
    let device = svd_ingestor::parse_svd(&xml).unwrap_or_else(|e| panic!("parse {svd_rel}: {e}"));
    let found = device
        .peripherals
        .iter()
        .find(|p| p.name == peripheral)
        .unwrap_or_else(|| panic!("{svd_rel} has no {peripheral} peripheral"));
    let desc = svd_ingestor::process_peripheral(&device, found)
        .unwrap_or_else(|e| panic!("process {peripheral} from {svd_rel}: {e}"));
    let map: HashMap<String, (u64, u32)> = desc
        .registers
        .iter()
        .map(|r| (r.id.clone(), (r.address_offset, r.reset_value)))
        .collect();
    assert!(
        map.len() >= 20,
        "{svd_rel} yielded only {} {peripheral} registers — the SVD parse has \
         broken and every comparison below would pass vacuously",
        map.len()
    );
    map
}

/// Every register the descriptor declares *that the vendor also names*, checked
/// against the vendor's own map: same offset, same reset value.
///
/// Registers the vendor does not name are skipped rather than failed: the S3
/// `GPIO` model is a 50-register subset chosen by whoever implemented the
/// behaviour, and naming a register the SVD omits is not by itself an error.
fn assert_reset_values_conform(
    descriptor_rel: &str,
    svd_rel: &str,
    peripheral: &str,
    min_shared: usize,
) {
    let desc = descriptor(descriptor_rel);
    let vendor = vendor_map(svd_rel, peripheral);

    let mut findings = Vec::new();
    let mut shared = 0usize;
    for reg in &desc.registers {
        let Some(&(offset, reset)) = vendor.get(&reg.id) else {
            continue;
        };
        shared += 1;
        if reg.address_offset != offset {
            findings.push(format!(
                "{}: modelled at {:#05X}, silicon puts it at {:#05X}",
                reg.id, reg.address_offset, offset
            ));
        }
        if reg.reset_value != reset {
            findings.push(format!(
                "{}: reset {:#010X}, silicon resets to {:#010X}",
                reg.id, reg.reset_value, reset
            ));
        }
    }

    assert!(
        shared >= min_shared,
        "{descriptor_rel} shares only {shared} register names with {svd_rel} \
         (expected at least {min_shared}) — the comparison has gone vacuous"
    );
    assert!(
        findings.is_empty(),
        "{descriptor_rel} disagrees with {svd_rel} ({} findings over {shared} \
         shared registers):\n  {}",
        findings.len(),
        findings.join("\n  ")
    );
}

fn reset_of(rel: &str, register: &str) -> u32 {
    descriptor(rel)
        .registers
        .iter()
        .find(|r| r.id == register)
        .unwrap_or_else(|| panic!("{rel} declares no {register}"))
        .reset_value
}

fn read32(rel: &str, offset: u64) -> u32 {
    GenericPeripheral::new(descriptor(rel))
        .read_u32(offset)
        .expect("read_u32")
}

// ── ESP32-S3: the fix ────────────────────────────────────────────────────────

#[test]
fn esp32s3_timg0_reset_values_match_the_vendor_svd() {
    assert_reset_values_conform(S3_TIMG0, S3_SVD, "TIMG0", 24);

    // The eight the corpus flagged, stated outright so a future reader does not
    // have to re-derive them from the SVD. Right column is the TRM v1.8 value.
    for (register, trm) in [
        ("T0CONFIG", 0x6000_2000u32),
        ("T1CONFIG", 0x6000_2000),
        ("WDTCONFIG0", 0x0004_C000),
        ("WDTCONFIG2", 0x018C_BA80),
        ("WDTCONFIG3", 0x07FF_FFFF),
        ("WDTCONFIG4", 0x000F_FFFF),
        ("WDTCONFIG5", 0x000F_FFFF),
        ("RTCCALICFG", 0x0001_3000),
    ] {
        assert_eq!(
            reset_of(S3_TIMG0, register),
            trm,
            "TIMG0.{register} resets to {trm:#010X} on this silicon \
             (ESP32-S3 TRM v1.8, chapter 12)"
        );
    }
}

#[test]
fn esp32s3_gpio_version_stamp_matches_the_vendor_svd() {
    // 0x01907040 is what GPIO_DATE_REG reads on an S3 (TRM v1.8 Register 6.34,
    // p.512). The modelled 0x02108190 is not this part's stamp and is not any
    // other Espressif part's either — the S2 reads 0x01905061, the C3
    // 0x02006130, the C2 0x02106190. It was invented.
    assert_eq!(
        reset_of(S3_GPIO, "REG_DATE"),
        0x0190_7040,
        "GPIO.REG_DATE is the S3's silicon version stamp"
    );
    // CLOCK_GATE already agreed, and 0036 promoted it to `proven`. Pinned so a
    // sweep over this file cannot quietly take it with the others.
    assert_eq!(reset_of(S3_GPIO, "CLOCK_GATE"), 0x0000_0001);
}

#[test]
fn esp32s3_timg0_answers_reads_with_the_trm_reset_values() {
    // A descriptor edit that never reaches the bus is indistinguishable from no
    // edit: `GenericPeripheral` fabricates a zero for any offset it does not
    // decode, so four of these eight registers would read 0 either way. Drive
    // the peripheral the way the bus does and demand the number.
    for (offset, register, trm) in [
        (0x000u64, "T0CONFIG", 0x6000_2000u32),
        (0x024, "T1CONFIG", 0x6000_2000),
        (0x048, "WDTCONFIG0", 0x0004_C000),
        (0x050, "WDTCONFIG2", 0x018C_BA80),
        (0x054, "WDTCONFIG3", 0x07FF_FFFF),
        (0x058, "WDTCONFIG4", 0x000F_FFFF),
        (0x05C, "WDTCONFIG5", 0x000F_FFFF),
        (0x068, "RTCCALICFG", 0x0001_3000),
    ] {
        assert_eq!(
            read32(S3_TIMG0, offset),
            trm,
            "reading TIMG0+{offset:#05X} ({register}) must yield {trm:#010X}; \
             a zero here means the offset is undecoded or the reset value is \
             still the old one"
        );
    }
    assert_eq!(
        read32(S3_GPIO, 0x6FC),
        0x0190_7040,
        "reading GPIO+0x6FC (REG_DATE) must yield the S3's version stamp"
    );
}

// ── The sibling parts, which must not move ───────────────────────────────────

#[test]
fn esp32c3_and_esp32_timg_gpio_stay_on_their_own_silicon() {
    assert_reset_values_conform(C3_TIMG0, C3_SVD, "TIMG0", 24);
    assert_reset_values_conform(ESP32_TIMG0, ESP32_SVD, "TIMG0", 24);
    assert_reset_values_conform(C3_GPIO, C3_SVD, "GPIO", 2);

    // The version stamps, spelled out. These are the values an edit that
    // "harmonised" the three files would destroy, and they are all different.
    assert_eq!(
        reset_of(C3_GPIO, "REG_DATE"),
        0x0200_6130,
        "the C3's GPIO version stamp is its own, not the S3's 0x01907040"
    );
    assert_ne!(
        reset_of(C3_GPIO, "REG_DATE"),
        reset_of(S3_GPIO, "REG_DATE"),
        "two different Espressif parts cannot carry the same silicon version \
         stamp; if these ever match, one file was copied onto the other"
    );

    // The C3 and classic ESP32 already hold the TRM numbers for the registers
    // the S3 got wrong. Pinned so this fix cannot be "applied" by dragging the
    // correct files onto the broken one's values.
    for rel in [C3_TIMG0, ESP32_TIMG0] {
        assert_eq!(reset_of(rel, "T0CONFIG"), 0x6000_2000, "{rel}");
        assert_eq!(reset_of(rel, "WDTCONFIG0"), 0x0004_C000, "{rel}");
        assert_eq!(reset_of(rel, "WDTCONFIG2"), 0x018C_BA80, "{rel}");
        assert_eq!(reset_of(rel, "RTCCALICFG"), 0x0001_3000, "{rel}");
    }
}
