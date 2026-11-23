// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Numerically stable diode series resistance must preserve the device equations,
//! exposed junctions, source currents and transient storage at unchanged limits.

use labwired_core::analog::{parse_netlist, AnalogError, Integration, Solver, MAX_UNKNOWNS};

const SCHOTTKY: &str =
    ".model DS D (IS=9.90n RS=14.0m BV=60 IBV=500u CJO=464p M=0.333 N=0.775 TT=7.20n)\n";

fn voltage(solver: &Solver, node: &str) -> f64 {
    solver.node_voltage(solver.circuit().node(node).expect("declared node"))
}

fn source_current(solver: &Solver, name: &str) -> f64 {
    solver.branch_current(
        solver
            .circuit()
            .branch_index(name)
            .expect("declared source"),
    )
}

fn close(actual: f64, expected: f64, tolerance: f64, label: &str) {
    assert!(
        actual.is_finite() && (actual - expected).abs() <= tolerance,
        "{label}: expected {expected:.17e}, got {actual:.17e}, tolerance {tolerance:.3e}"
    );
}

#[test]
fn low_series_resistance_at_high_common_mode_matches_independent_dc_root() {
    let netlist = format!("V1 supply 0 dc 24\nD1 drain supply DS\nR1 drain 0 1e12\n{SCHOTTKY}");
    let circuit = parse_netlist(&netlist).expect("three-element deck parses");
    assert_eq!(
        circuit.unknowns(),
        4,
        "RS retains its one existing internal node and no extra branch"
    );
    assert_eq!(
        circuit.branch_index("D1"),
        None,
        "i(D) must keep the device-law probe path"
    );
    let solver = Solver::new(circuit, Integration::BackwardEuler)
        .expect("finite-resistance leakage must solve at the unchanged Newton limits");

    // Independent 70-decimal-digit root of:
    // j=a*(1+0.014/1e12),
    // 9.9e-9*expm1((j-24)/(0.775*k*300/q)) + 1e-12*(j-24) + a/1e12 = 0.
    // k=1.380649e-23 and q=1.602176634e-19 are exact constants.
    let expected_anode = 23.999_951_370_804_566_f64;
    let expected_junction = 23.999_951_370_804_9_f64;
    let expected_current = -2.399_995_137_080_456_6e-11_f64;
    close(
        voltage(&solver, "drain"),
        expected_anode,
        1e-8,
        "external anode",
    );
    close(
        voltage(&solver, "D1#internal"),
        expected_junction,
        1e-8,
        "retained junction",
    );
    close(
        source_current(&solver, "V1"),
        expected_current,
        1e-15,
        "source leakage",
    );
    close(
        solver.diode_current(0),
        expected_current,
        1e-15,
        "diode conduction",
    );
    let load = voltage(&solver, "drain") / 1e12;
    close(
        source_current(&solver, "V1") + load,
        0.0,
        1e-15,
        "source/load KCL",
    );
    close(solver.diode_current(0) + load, 0.0, 1e-15, "anode KCL");
}

fn solve(netlist: &str, integration: Integration) -> Solver {
    Solver::new(parse_netlist(netlist).expect("fixture parses"), integration)
        .expect("fixture has a DC operating point")
}

fn diode_current(solver: &Solver, name: &str) -> f64 {
    let index = solver
        .circuit()
        .diodes
        .iter()
        .position(|d| d.name == name)
        .unwrap();
    solver.diode_current(index)
}

fn compare_nodes(actual: &Solver, reference: &Solver, nodes: &[(&str, &str)]) {
    for &(actual_name, reference_name) in nodes {
        close(
            voltage(actual, actual_name),
            voltage(reference, reference_name),
            1e-8,
            actual_name,
        );
    }
}

fn compare_sources(actual: &Solver, reference: &Solver, names: &[&str]) {
    for name in names {
        close(
            source_current(actual, name),
            source_current(reference, name),
            1e-10,
            name,
        );
        assert_eq!(
            actual.circuit().branch_index(name),
            reference.circuit().branch_index(name)
        );
    }
}

#[test]
fn low_series_resistance_with_an_unloaded_off_mosfet_keeps_leakage_kcl() {
    let netlist = format!("V1 supply 0 dc 24\nVgate gate 0 dc 0\nM1 drain gate 0 0 NM\nD1 drain supply DS\nRpull drain 0 1e12\n.model NM NMOS (VTO=2.1 KP=43.85965 LAMBDA=0 W=1e-4 L=1e-4)\n{SCHOTTKY}");
    let solver = solve(&netlist, Integration::BackwardEuler);
    let drain = voltage(&solver, "drain");
    assert!((23.99..24.0).contains(&drain), "unloaded drain {drain}");
    close(voltage(&solver, "gate"), 0.0, 1e-12, "off gate");
    let supply = source_current(&solver, "V1");
    assert!(
        (-1e-8..0.0).contains(&supply),
        "off-state source current {supply}"
    );
    close(supply, diode_current(&solver, "D1"), 1e-15, "cathode KCL");
}

#[test]
fn diode_storage_and_exposed_junction_branch_match_an_explicit_resistor() {
    let model = "Vdrive in 0 dc 0\nRdrive in a 100\nD1 a 0 DMOD\nVtap D1#internal tap dc 0\nRtap tap 0 1000\nCtap tap 0 220p\n.model DMOD D (IS=1e-14 RS=47 CJO=100p VJ=.8 M=.5 FC=.5 TT=100n VP=.2)\n";
    let reference = "Vdrive in 0 dc 0\nRdrive in a 100\nRseries a j 47\nD1 j 0 DMOD\nVtap j tap dc 0\nRtap tap 0 1000\nCtap tap 0 220p\n.model DMOD D (IS=1e-14 RS=0 CJO=100p VJ=.8 M=.5 FC=.5 TT=100n VP=.2)\n";
    for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
        let mut actual = solve(model, integration);
        let mut explicit = solve(reference, integration);
        assert_eq!(actual.circuit().unknowns(), explicit.circuit().unknowns());
        assert_eq!(actual.circuit().branch_index("D1"), None);
        let mut largest_storage_current = 0.0_f64;
        for input in [2.0, 2.0, 2.0, 2.0, -1.0, -1.0, -1.0, 1.0, 1.0, 0.0] {
            // Vdrive is the first voltage source in both independently authored decks.
            actual.set_voltage_source(0, input);
            explicit.set_voltage_source(0, input);
            actual.advance(10e-9).expect("model transient solves");
            explicit
                .advance(10e-9)
                .expect("explicit-R transient solves");
            compare_nodes(
                &actual,
                &explicit,
                &[("a", "a"), ("D1#internal", "j"), ("tap", "tap")],
            );
            compare_sources(&actual, &explicit, &["Vdrive", "Vtap"]);
            let conduction = diode_current(&actual, "D1");
            close(
                conduction,
                diode_current(&explicit, "D1"),
                1e-10,
                "diode conduction parity",
            );
            // i(D) is the junction conduction plus its fixed GMIN. It excludes
            // the explicitly nonzero depletion/diffusion storage current.
            let junction = voltage(&actual, "D1#internal");
            let thermal = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;
            let independent_conduction = 1e-14 * (junction / thermal).exp_m1() + 1e-12 * junction;
            close(
                conduction,
                independent_conduction,
                1e-10,
                "independent conduction-only oracle",
            );
            let storage =
                -source_current(&actual, "Vdrive") - source_current(&actual, "Vtap") - conduction;
            largest_storage_current = largest_storage_current.max(storage.abs());
        }
        assert!(
            largest_storage_current > 1e-5,
            "{integration:?} must exercise diode charge, observed {largest_storage_current} A"
        );
    }
}

#[test]
fn chained_internal_nodes_keep_external_connections_and_source_rows_in_either_order() {
    let models = ".model DM1 D (IS=1e-14 RS=47)\n.model DM2 D (IS=3e-14 N=1.1 RS=33)\n";
    let reference = "Vdrive in 0 dc 2\nRdrive in a 100\nRseries1 a j1 47\nD1 j1 0 DM1\nRseries2 j1 j2 33\nD2 j2 0 DM2\nVtap j2 tap dc 0\nRtap tap 0 2000\nVbias bias 0 dc .2\nRbias j1 bias 330\n.model DM1 D (IS=1e-14 RS=0)\n.model DM2 D (IS=3e-14 N=1.1 RS=0)\n";
    let explicit = solve(reference, Integration::BackwardEuler);
    for diodes in [
        "D1 a 0 DM1\nD2 D1#internal 0 DM2\n",
        "D2 D1#internal 0 DM2\nD1 a 0 DM1\n",
    ] {
        let netlist = format!("Vdrive in 0 dc 2\nRdrive in a 100\n{diodes}Vtap D2#internal tap dc 0\nRtap tap 0 2000\nVbias bias 0 dc .2\nRbias D1#internal bias 330\n{models}");
        let actual = solve(&netlist, Integration::BackwardEuler);
        compare_nodes(
            &actual,
            &explicit,
            &[
                ("a", "a"),
                ("D1#internal", "j1"),
                ("D2#internal", "j2"),
                ("tap", "tap"),
            ],
        );
        compare_sources(&actual, &explicit, &["Vdrive", "Vtap", "Vbias"]);
        assert_eq!(actual.circuit().unknowns(), explicit.circuit().unknowns());
        assert!(source_current(&actual, "Vtap").abs() > 1e-4);
        close(
            voltage(&actual, "a") - voltage(&actual, "D1#internal"),
            -47.0 * source_current(&actual, "Vdrive"),
            1e-8,
            "first series KVL",
        );
        close(
            voltage(&actual, "D1#internal") - voltage(&actual, "D2#internal"),
            33.0 * (diode_current(&actual, "D2") + source_current(&actual, "Vtap")),
            1e-8,
            "second series KVL",
        );
        close(
            source_current(&actual, "Vdrive")
                + source_current(&actual, "Vbias")
                + diode_current(&actual, "D1")
                + diode_current(&actual, "D2")
                + voltage(&actual, "tap") / 2000.0,
            0.0,
            1e-10,
            "complete external KCL",
        );
    }
}

#[test]
fn self_aliased_junction_preserves_the_physically_shorted_series_resistor() {
    let model = "V1 in 0 dc 1\nR1 in D1#internal 1000\nD1 D1#internal 0 DMOD\n.model DMOD D (IS=1e-14 RS=47)\n";
    let reference = "V1 in 0 dc 1\nR1 in j 1000\nD1 j 0 DMOD\n.model DMOD D (IS=1e-14 RS=0)\n";
    let actual = solve(model, Integration::BackwardEuler);
    let explicit = solve(reference, Integration::BackwardEuler);
    compare_nodes(&actual, &explicit, &[("D1#internal", "j")]);
    compare_sources(&actual, &explicit, &["V1"]);
    close(
        diode_current(&actual, "D1"),
        diode_current(&explicit, "D1"),
        1e-10,
        "self-alias conduction",
    );
}

#[test]
fn cyclic_internal_nodes_preserve_both_physical_resistors_in_either_order() {
    let reference = "V1 in 0 dc 2\nRdrive in j1 100\nRseries1 j2 j1 47\nRseries2 j1 j2 33\nD1 j1 0 DM1\nD2 j2 0 DM2\nVtap j2 tap dc 0\nRtap tap 0 2000\n.model DM1 D (IS=1e-14 RS=0)\n.model DM2 D (IS=3e-14 N=1.1 RS=0)\n";
    let explicit = solve(reference, Integration::BackwardEuler);
    for diodes in [
        "D1 D2#internal 0 DM1\nD2 D1#internal 0 DM2\n",
        "D2 D1#internal 0 DM2\nD1 D2#internal 0 DM1\n",
    ] {
        let netlist = format!("V1 in 0 dc 2\nRdrive in D1#internal 100\n{diodes}Vtap D2#internal tap dc 0\nRtap tap 0 2000\n.model DM1 D (IS=1e-14 RS=47)\n.model DM2 D (IS=3e-14 N=1.1 RS=33)\n");
        let actual = solve(&netlist, Integration::BackwardEuler);
        compare_nodes(
            &actual,
            &explicit,
            &[("D1#internal", "j1"), ("D2#internal", "j2"), ("tap", "tap")],
        );
        compare_sources(&actual, &explicit, &["V1", "Vtap"]);
        for name in ["D1", "D2"] {
            close(
                diode_current(&actual, name),
                diode_current(&explicit, name),
                1e-10,
                name,
            );
        }
    }
}

#[test]
fn grounded_anode_preserves_the_series_drop_and_source_current() {
    let model = "V1 cathode 0 dc 5\nD1 0 cathode DMOD\n.model DMOD D (IS=1n RS=47)\n";
    let reference =
        "V1 cathode 0 dc 5\nRseries 0 j 47\nD1 j cathode DMOD\n.model DMOD D (IS=1n RS=0)\n";
    let actual = solve(model, Integration::BackwardEuler);
    let explicit = solve(reference, Integration::BackwardEuler);
    compare_nodes(&actual, &explicit, &[("D1#internal", "j")]);
    compare_sources(&actual, &explicit, &["V1"]);
    close(
        voltage(&actual, "D1#internal"),
        -47.0 * diode_current(&actual, "D1"),
        1e-12,
        "grounded-anode Ohm law",
    );
}

#[test]
fn diode_series_resistance_uses_no_new_unknowns_and_preserves_the_capacity_guard() {
    let zero = parse_netlist("V1 a 0 dc .5\nD1 a 0 DMOD\n.model DMOD D (RS=0)\n").unwrap();
    assert_eq!(zero.unknowns(), 2);
    assert_eq!(zero.node("D1#internal"), None);
    assert_eq!(zero.branch_index("V1"), Some(0));
    Solver::new(zero, Integration::BackwardEuler).expect("zero-RS behavior remains supported");

    let mut netlist =
        "V1 in 0 dc 1\nRdrive in a 1000\nD1 a 0 DMOD\n.model DMOD D (RS=47)\n".to_owned();
    let initial = parse_netlist(&netlist).unwrap().unknowns();
    for index in initial..MAX_UNKNOWNS {
        netlist.push_str(&format!("Rextra{index} extra{index} 0 1000\n"));
    }
    let exact = parse_netlist(&netlist).unwrap();
    assert_eq!(
        MAX_UNKNOWNS, 128,
        "this numerical repair does not raise capacity"
    );
    assert_eq!(exact.unknowns(), MAX_UNKNOWNS);
    assert_eq!(exact.branch_count(), 1, "RS does not add a branch unknown");
    Solver::new(exact, Integration::BackwardEuler)
        .expect("a diode circuit at the existing limit solves");
    netlist.push_str("Roverflow overflow 0 1000\n");
    let too_large = Solver::new(parse_netlist(&netlist).unwrap(), Integration::BackwardEuler);
    assert!(
        matches!(too_large, Err(AnalogError::TooLarge { unknowns, max }) if unknowns == MAX_UNKNOWNS + 1 && max == MAX_UNKNOWNS)
    );
}
