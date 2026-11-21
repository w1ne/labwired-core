// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `E`/`G ... LAPLACE` transfer-function sources in the in-core engine.
//!
//! A Laplace source is how a user writes a filter, a sensor's response time
//! or a control loop's plant as `H(s)` without drawing its circuit. ngspice-42
//! has no `LAPLACE` keyword (its numparam fails on `V(in)`), so the
//! references are closed forms. The third-order Butterworth values were also
//! run in ngspice-42 as the XSPICE `s_xfer` code model on the same `H(s)`
//! (`num_coeff=[1] den_coeff=[1e-9 2e-6 2e-3 1]`, `.tran 1u 10m`,
//! `reltol=1e-9`): 9.863330e-2, 8.169549e-1 and 1.048358 V at 1, 3 and 6 ms.
//! Those are within 5.6e-5 V of the closed form (the `s_xfer` integrator's
//! own step error); this engine is held to 1e-6 V of the closed form.

mod analog {
    use labwired_core::analog::{parse_netlist, AnalogError, Integration, Solver};

    fn build(netlist: &str) -> Solver {
        let circuit = parse_netlist(netlist).unwrap_or_else(|error| panic!("{error}"));
        Solver::new(circuit, Integration::Trapezoidal).unwrap_or_else(|error| panic!("{error}"))
    }

    fn voltage(solver: &Solver, node: &str) -> f64 {
        let node = solver
            .circuit()
            .node(node)
            .unwrap_or_else(|| panic!("node `{node}` exists"));
        solver.node_voltage(node)
    }

    /// Step `Vin` from 0 to 1 V at t = 0+ and sample `node` at each time in
    /// `at`, in trapezoidal steps of `h` seconds.
    fn step_response(netlist: &str, node: &str, at: &[f64], h: f64) -> Vec<f64> {
        let mut solver = build(netlist);
        assert!(
            voltage(&solver, node).abs() < 1e-12,
            "`{node}` starts at rest"
        );
        let source = solver.voltage_source_index("Vin").expect("Vin exists");
        solver.set_voltage_source(source, 1.0);
        let mut t = 0.0;
        at.iter()
            .map(|&target| {
                let steps = ((target - t) / h).round() as usize;
                for _ in 0..steps {
                    solver.advance(h).expect("transient step solves");
                }
                t = target;
                voltage(&solver, node)
            })
            .collect()
    }

    fn assert_close(what: &str, got: f64, expected: f64, tolerance: f64) {
        assert!(
            (got - expected).abs() < tolerance,
            "{what}: expected {expected:e} V, got {got:e} V"
        );
    }

    #[test]
    fn laplace_operating_point_is_the_dc_gain() {
        // H(0) times the input: 3·2 V; G drives 1m·(1+s)/(2+s)·4 V = 2 mA
        // into 1 kΩ; the LTspice form reads V(in, 0).
        let solver = build(
            "* dc gain\n\
             Vin in 0 2\n\
             E1 e 0 LAPLACE {V(in)} = {3/(1+s*1m)}\n\
             Re e 0 1k\n\
             G1 0 g LAPLACE {V(in)*2} = {1m*(1+s)/(2+s)}\n\
             Rg g 0 1k\n\
             E2 l 0 in 0 laplace=4/(1+s)\n\
             Rl l 0 1k\n\
             .end\n",
        );
        assert_close("E1", voltage(&solver, "e"), 6.0, 1e-9);
        assert_close("G1", voltage(&solver, "g"), 2.0, 1e-9);
        assert_close("E2", voltage(&solver, "l"), 8.0, 1e-9);
    }

    #[test]
    fn first_order_lowpass_steps_in_closed_form() {
        // 1/(1+s·tau), tau = 1 ms: 1 − e^(−t/tau).
        let deck = "* rc\n.param tau=1m\nVin in 0 0\n\
                    E1 out 0 LAPLACE {V(in)} = {1/(1+s*tau)}\nR1 out 0 1k\n.end\n";
        let at = [0.5e-3, 1e-3, 2e-3, 5e-3];
        for (t, got) in at.iter().zip(step_response(deck, "out", &at, 1e-6)) {
            assert_close(&format!("t = {t}"), got, 1.0 - (-t / 1e-3_f64).exp(), 1e-6);
        }
    }

    #[test]
    fn third_order_butterworth_matches_closed_form_and_ngspice_s_xfer() {
        // wc = 1000 rad/s. Step response of 1/(p³ + 2p² + 2p + 1), p = s/wc:
        // 1 − e^(−τ) − (2/√3)·e^(−τ/2)·sin(√3·τ/2), τ = wc·t.
        let deck = "* butterworth\n.param wc=1k\nVin in 0 0\n\
                    E1 out 0 LAPLACE {V(in)} = {1/((s/wc)^3 + 2*(s/wc)^2 + 2*(s/wc) + 1)}\n\
                    R1 out 0 1k\n.end\n";
        let at = [1e-3, 3e-3, 6e-3];
        let ngspice = [9.863330e-2, 8.169549e-1, 1.048358];
        let got = step_response(deck, "out", &at, 1e-6);
        for ((t, got), reference) in at.iter().zip(got).zip(ngspice) {
            let tau = 1e3 * t;
            let closed = 1.0
                - (-tau).exp()
                - 2.0 / 3f64.sqrt() * (-tau / 2.0).exp() * (3f64.sqrt() * tau / 2.0).sin();
            assert_close(&format!("closed form at t = {t}"), got, closed, 1e-6);
            assert_close(&format!("ngspice s_xfer at t = {t}"), got, reference, 1e-4);
        }
    }

    #[test]
    fn lead_lag_in_a_subcircuit_passes_its_direct_term() {
        // (1 + s·tz)/(1 + s·tp) has equal orders, so the output jumps by
        // tz/tp at t = 0+ and settles as 1 − (1 − tz/tp)·e^(−t/tp). The
        // LTspice form inside a body checks that its control nodes flatten.
        let deck = "* lead lag\n\
                    .subckt leadlag a y tz=1m\n\
                    E1 y 0 a 0 laplace={(1+s*tz)/(1+s*4m)}\n\
                    .ends\n\
                    Vin in 0 0\nX1 in out leadlag tz=3m\nR1 out 0 1k\n.end\n";
        let (tz, tp) = (3e-3, 4e-3);
        let at = [1e-6, 1e-3, 4e-3, 12e-3];
        // The first sample is one step after the jump; the trapezoidal rule
        // carries the jump exactly, so the closed form holds there too.
        for (t, got) in at.iter().zip(step_response(deck, "out", &at, 1e-6)) {
            let closed = 1.0 - (1.0 - tz / tp) * (-t / tp).exp();
            assert_close(&format!("t = {t}"), got, closed, 1e-6);
        }
    }

    #[test]
    fn laplace_refuses_what_is_not_a_proper_rational_transfer() {
        for (transfer, words) in [
            ("{s*(1+s)/(1+s)}", "improper"),
            ("{1/s}", "pole at s = 0"),
            ("{exp(-s*1u)}", "not rational"),
            ("{1/(1+s)^0.5}", "integer power"),
            ("{1/(1+s*V(a))}", "unexpected"),
        ] {
            let deck = format!("* bad\nV1 a 0 1\nE1 b 0 LAPLACE {{V(a)}} = {transfer}\n.end\n");
            match parse_netlist(&deck) {
                Err(AnalogError::Parse { message, .. }) => assert!(
                    message.contains(words),
                    "{transfer}: message `{message}` should say `{words}`"
                ),
                other => panic!("{transfer} must be refused, got {other:?}"),
            }
        }
        // Without braces the input and transfer cannot be told apart.
        match parse_netlist("* bad\nV1 a 0 1\nE1 b 0 LAPLACE V(a) 1\n.end\n") {
            Err(AnalogError::Parse { message, .. }) => {
                assert!(message.contains("LAPLACE {<expr>} = {<H(s)>}"), "{message}")
            }
            other => panic!("a LAPLACE without braces must be refused, got {other:?}"),
        }
    }
}
