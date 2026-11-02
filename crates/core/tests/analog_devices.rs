// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The nonlinear half of the in-core analog engine: the diode, the BJT, the
//! JFET, the MOSFET, the Newton iteration that reaches them and the model
//! cards that parameterise them.
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
//! discovered: diode diffusion charge (`TT`) and MOSFET channel charge. BJT
//! constant ideal transit-time charge (`TF`/`TR`) is exercised below. Diode
//! depletion charge through `CJO`/`VJ`/`M`/`FC`, BJT base-junction depletion
//! charge through `CJE`/`CJC`, JFET gate-junction depletion charge through
//! `CGS`/`CGD`, and MOSFET linear overlap charge through `CGSO`/`CGDO`/`CGBO`
//! are exercised below. Circuits
//! depending on unsupported stored charge still belong to
//! `adapter: external_process`.

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

    #[test]
    fn diode_depletion_charge_and_capacitance_match_the_spice_integral() {
        let cjo: f64 = 4e-12;
        let vj = 0.75;
        let m = 0.333;
        let fc = 0.4;
        let voltage = 0.25;
        let x = 1.0 - voltage / vj;
        let expected_q = cjo * vj / (1.0 - m) * (1.0 - x.powf(1.0 - m));
        let expected_c = cjo * x.powf(-m);
        let (charge, capacitance) = device::diode_depletion_charge(voltage, cjo, vj, m, fc);
        assert!((charge - expected_q).abs() < 1e-26);
        assert!((capacitance - expected_c).abs() < 1e-25);

        let transition = fc * vj;
        let (q_before, c_before) =
            device::diode_depletion_charge(transition - 1e-10, cjo, vj, m, fc);
        let (q_after, c_after) = device::diode_depletion_charge(transition + 1e-10, cjo, vj, m, fc);
        assert!(
            (q_before - q_after).abs() < 3e-21,
            "charge is continuous at FC: {q_before} vs {q_after}"
        );
        assert!(
            (c_before - c_after).abs() < 1e-20,
            "capacitance is continuous at FC: {c_before} vs {c_after}"
        );

        let (q_m1, c_m1) = device::diode_depletion_charge(voltage, cjo, vj, 1.0, fc);
        assert!((q_m1 + cjo * vj * x.ln()).abs() < 1e-26);
        assert!((c_m1 - cjo / x).abs() < 1e-25);
    }

    #[test]
    fn diode_cjo_integrates_in_backward_euler_and_trapezoidal_steps() {
        let cjo = 10e-9;
        let resistance = 1e3;
        let h = 1e-6;
        let ratio = resistance * cjo / h;
        let netlist = format!(
            "V1 in 0 dc 0\nR1 in out {resistance}\nD1 out 0 DMOD\n\
             .model DMOD D(IS=1e-30 CJO={cjo} VJ=1 M=0)\n"
        );

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            let circuit = parse_netlist(&netlist).expect("diode capacitor circuit parses");
            let mut solver = Solver::new(circuit, integration).expect("solver builds");
            let out = solver.circuit().node("out").expect("out node");
            let source = solver.voltage_source_index("V1").expect("source exists");
            solver.set_voltage_source(source, 1.0);
            solver.advance(h).expect("first transient step solves");
            let v1 = solver.node_voltage(out);
            let expected_v1 = 1.0 / (1.0 + ratio);
            assert!(
                (v1 - expected_v1).abs() < 1e-8,
                "first BE restart step: {v1}"
            );

            solver.advance(h).expect("second transient step solves");
            let expected_v2 = match integration {
                Integration::BackwardEuler => (1.0 + ratio * expected_v1) / (1.0 + ratio),
                Integration::Trapezoidal => (1.0 + 3.0 * ratio * expected_v1) / (1.0 + 2.0 * ratio),
            };
            let v2 = solver.node_voltage(out);
            assert!(
                (v2 - expected_v2).abs() < 1e-8,
                "{integration:?} second step: expected {expected_v2} V, got {v2} V"
            );
        }
    }

    #[test]
    fn bjt_cje_integrates_as_base_emitter_charge_for_both_polarities() {
        let cje = 10e-9;
        let resistance = 1e3;
        let h = 1e-6;
        let ratio = resistance * cje / h;

        for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
            let netlist = format!(
                "V1 in 0 dc 0\nR1 in b {resistance}\nVc c 0 dc 0\n\
                 Q1 c b 0 QMOD\n.model QMOD {kind}(IS=1e-30 CJE={cje} VJE=1 MJE=0 FC=0.5)\n"
            );
            let circuit = parse_netlist(&netlist).expect("BJT junction-cap circuit parses");
            assert_eq!(circuit.bjts[0].model.cje, cje);
            assert_eq!(circuit.bjts[0].model.vje, 1.0);
            assert_eq!(circuit.bjts[0].model.mje, 0.0);
            assert_eq!(circuit.bjts[0].model.cjc, 0.0);

            for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                let mut solver = Solver::new(circuit.clone(), integration).expect("solver builds");
                let base = solver.circuit().node("b").expect("base node");
                let source = solver.voltage_source_index("V1").expect("source exists");
                solver.set_voltage_source(source, sign);
                solver.advance(h).expect("first transient step solves");
                let expected = sign / (1.0 + ratio);
                let actual = solver.node_voltage(base);
                assert!(
                    (actual - expected).abs() < 1e-8,
                    "{kind} {integration:?} base-emitter step: expected {expected} V, got {actual} V"
                );

                solver.advance(h).expect("second transient step solves");
                let expected_second = match integration {
                    Integration::BackwardEuler => {
                        sign * (1.0 + ratio * expected.abs()) / (1.0 + ratio)
                    }
                    Integration::Trapezoidal => {
                        sign * (1.0 + 3.0 * ratio * expected.abs()) / (1.0 + 2.0 * ratio)
                    }
                };
                let actual_second = solver.node_voltage(base);
                assert!(
                    (actual_second - expected_second).abs() < 1e-8,
                    "{kind} {integration:?} second base-emitter step: expected {expected_second} V, got {actual_second} V"
                );
            }
        }
    }

    #[test]
    fn bjt_mje_changes_the_transient_base_emitter_charge() {
        let cje = 10e-9;
        let vje = 0.75;
        let mje = 0.4;
        let fc = 0.5;
        let resistance = 1e3;
        let h = 1e-6;
        let charge =
            |voltage: f64| cje * vje / (1.0 - mje) * (1.0 - (1.0 - voltage / vje).powf(1.0 - mje));
        let mut lo = 0.0;
        let mut hi = fc * vje;
        for _ in 0..80 {
            let mid = 0.5 * (lo + hi);
            if (1.0 - mid) / resistance > charge(mid) / h {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let expected = 0.5 * (lo + hi);

        let mut solver = build(&format!(
            "V1 in 0 dc 0\nR1 in b {resistance}\nVc c 0 dc 0\nQ1 c b 0 QMOD\n\
             .model QMOD NPN(IS=1e-30 CJE={cje} VJE={vje} MJE={mje} FC={fc})\n"
        ));
        let base = solver.circuit().node("b").expect("base node");
        let source = solver.voltage_source_index("V1").expect("source exists");
        solver.set_voltage_source(source, 1.0);
        solver.advance(h).expect("graded B-E depletion step solves");
        let actual = solver.node_voltage(base);
        assert!(
            (actual - expected).abs() < 1e-8,
            "MJE={mje} charge curve should settle at {expected} V, got {actual} V"
        );
    }

    #[test]
    fn bjt_cjc_couples_a_base_step_into_the_collector() {
        let cjc = 10e-9;
        let resistance = 1e3;
        let h = 1e-6;
        let vjc = 0.75;
        let mjc = 0.4;
        let fc = 0.5;
        let charge =
            |voltage: f64| cjc * vjc / (1.0 - mjc) * (1.0 - (1.0 - voltage / vjc).powf(1.0 - mjc));
        let mut lo = 0.0;
        let mut hi = 1.0;
        for _ in 0..80 {
            let mid = 0.5 * (lo + hi);
            if mid / resistance < charge(1.0 - mid) / h {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let expected_magnitude = 0.5 * (lo + hi);

        for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
            let netlist = format!(
                "V1 b 0 dc 0\nR1 c 0 {resistance}\nQ1 c b 0 QMOD\n\
                 .model QMOD {kind}(IS=1e-30 CJC={cjc} VJC={vjc} MJC={mjc} FC={fc})\n"
            );
            let circuit = parse_netlist(&netlist).expect("BJT junction-cap circuit parses");
            assert_eq!(circuit.bjts[0].model.cje, 0.0);
            assert_eq!(circuit.bjts[0].model.cjc, cjc);
            assert_eq!(circuit.bjts[0].model.vjc, vjc);
            assert_eq!(circuit.bjts[0].model.mjc, mjc);

            let mut solver =
                Solver::new(circuit, Integration::BackwardEuler).expect("solver builds");
            let collector = solver.circuit().node("c").expect("collector node");
            let source = solver.voltage_source_index("V1").expect("source exists");
            solver.set_voltage_source(source, sign);
            solver.advance(h).expect("base step couples through CJC");
            let expected = sign * expected_magnitude;
            let actual = solver.node_voltage(collector);
            assert!(
                (actual - expected).abs() < 1e-8,
                "{kind} collector coupling: expected {expected} V, got {actual} V"
            );
        }
    }

    #[test]
    fn bjt_transit_charge_matches_transport_equations_and_partials() {
        let circuit = parse_netlist(
            "Q1 c b e QMOD\n\
             .model QMOD NPN(IS=2e-16 NF=1.2 NR=1.1 VAF=40 VAR=70 IKF=2m IKR=5m TF=12n TR=35n)\n",
        )
        .expect("transit-time model parses");
        let model = circuit.bjts[0].model;
        let (vbe, vbc) = (0.64, 0.35);
        let charge = device::bjt_transit_charge(vbe, vbc, model);
        let vtf = model.nf * VT;
        let vtr = model.nr * VT;
        let cbe = model.is * ((vbe / vtf).exp() - 1.0);
        let cbc = model.is * ((vbc / vtr).exp() - 1.0);
        let qb = (1.0 - vbc / model.vaf - vbe / model.var).recip()
            * (1.0 + (1.0 + 4.0 * (cbe / model.ikf + cbc / model.ikr)).sqrt())
            / 2.0;
        assert!((charge.qbe - model.tf * cbe / qb).abs() < 1e-25);
        assert!((charge.qbc - model.tr * cbc).abs() < 1e-25);

        let delta = 1e-5;
        let plus_be = device::bjt_transit_charge(vbe + delta, vbc, model);
        let minus_be = device::bjt_transit_charge(vbe - delta, vbc, model);
        let plus_bc = device::bjt_transit_charge(vbe, vbc + delta, model);
        let minus_bc = device::bjt_transit_charge(vbe, vbc - delta, model);
        let finite_difference = |plus: f64, minus: f64| (plus - minus) / (2.0 * delta);
        for (actual, expected) in [
            (
                charge.dqbe_dvbe,
                finite_difference(plus_be.qbe, minus_be.qbe),
            ),
            (
                charge.dqbe_dvbc,
                finite_difference(plus_bc.qbe, minus_bc.qbe),
            ),
            (
                charge.dqbc_dvbc,
                finite_difference(plus_bc.qbc, minus_bc.qbc),
            ),
        ] {
            assert!(
                (actual - expected).abs() < expected.abs() * 2e-5 + 1e-24,
                "analytic charge derivative {actual} differs from finite difference {expected}"
            );
        }
        assert_eq!(charge.dqbc_dvbe, 0.0);

        let reverse_be = device::bjt_transit_charge(-0.1, vbc, model);
        let expected_reverse_be = model.tf * model.is * ((-0.1 / vtf).exp() - 1.0);
        assert!((reverse_be.qbe - expected_reverse_be).abs() < 1e-25);
    }

    #[test]
    fn bjt_tf_and_tr_charge_integrate_for_both_polarities_and_rules() {
        let is = 1e-16;
        let beta_f = 100.0;
        let beta_r = 2.0;
        let resistance = 1e3;
        let h = 1e-9;

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
                for forward in [true, false] {
                    let tf = if forward { 10e-9 } else { 0.0 };
                    let tr = if forward { 0.0 } else { 10e-9 };
                    let (collector_v, emitter_v) = if forward {
                        (sign * 2.0, 0.0)
                    } else {
                        (0.0, sign * 2.0)
                    };
                    let netlist = format!(
                        "Vdrive drive 0 dc 0\nRdrive drive b {resistance}\n\
                         Vc c 0 dc {collector_v}\nVe e 0 dc {emitter_v}\nQ1 c b e QMOD\n\
                         .model QMOD {kind}(IS={is} BF={beta_f} BR={beta_r} TF={tf} TR={tr})\n"
                    );
                    let circuit = parse_netlist(&netlist).expect("BJT transit circuit parses");
                    let mut solver =
                        Solver::new(circuit, integration).expect("BJT transit solver builds");
                    let base = solver.circuit().node("b").expect("base node");
                    let source = solver
                        .voltage_source_index("Vdrive")
                        .expect("base drive exists");
                    solver.set_voltage_source(source, sign);

                    let charge = |v: f64| {
                        let voltage = v / VT;
                        if forward {
                            tf * is * (voltage.exp() - 1.0)
                        } else {
                            tr * is * (voltage.exp() - 1.0)
                        }
                    };
                    let junctions = |v: f64| {
                        if forward {
                            (v, v - 2.0)
                        } else {
                            (v - 2.0, v)
                        }
                    };
                    let base_current = |vbe: f64, vbc: f64| {
                        is / beta_f * ((vbe / VT).exp() - 1.0)
                            + is / beta_r * ((vbc / VT).exp() - 1.0)
                    };
                    let initial_v = sign * solver.node_voltage(base);
                    let mut previous_charge = charge(initial_v);
                    let mut previous_current = 0.0;
                    for step in 0..2 {
                        let charge_current = |v: f64| {
                            let q = charge(v);
                            if step == 0 || integration == Integration::BackwardEuler {
                                (q - previous_charge) / h
                            } else {
                                2.0 * (q - previous_charge) / h - previous_current
                            }
                        };
                        let residual = |v: f64| {
                            let (vbe, vbc) = junctions(v);
                            (1.0 - v) / resistance
                                - base_current(vbe, vbc)
                                - device::GMIN * (vbe + vbc)
                                - charge_current(v)
                        };
                        let mut lo = 0.0;
                        let mut hi = 1.0;
                        for _ in 0..100 {
                            let mid = 0.5 * (lo + hi);
                            if residual(mid) > 0.0 {
                                lo = mid;
                            } else {
                                hi = mid;
                            }
                        }
                        let expected = 0.5 * (lo + hi);
                        solver.advance(h).expect("transit charge step solves");
                        let actual = sign * solver.node_voltage(base);
                        assert!(
                            (actual - expected).abs() < 1e-8,
                            "{kind} {integration:?} {} step {step}: expected {expected} V, got {actual} V",
                            if forward { "TF" } else { "TR" }
                        );
                        let next_charge = charge(actual);
                        previous_current = (next_charge - previous_charge) / h;
                        previous_charge = next_charge;
                    }
                }
            }
        }
    }

    /// A reverse-biased diode with default `CJO=0` conducts its saturation
    /// current plus `GMIN*V` and has no diffusion charge. This leaves reverse
    /// recovery outside the in-core model.
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

    /// With the emitter and collector junction roles reversed, the old
    /// emitter carries the reverse transport current and its ratio to base
    /// current is `BR`. Check all three terminal currents against the
    /// Ebers–Moll equations for both polarities; forward-only checks would
    /// miss a swapped BC/BE stamp or a lost reverse gain.
    #[test]
    fn bjt_reverse_active_gain_matches_ebers_moll_by_hand() {
        let is = 1e-16;
        let bf = 80.0;
        let br = 4.0;
        let vbe = -4.35;
        let vbc = 0.65;
        let exp_be = (vbe / VT).exp();
        let exp_bc = (vbc / VT).exp();
        let junction_be = exp_be - 1.0;
        let junction_bc = exp_bc - 1.0;
        let ic = is * (exp_be - exp_bc) - (is / br) * junction_bc - device::GMIN * vbc;
        let ib = (is / bf) * junction_be + (is / br) * junction_bc + device::GMIN * (vbe + vbc);
        // The solver ties both base junctions with GMIN. In this reverse
        // active point, Vb−Ve = Vbe and Vb−Vc = Vbc.
        let ie = -(ic + ib);

        for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
            let kind = if polarity == Polarity::N {
                "NPN"
            } else {
                "PNP"
            };
            let netlist = format!(
                "Vb b 0 dc {}\nVc c 0 dc 0\nVe e 0 dc {}\nQ1 c b e QMOD\n\
                 .model QMOD {kind}(IS={is} BF={bf} BR={br} NF=1 NR=1)\n",
                sign * vbc,
                sign * (vbc - vbe)
            );
            let solver = build(&netlist);
            let actual_ic = sign * source_current(&solver, "Vc");
            let actual_ib = sign * source_current(&solver, "Vb");
            let actual_ie = sign * source_current(&solver, "Ve");

            assert!(
                relative(actual_ic, ic) < 1e-5,
                "{kind}: reverse collector terminal current should be {ic} A, got {actual_ic} A"
            );
            assert!(
                relative(actual_ib, ib) < 1e-5,
                "{kind}: reverse base current should be {ib} A, got {actual_ib} A"
            );
            assert!(
                relative(actual_ie, ie) < 1e-5,
                "{kind}: reverse emitter transport current should be {ie} A, got {actual_ie} A"
            );
            let reverse_gain = actual_ie / actual_ib;
            assert!(
                (reverse_gain - br).abs() / br < 1e-5,
                "{kind}: reverse common-emitter gain should be about {br}, got {reverse_gain}"
            );
        }
    }

    /// When both base junctions are forward biased, the Ebers–Moll transport
    /// currents oppose each other. Check all terminals against the equations
    /// for NPN and PNP; active-region tests do not exercise the BC injection
    /// term that dominates a saturated transistor.
    #[test]
    fn bjt_saturation_matches_ebers_moll_by_hand() {
        let is = 1e-16;
        let bf = 80.0;
        let br = 4.0;
        let vbe: f64 = 0.65;
        let vbc: f64 = 0.55;
        let exp_be = (vbe / VT).exp();
        let exp_bc = (vbc / VT).exp();
        let junction_be = exp_be - 1.0;
        let junction_bc = exp_bc - 1.0;
        let ic = is * (exp_be - exp_bc) - (is / br) * junction_bc - device::GMIN * vbc;
        let ib = (is / bf) * junction_be + (is / br) * junction_bc + device::GMIN * (vbe + vbc);
        let ie = -(ic + ib);

        for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
            let kind = if polarity == Polarity::N {
                "NPN"
            } else {
                "PNP"
            };
            let netlist = format!(
                "Vb b 0 dc {}\nVc c 0 dc {}\nVe e 0 dc 0\nQ1 c b e QMOD\n\
                 .model QMOD {kind}(IS={is} BF={bf} BR={br} NF=1 NR=1)\n",
                sign * vbe,
                sign * (vbe - vbc)
            );
            let solver = build(&netlist);
            let actual_ic = sign * source_current(&solver, "Vc");
            let actual_ib = sign * source_current(&solver, "Vb");
            let actual_ie = sign * source_current(&solver, "Ve");

            assert!(
                relative(actual_ic, ic) < 1e-5,
                "{kind}: saturated collector current should be {ic} A, got {actual_ic} A"
            );
            assert!(
                relative(actual_ib, ib) < 1e-5,
                "{kind}: saturated base current should be {ib} A, got {actual_ib} A"
            );
            assert!(
                relative(actual_ie, ie) < 1e-5,
                "{kind}: saturated emitter current should be {ie} A, got {actual_ie} A"
            );
        }
    }

    /// With high-injection terms disabled, Gummel–Poon's base-width factor is
    /// `1 - Vbc/VAF - Vbe/VAR`. Check all terminal currents against that
    /// equation in both operating directions and polarities.
    #[test]
    fn bjt_early_effect_matches_gummel_poon_by_hand() {
        let (is, bf, br, vaf, var) = (1e-16, 80.0, 4.0, 35.0, 50.0);
        for (vbe, vbc) in [(0.65, -4.35), (-4.35, 0.65)] {
            let exp_be = (vbe / VT).exp();
            let exp_bc = (vbc / VT).exp();
            let base_width = 1.0 - vbc / vaf - vbe / var;
            let ict = is * (exp_be - exp_bc) * base_width;
            let ibe = (is / bf) * (exp_be - 1.0);
            let ibc = (is / br) * (exp_bc - 1.0);
            let ic = ict - ibc - device::GMIN * vbc;
            let ib = ibe + ibc + device::GMIN * (vbe + vbc);
            let ie = -(ic + ib);

            for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
                let kind = if polarity == Polarity::N {
                    "NPN"
                } else {
                    "PNP"
                };
                let netlist = format!(
                    "Vb b 0 dc {}\nVc c 0 dc 0\nVe e 0 dc {}\nQ1 c b e QMOD\n\
                     .model QMOD {kind}(IS={is} BF={bf} BR={br} VAF={vaf} VAR={var})\n",
                    sign * vbc,
                    sign * (vbc - vbe),
                );
                let solver = build(&netlist);
                let actual_ic = sign * source_current(&solver, "Vc");
                let actual_ib = sign * source_current(&solver, "Vb");
                let actual_ie = sign * source_current(&solver, "Ve");
                let direction = if vbe > 0.0 { "forward" } else { "reverse" };

                assert!(
                    relative(actual_ic, ic) < 1e-5,
                    "{kind} {direction}: collector current should be {ic} A, got {actual_ic} A"
                );
                assert!(
                    relative(actual_ib, ib) < 1e-5,
                    "{kind} {direction}: base current should be {ib} A, got {actual_ib} A"
                );
                assert!(
                    relative(actual_ie, ie) < 1e-5,
                    "{kind} {direction}: emitter current should be {ie} A, got {actual_ie} A"
                );
            }
        }
    }

    #[test]
    fn bjt_early_effect_solves_a_resistively_loaded_collector() {
        let (vcc, rb, vbe, is, br, vaf) = (5.0, 1000.0, 0.65, 1e-16, 4.0, 50.0);
        let current_balance = |vc: f64| {
            let vbc = vbe - vc;
            let forward = (vbe / VT).exp();
            let reverse = (vbc / VT).exp();
            let ict = is * (forward - reverse) * (1.0 - vbc / vaf);
            let ibc = (is / br) * (reverse - 1.0);
            let collector_current = ict - ibc;
            (vcc - vc) / rb - collector_current - device::GMIN * (vc - vbe)
        };
        let mut lo = 0.0;
        let mut hi = vcc;
        for _ in 0..100 {
            let mid = (lo + hi) / 2.0;
            if current_balance(mid) > 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let expected_vc = (lo + hi) / 2.0;
        let solver = build(
            "Vcc supply 0 dc 5\nR1 supply c 1k\nVb b 0 dc 0.65\n\
             Q1 c b 0 QMOD\n.model QMOD NPN(IS=1e-16 BF=80 BR=4 VAF=50)\n",
        );
        let collector = solver.circuit().node("c").unwrap();
        let actual_vc = solver.node_voltage(collector);
        assert!(
            (actual_vc - expected_vc).abs() < 1e-8,
            "loaded collector should settle at {expected_vc} V with Early effect, got {actual_vc} V"
        );
    }

    #[test]
    fn bjt_early_effect_partials_match_transport_and_junction_derivatives() {
        let (is, bf, br, nf, nr, vaf, var) = (2e-15, 75.0, 3.0, 1.1, 1.2, 42.0, 65.0);
        let delta = 1e-7;
        for (vbe, vbc) in [(0.62, -0.1), (-0.1, 0.58)] {
            let currents = |vbe: f64, vbc: f64| {
                let forward = (vbe / (nf * VT)).exp();
                let reverse = (vbc / (nr * VT)).exp();
                let qinv = 1.0 - vbc / vaf - vbe / var;
                (
                    is * (forward - reverse) * qinv,
                    (is / bf) * (forward - 1.0),
                    (is / br) * (reverse - 1.0),
                )
            };
            let op = device::bjt_op(
                vbe,
                vbc,
                is,
                bf,
                br,
                nf,
                nr,
                vaf,
                var,
                f64::INFINITY,
                f64::INFINITY,
                0.0,
                1.5,
                0.0,
                2.0,
            );
            let (ict_plus, ibe_plus, _) = currents(vbe + delta, vbc);
            let (ict_minus, ibe_minus, _) = currents(vbe - delta, vbc);
            let expected_gif = (ict_plus - ict_minus) / (2.0 * delta);
            let expected_gpi = (ibe_plus - ibe_minus) / (2.0 * delta);
            let (ict_plus, _, ibc_plus) = currents(vbe, vbc + delta);
            let (ict_minus, _, ibc_minus) = currents(vbe, vbc - delta);
            let expected_gir = -(ict_plus - ict_minus) / (2.0 * delta);
            let expected_gmu = (ibc_plus - ibc_minus) / (2.0 * delta);

            assert!(
                relative(op.gif, expected_gif) < 1e-6,
                "gif: {} vs {expected_gif}",
                op.gif
            );
            assert!(
                relative(op.gir, expected_gir) < 1e-6,
                "gir: {} vs {expected_gir}",
                op.gir
            );
            assert!(
                relative(op.gpi, expected_gpi) < 1e-6,
                "gpi: {} vs {expected_gpi}",
                op.gpi
            );
            assert!(
                relative(op.gmu, expected_gmu) < 1e-6,
                "gmu: {} vs {expected_gmu}",
                op.gmu
            );
        }
    }

    #[test]
    fn zero_and_infinite_bjt_early_voltages_disable_base_width_modulation() {
        let parameters = (0.65, -4.35, 1e-16, 100.0, 1.0, 1.0, 1.0);
        let disabled_by_zero = device::bjt_op(
            parameters.0,
            parameters.1,
            parameters.2,
            parameters.3,
            parameters.4,
            parameters.5,
            parameters.6,
            0.0,
            0.0,
            f64::INFINITY,
            f64::INFINITY,
            0.0,
            1.5,
            0.0,
            2.0,
        );
        let disabled_by_infinity = device::bjt_op(
            parameters.0,
            parameters.1,
            parameters.2,
            parameters.3,
            parameters.4,
            parameters.5,
            parameters.6,
            f64::INFINITY,
            f64::INFINITY,
            f64::INFINITY,
            f64::INFINITY,
            0.0,
            1.5,
            0.0,
            2.0,
        );
        assert_eq!(disabled_by_zero, disabled_by_infinity);
    }

    /// Forward and reverse Gummel–Poon high-injection rolloff, checked against
    /// the closed-form base charge for both polarities and operation directions.
    #[test]
    fn bjt_high_injection_rolloff_matches_gummel_poon_by_hand() {
        let (is, bf, br, vaf, var, ikf, ikr) = (1e-16, 80.0, 4.0, 35.0, 50.0, 2e-4, 5e-4);
        for (vbe, vbc) in [(0.75, -4.25), (-4.25, 0.75)] {
            let forward = (vbe / VT).exp();
            let reverse = (vbc / VT).exp();
            let q1 = 1.0 / (1.0 - vbc / vaf - vbe / var);
            let q2 = is * (forward - 1.0) / ikf + is * (reverse - 1.0) / ikr;
            let qb = q1 * (1.0 + (1.0 + 4.0 * q2).sqrt()) / 2.0;
            let ict = is * (forward - reverse) / qb;
            let ibe = (is / bf) * (forward - 1.0);
            let ibc = (is / br) * (reverse - 1.0);
            let ic = ict - ibc - device::GMIN * vbc;
            let ib = ibe + ibc + device::GMIN * (vbe + vbc);
            let ie = -(ic + ib);
            let direction = if vbe > 0.0 { "forward" } else { "reverse" };

            for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
                let kind = if polarity == Polarity::N {
                    "NPN"
                } else {
                    "PNP"
                };
                let netlist = format!(
                    "Vb b 0 dc {}\nVc c 0 dc 0\nVe e 0 dc {}\nQ1 c b e QMOD\n\
                     .model QMOD {kind}(IS={is} BF={bf} BR={br} VAF={vaf} VAR={var} IKF={ikf} IKR={ikr})\n",
                    sign * vbc,
                    sign * (vbc - vbe),
                );
                let solver = build(&netlist);
                let actual_ic = sign * source_current(&solver, "Vc");
                let actual_ib = sign * source_current(&solver, "Vb");
                let actual_ie = sign * source_current(&solver, "Ve");

                assert!(
                    relative(actual_ic, ic) < 1e-5,
                    "{kind} {direction}: collector current should be {ic} A, got {actual_ic} A"
                );
                assert!(
                    relative(actual_ib, ib) < 1e-5,
                    "{kind} {direction}: base current should be {ib} A, got {actual_ib} A"
                );
                assert!(
                    relative(actual_ie, ie) < 1e-5,
                    "{kind} {direction}: emitter current should be {ie} A, got {actual_ie} A"
                );
                if vbe > 0.0 {
                    assert!(
                        actual_ic / actual_ib < bf,
                        "{kind}: IKF must reduce forward beta"
                    );
                } else {
                    assert!(
                        actual_ie / actual_ib < br,
                        "{kind}: IKR must reduce reverse beta"
                    );
                }
            }
        }
    }

    #[test]
    fn bjt_high_injection_partials_match_finite_difference() {
        let (is, bf, br, nf, nr, vaf, var, ikf, ikr) =
            (2e-15, 75.0, 3.0, 1.1, 1.2, 42.0, 65.0, 2e-4, 5e-4);
        let delta = 1e-7;
        let transport = |vbe: f64, vbc: f64| {
            let forward = (vbe / (nf * VT)).exp();
            let reverse = (vbc / (nr * VT)).exp();
            let q1 = 1.0 / (1.0 - vbc / vaf - vbe / var);
            let q2 = is * (forward - 1.0) / ikf + is * (reverse - 1.0) / ikr;
            let qb = q1 * (1.0 + (1.0 + 4.0 * q2).sqrt()) / 2.0;
            is * (forward - reverse) / qb
        };

        for (vbe, vbc) in [(0.68, -0.1), (-0.1, 0.62)] {
            let op = device::bjt_op(
                vbe, vbc, is, bf, br, nf, nr, vaf, var, ikf, ikr, 0.0, 1.5, 0.0, 2.0,
            );
            let expected_gif =
                (transport(vbe + delta, vbc) - transport(vbe - delta, vbc)) / (2.0 * delta);
            let expected_gir =
                -(transport(vbe, vbc + delta) - transport(vbe, vbc - delta)) / (2.0 * delta);
            assert!(
                relative(op.gif, expected_gif) < 1e-6,
                "gif: {} vs {expected_gif}",
                op.gif
            );
            assert!(
                relative(op.gir, expected_gir) < 1e-6,
                "gir: {} vs {expected_gir}",
                op.gir
            );
        }
    }

    #[test]
    fn bjt_high_injection_solves_a_resistively_loaded_collector() {
        let (vcc, resistance, vbe, is, br, vaf, ikf) = (5.0, 1_000.0, 0.75, 1e-16, 4.0, 50.0, 2e-4);
        let current_balance = |vc: f64| {
            let vbc = vbe - vc;
            let forward = (vbe / VT).exp();
            let reverse = (vbc / VT).exp();
            let q1 = 1.0 / (1.0 - vbc / vaf);
            let q2 = is * (forward - 1.0) / ikf;
            let qb = q1 * (1.0 + (1.0 + 4.0 * q2).sqrt()) / 2.0;
            let ict = is * (forward - reverse) / qb;
            let ibc = (is / br) * (reverse - 1.0);
            let collector_current = ict - ibc;
            (vcc - vc) / resistance - collector_current - device::GMIN * (vc - vbe)
        };
        let mut lo = 0.0;
        let mut hi = vcc;
        for _ in 0..100 {
            let mid = (lo + hi) / 2.0;
            if current_balance(mid) > 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let expected_vc = (lo + hi) / 2.0;
        let solver = build(
            "Vcc supply 0 dc 5\nR1 supply c 1k\nVb b 0 dc 0.75\n\
             Q1 c b 0 QMOD\n.model QMOD NPN(IS=1e-16 BF=80 BR=4 VAF=50 IKF=0.2m)\n",
        );
        let collector = solver.circuit().node("c").unwrap();
        let actual_vc = solver.node_voltage(collector);
        assert!(
            (actual_vc - expected_vc).abs() < 1e-8,
            "loaded collector should settle at {expected_vc} V with IKF rolloff, got {actual_vc} V"
        );
    }

    /// Base-emitter and base-collector recombination currents remain separate
    /// from transport current in forward and reverse operation.
    #[test]
    fn bjt_base_recombination_matches_gummel_poon_by_hand() {
        let (is, bf, br, ise, ne, isc, nc) = (1e-16, 80.0, 4.0, 1e-12, 1.5, 2e-11, 2.0);
        for (vbe, vbc) in [(0.65, -4.35), (-4.35, 0.65)] {
            let forward = (vbe / VT).exp();
            let reverse = (vbc / VT).exp();
            let transport = is * (forward - reverse);
            let ibe = (is / bf) * (forward - 1.0) + ise * ((vbe / (ne * VT)).exp() - 1.0);
            let ibc = (is / br) * (reverse - 1.0) + isc * ((vbc / (nc * VT)).exp() - 1.0);
            let ic = transport - ibc - device::GMIN * vbc;
            let ib = ibe + ibc + device::GMIN * (vbe + vbc);
            let ie = -(ic + ib);
            let direction = if vbe > 0.0 { "forward" } else { "reverse" };

            for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
                let kind = if polarity == Polarity::N {
                    "NPN"
                } else {
                    "PNP"
                };
                let netlist = format!(
                    "Vb b 0 dc {}\nVc c 0 dc 0\nVe e 0 dc {}\nQ1 c b e QMOD\n\
                     .model QMOD {kind}(IS={is} BF={bf} BR={br} ISE={ise} NE={ne} ISC={isc} NC={nc})\n",
                    sign * vbc,
                    sign * (vbc - vbe),
                );
                let solver = build(&netlist);
                let actual_ic = sign * source_current(&solver, "Vc");
                let actual_ib = sign * source_current(&solver, "Vb");
                let actual_ie = sign * source_current(&solver, "Ve");

                assert!(
                    relative(actual_ic, ic) < 1e-5,
                    "{kind} {direction}: collector current should be {ic} A, got {actual_ic} A"
                );
                assert!(
                    relative(actual_ib, ib) < 1e-5,
                    "{kind} {direction}: base current should be {ib} A, got {actual_ib} A"
                );
                assert!(
                    relative(actual_ie, ie) < 1e-5,
                    "{kind} {direction}: emitter current should be {ie} A, got {actual_ie} A"
                );
            }
        }
    }

    #[test]
    fn bjt_base_recombination_partials_match_finite_difference() {
        let (is, bf, br, nf, nr, ise, ne, isc, nc) =
            (2e-15, 75.0, 3.0, 1.1, 1.2, 3e-12, 1.5, 5e-12, 2.0);
        let delta = 1e-7;
        let base_emitter_current = |vbe: f64| {
            (is / bf) * ((vbe / (nf * VT)).exp() - 1.0) + ise * ((vbe / (ne * VT)).exp() - 1.0)
        };
        let base_collector_current = |vbc: f64| {
            (is / br) * ((vbc / (nr * VT)).exp() - 1.0) + isc * ((vbc / (nc * VT)).exp() - 1.0)
        };

        for (vbe, vbc) in [(0.62, -0.1), (-0.1, 0.58)] {
            let op = device::bjt_op(
                vbe,
                vbc,
                is,
                bf,
                br,
                nf,
                nr,
                f64::INFINITY,
                f64::INFINITY,
                f64::INFINITY,
                f64::INFINITY,
                ise,
                ne,
                isc,
                nc,
            );
            let expected_gpi = (base_emitter_current(vbe + delta)
                - base_emitter_current(vbe - delta))
                / (2.0 * delta);
            let expected_gmu = (base_collector_current(vbc + delta)
                - base_collector_current(vbc - delta))
                / (2.0 * delta);
            assert!(
                relative(op.gpi, expected_gpi) < 1e-6,
                "gpi: {} vs {expected_gpi}",
                op.gpi
            );
            assert!(
                relative(op.gmu, expected_gmu) < 1e-6,
                "gmu: {} vs {expected_gmu}",
                op.gmu
            );
        }
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

    /// The fourth terminal is not silently grounded: in cutoff, its two bulk
    /// junctions carry the hand-calculated reverse leakage, including GMIN.
    /// The normalized voltages keep both body diodes reverse biased, and the
    /// same stamp is checked for NMOS and PMOS.
    #[test]
    fn mosfet_bulk_terminal_carries_its_two_reverse_junction_currents() {
        let (vds, vbulk) = (0.4, -0.1);
        let is = 1e-14;
        let vbd = vbulk - vds;
        let vbs = vbulk;
        let bulk_to_drain = is * ((vbd / VT).exp() - 1.0) + device::GMIN * vbd;
        let bulk_to_source = is * ((vbs / VT).exp() - 1.0) + device::GMIN * vbs;
        let expected_drain = device::GMIN * vds - bulk_to_drain;
        let expected_bulk = bulk_to_drain + bulk_to_source;
        let expected_source = -device::GMIN * vds - bulk_to_source;

        for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
            let kind = if polarity == Polarity::N {
                "NMOS"
            } else {
                "PMOS"
            };
            let netlist = format!(
                "Vg g 0 dc 0\nVd d 0 dc {}\nVs s 0 dc 0\nVbulk b 0 dc {}\n\
                 M1 d g s b MMOD\n.model MMOD {kind}(VTO={} KP=20u LAMBDA=0 IS={is})\n",
                sign * vds,
                sign * vbulk,
                sign
            );
            let solver = build(&netlist);
            let actual_drain = sign * source_current(&solver, "Vd");
            let actual_bulk = sign * source_current(&solver, "Vbulk");
            let actual_source = sign * source_current(&solver, "Vs");

            assert!(
                (actual_drain - expected_drain).abs() < device::GMIN * 1e-3,
                "{kind}: drain current should include drain-source and drain-bulk GMIN ties; \
                 expected {expected_drain} A, got {actual_drain} A"
            );
            assert!(
                (actual_bulk - expected_bulk).abs() < device::GMIN * 1e-3,
                "{kind}: bulk current should be {expected_bulk} A, got {actual_bulk} A"
            );
            assert!(
                (actual_source - expected_source).abs() < device::GMIN * 1e-3,
                "{kind}: source current should include its bulk tie; expected {expected_source} A, \
                 got {actual_source} A"
            );
        }
    }

    /// The level-1 bulk junctions conduct from bulk to drain/source for NMOS,
    /// and in the opposite physical direction for PMOS. With the channel off,
    /// the two terminal currents reduce to the Shockley equation.
    #[test]
    fn mosfet_body_diodes_match_the_shockley_equation_for_both_polarities() {
        let is = 2e-14;
        let vbody = 0.55;
        let expected_each = is * ((vbody / VT).exp() - 1.0) + device::GMIN * vbody;

        for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
            let kind = if polarity == Polarity::N {
                "NMOS"
            } else {
                "PMOS"
            };
            let netlist = format!(
                "Vg g 0 dc 0\nVd d 0 dc 0\nVs s 0 dc 0\nVbulk b 0 dc {}\n\
                 M1 d g s b MMOD\n.model MMOD {kind}(VTO={} KP=20u LAMBDA=0 IS={is})\n",
                sign * vbody,
                sign
            );
            let solver = build(&netlist);
            assert_eq!(solver.circuit().mosfets[0].model.is, is);

            let bulk_current = sign * source_current(&solver, "Vbulk");
            let drain_current = sign * source_current(&solver, "Vd");
            let source_current = sign * source_current(&solver, "Vs");
            assert!(
                relative(bulk_current, 2.0 * expected_each) < 1e-6,
                "{kind}: bulk should supply both body diode currents ({}) A, got {bulk_current} A",
                2.0 * expected_each
            );
            assert!(
                relative(drain_current, -expected_each) < 1e-6,
                "{kind}: drain should receive one body diode current (-{expected_each} A), got {drain_current} A"
            );
            assert!(
                relative(source_current, -expected_each) < 1e-6,
                "{kind}: source should receive one body diode current (-{expected_each} A), got {source_current} A"
            );
        }
    }

    /// Level-1 body effect shifts the threshold for reverse body bias. Check
    /// its closed-form current and Newton stamp for both channel polarities,
    /// with either physical drain acting as the effective source.
    #[test]
    fn mosfet_body_effect_matches_level1_for_both_polarities_and_directions() {
        let (kp, w, l, lambda, gamma, phi, vto): (f64, f64, f64, f64, f64, f64, f64) =
            (20e-6, 100e-6, 10e-6, 0.02, 0.4, 0.7, 1.0);
        let beta = kp * w / l;
        let vds = 0.4;
        let vgs = 3.0;
        let vbs = -0.2;
        let is = 1e-14;
        let threshold = vto + gamma * ((phi - vbs).sqrt() - phi.sqrt());
        let overdrive = vgs - threshold;
        let channel = beta * (1.0 + lambda * vds) * vds * (overdrive - 0.5 * vds);
        let vbd = vbs - vds;
        let expected = channel + device::GMIN * vds - device::diode_op(vbd, is, 1.0).id;

        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            for reverse in [false, true] {
                let (drain, source, supply) = if reverse {
                    (0.0, vds, "Vs")
                } else {
                    (vds, 0.0, "Vd")
                };
                let netlist = format!(
                    "Vg g 0 dc {}\nVd d 0 dc {}\nVs s 0 dc {}\nVbulk b 0 dc {}\nM1 d g s b MMOD\n.model MMOD {kind}(VTO={} KP={kp} LAMBDA={lambda} GAMMA={gamma} PHI={phi} IS={is} W={w} L={l})\n",
                    sign * vgs,
                    sign * drain,
                    sign * source,
                    sign * vbs,
                    sign * vto,
                );
                let solver = build(&netlist);
                let measured = sign * source_current(&solver, supply);
                let direction = if reverse { "reverse" } else { "forward" };
                assert!(
                    relative(measured, expected) < 1e-6,
                    "{kind} {direction}: body-shifted current should be {expected} A, got {measured} A"
                );
            }
        }
    }

    /// The three level-1 gate overlap parameters become linear G-S, G-D and
    /// G-B capacitances after applying the instance width/length. Each route
    /// is solved as an RC transient for NMOS and PMOS under both integration
    /// rules; the first step restarts with BE and the second uses its selected
    /// rule and the prior charge/current history.
    #[test]
    fn mosfet_overlap_charge_integrates_cgso_cgdo_and_cgbo() {
        let width = 2e-3;
        let length = 3e-3;
        let resistance = 1e3;
        let h = 1e-12;
        let input = 0.2;
        let cases = [
            ("CGSO", "2.5p", 2.5e-12 * width, 0),
            ("CGDO", "5p", 5e-12 * width, 1),
            (
                "CGBO",
                "6.666666666666667p",
                6.666666666666667e-12 * length,
                2,
            ),
        ];

        for (parameter, value, capacitance, junction) in cases {
            for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
                let (drain, source, bulk) = match junction {
                    0 => ("0", "x", "0"),
                    1 => ("x", "0", "0"),
                    2 => ("0", "0", "x"),
                    _ => unreachable!(),
                };
                let vto = if kind == "NMOS" { 2.0 } else { -2.0 };
                let netlist = format!(
                    "Vg g 0 dc 0\nR1 x 0 {resistance}\n\
                     M1 {drain} g {source} {bulk} MMOD w={width} l={length}\n\
                     .model MMOD {kind}(VTO={vto} KP=20u IS=1e-30 {parameter}={value})\n"
                );
                let circuit = parse_netlist(&netlist).expect("MOS overlap model parses");
                let mosfet = &circuit.mosfets[0];
                assert!((mosfet.overlap_capacitances()[junction] - capacitance).abs() < 1e-27);
                for (index, other) in mosfet.overlap_capacitances().into_iter().enumerate() {
                    if index != junction {
                        assert_eq!(other, 0.0);
                    }
                }

                let initially_biased =
                    netlist.replacen("Vg g 0 dc 0", &format!("Vg g 0 dc {}", sign * input), 1);
                let mut biased_solver = Solver::new(
                    parse_netlist(&initially_biased).expect("prebiased MOS overlap parses"),
                    Integration::BackwardEuler,
                )
                .expect("prebiased MOS overlap solver builds");
                let biased_x = biased_solver
                    .circuit()
                    .node("x")
                    .expect("overlap terminal node");
                biased_solver
                    .advance(h)
                    .expect("operating-point overlap charge carries into transient");
                assert!(
                    biased_solver.node_voltage(biased_x).abs() < 1e-12,
                    "{kind} {parameter} must not create startup current from its operating-point charge"
                );

                let ratio = resistance * capacitance / h;
                let expected_first = sign * input * ratio / (1.0 + ratio);
                for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                    let mut solver = Solver::new(circuit.clone(), integration)
                        .expect("MOS overlap solver builds");
                    let x = solver.circuit().node("x").expect("overlap terminal node");
                    let source = solver.voltage_source_index("Vg").expect("gate source");
                    solver.set_voltage_source(source, sign * input);
                    solver.advance(h).expect("first MOS overlap step solves");
                    let actual_first = solver.node_voltage(x);
                    assert!(
                        (actual_first - expected_first).abs() < 1e-9,
                        "{kind} {parameter} first step: expected {expected_first} V, got {actual_first} V"
                    );

                    let expected_second = match integration {
                        Integration::BackwardEuler => ratio * expected_first / (1.0 + ratio),
                        Integration::Trapezoidal => {
                            ratio * (3.0 * expected_first - sign * input) / (1.0 + 2.0 * ratio)
                        }
                    };
                    solver.advance(h).expect("second MOS overlap step solves");
                    let actual_second = solver.node_voltage(x);
                    assert!(
                        (actual_second - expected_second).abs() < 1e-9,
                        "{kind} {parameter} {integration:?} second step: expected {expected_second} V, got {actual_second} V"
                    );
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // JFET
    // -----------------------------------------------------------------------

    /// The three level-1 channel regions are checked against the model's
    /// closed form, with both channel polarities and reverse gate-junction
    /// leakage included in the measured drain supply current.
    #[test]
    fn jfet_operating_point_matches_level1_channel_equation_by_hand() {
        let beta = 2e-4;
        let lambda = 0.02;
        let is = 1e-14;
        let vto = -2.0;

        for (polarity, sign) in [(Polarity::N, 1.0), (Polarity::P, -1.0)] {
            let kind = if polarity == Polarity::N {
                "NJF"
            } else {
                "PJF"
            };
            for (vgs, vds) in [(-2.5, 5.0), (0.0, 0.4), (0.0, 5.0)] {
                let netlist = format!(
                    "Vg g 0 dc {}\nVd d 0 dc {}\nVs s 0 dc 0\n\
                     J1 d g s JMOD\n.model JMOD {kind}(VTO={} BETA={beta} \
                     LAMBDA={lambda} IS={is})\n",
                    sign * vgs,
                    sign * vds,
                    sign * vto
                );
                let solver = build(&netlist);
                let vgst = vgs - vto;
                let channel = if vgst <= 0.0 {
                    0.0
                } else if vds < vgst {
                    beta * (1.0 + lambda * vds) * vds * (2.0 * vgst - vds)
                } else {
                    beta * (1.0 + lambda * vds) * vgst * vgst
                };
                let gate_drain_voltage = vgs - vds;
                let expected =
                    channel + device::GMIN * vds - device::diode_op(gate_drain_voltage, is, 1.0).id;
                let measured = sign * source_current(&solver, "Vd");
                let label = format!("{kind} Vgs={vgs} Vds={vds}");
                assert!(
                    relative(measured, expected) < 2e-5,
                    "{label}: expected {expected} A including gate leakage, got {measured} A"
                );
            }
        }
    }

    /// The JFET channel is symmetric, and each gate junction conducts when
    /// forward biased. Both behaviors are exercised through the MNA solver.
    #[test]
    fn jfet_reverse_channel_and_forward_gate_junction_are_solved() {
        let deck = |kind: &str, sign: f64, drain: f64, source: f64| {
            format!(
                "Vg g 0 dc 0\nVd d 0 dc {}\nVs s 0 dc {}\n\
                 J1 d g s JMOD\n.model JMOD {kind}(VTO={} BETA=200u IS=1e-12)\n",
                sign * drain,
                sign * source,
                -2.0 * sign
            )
        };
        for (kind, sign) in [("NJF", 1.0), ("PJF", -1.0)] {
            let forward = sign * source_current(&build(&deck(kind, sign, 0.4, 0.0)), "Vd");
            let reverse = sign * source_current(&build(&deck(kind, sign, 0.0, 0.4)), "Vs");
            assert!(
                forward > 1e-6,
                "{kind} forward channel current should be on, got {forward} A"
            );
            assert!(
                relative(reverse, forward) < 1e-6,
                "{kind} reverse channel should mirror: {forward} A vs {reverse} A"
            );
        }

        let is = 1e-12;
        let expected = 2.0 * device::diode_op(0.4, is, 1.0).id;
        for (kind, sign) in [("NJF", 1.0), ("PJF", -1.0)] {
            let netlist = format!(
                "Vg g 0 dc {}\nVd d 0 dc 0\nVs s 0 dc 0\n\
                 J1 d g s JMOD\n.model JMOD {kind}(IS={is})\n",
                sign * 0.4
            );
            let measured = sign * source_current(&build(&netlist), "Vg");
            assert!(
                relative(measured, expected) < 1e-5,
                "{kind} gate current should be the two forward junctions: expected \
                 {expected} A, got {measured} A"
            );
        }
    }

    /// Each level-1 JFET gate depletion capacitance is parsed and integrated
    /// as a nonlinear junction charge for NJF and PJF, under both companion
    /// rules. The expected node voltage is solved independently from the
    /// level-1 M=0.5 charge equation and the BE/trapezoidal current formulas.
    #[test]
    fn jfet_gate_depletion_charge_integrates_cgs_and_cgd_for_both_polarities() {
        let c0 = 10e-9;
        let pb = 0.75;
        let fc: f64 = 0.5;
        let resistance = 1e3;
        let h = 100e-6;
        let input = 0.6;
        let vfc = fc * pb;
        let qfc = c0 * pb / 0.5 * (1.0 - (1.0 - fc).sqrt());
        let cfc = c0 * (1.0 - fc).powf(-0.5);
        let slope = 0.5 * c0 * (1.0 - fc).powf(-1.5) / pb;
        let charge = |voltage: f64| {
            if voltage < vfc {
                c0 * pb / 0.5 * (1.0 - (1.0 - voltage / pb).sqrt())
            } else {
                let dv = voltage - vfc;
                qfc + cfc * dv + 0.5 * slope * dv * dv
            }
        };
        let solve_voltage = |previous_charge: f64, previous_current: f64, rule: Integration| {
            let mut lo = 0.0;
            // Trapezoidal integration can overshoot a step input because its
            // history term retains the previous positive current.
            let mut hi = input + 0.2;
            for _ in 0..100 {
                let mid = 0.5 * (lo + hi);
                let cap_current = match rule {
                    Integration::BackwardEuler => (charge(mid) - previous_charge) / h,
                    Integration::Trapezoidal => {
                        2.0 * (charge(mid) - previous_charge) / h - previous_current
                    }
                };
                if (input - mid) / resistance > cap_current {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            0.5 * (lo + hi)
        };

        for capacitance in ["CGS", "CGD"] {
            for (kind, sign) in [("NJF", 1.0), ("PJF", -1.0)] {
                let (cgs, cgd) = if capacitance == "CGS" {
                    (c0, 0.0)
                } else {
                    (0.0, c0)
                };
                let netlist = format!(
                    "V1 in 0 dc 0\nR1 in g {resistance}\nVd d 0 dc 0\nVs s 0 dc 0\n\
                     J1 d g s JMOD\n.model JMOD {kind}(VTO={} BETA=100u IS=1e-30 \
                     CGS={cgs} CGD={cgd} PB={pb} FC={fc})\n",
                    if kind == "NJF" { -2.0 } else { 2.0 }
                );
                let circuit = parse_netlist(&netlist).expect("JFET capacitance model parses");
                let model = circuit.jfets[0].model;
                assert_eq!(model.cgs, cgs);
                assert_eq!(model.cgd, cgd);
                assert_eq!(model.pb, pb);
                assert_eq!(model.fc, fc);

                for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                    let mut solver = Solver::new(circuit.clone(), integration)
                        .expect("JFET capacitance solver builds");
                    let gate = solver.circuit().node("g").expect("gate node");
                    let source = solver.voltage_source_index("V1").expect("input source");
                    solver.set_voltage_source(source, sign * input);

                    let first = solve_voltage(0.0, 0.0, Integration::BackwardEuler);
                    assert!(
                        first > vfc,
                        "the first step must exercise the FC continuation"
                    );
                    solver.advance(h).expect("first gate-charge step solves");
                    let actual_first = solver.node_voltage(gate);
                    assert!(
                        (actual_first - sign * first).abs() < 1e-8,
                        "{kind} {capacitance} first {integration:?} step: expected {} V, got {actual_first} V",
                        sign * first
                    );

                    let first_charge = charge(first);
                    let first_current = first_charge / h;
                    let second = solve_voltage(first_charge, first_current, integration);
                    assert!(
                        second > vfc,
                        "the second step must exercise the FC continuation"
                    );
                    solver.advance(h).expect("second gate-charge step solves");
                    let actual_second = solver.node_voltage(gate);
                    assert!(
                        (actual_second - sign * second).abs() < 1e-8,
                        "{kind} {capacitance} second {integration:?} step: expected {} V, got {actual_second} V",
                        sign * second
                    );
                }
            }
        }
    }

    #[test]
    fn jfet_gate_depletion_parameters_reject_invalid_values() {
        for parameter in ["CGS=-1n", "PB=0", "FC=1"] {
            let netlist = format!(
                "Vd d 0 dc 0\nVg g 0 dc 0\nVs s 0 dc 0\nJ1 d g s JMOD\n\
                 .model JMOD NJF({parameter})\n"
            );
            let error = parse_netlist(&netlist).expect_err("invalid JFET capacitance is refused");
            assert!(error.to_string().contains("JFET model"), "{error}");
        }
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
        assert!(build("V1 in 0 dc 0\nR1 in d 1k\nJ1 d in 0 NJF\n").is_nonlinear());
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
