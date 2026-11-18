//! Native model reference for the portable firmware browser fixture.
//! No Cpu, Machine, target instructions, or MMIO mapping participates here.

use labwired_core::flex::FlexRuntime;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const SYSTEM: &str = include_str!("../tests/fixtures/flex-mixed-signal/system.yaml");
const POMODORO_SYSTEM: &str = include_str!("../tests/fixtures/flex-heypcb-pomodoro/system.yaml");
const POMODORO_IMPORT: &str =
    include_str!("../tests/fixtures/flex-heypcb-pomodoro/mcp-import.json");
const POMODORO_SOURCE_MAP: &str =
    include_str!("../tests/fixtures/flex-heypcb-pomodoro/source-map.json");
const GPIO_REUSE_POMODORO: &str = include_str!("../tests/fixtures/flex-gpio-reuse/system.yaml");
const GPIO_REUSE_UNO: &str = include_str!("../tests/fixtures/flex-gpio-reuse/arduino-uno.yaml");

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

#[test]
fn flex_heypcb_pomodoro_behavior_ports_match_the_pinned_generic_import_graph() {
    let imported: serde_json::Value = serde_json::from_str(POMODORO_IMPORT).unwrap();
    let source_map: serde_json::Value = serde_json::from_str(POMODORO_SOURCE_MAP).unwrap();

    assert_eq!(imported["source"]["repository"], "HeyPCB/pomodoro-pal");
    assert_eq!(source_map["source"]["repository"], "HeyPCB/pomodoro-pal");
    assert_eq!(
        imported["source"]["revision"], source_map["source"]["revision"],
        "MCP import snapshot and behavior source map must use the same immutable project revision"
    );
    assert_eq!(
        imported["source"]["revision"], "1d6e50ff9998978e6f4521895042ae240fff7c42",
        "the source-backed demonstration must stay on its reviewed immutable revision"
    );
    assert_eq!(imported["source"]["license_spdx"], "CERN-OHL-S-2.0");
    assert_eq!(source_map["source"]["license_spdx"], "CERN-OHL-S-2.0");
    for artifact in ["schematic", "pcb", "license"] {
        assert_eq!(
            imported["source"]["files"][artifact], source_map["source"]["files"][artifact],
            "MCP import and source map must pin the same {artifact} file"
        );
    }
    assert_eq!(
        source_map["source"]["files"]["original_firmware_hex"]["path"],
        "firmware/pomodoro.hex"
    );
    assert_eq!(
        source_map["source"]["files"]["original_firmware_hex"]["git_blob"],
        "be13b893def87531ac8f06468b81b1da3b93c963",
        "the original firmware reference must remain pinned even though it is not executed"
    );
    assert_eq!(
        imported["generic_import"]["schematic"]["mode"],
        "twin_ready"
    );
    assert_eq!(imported["generic_import"]["schematic"]["mapped"], 36);
    assert_eq!(imported["generic_import"]["schematic"]["ignored"], 5);
    assert_eq!(imported["generic_import"]["schematic"]["dropped"], 0);
    assert_eq!(imported["generic_import"]["pcb"]["dropped_refs"][0], "BZ1");
    assert_eq!(
        imported["generic_import"]["pcb"]["board_file"]["components"],
        41
    );
    assert_eq!(imported["generic_import"]["pcb"]["board_file"]["nets"], 42);
    assert_eq!(
        imported["generic_import"]["pcb"]["board_file"]["shared_trace"]["name"],
        "LEAF"
    );
    assert_eq!(
        imported["generic_import"]["pcb"]["board_file"]["shared_trace"]["pads"],
        serde_json::json!(["R5.1", "R6.1", "U1.3"])
    );
    assert_eq!(imported["discovery"]["mcu"]["pd6"]["gpio"], 6);
    assert_eq!(imported["discovery"]["mcu"]["pin_count"], 22);
    assert_eq!(
        imported["discovery"]["led"]["simulation_backing"],
        "netlist-only"
    );
    assert!(!imported["validation"]["ok"].as_bool().unwrap());
    assert_eq!(imported["validation"]["error_count"], 1);
    assert_eq!(imported["validation"]["warning_count"], 2);
    assert_eq!(imported["saved_project_query"]["saved_projects"], 0);
    assert_eq!(
        imported["part_knowledge"]["ch32v003f4p6"]["outcome"],
        "NOT_FOUND"
    );
    assert_eq!(
        imported["part_knowledge"]["PKMCS0909E4000"]["outcome"],
        "NOT_FOUND"
    );

    let wires = imported["generic_import"]["schematic"]["source_slice_wire_edges"]
        .as_array()
        .expect("source-slice wire edges from the generic import");
    let canonical_edge = |left: &str, right: &str| {
        if left <= right {
            vec![left.to_owned(), right.to_owned()]
        } else {
            vec![right.to_owned(), left.to_owned()]
        }
    };
    let mut actual_edges: BTreeSet<Vec<String>> = BTreeSet::new();
    let mut adjacency: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for pair in wires {
        let [left, right] = pair.as_array().expect("wire has two endpoints").as_slice() else {
            panic!("wire must have exactly two endpoints: {pair}");
        };
        let left = left.as_str().unwrap().to_owned();
        let right = right.as_str().unwrap().to_owned();
        actual_edges.insert(canonical_edge(&left, &right));
        adjacency
            .entry(left.clone())
            .or_default()
            .push(right.clone());
        adjacency.entry(right).or_default().push(left);
    }

    let expected_edges: BTreeSet<Vec<String>> = [
        ["D1.A", "R5.2"],
        ["D2.A", "R6.2"],
        ["BZ1.-", "D1.K"],
        ["BZ1.-", "D2.K"],
        ["R5.1", "R6.1"],
        ["R5.1", "U1.PD6"],
    ]
    .into_iter()
    .map(|[left, right]| canonical_edge(left, right))
    .collect();
    assert_eq!(actual_edges, expected_edges);

    let connected = |start: &str, end: &str| {
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(start.to_owned());
        while let Some(node) = queue.pop_front() {
            if node == end {
                return true;
            }
            if seen.insert(node.clone()) {
                if let Some(neighbors) = adjacency.get(&node) {
                    queue.extend(neighbors.iter().cloned());
                }
            }
        }
        false
    };
    for pair in source_map["source_grounded_behavior_slice"]["source_connections"]
        .as_array()
        .unwrap()
    {
        let pair = pair.as_array().unwrap();
        assert!(
            connected(pair[0].as_str().unwrap(), pair[1].as_str().unwrap()),
            "source connection {:?} is absent from the pinned generic import wire graph",
            pair
        );
    }

    for signal in [
        "source.U1.PD6",
        "source.D1.logic_state",
        "source.D2.logic_state",
    ] {
        assert!(
            POMODORO_SYSTEM.contains(signal),
            "Flex contract omits {signal}"
        );
    }
    assert!(POMODORO_SYSTEM.contains("adapter: behavior"));
    assert!(POMODORO_SYSTEM.contains("primitive: gpio_device"));
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

fn run_gpio_reuse_reference(runtime: &mut FlexRuntime, positive: bool) -> u32 {
    let drive = runtime.port_id("drive").unwrap();
    let led = runtime.port_id("led_state").unwrap();
    if !positive {
        runtime.write(drive, 0.0).unwrap();
        runtime.advance_ns(5_000).unwrap();
        return if runtime.read(led).unwrap() == 0.0 {
            0
        } else {
            2
        };
    }

    runtime.write(drive, 1.0).unwrap();
    runtime.advance_ns(1_000).unwrap();
    if runtime.read(led).unwrap() != 1.0 {
        return 2;
    }
    runtime.write(drive, 0.0).unwrap();
    runtime.advance_ns(1_000).unwrap();
    if runtime.read(led).unwrap() != 0.0 {
        return 2;
    }
    runtime.write(drive, 1.0).unwrap();
    runtime.advance_ns(2_000).unwrap();
    if runtime.read(led).unwrap() != 1.0 {
        return 2;
    }
    runtime.write(drive, 0.0).unwrap();
    runtime.advance_ns(1_000).unwrap();
    if runtime.read(led).unwrap() != 0.0 {
        return 2;
    }
    1
}

fn gpio_reuse_evidence(yaml: &str, positive: bool) -> serde_json::Value {
    let mut runtime = FlexRuntime::from_yaml(yaml).expect("load GPIO reuse contract");
    let result = run_gpio_reuse_reference(&mut runtime, positive);
    serde_json::json!({
        "result": result,
        "time_ns": runtime.time_ns(),
        "trace": pomodoro_digital_evidence(&mut runtime),
    })
}

#[test]
fn flex_gpio_reuse_binds_one_data_driven_behavior_to_pomodoro_and_uno_with_held_out_workload() {
    let pomodoro: serde_yaml::Value = serde_yaml::from_str(GPIO_REUSE_POMODORO).unwrap();
    let uno: serde_yaml::Value = serde_yaml::from_str(GPIO_REUSE_UNO).unwrap();
    assert_eq!(
        pomodoro["cosim_models"][0]["config"]["behavior"],
        uno["cosim_models"][0]["config"]["behavior"],
        "both hardware descriptions must use the same reusable behavior block"
    );
    assert_eq!(
        pomodoro["cosim_models"][0]["inputs"]["drive"].as_str(),
        Some("source.U1.PD6")
    );
    assert_eq!(
        pomodoro["cosim_models"][0]["outputs"]["led"].as_str(),
        Some("source.D1.logic_state")
    );
    assert_eq!(
        uno["cosim_models"][0]["inputs"]["drive"].as_str(),
        Some("arduino.portb.pb5")
    );
    assert_eq!(
        uno["cosim_models"][0]["outputs"]["led"].as_str(),
        Some("arduino.status_led.logic_state")
    );

    let measured = serde_json::json!({
        "pomodoro": {
            "positive": gpio_reuse_evidence(GPIO_REUSE_POMODORO, true),
            "negative": gpio_reuse_evidence(GPIO_REUSE_POMODORO, false),
        },
        "arduino_uno": {
            "positive": gpio_reuse_evidence(GPIO_REUSE_UNO, true),
            "negative": gpio_reuse_evidence(GPIO_REUSE_UNO, false),
        },
    });
    assert_eq!(measured["pomodoro"]["positive"]["result"], 1);
    assert_eq!(measured["pomodoro"]["positive"]["time_ns"], 5_000);
    assert_eq!(
        measured["pomodoro"]["positive"]["trace"]["edges"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert_eq!(measured["pomodoro"]["negative"]["result"], 0);
    assert!(measured["pomodoro"]["negative"]["trace"]["edges"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(measured["arduino_uno"]["positive"]["result"], 1);
    assert_eq!(measured["arduino_uno"]["positive"]["time_ns"], 5_000);
    assert_eq!(
        measured["arduino_uno"]["positive"]["trace"]["edges"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert_eq!(measured["arduino_uno"]["negative"]["result"], 0);
    assert!(measured["arduino_uno"]["negative"]["trace"]["edges"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(measured["pomodoro"]["positive"]["trace"]["dropped"].as_u64() == Some(0));
    assert!(measured["arduino_uno"]["positive"]["trace"]["dropped"].as_u64() == Some(0));

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/flex-gpio-reuse/native.json");
    if std::env::var_os("LABWIRED_REGEN_FLEX_GPIO_REUSE").is_some() {
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
        "cross-hardware GPIO behavior evidence changed"
    );
}
