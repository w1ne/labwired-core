// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `.ac` small-signal analysis in the in-core engine.
//!
//! The analysis has no per-device AC code: `G` is the operating-point Newton
//! Jacobian and `C` is read back from the backward-Euler transient stamp. So
//! each test below takes a different element family (R/C, L/C, diode charge,
//! BJT and MOSFET with a Miller capacitance, BJT excess phase, a `LAPLACE`
//! source, a lossless `T` line) and holds the sweep to a closed form written from the device
//! equations, not from the engine. Operating points come from the solver; everything derived from
//! them (`gm`, `gd`, junction capacitance) is computed here.

mod analog {
    use labwired_core::analog::{
        parse_netlist, AcSweepKind, AnalogError, Integration, Phasor, Solver,
    };
    use std::f64::consts::PI;

    const GMIN: f64 = 1e-12;
    /// `kT/q` at the engine's nominal 300 K, from CODATA constants.
    const VT: f64 = 1.380_649e-23 * 300.0 / 1.602_176_634e-19;

    #[derive(Clone, Copy)]
    struct C64 {
        re: f64,
        im: f64,
    }

    impl C64 {
        fn new(re: f64, im: f64) -> Self {
            Self { re, im }
        }
        fn div(self, other: Self) -> Self {
            let d = other.re * other.re + other.im * other.im;
            Self::new(
                (self.re * other.re + self.im * other.im) / d,
                (self.im * other.re - self.re * other.im) / d,
            )
        }
    }

    fn build(netlist: &str) -> Solver {
        let circuit = parse_netlist(netlist).unwrap_or_else(|error| panic!("{error}"));
        Solver::new(circuit, Integration::BackwardEuler).unwrap_or_else(|error| panic!("{error}"))
    }

    fn node(solver: &Solver, name: &str) -> Option<usize> {
        solver
            .circuit()
            .node(name)
            .unwrap_or_else(|| panic!("node `{name}` exists"))
    }

    fn dc(solver: &Solver, name: &str) -> f64 {
        solver.node_voltage(node(solver, name))
    }

    fn assert_phasor(what: &str, got: Phasor, expected: C64, relative: f64) {
        let error = (got.re - expected.re).hypot(got.im - expected.im);
        let scale = expected.re.hypot(expected.im);
        assert!(
            error <= relative * scale,
            "{what}: expected {:e}{:+e}j, got {:e}{:+e}j (relative error {:e})",
            expected.re,
            expected.im,
            got.re,
            got.im,
            error / scale
        );
    }

    #[test]
    fn rc_low_pass_follows_its_closed_form_over_a_decade_sweep() {
        let mut solver = build(
            "* RC low-pass, fc = 159.15 Hz\n\
             V1 in 0 DC 0 AC 1\n\
             R1 in out 1k\n\
             C1 out 0 1u\n\
             .ac dec 10 1 100k\n",
        );
        let sweep = solver.circuit().ac_sweep.expect("deck has a sweep");
        assert_eq!(sweep.kind, AcSweepKind::Decade);
        let response = solver.ac_sweep().expect("RC sweep solves");
        let frequencies = response.frequencies();
        assert_eq!(frequencies.len(), 51, "1 Hz to 100 kHz at 10 per decade");
        assert!((frequencies[50] - 1e5).abs() < 1e-6);
        let out = node(&solver, "out");
        let input = node(&solver, "in");
        for (point, &f) in frequencies.iter().enumerate() {
            let wrc = 2.0 * PI * f * 1e3 * 1e-6;
            let expected = C64::new(1.0, 0.0).div(C64::new(1.0, wrc));
            assert_phasor(
                &format!("V(out) at {f} Hz"),
                response.node_voltage(point, out),
                expected,
                1e-9,
            );
            assert_phasor(
                "V(in)",
                response.node_voltage(point, input),
                C64::new(1.0, 0.0),
                1e-12,
            );
        }
        // At the corner the output is 3.01 dB down and 45° behind.
        let corner = solver.ac_analysis(&[1.0 / (2.0 * PI * 1e-3)]).unwrap();
        let v = corner.node_voltage(0, out);
        assert!((v.db() + 10.0 * 2f64.log10()).abs() < 1e-9, "{} dB", v.db());
        assert!((v.phase_degrees() + 45.0).abs() < 1e-9);
    }

    #[test]
    fn series_rlc_band_pass_peaks_at_resonance_with_its_q() {
        let (l, c, r) = (10e-3, 100e-9, 50.0);
        let mut solver = build(
            "* series RLC band-pass across R\n\
             V1 in 0 AC 1\n\
             L1 in a 10m\n\
             C1 a out 100n\n\
             R1 out 0 50\n\
             .ac lin 41 4k 6k\n",
        );
        let response = solver.ac_sweep().expect("RLC sweep solves");
        assert_eq!(response.frequencies().len(), 41);
        let out = node(&solver, "out");
        for (point, &f) in response.frequencies().iter().enumerate() {
            let w = 2.0 * PI * f;
            let expected = C64::new(r, 0.0).div(C64::new(r, w * l - 1.0 / (w * c)));
            assert_phasor(
                &format!("V(out) at {f} Hz"),
                response.node_voltage(point, out),
                expected,
                1e-9,
            );
            // The inductor's branch current is the loop current V(out)/R.
            let current = response.branch_current(point, 1);
            assert_phasor(
                "i(L1)",
                current,
                C64::new(expected.re / r, expected.im / r),
                1e-9,
            );
        }
        let f0 = 1.0 / (2.0 * PI * (l * c).sqrt());
        let q = (l / c).sqrt() / r;
        let at = solver
            .ac_analysis(&[
                f0,
                f0 * ((1.0 + 1.0 / (4.0 * q * q)).sqrt() + 1.0 / (2.0 * q)),
            ])
            .unwrap();
        assert!((at.node_voltage(0, out).magnitude() - 1.0).abs() < 1e-9);
        assert!(at.node_voltage(0, out).phase_degrees().abs() < 1e-6);
        // The upper half-power edge, f0·(√(1 + 1/4Q²) + 1/2Q), is 1/√2 and 45°.
        let edge = at.node_voltage(1, out);
        assert!((edge.magnitude() - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-9);
        assert!((edge.phase_degrees() + 45.0).abs() < 1e-6);
    }

    #[test]
    fn forward_diode_admittance_is_conductance_plus_diffusion_and_depletion_charge() {
        let (is, tt, cjo, vj, m, fc) = (1e-14, 10e-9, 2e-12, 0.8, 0.4, 0.5);
        let mut solver = build(
            "* 1 mA forward bias, 1 A AC test current\n\
             I1 0 a DC 1m AC 1\n\
             D1 a 0 DX\n\
             .model DX D(IS=1e-14 N=1 TT=10n CJO=2p VJ=0.8 M=0.4)\n",
        );
        let vd = dc(&solver, "a");
        let exponential = (vd / VT).exp();
        let id = is * (exponential - 1.0) + GMIN * vd;
        assert!(
            (id - 1e-3).abs() < 1e-12,
            "DC point carries 1 mA, got {id:e}"
        );
        assert!(
            vd > fc * vj,
            "forward bias is in the linearised depletion region"
        );
        let gd = is * exponential / VT;
        let depletion = cjo * (1.0 - fc).powf(-1.0 - m) * (1.0 - fc * (1.0 + m) + m * vd / vj);
        let capacitance = tt * gd + depletion;
        let frequencies = [1e3, 1e6, 1e7, 1e8, 1e9];
        let response = solver.ac_analysis(&frequencies).unwrap();
        let a = node(&solver, "a");
        for (point, &f) in frequencies.iter().enumerate() {
            let expected = C64::new(1.0, 0.0).div(C64::new(gd + GMIN, 2.0 * PI * f * capacitance));
            assert_phasor(
                &format!("Z(D1) at {f} Hz"),
                response.node_voltage(point, a),
                expected,
                1e-7,
            );
        }
        // The diffusion pole sits near 1/(2π·TT): above it the junction is
        // capacitive, at 1 kHz it is the 25.9 Ω small-signal resistance.
        assert!((response.node_voltage(0, a).magnitude() - 1.0 / gd).abs() < 1e-3);
        assert!(response.node_voltage(4, a).phase_degrees() < -85.0);
    }

    #[test]
    fn reverse_biased_diode_depletion_sets_the_rc_pole() {
        let (cjo, vj, m) = (20e-12, 0.7, 0.5);
        let mut solver = build(
            "* 5 V reverse bias through 10k\n\
             V1 in 0 DC 0 AC 1\n\
             R1 in a 10k\n\
             VR k 0 DC 5\n\
             D1 a k DV\n\
             .model DV D(IS=1e-14 CJO=20p VJ=0.7 M=0.5)\n",
        );
        let vd = dc(&solver, "a") - dc(&solver, "k");
        assert!(vd < -4.99);
        let gd = 1e-14 * (vd / VT).exp() / VT + GMIN;
        let cj = cjo * (1.0 - vd / vj).powf(-m);
        let frequencies = [1e4, 1e5, 1e6, 1e7];
        let response = solver.ac_analysis(&frequencies).unwrap();
        let a = node(&solver, "a");
        for (point, &f) in frequencies.iter().enumerate() {
            let expected =
                C64::new(1.0, 0.0).div(C64::new(1.0 + 1e4 * gd, 2.0 * PI * f * 1e4 * cj));
            assert_phasor(
                &format!("V(a) at {f} Hz"),
                response.node_voltage(point, a),
                expected,
                1e-9,
            );
        }
    }

    /// `V(out)/V(in)` of a transconductor `gm` with Miller capacitance `cm`
    /// from input to output, loaded by `rl` and `cl` to AC ground.
    fn miller_stage(gm: f64, cm: f64, rl: f64, cl: f64, f: f64) -> C64 {
        let w = 2.0 * PI * f;
        C64::new(-gm, w * cm).div(C64::new(1.0 / rl, w * (cl + cm)))
    }

    #[test]
    fn bjt_common_emitter_gain_and_miller_zero_follow_gm_and_cjc() {
        let mut solver = build(
            "* common emitter, ideal base drive\n\
             VCC vcc 0 DC 10\n\
             VB b 0 DC 0.65 AC 1\n\
             RC vcc c 10k\n\
             CL c 0 1n\n\
             Q1 c b 0 QN\n\
             .model QN NPN(IS=1e-15 BF=100 CJC=2p VJC=0.75 MJC=0.33)\n",
        );
        let vbe = dc(&solver, "b");
        let vbc = vbe - dc(&solver, "c");
        assert!(vbc < -5.0, "forward active, Vbc = {vbc}");
        // Gummel–Poon with no VAF/VAR/IKF/IKR: qb = 1, so gm = IS·e^(Vbe/Vt)/Vt.
        let gm = 1e-15 * (vbe / VT).exp() / VT;
        let ic = (10.0 - dc(&solver, "c")) / 10e3;
        assert!((gm * VT / ic - 1.0).abs() < 1e-6, "gm = Ic/Vt at this bias");
        let cjc = 2e-12 * (1.0 - vbc / 0.75).powf(-0.33);
        let frequencies = [10.0, 1e3, 1e5, 1e7, 1e9];
        let response = solver.ac_analysis(&frequencies).unwrap();
        let c = node(&solver, "c");
        for (point, &f) in frequencies.iter().enumerate() {
            assert_phasor(
                &format!("Av at {f} Hz"),
                response.node_voltage(point, c),
                miller_stage(gm, cjc, 10e3, 1e-9, f),
                1e-6,
            );
        }
        let low = response.node_voltage(0, c);
        assert!((low.magnitude() - gm * 10e3).abs() < 1e-3 * gm * 10e3);
        assert!(
            (low.phase_degrees().abs() - 180.0).abs() < 0.1,
            "inverting stage"
        );
    }

    #[test]
    fn bjt_excess_phase_delays_the_transconductance_by_td() {
        // `td = PTF·π/180·TF` = 60° of a 1 ns TF. The base and emitter are
        // held by sources, so the diffusion charge `TF·gm` carries no signal
        // and the only reactive effect left is the delayed transport current.
        let deck = |ptf: f64| {
            format!(
                "VCC vcc 0 DC 10\n\
                 VB b 0 DC 0.65 AC 1\n\
                 RC vcc c 10k\n\
                 Q1 c b 0 QN\n\
                 .model QN NPN(IS=1e-15 BF=100 TF=1n PTF={ptf})\n"
            )
        };
        let td = PI / 3.0 * 1e-9;
        let quarter = 1.0 / (4.0 * td);
        let frequencies = [1e3, 1e7, quarter, 2.0 * quarter, 3.0 * quarter];
        let mut delayed = build(&deck(60.0));
        let mut plain = build(&deck(0.0));
        assert_eq!(
            dc(&delayed, "c").to_bits(),
            dc(&plain, "c").to_bits(),
            "excess phase does not move the operating point"
        );
        let gm = 1e-15 * (dc(&delayed, "b") / VT).exp() / VT;
        let with = delayed.ac_analysis(&frequencies).unwrap();
        let without = plain.ac_analysis(&frequencies).unwrap();
        let c = node(&delayed, "c");
        for (point, &f) in frequencies.iter().enumerate() {
            let phase = -2.0 * PI * f * td;
            let gain = -gm * 10e3;
            assert_phasor(
                &format!("Av at {f} Hz"),
                with.node_voltage(point, c),
                C64::new(gain * phase.cos(), gain * phase.sin()),
                1e-6,
            );
            assert_phasor(
                &format!("Av without PTF at {f} Hz"),
                without.node_voltage(point, c),
                C64::new(gain, 0.0),
                1e-6,
            );
        }
        // A quarter period of delay turns the inverting stage's 180° into 90°.
        let v = with.node_voltage(2, c);
        assert!(
            (v.phase_degrees() - 90.0).abs() < 1e-4,
            "{}",
            v.phase_degrees()
        );
    }

    #[test]
    fn bjt_excess_phase_stage_matches_ngspice() {
        // ngspice-42, `.options reltol=1e-9 abstol=1e-18 vntol=1e-12`. A
        // biased, degenerated common emitter with RB/RE/RC, Early effect,
        // junction and diffusion charge and 45° of excess phase: the delayed
        // `∂Ict/∂Vbe` sees the internal B-E voltage, not a source.
        let mut solver = build(
            "VCC vcc 0 DC 12\n\
             V1 in 0 DC 0 AC 1\n\
             RS in bx 50\n\
             CIN bx b 1u\n\
             RB1 vcc b 47k\n\
             RB2 b 0 10k\n\
             RC vcc c 2.2k\n\
             RE e 0 100\n\
             CL c 0 2p\n\
             Q1 c b e QX\n\
             .model QX NPN(IS=2e-15 BF=150 BR=3 VAF=80 NF=1 TF=0.4n PTF=45 CJE=1p \
             VJE=0.75 MJE=0.33 CJC=0.8p VJC=0.75 MJC=0.33 RB=20 RE=1 RC=5)\n\
             .ac lin 6 50Meg 550Meg\n",
        );
        let response = solver.ac_sweep().unwrap();
        let reference = [
            (
                -7.224129503422e-01,
                9.2435031176555e-01,
                5.7507726086594e-01,
                2.1613365822235e-02,
            ),
            (
                4.1627909588598e-01,
                8.3345320600280e-01,
                6.1744357686207e-01,
                -2.514952600779e-02,
            ),
            (
                7.2241820320780e-01,
                4.2564692847893e-01,
                6.0948674191322e-01,
                -8.231582335237e-02,
            ),
            (
                7.7859436244152e-01,
                1.2591845468282e-01,
                5.7964017077762e-01,
                -1.259574168403e-01,
            ),
            (
                7.3688699600021e-01,
                -9.512946570129e-02,
                5.3810168348448e-01,
                -1.530564029621e-01,
            ),
            (
                6.3998094937785e-01,
                -2.519274899597e-01,
                4.9292553430249e-01,
                -1.611025962185e-01,
            ),
        ];
        let (c, e) = (node(&solver, "c"), node(&solver, "e"));
        for (point, &(c_re, c_im, e_re, e_im)) in reference.iter().enumerate() {
            let f = response.frequencies()[point];
            // ngspice-42's k/q is CODATA 2014; the engine's is CODATA 2018.
            assert_phasor(
                &format!("V(c) at {f} Hz"),
                response.node_voltage(point, c),
                C64::new(c_re, c_im),
                1e-5,
            );
            assert_phasor(
                &format!("V(e) at {f} Hz"),
                response.node_voltage(point, e),
                C64::new(e_re, e_im),
                1e-5,
            );
        }
    }

    #[test]
    fn mosfet_common_source_gain_and_miller_zero_follow_gm_and_cgdo() {
        let (kp, w, l, vto, cgdo) = (50e-6, 10e-6, 2e-6, 1.0, 3e-9);
        let mut solver = build(
            "* common source, ideal gate drive\n\
             VDD vdd 0 DC 10\n\
             VG g 0 DC 2 AC 1\n\
             RD vdd d 10k\n\
             CL d 0 1n\n\
             M1 d g 0 0 MN W=10u L=2u\n\
             .model MN NMOS(VTO=1 KP=50u LAMBDA=0 CGDO=3n)\n",
        );
        let vgs = dc(&solver, "g");
        let vds = dc(&solver, "d");
        assert!(vds > vgs - vto, "saturation, Vds = {vds}");
        let beta = kp * w / l;
        let ids = (10.0 - vds) / 10e3;
        // GMIN on the drain junction leaks ~9 pA beside the 125 µA channel.
        let square_law = beta / 2.0 * (vgs - vto).powi(2);
        assert!((ids / square_law - 1.0).abs() < 1e-6, "Ids = {ids:e}");
        let gm = beta * (vgs - vto);
        let frequencies = [10.0, 1e4, 1e6, 1e8];
        let response = solver.ac_analysis(&frequencies).unwrap();
        let d = node(&solver, "d");
        for (point, &f) in frequencies.iter().enumerate() {
            assert_phasor(
                &format!("Av at {f} Hz"),
                response.node_voltage(point, d),
                miller_stage(gm, cgdo * w, 10e3, 1e-9, f),
                1e-6,
            );
        }
    }

    #[test]
    fn laplace_biquad_matches_its_transfer_function() {
        let (w0, q) = (2.0 * PI * 1e3, 5.0);
        let mut solver = build(
            "* second-order low-pass, f0 = 1 kHz, Q = 5\n\
             .param w0=6283.185307179586 q=5\n\
             V1 in 0 AC 1\n\
             E1 out 0 LAPLACE {V(in)} = {w0*w0/(s*s + w0/q*s + w0*w0)}\n\
             R1 out 0 1k\n\
             .ac oct 4 100 10k\n",
        );
        let response = solver.ac_sweep().unwrap();
        assert_eq!(
            response.frequencies().len(),
            27,
            "100 Hz to 10 kHz at 4 per octave"
        );
        let out = node(&solver, "out");
        for (point, &f) in response.frequencies().iter().enumerate() {
            let s_im = 2.0 * PI * f;
            let expected =
                C64::new(w0 * w0, 0.0).div(C64::new(w0 * w0 - s_im * s_im, w0 / q * s_im));
            assert_phasor(
                &format!("H at {f} Hz"),
                response.node_voltage(point, out),
                expected,
                1e-9,
            );
        }
        let peak = solver.ac_analysis(&[1e3]).unwrap().node_voltage(0, out);
        assert!((peak.magnitude() - q).abs() < 1e-9);
        assert!((peak.phase_degrees() + 90.0).abs() < 1e-9);
    }

    #[test]
    fn ac_phase_and_current_source_stimulus_are_honoured() {
        let mut solver = build(
            "V1 in 0 DC 3 AC 2 90\n\
             R1 in out 1k\n\
             R2 out 0 1k\n\
             I2 0 x SIN(0 1m 1k) AC 1m -45\n\
             R3 x 0 2k\n",
        );
        assert!(
            (dc(&solver, "out") - 1.5).abs() < 1e-12,
            "DC part unchanged"
        );
        let response = solver.ac_analysis(&[50.0]).unwrap();
        assert_phasor(
            "V(out)",
            response.node_voltage(0, node(&solver, "out")),
            C64::new(0.0, 1.0),
            1e-12,
        );
        let h = std::f64::consts::FRAC_1_SQRT_2 * 2.0;
        assert_phasor(
            "V(x)",
            response.node_voltage(0, node(&solver, "x")),
            C64::new(h, -h),
            1e-12,
        );
        // A source with no `AC` is a small-signal zero.
        let quiet = build("V1 in 0 DC 3\nR1 in 0 1k\n")
            .ac_analysis(&[1e3])
            .unwrap();
        assert_eq!(quiet.node_voltage(0, Some(0)), Phasor::default());
    }

    #[test]
    fn ac_analysis_leaves_the_transient_bit_identical() {
        let deck = "V1 in 0 PULSE(0 1 0 1u 1u 50u 100u) AC 1\n\
                    R1 in a 1k\n\
                    D1 a b DX\n\
                    C1 b 0 10n\n\
                    L1 b 0 1m\n\
                    .model DX D(IS=1e-14 TT=5n CJO=3p)\n";
        let mut plain = build(deck);
        let mut probed = build(deck);
        for step in 0..200 {
            if step % 37 == 0 {
                probed.ac_analysis(&[1e3, 1e6]).expect("AC mid-run");
            }
            plain.advance(1e-7).unwrap();
            probed.advance(1e-7).unwrap();
            for name in ["a", "b"] {
                assert_eq!(
                    dc(&plain, name).to_bits(),
                    dc(&probed, name).to_bits(),
                    "step {step}"
                );
            }
        }
    }

    #[test]
    fn ac_analysis_leaves_an_excess_phase_transient_bit_identical() {
        // `.ac` turns the Weil filter off while it reads `C`; the transient
        // that resumes must not see that, nor any change to its history.
        let deck = "VCC vcc 0 DC 10\n\
                    V1 in 0 PULSE(0.6 0.7 0 1n 1n 20n 50n) AC 1\n\
                    RB in b 100\n\
                    RC vcc c 1k\n\
                    Q1 c b 0 QN\n\
                    .model QN NPN(IS=1e-15 BF=100 TF=1n PTF=40 CJC=1p CJE=1p)\n";
        let mut plain = build(deck);
        let mut probed = build(deck);
        for step in 0..300 {
            if step % 29 == 0 {
                probed.ac_analysis(&[1e6, 1e8]).expect("AC mid-run");
            }
            plain.advance(2e-10).unwrap();
            probed.advance(2e-10).unwrap();
            for name in ["b", "c"] {
                assert_eq!(
                    dc(&plain, name).to_bits(),
                    dc(&probed, name).to_bits(),
                    "step {step}"
                );
            }
        }
    }

    fn mul(a: C64, b: C64) -> C64 {
        C64::new(a.re * b.re - a.im * b.im, a.re * b.im + a.im * b.re)
    }

    fn add(a: C64, b: C64) -> C64 {
        C64::new(a.re + b.re, a.im + b.im)
    }

    /// `e^{-jθ}`.
    fn delay(theta: f64) -> C64 {
        C64::new(theta.cos(), -theta.sin())
    }

    #[test]
    fn matched_line_delays_the_wave_and_never_reflects() {
        let td = 1e-9;
        let mut solver = build(
            "V1 in 0 AC 1\n\
             R1 in a 50\n\
             T1 a 0 b 0 Z0=50 TD=1n\n\
             R2 b 0 50\n",
        );
        // Up to 1.7 GHz the phase wraps more than once: a lumped stand-in
        // for the delay would drift from the exact phasor long before that.
        let frequencies = [0.0, 1e6, 125e6, 250e6, 500e6, 999e6, 1.7e9];
        let response = solver.ac_analysis(&frequencies).unwrap();
        let (a, b) = (node(&solver, "a"), node(&solver, "b"));
        for (point, &f) in frequencies.iter().enumerate() {
            let theta = 2.0 * PI * f * td;
            assert_phasor(
                &format!("V(a) at {f} Hz"),
                response.node_voltage(point, a),
                C64::new(0.5, 0.0),
                1e-12,
            );
            let wave = delay(theta);
            assert_phasor(
                &format!("V(b) at {f} Hz"),
                response.node_voltage(point, b),
                C64::new(0.5 * wave.re, 0.5 * wave.im),
                1e-12,
            );
        }
    }

    #[test]
    fn mismatched_line_with_a_reactive_load_follows_the_reflection_formula() {
        // Z0 = 75 between a 50 ohm source and 150 ohm || 10 pF: the input
        // impedance and the transfer both come from the load's reflection
        // coefficient carried round the line, `Γ·e^{-2jθ}`.
        let (z0, td, rs, rl, cl) = (75.0, 2e-9, 50.0, 150.0, 10e-12);
        let mut solver = build(
            "V1 in 0 DC 1 AC 1\n\
             R1 in a 50\n\
             T1 a 0 b 0 Z0=75 TD=2n\n\
             R2 b 0 150\n\
             C2 b 0 10p\n\
             .ac lin 25 1Meg 481Meg\n",
        );
        assert!(
            (dc(&solver, "b") - rl / (rs + rl)).abs() < 1e-12,
            "at DC the line is a wire"
        );
        let response = solver.ac_sweep().unwrap();
        let (a, b) = (node(&solver, "a"), node(&solver, "b"));
        let one = C64::new(1.0, 0.0);
        for (point, &f) in response.frequencies().iter().enumerate() {
            let omega = 2.0 * PI * f;
            let load = C64::new(rl, 0.0).div(C64::new(1.0, omega * rl * cl));
            let gamma = add(load, C64::new(-z0, 0.0)).div(add(load, C64::new(z0, 0.0)));
            let round_trip = mul(gamma, delay(2.0 * omega * td));
            let z_in = mul(
                C64::new(z0, 0.0),
                add(one, round_trip).div(add(one, C64::new(-round_trip.re, -round_trip.im))),
            );
            let v_a = z_in.div(add(z_in, C64::new(rs, 0.0)));
            let v_b = mul(
                v_a,
                mul(add(one, gamma), delay(omega * td)).div(add(one, round_trip)),
            );
            assert_phasor(
                &format!("V(a) at {f} Hz"),
                response.node_voltage(point, a),
                v_a,
                1e-10,
            );
            assert_phasor(
                &format!("V(b) at {f} Hz"),
                response.node_voltage(point, b),
                v_b,
                1e-10,
            );
        }
    }

    #[test]
    fn line_with_a_floating_reference_matches_ngspice() {
        // ngspice-42, `.options reltol=1e-9 abstol=1e-18 vntol=1e-12`. Port
        // 2's reference `r` sits on a divider, so the far-port `-` terms
        // carry signal; the delay is given as `NL / F`.
        let mut solver = build(
            "V1 in 0 DC 0 AC 1\n\
             R1 in a 30\n\
             T1 a 0 b r Z0=50 F=100Meg NL=0.25\n\
             R2 b r 100\n\
             C2 b r 5p\n\
             R3 r 0 20\n\
             R4 in r 100\n\
             .ac lin 6 10Meg 260Meg\n",
        );
        let response = solver.ac_sweep().unwrap();
        let reference = [
            (
                7.621585266987e-01,
                -4.68407280669e-02,
                9.316542642342e-01,
                -1.08275047044e-01,
            ),
            (
                5.472339950326e-01,
                -1.46356261264e-01,
                6.856639094927e-01,
                -6.96518372775e-01,
            ),
            (
                5.063951049873e-01,
                1.286537422697e-01,
                -1.24333982704e-01,
                -8.32672212122e-01,
            ),
            (
                7.595373041392e-01,
                8.715295602015e-02,
                -5.33190623981e-01,
                -3.06075599786e-01,
            ),
            (
                7.149042192339e-01,
                -1.49752687284e-01,
                -5.78480031186e-01,
                2.222403264524e-01,
            ),
            (
                4.214806175715e-01,
                -7.21267112376e-02,
                -1.78326316368e-01,
                8.224483704284e-01,
            ),
        ];
        let (a, b, r) = (node(&solver, "a"), node(&solver, "b"), node(&solver, "r"));
        for (point, &(a_re, a_im, b_re, b_im)) in reference.iter().enumerate() {
            let f = response.frequencies()[point];
            assert_phasor(
                &format!("V(a) at {f} Hz"),
                response.node_voltage(point, a),
                C64::new(a_re, a_im),
                1e-9,
            );
            assert_phasor(
                &format!("V(b) at {f} Hz"),
                response.node_voltage(point, b),
                C64::new(b_re, b_im),
                1e-9,
            );
            assert_phasor(
                &format!("V(r) at {f} Hz"),
                response.node_voltage(point, r),
                C64::new(1.0 / 6.0, 0.0),
                1e-12,
            );
        }
    }

    #[test]
    fn ac_analysis_leaves_a_line_transient_bit_identical() {
        let deck = "V1 in 0 PULSE(0 1 0 1n 1n 20n 50n) AC 1\n\
                    R1 in a 50\n\
                    T1 a 0 b 0 Z0=75 TD=3n\n\
                    R2 b 0 200\n";
        let mut plain = build(deck);
        let mut probed = build(deck);
        for step in 0..300 {
            if step % 41 == 0 {
                probed.ac_analysis(&[1e6, 1e8]).expect("AC mid-run");
            }
            plain.advance(2e-10).unwrap();
            probed.advance(2e-10).unwrap();
            for name in ["a", "b"] {
                assert_eq!(
                    dc(&plain, name).to_bits(),
                    dc(&probed, name).to_bits(),
                    "step {step}"
                );
            }
        }
    }

    #[test]
    fn ac_sweep_without_an_ac_directive_is_refused() {
        assert!(matches!(
            build("V1 in 0 AC 1\nR1 in 0 1k\n").ac_sweep(),
            Err(AnalogError::Config(message)) if message.contains(".ac")
        ));
    }

    #[test]
    fn ac_directive_and_source_syntax_errors_name_the_line() {
        for (deck, needle) in [
            ("V1 a 0 AC 1\nR1 a 0 1\n.ac log 10 1 1k\n", "dec|oct|lin"),
            ("V1 a 0 AC 1\nR1 a 0 1\n.ac dec 10 0 1k\n", "fstart"),
            ("V1 a 0 AC 1\nR1 a 0 1\n.ac dec 10 1k 1\n", "fstart"),
            ("V1 a 0 AC 1\nR1 a 0 1\n.ac dec 2.5 1 1k\n", "whole number"),
            (
                "V1 a 0 AC 1\nR1 a 0 1\n.ac dec 10 1 1k\n.ac lin 2 1 2\n",
                "one `.ac`",
            ),
            ("V1 a 0 DC 1 AC\nR1 a 0 1\n", "AC <magnitude>"),
            ("V1 a 0 AC 1 AC 2\nR1 a 0 1\n", "twice"),
        ] {
            match parse_netlist(deck) {
                Err(AnalogError::Parse { message, .. }) => {
                    assert!(message.contains(needle), "{deck:?}: {message}")
                }
                other => panic!("{deck:?}: expected a parse error, got {other:?}"),
            }
        }
        let circuit = parse_netlist("V1 a 0 SIN(0 1 1k) AC 0.5\nR1 a 0 1\n").unwrap();
        let source = &circuit.voltage_sources[0];
        assert!(!source.wave.is_constant(), "the SIN part survives");
        let ac = source.ac.expect("AC part is kept");
        assert_eq!((ac.magnitude, ac.phase_degrees), (0.5, 0.0));
    }
}
