// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The `adapter: analog` co-simulation contract: manifest config, step
//! semantics, the trace ring, and what the machine publishes to instruments.

// The `analog` module puts every test under `analog::`, so the design's
// verification command `cargo test -p labwired-core analog` runs all of them.
mod analog {
    use labwired_config::{CosimAdapter as ManifestCosimAdapter, CosimModelConfig};
    use labwired_core::analog::{AnalogConfig, AnalogCosimAdapter, Integration};
    use labwired_core::cosim::{
        build_cosim_adapter_with_base, validate_analog_models, CosimAdapter, CosimRunner,
        CosimRunnerModel, CosimSignalValue, CosimStep,
    };
    use std::collections::{BTreeMap, HashMap};
    use std::path::{Path, PathBuf};

    const RC: &str = "* rc\nVgpio in 0 dc 0\nR1 in out 10k\nC1 out 0 100n\n.end\n";

    fn rc_config() -> AnalogConfig {
        AnalogConfig {
            vdd: 3.3,
            substeps: 10,
            integration: Integration::BackwardEuler,
            probes: BTreeMap::from([("v_out".to_string(), "v(out)".to_string())]),
            sources: BTreeMap::from([("gpio".to_string(), "Vgpio".to_string())]),
            trace: vec!["v(in)".to_string(), "i(Vgpio)".to_string()],
            trace_samples: 20_000,
        }
    }

    fn step_high(adapter: &mut AnalogCosimAdapter, time_ns: u64) -> f64 {
        let result = adapter
            .step(CosimStep {
                time_ns,
                dt_ns: 100_000,
                inputs: BTreeMap::from([("gpio".to_string(), CosimSignalValue::Bool(true))]),
            })
            .expect("step should solve");
        match result.outputs["v_out"] {
            CosimSignalValue::F64(volts) => volts,
            ref other => panic!("v_out should be a float, got {other:?}"),
        }
    }

    #[test]
    fn the_operating_point_comes_before_any_input() {
        let adapter = AnalogCosimAdapter::from_netlist(RC, &rc_config()).expect("adapter builds");
        let node = adapter.solver().circuit().node("out").expect("node out");
        assert_eq!(
            adapter.solver().node_voltage(node),
            0.0,
            "with Vgpio at its netlist dc of 0, the operating point is 0 V"
        );
        assert_eq!(adapter.time_ns(), 0);
    }

    #[test]
    fn time_ns_is_the_end_of_the_step_like_the_ngspice_wrapper() {
        let mut adapter =
            AnalogCosimAdapter::from_netlist(RC, &rc_config()).expect("adapter builds");
        // One tau is 1 ms = ten 100 us steps. After stepping to time_ns = 1_000_000
        // the node must be at 3.3·(1 − e⁻¹), which is what
        // `examples/cosim-spice-rc/README.md` documents for ngspice.
        let mut volts = 0.0;
        for step in 1..=10 {
            volts = step_high(&mut adapter, step * 100_000);
        }
        assert_eq!(adapter.time_ns(), 1_000_000);
        let expected = 3.3 * (1.0 - (-1.0_f64).exp());
        let error = (volts - expected).abs() / expected;
        assert!(
            error < 0.005,
            "after one tau: {volts} V vs {expected} V ({:.3} %)",
            error * 100.0
        );
    }

    #[test]
    fn a_boundary_already_passed_re_reads_rather_than_integrating_backwards() {
        let mut adapter =
            AnalogCosimAdapter::from_netlist(RC, &rc_config()).expect("adapter builds");
        let first = step_high(&mut adapter, 500_000);
        let again = step_high(&mut adapter, 400_000);
        assert_eq!(adapter.time_ns(), 500_000, "time never moves backwards");
        assert_eq!(first, again, "a stale boundary re-reads the same probes");
    }

    #[test]
    fn booleans_map_to_vdd_and_zero() {
        let mut adapter =
            AnalogCosimAdapter::from_netlist(RC, &rc_config()).expect("adapter builds");
        for step in 1..=30 {
            step_high(&mut adapter, step * 100_000);
        }
        let charged = step_high(&mut adapter, 3_100_000);
        // 3.1 tau: 3.3 V x (1 - e^-3.1) = 3.151 V.
        assert!(
            charged > 3.3 * 0.95,
            "three tau charges past 95 % of 3.3 V: {charged} V"
        );

        // Release the pin: the same tau discharges the node.
        let mut volts = charged;
        for step in 32..=62 {
            let result = adapter
                .step(CosimStep {
                    time_ns: step * 100_000,
                    dt_ns: 100_000,
                    inputs: BTreeMap::from([("gpio".to_string(), CosimSignalValue::Bool(false))]),
                })
                .expect("step should solve");
            volts = match result.outputs["v_out"] {
                CosimSignalValue::F64(value) => value,
                ref other => panic!("unexpected {other:?}"),
            };
        }
        assert!(
            volts < 3.3 * 0.05,
            "three tau discharges below 5 % of 3.3 V: {volts} V"
        );
    }

    #[test]
    fn a_number_input_is_taken_as_volts() {
        let mut adapter =
            AnalogCosimAdapter::from_netlist(RC, &rc_config()).expect("adapter builds");
        // Ten tau: settled to within 1e-4 of the drive.
        for step in 1..=100 {
            adapter
                .step(CosimStep {
                    time_ns: step * 100_000,
                    dt_ns: 100_000,
                    inputs: BTreeMap::from([("gpio".to_string(), CosimSignalValue::F64(1.8))]),
                })
                .expect("step should solve");
        }
        let node = adapter.solver().circuit().node("out").expect("node out");
        let settled = adapter.solver().node_voltage(node);
        assert!(
            (settled - 1.8).abs() < 1e-3,
            "a 1.8 V drive settles at 1.8 V, not vdd: {settled} V"
        );
    }

    #[test]
    fn an_input_that_is_not_a_circuit_source_is_ignored() {
        let mut adapter =
            AnalogCosimAdapter::from_netlist(RC, &rc_config()).expect("adapter builds");
        // Manifests routinely route one signal bundle into several models; the
        // ngspice wrapper ignores names it does not own, and so does this.
        adapter
            .step(CosimStep {
                time_ns: 100_000,
                dt_ns: 100_000,
                inputs: BTreeMap::from([(
                    "some.other.model.signal".to_string(),
                    CosimSignalValue::Bool(true),
                )]),
            })
            .expect("an unrelated routed input must not fail the step");
    }

    #[test]
    fn a_switch_control_needs_no_sources_entry() {
        let netlist = "* sw\nV1 vdd 0 dc 5\nR1 vdd out 1k\nS1 out 0 gate ron=1 roff=1meg\n\
                   C1 out 0 1p\n.end\n";
        let cfg = AnalogConfig {
            probes: BTreeMap::from([("v_out".to_string(), "v(out)".to_string())]),
            ..AnalogConfig::default()
        };
        let mut adapter = AnalogCosimAdapter::from_netlist(netlist, &cfg).expect("adapter builds");

        let open = match adapter
            .step(CosimStep {
                time_ns: 1_000,
                dt_ns: 1_000,
                inputs: BTreeMap::new(),
            })
            .expect("step")
            .outputs["v_out"]
        {
            CosimSignalValue::F64(volts) => volts,
            ref other => panic!("{other:?}"),
        };
        assert!(
            open > 4.9,
            "an open switch leaves the pull-up high: {open} V"
        );

        let closed = match adapter
            .step(CosimStep {
                time_ns: 2_000,
                dt_ns: 1_000,
                inputs: BTreeMap::from([("gate".to_string(), CosimSignalValue::Bool(true))]),
            })
            .expect("step")
            .outputs["v_out"]
        {
            CosimSignalValue::F64(volts) => volts,
            ref other => panic!("{other:?}"),
        };
        assert!(closed < 0.1, "a closed switch pulls it down: {closed} V");
    }

    #[test]
    fn the_trace_ring_is_bounded_and_keeps_the_newest_samples() {
        let cfg = AnalogConfig {
            trace_samples: 8,
            ..rc_config()
        };
        let mut adapter = AnalogCosimAdapter::from_netlist(RC, &cfg).expect("adapter builds");
        for step in 1..=50 {
            step_high(&mut adapter, step * 100_000);
        }

        let trace = adapter.trace_handle();
        let trace = trace.lock().expect("trace lock");
        assert_eq!(trace.len(), 8, "the ring never grows past trace_samples");

        let batch = trace.snapshot(0);
        assert_eq!(batch.samples.len(), 8);
        assert_eq!(
            batch
                .channels
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            vec!["v_out", "v(in)", "i(Vgpio)"],
            "routed probes first, then the extra trace expressions"
        );
        assert_eq!(batch.channels[0].unit, "V");
        assert_eq!(batch.channels[2].unit, "A", "a branch current is amps");
        // 51 rows were pushed (the operating point plus 50 steps); 43 fell out.
        assert_eq!(batch.dropped, 43);
        assert_eq!(batch.next_cursor, 51);
        assert_eq!(batch.samples.last().expect("newest").time_ns, 5_000_000);

        // A cursor read returns only what is newer.
        let tail = trace.snapshot(batch.next_cursor - 2);
        assert_eq!(tail.samples.len(), 2);
    }

    #[test]
    fn the_trace_records_the_operating_point_as_its_first_row() {
        let mut adapter =
            AnalogCosimAdapter::from_netlist(RC, &rc_config()).expect("adapter builds");
        step_high(&mut adapter, 100_000);
        let trace = adapter.trace_handle();
        let batch = trace.lock().expect("trace lock").snapshot(0);
        assert_eq!(batch.samples.len(), 2);
        assert_eq!(batch.samples[0].time_ns, 0);
        assert_eq!(batch.samples[0].cycle, 0);
        assert_eq!(batch.samples[0].values[0], 0.0);
        assert_eq!(batch.samples[1].time_ns, 100_000);
        assert_eq!(batch.samples[1].cycle, 1);
    }

    fn manifest_model(config: HashMap<String, serde_yaml::Value>) -> CosimModelConfig {
        CosimModelConfig {
            id: "rc_lowpass".to_string(),
            adapter: ManifestCosimAdapter::Analog,
            model: None,
            step_ns: 100_000,
            inputs: HashMap::from([("gpio".to_string(), "board.gpio.pa5".to_string())]),
            outputs: HashMap::from([("v_out".to_string(), "board.analog.pa0_volts".to_string())]),
            config,
        }
    }

    fn inline_config() -> HashMap<String, serde_yaml::Value> {
        HashMap::from([
            (
                "netlist_text".to_string(),
                serde_yaml::Value::String(RC.to_string()),
            ),
            (
                "probes".to_string(),
                serde_yaml::from_str("v_out: \"v(out)\"").unwrap(),
            ),
            (
                "sources".to_string(),
                serde_yaml::from_str("gpio: Vgpio").unwrap(),
            ),
        ])
    }

    #[test]
    fn the_registry_builds_an_analog_adapter_from_a_manifest_entry() {
        let config = manifest_model(inline_config());
        let mut adapter =
            build_cosim_adapter_with_base(&config, Path::new(".")).expect("registry builds analog");
        let result = adapter
            .step(CosimStep {
                time_ns: 1_000_000,
                dt_ns: 100_000,
                inputs: BTreeMap::from([("gpio".to_string(), CosimSignalValue::Bool(true))]),
            })
            .expect("step");
        match result.outputs["v_out"] {
            CosimSignalValue::F64(volts) => assert!(volts > 2.0 && volts < 2.2, "{volts} V"),
            ref other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_relative_netlist_path_resolves_against_the_manifest_directory() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/cosim-spice-rc");
        let mut config = inline_config();
        config.remove("netlist_text");
        config.insert(
            "netlist".to_string(),
            serde_yaml::Value::String("./rc.cir".to_string()),
        );
        build_cosim_adapter_with_base(&manifest_model(config), &base)
            .expect("./rc.cir resolves against the manifest directory");
    }

    #[test]
    fn an_unsupported_element_is_a_manifest_validation_error() {
        let mut config = inline_config();
        config.insert(
            "netlist_text".to_string(),
            // A subcircuit: still outside the subset now that `D` is inside it.
            serde_yaml::Value::String(
                "* subcircuit\nV1 a 0 dc 1\nX1 a b opamp\n.end\n".to_string(),
            ),
        );
        let issues = validate_analog_models(&[manifest_model(config)], Path::new("."));
        assert_eq!(issues.len(), 1, "one model, one issue: {issues:?}");
        assert!(
            issues[0].contains("needs ngspice")
                && issues[0].contains("adapter: external_process")
                && issues[0].contains("rc_lowpass"),
            "the issue must name the model and the adapter that can run it: {}",
            issues[0]
        );
    }

    #[test]
    fn a_probe_on_a_node_that_does_not_exist_is_refused() {
        let mut config = inline_config();
        config.insert(
            "probes".to_string(),
            serde_yaml::from_str("v_out: \"v(nowhere)\"").unwrap(),
        );
        // `expect_err` would need `Box<dyn CosimAdapter>: Debug`, which the trait
        // object cannot have; match instead.
        let Err(err) = build_cosim_adapter_with_base(&manifest_model(config), Path::new("."))
        else {
            panic!("a probe that reads nothing must not silently return 0 V");
        };
        assert!(err.to_string().contains("nowhere"), "{err}");
    }

    #[test]
    fn a_manifest_without_probes_is_refused() {
        let mut config = inline_config();
        config.remove("probes");
        let Err(err) = build_cosim_adapter_with_base(&manifest_model(config), Path::new("."))
        else {
            panic!("a model with no probe produces no outputs");
        };
        assert!(err.to_string().contains("probes"), "{err}");
    }

    #[test]
    fn the_runner_publishes_one_trace_for_the_machine() {
        let config = manifest_model(inline_config());
        let adapter = build_cosim_adapter_with_base(&config, Path::new(".")).expect("adapter");
        let mut runner = CosimRunner::new(vec![CosimRunnerModel::new(config, adapter)]);

        let mut signals =
            BTreeMap::from([("board.gpio.pa5".to_string(), CosimSignalValue::Bool(true))]);
        runner
            .step_until_with_signals(1_000_000, &mut signals)
            .expect("ten steps");

        let channels = runner.analog_channels();
        assert_eq!(
            channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["rc_lowpass.v_out"],
            "a single analog model is prefixed with its id like any other"
        );

        let batch = runner.analog_trace_snapshot(0);
        assert_eq!(batch.samples.len(), 11, "operating point plus ten steps");
        assert_eq!(batch.dropped, 0);
        assert!(
            batch.samples.last().expect("newest").values[0] > 2.0,
            "one tau charges past 2 V"
        );
        let routed = match signals["board.analog.pa0_volts"] {
            CosimSignalValue::F64(volts) => volts,
            ref other => panic!("{other:?}"),
        };
        let traced = f64::from(batch.samples.last().expect("newest").values[0]);
        assert!(
            (routed - traced).abs() < 1e-6,
            "the routed signal ({routed} V) and the trace ({traced} V) are the same \
         sample; the trace stores f32 for the scope's sake"
        );
    }

    /// A probe the manifest routes nowhere is still solved and traced: it is
    /// how a circuit publishes a node only an instrument reads. It must build,
    /// appear as `<model id>.<probe>`, and never leak into the signal store.
    #[test]
    fn an_unrouted_probe_is_traced_under_the_model_id() {
        let mut config = inline_config();
        config.insert(
            "probes".to_string(),
            serde_yaml::from_str("v_out: \"v(out)\"\nv_in: \"v(in)\"").unwrap(),
        );
        let mut model = manifest_model(config);
        model.id = "circuit".to_string();
        let adapter = build_cosim_adapter_with_base(&model, Path::new("."))
            .expect("a probe with no route is not an error");
        let mut runner = CosimRunner::new(vec![CosimRunnerModel::new(model, adapter)]);
        assert_eq!(
            runner
                .analog_channels()
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            vec!["circuit.v_in", "circuit.v_out"],
            "probes in name order, each under the model id"
        );

        let mut signals =
            BTreeMap::from([("board.gpio.pa5".to_string(), CosimSignalValue::Bool(true))]);
        let routed = runner
            .step_until_with_signals(500_000, &mut signals)
            .expect("five steps");
        assert!(
            routed.iter().all(|step| step
                .outputs
                .keys()
                .all(|path| path == "board.analog.pa0_volts")),
            "only the routed probe reaches a path: {routed:?}"
        );
        assert!(
            !signals.keys().any(|path| path.contains("v_in")),
            "the unrouted probe must not enter the signal store: {signals:?}"
        );

        let batch = runner.analog_trace_snapshot(0);
        let newest = batch.samples.last().expect("newest");
        assert!(
            (f64::from(newest.values[0]) - 3.3).abs() < 1e-3,
            "v(in) follows the driven source: {} V",
            newest.values[0]
        );
        assert!(
            newest.values[1] > 1.0 && newest.values[1] < 3.3,
            "v(out) is charging: {} V",
            newest.values[1]
        );
    }

    #[test]
    fn two_analog_models_share_one_ring_with_prefixed_channels() {
        let mut first = manifest_model(inline_config());
        first.id = "rc_a".to_string();
        let mut second = manifest_model(inline_config());
        second.id = "rc_b".to_string();
        second.step_ns = 200_000;

        let models = vec![
            CosimRunnerModel::new(
                first.clone(),
                build_cosim_adapter_with_base(&first, Path::new(".")).expect("a"),
            ),
            CosimRunnerModel::new(
                second.clone(),
                build_cosim_adapter_with_base(&second, Path::new(".")).expect("b"),
            ),
        ];
        let mut runner = CosimRunner::new(models);
        assert_eq!(
            runner
                .analog_channels()
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>(),
            vec!["rc_a.v_out", "rc_b.v_out"],
        );

        let mut signals =
            BTreeMap::from([("board.gpio.pa5".to_string(), CosimSignalValue::Bool(true))]);
        runner
            .step_until_with_signals(400_000, &mut signals)
            .expect("steps");

        let batch = runner.analog_trace_snapshot(0);
        assert_eq!(batch.channels.len(), 2);
        for sample in &batch.samples {
            assert_eq!(sample.values.len(), 2, "every row spans every channel");
        }
        // The slower model's column is carried forward between its own updates,
        // which is what a scope channel does between samples.
        assert!(
            batch.samples.iter().all(|s| s.values[1] >= 0.0),
            "no hole is left in a channel that did not step"
        );
    }

    #[test]
    fn a_machine_without_a_runner_answers_an_empty_batch() {
        let mut bus = labwired_core::bus::SystemBus::new();
        let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
        let machine = labwired_core::Machine::new(cpu, bus);
        assert!(machine.analog_channels().is_empty());
        let batch = machine.analog_trace_snapshot(0);
        assert!(batch.samples.is_empty());
        assert_eq!(batch.next_cursor, 0);
    }

    #[test]
    fn a_machine_with_a_runner_sees_the_runner_samples() {
        let config = manifest_model(inline_config());
        let adapter = build_cosim_adapter_with_base(&config, Path::new(".")).expect("adapter");
        let mut runner = CosimRunner::new(vec![CosimRunnerModel::new(config, adapter)]);

        let mut bus = labwired_core::bus::SystemBus::new();
        let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
        let mut machine = labwired_core::Machine::new(cpu, bus);
        machine.attach_analog_trace(runner.analog_trace_registry());

        let mut signals =
            BTreeMap::from([("board.gpio.pa5".to_string(), CosimSignalValue::Bool(true))]);
        runner
            .step_until_with_signals(500_000, &mut signals)
            .expect("five steps");

        assert_eq!(
            machine.analog_channels().len(),
            1,
            "the machine publishes the runner's channels"
        );
        let batch = machine.analog_trace_snapshot(0);
        assert_eq!(batch.samples.len(), 6);
        let second = machine.analog_trace_snapshot(batch.next_cursor);
        assert!(second.samples.is_empty(), "a cursor read acknowledges");
    }

    #[test]
    fn config_keys_are_read_from_the_manifest() {
        let config = HashMap::from([
            (
                "netlist_text".to_string(),
                serde_yaml::Value::String(RC.to_string()),
            ),
            ("vdd".to_string(), serde_yaml::from_str("5.0").unwrap()),
            ("substeps".to_string(), serde_yaml::from_str("4").unwrap()),
            (
                "integration".to_string(),
                serde_yaml::Value::String("trap".to_string()),
            ),
            (
                "probes".to_string(),
                serde_yaml::from_str("v_out: \"v(out)\"").unwrap(),
            ),
            (
                "sources".to_string(),
                serde_yaml::from_str("gpio: Vgpio").unwrap(),
            ),
            (
                "trace".to_string(),
                serde_yaml::from_str("[\"v(in)\"]").unwrap(),
            ),
            (
                "trace_samples".to_string(),
                serde_yaml::from_str("64").unwrap(),
            ),
        ]);
        let parsed = AnalogConfig::from_yaml(&config).expect("config parses");
        assert_eq!(parsed.vdd, 5.0);
        assert_eq!(parsed.substeps, 4);
        assert_eq!(parsed.integration, Integration::Trapezoidal);
        assert_eq!(parsed.trace, vec!["v(in)".to_string()]);
        assert_eq!(parsed.trace_samples, 64);

        let mut bad = config.clone();
        bad.insert(
            "integration".to_string(),
            serde_yaml::Value::String("gear2".to_string()),
        );
        let err = AnalogConfig::from_yaml(&bad).expect_err("an unknown rule must be refused");
        assert!(err.to_string().contains("gear2"), "{err}");
    }
}
