// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ngspice `.func` user-defined functions in the in-core engine.
//!
//! A vendor deck or a reusable user circuit names its arithmetic once
//! (`.func rntc(t) {...}`, `.func par(a, b) {...}`) and calls it from element
//! values, model cards and subcircuit parameters. Each deck below is the
//! exact text ngspice-42 ran (its `.control` block is skipped here), with
//! `.temp 27` so both engines use the same temperature.

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
            "{what}: ngspice-42 {reference:e}, got {got:e}"
        );
    }

    /// An NTC beta law at 50 °C, a parallel-resistor helper called with a
    /// nested call as its argument, and a diode card whose `IS` and `RS`
    /// come from functions. `rntc` uses the global `.param` values `r0` and
    /// `b` that it does not take as arguments; `=` before the body is
    /// optional, as in ngspice.
    const NTC_DIODE: &str = "\
* .func: thermistor divider and diode card
.temp 27
.options reltol=1e-10 vntol=1e-12 abstol=1e-15
.param r0=10k b=3950
.func par(x, y) {x*y/(x+y)}
.func rntc(t) = {r0*exp(b*(1/(t+273.15) - 1/298.15))}
.func isat(n) {2.5n*n}
.model DP D(IS={isat(2)} N={1.5+0.25} RS={par(20m, 20m)})
V1 in 0 5
R1 in a {par(r0, 2*r0)}
R2 a 0 {rntc(50)}
D1 a k DP
R3 k 0 {par(rntc(25), r0) / 5}
.control
op
set numdgt=12
print v(a) v(k) i(v1)
.endc
.end
";

    /// [`NTC_DIODE`] with every call evaluated by hand. ngspice-42 gives
    /// this deck the same answer as the one with functions.
    const NTC_DIODE_LITERAL: &str = "\
.temp 27
.model DP D(IS=5n N=1.75 RS=10m)
V1 in 0 5
R1 in a {10k*20k/(10k+20k)}
R2 a 0 {10k*exp(3950*(1/(50+273.15) - 1/298.15))}
D1 a k DP
R3 k 0 1k
.end
";

    /// Matches ngspice-42 `.op`. Fails if a function body is not bound to
    /// its arguments (`par(r0, 2*r0)` must be 6.667k, not 20k), if `rntc`
    /// loses the global `b`, or if a card parameter keeps its default.
    #[test]
    fn functions_drive_values_and_a_diode_card_like_ngspice() {
        let solver = build(NTC_DIODE);
        assert_close("v(a)", voltage(&solver, "a"), 8.803_899_816_104e-1, 1e-6);
        assert_close("v(k)", voltage(&solver, "k"), 3.725_833_133_247e-1, 1e-6);
        assert_close("i(v1)", branch(&solver, "V1"), -6.179_415_027_58e-4, 1e-6);
    }

    /// Expansion is a pure rewrite: the deck with functions and its
    /// hand-written twin solve to identical bits.
    #[test]
    fn function_deck_is_bit_identical_to_its_literal_twin() {
        let func = build(NTC_DIODE);
        let literal = build(NTC_DIODE_LITERAL);
        for node in ["in", "a", "k"] {
            assert_eq!(
                voltage(&func, node).to_bits(),
                voltage(&literal, node).to_bits(),
                "v({node})"
            );
        }
    }

    /// A second, unrelated device: a level-1 NMOS switch macro. Its body
    /// defines `wl` again, which shadows the global `wl` inside the body
    /// only, and `rg`, which exists only there. `X1` takes the default
    /// width (local `wl(10u)` = 40u). `X2` passes `w={wl(10u)}`, which the
    /// caller evaluates with the global `wl` (20u) before the body applies
    /// its own (80u). Drain voltages match ngspice-42.
    const SWITCHES: &str = "\
* .func local to a subckt, shadowing a global one
.temp 27
.options reltol=1e-12 vntol=1e-15 abstol=1e-18
.func wl(w) {w*2}
.subckt NSW d g s params: w=10u
.func wl(w) {w*4}
.func rg(x) {x*100}
RG g gi {rg(1)}
M1 d gi s s NCH w={wl(w)} l=1u
.model NCH NMOS(VTO=1.2 KP=60u LAMBDA=0.02)
.ends NSW
VDD vdd 0 5
VG g 0 3.3
R1 vdd d1 1k
R2 vdd d2 1k
X1 d1 g 0 NSW
X2 d2 g 0 NSW w={wl(10u)}
.control
op
set numdgt=12
print v(d1) v(d2)
.endc
.end
";

    /// Fails if the local `wl` leaks out (X2 would get 40u, not 20u, at the
    /// call) or the global one leaks in (X1 would get 20u).
    #[test]
    fn subckt_local_function_shadows_the_global_one_like_ngspice() {
        let solver = build(SWITCHES);
        assert_close("v(d1)", voltage(&solver, "d1"), 1.022_224_843_285, 1e-6);
        assert_close("v(d2)", voltage(&solver, "d2"), 5.017_643_876_789e-1, 1e-6);
    }

    /// A value written bare after `=` on a `.param` or `X` line may call a
    /// function, and a function may call another one. ngspice-42 gives
    /// i(v1) = -4.00000e-03 for this deck.
    #[test]
    fn bare_values_and_nested_functions_expand() {
        let solver = build(
            "\
.func half(x) {x/2}
.func quarter(x) {half(half(x))}
.param r=quarter(4k)
.subckt LOAD n params: r=1
R1 n 0 {r}
.ends
V1 a 0 2
X1 a LOAD r=half(r)
",
        );
        // 2 V across quarter(4k)/2 = 500 Ω.
        assert_close("i(v1)", branch(&solver, "V1"), -2.0 / 500.0, 1e-12);
    }

    /// What `.func` cannot express here is an error, never a guess.
    #[test]
    fn invalid_functions_are_refused() {
        let cases = [
            (
                ".func f(x) {f(x)}\nR1 a 0 {f(1)}\nV1 a 0 1\n",
                "`.func f` calls itself",
            ),
            (
                ".func f(x) {g(x)}\n.func g(x) {f(x)}\nR1 a 0 {f(1)}\nV1 a 0 1\n",
                "calls itself",
            ),
            (
                ".func f(x, y) {x+y}\nR1 a 0 {f(1)}\nV1 a 0 1\n",
                "`f` takes 2 arguments, got 1",
            ),
            (
                ".func f(x) {x}\n.func f(y) {y}\nR1 a 0 1\n",
                "defined twice",
            ),
            (".func sqrt(x) {x}\nR1 a 0 1\n", "shadow a built-in"),
            (".func f(x, x) {x}\nR1 a 0 1\n", "listed twice"),
            (".func f x\nR1 a 0 1\n", "expected `.func"),
            (
                ".func f(x) {x*k}\nR1 a 0 {f(1)}\nV1 a 0 1\n",
                "`k` is not defined",
            ),
        ];
        for (text, needle) in cases {
            let err = parse_netlist(text).expect_err(text);
            assert!(
                matches!(err, AnalogError::Parse { .. }) && err.to_string().contains(needle),
                "{text}: {err}"
            );
        }
        // A behavioural source keeps its own refusal: its braces are not
        // expanded.
        let err = parse_netlist(".func f(x) {x}\nE1 a 0 VALUE={f(v(b))}\nR1 b 0 1\n")
            .expect_err("value=");
        assert!(matches!(err, AnalogError::Unsupported { .. }), "{err}");
    }
}
