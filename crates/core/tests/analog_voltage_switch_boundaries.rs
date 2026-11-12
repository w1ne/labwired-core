//! Zero-width contacts use a single predicate; hysteretic relays retain memory.
use labwired_core::analog::{
    parse_netlist, AnalogConfig, AnalogCosimAdapter, AnalogInitialization, Integration, Solver,
};
use labwired_core::cosim::{CosimAdapter, CosimSignals, CosimStep};
use std::collections::BTreeMap;

fn pair(control: f64, signed: bool, hysteresis: f64) -> String {
    format!("Vdd supply 0 5\nVctrl ctl 0 {control:.17}\nRno supply no 1k\nRnc supply nc 1k\nSno no 0 ctl 0 vt=1 vh={hysteresis} signed={} invert=0 ron=1 roff=1meg\nSnc nc 0 ctl 0 vt=1 vh={hysteresis} signed={} invert=1 ron=1 roff=1meg\n", u8::from(signed), u8::from(signed))
}
fn check(solver: &Solver, no_closed: bool, nc_closed: bool) {
    for (name, closed) in [("no", no_closed), ("nc", nc_closed)] {
        let r = if closed { 1.0 } else { 1e6 };
        let expected = 5.0 * r / (1000.0 + r);
        let actual = solver.node_voltage(solver.circuit().node(name).unwrap());
        assert!(
            (actual - expected).abs() < 1e-10,
            "{name}: {actual:e} != {expected:e}"
        );
    }
}
fn empty_state() -> AnalogInitialization {
    AnalogInitialization::TransientState {
        capacitor_voltages_v: BTreeMap::new(),
        inductor_currents_a: BTreeMap::new(),
    }
}

#[test]
fn adjacent_float_threshold_values_are_complementary_for_both_sensing_modes() {
    let below = f64::from_bits(1.0_f64.to_bits() - 1);
    let above = f64::from_bits(1.0_f64.to_bits() + 1);
    for signed in [false, true] {
        for v in [below, 1.0, above, -below, -1.0, -above] {
            let effective = if signed { v } else { v.abs() };
            let expected = effective >= 1.0;
            for initial in [AnalogInitialization::OperatingPoint, empty_state()] {
                let s = Solver::new_with_initialization(
                    parse_netlist(&pair(v, signed, 0.0)).unwrap(),
                    Integration::BackwardEuler,
                    32,
                    &initial,
                )
                .unwrap();
                check(&s, expected, !expected);
            }
        }
    }
}

#[test]
fn threshold_result_has_no_prior_state_or_repeated_solve_dependence() {
    for signed in [false, true] {
        let mut solver = Solver::new(
            parse_netlist(&pair(0.0, signed, 0.0)).unwrap(),
            Integration::BackwardEuler,
        )
        .unwrap();
        let source = solver.voltage_source_index("Vctrl").unwrap();
        for v in [0.5_f64, 1.0, 1.5, 1.0, 0.5, -1.0, -1.5, -1.0, 1.0] {
            solver.set_voltage_source(source, v);
            let effective = if signed { v } else { v.abs() };
            let expected = effective >= 1.0;
            for _ in 0..3 {
                solver.solve_operating_point().unwrap();
                check(&solver, expected, !expected);
                solver.advance(1e-7).unwrap();
                check(&solver, expected, !expected);
            }
        }
    }
}

#[test]
fn repeated_adapter_t0_reads_and_steps_keep_exact_equality_contacts_complementary() {
    for signed in [false, true] {
        for initial in [AnalogInitialization::OperatingPoint, empty_state()] {
            let cfg = AnalogConfig {
                initialization: initial,
                ..AnalogConfig::default()
            };
            let mut adapter =
                AnalogCosimAdapter::from_netlist(&pair(1.0, signed, 0.0), &cfg).unwrap();
            for time_ns in [0, 0, 0, 100, 100, 200] {
                adapter
                    .step(CosimStep {
                        time_ns,
                        dt_ns: if time_ns == 0 { 0 } else { 100 },
                        inputs: CosimSignals::new(),
                    })
                    .unwrap();
                check(adapter.solver(), true, false);
            }
        }
    }
}

#[test]
fn positive_hysteresis_keeps_existing_pickup_hold_and_dropout_boundaries() {
    for signed in [false, true] {
        let mut solver = Solver::new(
            parse_netlist(&pair(0.0, signed, 0.25)).unwrap(),
            Integration::BackwardEuler,
        )
        .unwrap();
        let source = solver.voltage_source_index("Vctrl").unwrap();
        for (v, no, nc) in [
            (0.0, false, true),
            (1.0, true, false),
            (0.9, true, false),
            (0.75, false, true),
            (0.9, false, true),
            (1.0, true, false),
        ] {
            solver.set_voltage_source(source, v);
            solver.solve_operating_point().unwrap();
            check(&solver, no, nc);
        }
    }
}

#[test]
fn zero_threshold_is_admitted_only_for_a_signed_zero_width_comparison() {
    let base = "Vdd supply 0 5\nVctrl ctl 0 0\nRno supply no 1k\n";
    for options in ["vt=0 signed=1 vh=0", "vt=-0 signed=1 vh=-0"] {
        let deck = format!("{base}Sno no 0 ctl 0 {options} ron=1 roff=1meg\n");
        assert!(parse_netlist(&deck).is_ok(), "{options}");
    }
    for options in [
        "vt=0",
        "vt=0 signed=0 vh=0",
        "vt=0 signed=1 vh=.1",
        "vt=0 signed=1 vh=-.1",
        "vt=-.1 signed=1 vh=0",
        "vt=nan signed=1 vh=0",
        "vt=inf signed=1 vh=0",
    ] {
        let deck = format!("{base}Sno no 0 ctl 0 {options} ron=1 roff=1meg\n");
        assert!(parse_netlist(&deck).is_err(), "accepted {options}");
    }
}

#[test]
fn signed_zero_threshold_contacts_are_complementary_at_both_signed_zeros() {
    let deck = pair(-0.1, true, 0.0).replace("vt=1", "vt=0");
    for initial in [AnalogInitialization::OperatingPoint, empty_state()] {
        let mut solver = Solver::new_with_initialization(
            parse_netlist(&deck).unwrap(),
            Integration::BackwardEuler,
            32,
            &initial,
        )
        .unwrap();
        let source = solver.voltage_source_index("Vctrl").unwrap();
        for v in [-1e-9, -0.0, 0.0, 1e-9, 0.0, -1e-9] {
            solver.set_voltage_source(source, v);
            for _ in 0..3 {
                solver.solve_operating_point().unwrap();
                check(&solver, v >= 0.0, v < 0.0);
                solver.advance(1e-7).unwrap();
                check(&solver, v >= 0.0, v < 0.0);
            }
        }
    }
}
