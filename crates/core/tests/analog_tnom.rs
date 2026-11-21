// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `.options tnom=` in the in-core engine.
//!
//! `tnom` is ngspice's `CKTnomTemp`: the `TNOM` every model card and every
//! `tc1`/`tc2` resistor takes when it does not set its own. The engine used to
//! skip `.options` whole, so a deck written with `.options tnom=50` ran every
//! card at `TNOM` 27 °C. The references are ngspice-45 `.op` on the same text
//! with `reltol=1e-12 vntol=1e-15 abstol=1e-18` and `set numdgt=12`.

mod analog {
    use labwired_core::analog::{parse_netlist, Integration, Solver};

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

    const PROBES: [&str; 5] = ["a", "b", "c", "d", "sj"];

    /// Every probe of [`deck`]: the five node voltages, then `i(v1)`, `i(v2)`.
    fn probes(solver: &Solver) -> [f64; 7] {
        let mut out = [0.0; 7];
        for (slot, node) in out.iter_mut().zip(PROBES) {
            *slot = voltage(solver, node);
        }
        out[5] = branch(solver, "v1");
        out[6] = branch(solver, "v2");
        out
    }

    /// The `.temp` parity deck of `analog_temp.rs`: one device per temperature
    /// law. `card` is appended inside every `.model` card and `resistor` to
    /// the `R1` line, so a test can set `TNOM` per card instead of per deck;
    /// `directive` is written after the elements.
    fn deck(card: &str, resistor: &str, directive: &str) -> String {
        format!(
            "\
* .options tnom parity deck
V1 in 0 5
R1 in a 1k tc1=3m tc2=1u {resistor}
D1 a 0 DMOD
V2 vcc 0 5
RB vcc b 1Meg
Q1 c b 0 QMOD
RC vcc c 1k
M1 d g 0 0 NMOD w=10u l=1u
VG g 0 2
RD vcc d 2k
J1 dj 0 sj JMOD
RSJ sj 0 1k
RDJ vcc dj 1k
.model DMOD D(IS=1e-14 N=1 EG=1.11 XTI=3 RS=1 {card})
.model QMOD NPN(IS=1e-16 BF=100 XTB=1.5 EG=1.11 XTI=3 {card})
.model NMOD NMOS(level=1 VTO=0.7 KP=100u PHI=0.6 {card})
.model JMOD NJF(VTO=-2 BETA=1m BETATCE=-0.5 VTOTC=-2m {card})
{directive}
.options reltol=1e-12 vntol=1e-15 abstol=1e-18
.control
set numdgt=12
op
print v(a) v(b) v(c) v(d) v(sj) i(v1) i(v2)
.endc
.end
"
        )
    }

    /// ngspice-45 with `.options tnom=50` and no `.temp` (so 27 °C).
    const NGSPICE_TNOM50: [f64; 7] = [
        7.834318270055e-01,
        8.334537194508e-01,
        4.627026642845e+00,
        3.172489769491e+00,
        1.006530007185e+00,
        -4.52650231286e-03,
        -2.29742503088e-03,
    ];

    /// ngspice-45 with `.options tnom=50` and `.temp 85`.
    const NGSPICE_TNOM50_85C: [f64; 7] = [
        6.977730694604e-01,
        7.722858016275e-01,
        4.506715890476e+00,
        3.478676734133e+00,
        9.864490288458e-01,
        -3.88910658369e-03,
        -2.24462249148e-03,
    ];

    /// ngspice-45 at `.temp 27` with the default `TNOM` 27 °C.
    const NGSPICE_TNOM27: [f64; 7] = [
        6.971647361883e-01,
        7.520858289113e-01,
        4.575208267379e+00,
        3.309999993360e+00,
        9.999999996633e-01,
        -4.30283526381e-03,
        -2.27403965480e-03,
    ];

    const NAMES: [&str; 7] = ["v(a)", "v(b)", "v(c)", "v(d)", "v(sj)", "i(v1)", "i(v2)"];

    fn assert_matches(what: &str, got: [f64; 7], reference: [f64; 7], tolerance: f64) {
        for ((name, got), reference) in NAMES.iter().zip(got).zip(reference) {
            assert!(
                ((got - reference) / reference.abs()).abs() < tolerance,
                "{what} {name}: ngspice-45 {reference:e}, got {got:e}"
            );
        }
    }

    /// `.options tnom=50` moves every probe the way ngspice moves it, with
    /// and without `.temp`. Ignoring the option again would leave every card
    /// at `TNOM` 27 °C: 86 mV off on `v(a)` and 137 mV off on `v(d)`.
    #[test]
    fn options_tnom_reaches_every_device_law_like_ngspice() {
        let room = probes(&build(&deck("", "", ".options tnom=50")));
        assert_matches(".options tnom=50", room, NGSPICE_TNOM50, 1e-6);
        let hot = probes(&build(&deck("", "", ".options tnom=50\n.temp 85")));
        assert_matches(
            ".options tnom=50 at .temp 85",
            hot,
            NGSPICE_TNOM50_85C,
            1e-6,
        );
        // Each probe really is TNOM-sensitive, so the match above cannot come
        // from a law the option never reached.
        for ((name, shifted), nominal) in NAMES.iter().zip(NGSPICE_TNOM50).zip(NGSPICE_TNOM27) {
            assert!(
                ((shifted - nominal) / nominal).abs() > 5e-3,
                "{name} does not discriminate: {shifted:e} vs {nominal:e}"
            );
        }
    }

    /// `.options tnom=` is exactly a default `TNOM` on each card and `tnom` on
    /// each `tc1`/`tc2` resistor, wherever the card is written.
    #[test]
    fn options_tnom_is_bit_identical_to_card_tnom() {
        let cards = probes(&build(&deck("TNOM=50", "tnom=50", ".temp 27")));
        let after = probes(&build(&deck("", "", ".options tnom=50")));
        let before = probes(&build(&format!(".options tnom=50\n{}", deck("", "", ""))));
        assert_eq!(after, cards);
        assert_eq!(before, cards);
    }

    /// A card's own `TNOM` and a resistor's own `tnom` win over the option,
    /// as in ngspice.
    #[test]
    fn card_tnom_overrides_options_tnom() {
        let overridden = probes(&build(&deck("TNOM=27", "tnom=27", ".options tnom=50")));
        let plain = probes(&build(&deck("", "", ".temp 27")));
        assert_eq!(overridden, plain);
        assert_matches("TNOM=27 under tnom=50", overridden, NGSPICE_TNOM27, 1e-6);
    }

    /// `.option`, upper-case keys, spaces around `=`, other options on the
    /// same card and `.options temp=` all mean what ngspice reads.
    #[test]
    fn options_spellings_are_read_like_ngspice() {
        let reference = probes(&build(&deck("", "", ".options tnom=50\n.temp 85")));
        for directive in [
            ".option TNOM = 50 reltol=1e-3\n.temp 85",
            ".options temp=85 tnom=50",
            ".options gmin=1e-12 Temp= 85\n.options tnom =50",
        ] {
            assert_eq!(
                probes(&build(&deck("", "", directive))),
                reference,
                "{directive}"
            );
        }
    }

    /// Built-in parts keep the `TNOM` 27 °C their parameters were fitted at,
    /// and an NTC `beta` keeps its 25 °C datasheet nominal.
    #[test]
    fn options_tnom_leaves_builtin_parts_and_ntc_alone() {
        let builtin = |directive: &str| {
            voltage(
                &build(&format!("V1 in 0 5\nR1 in a 1k\nD1 a 0 D\n{directive}\n")),
                "a",
            )
        };
        assert_eq!(builtin(".options tnom=50"), builtin(".temp 27"));

        let ntc = voltage(
            &build("V1 in 0 3.3\nR1 in a 10k\nRT a 0 10k beta=3950 temp=25\n.options tnom=50\n"),
            "a",
        );
        assert!((ntc - 1.65).abs() < 1e-9, "{ntc}");
    }

    /// A temperature given twice, or one at or below absolute zero, is
    /// refused rather than reduced to one value.
    #[test]
    fn unrunnable_options_temperatures_are_refused() {
        let cases = [
            (
                ".temp 85\n.options temp=85\nR1 a 0 1\n",
                "`temp` is given twice",
            ),
            (".options temp=85\n.temp 85\nR1 a 0 1\n", "only one `.temp`"),
            (
                ".options tnom=50\n.options tnom=50\nR1 a 0 1\n",
                "`tnom` is given twice",
            ),
            (".options tnom=-300\nR1 a 0 1\n", "absolute zero"),
            (".options tnom=hot\nR1 a 0 1\n", "not a SPICE value"),
        ];
        for (text, needle) in cases {
            let err = parse_netlist(text).expect_err(text).to_string();
            assert!(err.contains(needle), "{text}: {err}");
        }
        // Options in `.control` are ngspice script, and other options are
        // still accepted and ignored.
        parse_netlist(".control\n.options tnom=-300\n.endc\nR1 a 0 1\n").expect("control block");
        parse_netlist(".options reltol=1e-3 noacct\nR1 a 0 1\n").expect("other options");
    }
}
