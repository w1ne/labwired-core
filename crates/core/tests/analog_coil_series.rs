// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A 2-pin solenoid coil (series R + L) switched low-side by an N-MOSFET with a
//! flyback diode, powered by a settable DC source, solved by the in-core MNA,
//! and the run-long `analog_series` that reports its current.

mod analog {
    use labwired_core::analog::{AnalogConfig, AnalogCosimAdapter, AnalogTrace, Integration};
    use labwired_core::cosim::{CosimAdapter, CosimSignalValue, CosimStep};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    /// 5 V supply, 25 ohm / 10 mH coil (tau = 0.4 ms), logic-level MOSFET, 1N4007-ish flyback.
    const COIL: &str = "* solenoid low-side\n\
        Vcoil vcc 0 dc 5\n\
        Vgate g 0 dc 0\n\
        RM1 vcc m1 25\n\
        LM1 m1 d 10m\n\
        MQ1 d g 0 0 IRLZ\n\
        DD1 d vcc DFLY\n\
        .model IRLZ NMOS (VTO=1.5 KP=11 LAMBDA=0.01)\n\
        .model DFLY D (IS=2.52e-9 N=1.752 RS=0)\n\
        .end\n";
    const STEP_NS: u64 = 100_000;

    fn config(ring: usize) -> AnalogConfig {
        AnalogConfig {
            vdd: 5.0,
            substeps: 4,
            integration: Integration::BackwardEuler,
            probes: BTreeMap::from([("coil".to_string(), "i(LM1)".to_string())]),
            sources: BTreeMap::from([("gate".to_string(), "Vgate".to_string())]),
            trace: vec!["v(d)".to_string()],
            trace_samples: ring,
        }
    }

    struct Bench {
        adapter: AnalogCosimAdapter,
        trace: Arc<Mutex<AnalogTrace>>,
        time_ns: u64,
    }

    impl Bench {
        fn new(ring: usize) -> Self {
            let mut adapter =
                AnalogCosimAdapter::from_netlist(COIL, &config(ring)).expect("coil netlist builds");
            let trace = Arc::new(Mutex::new(AnalogTrace::new(ring)));
            adapter.attach_analog_trace(&trace, "circuit");
            Self {
                adapter,
                trace,
                time_ns: 0,
            }
        }

        /// Hold the gate for `ms`, returning the coil current after every step.
        fn hold(&mut self, gate: bool, ms: u64) -> Vec<f64> {
            let mut out = Vec::new();
            for _ in 0..(ms * 1_000_000 / STEP_NS) {
                self.time_ns += STEP_NS;
                let result = self
                    .adapter
                    .step(CosimStep {
                        time_ns: self.time_ns,
                        dt_ns: STEP_NS,
                        inputs: BTreeMap::from([(
                            "gate".to_string(),
                            CosimSignalValue::Bool(gate),
                        )]),
                    })
                    .expect("step solves");
                match result.outputs["coil"] {
                    CosimSignalValue::F64(amps) => out.push(amps),
                    ref other => panic!("coil current should be a float, got {other:?}"),
                }
            }
            out
        }
    }

    #[test]
    fn coil_current_follows_the_mosfet_and_settles_at_v_over_r() {
        let mut bench = Bench::new(20_000);
        assert!(
            bench.hold(false, 2).iter().all(|i| i.abs() < 1e-6),
            "gate low: no coil current"
        );
        let on = bench.hold(true, 10);
        let expected = 5.0 / 25.0;
        // Inductive rise: the first step is well below the final value.
        assert!(
            on[0].abs() < 0.5 * expected,
            "first step after turn-on {} A",
            on[0]
        );
        let last = on.last().unwrap().abs();
        assert!(
            (last - expected).abs() / expected < 0.02,
            "settled {last} A vs {expected} A"
        );
    }

    #[test]
    fn turn_off_current_decays_through_the_flyback_diode() {
        let mut bench = Bench::new(20_000);
        bench.hold(true, 10);
        let off = bench.hold(false, 20);
        // The inductor keeps its current for a moment: it does not vanish in
        // one step, which is the flyback diode carrying it.
        assert!(
            off[0].abs() > 0.02,
            "one step after turn-off the coil still carries {} A",
            off[0]
        );
        // The diode clamps the drain one drop above the supply instead of the
        // inductive kick an unclamped switch would see.
        let drain = bench.trace.lock().unwrap().series();
        let vd = drain
            .channels
            .iter()
            .find(|c| c.name.ends_with("v(d)"))
            .expect("v(d) channel");
        let peak = vd.points.iter().map(|p| p.1).fold(f64::MIN, f64::max);
        assert!(
            peak < 5.0 + 1.5,
            "drain clamped near V_COIL + Vf, peaked at {peak} V"
        );
        // ...and well within 10 ms (25 tau) it is gone.
        assert!(off[100].abs() < 1e-3, "10 ms after turn-off {} A", off[100]);
    }

    #[test]
    fn the_series_covers_the_whole_run_past_the_scope_ring() {
        let mut bench = Bench::new(10);
        bench.hold(true, 20);
        bench.hold(false, 20);
        bench.hold(true, 20);
        let trace = bench.trace.lock().unwrap();
        let series = trace.series();
        assert_eq!(series.end_time_ns, 60_000_000);
        let coil = series
            .channels
            .iter()
            .find(|c| c.name.ends_with("coil"))
            .expect("coil channel");
        assert_eq!(coil.unit, "A");
        assert_eq!(coil.dropped, 0);
        let at = |ms: u64| {
            coil.points
                .iter()
                .take_while(|p| p.0 <= ms * 1_000_000)
                .last()
                .map(|p| p.1.abs())
                .unwrap_or(0.0)
        };
        assert!(at(19) > 0.19, "on at 19 ms: {}", at(19));
        assert!(at(39) < 1e-3, "off at 39 ms: {}", at(39));
        assert!(at(59) > 0.19, "on again at 59 ms: {}", at(59));
        // Change points, not samples: steady stretches add nothing.
        assert!(
            coil.points.len() < 600 / 2,
            "{} points for 600 samples",
            coil.points.len()
        );
        for pair in coil.points.windows(2) {
            assert!(pair[0].0 < pair[1].0, "points are time-ordered");
        }
    }

    #[test]
    fn a_full_series_channel_counts_what_it_drops() {
        let mut bench = Bench::new(10);
        bench.trace.lock().unwrap().set_series_cap(2);
        bench.hold(true, 5);
        let series = bench.trace.lock().unwrap().series();
        let coil = series
            .channels
            .iter()
            .find(|c| c.name.ends_with("coil"))
            .unwrap();
        assert_eq!(coil.points.len(), 2);
        assert!(coil.dropped > 0, "a truncated series says so");
    }
}
