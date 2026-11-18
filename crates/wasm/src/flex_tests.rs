//! Native model reference for the portable firmware browser fixture.
//! No Cpu, Machine, target instructions, or MMIO mapping participates here.

use labwired_core::flex::FlexRuntime;
use std::collections::BTreeMap;

const SYSTEM: &str = include_str!("../tests/fixtures/flex-mixed-signal/system.yaml");
const POMODORO_SYSTEM: &str = include_str!("../tests/fixtures/flex-heypcb-pomodoro/system.yaml");

fn run_model(yaml: &str, wrong_command: bool) -> (FlexRuntime, u32) {
    let mut runtime = FlexRuntime::from_yaml(yaml).expect("load portable Flex contract");
    let command = runtime.port_id("command").unwrap();
    let voltage = runtime.port_id("voltage").unwrap();
    runtime
        .write(command, if wrong_command { 0.0 } else { 1.0 })
        .unwrap();
    // Same host calls as firmware.rs. This is a native model reference;
    // test_browser_flex.mjs executes the actual compiled firmware module.
    for sample in 1..=160 {
        runtime.advance_ns(25_000).unwrap();
        if runtime.read(voltage).unwrap() >= 3.3 * 0.632 {
            return (runtime, sample * 25);
        }
    }
    (runtime, 0)
}

fn assert_rc_curve(runtime: &FlexRuntime, time_constant_ns: f64) {
    let trace = runtime.analog_trace_snapshot(0);
    assert!(trace.samples.len() >= 35);
    for sample in trace.samples {
        let charge_ns = sample.time_ns.saturating_sub(25_000) as f64;
        let expected = 3.3 * (1.0 - (-charge_ns / time_constant_ns).exp());
        // The analog block latches the ready signal at 25us. Subsequent
        // samples integrate that value over the completed interval. The
        // 25us solver step introduces a bounded discretization error.
        // Check the complete RC shape, not only a threshold that a
        // fabricated delayed step could satisfy.
        assert!(
            (f64::from(sample.values[0]) - expected).abs() < 0.04,
            "RC voltage at {}ns: {}V, analytic {}V",
            sample.time_ns,
            sample.values[0],
            expected
        );
    }
}

fn digital_evidence(runtime: &mut FlexRuntime, driven: bool) -> serde_json::Value {
    let trace = serde_json::to_value(runtime.digital_trace_snapshot(0)).unwrap();
    assert_eq!(trace["time_unit"], "ns");
    assert_eq!(trace["dropped"], 0);
    let channels = trace["channels"].as_array().unwrap();
    let names: Vec<_> = channels
        .iter()
        .map(|c| c["signal"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["control.enable", "device.ready"]);
    assert!(channels.iter().all(|c| c["initial"] == false));
    let expected_edges = if driven {
        serde_json::json!([
            {"ch": channels[0]["id"], "time_ns": 0, "value": 1},
            {"ch": channels[1]["id"], "time_ns": 1000, "value": 1}
        ])
    } else {
        serde_json::json!([])
    };
    assert_eq!(
        trace["edges"], expected_edges,
        "digital transition times and values"
    );
    trace
}

fn evidence(runtime: &mut FlexRuntime, tau_us: u32) -> serde_json::Value {
    let batch = runtime.analog_trace_snapshot(0);
    assert!(
        batch.samples.len() >= 35,
        "trace must contain the charge curve"
    );
    let channels: Vec<_> = batch.channels.iter().map(|c| &c.name).collect();
    assert_eq!(channels, ["rc.v_out"]);
    let rows: Vec<_> = batch
        .samples
        .iter()
        .map(|s| serde_json::json!([s.time_ns, format!("{:08x}", s.values[0].to_bits())]))
        .collect();
    serde_json::json!({"tau_us":tau_us, "time_ns":runtime.time_ns(), "trace":{"channels":channels,"rows":rows}, "digital":digital_evidence(runtime, true)})
}

#[test]
fn flex_portable_firmware_contract_has_a_native_analog_reference() {
    let (mut first, first_tau) = run_model(SYSTEM, false);
    let double_resistance = SYSTEM.replace("R1 in out 10k", "R1 in out 20k");
    assert_ne!(double_resistance, SYSTEM);
    let (mut second, second_tau) = run_model(&double_resistance, false);
    // R*C analytically gives 1ms and 2ms, with a 25us sample delay before
    // charging. Allow one extra solver sample for threshold quantization;
    // a constant output cannot satisfy both parameterized responses.
    assert!(
        (1000..=1050).contains(&first_tau),
        "1ms RC measured {first_tau}us"
    );
    assert!(
        (2000..=2050).contains(&second_tau),
        "2ms RC measured {second_tau}us"
    );
    assert!((second_tau as i32 - 2 * first_tau as i32).abs() <= 50);
    assert_rc_curve(&first, 1_000_000.0);
    assert_rc_curve(&second, 2_000_000.0);
    let measured = serde_json::json!({"cases":[evidence(&mut first,first_tau),evidence(&mut second,second_tau)]});
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/flex-mixed-signal/native.json");
    // Validate both physical responses before allowing reference regeneration.
    if std::env::var_os("LABWIRED_REGEN_FLEX_PILOT").is_some() {
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&measured).unwrap() + "\n",
        )
        .unwrap();
    }
    let expected: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        measured, expected,
        "native model observations changed; compiled browser runtime must match"
    );
}

#[test]
fn flex_wrong_command_cannot_generate_analog_completion() {
    let (mut runtime, tau) = run_model(SYSTEM, true);
    assert_eq!(tau, 0, "incorrect portable firmware must time out");
    assert_eq!(runtime.time_ns(), 4_000_000);
    digital_evidence(&mut runtime, false);
    let trace = runtime.analog_trace_snapshot(0);
    assert!(trace.samples.len() >= 160);
    assert!(
        trace.samples.iter().all(|s| s.values[0] == 0.0),
        "no invented analog response"
    );
}

#[test]
fn flex_portable_contract_rejects_invalid_access_and_missing_behavior() {
    let mut runtime = FlexRuntime::from_yaml(SYSTEM).unwrap();
    let command = runtime.port_id("command").unwrap();
    let voltage = runtime.port_id("voltage").unwrap();
    assert!(runtime.port_id("does_not_exist").is_err());
    assert!(runtime.write(u32::MAX, 1.0).is_err());
    assert!(runtime.read(u32::MAX).is_err());
    assert!(
        runtime.write(voltage, 1.0).is_err(),
        "read-only analog observation cannot be driven"
    );
    assert!(
        runtime.read(command).is_err(),
        "write-only command has no fabricated read value"
    );
    assert!(runtime.write(command, f64::NAN).is_err());
    assert!(runtime.write(command, f64::INFINITY).is_err());
    let wrong_unit = SYSTEM.replace("unit: V", "unit: mV");
    assert!(
        FlexRuntime::from_yaml(&wrong_unit).is_err(),
        "numeric observations cannot silently convert units"
    );
    let inconsistent_initial = SYSTEM.replace("initial: 0", "initial: 1");
    assert!(
        FlexRuntime::from_yaml(&inconsistent_initial).is_err(),
        "the analog DC source must agree with the initial digital drive"
    );
    let unbound = SYSTEM.replace(
        "name: voltage, signal: circuit.voltage",
        "name: voltage, signal: missing.voltage",
    );
    assert!(
        FlexRuntime::from_yaml(&unbound).is_err(),
        "a read port must have a model output"
    );
    let unsupported = SYSTEM.replace("primitive: gpio_device", "primitive: invented_device");
    assert!(
        FlexRuntime::from_yaml(&unsupported).is_err(),
        "unsupported behavior must not become a silent stub"
    );
}

fn pomodoro_pd6_reference(runtime: &mut FlexRuntime, negative_control: bool) -> u32 {
    let drive = runtime.port_id("u1_pd6").unwrap();
    let d1 = runtime.port_id("d1_state").unwrap();
    let d2 = runtime.port_id("d2_state").unwrap();
    if negative_control {
        runtime.write(drive, 0.0).unwrap();
        runtime.advance_ns(2_000).unwrap();
        return if runtime.read(d1).unwrap() == 0.0 && runtime.read(d2).unwrap() == 0.0 {
            0
        } else {
            2
        };
    }

    runtime.write(drive, 1.0).unwrap();
    runtime.advance_ns(1_000).unwrap();
    let asserted = runtime.read(d1).unwrap() == 1.0 && runtime.read(d2).unwrap() == 1.0;
    runtime.write(drive, 0.0).unwrap();
    runtime.advance_ns(1_000).unwrap();
    let cleared = runtime.read(d1).unwrap() == 0.0 && runtime.read(d2).unwrap() == 0.0;
    if asserted && cleared {
        1
    } else {
        2
    }
}

fn pomodoro_digital_evidence(runtime: &mut FlexRuntime) -> serde_json::Value {
    let batch = runtime.digital_trace_snapshot(0);
    let mut channels: Vec<_> = batch
        .channels
        .iter()
        .map(|channel| {
            (
                channel.id,
                serde_json::json!({
                    "signal": channel.signal,
                    "initial": channel.initial,
                }),
            )
        })
        .collect();
    channels.sort_by(|a, b| a.1["signal"].as_str().cmp(&b.1["signal"].as_str()));
    let signal_by_id: BTreeMap<_, _> = batch
        .channels
        .iter()
        .map(|channel| (channel.id, channel.signal.as_str()))
        .collect();
    let mut edges: Vec<_> = batch
        .edges
        .into_iter()
        .map(|edge| {
            let signal = signal_by_id
                .get(&edge.ch)
                .copied()
                .expect("digital edge references a channel");
            (edge.time_ns, signal, edge.value)
        })
        .collect();
    edges.sort_unstable();
    serde_json::json!({
        "time_unit": batch.time_unit,
        "dropped": batch.dropped,
        "channels": channels.into_iter().map(|(_, channel)| channel).collect::<Vec<_>>(),
        "edges": edges.into_iter().map(|(time_ns, signal, value)| serde_json::json!({
            "signal": signal,
            "time_ns": time_ns,
            "value": value,
        })).collect::<Vec<_>>(),
    })
}

#[test]
fn flex_heypcb_pomodoro_source_mapped_pd6_pair_has_native_reference_and_negative_control() {
    let mut positive = FlexRuntime::from_yaml(POMODORO_SYSTEM).expect("Pomodoro Flex manifest");
    let positive_result = pomodoro_pd6_reference(&mut positive, false);
    let positive_evidence = serde_json::json!({
        "result": positive_result,
        "time_ns": positive.time_ns(),
        "trace": pomodoro_digital_evidence(&mut positive),
    });
    assert_eq!(
        positive_result, 1,
        "source-mapped positive control must assert both LED logic outputs"
    );
    assert_eq!(positive.time_ns(), 2_000);
    let positive_channels = positive_evidence["trace"]["channels"]
        .as_array()
        .expect("positive channels");
    assert_eq!(positive_channels.len(), 3);
    assert_eq!(
        positive_evidence["trace"]["edges"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
    assert_eq!(positive_evidence["trace"]["dropped"], 0);

    let mut negative = FlexRuntime::from_yaml(POMODORO_SYSTEM).expect("Pomodoro Flex manifest");
    let negative_result = pomodoro_pd6_reference(&mut negative, true);
    let negative_evidence = serde_json::json!({
        "result": negative_result,
        "time_ns": negative.time_ns(),
        "trace": pomodoro_digital_evidence(&mut negative),
    });
    assert_eq!(
        negative_result, 0,
        "negative control must leave both outputs inactive"
    );
    assert_eq!(negative.time_ns(), 2_000);
    assert!(negative_evidence["trace"]["edges"]
        .as_array()
        .unwrap()
        .is_empty());

    let measured = serde_json::json!({
        "positive": positive_evidence,
        "negative": negative_evidence,
    });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/flex-heypcb-pomodoro/native.json");
    if std::env::var_os("LABWIRED_REGEN_FLEX_POMODORO").is_some() {
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&measured).unwrap() + "\n",
        )
        .unwrap();
    }
    let expected: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        measured, expected,
        "source-mapped native model observation changed"
    );
}
