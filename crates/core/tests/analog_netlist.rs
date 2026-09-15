// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Netlist-parser contract: the SPICE subset the in-core analog engine accepts,
//! and the errors it reports for everything else.

// The `analog` module puts every test under `analog::`, so the design's
// verification command `cargo test -p labwired-core analog` runs all of them.
mod analog {
    use labwired_core::analog::{parse_netlist, parse_spice_value, AnalogError};

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
        for line in [
            "D1 a b diode",
            "Q1 c b e npn",
            "M1 d g s b nmos",
            "X1 a b subckt",
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
}
