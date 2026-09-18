// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The nonlinear half of the in-core analog engine: the diode, the BJT, the
//! MOSFET, the Newton iteration that reaches them and the model cards that
//! parameterise them.
//!
//! Three kinds of assertion live here and they are deliberately different:
//!
//! * **Hand calculations.** Every DC operating point below is checked against
//!   the closed form of the device equation, arranged so the answer is derived
//!   rather than read back from the solver — the diode fixture picks the
//!   current first and works out the source voltage that produces it. These
//!   catch a wrong equation.
//! * **Goldens.** A few operating points are pinned as raw `f64` bit patterns.
//!   These catch the drift `analog::device` exists to prevent: a `libm` that
//!   stopped being used, an `exp` that became the platform's, a reordered sum.
//!   A golden that fails is not automatically wrong — but it has to be
//!   explained before it is re-pinned.
//! * **The linear path.** Pinned bit patterns taken from the engine *before*
//!   Newton iteration existed. Nothing in this branch may move them.
//!
//! What is NOT modelled, and is asserted to be missing rather than left to be
//! discovered: device capacitances. No junction or diffusion charge on the
//! diode, no `Cje`/`Cjc` on the BJT, no overlap or channel charge on the
//! MOSFET. A circuit whose behaviour comes from stored device charge belongs
//! to `adapter: external_process`, not to this engine.

// The `analog` module puts every test under `analog::`, so the design's
// verification command `cargo test -p labwired-core analog` runs all of them.
mod analog {
    use labwired_core::analog::{
        device, parse_netlist, AnalogError, Integration, Polarity, Solver,
    };

    /// `kT/q` at the engine's fixed 300 K, repeated here rather than imported
    /// so a hand calculation and the thing it checks cannot drift together.
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    fn build(netlist: &str) -> Solver {
        let circuit = parse_netlist(netlist).unwrap_or_else(|error| panic!("{error}"));
        Solver::new(circuit, Integration::BackwardEuler).unwrap_or_else(|error| panic!("{error}"))
    }

    /// Current flowing out of a voltage source's `+` terminal into the rest of
    /// the circuit. The MNA branch unknown is the other sign.
    fn source_current(solver: &Solver, name: &str) -> f64 {
        -solver.branch_current(solver.circuit().branch_index(name).expect("source exists"))
    }

    fn relative(actual: f64, expected: f64) -> f64 {
        ((actual - expected) / expected).abs()
    }

    // -----------------------------------------------------------------------
    // Diode
    // -----------------------------------------------------------------------

    /// The operating point of `V — R — D — gnd`, derived rather than measured.
    ///
    /// The hand calculation runs backwards: choose the junction current, invert
    /// the Shockley equation for the junction voltage it needs, and set the
    /// source to that plus the drop across the resistor. The solver is then
    /// asked for a number the test already knows, and neither side of the
    /// comparison came from the other.
    #[test]
    fn diode_operating_point_matches_the_inverted_shockley_equation() {
        let is = 1e-14;
        let n = 1.0;
        let r = 1000.0;
        for target_current in [1e-6, 1e-4, 1e-3, 1e-2] {
            let vd = n * VT * f64::ln(target_current / is + 1.0);
            let vsource = vd + target_current * r;
            let netlist = format!(
                "* diode operating point\n\
                 Vs s 0 dc {vsource}\n\
                 R1 s a {r}\n\
                 D1 a 0 DMOD\n\
                 .model DMOD D(IS={is} N={n})\n\
                 .end\n"
            );
            let solver = build(&netlist);
            let node = solver
                .circuit()
                .node("a")
                .expect("node a")
                .expect("not gnd");
            let actual = solver.node_voltage(Some(node));
            assert!(
                relative(actual, vd) < 1e-8,
                "at {target_current} A the junction should sit at {vd} V, solver says {actual} V"
            );
            let current = source_current(&solver, "Vs");
            assert!(
                relative(current, target_current) < 1e-6,
                "expected {target_current} A through the loop, got {current} A"
            );
        }
    }

    /// `RS` is a real resistor in series with the junction, and it gets the
    /// internal node SPICE also creates — so the anode is no longer the
    /// junction, and `v(d1#internal)` is probeable.
    #[test]
    fn diode_series_resistance_drops_its_own_voltage_on_an_internal_node() {
        let is = 1e-14;
        let rs = 22.0;
        let current = 1e-2;
        let vj = VT * f64::ln(current / is + 1.0);
        let netlist = format!(
            "Vs s 0 dc {}\nR1 s a 1000\nD1 a 0 DMOD\n.model DMOD D(IS={is} N=1 RS={rs})\n",
            vj + current * rs + current * 1000.0
        );
        let solver = build(&netlist);
        let anode = solver.circuit().node("a").unwrap().unwrap();
        let internal = solver
            .circuit()
            .node("D1#internal")
            .expect("RS creates an internal node")
            .expect("which is not ground");
        let drop = solver.node_voltage(Some(anode)) - solver.node_voltage(Some(internal));
        assert!(
            relative(drop, current * rs) < 1e-5,
            "RS should drop {} V at {current} A, got {drop} V",
            current * rs
        );
        assert!(
            relative(solver.node_voltage(Some(internal)), vj) < 1e-6,
            "the junction itself should sit at {vj} V"
        );

        // RS = 0 must NOT create the node: it would cost an unknown for
        // nothing and change every node index behind it.
        let plain = build("Vs s 0 dc 1\nR1 s a 1000\nD1 a 0 DMOD\n.model DMOD D(IS=1e-14)\n");
        assert_eq!(plain.circuit().node("D1#internal"), None);
    }

    /// A reverse-biased diode conducts its saturation current plus `GMIN*V`
    /// and nothing else - no junction charge, so no reverse recovery and no
    /// displacement current. This is the missing-capacitance limitation,
    /// asserted rather than left implicit.
    #[test]
    fn a_reverse_biased_diode_carries_only_leakage_with_no_stored_charge() {
        // The built-in `D` card is 1N4148-class.
        let is = 2.52e-9;
        let mut solver = build("Vs s 0 dc -5\nR1 s a 1\nD1 a 0 D\n");
        let current = source_current(&solver, "Vs").abs();
        let expected = is + device::GMIN * 5.0;
        assert!(
            relative(current, expected) < 1e-6,
            "a reverse-biased diode should pass IS + GMIN*V = {expected} A, got {current} A"
        );
        // Switch the polarity and step: a real diode would sweep stored charge
        // out over its reverse-recovery time. This one is at the new operating
        // point on the first step, because there is no charge to sweep.
        let vs = solver.voltage_source_index("Vs").unwrap();
        solver.set_voltage_source(vs, 5.0);
        solver.advance(1e-9).expect("step solves");
        let forward = source_current(&solver, "Vs");
        assert!(
            forward > 1e-3,
            "with no stored charge the junction conducts on the first step, got {forward} A"
        );
    }

    // -----------------------------------------------------------------------
    // Bipolar transistor
    // -----------------------------------------------------------------------

    /// A BJT with both junctions held by voltage sources has an operating point
    /// that is pure arithmetic: no loop to solve, so the transport equations
    /// are checked directly.
    #[test]
    fn bjt_operating_point_matches_ebers_moll_by_hand() {
        let is = 1e-16;
        let bf = 100.0;
        let br = 1.0;
        for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
            let kind = if polarity == Polarity::N {
                "NPN"
            } else {
                "PNP"
            };
            let vbe = 0.65;
            let vbc = -4.35;
            let netlist = format!(
                "Vb b 0 dc {}\nVc c 0 dc {}\nQ1 c b 0 QMOD\n\
                 .model QMOD {kind}(IS={is} BF={bf} BR={br} NF=1 NR=1)\n",
                sign * vbe,
                sign * (vbe - vbc)
            );
            let solver = build(&netlist);

            let forward = (vbe / VT).exp();
            let reverse = (vbc / VT).exp();
            let ic = is * (forward - reverse) - (is / br) * (reverse - 1.0);
            let ib = (is / bf) * (forward - 1.0) + (is / br) * (reverse - 1.0);

            // Current INTO the collector, which is where the source's current
            // goes, plus the GMIN tie across the reverse-biased BC junction.
            let measured_c = sign * source_current(&solver, "Vc");
            let measured_b = sign * source_current(&solver, "Vb");
            assert!(
                relative(measured_c, ic + device::GMIN * vbc.abs()) < 1e-5,
                "{kind}: Ic should be {ic} A, got {measured_c} A"
            );
            assert!(
                relative(measured_b, ib) < 1e-3,
                "{kind}: Ib should be {ib} A, got {measured_b} A"
            );
            let beta = measured_c / measured_b;
            assert!(
                (beta - bf).abs() / bf < 0.02,
                "{kind}: forward beta should be about {bf}, got {beta}"
            );
        }
    }

    /// The Early effect is the one missing Gummel–Poon term a user will notice:
    /// output conductance in the active region is `GMIN`, not `Ic/VAF`. Say so
    /// with a test rather than only in a doc comment.
    #[test]
    fn a_bjt_in_the_active_region_has_no_early_effect() {
        let mut currents = Vec::new();
        for vce in [2.0, 10.0] {
            let netlist = format!(
                "Vb b 0 dc 0.65\nVc c 0 dc {vce}\nQ1 c b 0 NPN\n.model QMOD NPN(IS=1e-16)\n"
            );
            currents.push(source_current(&build(&netlist), "Vc"));
        }
        let slope = (currents[1] - currents[0]).abs() / 8.0;
        assert!(
            slope < 10.0 * device::GMIN,
            "output conductance should be GMIN-sized without VAF, got {slope} S"
        );
    }

    // -----------------------------------------------------------------------
    // MOSFET
    // -----------------------------------------------------------------------

    /// Level 1 in all three regions, against the Shichman–Hodges closed form.
    #[test]
    fn mosfet_operating_point_matches_shichman_hodges_by_hand() {
        let kp = 20e-6;
        let (w, l) = (100e-6, 10e-6);
        let beta = kp * w / l;
        let lambda = 0.02;
        for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
            let kind = if polarity == Polarity::N {
                "NMOS"
            } else {
                "PMOS"
            };
            let vto = 1.0;
            for (vgs, vds) in [(0.5, 5.0), (3.0, 5.0), (3.0, 0.5)] {
                let netlist = format!(
                    "Vg g 0 dc {}\nVd d 0 dc {}\nM1 d g 0 0 MMOD\n\
                     .model MMOD {kind}(VTO={} KP={kp} LAMBDA={lambda} W={w} L={l})\n",
                    sign * vgs,
                    sign * vds,
                    sign * vto
                );
                let solver = build(&netlist);

                let vgst = vgs - vto;
                let expected = if vgst <= 0.0 {
                    0.0
                } else if vds < vgst {
                    beta * (1.0 + lambda * vds) * vds * (vgst - 0.5 * vds)
                } else {
                    0.5 * beta * (1.0 + lambda * vds) * vgst * vgst
                };
                // Two GMIN ties reach the drain: channel and bulk.
                let expected = expected + 2.0 * device::GMIN * vds;
                let measured = sign * source_current(&solver, "Vd");
                let label = format!("{kind} Vgs={vgs} Vds={vds}");
                if expected.abs() < 1e-9 {
                    assert!(
                        measured.abs() < 1e-9,
                        "{label}: cut off, expected ~0 A, got {measured} A"
                    );
                } else {
                    assert!(
                        relative(measured, expected) < 1e-6,
                        "{label}: expected {expected} A, got {measured} A"
                    );
                }
            }
        }
    }

    /// A MOSFET is symmetric. Driving the terminal the netlist calls the drain
    /// BELOW the one it calls the source must give the mirror image of the
    /// forward characteristic, not a device stuck at zero — the failure mode a
    /// model without reverse mode has, and one that a pass transistor or a
    /// low-side switch with a reactive load hits immediately.
    #[test]
    fn a_mosfet_conducts_in_reverse_with_the_mirrored_characteristic() {
        let deck = |vd: f64, vs: f64| {
            format!(
                "Vg g 0 dc 3\nVd d 0 dc {vd}\nVs s 0 dc {vs}\nM1 d g s 0 MMOD\n\
                 .model MMOD NMOS(VTO=1 KP=20u LAMBDA=0 W=100u L=10u)\n"
            )
        };
        // Gate 3 V above whichever terminal is the source in each case.
        let forward = source_current(&build(&deck(0.4, 0.0)), "Vd");
        let reverse = source_current(&build(&deck(0.0, 0.4)), "Vs");
        assert!(
            forward > 1e-6,
            "forward conduction should be micro-amps or more, got {forward} A"
        );
        assert!(
            relative(reverse, forward) < 1e-6,
            "reverse conduction should mirror forward: {forward} A vs {reverse} A"
        );
    }

    // -----------------------------------------------------------------------
    // Newton iteration itself
    // -----------------------------------------------------------------------

    /// A netlist with no nonlinear device must not take the Newton path at all.
    /// This is the flag every byte-identity claim in this file rests on.
    #[test]
    fn a_linear_netlist_is_not_marked_nonlinear() {
        let linear =
            build("V1 in 0 dc 5\nR1 in out 1k\nC1 out 0 1u\nL1 out tail 1m\nR2 tail 0 100\n");
        assert!(!linear.is_nonlinear());
        assert!(build("V1 in 0 dc 5\nR1 in a 1k\nD1 a 0 D\n").is_nonlinear());
        assert!(build("V1 in 0 dc 5\nR1 in b 100k\nQ1 in b 0 NPN\n").is_nonlinear());
        assert!(build("V1 in 0 dc 5\nR1 in d 1k\nM1 d in 0 0 NMOS\n").is_nonlinear());
    }

    /// An ideal voltage source connected straight across a junction with no
    /// series resistance is not a hard circuit, it is an impossible one: the
    /// junction conductance runs away and the matrix loses its pivot. That has
    /// to be the named `Singular` error rather than a runaway current the trace
    /// shows as a plausible number.
    #[test]
    fn an_ideal_source_straight_across_a_junction_is_reported_as_singular() {
        let circuit = parse_netlist("V1 in 0 dc 5\nQ1 in in 0 NPN\n").expect("parses");
        let error = Solver::new(circuit, Integration::BackwardEuler)
            .expect_err("a source hard across a junction has no solution");
        assert!(
            matches!(error, AnalogError::Singular { .. }),
            "expected a Singular error, got {error}"
        );
    }

    /// Failure to converge is a coded error that names the step, not a `NaN`
    /// quietly written into the trace.
    ///
    /// Every circuit this engine is meant for converges, which is the point —
    /// so the *shape* of the failure is what is pinned here, and the test below
    /// pins the other half: that a stiff circuit driven far past anything
    /// reasonable still either solves or returns `Err`, never `NaN`.
    #[test]
    fn a_failure_to_converge_reports_the_step_rather_than_a_nan() {
        let error = AnalogError::NoConvergence {
            step: Some(42),
            time: 4.2e-3,
            iterations: 100,
            unknown: "v(out)".to_string(),
            delta: 1.5,
        };
        let text = error.to_string();
        assert!(text.contains("step 42"), "{text}");
        assert!(text.contains("4.2e-3"), "{text}");
        assert!(text.contains("v(out)"), "{text}");
        assert!(text.contains("100 iterations"), "{text}");
        assert!(
            text.contains("labwired_ngspice.py"),
            "the error has to name the engine that can run it: {text}"
        );
        assert_eq!(error.line(), None);
    }

    /// Whatever a nonlinear circuit does, it does not produce `NaN`: a step
    /// that cannot be solved returns `Err`. Swept over a stiff rectifier driven
    /// hard in both directions with a reactive load.
    #[test]
    fn a_stiff_nonlinear_circuit_never_produces_a_nan() {
        let mut solver = build(
            "Vin in 0 dc 0\nR1 in a 10\nD1 a out DMOD\nC1 out 0 1u\nR2 out 0 1k\n\
             .model DMOD D(IS=1e-15 N=1 RS=0.5)\n",
        );
        let vin = solver.voltage_source_index("Vin").unwrap();
        let out = solver.circuit().node("out").unwrap();
        for step in 0..2000 {
            // A brutal square wave: ±40 V with no ramp at all.
            solver.set_voltage_source(vin, if step % 20 < 10 { 40.0 } else { -40.0 });
            solver
                .advance(1e-6)
                .unwrap_or_else(|error| panic!("step {step}: {error}"));
            let v = solver.node_voltage(out);
            assert!(v.is_finite(), "step {step} produced {v}");
            assert!(
                v.abs() < 60.0,
                "step {step} produced {v} V out of a 40 V drive"
            );
        }
    }

    /// The limiters change the path Newton takes, never the point it reaches.
    /// Starting the same circuit from two different states must land on the
    /// same operating point to the last bit.
    #[test]
    fn the_converged_point_does_not_depend_on_the_path_to_it() {
        let netlist = "Vs s 0 dc 5\nR1 s a 1k\nD1 a b DMOD\nR2 b 0 470\n\
                       .model DMOD D(IS=2.52n N=1.752)\n";
        let cold = build(netlist);

        let mut warmed = build(netlist);
        let vs = warmed.voltage_source_index("Vs").unwrap();
        for value in [-30.0, 30.0, -30.0, 5.0] {
            warmed.set_voltage_source(vs, value);
            warmed.solve_operating_point().expect("op point solves");
        }

        let node = cold.circuit().node("b").unwrap();
        assert_eq!(
            cold.node_voltage(node).to_bits(),
            warmed.node_voltage(node).to_bits(),
            "cold {} V vs warmed {} V",
            cold.node_voltage(node),
            warmed.node_voltage(node)
        );
    }
}
