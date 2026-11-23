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
    runtime_snapshot_refusal, validate_ngspice_models, NgspiceModelSpec,
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

fn rc_model(netlist: &Path, step_ns: u64) -> CosimModelConfig {
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
        netlist.display()
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
    let host: serde_json::Value = serde_json::from_str(&spec.host_json()).unwrap();
    assert_eq!(host["inputs"]["supply"]["element"], "Vsup");
    assert_eq!(host["outputs"]["v_out"]["node"], "out");
    assert!(host["netlist_path"].as_str().unwrap().ends_with("ldr.cir"));

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
    let good = dir.join("rc.cir");
    std::fs::write(&good, RC_NETLIST).unwrap();
    for (netlist, needle) in [
        ("* no tran\nV1 a 0 1\n.end\n", ".tran and .end"),
        ("* no end\nV1 a 0 1\n.tran 1u 1m\n", ".tran and .end"),
        (
            "* ctl\nV1 a 0 1\n.tran 1u 1m\n.control\nrun\n.endc\n.end\n",
            "control block",
        ),
    ] {
        let path = dir.join("bad.cir");
        std::fs::write(&path, netlist).unwrap();
        let err = NgspiceModelSpec::from_config(&rc_model(&path, 1000), &dir).unwrap_err();
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
            "id: rc\nadapter: ngspice\nmodel: {}\nstep_ns: 1000\ninputs: {{ gpio: board.gpio.pa5 }}\noutputs: {{ v_out: board.analog.pa0_volts }}\nconfig:\n  {inputs}\n  {outputs}\n",
            good.display()
        ));
        let err = NgspiceModelSpec::from_config(&config, &dir).unwrap_err();
        assert!(err.contains(needle), "expected '{needle}' in: {err}");
        assert!(err.contains("ngspice model 'rc'"), "{err}");
    }
    let gpio_in = model(&format!(
        "id: rc\nadapter: ngspice\nmodel: {}\noutputs: {{ high: board.gpio_in.pc13 }}\nconfig:\n  output_nodes: {{ high: {{ node: out, scale: 1, offset: 0 }} }}\n",
        good.display()
    ));
    let err = NgspiceModelSpec::from_config(&gpio_in, &dir).unwrap_err();
    assert!(err.contains("threshold is required"), "{err}");
    // The factory reports the same error before it starts any process.
    let Err(err) = build_cosim_adapter_with_base(&gpio_in, &dir) else {
        panic!("the factory must refuse a gpio_in output without a threshold");
    };
    assert!(err.to_string().contains("threshold is required"), "{err}");
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
    const STEP_NS: u64 = 123_457;
    let mut adapter = build_cosim_adapter_with_base(&rc_model(&netlist, STEP_NS), &dir).unwrap();
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
    let mut native = build_cosim_adapter_with_base(&rc_model(&netlist, 100_000), &dir).unwrap();
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
    let mut adapter = build_cosim_adapter_with_base(&rc_model(&netlist, 100_000), &dir).unwrap();
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

    // An element the netlist does not contain: ngspice refuses the alter.
    let mut config = rc_model(&netlist, 1000);
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
    let mut adapter = build_cosim_adapter_with_base(&rc_model(&netlist, 1000), &dir).unwrap();
    adapter
        .step(step(1000, 1000, &[("gpio", CosimSignalValue::Bool(true))]))
        .unwrap();
    let err = adapter
        .step(step(1000, 1000, &[("gpio", CosimSignalValue::Bool(true))]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("monotonically"), "{err}");

    // A GPIO route carries a logic level, as in the browser runtime.
    let mut adapter = build_cosim_adapter_with_base(&rc_model(&netlist, 1000), &dir).unwrap();
    let err = adapter
        .step(step(1000, 1000, &[("gpio", CosimSignalValue::F64(1.0))]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("must be boolean"), "{err}");
}

fn flex_ldr_yaml() -> String {
    let dir = root("examples/cosim-ngspice-ldr");
    let netlist = dir.join("ldr.cir").canonicalize().unwrap();
    std::fs::read_to_string(dir.join("flex-ldr.yaml"))
        .unwrap()
        .replace("model: ./ldr.cir", &format!("model: {}", netlist.display()))
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
    let mut flex = FlexRuntime::from_yaml(&flex_ldr_yaml()).expect("build Flex runtime");
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

    // A broken source mapping (negative control) fails at construction.
    let broken = flex_ldr_yaml().replace("element: Vsup", "element: Vmissing");
    let Err(err) = FlexRuntime::from_yaml(&broken) else {
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
        use labwired_core::flex_script::{run_script, FlexResultConfig};
        let dir = root("crates/wasm/tests/fixtures/flex-mixed-signal");
        let netlist = dir.join("rc.cir").canonicalize().unwrap();
        let manifest = edit(
            std::fs::read_to_string(dir.join("system-ngspice.yaml"))
                .unwrap()
                .replace("model: ./rc.cir", &format!("model: {}", netlist.display())),
        );
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
        run_script(&script, config, &manifest, &firmware)
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
