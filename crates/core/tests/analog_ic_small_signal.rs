// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `.ic` and `uic` do not move the small-signal operating point.
//!
//! ngspice's `.ac`, `.noise`, `.tf`, `.sens` and `.pz` linearise about a
//! `MODEDCOP` operating point: `CKTload` holds `.ic` nodes only in
//! `MODETRANOP`, and `uic` is a `.tran` option. A fresh solver's state is
//! the transient's operating point, where every `.ic` node is held (or,
//! under `uic`, every node is zero), so the small-signal analyses used to
//! linearise a diode at the `.ic` voltage instead of its free bias.
//!
//! The references are ngspice-42 `op`, `ac`, `noise`, `tf` and `sens` on
//! [`DECK`] with `.options temp=26.85 tnom=26.85 reltol=1e-12 abstol=1e-18
//! vntol=1e-15` (the engine's 300 K) and `numdgt=15`. ngspice gives the same
//! numbers with or without the `.ic` line.

mod analog {
    use labwired_core::analog::{parse_netlist, Integration, Solver};

    fn build(netlist: &str) -> Solver {
        let circuit = parse_netlist(netlist).unwrap_or_else(|error| panic!("{error}"));
        Solver::new(circuit, Integration::BackwardEuler).unwrap_or_else(|error| panic!("{error}"))
    }

    fn voltage(solver: &Solver, node: &str) -> f64 {
        solver.node_voltage(solver.circuit().node(node).expect("node exists"))
    }

    fn assert_close(what: &str, got: f64, expected: f64, relative: f64) {
        let error = (got - expected).abs();
        assert!(
            error <= relative * expected.abs() + 1e-15,
            "{what}: expected {expected:e}, got {got:e} (error {error:e})"
        );
    }

    /// A forward-biased diode at `a` (free bias 0.85 V) with `.ic v(a)=0.3`,
    /// where the diode would be almost off, and an RC behind it.
    const DECK: &str = "\
* diode ic small signal
v1 in 0 dc 5 ac 1
r1 in a 1k
d1 a 0 dmod
c1 a 0 1u
r2 a b 2k
c2 b 0 100n
.model dmod d is=1e-14 n=1.2 rs=5 cjo=2p tt=5n
.ic v(a)=0.3
.ac lin 3 100 10k
.noise v(b) v1 lin 2 100 1k
.tf v(b) v1
.sens v(b)
.tran 1n 1u
.end
";

    /// ngspice-42 `ac lin 3 100 10k`: `[f, re v(b), im v(b)]`.
    const AC_B: [[f64; 3]; 3] = [
        [1e2, 1.211611164672904e-02, -1.62735270309010e-03],
        [5.05e3, -3.79540884240520e-04, -1.73551911743662e-03],
        [1e4, -4.19830939267230e-04, -6.45147156777376e-04],
    ];

    fn assert_ac(solver: &mut Solver) {
        let b = solver.circuit().node("b").expect("node b exists");
        let response = solver.ac_sweep().unwrap_or_else(|error| panic!("{error}"));
        for (point, &[hertz, re, im]) in AC_B.iter().enumerate() {
            let got = response.node_voltage(point, b);
            let error = (got.re - re).hypot(got.im - im);
            assert!(
                error <= 1e-5 * re.hypot(im),
                "V(b) at {hertz} Hz: expected {re:e}{im:+e}j, got {:e}{:+e}j",
                got.re,
                got.im
            );
        }
    }

    #[test]
    fn transient_operating_point_still_holds_the_ic() {
        let solver = build(DECK);
        assert_close("v(a)", voltage(&solver, "a"), 0.3, 1e-9);
    }

    #[test]
    fn ac_linearises_about_the_free_operating_point() {
        let mut solver = build(DECK);
        assert_ac(&mut solver);
        // The transient's own starting point is not moved by the analysis.
        assert_close("v(a) after .ac", voltage(&solver, "a"), 0.3, 1e-9);
    }

    #[test]
    fn noise_linearises_about_the_free_operating_point() {
        let mut solver = build(DECK);
        let noise = solver
            .noise_sweep()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_close(
            "onoise_total",
            noise.output_total(),
            1.26873749949038e-7,
            1e-5,
        );
        assert_close(
            "inoise_total",
            noise.input_total(),
            1.665057572456594e-05,
            1e-5,
        );
    }

    #[test]
    fn tf_linearises_about_the_free_operating_point() {
        let mut solver = build(DECK);
        let tf = solver
            .transfer_function()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_close(
            "transfer_function",
            tf.transfer,
            1.232268240345803e-02,
            1e-5,
        );
        assert_close(
            "input_impedance",
            tf.input_impedance,
            1.012476425431582e+03,
            1e-5,
        );
        assert_close(
            "output_impedance",
            tf.output_impedance,
            2.012322682403458e+03,
            1e-5,
        );
    }

    #[test]
    fn sens_linearises_about_the_free_operating_point() {
        let mut solver = build(DECK);
        let sens = solver
            .sensitivity()
            .unwrap_or_else(|error| panic!("{error}"));
        let get = |name: &str| {
            sens.get(name)
                .unwrap_or_else(|| panic!("`.sens` reports `{name}`"))
        };
        assert_close("v1", get("v1"), 1.232268240299153e-02, 1e-5);
        assert_close("r1", get("r1"), -5.11312084891010e-05, 1e-5);
        assert_close("d1:is", get("d1:is"), -3.06401099504352e+12, 1e-4);
        assert_close("d1:n", get("d1:n"), 6.830448052203301e-01, 1e-4);
        assert_close("d1_area", get("d1_area"), -5.11315558551506e-02, 1e-4);
    }

    #[test]
    fn uic_does_not_zero_the_small_signal_operating_point() {
        // Under `uic` the transient starts from all-zero nodes; `.ac` still
        // uses the DC operating point.
        let deck = DECK
            .replace(".ic v(a)=0.3\n", "")
            .replace(".tran 1n 1u", ".tran 1n 1u uic");
        let mut solver = build(&deck);
        assert_close("v(a) under uic", voltage(&solver, "a"), 0.0, 1e-9);
        assert_ac(&mut solver);
    }

    #[test]
    fn advanced_solver_linearises_about_its_present_solution() {
        // After the transient has started the analyses keep linearising
        // about the present state, as documented; only the fresh solver's
        // held point is replaced.
        let mut held = build(DECK);
        held.advance(1e-9).expect("first step solves");
        let b = held.circuit().node("b").expect("node b exists");
        let at_present = held.ac_analysis(&[100.0]).unwrap().node_voltage(0, b);
        let free = build(&DECK.replace(".ic v(a)=0.3\n", ""))
            .ac_analysis(&[100.0])
            .unwrap()
            .node_voltage(0, b);
        assert!(
            (at_present.re - free.re).abs() > 1e-4,
            "a solver 1 ns after a 0.3 V `.ic` is still near 0.3 V, far from the free bias"
        );
    }
}
