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
    "nrf54lm20a",
    "r7fa6m5bh",
    "r7fa8m1ah",
    "ra4m1",
    "s32k388",
    "sam4s16c",
    "same70q21b",
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
];

/// Datasheet anchors for descriptors whose thresholds are absolute volts or
/// non-round ratios, so a transcription slip shows as a voltage, not a ratio:
/// (chip, VIL max volts, VIH min volts, source).
const DATASHEET_VOLTS: &[(&str, f64, f64, &str)] = &[
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
                return Ok(Outcome::Refused { key });
            }
            [RoutingError::UnknownPad { .. }] => continue,
            other => return Err(format!("{pad}: bind: {other:?}")),
        }
        let thresholds =
            InputThresholds::for_bus(&bus).map_err(|k| format!("bound without {k}"))?;
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
        return Ok(Outcome::Swept { pad, thresholds });
    }
    Ok(Outcome::NoPad)
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
