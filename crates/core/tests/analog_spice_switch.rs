// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SPICE's own switches in the in-core engine: `S n+ n- nc+ nc- <model>` with
//! an `SW` card and `W n+ n- <element> <model>` with a `CSW` card.
//!
//! This is the spelling every vendor deck and textbook uses for a relay
//! contact, a load switch, an over-current trip or a latch, so a deck that
//! holds one runs here as written. Each deck below is the exact text
//! ngspice-42 ran (its `.control` block is skipped here); the references are
//! its answers printed with 12 digits for `.op` and 7 for `.tran`.

mod analog {
    use labwired_core::analog::{
        parse_netlist, AnalogError, Integration, Solver, SwitchSense, ThresholdSwitchControl,
    };

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

    fn parse_error(netlist: &str) -> String {
        match parse_netlist(netlist) {
            Err(AnalogError::Parse { message, .. }) => message,
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    /// Seven pull-ups, each shunted by one switch, cover the state rule:
    /// `out` sits exactly on `VT + VH` and stays open (the comparison is
    /// strict); `o2` closes on 30 mA through `Vs`; `o3`/`o4` share a control
    /// inside the band and differ only by `ON`/`OFF`; `o5` senses the same
    /// 30 mA with the source reversed and stays open (the current is signed);
    /// `o6`/`o7` use a card with no parameters (`VT = 0`, `RON = 1`,
    /// `ROFF = 1e12`) with the control nodes swapped.
    const STATES: &str = "\
* sw op test
V1 vdd 0 5
VC c 0 2.0
R1 vdd out 1k
S1 out 0 c 0 smod
.model smod SW(VT=1.5 VH=0.5 RON=10 ROFF=1meg)
VL lv 0 3
RL lv ls 100
Vs ls 0 0
R2 vdd o2 1k
W1 o2 0 Vs wmod
.model wmod CSW(IT=20m IH=5m RON=2 ROFF=10meg)
VC3 c3 0 1.5
R3 vdd o3 1k
S3 o3 0 c3 0 smod ON
R4 vdd o4 1k
S4 o4 0 c3 0 smod OFF
R5 vdd o5 1k
W5 o5 0 Vs2 wmod
VL2 lv2 0 3
Vs2 0 ls2 0
RL2 lv2 ls2 100
R6 vdd o6 1k
S6 o6 0 c 0 sdef
.model sdef SW
R7 vdd o7 1k
S7 o7 0 0 c sdef
.control
op
set numdgt=12
print v(out) v(o2) v(o3) v(o4) v(o5) v(o6) v(o7)
.endc
.end
";

    #[test]
    fn operating_point_states_match_ngspice() {
        let solver = build(STATES);
        for (node, reference) in [
            ("out", 4.995004995005e+00),
            ("o2", 9.980039920160e-03),
            ("o3", 4.950495049505e-02),
            ("o4", 4.995004995005e+00),
            ("o5", 4.999500049995e+00),
            ("o6", 4.995004995005e-03),
            ("o7", 4.999999995000e+00),
        ] {
            assert_close(node, voltage(&solver, node), reference, 1e-9);
        }
        assert!(
            solver.switches_controlled_by("").is_empty(),
            "a SPICE switch is not a routed input"
        );
        let circuit = solver.circuit();
        let w1 = &circuit.switches[solver.switch_index("W1").expect("W1 exists")];
        assert_eq!((w1.ron, w1.roff), (2.0, 10e6));
        assert!(matches!(
            &w1.threshold,
            Some(ThresholdSwitchControl {
                sense: SwitchSense::Current { control, .. },
                threshold,
                hysteresis,
                initially_closed: false,
            }) if control == "Vs" && *threshold == 20e-3 && *hysteresis == 5e-3
        ));
    }

    /// One 0 → 3 V → 0 ramp drives an `SW` (band 1 V..2 V) directly and a
    /// `CSW` (band 10 mA..20 mA) through a 100 Ω load, so both must close at
    /// 6.667 ms and open at 16.667 ms. Each output has 100 nF to ground.
    const HYSTERESIS: &str = "\
* sw tran hysteresis
Vdd vdd 0 5
Vc c 0 PWL(0 0 10m 3 20m 0)
R1 vdd out 1k
S1 out 0 c 0 smod
C1 out 0 100n
.model smod SW(VT=1.5 VH=0.5 RON=10 ROFF=1meg)
Vl lv 0 PWL(0 0 10m 3 20m 0)
Rl lv ls 100
Vs ls 0 0
R2 vdd o2 1k
W1 o2 0 Vs wmod
C2 o2 0 100n
.model wmod CSW(IT=15m IH=5m RON=10 ROFF=1meg)
.options reltol=1e-9 abstol=1e-15 vntol=1e-12
.control
tran 1u 20m 0 1u
meas tran a1 find v(out) at=3m
meas tran ton when v(out)=2.5 fall=1
meas tran toff when v(out)=2.5 rise=1
.endc
.end
";

    #[test]
    fn transient_hysteresis_matches_ngspice() {
        let mut solver = build(HYSTERESIS);
        let step = 1e-6;
        // (time in steps, ngspice-42 value on both `out` and `o2`).
        let samples = [
            (3_000, 4.995005e+00),
            (6_000, 4.995005e+00),
            (8_000, 4.950495e-02),
            (12_000, 4.950495e-02),
            (15_500, 4.950495e-02),
            (18_000, 4.994997e+00),
            (19_900, 4.995005e+00),
        ];
        let mut next = samples.iter().peekable();
        let mut previous = voltage(&solver, "out");
        let (mut closed_at, mut opened_at) = (None, None);
        for index in 1..=20_000usize {
            solver
                .advance(step)
                .unwrap_or_else(|error| panic!("{error}"));
            let out = voltage(&solver, "out");
            if previous > 2.5 && out <= 2.5 {
                closed_at.get_or_insert(index as f64 * step);
            }
            if previous < 2.5 && out >= 2.5 {
                opened_at.get_or_insert(index as f64 * step);
            }
            previous = out;
            if next.peek().is_some_and(|(at, _)| *at == index) {
                let (_, reference) = next.next().expect("peeked");
                assert_close(&format!("out at step {index}"), out, *reference, 2e-6);
                assert_close(
                    &format!("o2 at step {index}"),
                    voltage(&solver, "o2"),
                    *reference,
                    2e-6,
                );
            }
        }
        // ngspice-42: 6.666976 ms and 16.73485 ms. The state here is taken
        // from the last accepted point, so an edge may land one step later.
        let closed_at = closed_at.expect("the contact closes");
        let opened_at = opened_at.expect("the contact opens");
        assert!(
            (closed_at - 6.666976e-3).abs() < 3e-6,
            "closes at {closed_at}"
        );
        assert!(
            (opened_at - 16.73485e-3).abs() < 3e-6,
            "opens at {opened_at}"
        );
    }

    /// A switch that senses its own output is a latch: `S1` feeds `g` from
    /// the supply once `g` passes 2.5 V and holds it there after the input
    /// returns to 0 V, because 4.5 V stays above the 1.5 V release level.
    const LATCH: &str = "\
* latch
Vdd vdd 0 5
Vin in 0 PWL(0 0 1m 0 2m 3 3m 3 4m 0 6m 0)
Rin in g 1k
S1 vdd g g 0 lat
Rg g 0 10k
C1 g 0 10n
.model lat SW(VT=2 VH=0.5 RON=100 ROFF=100meg)
.options reltol=1e-9 abstol=1e-15 vntol=1e-12
.control
tran 1u 6m 0 1u
meas tran g1 find v(g) at=0.9m
.endc
.end
";

    #[test]
    fn self_sensing_switch_latches_like_ngspice() {
        let mut solver = build(LATCH);
        let mut steps = 0;
        for (at, reference, tolerance) in [
            (900, 4.545413e-05, 1e-6),
            // Mid-ramp, where the two integration rules differ by a step.
            (1_500, 1.338876e+00, 2e-3),
            (2_900, 4.774775e+00, 1e-6),
            (5_900, 4.504505e+00, 1e-6),
        ] {
            while steps < at {
                solver
                    .advance(1e-6)
                    .unwrap_or_else(|error| panic!("{error}"));
                steps += 1;
            }
            assert_close(
                &format!("g at step {at}"),
                voltage(&solver, "g"),
                reference,
                tolerance,
            );
        }
    }

    /// The switches and their cards live inside a `.subckt`, so the nodes,
    /// the sensed `Vs` and both model names are flattened per call. `X1`
    /// carries 60 mA (closed), `X2` 40 mA (inside the band, open), and `X3`
    /// closes from 60 mA on a 5 V input, which also closes the `S2` clamp.
    const SUBCKT: &str = "\
* sw in subckt
.subckt trip in out sense_p sense_n
Vs sense_p sense_n 0
W1 in out Vs lim
S2 out 0 in 0 clampm
.model lim CSW(IT=40m IH=10m RON=0.5 ROFF=10meg)
.model clampm SW(VT=4 VH=0.2 RON=100 ROFF=1g)
.ends
Vdd vdd 0 3.3
Vsrc src 0 6
Rload src lp 100
X1 vdd o1 lp 0 trip
Ro1 o1 0 1k
Vsrc2 src2 0 4
Rload2 src2 lp2 100
X2 vdd o2 lp2 0 trip
Ro2 o2 0 1k
Vhi hi 0 5
Vsrc3 src3 0 6
Rload3 src3 lp3 100
X3 hi o3 lp3 0 trip
Ro3 o3 0 1k
.control
op
set numdgt=12
print v(o1) v(o2) v(o3)
.endc
.end
";

    #[test]
    fn switches_inside_a_subcircuit_match_ngspice() {
        let solver = build(SUBCKT);
        for (node, reference) in [
            ("o1", 3.298350822939e+00),
            ("o2", 3.299666733660e-04),
            ("o3", 4.972650422675e+00),
        ] {
            assert_close(node, voltage(&solver, node), reference, 1e-9);
        }
    }

    /// Inside the band a `W ... ON` holds closed, the rule `swload.c` uses
    /// for `S`. ngspice-42's `cswload.c` reads the band state from the
    /// previous time point instead, which no operating point has set, so it
    /// reports this contact open; the references are therefore closed forms.
    /// An inductor may also be the sensed element, as for `F` and `H`.
    #[test]
    fn current_switch_holds_its_on_flag_inside_the_band() {
        let solver = build(
            "* csw band\n\
             Vdd vdd 0 3.3\n\
             Vsrc src 0 4\n\
             Rload src lp 100\n\
             Lsense lp 0 1m\n\
             W1 vdd on Lsense lim ON\n\
             Ron on 0 1k\n\
             W2 vdd off Lsense lim\n\
             Roff off 0 1k\n\
             .model lim CSW(IT=40m IH=10m RON=0.5 ROFF=10meg)\n",
        );
        assert_close("on", voltage(&solver, "on"), 3.3 * 1000.0 / 1000.5, 1e-12);
        assert_close(
            "off",
            voltage(&solver, "off"),
            3.3 * 1e3 / (1e7 + 1e3),
            1e-12,
        );
    }

    #[test]
    fn malformed_switches_are_refused_by_name() {
        let deck =
            |lines: &str| format!("* bad\nV1 a 0 1\nR1 a b 1k\nVs b 0 0\nR2 a c 1k\n{lines}");
        for (lines, expected) in [
            ("S1 c 0 a 0 nomodel\n", "no `.model nomodel SW(...)` line"),
            ("W1 c 0 Vs nomodel\n", "no `.model nomodel CSW(...)` line"),
            (
                "W1 c 0 Vs m\n.model m SW(VT=1)\n",
                "needs a CSW model, but `m` is a SW model",
            ),
            (
                "S1 c 0 a 0 m\n.model m CSW(IT=1)\n",
                "needs a SW model, but `m` is a CSW model",
            ),
            (
                "S1 c 0 a 0 m\n.model m D(IS=1e-14)\n",
                "needs a SW model, but `m` is a D model",
            ),
            (
                "W1 c 0 R1 m\n.model m CSW(IT=1m)\n",
                "`R1` carries no branch current",
            ),
            (
                "S1 c 0 a 0 m\n.model m SW(VT=1 VH=-0.1)\n",
                "negative switch hysteresis is not modelled",
            ),
            (
                "S1 c 0 a 0 m\n.model m SW(VT=1 IT=1)\n",
                "`IT` is not a SW parameter",
            ),
            (
                "W1 c 0 Vs m\n.model m CSW(IT=1m LSER=1n)\n",
                "`LSER` is not a CSW parameter",
            ),
            (
                "S1 c 0 a 0 m\n.model m SW(RON=0)\n",
                "RON and ROFF must be positive",
            ),
            (
                "S1 c 0 a 0 m open\n.model m SW(VT=1)\n",
                "expected `S<name> n+ n- nc+ nc- <model> [on|off]`",
            ),
            (
                "W1 c 0 Vs\n.model m CSW(IT=1m)\n",
                "expected `W<name> n+ n- <element> <model> [on|off]`",
            ),
        ] {
            let message = parse_error(&deck(lines));
            assert!(
                message.contains(expected),
                "`{}`: expected `{expected}` in `{message}`",
                lines.trim_end()
            );
        }
    }
}
