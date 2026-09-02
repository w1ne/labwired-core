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
    assert!(
        bus.is_err(),
        "a nonexistent D/C pin must be refused, not guessed"
    );
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

/// The shipped BRD2709A lab manifest must actually build. A committed
/// system.yaml that no longer loads is worse than none: it looks like a
/// working starting point right up until someone runs it.
#[test]
fn the_shipped_brd2709a_st7789_lab_builds() {
    use labwired_config::SystemManifest;
    let path = repo("examples/brd2709a/st7789-system.yaml");
    let manifest = SystemManifest::from_file(&path).expect("load the shipped lab manifest");
    let chip_path = repo("configs/chips/efr32mg26.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("load efr32mg26 descriptor");
    let bus = SystemBus::from_config(&chip, &manifest);
    assert!(bus.is_ok(), "the shipped lab must build: {:?}", bus.err());
}

/// The pins in that manifest have to be REAL pins on this die, resolvable to a
/// GPIO output. PC00 and PC04 come off UG594 Table 3.1 (breakout pads 17 and
/// 16); if the chip descriptor ever stopped resolving them the panel would
/// attach with a dead D/C line and paint nothing, silently.
#[test]
fn the_lab_pins_resolve_on_this_die() {
    let ok = efr32_with(
        "st7789-170x320",
        "spi0",
        &[("cs_pin", "PC04".into()), ("dc_pin", "PC00".into())],
    );
    assert!(ok.is_ok(), "PC04/PC00 must resolve: {:?}", ok.err());
}

/// THE DECK: every onboarded part on one board, built from the shipped
/// manifest. This is the claim "all of them work with the board" as a gate.
///
/// It is also the only thing that would catch the block conflict: a display in
/// SPI mode and a mic in I2S mode cannot share one USART, because I2SCTRL.EN
/// switches the whole block. Put both on `spi0` and the second attach silently
/// wins -- there is no error, just a panel or a microphone that never answers.
#[test]
fn the_agent_deck_builds_with_every_part_on_it() {
    use labwired_config::SystemManifest;
    let path = repo("examples/brd2709a/agent-deck-system.yaml");
    let manifest = SystemManifest::from_file(&path).expect("load the deck manifest");

    // Every onboarded part is actually present -- a deck that quietly lost one
    // would still build.
    let types: Vec<&str> = manifest
        .external_devices
        .iter()
        .map(|d| d.r#type.as_str())
        .collect();
    for want in ["st7789-170x320", "inmp441", "slide-potentiometer"] {
        assert!(
            types.contains(&want),
            "the deck must carry {want}, has {types:?}"
        );
    }
    // Panel RES + BLK, encoder A/B/SW, button module, toggle. Named one by one
    // so a dropped contact fails here rather than silently shrinking the deck.
    let io: Vec<&str> = manifest.board_io.iter().map(|b| b.id.as_str()).collect();
    for want in [
        "tft_res", "tft_blk", "enc_clk", "enc_dt", "enc_sw", "btn", "toggle",
    ] {
        assert!(io.contains(&want), "the deck must wire {want}, has {io:?}");
    }
    assert_eq!(
        manifest.board_io.len(),
        7,
        "exactly those seven contacts, saw {io:?}"
    );

    // The pushbutton module DRIVES its SIG line, so it is the one active-HIGH
    // contact. Every other contact closes to ground. Getting this backwards
    // reads as a button stuck down, which looks like firmware, not wiring.
    for b in &manifest.board_io {
        let want_high = b.id == "btn" || b.id == "tft_blk";
        assert_eq!(b.active_high, want_high, "{} has the wrong polarity", b.id);
    }

    // The panel and the microphone must be on DIFFERENT blocks.
    let tft = manifest
        .external_devices
        .iter()
        .find(|d| d.id == "tft")
        .unwrap();
    let mic = manifest
        .external_devices
        .iter()
        .find(|d| d.id == "mic")
        .unwrap();
    assert_ne!(
        tft.connection, mic.connection,
        "SPI and I2S cannot share one USART: I2SCTRL.EN switches the whole block",
    );

    let chip_path = repo("configs/chips/efr32mg26.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("load efr32mg26 descriptor");
    let bus = SystemBus::from_config(&chip, &manifest);
    assert!(bus.is_ok(), "the deck must build: {:?}", bus.err());
}

/// No two parts on the deck may claim the same MCU pin. A duplicate would
/// build fine and produce a board nobody can wire.
#[test]
fn the_deck_assigns_every_pin_once() {
    use labwired_config::SystemManifest;

    // ⚠️ BOTH NAMESPACES, NORMALISED TO ONE. The first version of this gate
    // pushed "PC04" from a device config and "gpiod:3" from board_io into the
    // same list and asked for duplicates. Those two spellings can never be
    // equal, so the deck shipped with PD03 claimed TWICE — as the panel
    // backlight and as the toggle — and this test passed. A pin name is
    // canonicalised here so a collision between the two sources is reachable.
    fn canon(pin_name: &str) -> String {
        let t = pin_name.trim().trim_start_matches('P');
        let mut c = t.chars();
        let port = c.next().unwrap_or('?').to_ascii_lowercase();
        let idx: String = c.filter(|ch| ch.is_ascii_digit()).collect();
        let n: u32 = idx.trim_start_matches('0').parse().unwrap_or(0);
        format!("gpio{port}:{n}")
    }

    let manifest = SystemManifest::from_file(&repo("examples/brd2709a/agent-deck-system.yaml"))
        .expect("load the deck manifest");

    let mut claimed: Vec<(String, String)> = Vec::new();
    for d in &manifest.external_devices {
        for key in ["cs_pin", "dc_pin"] {
            if let Some(v) = d.config.get(key).and_then(|v| v.as_str()) {
                claimed.push((canon(v), format!("{}.{key}", d.id)));
            }
        }
    }
    for b in &manifest.board_io {
        claimed.push((format!("{}:{}", b.peripheral, b.pin), b.id.clone()));
    }

    let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (pin, owner) in &claimed {
        if let Some(prev) = seen.insert(pin.clone(), owner.clone()) {
            panic!("pin {pin} is claimed twice on the deck: by {prev} and by {owner}");
        }
    }

    // The normaliser must actually reach the device-config namespace, or this
    // gate silently degrades to "board_io has no duplicates" again.
    assert_eq!(
        canon("PD03"),
        "gpiod:3",
        "canon() must meet board_io's spelling"
    );
    assert!(
        seen.contains_key("gpioc:4") && seen.contains_key("gpioc:0"),
        "the panel's CS/DC must land in the same namespace as board_io, saw {:?}",
        seen.keys().collect::<Vec<_>>()
    );

    // ⚠️ ONLY NINE OF THE FIFTEEN PINS ARE MANIFEST DATA. The other six are
    // BUS pins, implied by `connection: spi0` / `spi2` / `iadc0` and the
    // chip's route — they appear nowhere in this file, so a gate cannot read
    // them off the manifest. They are named here instead, and the two sets are
    // checked to be disjoint and to cover the pad list exactly. That catches
    // the error that can really happen: a `cs_pin`/`dc_pin`/board_io entry
    // quietly landing on a pin the bus already drives.
    assert_eq!(
        claimed.len(),
        9,
        "nine deck pins are declarable; saw {} -> {:?}",
        claimed.len(),
        claimed
    );

    // tft SCK/MOSI on USART0, mic SCK/WS/SD on USART2, fader wiper on IADC0.
    const BUS_PINS: [&str; 6] = [
        "gpioc:3", "gpioc:2", "gpioa:4", "gpioa:5", "gpioa:7", "gpiod:2",
    ];
    for b in BUS_PINS {
        assert!(
            !seen.contains_key(b),
            "{b} is driven by a bus, but {} also claims it",
            seen[b]
        );
    }

    // UG594 Table 3.1 p.10 + Figure 3.5 p.9: the 28 pads carry FIFTEEN MCU
    // GPIO (plus four dedicated analog inputs, and GND/5V/VMCU/3V3/VREF/
    // BOARD_ID). The deck spends all fifteen, so a dropped pin is caught.
    const PADS: [&str; 15] = [
        "gpioc:7", "gpioc:5", "gpioa:4", "gpioa:5", "gpioc:0", "gpioa:7", "gpiod:3", "gpioc:2",
        "gpioc:1", "gpioc:3", "gpioc:4", "gpioc:6", "gpiod:2", "gpiod:5", "gpiod:4",
    ];
    let mut spent: Vec<&str> = seen.keys().map(|k| k.as_str()).collect();
    spent.extend(BUS_PINS);
    spent.sort_unstable();
    let mut pads = PADS.to_vec();
    pads.sort_unstable();
    assert_eq!(
        spent, pads,
        "the deck must spend each of the 15 breakout GPIO exactly once"
    );
}
