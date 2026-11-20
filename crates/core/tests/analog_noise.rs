// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `.noise` small-signal noise analysis in the in-core engine.
//!
//! One closed form holds the adjoint transfer, the temperature and the
//! input referral on an RC divider. The device decks are each held to an
//! ngspice-42 table (`.options reltol=1e-9 abstol=1e-18 vntol=1e-12`,
//! `set numdgt=12`): diode shot and `RS` noise, BJT collector and base shot
//! plus `RB`/`RC`/`RE` with junction and transit capacitance, MOSFET and JFET
//! channel thermal noise, and a current-source input with a differential
//! output. ngspice-42 uses CODATA 2014 `k` and `q`, 3.4e-7 apart from this
//! engine's in power, so the tables hold to 1e-5.

mod analog {
    use labwired_core::analog::{parse_netlist, Integration, NoiseMechanism, Solver};
    use std::f64::consts::PI;

    const BOLTZMANN: f64 = 1.380_649e-23;

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

    /// `[frequency, onoise_spectrum, inoise_spectrum]` rows and
    /// `(onoise_total, inoise_total)` from ngspice-42.
    fn assert_ngspice(deck: &str, table: &[[f64; 3]], totals: (f64, f64)) {
        let mut solver = build(deck);
        let noise = solver
            .noise_sweep()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(noise.frequencies().len(), table.len());
        for (point, &[hertz, output, input]) in table.iter().enumerate() {
            assert_close("frequency", noise.frequencies()[point], hertz, 1e-12);
            let at = format!("{hertz:e} Hz");
            assert_close(
                &format!("onoise at {at}"),
                noise.output_density(point).sqrt(),
                output,
                1e-5,
            );
            assert_close(
                &format!("inoise at {at}"),
                noise.input_density(point).sqrt(),
                input,
                1e-5,
            );
        }
        assert_close("onoise_total", noise.output_total(), totals.0, 1e-5);
        assert_close("inoise_total", noise.input_total(), totals.1, 1e-5);
    }

    const HEADER: &str = ".options temp=26.85 tnom=26.85\n";

    #[test]
    fn rc_divider_thermal_noise_follows_its_closed_form_at_the_deck_temperature() {
        let mut solver = build(
            "* 1k/3k divider with a 1 nF shunt at 75 C\n\
             .temp 75\n\
             V1 in 0 DC 0 AC 2\n\
             R1 in out 1k\n\
             R2 out 0 3k\n\
             C1 out 0 1n\n\
             .noise v(out) V1 dec 5 1k 10meg\n",
        );
        let noise = solver.noise_sweep().unwrap();
        assert_eq!(noise.frequencies().len(), 21);
        let kelvin = 75.0 + 273.15;
        let parallel = 1e3 * 3e3 / 4e3;
        for (point, &hertz) in noise.frequencies().iter().enumerate() {
            // Both resistors' currents see the same parallel RC; the sum is
            // `4kT·Rp` low-passed by `Rp·C`.
            let wrc = 2.0 * PI * hertz * parallel * 1e-9;
            let output = 4.0 * BOLTZMANN * kelvin * parallel / (1.0 + wrc * wrc);
            assert_close("output density", noise.output_density(point), output, 1e-9);
            // The gain is from a unit source: the `AC 2` does not count.
            let gain_squared = 0.75 * 0.75 / (1.0 + wrc * wrc);
            assert_close(
                "input density",
                noise.input_density(point),
                output / gain_squared,
                1e-9,
            );
        }
        let names: Vec<&str> = noise.sources().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["r1", "r2"]);
        assert!(noise
            .sources()
            .iter()
            .all(|s| s.mechanism == NoiseMechanism::Thermal));
        assert_close(
            "r1 current density",
            noise.sources()[0].density,
            4.0 * BOLTZMANN * kelvin / 1e3,
            1e-12,
        );
    }

    #[test]
    fn diode_shot_and_series_resistance_noise_match_ngspice() {
        let deck = format!(
            "{HEADER}V1 vcc 0 DC 5 AC 1\n\
             R1 vcc a 4.3k\n\
             D1 a 0 dmod\n\
             .model dmod D(IS=1e-14 N=1 RS=10)\n\
             .noise v(a) V1 dec 1 1k 100k\n"
        );
        assert_ngspice(
            &deck,
            &[
                [1e3, 6.137844906752e-10, 7.465968877321e-08],
                [1e4, 6.137844906752e-10, 7.465968877321e-08],
                [1e5, 6.137844906752e-10, 7.465968877321e-08],
            ],
            (1.931227814069e-07, 2.349112265609e-05),
        );
    }

    #[test]
    fn bjt_common_emitter_noise_matches_ngspice_through_its_poles() {
        let deck = format!(
            "{HEADER}VCC vcc 0 DC 10\n\
             VB bb 0 DC 0.75 AC 1\n\
             RS bb b 1k\n\
             Q1 c b e qm\n\
             RE e 0 100\n\
             RC vcc c 2k\n\
             .model qm NPN(IS=1e-15 BF=100 VAF=50 RB=50 RC=5 RE=1 CJE=2p CJC=1p TF=0.3n)\n\
             .noise v(c) VB dec 2 10k 1g\n"
        );
        assert_ngspice(
            &deck,
            &[
                [1.0e4, 5.743115855481e-08, 4.664218436558e-09],
                [3.162277660168e4, 5.743105850900e-08, 4.664218639012e-09],
                [1.0e5, 5.743005808059e-08, 4.664220663552e-09],
                [3.162277660168e5, 5.742005676472e-08, 4.664240908898e-09],
                [1.0e6, 5.732033945099e-08, 4.664443357181e-09],
                [3.162277660168e6, 5.635180538110e-08, 4.666467322052e-09],
                [1.0e7, 4.882695917612e-08, 4.686655423232e-09],
                [3.162277660168e7, 2.696034165741e-08, 4.883615973869e-09],
                [1.0e8, 1.246162849227e-08, 6.509895314631e-09],
                [3.162277660168e8, 8.149754916777e-09, 1.412917165794e-08],
                [1.0e9, 4.372862319894e-09, 2.578855169719e-08],
            ],
            (3.436416117509e-04, 9.256386485903e-04),
        );
    }

    #[test]
    fn mosfet_channel_and_terminal_resistance_noise_match_ngspice() {
        let deck = format!(
            "{HEADER}VDD vdd 0 DC 5\n\
             VG g 0 DC 2 AC 1\n\
             RD vdd d 5k\n\
             M1 d g 0 0 nm W=10u L=1u\n\
             .model nm NMOS(VTO=1 KP=50u LAMBDA=0.02 RD=20 RS=10)\n\
             .noise v(d) VG dec 1 1k 100k\n"
        );
        assert_ngspice(
            &deck,
            &[
                [1e3, 1.480589841504e-08, 5.701449834817e-09],
                [1e4, 1.480589841504e-08, 5.701449834817e-09],
                [1e5, 1.480589841504e-08, 5.701449834817e-09],
            ],
            (4.658567178842e-06, 1.793919310246e-06),
        );
    }

    #[test]
    fn jfet_channel_noise_matches_ngspice_with_gate_capacitance() {
        let deck = format!(
            "{HEADER}VDD vdd 0 DC 10\n\
             VG g 0 DC 0 AC 1\n\
             RD vdd d 2k\n\
             J1 d g s jm\n\
             RS s 0 500\n\
             .model jm NJF(VTO=-2 BETA=1m LAMBDA=0.01 RD=10 RS=10 CGS=2p CGD=1p)\n\
             .noise v(d) VG dec 2 1k 100meg\n"
        );
        assert_ngspice(
            &deck,
            &[
                [1.0e3, 9.686499979128e-09, 4.427500844342e-09],
                [3.162277660168e3, 9.686499978348e-09, 4.427500844223e-09],
                [1.0e4, 9.686499970543e-09, 4.427500843027e-09],
                [3.162277660168e4, 9.686499892493e-09, 4.427500831069e-09],
                [1.0e5, 9.686499111989e-09, 4.427500711488e-09],
                [3.162277660168e5, 9.686491306968e-09, 4.427499515683e-09],
                [1.0e6, 9.686413257937e-09, 4.427487557725e-09],
                [3.162277660168e6, 9.685632885604e-09, 4.427367987808e-09],
                [1.0e7, 9.677840939766e-09, 4.426173254050e-09],
                [3.162277660168e7, 9.601079419721e-09, 4.414321530171e-09],
                [1.0e8, 8.932445620400e-09, 4.304522560669e-09],
            ],
            (9.343778443294e-05, 4.431696539976e-05),
        );
    }

    #[test]
    fn current_source_input_and_differential_output_match_ngspice() {
        let deck = format!(
            "{HEADER}I1 0 n DC 1m AC 1\n\
             R1 n 0 1k\n\
             R2 n m 2k\n\
             C1 m 0 1u\n\
             .noise v(n, m) I1 dec 3 1 10k\n"
        );
        assert_ngspice(
            &deck,
            &[
                [1.0, 5.755668373858e-09, 4.581028978669e-10],
                [2.154434690032, 5.753189844268e-09, 2.126783725736e-10],
                [4.641588833613, 5.741755043292e-09, 9.881526176202e-11],
                [1.0e1, 5.690132793271e-09, 4.607804039851e-11],
                [2.154434690032e1, 5.477975358955e-09, 2.183859103129e-11],
                [4.641588833613e1, 4.853612848045e-09, 1.105656891614e-11],
                [1.0e2, 3.987098638307e-09, 6.770156179592e-12],
                [2.154434690032e2, 3.508288015081e-09, 5.419630726232e-12],
                [4.641588833613e2, 3.366013624902e-09, 5.081892736803e-12],
                [1.0e3, 3.332744594206e-09, 5.006146898899e-12],
                [2.154434690032e3, 3.325443546387e-09, 4.989677404250e-12],
                [4.641588833613e3, 3.323864255913e-09, 4.986122036642e-12],
                [1.0e4, 3.323523713039e-09, 4.985355723931e-12],
            ],
            (3.350439769606e-07, 5.896515572876e-10),
        );
    }

    #[test]
    fn bjt_noise_sources_are_named_as_ngspice_and_skip_the_substrate_junction() {
        let mut solver = build(&format!(
            "{HEADER}VCC vcc 0 DC 5\n\
             VB b 0 DC 0.7 AC 1\n\
             Q1 vcc b e qm\n\
             RE e 0 1k\n\
             .model qm NPN(IS=1e-15 BF=100 RB=50 RC=5 RE=1 CJS=1p ISS=1e-16)\n\
             .noise v(e) VB lin 1 1k 1k\n"
        ));
        let noise = solver.noise_sweep().unwrap();
        let names: Vec<&str> = noise.sources().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["re", "rc_q1", "rb_q1", "re_q1", "ic_q1", "ib_q1"]);
        let ic = &noise.sources()[4];
        assert_eq!(ic.mechanism, NoiseMechanism::Shot);
        // `2q·Ic`, with Ic from the solved emitter voltage.
        let emitter = solver.node_voltage(solver.circuit().node("e").unwrap());
        let ie = emitter / 1e3;
        let ib = noise.sources()[5].density / (2.0 * 1.602_176_634e-19);
        let icollector = ic.density / (2.0 * 1.602_176_634e-19);
        assert_close("Ic + Ib = Ie", icollector + ib, ie, 1e-6);
    }

    #[test]
    fn noise_analysis_leaves_the_transient_bit_identical() {
        let deck = "* CE stage with charge\n\
                    VCC vcc 0 DC 10\n\
                    VB bb 0 SIN(0.75 0.01 1meg) AC 1\n\
                    RS bb b 1k\n\
                    Q1 c b e qm\n\
                    RE e 0 100\n\
                    RC vcc c 2k\n\
                    .model qm NPN(IS=1e-15 BF=100 VAF=50 RB=50 CJE=2p CJC=1p TF=0.3n PTF=30)\n\
                    .noise v(c) VB dec 3 1k 1g\n";
        let run = |noise_first: bool| {
            let mut solver = build(deck);
            if noise_first {
                solver.noise_sweep().unwrap();
            }
            let c = solver.circuit().node("c").unwrap();
            (0..200)
                .map(|_| {
                    solver.advance(5e-9).unwrap();
                    solver.node_voltage(c).to_bits()
                })
                .collect::<Vec<u64>>()
        };
        assert_eq!(run(true), run(false));
    }

    #[test]
    fn noise_card_errors_name_the_problem() {
        let parse = |card: &str| {
            parse_netlist(&format!("V1 a 0 DC 1\nR1 a 0 1k\n{card}\n"))
                .err()
                .map(|error| error.to_string())
        };
        assert!(parse(".noise v(a) R1 dec 1 1 10")
            .unwrap()
            .contains("independent `V` or `I` source"));
        assert!(parse(".noise a V1 dec 1 1 10")
            .unwrap()
            .contains("expected `.noise"));
        assert!(parse(".noise v(a) V1 dec 1 10 1")
            .unwrap()
            .contains("`.noise` needs 0 < fstart"));
        assert!(parse(".noise v(a,) V1 dec 1 1 10").is_some());
        assert!(parse(".noise v(a) V1 dec 1 1 10 5").is_none());
        let mut solver = build("V1 a 0 DC 1\nR1 a 0 1k\n.noise v(b) V1 dec 1 1 10\n");
        let error = solver.noise_sweep().unwrap_err().to_string();
        assert!(error.contains("`b` is not in the netlist"), "{error}");
        let mut solver = build("V1 a 0 DC 1\nR1 a 0 1k\n.noise v(a) V2 dec 1 1 10\n");
        let error = solver.noise_sweep().unwrap_err().to_string();
        assert!(error.contains("`V2` is not an independent"), "{error}");
    }
}
