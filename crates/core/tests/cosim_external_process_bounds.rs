#![cfg(unix)]
use labwired_core::cosim::{
    CosimAdapter, CosimSignalValue, CosimSignals, CosimStep, ExternalProcessCosimAdapter,
};
use std::time::{Duration, Instant};
fn step(inputs: CosimSignals) -> CosimStep {
    CosimStep {
        time_ns: 100_000_000,
        dt_ns: 100_000_000,
        inputs,
    }
}
fn fixture(script: &str) -> ExternalProcessCosimAdapter {
    ExternalProcessCosimAdapter::spawn("python3", &["-u", "-c", script]).unwrap()
}
#[test]
fn response_and_request_io_have_one_bounded_deadline() {
    for script in ["import time; time.sleep(3)", "import sys,time; sys.stdin.readline(); sys.stdout.write('{'); sys.stdout.flush(); time.sleep(3)"] {
        let mut adapter = fixture(script);
        let start = Instant::now();
        assert!(adapter.step(step(CosimSignals::new())).is_err());
        assert!(start.elapsed() < Duration::from_millis(2800), "unbounded response wait: {:?}", start.elapsed());
    }
}
#[test]
fn oversized_valid_response_is_rejected() {
    let mut adapter = fixture("import sys,json; sys.stdin.readline(); print(json.dumps({'outputs':{'text':'x'*1000000}}))");
    assert!(
        adapter.step(step(CosimSignals::new())).is_err(),
        "oversized response accepted"
    );
}
#[test]
fn malformed_response_is_terminal_without_another_request() {
    let mut adapter = fixture("import sys; sys.stdin.readline(); print('invalid'); sys.stdin.readline(); print('{\"outputs\":{}}')");
    assert!(adapter.step(step(CosimSignals::new())).is_err());
    assert!(
        adapter.step(step(CosimSignals::new())).is_err(),
        "adapter retried failed protocol"
    );
}
#[test]
fn finite_request_is_required() {
    let mut adapter = fixture(
        "import sys,json; [print(json.dumps({'outputs':{}}),flush=True) for l in sys.stdin]",
    );
    let mut inputs = CosimSignals::new();
    inputs.insert("bad".into(), CosimSignalValue::F64(f64::NAN));
    assert!(
        adapter.step(step(inputs)).is_err(),
        "nonfinite request silently encoded null"
    );
}
#[test]
fn blocked_stdin_is_bounded() {
    let mut adapter = fixture("import time; time.sleep(3)");
    let mut inputs = CosimSignals::new();
    inputs.insert("payload".into(), CosimSignalValue::Text("x".repeat(60_000)));
    let start = Instant::now();
    assert!(adapter.step(step(inputs)).is_err());
    assert!(
        start.elapsed() < Duration::from_millis(2800),
        "unbounded write: {:?}",
        start.elapsed()
    );
}

#[test]
fn persistent_child_keeps_state_across_multiple_steps() {
    let mut adapter = fixture("import sys,json,os\nn=0\nfor l in sys.stdin:\n n+=1\n print(json.dumps({'outputs':{'count':n,'pid':os.getpid(),'bool':True,'number':0.5,'text':'value'}}),flush=True)");
    let mut pid = None;
    for n in 1..=4 {
        let result = adapter.step(step(CosimSignals::new())).unwrap();
        assert_eq!(result.outputs["count"], CosimSignalValue::I64(n));
        assert_eq!(result.outputs["bool"], CosimSignalValue::Bool(true));
        assert_eq!(result.outputs["number"], CosimSignalValue::F64(0.5));
        assert_eq!(
            result.outputs["text"],
            CosimSignalValue::Text("value".into())
        );
        if let Some(ref old) = pid {
            assert_eq!(old, &result.outputs["pid"]);
        }
        pid = Some(result.outputs["pid"].clone());
    }
}
#[test]
fn extra_frame_beyond_exact_read_boundary_is_terminal() {
    let mut adapter = fixture("import sys,os,json,time\nsys.stdin.readline()\nprefix=json.dumps({'outputs':{'count':1,'pad':''}},separators=(',',':'))\nfirst=json.dumps({'outputs':{'count':1,'pad':'x'*(4096-len(prefix)-1)}},separators=(',',':')).encode()+b'\\n'\nassert len(first)==4096\nos.write(1,first+b'{\"outputs\":{\"count\":999}}\\n')\nsys.stdin.readline()\ntime.sleep(10)");
    let result = adapter.step(step(CosimSignals::new()));
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("unsolicited queued bytes"));
    let later = Instant::now();
    assert!(adapter
        .step(step(CosimSignals::new()))
        .unwrap_err()
        .to_string()
        .contains("terminally failed"));
    assert!(later.elapsed() < Duration::from_millis(50));
}
#[test]
fn delayed_frame_queued_before_next_request_is_terminal() {
    struct OwnedDir(std::path::PathBuf);
    impl Drop for OwnedDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let owned = OwnedDir(std::env::temp_dir().join(format!(
        "labwired-framing-handshake-{}-{unique}",
        std::process::id()
    )));
    std::fs::create_dir(&owned.0).unwrap();
    let script = "import sys,os,time\nroot=sys.argv[1]\nsys.stdin.readline()\nos.write(1,b'{\"outputs\":{\"count\":1}}\\n')\nwhile not os.path.exists(root+'/emit'): time.sleep(.001)\ntime.sleep(.05)\nos.write(1,b'{\"outputs\":{\"count\":999}}\\n')\nwith open(root+'/queued','w') as ack: ack.write('queued')\nsys.stdin.readline()\ntime.sleep(10)";
    let mut adapter = ExternalProcessCosimAdapter::spawn(
        "python3",
        &["-u", "-c", script, owned.0.to_str().unwrap()],
    )
    .unwrap();
    assert_eq!(
        adapter.step(step(CosimSignals::new())).unwrap().outputs["count"],
        CosimSignalValue::I64(1)
    );
    // Trigger only after first success, then require an acknowledgement written
    // after os.write: the duplicate is demonstrably queued before the next step.
    std::fs::write(owned.0.join("emit"), b"emit").unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    while !owned.0.join("queued").exists() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        owned.0.join("queued").exists(),
        "fixture did not queue frame"
    );
    let start = Instant::now();
    let result = adapter.step(step(CosimSignals::new()));
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("unsolicited queued bytes"));
    assert!(start.elapsed() < Duration::from_millis(400));
    assert!(adapter
        .step(step(CosimSignals::new()))
        .unwrap_err()
        .to_string()
        .contains("terminally failed"));
}
#[test]
fn extra_frame_in_same_read_is_terminal() {
    let mut adapter = fixture("import sys,os,time\nsys.stdin.readline()\nos.write(1,b'{\"outputs\":{\"count\":1}}\\n{\"outputs\":{\"count\":999}}\\n')\ntime.sleep(10)");
    assert!(adapter
        .step(step(CosimSignals::new()))
        .unwrap_err()
        .to_string()
        .contains("unsolicited trailing bytes"));
    assert!(adapter
        .step(step(CosimSignals::new()))
        .unwrap_err()
        .to_string()
        .contains("terminally failed"));
}
#[test]
fn valid_exact_read_boundary_frame_preserves_sequential_protocol() {
    let mut adapter = fixture("import sys,os,json\nfor line in sys.stdin:\n prefix=json.dumps({'outputs':{'count':1,'pad':''}},separators=(',',':'))\n frame=json.dumps({'outputs':{'count':1,'pad':'x'*(4096-len(prefix)-1)}},separators=(',',':')).encode()+b'\\n'\n assert len(frame)==4096\n os.write(1,frame)");
    for _ in 0..3 {
        assert_eq!(
            adapter.step(step(CosimSignals::new())).unwrap().outputs["count"],
            CosimSignalValue::I64(1)
        );
    }
}
#[test]
fn complete_valid_one_shot_response_preserves_eof_semantics() {
    let mut adapter = fixture("import sys,os\nsys.stdin.readline()\nos.write(1,b'{\"outputs\":{\"count\":1}}\\n')\nos.close(1)");
    assert_eq!(
        adapter.step(step(CosimSignals::new())).unwrap().outputs["count"],
        CosimSignalValue::I64(1)
    );
    assert!(adapter.step(step(CosimSignals::new())).is_err());
    assert!(adapter
        .step(step(CosimSignals::new()))
        .unwrap_err()
        .to_string()
        .contains("terminally failed"));
}
fn short_fixture(script: &str) -> ExternalProcessCosimAdapter {
    use labwired_core::cosim::ExternalProcessCosimOptions;
    ExternalProcessCosimAdapter::spawn_with_options(
        "python3",
        &["-u", "-c", script],
        ExternalProcessCosimOptions {
            timeout: Duration::from_millis(180),
            ..Default::default()
        },
    )
    .unwrap()
}
#[test]
fn no_newline_child_exit_invalid_shapes_and_oversize_are_terminal() {
    for script in [
        "import sys,time; sys.stdin.readline(); sys.stdout.write('{'); sys.stdout.flush(); time.sleep(10)",
        "import sys; sys.stdin.readline(); sys.exit(1)",
        "import sys; sys.stdin.readline(); print('{\"outputs\":{\"nested\":[]}}')",
        "import sys; sys.stdin.readline(); print('{\"outputs\":{\"bad\":1e999}}')",
        "import sys; sys.stdin.readline(); sys.stdout.write('x'*1000000); sys.stdout.flush()",
    ] {
        let mut adapter = short_fixture(script);
        let start = Instant::now();
        assert!(adapter.step(step(CosimSignals::new())).is_err());
        assert!(start.elapsed() < Duration::from_millis(750));
        let later = Instant::now();
        let err = adapter.step(step(CosimSignals::new())).unwrap_err().to_string();
        assert!(err.contains("terminally failed"));
        assert!(later.elapsed() < Duration::from_millis(50));
    }
}
#[test]
fn request_text_count_and_json_escaping_are_bounded_before_io() {
    for value in [
        CosimSignalValue::Text("x".repeat(1_000_000)),
        CosimSignalValue::Text("\0".repeat(20_000)),
    ] {
        let mut adapter = short_fixture("import time; time.sleep(10)");
        let mut inputs = CosimSignals::new();
        inputs.insert("text".into(), value);
        let start = Instant::now();
        let err = adapter.step(step(inputs)).unwrap_err().to_string();
        assert!(err.contains("byte limit"));
        assert!(start.elapsed() < Duration::from_millis(150));
    }
}
#[test]
fn write_then_read_share_deadline() {
    let mut adapter = short_fixture("import sys,time; time.sleep(.1); sys.stdin.readline(); time.sleep(.15); print('{\"outputs\":{}}')");
    let mut inputs = CosimSignals::new();
    inputs.insert("text".into(), CosimSignalValue::Text("x".repeat(60_000)));
    let start = Instant::now();
    assert!(
        adapter.step(step(inputs)).is_err(),
        "read got a fresh deadline after blocked write"
    );
    assert!(start.elapsed() < Duration::from_millis(400));
}
#[cfg(target_os = "linux")]
fn alive(pid: i64) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .is_some_and(|s| !s.rsplit_once(") ").unwrap().1.starts_with('Z'))
}
#[test]
#[cfg(target_os = "linux")]
fn owned_descendant_inheriting_stdio_is_killed_on_timeout_and_drop() {
    for failure in [false, true] {
        let mut adapter=short_fixture("import sys,os,json,time\npid=os.fork()\nif pid==0: time.sleep(10); os._exit(0)\nsys.stdin.readline()\nprint(json.dumps({'outputs':{'child':pid,'parent':os.getpid()}}),flush=True)\ntime.sleep(10)");
        let output = adapter.step(step(CosimSignals::new())).unwrap().outputs;
        let pid = |key| match output[key] {
            CosimSignalValue::I64(n) => n,
            _ => panic!("pid"),
        };
        let child = pid("child");
        let parent = pid("parent");
        assert!(alive(child));
        assert!(alive(parent));
        let start = Instant::now();
        if failure {
            assert!(adapter.step(step(CosimSignals::new())).is_err());
        }
        drop(adapter);
        assert!(start.elapsed() < Duration::from_millis(750));
        let until = Instant::now() + Duration::from_millis(250);
        while alive(child) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!alive(child), "owned descendant survived");
        assert!(!alive(parent), "owned leader survived");
        assert!(
            !std::path::Path::new(&format!("/proc/{parent}")).exists(),
            "owned leader not reaped"
        );
    }
}
#[test]
fn options_reject_unbounded_settings() {
    use labwired_core::cosim::ExternalProcessCosimOptions;
    for options in [
        ExternalProcessCosimOptions {
            timeout: Duration::ZERO,
            ..Default::default()
        },
        ExternalProcessCosimOptions {
            max_response_bytes: usize::MAX,
            ..Default::default()
        },
        ExternalProcessCosimOptions {
            max_request_bytes: 0,
            ..Default::default()
        },
    ] {
        assert!(ExternalProcessCosimAdapter::spawn_with_options("python3", &[], options).is_err());
    }
}
