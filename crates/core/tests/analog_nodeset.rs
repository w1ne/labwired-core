// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `.nodeset V(node)=<value>` in the in-core engine.
//!
//! A nodeset is ngspice's hint for a circuit with more than one operating
//! point: the first Newton phase (`MODEINITJCT`/`MODEINITFIX`) holds each
//! named node at its value, then releases it, so the solve settles in the
//! state the hint chose. The engine used to accept the card and drop it, so
//! a latch written for ngspice solved to a different state here.
//!
//! The references are ngspice-42 `.op` / `.dc` on the same text with
//! `.options temp=26.85 tnom=26.85 reltol=1e-12 abstol=1e-18 vntol=1e-15`
//! (the engine's 300 K) and `numdgt=15`.

mod analog {
    use labwired_core::analog::{parse_netlist, AnalogError, Integration, Solver};

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

    fn assert_close(what: &str, got: f64, expected: f64) {
        let error = (got - expected).abs();
        assert!(
            error <= 1e-5 * expected.abs() + 1e-9,
            "{what}: expected {expected:e}, got {got:e} (error {error:e})"
        );
    }

    /// Two cross-coupled NPN inverters: symmetric, so it has two stable
    /// states and a metastable one between them.
    fn bjt_latch(directive: &str) -> String {
        format!(
            "\
* BJT latch
vcc vcc 0 5
rc1 vcc c1 1k
rc2 vcc c2 1k
rb1 c2 b1 10k
rb2 c1 b2 10k
q1 c1 b1 0 qn
q2 c2 b2 0 qn
.model qn npn(is=1e-14 bf=100)
{directive}
.op
.end
"
        )
    }

    /// The same topology in level-1 NMOS with resistor loads: a second,
    /// unrelated device law for the same mechanism.
    fn mos_latch(directive: &str) -> String {
        format!(
            "\
* NMOS latch
vdd vdd 0 3.3
rd1 vdd d1 10k
rd2 vdd d2 10k
m1 d1 d2 0 0 nm w=10u l=1u
m2 d2 d1 0 0 nm w=10u l=1u
.model nm nmos(level=1 vto=0.7 kp=50u lambda=0.02)
{directive}
.op
.end
"
        )
    }

    #[test]
    fn bjt_latch_settles_in_the_state_the_nodeset_chooses() {
        // Without a hint both engines find the metastable point.
        let plain = build(&bjt_latch(""));
        assert_close("plain v(c1)", voltage(&plain, "c1"), 1.078162);
        assert_close("plain v(c2)", voltage(&plain, "c2"), 1.078162);

        let high = build(&bjt_latch(".nodeset v(c1)=5 v(c2)=0"));
        assert_close("c1-high v(c1)", voltage(&high, "c1"), 4.609029);
        assert_close("c1-high v(c2)", voltage(&high, "c2"), 7.279645e-2);
        assert_close("c1-high v(b1)", voltage(&high, "b1"), 7.279650e-2);
        assert_close("c1-high v(b2)", voltage(&high, "b2"), 6.993145e-1);
        assert_close("c1-high i(vcc)", branch(&high, "vcc"), -5.31817e-3);

        // One node is enough, and the mirror deck gives the mirror state.
        let low = build(&bjt_latch(".nodeset v(c2)=5"));
        assert_close("c2-high v(c1)", voltage(&low, "c1"), 7.279645e-2);
        assert_close("c2-high v(c2)", voltage(&low, "c2"), 4.609029);
        assert_close("c2-high v(b1)", voltage(&low, "b1"), 6.993145e-1);
    }

    #[test]
    fn mos_latch_settles_in_the_state_the_nodeset_chooses() {
        let plain = build(&mos_latch(""));
        assert_close("plain v(d1)", voltage(&plain, "d1"), 1.529067);
        assert_close("plain v(d2)", voltage(&plain, "d2"), 1.529067);

        let high = build(&mos_latch(".nodeset v(d1)=3.3 v(d2)=0"));
        assert_close("d1-high v(d1)", voltage(&high, "d1"), 3.3);
        assert_close("d1-high v(d2)", voltage(&high, "d2"), 2.454027e-1);
        assert_close("d1-high i(vdd)", branch(&high, "vdd"), -3.05460e-4);

        // Spaces around `=` are accepted, as in `.ic`.
        let low = build(&mos_latch(".nodeset V(d2) = 3"));
        assert_close("d2-high v(d1)", voltage(&low, "d1"), 2.454027e-1);
        assert_close("d2-high v(d2)", voltage(&low, "d2"), 3.3);
    }

    #[test]
    fn nodeset_does_not_move_a_circuit_with_one_operating_point() {
        let deck = |directive: &str| {
            format!(
                "\
* diode ladder
v1 in 0 5
r1 in a 1k
d1 a 0 dm
r2 a b 2k
d2 b 0 dm
.model dm d(is=1e-14 n=1.05 rs=2)
{directive}
"
            )
        };
        // Hints far outside the supply, and one on a source-driven node.
        for directive in ["", ".nodeset v(a)=-20 v(b)=40", ".nodeset v(in)=1"] {
            let solver = build(&deck(directive));
            assert_close(
                &format!("{directive} v(a)"),
                voltage(&solver, "a"),
                7.349153e-1,
            );
            assert_close(
                &format!("{directive} v(b)"),
                voltage(&solver, "b"),
                6.119473e-1,
            );
            assert_close(
                &format!("{directive} i(v1)"),
                branch(&solver, "v1"),
                -4.26508e-3,
            );
        }
    }

    #[test]
    fn dc_sweep_holds_the_nodeset_only_at_the_first_point() {
        // The sweep starts with 2 mA into b1, where q1 is on whatever the
        // hint says, and walks down to 0 A. ngspice holds the nodeset only at
        // the first point and continues from there, so the latch stays with
        // c1 low all the way down — while `.op` at 0 A takes the hint.
        let solver = build(
            "\
* latch hysteresis
vcc vcc 0 5
rc1 vcc c1 1k
rc2 vcc c2 1k
rb1 c2 b1 10k
rb2 c1 b2 10k
q1 c1 b1 0 qn
q2 c2 b2 0 qn
i1 0 b1 0
.model qn npn(is=1e-14 bf=100)
.nodeset v(c1)=5 v(c2)=0
.dc i1 2m 0 -0.5m
",
        );
        assert_close("op v(c1)", voltage(&solver, "c1"), 4.609029);

        let response = solver.dc_sweep().expect("latch sweep solves");
        let c1 = solver.circuit().node("c1").expect("node c1");
        let c2 = solver.circuit().node("c2").expect("node c2");
        let expected = [
            (2e-3, 3.687313e-2, 4.610285),
            (1.5e-3, 4.027491e-2, 4.610029),
            (1e-3, 4.530775e-2, 4.609742),
            (5e-4, 5.375169e-2, 4.609414),
            (0.0, 7.279645e-2, 4.609029),
        ];
        assert_eq!(response.len(), expected.len());
        for (point, (amps, v1, v2)) in expected.into_iter().enumerate() {
            assert_close("i1", response.inner_value(point), amps);
            assert_close(
                &format!("{amps} A v(c1)"),
                response.node_voltage(point, c1),
                v1,
            );
            assert_close(
                &format!("{amps} A v(c2)"),
                response.node_voltage(point, c2),
                v2,
            );
        }
    }

    #[test]
    fn nodeset_errors_name_the_problem() {
        let message = |deck: &str| match parse_netlist(deck) {
            Err(AnalogError::Parse { message, .. }) => message,
            other => panic!("expected a parse error, got {other:?}"),
        };
        let deck = |card: &str| format!("V1 a 0 1\nR1 a 0 1k\n{card}\n");
        assert!(message(&deck(".nodeset")).contains("expected `.nodeset V(node)=<value>`"));
        assert!(message(&deck(".nodeset v(a)")).contains("expected `.nodeset V(node)=<value>`"));
        assert!(message(&deck(".nodeset a=1")).contains("not a node reference"));
        assert!(message(&deck(".nodeset v(0)=1")).contains("`.nodeset` cannot set the ground node"));
        // `.ic` keeps its own wording.
        assert!(message(&deck(".ic v(0)=1")).contains("`.ic` cannot set the ground node"));

        let circuit = parse_netlist(&deck(".nodeset v(a)=0.5 v(b)=2")).expect("parses");
        let a = circuit.node("a").expect("node a").expect("not ground");
        let b = circuit.node("b").expect("node b").expect("not ground");
        assert_eq!(circuit.node_nodeset, vec![(a, 0.5), (b, 2.0)]);
        assert!(circuit.node_ic.is_empty());
    }
}
