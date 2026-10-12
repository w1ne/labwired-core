//! Canonical export + normal attachment + reduced persistent plant. NO Linux guest evidence.
use labwired_config::SystemManifest;
use labwired_core::{
    bus::SystemBus,
    cosim::{CosimRunner, CosimSignalValue, CosimSignals},
    Bus, Machine,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/luckfox-pico-mini-a-linux/basket-native-runtime")
}
fn manifest() -> SystemManifest {
    SystemManifest::from_file(directory().join("system.json")).unwrap()
}
fn receipt() -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(directory().join("receipt.json")).unwrap()).unwrap()
}
fn signals() -> CosimSignals {
    receipt()["initial_signals"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, typed)| {
            let value = match typed["kind"].as_str().unwrap() {
                "Bool" => CosimSignalValue::Bool(typed["value"].as_bool().unwrap()),
                "F64" => CosimSignalValue::F64(typed["value"].as_f64().unwrap()),
                kind => panic!("unknown scalar kind {kind}"),
            };
            (key.clone(), value)
        })
        .collect()
}
#[test]
fn generated_system_manifest_roundtrips_and_hashes_match_actual_core_sources() {
    let m = manifest();
    assert!(m.validate_cosim_models().is_empty());
    assert!(m.validate_parts().is_ok());
    let roundtrip: SystemManifest =
        serde_yaml::from_str(&serde_yaml::to_string(&m).unwrap()).unwrap();
    assert_eq!(roundtrip.external_devices.len(), 2);
    assert_eq!(roundtrip.external_devices[0].id, "accelerometer");
    assert_eq!(roundtrip.external_devices[1].id, "wifi");
    let r = receipt();
    assert_eq!(
        r["circuit_sha256"],
        "38778604c2ae6b4f370de0849409abaf6e7884359b180094681ef68edfcd79f4"
    );
    assert_eq!(
        r["manifest_sha256"].as_str().unwrap(),
        format!(
            "{:x}",
            Sha256::digest(fs::read(directory().join("system.json")).unwrap())
        )
    );
    let root = directory().join(r["core_root"].as_str().unwrap());
    for source in r["core_sources"].as_array().unwrap() {
        let path = source["path"].as_str().unwrap();
        assert!(!Path::new(path).is_absolute());
        assert_eq!(
            source["sha256"].as_str().unwrap(),
            format!("{:x}", Sha256::digest(fs::read(root.join(path)).unwrap())),
            "changed {path}"
        );
    }
    assert_eq!(r["routes"]["sda"]["header_pad"], 14);
    assert_eq!(r["routes"]["scl"]["header_pad"], 15);
    assert_eq!(r["routes"]["tx"]["header_pad"], 10);
    assert_eq!(r["routes"]["rx"]["header_pad"], 11);
    assert_eq!(
        m.external_devices[0].config["i2c_address"].as_u64(),
        Some(0x53)
    );
}
#[test]
fn normal_attachment_preserves_canonical_component_ids_and_functional_supply_gate() {
    let mut b = SystemBus::empty();
    b.cpu_hz = 24_000_000;
    b.configure_armv7_rv1103().unwrap();
    b.configure_armv7_rv1103_i2c3().unwrap();
    b.configure_armv7_rv1103_uart4().unwrap();
    b.register_supply_domain("external3v3", 2.7).unwrap();
    b.attach_manifest_external_devices(&manifest()).unwrap();
    let mut machine = Machine::new(labwired_core::cpu::cortex_m::CortexM::new(), b);
    for channel in ["x", "y", "z"] {
        machine.set_input_on("accelerometer", channel, 0.5).unwrap();
    }
    assert!(machine.set_input_on("basket:adxl", "x", 1.).is_err());
    let inspected = machine.inspect(
        None,
        &labwired_core::inspect::InspectOpts {
            peripheral: None,
            include_bytes: false,
        },
    );
    assert!(inspected.devices.iter().any(|d| d.id == "accelerometer"
        && d.device_type.as_deref() == Some("adxl345")
        && d.attachment.address == Some(0x53)));
    assert!(inspected
        .devices
        .iter()
        .any(|d| d.id == "wifi" && d.device_type.as_deref() == Some("esp32c3_esp_at")));
    let initially_off_logs = machine.bus.device_logs("wifi").unwrap();
    machine.bus.write_u32(0xff4e0000, b'A' as u32).unwrap();
    assert_eq!(machine.bus.device_logs("wifi").unwrap(), initially_off_logs);
    let i = machine.bus.find_peripheral_index_by_name("uart4").unwrap();
    assert_eq!(machine.bus.peripherals[i].dev.read_u32(20).unwrap() & 1, 0);
    machine
        .bus
        .update_supply_domain("external3v3", 3.3)
        .unwrap();
    for v in b"AT\r\n" {
        machine.bus.write_u32(0xff4e0000, *v as u32).unwrap();
    }
    let mut output = Vec::new();
    for _ in 0..1100 {
        let uart = &mut machine.bus.peripherals[i].dev;
        uart.advance_attached_device_time_us(1000);
        if uart.read_u32(20).unwrap() & 1 != 0 {
            output.push(uart.read_u32(0).unwrap() as u8);
        }
    }
    assert_eq!(output, b"OK\r\n");
    let accepted_logs = machine.bus.device_logs("wifi").unwrap();
    machine.bus.update_supply_domain("external3v3", 0.).unwrap();
    for v in b"AT\r\n" {
        machine.bus.write_u32(0xff4e0000, *v as u32).unwrap();
    }
    assert_eq!(machine.bus.peripherals[i].dev.read_u32(20).unwrap() & 1, 0);
    assert_eq!(machine.bus.device_logs("wifi").unwrap(), accepted_logs);
    let inspect = machine.inspect(None, &Default::default());
    let peer = inspect.devices.iter().find(|d| d.id == "wifi").unwrap();
    assert_eq!(peer.artifacts[0].meta["powered"], false);
    assert_eq!(peer.artifacts[0].meta["command_bytes"], 0);
    assert_eq!(peer.artifacts[0].meta["reply_bytes"], 0);
}
#[test]
fn one_existing_persistent_plant_uses_typed_canonical_inputs_and_preserves_battery_state() {
    let m = manifest();
    let mut runner = CosimRunner::from_configs_with_base(&m.cosim_models, &directory()).unwrap();
    let mut s = signals();
    assert_eq!(s["basket.config.initial_soc"], CosimSignalValue::F64(0.8));
    assert_eq!(
        s["basket.config.capacity_mah"],
        CosimSignalValue::F64(2000.)
    );
    assert_eq!(
        s["basket.config.charge_limit_a"],
        CosimSignalValue::F64(0.445)
    );
    assert_eq!(s["basket.enable"], CosimSignalValue::Bool(true));
    runner.step_until_with_signals(100_000_000, &mut s).unwrap();
    let scalar = |s: &CosimSignals, key: &str| match s[key] {
        CosimSignalValue::F64(n) => n,
        CosimSignalValue::I64(n) => n as f64,
        _ => panic!("expected numeric {key}"),
    };
    let soc = scalar(&s, "basket.power.soc");
    assert!(soc < 0.8 && soc > 0.79);
    assert_eq!(scalar(&s, "basket.power.rail_5v"), 5.);
    assert_eq!(scalar(&s, "basket.power.rail_3v3"), 3.3);
    s.insert("basket.fault_short".into(), CosimSignalValue::Bool(true));
    runner.step_until_with_signals(200_000_000, &mut s).unwrap();
    assert_eq!(
        s["basket.power.cutoff_latched"],
        CosimSignalValue::Bool(true)
    );
    assert_eq!(scalar(&s, "basket.power.rail_3v3"), 0.);
    s.insert("basket.fault_short".into(), CosimSignalValue::Bool(false));
    runner.step_until_with_signals(300_000_000, &mut s).unwrap();
    assert_eq!(
        s["basket.power.cutoff_latched"],
        CosimSignalValue::Bool(true)
    );
    s.insert("basket.reset".into(), CosimSignalValue::Bool(true));
    runner.step_until_with_signals(400_000_000, &mut s).unwrap();
    assert_eq!(
        s["basket.power.cutoff_latched"],
        CosimSignalValue::Bool(false)
    );
    assert_eq!(scalar(&s, "basket.power.rail_3v3"), 3.3);
    s.insert("basket.reset".into(), CosimSignalValue::Bool(false));
    runner.step_until_with_signals(500_000_000, &mut s).unwrap();
    assert!(scalar(&s, "basket.power.soc") < soc);
    assert_eq!(runner.model_count(), 1);
}
