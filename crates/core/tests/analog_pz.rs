// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `.pz` pole-zero analysis in the in-core engine.
//!
//! Linear decks are held to their closed forms, which ngspice-42 prints to
//! the same digits. The twin-T notch is the case ngspice-42 gives up on
//! ("Pole-zero iteration limit reached"): it finds one of its three zeros,
//! so its `jω`-axis pair is held to the closed form only. Every deck is also
//! held to its own `.ac` response: `K·Π(jω − z)/Π(jω − p)` must equal the
//! `.ac` transfer at every frequency once `K` is set at one. The BJT and
//! MOSFET stages are held to ngspice-42 (`.options reltol=1e-9 abstol=1e-18
//! vntol=1e-12`, `set numdgt=12`) at 1e-5: ngspice-42 uses CODATA 2014 `k`
//! and `q`, about 3.4e-7 apart from this engine's.

mod analog {
    use labwired_core::analog::{
        parse_netlist, Integration, NodeRef, Phasor, PoleZeroResponse, Solver,
    };

    fn build(netlist: &str) -> Solver {
        let circuit = parse_netlist(netlist).unwrap_or_else(|error| panic!("{error}"));
        let mut solver = Solver::new(circuit, Integration::BackwardEuler)
            .unwrap_or_else(|error| panic!("{error}"));
        solver
            .solve_operating_point()
            .unwrap_or_else(|error| panic!("{error}"));
        solver
    }

    fn pz(netlist: &str) -> PoleZeroResponse {
        build(netlist)
            .pole_zero()
            .unwrap_or_else(|error| panic!("{error}"))
    }

    /// Each expected root `(re, im)` matched to a distinct found root within
    /// `relative` of the expected magnitude (or `1e-9` absolute at zero).
    fn assert_roots(what: &str, got: &[Phasor], expected: &[(f64, f64)], relative: f64) {
        assert_eq!(
            got.len(),
            expected.len(),
            "{what}: got {got:?}, expected {expected:?}"
        );
        let mut unused: Vec<&Phasor> = got.iter().collect();
        for &(re, im) in expected {
            let tolerance = (relative * re.hypot(im)).max(1e-9);
            let position = unused
                .iter()
                .position(|root| (root.re - re).hypot(root.im - im) <= tolerance)
                .unwrap_or_else(|| {
                    panic!("{what}: no root near ({re:e}, {im:e}) within {tolerance:e} in {got:?}")
                });
            unused.remove(position);
        }
    }

    fn quadratic_roots(a: f64, b: f64, c: f64) -> [(f64, f64); 2] {
        let discriminant = b * b - 4.0 * a * c;
        if discriminant >= 0.0 {
            let root = discriminant.sqrt();
            [
                ((-b - root) / (2.0 * a), 0.0),
                ((-b + root) / (2.0 * a), 0.0),
            ]
        } else {
            let root = (-discriminant).sqrt() / (2.0 * a);
            [(-b / (2.0 * a), -root), (-b / (2.0 * a), root)]
        }
    }

    fn mul(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
        (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
    }

    fn div(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
        let d = b.0 * b.0 + b.1 * b.1;
        ((a.0 * b.0 + a.1 * b.1) / d, (a.1 * b.0 - a.0 * b.1) / d)
    }

    /// `Π(jω − z)/Π(jω − p)` at `hertz`.
    fn rational(response: &PoleZeroResponse, hertz: f64) -> (f64, f64) {
        let s = (0.0, 2.0 * std::f64::consts::PI * hertz);
        let mut value = (1.0, 0.0);
        for zero in &response.zeros {
            value = mul(value, (s.0 - zero.re, s.1 - zero.im));
        }
        for pole in &response.poles {
            value = div(value, (s.0 - pole.re, s.1 - pole.im));
        }
        value
    }

    /// The `.ac` transfer `v(out)/AC drive` must equal `K·Π(jω − z)/Π(jω − p)`
    /// at every frequency, with `K` taken at the first.
    fn assert_matches_ac(netlist: &str, output: &str, frequencies: &[f64]) {
        let mut solver = build(netlist);
        let output: NodeRef = solver.circuit().node(output).expect("output node");
        let response = solver.pole_zero().unwrap_or_else(|error| panic!("{error}"));
        let ac = solver
            .ac_analysis(frequencies)
            .unwrap_or_else(|error| panic!("{error}"));
        let mut gain = None;
        for (point, &hertz) in frequencies.iter().enumerate() {
            let measured = ac.node_voltage(point, output);
            let model = rational(&response, hertz);
            let ratio = div((measured.re, measured.im), model);
            let k = *gain.get_or_insert(ratio);
            let error = (ratio.0 - k.0).hypot(ratio.1 - k.1) / k.0.hypot(k.1);
            assert!(
                error < 1e-8,
                "at {hertz} Hz: .ac {measured:?} vs K·rational {:?} (relative error {error:e})",
                mul(k, model)
            );
        }
    }

    const RC_LADDER: &str = "* two-section RC ladder, voltage in\n\
                             v1 in 0 dc 0 ac 1\n\
                             r1 in out 1k\n\
                             c1 out 0 1u\n\
                             r2 out o2 2k\n\
                             c2 o2 0 1n\n\
                             .pz in 0 o2 0 vol pz\n\
                             .end\n";

    #[test]
    fn rc_ladder_poles_match_the_closed_form_and_ngspice() {
        let (r1, c1, r2, c2) = (1e3, 1e-6, 2e3, 1e-9);
        let poles = quadratic_roots(r1 * c1 * r2 * c2, r1 * c1 + r2 * c2 + r1 * c2, 1.0);
        let response = pz(RC_LADDER);
        assert_roots("poles", &response.poles, &poles, 1e-12);
        // ngspice-42 prints exactly these.
        assert_roots(
            "poles",
            &response.poles,
            &[(-5.00501000999e+05, 0.0), (-9.98999001005e+02, 0.0)],
            1e-11,
        );
        assert!(response.zeros.is_empty(), "{:?}", response.zeros);
        assert_matches_ac(RC_LADDER, "o2", &[10.0, 1e3, 1e5]);
    }

    const TWIN_T: &str = "* twin-T notch, current in\n\
                          i1 0 in dc 0 ac 1\n\
                          rs in 0 10k\n\
                          r1 in a 10k\n\
                          r2 a out 10k\n\
                          c3 a 0 2n\n\
                          c1 in b 1n\n\
                          c2 b out 1n\n\
                          r3 b 0 5k\n\
                          rl out 0 100k\n\
                          .pz in 0 out 0 cur pz\n\
                          .end\n";

    #[test]
    fn twin_t_notch_zeros_lie_on_the_jw_axis() {
        // The two-port transfer admittance is −(1 + s²R²C²)/(2R(1 + sRC));
        // its internal-node determinant adds (1 + sRC)², so the zeros are
        // ±j/RC and −1/RC.
        let omega = 1.0 / (10e3 * 1e-9);
        let response = pz(TWIN_T);
        assert_roots(
            "zeros",
            &response.zeros,
            &[(-omega, 0.0), (0.0, -omega), (0.0, omega)],
            1e-9,
        );
        // ngspice-42's poles (it finds only the −1/RC zero).
        assert_roots(
            "poles",
            &response.poles,
            &[
                (-1.51839903364e+05, 0.0),
                (-1.00000000000e+05, 0.0),
                (-1.67875476168e+04, 0.0),
            ],
            1e-10,
        );
        assert_matches_ac(TWIN_T, "out", &[100.0, 15_000.0, 16_000.0, 1e6]);
    }

    const RLC_AND_ISOLATED_RC: &str = "* series RLC with an RC the input cannot reach\n\
                                       v1 in 0 ac 1\n\
                                       r1 in a 50\n\
                                       l1 a out 1m\n\
                                       c1 out 0 1u\n\
                                       r2 out 0 1k\n\
                                       rx x 0 1k\n\
                                       cx x 0 1n\n\
                                       .pz in 0 out 0 vol pz\n\
                                       .end\n";

    #[test]
    fn complex_poles_and_an_unreachable_mode_reported_as_both() {
        // Input shorted: (R1 + sL)(sC + 1/R2) + 1 = 0.
        let (r1, l, c, r2) = (50.0, 1e-3, 1e-6, 1e3);
        let [low, high] = quadratic_roots(l * c, r1 * c + l / r2, r1 / r2 + 1.0);
        let isolated = (-1.0 / (1e3 * 1e-9), 0.0);
        let response = pz(RLC_AND_ISOLATED_RC);
        assert_roots("poles", &response.poles, &[low, high, isolated], 1e-12);
        // ngspice-42 prints both, and the isolated mode as a zero too.
        assert_roots(
            "poles",
            &response.poles,
            &[
                (-1e6, 0.0),
                (-2.55e4, 1.999374902313e4),
                (-2.55e4, -1.99937490231e4),
            ],
            1e-11,
        );
        assert_roots("zeros", &response.zeros, &[isolated], 1e-12);
        // Conjugates in order, negative imaginary part first.
        assert!(response.poles[0].im < 0.0 && response.poles[1].im > 0.0);
        assert_matches_ac(RLC_AND_ISOLATED_RC, "out", &[100.0, 3e3, 1e5]);
    }

    #[test]
    fn a_zero_at_dc_takes_the_shifted_pencil() {
        // C–R high-pass: v(out)/v(in) = sRC/(1 + sRC).
        let deck = "v1 in 0 ac 1\n\
                    c1 in out 1u\n\
                    r1 out 0 1k\n\
                    .pz in 0 out 0 vol pz\n";
        let response = pz(deck);
        assert_roots("poles", &response.poles, &[(-1e3, 0.0)], 1e-12);
        assert_roots("zeros", &response.zeros, &[(0.0, 0.0)], 0.0);
        assert_matches_ac(deck, "out", &[1.0, 159.0, 1e4]);
    }

    #[test]
    fn a_v_source_across_the_input_is_taken_out() {
        // As ngspice: with `cur`, the port is open where `v1` was, so the
        // pole is 1/((r0 + r1)·c1); with `vol` it is shorted, 1/(r1·c1).
        let deck = |drive: &str| {
            format!(
                "v1 in 0 dc 2 ac 1\n\
                 c1 in out 1u\n\
                 r1 out 0 1k\n\
                 r0 in 0 10k\n\
                 .pz in 0 out 0 {drive} pz\n"
            )
        };
        let open = pz(&deck("cur"));
        assert_roots("cur poles", &open.poles, &[(-1.0 / 11e-3, 0.0)], 1e-12);
        assert_roots("cur zeros", &open.zeros, &[(0.0, 0.0)], 0.0);
        let shorted = pz(&deck("vol"));
        assert_roots("vol poles", &shorted.poles, &[(-1e3, 0.0)], 1e-12);
        assert_roots("vol zeros", &shorted.zeros, &[(0.0, 0.0)], 0.0);
        // A `V` elsewhere stays a short: `r0` to ground through `v2`.
        let elsewhere = pz("c1 in out 1u\n\
                            r1 out 0 1k\n\
                            r0 in y 10k\n\
                            v2 y 0 dc 0\n\
                            .pz in 0 out 0 cur pz\n");
        assert_roots("poles", &elsewhere.poles, &[(-1.0 / 11e-3, 0.0)], 1e-12);
    }

    const BJT_MILLER: &str = ".options temp=26.85 tnom=26.85\n\
                              vcc vcc 0 dc 10\n\
                              vin src 0 dc 0.7 ac 1\n\
                              rs src b 1k\n\
                              q1 c b 0 qn\n\
                              rc vcc c 2k\n\
                              .model qn npn is=1e-15 bf=100 cje=2p cjc=1p vaf=50\n\
                              .pz src 0 c 0 vol pz\n";

    #[test]
    fn bjt_miller_stage_matches_ngspice() {
        // The right-half-plane zero is gm/Cμ: the feed-forward through CJC.
        let response = pz(BJT_MILLER);
        assert_roots(
            "poles",
            &response.poles,
            &[(-8.30104215165e+09, 0.0), (-5.11367790703e+07, 0.0)],
            1e-5,
        );
        assert_roots("zeros", &response.zeros, &[(4.871343408938e+10, 0.0)], 1e-5);
        assert_matches_ac(BJT_MILLER, "c", &[1e5, 1e7, 1e9]);
    }

    const MOS_COMMON_SOURCE: &str = ".options temp=26.85 tnom=26.85\n\
                                     vdd vdd 0 dc 5\n\
                                     vin g0 0 dc 1.5 ac 1\n\
                                     rg g0 g 10k\n\
                                     m1 d g 0 0 nm l=1u w=10u\n\
                                     rd vdd d 5k\n\
                                     cl d 0 1p\n\
                                     .model nm nmos level=1 vto=0.7 kp=100u lambda=0.02 \
                                     cgso=1n cgdo=0.5n\n\
                                     .pz g0 0 d 0 vol pz\n";

    #[test]
    fn mosfet_common_source_matches_ngspice() {
        let response = pz(MOS_COMMON_SOURCE);
        assert_roots(
            "poles",
            &response.poles,
            &[(-6.96998655309e+09, 0.0), (-1.96762012039e+08, 0.0)],
            1e-5,
        );
        assert_roots("zeros", &response.zeros, &[(1.705426356077e+11, 0.0)], 1e-5);
        assert_matches_ac(MOS_COMMON_SOURCE, "d", &[1e5, 1e8, 1e10]);
    }

    #[test]
    fn pol_and_zer_select_and_bad_cards_are_refused() {
        let poles = pz(&RC_LADDER.replace("vol pz", "vol pol"));
        assert_eq!(poles.poles.len(), 2);
        assert!(poles.zeros.is_empty());
        let zeros = pz(&TWIN_T.replace("cur pz", "cur zer"));
        assert!(zeros.poles.is_empty());
        assert_eq!(zeros.zeros.len(), 3);

        for card in [
            ".pz in 0 out 0 vol",
            ".pz in 0 out 0 amp pz",
            ".pz in 0 out 0 vol both",
            ".pz in 0 out vol pz",
        ] {
            let deck = format!("v1 in 0 ac 1\nr1 in out 1k\nc1 out 0 1u\n{card}\n");
            assert!(parse_netlist(&deck).is_err(), "`{card}` should not parse");
        }
        let twice = RC_LADDER.replace(".end\n", ".pz in 0 out 0 vol pz\n");
        assert!(parse_netlist(&twice).is_err());

        let error = |deck: &str| build(deck).pole_zero().unwrap_err().to_string();
        assert!(
            error("v1 in 0 ac 1\nr1 in out 1k\nc1 out 0 1u\n.pz in 0 nowhere 0 vol pz\n")
                .contains("`nowhere`")
        );
        assert!(
            error("v1 in 0 ac 1\nr1 in out 1k\nc1 out 0 1u\n.pz in in out 0 vol pz\n")
                .contains("same node")
        );
        // A delay has no finite set of poles and zeros.
        assert!(error(
            "v1 in 0 ac 1\nr1 in a 50\nt1 a 0 out 0 z0=50 td=1n\nr2 out 0 50\n\
             .pz in 0 out 0 vol pz\n"
        )
        .contains("`T` line"));
        // The output does not depend on the input.
        assert!(error(
            "v1 in 0 ac 1\nr1 in 0 1k\nr2 out 0 1k\nc2 out 0 1u\n.pz in 0 out 0 vol zer\n"
        )
        .contains("no zeros"));
        // No card.
        assert!(error("v1 in 0 ac 1\nr1 in 0 1k\n").contains("no `.pz"));
    }

    #[test]
    fn pole_zero_leaves_the_transient_untouched() {
        let deck = BJT_MILLER.replace("dc 0.7 ac 1", "sin(0.7 0.01 1meg) ac 1");
        let mut plain = build(&deck);
        let mut probed = build(&deck);
        probed.pole_zero().unwrap_or_else(|error| panic!("{error}"));
        for _ in 0..50 {
            plain.advance(1e-8).unwrap();
            probed.advance(1e-8).unwrap();
        }
        for node in [Some(0), Some(1), Some(2)] {
            assert_eq!(
                plain.node_voltage(node).to_bits(),
                probed.node_voltage(node).to_bits()
            );
        }
    }
}
