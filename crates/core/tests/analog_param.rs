// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ngspice numparam in the in-core engine: `.param`, `{expression}` values
//! and subcircuit `params:`.
//!
//! Vendor model text and reusable user circuits are parameterised: a card
//! says `IS={is0}`, a macro says `.subckt NSW d g s params: w=10u`, and the
//! caller picks the value. Each deck below is the exact text ngspice-45 ran
//! (its `.control` block is skipped here), with `.temp 27` so both engines
//! use the same temperature and `reltol=1e-12 vntol=1e-15 abstol=1e-18` so
//! ngspice's own answer is converged past the tolerance asserted.

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

    fn assert_close(what: &str, got: f64, reference: f64, tolerance: f64) {
        let scale = reference.abs().max(1e-9);
        assert!(
            ((got - reference) / scale).abs() < tolerance,
            "{what}: ngspice-45 {reference:e}, got {got:e}"
        );
    }

    /// Global parameters, written before the parameter they depend on, feed
    /// element values and a diode card.
    const DIODE: &str = "\
* parametric divider and diode card
.temp 27
.options reltol=1e-12 vntol=1e-15 abstol=1e-18
.param rtop={2*rbot} rbot=10k
.param vsupply=5 is0=2.5n
.model DP D(IS={is0} N={1.5+0.25} RS={rbot/1meg})
V1 in 0 {vsupply}
R1 in a {rtop}
R2 a 0 {rbot}
D1 a k DP
R3 k 0 {rbot / 10}
.control
op
set numdgt=12
print v(a) v(k) i(v1)
.endc
.end
";

    /// [`DIODE`] with every expression written out by hand.
    const DIODE_LITERAL: &str = "\
.temp 27
.model DP D(IS=2.5n N=1.75 RS=0.01)
V1 in 0 5
R1 in a 20k
R2 a 0 10k
D1 a k DP
R3 k 0 1k
.end
";

    /// Element values and model-card parameters both take expressions, and
    /// `.param` lines resolve in dependency order (`rtop` uses `rbot`
    /// before it is written). Matches ngspice-45 `.op`; fails if a card
    /// parameter keeps its default or `rtop` resolves to anything but 20k.
    #[test]
    fn global_params_drive_values_and_a_diode_card_like_ngspice() {
        let solver = build(DIODE);
        assert_close("v(a)", voltage(&solver, "a"), 6.510_423_545_356e-1, 1e-6);
        assert_close("v(k)", voltage(&solver, "k"), 1.523_436_453_668e-1, 1e-6);
        assert_close("i(v1)", branch(&solver, "V1"), -2.174_478_822_73e-4, 1e-6);
    }

    /// Substitution is a pure rewrite: the parameterised deck and its
    /// hand-written twin solve to identical bits.
    #[test]
    fn param_deck_is_bit_identical_to_its_literal_twin() {
        let param = build(DIODE);
        let literal = build(DIODE_LITERAL);
        for node in ["in", "a", "k"] {
            assert_eq!(
                voltage(&param, node).to_bits(),
                voltage(&literal, node).to_bits(),
                "v({node})"
            );
        }
    }

    /// A second, unrelated device: a power-switch macro with `params:`
    /// defaults (`w`, `l`, `rg`), a body `.param` derived from them, and a
    /// local level-1 NMOS card. It is called with defaults (`X3`), with
    /// spaced overrides (`X4 ... w = {..} rg=1k`), and from inside `PAIR`,
    /// whose own parameter `k` sets the inner call's width. Drain voltages
    /// and the internal drain node two levels down match ngspice-45.
    const SWITCHES: &str = "\
* parametric power-switch macro, nested with dynamic scoping
.temp 27
.options reltol=1e-12 vntol=1e-15 abstol=1e-18
.param wbase=20u vdd=5
.subckt NSW d g s params: w=10u l=1u rg=100
.param rd={10*1u/l}
RG g gi {rg}
RD d di {rd}
M1 di gi s s NCH w={w} l={l}
.model NCH NMOS(VTO=1.2 KP=60u LAMBDA=0.02 GAMMA=0.4 PHI=0.65)
.ends NSW
.subckt PAIR d1 d2 g params: k=2
X1 d1 g 0 NSW w={k*wbase}
X2 d2 g 0 NSW w={wbase} l=2u
.ends PAIR
VDD vdd 0 {vdd}
VG g 0 3.3
R1 vdd d1 1k
R2 vdd d2 1k
R3 vdd d3 1k
R4 vdd d4 1k
XP d1 d2 g PAIR params: k={1+2}
X3 d3 g 0 NSW
X4 d4 g 0 NSW w = {wbase*4} rg=1k
.control
op
set numdgt=12
print v(d1) v(d2) v(d3) v(d4) v(xp.x1.di) v(x4.di)
.endc
.end
";

    /// Fails if an override is ignored (`d1`/`d4` would equal `d3`), if the
    /// body `.param rd` ignores the caller's `l` (`d2` would equal `d3`),
    /// or if `k` is read from the default rather than the call.
    #[test]
    fn subckt_params_size_each_mosfet_instance_like_ngspice() {
        let solver = build(SWITCHES);
        for (node, reference) in [
            ("d1", 7.086_124_316_695e-1),
            ("d2", 3.582_397_312_365),
            ("d3", 3.582_579_979_501),
            ("d4", 5.413_126_427_971e-1),
            ("XP.X1.di", 6.656_985_559_862e-1),
            ("X4.di", 4.967_257_692_251e-1),
        ] {
            assert_close(
                &format!("v({node})"),
                voltage(&solver, node),
                reference,
                1e-6,
            );
        }
        // `d2` (W/L = 20u/2u, RD = 5) and `d3` (10u/1u, RD = 10) differ
        // only through the body `.param`; the reference gap is 183 µV.
        assert!((voltage(&solver, "d3") - voltage(&solver, "d2")).abs() > 1e-4);
    }

    /// ngspice scopes dynamically: a subcircuit body sees the parameters of
    /// the call that placed it. ngspice-45 gives `v(n1)` = 5007 V for 1 A.
    #[test]
    fn nested_body_sees_its_callers_parameters() {
        let solver = build(
            "\
.param g=7
.subckt INNER n
R1 n 0 {k*1000+g}
.ends
.subckt OUTER n params: k=3
X1 n INNER
.ends
I1 0 n1 1
X1 n1 OUTER k=5
.end
",
        );
        assert_eq!(voltage(&solver, "n1"), 5007.0);
    }

    /// What numparam cannot express here is an error, never a guess.
    #[test]
    fn unresolvable_parameters_are_refused() {
        let cases = [
            ("R1 a 0 {r}\nV1 a 0 1\n", "`r` is not defined"),
            (".param a={b}\n.param b={a}\nR1 n 0 {a}\n", "depends back"),
            (".param a=1\n.param a=2\nR1 n 0 {a}\n", "defined twice"),
            (
                ".subckt S n params: r=1\nR1 n 0 {r}\n.ends\nX1 a S q=2\n",
                "has no parameter `q`",
            ),
            (
                ".subckt S n params: is0=1n\nD1 n 0 DL\n.model DL D(IS={is0})\n.ends\n",
                "per-instance model cards",
            ),
            ("R1 a 0 {1/0}\nV1 a 0 1\n", "not finite"),
            (
                "R1 a 0 {v(a)>1}\nV1 a 0 1\n",
                "outside the expression subset",
            ),
            ("R1 a 0 {1+}\n", "ends early"),
        ];
        for (text, needle) in cases {
            let err = parse_netlist(text).expect_err(text);
            assert!(
                matches!(err, AnalogError::Parse { .. }) && err.to_string().contains(needle),
                "{text}: {err}"
            );
        }
        // A behavioural source keeps its own refusal: its braces are not
        // numparam.
        let err = parse_netlist("E1 a 0 VALUE={v(b)*2}\nR1 b 0 1\n").expect_err("value=");
        assert!(matches!(err, AnalogError::Unsupported { .. }), "{err}");
    }
}
