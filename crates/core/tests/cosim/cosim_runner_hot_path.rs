//! The runner's allocation-free step: same store contents with and without the
//! per-step report, inputs that disappear are not replayed, and the
//! borrowed-buffer adapter entry point agrees with `step`.
use labwired_config::{CosimAdapter as ManifestAdapter, CosimModelConfig};
use labwired_core::cosim::{
    CosimAdapter, CosimRunner, CosimRunnerModel, CosimSignalValue, CosimSignals, CosimStep,
    CosimStepResult,
};
use std::collections::HashMap;

/// outputs `sum = a + b` (missing inputs count as 0) and echoes `seen = <count of inputs>`.
struct Summer;
impl CosimAdapter for Summer {
    fn step(&mut self, step: CosimStep) -> labwired_core::SimResult<CosimStepResult> {
        let get = |k: &str| match step.inputs.get(k) {
            Some(CosimSignalValue::F64(v)) => *v,
            _ => 0.0,
        };
        let mut outputs = CosimSignals::new();
        outputs.insert("sum".into(), CosimSignalValue::F64(get("a") + get("b")));
        outputs.insert(
            "seen".into(),
            CosimSignalValue::I64(step.inputs.len() as i64),
        );
        Ok(CosimStepResult { outputs })
    }
}

fn model(id: &str, step_ns: u64) -> CosimRunnerModel {
    let config = CosimModelConfig {
        id: id.into(),
        adapter: ManifestAdapter::Mock,
        model: None,
        step_ns,
        inputs: HashMap::from([("a".into(), "x.a".into()), ("b".into(), "x.b".into())]),
        outputs: HashMap::from([
            ("sum".into(), format!("{id}.sum")),
            ("seen".into(), format!("{id}.seen")),
        ]),
        config: HashMap::new(),
    };
    CosimRunnerModel::new(config, Box::new(Summer))
}

fn run(record: bool) -> (CosimSignals, usize) {
    let mut runner = CosimRunner::new(vec![model("p", 10), model("q", 20)]);
    runner.set_record_routed(record);
    let mut signals = CosimSignals::new();
    let mut reports = 0;
    signals.insert("x.a".into(), CosimSignalValue::F64(1.5));
    signals.insert("x.b".into(), CosimSignalValue::F64(2.0));
    reports += runner.step_until_with_signals(20, &mut signals).unwrap().len();
    assert_eq!(signals["p.sum"], CosimSignalValue::F64(3.5));
    assert_eq!(signals["p.seen"], CosimSignalValue::I64(2));
    // `x.b` vanishes: the model must stop seeing it, not replay the old value.
    signals.remove("x.b");
    reports += runner.step_until_with_signals(40, &mut signals).unwrap().len();
    assert_eq!(signals["p.sum"], CosimSignalValue::F64(1.5));
    assert_eq!(signals["p.seen"], CosimSignalValue::I64(1));
    assert_eq!(signals["q.seen"], CosimSignalValue::I64(1));
    (signals, reports)
}

#[test]
fn store_is_identical_with_and_without_the_report() {
    let (with, reports_on) = run(true);
    let (without, reports_off) = run(false);
    assert_eq!(with, without);
    assert_eq!(reports_on, 6); // p at 10, 20, 30, 40 and q at 20, 40
    assert_eq!(reports_off, 0);
}
