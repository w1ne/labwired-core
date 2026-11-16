// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Table lookups in the in-core engine: `E`/`G ... TABLE {<expr>} = (x, y)
//! ...` and the behavioural `pwl(x, x1, y1, ...)` function.
//!
//! A table is how a vendor macromodel or a user writes a measured transfer
//! curve (a sensor response, a clipper, a nonlinear load) without a device
//! model. Each deck below is the exact text ngspice-42 ran (its `.control`
//! block is skipped here); the references are its `.op` answers printed with
//! 12 digits.

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

    fn assert_close(what: &str, got: f64, reference: f64, tolerance: f64) {
        let scale = reference.abs().max(1e-9);
        assert!(
            ((got - reference) / scale).abs() < tolerance,
            "{what}: ngspice-42 {reference:e}, got {got:e}"
        );
    }

    /// `E1` sits inside the rounded corner at x = 1 (d = 0.1), `E2` inside
    /// the end corner at x = 4 (d = 0.3, a tenth of the 1..4 segment), `E3`
    /// below the table, where it holds the first value. `B1` extends its
    /// first segment below x = 1.5 and `B2` its last one past x = 0.9:
    /// `pwl()` neither clamps nor rounds.
    const LOOKUPS: &str = "\
* table and pwl
V1 in 0 0.95
E1 e 0 TABLE {V(in)} = (0,0) (1,1) (4,2)
Re e 0 1k
E2 f 0 TABLE {V(in)*4} = (0,0) (1,1) (4,2)
Rf f 0 1k
E3 h 0 TABLE {V(in)-2} = (0,0) (1,1) (4,2)
Rh h 0 1k
B1 b 0 V = pwl(V(in), 1.5,0, 2,1, 3,-1)
Rb b 0 1k
B2 c 0 V = pwl(V(in), -1,1, 0.9,2)
Rc c 0 1k
";

    #[test]
    fn table_rounds_corners_and_pwl_extrapolates_like_ngspice() {
        let solver = build(LOOKUPS);
        assert_close("v(e)", voltage(&solver, "e"), 9.458333333333e-01, 1e-9);
        assert_close("v(f)", voltage(&solver, "f"), 1.930555555556e+00, 1e-9);
        assert_close("v(h)", voltage(&solver, "h"), 0.0, 1e-9);
        assert_close("v(b)", voltage(&solver, "b"), -1.10000000000e+00, 1e-9);
        assert_close("v(c)", voltage(&solver, "c"), 2.026315789474e+00, 1e-9);
    }

    /// A `G` table as a nonlinear two-terminal load (a diode-like I-V curve)
    /// that Newton has to solve through: `G1` settles on a straight segment,
    /// `G2` inside the rounded knee at 0.8 V. The references are ngspice-42
    /// with `.options reltol=1e-12 abstol=1e-18 vntol=1e-15`: at the default
    /// `reltol=1e-3` its XSPICE table stops 8.5 µV short on `v(a2)`.
    const NONLINEAR_LOAD: &str = "\
* table nonlinear resistor
V1 s 0 5
R1 s a 1k
G1 a 0 TABLE {V(a)} = (0,0) (0.6,0.1m) (0.7,1m) (0.8,10m)
V2 s2 0 1.744
R2 s2 a2 100
G2 a2 0 TABLE {V(a2)} = (0,0) (0.6,0.1m) (0.7,1m) (0.8,10m)
";

    #[test]
    fn g_table_is_a_nonlinear_load_newton_solves() {
        let solver = build(NONLINEAR_LOAD);
        assert_close("v(a)", voltage(&solver, "a"), 7.362637362637e-01, 1e-9);
        assert_close("v(a2)", voltage(&solver, "a2"), 7.949516806865e-01, 1e-9);
    }

    /// A table inside a `.subckt`: the input names a port and a subckt
    /// parameter. `X3` lands in the end corner at x = 1.02.
    const IN_SUBCKT: &str = "\
* table in subckt with params
.subckt shaper in out params: g=2
E1 out 0 TABLE {g*V(in)} = (-1, -1) (0, 0) (1, 3)
.ends
V1 in 0 0.3
X1 in o1 shaper
X2 in o2 shaper g=10
R1 o1 0 1k
R2 o2 0 1k
X3 in o3 shaper g=3.4
R3 o3 0 1k
";

    #[test]
    fn table_in_a_subcircuit_reads_its_ports_and_params() {
        let solver = build(IN_SUBCKT);
        assert_close("v(o1)", voltage(&solver, "o1"), 1.8, 1e-9);
        assert_close("v(o2)", voltage(&solver, "o2"), 3.0, 1e-9);
        assert_close("v(o3)", voltage(&solver, "o3"), 2.952, 1e-9);
    }

    #[test]
    fn malformed_tables_are_refused_with_the_line() {
        let refused = |deck: &str, needle: &str| {
            let error = parse_netlist(deck).unwrap_err();
            assert!(error.to_string().contains(needle), "{deck}: {error}");
            assert!(error.line().is_some(), "{deck}: {error}");
        };
        // ngspice-42 answers 0 V for a descending table; refuse it instead.
        refused(
            "V1 a 0 1\nE1 b 0 TABLE {V(a)} = (1,0) (0,2)\n",
            "strictly ascending",
        );
        // One point does not converge in ngspice-42.
        refused("V1 a 0 1\nE1 b 0 TABLE {V(a)} = (0,0)\n", "at least two");
        refused("V1 a 0 1\nE1 b 0 TABLE {V(a)} = (0,0) (1)\n", "pairs");
        refused(
            "V1 a 0 1\nE1 b 0 TABLE V(a) = (0,0) (1,1)\n",
            "TABLE {<expr>}",
        );
        // ngspice-42 refuses all three of these `pwl` forms.
        refused(
            "V1 a 0 1\nB1 b 0 V=pwl(V(a), 0,0, 0,1, 2,3)\n",
            "strictly ascending",
        );
        refused("V1 a 0 1\nB1 b 0 V=pwl(V(a), 0,1+1, 2,3)\n", "literal");
        refused("V1 a 0 1\nB1 b 0 V=pwl(V(a), 0,1, 2)\n", "pairs");
        // `F`/`H ... TABLE` is fatal in ngspice-42.
        refused("V1 a 0 1\nF1 b 0 TABLE {I(V1)} = (0,0) (1,1)\n", "");
    }
}
