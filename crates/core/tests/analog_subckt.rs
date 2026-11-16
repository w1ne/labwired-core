// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Vendor SPICE text in the in-core engine: `+` continuation lines and
//! `.subckt` flattening.
//!
//! A part model copied from a datasheet is almost never one `.model` line:
//! it is a card split over `+` lines, usually wrapped in a `.subckt` that
//! adds package resistances, a body diode or a breakdown branch around the
//! primitives. Both are lowered here into what the solver already solves, so
//! the device physics is unchanged and the decks below are checked against
//! ngspice-45 `.op` on the same text, with ngspice's flattened names
//! (`X1.XHI.DI`, `V.X1.VZ`) used as the probes. The references were run with
//! `.temp 26.85`, `.options tnom=26.85` (the engine's fixed 300 K without
//! instance `TEMP`) and `reltol=1e-12 vntol=1e-15 abstol=1e-18`: at ngspice's
//! default `reltol=1e-3` its own zener answer moves by 11 µV.

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

    /// The MNA branch unknown of a voltage source, in ngspice's `i(V)` sign.
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

    /// A zener built the way vendor libraries build one: forward diode,
    /// reverse breakdown as a second diode in series with an internal source,
    /// and a series resistor. The cards span `+` lines.
    const ZENER: &str = "\
* vendor-style zener macro, called twice
.subckt DZ5V1 A K
D1 A K DF
DR K X DR
VZ X A 4.4
RS K K1 0.5
.model DF D(IS=2.5n N=1.75 RS=0.3
+ CJO=100p)
.model DR D(IS=1e-12 N=1.2)
.ends DZ5V1
V1 in 0 12
R1 in z1 1k
X1 0 z1 DZ5V1
V2 in2 0 -3
R2 in2 z2 220
X2 0 z2 DZ5V1
.end
";

    /// Hand-flattened [`ZENER`], written the way ngspice names it.
    const ZENER_FLAT: &str = "\
.model DZ5V1.DF D(IS=2.5n N=1.75 RS=0.3 CJO=100p)
.model DZ5V1.DR D(IS=1e-12 N=1.2)
V1 in 0 12
R1 in z1 1k
D.X1.D1 0 z1 DZ5V1.DF
D.X1.DR z1 X1.X DZ5V1.DR
V.X1.VZ X1.X 0 4.4
R.X1.RS z1 X1.K1 0.5
V2 in2 0 -3
R2 in2 z2 220
D.X2.D1 0 z2 DZ5V1.DF
D.X2.DR z2 X2.X DZ5V1.DR
V.X2.VZ X2.X 0 4.4
R.X2.RS z2 X2.K1 0.5
.end
";

    /// Reverse breakdown in one call and forward conduction in the other
    /// match ngspice-45 on the unflattened deck (`print v(z1) v(z2) i(v1)
    /// i(v2)`, `set numdgt=12`). Fails if a port is bound to the wrong
    /// caller node, if the local cards are not found, or if the internal
    /// source `VZ` is shared between the two calls.
    #[test]
    fn vendor_zener_subckt_matches_ngspice() {
        let solver = build(ZENER);
        assert_close("v(z1)", voltage(&solver, "z1"), 5.102_793_056_200, 1e-6);
        assert_close("v(z2)", voltage(&solver, "z2"), -0.693_038_257_004, 1e-6);
        assert_close("i(v1)", branch(&solver, "V1"), -6.897_206_943_80e-3, 1e-6);
        assert_close("i(v2)", branch(&solver, "V2"), 1.048_618_974_089e-2, 1e-6);
        // Each call owns its breakdown source, under ngspice's name.
        assert!(solver.circuit().branch_index("V.X1.VZ").is_some());
        assert!(solver.circuit().branch_index("V.X2.VZ").is_some());
    }

    /// Flattening is a pure rewrite: the subcircuit deck and its hand
    /// flattening solve to identical bits.
    #[test]
    fn subckt_deck_is_bit_identical_to_its_hand_flattening() {
        let nested = build(ZENER);
        let flat = build(ZENER_FLAT);
        for node in ["z1", "z2", "X1.X", "X2.X", "X1.K1"] {
            assert_eq!(
                voltage(&nested, node).to_bits(),
                voltage(&flat, node).to_bits(),
                "v({node})"
            );
        }
        for name in ["V1", "V2", "V.X1.VZ", "V.X2.VZ"] {
            assert_eq!(
                branch(&nested, name).to_bits(),
                branch(&flat, name).to_bits(),
                "i({name})"
            );
        }
    }

    /// A second, unrelated device: a power MOSFET macro (gate and drain
    /// resistance, body diode, local level-1 card with body-effect terms
    /// over `+` lines and a comment between them), called twice from a
    /// half-bridge `LEG` subcircuit that is itself called twice. Source
    /// follower and low-side switch operating points, and the internal drain
    /// node two levels down, match ngspice-45.
    #[test]
    fn nested_power_mosfet_subckt_matches_ngspice() {
        let solver = build(
            "\
.subckt PWRNFET D G S
RG G GI 2.2
RD D DI 0.04
M1 DI GI S S MINT L=2u W=1m
DBODY S D DBD
.model MINT NMOS(VTO=2.1 KP=20u LAMBDA=0.01
* thermal coefficients omitted in this deck
+ GAMMA=0.4 PHI=0.7)
.model DBD D(IS=1e-13 N=1.05 RS=0.01)
.ends
.subckt LEG VIN OUT GH GL
XHI VIN GH OUT PWRNFET
XLO OUT GL 0 PWRNFET
.ends
V1 vin 0 12
VGH gh 0 8
VGL gl 0 0
RL out 0 10
X1 vin out gh gl LEG
VGL2 gl2 0 4
RL2 vin out2 100
X2 vin out2 0 gl2 LEG
.end
",
        );
        assert_close("v(out)", voltage(&solver, "out"), 1.215_502_688_089, 1e-6);
        assert_close("v(out2)", voltage(&solver, "out2"), 10.014_256_996_68, 1e-6);
        assert_close(
            "v(x1.xhi.di)",
            voltage(&solver, "x1.xhi.di"),
            11.995_137_989_25,
            1e-6,
        );
        assert_close("i(v1)", branch(&solver, "V1"), -0.141_407_698_849, 1e-6);
    }

    /// A current-controlled source inside a body reads the branch of the
    /// body's own sense source, per call. A 1 mA and a 2 mA source each drive
    /// a call of a transimpedance sense cell (`H` gain 1 kΩ into 1 MΩ): the
    /// outputs are 1 V and 2 V. Fails if `H` names the unrenamed `VSENSE`,
    /// which does not exist, or one call's `VSENSE` for both.
    #[test]
    fn current_controlled_source_follows_its_local_sense_source() {
        let solver = build(
            "\
.subckt SENSE IN OUT
VSENSE IN 0 0
H1 OUT 0 VSENSE 1k
RLOAD OUT 0 1meg
.ends
I1 0 a 1m
I2 0 b 2m
XA a oa SENSE
XB b ob SENSE
",
        );
        assert_close("v(oa)", voltage(&solver, "oa"), 1.0, 1e-9);
        assert_close("v(ob)", voltage(&solver, "ob"), 2.0, 1e-9);
    }

    /// `+` lines join the card they continue, with comment lines between
    /// them skipped, and give the same circuit as the card on one line.
    #[test]
    fn continuation_lines_equal_the_single_line_card() {
        let split = build(
            "\
V1 a 0 5
R1 a k 1k
D1 k 0 DMOD
.model DMOD D(IS=1e-14
* a comment between continuations
+ N=1.9 ; trailing comment
+ RS=10)
",
        );
        let joined =
            build("V1 a 0 5\nR1 a k 1k\nD1 k 0 DMOD\n.model DMOD D(IS=1e-14 N=1.9 RS=10)\n");
        let unjoined = build("V1 a 0 5\nR1 a k 1k\nD1 k 0 DMOD\n.model DMOD D(IS=1e-14)\n");
        assert_eq!(
            voltage(&split, "k").to_bits(),
            voltage(&joined, "k").to_bits()
        );
        assert!((voltage(&split, "k") - voltage(&unjoined, "k")).abs() > 1e-3);
    }

    /// What is not flattened is refused, never approximated.
    #[test]
    fn subckt_constructs_outside_the_flattener_are_refused() {
        let unsupported = [
            // numparam: defaults would silently ignore the caller's value.
            ".subckt R2 a b params: r=1k\nR1 a b 1k\n.ends\nX1 n 0 R2\nV1 n 0 1\n",
            ".subckt R2 a b\nR1 a b 1k\n.ends\nX1 n 0 R2 r=2k\nV1 n 0 1\n",
            // Nested definitions and other directives inside a body.
            ".subckt A a b\n.subckt B c d\nR1 c d 1\n.ends\n.ends\n",
            ".subckt A a b\n.param r=1\nR1 a b 1\n.ends\n",
        ];
        for text in unsupported {
            let err = parse_netlist(text).expect_err(text);
            assert!(
                matches!(err, AnalogError::Unsupported { .. }),
                "{text}: {err}"
            );
        }
        let malformed = [
            (".subckt A a b\nR1 a b 1\n", "has no `.ends`"),
            ("R1 a 0 1\n.ends\n", "without a `.subckt`"),
            ("+ N=2\nR1 a 0 1\n", "no line to continue"),
            (
                ".subckt A a b\nR1 a b 1\n.ends\nX1 n A\n",
                "connects 1 nodes",
            ),
            (".subckt A a b\nX1 a b A\n.ends\nX1 n 0 A\n", "calls itself"),
            (".subckt A a a\nR1 a 0 1\n.ends\n", "listed twice"),
            (".subckt A a\n.ends\n.subckt a b\n.ends\n", "declared twice"),
        ];
        for (text, needle) in malformed {
            let err = parse_netlist(text).expect_err(text);
            assert!(
                matches!(err, AnalogError::Parse { .. }) && err.to_string().contains(needle),
                "{text}: {err}"
            );
        }
        // A call to a name that is neither a subcircuit nor an OPAMP/COMP
        // card is still a subcircuit the engine does not have.
        let err = parse_netlist("X1 a b nothing\n").expect_err("unknown subckt");
        assert!(matches!(err, AnalogError::Unsupported { .. }), "{err}");
    }
}
