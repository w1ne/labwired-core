// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Every part onboarded from the HEStore order must attach on the EFR32 --
//! the hackathon target -- and not merely on the STM32F103 they were first
//! modelled against.
//!
//! "Works on the board" is a claim that decays silently: a chip descriptor
//! that loses a peripheral, a kit whose transport stops matching, or a part
//! wired to a controller this die does not have, all fail at ATTACH time with
//! nothing downstream to notice. So each part below is built through
//! `SystemBus::from_config` against the real efr32mg26 descriptor, which is
//! the same path a lab takes.
//!
//! ⚠️ These parts are 3.3 V. The EFR32xG26 GPIO is NOT 5 V tolerant -- its
//! datasheet gives VDIGPIN abs max as VIOVDD + 0.3 V -- so nothing here may be
//! driven from a 5 V rail on the bench, whatever the twin accepts.

use labwired_config::{ChipDescriptor, ExternalDevice, SystemManifest};
use labwired_core::bus::SystemBus;
use std::collections::HashMap;
use std::path::PathBuf;

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Build an EFR32 bus carrying one external device, exactly as a lab would.
fn efr32_with(
    device_type: &str,
    connection: &str,
    config: &[(&str, serde_yaml::Value)],
) -> Result<SystemBus, String> {
    let chip_path = repo("configs/chips/efr32mg26.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("load efr32mg26 descriptor");
    let mut cfg = HashMap::new();
    for (k, v) in config {
        cfg.insert(k.to_string(), v.clone());
    }
    let manifest = SystemManifest {
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "efr32-parts-rig".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        cpu_hz: None,
        external_devices: vec![ExternalDevice {
            id: "part".to_string(),
            r#type: device_type.to_string(),
            connection: connection.to_string(),
            channel: None,
            route: Default::default(),
            config: cfg,
        }],
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        parts: Default::default(),
        memory_overrides: Default::default(),
    };
    SystemBus::from_config(&chip, &manifest).map_err(|e| e.to_string())
}

/// The 1.9in ST7789 panel over the EFR32's USART0 in SPI mode (`spi0`).
#[test]
fn the_st7789_panel_attaches_on_efr32() {
    let bus = efr32_with(
        "st7789-170x320",
        "spi0",
        &[("cs_pin", "PC00".into()), ("dc_pin", "PC01".into())],
    );
    assert!(bus.is_ok(), "ST7789 must attach on efr32: {:?}", bus.err());
}

/// The panel refuses a D/C pin that is not a driveable GPIO on THIS die --
/// the refusal has to survive the chip change, or the "no framing inference"
/// guarantee is only true on STM32.
#[test]
fn the_st7789_still_refuses_an_unresolvable_dc_pin_on_efr32() {
    let bus = efr32_with(
        "st7789-170x320",
        "spi0",
        &[("cs_pin", "PC00".into()), ("dc_pin", "PZ99".into())],
    );
    assert!(bus.is_err(), "a nonexistent D/C pin must be refused, not guessed");
}

/// The INMP441 on the same block in I2S mode. On EFR32 there is no dedicated
/// I2S peripheral -- I2S is a USART mode -- so the mic attaches to `spi0`.
#[test]
fn the_inmp441_microphone_attaches_on_efr32() {
    let bus = efr32_with("inmp441", "spi0", &[("channel", "left".into())]);
    assert!(bus.is_ok(), "INMP441 must attach on efr32: {:?}", bus.err());
}

/// A channel that is not a side of a stereo frame is refused rather than
/// silently defaulted: defaulting would hand back a working-looking mic that
/// is on the wrong half, which is the failure this part is prone to.
#[test]
fn the_microphone_refuses_a_channel_that_is_not_a_side() {
    let bus = efr32_with("inmp441", "spi0", &[("channel", "middle".into())]);
    assert!(bus.is_err(), "an invalid channel must be refused");
}

/// The slide fader reaches the shared potentiometer kit through core's
/// TYPE_ALIASES, on the EFR32's IADC.
#[test]
fn the_slide_potentiometer_attaches_on_efr32_iadc() {
    let bus = efr32_with("slide-potentiometer", "iadc0", &[("channel", 0.into())]);
    assert!(
        bus.is_ok(),
        "slide-potentiometer must reach the pot kit on efr32: {:?}",
        bus.err()
    );
}

/// And the canonical spelling must behave identically -- if these two ever
/// diverge, the alias has stopped meaning "the same part".
#[test]
fn the_canonical_potentiometer_matches_the_alias_on_efr32() {
    let aliased = efr32_with("slide-potentiometer", "iadc0", &[("channel", 0.into())]);
    let canonical = efr32_with("potentiometer", "iadc0", &[("channel", 0.into())]);
    assert_eq!(
        aliased.is_ok(),
        canonical.is_ok(),
        "the alias and its target must attach alike on this die",
    );
}
