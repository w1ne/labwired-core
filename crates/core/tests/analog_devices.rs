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
//! What remains unsupported is documented in `cosimulation_plugins.md`. Diode
//! forward/reverse high-injection knees (`IK`/`IKF`/`IKR`) and the BJT
//! high-current beta exponent (`NKF`), recombination
//! current (`ISR`/`NR`), diffusion charge (`TT`) and soft reverse recovery
//! (`TT`/`VP`), plus BJT forward bias-dependent and reverse
//! transit-time charge (`TF`/`TR`/`XTF`/`VTF`/`ITF`) are exercised below. Diode
//! depletion charge through `CJO`/`VJ`/`M`/`FC` and reverse breakdown emission
//! through `NBV`, BJT base-junction depletion charge through `CJE`/`CJC`,
//! JFET gate-junction depletion charge through
//! `CGS`/`CGD` and the Parker–Skellern channel fit `B`, MOSFET linear overlap
//! charge through `CGSO`/`CGDO`/`CGBO`,
//! MOSFET bulk-junction depletion charge through `CBD`/`CBS` and `CJ`/`CJSW`
//! geometry, MOSFET intrinsic channel capacitance from `TOX`, and JFET terminal
//! resistances `RD`/`RS`, and MOSFET terminal resistances from `RD`/`RS` or
//! `RSH` times per-instance `NRD`/`NRS`, and BJT current-dependent base
//! resistance through `RBM`/`IRB` are exercised below. Circuits depending
//! on unsupported stored charge still belong to `adapter: external_process`.

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

    /// `IK`/`IKF` and `IKR` reduce forward and reverse junction current at
    /// high injection. The source voltage is derived from the selected
    /// junction voltage and the ngspice current transform, so each case checks
    /// the nonlinear stamp rather than only its parsed model fields.
    #[test]
    fn diode_forward_and_reverse_knee_currents_match_the_spice_equation() {
        let (is, resistance) = (1e-14, 1e3);

        let forward_vd = 0.7;
        let forward_ik = 100e-6;
        let forward_base = is * ((forward_vd / VT).exp() - 1.0);
        let forward_current =
            forward_base / (1.0 + (forward_base / forward_ik).sqrt()) + device::GMIN * forward_vd;
        let forward_netlist = format!(
            "V1 in 0 dc {}\nR1 in out {resistance}\nD1 out 0 DMOD\n\
             .model DMOD D(IS={is} N=1 IK={forward_ik})\n",
            forward_vd + forward_current * resistance,
        );
        let forward = build(&forward_netlist);
        let forward_out = forward.circuit().node("out").expect("forward node");
        assert!(
            (forward.node_voltage(forward_out) - forward_vd).abs() < 1e-8,
            "forward junction should be {forward_vd} V, got {} V",
            forward.node_voltage(forward_out)
        );
        assert!(
            relative(source_current(&forward, "V1"), forward_current) < 1e-7,
            "forward knee current should be {forward_current} A, got {} A",
            source_current(&forward, "V1")
        );

        let reverse_vd = -5.0;
        let reverse_ikr = 10e-6;
        let reverse_model = parse_netlist(&format!(
            "V1 in 0 dc 0\nR1 in out {resistance}\nD1 out 0 DMOD\n\
             .model DMOD D(IS={is} N=1 BV=5 IBV=1m IKR={reverse_ikr})\n"
        ))
        .expect("reverse knee model parses");
        let diode = &reverse_model.diodes[0];
        let xbv = diode.breakdown.expect("BV enables reverse breakdown");
        let reverse_base = -is * (-(xbv + reverse_vd) / VT).exp();
        let reverse_current =
            reverse_base / (1.0 + (-reverse_base / reverse_ikr).sqrt()) + device::GMIN * reverse_vd;
        let reverse_netlist = format!(
            "V1 in 0 dc {}\nR1 in out {resistance}\nD1 out 0 DMOD\n\
             .model DMOD D(IS={is} N=1 BV=5 IBV=1m IKR={reverse_ikr})\n",
            reverse_vd + reverse_current * resistance,
        );
        let reverse = build(&reverse_netlist);
        let reverse_out = reverse.circuit().node("out").expect("reverse node");
        assert!(
            (reverse.node_voltage(reverse_out) - reverse_vd).abs() < 1e-7,
            "reverse junction should be {reverse_vd} V, got {} V",
            reverse.node_voltage(reverse_out)
        );
        assert!(
            relative(source_current(&reverse, "V1"), reverse_current) < 1e-6,
            "reverse knee current should be {reverse_current} A, got {} A",
            source_current(&reverse, "V1")
        );
    }

    /// `NBV` controls the reverse-breakdown exponential independently of the
    /// forward emission coefficient `N`; exercise it through model parsing and
    /// a solved circuit rather than only through the voltage-fit helper.
    #[test]
    fn diode_nondefault_breakdown_emission_matches_the_reverse_current_equation() {
        let (is, n, bv, ibv, nbv, vd) = (1e-14, 1.2, 5.0, 1e-3, 2.0, -5.05);
        let solver = build(&format!(
            "Vd a 0 dc {vd}\nD1 a 0 DBV\n\
             .model DBV D(IS={is} N={n} BV={bv} IBV={ibv} NBV={nbv})\n"
        ));
        let diode = &solver.circuit().diodes[0];
        assert_eq!(diode.model.nbv, Some(nbv));

        // At BV, IBV sets the reverse current; below BV, NBV sets its
        // exponential slope. The tiny correction from ngspice's effective-BV
        // fit is below 2e-9 relative for these values.
        let expected = -ibv * ((-vd - bv) / (nbv * VT)).exp() + 1e-12 * vd;
        let actual = source_current(&solver, "Vd");
        assert!(
            (actual - expected).abs() < 1e-10 + 2e-8 * expected.abs(),
            "NBV={nbv}: expected reverse current {expected} A, got {actual} A"
        );

        let default = build(&format!(
            "Vd a 0 dc {vd}\nD1 a 0 DBV\n\
             .model DBV D(IS={is} N={n} BV={bv} IBV={ibv})\n"
        ));
        let default_current = source_current(&default, "Vd");
        let expected_default = -ibv * ((-vd - bv) / (n * VT)).exp() + 1e-12 * vd;
        assert!(
            (default_current - expected_default).abs()
                < 1e-10 + 2e-8 * expected_default.abs(),
            "default NBV=N={n}: expected reverse current {expected_default} A, got {default_current} A"
        );
        assert!(
            (actual - default_current).abs() > 1e-3,
            "explicit NBV should change current: {actual} A vs default {default_current} A"
        );
    }

    /// Ngspice's `ISR`/`NR` branch adds recombination current in forward bias
    /// and holds its value at `−3·N·Vt` farther into reverse bias.
    #[test]
    fn diode_recombination_current_matches_the_forward_and_reverse_equations() {
        let (is, n, isr, nr, vj, m) = (1e-14, 1.3, 2e-8, 2.0, 0.75, 0.4);
        let recombination_current = |voltage: f64| {
            let boundary = -3.0 * n * VT;
            let voltage = voltage.max(boundary);
            let base = isr * ((voltage / (nr * VT)).exp() - 1.0);
            let x = 1.0 - voltage / vj;
            base * (x * x + 0.005).powf(m / 2.0)
        };

        for vd in [-0.3, -0.02, 0.15, 0.35] {
            let solver = build(&format!(
                "Vd a 0 dc {vd}\nD1 a 0 DREC\n.model DREC D(IS={is} N={n} ISR={isr} NR={nr} VJ={vj} M={m})\n"
            ));
            let expected =
                is * ((vd / (n * VT)).exp() - 1.0) + device::GMIN * vd + recombination_current(vd);
            let actual = source_current(&solver, "Vd");
            assert!(
                (actual - expected).abs() < 2e-12 + 2e-8 * expected.abs(),
                "at Vd={vd} V: expected {expected} A, got {actual} A"
            );
        }

        let (bv, ibv, vd) = (5.0, 1e-3, -6.0);
        let xbv = device::breakdown_voltage(bv, ibv, is, n);
        let solver = build(&format!(
            "Vd a 0 dc {vd}\nD1 a 0 DREC\n.model DREC D(IS={is} N={n} BV={bv} IBV={ibv} ISR={isr} NR={nr} VJ={vj} M={m})\n"
        ));
        let expected = device::diode_op_breakdown(vd, is, n, xbv, n).id + recombination_current(vd);
        let actual = source_current(&solver, "Vd");
        assert!(
            (actual - expected).abs() < 2e-12 + 2e-8 * expected.abs(),
            "at breakdown Vd={vd} V: expected {expected} A, got {actual} A"
        );

        // The analytic tangent also drives a loaded operating point to the
        // root of the independent diode equation.
        let supply = 0.35;
        let resistance = 10_000.0;
        let diode_current = |voltage: f64| {
            is * ((voltage / (n * VT)).exp() - 1.0)
                + device::GMIN * voltage
                + recombination_current(voltage)
        };
        let mut low = 0.0;
        let mut high = supply;
        for _ in 0..100 {
            let mid = (low + high) / 2.0;
            if supply - mid - resistance * diode_current(mid) > 0.0 {
                low = mid;
            } else {
                high = mid;
            }
        }
        let expected_vd = (low + high) / 2.0;
        let solver = build(&format!(
            "Vs s 0 dc {supply}\nR1 s a {resistance}\nD1 a 0 DREC\n.model DREC D(IS={is} N={n} ISR={isr} NR={nr} VJ={vj} M={m})\n"
        ));
        let anode = solver.circuit().node("a").expect("anode");
        let actual_vd = solver.node_voltage(anode);
        assert!(
            (actual_vd - expected_vd).abs() < 1e-8,
            "expected loaded diode voltage {expected_vd} V, got {actual_vd} V"
        );
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

    /// `TT` adds quasi-static diffusion charge `TT·Id` to the junction. The
    /// circuit checks the transient equation against a scalar root under both
    /// integration rules and confirms the operating-point charge is carried
    /// into the first step without a startup impulse.
    #[test]
    fn diode_tt_integrates_diffusion_charge_for_both_integration_rules() {
        let is = 1e-14;
        let tt = 100e-9;
        let resistance = 1e3;
        let h = 20e-9;
        let netlist = format!(
            "V1 in 0 dc 0\nR1 in out {resistance}\nD1 out 0 DMOD\n\
             .model DMOD D(IS={is} N=1 TT={tt})\n"
        );
        let circuit = parse_netlist(&netlist).expect("diode TT model parses");
        assert_eq!(circuit.diodes[0].model.tt, tt);

        let charge = |voltage: f64| {
            let op = device::diode_op(voltage, is, 1.0);
            device::diode_diffusion_charge(voltage, op, tt).0
        };

        // A diode already at its operating point must not create a transient
        // current just because its diffusion charge was initialized.
        let biased_netlist = netlist.replacen("V1 in 0 dc 0", "V1 in 0 dc 1", 1);
        let mut biased = Solver::new(
            parse_netlist(&biased_netlist).expect("prebiased diode TT model parses"),
            Integration::BackwardEuler,
        )
        .expect("prebiased diode TT solver builds");
        let biased_out = biased.circuit().node("out").expect("output node");
        let initial_biased_voltage = biased.node_voltage(biased_out);
        biased
            .advance(h)
            .expect("operating-point diffusion charge carries into transient");
        assert!(
            (biased.node_voltage(biased_out) - initial_biased_voltage).abs() < 1e-9,
            "initialized diffusion charge must not create a startup impulse"
        );

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            let mut solver =
                Solver::new(circuit.clone(), integration).expect("diode TT solver builds");
            let out = solver.circuit().node("out").expect("output node");
            let source = solver.voltage_source_index("V1").expect("source exists");
            solver.set_voltage_source(source, 1.0);

            let mut previous_charge = charge(solver.node_voltage(out));
            let mut previous_current = 0.0;
            for step in 0..2 {
                let charge_current = |voltage: f64| {
                    let next_charge = charge(voltage);
                    if step == 0 || integration == Integration::BackwardEuler {
                        (next_charge - previous_charge) / h
                    } else {
                        2.0 * (next_charge - previous_charge) / h - previous_current
                    }
                };
                let residual = |voltage: f64| {
                    (1.0 - voltage) / resistance
                        - device::diode_op(voltage, is, 1.0).id
                        - charge_current(voltage)
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
                solver
                    .advance(h)
                    .expect("diode diffusion-charge step solves");
                let actual = solver.node_voltage(out);
                assert!(
                    (actual - expected).abs() < 1e-8,
                    "{integration:?} step {step}: expected {expected} V, got {actual} V"
                );
                let next_charge = charge(actual);
                previous_current = (next_charge - previous_charge) / h;
                previous_charge = next_charge;
            }
        }
    }

    /// The diffusion charge must use the same high-injection-limited junction
    /// current as the diode's conductive stamp. Exercise the interaction in a
    /// solved transient circuit: an ideal `TT·I` charge would produce a
    /// measurably different node voltage once the base current is well above
    /// `IK`.
    #[test]
    fn diode_high_injection_scales_transit_charge_in_transient_steps() {
        let (is, ik, tt, resistance, h) = (1e-14, 100e-6, 100e-9, 1e3, 1e-6);
        let circuit = parse_netlist(&format!(
            "V1 in 0 dc 0\nR1 in out {resistance}\nD1 out 0 DMOD\n\
             .model DMOD D(IS={is} N=1 IK={ik} TT={tt})\n"
        ))
        .expect("high-injection diode with transit charge parses");

        // Independent forward-current transform from the SPICE IK equation.
        // GMIN is outside the high-injection correction and carries no stored
        // charge, so it is included in I(V) but excluded from Q(V).
        let base_current = |voltage: f64| is * ((voltage / VT).exp() - 1.0);
        let current = |voltage: f64| {
            let base = base_current(voltage);
            base / (1.0 + (base / ik).sqrt()) + device::GMIN * voltage
        };
        let charge = |voltage: f64| {
            let base = base_current(voltage);
            tt * base / (1.0 + (base / ik).sqrt())
        };

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            let mut solver =
                Solver::new(circuit.clone(), integration).expect("transit solver builds");
            let out = solver.circuit().node("out").expect("output node");
            let source = solver.voltage_source_index("V1").expect("source exists");
            solver.set_voltage_source(source, 2.0);

            let mut previous_charge = charge(solver.node_voltage(out));
            let mut previous_current = 0.0;
            for step in 0..2 {
                let rule = if step == 0 {
                    Integration::BackwardEuler
                } else {
                    integration
                };
                let residual = |voltage: f64| {
                    let next_charge = charge(voltage);
                    let charge_current = match rule {
                        Integration::BackwardEuler => (next_charge - previous_charge) / h,
                        Integration::Trapezoidal => {
                            2.0 * (next_charge - previous_charge) / h - previous_current
                        }
                    };
                    (2.0 - voltage) / resistance - current(voltage) - charge_current
                };
                let mut low = 0.0;
                let mut high = 2.0;
                for _ in 0..100 {
                    let mid = 0.5 * (low + high);
                    if residual(mid) > 0.0 {
                        low = mid;
                    } else {
                        high = mid;
                    }
                }
                let expected = 0.5 * (low + high);
                solver
                    .advance(h)
                    .expect("high-injection transit-charge step solves");
                let actual = solver.node_voltage(out);
                assert!(
                    (actual - expected).abs() < 1e-8,
                    "{integration:?} step {step}: expected {expected} V, got {actual} V"
                );

                let next_charge = charge(actual);
                previous_current = (next_charge - previous_charge) / h;
                previous_charge = next_charge;
                assert!(
                    base_current(actual) / ik > 10.0,
                    "the solved junction must be in high injection; V={actual} V"
                );
            }
        }
    }

    /// `VP` makes the transit charge a first-order state. This step-by-step
    /// scalar solve checks its implicit BE/trapezoidal equation through a
    /// forward-to-reverse transition, where the stored charge produces a
    /// reverse current instead of disappearing in one step.
    #[test]
    fn diode_vp_soft_recovery_matches_the_charge_state_equation() {
        let (is, tt, vp, resistance, h) = (1e-14, 100e-9, 0.2, 1e3, 10e-9);
        let circuit = parse_netlist(&format!(
            "V1 in 0 dc 0\nR1 in out {resistance}\nD1 out 0 DMOD\n\
             .model DMOD D(IS={is} N=1 TT={tt} VP={vp})\n"
        ))
        .expect("soft-recovery diode netlist parses");

        let step = |voltage: f64, previous_state: f64, previous_rate: f64, rule: Integration| {
            let diode = device::diode_op(voltage, is, 1.0);
            let tau = tt * vp;
            let (state, rate) = match rule {
                Integration::BackwardEuler => {
                    let ratio = tau / h;
                    let state = (-tt * diode.id + ratio * previous_state) / (1.0 + ratio);
                    (state, (state - previous_state) / h)
                }
                Integration::Trapezoidal => {
                    let ratio = tau / h;
                    let state =
                        (-tt * diode.id + 2.0 * ratio * previous_state + tau * previous_rate)
                            / (1.0 + 2.0 * ratio);
                    (state, 2.0 * (state - previous_state) / h - previous_rate)
                }
            };
            (state, rate, -(1.0 - vp) * rate)
        };

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            let mut solver =
                Solver::new(circuit.clone(), integration).expect("soft-recovery solver builds");
            let out = solver.circuit().node("out").expect("output node");
            let source = solver.voltage_source_index("V1").expect("source exists");
            let mut previous_state = -tt * device::diode_op(solver.node_voltage(out), is, 1.0).id;
            let mut previous_rate = 0.0;

            for (step_index, supply) in [1.0, 1.0, 1.0, 1.0, -1.0, -1.0, -1.0]
                .into_iter()
                .enumerate()
            {
                solver.set_voltage_source(source, supply);
                let rule = if step_index == 0 || step_index == 4 {
                    Integration::BackwardEuler
                } else {
                    integration
                };
                let residual = |voltage: f64| {
                    let (_, _, recovery_current) =
                        step(voltage, previous_state, previous_rate, rule);
                    (supply - voltage) / resistance
                        - device::diode_op(voltage, is, 1.0).id
                        - recovery_current
                };
                let mut low = supply - 10.0;
                let mut high = supply + 10.0;
                for _ in 0..100 {
                    let mid = 0.5 * (low + high);
                    if residual(mid) > 0.0 {
                        low = mid;
                    } else {
                        high = mid;
                    }
                }
                let expected = 0.5 * (low + high);

                solver
                    .advance(h)
                    .expect("soft-recovery transient step solves");
                let actual = solver.node_voltage(out);
                assert!(
                    (actual - expected).abs() < 1e-8,
                    "{integration:?} step {step_index}: expected {expected} V, got {actual} V"
                );
                if step_index == 4 {
                    let reverse_current = (supply - actual) / resistance;
                    assert!(
                        reverse_current < -1e-4,
                        "stored forward charge should drive reverse current, got {reverse_current} A"
                    );
                }
                let (next_state, next_rate, _) = step(actual, previous_state, previous_rate, rule);
                previous_state = next_state;
                previous_rate = next_rate;
            }
        }
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
    fn bjt_bias_dependent_transit_charge_matches_spice_and_partials() {
        let circuit = parse_netlist(
            "Q1 c b e QMOD\n\
             .model QMOD NPN(IS=2e-16 NF=1.2 NR=1.1 VAF=40 VAR=70 IKF=2m IKR=5m TF=12n TR=35n XTF=2.5 VTF=0.6 ITF=80n)\n",
        )
        .expect("bias-dependent transit-time model parses");
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
        let tf_adjustment = model.xtf * (vbc / model.vtf).exp() * (cbe / (cbe + model.itf)).powi(2);
        assert!((charge.qbe - model.tf * cbe * (1.0 + tf_adjustment) / qb).abs() < 1e-25);
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
    fn bjt_bias_dependent_tf_and_tr_charge_integrate_for_both_polarities_and_rules() {
        let is = 1e-16;
        let beta_f = 100.0;
        let beta_r = 2.0;
        let resistance = 1e3;
        let h = 1e-9;

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
                for forward in [true, false] {
                    let tf = if forward { 10e-9 } else { 0.0 };
                    let xtf = if forward { 2.0 } else { 0.0 };
                    let vtf = 0.8;
                    let itf = 25e-6;
                    let tr = if forward { 0.0 } else { 10e-9 };
                    let (collector_v, emitter_v) = if forward {
                        (sign * 2.0, 0.0)
                    } else {
                        (0.0, sign * 2.0)
                    };
                    let netlist = format!(
                        "Vdrive drive 0 dc 0\nRdrive drive b {resistance}\n\
                         Vc c 0 dc {collector_v}\nVe e 0 dc {emitter_v}\nQ1 c b e QMOD\n\
                         .model QMOD {kind}(IS={is} BF={beta_f} BR={beta_r} TF={tf} TR={tr} XTF={xtf} VTF={vtf} ITF={itf})\n"
                    );
                    let circuit = parse_netlist(&netlist).expect("BJT transit circuit parses");
                    let mut solver =
                        Solver::new(circuit, integration).expect("BJT transit solver builds");
                    let base = solver.circuit().node("b").expect("base node");
                    let source = solver
                        .voltage_source_index("Vdrive")
                        .expect("base drive exists");
                    solver.set_voltage_source(source, sign);

                    let junctions = |v: f64| {
                        if forward {
                            (v, v - 2.0)
                        } else {
                            (v - 2.0, v)
                        }
                    };
                    let charge = |v: f64| {
                        let (vbe, vbc) = junctions(v);
                        let current = is * ((vbe / VT).exp() - 1.0);
                        if forward {
                            tf * current
                                * (1.0
                                    + xtf * (vbc / vtf).exp() * (current / (current + itf)).powi(2))
                        } else {
                            tr * is * ((vbc / VT).exp() - 1.0)
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

    /// A reverse-biased diode with default `CJO=0` and `TT=0` conducts its
    /// saturation current plus `GMIN*V` and has no stored charge.
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

    #[test]
    fn bjt_terminal_resistances_match_explicit_series_resistors_for_both_polarities() {
        let (rb, rc, re) = (31.0, 18.0, 7.0);

        for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
            let model = format!("IS=1e-15 BF=100 BR=4 RB={rb} RC={rc} RE={re}");
            let modelled = format!(
                "Vc c 0 dc {}\nVdrive drive 0 dc {}\nRdrive drive b 10k\n\
                 Rload e 0 10k\nQ1 c b e QMOD\n.model QMOD {kind}({model})\n",
                sign * 5.0,
                sign * 1.0,
            );
            let explicit = format!(
                "Vc c 0 dc {}\nVdrive drive 0 dc {}\nRdrive drive b 10k\n\
                 Rload e 0 10k\nRbase b qb {rb}\nRcollector c qc {rc}\n\
                 Remitter e qe {re}\nQ1 qc qb qe QMOD\n.model QMOD {kind}(IS=1e-15 BF=100 BR=4)\n",
                sign * 5.0,
                sign * 1.0,
            );
            let actual = build(&modelled);
            let expected = build(&explicit);

            for (actual_name, expected_name) in [
                ("c", "c"),
                ("b", "b"),
                ("e", "e"),
                ("Q1#collector", "qc"),
                ("Q1#base", "qb"),
                ("Q1#emitter", "qe"),
            ] {
                let actual_node = actual.circuit().node(actual_name).expect("node exists");
                let expected_node = expected
                    .circuit()
                    .node(expected_name)
                    .expect("reference node exists");
                let actual_voltage = actual.node_voltage(actual_node);
                let expected_voltage = expected.node_voltage(expected_node);
                assert!(
                    (actual_voltage - expected_voltage).abs() < 1e-8,
                    "{kind} {actual_name}: expected {expected_voltage} V, got {actual_voltage} V"
                );
            }

            for source in ["Vc", "Vdrive"] {
                let actual_current = source_current(&actual, source);
                let expected_current = source_current(&expected, source);
                assert!(
                    (actual_current - expected_current).abs() < 1e-10,
                    "{kind} {source}: expected {expected_current} A, got {actual_current} A"
                );
            }
        }
    }

    /// Ngspice's RBM/IRB law makes the base spreading resistance fall from RB
    /// toward RBM as the solved base current rises. This drives both an NPN and
    /// PNP through that transition and checks the voltage drop/current ratio
    /// against the closed-form resistance interpolation.
    #[test]
    fn bjt_base_resistance_rolls_off_with_bias_for_both_polarities() {
        let (rb, rbm, irb) = (50.0, 5.0, 100e-6);
        let expected_resistance = |base_current: f64| {
            let x = (base_current / irb).max(1e-9);
            let z = ((1.0 + 14.59025 * x).sqrt() - 1.0) / (2.4317 * x.sqrt());
            let tan_z = z.tan();
            rbm + 3.0 * (rb - rbm) * (tan_z - z) / (z * tan_z * tan_z)
        };

        for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
            let mut measured = Vec::new();
            for drive in [0.75, 0.82] {
                let netlist = format!(
                    "Vb b 0 dc {}\nVc c 0 dc {}\nQ1 c b 0 QMOD\n\
                     .model QMOD {kind}(IS=1e-15 BF=100 BR=1 VAF=0 VAR=0 RB={rb} RBM={rbm} IRB={irb})\n",
                    sign * drive,
                    sign * 5.0,
                );
                let solver = build(&netlist);
                let base = solver
                    .circuit()
                    .node("b")
                    .expect("external base node")
                    .expect("base is not ground");
                let internal_base = solver
                    .circuit()
                    .node("Q1#base")
                    .expect("internal base node")
                    .expect("internal base is not ground");
                let base_current = sign * source_current(&solver, "Vb");
                let base_drop = sign
                    * (solver.node_voltage(Some(base)) - solver.node_voltage(Some(internal_base)));
                let actual_resistance = base_drop / base_current;
                let expected = expected_resistance(base_current);
                assert!(base_current > 0.0, "{kind}: expected forward base current");
                assert!(
                    relative(actual_resistance, expected) < 1e-5,
                    "{kind} at {drive} V: expected RB={expected} Ω from Ib={base_current} A, got {actual_resistance} Ω"
                );
                measured.push(actual_resistance);
            }
            assert!(
                measured[1] < measured[0],
                "{kind}: base resistance should roll off with increasing base current, got {measured:?}"
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

    /// Explicit `NKF` changes the exponent of the Gummel–Poon high-current
    /// base-charge term. Exercise the resulting terminal currents through the
    /// MNA solver rather than only checking that the model card parsed.
    #[test]
    fn bjt_nkf_controls_high_injection_rolloff_for_both_polarities_and_directions() {
        let (is, bf, br, vaf, var, ikf, ikr, nkf) = (1e-16, 80.0, 4.0, 35.0, 50.0, 2e-4, 5e-4, 0.8);

        for (vbe, vbc) in [(0.75, -4.25), (-4.25, 0.75)] {
            let forward = (vbe / VT).exp();
            let reverse = (vbc / VT).exp();
            let q1 = 1.0 / (1.0 - vbc / vaf - vbe / var);
            let q2 = is * (forward - 1.0) / ikf + is * (reverse - 1.0) / ikr;
            let qb = q1 * (1.0 + (1.0 + 4.0 * q2).powf(nkf)) / 2.0;
            let ict = is * (forward - reverse) / qb;
            let ibe = (is / bf) * (forward - 1.0);
            let ibc = (is / br) * (reverse - 1.0);
            let ic = ict - ibc - device::GMIN * vbc;
            let ib = ibe + ibc + device::GMIN * (vbe + vbc);
            let ie = -(ic + ib);
            let direction = if vbe > 0.0 { "forward" } else { "reverse" };

            for (kind, sign) in [("NPN", 1.0), ("PNP", -1.0)] {
                let netlist = format!(
                    "Vb b 0 dc {}\nVc c 0 dc 0\nVe e 0 dc {}\nQ1 c b e QMOD\n\
                     .model QMOD {kind}(IS={is} BF={bf} BR={br} VAF={vaf} VAR={var} IKF={ikf} IKR={ikr} NKF={nkf})\n",
                    sign * vbc,
                    sign * (vbc - vbe),
                );
                let solver = build(&netlist);
                assert_eq!(solver.circuit().bjts[0].model.nkf, Some(nkf));
                let actual_ic = sign * source_current(&solver, "Vc");
                let actual_ib = sign * source_current(&solver, "Vb");
                let actual_ie = sign * source_current(&solver, "Ve");

                assert!(
                    relative(actual_ic, ic) < 1e-5,
                    "{kind} {direction} NKF={nkf}: expected Ic={ic} A, got {actual_ic} A"
                );
                assert!(
                    relative(actual_ib, ib) < 1e-5,
                    "{kind} {direction} NKF={nkf}: expected Ib={ib} A, got {actual_ib} A"
                );
                assert!(
                    relative(actual_ie, ie) < 1e-5,
                    "{kind} {direction} NKF={nkf}: expected Ie={ie} A, got {actual_ie} A"
                );
            }
        }
    }

    /// The same NKF-dependent base charge feeds forward transit charge, so a
    /// non-default exponent must change Qbe as well as the DC transport gain.
    #[test]
    fn bjt_nkf_changes_forward_transit_charge() {
        let (is, nf, nr, vaf, var, ikf, ikr, nkf, tf) =
            (2e-16, 1.2, 1.1, 40.0, 70.0, 2e-3, 5e-3, 0.8, 12e-9);
        let circuit = parse_netlist(&format!(
            "Q1 c b e QMOD\n.model QMOD NPN(IS={is} NF={nf} NR={nr} VAF={vaf} VAR={var} IKF={ikf} IKR={ikr} NKF={nkf} TF={tf})\n"
        ))
        .expect("NKF transit-charge model parses");
        let model = circuit.bjts[0].model;
        let (vbe, vbc) = (0.64, 0.35);
        let forward = (vbe / (nf * VT)).exp();
        let reverse = (vbc / (nr * VT)).exp();
        let cbe = is * (forward - 1.0);
        let cbc = is * (reverse - 1.0);
        let q1 = 1.0 / (1.0 - vbc / vaf - vbe / var);
        let q2 = cbe / ikf + cbc / ikr;
        let qb = q1 * (1.0 + (1.0 + 4.0 * q2).powf(nkf)) / 2.0;
        let expected = tf * cbe / qb;
        let charge = device::bjt_transit_charge(vbe, vbc, model);
        let actual = charge.qbe;
        assert!(
            (actual - expected).abs() < 1e-24 + expected.abs() * 1e-8,
            "NKF={nkf}: expected Qbe={expected} C, got {actual} C"
        );

        let delta = 1e-6;
        let plus_be = device::bjt_transit_charge(vbe + delta, vbc, model).qbe;
        let minus_be = device::bjt_transit_charge(vbe - delta, vbc, model).qbe;
        let plus_bc = device::bjt_transit_charge(vbe, vbc + delta, model).qbe;
        let minus_bc = device::bjt_transit_charge(vbe, vbc - delta, model).qbe;
        let finite_difference = |plus: f64, minus: f64| (plus - minus) / (2.0 * delta);
        for (actual, expected) in [
            (charge.dqbe_dvbe, finite_difference(plus_be, minus_be)),
            (charge.dqbe_dvbc, finite_difference(plus_bc, minus_bc)),
        ] {
            assert!(
                (actual - expected).abs() < expected.abs() * 2e-5 + 1e-24,
                "NKF transit-charge derivative {actual} differs from finite difference {expected}"
            );
        }
    }

    #[test]
    fn bjt_nkf_high_injection_partials_match_finite_difference() {
        let (is, nf, nr, vaf, var, ikf, ikr, nkf) = (2e-15, 1.1, 1.2, 42.0, 65.0, 2e-4, 5e-4, 0.8);
        let delta = 1e-7;
        let transport = |vbe: f64, vbc: f64| {
            let forward = (vbe / (nf * VT)).exp();
            let reverse = (vbc / (nr * VT)).exp();
            let q1 = 1.0 / (1.0 - vbc / vaf - vbe / var);
            let q2 = is * (forward - 1.0) / ikf + is * (reverse - 1.0) / ikr;
            let qb = q1 * (1.0 + (1.0 + 4.0 * q2).powf(nkf)) / 2.0;
            is * (forward - reverse) / qb
        };

        for (vbe, vbc) in [(0.68, -0.1), (-0.1, 0.62)] {
            let op = device::bjt_op_with_nkf(
                vbe,
                vbc,
                is,
                75.0,
                3.0,
                nf,
                nr,
                vaf,
                var,
                ikf,
                ikr,
                0.0,
                1.5,
                0.0,
                2.0,
                Some(nkf),
            );
            let expected_gif =
                (transport(vbe + delta, vbc) - transport(vbe - delta, vbc)) / (2.0 * delta);
            let expected_gir =
                -(transport(vbe, vbc + delta) - transport(vbe, vbc - delta)) / (2.0 * delta);
            assert!(
                relative(op.gif, expected_gif) < 1e-6,
                "NKF={nkf} gif: {} vs {expected_gif}",
                op.gif
            );
            assert!(
                relative(op.gir, expected_gir) < 1e-6,
                "NKF={nkf} gir: {} vs {expected_gir}",
                op.gir
            );
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

    /// The two intrinsic body junctions carry nonlinear depletion charge.
    /// Each is driven into the forward-bias continuation so CBD/CBS, PB, MJ
    /// and FC participate in the MNA transient for both device polarities and
    /// companion integration rules.
    #[test]
    fn mosfet_bulk_junction_charge_integrates_cbd_and_cbs() {
        let c0 = 10e-9;
        let pb = 0.75;
        let mj = 0.4;
        let fc = 0.5;
        let resistance = 1e3;
        let h = 100e-6;
        let input = 0.6;
        let charge = |voltage: f64| device::junction_depletion_charge(voltage, c0, pb, mj, fc).0;
        let solve_voltage =
            |step_input: f64, previous_charge: f64, previous_current: f64, rule: Integration| {
                let mut lo = 0.0;
                let mut hi = step_input + 0.2;
                for _ in 0..100 {
                    let mid = 0.5 * (lo + hi);
                    let cap_current = match rule {
                        Integration::BackwardEuler => (charge(mid) - previous_charge) / h,
                        Integration::Trapezoidal => {
                            2.0 * (charge(mid) - previous_charge) / h - previous_current
                        }
                    };
                    if (step_input - mid) / resistance > cap_current {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                0.5 * (lo + hi)
            };

        for (parameter, drain, source) in [("CBD", "x", "0"), ("CBS", "0", "x")] {
            for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
                let (cbd, cbs) = if parameter == "CBD" {
                    (c0, 0.0)
                } else {
                    (0.0, c0)
                };
                let vto = if kind == "NMOS" { 1.0 } else { -1.0 };
                let netlist = format!(
                    "V1 in 0 dc {}\nR1 in x {resistance}\nM1 {drain} 0 {source} 0 MMOD\n\
                     .model MMOD {kind}(VTO={vto} KP=20u IS=1e-30 CBD={cbd} CBS={cbs} \
                     PB={pb} MJ={mj} FC={fc})\n",
                    -sign * input
                );
                let circuit = parse_netlist(&netlist).expect("MOSFET bulk capacitance parses");
                let model = circuit.mosfets[0].model;
                assert_eq!(model.cbd, cbd);
                assert_eq!(model.cbs, cbs);
                assert_eq!(model.pb, pb);
                assert_eq!(model.mj, mj);
                assert_eq!(model.fc, fc);

                for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                    let mut solver = Solver::new(circuit.clone(), integration)
                        .expect("MOSFET bulk capacitance solver builds");
                    let x = solver.circuit().node("x").expect("junction node");
                    let source_index = solver
                        .voltage_source_index("V1")
                        .expect("driving source exists");
                    let initial = -sign * solver.node_voltage(x);
                    assert!((initial - input).abs() < 1e-8);
                    solver
                        .advance(h)
                        .expect("initial bulk charge carries into transient");
                    let held = -sign * solver.node_voltage(x);
                    assert!(
                        (held - initial).abs() < 1e-8,
                        "an operating-point junction charge must not create a startup impulse"
                    );

                    let next_input = input + 0.3;
                    solver.set_voltage_source(source_index, -sign * next_input);

                    let mut previous_charge = charge(initial);
                    let mut previous_current = 0.0;
                    for step in 0..2 {
                        let rule = if step == 0 {
                            Integration::BackwardEuler
                        } else {
                            integration
                        };
                        let expected =
                            solve_voltage(next_input, previous_charge, previous_current, rule);
                        if step == 0 {
                            assert!(
                                expected > fc * pb,
                                "first step must exercise the FC continuation"
                            );
                        }
                        solver.advance(h).expect("MOSFET bulk charge step solves");
                        let actual = -sign * solver.node_voltage(x);
                        assert!(
                            (actual - expected).abs() < 1e-8,
                            "{kind} {parameter} {integration:?} step {step}: expected {expected} V, got {actual} V"
                        );

                        let next_charge = charge(expected);
                        previous_current = match rule {
                            Integration::BackwardEuler => (next_charge - previous_charge) / h,
                            Integration::Trapezoidal => {
                                2.0 * (next_charge - previous_charge) / h - previous_current
                            }
                        };
                        previous_charge = next_charge;
                    }
                }
            }
        }
    }

    /// MOS level 1 can derive bottom depletion capacitance from junction area
    /// (`CJ·AD/AS`) and add sidewall charge from perimeter (`CJSW·PD/PS`). The
    /// two grading coefficients are independent and both junctions are
    /// exercised through the transient MNA companion for NMOS/PMOS and both
    /// integration rules.
    #[test]
    fn mosfet_bulk_junction_geometry_integrates_area_and_sidewall_charge() {
        let cj = 2e-3;
        let cjsw = 5e-7;
        let pb = 0.75;
        let mj = 0.4;
        let mjsw = 0.2;
        let fc = 0.5;
        let resistance = 1e3;
        let h = 100e-6;
        let input = 0.6;
        let charge = |voltage: f64, area: f64, perimeter: f64| {
            let bottom = device::junction_depletion_charge(voltage, cj * area, pb, mj, fc).0;
            let sidewall =
                device::junction_depletion_charge(voltage, cjsw * perimeter, pb, mjsw, fc).0;
            bottom + sidewall
        };
        let solve_voltage = |step_input: f64,
                             previous_charge: f64,
                             previous_current: f64,
                             area: f64,
                             perimeter: f64,
                             rule: Integration| {
            let mut lo = 0.0;
            let mut hi = step_input + 0.2;
            for _ in 0..100 {
                let mid = 0.5 * (lo + hi);
                let cap_current = match rule {
                    Integration::BackwardEuler => {
                        (charge(mid, area, perimeter) - previous_charge) / h
                    }
                    Integration::Trapezoidal => {
                        2.0 * (charge(mid, area, perimeter) - previous_charge) / h
                            - previous_current
                    }
                };
                if (step_input - mid) / resistance > cap_current {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            0.5 * (lo + hi)
        };

        for (terminal, drain, source, area, perimeter) in [
            ("drain", "x", "0", 2e-6, 4e-3),
            ("source", "0", "x", 3e-6, 5e-3),
        ] {
            for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
                let vto = if kind == "NMOS" { 1.0 } else { -1.0 };
                let netlist = format!(
                    "V1 in 0 dc {}\nR1 in x {resistance}\n\
                     M1 {drain} 0 {source} 0 MMOD AD=2e-6 AS=3e-6 PD=4e-3 PS=5e-3\n\
                     .model MMOD {kind}(VTO={vto} KP=20u IS=1e-30 CJ={cj} CJSW={cjsw} \
                     PB={pb} MJ={mj} MJSW={mjsw} FC={fc})\n",
                    -sign * input
                );
                let circuit = parse_netlist(&netlist).expect("MOSFET geometry capacitance parses");
                for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                    let mut solver = Solver::new(circuit.clone(), integration)
                        .expect("MOSFET geometry capacitance solver builds");
                    let x = solver.circuit().node("x").expect("junction node");
                    let source_index = solver
                        .voltage_source_index("V1")
                        .expect("driving source exists");
                    let initial = -sign * solver.node_voltage(x);
                    assert!((initial - input).abs() < 1e-8);
                    solver
                        .advance(h)
                        .expect("initial geometry charge carries into transient");
                    let held = -sign * solver.node_voltage(x);
                    assert!(
                        (held - initial).abs() < 1e-8,
                        "operating-point geometry charge must not create a startup impulse"
                    );

                    let next_input = input + 0.3;
                    solver.set_voltage_source(source_index, -sign * next_input);
                    let mut previous_charge = charge(initial, area, perimeter);
                    let mut previous_current = 0.0;
                    for step in 0..2 {
                        let rule = if step == 0 {
                            Integration::BackwardEuler
                        } else {
                            integration
                        };
                        let expected = solve_voltage(
                            next_input,
                            previous_charge,
                            previous_current,
                            area,
                            perimeter,
                            rule,
                        );
                        if step == 0 {
                            assert!(
                                expected > fc * pb,
                                "{terminal} first step must exercise FC continuation"
                            );
                        }
                        solver
                            .advance(h)
                            .expect("MOSFET geometry charge step solves");
                        let actual = -sign * solver.node_voltage(x);
                        assert!(
                            (actual - expected).abs() < 1e-8,
                            "{kind} {terminal} {integration:?} step {step}: \
                             expected {expected} V, got {actual} V"
                        );
                        let next_charge = charge(expected, area, perimeter);
                        previous_current = match rule {
                            Integration::BackwardEuler => (next_charge - previous_charge) / h,
                            Integration::Trapezoidal => {
                                2.0 * (next_charge - previous_charge) / h - previous_current
                            }
                        };
                        previous_charge = next_charge;
                    }
                }
            }
        }
    }

    /// Meyer capacitance halves cover cutoff, depletion, linear and saturated
    /// channel bias. These are the endpoint values averaged by the transient
    /// charge companion, not fixed overlap capacitances.
    #[test]
    fn mosfet_tox_uses_level1_meyer_capacitance_regions() {
        let cox = 12e-12;
        let phi = 0.6;
        let cases = [
            ("cutoff", 0.0, 0.0, [0.0, 0.0, 6e-12]),
            ("depletion", 0.8, 0.0, [1e-12, 1e-12, 2e-12]),
            ("linear", 1.5, 0.2, [3.4375e-12, 2.4375e-12, 0.0]),
            ("saturation", 1.5, 0.6, [4e-12, 0.0, 0.0]),
        ];
        for (region, vgs, vds, expected) in cases {
            let actual = device::mos_meyer_capacitance_halves(vgs, vds, 0.0, 1.0, 0.0, phi, cox);
            for junction in 0..3 {
                assert!(
                    (actual[junction] - expected[junction]).abs() < 1e-26,
                    "{region} Meyer capacitance {junction}: expected {}, got {}",
                    expected[junction],
                    actual[junction]
                );
            }
        }
        assert_eq!(
            device::mos_meyer_capacitance_halves(1.5, 0.6, 0.0, 1.0, 0.0, phi, 0.0),
            [0.0; 3],
            "without TOX, intrinsic channel capacitance is disabled"
        );
        for tox in ["0", "-1n"] {
            let invalid = format!("M1 d g s b MMOD\n.model MMOD NMOS(TOX={tox})\n");
            let error = parse_netlist(&invalid)
                .expect_err("non-positive MOS oxide thickness must be rejected")
                .to_string();
            assert!(error.contains("TOX"), "unexpected TOX error: {error}");
        }
    }

    #[test]
    fn mosfet_ward_dutton_channel_charge_conserves_and_has_analytic_partials() {
        let cases = [
            ("zero-drain-bias", 1.6, 0.0, -0.1),
            ("linear", 1.6, 0.2, -0.1),
            ("saturated", 1.8, 1.0, -0.1),
            ("reverse", 1.6, -0.2, -0.1),
        ];
        for (kind, polarity) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            let vto = polarity;
            let circuit = parse_netlist(&format!(
                "M1 d g s b MOD w=2u l=1u\n.model MOD {kind}(VTO={vto} TOX=10n GAMMA=0.35 PHI=0.7 CHARGEMOD=WARD_DUTTON)\n"
            ))
            .expect("Ward-Dutton model parses");
            let mosfet = &circuit.mosfets[0];
            for (region, vgs, vds, vbs) in cases {
                let evaluate = |vgs, vds, vbs| {
                    device::mos_ward_dutton_channel_charge(mosfet, vgs, vds, vbs - vds, vbs)
                };
                let charge = evaluate(vgs, vds, vbs);
                assert!(
                    charge.q.iter().sum::<f64>().abs() < 1e-25,
                    "{kind} {region}: terminal charges do not sum to zero: {:?}",
                    charge.q
                );
                let reversed = vds < 0.0;
                let (vgs_eff, vds_eff, vbs_eff) = if reversed {
                    (vgs - vds, -vds, vbs - vds)
                } else {
                    (vgs, vds, vbs)
                };
                let sqrt_phi = 0.7_f64.sqrt();
                let body_root = if vbs_eff <= 0.0 {
                    (0.7 - vbs_eff).sqrt()
                } else {
                    (sqrt_phi - vbs_eff / (2.0 * sqrt_phi)).max(0.0)
                };
                let overdrive = vgs_eff - (1.0 + 0.35 * (body_root - sqrt_phi));
                let channel_drop = vds_eff.clamp(0.0, overdrive);
                let denominator = overdrive * channel_drop - 0.5 * channel_drop.powi(2);
                let cox = 3.453_133e-11 * mosfet.w * mosfet.l / 10e-9;
                let intervals = 40_000;
                let mut total_integral = 0.0;
                let mut drain_integral = 0.0;
                for point in 0..=intervals {
                    let x = point as f64 / intervals as f64;
                    let channel_charge_density = (overdrive.powi(2) - 2.0 * x * denominator).sqrt();
                    let weight = if point == 0 || point == intervals {
                        1.0
                    } else if point % 2 == 0 {
                        2.0
                    } else {
                        4.0
                    };
                    total_integral += weight * channel_charge_density;
                    drain_integral += weight * x * channel_charge_density;
                }
                let scale = 1.0 / (3.0 * intervals as f64);
                let expected_total = cox * total_integral * scale;
                let expected_drain = cox * drain_integral * scale;
                let physical_drain_charge = if reversed { charge.q[2] } else { charge.q[1] };
                assert!(
                    (charge.q[0] - polarity * expected_total).abs()
                        < expected_total.abs() * 1e-6,
                    "{kind} {region}: integrated gate charge differs from the charge-sheet quadrature"
                );
                assert!(
                    (physical_drain_charge + polarity * expected_drain).abs()
                        < expected_drain.abs() * 1e-6,
                    "{kind} {region}: drain partition differs from the Ward-Dutton quadrature"
                );
                for control in 0..3 {
                    let jacobian_sum = charge.dq_dv.iter().map(|row| row[control]).sum::<f64>();
                    assert!(
                        jacobian_sum.abs() < 1e-25,
                        "{kind} {region}: Jacobian column {control} does not conserve charge: {jacobian_sum}"
                    );
                }

                let delta = 1e-5;
                for terminal in 0..4 {
                    for control in 0..3 {
                        let mut plus = [vgs, vds, vbs];
                        let mut minus = [vgs, vds, vbs];
                        plus[control] += delta;
                        minus[control] -= delta;
                        let plus = evaluate(plus[0], plus[1], plus[2]).q[terminal];
                        let minus = evaluate(minus[0], minus[1], minus[2]).q[terminal];
                        let finite_difference = if vds == 0.0 && control == 1 {
                            (plus - charge.q[terminal]) / delta * polarity
                        } else {
                            (plus - minus) / (2.0 * delta) * polarity
                        };
                        let analytic = charge.dq_dv[terminal][control];
                        assert!(
                            (analytic - finite_difference).abs()
                                < finite_difference.abs() * 3e-5 + 1e-24,
                            "{kind} {region}, terminal {terminal}, control {control}: analytic {analytic}, finite difference {finite_difference}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn mosfet_ward_dutton_charge_drives_a_loaded_terminal_for_both_polarities_and_directions() {
        let width = 2e-6;
        let length = 1e-6;
        let tox = 10e-9;
        let resistance = 1e3;
        let h = 10e-9;
        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            for reverse in [false, true] {
                for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                    let (drain, source, sensed_terminal) = if reverse {
                        ("0", "x", 2)
                    } else {
                        ("x", "0", 1)
                    };
                    let netlist = format!(
                        "Vg g 0 dc {}\nRload x 0 {resistance}\nM1 {drain} g {source} 0 MOD w={width} l={length}\n.model MOD {kind}(VTO={} KP=1e-30 IS=1e-30 TOX={tox} CHARGEMOD=WARD_DUTTON)\n",
                        sign * 1.5,
                        sign
                    );
                    let circuit = parse_netlist(&netlist).expect("MOS channel charge parses");
                    let mosfet = circuit.mosfets[0].clone();
                    let mut solver = Solver::new(circuit, integration)
                        .expect("MOS channel charge solver builds");
                    let x = solver.circuit().node("x").expect("loaded terminal node");
                    let gate = solver
                        .voltage_source_index("Vg")
                        .expect("gate drive exists");
                    solver
                        .advance(h)
                        .expect("initially biased channel step solves");
                    let initial_x = solver.node_voltage(x);
                    assert!(initial_x.abs() < 1e-10);

                    let old_gate = sign * 1.5;
                    let new_gate = sign * 1.6;
                    let terminal_charge = |node_voltage: f64, gate_voltage: f64| {
                        let (vgs, vds, vbd, vbs) = if reverse {
                            (
                                sign * (gate_voltage - node_voltage),
                                sign * -node_voltage,
                                0.0,
                                sign * -node_voltage,
                            )
                        } else {
                            (
                                sign * gate_voltage,
                                sign * node_voltage,
                                sign * -node_voltage,
                                0.0,
                            )
                        };
                        device::mos_ward_dutton_channel_charge(&mosfet, vgs, vds, vbd, vbs).q
                            [sensed_terminal]
                    };
                    let old_charge = terminal_charge(initial_x, old_gate);
                    let residual = |voltage: f64| {
                        voltage / resistance + (terminal_charge(voltage, new_gate) - old_charge) / h
                    };
                    let mut low = -0.2;
                    let mut high = 0.2;
                    assert!(residual(low) * residual(high) < 0.0);
                    for _ in 0..80 {
                        let middle = 0.5 * (low + high);
                        if residual(middle) < 0.0 {
                            low = middle;
                        } else {
                            high = middle;
                        }
                    }
                    let expected = 0.5 * (low + high);

                    solver.set_voltage_source(gate, new_gate);
                    solver
                        .advance(h)
                        .expect("gate step through Ward-Dutton charge solves");
                    let actual = solver.node_voltage(x);
                    assert!(
                        (actual - expected).abs() < 1e-8,
                        "{kind} reverse={reverse} {integration:?}: expected {expected} V, got {actual} V"
                    );
                }
            }
        }
    }

    /// `TOX` produces an intrinsic gate-bulk capacitance in cutoff. A gate
    /// step charges that branch through the resistor for NMOS and PMOS, and
    /// the held bias exercises the selected trapezoidal history after the
    /// first BE step. The operating-point charge must not create a startup
    /// impulse.
    #[test]
    fn mosfet_tox_integrates_channel_charge_for_both_polarities_and_rules() {
        let width = 2e-6;
        let length = 1e-6;
        let tox = 10e-9;
        let cox = 3.453_133e-11 * width * length / tox;
        let resistance = 1e3;
        let h = 10e-12;
        let gate_step = 0.1;
        let ratio = resistance * cox / h;

        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            let vto = sign;
            let netlist = format!(
                "Vg g 0 dc {}\nR1 x 0 {resistance}\n\
                 M1 0 g 0 x MMOD w={width} l={length}\n\
                 .model MMOD {kind}(VTO={vto} KP=20u IS=1e-30 PHI=0.6 TOX={tox})\n",
                sign * gate_step
            );
            for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
                let mut solver = Solver::new(
                    parse_netlist(&netlist).expect("MOS TOX model parses"),
                    integration,
                )
                .expect("MOS TOX solver builds");
                let x = solver.circuit().node("x").expect("bulk node");
                let source = solver.voltage_source_index("Vg").expect("gate source");

                solver.advance(h).expect("initially biased TOX step solves");
                assert!(
                    solver.node_voltage(x).abs() < 1e-10,
                    "{kind} {integration:?}: operating-point channel charge must not create a startup impulse"
                );

                solver.set_voltage_source(source, sign * 2.0 * gate_step);
                solver.advance(h).expect("gate charge step solves");
                let first = solver.node_voltage(x);
                let expected_first = sign * gate_step * ratio / (1.0 + ratio);
                assert!(
                    (first - expected_first).abs() < 1e-8,
                    "{kind} {integration:?} first step: expected {expected_first} V, got {first} V"
                );

                solver.advance(h).expect("held gate charge step solves");
                let actual_second = solver.node_voltage(x);
                let expected_second = match integration {
                    Integration::BackwardEuler => ratio * expected_first / (1.0 + ratio),
                    Integration::Trapezoidal => {
                        ratio * (3.0 * expected_first - sign * gate_step) / (1.0 + 2.0 * ratio)
                    }
                };
                assert!(
                    (actual_second - expected_second).abs() < 1e-8,
                    "{kind} {integration:?} second step: expected {expected_second} V, got {actual_second} V"
                );
            }
        }
    }

    /// With V(d) below V(s), ngspice's Meyer G-S and G-D capacitances swap
    /// their physical terminals. Keep the transistor current negligible and
    /// drive a small gate step so the resistor response identifies which
    /// capacitance the MNA stamp placed on the physical source pin.
    #[test]
    fn mosfet_tox_maps_reversed_channel_capacitance_to_physical_pins() {
        let width = 2e-6;
        let length = 1e-6;
        let tox = 10e-9;
        let cox = 3.453_133e-11 * width * length / tox;
        let resistance = 1e3;
        let h = 10e-12;
        let gate_step = 1e-3;
        let vgs_eff = 1.5;
        let vds_eff = 0.2;
        let vgst = vgs_eff - 1.0;
        let vdsat = vgst;
        let vddif = 2.0 * vdsat - vds_eff;
        let cgd_half = cox * (1.0 - vdsat * vdsat / (vddif * vddif)) / 3.0;
        let capacitance = 2.0 * cgd_half;
        let ratio = resistance * capacitance / h;
        let netlist = format!(
            "Vg g 0 dc {vgs_eff}\nVbias bias 0 dc 0.2\n\
             R1 x bias {resistance}\nM1 0 g x 0 MMOD w={width} l={length}\n\
             .model MMOD NMOS(VTO=1 KP=1e-30 IS=1e-30 TOX={tox})\n"
        );
        let mut solver = Solver::new(
            parse_netlist(&netlist).expect("reversed MOS TOX model parses"),
            Integration::BackwardEuler,
        )
        .expect("reversed MOS TOX solver builds");
        let x = solver.circuit().node("x").expect("physical source node");
        let gate = solver.voltage_source_index("Vg").expect("gate source");
        solver
            .advance(h)
            .expect("reverse-biased channel step solves");
        let initial = solver.node_voltage(x);
        assert!((initial - 0.2).abs() < 1e-8);

        solver.set_voltage_source(gate, vgs_eff + gate_step);
        solver
            .advance(h)
            .expect("reversed channel gate step solves");
        let actual = solver.node_voltage(x) - initial;
        let expected = gate_step * ratio / (1.0 + ratio);
        assert!(
            (actual - expected).abs() < 1e-7,
            "reversed channel should map effective Cgd to physical G-S: expected {expected} V, got {actual} V"
        );
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

    /// The Parker–Skellern B fit changes both JFET channel regions. Its
    /// closed-form current and analytic partials are checked through the
    /// nonlinear MNA solve for both channel polarities.
    #[test]
    fn jfet_parker_skellern_b_matches_both_regions_and_polarities() {
        let beta = 2e-4;
        let lambda = 0.02;
        let is = 1e-30;
        let b = 1.4;
        let pb = 0.75;
        let vto = -2.0;
        let bfac = (1.0 - b) / (pb - vto);
        let cases = [(0.0, 0.4), (0.0, 5.0)];

        let channel_current = |vgs: f64, vds: f64| {
            let vgst = vgs - vto;
            let beta_p = beta * (1.0 + lambda * vds);
            if vgst <= 0.0 {
                0.0
            } else if vgst >= vds {
                let apart = 2.0 * b + 3.0 * bfac * (vgst - vds);
                beta_p * vds * (vds * (bfac * vds - b) + vgst * apart)
            } else {
                beta_p * vgst * vgst * (b + vgst * bfac)
            }
        };

        for (vgs, vds) in cases {
            let op = device::jfet_op_with_b(vgs, vds, vto, beta, lambda, b, pb);
            let dv = 1e-6;
            let gm = (channel_current(vgs + dv, vds) - channel_current(vgs - dv, vds)) / (2.0 * dv);
            let gds =
                (channel_current(vgs, vds + dv) - channel_current(vgs, vds - dv)) / (2.0 * dv);
            assert!(relative(op.gm, gm) < 1e-8, "Vgs={vgs} Vds={vds}: gm");
            assert!(relative(op.gds, gds) < 1e-8, "Vgs={vgs} Vds={vds}: gds");

            for (kind, sign) in [("NJF", 1.0), ("PJF", -1.0)] {
                let netlist = format!(
                    "Vg g 0 dc {}\nVd d 0 dc {}\nVs s 0 dc 0\nJ1 d g s JMOD\n.model JMOD {kind}(VTO={} BETA={beta} LAMBDA={lambda} IS={is} B={b} PB={pb})\n",
                    sign * vgs,
                    sign * vds,
                    sign * vto
                );
                let measured = sign * source_current(&build(&netlist), "Vd");
                let gate_drain_voltage = vgs - vds;
                let expected = channel_current(vgs, vds) + device::GMIN * vds
                    - device::diode_op(gate_drain_voltage, is, 1.0).id;
                assert!(
                    relative(measured, expected) < 2e-5,
                    "{kind} Vgs={vgs} Vds={vds}: expected {expected} A, got {measured} A"
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

    /// MOSFET `RD` and `RS` sit outside the intrinsic channel terminals. The
    /// terminal voltage drops are checked against the solved channel current,
    /// for both MOS polarities, exercising the model parameters in MNA.
    #[test]
    fn mosfet_terminal_resistances_drop_voltage_at_the_intrinsic_nodes() {
        let resistance = 100.0;
        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            for (parameter, terminal) in [("RD", "drain"), ("RS", "source")] {
                let netlist = format!(
                    "Vg g 0 dc {}\nVd d 0 dc {}\nVs s 0 dc 0\n\
                     M1 d g s 0 MMOD\n.model MMOD {kind}(VTO={} KP=20u LAMBDA=0 \
                     IS=1e-30 W=100u L=10u {parameter}={resistance})\n",
                    sign * 3.0,
                    sign * 0.4,
                    sign
                );
                let solver = build(&netlist);
                let external = solver
                    .circuit()
                    .node(if terminal == "drain" { "d" } else { "s" })
                    .expect("external terminal exists");
                let internal_name = format!("M1#{terminal}");
                let internal = solver
                    .circuit()
                    .node(&internal_name)
                    .expect("nonzero terminal resistance creates an internal node")
                    .expect("internal terminal is not ground");
                let current = sign * source_current(&solver, "Vd");
                assert!(current > 1e-6, "{kind} {parameter} current is {current} A");

                let (external_voltage, internal_voltage) = (
                    solver.node_voltage(external),
                    solver.node_voltage(Some(internal)),
                );
                let actual_drop = if parameter == "RD" {
                    sign * (external_voltage - internal_voltage)
                } else {
                    sign * (internal_voltage - external_voltage)
                };
                assert!(
                    relative(actual_drop, current * resistance) < 1e-6,
                    "{kind} {parameter} should drop {} V, got {actual_drop} V",
                    current * resistance
                );
            }
        }
    }

    /// The model's sheet resistance is multiplied by each instance's drain
    /// and source square counts, and those effective resistors participate in
    /// the solved channel path for both MOS polarities.
    #[test]
    fn mosfet_sheet_resistance_drops_voltage_by_instance_square_count() {
        let sheet_resistance = 25.0;
        let squares = [("NRD", "drain", 2.0), ("NRS", "source", 4.0)];
        for (kind, sign) in [("NMOS", 1.0), ("PMOS", -1.0)] {
            for (parameter, terminal, count) in squares {
                let resistance = sheet_resistance * count;
                let netlist = format!(
                    "Vg g 0 dc {}\nVd d 0 dc {}\nVs s 0 dc 0\n\
                     M1 d g s 0 MMOD {parameter}={count}\n\
                     .model MMOD {kind}(VTO={} KP=20u LAMBDA=0 IS=1e-30 \
                     W=100u L=10u RSH={sheet_resistance})\n",
                    sign * 3.0,
                    sign * 0.4,
                    sign
                );
                let solver = build(&netlist);
                let external = solver
                    .circuit()
                    .node(if terminal == "drain" { "d" } else { "s" })
                    .expect("external terminal exists");
                let internal_name = format!("M1#{terminal}");
                let internal = solver
                    .circuit()
                    .node(&internal_name)
                    .expect("nonzero sheet resistance creates an internal terminal")
                    .expect("internal terminal is not ground");
                let current = sign * source_current(&solver, "Vd");
                assert!(current > 1e-6, "{kind} {parameter} current is {current} A");

                let (external_voltage, internal_voltage) = (
                    solver.node_voltage(external),
                    solver.node_voltage(Some(internal)),
                );
                let actual_drop = if terminal == "drain" {
                    sign * (external_voltage - internal_voltage)
                } else {
                    sign * (internal_voltage - external_voltage)
                };
                assert!(
                    relative(actual_drop, current * resistance) < 1e-6,
                    "{kind} {parameter} should drop {} V, got {actual_drop} V",
                    current * resistance
                );
            }
        }
    }

    /// JFET `RD` and `RS` sit outside the intrinsic channel terminals. The
    /// terminal voltage drops are checked against the source current, for both
    /// channel polarities, so these model-card parameters are exercised by the
    /// MNA solver rather than only parsed.
    #[test]
    fn jfet_terminal_resistances_drop_voltage_at_the_intrinsic_nodes() {
        let resistance = 100.0;
        for (kind, sign, vto) in [("NJF", 1.0, -2.0), ("PJF", -1.0, 2.0)] {
            for (parameter, terminal, signed_drop) in [
                ("RD", "drain", "external - intrinsic"),
                ("RS", "source", "intrinsic - external"),
            ] {
                let netlist = format!(
                    "Vg g 0 dc 0\nVd d 0 dc {}\nVs s 0 dc 0\n\
                     J1 d g s JMOD\n.model JMOD {kind}(VTO={vto} BETA=200u \
                     LAMBDA=0.02 IS=1e-30 {parameter}={resistance})\n",
                    sign * 0.4
                );
                let solver = build(&netlist);
                let external = solver
                    .circuit()
                    .node(if terminal == "drain" { "d" } else { "s" })
                    .expect("external terminal exists");
                let internal_name = format!("J1#{terminal}");
                let internal = solver
                    .circuit()
                    .node(&internal_name)
                    .expect("nonzero terminal resistance creates an internal node")
                    .expect("internal terminal is not ground");
                let current = sign * source_current(&solver, "Vd");
                assert!(current > 1e-6, "{kind} {parameter} current is {current} A");

                let (external_voltage, internal_voltage) = (
                    solver.node_voltage(external),
                    solver.node_voltage(Some(internal)),
                );
                let actual_drop = if parameter == "RD" {
                    sign * (external_voltage - internal_voltage)
                } else {
                    sign * (internal_voltage - external_voltage)
                };
                assert!(
                    relative(actual_drop, current * resistance) < 1e-6,
                    "{kind} {parameter} {signed_drop} should be {} V, got {actual_drop} V",
                    current * resistance
                );
            }
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
