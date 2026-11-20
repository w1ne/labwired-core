// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! Opt-in integration of the exact packaged physical master and two devices.
//! All bytes pass through the existing World / timed UART network. The PHY
//! supply is a declared valid 24 V scenario; analog behavior is not qualified.
use labwired_config::{ChipDescriptor, EnvironmentManifest, SystemManifest};
use labwired_core::system::node::NodeFirmware;
use labwired_core::world::{ResolvedWorldNode, StepResults, World};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
const MASTER_SHA: &str = "26a1c6fdf4303cfba634cb20d3478672e55b5b648ced67dbfdefa8361f912bd9";
const DEVICE_SHA: &str = "0e6cfe0df8a355dcf0f7508ba9ffc46e655edd5e82a1fb31227aa45a6bde8508";
const MASTER: &str = "name: physical-iom\nchip: stm32l476\ncpu_hz: 80000000\nmax_peripheral_tick_interval: 1\nexternal_devices:\n- id: port1\n  type: l6360\n  connection: i2c1\n  config: {address: 96, uart: uart1, encq: PC0, enl: PA6, reset_n: PB5, incq: PA9, outcq: PA10, irq: PA4}\n- id: port2\n  type: l6360\n  connection: i2c1\n  config: {address: 97, uart: uart4, encq: PA5, enl: PA8, reset_n: PC1, incq: PA0, outcq: PA1, irq: PB0}\n";
const ENV: &str = "schema_version: '1.0'\nname: physical-iolink-station\nnodes:\n- {id: master, system: master.yaml, firmware: iolink-master.elf}\n- {id: input1, system: input1.yaml, firmware: iolink-device.elf}\n- {id: input2, system: input2.yaml, firmware: iolink-device.elf}\ninterconnects:\n- type: uart_network\n  nodes: [master, input1, input2]\n  config:\n    topology: star\n    hub: master\n    hub_uarts: [uart1, uart4]\n    spoke_uart: uart1\n    max_quantum_us: 1\n    markers:\n    - {node: master, peripheral: gpioc, pin: 0, name: port1-ENCQ}\n    - {node: master, peripheral: gpioa, pin: 5, name: port2-ENCQ}\n    - {node: input1, peripheral: gpioc, pin: 7, name: input1-EN}\n    - {node: input2, peripheral: gpioc, pin: 7, name: input2-EN}\n";
fn words(w: &World, node: &str, addr: u32, n: usize) -> Vec<u32> {
    w.machines[node]
        .read_memory(addr, n * 4)
        .unwrap()
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn memory(w: &World, node: &str, elf: &[u8], symbol: &str, n: usize) -> Vec<u8> {
    let addr = labwired_loader::resolve_symbol_in_elf(elf, symbol)
        .unwrap_or_else(|| panic!("missing {symbol}"));
    w.machines[node].read_memory(addr, n).unwrap()
}
fn scalar(w: &World, node: &str, elf: &[u8], symbol: &str) -> u32 {
    u32::from_le_bytes(memory(w, node, elf, symbol, 4).try_into().unwrap())
}
fn snapshot(w: &World, images: &BTreeMap<String, Vec<u8>>) -> Value {
    let mut nodes = serde_json::Map::new();
    for (id, elf) in images {
        let mut values = serde_json::Map::new();
        for name in ["g_startup_error", "SystemCoreClock"] {
            values.insert(name.into(), json!(scalar(w, id, elf, name)));
        }
        if id == "master" {
            for (name, n) in [
                ("g_master_state", 2),
                ("g_master_pd", 2),
                ("g_phy_faults", 2),
            ] {
                values.insert(name.into(), json!(memory(w, id, elf, name, n)));
            }
            for name in [
                "g_phy_error",
                "g_wake_count",
                "g_uart_errors",
                "g_rx_overflows",
                "g_master_tick_result",
            ] {
                let addr = labwired_loader::resolve_symbol_in_elf(elf, name).unwrap();
                values.insert(name.into(), json!(words(w, id, addr, 2)));
            }
            values.insert("uart4_brr".into(), json!(words(w, id, 0x40004c0c, 1)));
            values.insert("i2c1_isr".into(), json!(words(w, id, 0x40005418, 1)));
        } else {
            for name in ["g_device_state", "g_device_pd"] {
                values.insert(name.into(), json!(memory(w, id, elf, name, 1)[0]));
            }
            for name in [
                "g_phy_error",
                "g_wake_events",
                "g_overload_edges",
                "g_uart_errors",
                "g_rx_overflows",
                "g_tx_deadline_errors",
            ] {
                values.insert(name.into(), json!(scalar(w, id, elf, name)));
            }
            values.insert(
                "serial".into(),
                json!(String::from_utf8(memory(w, id, elf, "serial_number", 16)).unwrap()),
            );
            values.insert("uid_fixture".into(), json!(words(w, id, 0x1fff7590, 3)));
        }
        values.insert("uart1_brr".into(), json!(words(w, id, 0x4001380c, 1)));
        values.insert("gpioa_odr".into(), json!(words(w, id, 0x48000014, 1)));
        values.insert("gpiob_odr".into(), json!(words(w, id, 0x48000414, 1)));
        values.insert("gpioc_odr".into(), json!(words(w, id, 0x48000814, 1)));
        values.insert("pc".into(), json!(w.machines[id].get_pc()));
        nodes.insert(id.clone(), Value::Object(values));
    }
    json!({"time_ps":w.uart_network_now_ps(),"nodes":nodes})
}
fn run_to(w: &mut World, t_us: u64) {
    let mut results = StepResults::new();
    let target = t_us * 1_000_000;
    while w.uart_network_now_ps().unwrap() < target {
        w.step_all_into(&mut results);
        if let Some((id, error)) = results.first_error() {
            panic!("{id}: {error:?}");
        }
    }
}
fn save(dir: &Path, stages: &[Value], w: &World) {
    std::fs::write(dir.join("result.json"),serde_json::to_vec_pretty(&json!({
        "masterSha256":MASTER_SHA,"deviceSha256":DEVICE_SHA,"cpuHz":80000000,
        "scope":"Exact physical HAL images in the existing digital World, with explicit synthetic UID fixtures and valid PHY supplies. Not physical hardware or analog qualification.",
        "stages":stages,"network":w.uart_network_report(0)
    })).unwrap()).unwrap();
}
fn device_yaml(id: &str, pattern: u8, uid: [u32; 3]) -> String {
    format!("name: physical-{id}\nchip: stm32l476\ncpu_hz: 80000000\nmax_peripheral_tick_interval: 1\nperipherals:\n- id: uid\n  type: declarative\n  base_address: 536835472\n  size: 12B\n  config:\n    path: configs/peripherals/stm32l476/uid.yaml\n    reset_values: {{UID0: {}, UID1: {}, UID2: {}}}\nexternal_devices:\n- id: phy\n  type: l6362a\n  connection: uart1\n  config: {{en: PC7, in1: low, in2: PA9, outiq: PA10, diag: PA4, ol: PA6}}\n- id: di8\n  type: sn74hc165_gpio\n  connection: gpio\n  config: {{clock_pin: PB13, data_pin: PB14, latch_pin: PC6, inputs: {pattern}}}\n", uid[0], uid[1], uid[2])
}

#[test]
#[ignore = "requires immutable packaged physical ELFs and an explicit evidence directory"]
fn unchanged_master_and_two_devices_exchange_distinct_native_inputs() {
    let dir = PathBuf::from(
        std::env::var("IOLINK_STATION_REPORT_DIR").expect("IOLINK_STATION_REPORT_DIR"),
    );
    std::fs::create_dir_all(&dir).unwrap();
    let firmware = PathBuf::from(
        std::env::var("IOLINK_STATION_FIRMWARE_DIR").expect("IOLINK_STATION_FIRMWARE_DIR"),
    );
    let master = std::fs::read(firmware.join("iolink-master.elf")).unwrap();
    let device = std::fs::read(firmware.join("iolink-device.elf")).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&master)), MASTER_SHA);
    assert_eq!(format!("{:x}", Sha256::digest(&device)), DEVICE_SHA);
    let images = BTreeMap::from([
        ("master".to_string(), master),
        ("input1".to_string(), device.clone()),
        ("input2".to_string(), device),
    ]);
    let chip = ChipDescriptor::from_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/chips/stm32l476.yaml"
    ))
    .unwrap();
    let systems = BTreeMap::from([
        ("master", MASTER.to_string()),
        (
            "input1",
            device_yaml("input1", 0xa5, [0x1001, 0x2001, 0x3001]),
        ),
        (
            "input2",
            device_yaml("input2", 0x3c, [0x1002, 0x2002, 0x3002]),
        ),
    ]);
    std::fs::write(dir.join("environment.yaml"), ENV).unwrap();
    let resolved = systems
        .iter()
        .map(|(id, yaml)| {
            std::fs::write(dir.join(format!("{id}.yaml")), yaml).unwrap();
            ResolvedWorldNode {
                id: (*id).into(),
                system: serde_yaml::from_str::<SystemManifest>(yaml).unwrap(),
                chip: chip.clone(),
                firmware: NodeFirmware::from_bytes(images[*id].clone()),
                blobs: Default::default(),
            }
        })
        .collect();
    let env: EnvironmentManifest = serde_yaml::from_str(ENV).unwrap();
    let mut w = World::from_resolved(env, resolved).unwrap();
    // Resolve/apply the real GPIO input channels before the long boot, so a
    // caller spelling error fails before producing a partial run receipt.
    for (id, pattern) in [("input1", 0xa5u8), ("input2", 0x3cu8)] {
        for i in 0..8 {
            w.machines
                .get_mut(id)
                .unwrap()
                .set_input_channel(&format!("ch{i}"), f64::from((pattern >> i) & 1))
                .unwrap();
        }
    }
    let mut stages = Vec::new();
    for t in [20_000, 100_000, 500_000, 550_000, 600_000, 650_000] {
        run_to(&mut w, t);
        let s = snapshot(&w, &images);
        eprintln!("physical station {t} us: {}", s);
        stages.push(s);
        save(&dir, &stages, &w);
        for (id, elf) in &images {
            assert_eq!(scalar(&w, id, elf, "g_startup_error"), 0, "{id} startup");
            assert_eq!(
                scalar(&w, id, elf, "SystemCoreClock"),
                80_000_000,
                "{id} clock"
            );
        }
    }
    assert_eq!(
        memory(&w, "master", &images["master"], "g_master_state", 2),
        [3, 3]
    );
    assert_eq!(
        memory(&w, "master", &images["master"], "g_master_pd", 2),
        [0xa5, 0x3c]
    );
    for id in ["input1", "input2"] {
        // iolinki/include/iolinki/dll.h: ESTAB_COM=3, OPERATE=4.
        assert_eq!(memory(&w, id, &images[id], "g_device_state", 1), [4]);
        assert_eq!(scalar(&w, id, &images[id], "g_wake_events"), 1);
        assert_eq!(
            scalar(&w, id, &images[id], "g_overload_edges"),
            1,
            "only the actual wake opposition may assert OL"
        );
        assert_eq!(scalar(&w, id, &images[id], "g_phy_error"), 0);
        assert_eq!(scalar(&w, id, &images[id], "g_uart_errors"), 0);
        assert_eq!(scalar(&w, id, &images[id], "g_rx_overflows"), 0);
        assert_eq!(scalar(&w, id, &images[id], "g_tx_deadline_errors"), 0);
    }
    assert_ne!(
        memory(&w, "input1", &images["input1"], "serial_number", 16),
        memory(&w, "input2", &images["input2"], "serial_number", 16)
    );
    for (id, pattern) in [("input1", 0x5au8), ("input2", 0xc3u8)] {
        for i in 0..8 {
            w.machines
                .get_mut(id)
                .unwrap()
                .set_input_channel(&format!("ch{i}"), f64::from((pattern >> i) & 1))
                .unwrap();
        }
    }
    run_to(&mut w, 680_000);
    stages.push(snapshot(&w, &images));
    save(&dir, &stages, &w);
    assert_eq!(
        memory(&w, "master", &images["master"], "g_master_pd", 2),
        [0x5a, 0xc3]
    );
}
