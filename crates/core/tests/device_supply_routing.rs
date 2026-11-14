//! Route actual measured rails into native SPI power gates. Tiny Cortex-M
//! loops witness world scheduling; this does not qualify native FC firmware.
use labwired_config::{ChipDescriptor, CosimModelConfig, EnvironmentManifest, SystemManifest};
use labwired_core::cosim::{CosimSignalValue, CosimSignals, SignalPath, SignalRouter};
use labwired_core::peripherals::device::SpiDevice;
use labwired_core::system::node::NodeFirmware;
use labwired_core::world::{ResolvedWorldNode, World};
use labwired_core::{bus::SystemBus, inspect::InspectOpts};
use std::path::Path;

const RAILS: &str = "{vdd: {min_v: 1.71, max_v: 3.6}, vddio: {min_v: 1.2, max_v: 3.6}}";

fn firmware() -> Vec<u8> {
    // ELF32 ARM, vector table, ADD r0,#1; B loop. The register witnesses hold.
    let code = [0x3001u16, 0xe7fd];
    let length = 8 + code.len() as u32 * 2;
    let mut bytes = vec![0; 84 + length as usize];
    bytes[0..7].copy_from_slice(b"\x7fELF\x01\x01\x01");
    for (offset, value) in [(16, 2u16), (18, 40), (40, 52), (42, 32), (44, 1)] {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    for (offset, value) in [
        (20, 1u32),
        (24, 0x08000009),
        (28, 52),
        (36, 0x05000000),
        (52, 1),
        (56, 84),
        (60, 0x08000000),
        (64, 0x08000000),
        (68, length),
        (72, length),
        (76, 5),
        (80, 4),
        (84, 0x20001000),
        (88, 0x08000009),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (i, instruction) in code.iter().enumerate() {
        bytes[92 + i * 2..94 + i * 2].copy_from_slice(&instruction.to_le_bytes());
    }
    bytes
}

fn chip() -> ChipDescriptor {
    ChipDescriptor::from_file(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/stm32f401.yaml"),
    )
    .unwrap()
}
fn system(id: &str) -> SystemManifest {
    serde_yaml::from_str(&format!("name: routed-device-power\nchip: stm32f401\nexternal_devices:\n  - id: {id:?}\n    type: bmi270_spi\n    connection: spi1\n    config: {{cs_pin: PA4, supply_rails: {RAILS}}}\n")).unwrap()
}
fn route(id: &str, rail: &str) -> String {
    format!("board.device.{id}.power.{rail}_volts")
}
fn model(id: &str) -> CosimModelConfig {
    serde_yaml::from_str(&format!(
        "id: rails\nadapter: mock\nstep_ns: 1000\noutputs:\n  vdd: {}\n  vddio: {}\n",
        route(id, "vdd"),
        route(id, "vddio")
    ))
    .unwrap()
}
fn bus(id: &str) -> SystemBus {
    SystemBus::from_config(&chip(), &system(id)).unwrap()
}
fn signals(id: &str, vdd: CosimSignalValue, vddio: CosimSignalValue) -> CosimSignals {
    [(route(id, "vdd"), vdd), (route(id, "vddio"), vddio)]
        .into_iter()
        .collect()
}
fn slave(bus: &mut SystemBus) -> &mut dyn SpiDevice {
    let index = bus.find_peripheral_index_by_name("spi1").unwrap();
    bus.peripherals[index]
        .dev
        .spi_attached_devices_mut()
        .expect("spi1 exposes its attached devices")[0]
        .as_mut()
}
fn meta(bus: &mut SystemBus, id: &str) -> serde_json::Value {
    slave(bus).artifacts(id, &InspectOpts::default())[0]
        .meta
        .clone()
}
fn read(bus: &mut SystemBus, register: u8) -> u8 {
    let device = slave(bus);
    device.cs_select();
    device.transfer(register | 0x80);
    device.transfer(0);
    let value = device.transfer(0);
    device.cs_release();
    value
}

#[test]
fn device_power_paths_resolve_exact_id_and_declared_rails() {
    let id = "sensor.power.U9"; // Existing device IDs are exact, not reinterpreted path segments.
    assert_eq!(
        SignalPath::parse(&route(id, "vdd")),
        Some(SignalPath::DeviceSupply {
            device: id.into(),
            rail: "vdd".into()
        })
    );
    let mut bus = bus(id);
    let (mut router, errors) = SignalRouter::bind(&[model(id)], &bus);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(router
        .apply(
            &mut bus,
            &signals(id, CosimSignalValue::F64(1.8), CosimSignalValue::F64(3.3)),
            0
        )
        .is_empty());
    assert_eq!(meta(&mut bus, id)["powered"], true);
    for path in [
        "board.device.missing.power.vdd_volts",
        "board.device.sensor.power.U9.power.missing_volts",
        "board.device.U9.power",
        "board.device..power.vdd_volts",
        "board.device.U9.power._volts",
    ] {
        let mut config = model(id);
        config.outputs.insert("vdd".into(), path.into());
        let (_, errors) = SignalRouter::bind(&[config], &bus);
        assert!(!errors.is_empty(), "{path}");
    }
    let mut config = model(id);
    config
        .inputs
        .insert("illegal_read".into(), route(id, "vdd"));
    let (_, errors) = SignalRouter::bind(&[config], &bus);
    assert!(!errors.is_empty());
}

#[test]
fn duplicate_power_producers_are_refused_before_sink_deduplication() {
    let bus = bus("U9");
    let mut config = model("U9");
    config.outputs.insert("alias".into(), route("U9", "vdd"));
    let (_, errors) = SignalRouter::bind(&[config], &bus);
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("2 model output writers")),
        "{errors:?}"
    );
    let mut second = model("U9");
    second.id = "second".into();
    let (_, errors) = SignalRouter::bind(&[model("U9"), second], &bus);
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("2 model output writers")),
        "{errors:?}"
    );
}

#[test]
fn malformed_boundary_values_do_not_partially_cut_device_power() {
    let mut bus = bus("U9");
    let (mut router, errors) = SignalRouter::bind(&[model("U9")], &bus);
    assert!(errors.is_empty());
    assert!(router
        .apply(
            &mut bus,
            &signals("U9", CosimSignalValue::F64(1.8), CosimSignalValue::F64(3.3)),
            0
        )
        .is_empty());
    let before = meta(&mut bus, "U9");
    for invalid in [
        CosimSignalValue::Bool(true),
        CosimSignalValue::Text("3.3".into()),
        CosimSignalValue::F64(-1.0),
        CosimSignalValue::F64(f64::NAN),
        CosimSignalValue::F64(f64::INFINITY),
    ] {
        assert!(!router
            .apply(
                &mut bus,
                &signals("U9", CosimSignalValue::F64(0.0), invalid),
                1000
            )
            .is_empty());
        assert_eq!(
            meta(&mut bus, "U9"),
            before,
            "the other valid rail must not commit on failure"
        );
    }
    // Missing observations retain their last value, as the bus batch contract specifies.
    assert!(router
        .apply(
            &mut bus,
            &[(route("U9", "vdd"), CosimSignalValue::F64(3.3))]
                .into_iter()
                .collect(),
            2000
        )
        .is_empty());
    assert_eq!(meta(&mut bus, "U9")["powered"], true);
    assert_eq!(
        meta(&mut bus, "U9")["supply_rails"]["vddio"]["voltage_v"],
        3.3
    );
}

#[test]
fn routed_power_blocks_spi_transfers_and_clocks_until_both_rails_qualify() {
    let mut bus = bus("U9");
    let (mut router, errors) = SignalRouter::bind(&[model("U9")], &bus);
    assert!(errors.is_empty());
    let before = meta(&mut bus, "U9");
    assert_eq!(read(&mut bus, 0), 0xff);
    slave(&mut bus).advance_time_us(100_000);
    assert_eq!(
        meta(&mut bus, "U9")["model_time_ns"],
        before["model_time_ns"]
    );
    assert!(router
        .apply(
            &mut bus,
            &signals("U9", CosimSignalValue::F64(1.8), CosimSignalValue::F64(3.3)),
            0
        )
        .is_empty());
    slave(&mut bus).cs_select();
    slave(&mut bus).cs_release();
    slave(&mut bus).advance_time_us(449);
    assert_eq!(read(&mut bus, 0), 0xff);
    slave(&mut bus).advance_time_us(1);
    assert_eq!(read(&mut bus, 0), 0x24);
    assert!(router
        .apply(
            &mut bus,
            &signals("U9", CosimSignalValue::F64(1.8), CosimSignalValue::F64(0.0)),
            1000
        )
        .is_empty());
    let off = meta(&mut bus, "U9");
    assert_eq!(read(&mut bus, 0), 0xff);
    slave(&mut bus).advance_time_us(100_000);
    assert_eq!(meta(&mut bus, "U9")["model_time_ns"], off["model_time_ns"]);
    assert_eq!(meta(&mut bus, "U9")["accel_samples"], 0);
}

fn environment(dynamic: bool, vdd: f64, vddio: f64) -> EnvironmentManifest {
    let mut inputs = String::new();
    let mut sources = String::new();
    let mut outputs = String::new();
    let mut probes = String::new();
    let mut netlist = String::new();
    for node in ["a", "b"] {
        for (rail, value) in [("vdd", vdd), ("vddio", vddio)] {
            let name = format!("{node}_{rail}");
            netlist.push_str(&format!("        V{name} {name} 0 dc {value}\n"));
            outputs.push_str(&format!(
                "      {name}: node.{node}.{}\n",
                route("U9", rail)
            ));
            probes.push_str(&format!("        {name}: v({name})\n"));
            if dynamic {
                inputs.push_str(&format!("      {name}: ui.{node}.{rail}\n"));
                sources.push_str(&format!("        {name}: V{name}\n"));
            }
        }
    }
    let inputs = if dynamic {
        format!("    inputs:\n{inputs}")
    } else {
        String::new()
    };
    let sources = if dynamic {
        format!("      sources:\n{sources}")
    } else {
        String::new()
    };
    serde_yaml::from_str(&format!("name: measured-device-rails\nnodes:\n  - {{id: a, system: a.yaml, firmware: a.elf}}\n  - {{id: b, system: b.yaml, firmware: b.elf}}\ncosim_models:\n  - id: circuit\n    adapter: analog\n    step_ns: 1000\n{inputs}    outputs:\n{outputs}    config:\n      substeps: 1\n      netlist_text: |\n{netlist}{sources}      probes:\n{probes}")).unwrap()
}
fn from_environment(manifest: EnvironmentManifest) -> anyhow::Result<World> {
    let nodes = ["a", "b"]
        .into_iter()
        .map(|node| ResolvedWorldNode {
            id: node.into(),
            system: system("U9"),
            chip: chip(),
            firmware: NodeFirmware::from_bytes(firmware()),
            blobs: Default::default(),
        })
        .collect();
    World::from_resolved(manifest, nodes)
}
fn world_meta(world: &World, node: &str) -> serde_json::Value {
    world.machines[node]
        .inspect_devices(&InspectOpts::default())
        .into_iter()
        .find(|device| device.id == "U9" && device.declared)
        .expect("declared native SPI sensor is inspectable")
        .artifacts
        .into_iter()
        .find(|artifact| artifact.meta.get("powered").is_some())
        .expect("native sensor reports measured power state")
        .meta
}
fn set(world: &mut World, node: &str, vdd: f64, vddio: f64) {
    for (rail, value) in [("vdd", vdd), ("vddio", vddio)] {
        world
            .set_cosim_signal(&format!("ui.{node}.{rail}"), CosimSignalValue::F64(value))
            .unwrap();
    }
}
fn run(world: &mut World, time_ns: u64) {
    let results = world
        .run_until_ps(time_ns * 1000)
        .expect("analog world advances");
    assert!(results.values().all(Result::is_ok), "{results:?}");
    assert_eq!(world.cosim_status().unwrap().completed_time_ns, time_ns);
}

#[test]
fn world_prime_publishes_actual_rails_before_any_cpu_instruction() {
    let world = from_environment(environment(false, 1.8, 3.3)).unwrap();
    assert_eq!(world.cosim_status().unwrap().completed_time_ns, 0);
    for node in ["a", "b"] {
        let state = world_meta(&world, node);
        assert_eq!(world.machines[node].get_register(0), 0);
        assert_eq!(state["powered"], true);
        assert_eq!(state["supply_generation"], 1);
        assert_eq!(state["supply_rails"]["vdd"]["voltage_v"], 1.8);
        assert_eq!(state["supply_rails"]["vddio"]["voltage_v"], 3.3);
        assert_eq!(state["model_time_ns"], 0);
    }
}

#[test]
fn world_atomic_rail_swaps_have_no_intermediate_cycles_or_cross_node_aliases() {
    let mut world = from_environment(environment(true, 0.0, 0.0)).unwrap();
    set(&mut world, "a", 0.0, 3.3);
    run(&mut world, 1000);
    set(&mut world, "a", 1.8, 0.0);
    run(&mut world, 2000);
    assert_eq!(world_meta(&world, "a")["powered"], false);
    assert_eq!(world_meta(&world, "a")["supply_generation"], 0);
    set(&mut world, "a", 1.8, 3.3);
    run(&mut world, 3000);
    assert_eq!(world_meta(&world, "a")["supply_generation"], 1);
    set(&mut world, "a", 3.3, 1.8);
    run(&mut world, 4000);
    assert_eq!(world_meta(&world, "a")["powered"], true);
    assert_eq!(world_meta(&world, "a")["supply_generation"], 1);
    assert_eq!(world_meta(&world, "b")["powered"], false);
    assert_eq!(world_meta(&world, "b")["supply_generation"], 0);
}

#[test]
fn world_cpu_and_analog_time_advance_while_off_device_sample_clock_stays_parked() {
    let mut world = from_environment(environment(true, 0.0, 0.0)).unwrap();
    run(&mut world, 50_000);
    assert!(world.machines["a"].get_register(0) > 0);
    assert_eq!(world_meta(&world, "a")["model_time_ns"], 0);
    assert_eq!(world_meta(&world, "a")["accel_samples"], 0);
    set(&mut world, "a", 1.8, 3.3);
    run(&mut world, 100_000);
    assert!(world_meta(&world, "a")["model_time_ns"].as_u64().unwrap() > 0);
    assert_eq!(world_meta(&world, "b")["model_time_ns"], 0);
    set(&mut world, "a", 1.8, 0.0);
    run(&mut world, 101_000);
    let parked = world_meta(&world, "a")["model_time_ns"].clone();
    run(&mut world, 150_000);
    assert_eq!(world_meta(&world, "a")["model_time_ns"], parked);
}

#[test]
fn world_unknown_and_duplicate_device_supply_routes_fail_at_construction() {
    let mut unknown = environment(false, 1.8, 3.3);
    unknown.cosim_models[0].outputs.insert(
        "a_vdd".into(),
        "node.a.board.device.U9.power.unknown_volts".into(),
    );
    assert!(from_environment(unknown)
        .err()
        .unwrap()
        .to_string()
        .contains("no supply rail"));
    let mut duplicate = environment(false, 1.8, 3.3);
    duplicate.cosim_models[0].outputs.insert(
        "alias".into(),
        "node.a.board.device.U9.power.vdd_volts".into(),
    );
    assert!(from_environment(duplicate).is_err());
}
