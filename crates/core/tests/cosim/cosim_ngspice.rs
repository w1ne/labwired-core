// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Native `adapter: ngspice`: the libngspice host behind the generic
//! `external_process` transport, validated with the browser runtime's keys.
//!
//! Two kinds of test live here:
//! - manifest checks (`NgspiceModelSpec`), which need no solver and always run;
//! - real solver runs, which need python3 and libngspice. Without them each such
//!   test prints `SKIP:` and returns; set `LABWIRED_REQUIRE_NGSPICE=1` to make a
//!   missing solver a failure (use it where ngspice is installed on purpose).
//!
//! Every expected voltage is computed here from the circuit equations, not
//! read back from an earlier ngspice run.

use crate::common;
use common::root;
use labwired_config::{ChipDescriptor, CosimModelConfig, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cosim::ngspice::{
    runtime_snapshot_refusal, validate_ngspice_models, NgspiceInputLatch, NgspiceModelSpec,
};
use labwired_core::cosim::{
    build_cosim_adapter_with_base, AnalogCosimAdapter, CosimSession, CosimSignalValue,
    CosimSignals, CosimStep, RoutingError,
};
use labwired_core::cpu::CortexM;
use labwired_core::flex::FlexRuntime;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, Machine};
use std::path::{Path, PathBuf};
use std::process::Command;

const VDD: f64 = 3.3;
const R_FIXED: f64 = 10_000.0;
const C_ADC: f64 = 100e-9;

/// LDR law in `examples/cosim-ngspice-ldr/ldr.cir`: R10 = 10 kOhm, gamma 0.7.
fn ldr_ohms(lux: f64) -> f64 {
    10_000.0 * (lux.max(0.01) / 10.0).powf(-0.7)
}

fn ldr_volts(lux: f64) -> f64 {
    let r = ldr_ohms(lux);
    VDD * r / (R_FIXED + r)
}

fn ldr_tau_s(lux: f64) -> f64 {
    let r = ldr_ohms(lux);
    R_FIXED * r / (R_FIXED + r) * C_ADC
}

fn ngspice_ready() -> bool {
    let probe = Command::new("python3")
        .arg("-c")
        .arg(
            "import ctypes, ctypes.util, os;\
             ctypes.CDLL(os.environ.get('LABWIRED_NGSPICE_LIB') or \
             ctypes.util.find_library('ngspice') or 'libngspice.so.0')",
        )
        .output();
    let ready = matches!(probe, Ok(output) if output.status.success());
    if !ready {
        assert!(
            std::env::var_os("LABWIRED_REQUIRE_NGSPICE").is_none(),
            "LABWIRED_REQUIRE_NGSPICE is set, but python3 cannot load libngspice"
        );
        eprintln!(
            "SKIP: python3 cannot load libngspice (apt install libngspice0, or set \
             LABWIRED_NGSPICE_LIB); the native ngspice half of this test did not run"
        );
    }
    ready
}

/// A private directory for netlists written by one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("labwired-ngspice-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn model(yaml: &str) -> CosimModelConfig {
    serde_yaml::from_str(yaml).expect("model yaml")
}

const RC_NETLIST: &str =
    "* RC low-pass\nVgpio in 0 dc 0\nR1 in out 10k\nC1 out 0 100n\n.tran 1u 10 uic\n.end\n";

/// An RC model whose `model:` is `netlist`, resolved from the test's base dir.
fn rc_model(netlist: &str, step_ns: u64) -> CosimModelConfig {
    model(&format!(
        r#"
id: rc
adapter: ngspice
model: {}
step_ns: {step_ns}
inputs: {{ gpio: board.gpio.pa5 }}
outputs: {{ v_out: board.analog.pa0_volts }}
config:
  input_sources: {{ gpio: {{ element: Vgpio, scale: 3.3, offset: 0 }} }}
  output_nodes: {{ v_out: {{ node: out, scale: 1, offset: 0 }} }}
"#,
        netlist
    ))
}

fn step(time_ns: u64, dt_ns: u64, inputs: &[(&str, CosimSignalValue)]) -> CosimStep {
    CosimStep {
        time_ns,
        dt_ns,
        inputs: inputs
            .iter()
            .map(|(name, value)| ((*name).to_string(), value.clone()))
            .collect::<CosimSignals>(),
    }
}

fn volts(outputs: &CosimSignals, name: &str) -> f64 {
    match outputs.get(name) {
        Some(CosimSignalValue::F64(v)) => *v,
        other => panic!("output {name}: expected volts, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Manifest checks: no solver needed.

#[test]
fn the_committed_ldr_manifests_pass_the_ngspice_contract() {
    let dir = root("examples/cosim-ngspice-ldr");
    let system: SystemManifest =
        serde_yaml::from_str(&std::fs::read_to_string(dir.join("system.yaml")).unwrap()).unwrap();
    assert_eq!(
        validate_ngspice_models(&system.cosim_models, &dir),
        Vec::<String>::new()
    );
    let spec = NgspiceModelSpec::from_config(&system.cosim_models[0], &dir).unwrap();
    assert!(
        spec.inputs["supply"].logic,
        "a board.gpio route is a logic input"
    );
    assert!(!spec.inputs["lux"].logic);
    assert_eq!(
        spec.inputs["lux"].default_value,
        Some(CosimSignalValue::F64(100.0))
    );
    assert_eq!(
        spec.input_ranges["lux"],
        labwired_core::cosim::ngspice::NgspiceInputRange {
            min: Some(1.0),
            max: Some(100.0),
            unit: Some("lx".to_string()),
        }
    );
    let host: serde_json::Value = serde_json::from_str(&spec.host_json()).unwrap();
    assert_eq!(host["inputs"]["supply"]["element"], "Vsup");
    assert_eq!(host["outputs"]["v_out"]["node"], "out");
    // The host gets the validated text, not a path to read again.
    assert_eq!(
        host["netlist"].as_str().unwrap(),
        std::fs::read_to_string(dir.join("ldr.cir")).unwrap()
    );
    assert!(host.get("netlist_path").is_none());
    assert!(spec.netlist_path.ends_with("ldr.cir"));

    let flex = labwired_config::FlexManifest::from_yaml(
        &std::fs::read_to_string(dir.join("flex-ldr.yaml")).unwrap(),
    )
    .expect("Flex accepts adapter: ngspice with a netlist path");
    assert_eq!(
        validate_ngspice_models(&flex.cosim_models, &dir),
        Vec::<String>::new()
    );
}

#[test]
fn the_contract_rejects_what_the_browser_runtime_rejects() {
    let dir = scratch("contract");
    std::fs::write(dir.join("rc.cir"), RC_NETLIST).unwrap();
    for (netlist, needle) in [
        ("* no tran\nV1 a 0 1\n.end\n", ".tran and .end"),
        ("* no end\nV1 a 0 1\n.tran 1u 1m\n", ".tran and .end"),
        (
            "* ctl\nV1 a 0 1\n.tran 1u 1m\n.control\nrun\n.endc\n.end\n",
            "control block",
        ),
    ] {
        std::fs::write(dir.join("bad.cir"), netlist).unwrap();
        let err = NgspiceModelSpec::from_config(&rc_model("./bad.cir", 1000), &dir).unwrap_err();
        assert!(err.contains(needle), "{err}");
    }
    let cases: [(&str, &str, &str); 7] = [
        ("input_sources: {}", "output_nodes: { v_out: { node: out, scale: 1, offset: 0 } }", "input_sources.gpio is required"),
        ("input_sources: { gpio: { element: 9bad, scale: 1, offset: 0 } }", "output_nodes: { v_out: { node: out, scale: 1, offset: 0 } }", "element is invalid"),
        ("input_sources: { gpio: { element: Vgpio, scale: .nan, offset: 0 } }", "output_nodes: { v_out: { node: out, scale: 1, offset: 0 } }", "finite numbers"),
        ("input_sources: { gpio: { element: Vgpio, scale: 1, offset: 0, default_value: 2.5 } }", "output_nodes: { v_out: { node: out, scale: 1, offset: 0 } }", "default_value must match"),
        ("input_sources: { gpio: { element: Vgpio, scale: 1, offset: 0 }, extra: { element: V2, scale: 1, offset: 0 } }", "output_nodes: { v_out: { node: out, scale: 1, offset: 0 } }", "input_sources.extra has no matching"),
        ("input_sources: { gpio: { element: Vgpio, scale: 1, offset: 0 } }", "output_nodes: {}", "output_nodes.v_out is required"),
        ("input_sources: { gpio: { element: Vgpio, scale: 1, offset: 0 } }", "output_nodes: { v_out: { node: out, scale: 1, offset: 0 } }\n  trace_volts: [probe]", "output_nodes.probe is required"),
    ];
    for (inputs, outputs, needle) in cases {
        let config = model(&format!(
            "id: rc\nadapter: ngspice\nmodel: ./rc.cir\nstep_ns: 1000\ninputs: {{ gpio: board.gpio.pa5 }}\noutputs: {{ v_out: board.analog.pa0_volts }}\nconfig:\n  {inputs}\n  {outputs}\n",
        ));
        let err = NgspiceModelSpec::from_config(&config, &dir).unwrap_err();
        assert!(err.contains(needle), "expected '{needle}' in: {err}");
        assert!(err.contains("ngspice model 'rc'"), "{err}");
    }
    let gpio_in = model(
        "id: rc\nadapter: ngspice\nmodel: ./rc.cir\noutputs: { high: board.gpio_in.pc13 }\nconfig:\n  output_nodes: { high: { node: out, scale: 1, offset: 0 } }\n",
    );
    let err = NgspiceModelSpec::from_config(&gpio_in, &dir).unwrap_err();
    assert!(err.contains("threshold is required"), "{err}");
    // The factory reports the same error before it starts any process.
    let Err(err) = build_cosim_adapter_with_base(&gpio_in, &dir) else {
        panic!("the factory must refuse a gpio_in output without a threshold");
    };
    assert!(err.to_string().contains("threshold is required"), "{err}");
}

/// `config.input_ranges` declares the numeric domain a deck was extracted for.
/// The shared [`NgspiceInputLatch`] refuses a boundary outside it on both
/// hosts, which is what lets an in-core bounded element (a photoresistor's
/// `min_lux..max_lux`, a thermistor's temperature range) move to an ngspice
/// behavioural deck without losing its explicit stop.
#[test]
fn input_ranges_are_validated_before_a_solver_starts() {
    let dir = scratch("ranges");
    std::fs::write(dir.join("rc.cir"), RC_NETLIST).unwrap();
    let ranging = |inputs: &str, ranges: &str| {
        model(&format!(
            "id: rc\nadapter: ngspice\nmodel: ./rc.cir\nstep_ns: 1000\ninputs: {inputs}\noutputs: {{ v_out: board.analog.pa0_volts }}\nconfig:\n  input_sources: {{ lux: {{ element: Vlux, scale: 1, offset: 0 }} }}\n  output_nodes: {{ v_out: {{ node: out, scale: 1, offset: 0 }} }}\n  input_ranges: {ranges}\n"
        ))
    };
    let lux = "{ lux: bench.lux }";

    // Positive control: two bounds and a unit.
    let spec = NgspiceModelSpec::from_config(
        &ranging(lux, "{ lux: { min: 1, max: 100, unit: lx } }"),
        &dir,
    )
    .unwrap();
    assert_eq!(spec.input_ranges["lux"].min, Some(1.0));
    assert_eq!(spec.input_ranges["lux"].max, Some(100.0));
    assert_eq!(spec.input_ranges["lux"].unit.as_deref(), Some("lx"));
    // The unit is trimmed for the message; surrounding blanks are not data.
    let padded =
        NgspiceModelSpec::from_config(&ranging(lux, "{ lux: { min: 1, unit: \" lx \" } }"), &dir)
            .unwrap();
    assert_eq!(padded.input_ranges["lux"].unit.as_deref(), Some("lx"));
    // One bound is allowed; no bound is not.
    for ranges in ["{ lux: { min: 1 } }", "{ lux: { max: 100 } }"] {
        NgspiceModelSpec::from_config(&ranging(lux, ranges), &dir).unwrap();
    }

    for (inputs, ranges, needle) in [
        (lux, "{ lux: { minimum: 1 } }", "is not a range key"),
        (
            lux,
            "{ nothing: { min: 1 } }",
            "has no matching declared route",
        ),
        (lux, "{ lux: { min: 10, max: 1 } }", "greater than max"),
        (lux, "{ lux: {} }", "needs min, max or both"),
        (lux, "{ lux: { min: .nan } }", "must be a finite number"),
        (lux, "{ lux: { min: .inf } }", "must be a finite number"),
        (lux, "{ lux: [1] }", "must be a mapping"),
        (
            lux,
            "{ lux: { min: 1, unit: \"\" } }",
            "1-16 visible characters",
        ),
        (
            lux,
            "{ lux: { min: 1, unit: \"kilolux-per-square-metre\" } }",
            "1-16 visible characters",
        ),
    ] {
        let err = NgspiceModelSpec::from_config(&ranging(inputs, ranges), &dir).unwrap_err();
        assert!(err.contains(needle), "expected '{needle}' in: {err}");
        assert!(err.contains("ngspice model 'rc'"), "{err}");
    }
    // A logic route has no numeric domain to bound.
    let gpio = model(
        "id: rc\nadapter: ngspice\nmodel: ./rc.cir\nstep_ns: 1000\ninputs: { gpio: board.gpio.pa5 }\noutputs: { v_out: board.analog.pa0_volts }\nconfig:\n  input_sources: { gpio: { element: Vgpio, scale: 3.3, offset: 0 } }\n  output_nodes: { v_out: { node: out, scale: 1, offset: 0 } }\n  input_ranges: { gpio: { min: 0, max: 1 } }\n",
    );
    let err = NgspiceModelSpec::from_config(&gpio, &dir).unwrap_err();
    assert!(err.contains("applies to a logic input"), "{err}");
    // A non-mapping entry list is refused by the same shared reader.
    let err = NgspiceModelSpec::from_config(&ranging(lux, "[]"), &dir).unwrap_err();
    assert!(
        err.contains("config.input_ranges must be a mapping"),
        "{err}"
    );

    // `validate_ngspice_models` reports it too, before any host starts.
    let issues = validate_ngspice_models(
        std::slice::from_ref(&ranging(lux, "{ lux: { min: 10, max: 1 } }")),
        &dir,
    );
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert!(issues[0].contains("greater than max"), "{}", issues[0]);
}

/// The latch refuses an out-of-domain value before the boundary reaches either
/// host, so the two engines agree on *whether* to run, not only on the voltage.
#[test]
fn the_latch_refuses_an_input_outside_its_declared_range() {
    let dir = scratch("range-latch");
    std::fs::write(dir.join("rc.cir"), RC_NETLIST).unwrap();
    let config = model(
        "id: ldr\nadapter: ngspice\nmodel: ./rc.cir\nstep_ns: 1000\ninputs: { lux: bench.lux }\noutputs: { v_out: board.analog.pa0_volts }\nconfig:\n  input_sources: { lux: { element: Vlux, scale: 1, offset: 0 } }\n  output_nodes: { v_out: { node: out, scale: 1, offset: 0 } }\n  input_ranges: { lux: { min: 1, max: 100, unit: lx } }\n",
    );
    let mut latch = NgspiceInputLatch::new(&config).unwrap();
    for value in [1.0, 50.0, 100.0] {
        latch
            .sample(step(1000, 1000, &[("lux", CosimSignalValue::F64(value))]))
            .unwrap_or_else(|err| panic!("{value} lx is inside 1..100: {err}"));
    }
    for value in [0.0, 0.9, 100.1, 150.0] {
        let err = latch
            .sample(step(2000, 1000, &[("lux", CosimSignalValue::F64(value))]))
            .expect_err("outside 1..100 lx");
        assert_eq!(
            err,
            format!(
                "ngspice model 'ldr' input 'lux' is {value}, outside the declared 1..100 lx range"
            )
        );
    }
    // A value absent from the boundary is left to the host's default_value.
    latch.sample(step(3000, 1000, &[])).unwrap();
    // A boolean or text value is not a declared numeric domain.
    let err = latch
        .sample(step(4000, 1000, &[("lux", CosimSignalValue::Bool(true))]))
        .expect_err("a bool is not a number");
    assert!(err.contains("is not a number"), "{err}");
    // One-sided ranges render as a comparison in the message.
    let mut upper = config.clone();
    upper.config.insert(
        "input_ranges".into(),
        serde_yaml::from_str("{ lux: { max: 100 } }").unwrap(),
    );
    let err = NgspiceInputLatch::new(&upper)
        .unwrap()
        .sample(step(1000, 1000, &[("lux", CosimSignalValue::F64(101.0))]))
        .expect_err("above the upper bound");
    assert!(err.ends_with("outside the declared <= 100 range"), "{err}");
    // `previous_boundary` scores the *held* value: a bad write is refused at
    // the boundary where it would first drive the circuit, not when written.
    let mut previous = config.clone();
    previous
        .config
        .insert("input_sampling".into(), "previous_boundary".into());
    let mut latch = NgspiceInputLatch::new(&previous).unwrap();
    latch
        .sample(step(0, 0, &[("lux", CosimSignalValue::F64(10.0))]))
        .expect("prime with an in-range value");
    latch
        .sample(step(1000, 1000, &[("lux", CosimSignalValue::F64(150.0))]))
        .expect("the held 10 lx still drives this interval");
    let err = latch
        .sample(step(2000, 1000, &[("lux", CosimSignalValue::F64(10.0))]))
        .expect_err("the held 150 lx drives the next interval");
    assert!(err.contains("is 150, outside the declared"), "{err}");
}

/// An included file could carry a `.control` block with a `shell` command or
/// read any file. `.include`, `.inc` and `.lib` are refused (any case, after
/// leading blanks) before a process starts: the shell command never runs.
#[test]
fn included_netlists_are_refused_before_ngspice_runs() {
    let dir = scratch("include");
    let marker = dir.join("shell-ran");
    std::fs::write(
        dir.join("evil.cir"),
        format!(
            "* evil\n.control\nshell touch {}\n.endc\n",
            marker.display()
        ),
    )
    .unwrap();
    for directive in [
        ".include ./evil.cir",
        ".inc ./evil.cir",
        ".lib ./evil.cir tt",
        "  .INCLUDE ./evil.cir",
        "\t.Lib ./evil.cir",
    ] {
        std::fs::write(
            dir.join("rc.cir"),
            RC_NETLIST.replace(".tran", &format!("{directive}\n.tran")),
        )
        .unwrap();
        let name = directive.split_whitespace().next().unwrap();
        let err = NgspiceModelSpec::from_config(&rc_model("./rc.cir", 1000), &dir).unwrap_err();
        assert!(
            err.contains(&format!("{} directive", name.to_ascii_lowercase())),
            "{directive}: {err}"
        );
        let Err(err) = build_cosim_adapter_with_base(&rc_model("./rc.cir", 1000), &dir) else {
            panic!("{directive} must be refused by the factory");
        };
        assert!(err.to_string().contains("directive"), "{err}");
    }
    // Positive control: the same netlist without the directive passes.
    std::fs::write(dir.join("rc.cir"), RC_NETLIST).unwrap();
    NgspiceModelSpec::from_config(&rc_model("./rc.cir", 1000), &dir).unwrap();
    assert!(!marker.exists(), "an included control block ran");
}

/// `model:` names a file inside the manifest directory. An absolute path, a
/// `..` path and a symlink that leaves the directory are refused; a file in a
/// subdirectory and a symlink that stays inside are accepted.
#[test]
fn model_paths_are_confined_to_the_manifest_directory() {
    let root_dir = scratch("confine");
    let base = root_dir.join("manifest");
    let outside = root_dir.join("outside");
    std::fs::create_dir_all(base.join("nets")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("rc.cir"), RC_NETLIST).unwrap();
    std::fs::write(base.join("nets/rc.cir"), RC_NETLIST).unwrap();
    let outside_abs = outside.join("rc.cir").canonicalize().unwrap();

    let refused = |model_path: &str, needle: &str| {
        let err = NgspiceModelSpec::from_config(&rc_model(model_path, 1000), &base).unwrap_err();
        assert!(err.contains(needle), "{model_path}: {err}");
        assert!(
            build_cosim_adapter_with_base(&rc_model(model_path, 1000), &base).is_err(),
            "{model_path} must be refused by the factory"
        );
        let issues = validate_ngspice_models(&[rc_model(model_path, 1000)], &base);
        assert_eq!(issues.len(), 1, "{model_path}: {issues:?}");
    };
    refused("../outside/rc.cir", "outside the manifest directory");
    refused(
        "./nets/../../outside/rc.cir",
        "outside the manifest directory",
    );
    refused(
        &outside_abs.display().to_string(),
        "must be relative to the manifest directory",
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside_abs, base.join("escape.cir")).unwrap();
        refused("./escape.cir", "outside the manifest directory");
        std::os::unix::fs::symlink(base.join("nets/rc.cir"), base.join("inside.cir")).unwrap();
        let spec = NgspiceModelSpec::from_config(&rc_model("./inside.cir", 1000), &base).unwrap();
        assert!(spec.netlist_path.ends_with("nets/rc.cir"));
    }
    // Positive controls: a subdirectory, and `..` that stays inside.
    for inside in ["./nets/rc.cir", "nets/../nets/rc.cir"] {
        let spec = NgspiceModelSpec::from_config(&rc_model(inside, 1000), &base).unwrap();
        assert_eq!(spec.netlist, RC_NETLIST, "{inside}");
    }
}

/// The native host runs `python3 -I`: a `json.py` or `ctypes/` package in the
/// working directory does not load. The exact command the adapter spawns runs
/// in a poisoned directory (via `current_dir`, so the test process keeps its
/// own); the same command without `-I` imports the poison (negative control).
#[test]
fn native_host_ignores_modules_in_its_working_directory() {
    use labwired_core::cosim::ngspice::{native_host_args, NATIVE_HOST_PROGRAM};
    use std::io::Write;
    use std::process::Stdio;
    if !ngspice_ready() {
        return;
    }
    let dir = scratch("poison");
    std::fs::write(dir.join("rc.cir"), RC_NETLIST).unwrap();
    let poison = dir.join("cwd");
    std::fs::create_dir_all(poison.join("ctypes")).unwrap();
    let marker = dir.join("poison-ran");
    let payload = format!(
        "open({:?}, 'w').write('ran')\nraise SystemExit('poisoned module imported')\n",
        marker.display().to_string()
    );
    std::fs::write(poison.join("json.py"), &payload).unwrap();
    std::fs::write(poison.join("ctypes/__init__.py"), &payload).unwrap();

    let spec = NgspiceModelSpec::from_config(&rc_model("./rc.cir", 1000), &dir)
        .unwrap()
        .host_json();
    let run = |args: &[&str]| {
        let mut child = Command::new(NATIVE_HOST_PROGRAM)
            .args(args)
            .current_dir(&poison)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"{\"time_ns\": 1000, \"dt_ns\": 1000, \"inputs\": {\"gpio\": true}}\n")
            .unwrap();
        child.wait_with_output().unwrap()
    };

    let isolated = run(&native_host_args(&spec));
    let stdout = String::from_utf8_lossy(&isolated.stdout);
    assert!(
        isolated.status.success() && stdout.contains("\"outputs\""),
        "isolated host: {stdout} / {}",
        String::from_utf8_lossy(&isolated.stderr)
    );
    assert!(!marker.exists(), "the isolated host imported the poison");

    let open = run(&native_host_args(&spec)[1..]);
    assert!(!open.status.success(), "without -I the poison must load");
    assert!(marker.exists(), "without -I the poison must load");
}

/// A machine session sends no `(0, 0)` initialization step, so
/// `previous_boundary` is refused there before any adapter is built; Flex
/// primes its models and keeps it (see `flex_script_runs`).
#[test]
fn machine_sessions_refuse_previous_boundary_sampling() {
    use labwired_core::cosim::ngspice::machine_sampling_refusal;
    let dir = scratch("sampling");
    std::fs::write(dir.join("rc.cir"), RC_NETLIST).unwrap();
    let mut previous = rc_model("./rc.cir", 1000);
    previous
        .config
        .insert("input_sampling".into(), "previous_boundary".into());
    let mut current = previous.clone();
    current
        .config
        .insert("input_sampling".into(), "current_boundary".into());

    let refusal = machine_sampling_refusal(&previous).expect("previous_boundary is refused");
    assert!(refusal.contains("ngspice model 'rc'"), "{refusal}");
    assert_eq!(machine_sampling_refusal(&current), None);
    assert_eq!(machine_sampling_refusal(&rc_model("./rc.cir", 1000)), None);
    // The spec itself stays valid: Flex uses it.
    NgspiceModelSpec::from_config(&previous, &dir).unwrap();

    let issues = validate_ngspice_models(std::slice::from_ref(&previous), &dir);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert!(issues[0].contains("previous_boundary"), "{}", issues[0]);
    assert_eq!(
        validate_ngspice_models(std::slice::from_ref(&current), &dir),
        Vec::<String>::new()
    );

    let bus = SystemBus::new();
    let Err(err) = CosimSession::new_with_factory(std::slice::from_ref(&previous), &bus, |_| {
        panic!("no adapter may be built for a refused manifest")
    }) else {
        panic!("a machine session must refuse previous_boundary");
    };
    assert!(err.to_string().contains("previous_boundary"), "{err}");
    // Negative control: current_boundary reaches the factory.
    let mut built = 0;
    let _ = CosimSession::new_with_factory(std::slice::from_ref(&current), &bus, |_| {
        built += 1;
        Err(labwired_core::SimulationError::Other(
            "factory reached".into(),
        ))
    });
    assert_eq!(built, 1);
}

/// Migration gap made executable: the in-core engine cannot run the LDR deck
/// (behavioral `B` source), so it keeps needing ngspice.
#[test]
fn the_in_core_solver_refuses_the_ldr_netlist() {
    let netlist = std::fs::read_to_string(root("examples/cosim-ngspice-ldr/ldr.cir")).unwrap();
    let mut config = model(
        "id: ldr\nadapter: analog\nstep_ns: 100000\ninputs: { supply: board.gpio.pa5 }\noutputs: { v_out: board.analog.pa0_volts }\nconfig:\n  probes: { v_out: \"v(out)\" }\n  sources: { supply: Vsup }\n",
    );
    config
        .config
        .insert("netlist_text".into(), serde_yaml::Value::String(netlist));
    let Err(err) = AnalogCosimAdapter::from_manifest_config(&config, Path::new(".")) else {
        panic!("the in-core engine has no B source");
    };
    eprintln!("in-core refusal: {err}");
}

// ---------------------------------------------------------------------------
// Real ngspice runs.

/// Exact boundaries: each step ends at the requested time (the host checks it
/// to 32 ulp and fails otherwise), so an RC sampled at an odd period follows
/// the analytic curve. Without a solver breakpoint ngspice-42 ran a 200 us
/// stop to the end of `.tran`; that would show as a settled voltage here.
#[test]
fn native_steps_land_on_odd_boundaries_and_follow_the_rc_curve() {
    if !ngspice_ready() {
        return;
    }
    let dir = scratch("odd");
    let netlist = dir.join("rc.cir");
    std::fs::write(&netlist, RC_NETLIST).unwrap();
    let rc = "./rc.cir";
    const STEP_NS: u64 = 123_457;
    let mut adapter = build_cosim_adapter_with_base(&rc_model(rc, STEP_NS), &dir).unwrap();
    let tau = 10e3 * 100e-9;
    let mut worst = 0.0f64;
    for k in 1..=40u64 {
        let t_ns = k * STEP_NS;
        let out = adapter
            .step(step(
                t_ns,
                STEP_NS,
                &[("gpio", CosimSignalValue::Bool(true))],
            ))
            .unwrap()
            .outputs;
        let expected = VDD * (1.0 - (-(t_ns as f64) * 1e-9 / tau).exp());
        let error = (volts(&out, "v_out") - expected).abs() / VDD;
        worst = worst.max(error);
        assert!(
            error < 2e-3,
            "step {k} t={t_ns} ns: {} V vs {expected} V",
            volts(&out, "v_out")
        );
    }
    eprintln!(
        "native ngspice vs analytic RC, 40 steps of {STEP_NS} ns: worst {:.4} % FS",
        worst * 100.0
    );
}

/// Parity with the in-core engine on the deck both can run: the same RC, the
/// same 100 us boundaries, the same firmware level, within 1 % of full scale.
#[test]
fn native_ngspice_matches_the_in_core_solver_on_a_shared_rc() {
    if !ngspice_ready() {
        return;
    }
    let dir = scratch("parity");
    let netlist = dir.join("rc.cir");
    std::fs::write(&netlist, RC_NETLIST).unwrap();
    let rc = "./rc.cir";
    let mut native = build_cosim_adapter_with_base(&rc_model(rc, 100_000), &dir).unwrap();
    let mut in_core = build_cosim_adapter_with_base(
        &model(&format!(
            "id: rc\nadapter: analog\nstep_ns: 100000\ninputs: {{ gpio: board.gpio.pa5 }}\noutputs: {{ v_out: board.analog.pa0_volts }}\nconfig:\n  netlist: {}\n  vdd: 3.3\n  substeps: 10\n  probes: {{ v_out: \"v(out)\" }}\n  sources: {{ gpio: Vgpio }}\n",
            netlist.display()
        )),
        &dir,
    )
    .unwrap();
    let mut worst = 0.0f64;
    for k in 1..=20u64 {
        let s = step(
            k * 100_000,
            100_000,
            &[("gpio", CosimSignalValue::Bool(true))],
        );
        let n = volts(&native.step(s.clone()).unwrap().outputs, "v_out");
        let a = volts(&in_core.step(s).unwrap().outputs, "v_out");
        let error = (n - a).abs() / VDD;
        worst = worst.max(error);
        assert!(error < 0.01, "step {k}: ngspice {n} V vs in-core {a} V");
    }
    eprintln!(
        "native ngspice vs in-core over 20 x 100 us: worst {:.4} % FS",
        worst * 100.0
    );
}

/// Parity on the part the in-core engine had to itself: a bounded
/// photoresistor curve. `ldr.cir` writes the GL5528 law as a behavioural
/// current (`B` source), which the in-core engine cannot parse; the in-core
/// engine offers the same curve as a `photoresistors` entry. Both are the same
/// divider, the same 15 kOhm reference at 10 lx, the same 0.6 slope and the
/// same 1..100 lx domain, so the two engines must agree on the voltage *and*
/// on refusing a value outside that domain. This is the coverage evidence that
/// lets a bounded-curve Flex fixture move to ngspice.
#[test]
fn a_photoresistor_deck_matches_the_in_core_curve_within_its_range() {
    if !ngspice_ready() {
        return;
    }
    let dir = scratch("photoresistor");
    std::fs::write(
        dir.join("ldr.cir"),
        "* GL5528 divider, 15 kOhm at 10 lx, gamma 0.6, per the Flex fixture.\n\
         V3V3 3v3 0 dc 3.3\n\
         Vlux lux 0 dc 10\n\
         Bldr 3v3 sns_an1 I = V(3v3,sns_an1) / (15k * pow(max(V(lux), 0.01) / 10, -0.6))\n\
         R6 sns_an1 0 10k\n\
         C7 sns_an1 0 100n\n\
         .tran 10u 1 uic\n\
         .end\n",
    )
    .unwrap();
    let spiced = model(
        "id: ldr\nadapter: ngspice\nmodel: ./ldr.cir\nstep_ns: 50000\ninputs: { lux: bench.lux }\noutputs: { voltage: ldr.out_volts }\nconfig:\n  input_sources: { lux: { element: Vlux, scale: 1, offset: 0 } }\n  output_nodes: { voltage: { node: sns_an1, scale: 1, offset: 0 } }\n  input_ranges: { lux: { min: 1, max: 100, unit: lx } }\n  trace_volts: [voltage]\n",
    );
    let in_core = model(
        "id: ldr\nadapter: analog\nstep_ns: 50000\ninputs: { lux: bench.lux }\noutputs: { voltage: ldr.out_volts }\nconfig:\n  netlist_text: |\n    V3V3 3v3 0 dc 3.3\n    RU4 3v3 sns_an1 15k\n    R6 sns_an1 0 10k\n    C7 sns_an1 0 100n\n  vdd: 3.3\n  probes: { voltage: \"v(sns_an1)\" }\n  photoresistors:\n    lux: { resistor: RU4, r_ref_ohms: 15000, reference_lux: 10, gamma: 0.6, min_lux: 1, max_lux: 100 }\n",
    );
    assert_eq!(
        validate_ngspice_models(std::slice::from_ref(&spiced), &dir),
        Vec::<String>::new()
    );
    let mut native = build_cosim_adapter_with_base(&spiced, &dir).unwrap();
    let mut analog = build_cosim_adapter_with_base(&in_core, &dir).unwrap();
    // 10 ms is more than ten time constants at both ends of the range, so both
    // engines report the divider's settled value rather than an integrator's
    // residue.
    const STEP_NS: u64 = 50_000;
    const SETTLE: u64 = 200;
    // One monotonic timeline for both engines: the host refuses a repeated or
    // earlier boundary, so each light level gets its own settle window.
    let mut t_ns = 0;
    for lux in [1.0f64, 10.0, 100.0] {
        let expected = VDD * R_FIXED / (R_FIXED + 15_000.0 * (lux / 10.0).powf(-0.6));
        let mut spiced_v = f64::NAN;
        let mut analog_v = f64::NAN;
        for _ in 0..SETTLE {
            t_ns += STEP_NS;
            let s = step(t_ns, STEP_NS, &[("lux", CosimSignalValue::F64(lux))]);
            spiced_v = volts(&native.step(s.clone()).unwrap().outputs, "voltage");
            analog_v = volts(&analog.step(s).unwrap().outputs, "voltage");
        }
        assert!(
            (spiced_v - expected).abs() < 2e-3,
            "{lux} lx: ngspice {spiced_v} V vs {expected} V"
        );
        assert!(
            (analog_v - expected).abs() < 2e-3,
            "{lux} lx: in-core {analog_v} V vs {expected} V"
        );
        assert!(
            (spiced_v - analog_v).abs() < 2e-3,
            "{lux} lx: ngspice {spiced_v} V vs in-core {analog_v} V"
        );
        eprintln!(
            "{lux} lx: ngspice {spiced_v:.6} V, in-core {analog_v:.6} V, analytic {expected:.6} V"
        );
    }
    // Outside the declared domain both stop, with their own wording.
    let err = analog
        .step(step(
            99_000_000,
            STEP_NS,
            &[("lux", CosimSignalValue::F64(150.0))],
        ))
        .expect_err("the in-core curve is bounded");
    assert!(
        err.to_string()
            .contains("photoresistor illuminance must be within 1..100 lx"),
        "{err}"
    );
    let mut spiced = build_cosim_adapter_with_base(&spiced, &dir).unwrap();
    let err = spiced
        .step(step(
            STEP_NS,
            STEP_NS,
            &[("lux", CosimSignalValue::F64(150.0))],
        ))
        .expect_err("the declared ngspice domain is bounded");
    assert!(
        err.to_string()
            .contains("input 'lux' is 150, outside the declared 1..100 lx range"),
        "{err}"
    );
}

/// Continuity across the runner's `(0, 0)` priming step: the primed value is
/// where the `uic` transient starts (C uncharged), and the next boundaries
/// follow the RC curve from it. A DC operating point here would report
/// 3.3 V and then drop to 0.31 V at the first boundary.
#[test]
fn native_priming_step_is_the_start_of_the_transient() {
    if !ngspice_ready() {
        return;
    }
    let dir = scratch("priming");
    let netlist = dir.join("rc.cir");
    std::fs::write(&netlist, RC_NETLIST).unwrap();
    let rc = "./rc.cir";
    let mut adapter = build_cosim_adapter_with_base(&rc_model(rc, 100_000), &dir).unwrap();
    let high = [("gpio", CosimSignalValue::Bool(true))];
    let primed = volts(&adapter.step(step(0, 0, &high)).unwrap().outputs, "v_out");
    assert!(
        primed.abs() < 1e-3,
        "primed {primed} V, expected the uic start (0 V)"
    );
    let tau = 10e3 * 100e-9;
    for k in 1..=3u64 {
        let t_ns = k * 100_000;
        let v = volts(
            &adapter.step(step(t_ns, 100_000, &high)).unwrap().outputs,
            "v_out",
        );
        let expected = VDD * (1.0 - (-(t_ns as f64) * 1e-9 / tau).exp());
        assert!(
            (v - expected).abs() < 2e-3 * VDD,
            "t={t_ns} ns after priming: {v} V vs {expected} V"
        );
    }
}

/// Snapshot/restore is refused, not approximated: any `adapter: ngspice`
/// model makes the refusal fire (the browser `take/apply_runtime_snapshot`
/// and native `--capture-app-entry` / `--resume-snapshot` both read it); a
/// session of in-core models is not refused (negative control).
#[test]
fn runtime_snapshots_are_refused_with_an_ngspice_model() {
    let dir = root("examples/cosim-ngspice-ldr");
    let system: SystemManifest =
        serde_yaml::from_str(&std::fs::read_to_string(dir.join("system.yaml")).unwrap()).unwrap();
    let reason = runtime_snapshot_refusal(&system.cosim_models).expect("ngspice is refused");
    assert!(reason.contains("ngspice"), "{reason}");
    let in_core =
        model("id: rc\nadapter: analog\nstep_ns: 100000\ninputs: {}\noutputs: {}\nconfig: {}\n");
    assert_eq!(
        runtime_snapshot_refusal(std::slice::from_ref(&in_core)),
        None
    );
    assert_eq!(runtime_snapshot_refusal(&[]), None);
}

#[test]
fn native_solver_errors_name_the_model_and_stop_the_adapter() {
    if !ngspice_ready() {
        return;
    }
    let dir = scratch("errors");
    let netlist = dir.join("rc.cir");
    std::fs::write(&netlist, RC_NETLIST).unwrap();
    let rc = "./rc.cir";

    // An element the netlist does not contain: ngspice refuses the alter.
    let mut config = rc_model(rc, 1000);
    let mut bad = config.clone();
    bad.config.insert(
        "input_sources".into(),
        serde_yaml::from_str("{ gpio: { element: Vnope, scale: 3.3, offset: 0 } }").unwrap(),
    );
    let mut adapter = build_cosim_adapter_with_base(&bad, &dir).unwrap();
    let err = adapter
        .step(step(1000, 1000, &[("gpio", CosimSignalValue::Bool(true))]))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("ngspice model 'rc'") && err.contains("vnope"),
        "{err}"
    );
    let again = adapter
        .step(step(2000, 1000, &[("gpio", CosimSignalValue::Bool(true))]))
        .unwrap_err()
        .to_string();
    assert!(again.contains("terminally failed"), "{again}");

    // A node the netlist does not contain.
    config.config.insert(
        "output_nodes".into(),
        serde_yaml::from_str("{ v_out: { node: nowhere, scale: 1, offset: 0 } }").unwrap(),
    );
    let mut adapter = build_cosim_adapter_with_base(&config, &dir).unwrap();
    let err = adapter
        .step(step(1000, 1000, &[("gpio", CosimSignalValue::Bool(true))]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("nowhere"), "{err}");

    // Time must advance: a repeated boundary is a host error, not a no-op.
    let mut adapter = build_cosim_adapter_with_base(&rc_model(rc, 1000), &dir).unwrap();
    adapter
        .step(step(1000, 1000, &[("gpio", CosimSignalValue::Bool(true))]))
        .unwrap();
    let err = adapter
        .step(step(1000, 1000, &[("gpio", CosimSignalValue::Bool(true))]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("monotonically"), "{err}");

    // A GPIO route carries a logic level, as in the browser runtime.
    let mut adapter = build_cosim_adapter_with_base(&rc_model(rc, 1000), &dir).unwrap();
    let err = adapter
        .step(step(1000, 1000, &[("gpio", CosimSignalValue::F64(1.0))]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("must be boolean"), "{err}");
}

fn flex_ldr_dir() -> PathBuf {
    root("examples/cosim-ngspice-ldr")
}

/// The committed manifest as is: `model: ./ldr.cir`, resolved from its
/// directory by [`FlexRuntime::from_yaml_with_base`].
fn flex_ldr_yaml() -> String {
    std::fs::read_to_string(flex_ldr_dir().join("flex-ldr.yaml")).unwrap()
}

/// Flex: host-stepped firmware ports drive an ngspice LDR divider.
/// Positive controls: the bright and dark settled voltages and the RC rise
/// after enable, all from the circuit equations. Negative controls: with the
/// divider unpowered the light level changes nothing, and a broken source
/// mapping fails construction instead of running.
#[test]
fn flex_ldr_divider_runs_on_native_ngspice() {
    if !ngspice_ready() {
        return;
    }
    let mut flex = FlexRuntime::from_yaml_with_base(&flex_ldr_yaml(), &flex_ldr_dir())
        .expect("build Flex runtime");
    let enable = flex.port_id("enable").unwrap();
    let light = flex.port_id("light").unwrap();
    let v = flex.port_id("ldr_volts").unwrap();
    let dark = flex.port_id("dark").unwrap();

    // Unpowered (negative control): no light level moves the node.
    flex.advance_ns(1_000_000).unwrap();
    assert!(flex.read(v).unwrap().abs() < 1e-9);
    flex.write(light, 1.0).unwrap();
    flex.advance_ns(1_000_000).unwrap();
    assert!(
        flex.read(v).unwrap().abs() < 1e-9,
        "unpowered divider must stay at 0 V"
    );
    assert_eq!(flex.read(dark).unwrap(), 0.0);

    // Bright, powered: one 100 us boundary after enable is a first-order rise.
    flex.write(light, 100.0).unwrap();
    flex.write(enable, 1.0).unwrap();
    flex.advance_ns(100_000).unwrap();
    let rise = ldr_volts(100.0) * (1.0 - (-100e-6 / ldr_tau_s(100.0)).exp());
    let first = flex.read(v).unwrap();
    assert!(
        (first - rise).abs() < 0.01 * VDD,
        "after 100 us: {first} V, expected {rise} V"
    );
    flex.advance_ns(9_900_000).unwrap();
    let bright = flex.read(v).unwrap();
    assert!(
        (bright - ldr_volts(100.0)).abs() < 1e-4,
        "bright {bright} V vs {} V",
        ldr_volts(100.0)
    );
    assert_eq!(flex.read(dark).unwrap(), 0.0);

    // Dark: the LDR resistance rises and the threshold output flips.
    flex.write(light, 1.0).unwrap();
    flex.advance_ns(10_000_000).unwrap();
    let dim = flex.read(v).unwrap();
    assert!(
        (dim - ldr_volts(1.0)).abs() < 1e-4,
        "dark {dim} V vs {} V",
        ldr_volts(1.0)
    );
    assert_eq!(flex.read(dark).unwrap(), 1.0);
    assert!(
        dim > bright + 2.0,
        "bright and dark must be distinct observables"
    );

    // The routed voltage is on the scope as a volts channel.
    let batch = flex.analog_trace_snapshot(0);
    assert_eq!(batch.channels.len(), 1);
    assert_eq!(batch.channels[0].name, "ldr.v_out");
    assert_eq!(batch.channels[0].unit, "V");
    let last = batch.samples.last().unwrap();
    assert_eq!(last.time_ns, flex.time_ns());
    assert!((f64::from(last.values[0]) - dim).abs() < 1e-5);
    eprintln!(
        "Flex LDR: 100 us rise {first:.6} V (model {rise:.6}), bright {bright:.6} V (model {:.6}), dark {dim:.6} V (model {:.6}), {} scope samples",
        ldr_volts(100.0),
        ldr_volts(1.0),
        batch.samples.len()
    );

    // The declared domain is the deck's, and a value outside it stops the run
    // instead of extrapolating the curve.
    flex.write(light, 150.0).unwrap();
    let err = flex
        .advance_ns(100_000)
        .expect_err("150 lx is outside the deck's declared range");
    assert!(
        err.to_string()
            .contains("input 'lux' is 150, outside the declared 1..100 lx range"),
        "{err}"
    );

    // A broken source mapping (negative control) fails at construction.
    let broken = flex_ldr_yaml().replace("element: Vsup", "element: Vmissing");
    let Err(err) = FlexRuntime::from_yaml_with_base(&broken, &flex_ldr_dir()) else {
        panic!("ngspice refuses the alter");
    };
    let err = err.to_string();
    assert!(err.contains("vmissing"), "{err}");
}

fn f401_machine() -> Machine<CortexM> {
    let chip_path = root("configs/chips/stm32f401.yaml");
    let descriptor = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "name: \"ngspice-ldr\"\nchip: \"{}\"\nexternal_devices: []\n",
        chip_path.display()
    ))
    .expect("parse manifest");
    let mut bus = SystemBus::from_config(&descriptor, &manifest).expect("build bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(&root("tests/fixtures/stm32f401-blinky.elf"))
        .expect("load blinky fixture");
    machine.load_firmware(&image).expect("load firmware");
    machine
}

/// Firmware stepping: real Cortex-M firmware drives PA5 (the divider supply),
/// ngspice solves the LDR every 100 us in lockstep, and the node voltage is
/// routed to PA0. The light level is a `ui.` co-simulation signal.
#[test]
fn firmware_switches_the_ngspice_ldr_divider_into_pa0() {
    if !ngspice_ready() {
        return;
    }
    let dir = root("examples/cosim-ngspice-ldr");
    let system: SystemManifest =
        serde_yaml::from_str(&std::fs::read_to_string(dir.join("system.yaml")).unwrap()).unwrap();
    let mut machine = f401_machine();
    let mut session = CosimSession::new(&system.cosim_models, &dir, &machine.bus)
        .expect("build session")
        .expect("one model");
    assert_eq!(session.binding_errors(), &[] as &[RoutingError]);
    assert!(session.has_ngspice());
    session.set_signal_number("ui.ldr.lux", 100.0).unwrap();

    let pa0 = |session: &CosimSession| match session.signals().get("board.analog.pa0_volts") {
        Some(CosimSignalValue::F64(v)) => *v,
        _ => 0.0,
    };
    let pa5 = |session: &CosimSession| {
        matches!(
            session.signals().get("board.gpio.pa5"),
            Some(CosimSignalValue::Bool(true))
        )
    };

    // Until the firmware drives PA5 high the divider is unpowered.
    let mut boundaries_before_high = 0u32;
    while !pa5(&session) {
        assert!(
            pa0(&session).abs() < 1e-9,
            "PA0 moved before firmware powered the divider"
        );
        session
            .advance(&mut machine, AdvanceRequest::run(None))
            .expect("advance one boundary");
        boundaries_before_high += 1;
        assert!(
            boundaries_before_high < 2_000,
            "firmware never drove PA5 high in 200 ms"
        );
    }
    // 50 boundaries = 5 ms, 30 time constants at 100 lx.
    for _ in 0..50 {
        session
            .advance(&mut machine, AdvanceRequest::run(None))
            .unwrap();
        assert!(pa5(&session), "blink keeps LD2 on for the first second");
    }
    let bright = pa0(&session);
    assert!(
        (bright - ldr_volts(100.0)).abs() < 1e-4,
        "bright {bright} V"
    );

    session.set_signal_number("ui.ldr.lux", 1.0).unwrap();
    for _ in 0..100 {
        session
            .advance(&mut machine, AdvanceRequest::run(None))
            .unwrap();
    }
    let dim = pa0(&session);
    assert!((dim - ldr_volts(1.0)).abs() < 1e-4, "dark {dim} V");
    eprintln!(
        "firmware LDR: PA5 high after {boundaries_before_high} boundaries; PA0 bright {bright:.6} V, dark {dim:.6} V at cycle {}",
        machine.total_cycles
    );
}

// ---------------------------------------------------------------------------
// Flex test scripts: the portable firmware of `flex-mixed-signal` through
// `run_script`, with its RC solved by native ngspice.

// Needs the `flex-script` feature (the wasmi runner):
// `cargo test -p labwired-core --features flex-script --test cosim cosim_ngspice::`.
#[cfg(feature = "flex-script")]
mod flex_script_runs {
    use super::*;

    /// rc.cir: 10 kOhm * 100 nF.
    const FLEX_RC_TAU_S: f64 = 1e-3;
    /// `previous_boundary`: the drive written at 1 us is latched at 25 us.
    const FLEX_RC_DRIVE_FROM_NS: u64 = 25_000;

    fn flex_rc_volts(time_ns: u64) -> f64 {
        let t = time_ns.saturating_sub(FLEX_RC_DRIVE_FROM_NS) as f64 * 1e-9;
        VDD * (1.0 - (-t / FLEX_RC_TAU_S).exp())
    }

    fn run_flex_rc_script(name: &str) -> labwired_core::flex_script::FlexScriptRun {
        run_flex_rc_script_with(name, |manifest| manifest)
    }

    /// Run a `flex-mixed-signal` script on system-ngspice.yaml, after `edit`.
    fn run_flex_rc_script_with(
        name: &str,
        edit: impl FnOnce(String) -> String,
    ) -> labwired_core::flex_script::FlexScriptRun {
        use labwired_core::flex_script::{run_script_with_factory, FlexResultConfig};
        let dir = root("crates/wasm/tests/fixtures/flex-mixed-signal");
        // `model: ./rc.cir` stays relative and resolves from the fixture dir.
        let manifest = edit(std::fs::read_to_string(dir.join("system-ngspice.yaml")).unwrap());
        let firmware = std::fs::read(dir.join("firmware.wasm")).unwrap();
        let script: labwired_config::FlexTestScript =
            serde_yaml::from_str(&std::fs::read_to_string(dir.join(name)).unwrap()).unwrap();
        script.validate().unwrap();
        let config = FlexResultConfig {
            script: name.into(),
            flex: script.inputs.flex.clone().into(),
            firmware: script.inputs.firmware.clone().into(),
            firmware_sha256: None,
            sequence: script.steps(),
            limits: labwired_config::FlexTestLimits {
                max_steps: script.limits.max_steps,
                max_time_ns: script.limits.max_time_ns,
            },
        };
        run_script_with_factory(&script, config, &manifest, &firmware, |model| {
            build_cosim_adapter_with_base(model, &dir)
        })
    }

    fn flex_rc_samples(run: &labwired_core::flex_script::FlexScriptRun) -> Vec<(u64, f64)> {
        let batch = run.runtime.as_ref().unwrap().analog_trace_snapshot(0);
        assert_eq!(batch.channels.len(), 1);
        assert_eq!(batch.channels[0].name, "rc.v_out");
        assert_eq!(batch.channels[0].unit, "V");
        assert_eq!(batch.dropped, 0);
        batch
            .samples
            .iter()
            .map(|sample| (sample.time_ns, f64::from(sample.values[0])))
            .collect()
    }

    /// The netlist is a second file. The run report must name it and hash its
    /// exact bytes, because the manifest hash cannot.
    #[test]
    fn flex_script_pins_the_ngspice_netlist() {
        if !ngspice_ready() {
            return;
        }
        use sha2::{Digest, Sha256};
        let run = run_flex_rc_script("ngspice-enable.yaml");
        let report = run.result.run_report.as_ref().expect("run report");
        let model = report
            .models
            .iter()
            .find(|model| model.id == "rc")
            .expect("rc model");
        let pin = model
            .source
            .as_ref()
            .expect("the ngspice netlist is pinned");
        assert_eq!(pin.kind, "spice_netlist");
        assert_eq!(pin.origin, "file");
        assert_eq!(pin.path.as_deref(), Some("./rc.cir"));
        let dir = root("crates/wasm/tests/fixtures/flex-mixed-signal");
        let bytes = std::fs::read(dir.join("rc.cir")).unwrap();
        assert_eq!(pin.bytes, bytes.len());
        assert_eq!(pin.sha256, format!("{:x}", Sha256::digest(&bytes)));
    }

    /// Positive control: firmware mode 0 enables the RC; the run passes every
    /// assertion of `ngspice-enable.yaml` and the ngspice node follows the
    /// analytic previous-boundary response at every 25 us boundary.
    #[test]
    fn flex_script_firmware_enables_the_ngspice_rc() {
        if !ngspice_ready() {
            return;
        }
        let run = run_flex_rc_script("ngspice-enable.yaml");
        let result = &run.result;
        assert_eq!(
            (result.status, result.message.as_deref()),
            ("pass", None),
            "{:#?}",
            result
                .assertions
                .iter()
                .filter(|a| !a.passed)
                .collect::<Vec<_>>()
        );
        assert_eq!(result.entry_result, Some(1025));
        assert_eq!(result.time_ns, 1_025_000);
        assert!(result.assertions.iter().all(|a| a.passed));
        let samples = flex_rc_samples(&run);
        assert_eq!(samples.len(), 42, "priming row plus 41 boundaries");
        let mut worst = 0.0f64;
        for &(time_ns, volts) in &samples {
            worst = worst.max((volts - flex_rc_volts(time_ns)).abs());
        }
        // The scope stores f32 (about 2.4e-7 V resolution near 2 V).
        assert!(
            worst < 1e-5,
            "ngspice RC departs from the analytic curve by {worst} V"
        );
        let at = |time_ns| samples.iter().find(|s| s.0 == time_ns).unwrap().1;
        assert_eq!(
            at(25_000),
            0.0,
            "the drive must not reach the circuit before it is latched"
        );
        eprintln!(
            "Flex ngspice RC: entry_result {:?}, v(1000 us) {:.6} V (analytic {:.6}), v(1025 us) {:.6} V (analytic {:.6}), worst |error| {worst:.3e} V over {} samples",
            result.entry_result,
            at(1_000_000),
            flex_rc_volts(1_000_000),
            at(1_025_000),
            flex_rc_volts(1_025_000),
            samples.len()
        );
    }

    /// Negative control: firmware mode 1 submits the wrong command; the ngspice
    /// RC stays at exactly 0 V and the firmware times out.
    #[test]
    fn flex_script_wrong_command_leaves_the_ngspice_rc_low() {
        if !ngspice_ready() {
            return;
        }
        let run = run_flex_rc_script("ngspice-disabled.yaml");
        let result = &run.result;
        assert_eq!(result.status, "pass", "{:?}", result.message);
        assert_eq!(result.entry_result, Some(0));
        assert_eq!(result.time_ns, 4_000_000);
        let samples = flex_rc_samples(&run);
        assert_eq!(samples.len(), 161);
        assert!(samples.iter().all(|&(_, volts)| volts == 0.0));
    }

    /// A broken source mapping fails while the models are built: a config
    /// error with ngspice's reason, and no firmware step.
    #[test]
    fn flex_script_with_a_broken_ngspice_model_is_a_config_error() {
        if !ngspice_ready() {
            return;
        }
        use labwired_core::flex_script::FlexStopReason;
        let run = run_flex_rc_script_with("ngspice-enable.yaml", |manifest| {
            manifest.replace("element: Vdrive", "element: Vmissing")
        });
        assert_eq!(run.result.status, "error");
        assert!(matches!(
            run.result.stop_reason,
            FlexStopReason::ConfigError
        ));
        assert_eq!(run.result.steps_executed, 0);
        let message = run.result.message.unwrap();
        assert!(message.contains("vmissing"), "{message}");
    }
}

/// No solver: `previous_boundary` hands each boundary the inputs latched at
/// the previous one, needs the (0, 0) step first, and rejects other values.
#[test]
fn ngspice_previous_boundary_latch_delays_inputs_by_one_boundary() {
    use labwired_core::cosim::ngspice::NgspiceInputLatch;
    let config = |sampling: &str| {
        model(&format!(
            "id: rc\nadapter: ngspice\nmodel: ./rc.cir\nstep_ns: 1000\ninputs: {{ gpio: fw.drive }}\noutputs: {{ v_out: fw.v }}\nconfig: {{ input_sampling: {sampling} }}\n"
        ))
    };
    let high = CosimSignalValue::Bool(true);
    let low = CosimSignalValue::Bool(false);
    let mut latch = NgspiceInputLatch::new(&config("previous_boundary")).unwrap();
    let seen = |step: CosimStep| step.inputs["gpio"].clone();
    assert_eq!(
        seen(latch.sample(step(0, 0, &[("gpio", low.clone())])).unwrap()),
        low
    );
    assert_eq!(
        seen(
            latch
                .sample(step(1000, 1000, &[("gpio", high.clone())]))
                .unwrap()
        ),
        low
    );
    assert_eq!(
        seen(
            latch
                .sample(step(2000, 1000, &[("gpio", high.clone())]))
                .unwrap()
        ),
        high
    );

    let mut unprimed = NgspiceInputLatch::new(&config("previous_boundary")).unwrap();
    let err = unprimed
        .sample(step(1000, 1000, &[("gpio", high.clone())]))
        .unwrap_err();
    assert!(err.contains("initialization step"), "{err}");

    let mut current = NgspiceInputLatch::new(&config("current_boundary")).unwrap();
    assert_eq!(
        seen(
            current
                .sample(step(1000, 1000, &[("gpio", high.clone())]))
                .unwrap()
        ),
        high
    );
    let Err(err) = NgspiceInputLatch::new(&config("sometimes")) else {
        panic!("an unknown input_sampling must be rejected");
    };
    assert!(err.contains("config.input_sampling"), "{err}");
}
