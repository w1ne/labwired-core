// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The MNA transient solver against closed-form answers.
//!
//! Every circuit here has an exact analytic solution, so the assertions are
//! against physics rather than against a previous run of this code. The
//! tolerances are the design's: 0.5 % for backward Euler at 10 substeps, 0.05 %
//! for trapezoidal.

// The `analog` module puts every test under `analog::`, so the design's
// verification command `cargo test -p labwired-core analog` runs all of them.
mod analog {
    use labwired_core::analog::{parse_netlist, AnalogError, Integration, Solver, MAX_UNKNOWNS};

    /// Step the solver to `t` seconds in `steps` equal internal steps.
    fn run_to(solver: &mut Solver, t: f64, steps: usize) {
        let h = t / steps as f64;
        for _ in 0..steps {
            solver.advance(h).expect("transient step should solve");
        }
    }

    fn rc_solver(integration: Integration) -> Solver {
        // 10 kΩ / 100 nF: tau = 1 ms. Vgpio is stepped to 3.3 V at t = 0+.
        let circuit = parse_netlist("* rc\nVgpio in 0 dc 0\nR1 in out 10k\nC1 out 0 100n\n.end\n")
            .expect("rc netlist parses");
        let mut solver = Solver::new(circuit, integration).expect("solver builds");
        let source = solver.voltage_source_index("Vgpio").expect("Vgpio exists");
        solver.set_voltage_source(source, 3.3);
        solver
    }

    /// Run the RC fixture at the engine's own cadence (a 100 us co-sim step split
    /// into 10 substeps, against tau = 1 ms) and check `out` at tau, 2 tau, 3 tau.
    fn rc_step_response_error(integration: Integration) -> Vec<(u32, f64, f64, f64)> {
        let mut solver = rc_solver(integration);
        let node = solver.circuit().node("out").expect("node out");
        let mut taken = 0;
        let mut out = Vec::new();
        for multiple in 1..=3u32 {
            // tau = 1 ms is exactly ten 100 us co-sim steps.
            while taken < multiple * 10 {
                run_to(&mut solver, 100e-6, 10);
                taken += 1;
            }
            let expected = 3.3 * (1.0 - (-f64::from(multiple)).exp());
            let actual = solver.node_voltage(node);
            out.push((
                multiple,
                actual,
                expected,
                (actual - expected).abs() / expected,
            ));
        }
        out
    }

    #[test]
    fn rc_step_response_matches_closed_form_with_backward_euler() {
        for (multiple, actual, expected, error) in
            rc_step_response_error(Integration::BackwardEuler)
        {
            assert!(
                error < 0.005,
                "{multiple} tau: BE gave {actual} V, closed form {expected} V, \
             relative error {:.4} %",
                error * 100.0
            );
        }
    }

    #[test]
    fn rc_step_response_is_an_order_of_magnitude_better_with_trapezoidal() {
        for (multiple, actual, expected, error) in rc_step_response_error(Integration::Trapezoidal)
        {
            assert!(
                error < 0.0005,
                "{multiple} tau: trapezoidal gave {actual} V, closed form {expected} V, \
             relative error {:.5} %",
                error * 100.0
            );
        }
    }

    #[test]
    fn rl_current_matches_closed_form() {
        // 10 V across 100 Ω in series with 10 mH: tau = L/R = 100 us,
        // i(t) = (V/R)·(1 − e^(−t/tau)).
        //
        // `ic=0` is what makes this a STEP response: the DC operating point of an
        // inductor is a short, so without it the branch starts at its final 100 mA
        // and there is no transient to check. That is correct SPICE behaviour and
        // exactly why `ic=` exists.
        let circuit = parse_netlist("* rl\nV1 a 0 dc 10\nR1 a b 100\nL1 b 0 10m ic=0\n.end\n")
            .expect("rl netlist parses");
        let mut solver = Solver::new(circuit, Integration::Trapezoidal).expect("solver builds");
        assert_eq!(solver.inductor_current(0), 0.0, "`ic=0` starts it at rest");

        let tau = 10e-3 / 100.0;
        let final_current = 10.0 / 100.0;
        let mut t = 0.0;
        for multiple in 1..=3 {
            let target = f64::from(multiple) * tau;
            run_to(&mut solver, target - t, 200);
            t = target;
            let expected = final_current * (1.0 - (-f64::from(multiple)).exp());
            let actual = solver.inductor_current(0);
            let error = (actual - expected).abs() / expected;
            assert!(
                error < 0.001,
                "{multiple} tau: RL current {actual} A vs closed form {expected} A ({:.4} %)",
                error * 100.0
            );
        }
    }

    /// Value of node `name` after running `deck` to each time in `at`, in
    /// trapezoidal steps of `h` seconds.
    fn uic_samples(deck: &str, name: &str, at: &[f64], h: f64) -> Vec<f64> {
        let circuit = parse_netlist(deck).expect("deck parses");
        let mut solver = Solver::new(circuit, Integration::Trapezoidal).expect("solver builds");
        let node = solver.circuit().node(name).expect("probe node exists");
        let mut t = 0.0;
        at.iter()
            .map(|&target| {
                let steps = ((target - t) / h).round() as usize;
                run_to(&mut solver, target - t, steps);
                t = target;
                solver.node_voltage(node)
            })
            .collect()
    }

    #[test]
    fn tran_uic_starts_an_rc_from_its_dot_ic_not_its_operating_point() {
        // 1 kΩ / 1 µF from a 5 V rail, tau = 1 ms. The operating point holds
        // `out` at 5 V; under `uic` it starts at the `.ic` 2 V and charges as
        // 5 − 3·e^(−t/tau). ngspice 45 `.tran 1u 3m uic` gives 3.896362 V at
        // 1 ms and 4.593994 V at 2 ms, the same as the closed form.
        let body = "V1 in 0 dc 5\nR1 in out 1k\nC1 out 0 1u\n.ic V(out)=2\n";
        let uic = format!("* rc\n{body}.tran 1u 3m UIC\n.end\n");
        assert!(
            parse_netlist(&uic).unwrap().uic,
            "`UIC` is case-insensitive"
        );
        assert!(!parse_netlist(&uic.replace(" UIC", "")).unwrap().uic);

        let at = [0.0, 1e-3, 2e-3];
        let got = uic_samples(&uic, "out", &at, 1e-6);
        assert!(
            (got[0] - 2.0).abs() < 1e-12,
            "uic starts at .ic: {}",
            got[0]
        );
        for (index, &t) in at.iter().enumerate().skip(1) {
            let expected = 5.0 - 3.0 * (-t / 1e-3).exp();
            assert!(
                (got[index] - expected).abs() < 1e-4,
                "uic RC at {t} s: {} V vs closed form {expected} V",
                got[index]
            );
        }

        // With no `.ic` at all the two modes split: the operating point
        // starts the RC charged, `uic` starts it empty.
        let bare = "V1 in 0 dc 5\nR1 in out 1k\nC1 out 0 1u\n";
        let held = uic_samples(
            &format!("* rc\n{bare}.tran 1u 3m\n.end\n"),
            "out",
            &[0.0],
            1e-6,
        );
        let empty = uic_samples(
            &format!("* rc\n{bare}.tran 1u 3m uic\n.end\n"),
            "out",
            &[0.0],
            1e-6,
        );
        assert!(
            (held[0] - 5.0).abs() < 1e-9,
            "no uic: charged ({} V)",
            held[0]
        );
        assert!(empty[0].abs() < 1e-12, "uic: empty ({} V)", empty[0]);
    }

    #[test]
    fn tran_uic_rings_a_series_rlc_from_rest() {
        // 20 Ω, 10 mH, 1 µF from a 5 V step: alpha = 1000 /s, omega_d ≈ 9950
        // rad/s. Under `uic` both reactive elements start at zero with no `ic=`
        // anywhere, so the capacitor overshoots and rings. ngspice 45 gives
        // 6.290332, 4.507278, 6.684280 V at 0.2, 0.5, 1 ms.
        let deck = "* rlc\nV1 in 0 dc 5\nR1 in a 20\nL1 a out 10m\nC1 out 0 1u\n\
                    .tran 0.5u 2m uic\n.end\n";
        let (alpha, w0) = (20.0 / (2.0 * 10e-3), 1.0 / (10e-3f64 * 1e-6).sqrt());
        let wd = (w0 * w0 - alpha * alpha).sqrt();
        let at = [0.2e-3, 0.5e-3, 1e-3];
        let got = uic_samples(deck, "out", &at, 0.25e-6);
        for (index, &t) in at.iter().enumerate() {
            let expected =
                5.0 * (1.0 - (-alpha * t).exp() * ((wd * t).cos() + alpha / wd * (wd * t).sin()));
            assert!(
                (got[index] - expected).abs() < 1e-3,
                "uic RLC at {t} s: {} V vs closed form {expected} V",
                got[index]
            );
        }
    }

    #[test]
    fn tran_uic_charges_through_a_diode_like_ngspice() {
        // Nonlinear: the Newton for the first step starts from the all-zero
        // `uic` state rather than an operating point. Values are ngspice 45
        // `.tran 0.1u 1m uic` on this deck.
        let deck = "* d\nV1 in 0 dc 5\nR1 in a 100\nD1 a out DMOD\nC1 out 0 1u\n\
                    R2 out 0 10k\n.model DMOD D(IS=1e-14 N=1)\n.tran 0.1u 1m uic\n.end\n";
        let at = [50e-6, 200e-6, 1e-3];
        let ngspice = [1.670324, 3.676337, 4.321201];
        let got = uic_samples(deck, "out", &at, 0.1e-6);
        for index in 0..at.len() {
            assert!(
                (got[index] - ngspice[index]).abs() < 2e-3,
                "uic diode charge at {} s: {} V vs ngspice {} V",
                at[index],
                got[index],
                ngspice[index]
            );
        }
    }

    #[test]
    fn tran_uic_diode_ic_discharges_its_junction_charge_in_closed_form() {
        // A reverse-biased abrupt junction (M = 0.5) seeded at `ic=-3` and
        // discharged through R. With C(v) = CJO/√(1 + v/VJ) for reverse volts
        // v, R·C(v)·dv/dt = −v integrates to
        //   t(v) = R·CJO·[ln((s−1)/(s+1))] from v to V0,  s = √(1 + v/VJ).
        // The node starts at 0 V under `uic`; only the seeded charge can lift
        // it. (ngspice 45 parses diode `ic=` but `show d1 : ic` reads 0 and the
        // node stays at 0 V, so the closed form is the reference here.)
        let (r, cjo, vj, v0) = (10e3, 100e-12, 0.7, 3.0);
        let body = "R1 a 0 10k\nD1 0 a DMOD ic=-3\n\
                    .model DMOD D(IS=1e-14 CJO=100p VJ=0.7 M=0.5)\n";
        let deck = format!("* d ic\n{body}.tran 1n 3u uic\n.end\n");
        let at = [0.2e-6, 0.5e-6, 1e-6];
        let got = uic_samples(&deck, "a", &at, 0.5e-9);
        let log_ratio = |v: f64| {
            let s = (1.0 + v / vj).sqrt();
            ((s - 1.0) / (s + 1.0)).ln()
        };
        for (index, &t) in at.iter().enumerate() {
            assert!(
                got[index] > 0.05 && got[index] < v0,
                "seeded junction must hold the node up at {t} s: {} V",
                got[index]
            );
            let elapsed = r * cjo * (log_ratio(v0) - log_ratio(got[index]));
            assert!(
                (elapsed - t).abs() / t < 0.01,
                "diode ic= discharge reached {} V at {t} s; closed form puts that at {elapsed} s",
                got[index]
            );
        }

        // SPICE ignores instance `ic=` without `uic`: the operating point
        // holds the node at 0 V and nothing discharges.
        let no_uic = uic_samples(&format!("* d\n{body}.tran 1n 3u\n.end\n"), "a", &at, 0.5e-9);
        assert!(no_uic.iter().all(|v| v.abs() < 1e-9), "{no_uic:?}");
        // And a wrong arity is refused rather than silently truncated.
        assert!(parse_netlist("R1 a 0 1k\nD1 0 a DMOD ic=1,2\n.model DMOD D(IS=1e-14)\n").is_err());
    }

    #[test]
    fn tran_uic_bjt_ic_seeds_both_junction_charges_like_ngspice() {
        // `ic=<vbe>,<vce>` = −2, 5: vbc = −7. Both depletion charges discharge
        // through the base and collector resistors, coupled through CJC.
        // Values are ngspice 45 `.tran 1n 3u uic` on this deck.
        let deck = "* q ic\nRb b 0 10k\nRc c 0 20k\nQ1 c b 0 QMOD ic=-2,5\n\
                    .model QMOD NPN(IS=1e-15 BF=100 CJE=20p VJE=0.75 MJE=0.33 \
                    CJC=10p VJC=0.75 MJC=0.33)\n.tran 1n 3u uic\n.end\n";
        let at = [0.1e-6, 0.3e-6];
        let base = uic_samples(deck, "b", &at, 0.5e-9);
        let collector = uic_samples(deck, "c", &[0.1e-6, 0.3e-6, 1e-6], 0.5e-9);
        let expect = [
            (base[0], -1.740669),
            (base[1], -0.8756738),
            (collector[0], 2.077584),
            (collector[1], 0.8144268),
            (collector[2], 0.1038266),
        ];
        for (got, ngspice) in expect {
            assert!(
                (got - ngspice).abs() < 5e-3,
                "BJT ic=: {got} V vs ngspice {ngspice} V"
            );
        }
    }

    #[test]
    fn tran_uic_jfet_ic_seeds_both_gate_charges_like_ngspice() {
        // `ic=<vds>,<vgs>` = 4, −3: vgd = −7, channel pinched off (VTO = −2).
        // As the gate discharges the channel turns on and drains `d`.
        // Values are ngspice 45 `.tran 1n 3u uic` on this deck.
        let deck = "* j ic\nRg g 0 10k\nRd d 0 20k\nJ1 d g 0 JMOD ic=4,-3\n\
                    .model JMOD NJF(VTO=-2 BETA=1e-4 CGS=20p CGD=10p PB=0.8)\n\
                    .tran 1n 3u uic\n.end\n";
        let gate = uic_samples(deck, "g", &[0.1e-6, 0.3e-6], 0.5e-9);
        let drain = uic_samples(deck, "d", &[0.1e-6, 0.3e-6, 1e-6], 0.5e-9);
        let expect = [
            (gate[0], -1.860783),
            (gate[1], -0.8536155),
            (drain[0], 1.680691),
            (drain[1], 0.1237690),
            (drain[2], 0.005011552),
        ];
        for (got, ngspice) in expect {
            assert!(
                (got - ngspice).abs() < 5e-3,
                "JFET ic=: {got} V vs ngspice {ngspice} V"
            );
        }
    }

    /// ngspice 45 `meas tran ... find v(node) at=` on the MOSFET `ic=` decks.
    fn assert_matches_ngspice(what: &str, pairs: &[(f64, f64)]) {
        for &(got, ngspice) in pairs {
            assert!(
                (got - ngspice).abs() < 5e-3,
                "{what}: {got} V vs ngspice {ngspice} V"
            );
        }
    }

    #[test]
    fn tran_uic_mosfet_ic_seeds_both_bulk_junction_charges_like_ngspice() {
        // `ic=<vds>,<vgs>,<vbs>` = 4, 2, −3: vbd = −7. No TOX and no overlap,
        // so only the bulk depletion charges hold state; they discharge
        // through the drain and bulk resistors, coupled through CBD. The
        // gate has no capacitance and stays at 0 V, so the channel is off.
        // Values are ngspice 45 `.tran 1n 3u uic` on this deck.
        let deck = "* m ic\nRd d 0 20k\nRg g 0 10k\nRb b 0 10k\nM1 d g 0 b NM ic=4,2,-3\n\
                    .model NM NMOS(VTO=1 KP=2e-5 CBD=10p CBS=20p PB=0.8 MJ=0.5)\n\
                    .tran 1n 3u uic\n.end\n";
        let at = [0.1e-6, 0.3e-6, 1e-6];
        let drain = uic_samples(deck, "d", &at, 0.5e-9);
        let bulk = uic_samples(deck, "b", &at, 0.5e-9);
        let gate = uic_samples(deck, "g", &at[..1], 0.5e-9);
        assert_matches_ngspice(
            "MOSFET bulk ic=",
            &[
                (drain[0], 1.683600),
                (drain[1], 0.6649946),
                (drain[2], 0.08093700),
                (bulk[0], -1.859984),
                (bulk[1], -0.7634749),
                (bulk[2], -0.08429095),
                (gate[0], 0.0),
            ],
        );

        // Without `uic` the instance ic= is ignored, as in SPICE: the
        // operating point is all zero and nothing moves.
        let dc = deck.replace(" uic", "");
        assert!(uic_samples(&dc, "d", &at[..1], 0.5e-9)[0].abs() < 1e-9);

        // ngspice's MOS1 ic= takes one to three values; more is refused.
        let error = parse_netlist(
            "M1 d g 0 0 NM ic=1,2,0,4\n.model NM NMOS(VTO=1 KP=1e-4)\nR1 d 0 1k\nR2 g 0 1k\n",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("MOSFET ic= takes one to three"), "{error}");
    }

    #[test]
    fn tran_uic_mosfet_ic_seeds_overlap_charge_like_explicit_capacitors() {
        // CGSO/CGDO are linear, so `ic=4,2,-3` on them is exactly two
        // explicit capacitors at vgs = 2 V and vgd = −2 V. ngspice 45's own
        // MOSFET `ic=` (and `.ic` on the nodes) does NOT give that circuit:
        // its MODEINITTRAN start drops gate charge in the first picoseconds
        // (v(g) 2.0 → 1.5 V in one 0.84 ps step) and lands at d = 1.617 V at
        // 0.1 µs. These values are ngspice 45 on the explicit-capacitor
        // deck, the charge-conserving answer:
        //   Cgs g 0 0.1p ic=2 / Cgd g d 0.1p ic=-2 / M1 ... (no CGSO/CGDO).
        let deck = "* m ic overlap\nRd d 0 20k\nRg g 0 10k\nRb b 0 10k\n\
                    M1 d g 0 b NM ic=4,2,-3 L=10u W=100u\n\
                    .model NM NMOS(VTO=1 KP=2e-5 CBD=10p CBS=20p PB=0.8 MJ=0.5 \
                    CGSO=1n CGDO=1n)\n.tran 1n 3u uic\n.end\n";
        let at = [0.1e-6, 1e-6];
        let drain = uic_samples(deck, "d", &at, 0.5e-9);
        let gate = uic_samples(deck, "g", &at[..1], 0.5e-9);
        let bulk = uic_samples(deck, "b", &at[..1], 0.5e-9);
        assert_matches_ngspice(
            "MOSFET overlap ic=",
            &[
                (drain[0], 1.697863),
                (drain[1], 0.08174079),
                (gate[0], -0.01051855),
                (bulk[0], -1.855384),
            ],
        );
        // And the same equivalence inside this engine, tighter.
        let explicit = "* explicit\nRd d 0 20k\nRg g 0 10k\nRb b 0 10k\n\
                        Cgs g 0 0.1p ic=2\nCgd g d 0.1p ic=-2\n\
                        M1 d g 0 b NM ic=4,2,-3 L=10u W=100u\n\
                        .model NM NMOS(VTO=1 KP=2e-5 CBD=10p CBS=20p PB=0.8 MJ=0.5)\n\
                        .tran 1n 3u uic\n.end\n";
        for node in ["d", "g", "b"] {
            let built_in = uic_samples(deck, node, &[1e-9, 0.1e-6], 0.5e-9);
            let reference = uic_samples(explicit, node, &[1e-9, 0.1e-6], 0.5e-9);
            for (got, want) in built_in.iter().zip(&reference) {
                assert!((got - want).abs() < 1e-6, "v({node}): {got} V vs {want} V");
            }
        }
    }

    #[test]
    fn tran_uic_mosfet_ic_seeds_meyer_charge_like_dot_ic_on_its_terminals() {
        // With TOX the Meyer charge is nonlinear, so there is no explicit-
        // capacitor twin. Instance `ic=4,2,-3` must start the device exactly
        // where `.ic V(d)=4 V(g)=2 V(b)=-3` on its terminals does, as it does
        // in ngspice (both give d = 1.557533 V at 0.1 µs there, with the
        // MODEINITTRAN charge loss described in the overlap test).
        let model = ".model NM NMOS(VTO=1 KP=2e-5 CBD=10p CBS=20p PB=0.8 MJ=0.5 \
                     TOX=20n CGSO=1n CGDO=1n)\n.tran 1n 3u uic\n.end\n";
        let instance = format!(
            "* m ic\nRd d 0 20k\nRg g 0 10k\nRb b 0 10k\n\
             M1 d g 0 b NM ic=4,2,-3 L=10u W=100u\n{model}"
        );
        let nodes = format!(
            "* .ic\nRd d 0 20k\nRg g 0 10k\nRb b 0 10k\n\
             M1 d g 0 b NM L=10u W=100u\n.ic V(d)=4 V(g)=2 V(b)=-3\n{model}"
        );
        let at = [1e-9, 0.1e-6, 1e-6];
        for node in ["d", "g", "b"] {
            let from_ic = uic_samples(&instance, node, &at, 0.5e-9);
            let from_nodes = uic_samples(&nodes, node, &at, 0.5e-9);
            for (got, want) in from_ic.iter().zip(&from_nodes) {
                assert!((got - want).abs() < 1e-6, "v({node}): {got} V vs {want} V");
            }
        }
        // The gate holds its seeded 2 V charge rather than starting at 0 V:
        // after 1 ns it has only begun to discharge through Rg.
        let gate = uic_samples(&instance, "g", &[1e-9], 0.5e-9)[0];
        assert!(gate > 1.8 && gate < 2.0, "gate after 1 ns: {gate} V");
        // A partial ic= takes the rest from the terminals (MOS1getic):
        // `ic=4` alone leaves vgs and vbs at their 0 V nodes.
        let partial = instance.replace("ic=4,2,-3", "ic=4");
        let gate = uic_samples(&partial, "g", &[1e-9], 0.5e-9)[0];
        assert!(gate.abs() < 0.5, "partial ic= gate: {gate} V");
    }

    #[test]
    fn mutually_coupled_inductors_match_the_be_and_trapezoidal_companions() {
        const L1: f64 = 4e-3;
        const L2: f64 = 1e-3;
        const M: f64 = 1e-3; // k = 0.5
        const R1: f64 = 10.0;
        const R2: f64 = 20.0;
        const SOURCE: f64 = 5.0;
        const H: f64 = 100e-6;

        let circuit = parse_netlist(
            "V1 drive 0 dc 0\nR1 drive primary 10\nL1 primary 0 4m ic=0\n\
             R2 secondary 0 20\nL2 secondary 0 1m ic=0\nK1 L1 L2 0.5\n",
        )
        .expect("coupled-inductor deck parses");

        for integration in [Integration::BackwardEuler, Integration::Trapezoidal] {
            let mut solver = Solver::new(circuit.clone(), integration).expect("solver builds");
            let source = solver.voltage_source_index("V1").expect("source exists");
            solver.set_voltage_source(source, SOURCE);

            let mut currents = [0.0, 0.0];
            let mut voltages = [0.0, 0.0];
            for step in 0..2 {
                // The first step after changing the source restarts with BE;
                // the second uses the requested integration rule.
                let trapezoidal = step != 0 && integration == Integration::Trapezoidal;
                let scale = if trapezoidal { 2.0 / H } else { 1.0 / H };
                let previous_term = if trapezoidal {
                    [
                        scale * (L1 * currents[0] + M * currents[1]) + voltages[0],
                        scale * (M * currents[0] + L2 * currents[1]) + voltages[1],
                    ]
                } else {
                    [
                        scale * (L1 * currents[0] + M * currents[1]),
                        scale * (M * currents[0] + L2 * currents[1]),
                    ]
                };
                let a = R1 + scale * L1;
                let b = scale * M;
                let d = R2 + scale * L2;
                let determinant = a * d - b * b;
                let expected = [
                    (d * (SOURCE + previous_term[0]) - b * previous_term[1]) / determinant,
                    (a * previous_term[1] - b * (SOURCE + previous_term[0])) / determinant,
                ];

                solver.advance(H).expect("coupled transient step solves");
                let actual = [solver.inductor_current(0), solver.inductor_current(1)];
                for coil in 0..2 {
                    assert!(
                        (actual[coil] - expected[coil]).abs() < 1e-11,
                        "{integration:?} step {step}, coil {coil}: expected {} A, got {} A",
                        expected[coil],
                        actual[coil]
                    );
                }
                currents = actual;
                voltages = [SOURCE - R1 * currents[0], -R2 * currents[1]];
            }
            assert!(
                solver.inductor_current(1) < 0.0,
                "a positive mutual coupling induces the opposing secondary-loop current"
            );
        }
    }

    #[test]
    fn underdamped_rlc_matches_closed_form() {
        // Series RLC driven by a 1 V step: R = 20 Ω, L = 1 mH, C = 1 uF.
        // alpha = R/2L = 10 000 s^-1, w0 = 1/sqrt(LC) = 31 623 rad/s, so the
        // circuit is underdamped and the capacitor voltage overshoots:
        //   vC(t) = V·[1 − e^(−alpha t)·(cos(wd t) + (alpha/wd)·sin(wd t))]
        //
        // `ic=0` again: with the capacitor open at DC no current flows, so the
        // operating point already sits at the 1 V rail.
        let circuit =
            parse_netlist("* rlc\nV1 a 0 dc 1\nR1 a b 20\nL1 b c 1m\nC1 c 0 1u ic=0\n.end\n")
                .expect("rlc netlist parses");
        let mut solver = Solver::new(circuit, Integration::Trapezoidal).expect("solver builds");
        let node = solver.circuit().node("c").expect("node c");
        assert_eq!(solver.capacitor_voltage(0), 0.0);
        assert_eq!(solver.inductor_current(0), 0.0);

        let r = 20.0_f64;
        let l = 1e-3_f64;
        let c = 1e-6_f64;
        let alpha = r / (2.0 * l);
        let w0 = 1.0 / (l * c).sqrt();
        let wd = (w0 * w0 - alpha * alpha).sqrt();
        assert!(wd > 0.0, "this fixture must be underdamped");

        // Across the first ringing period, including the overshoot peak.
        let mut peak = 0.0_f64;
        let mut t = 0.0;
        for target in [20e-6, 50e-6, 100e-6, 200e-6, 400e-6] {
            run_to(&mut solver, target - t, 2_000);
            t = target;
            let expected =
                1.0 - (-alpha * t).exp() * ((wd * t).cos() + (alpha / wd) * (wd * t).sin());
            let actual = solver.node_voltage(node);
            peak = peak.max(actual);
            assert!(
                (actual - expected).abs() < 2e-3,
                "t={t:e}s: RLC capacitor {actual} V vs closed form {expected} V"
            );
        }
        // The ring is physics, not numerical noise: an underdamped step overshoots
        // its final value and then settles back to it.
        assert!(
            peak > 1.05,
            "an underdamped step must overshoot, peaked at {peak} V"
        );
        assert!(
            (solver.node_voltage(node) - 1.0).abs() < 0.05,
            "and settle by 400 us, at {} V",
            solver.node_voltage(node)
        );
    }

    #[test]
    fn switch_conducts_at_ron_and_blocks_at_roff() {
        // A 1 kΩ pull-up to 5 V, shunted to ground by the switch.
        let circuit = parse_netlist(
            "* switch\nV1 vdd 0 dc 5\nR1 vdd out 1k\nS1 out 0 gate ron=1 roff=1meg\n\
         C1 out 0 1p\n.end\n",
        )
        .expect("switch netlist parses");
        let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver builds");
        let node = solver.circuit().node("out").expect("node out");
        let switch = solver.switches_controlled_by("gate");
        assert_eq!(switch, vec![0], "one switch is driven by `gate`");

        // Open: the divider is 1 k / 1 Meg, so `out` sits just under 5 V.
        run_to(&mut solver, 1e-6, 100);
        let open = solver.node_voltage(node);
        let expected_open = 5.0 * 1e6 / (1e6 + 1e3);
        assert!(
            (open - expected_open).abs() < 1e-3,
            "open switch: {open} V, expected {expected_open} V"
        );

        // Closed: 1 k / 1 Ω pulls it to a few millivolts.
        solver.set_switch(0, true);
        run_to(&mut solver, 1e-6, 100);
        let closed = solver.node_voltage(node);
        let expected_closed = 5.0 * 1.0 / (1.0 + 1e3);
        assert!(
            (closed - expected_closed).abs() < 1e-3,
            "closed switch: {closed} V, expected {expected_closed} V"
        );
        assert!(
            closed < open / 100.0,
            "closing the switch must pull `out` down"
        );
    }

    #[test]
    fn operating_point_uses_netlist_dc_values() {
        // A pull-up sits at Vdd before anything is routed, exactly like a board
        // out of reset — the property the ngspice wrapper preserves by pausing the
        // transient just after t = 0.
        let circuit = parse_netlist("* op\nV1 vdd 0 dc 3.3\nR1 vdd out 10k\nC1 out 0 100n\n.end\n")
            .expect("netlist parses");
        let solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver builds");
        let node = solver.circuit().node("out").expect("node out");
        assert!(
            (solver.node_voltage(node) - 3.3).abs() < 1e-9,
            "an unloaded pull-up is at Vdd at the operating point, not 0 V"
        );
        assert!(
            (solver.capacitor_voltage(0) - 3.3).abs() < 1e-9,
            "the capacitor starts charged to the operating point"
        );
    }

    #[test]
    fn initial_conditions_override_the_operating_point() {
        let circuit =
            parse_netlist("* ic\nV1 vdd 0 dc 3.3\nR1 vdd out 10k\nC1 out 0 100n ic=1.0\n.end\n")
                .expect("netlist parses");
        let mut solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver builds");
        assert!((solver.capacitor_voltage(0) - 1.0).abs() < 1e-12);

        // From 1 V the node charges toward 3.3 V with the same tau.
        let tau = 10e3 * 100e-9;
        run_to(&mut solver, tau, 1_000);
        let node = solver.circuit().node("out").expect("node out");
        let expected = 3.3 + (1.0 - 3.3) * (-1.0_f64).exp();
        assert!(
            (solver.node_voltage(node) - expected).abs() < 2e-3,
            "charging from ic=1.0: {} V vs {expected} V",
            solver.node_voltage(node)
        );
    }

    #[test]
    fn dot_ic_seeds_a_node() {
        let circuit = parse_netlist(
            "* dotic\nV1 vdd 0 dc 5\nR1 vdd out 1k\nC1 out 0 1u\n.ic V(out)=2\n.end\n",
        )
        .expect("netlist parses");
        let solver = Solver::new(circuit, Integration::BackwardEuler).expect("solver builds");
        assert!((solver.capacitor_voltage(0) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn a_circuit_over_the_unknown_limit_is_refused() {
        // One voltage source plus a chain of resistors: each new node is one more
        // unknown, so 64 nodes + 1 branch current crosses the ceiling.
        let mut netlist = String::from("* ladder\nV1 n0 0 dc 1\n");
        let nodes = MAX_UNKNOWNS;
        for index in 0..nodes {
            netlist.push_str(&format!("R{index} n{index} n{} 1k\n", index + 1));
        }
        netlist.push_str(".end\n");

        let circuit = parse_netlist(&netlist).expect("ladder parses");
        assert!(circuit.unknowns() > MAX_UNKNOWNS);
        let err = Solver::new(circuit, Integration::BackwardEuler)
            .expect_err("an over-sized circuit must be refused, not silently truncated");
        match err {
            AnalogError::TooLarge { unknowns, max } => {
                assert_eq!(max, MAX_UNKNOWNS);
                assert!(unknowns > MAX_UNKNOWNS);
            }
            other => panic!("expected TooLarge, got {other}"),
        }
        assert!(
            err.to_string().contains("labwired_ngspice.py"),
            "the error must point at the adapter that can solve it: {err}"
        );
    }

    #[test]
    fn a_circuit_at_the_unknown_limit_is_accepted() {
        let mut netlist = String::from("* ladder\nV1 n0 0 dc 1\n");
        // 63 nodes + 1 voltage-source branch current = exactly 64.
        for index in 0..62 {
            netlist.push_str(&format!("R{index} n{index} n{} 1k\n", index + 1));
        }
        netlist.push_str("Rend n62 0 1k\n.end\n");
        let circuit = parse_netlist(&netlist).expect("ladder parses");
        assert_eq!(circuit.unknowns(), MAX_UNKNOWNS);
        Solver::new(circuit, Integration::BackwardEuler).expect("64 unknowns is inside the limit");
    }

    #[test]
    fn a_floating_node_is_reported_not_silently_zeroed() {
        let circuit =
            parse_netlist("* floating\nV1 a 0 dc 1\nR1 b c 1k\n.end\n").expect("netlist parses");
        let err = Solver::new(circuit, Integration::BackwardEuler)
            .expect_err("a node with no DC path to ground has no unique solution");
        assert!(matches!(err, AnalogError::Singular { .. }), "{err}");
    }

    #[test]
    fn the_same_netlist_and_inputs_give_bit_identical_results() {
        let run = || {
            let mut solver = rc_solver(Integration::Trapezoidal);
            let node = solver.circuit().node("out").expect("node out");
            let mut samples = Vec::new();
            for _ in 0..50 {
                run_to(&mut solver, 100e-6, 10);
                samples.push(solver.node_voltage(node).to_bits());
            }
            samples
        };
        assert_eq!(run(), run(), "the solver must be bit-deterministic");
    }
}
