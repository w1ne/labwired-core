use labwired_core::cosim::{
    CosimAdapter, CosimSignalValue, CosimStep, CosimStepResult, StaticCosimAdapter,
};
use std::collections::BTreeMap;

#[test]
fn static_cosim_adapter_returns_configured_outputs_for_a_step() {
    let mut outputs = BTreeMap::new();
    outputs.insert("plant_value".to_string(), CosimSignalValue::F64(48.0));
    outputs.insert("plant_ready".to_string(), CosimSignalValue::Bool(true));
    let mut adapter = StaticCosimAdapter::new(outputs.clone());

    let mut inputs = BTreeMap::new();
    inputs.insert(
        "controller_enable".to_string(),
        CosimSignalValue::Bool(true),
    );

    let result = adapter
        .step(CosimStep {
            time_ns: 20_000,
            dt_ns: 10_000,
            inputs,
        })
        .expect("static co-sim step should succeed");

    assert_eq!(result, CosimStepResult { outputs });
}

#[test]
fn external_process_adapter_round_trips_one_jsonl_step() {
    use labwired_core::cosim::ExternalProcessCosimAdapter;

    let script = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/cosim_echo.py");
    let mut adapter =
        ExternalProcessCosimAdapter::spawn("python3", &[script.to_string_lossy().as_ref()])
            .expect("spawn echo adapter");

    let mut inputs = BTreeMap::new();
    inputs.insert(
        "controller_enable".to_string(),
        CosimSignalValue::Bool(true),
    );

    let result = adapter
        .step(CosimStep {
            time_ns: 10_000,
            dt_ns: 10_000,
            inputs,
        })
        .expect("step should round trip");

    assert_eq!(result.outputs["ack"], CosimSignalValue::Bool(true));
    assert_eq!(result.outputs["time_ns"], CosimSignalValue::I64(10_000));
}

#[test]
fn discrete_plant_sums_enabled_channels() {
    use labwired_core::cosim::ExternalProcessCosimAdapter;

    let script = std::env::current_dir()
        .unwrap()
        .join("../../examples/cosim-plant-demo/models/discrete-plant.py");
    let mut adapter =
        ExternalProcessCosimAdapter::spawn("python3", &[script.to_string_lossy().as_ref()])
            .expect("spawn discrete plant adapter");

    let mut inputs = BTreeMap::new();
    for index in 0..12 {
        inputs.insert(
            format!("channel{index}_enabled"),
            CosimSignalValue::Bool(true),
        );
    }

    let result = adapter
        .step(CosimStep {
            time_ns: 10_000,
            dt_ns: 10_000,
            inputs,
        })
        .expect("discrete plant step should return outputs");

    assert_eq!(result.outputs["v_out"], CosimSignalValue::F64(12.0));
    assert_eq!(result.outputs["active_channels"], CosimSignalValue::I64(12));
}

#[test]
fn discrete_plant_excludes_disabled_channels() {
    use labwired_core::cosim::ExternalProcessCosimAdapter;

    let script = std::env::current_dir()
        .unwrap()
        .join("../../examples/cosim-plant-demo/models/discrete-plant.py");
    let mut adapter =
        ExternalProcessCosimAdapter::spawn("python3", &[script.to_string_lossy().as_ref()])
            .expect("spawn discrete plant adapter");

    let mut inputs = BTreeMap::new();
    for index in 0..12 {
        inputs.insert(
            format!("channel{index}_enabled"),
            CosimSignalValue::Bool(true),
        );
    }
    inputs.insert(
        "disabled_channels".to_string(),
        CosimSignalValue::Text("[\"channel11\", 0]".to_string()),
    );

    let result = adapter
        .step(CosimStep {
            time_ns: 20_000,
            dt_ns: 10_000,
            inputs,
        })
        .expect("discrete plant step should return outputs");

    assert_eq!(result.outputs["v_out"], CosimSignalValue::F64(10.0));
    assert_eq!(result.outputs["active_channels"], CosimSignalValue::I64(10));
}

#[test]
fn cosim_registry_builds_mock_adapter_from_manifest_config() {
    use labwired_config::{CosimAdapter as ManifestCosimAdapter, CosimModelConfig};
    use labwired_core::cosim::build_cosim_adapter;
    use std::collections::HashMap;

    let config = CosimModelConfig {
        id: "mock_plant".to_string(),
        adapter: ManifestCosimAdapter::Mock,
        model: None,
        step_ns: 10_000,
        inputs: HashMap::new(),
        outputs: HashMap::new(),
        config: HashMap::from([(
            "outputs".to_string(),
            serde_yaml::from_str(
                r#"
plant_value: 48.0
active_channels: 12
plant_ready: true
label: generic-plant
"#,
            )
            .unwrap(),
        )]),
    };

    let mut adapter = build_cosim_adapter(&config).expect("build mock adapter");
    let result = adapter
        .step(CosimStep {
            time_ns: 10_000,
            dt_ns: 10_000,
            inputs: BTreeMap::new(),
        })
        .expect("mock step should succeed");

    assert_eq!(result.outputs["plant_value"], CosimSignalValue::F64(48.0));
    assert_eq!(result.outputs["active_channels"], CosimSignalValue::I64(12));
    assert_eq!(result.outputs["plant_ready"], CosimSignalValue::Bool(true));
    assert_eq!(
        result.outputs["label"],
        CosimSignalValue::Text("generic-plant".to_string())
    );
}

#[test]
fn cosim_runner_steps_models_at_configured_boundaries() {
    use labwired_config::{CosimAdapter as ManifestCosimAdapter, CosimModelConfig};
    use labwired_core::cosim::{CosimRunner, CosimRunnerModel};
    use std::collections::HashMap;

    let config = CosimModelConfig {
        id: "mock_plant".to_string(),
        adapter: ManifestCosimAdapter::Mock,
        model: None,
        step_ns: 10,
        inputs: HashMap::new(),
        outputs: HashMap::new(),
        config: HashMap::from([(
            "outputs".to_string(),
            serde_yaml::from_str("plant_value: 1.0").unwrap(),
        )]),
    };
    let adapter = labwired_core::cosim::build_cosim_adapter(&config).expect("build mock adapter");
    let mut runner = CosimRunner::new(vec![CosimRunnerModel::new(config, adapter)]);

    assert!(runner.step_until(9, BTreeMap::new()).unwrap().is_empty());

    let first = runner.step_until(10, BTreeMap::new()).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].model_id, "mock_plant");
    assert_eq!(
        first[0].result.outputs["plant_value"],
        CosimSignalValue::F64(1.0)
    );

    assert!(runner.step_until(19, BTreeMap::new()).unwrap().is_empty());
    assert_eq!(runner.step_until(20, BTreeMap::new()).unwrap().len(), 1);
}

#[test]
fn cosim_runner_maps_signal_paths_into_model_inputs_and_outputs() {
    use labwired_config::{CosimAdapter as ManifestCosimAdapter, CosimModelConfig};
    use labwired_core::cosim::{CosimRunner, CosimRunnerModel};
    use std::collections::HashMap;

    struct GenericPlantAdapter;

    impl CosimAdapter for GenericPlantAdapter {
        fn step(&mut self, step: CosimStep) -> labwired_core::SimResult<CosimStepResult> {
            let enabled = matches!(
                step.inputs.get("controller_enable"),
                Some(CosimSignalValue::Bool(true))
            );
            let load = match step.inputs.get("load_torque_nm") {
                Some(CosimSignalValue::F64(value)) => *value,
                _ => 0.0,
            };
            let speed = if enabled { 1000.0 - load * 10.0 } else { 0.0 };
            Ok(CosimStepResult {
                outputs: BTreeMap::from([
                    ("shaft_speed_rpm".to_string(), CosimSignalValue::F64(speed)),
                    ("plant_ready".to_string(), CosimSignalValue::Bool(enabled)),
                ]),
            })
        }
    }

    let config = CosimModelConfig {
        id: "generic_plant".to_string(),
        adapter: ManifestCosimAdapter::Mock,
        model: None,
        step_ns: 10,
        inputs: HashMap::from([
            (
                "controller_enable".to_string(),
                "control.enable".to_string(),
            ),
            (
                "load_torque_nm".to_string(),
                "plant.load.torque_nm".to_string(),
            ),
        ]),
        outputs: HashMap::from([
            (
                "shaft_speed_rpm".to_string(),
                "observables.shaft_speed_rpm".to_string(),
            ),
            (
                "plant_ready".to_string(),
                "observables.plant_ready".to_string(),
            ),
        ]),
        config: HashMap::new(),
    };
    let mut runner = CosimRunner::new(vec![CosimRunnerModel::new(
        config,
        Box::new(GenericPlantAdapter),
    )]);

    let mut signals = BTreeMap::from([
        ("control.enable".to_string(), CosimSignalValue::Bool(true)),
        (
            "plant.load.torque_nm".to_string(),
            CosimSignalValue::F64(12.5),
        ),
    ]);

    let routed = runner.step_until_with_signals(10, &mut signals).unwrap();

    assert_eq!(routed.len(), 1);
    assert_eq!(
        routed[0].outputs["observables.shaft_speed_rpm"],
        CosimSignalValue::F64(875.0)
    );
    assert_eq!(
        signals["observables.plant_ready"],
        CosimSignalValue::Bool(true)
    );
    assert_eq!(
        signals["observables.shaft_speed_rpm"],
        CosimSignalValue::F64(875.0)
    );
}

#[test]
fn manifest_builds_runner_with_manifest_relative_model_path() {
    use labwired_config::SystemManifest;
    use labwired_core::cosim::CosimRunner;

    let manifest_path = std::env::current_dir()
        .unwrap()
        .join("../../examples/cosim-plant-demo/system.yaml");
    let manifest = SystemManifest::from_file(&manifest_path).unwrap();
    let mut runner = CosimRunner::from_configs_with_base(
        &manifest.cosim_models,
        manifest_path.parent().unwrap(),
    )
    .expect("build plant-demo runner");

    let mut signals = BTreeMap::new();
    for index in 0..12 {
        signals.insert(
            format!("plant.channels.{index}.enabled"),
            CosimSignalValue::Bool(true),
        );
    }
    signals.insert(
        "scenario.disabled_channels".to_string(),
        CosimSignalValue::Text("[\"channel11\", 0]".to_string()),
    );

    let routed = runner
        .step_until_with_signals(10_000, &mut signals)
        .unwrap();

    assert_eq!(routed.len(), 1);
    assert_eq!(signals["plant.active_channels"], CosimSignalValue::I64(10));
    assert_eq!(signals["plant.output.voltage"], CosimSignalValue::F64(10.0));
    assert_eq!(signals["plant.output.current"], CosimSignalValue::F64(10.0));
}

#[test]
fn cosim_runner_steps_models_in_global_chronological_order() {
    use labwired_config::{CosimAdapter as ManifestCosimAdapter, CosimModelConfig};
    use labwired_core::cosim::{CosimRunner, CosimRunnerModel};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    type RecordedCall = (String, u64, Option<CosimSignalValue>);

    struct RecordingAdapter {
        model_id: String,
        calls: Arc<Mutex<Vec<RecordedCall>>>,
        output_voltage_from_time: bool,
    }

    impl CosimAdapter for RecordingAdapter {
        fn step(&mut self, step: CosimStep) -> labwired_core::SimResult<CosimStepResult> {
            self.calls.lock().unwrap().push((
                self.model_id.clone(),
                step.time_ns,
                step.inputs.get("voltage").cloned(),
            ));

            let outputs = if self.output_voltage_from_time {
                BTreeMap::from([(
                    "voltage".to_string(),
                    CosimSignalValue::F64(step.time_ns as f64),
                )])
            } else {
                BTreeMap::new()
            };
            Ok(CosimStepResult { outputs })
        }
    }

    let calls = Arc::new(Mutex::new(Vec::new()));
    // Put the slower model first to ensure scheduling is by deadline, not model order.
    let slow = CosimModelConfig {
        id: "slow_reader".to_string(),
        adapter: ManifestCosimAdapter::Mock,
        model: None,
        step_ns: 20,
        inputs: HashMap::from([("voltage".to_string(), "plant.voltage".to_string())]),
        outputs: HashMap::new(),
        config: HashMap::new(),
    };
    let fast = CosimModelConfig {
        id: "fast_writer".to_string(),
        adapter: ManifestCosimAdapter::Mock,
        model: None,
        step_ns: 10,
        inputs: HashMap::new(),
        outputs: HashMap::from([("voltage".to_string(), "plant.voltage".to_string())]),
        config: HashMap::new(),
    };
    let mut runner = CosimRunner::new(vec![
        CosimRunnerModel::new(
            slow,
            Box::new(RecordingAdapter {
                model_id: "slow_reader".to_string(),
                calls: Arc::clone(&calls),
                output_voltage_from_time: false,
            }),
        ),
        CosimRunnerModel::new(
            fast,
            Box::new(RecordingAdapter {
                model_id: "fast_writer".to_string(),
                calls: Arc::clone(&calls),
                output_voltage_from_time: true,
            }),
        ),
    ]);
    let mut signals = BTreeMap::from([("plant.voltage".to_string(), CosimSignalValue::F64(0.0))]);

    runner.step_until_with_signals(20, &mut signals).unwrap();

    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            ("fast_writer".to_string(), 10, None),
            (
                "slow_reader".to_string(),
                20,
                Some(CosimSignalValue::F64(10.0))
            ),
            ("fast_writer".to_string(), 20, None),
        ]
    );
}

#[test]
fn cosim_runner_uses_pre_boundary_snapshot_for_models_due_together() {
    use labwired_config::{CosimAdapter as ManifestCosimAdapter, CosimModelConfig};
    use labwired_core::cosim::{CosimRunner, CosimRunnerModel};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    let reader_inputs = Arc::new(Mutex::new(Vec::new()));

    struct WriterAdapter;

    impl CosimAdapter for WriterAdapter {
        fn step(&mut self, _step: CosimStep) -> labwired_core::SimResult<CosimStepResult> {
            Ok(CosimStepResult {
                outputs: BTreeMap::from([("voltage".to_string(), CosimSignalValue::F64(2.0))]),
            })
        }
    }

    struct ReaderAdapter(Arc<Mutex<Vec<Option<CosimSignalValue>>>>);

    impl CosimAdapter for ReaderAdapter {
        fn step(&mut self, step: CosimStep) -> labwired_core::SimResult<CosimStepResult> {
            self.0
                .lock()
                .unwrap()
                .push(step.inputs.get("voltage").cloned());
            Ok(CosimStepResult::default())
        }
    }

    let writer = CosimModelConfig {
        id: "writer".to_string(),
        adapter: ManifestCosimAdapter::Mock,
        model: None,
        step_ns: 10,
        inputs: HashMap::new(),
        outputs: HashMap::from([("voltage".to_string(), "plant.voltage".to_string())]),
        config: HashMap::new(),
    };
    let reader = CosimModelConfig {
        id: "reader".to_string(),
        adapter: ManifestCosimAdapter::Mock,
        model: None,
        step_ns: 10,
        inputs: HashMap::from([("voltage".to_string(), "plant.voltage".to_string())]),
        outputs: HashMap::new(),
        config: HashMap::new(),
    };
    let mut runner = CosimRunner::new(vec![
        CosimRunnerModel::new(writer, Box::new(WriterAdapter)),
        CosimRunnerModel::new(reader, Box::new(ReaderAdapter(Arc::clone(&reader_inputs)))),
    ]);
    let mut signals = BTreeMap::from([("plant.voltage".to_string(), CosimSignalValue::F64(1.0))]);

    runner.step_until_with_signals(10, &mut signals).unwrap();

    assert_eq!(
        *reader_inputs.lock().unwrap(),
        vec![Some(CosimSignalValue::F64(1.0))]
    );
    assert_eq!(signals["plant.voltage"], CosimSignalValue::F64(2.0));
}

#[test]
fn runner_rejects_zero_and_overflowing_periods_without_stepping() {
    use labwired_config::{CosimAdapter as ManifestAdapter, CosimModelConfig};
    use labwired_core::cosim::{CosimRunner, CosimRunnerModel};
    use std::collections::HashMap;
    for period in [0, u64::MAX] {
        let config = CosimModelConfig {
            id: "invalid".into(),
            adapter: ManifestAdapter::Mock,
            model: None,
            step_ns: period,
            inputs: HashMap::new(),
            outputs: HashMap::new(),
            config: HashMap::new(),
        };
        let mut runner = CosimRunner::new(vec![CosimRunnerModel::new(
            config,
            Box::new(StaticCosimAdapter::new(BTreeMap::new())),
        )]);
        let error = runner
            .step_until(period, BTreeMap::new())
            .unwrap_err()
            .to_string();
        assert!(error.contains("invalid"), "{error}");
        assert!(
            error.contains(if period == 0 { "step_ns" } else { "overflow" }),
            "{error}"
        );
    }
}
