// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `.sens ... ac` AC sensitivity in the in-core engine.
//!
//! ngspice-42 (`set numdgt=12`) gives the reference for a linear deck with
//! every element kind the AC form reports: `R`, `C`, coupled `L` and `K`,
//! `E`/`F`/`G`/`H` gains and the `AC` magnitude and phase of a `V` and an
//! `I`. ngspice perturbs each value by a part in 10⁶, so its table holds to
//! about 1e-5. Only the first point of an ngspice-42 sweep is right: after
//! it the plain resistor entries are zero and every other entry is wrong
//! (a sweep that starts at 1 MHz gives the engine's value there, one that
//! passes through it does not). So each table column is the first point of
//! its own run, and the resistor values are its `<name>_ac` vector, which
//! is the same derivative.
//!
//! Three entries differ from ngspice-42 by definition or by fault, and a
//! central difference of the deck holds them instead. ngspice moves a
//! coupled inductor with the mutual inductance `M` held, the engine with the
//! deck's `k` held (`M = k·sqrt(L1·L2)` moves too): an ngspice `.ac`
//! difference with `M` held gives ngspice's `l1`. ngspice's `k1` entry is
//! not `∂/∂k`: an ngspice `.ac` difference in `k` gives the engine's value.
//!
//! ngspice-42 does not solve the operating point for this analysis, so it is
//! no reference for a nonlinear device. A central difference of the `.ac`
//! output gives the reference there instead: for a `C`, an `AC` value or a
//! gain the operating point does not move, so the held-operating-point
//! derivative is the whole derivative.

mod analog {
    use labwired_core::analog::{parse_netlist, Integration, Phasor, Solver, TransferOutput};

    fn build(netlist: &str) -> Solver {
        let circuit = parse_netlist(netlist).unwrap_or_else(|error| panic!("{error}"));
        let mut solver = Solver::new(circuit, Integration::BackwardEuler)
            .unwrap_or_else(|error| panic!("{error}"));
        solver
            .solve_operating_point()
            .unwrap_or_else(|error| panic!("{error}"));
        solver
    }

    fn assert_phasor(what: &str, got: Phasor, expected: (f64, f64), relative: f64) {
        let scale = libm::hypot(expected.0, expected.1);
        let error = libm::hypot(got.re - expected.0, got.im - expected.1) / scale;
        assert!(
            error <= relative,
            "{what}: expected {expected:?}, got ({:e}, {:e}) (relative error {error:e})",
            got.re,
            got.im
        );
    }

    fn v(node: &str) -> TransferOutput {
        TransferOutput::Voltage {
            node: node.to_string(),
            reference: None,
        }
    }

    /// Every element kind the AC form reports, in one linear network: a
    /// phased voltage drive into a coupled pair, a phased current drive on
    /// the secondary, and every controlled source into one summing node.
    const LINEAR: &str = "* every .sens ac element kind\n\
                          v1 in 0 dc 0 ac 2 30\n\
                          i1 0 out dc 0 ac 1m -45\n\
                          r1 in a 50\n\
                          l1 a 0 10u\n\
                          l2 out 0 40u\n\
                          k1 l1 l2 0.8\n\
                          r2 out 0 200\n\
                          c2 out 0 1n\n\
                          g1 0 out2 out 0 2m\n\
                          r4 out2 0 1k\n\
                          f1 0 out2 v1 0.1\n\
                          h1 o3 0 v1 7\n\
                          r5 o3 out2 3k\n\
                          e1 o4 0 out 0 2\n\
                          l3 o4 o5 2u\n\
                          r6 o5 out2 5k\n";

    /// ngspice-42 `.sens v(out2) ac dec 1 <f> 1e8` on [`LINEAR`], read at its
    /// first point: the phasor at 100 kHz and at 1 MHz.
    const LINEAR_TABLE: &[(&str, [(f64, f64); 2])] = &[
        (
            "c2",
            [
                (3.460082215188e+04, 1.334158560915e+07),
                (-1.069274382539e+09, -3.046119859914e+09),
            ],
        ),
        (
            "e1_gain",
            [
                (-1.126519138900e-02, 5.178738435292e-02),
                (2.556181779774e-01, -2.493793737577e-02),
            ],
        ),
        (
            "f1_gain",
            [
                (-2.349099160033e+01, -9.910129412959e+00),
                (-1.552658814976e+01, -6.998650586944e+00),
            ],
        ),
        (
            "g1_gain",
            [
                (-5.639103489090e+01, 2.589227655049e+02),
                (1.278404269237e+03, -1.214774941174e+02),
            ],
        ),
        (
            "h1_gain",
            [
                (-7.830330533318e-03, -3.303376470934e-03),
                (-5.175529383170e-03, -2.332883528944e-03),
            ],
        ),
        (
            "i1_acmag",
            [
                (4.404190194944e+01, 2.812986246013e+01),
                (1.348020794190e+02, -2.234254449570e+02),
            ],
        ),
        (
            "i1_acphase",
            [
                (-4.909584161963e-04, 7.686763908036e-04),
                (3.899510571881e-03, 2.352738593005e-03),
            ],
        ),
        (
            "l3",
            [
                (1.943372313698e+01, -3.879524926291e+01),
                (1.022349924141e+02, -4.006106274001e+02),
            ],
        ),
        (
            "r1",
            [
                (5.112506055271e-02, 1.896490569865e-03),
                (-1.643628498545e-02, 1.196618345577e-02),
            ],
        ),
        (
            "r2",
            [
                (-5.308442938454e-04, 1.376721593652e-06),
                (1.212011370678e-02, -4.254503334115e-03),
            ],
        ),
        (
            "r4",
            [
                (-1.656005563978e-03, -2.561733438567e-04),
                (9.648833022441e-04, -6.577440835862e-04),
            ],
        ),
        (
            "r5",
            [
                (-1.657298652274e-04, -2.075583414608e-05),
                (1.192854789437e-04, -6.763928648898e-05),
            ],
        ),
        (
            "r6",
            [
                (-6.174449513986e-05, -3.092970006840e-05),
                (-6.375909783980e-05, -1.627118812669e-05),
            ],
        ),
        (
            "v1_acmag",
            [
                (-1.291632913992e+00, -2.104230695375e-01),
                (6.721776472421e-01, -3.928003307006e-01),
            ],
        ),
        (
            "v1_acphase",
            [
                (7.345162586087e-03, -4.508649224103e-02),
                (1.371131200372e-02, 2.346342980450e-02),
            ],
        ),
    ];

    #[test]
    fn linear_network_matches_ngspice() {
        let deck = format!("{LINEAR}.sens v(out2) ac dec 1 1e5 1e6\n");
        let sens = build(&deck)
            .ac_sensitivity()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(sens.frequencies, [1e5, 1e6]);
        // A sweep answers each point as a run of that point alone does.
        let alone = build(LINEAR)
            .ac_sensitivity_analysis(&v("out2"), &[1e6])
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(alone.get("c2").unwrap()[0], sens.get("c2").unwrap()[1]);
        for &(name, expected) in LINEAR_TABLE {
            let got = sens
                .get(name)
                .unwrap_or_else(|| panic!("no `{name}` in {:?}", sens.entries));
            for (point, &expected) in expected.iter().enumerate() {
                assert_phasor(&format!("{name} @ {point}"), got[point], expected, 1e-5);
            }
        }
        // The rest are the ones ngspice-42 reports as zero in AC (a `V`/`I`
        // DC value is not an AC entry at all here) or leaves out (`i1` has
        // no `_acphase` there only when the card writes no phase).
        let names: Vec<&str> = sens.entries.iter().map(|(name, _)| name.as_str()).collect();
        let mut expected: Vec<&str> = LINEAR_TABLE.iter().map(|&(name, _)| name).collect();
        expected.extend(["k1", "l1", "l2"]);
        expected.sort_unstable();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, expected, "entries {names:?}");
    }

    /// A common-emitter BJT stage with junction and load capacitance.
    const STAGE: &str = "* ce stage\n\
                         vcc vcc 0 dc 12 ac 0.25 10\n\
                         vin in 0 dc 2 ac 1\n\
                         rb in b 10k\n\
                         q1 c b e qn\n\
                         rc vcc c 4.7k\n\
                         re e 0 1k\n\
                         cl c 0 10n\n\
                         ce e 0 1u\n\
                         vx x 0 dc 0 ac 0.5\n\
                         gm 0 c x 0 1u\n\
                         .model qn npn is=1e-15 bf=150 vaf=80 cje=2p cjc=1p tf=0.3n\n";

    /// The `.ac` output `v(node)` of `deck` at `hertz`.
    fn output(deck: &str, node: &str, hertz: f64) -> Phasor {
        let mut solver = build(deck);
        let node = solver.circuit().node(node).unwrap();
        solver
            .ac_analysis(&[hertz])
            .unwrap_or_else(|error| panic!("{error}"))
            .node_voltage(0, node)
    }

    /// Token `at` of the unique deck line starting `prefix`, and the deck
    /// with that token's value scaled by `factor`.
    fn scaled(deck: &str, prefix: &str, at: usize, factor: f64) -> (f64, String) {
        let lines: Vec<&str> = deck.lines().filter(|l| l.starts_with(prefix)).collect();
        assert_eq!(lines.len(), 1, "`{prefix}` is not unique");
        let mut tokens: Vec<String> = lines[0].split(' ').map(str::to_string).collect();
        let value = labwired_core::analog::parse_spice_value(&tokens[at]).unwrap();
        tokens[at] = format!("{:.17e}", value * factor);
        (value, deck.replace(lines[0], &tokens.join(" ")))
    }

    /// Hold each named entry of `deck`'s sensitivity of `v(node)` at
    /// `hertz` to a central difference of token `at` on the line starting
    /// `prefix`.
    fn assert_differences(deck: &str, node: &str, hertz: f64, entries: &[(&str, &str, usize)]) {
        const STEP: f64 = 1e-4;
        let sens = build(deck)
            .ac_sensitivity_analysis(&v(node), &[hertz])
            .unwrap_or_else(|error| panic!("{error}"));
        for &(name, prefix, at) in entries {
            let (value, up) = scaled(deck, prefix, at, 1.0 + STEP);
            let (_, down) = scaled(deck, prefix, at, 1.0 - STEP);
            let (up, down) = (output(&up, node, hertz), output(&down, node, hertz));
            let slope = (
                (up.re - down.re) / (2.0 * STEP * value),
                (up.im - down.im) / (2.0 * STEP * value),
            );
            let got = sens.get(name).unwrap_or_else(|| panic!("no `{name}`"))[0];
            assert_phasor(name, got, slope, 1e-5);
        }
    }

    #[test]
    fn every_linear_entry_matches_a_central_difference() {
        for hertz in [3e4, 7e5] {
            assert_differences(
                LINEAR,
                "out2",
                hertz,
                &[
                    ("c2", "c2 ", 3),
                    ("e1_gain", "e1 ", 5),
                    ("f1_gain", "f1 ", 4),
                    ("g1_gain", "g1 ", 5),
                    ("h1_gain", "h1 ", 4),
                    ("i1_acmag", "i1 ", 6),
                    ("i1_acphase", "i1 ", 7),
                    ("k1", "k1 ", 3),
                    ("l1", "l1 ", 3),
                    ("l2", "l2 ", 3),
                    ("l3", "l3 ", 3),
                    ("r1", "r1 ", 3),
                    ("r2", "r2 ", 3),
                    ("r4", "r4 ", 3),
                    ("r5", "r5 ", 3),
                    ("r6", "r6 ", 3),
                    ("v1_acmag", "v1 ", 6),
                    ("v1_acphase", "v1 ", 7),
                ],
            );
        }
    }

    #[test]
    fn nonlinear_stage_matches_a_central_difference() {
        const HERTZ: f64 = 1e3;
        // Values that leave the operating point where it is.
        assert_differences(
            STAGE,
            "c",
            HERTZ,
            &[
                ("cl", "cl ", 3),
                ("ce", "ce ", 3),
                ("gm_gain", "gm ", 5),
                ("vin_acmag", "vin ", 6),
                ("vcc_acmag", "vcc ", 6),
                ("vcc_acphase", "vcc ", 7),
            ],
        );
        // The stage has gain, so the check is not of a vanishing quantity.
        assert!(output(STAGE, "c", HERTZ).magnitude() > 1.0);
    }

    /// For a resistor the operating point moves too, and the held-operating
    /// point derivative is the linearised network's: the same circuit with
    /// the transistor replaced by its small-signal equivalent gives it
    /// exactly, so a central difference on that circuit checks it.
    #[test]
    fn resistor_entry_holds_the_operating_point() {
        const HERTZ: f64 = 1e4;
        let stage = "* resistive divider into a diode\n\
                     v1 in 0 dc 5 ac 1\n\
                     r1 in a 1k\n\
                     d1 a 0 dd\n\
                     .model dd d is=1e-14\n";
        let mut solver = build(stage);
        let sens = solver
            .ac_sensitivity_analysis(&v("a"), &[HERTZ])
            .unwrap_or_else(|error| panic!("{error}"));
        // At the held operating point the diode is a conductance `gd`, so
        // `v(a) = 1/(1 + r1·gd)` and `∂v(a)/∂r1 = −gd/(1 + r1·gd)²`.
        let a = solver.circuit().node("a").unwrap();
        let vd = solver.node_voltage(a);
        let vt = 1.380649e-23 * 300.0 / 1.602176634e-19;
        let gd = 1e-14 * libm::exp(vd / vt) / vt;
        let expected = -gd / ((1.0 + 1e3 * gd) * (1.0 + 1e3 * gd));
        assert_phasor("r1", sens.get("r1").unwrap()[0], (expected, 0.0), 1e-3);
    }

    #[test]
    fn ac_sensitivity_leaves_the_transient_bit_identical() {
        let run = |sens: bool| {
            let mut solver = build(STAGE);
            if sens {
                solver
                    .ac_sensitivity_analysis(&v("c"), &[1e3, 1e6])
                    .unwrap();
            }
            (0..50)
                .map(|_| {
                    solver.advance(1e-6).unwrap();
                    let c = solver.circuit().node("c").unwrap();
                    solver.node_voltage(c).to_bits()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(false), run(true));
    }

    #[test]
    fn the_card_selects_its_form_and_malformed_cards_are_refused() {
        let mut ac = build(&format!("{LINEAR}.sens v(out2) ac oct 2 1k 4k\n"));
        assert_eq!(ac.ac_sensitivity().unwrap().frequencies.len(), 5);
        assert!(ac.sensitivity().is_err(), "DC run of an AC card");
        let mut dc = build(&format!("{LINEAR}.sens v(out2)\n"));
        assert!(dc.ac_sensitivity().is_err(), "AC run of a DC card");
        assert!(dc.sensitivity().is_ok());
        for card in [
            ".sens v(out2) ac",
            ".sens v(out2) ac dec 10 1",
            ".sens v(out2) ac dec 10 1 1k extra",
            ".sens v(out2) ac log 10 1 1k",
            ".sens v(out2) dc dec 10 1 1k",
        ] {
            let deck = format!("{LINEAR}{card}\n");
            assert!(parse_netlist(&deck).is_err(), "`{card}` parsed");
        }
        let mut solver = build(LINEAR);
        assert!(solver.ac_sensitivity().is_err(), "no card");
        assert!(solver
            .ac_sensitivity_analysis(&v("nowhere"), &[1e3])
            .is_err());
        assert!(solver.ac_sensitivity_analysis(&v("out2"), &[-1.0]).is_err());
    }
}
