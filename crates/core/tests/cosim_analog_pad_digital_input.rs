// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! One solved node on one pad, read two ways: the same NTC divider drives
//! `board.analog.<pad>_volts` (the pad's ADC input) and `board.gpio_in.<pad>`
//! (its Schmitt input buffer). On these boards the pad is in no `pins:` list,
//! so the digital route exists only through the generic label rule
//! (`labwired_core::pins::parse_pad_label`): Kinetis `PT<port><n>`, Microchip
//! `R<port><n>`, and a CC2538 PL061 port that publishes no IDR offset.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use labwired_core::Bus;
use std::path::Path;

/// K64 GPIOB PDIR (K64 RM 55.2.6): the register firmware reads.
const K64_GPIOB_PDIR: u64 = 0x400F_F040 + 0x10;

fn ntc_resistance_ohms(temperature_c: f64) -> f64 {
    10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp()
}

fn divider_volts(temperature_c: f64) -> f64 {
    let ntc = ntc_resistance_ohms(temperature_c);
    3.3 * ntc / (10_000.0 + ntc)
}

fn ntc_on_one_pad(pad: &str) -> String {
    format!(
        r#"
- id: "ntc_divider"
  adapter: "analog"
  step_ns: 1000
  inputs:
    temperature: "ui.ntc.temperature_c"
  outputs:
    v_adc: "board.analog.{pad}_volts"
    v_gpio: "board.gpio_in.{pad}"
  config:
    netlist_text: |
      Vcc vcc 0 dc 3.3
      Rpull vcc sense 10k
      Rntc sense 0 10k beta=3950 tnom=25 temp=25
    probes:
      v_adc: "v(sense)"
      v_gpio: "v(sense)"
    sources:
      temperature: "Rntc"
    vdd: 3.3
"#
    )
}

/// The firmware-visible input level of `pad`, read from the PinPort the
/// test names (not through the resolver under test).
fn input_level(bus: &SystemBus, port: &str, pin: u8) -> bool {
    let index = bus
        .find_peripheral_index_by_name(port)
        .unwrap_or_else(|| panic!("no {port}"));
    bus.peripherals[index]
        .dev
        .pins()
        .and_then(|p| p.input(pin))
        .unwrap_or_else(|| panic!("{port} pin {pin} has no input"))
}

fn run_board(system: &str, pad: &str, port: &str, pin: u8, check: impl Fn(&SystemBus, bool)) {
    let system_path = root(system);
    let mut profile = SystemManifest::from_file(&system_path).expect("load board profile");
    profile.cosim_models = serde_yaml::from_str(&ntc_on_one_pad(pad)).expect("parse netlist");
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    assert!(
        !chip
            .pins
            .keys()
            .any(|label| label.eq_ignore_ascii_case(pad)),
        "{system}: {pad} must be absent from `pins:` for this test to prove the label rule"
    );
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("one model");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{system}: {pad} must bind as an ADC input AND as a digital input"
    );
    let mut cycles = 0;
    for (temperature_c, level) in [
        (-10.0_f64, true), // 2.82 V: above every VIH here
        (25.0, true),      // 1.65 V: inside the band, keeps high
        (80.0, false),     // 0.37 V: below every VIL here
        (25.0, false),     // inside the band, keeps low
        (-10.0, true),
    ] {
        session
            .set_signal_number("ui.ntc.temperature_c", temperature_c)
            .unwrap();
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session.advance_to(cycles, &mut bus).expect("solve");
        assert_eq!(steps.len(), 1);
        assert!(errors.is_empty(), "{system}: route errors {errors:?}");
        let solved = match session.signals().get(&format!("board.gpio_in.{pad}")) {
            Some(CosimSignalValue::F64(volts)) => *volts,
            other => panic!("{system}: no solved voltage on {pad}: {other:?}"),
        };
        assert!(
            (solved - divider_volts(temperature_c)).abs() < 1e-9,
            "{system} at {temperature_c} °C: Beta divider {} V, solved {solved} V",
            divider_volts(temperature_c)
        );
        assert_eq!(
            input_level(&bus, port, pin),
            level,
            "{system} {pad} at {temperature_c} °C ({solved:.3} V) must read {level}"
        );
        check(&bus, level);
    }
}

#[test]
fn frdm_k64f_ptb10_is_one_node_on_adc1_and_gpiob_pdir() {
    run_board(
        "configs/systems/frdm-k64f.yaml",
        "PTB10",
        "gpiob",
        10,
        |bus, level| {
            let pdir = bus.read_u32(K64_GPIOB_PDIR).expect("read GPIOB PDIR");
            assert_eq!(pdir & (1 << 10) != 0, level, "GPIOB_PDIR bit 10");
        },
    );
}

#[test]
fn dspic33fj32mc202_rb2_is_one_node_on_an4_and_portb() {
    run_board(
        "configs/systems/dspic33fj32mc202-demo.yaml",
        "RB2",
        "gpiob",
        2,
        |_, _| {},
    );
}

#[test]
fn cc2538dk_pa2_is_one_node_on_the_soc_adc_and_gpio_a() {
    run_board(
        "configs/systems/cc2538dk.yaml",
        "PA2",
        "gpioa",
        2,
        |_, _| {},
    );
}
