// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **Declarative-coverage ratchet.**
//!
//! The direction of travel for off-chip devices is YAML: a part described by a
//! `configs/devices/*.yaml` descriptor runs on native and wasm, ports without a
//! rebuild, and is reviewable by someone holding the datasheet. A part written
//! as Rust in `peripherals/components/` is engine code — it ships in every
//! binary, it can only be changed by a Rust programmer, and each one is another
//! thing the declarative engine has to stay bug-compatible with.
//!
//! Two numbers say where that migration stands, and this file pins both:
//!
//! * **the YAML count only goes up** — deleting a descriptor (or quietly
//!   replacing it with Rust) fails here;
//! * **the Rust count only goes down** — adding a hand-written device model
//!   fails here, which is the whole point. A new part is a YAML file unless
//!   there is a reason it cannot be, and "there is a reason" should be an
//!   argument made in a PR, not a file that appears.
//!
//! Neither number is a quality bar on its own. Read together they are the only
//! measurement of whether the YAML device machine is actually replacing engine
//! code or merely accumulating alongside it.

use std::collections::BTreeSet;
use std::path::PathBuf;

// ─── the baseline ──────────────────────────────────────────────────────────

/// Device types modelled as YAML today (`configs/devices/*.yaml`).
///
/// ⚠️ THE YAML COUNT ONLY GOES UP. Raise this when you add a descriptor.
///
/// 50 → 58: the eight `logic_gate` 74-series descriptors (`74hc04`, `74hc00`,
/// `74hc08`, `74hc32`, `74hc125`, `74lvc1t45`, `74hc245`, `74cbtlv3257`). No
/// Rust model was deleted in the same change, so the Rust baseline below is
/// unchanged — the new primitive itself is `declarative_logic.rs`, which the
/// [`ENGINE_PREFIX`] rule excludes as engine rather than as a part.
///
/// 58 → 60: the two tri-colour e-paper descriptors. Both DID delete their Rust
/// model, so the Rust baseline below falls by two in the same commit.
///
/// 60 → 62: `tm1637_7seg.yaml` and `seven_segment.yaml`, the two pin-driven
/// segment displays. Both DID delete their Rust model, so the Rust baseline
/// below falls by two in the same commit. They are the first descriptors to
/// declare an `artifact:` — until that key existed a ported display would
/// simulate perfectly and inspect as nothing.
///
/// 62 → 65: `max7219.yaml`, `hc595.yaml` and `hc595_7seg.yaml`, the three
/// shift-register display drivers. All three DID delete their Rust model, so
/// the Rust baseline below falls by three in the same commit. They are the
/// first `spi_device` descriptors with NO register map at all — a part whose
/// unit of work is a MESSAGE rather than a register — and the first to dispatch
/// a rule on a frame's own bytes (`frames.opcode_byte` / `frame_byte(N)`).
///
/// 65 → 71: the six analog plants (`ldr`, `potentiometer`, `ntc_thermistor`,
/// `mq6`, `soil_moisture`, `lipo_charger`). All six DID delete their Rust
/// model, so the Rust baseline below falls by six in the same commit. They are
/// the first `analog_source` descriptors to state an EQUATION rather than a
/// graph (`analog.formula`, with `pow()` and `exp()` added to the expression
/// language for the CdS power law and the NTC beta equation), and the first to
/// drive more than one stimulus channel.
///
/// 71 → 73: `vl53l1x.yaml` and `bno055.yaml`, two I²C register shells. Both DID
/// delete their Rust model, so the Rust baseline below falls by two in the same
/// commit. Neither needed a new key: the VL53L1X is the first shipped part to
/// pair `pointer_width: 2` with `auto_increment`, and the BNO055 is the first
/// to use `page_register` to say "this twin has no page 1" rather than to
/// alias a register.
///
/// 73 → 74: `bmp280.yaml`. ⚠️ A port of a CONSTANT part — the hand model's raw
/// ADC words were two literals and it declared no stimulus channels — so the
/// count goes up without a sensor being gained. The descriptor's header says so
/// and says what driving it would take.
///
/// 74 → 75: `sn74hc165.yaml`. It DID delete its Rust model, so the Rust
/// baseline below falls by one in the same commit. The key it needed is
/// `metadata.inputs[].bits:` — ONE declared channel standing for eight, seeded
/// together by the single integer `inputs:` key four shipped manifests set.
/// Per-channel seeding could not express it, so the key and the port landed
/// together (see `sn74hc165_migration_parity.rs`).
///
/// 75 → 76: `aht20.yaml`. It DID delete its Rust model, so the Rust baseline
/// falls by one in the same commit. Three keys: `crc8.covers: { bytes: N }`,
/// `response[].fields` (a packed word whose fields straddle byte boundaries),
/// and `i2c.not_ready_byte`. ⚠️ Its BUSY bit stops being a COUNT of status
/// reads and becomes the datasheet's 80 ms — which broke the shipped
/// `nucleo-f407-i2c` firmware, because that firmware never waited.
/// 77 → 78: DHT11 frame packing gets its own GPIO schedule descriptor.
const YAML_DEVICES_BASELINE: usize = 81;

/// Device models still hand-written in Rust
/// (`crates/core/src/peripherals/components/*.rs`, minus [`EXCLUDED`]).
///
/// ⚠️ THE RUST COUNT ONLY GOES DOWN. Lower this when you port one to YAML.
///
/// 45 → 43: `tm1637_7seg.rs` and `seven_segment.rs` are deleted, ported to the
/// descriptors counted above. `declarative_artifact.rs` is added in the same
/// change and is NOT counted — it is the ENGINE that renders a declared
/// artifact, with no model behind it, and `ENGINE_PREFIX` excludes it for the
/// same reason it excludes `rule_machine.rs`.
///
/// 43 → 40: `max7219.rs`, `hc595.rs` and `hc595_7seg.rs` are deleted, ported to
/// the descriptors counted above. No engine file is added in the same change —
/// the three ports needed new KEYS (`frame_byte()`, `frames.opcode_byte`,
/// `frames.discard_partial`, `artifact.blank_when` / `fill_when`, and `powered:`
/// honoured by two primitives) rather than a new primitive.
///
/// 40 → 34: `ldr.rs`, `potentiometer.rs`, `ntc_thermistor.rs`, `mq6.rs`,
/// `soil_moisture.rs` and `lipo_charger.rs` are deleted, ported to the
/// descriptors counted above. No engine file is added in the same change — the
/// six ports grew the EXISTING `declarative_analog.rs` primitive three keys
/// (`analog.formula`, `analog.source`, `analog.encode`), one on `derived:`
/// (`when:`, a threshold on a boolean channel) and two functions in the
/// expression language (`pow`, `exp`).
///
/// 34 → 32: HOUSEKEEPING, no port. Two files in `components/` were being
/// counted as hand-written device models and are neither:
///
/// * `pca9685.rs` — the PCA9685 was ported to `configs/devices/pca9685.yaml`
///   long ago and `build_i2c_device` has routed the type to the descriptor
///   since. The struct survives only as the byte-parity ORACLE
///   `tests/pca9685_tmp102_parity.rs` drives, which is exactly what
///   `veml7700.rs` is already excluded for. Counting it said a part was
///   un-ported when it was ported, which makes the number report the opposite
///   of the truth.
/// * `supply.rs` — the `powered` key's one home. It has no `I2cDevice` impl of
///   its own beyond a DECORATOR, no `PeripheralKit`, no descriptor and no
///   `device_type`; it is read by every I²C kit, by `declarative_spi`,
///   `declarative_gpio` and `ili9341_parallel`. Engine, the same as
///   `rule_machine.rs` and `i80_panel.rs`.
///
/// Neither file is deleted: the oracle is what proves the descriptor, and the
/// engine module is live code. What changes is that the ratchet stops calling
/// them parts.
///
/// 32 → 30: `vl53l1x.rs` and `bno055.rs` are deleted, ported to the descriptors
/// counted above.
///
/// 30 → 29: the BMP280 is ported to the descriptor counted above and
/// `bmp280.rs` moves to [`EXCLUDED`] as its byte-parity oracle — the third
/// file to take that route, after `veml7700.rs` and `pca9685.rs`. It is not
/// DELETED for one reason worth writing down: the ESP32 and ESP32-C3 I²C
/// controller tests attach it as a generic register-pointer slave, and
/// `crates/core/src/peripherals/esp32c3/` is covered by a silicon drift-ack
/// digest in `validation/manifest.yaml`, so editing a `#[cfg(test)]` module
/// inside that directory turns `generate_validation_status.py --check --drift`
/// red for a change that touches no model.
///
/// 29 → 28: `sn74hc165.rs` is DELETED, ported to the descriptor counted above.
/// Not kept as an oracle — nothing in the tree attaches it as a generic slave,
/// and the transcript it produced is reproduced verbatim inside
/// `tests/sn74hc165_migration_parity.rs`.
///
/// 28 → 27: `aht20.rs` is DELETED, ported to the descriptor counted above. Not
/// kept as an oracle: its BUSY thunk and its constant measurement are the two
/// things the port deliberately changes, so an oracle in `components/` would be
/// asserting both. It is reproduced verbatim inside
/// `tests/aht20_migration_parity.rs`.
/// 27 → 26: keypad.rs deleted; its existing descriptor now uses gpio_device.
/// 26 → 25: rotary_encoder.rs deleted; Gray phases and cadence now live in YAML.
/// 25 → 24: BME280 uses exact integer YAML; Rust remains only as an oracle/controller fixture.
/// 24 → 23: DHT22 production routing uses GPIO schedules; Rust is a parity oracle.
const RUST_DEVICES_BASELINE: usize = 23;

/// Files in `components/` that are NOT a device model, with the reason. Listed
/// here rather than pattern-matched so every exemption is a line someone wrote
/// on purpose — a silent filter is how a ratchet stops counting the thing it
/// was built to count.
const EXCLUDED: &[(&str, &str)] = &[
    ("dht22.rs", "unchanged waveform parity oracle; production attachment uses dht22.yaml and dht11.yaml"),
    ("gpio_schedule.rs", "bounded finite waveform engine shared by GPIO descriptors, not a device model"),
    (
        "bme280.rs",
        "unchanged byte-parity oracle and nRF52 serial-instance generic I2C slave fixture; production factory and kit routing use bme280.yaml",
    ),
    ("mod.rs", "the module tree, not a device"),
    (
        "i2c_factory.rs",
        "the factory that BUILDS devices from a descriptor",
    ),
    (
        "mux_fixture.rs",
        "test-only TCA9548A topology shared by the per-controller tests",
    ),
    (
        "seven_seg_font.rs",
        "a glyph table shared by the 7-segment models",
    ),
    (
        "sensirion.rs",
        "the shared Sensirion CRC-8 helper, not a part",
    ),
    (
        "iolink_native.rs",
        "`#![cfg(feature = \"iolink-native\")]` host bridge to a real master, not a simulated device",
    ),
    (
        "veml7700.rs",
        "`#[cfg(test)]` hand-written oracle retained only to prove the YAML VEML7700 byte-identical; the shipping part is the descriptor",
    ),
    (
        "veml7700_parity.rs",
        "`#[cfg(test)]` harness for the oracle above",
    ),
    (
        "bmp280.rs",
        "hand-written oracle retained only to prove the YAML BMP280 \
         byte-identical (`tests/bmp280_migration_parity.rs`); the shipping part \
         is `configs/devices/bmp280.yaml`, and both `build_i2c_device` and the \
         kit registry route `bmp280` there. NOT deleted, because the ESP32 and \
         ESP32-C3 controller tests attach it as a generic register-pointer \
         slave and `crates/core/src/peripherals/esp32c3/` carries a silicon \
         drift-ack DIGEST that an edit to a test module inside it invalidates",
    ),
    (
        "pca9685.rs",
        "hand-written oracle retained only to prove the YAML PCA9685 \
         byte-identical (`tests/pca9685_tmp102_parity.rs`); the shipping part is \
         `configs/devices/pca9685.yaml`, and `build_i2c_device` routes `pca9685` \
         to the descriptor, so nothing but the parity test can reach this struct",
    ),
    (
        "supply.rs",
        "ENGINE INFRASTRUCTURE, not a part: the one home for the `powered` \
         config key (`powered_from_config` / `powered_from_placement`, the \
         `UnpoweredI2cDevice` decorator, `mark_unpowered`). It models no device \
         — every I2C kit, both declarative primitives and nine display models \
         read it",
    ),
    (
        "rule_machine.rs",
        "the Tier-2 rule ENGINE every declarative part's `rules:` runs on, not a part",
    ),
    (
        "i80_panel.rs",
        "the ONE-METHOD 8080-parallel seam `Esp32s3LcdCam` strobes — a trait \
         declaration with no model behind it, and the thing that lets the \
         parallel panel become a descriptor at all",
    ),
];

/// The declarative ENGINE itself (`declarative_*.rs`): the primitives every
/// YAML descriptor is interpreted by. Excluded by prefix because the set grows
/// with each new primitive and each addition is a move toward YAML, not away.
const ENGINE_PREFIX: &str = "declarative_";

/// Out-of-line `#[cfg(test)]` modules (`<model>_tests.rs`), which are a model's
/// TESTS and not a model.
///
/// Excluded by suffix rather than by name because they arrive in batches: the
/// inline-test-module split (#1142) created `mcp2515_tests.rs` here in one
/// commit, and each such file counted as a brand-new hand-written device — the
/// ratchet read a pure code move as the migration going backwards. A suffix
/// rule is a pattern and this file's own rule is that a silent filter is how a
/// ratchet stops counting; this one is neither silent (it is printed in the
/// exclusion list with its reason, and the matched files are named) nor able to
/// hide a real device, since `foo_tests.rs` is the compiler-visible name of a
/// test module and never of a part.
const TEST_MODULE_SUFFIX: &str = "_tests.rs";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn yaml_devices() -> BTreeSet<String> {
    let dir = repo_root().join("configs/devices");
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {dir:?}: {e}"))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect()
}

fn rust_devices() -> BTreeSet<String> {
    let dir = repo_root().join("crates/core/src/peripherals/components");
    let excluded: BTreeSet<&str> = EXCLUDED.iter().map(|(f, _)| *f).collect();
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {dir:?}: {e}"))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .filter(|name| {
            !excluded.contains(name.as_str())
                && !name.starts_with(ENGINE_PREFIX)
                && !name.ends_with(TEST_MODULE_SUFFIX)
        })
        .collect()
}

/// The out-of-line test modules the suffix rule dropped, so the exclusion is
/// reported by NAME rather than as a count.
fn excluded_test_modules() -> BTreeSet<String> {
    let dir = repo_root().join("crates/core/src/peripherals/components");
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {dir:?}: {e}"))
        .map(|e| e.expect("dir entry").path())
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(TEST_MODULE_SUFFIX))
        .collect()
}

#[test]
fn declarative_coverage_only_improves() {
    let yaml = yaml_devices();
    let rust = rust_devices();

    println!("declarative coverage:");
    println!(
        "  YAML device descriptors : {:>3}  (baseline {YAML_DEVICES_BASELINE}, only goes UP)",
        yaml.len()
    );
    println!(
        "  hand-written Rust models: {:>3}  (baseline {RUST_DEVICES_BASELINE}, only goes DOWN)",
        rust.len()
    );
    println!("  excluded from the Rust count ({}):", EXCLUDED.len());
    for (file, why) in EXCLUDED {
        println!("    {file:<22} {why}");
    }
    println!(
        "    {ENGINE_PREFIX}*.rs{:<11} the declarative engine itself",
        ""
    );
    println!(
        "    *{TEST_MODULE_SUFFIX:<21} an out-of-line #[cfg(test)] module, not a device: {:?}",
        excluded_test_modules().iter().collect::<Vec<_>>()
    );
    println!("  YAML: {:?}", yaml.iter().collect::<Vec<_>>());

    assert!(
        yaml.len() >= YAML_DEVICES_BASELINE,
        "the YAML device count FELL, {} < {YAML_DEVICES_BASELINE}. A descriptor was \
         deleted or renamed away. If that is deliberate (a part was dropped from the \
         catalog) lower the baseline in the same commit and say why; if a part moved \
         BACK to Rust, that is the direction this ratchet exists to refuse.",
        yaml.len()
    );
    assert!(
        rust.len() <= RUST_DEVICES_BASELINE,
        "the hand-written Rust device count GREW, {} > {RUST_DEVICES_BASELINE}. A new \
         device belongs in configs/devices/*.yaml unless the declarative primitives \
         genuinely cannot express it. If they cannot, raise the baseline in the same \
         commit with the reason — and consider whether the missing primitive is the \
         real change.\\nCurrent Rust models: {:?}",
        rust.len(),
        rust.iter().collect::<Vec<_>>()
    );

    // A baseline that has drifted BELOW the truth silently stops ratcheting:
    // it would accept a regression back up to the stale number. Same for YAML.
    assert_eq!(
        yaml.len(),
        YAML_DEVICES_BASELINE,
        "the YAML count improved to {} — raise YAML_DEVICES_BASELINE to lock it in",
        yaml.len()
    );
    assert_eq!(
        rust.len(),
        RUST_DEVICES_BASELINE,
        "the Rust count improved to {} — lower RUST_DEVICES_BASELINE to lock it in",
        rust.len()
    );
}

/// Every exclusion must name a file that exists. A stale entry is how an
/// exclusion list starts excluding nothing while still reading as deliberate.
#[test]
fn every_exclusion_names_a_real_file() {
    let dir = repo_root().join("crates/core/src/peripherals/components");
    for (file, why) in EXCLUDED {
        assert!(
            dir.join(file).is_file(),
            "EXCLUDED lists {file} ({why}) but no such file exists in {dir:?}"
        );
    }
}
