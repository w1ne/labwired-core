// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `.temp` in the in-core engine.
//!
//! A deck written for ngspice states its temperature once, with `.temp`, and
//! leaves every instance without `temp=`. The engine used to accept the card
//! and drop it, so such a deck silently ran every junction at the fixed 300 K
//! instead. `.temp` is now the default instance `TEMP`, so the deck below
//! reaches the diode `EG`/`XTI`, BJT `XTB`, MOSFET `KP`/`PHI`, JFET
//! `VTOTC`/`BETATCE` and resistor `TC1`/`TC2` laws without any per-instance
//! edit. The references are ngspice-45 `.op` on the same text with
//! `reltol=1e-12 vntol=1e-15 abstol=1e-18` and `set numdgt=12`, at model
//! `TNOM` 27 °C (ngspice's default `.options tnom`).

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

    /// One device per temperature law, each biased where its law moves the
    /// probe: a forward diode, an active-region NPN, a saturated NMOS and a
    /// self-biased JFET. `instance` is appended to every `R1`, `D1`, `Q1`,
    /// `M1` and `J1` line; `directive` is written after the elements, where
    /// vendor decks usually put it.
    fn deck(instance: &str, directive: &str) -> String {
        format!(
            "\
* .temp parity deck
V1 in 0 5
R1 in a 1k tc1=3m tc2=1u {instance}
D1 a 0 DMOD {instance}
V2 vcc 0 5
RB vcc b 1Meg
Q1 c b 0 QMOD {instance}
RC vcc c 1k
M1 d g 0 0 NMOD w=10u l=1u {instance}
VG g 0 2
RD vcc d 2k
J1 dj 0 sj JMOD {instance}
RSJ sj 0 1k
RDJ vcc dj 1k
.model DMOD D(IS=1e-14 N=1 EG=1.11 XTI=3 RS=1)
.model QMOD NPN(IS=1e-16 BF=100 XTB=1.5 EG=1.11 XTI=3)
.model NMOD NMOS(level=1 VTO=0.7 KP=100u PHI=0.6)
.model JMOD NJF(VTO=-2 BETA=1m BETATCE=-0.5 VTOTC=-2m)
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

    /// ngspice-45 at `.temp 85`.
    const NGSPICE_85C: [f64; 7] = [
        5.953217347888e-01,
        6.752977676683e-01,
        4.436302054129e+00,
        3.585265504296e+00,
        9.752045475388e-01,
        -3.74113550713e-03,
        -2.25059446935e-03,
    ];

    /// ngspice-45 at `.temp 27`, the model `TNOM`.
    const NGSPICE_27C: [f64; 7] = [
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

    /// The deck's `.temp 85` moves every probe the way ngspice moves it.
    /// Dropping `.temp` again would leave all five devices at 300 K, which is
    /// 100 mV off on `v(a)` and 275 mV off on `v(d)`.
    #[test]
    fn dot_temp_reaches_every_device_law_like_ngspice() {
        let hot = probes(&build(&deck("", ".temp 85")));
        assert_matches(".temp 85", hot, NGSPICE_85C, 1e-6);
        let nominal = probes(&build(&deck("", ".temp 27")));
        assert_matches(".temp 27", nominal, NGSPICE_27C, 1e-6);
        // Each probe really is temperature-sensitive, so the match above
        // cannot come from a law the directive never reached.
        for ((name, hot), nominal) in NAMES.iter().zip(NGSPICE_85C).zip(NGSPICE_27C) {
            assert!(
                ((hot - nominal) / nominal).abs() > 5e-3,
                "{name} does not discriminate: {hot:e} vs {nominal:e}"
            );
        }
    }

    /// `.temp` is exactly a default `temp=` on each instance, wherever the
    /// card is written.
    #[test]
    fn dot_temp_is_bit_identical_to_instance_temp() {
        let instance = probes(&build(&deck("temp=85", "")));
        let after = probes(&build(&deck("", ".temp 85")));
        let before = probes(&build(&format!(".temp 85\n{}", deck("", ""))));
        assert_eq!(after, instance);
        assert_eq!(before, instance);
    }

    /// An instance `temp=` wins over `.temp`, as in ngspice.
    #[test]
    fn instance_temp_overrides_dot_temp() {
        let overridden = probes(&build(&deck("temp=27", ".temp 85")));
        let explicit = probes(&build(&deck("temp=27", "")));
        assert_eq!(overridden, explicit);
        assert_matches("temp=27 under .temp 85", overridden, NGSPICE_27C, 1e-6);
    }

    /// An NTC thermistor without its own `temp=` sits at the deck temperature.
    #[test]
    fn dot_temp_sets_ntc_temperature() {
        let ntc = |line: &str, directive: &str| {
            let solver = build(&format!(
                "V1 in 0 3.3\nR1 in a 10k\nRT a 0 10k beta=3950 {line}\n{directive}\n"
            ));
            voltage(&solver, "a")
        };
        let hot = ntc("", ".temp 50");
        assert_eq!(hot, ntc("temp=50", ""));
        // R(50 °C) = 10k·exp(3950·(1/323.15 − 1/298.15)) ≈ 3.588 kΩ.
        let r = 10e3 * (3950.0 * (1.0 / 323.15 - 1.0 / 298.15_f64)).exp();
        assert!((hot - 3.3 * r / (r + 10e3)).abs() < 1e-9, "{hot}");
        assert!((ntc("", "") - 1.65).abs() < 1e-9);
    }

    /// A sweep, a second card or a value at or below absolute zero is refused
    /// rather than reduced to one temperature.
    #[test]
    fn unrunnable_dot_temp_is_refused() {
        let cases = [
            (".temp 27 85\nR1 a 0 1\n", "only one `.temp`"),
            (".temp 27\n.temp 85\nR1 a 0 1\n", "only one `.temp`"),
            (".temp\nR1 a 0 1\n", "expected `.temp"),
            (".temp -273.15\nR1 a 0 1\n", "absolute zero"),
            (".temp hot\nR1 a 0 1\n", "not a SPICE value"),
        ];
        for (text, needle) in cases {
            let err = parse_netlist(text).expect_err(text).to_string();
            assert!(err.contains(needle), "{text}: {err}");
        }
        // A `.temp` inside `.control` is ngspice script, not the circuit card.
        parse_netlist(".control\n.temp 27 85\n.endc\nR1 a 0 1\n").expect("control block");
    }
}
