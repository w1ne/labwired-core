// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `.sens` DC sensitivity in the in-core engine.
//!
//! Every entry is checked twice. A central difference re-solves the operating
//! point with the value moved by a part in 10⁴, which proves the adjoint
//! result without any outside reference. ngspice-42 (`set numdgt=12`) gives
//! the second reference: a linear deck with every element kind `.sens`
//! reports, and a degenerated BJT stage read as a voltage and as a supply
//! current. ngspice perturbs each value by a part in 10⁶ and uses CODATA
//! 2014 `k` and `q`, so its tables hold to 1e-5.

mod analog {
    use labwired_core::analog::{parse_netlist, Integration, Solver, TransferOutput};

    fn build(netlist: &str) -> Solver {
        let circuit = parse_netlist(netlist).unwrap_or_else(|error| panic!("{error}"));
        let mut solver = Solver::new(circuit, Integration::BackwardEuler)
            .unwrap_or_else(|error| panic!("{error}"));
        solver
            .solve_operating_point()
            .unwrap_or_else(|error| panic!("{error}"));
        solver
    }

    fn assert_close(what: &str, got: f64, expected: f64, relative: f64) {
        let error = ((got - expected) / expected).abs();
        assert!(
            error <= relative,
            "{what}: expected {expected:e}, got {got:e} (relative error {error:e})"
        );
    }

    /// Hold every `(name, value)` of an ngspice-42 table; a `0.0` entry must
    /// be zero to `absolute`.
    fn assert_table(deck: &str, table: &[(&str, f64)], relative: f64, absolute: f64) {
        let sens = build(deck)
            .sensitivity()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            sens.entries.len(),
            table.len(),
            "entries {:?}",
            sens.entries
        );
        for &(name, expected) in table {
            let got = sens
                .get(name)
                .unwrap_or_else(|| panic!("no `{name}` in {:?}", sens.entries));
            if expected == 0.0 {
                assert!(got.abs() <= absolute, "{name}: expected 0, got {got:e}");
            } else {
                assert_close(name, got, expected, relative);
            }
        }
    }

    /// The output of `deck` at its operating point, read from a `.tf`-style
    /// port so the same probe serves the difference check.
    fn output(deck: &str, probe: &TransferOutput) -> f64 {
        let solver = build(deck);
        let circuit = solver.circuit();
        match probe {
            TransferOutput::Voltage { node, reference } => {
                let at = |name: &str| solver.node_voltage(circuit.node(name).unwrap());
                at(node) - reference.as_deref().map_or(0.0, at)
            }
            TransferOutput::Current(element) => {
                solver.branch_current(circuit.branch_index(element).unwrap())
            }
        }
    }

    /// The element value on `line`: its last plain number, and that
    /// token's position.
    fn value(line: &str) -> (usize, f64) {
        line.split(' ')
            .enumerate()
            .filter_map(|(at, token)| token.parse().ok().map(|value| (at, value)))
            .last()
            .unwrap()
    }

    /// Move the value on `line` by `factor` and re-solve.
    fn perturbed(deck: &str, line: &str, factor: f64, probe: &TransferOutput) -> f64 {
        let (position, value) = value(line);
        let mut tokens: Vec<String> = line.split(' ').map(str::to_string).collect();
        tokens[position] = format!("{:.17e}", value * factor);
        assert_eq!(deck.matches(line).count(), 1, "`{line}` is not unique");
        output(&deck.replace(line, &tokens.join(" ")), probe)
    }

    /// Hold the named entry to a central difference about `line`'s value.
    fn assert_difference(deck: &str, probe: &TransferOutput, name: &str, line: &str) {
        const STEP: f64 = 1e-4;
        let (_, value) = value(line);
        let slope = (perturbed(deck, line, 1.0 + STEP, probe)
            - perturbed(deck, line, 1.0 - STEP, probe))
            / (2.0 * STEP * value);
        let sens = build(deck)
            .sensitivity_analysis(probe)
            .unwrap_or_else(|error| panic!("{error}"));
        let got = sens.get(name).unwrap();
        assert_close(name, got, slope, 1e-6);
    }

    fn v(node: &str) -> TransferOutput {
        TransferOutput::Voltage {
            node: node.to_string(),
            reference: None,
        }
    }

    /// Every element kind `.sens` reports, in one linear network.
    const LINEAR: &str = "* every .sens element kind\n\
                          V1 in 0 DC 1\n\
                          R1 in out 1000\n\
                          R2 out 0 3000\n\
                          R3 out x 500\n\
                          I1 0 x DC 0.001\n\
                          E1 y 0 out 0 2\n\
                          R4 y z 1000\n\
                          G1 0 z out 0 0.001\n\
                          R5 z 0 2000\n\
                          VA z za 0\n\
                          RA za 0 5000\n\
                          F1 0 z VA 0.5\n\
                          H1 h 0 VA 2000\n\
                          RH h z 3000\n\
                          .sens v(z)\n";

    #[test]
    fn a_linear_network_matches_ngspice() {
        // R3 carries the I1 current but sits in series with it, so v(z)
        // does not depend on it: ngspice prints -0.
        assert_table(
            LINEAR,
            &[
                ("v1", 1.249999999897e+00),
                ("r1", 6.249993750351e-04),
                ("r2", 2.083331249891e-04),
                ("r3", 0.0),
                ("i1", 1.249999999884e+03),
                ("e1_gain", 8.333333332648e-01),
                ("r4", -2.77777500016e-04),
                ("g1_gain", 8.333333332561e+02),
                ("r5", 3.472218750195e-04),
                ("va", -1.85185185185e-02),
                ("ra", -9.25925000027e-06),
                ("f1_gain", 2.777777777549e-01),
                ("h1_gain", 9.259259259040e-05),
                ("rh", 9.259249999515e-05),
            ],
            1e-5,
            1e-15,
        );
    }

    #[test]
    fn every_linear_entry_matches_a_central_difference() {
        let deck = LINEAR.replace(".sens v(z)\n", "");
        let probe = v("z");
        for (name, line) in [
            ("v1", "V1 in 0 DC 1"),
            ("r1", "R1 in out 1000"),
            ("r2", "R2 out 0 3000"),
            ("i1", "I1 0 x DC 0.001"),
            ("e1_gain", "E1 y 0 out 0 2"),
            ("r4", "R4 y z 1000"),
            ("g1_gain", "G1 0 z out 0 0.001"),
            ("r5", "R5 z 0 2000"),
            ("ra", "RA za 0 5000"),
            ("f1_gain", "F1 0 z VA 0.5"),
            ("h1_gain", "H1 h 0 VA 2000"),
            ("rh", "RH h z 3000"),
        ] {
            assert_difference(&deck, &probe, name, line);
        }
    }

    const BJT: &str = "* degenerated BJT common-emitter stage\n\
                       .options temp=26.85 tnom=26.85\n\
                       VCC vcc 0 DC 12\n\
                       VB b0 0 DC 2\n\
                       RB b0 b 10000\n\
                       RC vcc c 4700\n\
                       RE e 0 1000\n\
                       Q1 c b e qn\n\
                       .model qn npn is=1e-14 bf=150 vaf=80 ise=1e-15 ikf=0.05 \
                       rb=50 rc=2 re=0.5\n";

    #[test]
    fn a_degenerated_bjt_stage_matches_ngspice() {
        // ngspice-42 also prints the `q1:` model and `q1_` instance entries,
        // which this engine does not report.
        assert_table(
            &format!("{BJT}.sens v(c)\n"),
            &[
                ("vcc", 9.940140823639e-01),
                ("vb", -4.26041132955e+00),
                ("rb", 3.462805920165e-05),
                ("rc", -1.24224039213e-03),
                ("re", 5.366483425667e-03),
            ],
            1e-5,
            0.0,
        );
    }

    #[test]
    fn a_bjt_supply_current_matches_ngspice() {
        // `rc` is the Early effect alone, a part in 10³ of `re`; ngspice's
        // part-in-10⁶ perturbation leaves it good to about 1e-4.
        assert_table(
            &format!("{BJT}.sens i(vcc)\n"),
            &[
                ("vcc", -1.27359949528e-06),
                ("vb", -9.06470495649e-04),
                ("rb", 7.367672170563e-09),
                ("rc", 1.591644187438e-09),
                ("re", 1.141804984184e-06),
            ],
            2e-4,
            0.0,
        );
    }

    #[test]
    fn every_bjt_entry_matches_a_central_difference() {
        for probe in [v("c"), TransferOutput::Current("VCC".to_string())] {
            for (name, line) in [
                ("vcc", "VCC vcc 0 DC 12"),
                ("vb", "VB b0 0 DC 2"),
                ("rb", "RB b0 b 10000"),
                ("rc", "RC vcc c 4700"),
                ("re", "RE e 0 1000"),
            ] {
                assert_difference(BJT, &probe, name, line);
            }
        }
    }

    #[test]
    fn a_temperature_law_scales_with_the_nominal_resistance() {
        // R1 is 1k at 25 °C and 1.1k at 125 °C: the divider output's slope
        // per nominal ohm is the slope per actual ohm times 1.1.
        let deck = "V1 in 0 DC 1\n\
                    R1 in out 1000 tc1=0.001\n\
                    R2 out 0 1000\n\
                    .options temp=125 tnom=25\n\
                    .sens v(out)\n";
        let sens = build(deck).sensitivity().unwrap();
        let (r1, r2) = (1100.0, 1000.0);
        let per_ohm = -r2 / ((r1 + r2) * (r1 + r2));
        assert_close("r1", sens.get("r1").unwrap(), per_ohm * 1.1, 1e-12);
        assert_difference(
            &deck.replace(".sens v(out)\n", ""),
            &v("out"),
            "r1",
            "R1 in out 1000 tc1=0.001",
        );
    }

    #[test]
    fn the_transient_after_sens_is_bit_identical() {
        let deck = format!("{BJT}VS b0 b1 SIN(0 0.1 1k)\n");
        let deck = deck.replace("RB b0 b 10000", "RB b1 b 10000");
        let run = |sens: bool| {
            let mut solver = build(&deck);
            if sens {
                solver.sensitivity_analysis(&v("c")).unwrap();
            }
            (0..50)
                .map(|_| {
                    solver.advance(1e-5).unwrap();
                    let c = solver.circuit().node("c").unwrap();
                    solver.node_voltage(c).to_bits()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(false), run(true));
    }

    #[test]
    fn malformed_and_unresolvable_cards_are_refused() {
        for card in [
            ".sens",
            ".sens out",
            ".sens v(c) extra",
            ".sens i(vcc,vb)",
            ".sens v(c) ac dec 10 1",
        ] {
            let deck = format!("{BJT}{card}\n");
            assert!(parse_netlist(&deck).is_err(), "`{card}` parsed");
        }
        let deck = format!("{BJT}.sens v(c)\n.sens v(b)\n");
        assert!(parse_netlist(&deck).is_err(), "two cards parsed");
        let mut solver = build(BJT);
        assert!(solver.sensitivity().is_err(), "no card");
        for probe in [
            v("nowhere"),
            TransferOutput::Current("RC".to_string()),
            TransferOutput::Voltage {
                node: "c".to_string(),
                reference: Some("c".to_string()),
            },
        ] {
            assert!(solver.sensitivity_analysis(&probe).is_err(), "{probe:?}");
        }
    }
}
