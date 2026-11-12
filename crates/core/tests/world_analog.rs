//! The world shares one real MNA deck. Same-named MCU pads keep their owners;
//! changing one load alters the common rail observed by the other controller.
use labwired_config::{ChipDescriptor, EnvironmentManifest, SystemManifest};
use labwired_core::cosim::CosimSignalValue;
use labwired_core::system::node::NodeFirmware;
use labwired_core::world::{MachineTrait, ResolvedWorldNode, World};
use std::path::Path;

const GPIO: u64 = 0x4002_0000;
const ADC: u64 = 0x4001_2000;
const MODEL: &str = r#"
cosim_models:
  - id: circuit
    adapter: analog
    step_ns: 1000
    inputs:
      ctl_a: node.a.board.gpio_pad.pa8
      ctl_b: node.b.board.gpio_pad.pa8
    outputs:
      sense_a: node.a.adc.adc1.0_volts
      sense_b: node.b.adc.adc1.0_volts
    config:
      substeps: 1
      netlist_text: |
        Vbat battery 0 dc 3.3
        Rsource battery rail 2
        Sa rail a ctl_a ron=0.1 roff=1e12
        Sb rail b ctl_b ron=0.1 roff=1e12
        Ra a 0 10
        Rb b 0 20
        .end
      probes:
        rail: v(rail)
        source_current: i(Vbat)
        sense_a: v(a)
        sense_b: v(b)
"#;

fn firmware() -> Vec<u8> {
    // ELF32 ARM with reset vectors and `b .`; no native host stand-in for CPU.
    firmware_with(&[0xe7fe])
}

fn firmware_with(code: &[u16]) -> Vec<u8> {
    let length = 8 + code.len() as u32 * 2;
    let mut bytes = vec![0; 84 + length as usize];
    bytes[0..7].copy_from_slice(b"\x7fELF\x01\x01\x01");
    for (offset, value) in [(16, 2u16), (18, 40), (40, 52), (42, 32), (44, 1)] {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    for (offset, value) in [
        (20, 1u32),
        (24, 0x0800_0009),
        (28, 52),
        (36, 0x0500_0000),
        (52, 1),
        (56, 84),
        (60, 0x0800_0000),
        (64, 0x0800_0000),
        (68, length),
        (72, length),
        (76, 5),
        (80, 4),
        (84, 0x2000_1000),
        (88, 0x0800_0009),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (i, instruction) in code.iter().enumerate() {
        bytes[92 + i * 2..94 + i * 2].copy_from_slice(&instruction.to_le_bytes());
    }
    bytes
}

fn world(order: &[&str], model: &str) -> anyhow::Result<World> {
    configured_world(order, model, None, None, false)
}

fn configured_world(
    order: &[&str],
    model: &str,
    program_a: Option<&[u16]>,
    voltage_b: Option<f64>,
    reset_metadata: bool,
) -> anyhow::Result<World> {
    configured_world_with_limits(order, model, program_a, voltage_b, reset_metadata, None)
}

fn configured_world_with_limits(
    order: &[&str],
    model: &str,
    program_a: Option<&[u16]>,
    voltage_b: Option<f64>,
    reset_metadata: bool,
    gpio_limits: Option<labwired_config::GpioInputThresholds>,
) -> anyhow::Result<World> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut yaml = "name: coupled\nnodes:\n".to_string();
    for id in order {
        yaml.push_str(&format!(
            "  - {{id: {id}, system: {id}.yaml, firmware: {id}.elf}}\n"
        ));
    }
    yaml.push_str(model);
    let manifest: EnvironmentManifest = serde_yaml::from_str(&yaml)?;
    let mut nodes = Vec::new();
    for id in order {
        let mut chip = ChipDescriptor::from_file(root.join("configs/chips/stm32f401.yaml"))?;
        if let Some(limits) = gpio_limits {
            // A generic router fixture on the shipped F401 bus. The supplied
            // limits are fixture data, not a claim about F401 silicon.
            chip.gpio_input_thresholds = Some(limits);
        }
        if *id == "b" {
            if let Some(volts) = voltage_b {
                chip.io_voltage_v = Some(volts);
            }
        }
        if reset_metadata {
            // This fixture uses the explicit AT32 reset limits with a shipped
            // F401 CPU/bus to test generic routing, not F401 reset silicon.
            chip.reset_input = Some(serde_yaml::from_str("pin: NRST\nactive_low: true\nvil_max_v: 0.72\nvih_min_v: 2.0\nmin_assert_ns: 80000\ninvalid_pulse_ns: 40000\npull_up_ohms: 40000\n")?);
        }
        let system: SystemManifest = serde_yaml::from_str("name: controller\nchip: stm32f401\n")?;
        nodes.push(ResolvedWorldNode {
            id: (*id).to_string(),
            system,
            chip,
            firmware: NodeFirmware::from_bytes(if *id == "a" {
                program_a.map_or_else(firmware, firmware_with)
            } else {
                firmware()
            }),
            blobs: Default::default(),
        });
    }
    World::from_resolved(manifest, nodes)
}

fn wr(machine: &mut dyn MachineTrait, address: u64, value: u32) {
    for byte in (0..4).rev() {
        machine
            .write_u8(address + byte, (value >> (byte * 8)) as u8)
            .unwrap();
    }
}
fn rd(machine: &dyn MachineTrait, address: u64) -> u32 {
    u32::from_le_bytes(std::array::from_fn(|b| {
        machine.read_u8(address + b as u64).unwrap()
    }))
}
fn drive(world: &mut World, id: &str, high: bool) {
    let m = world.machines.get_mut(id).unwrap().as_mut();
    wr(m, GPIO, 1 << 16); // PA8 output; PA0 remains an analog observation.
    wr(m, GPIO + 0x14, u32::from(high) << 8);
}
fn step(world: &mut World) {
    let before = world.cosim_status().unwrap().model_steps;
    for _ in 0..10_000 {
        let result = world.step_all();
        assert!(result.values().all(Result::is_ok), "{result:?}");
        if world.cosim_status().unwrap().model_steps > before {
            return;
        }
    }
    panic!("shared analog boundary was never reached");
}
fn volts(world: &World, name: &str) -> f64 {
    let channels = world.analog_channels();
    let channel = channels
        .iter()
        .position(|c| c.name == format!("circuit.{name}"))
        .unwrap();
    let trace = world.analog_trace_snapshot(0);
    f64::from(trace.samples.last().unwrap().values[channel])
}
fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-6, "{actual} vs {expected}");
}

#[test]
fn two_pa8_pads_control_independent_switches_on_one_shared_rail() {
    let mut w = world(&["a", "b"], MODEL).unwrap();
    drive(&mut w, "a", true);
    drive(&mut w, "b", false);
    step(&mut w);
    let one_load_a = 3.3 * 10.0 / 12.1;
    close(volts(&w, "sense_a"), one_load_a);
    assert!(volts(&w, "sense_b") < 1e-8);
    assert_eq!(
        w.cosim_signals().unwrap()["node.a.board.gpio_pad.pa8"],
        CosimSignalValue::Bool(true)
    );
    assert_eq!(
        w.cosim_signals().unwrap()["node.b.board.gpio_pad.pa8"],
        CosimSignalValue::Bool(false)
    );
    drive(&mut w, "b", true);
    step(&mut w);
    let parallel = 1.0 / (1.0 / 10.1 + 1.0 / 20.1);
    let rail = 3.3 * parallel / (2.0 + parallel);
    close(volts(&w, "rail"), rail);
    close(volts(&w, "sense_a"), rail * 10.0 / 10.1);
    close(volts(&w, "sense_b"), rail * 20.0 / 20.1);
    assert!(
        volts(&w, "sense_a") < one_load_a,
        "B's load must affect A through the same rail"
    );
    close(
        -volts(&w, "source_current"),
        volts(&w, "sense_a") / 10.0 + volts(&w, "sense_b") / 20.0,
    );
    assert_eq!(
        w.cosim_status().unwrap().model_steps,
        2,
        "one circuit solve per boundary, not per MCU"
    );
    assert_eq!(
        w.analog_trace_snapshot(0)
            .samples
            .iter()
            .map(|s| s.time_ns)
            .collect::<Vec<_>>(),
        vec![0, 1000, 2000]
    );
}

#[test]
fn analog_results_reach_each_real_adc_independently() {
    let mut w = world(&["a", "b"], MODEL).unwrap();
    drive(&mut w, "a", true);
    drive(&mut w, "b", false);
    step(&mut w);
    for id in ["a", "b"] {
        let m = w.machines.get_mut(id).unwrap().as_mut();
        wr(m, 0x4002_3844, 1 << 8); // APB2 ADC1 clock
        wr(m, ADC + 0x34, 0); // SQ1 channel 0
        wr(m, ADC + 0x08, 1); // ADON before SWSTART
        wr(m, ADC + 0x08, (1 << 30) | 1); // SWSTART + ADON
    }
    step(&mut w);
    let a = rd(w.machines["a"].as_ref(), ADC + 0x4c);
    let b = rd(w.machines["b"].as_ref(), ADC + 0x4c);
    assert!(
        a > 3300 && a < 3450,
        "A ADC must observe its loaded branch: {a}"
    );
    assert_eq!(b, 0, "B ADC must observe its off branch");
}

#[test]
fn node_order_and_host_step_chunking_do_not_duplicate_or_reorder_solves() {
    let mut a = world(&["a", "b"], MODEL).unwrap();
    let mut b = world(&["b", "a"], MODEL).unwrap();
    for w in [&mut a, &mut b] {
        drive(w, "a", true);
        drive(w, "b", false);
    }
    for _ in 0..4 {
        step(&mut a);
    }
    for (_, result) in b.step_rounds(4) {
        result.unwrap();
    }
    assert_eq!(
        a.cosim_status().unwrap().model_steps,
        b.cosim_status().unwrap().model_steps
    );
    assert_eq!(
        serde_json::to_value(a.analog_trace_snapshot(0)).unwrap(),
        serde_json::to_value(b.analog_trace_snapshot(0)).unwrap()
    );
    assert_eq!(a.round_now_ps(), Some(4_000_000));
}

#[test]
fn bad_routes_remain_fail_closed() {
    for (old, new, reason) in [
        (
            "node.a.board.gpio_pad.pa8",
            "node.a.board.gpio_pad.absent",
            "analog bindings",
        ),
        (
            "node.a.board.gpio_pad.pa8",
            "node.a.board.missing.pa8",
            "unknown hardware route",
        ),
        (
            "node.a.board.gpio_pad.pa8",
            "node.missing.board.gpio_pad.pa8",
            "unknown node",
        ),
    ] {
        let error = world(&["a", "b"], &MODEL.replace(old, new))
            .err()
            .map(|error| format!("{error:#}"))
            .expect("must refuse");
        assert!(error.contains(reason), "{error}");
    }
}

#[test]
fn firmware_edges_use_the_next_interval_not_the_completed_intervals_past() {
    // Configure PA8 and set it high inside the first 1 us interval.
    let program = [
        0x4803, 0x4904, 0x6001, 0x2101, 0x0209, 0x6141, 0xe7fe, 0xbf00, 0x0000, 0x4002, 0x0000,
        0x0001,
    ];
    let mut w = configured_world(&["a", "b"], MODEL, Some(&program), None, false).unwrap();
    step(&mut w);
    assert_ne!(
        rd(w.machines["a"].as_ref(), GPIO + 0x14) & (1 << 8),
        0,
        "firmware changed PA8 within the interval"
    );
    assert!(
        volts(&w, "sense_a") < 1e-8,
        "the end-of-interval high must not drive the interval's past"
    );
    step(&mut w);
    close(volts(&w, "sense_a"), 3.3 * 10.0 / 12.1);
    let status = w.cosim_status().unwrap();
    assert_eq!(status.step_ns, 1000);
    assert_eq!(status.completed_time_ns, 2000);
    assert_eq!(status.coupling, "interval-start-zero-order-hold");
}

const VOLTAGE_MODEL: &str = r#"
cosim_models:
  - id: circuit
    adapter: analog
    step_ns: 1000
    inputs:
      drive_a: node.a.board.gpio_pad_volts.pa8
      drive_b: node.b.board.gpio_pad_volts.pa8
      ctl_a: node.a.board.gpio_driven.pa8
      ctl_b: node.b.board.gpio_driven.pa8
    outputs:
      sense_a: node.a.adc.adc1.0_volts
      sense_b: node.b.adc.adc1.0_volts
    config:
      substeps: 1
      netlist_text: |
        Va drive_a 0 dc 0
        Vb drive_b 0 dc 0
        Sa drive_a a ctl_a ron=0.1 roff=1e12
        Sb drive_b b ctl_b ron=0.1 roff=1e12
        Ra a 0 1k
        Rb b 0 1k
        .end
      sources: {drive_a: Va, drive_b: Vb}
      probes: {sense_a: 'v(a)', sense_b: 'v(b)'}
"#;

#[test]
fn per_node_drive_voltage_and_high_impedance_are_independent() {
    let mut w = configured_world(&["a", "b"], VOLTAGE_MODEL, None, Some(1.8), false).unwrap();
    drive(&mut w, "a", true);
    drive(&mut w, "b", true);
    step(&mut w);
    close(volts(&w, "sense_a"), 3.3 * 1000.0 / 1000.1);
    close(volts(&w, "sense_b"), 1.8 * 1000.0 / 1000.1);
    wr(w.machines.get_mut("b").unwrap().as_mut(), GPIO, 0); // high impedance
    step(&mut w);
    assert!(volts(&w, "sense_b") < 1e-7);
    close(volts(&w, "sense_a"), 3.3 * 1000.0 / 1000.1);
}

#[test]
fn drive_amplitudes_follow_each_modeled_supply_rail() {
    let model = VOLTAGE_MODEL
        .replace("    config:", "      supply_a: node.a.board.power.vdd_volts\n      supply_b: node.b.board.power.vdd_volts\n    config:")
        .replace("        Va drive_a", "        Vrail_a supply_a 0 dc 3.0\n        Vrail_b supply_b 0 dc 2.4\n        Va drive_a")
        .replace("probes: {sense_a: 'v(a)', sense_b: 'v(b)'}", "probes: {sense_a: 'v(a)', sense_b: 'v(b)', supply_a: 'v(supply_a)', supply_b: 'v(supply_b)'}");
    let mut w = world(&["a", "b"], &model).unwrap();
    step(&mut w); // each real supply supervisor releases and resets its MCU
    drive(&mut w, "a", true);
    drive(&mut w, "b", true);
    step(&mut w);
    close(volts(&w, "sense_a"), 3.0 * 1000.0 / 1000.1);
    close(volts(&w, "sense_b"), 2.4 * 1000.0 / 1000.1);
}

const RESET_MODEL: &str = r#"
cosim_models:
  - id: circuit
    adapter: analog
    step_ns: 10000
    inputs:
      press_a: ui.reset_a
    outputs:
      power_a: node.a.board.power.vdd_volts
      power_b: node.b.board.power.vdd_volts
      reset_a: node.a.board.reset.nrst_volts
      reset_b: node.b.board.reset.nrst_volts
    config:
      substeps: 4
      netlist_text: |
        Vrail vdd 0 dc 3.3
        Rinternal_a vdd nrst_a 40k
        Rexternal_a vdd nrst_a 10k
        Ca nrst_a 0 100n
        Rinternal_b vdd nrst_b 40k
        Rexternal_b vdd nrst_b 10k
        Cb nrst_b 0 100n
        Sreset_a nrst_a 0 press_a ron=0.1 roff=1e12
        .end
      probes: {power_a: 'v(vdd)', power_b: 'v(vdd)', reset_a: 'v(nrst_a)', reset_b: 'v(nrst_b)'}
"#;

#[test]
fn separate_reset_pins_hold_actual_cpus_and_release_from_the_same_mna_rc_network() {
    let counter = [0x2200, 0x3201, 0xe7fd]; // movs r2,#0; adds r2,#1; b to adds
    let mut w = configured_world(&["a", "b"], RESET_MODEL, Some(&counter), None, true).unwrap();
    step(&mut w);
    assert!(w.machines["a"].get_register(2) > 0);
    wr(
        w.machines.get_mut("a").unwrap().as_mut(),
        0x2000_0000,
        0x1357_2468,
    );
    w.set_cosim_signal("ui.reset_a", CosimSignalValue::Bool(true))
        .unwrap();
    for _ in 0..10 {
        step(&mut w);
    }
    let status = w.cosim_status().unwrap();
    assert!(status.reset_inputs["a"].held);
    assert!(!status.reset_inputs["b"].held);
    assert_eq!(status.reset_inputs["a"].assertions, 1);
    assert_eq!(status.reset_inputs["b"].assertions, 0);
    assert_eq!(
        w.machines["a"].get_pc() & !1,
        0x0800_0008,
        "pin assertion restarts at the actual reset vector"
    );
    let held_register = w.machines["a"].get_register(2);
    for _ in 0..5 {
        step(&mut w);
    }
    assert_eq!(
        w.machines["a"].get_register(2),
        held_register,
        "reset-held CPU executes no instructions while world time advances"
    );
    assert_eq!(
        rd(w.machines["a"].as_ref(), 0x2000_0000),
        0x1357_2468,
        "reset preserves SRAM"
    );
    w.set_cosim_signal("ui.reset_a", CosimSignalValue::Bool(false))
        .unwrap();
    for _ in 0..10 {
        step(&mut w);
    }
    assert!(
        w.cosim_status().unwrap().reset_inputs["a"].held,
        "opening the button does not skip RC charging"
    );
    for _ in 0..100 {
        step(&mut w);
        if !w.cosim_status().unwrap().reset_inputs["a"].held {
            break;
        }
    }
    assert!(!w.cosim_status().unwrap().reset_inputs["a"].held);
    assert!(volts(&w, "reset_a") >= 2.0);
    step(&mut w);
    assert!(
        w.machines["a"].get_register(2) > 0,
        "CPU resumes only after the qualified high level"
    );
}

#[test]
fn supply_and_external_reset_holds_are_independent() {
    let model = RESET_MODEL
        .replace(
            "      press_a: ui.reset_a",
            "      press_a: ui.reset_a\n      rail: ui.rail",
        )
        .replace("Vrail vdd 0 dc 3.3", "Vrail vdd 0 dc 0")
        .replace(
            "      substeps: 4",
            "      substeps: 4\n      sources: {rail: Vrail}",
        );
    let counter = [0x3201, 0xe7fd];
    let mut w = configured_world(&["a", "b"], &model, Some(&counter), None, true).unwrap();
    w.set_cosim_signal("ui.rail", CosimSignalValue::F64(3.3))
        .unwrap();
    for _ in 0..10 {
        step(&mut w);
    }
    assert!(
        w.cosim_status().unwrap().reset_inputs["a"].held,
        "VDD release cannot bypass the still-low RC reset pin"
    );
    assert_eq!(w.machines["a"].get_register(2), 0);
    for _ in 0..100 {
        step(&mut w);
    }
    assert!(w.machines["a"].get_register(2) > 0);
    w.set_cosim_signal("ui.rail", CosimSignalValue::F64(0.0))
        .unwrap();
    step(&mut w);
    let count = w.machines["a"].get_register(2);
    assert!(
        !w.cosim_status().unwrap().reset_inputs["a"].held,
        "reset capacitor still holds a high voltage"
    );
    step(&mut w);
    assert_eq!(
        w.machines["a"].get_register(2),
        count,
        "high NRST cannot bypass absent VDD"
    );
}

#[test]
fn reset_routes_require_declared_reset_metadata() {
    let error = world(&["a", "b"], RESET_MODEL)
        .err()
        .map(|e| format!("{e:#}"))
        .unwrap();
    assert!(error.contains("no reset_input declaration"), "{error}");
}

#[test]
fn digital_nets_and_analog_islands_have_one_physical_pad_owner() {
    let digital = r#"
interconnects:
  - type: gpio_net
    nodes: [a, b]
    config:
      name: separate-digital-wire
      latency_ns: 100
      members:
        - {node: a, peripheral: gpioa, pin: 8}
        - {node: b, peripheral: gpioa, pin: 8}
"#;
    let error = world(&["a", "b"], &(MODEL.to_string() + digital))
        .err()
        .map(|e| format!("{e:#}"))
        .unwrap();
    assert!(error.contains("both gpio_net and world analog"), "{error}");
    let mut w = world(
        &["a", "b"],
        &(MODEL.to_string() + &digital.replace("pin: 8", "pin: 6")),
    )
    .unwrap();
    drive(&mut w, "a", true);
    drive(&mut w, "b", false);
    step(&mut w);
    step(&mut w);
    assert_eq!(w.cosim_status().unwrap().model_steps, 2);
    close(volts(&w, "sense_a"), 3.3 * 10.0 / 12.1);
}

#[test]
fn output_aliases_and_missing_probes_cannot_silently_replace_observations() {
    let aliased = MODEL.replace("node.b.adc.adc1.0_volts", "node.a.board.analog.pa0_volts");
    let error = world(&["a", "b"], &aliased)
        .err()
        .expect("aliases must refuse")
        .to_string();
    assert!(error.contains("multiple output writers"), "{error}");
    let missing_probe = MODEL.replace("sense_a: v(a)", "unrouted: v(a)");
    let error = world(&["a", "b"], &missing_probe)
        .err()
        .expect("missing probe must refuse")
        .to_string();
    assert!(error.contains("no solved probe"), "{error}");
}

fn digital_wire(latency_ns: u64) -> String {
    format!(
        r#"
interconnects:
  - type: gpio_net
    nodes: [a, b]
    config:
      name: independent-digital-wire
      latency_ns: {latency_ns}
      members:
        - {{node: a, peripheral: gpioa, pin: 6}}
        - {{node: b, peripheral: gpioa, pin: 6}}
"#
    )
}

fn check_three_analog_steps(w: &World) {
    let status = w.cosim_status().unwrap();
    assert_eq!(status.model_steps, 3);
    assert_eq!(status.completed_time_ns, 3000);
    assert_eq!(
        w.analog_trace_snapshot(0)
            .samples
            .iter()
            .map(|sample| sample.time_ns)
            .collect::<Vec<_>>(),
        vec![0, 1000, 2000, 3000]
    );
    close(volts(w, "sense_a"), 3.3 * 10.0 / 12.1);
}

#[test]
fn world_advance_api_run_until_ps_steps_an_analog_only_world() {
    let mut w = world(&["a", "b"], MODEL).unwrap();
    drive(&mut w, "a", true);
    let results = w
        .run_until_ps(3_000_000)
        .expect("an analog-only world has a round clock");
    assert!(results.values().all(Result::is_ok), "{results:?}");
    assert_eq!(w.round_now_ps(), Some(3_000_000));
    check_three_analog_steps(&w);
}

#[test]
fn world_advance_api_run_until_ps_keeps_analog_steps_in_a_gpio_world() {
    let mut w = world(&["a", "b"], &(MODEL.to_string() + &digital_wire(100))).unwrap();
    drive(&mut w, "a", true);
    let results = w.run_until_ps(3_500_000).unwrap();
    assert!(results.values().all(Result::is_ok), "{results:?}");
    assert_eq!(w.round_now_ps(), Some(3_500_000));
    check_three_analog_steps(&w);
}

#[test]
fn world_advance_api_step_rounds_respects_the_analog_boundary_before_gpio_latency() {
    let mut w = world(&["a", "b"], &(MODEL.to_string() + &digital_wire(1200))).unwrap();
    drive(&mut w, "a", true);
    let results = w.step_rounds(3);
    assert!(results.values().all(Result::is_ok), "{results:?}");
    assert_eq!(
        w.round_now_ps(),
        Some(3_000_000),
        "the 1 us analog boundary takes precedence over 1.2 us GPIO lookahead"
    );
    check_three_analog_steps(&w);
}

#[test]
fn gpio_voltage_thresholds_follow_each_nodes_same_boundary_vdd() {
    let model = r#"
cosim_models:
  - id: circuit
    adapter: analog
    step_ns: 1000
    inputs: {rail_b: ui.rail_b}
    outputs:
      supply_a: node.a.board.power.vdd_volts
      supply_b: node.b.board.power.vdd_volts
      input_a: node.a.board.gpio_in.pa2
      input_b: node.b.board.gpio_in.pa2
    config:
      netlist_text: |
        Va vdd_a 0 dc 2.1
        Vb vdd_b 0 dc 3.3
        Vsignal signal 0 dc 2.1
        Ra vdd_a 0 10k
        Rb vdd_b 0 10k
        Rs signal 0 10k
        .end
      sources: {rail_b: Vb}
      probes: {supply_a: 'v(vdd_a)', supply_b: 'v(vdd_b)', input_a: 'v(signal)', input_b: 'v(signal)'}
"#;
    let mut w = world(&["a", "b"], model).unwrap();
    let high = |w: &World, id: &str| rd(w.machines[id].as_ref(), GPIO + 0x10) & (1 << 2) != 0;
    assert!(
        high(&w, "a"),
        "2.1 V is high on the actual 2.1 V rail; nominal VIH=2.31 V must not win"
    );
    assert!(
        !high(&w, "b"),
        "the same input is below VIH on the independent 3.3 V rail"
    );
    w.set_cosim_signal("ui.rail_b", CosimSignalValue::F64(3.3))
        .unwrap();
    step(&mut w);
    w.set_cosim_signal("ui.rail_b", CosimSignalValue::F64(2.1))
        .unwrap();
    step(&mut w);
    assert!(
        high(&w, "b"),
        "the GPIO observation must use this boundary's changed VDD"
    );
    w.set_cosim_signal("ui.rail_b", CosimSignalValue::F64(0.0))
        .unwrap();
    step(&mut w);
    assert!(!high(&w, "b"), "zero VDD has no valid logic high");
    assert!(
        high(&w, "a"),
        "powering off node b must not change node a's thresholds"
    );
}

#[test]
fn affine_gpio_inputs_use_each_nodes_routed_supply_at_the_same_boundary() {
    let model = r#"
cosim_models:
  - id: circuit
    adapter: analog
    step_ns: 1000
    inputs: {rail_b: ui.rail_b}
    outputs:
      supply_a: node.a.board.power.vdd_volts
      supply_b: node.b.board.power.vdd_volts
      input_a: node.a.board.gpio_in.pa2
      input_b: node.b.board.gpio_in.pa2
    config:
      netlist_text: |
        Va vdd_a 0 dc 2.1
        Vb vdd_b 0 dc 3.3
        Vsignal signal 0 dc 1.6
        Ra vdd_a 0 10k
        Rb vdd_b 0 10k
        Rs signal 0 10k
        .end
      sources: {rail_b: Vb}
      probes: {supply_a: 'v(vdd_a)', supply_b: 'v(vdd_b)', input_a: 'v(signal)', input_b: 'v(signal)'}
"#;
    // AT32F421 Table32 guaranteed limits, on a generic F401 routing fixture.
    let limits =
        serde_yaml::from_str("vil: 0.28\nvih: 0.31\nvil_offset_v: 0.1\nvih_offset_v: 0.8\n")
            .unwrap();
    let mut w =
        configured_world_with_limits(&["a", "b"], model, None, None, false, Some(limits)).unwrap();
    let high = |w: &World, id: &str| rd(w.machines[id].as_ref(), GPIO + 0x10) & (1 << 2) != 0;
    assert!(high(&w, "a"), "1.6 V exceeds VIH1.451 V at VDD2.1 V");
    assert!(!high(&w, "b"), "1.6 V remains below VIH1.823 V at VDD3.3 V");
    w.set_cosim_signal("ui.rail_b", CosimSignalValue::F64(2.1))
        .unwrap();
    step(&mut w);
    assert!(
        high(&w, "b"),
        "new rail and affine threshold must apply at the same boundary"
    );
    w.set_cosim_signal("ui.rail_b", CosimSignalValue::F64(0.0))
        .unwrap();
    step(&mut w);
    assert!(!high(&w, "b"), "an unpowered node has no valid high level");
    assert!(high(&w, "a"));
}

#[test]
fn world_status_preserves_the_actual_initialization_contract() {
    let original = world(&["a", "b"], MODEL).unwrap();
    let op = &original.cosim_status().unwrap().initialization;
    assert_eq!(serde_json::to_value(op).unwrap()["mode"], "operating_point");
    assert_eq!(op.base_unknowns, op.initialization_unknowns);
    let explicit = MODEL.replace("      substeps: 1", "      substeps: 1\n      initialization:\n        mode: transient_state\n        capacitor_voltages_v: { Ca: 0 }\n        inductor_currents_a: {}")
        .replace("        Ra a 0 10", "        Ra a 0 10\n        Ca a 0 1u");
    let mut w = world(&["a", "b"], &explicit).unwrap();
    let initial = serde_json::to_value(&w.cosim_status().unwrap().initialization).unwrap();
    assert_eq!(initial["mode"], "transient_state");
    assert_eq!(
        initial["initialization_unknowns"].as_u64().unwrap(),
        initial["base_unknowns"].as_u64().unwrap() + 1
    );
    assert_eq!(w.cosim_status().unwrap().completed_time_ns, 0);
    step(&mut w);
    assert_eq!(
        serde_json::to_value(&w.cosim_status().unwrap().initialization).unwrap(),
        initial
    );
}
