// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SPICE's lossless transmission line, `T n1+ n1- n2+ n2- Z0=.. TD=..`, in
//! the in-core engine.
//!
//! This is the spelling a deck uses for a cable, a PCB trace or a delay, so
//! a deck that holds one runs here as written. The operating-point and
//! bounce references are ngspice-42's answers for the exact text below (its
//! `.control` block is skipped here), printed with 12 digits for `.op` and 7
//! for `.tran`; the quarter-wave and long-step cases are closed forms.

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

    fn assert_close(what: &str, got: f64, reference: f64, tolerance: f64) {
        let scale = reference.abs().max(1e-9);
        assert!(
            ((got - reference) / scale).abs() < tolerance,
            "{what}: reference {reference:e}, got {got:e}"
        );
    }

    fn parse_error(netlist: &str) -> String {
        match parse_netlist(netlist) {
            Err(AnalogError::Parse { message, .. }) => message,
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    /// At DC a line is two wires. `T2` floats both of its reference nodes on
    /// resistors: only the differential mode exists, so the current that
    /// enters `b` leaves at `ref`, and the same current enters `ref2` and
    /// leaves at `far`, which pulls `ref2` BELOW ground. `F` with `NL` gives
    /// the delay as `NL / F`.
    const OPERATING_POINT: &str = "\
* tline op
V1 in 0 5
Rs in a 50
T1 a 0 b 0 Z0=75 TD=1n
RL b 0 100
T2 b ref far ref2 Z0=120 F=100meg NL=0.5
Rref ref 0 10
Rref2 ref2 0 30
Rfar far 0 1k
.control
set numdgt=12
op
print v(a) v(b) v(far) v(ref) v(ref2) i(V1)
.endc
.end
";

    #[test]
    fn operating_point_matches_ngspice_and_carries_only_the_differential_mode() {
        let solver = build(OPERATING_POINT);
        for (node, reference) in [
            ("a", 3.229813664597e+00),
            ("b", 3.229813664595e+00),
            ("far", 3.105590062110e+00),
            ("ref", 3.105590062110e-02),
            ("ref2", -9.31677018633e-02),
        ] {
            assert_close(node, voltage(&solver, node), reference, 1e-10);
        }
        let source = solver.voltage_source_index("V1").unwrap();
        assert_close(
            "i(V1)",
            solver.branch_current(source),
            -3.54037267081e-02,
            1e-10,
        );
        let lines = &solver.circuit().transmission_lines;
        assert_eq!(lines[0].delay, 1e-9);
        assert_eq!(lines[1].delay, 0.5 / 100e6);
        assert_eq!(lines[1].z0, 120.0);
    }

    /// A 1 V edge into a mismatched line: 25 Ω source, 50 Ω line, 200 Ω
    /// load. The near end steps to `Z0/(Rs+Z0)` = 2/3 V at once and the far
    /// end stays at 0 V for one delay; after that each end moves once per
    /// round trip by the product of the reflection coefficients (+0.6 at the
    /// load, −1/3 at the source) towards the DC divider 8/9 V.
    const BOUNCE: &str = "\
* tline bounce
V1 in 0 PWL(0 0 1n 1)
Rs in a 25
T1 a 0 b 0 Z0=50 TD=10n
RL b 0 200
.options reltol=1e-9 abstol=1e-15 vntol=1e-12
.control
set numdgt=10
tran 1n 100n 0 0.1n
meas tran a5 find v(a) at=5n
meas tran b105 find v(b) at=10.5n
.endc
.end
";

    /// `(node, time in ns, ngspice-42 volts)`.
    const BOUNCE_REFERENCE: [(&str, u32, f64); 11] = [
        ("b", 5, 0.0),
        ("a", 5, 6.666667e-01),
        ("b", 15, 1.066667e+00),
        ("a", 25, 9.333333e-01),
        ("b", 35, 8.533333e-01),
        ("a", 45, 8.800000e-01),
        ("b", 55, 8.960000e-01),
        ("a", 65, 8.906667e-01),
        ("b", 75, 8.874667e-01),
        ("a", 85, 8.885333e-01),
        ("b", 95, 8.891733e-01),
    ];

    #[test]
    fn bounce_diagram_matches_ngspice_under_both_integration_rules() {
        for rule in [Integration::BackwardEuler, Integration::Trapezoidal] {
            let circuit = parse_netlist(BOUNCE).unwrap();
            let mut solver = Solver::new(circuit, rule).unwrap();
            let mut seen = 0;
            for step in 1..=200u32 {
                solver.advance(0.5e-9).unwrap();
                if step == 21 {
                    // Half way up the edge as it arrives: the delay is exact,
                    // not rounded to a whole number of steps of the caller.
                    assert_close("v(b) at 10.5 ns", voltage(&solver, "b"), 5.333333e-01, 1e-6);
                }
                for (node, at_ns, reference) in BOUNCE_REFERENCE {
                    if step == 2 * at_ns {
                        seen += 1;
                        let got = voltage(&solver, node);
                        if reference == 0.0 {
                            assert!(got.abs() < 1e-12, "v(b) before the edge arrives: {got}");
                        } else {
                            assert_close(&format!("v({node}) at {at_ns} ns"), got, reference, 1e-6);
                        }
                    }
                }
            }
            assert_eq!(seen, BOUNCE_REFERENCE.len());
        }
    }

    /// The staircase does not depend on the step dividing the delay: with a
    /// step of 0.3 ns against 10 ns the plateaus are the same, because the
    /// delayed wave is interpolated, not read from the nearest step. What
    /// the interpolation costs is the edge: each pass down the line widens
    /// it by up to one step, so the plateaus are read 5 ns clear of it.
    #[test]
    fn plateaus_hold_when_the_step_does_not_divide_the_delay() {
        let mut solver = build(BOUNCE);
        let h = 0.3e-9;
        let mut plateaus = Vec::new();
        for step in 1..=320u32 {
            solver.advance(h).unwrap();
            let t_ns = f64::from(step) * 0.3;
            // Arrivals at `b` are at 10, 30, 50, 70 and 90 ns.
            let phase = (t_ns - 10.0).rem_euclid(20.0);
            if t_ns > 10.0 && (5.0..18.0).contains(&phase) {
                plateaus.push(((t_ns - 10.0) / 20.0) as usize);
                let reference = [1.066667, 0.8533333, 0.896, 0.8874667, 0.8891733]
                    [((t_ns - 10.0) / 20.0) as usize];
                assert_close(
                    &format!("v(b) at {t_ns} ns"),
                    voltage(&solver, "b"),
                    reference,
                    1e-6,
                );
            }
        }
        assert_eq!(plateaus.last(), Some(&4), "all five plateaus were checked");
    }

    /// A quarter-wave line is an impedance inverter: 100 Ω seen through
    /// 50 Ω, a quarter wavelength long, is `Z0²/RL` = 25 Ω. Behind a 25 Ω
    /// source the input is therefore matched at HALF the source amplitude,
    /// and all of that power reaches the load, which makes the load
    /// amplitude `sqrt(RL/Zin)` times the input: the full 1 V. A wire would
    /// put 0.8 V on both nodes.
    #[test]
    fn quarter_wave_line_inverts_the_load_impedance() {
        let mut solver = build(
            "* quarter wave transformer\n\
             V1 in 0 SIN(0 1 10meg)\n\
             Rs in a 25\n\
             T1 a 0 b 0 Z0=50 F=10meg\n\
             RL b 0 100\n",
        );
        assert_eq!(solver.circuit().transmission_lines[0].delay, 0.25 / 10e6);
        let h = 0.25e-9;
        // 40 periods to let the start-up reflections (1/9 per round trip)
        // die, then one period of 400 steps to read the amplitudes.
        for _ in 0..40 * 400 {
            solver.advance(h).unwrap();
        }
        let (mut peak_a, mut peak_b) = (0.0f64, 0.0f64);
        for _ in 0..400 {
            solver.advance(h).unwrap();
            peak_a = peak_a.max(voltage(&solver, "a").abs());
            peak_b = peak_b.max(voltage(&solver, "b").abs());
        }
        assert_close("input amplitude", peak_a, 0.5, 1e-4);
        assert_close("load amplitude", peak_b, 1.0, 1e-4);
    }

    /// A caller whose step is LONGER than the delay still gets a stable,
    /// settling line: the delayed wave falls inside the step and is tied to
    /// the step's own end point. With the step at 1000 delays the line is
    /// electrically short and the mismatched circuit sits on the DC divider
    /// of two wires; with the step at 1.6 delays it rings down to it.
    #[test]
    fn a_step_longer_than_the_delay_settles_on_the_dc_answer() {
        for (h, steps, tolerance) in [(10e-6, 3, 1e-2), (16e-9, 60, 1e-6)] {
            let mut solver = build(
                "* long step\n\
                 V1 in 0 dc 0\n\
                 Rs in a 25\n\
                 T1 a 0 b 0 Z0=50 TD=10n\n\
                 RL b 0 200\n",
            );
            let source = solver.voltage_source_index("V1").unwrap();
            solver.set_voltage_source(source, 1.0);
            let mut largest = 0.0f64;
            for _ in 0..steps {
                solver.advance(h).unwrap();
                largest = largest.max(voltage(&solver, "b").abs());
            }
            for node in ["a", "b"] {
                assert_close(
                    &format!("v({node}) with h = {h:e}"),
                    voltage(&solver, node),
                    8.0 / 9.0,
                    tolerance,
                );
            }
            assert!(
                largest < 1.2,
                "h = {h:e}: the far end never exceeds the first overshoot, got {largest}"
            );
        }
    }

    /// A line at rest loses nothing: the solver stops recording a settled
    /// line, and after a long idle stretch at a fixed input a new edge still
    /// takes exactly one delay to arrive.
    #[test]
    fn an_edge_after_a_long_rest_still_arrives_one_delay_later() {
        let mut solver = build(
            "* rest then edge\n\
             V1 in 0 dc 1\n\
             Rs in a 50\n\
             T1 a 0 b 0 Z0=50 TD=10n\n\
             RL b 0 50\n",
        );
        let h = 1e-9;
        for _ in 0..5_000 {
            solver.advance(h).unwrap();
        }
        assert_close("rest", voltage(&solver, "b"), 0.5, 1e-12);
        let source = solver.voltage_source_index("V1").unwrap();
        solver.set_voltage_source(source, 3.0);
        for step in 1..=10 {
            solver.advance(h).unwrap();
            assert_close(
                "near end follows at once",
                voltage(&solver, "a"),
                1.5,
                1e-12,
            );
            assert_close(
                &format!("far end {step} ns after the edge"),
                voltage(&solver, "b"),
                0.5,
                1e-12,
            );
        }
        solver.advance(h).unwrap();
        // 5 us of 1 ns steps leaves the clock a few 1e-22 s off the grid.
        assert_close("far end after one delay", voltage(&solver, "b"), 1.5, 1e-9);
    }

    /// A line inside a `.subckt` is flattened like any other element, and
    /// its instance sees the caller's nodes.
    #[test]
    fn a_line_inside_a_subcircuit_is_flattened() {
        let solver = build(
            "* cable as a subcircuit\n\
             .subckt cable near far\n\
             T1 near 0 mid 0 Z0=50 TD=2n\n\
             T2 mid 0 far 0 Z0=50 TD=3n\n\
             .ends\n\
             V1 in 0 2\n\
             Rs in a 50\n\
             X1 a b cable\n\
             RL b 0 50\n",
        );
        assert_close("v(b)", voltage(&solver, "b"), 1.0, 1e-12);
        let delays: Vec<f64> = solver
            .circuit()
            .transmission_lines
            .iter()
            .map(|line| line.delay)
            .collect();
        assert_eq!(delays, [2e-9, 3e-9]);
        assert_eq!(solver.circuit().branch_count(), 5);
        assert_eq!(solver.circuit().branch_name(1), "T.X1.T1");
        assert_eq!(solver.circuit().branch_name(4), "T.X1.T2");
    }

    #[test]
    fn a_malformed_line_is_refused_with_the_reason() {
        for (line, reason) in [
            ("T1 a 0 b 0 TD=1n", "needs `Z0=<ohms>`"),
            ("T1 a 0 b 0 Z0=50", "needs its delay"),
            ("T1 a 0 b 0 Z0=50 TD=1n F=1meg", "not both"),
            ("T1 a 0 b 0 Z0=50 TD=1n NL=0.5", "`NL=` needs `F=`"),
            ("T1 a 0 b 0 Z0=0 TD=1n", "`z0` must be positive"),
            ("T1 a 0 b 0 Z0=50 TD=-1n", "`td` must be positive"),
            ("T1 a 0 b 0 Z0=50 Z0=75 TD=1n", "`z0` is given twice"),
            ("T1 a 0 b 0 Z0=50 TD=1n IC=0,0,0,0", "`ic=` is not modelled"),
            ("T1 a 0 b 0 Z0=50 TD=1n REL=2", "unknown parameter `rel`"),
            ("T1 a 0 b Z0=50 TD=1n", "expected `T<name>"),
        ] {
            let message = parse_error(&format!("* deck\nV1 a 0 1\nR1 b 0 50\n{line}\n"));
            assert!(message.contains(reason), "`{line}`: {message}");
        }
    }
}
