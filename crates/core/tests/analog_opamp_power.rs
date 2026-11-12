//! Optional OPAMP supply bounds are a functional envelope, not a silicon power model.
use labwired_core::analog::{
    parse_netlist, AnalogConfig, AnalogCosimAdapter, AnalogError, Integration, Solver,
};

fn voltage(s: &Solver, name: &str) -> f64 {
    s.node_voltage(s.circuit().node(name).unwrap())
}
fn current(s: &Solver, name: &str) -> f64 {
    s.branch_current(s.circuit().branch_index(name).unwrap())
}
fn solve(deck: &str) -> Result<Solver, AnalogError> {
    Solver::new(parse_netlist(deck)?, Integration::BackwardEuler)
}
fn follower(rail: f64, extra: &str, qualified: bool, load: bool) -> String {
    format!(
        ".model amp OPAMP(AOL=100k GBW=1meg ROUT=20.2 DROP_HI=.025 DROP_LO=.025 {extra} {})\nVrail rail 0 DC {rail}\nVin input 0 DC 1\nXamp input out rail 0 out amp\n{}",
        if qualified {"VMIN=2.5 VMAX=5.5"} else {""},
        if load {"Rload out 0 1k\n"} else {""},
    )
}

#[test]
fn supply_bounds_are_an_explicit_finite_ordered_pair() {
    for bounds in [
        "VMIN=2.5",
        "VMAX=5.5",
        "VMIN=0 VMAX=5.5",
        "VMIN=-1 VMAX=5.5",
        "VMIN=5.5 VMAX=2.5",
        "VMIN=nan VMAX=5.5",
        "VMIN=2.5 VMAX=inf",
        "VMIN=.04 VMAX=5.5",
    ] {
        let deck = follower(3.3, bounds, false, true);
        assert!(parse_netlist(&deck).is_err(), "accepted {bounds}");
    }
    assert!(parse_netlist(&follower(3.3, "", true, true)).is_ok());
    assert!(parse_netlist(&follower(2.5, "VMIN=2.5 VMAX=2.5", false, true)).is_ok());
}

#[test]
fn powered_unloaded_follower_is_not_rejected_from_the_zero_initial_guess() {
    let s = solve(&follower(3.3, "", true, false)).unwrap();
    assert!((voltage(&s, "out") - 1.0).abs() < 2e-5);
}

#[test]
fn an_unpowered_unconnected_output_remains_singular_without_an_artificial_shunt() {
    assert!(matches!(
        solve(&follower(0.0, "", true, false)),
        Err(AnalogError::Singular { .. })
    ));
}

#[test]
fn zero_supply_gates_active_paths_but_keeps_passive_input_resistances() {
    for extra in ["", "IOLIM=.02 SR=150meg"] {
        let deck=format!(".model amp OPAMP(AOL=100k GBW=100meg ROUT=20.2 VOS=.01 CMRR=60 IBIAS=1u IOS=.2u IQ=.005 RIN=100meg RINCM=10meg CINCM=1p CIND=2p DROP_HI=.025 DROP_LO=.025 VMIN=2.5 VMAX=5.5 {extra})\nVrail rail 0 0\nVin input 0 1\nVminus minus 0 0\nXamp input minus rail 0 out amp\nRload out 0 1k\n");
        let s = solve(&deck).unwrap();
        assert_eq!(voltage(&s, "out"), 0.0);
        assert!(current(&s, "Vrail").abs() < 1e-15);
        assert!((current(&s, "Vin") + 1.1e-7).abs() < 1e-15);
        for i in 0..s.circuit().diodes.len() {
            assert_eq!(s.diode_current(i), 0.0);
        }
    }
}

#[test]
fn unpowered_output_is_high_impedance_for_both_norton_and_iolim_stages() {
    for extra in ["", "IOLIM=.02 SR=150meg"] {
        let deck=format!(".model amp OPAMP(ROUT=20.2 VOS=.02 IQ=.01 IBIAS=1u DROP_HI=.025 DROP_LO=.025 VMIN=2.5 VMAX=5.5 {extra})\nVrail rail 0 0\nVin input 0 1\nXamp input 0 rail 0 out amp\nVprobe out 0 .3\n");
        let s = solve(&deck).unwrap();
        assert!(
            current(&s, "Vprobe").abs() < 1e-15,
            "{extra}: {}",
            current(&s, "Vprobe")
        );
        assert!(current(&s, "Vrail").abs() < 1e-15);
    }
}

#[test]
fn supply_envelope_is_signed_and_uses_the_difference_between_rails() {
    for rail in [-3.3, 5.5001] {
        let error = solve(&follower(rail, "", true, true))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("solved supply span"), "{rail}: {error}");
    }
    let deck=".model amp OPAMP(AOL=100k GBW=1meg ROUT=20.2 VMIN=2.5 VMAX=5.5)\nVplus plus 0 5.3\nVminus minus 0 2\nVin input 0 2.5\nXamp input out plus minus out amp\nRload out 0 1k\n";
    let s = solve(deck).unwrap();
    assert!((voltage(&s, "out") - 2.5).abs() < 1e-3);
}

#[test]
fn exact_minimum_and_maximum_are_on_and_the_lower_positive_span_is_off() {
    for rail in [0.0, 2.499999, 2.5, 5.5] {
        let s = solve(&follower(rail, "", true, true)).unwrap();
        if rail < 2.5 {
            assert!(voltage(&s, "out").abs() < 1e-12);
        } else {
            assert!((voltage(&s, "out") - 1.0).abs() < 1e-3);
        }
    }
}

#[test]
fn omission_preserves_nominal_legacy_results_bit_for_bit() {
    for extra in [
        "",
        "IQ=2u IBIAS=1n IOS=.1n VOS=.001 CMRR=80 RIN=100meg RINCM=100meg",
        "IOLIM=.01 SR=1meg",
    ] {
        let mut legacy = solve(&follower(3.3, extra, false, true)).unwrap();
        let mut qualified = solve(&follower(3.3, extra, true, true)).unwrap();
        for _ in 0..8 {
            for name in ["rail", "input", "out", "Xamp#p"] {
                assert_eq!(
                    voltage(&legacy, name).to_bits(),
                    voltage(&qualified, name).to_bits(),
                    "{extra}: {name}"
                );
            }
            assert_eq!(
                current(&legacy, "Vrail").to_bits(),
                current(&qualified, "Vrail").to_bits()
            );
            legacy.advance(1e-7).unwrap();
            qualified.advance(1e-7).unwrap();
        }
    }
}

#[test]
fn cold_ramp_qualifies_the_same_solved_boundary_without_stale_drive() {
    let deck = follower(0.0, "SR=150meg", true, true).replace(
        "Vrail rail 0 DC 0",
        "Vrail rail 0 PULSE(0 3.3 1u 2u 2u 10u 20u)",
    );
    let mut s = solve(&deck).unwrap();
    assert_eq!(voltage(&s, "out"), 0.0);
    for _ in 0..50 {
        s.advance(1e-7).unwrap();
        if voltage(&s, "rail") < 2.5 {
            assert!(voltage(&s, "out").abs() < 1e-12);
        }
    }
    assert!((voltage(&s, "out") - 1.0).abs() < 1e-3);
}

#[test]
fn power_loss_preserves_compensation_charge_and_its_passive_decay() {
    let deck = follower(3.3, "SR=150meg", true, true).replace("GBW=1meg", "GBW=1k");
    let mut s = solve(&deck).unwrap();
    let cap = s
        .circuit()
        .capacitors
        .iter()
        .position(|c| c.name == "Xamp#cg")
        .unwrap();
    let c = s.circuit().capacitors[cap].farads;
    let initial = s.capacitor_voltage(cap);
    assert!(initial > 0.9);
    s.set_voltage_source(s.voltage_source_index("Vrail").unwrap(), 0.0);
    let h = 1e-4;
    s.advance(h).unwrap();
    let expected = initial / (1.0 + h / (1e6 * c));
    assert!((s.capacitor_voltage(cap) - expected).abs() < 1e-12);
    assert!(voltage(&s, "out").abs() < 1e-12);
    for i in 0..s.circuit().diodes.len() {
        assert_eq!(s.diode_current(i), 0.0);
    }
    s.set_voltage_source(s.voltage_source_index("Vrail").unwrap(), 3.3);
    s.advance(1e-7).unwrap();
    assert!(
        voltage(&s, "out") > 0.5,
        "stored compensation state was replaced"
    );
}

#[test]
fn a_later_overvoltage_refuses_instead_of_publishing_an_off_state() {
    let mut s = solve(&follower(3.3, "", true, true)).unwrap();
    s.set_voltage_source(s.voltage_source_index("Vrail").unwrap(), 5.6);
    let error = s.advance(1e-7).unwrap_err().to_string();
    assert!(error.contains("solved supply span"), "{error}");
}

#[test]
fn independent_amplifiers_do_not_share_a_supply_gate() {
    let deck=".model amp OPAMP(AOL=100k VMIN=2.5 VMAX=5.5 DROP_HI=.025 DROP_LO=.025)\nVrail rail 0 3.3\nVin input 0 1\nXon input on rail 0 on amp\nXoff input off 0 0 off amp\nRon on 0 1k\nRoff off 0 1k\n";
    let s = solve(deck).unwrap();
    assert!((voltage(&s, "on") - 1.0).abs() < 1e-3);
    assert_eq!(voltage(&s, "off"), 0.0);
}

#[test]
fn routed_sources_cannot_rewrite_a_qualified_macro_primitive() {
    for element in ["Xamp#vhi", " XAMP#iq ", "Xamp#ro", "Xamp#rg", "Xamp#ibp"] {
        let cfg = AnalogConfig {
            sources: [("override".to_string(), element.to_string())].into(),
            ..AnalogConfig::default()
        };
        let error =
            AnalogCosimAdapter::from_netlist(&follower(3.3, "IQ=1u IBIAS=1n", true, true), &cfg)
                .err()
                .unwrap()
                .to_string();
        assert!(error.contains("one macro owner"), "{element}: {error}");
    }
    let cfg = AnalogConfig {
        sources: [("supply".to_string(), "Vrail".to_string())].into(),
        ..AnalogConfig::default()
    };
    assert!(AnalogCosimAdapter::from_netlist(&follower(3.3, "", true, true), &cfg).is_ok());
}

#[test]
fn no_self_consistent_active_set_refuses_instead_of_publishing_one_side() {
    // On draws the supply below VMIN; off lets it return above VMIN. No
    // hysteresis or artificial stable voltage is declared by this model.
    let deck = follower(2.6, "IQ=.0002", true, true).replace(
        "Vrail rail 0 DC 2.6",
        "Vrail source 0 DC 2.6\nRsource source rail 1k",
    );
    assert!(matches!(
        solve(&deck),
        Err(AnalogError::NoConvergence { .. })
    ));
}

#[test]
fn terminal_power_refusal_keeps_last_accepted_clamp_readback() {
    for (bad_supply, load) in [(5.6, true), (0.0, false)] {
        let deck = follower(3.3, "", true, load).replace("Vin input 0 DC 1", "Vin input 0 DC 10");
        let mut s = solve(&deck).unwrap();
        let diode = s
            .circuit()
            .diodes
            .iter()
            .position(|d| d.name == "Xamp#dhi")
            .unwrap();
        let before = s.diode_current(diode);
        let old_output = voltage(&s, "out");
        assert!(before > 1e-3);
        s.set_voltage_source(s.voltage_source_index("Vrail").unwrap(), bad_supply);
        assert!(s.advance(1e-7).is_err());
        assert_eq!(voltage(&s, "out").to_bits(), old_output.to_bits());
        assert_eq!(
            s.diode_current(diode).to_bits(),
            before.to_bits(),
            "refused {bad_supply} V changed accepted current"
        );
        s.set_voltage_source(s.voltage_source_index("Vrail").unwrap(), 3.3);
        s.advance(1e-7)
            .expect("a rejected trial must not poison a valid powered retry");
        assert!(s.diode_current(diode) > 1e-3);
    }
}
