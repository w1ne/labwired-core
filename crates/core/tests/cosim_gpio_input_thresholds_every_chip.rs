// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A solved voltage on `board.gpio_in.<pad>` becomes the level firmware reads,
//! on every shipped chip that declares datasheet input thresholds.
//!
//! For each chip descriptor, one source drives one pad through a series
//! resistor in the MNA solver. The sweep walks the pad 20 mV either side of
//! the descriptor's VIL and VIH, so a threshold off by more than that, a
//! threshold that never reaches the pad's input buffer, or an inverted band
//! fails here. Chips without `io_voltage_v` + `gpio_input_thresholds` must
//! refuse the route at session build, and must be listed in
//! [`NOT_TRANSCRIBED`], so the list can only shrink.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, CosimModelConfig, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, InputThresholds, RoutingError};
use labwired_core::pins::External;

/// Chips whose datasheet input thresholds are not transcribed yet. A volts
/// route to `board.gpio_in.<pad>` is refused on these. Remove an entry when
/// its descriptor gets `io_voltage_v` + `gpio_input_thresholds`.
const NOT_TRANSCRIBED: &[&str] = &[
    "amap42kp",
    "efr32mg26",
    "esp32c3",
    "esp32s3",
    "mimxrt595s",
    "nrf51822",
    "nrf52832",
    "nrf52833",
    "nrf52840",
    "nrf54l15",
    // Declared-only descriptor (CPU and memory): no GPIO block and no levels.
    "rv1103",
    "s32k388",
    "sam4s16c",
    "stm32f746",
    "stm32f767",
    "stm32g071",
    "stm32g474re",
    "stm32h563",
    "stm32h735",
    "stm32l073",
    "stm32u545",
    "stm32u575",
    "stm32u5a5zj",
    "stm32wba52",
    // No GPIO/PORT block is declared; the ADC0_SEn pads are analog routes.
    "vegaboard-ri5cy",
    // Declared-only descriptor (CPU and memory): no GPIO block and no levels.
    "xck26-rpu",
];

/// Datasheet anchors for descriptors whose thresholds are absolute volts or
/// non-round ratios, so a transcription slip shows as a voltage, not a ratio:
/// (chip, VIL max volts, VIH min volts, source).
const DATASHEET_VOLTS: &[(&str, f64, f64, &str)] = &[
    (
        "nrf54lm20a",
        0.54,
        1.26,
        "nRF54LM20A datasheet v1.0 11.7.1, 0.3/0.7 x VDD, nRF54LM20 DK default VDD = 1.8 V",
    ),
    (
        "ra4m1",
        1.0,
        4.0,
        "RA4M1 R01DS0355EJ0110 Table 2.4, ports 0.2/0.8 x VCC, Uno R4 Minima VCC = 5 V",
    ),
    (
        "rp2040",
        0.8,
        2.0,
        "RP2040 datasheet Table 625, IOVDD = 3.3 V",
    ),
    (
        "rp2350",
        0.8,
        2.0,
        "RP2350 datasheet Table 1683, IOVDD = 3.3 V",
    ),
    (
        "pic24f04ka200",
        0.66,
        2.64,
        "DS30009937C DI10/DI20 at VDD = 3.3 V",
    ),
    (
        "pic24f04ka201",
        0.66,
        2.64,
        "DS30009937C DI10/DI20 at VDD = 3.3 V",
    ),
    (
        "pic24f08ka101",
        0.66,
        2.64,
        "DS39927C DI10/DI20 at VDD = 3.3 V",
    ),
    (
        "pic24f08ka102",
        0.66,
        2.64,
        "DS39927C DI10/DI20 at VDD = 3.3 V",
    ),
    (
        "pic24f16ka101",
        0.66,
        2.64,
        "DS39927C DI10/DI20 at VDD = 3.3 V",
    ),
    (
        "pic24f16ka102",
        0.66,
        2.64,
        "DS39927C DI10/DI20 at VDD = 3.3 V",
    ),
    (
        "esp32c6",
        0.825,
        2.475,
        "ESP32-C6 datasheet Table 5-4 at 3.3 V",
    ),
    (
        "esp32s3-zero",
        0.825,
        2.475,
        "ESP32-S3 datasheet Table 5-4 at 3.3 V",
    ),
    (
        "same70q21b",
        0.99,
        2.31,
        "DS60001527J Table 58-4 GPIO/GPIO_AD at VDDIO = 3.3 V",
    ),
];

/// Datasheet anchors for pads in a descriptor `pad_classes` entry, whose limits
/// differ from the chip default: (chip, pad, VIL max volts, VIH min volts,
/// source). Every class pad of every descriptor must be listed here.
const CLASS_VOLTS: &[(&str, &str, f64, f64, &str)] = &[
    (
        "atmega328p",
        "PB6",
        0.5,
        3.5,
        "DS40002061B Table 30-1 VIL1/VIH1 (XTAL1) at VCC = 5 V",
    ),
    (
        "same70q21b",
        "PB4",
        0.7,
        1.8,
        "DS60001527J Table 58-4 GPIO_MLB",
    ),
    (
        "same70q21b",
        "PB5",
        0.7,
        1.8,
        "DS60001527J Table 58-4 GPIO_MLB",
    ),
    (
        "same70q21b",
        "PD10",
        0.7,
        1.8,
        "DS60001527J Table 58-4 GPIO_MLB",
    ),
];

/// Chips whose input pads use a label no generic shape covers: the LiteX
/// control-and-status block names its pads `<csr>.<bit>` (switches 32..63).
const PAD_OVERRIDES: &[(&str, &str)] = &[("arty-litex-vexriscv", "cas.32")];

fn all_chips() -> Vec<(String, ChipDescriptor)> {
    let mut chips: Vec<(String, ChipDescriptor)> = std::fs::read_dir(root("configs/chips"))
        .expect("read configs/chips")
        .filter_map(|entry| {
            let path = entry.expect("dir entry").path();
            if path.extension()? != "yaml" {
                return None;
            }
            let name = path.file_stem()?.to_str()?.to_string();
            let desc = ChipDescriptor::from_file(&path)
                .unwrap_or_else(|e| panic!("load {}: {e}", path.display()));
            Some((name, desc))
        })
        .collect();
    chips.sort_by(|a, b| a.0.cmp(&b.0));
    chips
}

fn chip_bus(chip: &str, desc: &ChipDescriptor) -> Result<SystemBus, String> {
    let chip_path = root(&format!("configs/chips/{chip}.yaml"));
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "name: \"thresholds-{chip}\"\nchip: \"{}\"\nexternal_devices: []\n",
        chip_path.display()
    ))
    .expect("parse manifest");
    SystemBus::from_config(desc, &manifest).map_err(|e| format!("{e:#}"))
}

/// A source `Vsrc` behind 1 kOhm drives `pad`'s input buffer. The pad draws no
/// current, so the solved node equals the source; the solver still has to
/// produce it.
fn source_on_pad(pad: &str) -> Vec<CosimModelConfig> {
    serde_yaml::from_str(&format!(
        r#"
- id: "pad_source"
  adapter: "analog"
  step_ns: 1000
  inputs:
    volts: "ui.src.volts"
  outputs:
    v_pad: "board.gpio_in.{pad}"
  config:
    netlist_text: |
      Vsrc src 0 dc 0
      Rser src pad 1k
      Rleak pad 0 1G
    probes:
      v_pad: "v(pad)"
    sources:
      volts: "Vsrc"
    vdd: 3.3
"#
    ))
    .expect("parse netlist")
}

/// Candidate labels for one GPIO pad on `desc`: analog pads first (the last
/// round proved each resolves as a digital input), then `pins:` labels, then
/// the common bank-0 pin-0 label shapes.
fn candidate_pads(chip: &str, desc: &ChipDescriptor) -> Vec<String> {
    let mut pads: Vec<String> = PAD_OVERRIDES
        .iter()
        .filter(|(c, _)| *c == chip)
        .map(|(_, pad)| pad.to_string())
        .collect();
    pads.extend(desc.analog_pins.keys().cloned());
    let mut labels: Vec<String> = desc.pins.keys().cloned().collect();
    labels.sort();
    pads.extend(labels);
    pads.extend(
        [
            "PA0", "P0.00", "PTA0", "RA0", "GP0", "GPIO0", "P000", "PIO0_0", "P0_0",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    pads
}

/// Every `(peripheral name, pin)` whose input buffer an external driver
/// holds high.
fn externally_high(bus: &SystemBus) -> Vec<(String, u8)> {
    let mut high = Vec::new();
    for p in &bus.peripherals {
        let Some(port) = p.dev.pins() else { continue };
        for pin in 0..port.pin_count() {
            if port.external(pin) == External::Level(true) {
                high.push((p.name.clone(), pin));
            }
        }
    }
    high
}

enum Outcome {
    Swept {
        pad: String,
        thresholds: InputThresholds,
    },
    Refused {
        key: &'static str,
    },
    NoPad,
}

fn sweep_chip(chip: &str, desc: &ChipDescriptor) -> Result<Outcome, String> {
    for pad in candidate_pads(chip, desc) {
        if let Some(outcome) = sweep_pad(chip, desc, &pad)? {
            return Ok(outcome);
        }
    }
    Ok(Outcome::NoPad)
}

/// Sweep one pad about the thresholds the engine applies to it. `None` when
/// the label does not resolve on this chip.
fn sweep_pad(chip: &str, desc: &ChipDescriptor, pad: &str) -> Result<Option<Outcome>, String> {
    let pad = pad.to_string();
    {
        let mut bus = chip_bus(chip, desc)?;
        let models = source_on_pad(&pad);
        let mut session = match CosimSession::new(&models, std::path::Path::new("."), &bus) {
            Ok(Some(session)) => session,
            Ok(None) => return Err("no model".into()),
            Err(e) => return Err(format!("session: {e:#}")),
        };
        match session.binding_errors() {
            [] => {}
            [RoutingError::NoInputThresholds { key, .. }] => {
                return Ok(Some(Outcome::Refused { key }));
            }
            [RoutingError::UnknownPad { .. }] => return Ok(None),
            other => return Err(format!("{pad}: bind: {other:?}")),
        }
        let thresholds = InputThresholds::for_pad(&bus, &pad)
            .ok_or_else(|| format!("{pad}: bound but does not resolve"))?
            .map_err(|k| format!("bound without {k}"))?;
        let (vil, vih) = (thresholds.vil_volts, thresholds.vih_volts);
        // The first point sets a known low: inside the band a Schmitt input
        // keeps its previous level, and a pad's reset level may be high.
        let sweep = [
            (vil - 0.02, false), // just under VIL: low
            (vih - 0.02, false), // just under VIH: stays low
            (vih + 0.02, true),  // just over VIH: rises
            (vil + 0.02, true),  // just over VIL: keeps high (hysteresis)
            (vil - 0.02, false), // just under VIL: falls
            (vih - 0.02, false), // back inside the band: keeps low
        ];
        // Other drivers (a UART holding its RX line idle-high) are not the pad.
        let baseline = externally_high(&bus);
        let mut cycles = 0;
        let mut pin = None;
        for (volts, level) in sweep {
            session.set_signal_number("ui.src.volts", volts).unwrap();
            cycles += session.cycles_until_boundary(cycles);
            let (_, errors) = session
                .advance_to(cycles, &mut bus)
                .map_err(|e| format!("{pad}: solve: {e:#}"))?;
            if !errors.is_empty() {
                return Err(format!("{pad}: apply: {errors:?}"));
            }
            let solved = match session.signals().get(&format!("board.gpio_in.{pad}")) {
                Some(CosimSignalValue::F64(v)) => *v,
                other => return Err(format!("{pad}: no solved volts: {other:?}")),
            };
            if (solved - volts).abs() > 1e-4 {
                return Err(format!("{pad}: drove {volts} V, solved {solved} V"));
            }
            let high: Vec<_> = externally_high(&bus)
                .into_iter()
                .filter(|p| !baseline.contains(p))
                .collect();
            if level {
                match high.as_slice() {
                    [one] => pin = Some(one.clone()),
                    other => {
                        return Err(format!(
                            "{pad} at {volts:.3} V (VIL {vil:.3}, VIH {vih:.3}) must hold one \
                             input high, holds {other:?}"
                        ))
                    }
                }
            } else if let Some(p) = &pin {
                if high.contains(p) {
                    return Err(format!(
                        "{pad} at {volts:.3} V (VIL {vil:.3}, VIH {vih:.3}) must read low"
                    ));
                }
            } else if !high.is_empty() {
                return Err(format!(
                    "{pad} at {volts:.3} V (VIH {vih:.3}) rose early: {high:?}"
                ));
            }
        }
        Ok(Some(Outcome::Swept { pad, thresholds }))
    }
}

#[test]
fn every_transcribed_chip_turns_a_solved_pad_voltage_into_its_datasheet_level() {
    let mut problems = Vec::new();
    let mut refused = Vec::new();
    let mut swept = Vec::new();
    for (chip, desc) in all_chips() {
        let declares = desc.io_voltage_v.is_some() && desc.gpio_input_thresholds.is_some();
        match sweep_chip(&chip, &desc) {
            Ok(Outcome::Swept { pad, thresholds }) => {
                if let Some((_, vil, vih, source)) =
                    DATASHEET_VOLTS.iter().find(|(c, ..)| *c == chip)
                {
                    if (thresholds.vil_volts - vil).abs() > 0.005
                        || (thresholds.vih_volts - vih).abs() > 0.005
                    {
                        problems.push(format!(
                            "{chip}: VIL/VIH {:.4}/{:.4} V, datasheet {vil}/{vih} V ({source})",
                            thresholds.vil_volts, thresholds.vih_volts
                        ));
                    }
                }
                swept.push(format!("{chip}:{pad}"));
            }
            Ok(Outcome::Refused { key }) => {
                if declares {
                    problems.push(format!("{chip}: refused for {key} but declares both keys"));
                }
                refused.push(chip);
            }
            // A descriptor with no GPIO block has no input buffer to drive.
            Ok(Outcome::NoPad) if declares => problems.push(format!(
                "{chip}: declares thresholds but no pad label resolves"
            )),
            Ok(Outcome::NoPad) => {}
            Err(e) if e.contains("build bus") || chip_bus(&chip, &desc).is_err() => {}
            Err(e) => problems.push(format!("{chip}: {e}")),
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    let mut expected: Vec<String> = NOT_TRANSCRIBED.iter().map(|s| s.to_string()).collect();
    expected.sort();
    refused.sort();
    assert_eq!(
        refused, expected,
        "exactly the chips listed in NOT_TRANSCRIBED may refuse a volts route"
    );
    for (chip, ..) in DATASHEET_VOLTS {
        assert!(
            swept.iter().any(|s| s.starts_with(&format!("{chip}:"))),
            "{chip}: datasheet anchor never swept"
        );
    }
    eprintln!("{} chips swept: {}", swept.len(), swept.join(" "));
}

/// A pad in a datasheet pad class switches at the class limits, not the chip
/// default, end to end through the solver.
///
/// The sweep steps 20 mV about each threshold. Every class here differs from
/// its chip default by more than that at both edges (asserted), so a router
/// that ignored the class and used the default would read the wrong level at
/// some sweep point and fail. The class pads span two unrelated families and
/// both limit forms: a ratio of VCC (ATmega328P XTAL1) and fixed volts
/// (SAM E70 GPIO_MLB).
#[test]
fn every_pad_class_pad_switches_at_its_datasheet_limits() {
    let mut problems = Vec::new();
    let mut listed: Vec<(String, String)> = Vec::new();
    for (chip, desc) in all_chips() {
        let Some(t) = &desc.gpio_input_thresholds else {
            continue;
        };
        let io = desc
            .io_voltage_v
            .expect("thresholds come with io_voltage_v");
        let (default_vil, default_vih) = (t.vil * io, t.vih * io);
        for class in &t.pad_classes {
            for pad in &class.pads {
                listed.push((chip.clone(), pad.clone()));
                let Some((.., vil, vih, source)) =
                    CLASS_VOLTS.iter().find(|(c, p, ..)| *c == chip && p == pad)
                else {
                    problems.push(format!(
                        "{chip}:{pad} ({}) has no CLASS_VOLTS anchor",
                        class.name
                    ));
                    continue;
                };
                if (vil - default_vil).abs() <= 0.04 && (vih - default_vih).abs() <= 0.04 {
                    problems.push(format!(
                        "{chip}:{pad}: class {vil}/{vih} V is within the sweep step of the \
                         default {default_vil:.3}/{default_vih:.3} V, so the sweep cannot tell them apart"
                    ));
                }
                // Lower case too: class membership is by resolved input, not
                // by label spelling.
                for label in [pad.clone(), pad.to_ascii_lowercase()] {
                    match sweep_pad(&chip, &desc, &label) {
                        Ok(Some(Outcome::Swept { thresholds, .. })) => {
                            if (thresholds.vil_volts - vil).abs() > 0.005
                                || (thresholds.vih_volts - vih).abs() > 0.005
                            {
                                problems.push(format!(
                                    "{chip}:{label}: VIL/VIH {:.4}/{:.4} V, datasheet \
                                     {vil}/{vih} V ({source})",
                                    thresholds.vil_volts, thresholds.vih_volts
                                ));
                            }
                        }
                        Ok(Some(Outcome::Refused { key })) => {
                            problems.push(format!("{chip}:{label}: refused for {key}"))
                        }
                        Ok(Some(Outcome::NoPad)) | Ok(None) => {
                            problems.push(format!("{chip}:{label}: class pad does not resolve"))
                        }
                        Err(e) => problems.push(format!("{chip}:{label}: {e}")),
                    }
                }
            }
        }
    }
    for (chip, pad, ..) in CLASS_VOLTS {
        if !listed.iter().any(|(c, p)| c == chip && p == pad) {
            problems.push(format!(
                "{chip}:{pad}: anchored but in no descriptor pad class"
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    let chips: std::collections::BTreeSet<_> = listed.iter().map(|(c, _)| c.as_str()).collect();
    assert!(
        chips.len() >= 2,
        "pad classes must be proven on more than one chip family: {chips:?}"
    );
}
