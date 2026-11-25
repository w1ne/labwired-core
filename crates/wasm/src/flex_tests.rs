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
const HEYPCB_LDR_SYSTEM: &str = include_str!("../tests/fixtures/flex-heypcb-esp32-ldr/system.yaml");
const HEYPCB_LDR_IMPORT: &str =
    include_str!("../tests/fixtures/flex-heypcb-esp32-ldr/mcp-import.json");
const HEYPCB_LDR_SOURCE_MAP: &str =
    include_str!("../tests/fixtures/flex-heypcb-esp32-ldr/source-map.json");
const HEYPCB_LDR_BROWSER: &str =
    include_str!("../tests/fixtures/flex-heypcb-esp32-ldr/browser.json");
const HEYPCB_LDR_FIRMWARE: &[u8] =
    include_bytes!("../tests/fixtures/flex-heypcb-esp32-ldr/firmware.wasm");
const HEYPCB_LDR_FIRMWARE_SOURCE: &[u8] =
    include_bytes!("../tests/fixtures/flex-heypcb-esp32-ldr/firmware.rs");
const HEYPCB_LDR_SYSTEM_BYTES: &[u8] =
    include_bytes!("../tests/fixtures/flex-heypcb-esp32-ldr/system.yaml");
const HEYPCB_LDR_SOURCE_MAP_BYTES: &[u8] =
    include_bytes!("../tests/fixtures/flex-heypcb-esp32-ldr/source-map.json");
const LDR_NETWORK_SYSTEM: &str =
    include_str!("../tests/fixtures/flex-heypcb-ldr-network/system.yaml");
const LDR_NETWORK_SOURCE_MAP: &str =
    include_str!("../tests/fixtures/flex-heypcb-ldr-network/source-map.json");
const LDR_NETWORK_BROWSER: &str =
    include_str!("../tests/fixtures/flex-heypcb-ldr-network/browser.json");
const LDR_NETWORK_NATIVE: &str =
    include_str!("../tests/fixtures/flex-heypcb-ldr-network/native.json");
const LDR_NETWORK_FIRMWARE: &[u8] =
    include_bytes!("../tests/fixtures/flex-heypcb-ldr-network/firmware.wasm");
const LDR_NETWORK_FIRMWARE_SOURCE: &[u8] =
    include_bytes!("../tests/fixtures/flex-heypcb-ldr-network/firmware.rs");
const LIPO_SYSTEM: &str = include_str!("../tests/fixtures/flex-analog-multi-input/system.yaml");
const LIPO_BROWSER: &str = include_str!("../tests/fixtures/flex-analog-multi-input/browser.json");
const LIPO_NATIVE: &str = include_str!("../tests/fixtures/flex-analog-multi-input/native.json");
const LIPO_FIRMWARE: &[u8] =
    include_bytes!("../tests/fixtures/flex-analog-multi-input/firmware.wasm");
const LIPO_FIRMWARE_SOURCE: &[u8] =
    include_bytes!("../tests/fixtures/flex-analog-multi-input/firmware.rs");
const LIPO_SYSTEM_BYTES: &[u8] =
    include_bytes!("../tests/fixtures/flex-analog-multi-input/system.yaml");
const TMP36_SYSTEM: &str = include_str!("../tests/fixtures/flex-analog-tmp36/system.yaml");
const TMP36_BROWSER: &str = include_str!("../tests/fixtures/flex-analog-tmp36/browser.json");
const TMP36_NATIVE: &str = include_str!("../tests/fixtures/flex-analog-tmp36/native.json");
const TMP36_FIRMWARE: &[u8] = include_bytes!("../tests/fixtures/flex-analog-tmp36/firmware.wasm");
const TMP36_FIRMWARE_SOURCE: &[u8] =
    include_bytes!("../tests/fixtures/flex-analog-tmp36/firmware.rs");
const TMP36_SYSTEM_BYTES: &[u8] = include_bytes!("../tests/fixtures/flex-analog-tmp36/system.yaml");
const GP2Y_SYSTEM: &str = include_str!("../tests/fixtures/flex-analog-gp2y0a21/system.yaml");
const GP2Y_BROWSER: &str = include_str!("../tests/fixtures/flex-analog-gp2y0a21/browser.json");
const GP2Y_NATIVE: &str = include_str!("../tests/fixtures/flex-analog-gp2y0a21/native.json");
const GP2Y_FIRMWARE: &[u8] = include_bytes!("../tests/fixtures/flex-analog-gp2y0a21/firmware.wasm");
const GP2Y_FIRMWARE_SOURCE: &[u8] =
    include_bytes!("../tests/fixtures/flex-analog-gp2y0a21/firmware.rs");
const GP2Y_SYSTEM_BYTES: &[u8] =
    include_bytes!("../tests/fixtures/flex-analog-gp2y0a21/system.yaml");
const DUAL_GP2Y_SYSTEM: &str = include_str!("../tests/fixtures/flex-dual-gp2y0a21/system.yaml");
const DUAL_GP2Y_BROWSER: &str = include_str!("../tests/fixtures/flex-dual-gp2y0a21/browser.json");
const DUAL_GP2Y_FIRMWARE: &[u8] =
    include_bytes!("../tests/fixtures/flex-dual-gp2y0a21/firmware.wasm");
const DUAL_GP2Y_FIRMWARE_SOURCE: &[u8] =
    include_bytes!("../tests/fixtures/flex-dual-gp2y0a21/firmware.rs");
const DUAL_GP2Y_SYSTEM_BYTES: &[u8] =
    include_bytes!("../tests/fixtures/flex-dual-gp2y0a21/system.yaml");
const NTC_SYSTEM: &str = include_str!("../tests/fixtures/flex-analog-ntc/system.yaml");
const NTC_BROWSER: &str = include_str!("../tests/fixtures/flex-analog-ntc/browser.json");
const NTC_NATIVE: &str = include_str!("../tests/fixtures/flex-analog-ntc/native.json");
const NTC_FIRMWARE: &[u8] = include_bytes!("../tests/fixtures/flex-analog-ntc/firmware.wasm");
const NTC_FIRMWARE_SOURCE: &[u8] = include_bytes!("../tests/fixtures/flex-analog-ntc/firmware.rs");
const NTC_SYSTEM_BYTES: &[u8] = include_bytes!("../tests/fixtures/flex-analog-ntc/system.yaml");
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

    // Tie the runtime port names and model aliases back to the source map.
    // Checking only that these strings occur in the YAML would allow a port
    // to be wired to a different source signal while still passing this test.
    let system: serde_yaml::Value = serde_yaml::from_str(POMODORO_SYSTEM).unwrap();
    let ports = system["ports"].as_sequence().expect("Flex ports");
    let port_for = |name: &str| {
        ports
            .iter()
            .find(|port| port["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("Flex manifest is missing port '{name}'"))
    };
    let source_slice = &source_map["source_grounded_behavior_slice"];
    let gpio_signal = format!(
        "source.{}.{}",
        source_slice["mcu_ref"].as_str().unwrap(),
        source_slice["gpio_pin"].as_str().unwrap()
    );
    let gpio_port = port_for("u1_pd6");
    assert_eq!(gpio_port["signal"].as_str(), Some(gpio_signal.as_str()));
    assert_eq!(gpio_port["access"].as_str(), Some("write"));
    assert_eq!(gpio_port["kind"].as_str(), Some("bool"));
    assert_eq!(gpio_port["initial"].as_i64(), Some(0));

    let models = system["cosim_models"]
        .as_sequence()
        .expect("Flex co-simulation models");
    assert_eq!(
        models.len(),
        1,
        "the source slice has one shared behavior model"
    );
    let model = &models[0];
    assert_eq!(model["id"].as_str(), Some("source-pd6-green-indicators"));
    assert_eq!(model["adapter"].as_str(), Some("behavior"));
    assert_eq!(model["step_ns"].as_u64(), Some(1_000));
    assert_eq!(
        model["inputs"]["drive"].as_str(),
        Some(gpio_signal.as_str())
    );
    let source_components = source_slice["components"]
        .as_array()
        .expect("source-slice components");
    for (port_name, alias, component_ref) in [("d1_state", "d1", "D1"), ("d2_state", "d2", "D2")] {
        assert!(
            source_components
                .iter()
                .any(|component| component["ref"].as_str() == Some(component_ref)),
            "source map omits indicator {component_ref}"
        );
        let expected_signal = format!("source.{component_ref}.logic_state");
        assert_eq!(
            model["outputs"][alias].as_str(),
            Some(expected_signal.as_str())
        );
        let read_port = port_for(port_name);
        assert_eq!(read_port["signal"].as_str(), Some(expected_signal.as_str()));
        assert_eq!(read_port["access"].as_str(), Some("read"));
        assert_eq!(read_port["kind"].as_str(), Some("bool"));
    }
    assert_eq!(
        model["config"]["behavior"]["primitive"].as_str(),
        Some("gpio_device")
    );
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

fn heypcb_ldr_reference(runtime: &mut FlexRuntime, lux: f64) -> serde_json::Value {
    let lux_port = runtime.port_id("lux").unwrap();
    let sensor_port = runtime.port_id("sensor_mv").unwrap();
    let led_drive = runtime.port_id("user_led_drive").unwrap();
    let led_state = runtime.port_id("user_led_state").unwrap();

    runtime.write(lux_port, lux).unwrap();
    runtime.advance_ns(1_000).unwrap();
    let sensor_mv = runtime.read(sensor_port).unwrap();
    let adc_code = (sensor_mv * 4095.0 / 3300.0) as u32;
    let expected_led = adc_code >= 2048;
    runtime
        .write(led_drive, if expected_led { 1.0 } else { 0.0 })
        .unwrap();
    runtime.advance_ns(1_000).unwrap();
    let asserted = runtime.read(led_state).unwrap() == f64::from(u8::from(expected_led));
    let cleared = if expected_led {
        runtime.write(led_drive, 0.0).unwrap();
        runtime.advance_ns(1_000).unwrap();
        runtime.read(led_state).unwrap() == 0.0
    } else {
        asserted
    };

    let mut trace = pomodoro_digital_evidence(runtime);
    for channel in trace["channels"].as_array_mut().unwrap() {
        let signal = channel["signal"].as_str().unwrap();
        let names: Vec<_> = runtime
            .manifest()
            .ports
            .iter()
            .filter(|port| {
                port.kind == labwired_config::FlexPortKind::Bool && port.signal == signal
            })
            .map(|port| port.name.clone())
            .collect();
        channel["port_names"] = serde_json::json!(names);
    }

    serde_json::json!({
        "result": if asserted && cleared { adc_code } else { u32::MAX },
        "lux": lux,
        "sensor_mv": sensor_mv,
        "adc_code": adc_code,
        "led_final": runtime.read(led_state).unwrap(),
        "time_ns": runtime.time_ns(),
        "trace": trace,
        "analog_trace": heypcb_ldr_analog_evidence(runtime),
    })
}

fn heypcb_ldr_analog_evidence(runtime: &FlexRuntime) -> serde_json::Value {
    let batch = runtime.analog_trace_snapshot(0);
    assert_eq!(batch.time_unit, "ns");
    assert_eq!(batch.channels.len(), 1);
    assert_eq!(batch.channels[0].name, "source-u4-gl5528.voltage_mv");
    assert_eq!(batch.channels[0].unit, "mV");
    assert_eq!(batch.channels[0].port_names, ["sensor_mv"]);
    assert!(
        !batch.samples.is_empty(),
        "the LDR scope trace must contain samples"
    );
    serde_json::json!({
        "time_unit": batch.time_unit,
        "channels": batch.channels,
        "dropped": batch.dropped,
        "next_cursor": batch.next_cursor,
        "samples": batch.samples.into_iter().map(|sample| serde_json::json!({
            "cycle": sample.cycle,
            "time_ns": sample.time_ns,
            "values": sample.values,
        })).collect::<Vec<_>>(),
    })
}

/// Same host calls as flex-heypcb-ldr-network/firmware.rs.
fn ldr_network_reference(yaml: &str, lux: f64) -> serde_json::Value {
    let mut runtime = FlexRuntime::from_yaml(yaml).expect("LDR sensor-network manifest");
    let lux_port = runtime.port_id("lux").unwrap();
    let sensor_port = runtime.port_id("sensor_v").unwrap();
    let led_drive = runtime.port_id("user_led_drive").unwrap();
    let led_state = runtime.port_id("user_led_state").unwrap();
    let initial_v = runtime.read(sensor_port).unwrap();

    runtime.write(lux_port, lux).unwrap();
    runtime.advance_ns(4_000_000).unwrap();
    let sensor_v = runtime.read(sensor_port).unwrap();
    let adc_code = (sensor_v * 4095.0 / 3.3) as u32;
    let expected_led = adc_code >= 2048;
    runtime
        .write(led_drive, if expected_led { 1.0 } else { 0.0 })
        .unwrap();
    runtime.advance_ns(1_000).unwrap();
    let asserted = runtime.read(led_state).unwrap() == f64::from(u8::from(expected_led));
    let cleared = if expected_led {
        runtime.write(led_drive, 0.0).unwrap();
        runtime.advance_ns(1_000).unwrap();
        runtime.read(led_state).unwrap() == 0.0
    } else {
        asserted
    };

    let analog = runtime.analog_trace_snapshot(0);
    let digital = runtime.digital_trace_snapshot(0);
    assert_eq!(analog.time_unit, "ns");
    assert_eq!(analog.channels.len(), 1);
    assert_eq!(analog.channels[0].name, "source-sns-an1.voltage");
    assert_eq!(analog.channels[0].unit, "V");
    assert_eq!(analog.channels[0].port_names, ["sensor_v"]);
    assert_eq!(digital.time_unit, "ns");
    serde_json::json!({
        "result": if asserted && cleared { adc_code } else { u32::MAX },
        "lux": lux,
        "initial_v": initial_v,
        "sensor_v": sensor_v,
        "adc_code": adc_code,
        "led_final": runtime.read(led_state).unwrap(),
        "time_ns": runtime.time_ns(),
        "analog": {
            "time_unit": analog.time_unit,
            "channels": analog.channels,
            "dropped": analog.dropped,
            "samples": analog.samples.into_iter().map(|sample| serde_json::json!({
                "cycle": sample.cycle,
                "time_ns": sample.time_ns,
                "values": sample.values,
            })).collect::<Vec<_>>(),
        },
        "digital": {
            "time_unit": digital.time_unit,
            "channels": digital.channels,
            "dropped": digital.dropped,
            "edges": digital.edges.into_iter().map(|edge| serde_json::json!({
                "ch": edge.ch,
                "time_ns": edge.time_ns,
                "value": edge.value,
            })).collect::<Vec<_>>(),
        },
    })
}

/// Settled SNS_AN1 volts for the pinned divider: U4 above R6 = 10 kΩ on 3.3 V.
fn ldr_network_divider_v(r10_ohms: f64, lux: f64) -> f64 {
    let r_u4 = r10_ohms * (lux / 10.0_f64).powf(-0.6);
    3.3 * 10_000.0 / (r_u4 + 10_000.0)
}

fn lipo_analog_reference(soc_pct: f64, usb_present: f64) -> serde_json::Value {
    let mut runtime = FlexRuntime::from_yaml(LIPO_SYSTEM).expect("load multi-input Lipo source");
    let soc = runtime.port_id("soc_pct").unwrap();
    let usb = runtime.port_id("usb_present").unwrap();
    let battery = runtime.port_id("battery_mv").unwrap();
    let default_mv = runtime.read(battery).unwrap();
    runtime.write(soc, soc_pct).unwrap();
    runtime.write(usb, usb_present).unwrap();
    runtime.advance_ns(1_000).unwrap();
    let result_mv = runtime.read(battery).unwrap();
    let batch = runtime.analog_trace_snapshot(0);
    assert_eq!(batch.dropped, 0);
    assert_eq!(batch.time_unit, "ns");
    assert_eq!(batch.channels.len(), 1);
    assert_eq!(batch.channels[0].name, "lipo-battery.voltage_mv");
    assert_eq!(batch.channels[0].unit, "mV");
    assert_eq!(batch.channels[0].port_names, ["battery_mv"]);
    assert_eq!(batch.samples.len(), 2);
    serde_json::json!({
        "soc_pct": soc_pct,
        "usb_present": usb_present,
        "default_mv": default_mv,
        "result_mv": result_mv,
        "time_ns": runtime.time_ns(),
        "trace": {
            "time_unit": batch.time_unit,
            "channels": batch.channels,
            "dropped": batch.dropped,
            "samples": batch.samples.into_iter().map(|sample| serde_json::json!({
                "cycle": sample.cycle,
                "time_ns": sample.time_ns,
                "values": sample.values,
            })).collect::<Vec<_>>(),
        },
    })
}

#[test]
fn flex_lipo_charger_reuses_multi_input_catalog_source_and_dimensionless_unit() {
    let yaml: serde_yaml::Value = serde_yaml::from_str(LIPO_SYSTEM).unwrap();
    let ports = yaml["ports"].as_sequence().unwrap();
    assert_eq!(
        ports
            .iter()
            .find(|port| port["name"] == "usb_present")
            .unwrap()["unit"],
        "1",
        "the catalog's empty dimensionless input unit must be explicit in Flex"
    );
    assert_eq!(
        yaml["cosim_models"][0]["config"]["device_type"],
        "lipo_charger"
    );

    let positive = lipo_analog_reference(50.0, 1.0);
    assert_eq!(positive["default_mv"], 1_875.0);
    assert_eq!(positive["result_mv"], 1_950.0);
    assert_eq!(positive["time_ns"], 1_000);
    let negative = lipo_analog_reference(50.0, 0.0);
    assert_eq!(negative["result_mv"], 1_875.0);
    let high_charge = lipo_analog_reference(100.0, 0.0);
    assert_eq!(high_charge["result_mv"], 2_100.0);
    assert!(positive["result_mv"].as_f64().unwrap() > negative["result_mv"].as_f64().unwrap());

    let measured = serde_json::json!({
        "positive": positive,
        "negative": negative,
        "high_charge": high_charge,
    });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/flex-analog-multi-input/native.json");
    if std::env::var_os("LABWIRED_REGEN_FLEX_LIPO").is_some() {
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
        "multi-input catalog analog observations changed"
    );
}

fn normalize_numbers(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Number(number) => {
            serde_json::json!(number.as_f64().expect("finite fixture number"))
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(normalize_numbers).collect())
        }
        serde_json::Value::Object(fields) => serde_json::Value::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), normalize_numbers(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// The browser evidence of the catalog analog fixtures that share one shape:
/// the committed browser run names the exact firmware, firmware source and
/// manifest bytes, claims native-model equality, and every case's observations
/// equal the native instrument capture. (The GP2Y0A21, dual-sensor and LDR
/// evidence tests also pin traces, run reports or source maps, so they keep
/// their own bodies.)
struct BrowserEvidence {
    name: &'static str,
    browser: &'static str,
    native: &'static str,
    firmware: &'static [u8],
    firmware_source: &'static [u8],
    system_bytes: &'static [u8],
    cases: &'static [&'static str],
    /// Failure text per case; `{case}` is replaced by the case name.
    case_msg: &'static str,
}

const BROWSER_EVIDENCE: &[BrowserEvidence] = &[
    BrowserEvidence {
        name: "lipo",
        browser: LIPO_BROWSER,
        native: LIPO_NATIVE,
        firmware: LIPO_FIRMWARE,
        firmware_source: LIPO_FIRMWARE_SOURCE,
        system_bytes: LIPO_SYSTEM_BYTES,
        cases: &["positive", "negative", "high_charge"],
        case_msg:
            "browser evidence for {case} must include the exact native-matched instrument samples",
    },
    BrowserEvidence {
        name: "tmp36",
        browser: TMP36_BROWSER,
        native: TMP36_NATIVE,
        firmware: TMP36_FIRMWARE,
        firmware_source: TMP36_FIRMWARE_SOURCE,
        system_bytes: TMP36_SYSTEM_BYTES,
        cases: &["cold", "zero", "default", "hot"],
        case_msg: "browser evidence for {case} must match the native instrument trace",
    },
    BrowserEvidence {
        name: "ntc",
        browser: NTC_BROWSER,
        native: NTC_NATIVE,
        firmware: NTC_FIRMWARE,
        firmware_source: NTC_FIRMWARE_SOURCE,
        system_bytes: NTC_SYSTEM_BYTES,
        cases: &["cold", "ambient", "hot"],
        case_msg: "browser NTC evidence for {case} must equal native analog/digital traces",
    },
];

#[test]
fn every_catalog_analog_browser_evidence_pins_firmware_and_manifest_inputs() {
    for row in BROWSER_EVIDENCE {
        let name = row.name;
        let browser: serde_json::Value = serde_json::from_str(row.browser).unwrap();
        let native: serde_json::Value = serde_json::from_str(row.native).unwrap();
        let core_commit = browser["core_commit"].as_str().unwrap();
        assert_eq!(core_commit.len(), 40, "{name}");
        assert!(
            core_commit.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "{name}"
        );
        assert_eq!(browser["native_model_equality"], true, "{name}");
        assert_eq!(browser["firmware_sha256"], sha256(row.firmware), "{name}");
        assert_eq!(
            browser["firmware_source_sha256"],
            sha256(row.firmware_source),
            "{name}"
        );
        assert_eq!(browser["system_sha256"], sha256(row.system_bytes), "{name}");
        for case in row.cases {
            assert_eq!(
                normalize_numbers(&browser["observations"][case]),
                normalize_numbers(&native[case]),
                "{name}: {}",
                row.case_msg.replace("{case}", case)
            );
        }
    }
}

fn tmp36_analog_reference(temperature_c: f64) -> serde_json::Value {
    let mut runtime = FlexRuntime::from_yaml(TMP36_SYSTEM).expect("load registered TMP36 source");
    let temperature = runtime.port_id("temperature_c").unwrap();
    let voltage = runtime.port_id("temperature_mv").unwrap();
    let default_mv = runtime.read(voltage).unwrap();
    runtime.write(temperature, temperature_c).unwrap();
    runtime.advance_ns(1_000).unwrap();
    let result_mv = runtime.read(voltage).unwrap();
    let batch = runtime.analog_trace_snapshot(0);
    assert_eq!(batch.time_unit, "ns");
    assert_eq!(batch.channels.len(), 1);
    assert_eq!(batch.channels[0].name, "tmp36-sensor.voltage_mv");
    assert_eq!(batch.channels[0].unit, "mV");
    assert_eq!(batch.channels[0].port_names, ["temperature_mv"]);
    assert_eq!(batch.samples.len(), 2);
    serde_json::json!({
        "temperature_c": temperature_c,
        "default_mv": default_mv,
        "result_mv": result_mv,
        "time_ns": runtime.time_ns(),
        "trace": {
            "time_unit": batch.time_unit,
            "channels": batch.channels,
            "dropped": batch.dropped,
            "samples": batch.samples.into_iter().map(|sample| serde_json::json!({
                "cycle": sample.cycle,
                "time_ns": sample.time_ns,
                "values": sample.values,
            })).collect::<Vec<_>>(),
        },
    })
}

#[test]
fn flex_tmp36_reuses_formula_source_with_temperature_units_and_scope_trace() {
    let yaml: serde_yaml::Value = serde_yaml::from_str(TMP36_SYSTEM).unwrap();
    assert_eq!(yaml["ports"][0]["unit"], "°C");
    assert_eq!(yaml["cosim_models"][0]["config"]["device_type"], "tmp36");

    let cold = tmp36_analog_reference(-40.0);
    let zero = tmp36_analog_reference(0.0);
    let default = tmp36_analog_reference(25.0);
    let hot = tmp36_analog_reference(125.0);
    assert_eq!(cold["default_mv"], 750.0);
    assert_eq!(cold["result_mv"], 100.0);
    assert_eq!(zero["result_mv"], 500.0);
    assert_eq!(default["result_mv"], 750.0);
    assert_eq!(hot["result_mv"], 1_750.0);
    assert!(hot["result_mv"].as_f64().unwrap() > default["result_mv"].as_f64().unwrap());
    assert!(cold["result_mv"].as_f64().unwrap() < default["result_mv"].as_f64().unwrap());

    let measured = serde_json::json!({"cold": cold, "zero": zero, "default": default, "hot": hot});
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/flex-analog-tmp36/native.json");
    if std::env::var_os("LABWIRED_REGEN_FLEX_TMP36").is_some() {
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
        "formula-backed analog observations changed"
    );
}

fn gp2y0a21_reference() -> serde_json::Value {
    let mut runtime = FlexRuntime::from_yaml(GP2Y_SYSTEM).expect("load registered Sharp source");
    let distance = runtime.port_id("distance_mm").unwrap();
    let voltage = runtime.port_id("distance_mv").unwrap();
    let drive = runtime.port_id("near_drive").unwrap();
    let led = runtime.port_id("near_led").unwrap();
    let points = [
        (100u32, 2300u16),
        (150, 1650),
        (200, 1300),
        (300, 930),
        (400, 730),
        (500, 600),
        (600, 510),
        (700, 440),
        (800, 400),
    ];
    assert_eq!(runtime.read(voltage).unwrap(), 2300.0);

    let mut observations = Vec::new();
    for (distance_mm, expected_mv) in points {
        runtime.write(distance, f64::from(distance_mm)).unwrap();
        runtime.advance_ns(1_000).unwrap();
        let sensor_mv = runtime.read(voltage).unwrap() as u16;
        assert_eq!(sensor_mv, expected_mv, "distance {distance_mm} mm");
        runtime
            .write(drive, if sensor_mv >= 1_500 { 1.0 } else { 0.0 })
            .unwrap();
        runtime.advance_ns(1_000).unwrap();
        observations.push(serde_json::json!({
            "distance_mm": distance_mm,
            "sensor_mv": sensor_mv,
            "near_led": runtime.read(led).unwrap() != 0.0,
        }));
    }

    let analog = runtime.analog_trace_snapshot(0);
    assert_eq!(analog.time_unit, "ns");
    assert_eq!(analog.dropped, 0);
    assert_eq!(analog.channels.len(), 1);
    assert_eq!(analog.channels[0].name, "gp2y0a21-sensor.voltage_mv");
    assert_eq!(analog.channels[0].unit, "mV");
    assert_eq!(analog.channels[0].port_names, ["distance_mv"]);
    assert_eq!(analog.samples.len(), 19);

    let digital = runtime.digital_trace_snapshot(0);
    assert_eq!(digital.time_unit, "ns");
    assert_eq!(digital.dropped, 0);
    assert!(digital
        .channels
        .iter()
        .any(|channel| channel.port_names == ["near_led"]));
    assert_eq!(
        digital
            .edges
            .iter()
            .map(|edge| (edge.time_ns, edge.value))
            .collect::<Vec<_>>(),
        [(1_000, 1), (2_000, 1), (5_000, 0), (6_000, 0)]
    );
    assert_eq!(runtime.time_ns(), 18_000);

    use sha2::{Digest, Sha256};
    serde_json::json!({
        "observations": observations,
        "time_ns": runtime.time_ns(),
        "analog_trace": {
            "time_unit": analog.time_unit,
            "channels": analog.channels,
            "dropped": analog.dropped,
            "samples": analog.samples.into_iter().map(|sample| serde_json::json!({
                "cycle": sample.cycle,
                "time_ns": sample.time_ns,
                "values": sample.values,
            })).collect::<Vec<_>>(),
        },
        "digital_trace": {
            "time_unit": digital.time_unit,
            "channels": digital.channels,
            "dropped": digital.dropped,
            "edges": digital.edges,
        },
        "run_report": {
            "manifest_sha256": format!("{:x}", Sha256::digest(GP2Y_SYSTEM_BYTES)),
        },
    })
}

#[test]
fn flex_gp2y0a21_reuses_the_corrected_catalog_curve_in_both_instruments() {
    let measured = gp2y0a21_reference();
    assert_eq!(measured["observations"][0]["sensor_mv"], 2300);
    assert_eq!(measured["observations"][1]["sensor_mv"], 1650);
    assert_eq!(measured["observations"][2]["sensor_mv"], 1300);
    assert_eq!(measured["observations"][8]["sensor_mv"], 400);
    let near_channel = measured["digital_trace"]["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["port_names"][0] == "near_led")
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    assert_eq!(
        measured["digital_trace"]["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|edge| edge["ch"].as_u64() == Some(near_channel))
            .map(|edge| (
                edge["time_ns"].as_u64().unwrap(),
                edge["value"].as_u64().unwrap()
            ))
            .collect::<Vec<_>>(),
        [(2_000, 1), (6_000, 0)]
    );

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/flex-analog-gp2y0a21/native.json");
    if std::env::var_os("LABWIRED_REGEN_FLEX_GP2Y0A21").is_some() {
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&measured).unwrap() + "\n",
        )
        .unwrap();
    }
    let expected: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(measured, expected, "Sharp source model reference changed");
}

#[test]
fn flex_gp2y0a21_browser_proof_pins_firmware_and_model_inputs() {
    let browser: serde_json::Value = serde_json::from_str(GP2Y_BROWSER).unwrap();
    let native: serde_json::Value = serde_json::from_str(GP2Y_NATIVE).unwrap();
    let core_commit = browser["core_commit"].as_str().unwrap();
    assert_eq!(core_commit.len(), 40);
    assert!(core_commit.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(browser["native_model_equality"], true);
    assert_eq!(browser["firmware_sha256"], sha256(GP2Y_FIRMWARE));
    assert_eq!(
        browser["firmware_source_sha256"],
        sha256(GP2Y_FIRMWARE_SOURCE)
    );
    assert_eq!(browser["system_sha256"], sha256(GP2Y_SYSTEM_BYTES));
    assert_eq!(
        browser["run_report"]["manifest_sha256"],
        sha256(GP2Y_SYSTEM_BYTES)
    );
    assert_eq!(
        normalize_numbers(&browser["observations"]),
        normalize_numbers(&native["observations"])
    );
    for (browser_trace, native_trace) in [("analog", "analog_trace"), ("digital", "digital_trace")]
    {
        assert_eq!(
            normalize_numbers(&browser["traces"][browser_trace]),
            normalize_numbers(&native[native_trace]),
            "browser {browser_trace} must match the native instrument capture"
        );
    }
}

fn dual_gp2y0a21_reference() -> serde_json::Value {
    use sha2::{Digest, Sha256};

    let mut runtime =
        FlexRuntime::from_yaml(DUAL_GP2Y_SYSTEM).expect("load two instances of the Sharp source");
    let left_distance = runtime.port_id("left_distance_mm").unwrap();
    let left_voltage = runtime.port_id("left_distance_mv").unwrap();
    let left_drive = runtime.port_id("left_near_drive").unwrap();
    let left_led = runtime.port_id("left_near_led").unwrap();
    let right_distance = runtime.port_id("right_distance_mm").unwrap();
    let right_voltage = runtime.port_id("right_distance_mv").unwrap();
    let right_drive = runtime.port_id("right_near_drive").unwrap();
    let right_led = runtime.port_id("right_near_led").unwrap();

    let points = [(100u32, 800u32), (800, 100), (700, 200)];
    let mut observations = Vec::new();
    for (left_mm, right_mm) in points {
        runtime.write(left_distance, f64::from(left_mm)).unwrap();
        runtime.write(right_distance, f64::from(right_mm)).unwrap();
        runtime.advance_ns(1_000).unwrap();
        let left_mv = runtime.read(left_voltage).unwrap() as u16;
        let right_mv = runtime.read(right_voltage).unwrap() as u16;
        runtime
            .write(left_drive, f64::from(left_mv >= 1_500))
            .unwrap();
        runtime
            .write(right_drive, f64::from(right_mv >= 1_500))
            .unwrap();
        runtime.advance_ns(1_000).unwrap();
        observations.push(serde_json::json!({
            "left_distance_mm": left_mm,
            "left_distance_mv": left_mv,
            "left_near_led": runtime.read(left_led).unwrap() != 0.0,
            "right_distance_mm": right_mm,
            "right_distance_mv": right_mv,
            "right_near_led": runtime.read(right_led).unwrap() != 0.0,
        }));
    }

    let analog = runtime.analog_trace_snapshot(0);
    assert_eq!(analog.time_unit, "ns");
    assert_eq!(analog.dropped, 0);
    assert_eq!(analog.channels.len(), 2);
    assert_eq!(analog.channels[0].name, "left-sensor.voltage_mv");
    assert_eq!(analog.channels[0].port_names, ["left_distance_mv"]);
    assert_eq!(analog.channels[1].name, "right-sensor.voltage_mv");
    assert_eq!(analog.channels[1].port_names, ["right_distance_mv"]);
    assert_eq!(analog.samples.len(), 13);
    assert_eq!(analog.samples[0].time_ns, 0);
    assert_eq!(analog.samples[0].values, [2_300.0, 400.0]);

    let digital = runtime.digital_trace_snapshot(0);
    assert_eq!(digital.time_unit, "ns");
    assert_eq!(digital.dropped, 0);
    assert!(digital
        .channels
        .iter()
        .any(|channel| { channel.port_names == ["left_near_led"] }));
    assert!(digital
        .channels
        .iter()
        .any(|channel| { channel.port_names == ["right_near_led"] }));
    for (port, edges) in [
        ("left_near_led", vec![(2_000, 1), (4_000, 0)]),
        ("right_near_led", vec![(4_000, 1), (6_000, 0)]),
    ] {
        let channel_id = digital
            .channels
            .iter()
            .find(|channel| channel.port_names == [port])
            .unwrap()
            .id;
        assert_eq!(
            digital
                .edges
                .iter()
                .filter(|edge| edge.ch == channel_id)
                .map(|edge| (edge.time_ns, edge.value))
                .collect::<Vec<_>>(),
            edges,
            "independent indicator transitions for {port}"
        );
    }
    assert_eq!(runtime.time_ns(), 6_000);

    serde_json::json!({
        "observations": observations,
        "time_ns": runtime.time_ns(),
        "analog_trace": {
            "time_unit": analog.time_unit,
            "channels": analog.channels,
            "dropped": analog.dropped,
            "samples": analog.samples.into_iter().map(|sample| serde_json::json!({
                "cycle": sample.cycle,
                "time_ns": sample.time_ns,
                "values": sample.values,
            })).collect::<Vec<_>>(),
        },
        "digital_trace": {
            "time_unit": digital.time_unit,
            "channels": digital.channels,
            "dropped": digital.dropped,
            "edges": digital.edges,
        },
        "run_report": {
            "manifest_sha256": format!("{:x}", Sha256::digest(DUAL_GP2Y_SYSTEM_BYTES)),
        },
    })
}

#[test]
fn flex_dual_sensor_reuses_one_catalog_model_for_independent_instances() {
    let measured = dual_gp2y0a21_reference();
    assert_eq!(measured["observations"][0]["left_distance_mv"], 2300);
    assert_eq!(measured["observations"][0]["right_distance_mv"], 400);
    assert_eq!(measured["observations"][0]["left_near_led"], true);
    assert_eq!(measured["observations"][0]["right_near_led"], false);
    assert_eq!(measured["observations"][1]["left_distance_mv"], 400);
    assert_eq!(measured["observations"][1]["right_distance_mv"], 2300);
    assert_eq!(measured["observations"][1]["left_near_led"], false);
    assert_eq!(measured["observations"][1]["right_near_led"], true);
    assert_eq!(measured["observations"][2]["left_distance_mv"], 440);
    assert_eq!(measured["observations"][2]["right_distance_mv"], 1300);
    assert_eq!(measured["observations"][2]["left_near_led"], false);
    assert_eq!(measured["observations"][2]["right_near_led"], false);

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/flex-dual-gp2y0a21/native.json");
    if std::env::var_os("LABWIRED_REGEN_FLEX_DUAL_GP2Y0A21").is_some() {
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&measured).unwrap() + "\n",
        )
        .unwrap();
    }
    let expected: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&path).expect("read native dual-sensor evidence"),
    )
    .unwrap();
    assert_eq!(
        measured, expected,
        "independent instances must share one source-backed catalog model"
    );
}

#[test]
fn flex_dual_sensor_browser_proof_pins_firmware_and_initial_trace() {
    let browser: serde_json::Value = serde_json::from_str(DUAL_GP2Y_BROWSER).unwrap();
    let native_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/flex-dual-gp2y0a21/native.json");
    let native: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(native_path).unwrap()).unwrap();
    let core_commit = browser["core_commit"].as_str().unwrap();
    assert_eq!(core_commit.len(), 40);
    assert!(core_commit.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(browser["status"], "passed");
    assert_eq!(browser["browser_driver"], "Playwright");
    assert_eq!(browser["native_model_equality"], true);
    assert_eq!(browser["firmware_sha256"], sha256(DUAL_GP2Y_FIRMWARE));
    assert_eq!(
        browser["firmware_source_sha256"],
        sha256(DUAL_GP2Y_FIRMWARE_SOURCE)
    );
    assert_eq!(browser["system_sha256"], sha256(DUAL_GP2Y_SYSTEM_BYTES));
    assert_eq!(
        normalize_numbers(&browser["observations"]),
        normalize_numbers(&native["observations"])
    );
    assert_eq!(
        normalize_numbers(&browser["traces"]["analog"]),
        normalize_numbers(&native["analog_trace"]),
        "browser scope must retain both real initial outputs at 0 ns"
    );
    assert_eq!(
        browser["traces"]["analog"]["samples"][0]["values"],
        serde_json::json!([2_300, 400]),
        "no unresolved zero-valued channel may leak into the initial browser scope sample"
    );
    assert_eq!(
        normalize_numbers(&browser["traces"]["digital"]),
        normalize_numbers(&native["digital_trace"]),
        "browser logic analyzer transitions must match the native reference"
    );
    assert_eq!(
        browser["run_report"]["manifest_sha256"],
        native["run_report"]["manifest_sha256"]
    );
}

#[test]
fn flex_heypcb_ldr_runtime_ports_match_the_pinned_generic_import() {
    let imported: serde_json::Value = serde_json::from_str(HEYPCB_LDR_IMPORT).unwrap();
    let source_map: serde_json::Value = serde_json::from_str(HEYPCB_LDR_SOURCE_MAP).unwrap();
    let source = &source_map["source"];
    assert_eq!(source["repository"], "HeyPCB/esp32-sensors-1");
    assert_eq!(
        source["revision"],
        "57a2122a19ac1aa9c092e43fb5b123c461508752"
    );
    assert_eq!(source["asset"], "boards/ldr/kicad/esp32_ldr");
    assert_eq!(source["license_spdx"], "CERN-OHL-S-2.0");
    assert_eq!(
        source["title"],
        "HeyPCB ESP32-S3 LDR - GL5528 photoresistor light sensor"
    );
    assert_eq!(source["board_revision"], "A");
    for artifact in ["schematic", "pcb", "license"] {
        assert_eq!(
            imported["source"]["files"][artifact], source["files"][artifact],
            "live MCP import and source map must pin the same {artifact} blob"
        );
    }
    assert_eq!(
        source["files"]["schematic"]["git_blob"],
        "64667760dad1369a130b8935718b7a262d746d66"
    );
    assert_eq!(
        source["files"]["pcb"]["git_blob"],
        "54b05837a3e254b6a424b2ea2eb97a4b8318e578"
    );
    assert_eq!(
        source["files"]["board_json"]["git_blob"],
        "409e43b9a6afe61c082635c30461e433e8d39772"
    );
    assert_eq!(
        source["files"]["license"]["git_blob"],
        "0f1092207caa645ff426e0cc40b7eaad69caecb1"
    );

    let schematic = &imported["generic_import"]["schematic"];
    assert_eq!(schematic["mode"], "twin_ready");
    assert_eq!(schematic["mapped"], 18);
    assert_eq!(schematic["ignored"], 1);
    assert_eq!(schematic["dropped"], 3);
    assert_eq!(schematic["diagram_parts"], 18);
    assert_eq!(schematic["diagram_wire_edges"], 27);
    let mapping = schematic["selected_mapping"].as_array().unwrap();
    let ldr = mapping.iter().find(|item| item["ref"] == "U4").unwrap();
    assert_eq!(ldr["status"], "dropped");
    assert!(ldr["reason"]
        .as_str()
        .unwrap()
        .contains("2-terminal conductance not modeled"));
    assert!(ldr["reason"]
        .as_str()
        .unwrap()
        .contains("not electrically equivalent"));
    let imported_edges = schematic["relevant_import_wires"].as_array().unwrap();
    for edge in [
        serde_json::json!(["C7.1", "U3.GPIO1"]),
        serde_json::json!(["C7.1", "R6.1"]),
        serde_json::json!(["R4.1", "U3.GPIO48"]),
        serde_json::json!(["D1.A", "R4.2"]),
    ] {
        assert!(
            imported_edges.contains(&edge),
            "import snapshot omits {edge}"
        );
    }
    assert!(imported_edges
        .iter()
        .all(|edge| !edge.to_string().contains("U4")));
    assert_eq!(imported["generic_import"]["pcb"]["mapped"], 17);
    assert_eq!(imported["generic_import"]["pcb"]["ignored"], 1);
    assert_eq!(imported["generic_import"]["pcb"]["dropped"], 4);
    let pcb = &imported["generic_import"]["pcb"];
    let pcb_ldr = pcb["selected_mapping"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["ref"] == "U4")
        .unwrap();
    assert_eq!(pcb_ldr["status"], "mapped");
    assert_eq!(pcb_ldr["catalog_type"], "resistor");
    assert!(pcb["import_gap"]
        .as_str()
        .unwrap()
        .contains("not electrically equivalent"));
    assert!(source["original_board_firmware"]
        .as_str()
        .unwrap()
        .contains("none"));

    let board_identity = &source_map["source_board_identity"];
    assert_eq!(board_identity["mcu_ref"], "U3");
    assert_eq!(board_identity["sensor_ref"], "U4");
    assert_eq!(board_identity["user_led_ref"], "D1");
    let source_connections = source_map["source_grounded_behavior_slice"]["source_connections"]
        .as_array()
        .unwrap();
    let sns_net = source_connections
        .iter()
        .find(|connection| connection["net"] == "SNS_AN1")
        .expect("pinned source map's sensor net");
    assert_eq!(
        sns_net["endpoints"],
        serde_json::json!(["U3.GPIO1", "U4.2", "R6.1", "C7.1"])
    );

    let runtime_map = &source_map["runtime_bindings"];
    let yaml: serde_yaml::Value = serde_yaml::from_str(HEYPCB_LDR_SYSTEM).unwrap();
    let ports = yaml["ports"].as_sequence().unwrap();
    for (name, expected_signal, access, kind) in [
        ("lux", "environment.lux", "write", "number"),
        ("sensor_mv", "source.U3.GPIO1", "read", "number"),
        ("user_led_drive", "source.U3.GPIO48", "write", "bool"),
        ("user_led_state", "source.D1.logic_state", "read", "bool"),
    ] {
        let port = ports
            .iter()
            .find(|port| port["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("missing Flex port {name}"));
        assert_eq!(port["signal"].as_str(), Some(expected_signal));
        assert_eq!(port["access"].as_str(), Some(access));
        assert_eq!(port["kind"].as_str(), Some(kind));
    }
    assert_eq!(runtime_map["sensor_model"]["config_device_type"], "ldr");
    assert_eq!(runtime_map["sensor_model"]["output"], "source.U3.GPIO1");
    assert_eq!(runtime_map["led_model"]["input"], "source.U3.GPIO48");
    assert_eq!(runtime_map["led_model"]["output"], "source.D1.logic_state");

    let mut runtime =
        FlexRuntime::from_yaml(HEYPCB_LDR_SYSTEM).expect("load LDR Flex behavior slice");
    let lux = runtime.port_id("lux").unwrap();
    let sensor = runtime.port_id("sensor_mv").unwrap();
    let led_drive = runtime.port_id("user_led_drive").unwrap();
    let led_state = runtime.port_id("user_led_state").unwrap();
    assert!(runtime.read(sensor).is_ok());
    assert!(runtime.write(led_state, 1.0).is_err());
    assert!(runtime.read(led_drive).is_err());
    assert!(runtime.write(lux, 100_001.0).is_ok());
    assert!(
        runtime.advance_ns(1_000).is_err(),
        "the LDR catalog model must enforce its 100000 lx input range"
    );
}

#[test]
fn flex_heypcb_ldr_portable_behavior_has_native_reference_and_dark_negative_control() {
    let mut positive = FlexRuntime::from_yaml(HEYPCB_LDR_SYSTEM).expect("LDR Flex manifest");
    let positive_evidence = heypcb_ldr_reference(&mut positive, 100_000.0);
    let channels = positive_evidence["trace"]["channels"].as_array().unwrap();
    assert!(channels.iter().any(|channel| {
        channel["signal"] == "source.U3.GPIO48"
            && channel["port_names"] == serde_json::json!(["user_led_drive"])
    }));
    assert!(channels.iter().any(|channel| {
        channel["signal"] == "source.D1.logic_state"
            && channel["port_names"] == serde_json::json!(["user_led_state"])
    }));
    assert_eq!(positive_evidence["result"], positive_evidence["adc_code"]);
    assert!(positive_evidence["adc_code"].as_u64().unwrap() >= 4080);
    assert_eq!(positive_evidence["led_final"], 0.0);
    assert_eq!(positive.time_ns(), 3_000);
    assert_eq!(positive_evidence["trace"]["dropped"], 0);
    assert_eq!(
        positive_evidence["trace"]["edges"]
            .as_array()
            .unwrap()
            .len(),
        4
    );

    let mut negative = FlexRuntime::from_yaml(HEYPCB_LDR_SYSTEM).expect("LDR Flex manifest");
    let negative_evidence = heypcb_ldr_reference(&mut negative, 0.0);
    assert_eq!(negative_evidence["result"], 0);
    assert_eq!(negative_evidence["sensor_mv"], 0.0);
    assert_eq!(negative_evidence["led_final"], 0.0);
    assert_eq!(negative.time_ns(), 2_000);
    assert!(negative_evidence["trace"]["edges"]
        .as_array()
        .unwrap()
        .is_empty());

    let measured =
        serde_json::json!({ "positive": positive_evidence, "negative": negative_evidence });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/flex-heypcb-esp32-ldr/native.json");
    if std::env::var_os("LABWIRED_REGEN_FLEX_HEYPCB_LDR").is_some() {
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
        "source-mapped LDR model observations changed"
    );
}

#[test]
fn flex_heypcb_ldr_network_netlist_matches_the_pinned_source_nets() {
    let source_map: serde_json::Value = serde_json::from_str(LDR_NETWORK_SOURCE_MAP).unwrap();
    let slice_map: serde_json::Value = serde_json::from_str(HEYPCB_LDR_SOURCE_MAP).unwrap();
    for key in ["repository", "revision", "asset", "license_spdx"] {
        assert_eq!(source_map["source"][key], slice_map["source"][key], "{key}");
    }
    for file in ["schematic", "pcb", "board_json", "board_readme", "license"] {
        assert_eq!(
            source_map["source"]["files"][file], slice_map["source"]["files"][file],
            "{file} blob must match the existing pinned LDR source map"
        );
    }

    let yaml: serde_yaml::Value = serde_yaml::from_str(LDR_NETWORK_SYSTEM).unwrap();
    let sensor = &yaml["cosim_models"][0];
    assert_eq!(sensor["adapter"], "analog");
    let netlist: Vec<Vec<String>> = sensor["config"]["netlist_text"]
        .as_str()
        .unwrap()
        .lines()
        .map(|line| line.split_whitespace().map(str::to_lowercase).collect())
        .collect();
    let node = |net: &str| match net {
        "GND" => "0".to_owned(),
        other => other.to_lowercase(),
    };
    let elements = source_map["sensor_network"]["elements"].as_array().unwrap();
    assert_eq!(netlist.len(), elements.len());
    for element in elements {
        let name = element["netlist"].as_str().unwrap().to_lowercase();
        let line = netlist
            .iter()
            .find(|line| line[0] == name)
            .unwrap_or_else(|| panic!("netlist has no {name}"));
        let nets: Vec<String> = match element.get("pads") {
            Some(pads) => ["1", "2"]
                .iter()
                .map(|pad| node(pads[*pad].as_str().unwrap()))
                .collect(),
            None => element["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|net| node(net.as_str().unwrap()))
                .collect(),
        };
        assert_eq!(line[1..3], nets[..], "{name} terminals");
    }
    assert_eq!(
        sensor["config"]["photoresistors"]["lux"],
        serde_yaml::to_value(&source_map["photoresistor_data"]["model_parameters"])
            .map(|mut value| {
                value["resistor"] = "RU4".into();
                value
            })
            .unwrap()
    );
    let row = &source_map["photoresistor_data"]["gl5528_row"];
    assert_eq!(row["gamma_100_10"], 0.6);
    assert_eq!(
        row["light_resistance_10lx_kohm"],
        serde_json::json!([10, 20])
    );
    assert_eq!(sensor["outputs"]["voltage"], "source.U3.GPIO1");
}

#[test]
fn flex_heypcb_ldr_network_solves_divider_and_c7_transient_with_dim_negative_control() {
    let bright = ldr_network_reference(LDR_NETWORK_SYSTEM, 100.0);
    let dim = ldr_network_reference(LDR_NETWORK_SYSTEM, 1.0);

    // Operating point at the 10 lx reference: 3.3 V * 10k / (15k + 10k).
    assert!((bright["initial_v"].as_f64().unwrap() - 1.32).abs() < 1e-6);
    let bright_v = bright["sensor_v"].as_f64().unwrap();
    let dim_v = dim["sensor_v"].as_f64().unwrap();
    assert!(
        (bright_v - ldr_network_divider_v(15_000.0, 100.0)).abs() < 1e-3,
        "{bright_v}"
    );
    // 4 ms is about 4.7 time constants at 1 lx; under 1% of the step remains.
    assert!(
        (dim_v - ldr_network_divider_v(15_000.0, 1.0)).abs() < 0.01,
        "{dim_v}"
    );
    assert_eq!(bright["result"], bright["adc_code"]);
    assert!(bright["adc_code"].as_u64().unwrap() >= 2048);
    assert_eq!(bright["digital"]["edges"].as_array().unwrap().len(), 4);
    assert_eq!(bright["time_ns"], 4_002_000);
    assert_eq!(dim["result"], dim["adc_code"]);
    assert!(dim["adc_code"].as_u64().unwrap() < 2048);
    assert!(dim["digital"]["edges"].as_array().unwrap().is_empty());
    assert_eq!(dim["time_ns"], 4_001_000);

    // C7 charges through R6 ∥ U4. At 100 lx that is 100 nF * 2.737 kΩ =
    // 274 µs; lux written at t=0 is latched at the first 50 µs boundary.
    let samples = bright["analog"]["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 81);
    assert_eq!(bright["analog"]["dropped"], 0);
    let threshold = 1.32 + 0.632 * (ldr_network_divider_v(15_000.0, 100.0) - 1.32);
    let crossing = samples
        .iter()
        .find(|sample| sample["values"][0].as_f64().unwrap() >= threshold)
        .map(|sample| sample["time_ns"].as_u64().unwrap())
        .unwrap();
    assert!(
        (300_000..=400_000).contains(&crossing),
        "63.2% crossing at {crossing} ns"
    );
    let volts: Vec<f64> = samples
        .iter()
        .map(|sample| sample["values"][0].as_f64().unwrap())
        .collect();
    assert!(volts.windows(2).all(|pair| pair[0] <= pair[1] + 1e-6));

    let measured = serde_json::json!({ "bright": bright, "dim": dim });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/flex-heypcb-ldr-network/native.json");
    if std::env::var_os("LABWIRED_REGEN_FLEX_LDR_NETWORK").is_some() {
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&measured).unwrap() + "\n",
        )
        .unwrap();
    }
    // Compared as text: the volts are not integers, and a parse round trip
    // may move the last bit.
    assert_eq!(
        serde_json::to_string_pretty(&measured).unwrap() + "\n",
        std::fs::read_to_string(path).unwrap(),
        "LDR sensor-network observations changed"
    );
}

#[test]
fn flex_heypcb_ldr_network_led_decision_holds_at_both_light_resistance_corners() {
    for (netlist_value, r10_ohms) in [("10k", 10_000.0), ("20k", 20_000.0)] {
        let yaml = LDR_NETWORK_SYSTEM
            .replace(
                "RU4 3v3 sns_an1 15k",
                &format!("RU4 3v3 sns_an1 {netlist_value}"),
            )
            .replace("r_ref_ohms: 15000", &format!("r_ref_ohms: {r10_ohms}"));
        assert_ne!(yaml, LDR_NETWORK_SYSTEM);
        let bright = ldr_network_reference(&yaml, 100.0);
        let dim = ldr_network_reference(&yaml, 1.0);
        assert!(
            (bright["sensor_v"].as_f64().unwrap() - ldr_network_divider_v(r10_ohms, 100.0)).abs()
                < 1e-3
        );
        assert!(
            (dim["sensor_v"].as_f64().unwrap() - ldr_network_divider_v(r10_ohms, 1.0)).abs() < 0.02
        );
        assert!(
            bright["adc_code"].as_u64().unwrap() >= 2048,
            "{netlist_value}"
        );
        assert_eq!(bright["digital"]["edges"].as_array().unwrap().len(), 4);
        assert!(dim["adc_code"].as_u64().unwrap() < 2048, "{netlist_value}");
        assert!(dim["digital"]["edges"].as_array().unwrap().is_empty());
    }
}

#[test]
fn flex_heypcb_ldr_network_refuses_illuminance_outside_the_data_sheet_range() {
    for lux in [0.0, 0.5, 100_000.0] {
        let mut runtime = FlexRuntime::from_yaml(LDR_NETWORK_SYSTEM).unwrap();
        let lux_port = runtime.port_id("lux").unwrap();
        let sensor_port = runtime.port_id("sensor_v").unwrap();
        let before = runtime.read(sensor_port).unwrap();
        let error = match runtime.write(lux_port, lux) {
            Err(error) => error,
            Ok(()) => runtime.advance_ns(50_000).unwrap_err(),
        };
        assert!(
            error
                .to_string()
                .contains("photoresistor illuminance must be within 1..100 lx"),
            "{lux} lx: {error}"
        );
        // No invented reading: the stopped run answers neither a read nor a
        // further advance, and simulated time has not moved.
        assert!((before - 1.32).abs() < 1e-6);
        assert!(runtime
            .read(sensor_port)
            .unwrap_err()
            .to_string()
            .contains("Flex runtime stopped"));
        assert!(runtime
            .advance_ns(50_000)
            .unwrap_err()
            .to_string()
            .contains("Flex runtime stopped"));
        assert_eq!(runtime.time_ns(), 0);
    }
}

#[test]
fn flex_heypcb_ldr_network_browser_evidence_pins_source_and_exact_runtime_inputs() {
    let browser: serde_json::Value = serde_json::from_str(LDR_NETWORK_BROWSER).unwrap();
    let native: serde_json::Value = serde_json::from_str(LDR_NETWORK_NATIVE).unwrap();
    let source_map: serde_json::Value = serde_json::from_str(LDR_NETWORK_SOURCE_MAP).unwrap();
    let core_commit = browser["core_commit"].as_str().unwrap();
    assert_eq!(core_commit.len(), 40);
    assert!(core_commit.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(browser["status"], "passed");
    assert_eq!(browser["cpu_emulation"], false);
    assert_eq!(browser["native_model_equality"], true);
    assert_eq!(browser["firmware_sha256"], sha256(LDR_NETWORK_FIRMWARE));
    assert_eq!(
        browser["firmware_source_sha256"],
        sha256(LDR_NETWORK_FIRMWARE_SOURCE)
    );
    assert_eq!(
        browser["system_sha256"],
        sha256(LDR_NETWORK_SYSTEM.as_bytes())
    );
    assert_eq!(
        browser["run_report"]["manifest_sha256"],
        browser["system_sha256"]
    );
    assert_eq!(
        browser["source_map_sha256"],
        sha256(LDR_NETWORK_SOURCE_MAP.as_bytes())
    );
    for key in ["repository", "revision", "asset", "license_spdx", "files"] {
        assert_eq!(browser["source"][key], source_map["source"][key], "{key}");
    }
    for case in ["bright", "dim"] {
        assert_eq!(
            normalize_numbers(&browser["observations"][case]),
            normalize_numbers(&native[case]),
            "browser evidence for {case} must match the native instrument traces"
        );
    }
    assert_eq!(browser["c7_transient_63pct_crossing_ns"], 350_000);
    let refusals = browser["out_of_range_refusals"].as_array().unwrap();
    assert_eq!(refusals.len(), 2);
    for refusal in refusals {
        assert!(refusal["error"]
            .as_str()
            .unwrap()
            .contains("photoresistor illuminance must be within 1..100 lx"));
        assert!(refusal["read_after"]
            .as_str()
            .unwrap()
            .contains("Flex runtime stopped"));
        assert_eq!(refusal["time_ns"], 0);
        assert_eq!(refusal["led_edges"], 0);
    }
    let scope = &browser["run_report"]["execution_scope"];
    assert_eq!(scope["original_target_firmware_executed"], false);
    assert_eq!(scope["full_board_coverage"], "not_established");
    assert_eq!(scope["chip_parity"], "not_established");
}

#[test]
fn flex_heypcb_ldr_trace_snapshots_report_history_lost_to_late_consumers() {
    let mut runtime =
        FlexRuntime::from_yaml(HEYPCB_LDR_SYSTEM).expect("load pinned LDR behavior slice");
    let drive = runtime.port_id("user_led_drive").unwrap();

    // The consumer intentionally waits until both bounded rings overflow.
    // This checks the existing named logic and scope traces expose their loss
    // rather than making a truncated history look quiet or complete.
    for step in 0..32_769 {
        runtime
            .write(drive, if step % 2 == 0 { 1.0 } else { 0.0 })
            .unwrap();
        runtime.advance_ns(1_000).unwrap();
    }

    let digital = runtime.digital_trace_snapshot(0);
    assert!(
        digital.dropped > 0,
        "late logic-analyzer read hid lost edges"
    );
    assert!(
        !digital.edges.is_empty(),
        "late read lost the retained edge window"
    );
    let digital_tail = runtime.digital_trace_snapshot(digital.cursor);
    assert!(
        digital_tail.edges.is_empty(),
        "digital cursor replayed old edges"
    );
    assert_eq!(digital_tail.dropped, digital.dropped);

    let analog = runtime.analog_trace_snapshot(0);
    assert!(
        analog.dropped > 0,
        "late oscilloscope read hid lost samples"
    );
    assert!(
        !analog.samples.is_empty(),
        "late read lost the retained sample window"
    );
    let analog_tail = runtime.analog_trace_snapshot(analog.next_cursor);
    assert!(
        analog_tail.samples.is_empty(),
        "analog cursor replayed old samples"
    );
    assert_eq!(analog_tail.dropped, analog.dropped);
}

#[test]
fn flex_heypcb_ldr_browser_evidence_pins_source_and_exact_runtime_inputs() {
    let browser: serde_json::Value = serde_json::from_str(HEYPCB_LDR_BROWSER).unwrap();
    let source_map: serde_json::Value = serde_json::from_str(HEYPCB_LDR_SOURCE_MAP).unwrap();

    let core_commit = browser["core_commit"].as_str().unwrap();
    assert_eq!(core_commit.len(), 40);
    assert!(core_commit.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(browser["native_model_equality"], true);
    assert!(browser["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| check == "named-digital-port-aliases"));
    assert!(browser["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| check == "trace-overflow-reporting"));
    assert!(
        browser["trace_overflow"]["digital"]["dropped"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        browser["trace_overflow"]["digital"]["retained_edges"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        browser["trace_overflow"]["analog"]["dropped"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        browser["trace_overflow"]["analog"]["retained_samples"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(
        browser["digital_port_aliases"],
        serde_json::json!([
            {"signal": "source.D1.logic_state", "port_names": ["user_led_state"]},
            {"signal": "source.U3.GPIO48", "port_names": ["user_led_drive"]}
        ])
    );
    assert_eq!(browser["firmware_sha256"], sha256(HEYPCB_LDR_FIRMWARE));
    assert_eq!(
        browser["firmware_source_sha256"],
        sha256(HEYPCB_LDR_FIRMWARE_SOURCE)
    );
    assert_eq!(browser["system_sha256"], sha256(HEYPCB_LDR_SYSTEM_BYTES));
    assert_eq!(
        browser["run_report"]["manifest_sha256"],
        browser["system_sha256"]
    );
    assert_eq!(
        browser["source_map_sha256"],
        sha256(HEYPCB_LDR_SOURCE_MAP_BYTES)
    );
    assert_eq!(
        browser["source"]["repository"],
        source_map["source"]["repository"]
    );
    assert_eq!(
        browser["source"]["revision"],
        source_map["source"]["revision"]
    );
    assert_eq!(browser["source"]["asset"], source_map["source"]["asset"]);
    assert_eq!(
        browser["source"]["license_spdx"],
        source_map["source"]["license_spdx"]
    );
    assert_eq!(browser["source"]["files"], source_map["source"]["files"]);
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
    let second_led = runtime.port_id("led_state_2").ok();
    let outputs_are = |runtime: &FlexRuntime, value: f64| {
        runtime.read(led).unwrap() == value
            && match second_led {
                Some(port) => runtime.read(port).unwrap() == value,
                None => true,
            }
    };
    if !positive {
        runtime.write(drive, 0.0).unwrap();
        runtime.advance_ns(5_000).unwrap();
        return if outputs_are(runtime, 0.0) { 0 } else { 2 };
    }

    runtime.write(drive, 1.0).unwrap();
    runtime.advance_ns(1_000).unwrap();
    if !outputs_are(runtime, 1.0) {
        return 2;
    }
    runtime.write(drive, 0.0).unwrap();
    runtime.advance_ns(1_000).unwrap();
    if !outputs_are(runtime, 0.0) {
        return 2;
    }
    runtime.write(drive, 1.0).unwrap();
    runtime.advance_ns(2_000).unwrap();
    if !outputs_are(runtime, 1.0) {
        return 2;
    }
    runtime.write(drive, 0.0).unwrap();
    runtime.advance_ns(1_000).unwrap();
    if !outputs_are(runtime, 0.0) {
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

fn ntc_analog_alarm_reference(temperature_c: f64) -> serde_json::Value {
    let mut runtime = FlexRuntime::from_yaml(NTC_SYSTEM).expect("load registered NTC source");
    let temperature = runtime.port_id("temperature_c").unwrap();
    let voltage = runtime.port_id("temperature_mv").unwrap();
    let alarm_drive = runtime.port_id("hot_alarm_drive").unwrap();
    let alarm_state = runtime.port_id("hot_alarm_state").unwrap();
    runtime.write(temperature, temperature_c).unwrap();
    runtime.advance_ns(1_000).unwrap();
    let voltage_mv = runtime.read(voltage).unwrap();
    runtime
        .write(alarm_drive, if voltage_mv >= 2_000.0 { 1.0 } else { 0.0 })
        .unwrap();
    runtime.advance_ns(1_000).unwrap();
    let analog = runtime.analog_trace_snapshot(0);
    let digital = runtime.digital_trace_snapshot(0);
    assert_eq!(analog.time_unit, "ns");
    assert_eq!(analog.channels.len(), 1);
    assert_eq!(analog.channels[0].name, "ntc-sensor.voltage_mv");
    assert_eq!(analog.channels[0].unit, "mV");
    assert_eq!(analog.channels[0].port_names, ["temperature_mv"]);
    assert_eq!(analog.samples.len(), 3);
    assert_eq!(digital.time_unit, "ns");
    serde_json::json!({
        "temperature_c": temperature_c,
        "voltage_mv": voltage_mv,
        "alarm_state": runtime.read(alarm_state).unwrap() == 1.0,
        "time_ns": runtime.time_ns(),
        "analog": {
            "time_unit": analog.time_unit,
            "channels": analog.channels,
            "dropped": analog.dropped,
            "samples": analog.samples.into_iter().map(|sample| serde_json::json!({
                "cycle": sample.cycle,
                "time_ns": sample.time_ns,
                "values": sample.values,
            })).collect::<Vec<_>>(),
        },
        "digital": {
            "time_unit": digital.time_unit,
            "channels": digital.channels,
            "dropped": digital.dropped,
            "edges": digital.edges.into_iter().map(|edge| serde_json::json!({
                "ch": edge.ch,
                "time_ns": edge.time_ns,
                "value": edge.value,
            })).collect::<Vec<_>>(),
        },
    })
}

#[test]
fn flex_ntc_reuses_registered_formula_for_a_portable_overtemperature_workload() {
    let yaml: serde_yaml::Value = serde_yaml::from_str(NTC_SYSTEM).unwrap();
    assert_eq!(
        yaml["cosim_models"][0]["config"]["device_type"],
        "ntc-thermistor"
    );
    assert_eq!(yaml["ports"][0]["unit"], "°C");
    assert_eq!(yaml["ports"][1]["unit"], "mV");

    let cold = ntc_analog_alarm_reference(0.0);
    let ambient = ntc_analog_alarm_reference(25.0);
    let hot = ntc_analog_alarm_reference(50.0);
    assert!(cold["voltage_mv"].as_f64().unwrap() < 2_000.0);
    assert_eq!(ambient["voltage_mv"], 1_650.0);
    assert!(hot["voltage_mv"].as_f64().unwrap() > 2_000.0);
    assert_eq!(cold["alarm_state"], false);
    assert_eq!(ambient["alarm_state"], false);
    assert_eq!(hot["alarm_state"], true);
    assert_eq!(hot["digital"]["edges"].as_array().unwrap().len(), 2);
    assert_eq!(hot["digital"]["edges"][0]["time_ns"], 1_000);
    assert_eq!(hot["digital"]["edges"][1]["time_ns"], 2_000);

    let measured = serde_json::json!({ "cold": cold, "ambient": ambient, "hot": hot });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/flex-analog-ntc/native.json");
    if std::env::var_os("LABWIRED_REGEN_FLEX_NTC").is_some() {
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&measured).unwrap() + "\n",
        )
        .unwrap();
    }
    let expected: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(measured, expected, "registered NTC workload changed");
}

#[test]
fn flex_gpio_reuse_binds_shared_behavior_to_pomodoro_and_uno_with_held_out_workload() {
    let pomodoro: serde_yaml::Value = serde_yaml::from_str(GPIO_REUSE_POMODORO).unwrap();
    let uno: serde_yaml::Value = serde_yaml::from_str(GPIO_REUSE_UNO).unwrap();
    assert_eq!(
        pomodoro["cosim_models"][0]["config"]["behavior"],
        uno["cosim_models"][0]["config"]["behavior"],
        "both hardware descriptions must use the same reusable behavior block"
    );
    assert_eq!(
        pomodoro["cosim_models"][1]["config"]["behavior"],
        uno["cosim_models"][0]["config"]["behavior"],
        "the second Pomodoro indicator must use the same reusable behavior block"
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
        pomodoro["cosim_models"][1]["outputs"]["led"].as_str(),
        Some("source.D2.logic_state")
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
        12
    );
    for output in ["source.D1.logic_state", "source.D2.logic_state"] {
        assert_eq!(
            measured["pomodoro"]["positive"]["trace"]["edges"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|edge| edge["signal"] == output)
                .count(),
            4,
            "both source-mapped indicators must follow the held-out pulse workload"
        );
    }
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

const POMODORO_C_SCRIPT: &str =
    include_str!("../tests/fixtures/flex-heypcb-pomodoro-c/test-untouched-stays-idle.yaml");
const POMODORO_C_SYSTEM: &str =
    include_str!("../tests/fixtures/flex-heypcb-pomodoro-c/system.yaml");
const POMODORO_C_FIRMWARE: &[u8] =
    include_bytes!("../tests/fixtures/flex-heypcb-pomodoro-c/firmware.wasm");

#[test]
fn flex_script_binding_suspends_an_endless_main_and_hands_over_the_instruments() {
    let mut run = crate::flex::run_test_script(
        "test-untouched-stays-idle.yaml",
        POMODORO_C_SCRIPT,
        POMODORO_C_SYSTEM,
        POMODORO_C_FIRMWARE,
    )
    .unwrap();
    let result = serde_json::to_value(&run.result).unwrap();
    assert_eq!(result["status"], "pass", "{result:#}");
    assert_eq!(result["stop_reason"], "completed");
    assert_eq!(result["time_ns"], 300_000_000_u64);
    // The names are the ones the script writes; the browser has no paths.
    assert_eq!(result["config"]["script"], "test-untouched-stays-idle.yaml");
    assert_eq!(result["config"]["flex"], "system.yaml");
    assert_eq!(result["config"]["firmware"], "firmware.wasm");
    // `main` never returns: the call and both resumes end suspended.
    let steps = result["steps"].as_array().unwrap();
    let suspended: Vec<usize> = (0..steps.len())
        .filter(|index| steps[*index]["suspended"] == true)
        .collect();
    assert_eq!(suspended, [15, 16, 17]);
    assert!(result["entry_result"].is_null());
    assert_eq!(result["assertions"].as_array().unwrap().len(), 10);
    // The runtime the binding hands to the instruments holds the capture.
    let runtime = run.runtime.as_mut().unwrap();
    assert_eq!(runtime.time_ns(), 300_000_000);
    let digital = runtime.digital_trace_snapshot(0);
    assert_eq!(digital.dropped, 0);
    assert_eq!(
        digital.edges.len() as u64,
        result["traces"]["digital_edges"].as_u64().unwrap()
    );
    assert!(digital.edges.len() >= 474, "the start-up chirp is captured");
}

#[test]
fn flex_script_binding_reports_bad_scripts_and_limits_without_a_pass() {
    let run = |script: &str| {
        crate::flex::run_test_script(
            "edited.yaml",
            script,
            POMODORO_C_SYSTEM,
            POMODORO_C_FIRMWARE,
        )
    };
    // A script that does not parse or validate never reaches the runner.
    let error = run("schema_version: \"1.0\"\ninputs: { flex: system.yaml }\n")
        .err()
        .unwrap();
    assert!(
        error.starts_with("Failed to parse Flex Test Script YAML"),
        "{error}"
    );
    let error = run(&POMODORO_C_SCRIPT.replace("schema_version: \"1.0\"", "schema_version: \"9\""))
        .err()
        .unwrap();
    assert!(error.contains("Unsupported schema_version"), "{error}");

    // An unknown port is a configuration error: no step runs, no runtime.
    let unknown =
        run(&POMODORO_C_SCRIPT.replace("port: leaf, count: 0", "port: lead, count: 0")).unwrap();
    assert_eq!(unknown.result.status, "error");
    let result = serde_json::to_value(&unknown.result).unwrap();
    assert_eq!(result["stop_reason"], "config_error");
    assert!(result["message"].as_str().unwrap().contains("'lead'"));
    assert!(result["steps"].as_array().unwrap().is_empty());
    assert!(unknown.runtime.is_none());

    // A fuel limit below the start-up chirp stops the run; it cannot pass.
    let starved =
        run(&POMODORO_C_SCRIPT.replace("max_steps: 200000000", "max_steps: 100000")).unwrap();
    let result = serde_json::to_value(&starved.result).unwrap();
    assert_eq!(result["stop_reason"], "max_steps");
    assert_eq!(result["status"], "fail");
    // wasmi charges fuel for each block, so the stop is at or just below the limit.
    let used = result["steps_executed"].as_u64().unwrap();
    assert!((99_000..=100_000).contains(&used), "{used}");

    // With no `until_ns` the endless `main` runs into the time limit.
    let endless = run(&POMODORO_C_SCRIPT
        .replace("__main_void, until_ns: 200000000", "__main_void")
        .replace("  - resume: { until_ns: 250000000 }\n", "")
        .replace("  - resume: { until_ns: 300000000 }\n", ""))
    .unwrap();
    let result = serde_json::to_value(&endless.result).unwrap();
    assert_eq!(result["stop_reason"], "max_time_ns", "{result:#}");
    assert_eq!(result["status"], "fail");
    assert_eq!(result["time_ns"], 299_996_000_u64);
}

#[test]
fn flex_script_binding_refuses_ngspice_without_a_callback() {
    // `WasmFlexTestRun::run` has no solver callback: an `adapter: ngspice`
    // model is a configuration error, as in `WasmFlexRuntime::new_from_yaml`,
    // and never reaches the native factory or a netlist path.
    let run = crate::flex::run_test_script(
        "ngspice-enable.yaml",
        include_str!("../tests/fixtures/flex-mixed-signal/ngspice-enable.yaml"),
        include_str!("../tests/fixtures/flex-mixed-signal/system-ngspice.yaml"),
        include_bytes!("../tests/fixtures/flex-mixed-signal/firmware.wasm"),
    )
    .unwrap();
    let result = serde_json::to_value(&run.result).unwrap();
    assert_eq!(result["status"], "error", "{result:#}");
    assert_eq!(result["stop_reason"], "config_error");
    let message = result["message"].as_str().unwrap();
    assert!(
        message.contains("co-sim model rc needs an ngspice callback"),
        "{message}"
    );
    assert!(run.runtime.is_none());
}

/// One pinned-source contract for every heypcb sensor fixture. Each board is a
/// row: the common checks (same pinned repository revision and LICENSE blob as
/// the LDR fixture, the board's own asset path and five pinned file blobs, a
/// runtime that loads with complete scope notes, no original-target firmware,
/// and every pinned port resolving) run for all rows; `check` carries the
/// assertions specific to the board (strapping, bus address, net names, model
/// binding).
struct HeypcbFixture {
    /// Directory under `boards/` in the pinned repository.
    board: &'static str,
    /// Pinned `source.asset`; `None` where the fixture never pinned it.
    asset: Option<&'static str>,
    source_map: &'static str,
    system: &'static str,
    check: fn(&serde_json::Value, &serde_yaml::Value, &labwired_core::flex::FlexRunReport),
}

/// The BME280 fixture's bus is the pinned board's bus: same source pin as the
/// LDR fixture of the same repository, U4 strapped for I2C at the address the
/// manifest gives the catalog model, and firmware built from the committed
/// source.
fn check_heypcb_bme280(
    source_map: &serde_json::Value,
    yaml: &serde_yaml::Value,
    report: &labwired_core::flex::FlexRunReport,
) {
    // U4: the datasheet's I2C straps, on the bus nets of the board.
    let u4 = &source_map["sensor_bus"]["elements"][0];
    assert_eq!(u4["source_ref"], "U4");
    assert_eq!(u4["pads"]["2"], "3V3", "CSB high selects I2C");
    assert_eq!(u4["pads"]["5"], "GND", "SDO low selects 0x76");
    assert_eq!(u4["pads"]["3"], "I2C_SDA");
    assert_eq!(u4["pads"]["4"], "I2C_SCL");

    let bus = &yaml["i2c_buses"][0];
    let binding = &source_map["runtime_bindings"];
    assert_eq!(bus["name"], binding["bus"]["name"].as_str().unwrap());
    assert_eq!(
        bus["frequency_hz"],
        binding["bus"]["frequency_hz"].as_u64().unwrap()
    );
    let device = &bus["devices"][0];
    assert_eq!(device["id"], binding["device"]["id"].as_str().unwrap());
    assert_eq!(device["type"], binding["device"]["type"].as_str().unwrap());
    assert_eq!(device["config"]["i2c_address"], 0x76);
    assert_eq!(binding["device"]["address"], "0x76");

    assert_eq!(report.i2c_buses.len(), 1);
    assert_eq!(report.i2c_buses[0].devices[0].address, 0x76);
}

/// The SHT40 fixture is a second board of the same pinned repository: same
/// revision and license, U4 on the bus nets of the pinned PCB, and the
/// address of the order code in the pinned BOM given to the catalog model.
fn check_heypcb_sht40(
    source_map: &serde_json::Value,
    yaml: &serde_yaml::Value,
    report: &labwired_core::flex::FlexRunReport,
) {
    let u4 = &source_map["sensor_bus"]["elements"][0];
    assert_eq!(u4["source_ref"], "U4");
    assert_eq!(u4["source_part"], "SHT40-AD1B-R2", "-AD1B is address 0x44");
    assert_eq!(u4["pads"]["1"], "I2C_SDA");
    assert_eq!(u4["pads"]["2"], "I2C_SCL");

    let bus = &yaml["i2c_buses"][0];
    let binding = &source_map["runtime_bindings"];
    assert_eq!(bus["name"], binding["bus"]["name"].as_str().unwrap());
    let device = &bus["devices"][0];
    assert_eq!(device["id"], binding["device"]["id"].as_str().unwrap());
    assert_eq!(device["type"], "sht40");
    assert_eq!(device["config"]["i2c_address"], 0x44);

    assert_eq!(report.i2c_buses[0].devices[0].address, 0x44);
}

/// The TMP117 fixture is a third I2C board of the same pinned repository,
/// with no Flex code of its own: U4 on the bus nets of the pinned PCB, ADD0
/// on GND for 0x48, and the ALERT net reported as unsupported.
fn check_heypcb_tmp117(
    source_map: &serde_json::Value,
    yaml: &serde_yaml::Value,
    report: &labwired_core::flex::FlexRunReport,
) {
    let u4 = &source_map["sensor_bus"]["elements"][0];
    assert_eq!(u4["source_ref"], "U4");
    assert_eq!(u4["source_part"], "TMP117AIDRVR");
    assert_eq!(u4["pads"]["1"], "I2C_SCL");
    assert_eq!(u4["pads"]["6"], "I2C_SDA");
    assert_eq!(u4["pads"]["4"], "GND", "ADD0 on GND is address 0x48");
    assert_eq!(u4["pads"]["3"], "SNS_INT");

    let bus = &yaml["i2c_buses"][0];
    let binding = &source_map["runtime_bindings"];
    assert_eq!(bus["name"], binding["bus"]["name"].as_str().unwrap());
    let device = &bus["devices"][0];
    assert_eq!(device["id"], binding["device"]["id"].as_str().unwrap());
    assert_eq!(device["type"], "tmp117");
    assert_eq!(device["config"]["i2c_address"], 0x48);

    assert_eq!(report.i2c_buses[0].devices[0].address, 0x48);
    // The model has no alert comparator; the manifest must say so.
    assert!(yaml["unsupported_checks"]
        .as_sequence()
        .unwrap()
        .iter()
        .any(|check| check.as_str().unwrap().contains("SNS_INT")));
}

/// The MCP9808 fixture is an I2C board of the same pinned repository, with
/// no Flex code of its own: U4 on the bus nets of the pinned PCB, A0-A2 on
/// GND for 0x18, the placement noise set to 0 and stated as an assumption,
/// and the ALERT net reported as unsupported.
fn check_heypcb_mcp9808(
    source_map: &serde_json::Value,
    yaml: &serde_yaml::Value,
    report: &labwired_core::flex::FlexRunReport,
) {
    let u4 = &source_map["sensor_bus"]["elements"][0];
    assert_eq!(u4["source_ref"], "U4");
    assert_eq!(u4["source_part"], "MCP9808T-E/MS");
    assert_eq!(u4["pads"]["1"], "I2C_SDA");
    assert_eq!(u4["pads"]["2"], "I2C_SCL");
    assert_eq!(u4["pads"]["3"], "SNS_INT");
    for pad in ["5", "6", "7"] {
        assert_eq!(u4["pads"][pad], "GND", "A0-A2 on GND is address 0x18");
    }

    let bus = &yaml["i2c_buses"][0];
    let binding = &source_map["runtime_bindings"];
    assert_eq!(bus["name"], binding["bus"]["name"].as_str().unwrap());
    let device = &bus["devices"][0];
    assert_eq!(device["id"], binding["device"]["id"].as_str().unwrap());
    assert_eq!(device["type"], "mcp9808");
    assert_eq!(device["config"]["i2c_address"], 0x18);
    assert_eq!(device["config"]["noise_sigma"], 0);

    assert_eq!(report.i2c_buses[0].devices[0].address, 0x18);
    let listed = |key: &str, needle: &str| {
        yaml[key]
            .as_sequence()
            .unwrap()
            .iter()
            .any(|entry| entry.as_str().unwrap().contains(needle))
    };
    assert!(listed("assumptions", "`noise_sigma` to 0"));
    assert!(listed("unsupported_checks", "SNS_INT"));
}

/// The SCD41 fixture is a fourth I2C board of the same pinned repository,
/// with no Flex code of its own: U4 on the bus nets of the pinned PCB at its
/// fixed address, and the always-ready data status reported as unsupported.
fn check_heypcb_scd41(
    source_map: &serde_json::Value,
    yaml: &serde_yaml::Value,
    report: &labwired_core::flex::FlexRunReport,
) {
    let u4 = &source_map["sensor_bus"]["elements"][0];
    assert_eq!(u4["source_ref"], "U4");
    assert_eq!(u4["source_part"], "SCD41-D-R2");
    assert_eq!(u4["pads"]["8"], "I2C_SCL");
    assert_eq!(u4["pads"]["10"], "I2C_SDA");

    let bus = &yaml["i2c_buses"][0];
    let binding = &source_map["runtime_bindings"];
    assert_eq!(bus["name"], binding["bus"]["name"].as_str().unwrap());
    let device = &bus["devices"][0];
    assert_eq!(device["id"], binding["device"]["id"].as_str().unwrap());
    assert_eq!(device["type"], "scd41");
    assert_eq!(device["config"]["i2c_address"], 0x62);

    assert_eq!(report.i2c_buses[0].devices[0].address, 0x62);
    // The model reports data ready at all times; the manifest must say so.
    assert!(yaml["unsupported_checks"]
        .as_sequence()
        .unwrap()
        .iter()
        .any(|check| check.as_str().unwrap().contains("5 s periodic")));
}

/// The DS18B20 fixture is a 1-Wire board of the same pinned repository:
/// U4 is the catalog `ds18b20` model on the `onewire_device` primitive, its
/// DQ role on the SNS_IO1 net of the pinned PCB, with the R6 pull-up.
#[test]
fn flex_heypcb_ds18b20_line_matches_the_pinned_source_nets() {
    const SOURCE_MAP: &str =
        include_str!("../tests/fixtures/flex-heypcb-esp32-ds18b20/source-map.json");
    const SYSTEM: &str = include_str!("../tests/fixtures/flex-heypcb-esp32-ds18b20/system.yaml");

    let source_map: serde_json::Value = serde_json::from_str(SOURCE_MAP).unwrap();
    let sibling: serde_json::Value = serde_json::from_str(HEYPCB_LDR_SOURCE_MAP).unwrap();
    for key in ["repository", "revision", "license_spdx"] {
        assert_eq!(source_map["source"][key], sibling["source"][key], "{key}");
    }
    assert_eq!(
        source_map["source"]["files"]["license"],
        sibling["source"]["files"]["license"]
    );
    assert_eq!(
        source_map["source"]["asset"],
        "boards/ds18b20/kicad/esp32_ds18b20"
    );
    for file in ["schematic", "pcb", "bom", "board_json", "board_readme"] {
        let entry = &source_map["source"]["files"][file];
        assert!(entry["path"]
            .as_str()
            .unwrap()
            .starts_with("boards/ds18b20/"));
        assert_eq!(entry["git_blob"].as_str().unwrap().len(), 40, "{file}");
    }

    let elements = &source_map["sensor_line"]["elements"];
    assert_eq!(elements[0]["source_ref"], "U4");
    assert_eq!(elements[0]["pads"]["2"], "SNS_IO1");
    assert_eq!(elements[0]["pads"]["3"], "3V3");
    assert_eq!(elements[1]["source_ref"], "R6");
    assert_eq!(elements[1]["pads"]["2"], "SNS_IO1");
    assert_eq!(
        source_map["sensor_line"]["controller"]["endpoints"]["SNS_IO1"],
        "U3 pad 4 (IO4)"
    );

    let yaml: serde_yaml::Value = serde_yaml::from_str(SYSTEM).unwrap();
    let model = &yaml["cosim_models"][0];
    let binding = &source_map["runtime_bindings"];
    assert_eq!(model["id"], binding["model"]["id"].as_str().unwrap());
    assert_eq!(model["config"]["device_type"], "ds18b20");
    assert_eq!(model["inputs"]["DQ"], "source.U3.GPIO4");
    assert_eq!(model["outputs"]["DQ"], "source.SNS_IO1");

    let runtime = FlexRuntime::from_yaml(SYSTEM).unwrap();
    let report = runtime.run_report();
    assert!(report.scope_notes_complete);
    assert!(!report.execution_scope.original_target_firmware_executed);
    assert_eq!(report.execution_scope.chip_parity, "not_established");
    // The catalog model has no alarm search; the manifest must say so.
    assert!(yaml["unsupported_checks"]
        .as_sequence()
        .unwrap()
        .iter()
        .any(|check| check.as_str().unwrap().contains("ALARM SEARCH")));
    for port in binding["ports"].as_object().unwrap().keys() {
        runtime.port_id(port).unwrap();
    }
    assert_eq!(
        report.ports.len(),
        binding["ports"].as_object().unwrap().len()
    );
}

/// The DHT11 fixture is a single-wire board of the same pinned repository:
/// U4 is the catalog `dht11` pins-only model on the `behavior` adapter, its
/// DATA role on the SNS_IO1 net of the pinned PCB, with the R6 pull-up.
fn check_heypcb_dht11(
    source_map: &serde_json::Value,
    yaml: &serde_yaml::Value,
    _report: &labwired_core::flex::FlexRunReport,
) {
    let elements = &source_map["sensor_line"]["elements"];
    assert_eq!(elements[0]["source_ref"], "U4");
    assert_eq!(elements[0]["source_part"], "DHT11");
    assert_eq!(elements[0]["pads"]["2"], "SNS_IO1");
    assert_eq!(elements[1]["source_ref"], "R6");
    assert_eq!(
        elements[1]["pads"]["2"], "SNS_IO1",
        "R6 pulls the data line up"
    );
    assert_eq!(elements[1]["pads"]["1"], "3V3");

    let model = &yaml["cosim_models"][0];
    let binding = &source_map["runtime_bindings"]["device"];
    assert_eq!(model["id"], binding["id"].as_str().unwrap());
    assert_eq!(model["adapter"], "behavior");
    assert_eq!(model["config"]["device_type"], "dht11");
    assert_eq!(model["inputs"]["DATA"], "source.U3.GPIO4");
    assert_eq!(model["outputs"]["DATA"], "source.net.SNS_IO1");
}

/// The MCP9700 fixture is an analog board of the same pinned repository: U4
/// is the catalog `mcp9700` analog source, its VOUT pad on the SNS_AN1 net
/// that the pinned PCB routes to IO1.
fn check_heypcb_mcp9700(
    source_map: &serde_json::Value,
    yaml: &serde_yaml::Value,
    _report: &labwired_core::flex::FlexRunReport,
) {
    // The data sheet pin table and the PCB agree: VOUT is pad 3.
    let facts = &source_map["part_source"]["facts"];
    assert_eq!(facts["sc70_5_pins"]["3"], "VOUT");
    let u4 = &source_map["sensor_line"]["elements"][0];
    assert_eq!(u4["source_ref"], "U4");
    assert_eq!(u4["source_part"], "MCP9700T-E/LT");
    assert_eq!(u4["pads"]["3"], "SNS_AN1");
    assert_eq!(u4["pads"]["4"], "3V3");
    assert_eq!(u4["pads"]["2"], "GND");

    let model = &yaml["cosim_models"][0];
    let binding = &source_map["runtime_bindings"]["device"];
    assert_eq!(model["id"], binding["id"].as_str().unwrap());
    assert_eq!(model["adapter"], "behavior");
    assert_eq!(model["config"]["device_type"], "mcp9700");
    assert_eq!(model["outputs"]["voltage_mv"], "source.net.SNS_AN1");
}

/// The MCP9701A fixture is the next board of the same series: U4 is the
/// catalog `mcp9701` analog source in a TO-92 package, its VOUT pin 2 on the
/// SNS_AN1 net that the pinned PCB routes to IO1.
fn check_heypcb_mcp9701a(
    source_map: &serde_json::Value,
    yaml: &serde_yaml::Value,
    _report: &labwired_core::flex::FlexRunReport,
) {
    // The data sheet TO-92 pin table and the PCB agree: VOUT is pin 2.
    let facts = &source_map["part_source"]["facts"];
    assert_eq!(facts["to92_3_pins"]["2"], "VOUT");
    assert_eq!(facts["V_0C_mV"], 400);
    assert_eq!(facts["T_C_mV_per_C"], 19.5);
    let u4 = &source_map["sensor_line"]["elements"][0];
    assert_eq!(u4["source_ref"], "U4");
    assert_eq!(u4["source_part"], "MCP9701A-E/TO");
    assert_eq!(u4["pads"]["1"], "3V3");
    assert_eq!(u4["pads"]["2"], "SNS_AN1");
    assert_eq!(u4["pads"]["3"], "GND");

    let model = &yaml["cosim_models"][0];
    let binding = &source_map["runtime_bindings"]["device"];
    assert_eq!(model["id"], binding["id"].as_str().unwrap());
    assert_eq!(model["adapter"], "behavior");
    assert_eq!(model["config"]["device_type"], "mcp9701");
    assert_eq!(model["outputs"]["voltage_mv"], "source.net.SNS_AN1");
}

/// The NTC fixture solves the board's own divider: R6 from 3V3 to SNS_AN1,
/// U4 and C7 from SNS_AN1 to GND, as the pinned PCB routes them. U4 is a
/// netlist resistor bound to the analog adapter's beta-curve thermistor with
/// the Murata catalog values and a declared range inside the catalog table.
fn check_heypcb_ntc(
    source_map: &serde_json::Value,
    yaml: &serde_yaml::Value,
    _report: &labwired_core::flex::FlexRunReport,
) {
    let model = &yaml["cosim_models"][0];
    let binding = &source_map["runtime_bindings"]["network"];
    assert_eq!(model["id"], binding["id"].as_str().unwrap());
    assert_eq!(model["adapter"], "analog");
    assert_eq!(model["outputs"]["voltage"], "source.net.SNS_AN1");

    // Every netlist element sits between the nets its source pads are on.
    let netlist = model["config"]["netlist_text"].as_str().unwrap();
    let net = |name: &str| match name {
        "3V3" => "3v3",
        "SNS_AN1" => "sns_an1",
        "GND" => "0",
        other => panic!("unexpected net {other}"),
    };
    for element in source_map["sensor_line"]["elements"].as_array().unwrap() {
        let reference = element["source_ref"].as_str().unwrap();
        let name = if reference == "U4" {
            "RU4".to_string()
        } else {
            reference.to_string()
        };
        let line = netlist
            .lines()
            .find(|line| line.split_whitespace().next() == Some(name.as_str()))
            .unwrap_or_else(|| panic!("{name} is not in the netlist"));
        let fields: Vec<&str> = line.split_whitespace().collect();
        let pads = &element["pads"];
        assert_eq!(fields[1], net(pads["1"].as_str().unwrap()), "{reference}");
        assert_eq!(fields[2], net(pads["2"].as_str().unwrap()), "{reference}");
    }

    // The thermistor uses the catalog values; its range stays where the beta
    // equation is within 1.7 % of the catalog table.
    let facts = &source_map["part_source"]["facts"];
    let ntc = &model["config"]["thermistors"]["temperature"];
    assert_eq!(
        ntc["resistor"],
        binding["thermistor_resistor"].as_str().unwrap()
    );
    assert_eq!(ntc["r_ref_ohms"].as_f64(), facts["R25_ohm"].as_f64());
    assert_eq!(ntc["beta_kelvin"].as_f64(), facts["B25_50_K"].as_f64());
    let table = facts["center_value_table_kohm"].as_object().unwrap();
    let (min_c, max_c) = (
        ntc["min_c"].as_f64().unwrap(),
        ntc["max_c"].as_f64().unwrap(),
    );
    for (celsius, kohm) in table {
        let celsius: f64 = celsius.parse().unwrap();
        let beta = 10.0 * (3380.0 * (1.0 / (celsius + 273.15) - 1.0 / 298.15)).exp();
        let deviation = (beta / kohm.as_f64().unwrap() - 1.0).abs();
        if (min_c..=max_c).contains(&celsius) {
            assert!(deviation <= 0.0171, "{celsius} C deviates {deviation}");
        } else {
            assert!(deviation > 0.0171, "{celsius} C could be inside the range");
        }
    }
}

/// The MAX31855 fixture is a third board of the same pinned repository, and
/// the first on a Flex SPI bus: U4 on the SPI nets of the pinned PCB, its
/// chip select the GPIO of the SPI_CS net.
fn check_heypcb_max31855(
    source_map: &serde_json::Value,
    yaml: &serde_yaml::Value,
    report: &labwired_core::flex::FlexRunReport,
) {
    let u4 = &source_map["sensor_bus"]["elements"][0];
    assert_eq!(u4["source_ref"], "U4");
    assert_eq!(u4["source_part"], "MAX31855KASA+T");
    assert_eq!(u4["pads"]["5"], "SPI_SCK");
    assert_eq!(u4["pads"]["6"], "SPI_CS");
    assert_eq!(u4["pads"]["7"], "SPI_MISO");
    assert_eq!(
        source_map["sensor_bus"]["controller"]["endpoints"]["SPI_CS"],
        "U3 pad 18 (IO10)"
    );

    let bus = &yaml["spi_buses"][0];
    let binding = &source_map["runtime_bindings"];
    assert_eq!(bus["name"], binding["bus"]["name"].as_str().unwrap());
    let device = &bus["devices"][0];
    assert_eq!(device["id"], binding["device"]["id"].as_str().unwrap());
    assert_eq!(device["type"], "max31855");
    assert_eq!(device["cs"], 10);

    assert_eq!(report.spi_buses[0].devices[0].cs, 10);
}

macro_rules! heypcb_fixture {
    ($board:literal, $asset:expr, $check:expr) => {
        HeypcbFixture {
            board: $board,
            asset: $asset,
            source_map: include_str!(concat!(
                "../tests/fixtures/flex-heypcb-esp32-",
                $board,
                "/source-map.json"
            )),
            system: include_str!(concat!(
                "../tests/fixtures/flex-heypcb-esp32-",
                $board,
                "/system.yaml"
            )),
            check: $check,
        }
    };
}

const HEYPCB_FIXTURES: &[HeypcbFixture] = &[
    heypcb_fixture!(
        "bme280",
        Some("boards/bme280/kicad/esp32_bme280"),
        check_heypcb_bme280
    ),
    heypcb_fixture!(
        "sht40",
        Some("boards/sht40/kicad/esp32_sht40"),
        check_heypcb_sht40
    ),
    heypcb_fixture!(
        "tmp117",
        Some("boards/tmp117/kicad/esp32_tmp117"),
        check_heypcb_tmp117
    ),
    heypcb_fixture!(
        "mcp9808",
        Some("boards/mcp9808/kicad/esp32_mcp9808"),
        check_heypcb_mcp9808
    ),
    heypcb_fixture!(
        "scd41",
        Some("boards/scd41/kicad/esp32_scd41"),
        check_heypcb_scd41
    ),
    heypcb_fixture!(
        "dht11",
        Some("boards/dht11/kicad/esp32_dht11"),
        check_heypcb_dht11
    ),
    heypcb_fixture!(
        "mcp9700",
        Some("boards/mcp9700/kicad/esp32_mcp9700"),
        check_heypcb_mcp9700
    ),
    // MCP9701A: the fixture directory and the pinned board are both `mcp9701`.
    heypcb_fixture!(
        "mcp9701",
        Some("boards/mcp9701/kicad/esp32_mcp9701"),
        check_heypcb_mcp9701a
    ),
    heypcb_fixture!("ntc", Some("boards/ntc/kicad/esp32_ntc"), check_heypcb_ntc),
    // MAX31855 never pinned `source.asset`.
    heypcb_fixture!("max31855", None, check_heypcb_max31855),
];

#[test]
fn every_heypcb_fixture_matches_its_pinned_source_nets() {
    for row in HEYPCB_FIXTURES {
        eprintln!("heypcb fixture: {}", row.board);
        let source_map: serde_json::Value = serde_json::from_str(row.source_map).unwrap();
        let sibling: serde_json::Value = serde_json::from_str(HEYPCB_LDR_SOURCE_MAP).unwrap();
        for key in ["repository", "revision", "license_spdx"] {
            assert_eq!(
                source_map["source"][key], sibling["source"][key],
                "{}: {key}",
                row.board
            );
        }
        assert_eq!(
            source_map["source"]["files"]["license"], sibling["source"]["files"]["license"],
            "{}: one repository revision has one LICENSE blob",
            row.board
        );
        if let Some(asset) = row.asset {
            assert_eq!(source_map["source"]["asset"], asset, "{}", row.board);
        }
        let prefix = format!("boards/{}/", row.board);
        for file in ["schematic", "pcb", "bom", "board_json", "board_readme"] {
            let entry = &source_map["source"]["files"][file];
            assert!(
                entry["path"].as_str().unwrap().starts_with(&prefix),
                "{}: {file}",
                row.board
            );
            assert_eq!(
                entry["git_blob"].as_str().unwrap().len(),
                40,
                "{}: {file}",
                row.board
            );
        }

        let yaml: serde_yaml::Value = serde_yaml::from_str(row.system).unwrap();
        let runtime = FlexRuntime::from_yaml(row.system).unwrap();
        let report = runtime.run_report();
        assert!(report.scope_notes_complete, "{}", row.board);
        assert!(
            !report.execution_scope.original_target_firmware_executed,
            "{}",
            row.board
        );
        assert_eq!(
            report.execution_scope.chip_parity, "not_established",
            "{}",
            row.board
        );
        (row.check)(&source_map, &yaml, &report);
        let ports = &source_map["runtime_bindings"]["ports"];
        for port in ports.as_object().unwrap().keys() {
            runtime.port_id(port).unwrap();
        }
        assert_eq!(
            report.ports.len(),
            ports.as_object().unwrap().len(),
            "{}",
            row.board
        );
    }
}

/// The Chromium receipt of `scripts/test_browser_flex_scripts.mjs` covers
/// every committed `test-*.yaml`, and the shared runner still gives the
/// result text that Chromium and the CLI gave.
#[test]
fn flex_browser_script_receipt_covers_every_script_and_matches_the_shared_runner() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let receipt: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(fixtures.join("browser-flex-scripts.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["status"], "passed");
    assert_eq!(receipt["cpu_emulation"], false);
    assert_eq!(receipt["execution_path"], "flex_portable_wasm");
    assert!(!receipt["browser"].as_str().unwrap().is_empty());
    let core_commit = receipt["core_commit"].as_str().unwrap();
    assert_eq!(core_commit.len(), 40);
    assert!(core_commit.bytes().all(|byte| byte.is_ascii_hexdigit()));

    let mut on_disk = BTreeSet::new();
    for fixture in std::fs::read_dir(&fixtures).unwrap() {
        let fixture = fixture.unwrap();
        let name = fixture.file_name().into_string().unwrap();
        if !name.starts_with("flex-") || !fixture.path().is_dir() {
            continue;
        }
        for script in std::fs::read_dir(fixture.path()).unwrap() {
            let script = script.unwrap().file_name().into_string().unwrap();
            if script.starts_with("test-") && script.ends_with(".yaml") {
                on_disk.insert((name.clone(), script));
            }
        }
    }
    let rows = receipt["scripts"].as_array().unwrap();
    let listed: BTreeSet<(String, String)> = rows
        .iter()
        .map(|row| {
            (
                row["fixture"].as_str().unwrap().to_owned(),
                row["script"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        listed, on_disk,
        "the browser receipt must list every committed Flex test script; rerun scripts/test_browser_flex_scripts.mjs"
    );
    assert_eq!(rows.len(), listed.len(), "a script is listed twice");
    assert!(rows.len() >= 35);
    assert!(receipt["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| check == "i2c-transfer-record-equality"));
    assert!(receipt["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| check == "console-record-equality"));
    assert!(receipt["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| check == "analog-series-equality"));

    let mut suspended_steps = 0;
    let mut i2c_transfers = 0;
    let mut console_bytes = 0;
    for row in rows {
        let name = row["script"].as_str().unwrap();
        let dir = fixtures.join(row["fixture"].as_str().unwrap());
        let label = format!("{}/{name}", row["fixture"].as_str().unwrap());
        let script_text = std::fs::read_to_string(dir.join(name)).unwrap();
        let script: labwired_config::FlexTestScript = serde_yaml::from_str(&script_text).unwrap();
        let manifest = std::fs::read_to_string(dir.join(&script.inputs.flex)).unwrap();
        let firmware = std::fs::read(dir.join(&script.inputs.firmware)).unwrap();
        assert_eq!(
            row["script_sha256"],
            sha256(script_text.as_bytes()),
            "{label}"
        );
        assert_eq!(
            row["manifest_sha256"],
            sha256(manifest.as_bytes()),
            "{label}"
        );
        assert_eq!(row["firmware_sha256"], sha256(&firmware), "{label}");

        let run = crate::flex::run_test_script(name, &script_text, &manifest, &firmware).unwrap();
        assert_eq!(run.result.status, "pass", "{label}");
        assert_eq!(row["status"], "pass", "{label}");
        let text = serde_json::to_string_pretty(&run.result).unwrap();
        assert_eq!(
            row["result_sha256"],
            sha256(text.as_bytes()),
            "{label}: the runner no longer gives the result text of the browser receipt; rerun scripts/test_browser_flex_scripts.mjs"
        );
        suspended_steps += row["suspended_steps"].as_u64().unwrap();
        i2c_transfers += row["i2c_transfers"].as_u64().unwrap();
        let runtime = run.runtime.unwrap();
        assert_eq!(
            runtime.i2c_trace_snapshot(0).next_cursor,
            row["i2c_transfers"],
            "{label}"
        );
        let bytes = runtime.console_capture().map_or(0, |console| console.bytes);
        assert_eq!(row["console_bytes"], bytes, "{label}");
        console_bytes += bytes;
    }
    assert!(
        console_bytes >= 200,
        "the receipt must include the console scripts"
    );
    assert!(
        i2c_transfers >= 21,
        "the receipt must include the I2C bus scripts"
    );
    assert!(
        suspended_steps >= 13,
        "the receipt must include the endless-`main` scripts"
    );
}
