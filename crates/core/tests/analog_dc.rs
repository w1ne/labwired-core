// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `.dc` source sweeps in the in-core engine.
//!
//! A sweep is the operating point solved once per source value, so these
//! tests hold each point to a closed form written from the device equations
//! (divider, diode law, Gummel-Poon forward-active collector current with the
//! Early factor), not from the engine. Nothing is device-specific in the
//! sweep itself: the same code drives a resistor, a diode and a BJT, and a
//! swept `R` sets a divider ratio or a BJT's base bias the same way. A swept
//! `TEMP` is held to ngspice-42 and to the same deck run with a `.temp` card.

mod analog {
    use labwired_core::analog::{
        parse_netlist, AnalogError, DcSweep, DcSweepSource, Integration, Solver,
    };

    const GMIN: f64 = 1e-12;
    /// `kT/q` at the engine's nominal 300 K, from CODATA constants.
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    fn build(netlist: &str) -> Solver {
        let circuit = parse_netlist(netlist).unwrap_or_else(|error| panic!("{error}"));
        Solver::new(circuit, Integration::BackwardEuler).unwrap_or_else(|error| panic!("{error}"))
    }

    fn node(solver: &Solver, name: &str) -> Option<usize> {
        solver
            .circuit()
            .node(name)
            .unwrap_or_else(|| panic!("node `{name}` exists"))
    }

    fn assert_close(what: &str, got: f64, expected: f64, relative: f64, absolute: f64) {
        let error = (got - expected).abs();
        assert!(
            error <= relative * expected.abs() + absolute,
            "{what}: expected {expected:e}, got {got:e} (error {error:e})"
        );
    }

    #[test]
    fn divider_sweep_steps_like_ngspice_and_opens_capacitors() {
        let solver = build(
            "* 1k/3k divider, a capacitor that must be open at DC\n\
             V1 in 0 DC 0\n\
             R1 in out 1k\n\
             R2 out 0 3k\n\
             C1 out 0 1u\n\
             .dc V1 0 1 0.1\n",
        );
        let response = solver.dc_sweep().expect("divider sweep solves");
        // 0 to 1 V by 0.1 V is 11 points, the last one kept despite rounding.
        assert_eq!(response.len(), 11);
        assert_eq!(response.outer_value(0), None);
        let out = node(&solver, "out");
        for point in 0..response.len() {
            let volts = response.inner_value(point);
            assert_close("sweep value", volts, 0.1 * point as f64, 1e-12, 1e-15);
            assert_close(
                "V(out)",
                response.node_voltage(point, out),
                volts * 0.75,
                1e-12,
                1e-15,
            );
            // SPICE: the source current flows from n+ through the source.
            assert_close(
                "I(V1)",
                response.branch_current(point, 0),
                -volts / 4e3,
                1e-12,
                1e-18,
            );
        }
    }

    #[test]
    fn descending_current_sweep_follows_the_diode_law() {
        let solver = build(
            "I1 0 a DC 0\n\
             D1 a 0 DX\n\
             .model DX D(IS=1e-14 N=1.5)\n\
             .dc I1 1m 0 -100u\n",
        );
        let response = solver.dc_sweep().expect("diode sweep solves");
        assert_eq!(response.len(), 11);
        let a = node(&solver, "a");
        let nvt = 1.5 * VT;
        for point in 0..response.len() {
            let amps = response.inner_value(point);
            assert_close(
                "sweep value",
                amps,
                1e-3 - 1e-4 * point as f64,
                1e-12,
                1e-18,
            );
            // Independent root of I = IS·(e^{V/nVT} − 1) + GMIN·V.
            let mut volts = nvt * (amps / 1e-14 + 1.0).ln();
            for _ in 0..50 {
                let exp = (volts / nvt).exp();
                let f = 1e-14 * (exp - 1.0) + GMIN * volts - amps;
                volts -= f / (1e-14 * exp / nvt + GMIN);
            }
            assert_close("V(a)", response.node_voltage(point, a), volts, 1e-9, 1e-12);
        }
        // Spot value: the 1 mA point sits on the knee, not at a linear guess.
        assert!((0.9..1.0).contains(&response.node_voltage(0, a)));
    }

    #[test]
    fn nested_sweep_gives_bjt_output_curves_with_the_early_slope() {
        let solver = build(
            "* NPN output characteristic: VCE inner, IB outer\n\
             IB 0 b DC 0\n\
             VCE c 0 DC 0\n\
             Q1 c b 0 QN\n\
             .model QN NPN(IS=1e-15 BF=100 BR=1 VAF=50)\n\
             .dc VCE 1 5 1 IB 10u 30u 10u\n",
        );
        let response = solver.dc_sweep().expect("BJT sweep solves");
        assert_eq!(response.inner_len(), 5);
        assert_eq!(response.len(), 15);
        let b = node(&solver, "b");
        let mut previous_slope = 0.0;
        for curve in 0..3 {
            let base = 10e-6 * (curve + 1) as f64;
            let mut first = f64::NAN;
            for step in 0..5 {
                let point = curve * 5 + step;
                assert_close("IB", response.outer_value(point).unwrap(), base, 1e-12, 0.0);
                let vce = response.inner_value(point);
                assert_close("VCE", vce, (step + 1) as f64, 1e-12, 0.0);
                let vbe = response.node_voltage(point, b);
                let vbc = vbe - vce;
                // Gummel-Poon with only VAF: qb = 1/(1 − vbc/VAF),
                // Ic = (IS·(e^{vbe/VT} − e^{vbc/VT}))·(1 − vbc/VAF)
                //      − IS/BR·(e^{vbc/VT} − 1), plus GMIN across both junctions.
                let ibe = 1e-15 * ((vbe / VT).exp() - 1.0);
                let ibc = 1e-15 * ((vbc / VT).exp() - 1.0);
                let expected_ic = (ibe - ibc) * (1.0 - vbc / 50.0) - ibc / 1.0 - GMIN * vbc;
                let expected_ib = ibe / 100.0 + GMIN * vbe + ibc / 1.0 + GMIN * vbc;
                assert_close("IB balance", expected_ib, base, 1e-9, 1e-15);
                let ic = -response.branch_current(point, 0);
                // Newton stops at ngspice's ABSTOL scale (pA); the Early term
                // moves Ic by ~8 % across the sweep, far above this bound.
                assert_close("IC", ic, expected_ic, 1e-8, 0.0);
                if step == 0 {
                    first = ic;
                }
                if step == 4 {
                    // Early: ΔIc/ΔVce ≈ BF·IB/VAF, and grows with IB.
                    let slope = (ic - first) / 4.0;
                    assert_close("Early slope", slope, 100.0 * base / 50.0, 0.05, 0.0);
                    assert!(slope > previous_slope);
                    previous_slope = slope;
                }
            }
        }
    }

    /// ngspice-42 `.dc VCE 1 5 1 IB 10u 30u 10u` on the same deck
    /// (`.options temp=26.85 tnom=26.85 reltol=1e-9 abstol=1e-18 vntol=1e-12`,
    /// `numdgt=10`), `print -i(vce) v(b)`. Fails if
    /// the sweep order, the source sign or the Early factor differs.
    #[test]
    fn nested_bjt_sweep_matches_ngspice() {
        const NGSPICE: [(f64, f64); 15] = [
            (1.0057136201e-03, 7.1431690843e-01),
            (1.0257137238e-03, 7.1431691101e-01),
            (1.0457138314e-03, 7.1431691360e-01),
            (1.0657139431e-03, 7.1431691618e-01),
            (1.0857140588e-03, 7.1431691877e-01),
            (2.0107105089e-03, 7.3223614361e-01),
            (2.0507106105e-03, 7.3223614491e-01),
            (2.0907107200e-03, 7.3223614620e-01),
            (2.1307108316e-03, 7.3223614749e-01),
            (2.1707109472e-03, 7.3223614878e-01),
            (3.0154368591e-03, 7.4271822413e-01),
            (3.0754369606e-03, 7.4271822499e-01),
            (3.1354370701e-03, 7.4271822585e-01),
            (3.1954371816e-03, 7.4271822671e-01),
            (3.2554372971e-03, 7.4271822757e-01),
        ];
        let solver = build(
            "IB 0 b DC 0
VCE c 0 DC 0
Q1 c b 0 QN
\
             .model QN NPN(IS=1e-15 BF=100 BR=1 VAF=50)
\
             .dc VCE 1 5 1 IB 10u 30u 10u
",
        );
        let response = solver.dc_sweep().unwrap();
        let b = node(&solver, "b");
        for (point, (ic, vbe)) in NGSPICE.into_iter().enumerate() {
            // Ten printed digits; the engine's own Newton stops near pA.
            assert_close("IC", -response.branch_current(point, 0), ic, 1e-8, 0.0);
            // ngspice-42 takes k and q from CODATA 2014, the engine from
            // CODATA 2018: kT/q differs by 3.4e-7, and so does VBE.
            assert_close("VBE", response.node_voltage(point, b), vbe, 1e-6, 0.0);
        }
    }

    #[test]
    fn sweep_leaves_the_transient_state_untouched() {
        let netlist = "V1 in 0 DC 1\nR1 in out 1k\nC1 out 0 1u\nD1 out 0 DX\n\
                       .model DX D(IS=1e-14)\n.dc V1 0 2 0.5\n";
        let mut swept = build(netlist);
        let mut fresh = build(netlist);
        swept.solve_operating_point().unwrap();
        fresh.solve_operating_point().unwrap();
        swept.set_voltage_source(0, 3.0);
        fresh.set_voltage_source(0, 3.0);
        for _ in 0..3 {
            swept.advance(1e-5).unwrap();
            fresh.advance(1e-5).unwrap();
        }
        let response = swept.dc_sweep().unwrap();
        assert_eq!(response.len(), 5);
        for _ in 0..3 {
            swept.advance(1e-5).unwrap();
            fresh.advance(1e-5).unwrap();
        }
        let out = node(&fresh, "out");
        assert_eq!(
            swept.node_voltage(out).to_bits(),
            fresh.node_voltage(out).to_bits()
        );
    }

    #[test]
    fn dc_sweep_ignores_uic_and_needs_a_real_source() {
        let solver =
            build("V1 in 0 DC 0\nR1 in out 1k\nR2 out 0 1k\n.tran 1u 1m uic\n.dc V1 2 2 1\n");
        let response = solver.dc_sweep().unwrap();
        assert_eq!(response.len(), 1);
        assert_close(
            "V(out)",
            response.node_voltage(0, node(&solver, "out")),
            1.0,
            1e-12,
            0.0,
        );

        let missing = build("V1 in 0 DC 0\nR1 in 0 1k\n.dc V9 0 1 1\n").dc_sweep();
        assert!(
            matches!(&missing, Err(AnalogError::Config(message)) if message.contains("V9")),
            "{missing:?}"
        );
        assert!(matches!(
            build("V1 in 0 DC 1\nR1 in 0 1k\n").dc_sweep(),
            Err(AnalogError::Config(message)) if message.contains(".dc")
        ));
    }

    /// ngspice-42 `.dc R1 1k 3k 1k V1 1 2 1` at `.temp 77`, `R1 ... tc1=0.01`,
    /// `print v(out) i(v1)` with `numdgt=10`. ngspice sweeps the nominal
    /// value and still applies `tc1`: R1 = 1k is 1.5k at 77 °C. Fails if the
    /// sweep replaced the effective resistance or dropped the temperature law.
    #[test]
    fn resistor_sweep_keeps_the_tc1_law_like_ngspice() {
        const NGSPICE: [(f64, f64, f64, f64); 6] = [
            (1e3, 1.0, 4.0000000000e-01, -4.000000000e-04),
            (2e3, 1.0, 2.5000000000e-01, -2.500000000e-04),
            (3e3, 1.0, 1.8181818182e-01, -1.818181818e-04),
            (1e3, 2.0, 8.0000000000e-01, -8.000000000e-04),
            (2e3, 2.0, 5.0000000000e-01, -5.000000000e-04),
            (3e3, 2.0, 3.6363636364e-01, -3.636363636e-04),
        ];
        let solver = build(
            "V1 in 0 DC 1\n\
             R1 in out 1k tc1=0.01\n\
             R2 out 0 1k\n\
             .temp 77\n\
             .dc R1 1k 3k 1k V1 1 2 1\n",
        );
        let response = solver.dc_sweep().expect("resistor sweep solves");
        assert_eq!(response.len(), NGSPICE.len());
        let out = node(&solver, "out");
        for (point, (ohms, volts, v_out, i_v1)) in NGSPICE.into_iter().enumerate() {
            assert_close("R1", response.inner_value(point), ohms, 1e-12, 0.0);
            assert_close(
                "V1",
                response.outer_value(point).unwrap(),
                volts,
                1e-12,
                0.0,
            );
            // Closed form: R1 = ohms·(1 + 0.01·(77 − 27)) over a 1k leg.
            let expected = volts * 1e3 / (ohms * 1.5 + 1e3);
            assert_close(
                "V(out) closed form",
                response.node_voltage(point, out),
                expected,
                1e-9,
                0.0,
            );
            assert_close(
                "V(out)",
                response.node_voltage(point, out),
                v_out,
                1e-9,
                0.0,
            );
            assert_close("I(V1)", response.branch_current(point, 0), i_v1, 1e-9, 0.0);
        }
        // The sweep ran on a copy: the solver's own R1 is still 1k.
        assert_eq!(solver.circuit().resistors[0].ohms, 1e3);
    }

    /// ngspice-42 `.dc RB 200k 600k 200k VCC 4 6 2` on the same deck
    /// (`.options temp=26.85 tnom=26.85 reltol=1e-9 abstol=1e-18 vntol=1e-12`,
    /// `numdgt=10`), `print v(c) v(b)`. A swept base resistor sets the
    /// Gummel-Poon bias point: fails if the stamp keeps the old resistance.
    #[test]
    fn base_resistor_sweep_sets_bjt_bias_like_ngspice() {
        const NGSPICE: [(f64, f64); 6] = [
            (2.3116614582e+00, 7.2705016655e-01),
            (3.1373665388e+00, 7.0927098820e-01),
            (3.4198702863e+00, 6.9887048927e-01),
            (3.2382009387e+00, 7.3931871713e-01),
            (4.5785730011e+00, 7.2148696757e-01),
            (5.0421524935e+00, 7.1105592799e-01),
        ];
        let solver = build(
            "VCC vcc 0 DC 5\n\
             RB vcc b 100k\n\
             RC vcc c 1k\n\
             Q1 c b 0 QN\n\
             .model QN NPN(IS=1e-15 BF=100 BR=1 VAF=50)\n\
             .dc RB 200k 600k 200k VCC 4 6 2\n",
        );
        let response = solver.dc_sweep().expect("base resistor sweep solves");
        assert_eq!(response.len(), NGSPICE.len());
        let (b, c) = (node(&solver, "b"), node(&solver, "c"));
        for (point, (v_c, v_b)) in NGSPICE.into_iter().enumerate() {
            assert_close("V(c)", response.node_voltage(point, c), v_c, 1e-7, 0.0);
            // CODATA 2014 (ngspice) vs 2018 (engine) kT/q: 3.4e-7 relative.
            assert_close("VBE", response.node_voltage(point, b), v_b, 1e-6, 0.0);
        }
    }

    /// `Solver::set_resistor` is the sweep's own setter; on a linear circuit
    /// the next transient step must refactor, not reuse the cached matrix.
    #[test]
    fn set_resistor_takes_effect_on_the_next_step() {
        let mut solver = build("V1 in 0 DC 1\nR1 in out 1k\nR2 out 0 1k\n");
        let out = node(&solver, "out");
        solver.advance(1e-6).unwrap();
        assert_close("V(out) before", solver.node_voltage(out), 0.5, 1e-12, 0.0);
        let r1 = solver.resistor_index("r1").expect("R1 by name, any case");
        assert!(solver.set_resistor(r1, 3e3));
        solver.advance(1e-6).unwrap();
        assert_close("V(out) after", solver.node_voltage(out), 0.25, 1e-12, 0.0);
        assert!(!solver.set_resistor(r1, 0.0));
        assert!(!solver.set_resistor(r1, f64::NAN));
        assert_eq!(solver.circuit().resistors[r1].ohms, 3e3);
    }

    #[test]
    fn resistor_sweep_through_zero_ohm_is_refused() {
        let solver = build("V1 a 0 1\nR1 a 0 1k\n.dc R1 -1k 1k 1k\n");
        let result = solver.dc_sweep();
        assert!(
            matches!(&result, Err(AnalogError::Config(message)) if message.contains("R1")),
            "{result:?}"
        );
    }

    #[test]
    fn dc_directive_errors_name_the_problem() {
        for (deck, needle) in [
            ("V1 a 0 1\nR1 a 0 1\n.dc V1 0 1\n", "expected `.dc"),
            ("V1 a 0 1\nC1 a 0 1\n.dc C1 0 1 0.1\n", "`C1` is not one"),
            ("V1 a 0 1\nR1 a 0 1\n.dc V1 0 1 0\n", "non-zero"),
            ("V1 a 0 1\nR1 a 0 1\n.dc V1 0 1 -0.1\n", "toward stop"),
            ("V1 a 0 1\nR1 a 0 1\n.dc V1 0 1 1u\n", "100001"),
            ("V1 a 0 1\nR1 a 0 1\n.dc V1 0 1 1 V1 0 1 1\n", "same source"),
            (
                "V1 a 0 1\nR1 a 0 1\n.dc V1 0 1 1\n.dc V1 0 2 1\n",
                "one `.dc`",
            ),
        ] {
            let message = parse_netlist(deck).expect_err(deck).to_string();
            assert!(message.contains(needle), "{deck}: {message}");
        }
    }

    /// Diode, BJT, level-1 MOSFET and a `tc1`/`tc2` collector resistor in one
    /// deck, swept over temperature. Reference: ngspice-42 on the same deck
    /// with `.options reltol=1e-9 abstol=1e-18 vntol=1e-12` and
    /// `set numdgt=10`.
    const TEMPERATURE_DECK: &str = "I1 0 a DC 1m\n\
        D1 a 0 DMOD\n\
        VCC c 0 DC 5\n\
        RC c col 1k tc1=2e-3 tc2=1e-6\n\
        RB c b 470k\n\
        Q1 col b 0 QMOD\n\
        VG g 0 DC 2\n\
        RD c dr 2k\n\
        M1 dr g 0 0 NMOD l=1u w=10u\n\
        .model DMOD D(is=1e-14 n=1.05 rs=2 eg=1.11 xti=3)\n\
        .model QMOD NPN(is=2e-15 bf=150 vaf=80 xtb=1.5 eg=1.11 xti=3)\n\
        .model NMOD NMOS(level=1 vto=0.8 kp=60u lambda=0.02 phi=0.65 gamma=0.4)\n";

    #[test]
    fn temperature_sweep_matches_ngspice_for_diode_bjt_mosfet_and_tc_resistor() {
        let solver = build(&format!("{TEMPERATURE_DECK}.dc TEMP -40 125 55\n"));
        let response = solver.dc_sweep().expect("temperature sweep solves");
        // index, °C, V(a), V(col), V(b), V(dr)
        const NGSPICE: [[f64; 5]; 4] = [
            [
                -40.0,
                7.9932674114e-01,
                4.1680601180e+00,
                8.0248040790e-01,
                3.8404742835e+00,
            ],
            [
                15.0,
                7.0978978228e-01,
                3.6998730379e+00,
                7.2251650669e-01,
                4.0351227548e+00,
            ],
            [
                70.0,
                6.1752215611e-01,
                3.0934727939e+00,
                6.4136388296e-01,
                4.1526046775e+00,
            ],
            [
                125.0,
                5.2296573643e-01,
                2.3316654664e+00,
                5.5924103430e-01,
                4.2265117585e+00,
            ],
        ];
        assert_eq!(response.len(), NGSPICE.len());
        let probes = ["a", "col", "b", "dr"].map(|name| node(&solver, name));
        for (point, row) in NGSPICE.iter().enumerate() {
            assert_close("TEMP", response.inner_value(point), row[0], 1e-12, 1e-12);
            for (probe, (&index, &expected)) in probes.iter().zip(&row[1..]).enumerate() {
                assert_close(
                    &format!(
                        "{} at {} °C",
                        ["V(a)", "V(col)", "V(b)", "V(dr)"][probe],
                        row[0]
                    ),
                    response.node_voltage(point, index),
                    expected,
                    // CODATA 2014 (ngspice) vs 2018 (engine) kT/q.
                    2e-6,
                    0.0,
                );
            }
        }
        // The sweep runs on a copy: the solver keeps its nominal devices.
        assert_eq!(
            solver.circuit().diodes,
            build(TEMPERATURE_DECK).circuit().diodes
        );
    }

    /// Every point of a `TEMP` sweep is the operating point of the same deck
    /// with that `.temp` card, for every temperature-dependent element kind:
    /// diode, BJT, MOSFET, JFET, NTC (`beta`) and `tc1` resistor. An element
    /// with its own `temp=` does not follow the sweep.
    #[test]
    fn temperature_sweep_point_equals_the_deck_with_that_temp_card() {
        let deck = format!(
            "{TEMPERATURE_DECK}\
             VJ jd 0 DC 3\n\
             RJ jd jdr 1k\n\
             J1 jdr 0 0 JMOD\n\
             .model JMOD NJF(vto=-1.5 beta=1m lambda=0.01 is=1e-14)\n\
             VN ntc 0 DC 3.3\n\
             RN ntc mid 10k beta=3950\n\
             RF mid 0 10k\n\
             I2 0 fixed DC 1m\n\
             D2 fixed 0 DMOD temp=85\n"
        );
        let swept = build(&format!("{deck}.dc TEMP -20 100 40\n"));
        let response = swept.dc_sweep().expect("temperature sweep solves");
        assert_eq!(response.len(), 4);
        let fixed = node(&swept, "fixed");
        let mut previous_mid = None;
        for point in 0..response.len() {
            let celsius = response.inner_value(point);
            let mut reference = build(&format!("{deck}.temp {celsius}\n"));
            reference
                .solve_operating_point()
                .expect("reference OP solves");
            for index in 0..reference.circuit().node_count() {
                let name = reference.circuit().node_name(index).to_string();
                assert_close(
                    &format!("V({name}) at {celsius} °C"),
                    response.node_voltage(point, Some(index)),
                    reference.node_voltage(Some(index)),
                    1e-9,
                    1e-12,
                );
            }
            // D2 stays at its own 85 °C whatever the sweep says.
            let at_85 = {
                let mut solver = build(&format!("{deck}.temp 85\n"));
                solver.solve_operating_point().unwrap();
                solver.node_voltage(fixed)
            };
            assert_close(
                "V(fixed)",
                response.node_voltage(point, fixed),
                at_85,
                1e-9,
                1e-12,
            );
            // The NTC divider output rises with temperature.
            let mid = response.node_voltage(point, node(&swept, "mid"));
            if let Some(previous) = previous_mid {
                assert!(
                    mid > previous,
                    "NTC mid {mid} not above {previous} at {celsius} °C"
                );
            }
            previous_mid = Some(mid);
        }
    }

    /// `TEMP` nests with an element sweep in either order. A swept `R` keeps
    /// its swept nominal value while the temperature lowers the deck again.
    #[test]
    fn temperature_sweep_nests_with_source_and_resistor_sweeps() {
        let deck = "V1 in 0 DC 0\n\
            R1 in a 1k tc1=4e-3\n\
            D1 a 0 DX\n\
            .model DX D(is=1e-14 n=1.2 eg=1.11 xti=3)\n";
        let reference = |celsius: f64, volts: f64, ohms: f64| {
            let mut solver = build(&format!(
                "V1 in 0 DC {volts}\nR1 in a {ohms} tc1=4e-3\nD1 a 0 DX\n\
                 .model DX D(is=1e-14 n=1.2 eg=1.11 xti=3)\n.temp {celsius}\n"
            ));
            solver.solve_operating_point().unwrap();
            solver.node_voltage(node(&solver, "a"))
        };

        let solver = build(&format!("{deck}.dc V1 1 3 1 TEMP 0 100 50\n"));
        let response = solver.dc_sweep().expect("V inside TEMP solves");
        assert_eq!(response.len(), 9);
        let a = node(&solver, "a");
        for point in 0..response.len() {
            let celsius = response.outer_value(point).unwrap();
            let volts = response.inner_value(point);
            assert_close(
                &format!("V(a) at {volts} V, {celsius} °C"),
                response.node_voltage(point, a),
                reference(celsius, volts, 1e3),
                1e-9,
                1e-12,
            );
        }

        let deck = deck.replacen("DC 0", "DC 2", 1);
        let solver = build(&format!("{deck}.dc TEMP 0 100 50 R1 1k 3k 2k\n"));
        let response = solver.dc_sweep().expect("TEMP inside R solves");
        assert_eq!(response.len(), 6);
        for point in 0..response.len() {
            let ohms = response.outer_value(point).unwrap();
            let celsius = response.inner_value(point);
            assert_close(
                &format!("V(a) at {ohms} ohm, {celsius} °C"),
                response.node_voltage(point, a),
                reference(celsius, 2.0, ohms),
                1e-9,
                1e-12,
            );
        }
    }

    #[test]
    fn temperature_sweep_needs_its_parsed_deck_and_stays_above_absolute_zero() {
        let message = parse_netlist("V1 a 0 1\nR1 a 0 1\n.dc TEMP -300 0 100\n")
            .expect_err("below absolute zero")
            .to_string();
        assert!(message.contains("absolute zero"), "{message}");
        // A sweep handed in by the caller on a deck with no `.dc` card has no
        // deck to lower again.
        let solver = build("I1 0 a 1m\nD1 a 0 DX\n.model DX D(is=1e-14)\n");
        let sweep = DcSweep {
            inner: DcSweepSource {
                source: "TEMP".to_string(),
                start: 0.0,
                stop: 50.0,
                step: 50.0,
            },
            outer: None,
        };
        let result = solver.dc_analysis(&sweep);
        assert!(
            matches!(&result, Err(AnalogError::Config(message)) if message.contains("`.dc TEMP`")),
            "{result:?}"
        );
    }
}
