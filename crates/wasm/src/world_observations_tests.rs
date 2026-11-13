// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Native side of the real WasmWorld boundary; JS serialization and unknown
//! node errors are exercised by world-observations-wasm.test.ts after build.
use super::*;
use labwired_core::bus::SystemBus;
use labwired_core::cpu::cortex_m::CortexM;
use labwired_core::peripherals::i2c::I2c;
use labwired_core::world::MachineTrait;
use labwired_core::Machine;

fn motor_world() -> WasmWorld {
    let manifest = serde_yaml::from_str(
        r#"
schema_version: "1.0"
name: native-node-observations
nodes:
  - { id: alpha, system: unused.yaml, firmware: unused.elf }
  - { id: beta, system: unused.yaml, firmware: unused.elf }
interconnects: []
"#,
    )
    .unwrap();
    // This checked CubeMX fixture has a real Thumb reset vector and exercises
    // native UART/timer state. The attached motors are observed while disabled;
    // existing six-step parity tests separately cover driven motor dynamics.
    let system = include_str!("../../../examples/nucleo-l476rg-bldc/system.yaml");
    let nodes = [
        ("alpha", system.to_owned()),
        ("beta", system.replace("24.0", "12.0")),
    ]
    .into_iter()
    .map(|(id, system_yaml)| ResolvedNodeInput {
        id: id.into(),
        system_yaml,
        chip_yaml: include_str!("../../../configs/chips/stm32l476.yaml").into(),
        firmware: include_bytes!("../../../tests/fixtures/nucleo-l476rg-cubemx-hal.elf").to_vec(),
        blobs: HashMap::new(),
    })
    .collect();
    WasmWorld::from_node_inputs(manifest, nodes).expect("real resolved motor World")
}

#[test]
fn real_nodes_expose_motor_history_and_independent_live_cursors() {
    let mut world = motor_world();
    for (id, voltage) in [("alpha", 24.0), ("beta", 12.0)] {
        let state = world.world.machines[id].actuator_states();
        assert_eq!(state.len(), 1);
        assert_eq!(state[0]["id"], "drive_motor");
        assert_eq!(state[0]["kind"], "bldc-motor");
        assert_eq!(state[0]["bus_voltage_v"], voltage);
    }
    for (id, result) in world.world.step_rounds(100_000) {
        result.unwrap_or_else(|error| panic!("{id}: {error}"));
    }
    let alpha = &world.world.machines["alpha"];
    let beta = &world.world.machines["beta"];
    let a = alpha
        .live_numeric_capture(0, 20_000)
        .expect("native motor producer");
    let b = beta
        .live_numeric_capture(0, 20_000)
        .expect("native motor producer");
    assert_ne!(a.epoch, b.epoch);
    assert!(!a.points.is_empty());
    assert!(!b.points.is_empty());
    assert!(a
        .channels
        .iter()
        .any(|c| c.name == "motor.drive_motor.speed_rpm" && c.unit == "rpm"));
    assert!(a
        .channels
        .iter()
        .any(|c| c.name == "motor.drive_motor.current_a" && c.unit == "A"));
    assert_eq!(alpha.live_numeric_capture(0, 20_000).unwrap(), a);
    let limited = alpha.live_numeric_capture(0, 1).unwrap();
    assert_eq!(limited.points.len(), 1);
    assert_eq!(limited.next_cursor, a.next_cursor);
    assert!(limited.omitted > 0);
    assert!(alpha
        .live_numeric_capture(a.next_cursor, 20_000)
        .unwrap()
        .points
        .is_empty());
    assert_eq!(beta.live_numeric_capture(0, 20_000).unwrap(), b);
    let series = alpha.analog_series().expect("native motor history");
    assert!(series.end_time_ns > 0);
    assert!(series
        .channels
        .iter()
        .any(|c| c.name == "motor.drive_motor.speed_rpm" && !c.points.is_empty()));
    assert_eq!(alpha.analog_series().unwrap(), series);
    // Motor history is not relabelled as coupled-circuit samples.
    assert!(alpha.analog_channels().is_empty());
    assert!(alpha.analog_trace_snapshot(0).samples.is_empty());
    assert!(world.world.analog_channels().is_empty());
    assert!(world.world.analog_series().is_none());
}

#[test]
fn node_logic_discovery_routing_and_decoder_snapshot_come_from_native_devices() {
    let mut world = motor_world();
    for (_, result) in world.world.step_rounds(100_000) {
        result.unwrap();
    }
    let alpha = &world.world.machines["alpha"];
    let wires = alpha.wire_surface();
    assert!(
        wires
            .iter()
            .any(|(id, lines)| id == "uart2" && lines.iter().any(|line| line == "TX")),
        "{wires:?}"
    );
    assert!(alpha.gpio_routing("gpioa", 8).is_some());
    assert!(alpha.gpio_routing("missing", 8).is_none());
    assert!(alpha.peripheral_snapshot("uart2").is_some());
    assert!(alpha.peripheral_snapshot("missing").is_none());
    let before = alpha.total_cycles();
    let trace = alpha.bus_trace_snapshot();
    assert!(
        trace.iter().any(|event| matches!(
            &event.payload,
            labwired_core::bus::bus_trace::BusPayload::Uart { .. }
        )),
        "real UART activity required"
    );
    assert_eq!(alpha.total_cycles(), before);
    assert_eq!(
        serde_json::to_value(alpha.bus_trace_snapshot()).unwrap(),
        serde_json::to_value(trace).unwrap()
    );
}

fn sensor_machine() -> Machine<CortexM> {
    let chip: ChipDescriptor =
        serde_yaml::from_str(include_str!("../../../configs/chips/stm32l476.yaml")).unwrap();
    let system: SystemManifest = serde_yaml::from_str(
        r#"
name: native-bmi270-inputs
chip: unused
external_devices:
  - { id: imu, type: bmi270, connection: i2c1, config: { i2c_address: 0x68 } }
  - { id: second, type: bmi270, connection: i2c1, config: { i2c_address: 0x69 } }
"#,
    )
    .unwrap();
    Machine::new(
        CortexM::new(),
        SystemBus::from_config(&chip, &system).unwrap(),
    )
}

/// Read the actual BMI270 measurement register burst, not input intent.
fn sensor_registers(machine: &mut Machine<CortexM>, component: &str) -> Vec<i16> {
    for entry in &mut machine.bus.peripherals {
        let Some(i2c) = entry
            .dev
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<I2c>())
        else {
            continue;
        };
        for cell in i2c.attached_devices() {
            let mut dev = cell.borrow_mut();
            if !dev
                .as_sim_input_mut()
                .is_some_and(|input| input.component_id() == Some(component))
            {
                continue;
            }
            dev.stop();
            dev.write(0x0c);
            let bytes: Vec<_> = (0..12).map(|_| dev.read()).collect();
            return bytes
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect();
        }
    }
    panic!("missing native sensor {component}");
}

#[test]
fn world_machine_qualified_batches_reach_bmi270_registers_atomically() {
    let mut machine = sensor_machine();
    let untouched = sensor_registers(&mut machine, "second");
    let channels = MachineTrait::list_input_channels(&mut machine);
    assert!(channels
        .iter()
        .any(|(id, c)| id == "imu" && c.key == "ax" && c.unit == "g"));
    MachineTrait::set_input_channels(
        &mut machine,
        &[
            (Some("imu"), "ax", 1.0),
            (Some("imu"), "ay", -0.5),
            (Some("imu"), "az", 0.25),
            (Some("imu"), "gx", 250.0),
            (Some("imu"), "gy", -1000.0),
            (Some("imu"), "gz", 125.0),
        ],
    )
    .unwrap();
    let driven = vec![4096, -2048, 1024, 4096, -16384, 2048];
    assert_eq!(sensor_registers(&mut machine, "imu"), driven);
    assert_eq!(sensor_registers(&mut machine, "second"), untouched);
    for invalid in [
        vec![(Some("imu"), "ax", -1.0), (Some("imu"), "missing", 0.0)],
        vec![(Some("imu"), "ax", -1.0), (Some("imu"), "ay", f64::NAN)],
        vec![(Some("missing"), "ax", 1.0)],
        vec![(None, "ax", 1.0)],
    ] {
        assert!(MachineTrait::set_input_channels(&mut machine, &invalid).is_err());
        assert_eq!(sensor_registers(&mut machine, "imu"), driven);
        assert_eq!(sensor_registers(&mut machine, "second"), untouched);
    }
    assert_eq!(
        machine.total_cycles, 0,
        "atomic writes must not execute firmware"
    );
    assert!(MachineTrait::live_numeric_capture(&machine, 0, 10).is_none());
    assert!(MachineTrait::actuator_states(&machine).is_empty());
}

#[test]
fn shared_live_capture_validation_rejects_lossy_javascript_cursors() {
    use crate::traces::live_numeric_args;
    for value in [-1.0, 0.5, f64::NAN, f64::INFINITY, 9_007_199_254_740_992.0] {
        assert!(live_numeric_args(value, 10.0).is_err());
    }
    for value in [0.0, -1.0, 0.5, f64::NAN, f64::INFINITY] {
        assert!(live_numeric_args(0.0, value).is_err());
    }
    assert_eq!(
        live_numeric_args(0.0, 9_007_199_254_740_991.0).unwrap(),
        (0, labwired_core::live_numeric::DEFAULT_LIVE_NUMERIC_POINTS)
    );
}
