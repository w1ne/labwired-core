// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `.dc` source sweeps in the in-core engine.
//!
//! A sweep is the operating point solved once per source value, so these
//! tests hold each point to a closed form written from the device equations
//! (divider, diode law, Gummel-Poon forward-active collector current with the
//! Early factor), not from the engine. Nothing is device-specific in the
//! sweep itself: the same code drives a resistor, a diode and a BJT.

mod analog {
    use labwired_core::analog::{parse_netlist, AnalogError, Integration, Solver};

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

    #[test]
    fn dc_directive_errors_name_the_problem() {
        for (deck, needle) in [
            ("V1 a 0 1\nR1 a 0 1\n.dc V1 0 1\n", "expected `.dc"),
            ("V1 a 0 1\nR1 a 0 1\n.dc R1 0 1 0.1\n", "`R1` is not one"),
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
}
