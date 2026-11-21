// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `.tf` DC small-signal transfer function in the in-core engine.
//!
//! A resistor network holds every output and input form to its closed form,
//! which ngspice-42 prints to the same digits. The device decks are each held
//! to ngspice-42 (`.options reltol=1e-9 abstol=1e-18 vntol=1e-12`,
//! `set numdgt=12`): a degenerated BJT common-emitter stage with `RB`/`RC`/`RE`,
//! `ISE` and `IKF`, a BJT collector current read through a `V` ammeter, a
//! MOSFET source follower with body effect, a diode's small-signal resistance
//! with `RS`, a JFET with terminal resistances, and an `E` feedback loop.
//! ngspice-42 uses CODATA 2014 `k` and `q`, about 3.4e-7 apart from this
//! engine's, so the device tables hold to 1e-5.

mod analog {
    use labwired_core::analog::{parse_netlist, Integration, Solver, TransferOutput};

    fn build(netlist: &str) -> Solver {
        let circuit = parse_netlist(netlist).unwrap_or_else(|error| panic!("{error}"));
        let mut solver = Solver::new(circuit, Integration::BackwardEuler)
            .unwrap_or_else(|error| panic!("{error}"));
        solver
            .solve_operating_point()
            .unwrap_or_else(|error| panic!("{error}"));
        solver
    }

    fn assert_close(what: &str, got: f64, expected: f64, relative: f64) {
        let error = ((got - expected) / expected).abs();
        assert!(
            error <= relative,
            "{what}: expected {expected:e}, got {got:e} (relative error {error:e})"
        );
    }

    /// `(transfer_function, input_impedance, output_impedance)` from
    /// ngspice-42 for the deck's own `.tf` card.
    fn assert_ngspice(deck: &str, expected: (f64, f64, f64), relative: f64) {
        let tf = build(deck)
            .transfer_function()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_close("transfer_function", tf.transfer, expected.0, relative);
        assert_close("input_impedance", tf.input_impedance, expected.1, relative);
        assert_close(
            "output_impedance",
            tf.output_impedance,
            expected.2,
            relative,
        );
    }

    const HEADER: &str = ".options temp=26.85 tnom=26.85\n";

    const DIVIDER: &str = "* 1k/3k divider, 500 R to an I source\n\
                           V1 in 0 DC 1\n\
                           R1 in out 1k\n\
                           R2 out 0 3k\n\
                           R3 out x 500\n\
                           I1 0 x DC 1m\n";

    fn divider(output: TransferOutput, input: &str) -> (f64, f64, f64) {
        let tf = build(DIVIDER)
            .transfer_analysis(&output, input)
            .unwrap_or_else(|error| panic!("{error}"));
        (tf.transfer, tf.input_impedance, tf.output_impedance)
    }

    fn v(node: &str, reference: Option<&str>) -> TransferOutput {
        TransferOutput::Voltage {
            node: node.to_string(),
            reference: reference.map(str::to_string),
        }
    }

    fn i(element: &str) -> TransferOutput {
        TransferOutput::Current(element.to_string())
    }

    #[test]
    fn a_resistor_network_gives_every_output_and_input_form_in_closed_form() {
        // Every value is also what ngspice-42 prints for the same card.
        let close = |what: &str, got: (f64, f64, f64), expected: (f64, f64, f64)| {
            assert_close(&format!("{what} transfer"), got.0, expected.0, 1e-12);
            assert_close(&format!("{what} input"), got.1, expected.1, 1e-12);
            assert_close(&format!("{what} output"), got.2, expected.2, 1e-12);
        };
        // V1 sees R1 + R2; `out` sees R1 ∥ R2 with V1 shorted.
        close(
            "v(out) V1",
            divider(v("out", None), "V1"),
            (0.75, 4e3, 750.0),
        );
        // I1 drives 1 A into `x`; the open source leaves R3 in series with
        // R1 ∥ R2, and `v(x, out)` sees R3 alone.
        close(
            "v(x,out) I1",
            divider(v("x", Some("out")), "I1"),
            (500.0, 1250.0, 500.0),
        );
        close(
            "v(out) I1",
            divider(v("out", None), "I1"),
            (750.0, 1250.0, 750.0),
        );
        // V1 delivers 1/4k A, which flows out of its `+` terminal, so its
        // branch current (`+` to `-` through the source) is negative; the
        // same source as output reuses the input resistance.
        close("i(V1) V1", divider(i("V1"), "V1"), (-2.5e-4, 4e3, 4e3));
        // 3/4 of I1's current returns through V1 from `+` to `-`. A unit
        // voltage in series with V1 drives it through R1 + R2: 4k. ngspice-42
        // prints 1e20 here, because `tfanal.c` takes `1/max(1e-20, I)`.
        close("i(V1) I1", divider(i("V1"), "I1"), (0.75, 1250.0, 4e3));
    }

    #[test]
    fn the_deck_card_reads_split_probes_and_an_inductor_branch() {
        // `v(x, out)` split at the blank, and `L1` (a short at DC) carries
        // all of `I1`'s current to `x`.
        let deck = format!("{DIVIDER}L1 x y 1m\nR4 y 0 1k\n.tf v(x, out) I1\n");
        let tf = build(&deck).transfer_function().unwrap();
        // 1 A into `x` splits between R4 (1k) and R3 + R1 ∥ R2 (1250).
        let rx = 1e3 * 1250.0 / 2250.0;
        assert_close("transfer", tf.transfer, rx * 500.0 / 1250.0, 1e-12);
        assert_close("input", tf.input_impedance, rx, 1e-12);
        assert_close(
            "output",
            tf.output_impedance,
            500.0 * 1750.0 / 2250.0,
            1e-12,
        );
        // A unit voltage in series with L1 sees R4 + (R3 + R1 ∥ R2): the
        // inductor's branch row takes the same sign as a source's.
        let tf = build(&format!("{DIVIDER}L1 x y 1m\nR4 y 0 1k\n.tf i(L1) V1\n"))
            .transfer_function()
            .unwrap();
        assert_close("i(L1) output", tf.output_impedance, 2250.0, 1e-12);
        // `out` sees R1 from V1 and R2 ∥ (R3 + L1 + R4) to ground; the
        // current runs from `x` to `y` through L1.
        let rp = 3e3 * 1500.0 / 4500.0;
        let vout = rp / (1e3 + rp);
        assert_close("i(L1) transfer", tf.transfer, vout / 1500.0, 1e-12);
    }

    #[test]
    fn a_degenerated_bjt_stage_matches_ngspice() {
        assert_ngspice(
            &format!(
                "* common emitter, divider bias, 1 meg source\n{HEADER}\
                 VCC vcc 0 DC 12\n\
                 VIN in 0 DC 0\n\
                 RB1 vcc b 47k\n\
                 RB2 b 0 10k\n\
                 CIN in b 1u\n\
                 RS in b 1meg\n\
                 RC vcc c 4.7k\n\
                 RE e 0 1k\n\
                 Q1 c b e QN\n\
                 .model QN NPN(IS=1e-14 BF=150 VAF=80 RB=50 RC=2 RE=0.5 ISE=1e-13 NE=1.6 IKF=0.05)\n\
                 .tf v(c) VIN\n"
            ),
            (-3.51384863490e-02, 1.007762615716e+06, 4.672396776314e+03),
            1e-5,
        );
    }

    #[test]
    fn a_bjt_collector_current_read_through_an_ammeter_matches_ngspice() {
        let deck = format!(
            "* gm of a degenerated BJT\n{HEADER}\
             VCC vcc 0 DC 12\n\
             VM vcc c DC 0\n\
             VB b 0 DC 0.7\n\
             RE e 0 100\n\
             Q1 c b e QN\n\
             .model QN NPN(IS=1e-14 BF=150 VAF=80 RB=50 RC=2 RE=0.5)\n\
             .tf i(VM) VB\n"
        );
        let tf = build(&deck).transfer_function().unwrap();
        assert_close("transfer", tf.transfer, 6.943764248003e-03, 1e-5);
        assert_close("input", tf.input_impedance, 2.462927419787e+04, 1e-5);
        // ngspice-42 prints 1e20 (see the divider test); the collector's
        // own output resistance, about VAF/IC plus degeneration, is finite.
        assert!(
            tf.output_impedance > 1e5 && tf.output_impedance.is_finite(),
            "collector output resistance {}",
            tf.output_impedance
        );
    }

    #[test]
    fn a_mosfet_source_follower_with_body_effect_matches_ngspice() {
        let tf = build(&format!(
            "* source follower, bulk at ground\n{HEADER}\
             VDD vdd 0 DC 5\n\
             VG g 0 DC 3\n\
             M1 vdd g s 0 NM w=20u l=2u\n\
             RS s 0 10k\n\
             .model NM NMOS(VTO=0.8 KP=50u GAMMA=0.5 PHI=0.65 LAMBDA=0.03)\n\
             .tf v(s) VG\n"
        ))
        .transfer_function()
        .unwrap();
        // The body effect is what holds the gain below gm/(gm + 1/RS).
        assert_close("transfer", tf.transfer, 6.852947048004e-01, 1e-5);
        assert_close("output", tf.output_impedance, 1.840859349542e+03, 1e-5);
        // ngspice-42 prints 1e20: the gate draws no current.
        assert_eq!(tf.input_impedance, f64::INFINITY);
    }

    #[test]
    fn a_diode_small_signal_resistance_matches_ngspice() {
        assert_ngspice(
            &format!(
                "* 1 mA into a diode\n{HEADER}\
                 I1 0 a DC 1m\n\
                 D1 a 0 DX\n\
                 .model DX D(IS=1e-14 N=1.5 RS=2)\n\
                 .tf v(a) I1\n"
            ),
            (4.077798655367e+01, 4.077798655367e+01, 4.077798655367e+01),
            1e-5,
        );
    }

    #[test]
    fn a_jfet_stage_with_terminal_resistances_matches_ngspice() {
        assert_ngspice(
            &format!(
                "* JFET common source\n{HEADER}\
                 VDD vdd 0 DC 10\n\
                 VG g 0 DC -0.5\n\
                 RD vdd d 2k\n\
                 J1 d g s JN\n\
                 RS s 0 200\n\
                 .model JN NJF(VTO=-2 BETA=1m LAMBDA=0.02 RD=10 RS=10)\n\
                 .tf v(d) VG\n"
            ),
            (-3.27921586400e+00, 2.019673536933e+11, 1.931965146997e+03),
            1e-5,
        );
    }

    #[test]
    fn an_e_feedback_loop_matches_ngspice() {
        let tf = build(
            "* non-inverting gain of 10 around a 1e5 VCVS\n\
             VIN in 0 DC 0.1\n\
             E1 out 0 inp fb 1e5\n\
             R0 in inp 1k\n\
             R1 fb 0 1k\n\
             R2 out fb 9k\n\
             RL out 0 100\n\
             .tf v(out) VIN\n",
        )
        .transfer_function()
        .unwrap();
        // `A/(1 + A/10)`; the conductances come out of the `C/h`
        // subtraction `.ac` uses, which leaves about 1e-12.
        assert_close("transfer", tf.transfer, 1e5 / (1.0 + 1e4), 1e-9);
        // An ideal `E` output and an input that draws nothing: ngspice-42
        // prints 0 and 1e20.
        assert_eq!(tf.output_impedance, 0.0);
        assert_eq!(tf.input_impedance, f64::INFINITY);
    }

    #[test]
    fn the_transient_after_tf_is_bit_identical() {
        let deck = "* RC with a diode\n\
                    V1 in 0 PULSE(0 2 1u 1u 1u 5u 10u)\n\
                    R1 in a 1k\n\
                    D1 a 0 D\n\
                    C1 a 0 1n\n\
                    .tf v(a) V1\n";
        let run = |analyse: bool| {
            let mut solver = build(deck);
            if analyse {
                solver.transfer_function().unwrap();
            }
            let node = solver.circuit().node("a").unwrap();
            (0..40)
                .map(|_| {
                    solver.advance(1e-7).unwrap();
                    solver.node_voltage(node).to_bits()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(true), run(false));
    }

    #[test]
    fn malformed_and_unresolvable_cards_are_refused() {
        let parse = |card: &str| parse_netlist(&format!("{DIVIDER}{card}\n"));
        for bad in [
            ".tf v(out) R1",
            ".tf v(out)",
            ".tf v(out) V1 I1",
            ".tf q(out) V1",
            ".tf i(V1, out) V1",
            ".tf v(out,) V1",
        ] {
            assert!(parse(bad).is_err(), "`{bad}` was accepted");
        }
        let twice = parse(".tf v(out) V1\n.tf v(x) I1").unwrap_err().to_string();
        assert!(twice.contains("only one `.tf`"), "{twice}");

        let mut solver = build(DIVIDER);
        assert!(solver.transfer_function().is_err());
        for (output, input) in [
            (v("nowhere", None), "V1"),
            (v("out", Some("out")), "V1"),
            (i("R1"), "V1"),
            (v("out", None), "V9"),
        ] {
            assert!(
                solver.transfer_analysis(&output, input).is_err(),
                "{output:?} from {input} was accepted"
            );
        }
    }
}
