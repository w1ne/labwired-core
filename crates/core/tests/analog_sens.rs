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
//! 2014 `k` and `q`, so its tables hold to 1e-5. The `<device>:<param>`
//! model entries are held the same two ways: ngspice-42 for `D`, `Q` and `M`
//! (it has no `J` sensitivity, and no `D` `rs` entry), and a central
//! difference of the `.model` card itself.

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
        // ngspice-42 also prints the `q1:` parameters the card leaves at
        // their defaults, and `q1_areab`/`q1_areac`, which this engine does
        // not report. The `q1:` and `q1_` values are ngspice central
        // differences of `.op` (see `MODEL_ORACLE`).
        assert_table(
            &format!("{BJT}.sens v(c)\n"),
            &[
                ("vcc", 9.940140823639e-01),
                ("vb", -4.26041132955e+00),
                ("rb", 3.462805920165e-05),
                ("rc", -1.24224039213e-03),
                ("re", 5.366483425667e-03),
                ("q1:is", -1.110651226854e+13),
                ("q1:bf", -2.543674740233e-03),
                ("q1:vaf", 3.146687254096e-04),
                ("q1:ise", 1.155393083430e+12),
                ("q1:ikf", -2.285280099379e-01),
                ("q1:rb", 3.462803501719e-05),
                ("q1:rc", 7.480718400288e-06),
                ("q1:re", 5.366519735261e-03),
                ("q1_area", -1.257657456843e-01),
                ("q1_m", -1.257659364207e-01),
                ("q1_temp", -7.504881456377e-03),
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
                ("q1:is", -2.363087715090e+09),
                ("q1:bf", -5.412073916698e-07),
                ("q1:vaf", 6.695079312182e-08),
                ("q1:ise", 2.458282991231e+08),
                ("q1:ikf", -4.862297999127e-05),
                ("q1:rb", 7.367667001663e-09),
                ("q1:rc", 1.591642250398e-09),
                ("q1:re", 1.141812701246e-06),
                ("q1_area", -2.675866929945e-05),
                ("q1_m", -2.675870984970e-05),
                ("q1_temp", -1.596783288666e-06),
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

    // MODEL_ORACLE: ngspice-42's `.sens` moves a model parameter by a
    // one-sided part in 10⁶, which leaves `q1:vaf` 4e-5 and `q1:ise` 8e-4
    // off its own central difference. The `<device>:<param>` tables are
    // therefore ngspice central differences of `.op` (`set numdgt=15`,
    // `reltol=1e-12 abstol=1e-18 vntol=1e-15` so that Newton noise does not
    // swamp a weak parameter, the parameter moved by a part in 10⁴ each
    // way), with each instance on its own copy of the card when two share
    // one. The diode deck's `v1` and `r1` come the same way. `q1:rc` acts
    // through the Early effect alone and moves the outputs by parts in
    // 10¹⁰, below what even that ngspice difference resolves, so its two
    // values use a 2 % step (1 % gives the same to 1e-6; `rc` is near
    // linear there).

    /// Move `param=` on the deck's `.model` line by `factor` and re-solve.
    fn card_moved(deck: &str, card: &str, param: &str, factor: f64, probe: &TransferOutput) -> f64 {
        assert_eq!(deck.matches(card).count(), 1, "`{card}` is not unique");
        let prefix = format!("{param}=");
        let moved: Vec<String> = card
            .split(' ')
            .map(|token| match token.strip_prefix(&prefix) {
                Some(raw) => format!("{prefix}{:.17e}", raw.parse::<f64>().unwrap() * factor),
                None => token.to_string(),
            })
            .collect();
        output(&deck.replace(card, &moved.join(" ")), probe)
    }

    /// The central difference of the output with `param=` on `card` moved
    /// for every instance of the card at once, per unit of the parameter.
    fn card_slope(deck: &str, card: &str, param: &str, probe: &TransferOutput) -> f64 {
        const STEP: f64 = 1e-4;
        let value: f64 = card
            .split(' ')
            .find_map(|token| token.strip_prefix(&format!("{param}=")))
            .unwrap()
            .parse()
            .unwrap();
        (card_moved(deck, card, param, 1.0 + STEP, probe)
            - card_moved(deck, card, param, 1.0 - STEP, probe))
            / (2.0 * STEP * value)
    }

    fn model_entry(deck: &str, probe: &TransferOutput, name: &str) -> f64 {
        let sens = build(deck)
            .model_sensitivity_analysis(probe)
            .unwrap_or_else(|error| panic!("{error}"));
        sens.get(name)
            .unwrap_or_else(|| panic!("no `{name}` in {:?}", sens.entries))
    }

    const MOS: &str = "* level-1 NMOS with a drain resistor\n\
                       VDD d 0 DC 5\n\
                       VG g 0 DC 2\n\
                       RD d dr 10000\n\
                       M1 dr g 0 0 nm w=10u l=2u\n\
                       .model nm nmos level=1 vto=0.8 kp=50u lambda=0.02 gamma=0.4 phi=0.65\n";

    #[test]
    fn a_mosfet_stage_matches_ngspice() {
        // The bulk is on the source, so `gamma` and `phi` do not move the
        // threshold: ngspice prints 0, and so does this engine, exactly.
        // `level` is a selector and is not reported. The deck has no
        // temperature, so there is no `m1_temp`.
        assert_table(
            &format!("{MOS}.sens v(dr)\n"),
            &[
                ("vdd", 9.652509558973e-01),
                ("vg", -3.07464107387e+00),
                ("rd", -1.84478282965e-04),
                ("m1:vto", 3.074641073234e+00),
                ("m1:kp", -3.689569288978e+04),
                ("m1:lambda", -5.366646189331e+00),
                ("m1:gamma", 0.0),
                ("m1:phi", 0.0),
                ("m1_w", -1.844784644494e+05),
                ("m1_l", 9.223923308321e+05),
                ("m1_m", -1.844784644591e+00),
            ],
            1e-5,
            0.0,
        );
    }

    const DIODES: &str = "* two diodes on one card, areas 1 and 2\n\
                          .options temp=26.85 tnom=26.85\n\
                          V1 a 0 DC 5\n\
                          R1 a k 1000\n\
                          D1 k m dm\n\
                          D2 m 0 dm 2\n\
                          .model dm d is=1e-14 n=1.5 rs=2\n";

    #[test]
    fn each_instance_of_a_diode_card_matches_ngspice() {
        // ngspice-42's own `.sens` has no `rs` entry (it does not move the
        // series conductance); the `d1:`/`d2:` values here are ngspice
        // central differences of `.op` with each instance on its own copy
        // of the card (see `MODEL_ORACLE`). D2's series resistance is
        // `rs/2`, so it carries half of D1's `∂v/∂rs`. ngspice-42's own
        // `.sens` prints `d1_m` as 0; the `_m` values are the `.op`
        // difference, which is twice `d2_area` for D2 (area 2).
        let sens = build(&format!("{DIODES}.sens v(k)\n"))
            .sensitivity()
            .unwrap();
        for (name, expected) in [
            ("v1", 2.829620023892e-02),
            ("r1", -8.401697201510e-05),
            ("d1:is", -3.768071696886e+12),
            ("d1:n", 6.636008149474e-01),
            ("d1:rs", 2.885179297696e-03),
            ("d2:is", -3.768071699994e+12),
            ("d2:n", 6.461886265274e-01),
            ("d2:rs", 1.442589634415e-03),
            ("d1_area", -4.345107567083e-02),
            ("d1_m", -4.345107567083e-02),
            ("d1_temp", -5.285047635528e-04),
            ("d2_area", -2.028294814216e-02),
            ("d2_m", -4.056589632428e-02),
            ("d2_temp", -6.155657056676e-04),
        ] {
            assert_close(name, sens.get(name).unwrap(), expected, 1e-5);
        }
        let names: Vec<&str> = sens.entries.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "r1", "v1", "d1:is", "d1:n", "d1:rs", "d2:is", "d2:n", "d2:rs", "d1_area", "d1_m",
                "d1_temp", "d2_area", "d2_m", "d2_temp"
            ]
        );
    }

    #[test]
    fn instance_entries_sum_to_the_card_move() {
        // Moving the card moves both instances, so its slope is the sum of
        // the two per-instance entries. The same current flows through
        // `rs` and `rs/2`, so D2's `∂v/∂rs` is half of D1's.
        let card = ".model dm d is=1e-14 n=1.5 rs=2";
        let probe = v("k");
        let deck = format!("{DIODES}.sens v(k)\n");
        for param in ["is", "n", "rs"] {
            let sum = model_entry(&deck, &probe, &format!("d1:{param}"))
                + model_entry(&deck, &probe, &format!("d2:{param}"));
            assert_close(param, sum, card_slope(DIODES, card, param, &probe), 1e-6);
        }
        let d1 = model_entry(&deck, &probe, "d1:rs");
        assert_close("rs", model_entry(&deck, &probe, "d2:rs"), d1 / 2.0, 1e-6);
    }

    const JFET: &str = "* self-biased n-JFET\n\
                        VDD d 0 DC 9\n\
                        RD d dr 2000\n\
                        RS s 0 500\n\
                        J1 dr 0 s jn\n\
                        .model jn njf vto=-2 beta=1e-3 lambda=0.01\n";

    #[test]
    fn every_jfet_parameter_matches_a_central_difference() {
        // ngspice-42 has no JFET sensitivity at all, so the difference is
        // the reference; a lone instance makes the card move the instance's.
        let card = ".model jn njf vto=-2 beta=1e-3 lambda=0.01";
        for probe in [v("dr"), v("s"), TransferOutput::Current("VDD".to_string())] {
            let deck = format!("{JFET}.sens v(dr)\n");
            for param in ["vto", "beta", "lambda"] {
                let got = model_entry(&deck, &probe, &format!("j1:{param}"));
                assert_close(param, got, card_slope(JFET, card, param, &probe), 1e-6);
            }
        }
    }

    fn instance_entry(deck: &str, probe: &TransferOutput, name: &str) -> f64 {
        let sens = build(deck)
            .instance_sensitivity_analysis(probe)
            .unwrap_or_else(|error| panic!("{error}"));
        sens.get(name)
            .unwrap_or_else(|| panic!("no `{name}` in {:?}", sens.entries))
    }

    const HOT_JFET: &str = "* self-biased n-JFET at 40 °C\n\
                            .options temp=40 tnom=26.85\n\
                            VDD d 0 DC 9\n\
                            RD d dr 2000\n\
                            RS s 0 500\n\
                            J1 dr 0 s jn area=1.5\n\
                            .model jn njf vto=-2 beta=1e-3 lambda=0.01 is=1e-14 \
                            vtotc=-2.5e-3 betatce=-0.5\n\
                            .sens v(dr)\n";

    #[test]
    fn jfet_instance_entries_match_ngspice() {
        // ngspice-42 has no JFET sensitivity; these are its `.op` central
        // differences (see `MODEL_ORACLE`). `vtotc` and `betatce` give the
        // temperature a channel law to move.
        for (probe, table) in [
            (
                v("dr"),
                [
                    ("j1_area", -9.100785898812e-01),
                    ("j1_m", -1.365117896484e+00),
                    ("j1_temp", 6.823125508806e-04),
                ],
            ),
            (
                v("s"),
                [
                    ("j1_area", 2.275196477508e-01),
                    ("j1_m", 3.412794714153e-01),
                    ("j1_temp", -1.705781499214e-04),
                ],
            ),
        ] {
            for (name, expected) in table {
                assert_close(name, instance_entry(HOT_JFET, &probe, name), expected, 1e-5);
            }
        }
    }

    #[test]
    fn area_and_multiplier_agree_where_the_laws_coincide() {
        // A unit-area JFET scales every law it stamps the same way under
        // AREA and under M, so the two entries are equal; at AREA = 1.5,
        // `_m` is `1.5·_area`.
        let unit = HOT_JFET.replace(" area=1.5", "");
        let probe = v("dr");
        let area = instance_entry(&unit, &probe, "j1_area");
        assert_close("j1_m", instance_entry(&unit, &probe, "j1_m"), area, 1e-6);
        let area = instance_entry(HOT_JFET, &probe, "j1_area");
        assert_close(
            "j1_m",
            instance_entry(HOT_JFET, &probe, "j1_m"),
            1.5 * area,
            1e-6,
        );
    }

    #[test]
    fn instance_temp_moves_the_device_alone() {
        // `_temp` moves one instance's temperature; the deck's `.temp` moves
        // every device and resistor law. With one temperature-sensitive
        // device and no resistor law, the two are the same derivative.
        let deck = HOT_JFET.replace(".sens v(dr)\n", "");
        let at = |celsius: f64| {
            output(
                &deck.replace("temp=40 ", &format!("temp={celsius:.17e} ")),
                &v("dr"),
            )
        };
        let step = 1e-3 * (40.0 + 273.15);
        let slope = (at(40.0 + step) - at(40.0 - step)) / (2.0 * step);
        assert_close(
            "j1_temp",
            instance_entry(HOT_JFET, &v("dr"), "j1_temp"),
            slope,
            1e-5,
        );
        // Without a temperature the device has no `_temp` entry.
        let cold = HOT_JFET.replace(".options temp=40 tnom=26.85\n", "");
        let sens = build(&cold)
            .instance_sensitivity_analysis(&v("dr"))
            .unwrap();
        let names: Vec<&str> = sens.entries.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["j1_area", "j1_m"]);
    }

    #[test]
    fn model_entries_need_the_deck_and_leave_the_solver_alone() {
        let mut solver = build(BJT);
        assert!(
            solver.model_sensitivity_analysis(&v("c")).is_err(),
            "a deck without `.sens` cannot be lowered again"
        );
        assert!(solver.instance_sensitivity_analysis(&v("c")).is_err());
        let mut solver = build(&format!("{BJT}.sens v(c)\n"));
        let c = solver.circuit().node("c").unwrap();
        let before = solver.node_voltage(c).to_bits();
        solver.model_sensitivity_analysis(&v("c")).unwrap();
        solver.instance_sensitivity_analysis(&v("c")).unwrap();
        assert_eq!(solver.node_voltage(c).to_bits(), before);
        solver.advance(1e-6).unwrap();
        assert!(solver.model_sensitivity_analysis(&v("nowhere")).is_err());
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
