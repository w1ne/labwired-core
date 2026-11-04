// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Netlist-parser contract: the SPICE subset the in-core analog engine accepts,
//! and the errors it reports for everything else.

// The `analog` module puts every test under `analog::`, so the design's
// verification command `cargo test -p labwired-core analog` runs all of them.
mod analog {
    use labwired_core::analog::{
        parse_netlist, parse_spice_value, AnalogError, Integration, Polarity, Solver, Waveform,
    };

    const RC: &str = "\
* GPIO drives an RC low-pass; the firmware ADC samples node `out`.
Vgpio in 0 dc 0
R1 in out 10k
C1 out 0 100n
.end
";

    #[test]
    fn parses_the_worked_rc_example() {
        let circuit = parse_netlist(RC).expect("rc netlist parses");

        assert_eq!(circuit.resistors.len(), 1);
        assert_eq!(circuit.resistors[0].name, "R1");
        assert_eq!(circuit.resistors[0].ohms, 10_000.0);

        assert_eq!(circuit.capacitors.len(), 1);
        assert_eq!(circuit.capacitors[0].farads, 100e-9);
        assert_eq!(circuit.capacitors[0].ic, None);

        assert_eq!(circuit.voltage_sources.len(), 1);
        assert_eq!(circuit.voltage_sources[0].name, "Vgpio");
        assert_eq!(circuit.voltage_sources[0].dc, 0.0);

        // `in` and `out`; `0` is ground and never a node.
        assert_eq!(circuit.node_count(), 2);
        assert_eq!(circuit.node("0"), Some(None));
        assert_eq!(circuit.node("gnd"), Some(None));
        assert_eq!(circuit.node("GND"), Some(None));
        assert!(circuit.node("out").expect("node `out` is known").is_some());
        assert_eq!(circuit.node("nope"), None);

        // 2 nodes + 1 voltage-source branch current.
        assert_eq!(circuit.unknowns(), 3);
    }

    #[test]
    fn parses_coupled_inductors_and_resolves_forward_references() {
        let circuit =
            parse_netlist("Kx Lprimary Lsecondary -500m\nLprimary p 0 4m\nLsecondary s 0 1m\n")
                .expect("coupling may precede the referenced inductors");

        assert_eq!(circuit.inductors.len(), 2);
        assert_eq!(circuit.mutual_inductors.len(), 1);
        let mutual = &circuit.mutual_inductors[0];
        assert_eq!(mutual.name, "Kx");
        assert_eq!(mutual.first, 0);
        assert_eq!(mutual.second, 1);
        assert!((mutual.mutual_henries + 1e-3).abs() < 1e-18);
        assert_eq!(circuit.unknowns(), 4, "K adds no nodes or branch unknowns");
    }

    #[test]
    fn refuses_invalid_mutual_inductor_declarations() {
        for text in [
            "L1 a 0 1m\nL2 b 0 1m\nK1 L1 L2 1.01\n",
            "L1 a 0 1m\nK1 L1 Lmissing 0.5\n",
            "L1 a 0 1m\nK1 L1 L1 0.5\n",
            "L1 a 0 1m\nR2 b 0 1k\nK1 L1 R2 0.5\n",
            "L1 a 0 1m\nL2 b 0 1m\nK1 L1 L2 0.5\nK2 L2 L1 0.5\n",
        ] {
            assert!(parse_netlist(text).is_err(), "must refuse `{text}`");
        }
    }

    #[test]
    fn parses_beta_model_ntc_resistors_with_an_external_temperature() {
        let circuit = parse_netlist(
            "Vcc vcc 0 dc 3.3\nRntc vcc sense 10k beta=3950 tnom=25 temp=50\nRpull sense 0 10k\n",
        )
        .expect("NTC resistor parses");
        let ntc = &circuit.resistors[0];
        let model = ntc.ntc.expect("beta attaches an NTC model");
        assert_eq!(ntc.ohms, 10_000.0, "ohms is R at the nominal temperature");
        assert_eq!(model.beta, 3950.0);
        assert_eq!(model.nominal_temperature_celsius, 25.0);
        assert_eq!(model.temperature_celsius, 50.0);
        let expected = 10_000.0 * f64::exp(3950.0 * (1.0 / (50.0 + 273.15) - 1.0 / 298.15));
        assert!((ntc.resistance_ohms() - expected).abs() / expected < 1e-12);
        assert!(
            circuit.resistors[1].ntc.is_none(),
            "plain resistors stay linear"
        );
    }

    #[test]
    fn refuses_invalid_ntc_parameters() {
        for text in [
            "Rntc a 0 10k beta=0",
            "Rntc a 0 -10k beta=3950",
            "Rntc a 0 10k beta=3950 temp=-273.15",
            "Rntc a 0 10k temp=25",
        ] {
            assert!(parse_netlist(text).is_err(), "must refuse `{text}`");
        }
    }

    #[test]
    fn changing_ntc_temperature_rebuilds_the_mna_resistor_stamp() {
        let circuit =
            parse_netlist("Vcc vcc 0 dc 3.3\nRntc vcc sense 10k beta=3950\nRpull sense 0 10k\n")
                .expect("NTC divider parses");
        let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver builds");
        let sense = solver.circuit().node("sense").expect("sense node");
        let thermistor = solver
            .thermistor_index("rntc")
            .expect("case-insensitive lookup");
        let at_25_c = solver.node_voltage(sense);
        assert!((at_25_c - 1.65).abs() < 1e-6, "25 °C divider: {at_25_c} V");

        assert!(solver.set_thermistor_temperature(thermistor, 80.0));
        solver.advance(1e-6).expect("hot operating point solves");
        let hot = solver.node_voltage(sense);
        assert!(hot > at_25_c, "NTC divider rises when hot: {hot} V");

        assert!(solver.set_thermistor_temperature(thermistor, -10.0));
        solver.advance(1e-6).expect("cold operating point solves");
        let cold = solver.node_voltage(sense);
        assert!(cold < at_25_c, "NTC divider falls when cold: {cold} V");

        assert!(!solver.set_thermistor_temperature(thermistor, -273.15));
        assert!(!solver.set_thermistor_temperature(thermistor, f64::NAN));
    }

    #[test]
    fn parses_every_element_of_the_subset() {
        let text = "\
* every supported element
V1 a 0 dc 5
I1 b 0 dc 1m
R1 a b 1k
C1 b 0 10u ic=0.5
L1 b c 2.2m ic=-0.25
S1 c 0 gate ron=1.5 roff=1meg
J1 b c b NJF
.ic V(a)=5 V(b)=1.25
.end
";
        let circuit = parse_netlist(text).expect("subset parses");

        assert_eq!(circuit.current_sources[0].dc, 1e-3);
        assert_eq!(circuit.capacitors[0].ic, Some(0.5));
        assert_eq!(circuit.inductors[0].henries, 2.2e-3);
        assert_eq!(circuit.inductors[0].ic, Some(-0.25));

        let switch = &circuit.switches[0];
        assert_eq!(switch.ctrl, "gate");
        assert_eq!(switch.ron, 1.5);
        assert_eq!(switch.roff, 1e6);
        assert_eq!(circuit.jfets.len(), 1);
        assert_eq!(circuit.jfets[0].model.polarity, Polarity::N);
        assert!(
            switch.voltage.is_none(),
            "digital form has no voltage control"
        );

        assert_eq!(circuit.node_ic.len(), 2);

        // 3 nodes (a, b, c) + 1 voltage source + 1 inductor branch current.
        assert_eq!(circuit.unknowns(), 5);
    }

    #[test]
    fn parses_a_voltage_controlled_switch() {
        let text = "\
* voltage switch
S1 a b sense+ sense- vt=3.75 vh=3.25 invert=1 ron=0.1 roff=1meg
.end
";
        let circuit = parse_netlist(text).expect("voltage switch parses");
        let switch = &circuit.switches[0];
        assert!(switch.ctrl.is_empty(), "voltage form leaves ctrl empty");
        let control = switch.voltage.as_ref().expect("voltage control set");
        assert!((control.vt - 3.75).abs() < 1e-12);
        assert!((control.vh - 3.25).abs() < 1e-12);
        assert!(control.invert);
        assert!((switch.ron - 0.1).abs() < 1e-12);
        assert!((switch.roff - 1e6).abs() < 1e-6);
        // Sense nodes were interned.
        assert!(circuit.node("sense+").is_some());
        assert!(circuit.node("sense-").is_some());
    }

    #[test]
    fn spice_value_suffixes() {
        for (text, expected) in [
            ("10k", 10e3),
            ("1meg", 1e6),
            ("2.2MEG", 2.2e6),
            ("100n", 100e-9),
            ("4p", 4e-12),
            ("5u", 5e-6),
            ("3m", 3e-3),
            ("1g", 1e9),
            ("2t", 2e12),
            ("7f", 7e-15),
            ("-1.5", -1.5),
            ("1e-6", 1e-6),
            ("2.5E3", 2.5e3),
            // Trailing unit letters are ignored, as in SPICE.
            ("100nF", 100e-9),
            ("10kohm", 10e3),
        ] {
            let value = parse_spice_value(text).unwrap_or_else(|| panic!("{text} should parse"));
            assert!(
                (value - expected).abs() <= expected.abs() * 1e-12,
                "{text} parsed as {value}, expected {expected}"
            );
        }
        assert_eq!(parse_spice_value("k"), None);
        assert_eq!(parse_spice_value(""), None);
    }

    #[test]
    fn unknown_element_letters_point_at_ngspice() {
        // `D`, `Q`, `M` and `J` used to be on this list and are now in the subset,
        // and so are the four LINEAR controlled sources and an `X` whose model
        // is an OPAMP/COMP card. What is left is what still has no in-core
        // term: a subcircuit needs a flattener, a model library needs a file
        // loader, a MESFET and a POLY/VALUE controlled source need equations
        // nobody has written here.
        for line in [
            "X1 a b subckt",
            "Z1 d g s mesfet",
            "E1 a b POLY(2) c 0 d 0 0 1 1",
            "G1 a b VALUE={V(c)*V(d)}",
            ".include models/bsim.lib",
            ".lib vendor.lib tt",
        ] {
            let text = format!("* header\n{line}\n.end\n");
            let err = parse_netlist(&text).expect_err("unsupported element must fail");
            assert_eq!(
                err.to_string(),
                format!(
                    "element `{line}` needs ngspice; use `adapter: external_process` \
                 with `tools/cosim/labwired_ngspice.py`"
                )
            );
            assert_eq!(err.line(), Some(2), "error must carry the 1-based line");
            assert_eq!(err.text(), Some(line));
        }
    }

    #[test]
    fn parse_errors_name_the_line_and_its_text() {
        let text = "\
* header
R1 in out 10k
C1 out 0
.end
";
        let err = parse_netlist(text).expect_err("short capacitor line must fail");
        assert_eq!(err.line(), Some(3));
        assert_eq!(err.text(), Some("C1 out 0"));
        assert!(
            err.to_string().contains("line 3"),
            "message should name the line: {err}"
        );
        assert!(matches!(err, AnalogError::Parse { .. }));

        let bad_value =
            parse_netlist("* h\nR1 in out ten\n.end\n").expect_err("bad value must fail");
        assert_eq!(bad_value.line(), Some(2));
        assert!(
            bad_value.to_string().contains("ten"),
            "message should quote the offending value: {bad_value}"
        );
    }

    #[test]
    fn comments_blank_lines_and_trailing_comments_are_ignored() {
        let text = "\
* leading comment
    * indented comment

R1 in out 10k ; trailing comment
C1 out 0 100n $ dollar comment
.END
";
        let circuit = parse_netlist(text).expect("comments are skipped");
        assert_eq!(circuit.resistors.len(), 1);
        assert_eq!(circuit.capacitors.len(), 1);
    }

    #[test]
    fn missing_end_is_accepted_and_lines_after_end_are_rejected() {
        let no_end = parse_netlist("* h\nR1 a 0 1k\n").expect("`.end` is optional");
        assert_eq!(no_end.resistors.len(), 1);

        let err = parse_netlist("* h\nR1 a 0 1k\n.end\nR2 a 0 1k\n").expect_err("text after .end");
        assert_eq!(err.line(), Some(4));
    }

    #[test]
    fn duplicate_element_names_are_rejected() {
        let err = parse_netlist("* h\nR1 a 0 1k\nR1 a 0 2k\n.end\n").expect_err("duplicate name");
        assert_eq!(err.line(), Some(3));
        assert!(err.to_string().contains("R1"), "{err}");
    }

    // -----------------------------------------------------------------------
    // Model cards and the three nonlinear elements
    // -----------------------------------------------------------------------

    /// An element may name a `.model` card written anywhere in the file,
    /// including after it — SPICE decks routinely put the model library at the
    /// bottom, and refusing that would mean hand-editing every one.
    #[test]
    fn a_model_card_may_be_declared_after_the_elements_that_use_it() {
        let circuit = parse_netlist(
            "* forward reference\n\
             Vs s 0 dc 5\n\
             R1 s a 1k\n\
             D1 a 0 DMOD\n\
             .model DMOD D(IS=4n N=1.8 RS=0.5)\n\
             .end\n",
        )
        .expect("forward model reference parses");
        assert_eq!(circuit.diodes.len(), 1);
        assert_eq!(circuit.diodes[0].model.is, 4e-9);
        assert_eq!(circuit.diodes[0].model.n, 1.8);
        assert_eq!(circuit.diodes[0].model.rs, 0.5);
    }

    /// The diode's depletion and diffusion-charge parameters resolve into the
    /// model; unsupported temperature parameters remain accepted for vendor
    /// card compatibility.
    #[test]
    fn diode_charge_and_recombination_parameters_resolve_with_vendor_parameters() {
        let circuit = parse_netlist(
            "D1 a 0 D1N4148\n\
             .model D1N4148 D(IS=2.52n RS=0.568 N=1.752 IKF=100m IKR=20m CJO=4p M=0.333 FC=0.4 TT=11.54n VP=0.2 \
             VJ=0.75 BV=100 IBV=100u ISR=8p NR=1.8 EG=1.11 XTI=3)\n",
        )
        .expect("a vendor card parses");
        assert_eq!(circuit.diodes[0].model.is, 2.52e-9);
        assert_eq!(circuit.diodes[0].model.n, 1.752);
        assert_eq!(circuit.diodes[0].model.ik, 100e-3);
        assert_eq!(circuit.diodes[0].model.ikr, 20e-3);
        assert_eq!(circuit.diodes[0].model.isr, 8e-12);
        assert_eq!(circuit.diodes[0].model.nr, 1.8);
        assert_eq!(circuit.diodes[0].model.rs, 0.568);
        assert_eq!(circuit.diodes[0].model.tt, 11.54e-9);
        assert_eq!(circuit.diodes[0].model.vp, 0.2);
        assert_eq!(circuit.diodes[0].model.cjo, 4e-12);
        assert_eq!(circuit.diodes[0].model.m, 0.333);
        assert_eq!(circuit.diodes[0].model.vj, 0.75);
        assert_eq!(circuit.diodes[0].model.fc, 0.4);
    }

    #[test]
    fn invalid_diode_model_parameters_are_rejected() {
        for parameters in [
            "IK=-1m", "IKR=-1m", "TT=-1n", "VP=-0.1", "CJO=-1p", "VJ=0", "M=-0.1", "FC=-0.1",
            "FC=1", "ISR=-1p", "NR=0",
        ] {
            let text = format!("D1 a 0 DMOD\n.model DMOD D({parameters})\n");
            let error = parse_netlist(&text).expect_err("invalid junction parameters are refused");
            assert!(matches!(error, AnalogError::Parse { .. }), "{error}");
        }
    }

    #[test]
    fn bjt_early_and_high_injection_parameters_resolve_and_default_to_infinity() {
        let circuit = parse_netlist(
            "Q1 c b e NPN\nQ2 c2 b2 e2 QMOD\n\
             .model QMOD NPN(VAF=75 VAR=120 IKF=5m IKR=2m ISE=3p NE=1.6 ISC=4p NC=1.8)\n",
        )
        .expect("BJT model parameters parse");
        assert_eq!(circuit.bjts[0].model.vaf, f64::INFINITY);
        assert_eq!(circuit.bjts[0].model.var, f64::INFINITY);
        assert_eq!(circuit.bjts[0].model.ikf, f64::INFINITY);
        assert_eq!(circuit.bjts[0].model.ikr, f64::INFINITY);
        assert_eq!(circuit.bjts[0].model.nkf, None);
        assert_eq!(circuit.bjts[0].model.ise, 0.0);
        assert_eq!(circuit.bjts[0].model.ne, 1.5);
        assert_eq!(circuit.bjts[0].model.isc, 0.0);
        assert_eq!(circuit.bjts[0].model.nc, 2.0);
        assert_eq!(circuit.bjts[1].model.vaf, 75.0);
        assert_eq!(circuit.bjts[1].model.var, 120.0);
        assert_eq!(circuit.bjts[1].model.ikf, 5e-3);
        assert_eq!(circuit.bjts[1].model.ikr, 2e-3);
        assert_eq!(circuit.bjts[1].model.ise, 3e-12);
        assert_eq!(circuit.bjts[1].model.ne, 1.6);
        assert_eq!(circuit.bjts[1].model.isc, 4e-12);
        assert_eq!(circuit.bjts[1].model.nc, 1.8);
    }

    #[test]
    fn bjt_nkf_exponent_and_legacy_alias_parse_and_validate() {
        for (parameter, expected) in [("NKF=0.8", 0.8), ("NK=0.7", 0.7)] {
            let circuit = parse_netlist(&format!("Q1 c b e QMOD\n.model QMOD NPN({parameter})\n"))
                .expect("a finite nonnegative NKF exponent parses");
            assert_eq!(circuit.bjts[0].model.nkf, Some(expected));
        }

        for parameter in ["NKF=-0.1"] {
            let error = parse_netlist(&format!("Q1 c b e QMOD\n.model QMOD NPN({parameter})\n"))
                .expect_err("a negative NKF exponent is rejected");
            assert!(error.to_string().contains("NKF >= 0"), "{error}");
        }
        for parameter in ["NKF=inf", "NKF=nan", "NKF=1e999"] {
            parse_netlist(&format!("Q1 c b e QMOD\n.model QMOD NPN({parameter})\n"))
                .expect_err("a nonfinite NKF exponent is rejected");
        }
    }

    #[test]
    fn bjt_depletion_parameters_resolve_and_validate() {
        let circuit = parse_netlist(
            "Q1 c b e QMOD\n\
             .model QMOD NPN(CJE=4p VJE=0.7 MJE=0.4 CJC=2p VJC=0.8 MJC=0.3 FC=0.6)\n",
        )
        .expect("BJT depletion parameters parse");
        let model = circuit.bjts[0].model;
        assert_eq!(model.cje, 4e-12);
        assert_eq!(model.vje, 0.7);
        assert_eq!(model.mje, 0.4);
        assert_eq!(model.cjc, 2e-12);
        assert_eq!(model.vjc, 0.8);
        assert_eq!(model.mjc, 0.3);
        assert_eq!(model.fc, 0.6);

        let defaults = parse_netlist("Q1 c b e QMOD\n.model QMOD PNP\n")
            .expect("omitted BJT charge parameters use SPICE defaults");
        let defaults = defaults.bjts[0].model;
        assert_eq!(defaults.cje, 0.0);
        assert_eq!(defaults.vje, 0.75);
        assert_eq!(defaults.mje, 0.33);
        assert_eq!(defaults.cjc, 0.0);
        assert_eq!(defaults.vjc, 0.75);
        assert_eq!(defaults.mjc, 0.33);
        assert_eq!(defaults.fc, 0.5);

        for parameters in [
            "CJE=-1p", "VJE=0", "MJE=-0.1", "CJC=-1p", "VJC=0", "MJC=-0.1", "FC=-0.1", "FC=1",
        ] {
            let text = format!("Q1 c b e QMOD\n.model QMOD NPN({parameters})\n");
            let error =
                parse_netlist(&text).expect_err("invalid BJT junction parameters are refused");
            assert!(matches!(error, AnalogError::Parse { .. }), "{error}");
        }
    }

    #[test]
    fn bjt_transit_time_parameters_resolve_and_validate() {
        let circuit = parse_netlist(
            "Q1 c b e QMOD\n\
             .model QMOD NPN(TF=12n TR=35n XTF=2 VTF=1 ITF=10m)\n",
        )
        .expect("ideal and bias-dependent BJT transit parameters parse");
        assert_eq!(circuit.bjts[0].model.tf, 12e-9);
        assert_eq!(circuit.bjts[0].model.tr, 35e-9);
        assert_eq!(circuit.bjts[0].model.xtf, 2.0);
        assert_eq!(circuit.bjts[0].model.vtf, 1.0);
        assert_eq!(circuit.bjts[0].model.itf, 10e-3);

        let defaults = parse_netlist("Q1 c b e QMOD\n.model QMOD NPN\n").unwrap();
        assert_eq!(defaults.bjts[0].model.tf, 0.0);
        assert_eq!(defaults.bjts[0].model.tr, 0.0);
        assert_eq!(defaults.bjts[0].model.xtf, 0.0);
        assert_eq!(defaults.bjts[0].model.vtf, f64::INFINITY);
        assert_eq!(defaults.bjts[0].model.itf, 0.0);

        for parameter in ["TF=-1n", "TR=-1n", "XTF=-1", "VTF=-1", "ITF=-1"] {
            let error = parse_netlist(&format!("Q1 c b e QMOD\n.model QMOD NPN({parameter})\n"))
                .expect_err("negative transit parameters are invalid");
            assert!(error.to_string().contains("TF/TR/XTF/ITF >= 0"), "{error}");
        }
    }

    #[test]
    fn bjt_terminal_resistances_create_internal_junction_nodes_and_validate() {
        let circuit = parse_netlist("Q1 c b e QMOD\n.model QMOD NPN(RB=12 RC=3 RE=0.5)\n")
            .expect("BJT terminal resistances parse");
        let bjt = &circuit.bjts[0];
        assert_eq!(bjt.model.rb, 12.0);
        assert_eq!(bjt.model.rc, 3.0);
        assert_eq!(bjt.model.re, 0.5);
        assert_eq!(bjt.model.rbm, None);
        assert_eq!(bjt.model.irb, f64::INFINITY);
        assert_eq!(circuit.node_count(), 6);
        assert_eq!(
            bjt.junction_b,
            circuit.node("Q1#base").expect("base junction exists")
        );
        assert_eq!(
            bjt.junction_c,
            circuit
                .node("Q1#collector")
                .expect("collector junction exists")
        );
        assert_eq!(
            bjt.junction_e,
            circuit.node("Q1#emitter").expect("emitter junction exists")
        );
        assert_ne!(bjt.junction_b, bjt.b);
        assert_ne!(bjt.junction_c, bjt.c);
        assert_ne!(bjt.junction_e, bjt.e);

        let defaults = parse_netlist("Q1 c b e QMOD\n.model QMOD NPN\n")
            .expect("omitted terminal resistances default to zero");
        let bjt = &defaults.bjts[0];
        assert_eq!(bjt.model.rb, 0.0);
        assert_eq!(bjt.model.rc, 0.0);
        assert_eq!(bjt.model.re, 0.0);
        assert_eq!(defaults.node_count(), 3);
        assert_eq!(bjt.junction_b, bjt.b);
        assert_eq!(bjt.junction_c, bjt.c);
        assert_eq!(bjt.junction_e, bjt.e);

        for parameter in ["RB=-1", "RC=-1", "RE=-1"] {
            let error = parse_netlist(&format!("Q1 c b e QMOD\n.model QMOD NPN({parameter})\n"))
                .expect_err("negative BJT terminal resistance is invalid");
            assert!(error.to_string().contains("RB/RC/RE >= 0"), "{error}");
        }

        let rolloff = parse_netlist("Q1 c b e QMOD\n.model QMOD NPN(RB=60 RBM=10 IRB=2m)\n")
            .expect("current-dependent BJT base-resistance parameters parse");
        assert_eq!(rolloff.bjts[0].model.rb, 60.0);
        assert_eq!(rolloff.bjts[0].model.rbm, Some(10.0));
        assert_eq!(rolloff.bjts[0].model.irb, 2e-3);

        for parameters in ["RB=10 RBM=11", "RBM=-1", "IRB=0", "IRB=-1"] {
            let error = parse_netlist(&format!("Q1 c b e QMOD\n.model QMOD NPN({parameters})\n"))
                .expect_err("invalid current-dependent base-resistance parameters are refused");
            assert!(
                error.to_string().contains("0 <= RBM <= RB and IRB > 0"),
                "{error}"
            );
        }

        let collision = parse_netlist("Q1 c b e QMOD\nR1 Q1#base 0 1k\n.model QMOD NPN(RB=10)\n")
            .expect_err("a user node cannot alias an internal BJT junction");
        assert!(collision.to_string().contains("Q1#base"), "{collision}");
    }

    #[test]
    fn jfet_terminal_resistances_create_internal_nodes_and_validate() {
        let circuit = parse_netlist("J1 d g s JMOD\n.model JMOD NJF(RD=12 RS=3)\n")
            .expect("JFET terminal resistances parse");
        let jfet = &circuit.jfets[0];
        assert_eq!(jfet.model.rd, 12.0);
        assert_eq!(jfet.model.rs, 3.0);
        assert_eq!(circuit.node_count(), 5);
        assert_eq!(
            jfet.junction_d,
            circuit.node("J1#drain").expect("drain junction exists")
        );
        assert_eq!(
            jfet.junction_s,
            circuit.node("J1#source").expect("source junction exists")
        );
        assert_ne!(jfet.junction_d, jfet.d);
        assert_ne!(jfet.junction_s, jfet.s);

        let defaults = parse_netlist("J1 d g s JMOD\n.model JMOD PJF\n")
            .expect("omitted JFET terminal resistances default to zero");
        let jfet = &defaults.jfets[0];
        assert_eq!(jfet.model.rd, 0.0);
        assert_eq!(jfet.model.rs, 0.0);
        assert_eq!(defaults.node_count(), 3);
        assert_eq!(jfet.junction_d, jfet.d);
        assert_eq!(jfet.junction_s, jfet.s);

        for parameter in ["RD=-1", "RS=-1"] {
            let error = parse_netlist(&format!("J1 d g s JMOD\n.model JMOD NJF({parameter})\n"))
                .expect_err("negative JFET terminal resistance is invalid");
            assert!(
                error.to_string().contains("RD/RS >= 0 and finite"),
                "{error}"
            );
        }
        for parameter in ["RD=inf", "RS=nan"] {
            parse_netlist(&format!("J1 d g s JMOD\n.model JMOD NJF({parameter})\n"))
                .expect_err("nonfinite spellings are not SPICE numeric values");
        }

        let collision = parse_netlist("J1 d g s JMOD\nR1 J1#drain 0 1k\n.model JMOD NJF(RD=10)\n")
            .expect_err("a user node cannot alias an internal JFET terminal");
        assert!(collision.to_string().contains("J1#drain"), "{collision}");
    }

    #[test]
    fn nonpositive_bjt_high_injection_knees_are_rejected() {
        for parameter in ["IKF=0", "IKR=-1m"] {
            let error = parse_netlist(&format!("Q1 c b e QMOD\n.model QMOD NPN({parameter})\n"))
                .expect_err("a zero or negative high-injection knee is invalid");
            assert!(error.to_string().contains("IKF > 0"), "{error}");
            assert!(error.to_string().contains("IKR > 0"), "{error}");
        }
    }

    #[test]
    fn invalid_bjt_base_recombination_parameters_are_rejected() {
        for parameter in ["ISE=-1u", "ISC=-1u", "NE=0", "NC=-1"] {
            let error = parse_netlist(&format!("Q1 c b e QMOD\n.model QMOD NPN({parameter})\n"))
                .expect_err("invalid base recombination parameters must be refused");
            assert!(
                error
                    .to_string()
                    .contains("ISE >= 0, ISC >= 0, NE > 0 and NC > 0"),
                "{error}"
            );
        }
    }

    /// A MOSFET `LEVEL` other than 1 is the one parameter that is refused
    /// rather than ignored: solving a BSIM card with Shichman-Hodges would be
    /// wrong by orders of magnitude, not by a capacitance.
    #[test]
    fn a_mosfet_level_this_engine_cannot_honour_is_refused_by_name() {
        let err = parse_netlist("M1 d g s b M\n.model M NMOS(LEVEL=49 VTO=0.7)\n")
            .expect_err("level 49 is not level 1");
        assert!(err.to_string().contains("LEVEL"), "{err}");
        assert!(err.to_string().contains("labwired_ngspice.py"), "{err}");
    }

    /// The built-in cards let a part be dropped in with no model library at
    /// all, which is what the catalog will emit.
    #[test]
    fn builtin_semiconductor_models_need_no_model_line() {
        let circuit = parse_netlist(
            "D1 a 0 D\nQ1 c b e NPN\nQ2 c2 b2 e2 PNP\n\
             M1 d g s 0 NMOS\nM2 d2 g2 s2 0 PMOS\n\
             J1 jd jg js NJF\nJ2 jd2 jg2 js2 PJF\n",
        )
        .expect("built-in models resolve");
        assert_eq!(circuit.diodes[0].model.is, 2.52e-9);
        assert_eq!(circuit.bjts[0].model.polarity, Polarity::N);
        assert_eq!(circuit.bjts[1].model.polarity, Polarity::P);
        assert_eq!(circuit.mosfets[0].model.polarity, Polarity::N);
        assert_eq!(circuit.mosfets[1].model.polarity, Polarity::P);
        assert_eq!(circuit.mosfets[0].model.is, 1e-14);
        assert_eq!(circuit.mosfets[1].model.is, 1e-14);
        assert_eq!(circuit.mosfets[0].model.gamma, 0.0);
        assert_eq!(circuit.mosfets[0].model.phi, 0.6);
        // A PMOS threshold is negative, as in ngspice.
        assert!(circuit.mosfets[1].model.vto < 0.0);
        assert_eq!(circuit.jfets[0].model.polarity, Polarity::N);
        assert_eq!(circuit.jfets[0].model.vto, -2.0);
        assert_eq!(circuit.jfets[1].model.polarity, Polarity::P);
        assert_eq!(circuit.jfets[1].model.vto, 2.0);
        assert!(circuit.is_nonlinear());
    }

    #[test]
    fn mosfet_body_effect_parameters_are_parsed_and_require_positive_phi() {
        let circuit =
            parse_netlist("M1 d g s b MOD\n.model MOD NMOS(VTO=0.8 KP=20u GAMMA=0.42 PHI=0.65)\n")
                .expect("body-effect parameters parse");
        assert_eq!(circuit.mosfets[0].model.gamma, 0.42);
        assert_eq!(circuit.mosfets[0].model.phi, 0.65);

        let error = parse_netlist("M1 d g s b MOD\n.model MOD NMOS(GAMMA=0.4 PHI=0)\n")
            .expect_err("body effect requires a positive surface potential");
        assert!(error.to_string().contains("PHI > 0"), "{error}");
    }

    #[test]
    fn mosfet_overlap_parameters_scale_with_instance_width_and_length() {
        let circuit = parse_netlist(
            "M1 d g s b MOD w=200u l=2u\n\
             .model MOD NMOS(CGSO=2p CGDO=3p CGBO=4p W=100u L=1u)\n",
        )
        .expect("overlap parameters parse");
        let mosfet = &circuit.mosfets[0];
        assert_eq!(mosfet.model.cgso, 2e-12);
        assert_eq!(mosfet.model.cgdo, 3e-12);
        assert_eq!(mosfet.model.cgbo, 4e-12);
        assert_eq!(mosfet.w, 200e-6);
        assert_eq!(mosfet.l, 2e-6);
        for (actual, expected) in mosfet
            .overlap_capacitances()
            .into_iter()
            .zip([4e-16, 6e-16, 8e-18])
        {
            assert!((actual - expected).abs() < expected * 1e-12);
        }

        for parameter in ["CGSO=-1p", "CGDO=-1p", "CGBO=-1p"] {
            let error = parse_netlist(&format!("M1 d g s b MOD\n.model MOD NMOS({parameter})\n"))
                .expect_err("negative overlap capacitance is rejected");
            assert!(error.to_string().contains("CGSO/CGDO/CGBO"), "{error}");
        }
    }

    #[test]
    fn mosfet_charge_model_selects_ward_dutton_and_requires_tox() {
        let circuit =
            parse_netlist("M1 d g s b MOD\n.model MOD NMOS(VTO=1 TOX=10n CHARGEMOD=WARD_DUTTON)\n")
                .expect("Ward-Dutton channel charge parses");
        assert_eq!(
            circuit.mosfets[0].model.channel_charge_model,
            labwired_core::analog::MosChannelChargeModel::WardDutton
        );

        let default = parse_netlist("M1 d g s b MOD\n.model MOD NMOS(TOX=10n)\n")
            .expect("Meyer remains the default model");
        assert_eq!(
            default.mosfets[0].model.channel_charge_model,
            labwired_core::analog::MosChannelChargeModel::Meyer
        );

        for setting in ["CHARGEMOD=NOPE", "CHARGEMOD=1"] {
            let error = parse_netlist(&format!(
                "M1 d g s b MOD\n.model MOD NMOS(TOX=10n {setting})\n"
            ))
            .expect_err("unknown channel charge mode is rejected");
            assert!(error.to_string().contains("CHARGEMOD"), "{error}");
        }
        let error = parse_netlist("M1 d g s b MOD\n.model MOD NMOS(CHARGEMOD=WARD_DUTTON)\n")
            .expect_err("charge-conserving mode needs an oxide thickness");
        assert!(error.to_string().contains("needs `TOX`"), "{error}");
    }

    #[test]
    fn mosfet_bulk_junction_capacitance_parameters_parse_and_validate() {
        let circuit = parse_netlist(
            "M1 d g s b MOD AD=2e-12 AS=3e-12 PD=4e-6 PS=5e-6\n\
             .model MOD NMOS(CBD=2p CBS=3p CJ=4e-3 CJSW=5e-7 PB=0.9 MJ=0.4 MJSW=0.3 FC=0.3)\n",
        )
        .expect("bulk junction capacitance parameters parse");
        let model = circuit.mosfets[0].model;
        assert_eq!(model.cbd, 2e-12);
        assert_eq!(model.cbs, 3e-12);
        assert!(model.cbd_given);
        assert!(model.cbs_given);
        assert_eq!(model.cj, 4e-3);
        assert_eq!(model.cjsw, 5e-7);
        assert_eq!(model.pb, 0.9);
        assert_eq!(model.mj, 0.4);
        assert_eq!(model.mjsw, 0.3);
        assert_eq!(model.fc, 0.3);
        let mosfet = &circuit.mosfets[0];
        assert_eq!(
            [mosfet.ad, mosfet.as_, mosfet.pd, mosfet.ps],
            [2e-12, 3e-12, 4e-6, 5e-6]
        );

        let defaults = parse_netlist("M1 d g s b MOD\n.model MOD PMOS\n")
            .expect("MOSFET junction capacitance defaults parse");
        let model = defaults.mosfets[0].model;
        assert_eq!(model.cbd, 0.0);
        assert_eq!(model.cbs, 0.0);
        assert!(!model.cbd_given);
        assert!(!model.cbs_given);
        assert_eq!(model.cj, 0.0);
        assert_eq!(model.cjsw, 0.0);
        assert_eq!(model.pb, 0.8);
        assert_eq!(model.mj, 0.5);
        assert_eq!(model.mjsw, 0.5);
        assert_eq!(model.fc, 0.5);

        for parameter in [
            "CBD=-1f",
            "CBS=-1f",
            "CJ=-1f",
            "CJSW=-1f",
            "PB=0",
            "MJ=-0.1",
            "MJSW=-0.1",
            "FC=-0.1",
            "FC=1",
        ] {
            let error = parse_netlist(&format!("M1 d g s b MOD\n.model MOD NMOS({parameter})\n"))
                .expect_err("invalid bulk junction capacitance parameter is rejected");
            assert!(error.to_string().contains("CBD/CBS"), "{error}");
        }

        for parameter in ["AD=-1", "AS=-1", "PD=-1", "PS=-1"] {
            let error = parse_netlist(&format!("M1 d g s b MOD {parameter}\n.model MOD NMOS\n"))
                .expect_err("negative diffusion geometry is rejected");
            assert!(error.to_string().contains("ad, as, pd and ps"), "{error}");
        }

        let direct_zero = parse_netlist("M1 d g s b MOD AD=1e-6\n.model MOD NMOS(CBD=0 CJ=1e-3)\n")
            .expect("explicit zero CBD can override area-derived capacitance");
        let mosfet = &direct_zero.mosfets[0];
        assert_eq!(mosfet.bulk_junction_capacitances(true), [0.0, 0.0]);

        let overflow =
            parse_netlist("M1 d g s b MOD PD=1e308\n.model MOD NMOS(CBD=1e308 CJSW=1)\n")
                .expect_err("the combined effective junction capacitance must be finite");
        assert!(overflow
            .to_string()
            .contains("effective junction capacitances"));
    }

    #[test]
    fn mosfet_terminal_resistances_create_internal_nodes_and_validate() {
        let circuit = parse_netlist("M1 d g s 0 MMOD\n.model MMOD NMOS(RD=12 RS=3)\n")
            .expect("MOSFET terminal resistances parse");
        let mosfet = &circuit.mosfets[0];
        assert_eq!(mosfet.model.rd, 12.0);
        assert_eq!(mosfet.model.rs, 3.0);
        assert_eq!(circuit.node_count(), 5);
        assert_eq!(
            mosfet.junction_d,
            circuit.node("M1#drain").expect("drain junction exists")
        );
        assert_eq!(
            mosfet.junction_s,
            circuit.node("M1#source").expect("source junction exists")
        );
        assert_ne!(mosfet.junction_d, mosfet.d);
        assert_ne!(mosfet.junction_s, mosfet.s);

        let defaults = parse_netlist("M1 d g s 0 MMOD\n.model MMOD PMOS\n")
            .expect("omitted MOSFET terminal resistances default to zero");
        let mosfet = &defaults.mosfets[0];
        assert_eq!(mosfet.model.rd, 0.0);
        assert_eq!(mosfet.model.rs, 0.0);
        assert_eq!(defaults.node_count(), 3);
        assert_eq!(mosfet.junction_d, mosfet.d);
        assert_eq!(mosfet.junction_s, mosfet.s);

        for parameter in ["RD=-1", "RS=-1"] {
            let error = parse_netlist(&format!("M1 d g s 0 MMOD\n.model MMOD NMOS({parameter})\n"))
                .expect_err("negative MOSFET terminal resistance is invalid");
            assert!(
                error.to_string().contains("RD/RS/RSH >= 0 and finite"),
                "{error}"
            );
        }
        for parameter in ["RD=inf", "RS=nan"] {
            parse_netlist(&format!("M1 d g s 0 MMOD\n.model MMOD NMOS({parameter})\n"))
                .expect_err("nonfinite terminal resistance is invalid");
        }

        let collision =
            parse_netlist("M1 d g s 0 MMOD\nR1 M1#drain 0 1k\n.model MMOD NMOS(RD=10)\n")
                .expect_err("a user node cannot alias an internal MOSFET terminal");
        assert!(collision.to_string().contains("M1#drain"), "{collision}");
    }

    #[test]
    fn mosfet_sheet_resistance_uses_instance_square_counts() {
        let circuit = parse_netlist("M1 d g s 0 MMOD nrd=2 nrs=4\n.model MMOD NMOS(RSH=3)\n")
            .expect("MOSFET sheet resistance and instance square counts parse");
        let mosfet = &circuit.mosfets[0];
        assert_eq!(mosfet.model.rsh, 3.0);
        assert_eq!(mosfet.drain_resistance_ohms, 6.0);
        assert_eq!(mosfet.source_resistance_ohms, 12.0);
        assert_ne!(mosfet.junction_d, mosfet.d);
        assert_ne!(mosfet.junction_s, mosfet.s);

        let defaults = parse_netlist("M1 d g s 0 MMOD\n.model MMOD NMOS(RSH=3)\n")
            .expect("omitted NRD/NRS default to one square");
        assert_eq!(defaults.mosfets[0].drain_resistance_ohms, 3.0);
        assert_eq!(defaults.mosfets[0].source_resistance_ohms, 3.0);

        let direct_overrides_sheet =
            parse_netlist("M1 d g s 0 MMOD nrd=2 nrs=4\n.model MMOD NMOS(RD=7 RS=8 RSH=3)\n")
                .expect("direct terminal resistances take precedence");
        assert_eq!(direct_overrides_sheet.mosfets[0].drain_resistance_ohms, 7.0);
        assert_eq!(
            direct_overrides_sheet.mosfets[0].source_resistance_ohms,
            8.0
        );

        let explicit_zero_disables_sheet =
            parse_netlist("M1 d g s 0 MMOD nrd=2 nrs=4\n.model MMOD NMOS(RD=0 RS=0 RSH=3)\n")
                .expect("explicit zero terminal resistances disable the sheet path");
        assert_eq!(
            explicit_zero_disables_sheet.mosfets[0].drain_resistance_ohms,
            0.0
        );
        assert_eq!(
            explicit_zero_disables_sheet.mosfets[0].source_resistance_ohms,
            0.0
        );

        for parameter in ["RSH=-1", "RSH=inf", "RSH=nan", "RSH=1e999"] {
            let error = parse_netlist(&format!("M1 d g s 0 MMOD\n.model MMOD NMOS({parameter})\n"))
                .expect_err("invalid sheet resistance is rejected");
            if parameter == "RSH=-1" || parameter == "RSH=1e999" {
                assert!(
                    error.to_string().contains("RD/RS/RSH >= 0 and finite"),
                    "{error}"
                );
            }
        }
        for parameter in ["NRD=-1", "NRS=inf", "NRS=nan", "NRS=1e999"] {
            let error = parse_netlist(&format!(
                "M1 d g s 0 MMOD {parameter}\n.model MMOD NMOS(RSH=3)\n"
            ))
            .expect_err("invalid diffusion square count is rejected");
            if parameter == "NRD=-1" || parameter == "NRS=1e999" {
                assert!(
                    error.to_string().contains("nrd and nrs must be >= 0"),
                    "{error}"
                );
            }
        }
    }

    #[test]
    fn jfet_model_parameters_are_parsed_and_unsupported_levels_are_refused() {
        let circuit = parse_netlist(
            "J1 d g s MOD\n.model MOD NJF(VTO=-1.8 BETA=2m LAMBDA=0.02 IS=4p B=1.2)\n",
        )
        .expect("level-1 JFET parameters parse");
        let model = circuit.jfets[0].model;
        assert_eq!(model.vto, -1.8);
        assert_eq!(model.beta, 2e-3);
        assert_eq!(model.lambda, 0.02);
        assert_eq!(model.b, 1.2);
        assert_eq!(model.is, 4e-12);

        for (card, rejected) in [("NJF(LEVEL=2)", "LEVEL=2")] {
            let err = parse_netlist(&format!("J1 d g s MOD\n.model MOD {card}\n"))
                .expect_err("unsupported JFET equations must be refused");
            assert!(err.to_string().contains(rejected), "{err}");
            assert!(err.to_string().contains("labwired_ngspice.py"), "{err}");
        }

        for b in ["0", "-0.1", "1e999"] {
            let err = parse_netlist(&format!("J1 d g s MOD\n.model MOD NJF(B={b})\n"))
                .expect_err("non-positive or non-finite JFET B must be refused");
            assert!(err.to_string().contains("B > 0 and finite"), "{err}");
        }
    }

    /// `W` and `L` on the element line override the card, and `beta` is what
    /// the solver actually stamps.
    #[test]
    fn mosfet_width_and_length_come_from_the_element_line_when_it_gives_them() {
        let circuit =
            parse_netlist("M1 d g s 0 M w=200u l=2u\n.model M NMOS(KP=20u)\n").expect("w/l parse");
        assert_eq!(circuit.mosfets[0].beta, 20e-6 * 200e-6 / 2e-6);

        let defaulted =
            parse_netlist("M1 d g s 0 M\n.model M NMOS(KP=20u W=10u L=1u)\n").expect("card w/l");
        assert_eq!(defaulted.mosfets[0].beta, 20e-6 * 10e-6 / 1e-6);
    }

    /// An element that names a card of the wrong type is a typo worth catching,
    /// not a silent reinterpretation.
    #[test]
    fn an_element_naming_the_wrong_kind_of_model_is_rejected() {
        let err = parse_netlist("Q1 c b e DMOD\n.model DMOD D(IS=1n)\n")
            .expect_err("a diode card is not a BJT card");
        assert!(err.to_string().contains("Q1"), "{err}");
        assert!(err.to_string().contains("D model"), "{err}");

        let missing =
            parse_netlist("D1 a 0 NOSUCH\n").expect_err("an undeclared, non-built-in model");
        assert!(missing.to_string().contains("NOSUCH"), "{missing}");
    }

    /// `RS > 0` costs an internal node, which is an unknown the caller can see
    /// in `unknowns()` and probe by name.
    #[test]
    fn a_diode_series_resistance_adds_exactly_one_internal_node() {
        let plain = parse_netlist("D1 a 0 DM\n.model DM D(IS=1n RS=0)\n").expect("parses");
        let with_rs = parse_netlist("D1 a 0 DM\n.model DM D(IS=1n RS=1)\n").expect("parses");
        assert_eq!(with_rs.node_count(), plain.node_count() + 1);
        assert_eq!(with_rs.diodes[0].anode, plain.diodes[0].anode);
        assert_ne!(with_rs.diodes[0].junction_anode, with_rs.diodes[0].anode);
        assert_eq!(plain.diodes[0].junction_anode, plain.diodes[0].anode);
    }

    // -----------------------------------------------------------------------
    // Transient sources
    // -----------------------------------------------------------------------

    /// `SIN` and `PULSE` with SPICE's own argument defaults, and the value at
    /// `t = 0` that the operating point is solved with.
    #[test]
    fn transient_source_functions_parse_with_spice_semantics() {
        let circuit = parse_netlist(
            "Vsin a 0 SIN(1 5 1k)\n\
             Vpulse b 0 PULSE(0 3.3 1u 10n 10n 5u 10u)\n\
             Vdc c 0 dc 2\n\
             Vbare d 0 7\n",
        )
        .expect("source functions parse");

        assert!(!circuit.voltage_sources[0].wave.is_constant());
        assert_eq!(
            circuit.voltage_sources[0].dc, 1.0,
            "SIN starts at its offset"
        );
        // A quarter period after t = 0 the sine is at its peak.
        let quarter = circuit.voltage_sources[0].wave.at(250e-6);
        assert!((quarter - 6.0).abs() < 1e-9, "got {quarter}");

        let pulse = circuit.voltage_sources[1].wave;
        assert_eq!(circuit.voltage_sources[1].dc, 0.0, "PULSE starts at v1");
        assert_eq!(pulse.at(0.0), 0.0);
        assert_eq!(pulse.at(1e-6), 0.0, "the delay ends at the first edge");
        assert!(
            (pulse.at(1.005e-6) - 1.65).abs() < 1e-12,
            "half way up the rising edge, got {}",
            pulse.at(1.005e-6)
        );
        assert_eq!(pulse.at(5e-6), 3.3, "the flat top");
        assert_eq!(pulse.at(11e-6), 0.0, "one period on, back at the start");
        assert_eq!(
            pulse.at(1_001e-6),
            0.0,
            "and still in phase 100 periods later"
        );

        assert!(circuit.voltage_sources[2].wave.is_constant());
        assert_eq!(circuit.voltage_sources[3].wave, Waveform::Dc(7.0));
        assert!(circuit.has_waveforms());
        assert!(!parse_netlist("V1 a 0 dc 5\n").unwrap().has_waveforms());
    }

    /// A pulse whose edges and top do not fit inside its period is a typo that
    /// would otherwise run as a waveform the author did not draw.
    #[test]
    fn a_pulse_that_cannot_fit_its_period_is_rejected() {
        let err = parse_netlist("V1 a 0 PULSE(0 5 0 1u 1u 9u 10u)\n")
            .expect_err("tr + pw + tf exceeds per");
        assert!(err.to_string().contains("per"), "{err}");
        parse_netlist("V1 a 0 PULSE(0 5 0 1u 1u 8u 10u)\n").expect("exactly fitting is fine");
    }

    // -----------------------------------------------------------------------
    // Analysis cards
    // -----------------------------------------------------------------------

    /// One deck has to drive both engines, so ngspice's analysis and scripting
    /// cards are skipped rather than refused. They are not acted on: this
    /// engine's run length and outputs come from the manifest.
    #[test]
    fn ngspice_analysis_cards_and_control_blocks_are_skipped() {
        let circuit = parse_netlist(
            "* a deck that ngspice can also run\n\
             Vin in 0 SIN(0 5 1k)\n\
             R1 in 0 1k\n\
             .options temp=26.85 reltol=1e-9\n\
             .control\n\
             tran 1u 2m\n\
             linearize v(in)\n\
             wrdata out.data v(in)\n\
             .endc\n\
             .end\n",
        )
        .expect("analysis cards are skipped");
        assert_eq!(circuit.resistors.len(), 1);
        assert_eq!(circuit.voltage_sources.len(), 1);
        assert_eq!(
            circuit.node_count(),
            1,
            "no node came from the control block"
        );

        // A directive that is NOT on the skip list is still a hard error, so a
        // typo cannot silently drop an element line.
        let err = parse_netlist("R1 a 0 1k\n.subckt thing a b\n").expect_err("unknown directive");
        assert!(matches!(err, AnalogError::Unsupported { .. }), "{err}");
    }
}
