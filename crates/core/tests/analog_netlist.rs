// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Netlist-parser contract: the SPICE subset the in-core analog engine accepts,
//! and the errors it reports for everything else.

// The `analog` module puts every test under `analog::`, so the design's
// verification command `cargo test -p labwired-core analog` runs all of them.
mod analog {
    use labwired_core::analog::{
        parse_netlist, parse_spice_value, AnalogError, Polarity, Waveform,
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
    fn parses_every_element_of_the_subset() {
        let text = "\
* every supported element
V1 a 0 dc 5
I1 b 0 dc 1m
R1 a b 1k
C1 b 0 10u ic=0.5
L1 b c 2.2m ic=-0.25
S1 c 0 gate ron=1.5 roff=1meg
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

        assert_eq!(circuit.node_ic.len(), 2);

        // 3 nodes (a, b, c) + 1 voltage source + 1 inductor branch current.
        assert_eq!(circuit.unknowns(), 5);
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
        // `D`, `Q` and `M` used to be on this list and are now in the subset.
        // What is left is what still has no in-core term: a subcircuit needs a
        // flattener, a model library needs a file loader, a JFET and a
        // controlled source need equations nobody has written here.
        for line in [
            "X1 a b subckt",
            "J1 d g s jfet",
            "E1 a b c d 10",
            "K1 L1 L2 0.9",
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

    /// Parameters that carry charge or temperature behaviour are accepted and
    /// dropped, so a card pasted off a datasheet runs. That is a deliberate
    /// choice and it has a cost, which `analog::device` states.
    #[test]
    fn unmodelled_model_parameters_are_accepted_and_ignored() {
        let circuit = parse_netlist(
            "D1 a 0 D1N4148\n\
             .model D1N4148 D(IS=2.52n RS=0.568 N=1.752 CJO=4p M=0.333 TT=11.54n \
             VJ=0.75 BV=100 IBV=100u EG=1.11 XTI=3)\n",
        )
        .expect("a vendor card parses");
        assert_eq!(circuit.diodes[0].model.is, 2.52e-9);
        assert_eq!(circuit.diodes[0].model.n, 1.752);
        assert_eq!(circuit.diodes[0].model.rs, 0.568);
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
    fn the_five_builtin_models_need_no_model_line() {
        let circuit = parse_netlist(
            "D1 a 0 D\nQ1 c b e NPN\nQ2 c2 b2 e2 PNP\n\
             M1 d g s 0 NMOS\nM2 d2 g2 s2 0 PMOS\n",
        )
        .expect("built-in models resolve");
        assert_eq!(circuit.diodes[0].model.is, 2.52e-9);
        assert_eq!(circuit.bjts[0].model.polarity, Polarity::N);
        assert_eq!(circuit.bjts[1].model.polarity, Polarity::P);
        assert_eq!(circuit.mosfets[0].model.polarity, Polarity::N);
        assert_eq!(circuit.mosfets[1].model.polarity, Polarity::P);
        // A PMOS threshold is negative, as in ngspice.
        assert!(circuit.mosfets[1].model.vto < 0.0);
        assert!(circuit.is_nonlinear());
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
