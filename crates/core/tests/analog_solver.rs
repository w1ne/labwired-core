// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The MNA transient solver against closed-form answers.
//!
//! Every circuit here has an exact analytic solution, so the assertions are
//! against physics rather than against a previous run of this code. The
//! tolerances are the design's: 0.5 % for backward Euler at 10 substeps, 0.05 %
//! for trapezoidal.

// The `analog` module puts every test under `analog::`, so the design's
// verification command `cargo test -p labwired-core analog` runs all of them.
mod analog {
    use labwired_core::analog::{parse_netlist, AnalogError, Integration, Solver, MAX_UNKNOWNS};

    /// Step the solver to `t` seconds in `steps` equal internal steps.
    fn run_to(solver: &mut Solver, t: f64, steps: usize) {
        let h = t / steps as f64;
        for _ in 0..steps {
            solver.advance(h).expect("transient step should solve");
        }
    }

    fn rc_solver(integration: Integration) -> Solver {
        // 10 kΩ / 100 nF: tau = 1 ms. Vgpio is stepped to 3.3 V at t = 0+.
        let circuit = parse_netlist("* rc\nVgpio in 0 dc 0\nR1 in out 10k\nC1 out 0 100n\n.end\n")
            .expect("rc netlist parses");
        let mut solver = Solver::new(circuit, integration).expect("solver builds");
        let source = solver.voltage_source_index("Vgpio").expect("Vgpio exists");
        solver.set_voltage_source(source, 3.3);
        solver
    }

    /// Run the RC fixture at the engine's own cadence (a 100 us co-sim step split
    /// into 10 substeps, against tau = 1 ms) and check `out` at tau, 2 tau, 3 tau.
    fn rc_step_response_error(integration: Integration) -> Vec<(u32, f64, f64, f64)> {
        let mut solver = rc_solver(integration);
        let node = solver.circuit().node("out").expect("node out");
        let mut taken = 0;
        let mut out = Vec::new();
        for multiple in 1..=3u32 {
            // tau = 1 ms is exactly ten 100 us co-sim steps.
            while taken < multiple * 10 {
                run_to(&mut solver, 100e-6, 10);
                taken += 1;
            }
            let expected = 3.3 * (1.0 - (-f64::from(multiple)).exp());
            let actual = solver.node_voltage(node);
            out.push((
                multiple,
                actual,
                expected,
                (actual - expected).abs() / expected,
            ));
        }
        out
    }

    #[test]
    fn rc_step_response_matches_closed_form_with_backward_euler() {
        for (multiple, actual, expected, error) in
            rc_step_response_error(Integration::BackwardEuler)
        {
            assert!(
                error < 0.005,
                "{multiple} tau: BE gave {actual} V, closed form {expected} V, \
             relative error {:.4} %",
                error * 100.0
            );
        }
    }

    #[test]
    fn rc_step_response_is_an_order_of_magnitude_better_with_trapezoidal() {
        for (multiple, actual, expected, error) in rc_step_response_error(Integration::Trapezoidal)
        {
            assert!(
                error < 0.0005,
                "{multiple} tau: trapezoidal gave {actual} V, closed form {expected} V, \
             relative error {:.5} %",
                error * 100.0
            );
        }
    }

    #[test]
    fn rl_current_matches_closed_form() {
        // 10 V across 100 Ω in series with 10 mH: tau = L/R = 100 us,
        // i(t) = (V/R)·(1 − e^(−t/tau)).
        //
        // `ic=0` is what makes this a STEP response: the DC operating point of an
        // inductor is a short, so without it the branch starts at its final 100 mA
        // and there is no transient to check. That is correct SPICE behaviour and
        // exactly why `ic=` exists.
        let circuit = parse_netlist("* rl\nV1 a 0 dc 10\nR1 a b 100\nL1 b 0 10m ic=0\n.end\n")
            .expect("rl netlist parses");
        let mut solver = Solver::new(circuit, Integration::Trapezoidal).expect("solver builds");
        assert_eq!(solver.inductor_current(0), 0.0, "`ic=0` starts it at rest");

        let tau = 10e-3 / 100.0;
        let final_current = 10.0 / 100.0;
        let mut t = 0.0;
        for multiple in 1..=3 {
            let target = f64::from(multiple) * tau;
            run_to(&mut solver, target - t, 200);
            t = target;
            let expected = final_current * (1.0 - (-f64::from(multiple)).exp());
            let actual = solver.inductor_current(0);
            let error = (actual - expected).abs() / expected;
            assert!(
                error < 0.001,
                "{multiple} tau: RL current {actual} A vs closed form {expected} A ({:.4} %)",
                error * 100.0
            );
        }
    }

    #[test]
    fn underdamped_rlc_matches_closed_form() {
        // Series RLC driven by a 1 V step: R = 20 Ω, L = 1 mH, C = 1 uF.
        // alpha = R/2L = 10 000 s^-1, w0 = 1/sqrt(LC) = 31 623 rad/s, so the
        // circuit is underdamped and the capacitor voltage overshoots:
        //   vC(t) = V·[1 − e^(−alpha t)·(cos(wd t) + (alpha/wd)·sin(wd t))]
        //
        // `ic=0` again: with the capacitor open at DC no current flows, so the
        // operating point already sits at the 1 V rail.
        let circuit =
            parse_netlist("* rlc\nV1 a 0 dc 1\nR1 a b 20\nL1 b c 1m\nC1 c 0 1u ic=0\n.end\n")
                .expect("rlc netlist parses");
        let mut solver = Solver::new(circuit, Integration::Trapezoidal).expect("solver builds");
        let node = solver.circuit().node("c").expect("node c");
        assert_eq!(solver.capacitor_voltage(0), 0.0);
        assert_eq!(solver.inductor_current(0), 0.0);

        let r = 20.0_f64;
        let l = 1e-3_f64;
        let c = 1e-6_f64;
        let alpha = r / (2.0 * l);
        let w0 = 1.0 / (l * c).sqrt();
        let wd = (w0 * w0 - alpha * alpha).sqrt();
        assert!(wd > 0.0, "this fixture must be underdamped");

        // Across the first ringing period, including the overshoot peak.
        let mut peak = 0.0_f64;
        let mut t = 0.0;
        for target in [20e-6, 50e-6, 100e-6, 200e-6, 400e-6] {
            run_to(&mut solver, target - t, 2_000);
            t = target;
            let expected =
                1.0 - (-alpha * t).exp() * ((wd * t).cos() + (alpha / wd) * (wd * t).sin());
            let actual = solver.node_voltage(node);
            peak = peak.max(actual);
            assert!(
                (actual - expected).abs() < 2e-3,
                "t={t:e}s: RLC capacitor {actual} V vs closed form {expected} V"
            );
        }
        // The ring is physics, not numerical noise: an underdamped step overshoots
        // its final value and then settles back to it.
        assert!(
            peak > 1.05,
            "an underdamped step must overshoot, peaked at {peak} V"
        );
        assert!(
            (solver.node_voltage(node) - 1.0).abs() < 0.05,
            "and settle by 400 us, at {} V",
            solver.node_voltage(node)
        );
    }

    #[test]
    fn switch_conducts_at_ron_and_blocks_at_roff() {
        // A 1 kΩ pull-up to 5 V, shunted to ground by the switch.
        let circuit = parse_netlist(
            "* switch\nV1 vdd 0 dc 5\nR1 vdd out 1k\nS1 out 0 gate ron=1 roff=1meg\n\
         C1 out 0 1p\n.end\n",
        )
        .expect("switch netlist parses");
        let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver builds");
        let node = solver.circuit().node("out").expect("node out");
        let switch = solver.switches_controlled_by("gate");
        assert_eq!(switch, vec![0], "one switch is driven by `gate`");

        // Open: the divider is 1 k / 1 Meg, so `out` sits just under 5 V.
        run_to(&mut solver, 1e-6, 100);
        let open = solver.node_voltage(node);
        let expected_open = 5.0 * 1e6 / (1e6 + 1e3);
        assert!(
            (open - expected_open).abs() < 1e-3,
            "open switch: {open} V, expected {expected_open} V"
        );

        // Closed: 1 k / 1 Ω pulls it to a few millivolts.
        solver.set_switch(0, true);
        run_to(&mut solver, 1e-6, 100);
        let closed = solver.node_voltage(node);
        let expected_closed = 5.0 * 1.0 / (1.0 + 1e3);
        assert!(
            (closed - expected_closed).abs() < 1e-3,
            "closed switch: {closed} V, expected {expected_closed} V"
        );
        assert!(
            closed < open / 100.0,
            "closing the switch must pull `out` down"
        );
    }

    #[test]
    fn operating_point_uses_netlist_dc_values() {
        // A pull-up sits at Vdd before anything is routed, exactly like a board
        // out of reset — the property the ngspice wrapper preserves by pausing the
        // transient just after t = 0.
        let circuit = parse_netlist("* op\nV1 vdd 0 dc 3.3\nR1 vdd out 10k\nC1 out 0 100n\n.end\n")
            .expect("netlist parses");
        let solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver builds");
        let node = solver.circuit().node("out").expect("node out");
        assert!(
            (solver.node_voltage(node) - 3.3).abs() < 1e-9,
            "an unloaded pull-up is at Vdd at the operating point, not 0 V"
        );
        assert!(
            (solver.capacitor_voltage(0) - 3.3).abs() < 1e-9,
            "the capacitor starts charged to the operating point"
        );
    }

    #[test]
    fn initial_conditions_override_the_operating_point() {
        let circuit =
            parse_netlist("* ic\nV1 vdd 0 dc 3.3\nR1 vdd out 10k\nC1 out 0 100n ic=1.0\n.end\n")
                .expect("netlist parses");
        let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver builds");
        assert!((solver.capacitor_voltage(0) - 1.0).abs() < 1e-12);

        // From 1 V the node charges toward 3.3 V with the same tau.
        let tau = 10e3 * 100e-9;
        run_to(&mut solver, tau, 1_000);
        let node = solver.circuit().node("out").expect("node out");
        let expected = 3.3 + (1.0 - 3.3) * (-1.0_f64).exp();
        assert!(
            (solver.node_voltage(node) - expected).abs() < 2e-3,
            "charging from ic=1.0: {} V vs {expected} V",
            solver.node_voltage(node)
        );
    }

    #[test]
    fn dot_ic_seeds_a_node() {
        let circuit = parse_netlist(
            "* dotic\nV1 vdd 0 dc 5\nR1 vdd out 1k\nC1 out 0 1u\n.ic V(out)=2\n.end\n",
        )
        .expect("netlist parses");
        let solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver builds");
        assert!((solver.capacitor_voltage(0) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn a_circuit_over_the_unknown_limit_is_refused() {
        // One voltage source plus a chain of resistors: each new node is one more
        // unknown, so 64 nodes + 1 branch current crosses the ceiling.
        let mut netlist = String::from("* ladder\nV1 n0 0 dc 1\n");
        let nodes = MAX_UNKNOWNS;
        for index in 0..nodes {
            netlist.push_str(&format!("R{index} n{index} n{} 1k\n", index + 1));
        }
        netlist.push_str(".end\n");

        let circuit = parse_netlist(&netlist).expect("ladder parses");
        assert!(circuit.unknowns() > MAX_UNKNOWNS);
        let err = Solver::new(circuit, Integration::BackwardEuler)
            .expect_err("an over-sized circuit must be refused, not silently truncated");
        match err {
            AnalogError::TooLarge { unknowns, max } => {
                assert_eq!(max, MAX_UNKNOWNS);
                assert!(unknowns > MAX_UNKNOWNS);
            }
            other => panic!("expected TooLarge, got {other}"),
        }
        assert!(
            err.to_string().contains("labwired_ngspice.py"),
            "the error must point at the adapter that can solve it: {err}"
        );
    }

    #[test]
    fn a_circuit_at_the_unknown_limit_is_accepted() {
        let mut netlist = String::from("* ladder\nV1 n0 0 dc 1\n");
        // 63 nodes + 1 voltage-source branch current = exactly 64.
        for index in 0..62 {
            netlist.push_str(&format!("R{index} n{index} n{} 1k\n", index + 1));
        }
        netlist.push_str("Rend n62 0 1k\n.end\n");
        let circuit = parse_netlist(&netlist).expect("ladder parses");
        assert_eq!(circuit.unknowns(), MAX_UNKNOWNS);
        Solver::new(circuit, Integration::BackwardEuler).expect("64 unknowns is inside the limit");
    }

    #[test]
    fn a_floating_node_is_reported_not_silently_zeroed() {
        let circuit =
            parse_netlist("* floating\nV1 a 0 dc 1\nR1 b c 1k\n.end\n").expect("netlist parses");
        let err = Solver::new(circuit, Integration::BackwardEuler)
            .expect_err("a node with no DC path to ground has no unique solution");
        assert!(matches!(err, AnalogError::Singular { .. }), "{err}");
    }

    #[test]
    fn the_same_netlist_and_inputs_give_bit_identical_results() {
        let run = || {
            let mut solver = rc_solver(Integration::Trapezoidal);
            let node = solver.circuit().node("out").expect("node out");
            let mut samples = Vec::new();
            for _ in 0..50 {
                run_to(&mut solver, 100e-6, 10);
                samples.push(solver.node_voltage(node).to_bits());
            }
            samples
        };
        assert_eq!(run(), run(), "the solver must be bit-deterministic");
    }
}
