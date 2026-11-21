// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `.ic V(node)=<value>` held through the transient's operating point.
//!
//! ngspice's `CKTload` holds every `.ic` node at its value in every Newton
//! phase of the transient operating point (`MODETRANOP` without `MODEUIC`),
//! so the rest of the circuit solves against it. The engine used to solve a
//! free operating point and then overwrite only the `.ic` nodes, which left
//! the other nodes, the capacitors on them and the source currents at the
//! free solution. `.op` and `.dc` ignore `.ic` in ngspice.
//!
//! The references are ngspice-42 `tran` point 0 and `.op` / `.dc` on the
//! same text with `.options temp=26.85 tnom=26.85 reltol=1e-12 abstol=1e-18
//! vntol=1e-15` (the engine's 300 K) and `numdgt=15`.

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

    fn assert_close(what: &str, got: f64, expected: f64) {
        let error = (got - expected).abs();
        assert!(
            error <= 1e-5 * expected.abs() + 1e-9,
            "{what}: expected {expected:e}, got {got:e} (error {error:e})"
        );
    }

    /// A divider whose midpoint `a` carries the `.ic`, and a second RC
    /// downstream of it that has no `.ic` of its own.
    const LADDER: &str = "\
* ic ladder
v1 in 0 10
r1 in a 1k
r2 a 0 1k
c1 a 0 1u
r3 a b 1k
c2 b 0 1u
r4 b 0 1k
.ic v(a)=2
.tran 1n 1u
.end
";

    #[test]
    fn linear_ladder_solves_downstream_of_the_held_node() {
        let mut solver = build(LADDER);
        assert_close("v(a)", voltage(&solver, "a"), 2.0);
        // Free operating point: v(b) = 2 V, i(v1) = -6 mA.
        assert_close("v(b)", voltage(&solver, "b"), 1.0);
        assert_close("i(v1)", branch(&solver, "v1"), -8e-3);

        // c2 starts at the held solution, not the free one: the net current
        // into b is zero, so one step leaves it at 1 V.
        solver.advance(1e-9).expect("first step solves");
        assert_close("v(b) after 1 ns", voltage(&solver, "b"), 1.0);
    }

    #[test]
    fn diode_solves_against_the_held_node() {
        let solver = build(
            "\
* ic diode
v1 in 0 5
r1 in a 1k
c1 a 0 1u
r2 a b 1k
d1 b 0 dm
.model dm d(is=1e-14 n=1.05 rs=2)
.ic v(a)=1
.tran 1n 1u
.end
",
        );
        assert_close("v(a)", voltage(&solver, "a"), 1.0);
        // Free operating point: v(b) = 7.125167e-1.
        assert_close("v(b)", voltage(&solver, "b"), 6.590073e-1);
        assert_close("i(v1)", branch(&solver, "v1"), -4e-3);
    }

    #[test]
    fn mosfet_drain_follows_the_held_gate() {
        let solver = build(
            "\
* ic nmos
vdd vdd 0 5
rg1 vdd g 100k
rg2 g 0 100k
cg g 0 1n
rd vdd d 10k
m1 d g 0 0 nm w=10u l=1u
.model nm nmos(level=1 vto=0.7 kp=50u lambda=0.02)
.ic v(g)=1.2
.tran 1n 1u
.end
",
        );
        assert_close("v(g)", voltage(&solver, "g"), 1.2);
        // Free operating point: v(d) = 5.786509e-1, i(vdd) = -4.67135e-4.
        assert_close("v(d)", voltage(&solver, "d"), 4.320988);
        assert_close("i(vdd)", branch(&solver, "vdd"), -1.05901e-4);
    }

    #[test]
    fn ic_overrides_a_nodeset_on_the_same_node() {
        // The nodeset alone puts c1 low; the `.ic` on the same node is
        // stamped after it and wins, so the latch starts with c1 high and
        // q2 driven by it.
        let solver = build(
            "\
* latch ic over nodeset
vcc vcc 0 5
rc1 vcc c1 1k
rc2 vcc c2 1k
rb1 c2 b1 10k
rb2 c1 b2 10k
q1 c1 b1 0 qn
q2 c2 b2 0 qn
cl c1 0 1n
.model qn npn(is=1e-14 bf=100)
.nodeset v(c1)=0
.ic v(c1)=5
.tran 1n 1u
.end
",
        );
        assert_close("v(c1)", voltage(&solver, "c1"), 5.0);
        assert_close("v(c2)", voltage(&solver, "c2"), 7.036214e-2);
        assert_close("v(b2)", voltage(&solver, "b2"), 6.996768e-1);
        assert_close("i(vcc)", branch(&solver, "vcc"), -4.92964e-3);
    }

    #[test]
    fn dc_sweep_ignores_ic() {
        let deck = LADDER.replace(".tran 1n 1u", ".dc v1 10 10 1");
        let solver = build(&deck);
        let response = solver.dc_sweep().expect("ladder sweep solves");
        let a = solver.circuit().node("a").expect("node a");
        let b = solver.circuit().node("b").expect("node b");
        assert_eq!(response.len(), 1);
        assert_close("dc v(a)", response.node_voltage(0, a), 4.0);
        assert_close("dc v(b)", response.node_voltage(0, b), 2.0);
    }
}
