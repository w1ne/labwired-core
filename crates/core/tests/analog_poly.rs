// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SPICE2 polynomial dependent sources (`E`/`G`/`F`/`H ... POLY(n) ...`) in
//! the in-core engine.
//!
//! Vendor macromodels written for SPICE2 and PSpice use `POLY` for summing
//! nodes, multipliers and soft nonlinearities. Each deck below is the exact
//! text ngspice-42 ran (its `.control` block is skipped here); the references
//! are its `.op` answers printed with 12 digits.

mod analog {
    use labwired_core::analog::{parse_netlist, Integration, Solver};

    fn build(netlist: &str) -> Solver {
        let circuit = parse_netlist(netlist).unwrap_or_else(|error| panic!("{error}"));
        Solver::new(circuit, Integration::BackwardEuler).unwrap_or_else(|error| panic!("{error}"))
    }

    fn voltage(solver: &Solver, node: &str) -> f64 {
        let node = solver
            .circuit()
            .node(node)
            .unwrap_or_else(|| panic!("node `{node}` exists"));
        solver.node_voltage(node)
    }

    fn branch(solver: &Solver, name: &str) -> f64 {
        solver.branch_current(
            solver
                .circuit()
                .branch_index(name)
                .unwrap_or_else(|| panic!("branch `{name}` exists")),
        )
    }

    fn assert_close(what: &str, got: f64, reference: f64, tolerance: f64) {
        let scale = reference.abs().max(1e-9);
        assert!(
            ((got - reference) / scale).abs() < tolerance,
            "{what}: ngspice-42 {reference:e}, got {got:e}"
        );
    }

    /// One of each letter. `E1` reaches a cubic term (`p6·x1³`), `G1` has a
    /// single coefficient (the constant `p0` in ngspice-42), `F1` squares a
    /// branch current and `H1` reads two branch currents.
    const FOUR_LETTERS: &str = "\
* poly test
V1 a 0 2
V2 b 0 0.5
R1 a 0 1k
R2 b 0 1k
E1 o 0 POLY(2) (a,0) (b,0) 0.1 1 2 0.3 0.4 0.5 0.01
Ro o 0 1k
G1 0 g POLY(1) a 0 1m
Rg g 0 1k
Vs a s 0
Rs s 0 2k
F1 0 f POLY(1) Vs 0.5m 2 1000
Rf f 0 1k
H1 h 0 POLY(2) Vs V2 0.1 100 0 50000
Rh h 0 1k
.end
";

    #[test]
    fn every_letter_matches_ngspice() {
        let solver = build(FOUR_LETTERS);
        // 0.1 + 2 + 2·0.5 + 0.3·4 + 0.4·1 + 0.5·0.25 + 0.01·8.
        assert_close("v(o)", voltage(&solver, "o"), 4.905, 1e-12);
        assert_close("v(g)", voltage(&solver, "g"), 1.0, 1e-12);
        // 0.5 mA + 2·1 mA + 1000·(1 mA)².
        assert_close("v(f)", voltage(&solver, "f"), 3.5, 1e-12);
        // 0.1 + 100·1 mA + 0·i(V2) + 50000·(1 mA)².
        assert_close("v(h)", voltage(&solver, "h"), 0.25, 1e-12);
        assert_close("i(v1)", branch(&solver, "V1"), -3.0e-3, 1e-12);
        // A voltage-form POLY carries a branch current like any `E`/`H`.
        assert!(solver.circuit().branch_index("H1").is_some());
    }

    /// Three controls: the degree-2 terms run `x1², x1·x2, x1·x3, x2², ...`,
    /// so `x1·x3` comes before `x2²`, and the eleventh coefficient is `x1³`.
    const TERM_ORDER: &str = "\
* POLY(3) term order up to degree 2
V1 a 0 2
V2 b 0 3
V3 c 0 5
R1 a 0 1k
R2 b 0 1k
R3 c 0 1k
E1 o 0 POLY(3) a 0 b 0 c 0 0 0 0 0 0 0 1m 10 0 0 0
Ro o 0 1k
E2 q 0 POLY(3) (a,0) (b,0) (c,0) 0 0 0 0 0 0 0 0 0 0 100u
Rq q 0 1k
.end
";

    #[test]
    fn higher_terms_follow_spice2_order() {
        let solver = build(TERM_ORDER);
        assert_close("v(o)", voltage(&solver, "o"), 90.01, 1e-12);
        assert_close("v(q)", voltage(&solver, "q"), 8.0e-4, 1e-12);
    }

    /// A second, unrelated circuit: a diode node loaded by its own square-law
    /// `G` (feedback), sensed by a `POLY` amplifier inside a subcircuit and by
    /// an `H` on the diode current. Newton has to solve the loop.
    const DIODE_LOOP: &str = "\
* POLY in a nonlinear loop: diode + square-law feedback + subcircuit
.temp 27
.options reltol=1e-10 vntol=1e-12 abstol=1e-15
.subckt amp in out
E1 out 0 POLY(1) (in, 0) 0.2 3 -0.4
Ro out 0 10k
.ends
V1 in 0 5
R1 in a 1k
D1 a k dmod
Vk k 0 0
G1 a 0 POLY(1) a 0 0 0.5m 0.2m
X1 a y amp
H1 h 0 POLY(1) Vk 0 200 1e4
Rh h 0 1k
.model dmod D(IS=1e-14 N=1.05 RS=2)
.end
";

    #[test]
    fn diode_feedback_loop_and_subcircuit_match_ngspice() {
        let solver = build(DIODE_LOOP);
        assert_close("v(a)", voltage(&solver, "a"), 7.316878240762e-01, 1e-6);
        assert_close("v(y)", voltage(&solver, "y"), 2.180916643468e+00, 1e-6);
        assert_close("v(h)", voltage(&solver, "h"), 9.031291905376e-01, 1e-6);
        assert_close("i(v1)", branch(&solver, "V1"), -4.26831217592e-03, 1e-6);
        assert_close("i(vk)", branch(&solver, "Vk"), 3.795394849505e-03, 1e-6);
    }

    #[test]
    fn mistakes_are_refused_with_the_line() {
        let refused = |deck: &str, needle: &str| {
            let error = parse_netlist(deck).unwrap_err();
            assert!(error.to_string().contains(needle), "{deck}: {error}");
            assert!(error.line().is_some(), "{deck}: {error}");
        };
        refused(
            "V1 a 0 1\nE1 b 0 POLY(0) a 0 1\n",
            "whole number of controls",
        );
        refused("V1 a 0 1\nE1 b 0 POLY(2) a 0 1\n", "needs 4 control nodes");
        refused("V1 a 0 1\nF1 b 0 POLY(1) V1\n", "needs 1 control elements");
        refused(
            "V1 a 0 1\nE1 b 0 POLY(1) a 0 0 1 ic=0.5\n",
            "initial-condition",
        );
        refused(
            "V1 a 0 1\nG1 b 0 POLY(1) nowhere 0 0 1m\n",
            "node `nowhere`",
        );
        refused(
            "R1 a 0 1k\nH1 b 0 POLY(1) R1 0 1\n",
            "carries no branch current",
        );
    }
}
