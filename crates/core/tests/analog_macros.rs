// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Hand calculations, grammar and goldens for the dependent sources, the
//! op-amp and comparator macros, and diode breakdown.
//!
//! `analog_ngspice_macros.rs` holds the differential against ngspice. This file
//! holds what a differential cannot: closed forms derived without the solver,
//! the netlist errors, the branch numbering the adapter's `i(...)` probes rely
//! on, and raw-bit goldens that catch platform drift (`libm` not being used, a
//! reordered sum) the way `analog_devices.rs` does for the older devices.

mod analog {
    use labwired_core::analog::{
        device, parse_netlist, AnalogError, Integration, Solver, MACRO_GAIN_RESISTANCE,
    };

    fn build(netlist: &str) -> Solver {
        let circuit = parse_netlist(netlist).unwrap_or_else(|error| panic!("{error}"));
        Solver::new(circuit, Integration::BackwardEuler).unwrap_or_else(|error| panic!("{error}"))
    }

    fn v(solver: &Solver, node: &str) -> f64 {
        solver.node_voltage(solver.circuit().node(node).expect("node exists"))
    }

    fn relative(actual: f64, expected: f64) -> f64 {
        ((actual - expected) / expected).abs()
    }

    fn macro_input_after_one_tau(x_line: &str, model_line: &str) -> f64 {
        let resistance = 100e3;
        let capacitance = 10e-12;
        let tau = resistance * capacitance;
        let mut solver = build(&format!(
            "Vcc vcc 0 dc 5\nVdrive src 0 dc 0\nRdrive src inp {resistance}\n\
             {x_line}\n{model_line}\n"
        ));
        let source = solver.voltage_source_index("Vdrive").expect("drive source");
        solver.set_voltage_source(source, 1.0);
        let steps = 10_000u32;
        for _ in 0..steps {
            solver.advance(tau / f64::from(steps)).unwrap();
        }
        v(&solver, "inp")
    }

    // -----------------------------------------------------------------------
    // Linear dependent sources
    // -----------------------------------------------------------------------

    #[test]
    fn vcvs_and_vccs_match_their_defining_equations() {
        let solver = build(
            "Vin in 0 dc 0.3\n\
             E1 a 0 in 0 7.5\n\
             Ra a 0 1k\n\
             G1 0 b in 0 2m\n\
             Rb b 0 2k\n",
        );
        // E: v(a) = 7.5 · 0.3 exactly, whatever loads it.
        assert!(relative(v(&solver, "a"), 2.25) < 1e-12);
        // G: 2 mS · 0.3 V = 0.6 mA flows from 0 through the source into b.
        assert!(relative(v(&solver, "b"), 0.6e-3 * 2e3) < 1e-12);
        assert!(
            !solver.is_nonlinear(),
            "linear sources keep the linear path"
        );
    }

    #[test]
    fn cccs_and_ccvs_read_the_named_branch_current() {
        // Vsense carries 1 mA (5 V over 5 kΩ). F mirrors it ×4 into 1 kΩ; H
        // turns it into a voltage through 2.5 kΩ of transresistance.
        let solver = build(
            "Vs s 0 dc 5\n\
             Vsense s t dc 0\n\
             R1 t 0 5k\n\
             F1 0 f Vsense 4\n\
             Rf f 0 1k\n\
             H1 h 0 Vsense 2.5k\n\
             Rh h 0 10k\n",
        );
        assert!(relative(v(&solver, "f"), 4e-3 * 1e3) < 1e-12);
        assert!(relative(v(&solver, "h"), 1e-3 * 2.5e3) < 1e-12);
    }

    #[test]
    fn dependent_sources_are_numbered_after_every_v_and_l() {
        let circuit = parse_netlist(
            "E1 a 0 b 0 2\n\
             V1 b 0 dc 1\n\
             H1 c 0 V1 1k\n\
             L1 a c 1m\n\
             Rc c 0 1k\n",
        )
        .expect("parses");
        // V, then L, then E, then H — so adding a dependent source never
        // renumbers a probe that already existed.
        assert_eq!(circuit.branch_index("V1"), Some(0));
        assert_eq!(circuit.branch_index("L1"), Some(1));
        assert_eq!(circuit.branch_index("e1"), Some(2));
        assert_eq!(circuit.branch_index("H1"), Some(3));
        assert_eq!(circuit.branch_name(2), "E1");
        assert_eq!(circuit.branch_name(3), "H1");
        assert_eq!(circuit.branch_count(), 4);
    }

    #[test]
    fn dependent_source_errors_are_named() {
        let error = parse_netlist("R1 a 0 1k\nF1 0 b R1 3\nRb b 0 1k\n").unwrap_err();
        assert!(
            error.to_string().contains("carries no branch current"),
            "{error}"
        );
        assert_eq!(error.line(), Some(2));

        let error = parse_netlist("E1 a 0 b 0\n").unwrap_err();
        assert!(
            error.to_string().contains("E<name> n+ n- nc+ nc- <gain>"),
            "{error}"
        );

        for line in [
            "E1 a 0 POLY(1) b 0 0 1",
            "G1 a 0 VALUE={V(b)*2}",
            "H1 a 0 TABLE {x}",
        ] {
            let error = parse_netlist(&format!("{line}\n")).unwrap_err();
            assert!(
                matches!(error, AnalogError::Unsupported { .. }),
                "`{line}` must point at ngspice, got {error}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Op-amp
    // -----------------------------------------------------------------------

    /// Inverting amplifier at DC, solved by hand from the macro's definition:
    /// the gain node is `p = AOL·(v+ − v−)`, the output is `p` behind `ROUT`.
    #[test]
    fn opamp_inverting_gain_is_the_finite_gain_closed_form() {
        let (aol, rout, rin, rf, rl, vin) = (1e4, 200.0, 10e3, 47e3, 2e3, 0.1);
        let solver = build(&format!(
            "Vcc vcc 0 dc 15\nVee vee 0 dc -15\nVin in 0 dc {vin}\n\
             Rin in n {rin}\nRf n out {rf}\nRl out 0 {rl}\n\
             X1 0 n vcc vee out AMP\n\
             .model AMP OPAMP(AOL={aol} GBW=1meg ROUT={rout})\n"
        ));
        // Unknowns n, out, p with p = −aol·n:
        //   (vin − n)/rin + (out − n)/rf = 0
        //   (p − out)/rout = (out − n)/rf + out/rl
        // Eliminate: n = (vin/rin + out/rf) / (1/rin + 1/rf) = a·vin + b·out,
        // then (−aol·n − out)/rout − (out − n)/rf − out/rl = 0 is linear in out.
        let g = 1.0 / rin + 1.0 / rf;
        let (a, b) = (1.0 / rin / g, 1.0 / rf / g);
        let coeff_out = (-aol * b - 1.0) / rout - (1.0 - b) / rf - 1.0 / rl;
        let coeff_vin = (-aol * a) / rout + a / rf;
        let expected = -coeff_vin * vin / coeff_out;
        let out = v(&solver, "out");
        // The clamp diodes' GMIN leakage into the 1 MΩ gain node is the only
        // term the hand calculation leaves out: ~µV.
        assert!(
            (out - expected).abs() < 1e-5,
            "out = {out} V, hand calculation {expected} V"
        );
        // And it is visibly NOT the ideal −Rf/Rin: a noise gain of 5.7 against
        // AOL = 10⁴ costs ~0.06 %.
        assert!(relative(out, -rf / rin * vin) > 3e-4);
    }

    #[test]
    fn opamp_follower_carries_the_offset_voltage() {
        let solver = build(
            "Vcc vcc 0 dc 5\nVin in 0 dc 1.2\n\
             X1 in out vcc 0 out AMP\n\
             .model AMP OPAMP(AOL=1e5 GBW=1meg ROUT=10 VOS=5m)\n",
        );
        // out = aol·(vin − out + vos) with no load on out.
        let expected = 1e5 * (1.2 + 5e-3) / (1.0 + 1e5);
        assert!((v(&solver, "out") - expected).abs() < 1e-6);
    }

    #[test]
    fn opamp_input_bias_current_drops_across_both_input_source_resistances() {
        let (aol, rout, load, ibias) = (100.0, 100.0, 10e3, 10e-9);
        // Equal 1 V sources leave a 0.5 mV differential after the same bias
        // current flows through the unequal source resistances. Reversing the
        // current and the resistor order checks the signed-current convention.
        for (current, r_plus, r_minus) in [(ibias, 50e3, 100e3), (-ibias, 100e3, 50e3)] {
            let solver = build(&format!(
                "Vcc vcc 0 dc 5\nVee vee 0 dc 0\n\
                 Vp vp 0 dc 1\nVm vm 0 dc 1\n\
                 Rp vp inp {r_plus}\nRm vm inn {r_minus}\nRl out 0 {load}\n\
                 X1 inp inn vcc vee out AMP\n\
                 .model AMP OPAMP(AOL={aol} GBW=1meg ROUT={rout} IBIAS={current})\n"
            ));
            let differential = current * (r_minus - r_plus);
            let gain_node = aol * differential;
            let expected = gain_node * load / (load + rout);
            let actual = v(&solver, "out");
            assert!(
                // Ignore the clamp diodes' reverse leakage at the gain node.
                (actual - expected).abs() < 1e-5,
                "IBIAS={current}: out = {actual} V, expected {expected} V"
            );
        }
    }

    #[test]
    fn opamp_input_offset_current_splits_around_the_average_bias_current() {
        let (aol, rout, load, ibias, resistance) = (2.0, 100.0, 10e3, 10e-9, 100e3);
        for ios in [4e-9, -4e-9] {
            let solver = build(&format!(
                "Vcc vcc 0 dc 5\nVee vee 0 dc 0\n\
                 Vp vp 0 dc 1\nVm vm 0 dc 1\n\
                 Rp vp inp {resistance}\nRm vm inn {resistance}\nRl out 0 {load}\n\
                 X1 inp inn vcc vee out AMP\n\
                 .model AMP OPAMP(AOL={aol} GBW=1meg ROUT={rout} IBIAS={ibias} IOS={ios})\n"
            ));
            let input_plus = ibias + 0.5 * ios;
            let input_minus = ibias - 0.5 * ios;
            let expected_inp = 1.0 - input_plus * resistance;
            let expected_inn = 1.0 - input_minus * resistance;
            assert!(
                (v(&solver, "inp") - expected_inp).abs() < 1e-8,
                "IOS={ios}: in+ = {} V, expected {expected_inp} V",
                v(&solver, "inp")
            );
            assert!(
                (v(&solver, "inn") - expected_inn).abs() < 1e-8,
                "IOS={ios}: in- = {} V, expected {expected_inn} V",
                v(&solver, "inn")
            );

            let differential = expected_inp - expected_inn;
            let gain_node = aol * differential;
            let expected_out = gain_node * load / (load + rout);
            assert!(
                (v(&solver, "out") - expected_out).abs() < 1e-5,
                "IOS={ios}: out = {} V, expected {expected_out} V",
                v(&solver, "out")
            );
        }
    }

    #[test]
    fn opamp_input_bias_current_must_be_finite() {
        for (parameter, message) in [
            ("IBIAS=1e999", "IBIAS must be finite"),
            ("IOS=1e999", "IOS must be finite"),
            (
                "IBIAS=1.7e308 IOS=1.7e308",
                "IBIAS and IOS produce a non-finite input current",
            ),
        ] {
            let error = parse_netlist(&format!(
                "X1 inp inn vcc vee out AMP\n.model AMP OPAMP({parameter})\n"
            ))
            .expect_err("non-finite input currents cannot be stamped");
            assert!(error.to_string().contains(message), "{error}");
        }
    }

    #[test]
    fn comparator_input_bias_current_drops_across_both_input_source_resistances() {
        let current = 10e-9;
        for bias in [current, -current] {
            let solver = build(&format!(
                "Vcc vcc 0 dc 5\nVee vee 0 dc 0\n\
                 Vp vp 0 dc 1\nVm vm 0 dc 1\n\
                 Rp vp inp 50k\nRm vm inn 100k\nRpu vcc out 10k\n\
                 X1 inp inn vcc vee out CMP\n\
                 .model CMP COMP(AOL=10 GBW=1meg ROUT=100 IBIAS={bias})\n"
            ));
            let expected_inp = 1.0 - bias * 50e3;
            let expected_inn = 1.0 - bias * 100e3;
            let actual_inp = v(&solver, "inp");
            let actual_inn = v(&solver, "inn");
            assert!(
                (actual_inp - expected_inp).abs() < 1e-8,
                "IBIAS={bias}: in+ = {actual_inp} V, expected {expected_inp} V"
            );
            assert!(
                (actual_inn - expected_inn).abs() < 1e-8,
                "IBIAS={bias}: in- = {actual_inn} V, expected {expected_inn} V"
            );
        }
    }

    #[test]
    fn comparator_input_offset_current_splits_around_the_average_bias_current() {
        let (ibias, ios, resistance) = (10e-9, 4e-9, 100e3);
        for offset_current in [ios, -ios] {
            let solver = build(&format!(
                "Vcc vcc 0 dc 5\nVee vee 0 dc 0\n\
                 Vp vp 0 dc 1\nVm vm 0 dc 1\n\
                 Rp vp inp {resistance}\nRm vm inn {resistance}\nRpu vcc out 10k\n\
                 X1 inp inn vcc vee out CMP\n\
                 .model CMP COMP(AOL=10 GBW=1meg ROUT=100 IBIAS={ibias} IOS={offset_current})\n"
            ));
            let input_plus = ibias + 0.5 * offset_current;
            let input_minus = ibias - 0.5 * offset_current;
            let expected_inp = 1.0 - input_plus * resistance;
            let expected_inn = 1.0 - input_minus * resistance;
            assert!(
                (v(&solver, "inp") - expected_inp).abs() < 1e-8,
                "IOS={offset_current}: in+ = {} V, expected {expected_inp} V",
                v(&solver, "inp")
            );
            assert!(
                (v(&solver, "inn") - expected_inn).abs() < 1e-8,
                "IOS={offset_current}: in- = {} V, expected {expected_inn} V",
                v(&solver, "inn")
            );
        }
    }

    #[test]
    fn opamp_and_comparator_input_capacitances_load_the_source_in_transient_runs() {
        // CINCM loads each input to V−; CIND loads the differential input.
        // With the negative input held at ground, these 4 pF + 6 pF capacitors
        // give the driven input a 1 µs time constant through 100 kΩ.
        let expected = 1.0 - (1.0 + 1.0 / 10_000.0f64).powf(-10_000.0);
        for (x_line, model_line) in [
            (
                "X1 inp 0 vcc 0 out AMP",
                ".model AMP OPAMP(AOL=2 GBW=1meg ROUT=100 CINCM=4p CIND=6p)",
            ),
            (
                "X1 inp 0 vcc 0 out CMP",
                ".model CMP COMP(AOL=2 GBW=1meg ROUT=100 CINCM=4p CIND=6p OUTPUT=PP)",
            ),
        ] {
            let actual = macro_input_after_one_tau(x_line, model_line);
            assert!(
                (actual - expected).abs() < 1e-5,
                "{x_line}: input is {actual} V after one tau, expected {expected} V"
            );
        }
    }

    #[test]
    fn macro_input_capacitances_must_be_finite_and_non_negative() {
        for (kind, x_line, model_name, output) in [
            ("OPAMP", "X1 inp 0 vcc 0 out AMP", "AMP", ""),
            ("COMP", "X1 inp 0 vcc 0 out CMP", "CMP", " OUTPUT=PP"),
        ] {
            for (params, message) in [
                ("CINCM=-1p", "CINCM must be finite and non-negative"),
                ("CIND=-1p", "CIND must be finite and non-negative"),
                ("CINCM=1e999", "CINCM must be finite and non-negative"),
                ("CIND=1e999", "CIND must be finite and non-negative"),
            ] {
                let error = parse_netlist(&format!(
                    "{x_line}\n.model {model_name} {kind}({params}{output})\n"
                ))
                .expect_err("invalid input capacitance must be rejected");
                assert!(error.to_string().contains(message), "{error}");
            }
        }
    }

    #[test]
    fn opamp_and_comparator_quiescent_current_flows_between_supply_pins() {
        let iq = 2e-3;
        for (x_line, model_line, inputs) in [
            (
                "X1 inp inn vcc vee out AMP",
                ".model AMP OPAMP(AOL=2 GBW=1meg ROUT=100 IQ=2m)",
                "Vinp inp 0 dc 0.25\nVinn inn 0 dc 0",
            ),
            (
                "X1 inp inn vcc vee out CMP",
                ".model CMP COMP(AOL=2 GBW=1meg ROUT=100 IQ=2m OUTPUT=PP)",
                "Vinp inp 0 dc 0\nVinn inn 0 dc 0.25",
            ),
        ] {
            let solver = build(&format!(
                "Vcc vcc 0 dc 5\nVee vee 0 dc -5\n{inputs}\nRl out 0 10k\n\
                 {x_line}\n{model_line}\n"
            ));
            let from_vcc = -source_current(&solver, "Vcc");
            let into_vee = source_current(&solver, "Vee");
            assert!(
                (from_vcc - iq).abs() < 1e-8,
                "{x_line}: V+ supplies {from_vcc} A, expected {iq} A"
            );
            assert!(
                (into_vee - iq).abs() < 1e-8,
                "{x_line}: V− returns {into_vee} A, expected {iq} A"
            );
        }
    }

    #[test]
    fn macro_quiescent_current_must_be_finite_and_non_negative() {
        for (kind, x_line, model_name, output) in [
            ("OPAMP", "X1 inp 0 vcc 0 out AMP", "AMP", ""),
            ("COMP", "X1 inp 0 vcc 0 out CMP", "CMP", " OUTPUT=PP"),
        ] {
            for value in ["-1m", "1e999"] {
                let error = parse_netlist(&format!(
                    "{x_line}\n.model {model_name} {kind}(IQ={value}{output})\n"
                ))
                .expect_err("invalid quiescent current must be rejected");
                assert!(
                    error
                        .to_string()
                        .contains("IQ must be finite and non-negative"),
                    "{error}"
                );
            }
        }
    }

    #[test]
    fn opamp_and_comparator_differential_input_resistance_draws_between_inputs() {
        let rin = 10e3;
        for (x_line, model_line) in [
            (
                "X1 inp inn vcc vee out AMP",
                ".model AMP OPAMP(AOL=2 GBW=1meg ROUT=100 RIN=10k)",
            ),
            (
                "X1 inp inn vcc vee out CMP",
                ".model CMP COMP(AOL=2 GBW=1meg ROUT=100 RIN=10k OUTPUT=PP)",
            ),
        ] {
            for (vp, vn) in [(0.5, -0.25), (-0.25, 0.5), (0.75, 0.75)] {
                let solver = build(&format!(
                    "Vcc vcc 0 dc 5\nVee vee 0 dc -5\n\
                     Vp inp 0 dc {vp}\nVn inn 0 dc {vn}\n\
                     Rl out 0 10k\n{x_line}\n{model_line}\n"
                ));
                let expected = (vp - vn) / rin;
                let from_plus = -source_current(&solver, "Vp");
                let into_minus = source_current(&solver, "Vn");
                assert!(
                    (from_plus - expected).abs() < 1e-10,
                    "{x_line}: in+ current is {from_plus} A, expected {expected} A"
                );
                assert!(
                    (into_minus - expected).abs() < 1e-10,
                    "{x_line}: in- current is {into_minus} A, expected {expected} A"
                );
            }
        }
    }

    #[test]
    fn macro_differential_input_resistance_must_be_finite_and_non_negative() {
        for (kind, x_line, model_name, output) in [
            ("OPAMP", "X1 inp 0 vcc 0 out AMP", "AMP", ""),
            ("COMP", "X1 inp 0 vcc 0 out CMP", "CMP", " OUTPUT=PP"),
        ] {
            for value in ["-1", "1e999"] {
                let error = parse_netlist(&format!(
                    "{x_line}\n.model {model_name} {kind}(RIN={value}{output})\n"
                ))
                .expect_err("invalid differential input resistance must be rejected");
                assert!(
                    error
                        .to_string()
                        .contains("RIN must be finite and non-negative"),
                    "{error}"
                );
            }
        }
    }

    #[test]
    fn opamp_and_comparator_common_mode_input_resistance_loads_each_input_to_vminus() {
        let rin_cm = 100e3;
        let (input, vminus) = (1.0, -1.0);
        let expected = (input - vminus) / rin_cm;
        for (x_line, model_line) in [
            (
                "X1 inp inn vcc vee out AMP",
                ".model AMP OPAMP(AOL=2 GBW=1meg ROUT=100 RINCM=100k)",
            ),
            (
                "X1 inp inn vcc vee out CMP",
                ".model CMP COMP(AOL=2 GBW=1meg ROUT=100 RINCM=100k OUTPUT=PP)",
            ),
        ] {
            let solver = build(&format!(
                "Vcc vcc 0 dc 5\nVee vee 0 dc {vminus}\n\
                 Vp inp 0 dc {input}\nVn inn 0 dc {input}\n\
                 Rl out 0 10k\n{x_line}\n{model_line}\n"
            ));
            let from_plus = -source_current(&solver, "Vp");
            let from_minus = -source_current(&solver, "Vn");
            assert!(
                (from_plus - expected).abs() < 1e-10,
                "{x_line}: in+ current is {from_plus} A, expected {expected} A"
            );
            assert!(
                (from_minus - expected).abs() < 1e-10,
                "{x_line}: in- current is {from_minus} A, expected {expected} A"
            );
        }
    }

    #[test]
    fn macro_common_mode_input_resistance_must_be_finite_and_non_negative() {
        for (kind, x_line, model_name, output) in [
            ("OPAMP", "X1 inp 0 vcc 0 out AMP", "AMP", ""),
            ("COMP", "X1 inp 0 vcc 0 out CMP", "CMP", " OUTPUT=PP"),
        ] {
            for value in ["-1", "1e999"] {
                let error = parse_netlist(&format!(
                    "{x_line}\n.model {model_name} {kind}(RINCM={value}{output})\n"
                ))
                .expect_err("invalid common-mode input resistance must be rejected");
                assert!(
                    error
                        .to_string()
                        .contains("RINCM must be finite and non-negative"),
                    "{error}"
                );
            }
        }
    }

    #[test]
    fn comparator_input_bias_current_must_be_finite() {
        for (parameter, message) in [
            ("IBIAS=1e999", "IBIAS must be finite"),
            ("IOS=1e999", "IOS must be finite"),
            (
                "IBIAS=1.7e308 IOS=1.7e308",
                "IBIAS and IOS produce a non-finite input current",
            ),
        ] {
            let error = parse_netlist(&format!(
                "X1 inp inn vcc vee out CMP\n.model CMP COMP({parameter})\n"
            ))
            .expect_err("non-finite input currents cannot be stamped");
            assert!(error.to_string().contains(message), "{error}");
        }
    }

    #[test]
    fn a_saturated_opamp_sits_at_its_rails_less_the_drop() {
        // LM358: DROP_HI = 1.5 V, DROP_LO = 0. Open loop, driven both ways,
        // into 10 kΩ through ROUT = 50 Ω: the high rail reads 10.5 V · 10 k /
        // 10.05 k = 10.448 V plus the clamp's few millivolts.
        for (vin, low, high) in [(0.5, 10.448, 10.48), (-0.5, -0.03, 0.0)] {
            let solver = build(&format!(
                "Vcc vcc 0 dc 12\nVin in 0 dc {vin}\nRl out 0 10k\n\
                 X1 in 0 vcc 0 out LM358\n"
            ));
            let out = v(&solver, "out");
            assert!(
                out > low && out < high,
                "vin = {vin}: out = {out} V, expected within [{low}, {high}]"
            );
        }
    }

    #[test]
    fn the_single_pole_sits_at_gbw_over_aol() {
        // Open loop, a 1 mV step: the gain node charges toward AOL·1 mV with
        // tau = Rg·Cg = AOL/(2π·GBW). At one tau it is 63.2 % of the way.
        let (aol, gbw) = (1e4, 1e6);
        let tau = aol / (2.0 * std::f64::consts::PI * gbw);
        let mut solver = build(&format!(
            "Vcc vcc 0 dc 50\nVee vee 0 dc -50\nVin in 0 dc 0\n\
             X1 in 0 vcc vee out AMP\n.model AMP OPAMP(AOL={aol} GBW={gbw} ROUT=1)\n"
        ));
        let vin = solver.voltage_source_index("Vin").unwrap();
        solver.set_voltage_source(vin, 1e-3);
        let steps = 20_000;
        for _ in 0..steps {
            solver.advance(tau / f64::from(steps)).unwrap();
        }
        let expected = aol * 1e-3 * (1.0 - (-1.0f64).exp());
        // Backward Euler at tau/20000 is within 3e-5 of the exponential.
        assert!(
            relative(v(&solver, "out"), expected) < 1e-4,
            "out = {} V, expected {expected} V",
            v(&solver, "out")
        );
        assert_eq!(MACRO_GAIN_RESISTANCE, 1e6);
    }

    #[test]
    fn x_lines_that_are_not_opamps_still_point_at_ngspice() {
        let error = parse_netlist("X1 a b c d e SOMEVENDORSUBCKT\n").unwrap_err();
        assert!(matches!(error, AnalogError::Unsupported { .. }), "{error}");
        // A diode card is not an op-amp either.
        let error = parse_netlist("X1 a b c d e D\n").unwrap_err();
        assert!(matches!(error, AnalogError::Unsupported { .. }), "{error}");

        let error = parse_netlist("X1 a b c out LM358\n").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("expected `X<name> in+ in- v+ v- out LM358`"),
            "{error}"
        );
        let error =
            parse_netlist("X1 a b c d e AMP\n.model AMP OPAMP(AOL=1e5 GAIN=3)\n").unwrap_err();
        assert!(
            error.to_string().contains("not an OPAMP parameter"),
            "{error}"
        );
        let error = parse_netlist("X1 a b c d e CMP\n.model CMP COMP(OUTPUT=TOTEM)\n").unwrap_err();
        assert!(error.to_string().contains("`OUTPUT=TOTEM`"), "{error}");
    }

    // -----------------------------------------------------------------------
    // Comparator
    // -----------------------------------------------------------------------

    #[test]
    fn comparator_input_offset_voltage_shifts_the_switching_threshold_in_both_polarities() {
        // With no hysteresis the documented threshold is V(in+) − V(in−) =
        // −VOS. Check both sides of that shifted threshold, for positive and
        // negative offsets, through the open-collector output stage.
        for (vos, input, expect_sink) in [
            (50e-3, 0.925, true),
            (50e-3, 0.975, false),
            (-50e-3, 1.025, true),
            (-50e-3, 1.075, false),
        ] {
            let solver = build(&format!(
                "Vcc vcc 0 dc 5\nVp inp 0 dc {input}\nVn inn 0 dc 1\n\
                 Rpu vcc out 10k\nX1 inp inn vcc 0 out CMP\n\
                 .model CMP COMP(AOL=1e5 GBW=1meg ROUT=60 VOS={vos})\n"
            ));
            let out = v(&solver, "out");
            if expect_sink {
                assert!(
                    out < 0.1,
                    "VOS={vos} V, input={input} V: output should sink, got {out} V"
                );
            } else {
                assert!(
                    out > 4.9,
                    "VOS={vos} V, input={input} V: output should float high, got {out} V"
                );
            }
        }
    }

    #[test]
    fn an_open_collector_comparator_sinks_or_floats() {
        for (inp, expect_low) in [(1.0, true), (2.0, false)] {
            let solver = build(&format!(
                "Vcc vcc 0 dc 5\nVp p 0 dc {inp}\nVn n 0 dc 1.5\nRpu vcc out 10k\n\
                 X1 p n vcc 0 out LM393\n"
            ));
            let out = v(&solver, "out");
            if expect_low {
                // ROUT = 37.5 Ω against 10 kΩ: 18.7 mV, a little less because
                // the decision node overdrives the switch by its clamp drop.
                let divider = 5.0 * 37.5 / (10e3 + 37.5);
                assert!(
                    out < divider && out > 0.9 * divider,
                    "sinking: out = {out} V, divider {divider} V"
                );
            } else {
                assert!(out > 4.999, "floating: out = {out} V pulled up to 5 V");
            }
        }
    }

    /// Internal hysteresis: ramp the input slowly up and back down and read
    /// the input voltage at which the push-pull output flips each way.
    #[test]
    fn internal_hysteresis_puts_the_thresholds_at_plus_minus_half_vhys() {
        let mut solver = build(
            "Vcc vcc 0 dc 3.3\nVin in 0 dc -0.5\n\
             X1 in 0 vcc 0 out CMP\n\
             .model CMP COMP(AOL=2e5 GBW=12meg ROUT=20 VHYS=0.2 OUTPUT=PP)\n",
        );
        let source = solver.voltage_source_index("Vin").unwrap();
        let out = solver.circuit().node("out").unwrap();
        let mut rising = None;
        let mut falling = None;
        let steps = 10_000;
        for step in 0..=2 * steps {
            // −0.5 V → +0.5 V → −0.5 V at 100 V/s (1 µs steps): slow enough
            // that the comparator's own response delay — the overdrive it
            // needs to slew its decision node — stays near 1 mV.
            let x = f64::from(step) / f64::from(steps);
            let vin = if step <= steps { -0.5 + x } else { 1.5 - x };
            let before = solver.node_voltage(out);
            solver.set_voltage_source(source, vin);
            solver.advance(1e-6).unwrap();
            let after = solver.node_voltage(out);
            if before < 1.65 && after >= 1.65 {
                rising.get_or_insert(vin);
            }
            if before >= 1.65 && after < 1.65 {
                falling.get_or_insert(vin);
            }
        }
        let (rising, falling) = (rising.expect("rose"), falling.expect("fell"));
        assert!((rising - 0.1).abs() < 2e-3, "rising threshold {rising} V");
        assert!(
            (falling + 0.1).abs() < 2e-3,
            "falling threshold {falling} V"
        );
    }

    /// A Schmitt trigger's edge is regenerative: at a 20 ns step Newton used
    /// to cycle between the switch's linear and saturated regions and give up.
    /// The step is now cut and retried, as SPICE cuts its timestep.
    #[test]
    fn a_regenerative_edge_is_solved_by_cutting_the_step() {
        let mut solver = build(
            "Vcc vcc 0 dc 5\nVref ref 0 dc 2.5\n\
             Vin in 0 PULSE(0 5 0 100u 100u 0 200u)\n\
             Rpu vcc out 10k\nR1 ref inp 10k\nR2 out inp 100k\n\
             X1 inp in vcc 0 out LM393\n",
        );
        let out = solver.circuit().node("out").unwrap();
        let mut edges = 0;
        let mut high = solver.node_voltage(out) > 2.5;
        for step in 0..20_000 {
            solver
                .advance(20e-9)
                .unwrap_or_else(|error| panic!("step {step}: {error}"));
            let now = solver.node_voltage(out) > 2.5;
            edges += usize::from(now != high);
            high = now;
        }
        assert_eq!(solver.step_index(), 20_000, "cut steps count once");
        assert_eq!(
            edges, 4,
            "one fall and one rise per 200 µs triangle, two triangles"
        );
    }

    // -----------------------------------------------------------------------
    // Diode breakdown
    // -----------------------------------------------------------------------

    /// With `IBV` forced through the junction (no `RS`), the reverse voltage is
    /// `BV` — ngspice's knee fit places it there — up to the reverse
    /// saturation current `Is·(1 − xbv/Vt)` the fit leaves out.
    #[test]
    fn at_ibv_the_junction_sits_at_bv() {
        let solver = build("Iz 0 k dc 5m\nD1 0 k DZ\n.model DZ D(IS=1e-14 N=1 BV=6.2 IBV=5m)\n");
        let vz = v(&solver, "k");
        assert!(relative(vz, 6.2) < 1e-6, "vz = {vz} V at IBV");
    }

    #[test]
    fn breakdown_voltage_is_ngspices_knee_fit() {
        let vt = device::THERMAL_VOLTAGE;
        let (bv, ibv, is, nbv) = (4.8, 49e-3, 1e-11, 1.2);
        let xbv = device::breakdown_voltage(bv, ibv, is, nbv);
        // The fixed point it converges to: IBV = Is·(exp((BV − xbv)/(NBV·Vt)) − 1 + xbv/Vt).
        let fitted = is * (((bv - xbv) / (nbv * vt)).exp() - 1.0 + xbv / vt);
        assert!(
            relative(fitted, ibv) < 1e-8,
            "fit {fitted} A vs IBV {ibv} A"
        );
        // An IBV below Is·BV/Vt cannot be reached: ngspice keeps BV as written.
        assert_eq!(device::breakdown_voltage(5.0, 1e-15, 1e-14, 1.0), 5.0);
    }

    #[test]
    fn the_zener_regulates_and_the_tvs_clamps() {
        // 1N4733A from 9 V through 100 Ω: ~39 mA, VZ within a few tenths of
        // the 5.1 V it is rated at 49 mA.
        let solver = build("Vin in 0 dc 9\nR1 in k 100\nD1 0 k 1N4733A\n");
        let vz = v(&solver, "k");
        assert!(vz > 4.9 && vz < 5.15, "VZ = {vz} V");
        // SMAJ5.0A at its rated peak pulse current, 43.5 A: VC = 9.2 V.
        let solver = build("Ipp 0 k dc 43.5\nD1 0 k SMAJ5.0A\n");
        let vc = v(&solver, "k");
        assert!((vc - 9.2).abs() < 0.05, "VC = {vc} V at IPP");
        // And it stands off its working voltage: at 5 V it leaks nothing.
        let solver = build("Vin k 0 dc 5\nD1 0 k SMAJ5.0A\n");
        let leak = -solver.branch_current(0);
        assert!(leak.abs() < 1e-9, "leakage at VRWM {leak} A");
    }

    #[test]
    fn breakdown_parameters_are_validated() {
        let error = parse_netlist("D1 a 0 DZ\n.model DZ D(BV=5 IBV=0)\nR1 a 0 1k\n").unwrap_err();
        assert!(error.to_string().contains("BV > 0, IBV > 0"), "{error}");
    }

    // -----------------------------------------------------------------------
    // Goldens
    // -----------------------------------------------------------------------

    /// FNV-1a over the raw bits of one node at every step.
    fn trajectory_hash(netlist: &str, node: &str, h: f64, steps: u32) -> u64 {
        let circuit = parse_netlist(netlist).expect("parses");
        let mut solver = Solver::new(circuit, Integration::Trapezoidal).expect("builds");
        let node = solver.circuit().node(node).expect("node");
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for _ in 0..steps {
            solver.advance(h).expect("step");
            for byte in solver.node_voltage(node).to_bits().to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        hash
    }

    /// Pinned trajectories of an LM358 amplifier driven into clipping, a
    /// 1N4733A regulator swept through its knee, and an LM393 Schmitt trigger.
    /// They catch drift, not physics (the differential does that): a changed
    /// hash has to be explained before it is re-pinned.
    #[test]
    fn macro_trajectories_are_bit_stable() {
        let opamp = trajectory_hash(
            "Vcc vcc 0 dc 12\nVee vee 0 dc -12\nVin in 0 SIN(0 1.2 20k)\n\
             Rin in inn 10k\nRf inn out 100k\nRl out 0 10k\nXU1 0 inn vcc vee out LM358\n",
            "out",
            50e-9,
            3000,
        );
        let zener = trajectory_hash(
            "Vin in 0 PULSE(0 9 0 1m 1m 1m 4m)\nR1 in k 100\nRl k 0 10k\nD1 0 k 1N4733A\n",
            "k",
            1e-6,
            3500,
        );
        let schmitt = trajectory_hash(
            "Vcc vcc 0 dc 5\nVref ref 0 dc 2.5\nVin in 0 PULSE(0 5 0 10u 10u 0 20u)\n\
             Rpu vcc out 10k\nR1 ref inp 10k\nR2 out inp 100k\nXU1 inp in vcc 0 out LM393\n",
            "out",
            1e-9,
            40_000,
        );
        assert_eq!(
            [opamp, zener, schmitt],
            [OPAMP_HASH, ZENER_HASH, SCHMITT_HASH],
            "got [{opamp:#018x}, {zener:#018x}, {schmitt:#018x}]"
        );
    }

    const OPAMP_HASH: u64 = 0x434d_e1ed_0fe0_c115;
    const ZENER_HASH: u64 = 0x272a_2e74_684b_dc23;
    // The comparator's open-collector MOSFET now includes its reverse-biased
    // bulk-drain junction, so the Schmitt output trajectory includes that
    // junction's IS + GMIN leakage. The op-amp and zener hashes are unchanged.
    const SCHMITT_HASH: u64 = 0xbad1_3c95_8766_f606;

    // -----------------------------------------------------------------------
    // Regulators
    // -----------------------------------------------------------------------

    fn source_current(solver: &Solver, name: &str) -> f64 {
        let index = solver.circuit().branch_index(name).expect("branch exists");
        solver.branch_current(index)
    }

    /// AMS1117-3.3 regulating from 9 V into 33 Ω: the output is VOUT plus the
    /// line term minus ROUT·I, and the input carries the output current plus
    /// IQ — a pass element, nothing lost but the headroom.
    #[test]
    fn ldo_regulates_and_passes_its_output_current_to_the_input() {
        let solver = build("Vin in 0 dc 9\nRl out 0 33\nXU1 in out 0 AMS1117-3.3\n");
        let line = 1e-3 / (12.0 - 4.75);
        let rout = 1e-3 / 0.8;
        let target = 3.3 + line * (9.0 - 3.3 - 1.1);
        let out = v(&solver, "out");
        // I = out/33, so out = target − ROUT·out/33.
        let expected = target / (1.0 + rout / 33.0);
        // Within 0.2 mV: GMIN (1 pS, as in ngspice) across the reverse-biased
        // current-limit clamp — K·(ILIM − I) ≈ 100 V — leaks ~0.1 nA into the
        // 1 MΩ set-point node.
        assert!((out - expected).abs() < 2e-4, "out {out} vs {expected}");
        let i_in = -source_current(&solver, "Vin");
        let expected_in = out / 33.0 + 5e-3 * (target / 3.3);
        assert!(
            (i_in - expected_in).abs() < 1e-6,
            "input current {i_in} vs {expected_in}"
        );
    }

    /// In dropout the output is the input less VDO (plus one clamp drop, a
    /// few millivolts), not the set point.
    #[test]
    fn ldo_in_dropout_follows_the_input_less_its_dropout() {
        let solver = build("Vin in 0 dc 4.0\nRl out 0 330\nXU1 in out 0 AMS1117-3.3\n");
        let out = v(&solver, "out");
        assert!(
            out > 4.0 - 1.1 && out < 4.0 - 1.1 + 0.015,
            "AMS1117-3.3 at 4.0 V in should sit ~2.9 V, got {out}"
        );
    }

    /// LM317 with the datasheet divider: VREF·(1 + R2/R1) + IADJ·R2.
    #[test]
    fn lm317_output_is_the_datasheet_formula() {
        let solver = build(
            "Vin in 0 dc 12\nR1 out adj 240\nR2 adj 0 720\nRl out 0 100\nXU1 in out adj LM317\n",
        );
        let out = v(&solver, "out");
        // IADJ scales with how far the part is into regulation; here fully.
        let expected = 1.25 * (1.0 + 720.0 / 240.0) + 50e-6 * 720.0;
        assert!(
            relative(out, expected) < 2e-3,
            "LM317 out {out} vs {expected} (the rest is line and load regulation)"
        );
    }

    /// A buck's input current is the output power over EFF·v(in), plus IQ.
    #[test]
    fn buck_draws_output_power_over_efficiency() {
        let solver = build("Vin in 0 dc 12\nRl out 0 5\nXU1 in out 0 LM2596-5.0\n");
        let out = v(&solver, "out");
        assert!((out - 5.0).abs() < 0.01, "LM2596-5.0 out {out}");
        let i_in = -source_current(&solver, "Vin");
        let p_out = out * out / 5.0;
        let expected = p_out / (0.80 * 12.0) + 5e-3 * (5.0 / 5.0);
        assert!(
            relative(i_in, expected) < 1e-3,
            "input current {i_in} vs P/(EFF·Vin) + IQ = {expected}"
        );
        // And it is a buck: less current in than out.
        assert!(i_in < out / 5.0);
    }

    /// Past ILIM the output current is held at ILIM (to within the limit
    /// clamp's few milliamps).
    #[test]
    fn current_limit_holds_the_output_at_ilim() {
        let solver = build("Vin in 0 dc 9\nRl out 0 1\nXU1 in out 0 LM7805\n");
        let current = v(&solver, "out") / 1.0;
        assert!(
            (current - 2.1).abs() < 0.01,
            "LM7805 into 1 Ω must current-limit at 2.1 A, got {current} A"
        );
    }

    #[test]
    fn regulator_lines_are_checked() {
        let error = |netlist: &str| match parse_netlist(netlist) {
            Err(AnalogError::Parse { message, .. }) => message,
            other => panic!("expected a parse error, got {other:?}"),
        };
        assert!(
            error("Vin in 0 dc 5\nXU1 in out AMS1117-3.3\nRl out 0 1k\n")
                .contains("expected `X<name> in out gnd AMS1117-3.3`")
        );
        assert!(
            error("Vin in 0 dc 5\nXU1 in out 0 fb R\nRl out 0 1k\nRf fb 0 1k\n.model R LDO(VOUT=3.3 EFF=0.9)\n")
                .contains("`EFF` is not a LDO parameter")
        );
        assert!(error(
            "Vin in 0 dc 5\nXU1 in out 0 B\nRl out 0 1k\n.model B BUCK(VOUT=3.3 VREF=0.8)\n"
        )
        .contains("exactly one of VOUT"));
        assert!(error(
            "Vin in 0 dc 5\nXU1 in out 0 B\nRl out 0 1k\n.model B BUCK(VOUT=3.3 EFF=1.5)\n"
        )
        .contains("0 < EFF <= 1"));
    }
}
