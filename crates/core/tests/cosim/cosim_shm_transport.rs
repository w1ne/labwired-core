#![cfg(target_os = "linux")]
//! The opt-in shared-memory transport for external-process models, with the
//! Python and C clients under `tools/cosim/`, and the JSONL fallback.
use labwired_core::cosim::{
    CosimAdapter, CosimSignalValue, CosimSignals, ExternalProcessCosimAdapter,
    ExternalProcessCosimOptions,
};
use std::path::PathBuf;
use std::time::Duration;

fn tools() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/cosim")
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn run_steps(adapter: &mut ExternalProcessCosimAdapter) {
    let mut outputs = CosimSignals::new();
    for (n, level) in [(1u64, true), (2, false), (3, true)] {
        let mut inputs = CosimSignals::new();
        inputs.insert("gpio".into(), CosimSignalValue::Bool(level));
        adapter
            .step_into(n * 1000, 1000, &inputs, &mut outputs)
            .unwrap();
        let expect = if level { 3.3 } else { 0.0 };
        assert_eq!(outputs.get("v_out"), Some(&CosimSignalValue::F64(expect)));
    }
}

#[test]
fn python_client_steps_over_shm() {
    let script = tools().join("echo_shm.py");
    let mut adapter = ExternalProcessCosimAdapter::spawn_shm(
        "python3",
        &[script.to_str().unwrap()],
        ExternalProcessCosimOptions::default(),
        &names(&["gpio"]),
        &names(&["v_out"]),
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(adapter.uses_shm(), "python client did not negotiate shm");
    run_steps(&mut adapter);
}

#[test]
fn c_client_steps_over_shm() {
    let dir = std::env::temp_dir().join(format!("lw-shm-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("echo_shm");
    let built = std::process::Command::new("cc")
        .arg("-O2")
        .arg("-o")
        .arg(&exe)
        .arg(tools().join("echo_shm.c"))
        .status();
    let Ok(status) = built else {
        eprintln!("no C compiler; skipping");
        return;
    };
    assert!(status.success(), "echo_shm.c failed to build");
    let mut adapter = ExternalProcessCosimAdapter::spawn_shm(
        exe.to_str().unwrap(),
        &[],
        ExternalProcessCosimOptions::default(),
        &names(&["gpio"]),
        &names(&["v_out"]),
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(adapter.uses_shm());
    run_steps(&mut adapter);
    drop(adapter);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn silent_jsonl_child_falls_back_to_pipes() {
    let script = "import sys,json\n\
        for l in sys.stdin:\n\
        \x20   g=json.loads(l)['inputs'].get('gpio')\n\
        \x20   print(json.dumps({'outputs':{'v_out':3.3 if g else 0.0}}),flush=True)";
    let mut adapter = ExternalProcessCosimAdapter::spawn_shm(
        "python3",
        &["-u", "-c", script],
        ExternalProcessCosimOptions::default(),
        &names(&["gpio"]),
        &names(&["v_out"]),
        Duration::from_millis(300),
    )
    .unwrap();
    assert!(!adapter.uses_shm());
    run_steps(&mut adapter);
}

#[test]
fn shm_rejects_text_and_undeclared_inputs_and_dead_child() {
    let script = tools().join("echo_shm.py");
    let mut adapter = ExternalProcessCosimAdapter::spawn_shm(
        "python3",
        &[script.to_str().unwrap()],
        ExternalProcessCosimOptions::default(),
        &names(&["gpio"]),
        &names(&["v_out"]),
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(adapter.uses_shm());
    let mut outputs = CosimSignals::new();
    let mut inputs = CosimSignals::new();
    inputs.insert("other".into(), CosimSignalValue::F64(1.0));
    assert!(adapter.step_into(1, 1, &inputs, &mut outputs).is_err());
    // An error is terminal, like every other adapter failure.
    assert!(adapter
        .step_into(2, 1, &CosimSignals::new(), &mut outputs)
        .is_err());

    let mut dying = ExternalProcessCosimAdapter::spawn_shm(
        "python3",
        &[
            "-c",
            "import os,sys,time\nsys.stdout.write('{\"labwired_shm\":1}\\n');sys.stdout.flush()\nos._exit(0)",
        ],
        ExternalProcessCosimOptions::default(),
        &names(&["gpio"]),
        &names(&["v_out"]),
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(dying.uses_shm());
    let start = std::time::Instant::now();
    assert!(dying
        .step_into(1, 1, &CosimSignals::new(), &mut outputs)
        .is_err());
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "dead child not noticed promptly"
    );
}
