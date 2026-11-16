// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A `LAPLACE` source between a sensor and an ADC pad, end to end through
//! the co-simulation router: an NTC divider feeds a critically damped
//! anti-alias filter `1/(1 + s·tau)²` written as `H(s)`, and the filter
//! output drives `board.analog.<pad>_volts`. After a temperature step the
//! pad must follow the filter's closed-form step response
//! `1 − (1 + t/tau)·e^(−t/tau)` co-sim step by co-sim step, not jump to the
//! divider voltage. The same netlist runs on two unrelated chips' ADC pads.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::{CosimSession, CosimSignalValue, RoutingError};
use std::path::Path;

/// Filter time constant; one co-sim step is 1 us.
const TAU_S: f64 = 20e-6;

fn divider_volts(temperature_c: f64) -> f64 {
    let ntc = 10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp();
    3.3 * ntc / (10_000.0 + ntc)
}

fn filtered_ntc_on(pad: &str) -> String {
    format!(
        r#"
- id: "ntc_filter"
  adapter: "analog"
  step_ns: 1000
  inputs:
    temperature: "ui.ntc.temperature_c"
  outputs:
    v_adc: "board.analog.{pad}_volts"
  config:
    substeps: 10
    integration: trap
    netlist_text: |
      .param tau={TAU_S}
      Vcc vcc 0 dc 3.3
      Rpull vcc sense 10k
      Rntc sense 0 10k beta=3950 tnom=25 temp=25
      Eaa out 0 LAPLACE {{V(sense)}} = {{1/(1+s*tau)^2}}
      Rload out 0 100k
    probes:
      v_adc: "v(out)"
    sources:
      temperature: "Rntc"
    vdd: 3.3
"#
    )
}

fn filter_follows_closed_form_on(system: &str, pad: &str) {
    let system_path = root(system);
    let mut profile = SystemManifest::from_file(&system_path).expect("load board profile");
    profile.cosim_models = serde_yaml::from_str(&filtered_ntc_on(pad)).expect("parse model");
    let chip_path = system_path.parent().unwrap().join(&profile.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    let mut bus = SystemBus::from_config(&chip, &profile).expect("build bus");
    let mut session = CosimSession::new(&profile.cosim_models, Path::new("."), &bus)
        .expect("build analog co-simulation")
        .expect("one model");
    assert_eq!(
        session.binding_errors(),
        &[] as &[RoutingError],
        "{system}: {pad} must bind as an ADC input"
    );
    let signal = format!("board.analog.{pad}_volts");
    let mut cycles = 0;
    let mut step = |session: &mut CosimSession, bus: &mut SystemBus| {
        cycles += session.cycles_until_boundary(cycles);
        let (steps, errors) = session.advance_to(cycles, bus).expect("solve");
        assert_eq!(steps.len(), 1);
        assert!(errors.is_empty(), "{system}: route errors {errors:?}");
        match session.signals().get(&signal) {
            Some(CosimSignalValue::F64(volts)) => *volts,
            other => panic!("{system}: no solved voltage on {pad}: {other:?}"),
        }
    };

    session
        .set_signal_number("ui.ntc.temperature_c", 25.0)
        .unwrap();
    let start = step(&mut session, &mut bus);
    assert!(
        (start - divider_volts(25.0)).abs() < 1e-9,
        "{system}: the operating point settles the filter at the divider, got {start} V"
    );

    session
        .set_signal_number("ui.ntc.temperature_c", -10.0)
        .unwrap();
    let swing = divider_volts(-10.0) - divider_volts(25.0);
    for n in 1..=100u32 {
        let got = step(&mut session, &mut bus);
        let t = f64::from(n) * 1e-6;
        let x = t / TAU_S;
        let expected = divider_volts(25.0) + swing * (1.0 - (1.0 + x) * (-x).exp());
        // 10 trapezoidal substeps of 0.1 us against tau = 20 us.
        assert!(
            (got - expected).abs() < 1e-4 * swing.abs(),
            "{system} {pad} at {t:e} s: closed form {expected} V, solved {got} V"
        );
        if n == 1 {
            assert!(
                (got - divider_volts(25.0)).abs() < 0.01 * swing.abs(),
                "{system}: one step after the temperature step the filter has barely \
                 moved; a pad at {got} V means the LAPLACE source was bypassed"
            );
        }
    }
}

#[test]
fn frdm_k64f_adc_pad_sees_a_laplace_anti_alias_filter_step() {
    filter_follows_closed_form_on("configs/systems/frdm-k64f.yaml", "PTB10");
}

#[test]
fn dspic33fj32mc202_adc_pad_sees_a_laplace_anti_alias_filter_step() {
    filter_follows_closed_form_on("configs/systems/dspic33fj32mc202-demo.yaml", "RB2");
}
