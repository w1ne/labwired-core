// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Behavioural sources (`B ... V=`/`I=`, `E`/`G ... VALUE={...}`) in the
//! in-core engine.
//!
//! A behavioural source lets a user wire any analog law (a sensor, a soft
//! limiter, a transconductor) without a bespoke device model. Each deck below
//! is the exact text ngspice-42 ran (its `.control` block is skipped here);
//! the references are its `.op` answers printed with 12 digits.

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

    /// A square-law current load, a zero-volt ammeter, and a voltage source
    /// built from that ammeter's current and a two-node difference. `I(B2)`
    /// is the branch current of a voltage-form `B`, as in ngspice.
    const SQUARE_LAW: &str = "\
* B-source: square-law load and a current-sensing voltage
.temp 27
.options reltol=1e-10 vntol=1e-12 abstol=1e-15
V1 in 0 5
R1 in a 1k
B1 a 0 I=2m*V(a)^2
Vs a b 0
R2 b 0 2k
B2 c 0 V=1k*I(Vs) + 0.5*V(a,c0)
Rc0 c0 0 1k
I0 0 c0 1m
R3 c 0 1k
.control
op
set numdgt=12
print v(a) v(b) v(c) i(v1) i(vs) i(b2)
.endc
.end
";

    #[test]
    fn square_law_load_and_current_sense_match_ngspice() {
        let solver = build(SQUARE_LAW);
        assert_close("v(a)", voltage(&solver, "a"), 1.25, 1e-9);
        assert_close("v(b)", voltage(&solver, "b"), 1.25, 1e-9);
        assert_close("v(c)", voltage(&solver, "c"), 0.75, 1e-9);
        assert_close("i(v1)", branch(&solver, "V1"), -3.75e-3, 1e-9);
        assert_close("i(vs)", branch(&solver, "Vs"), 6.25e-4, 1e-9);
        assert_close("i(b2)", branch(&solver, "B2"), -7.5e-4, 1e-9);
    }

    /// Two unrelated devices with no model of their own: an NTC thermistor
    /// written as its beta law (`B ... I=`, parameters inside the braces), a
    /// soft limiter (`E ... VALUE=` with `?:`) and a tanh transconductor
    /// (`G ... VALUE=`) that biases a real Gummel–Poon NPN follower.
    const SENSOR_CHAIN: &str = "\
* B-source thermistor law and E/G VALUE forms with a real BJT
.temp 27
.options reltol=1e-10 vntol=1e-12 abstol=1e-15
.param r0=10k beta=3950 t=50
V1 vcc 0 3.3
R1 vcc s 2k
B1 s 0 I={V(s)/(r0*exp(beta*(1/(t+273.15) - 1/298.15)))}
E1 x 0 VALUE={V(s) > 1.2 ? 1.2 + 0.1*(V(s)-1.2) : V(s)}
G1 vcc y VALUE={1m*tanh((V(x) - 1.2)/0.2) + 1m}
Ry y 0 1k
.model QN NPN(IS=1e-14 BF=120 VAF=80)
Q1 vcc y e QN
Re e 0 4.7k
.control
op
set numdgt=12
print v(s) v(x) v(y) v(e) i(v1) i(e1)
.endc
.end
";

    #[test]
    fn thermistor_limiter_and_transconductor_match_ngspice() {
        let solver = build(SENSOR_CHAIN);
        assert_close("v(s)", voltage(&solver, "s"), 2.118936227792, 1e-9);
        assert_close("v(x)", voltage(&solver, "x"), 1.291893622779, 1e-9);
        assert_close("v(y)", voltage(&solver, "y"), 1.428242935106, 1e-9);
        // The emitter is the BJT's own answer; the engine's Gummel–Poon
        // sits 2.4e-7 relative from ngspice here with no behavioural source
        // involved; `analog_include.rs` allows its NPN the same 1e-6.
        assert_close("v(e)", voltage(&solver, "e"), 8.191212163971e-1, 1e-6);
        // The supply carries the BJT's collector current too, with that
        // same gap scaled by its 0.17 mA share of 2.19 mA.
        assert_close("i(v1)", branch(&solver, "V1"), -2.19305593438e-3, 1e-7);
        assert_eq!(branch(&solver, "E1"), 0.0, "E1 drives only V(x) readers");
    }

    /// The same thermistor at another temperature must move the answer the
    /// way the beta law says: a hotter NTC pulls the divider down.
    #[test]
    fn thermistor_law_follows_its_parameter() {
        let at = |t: f64| {
            let deck = SENSOR_CHAIN.replace("t=50", &format!("t={t}"));
            voltage(&build(&deck), "s")
        };
        let rntc = |t: f64| 10e3 * (3950.0 * (1.0 / (t + 273.15) - 1.0 / 298.15)).exp();
        for t in [0.0, 25.0, 85.0] {
            let expected = 3.3 * rntc(t) / (2e3 + rntc(t));
            assert_close(&format!("v(s) at {t} °C"), at(t), expected, 1e-9);
        }
    }

    /// `V(...)` on a port and on an internal node, and `I(...)` of an
    /// internal source, renamed per call; `{k}` from each call's `params:`.
    const SUBCIRCUIT: &str = "\
* B-sources inside a subcircuit, placed twice
.temp 27
.options reltol=1e-10 vntol=1e-12 abstol=1e-15
.subckt sqrtload in out params: k=1m
Vsense in mid 0
B1 mid 0 I={k}*sqrt(V(mid))
B2 out 0 V=500*I(Vsense) + V(mid)/10
Rout out 0 10k
.ends
V1 vcc 0 4
R1 vcc a 1k
X1 a o1 sqrtload k=2m
R2 vcc b 3k
X2 b o2 sqrtload
.control
op
set numdgt=12
print v(a) v(o1) v(b) v(o2) i(v1)
.endc
.end
";

    #[test]
    fn subcircuit_behavioural_sources_match_ngspice() {
        let solver = build(SUBCIRCUIT);
        assert_close("v(a)", voltage(&solver, "a"), 1.527864045, 1e-9);
        assert_close("v(o1)", voltage(&solver, "o1"), 1.388854382, 1e-9);
        assert_close("v(b)", voltage(&solver, "b"), 1.0, 1e-9);
        assert_close("v(o2)", voltage(&solver, "o2"), 0.6, 1e-9);
        assert_close("i(v1)", branch(&solver, "V1"), -3.472135955e-3, 1e-9);
        assert!(solver.circuit().branch_index("B.X1.B2").is_some());
    }

    /// `time` is the end of the step being solved, so a `V=` source of time
    /// alone is exact at every step, and a `B ... I=` charging a capacitor
    /// integrates like an `I` source with the same waveform.
    #[test]
    fn time_drives_the_transient() {
        let mut solver = build(
            "\
B1 o 0 V=2*sin(2*pi*1k*time) + 0.5
R1 o 0 1k
B2 0 c I=1m*u(time - 0.5m)
C1 c 0 1u
Rleak c 0 1g
",
        );
        assert_close("v(o) at 0", voltage(&solver, "o"), 0.5, 1e-12);
        assert_eq!(voltage(&solver, "c"), 0.0);
        let h = 1e-5;
        for step in 1..=100 {
            solver.advance(h).unwrap();
            let t = step as f64 * h;
            let expected = 2.0 * (2.0 * std::f64::consts::PI * 1e3 * t).sin() + 0.5;
            assert!(
                (voltage(&solver, "o") - expected).abs() < 1e-12,
                "v(o) at {t}"
            );
        }
        // 1 mA into 1 µF for the 0.5 ms after the step: 0.5 V, give or take
        // the one 10 mV step that lands on the edge of u().
        let charged = voltage(&solver, "c");
        assert!((charged - 0.5).abs() <= 0.0101, "v(c) at 1 ms: {charged}");
    }

    #[test]
    fn mistakes_are_refused_with_the_line() {
        let refused = |deck: &str, needle: &str| {
            let error = parse_netlist(deck).unwrap_err();
            assert!(error.to_string().contains(needle), "{deck}: {error}");
            assert!(error.line().is_some(), "{deck}: {error}");
        };
        refused("V1 a 0 1\nB1 b 0 V=V(nowhere)\n", "node `nowhere`");
        refused("R1 a 0 1k\nB1 a 0 I=I(R1)\n", "carries no branch current");
        refused("B1 a 0 X=1\n", "`V=<expr>` or `I=<expr>`");
        refused("B1 a 0 V=temper\n", "not a behavioural quantity");
        refused("B1 a 0 V=foo(V(a))\n", "function `foo`");
        refused("R1 a 0 1\nB1 a 0 V=1\nB1 a 0 V=2\n", "declared twice");
        // POLY and TABLE stay outside the subset.
        refused("V1 a 0 1\nE1 b 0 POLY(1) a 0 0 1\n", "");
    }

    #[test]
    fn leaving_the_domain_names_the_source() {
        let circuit = parse_netlist("V1 a 0 -1\nB1 b 0 V=sqrt(V(a))\nR1 b 0 1k\n").unwrap();
        let error = Solver::new(circuit, Integration::BackwardEuler).unwrap_err();
        match &error {
            AnalogError::Evaluation { element, .. } => assert_eq!(element, "B1"),
            other => panic!("expected an evaluation error, got {other}"),
        }
        assert!(
            error.to_string().contains("out of range for `sqrt`"),
            "{error}"
        );
    }
}
