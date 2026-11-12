//! Generic multi-rail admission, atomic boundary publication and native SPI
//! power-cycle behavior. No damage, inrush or analog load claim.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{bus::SystemBus, inspect::InspectOpts, peripherals::device::SpiDevice, Bus};
use std::path::PathBuf;

const RAILS: &str = "{vdd: {min_v: 1.71, max_v: 3.6}, vddio: {min_v: 1.2, max_v: 3.6}}";
const IMAGE: &[u8] = include_bytes!("fixtures/bmi270/maximum-fifo.bin");

fn create(config: &str, kind: &str) -> anyhow::Result<SystemBus> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip = ChipDescriptor::from_file(root.join("configs/chips/stm32l476.yaml"))?;
    let system: SystemManifest = serde_yaml::from_str(&format!(
        "name: device-power\nchip: stm32l476rg\nexternal_devices:\n  - id: U9\n    type: {kind}\n    connection: spi1\n    config: {config}\n"
    ))?;
    let mut bus = SystemBus::from_config(&chip, &system)?;
    bus.write_u32(0x4002_104c, 1)?;
    Ok(bus)
}
fn routed() -> SystemBus {
    create(
        &format!("{{cs_pin: PA4, supply_rails: {RAILS}}}"),
        "bmi270_spi",
    )
    .unwrap()
}
fn slave(bus: &mut SystemBus) -> &mut dyn SpiDevice {
    let i = bus.find_peripheral_index_by_name("spi1").unwrap();
    let spi = bus.peripherals[i]
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<labwired_core::peripherals::spi::Spi>()
        .unwrap();
    spi.attached_devices[0].as_mut()
}
fn meta(bus: &mut SystemBus) -> serde_json::Value {
    slave(bus).artifacts("U9", &InspectOpts::default())[0]
        .meta
        .clone()
}
fn apply(bus: &mut SystemBus, updates: &[(&str, f64)]) {
    let values = updates
        .iter()
        .map(|(rail, v)| ("U9".into(), (*rail).into(), *v))
        .collect::<Vec<_>>();
    bus.apply_device_supply_voltages(&values).unwrap();
}
fn read(bus: &mut SystemBus, reg: u8) -> u8 {
    let d = slave(bus);
    d.cs_select();
    d.transfer(reg | 0x80);
    d.transfer(0);
    let result = d.transfer(0);
    d.cs_release();
    result
}
fn write(bus: &mut SystemBus, reg: u8, data: &[u8]) {
    let d = slave(bus);
    d.cs_select();
    d.transfer(reg);
    for value in data {
        d.transfer(*value);
    }
    d.cs_release();
}
fn select(bus: &mut SystemBus) {
    let d = slave(bus);
    d.cs_select();
    d.cs_release();
    d.advance_time_us(450);
    assert_eq!(read(bus, 0), 0x24);
}
fn initialize(bus: &mut SystemBus) {
    select(bus);
    write(bus, 0x7c, &[0]);
    slave(bus).advance_time_us(450);
    write(bus, 0x59, &[0]);
    write(bus, 0x5e, IMAGE);
    write(bus, 0x59, &[1]);
    slave(bus).advance_time_us(20_000);
    assert_eq!(read(bus, 0x21), 1);
}

#[test]
fn unknown_and_partial_rails_cannot_power_the_native_spi_device() {
    let mut bus = routed();
    assert_eq!(meta(&mut bus)["supply_rails"]["vdd"]["state"], "unknown");
    assert_eq!(read(&mut bus, 0), 0xff);
    slave(&mut bus).advance_time_us(50_000);
    assert_eq!(meta(&mut bus)["accel_samples"], 0);
    apply(&mut bus, &[("vdd", 1.8)]);
    assert_eq!(meta(&mut bus)["powered"], false);
    assert_eq!(
        meta(&mut bus)["supply_rails"]["vddio"]["voltage_v"],
        serde_json::Value::Null
    );
    assert_eq!(read(&mut bus, 0), 0xff);
    apply(&mut bus, &[("vddio", 3.3)]);
    assert_eq!(meta(&mut bus)["powered"], true);
    let d = slave(&mut bus);
    d.cs_select();
    d.cs_release();
    d.advance_time_us(449);
    assert_eq!(read(&mut bus, 0), 0xff);
    slave(&mut bus).advance_time_us(1);
    assert_eq!(read(&mut bus, 0), 0x24);
}

#[test]
fn inclusive_bounds_and_both_rails_gate_operation_without_a_damage_claim() {
    let mut bus = routed();
    for (vdd, vddio, on) in [
        (1.71, 1.2, true),
        (3.6, 3.6, true),
        (1.709, 3.3, false),
        (1.8, 1.199, false),
        (3.601, 3.3, false),
        (1.8, 3.601, false),
    ] {
        apply(&mut bus, &[("vdd", vdd), ("vddio", vddio)]);
        assert_eq!(meta(&mut bus)["powered"], on, "VDD={vdd}, VDDIO={vddio}");
        if !on {
            assert_eq!(read(&mut bus, 0), 0xff);
        }
    }
    assert_eq!(
        meta(&mut bus)["supply_rails"]["vddio"]["state"],
        "out_of_range"
    );
    assert!(meta(&mut bus)["supply_scope"]
        .as_str()
        .unwrap()
        .contains("no damage"));
}

#[test]
fn a_rail_swap_is_atomic_and_does_not_invent_an_intermediate_power_cycle() {
    let mut bus = routed();
    apply(&mut bus, &[("vdd", 0.0), ("vddio", 3.3)]);
    assert_eq!(meta(&mut bus)["supply_generation"], 0);
    // Applying VDD first in isolation would briefly turn the part on.
    apply(&mut bus, &[("vdd", 1.8), ("vddio", 0.0)]);
    assert_eq!(meta(&mut bus)["powered"], false);
    assert_eq!(meta(&mut bus)["supply_generation"], 0);
    apply(&mut bus, &[("vddio", 3.3)]);
    initialize(&mut bus);
    let epoch = meta(&mut bus)["supply_generation"].clone();
    apply(&mut bus, &[("vdd", 3.3), ("vddio", 1.8)]);
    assert_eq!(meta(&mut bus)["supply_generation"], epoch);
    assert_eq!(
        read(&mut bus, 0x21),
        1,
        "healthy rail updates retain initialization"
    );
}

#[test]
fn an_invalid_batch_changes_neither_rail_state_nor_sensor_configuration() {
    let mut bus = routed();
    apply(&mut bus, &[("vdd", 1.8), ("vddio", 3.3)]);
    initialize(&mut bus);
    let before = meta(&mut bus);
    for invalid in [
        ("missing", "vdd", 0.0),
        ("U9", "missing", 0.0),
        ("U9", "vddio", -1.0),
        ("U9", "vddio", f64::NAN),
        ("U9", "vddio", f64::INFINITY),
        ("U9", "vdd", 1.8),
    ] {
        let batch = vec![
            ("U9".into(), "vdd".into(), 0.0),
            (invalid.0.into(), invalid.1.into(), invalid.2),
        ];
        assert!(bus.apply_device_supply_voltages(&batch).is_err());
        assert_eq!(meta(&mut bus), before, "rejected batch must be atomic");
    }
    assert!(bus.validate_device_supply_route("U9", "vdd").is_ok());
    assert!(bus.validate_device_supply_route("U9", "vcc").is_err());
    assert!(bus.validate_device_supply_route("other", "vdd").is_err());
    bus.apply_device_supply_voltages(&[]).unwrap();
    assert_eq!(meta(&mut bus), before);
}

#[test]
fn cutoff_and_restore_reset_real_config_ram_before_any_controller_poll() {
    let mut bus = routed();
    apply(&mut bus, &[("vdd", 1.8), ("vddio", 3.3)]);
    initialize(&mut bus);
    write(&mut bus, 0x7d, &[4]);
    slave(&mut bus).advance_time_us(10_000);
    assert_eq!(meta(&mut bus)["accel_samples"], 1);
    apply(&mut bus, &[("vddio", 0.0)]);
    let off = meta(&mut bus); // no bus tick or SPI access between cutoff and inspect
    assert_eq!(off["powered"], false);
    assert_eq!(off["init_status"], 0);
    assert_eq!(off["initialized_bytes"], 0);
    assert_eq!(off["accel_samples"], 0);
    slave(&mut bus).advance_time_us(100_000);
    assert_eq!(meta(&mut bus)["accel_samples"], 0);
    apply(&mut bus, &[("vddio", 3.3)]);
    assert_eq!(meta(&mut bus)["init_status"], 0);
    select(&mut bus);
    assert_eq!(read(&mut bus, 0x21), 0);
    initialize(&mut bus);
}

#[test]
fn resets_preserve_unsupported_transaction_evidence() {
    let mut bus = routed();
    apply(&mut bus, &[("vdd", 1.8), ("vddio", 3.3)]);
    initialize(&mut bus);
    assert_eq!(
        meta(&mut bus)["unsupported_requests"],
        serde_json::json!([])
    );
    read(&mut bus, 0x26); // FIFO data register, outside the sampled-data scope
    let evidence = meta(&mut bus)["unsupported_requests"].clone();
    assert!(evidence
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "FIFO readout"));
    write(&mut bus, 0x7e, &[0xb6]);
    assert_eq!(meta(&mut bus)["unsupported_requests"], evidence);
    apply(&mut bus, &[("vdd", 0.0)]);
    apply(&mut bus, &[("vdd", 1.8)]);
    assert_eq!(meta(&mut bus)["unsupported_requests"], evidence);
}

#[test]
fn static_off_and_legacy_implicit_power_remain_distinct() {
    let mut off = create(
        &format!("{{cs_pin: PA4, powered: false, supply_rails: {RAILS}}}"),
        "bmi270_spi",
    )
    .unwrap();
    apply(&mut off, &[("vdd", 1.8), ("vddio", 3.3)]);
    assert_eq!(meta(&mut off)["powered"], false);
    assert_eq!(read(&mut off, 0), 0xff);
    let mut legacy = create("{cs_pin: PA4}", "bmi270_spi").unwrap();
    select(&mut legacy);
    assert!(!meta(&mut legacy)
        .as_object()
        .unwrap()
        .contains_key("supply_rails"));
    assert!(legacy.validate_device_supply_route("U9", "vdd").is_err());
}

#[test]
fn malformed_ranges_and_spi_models_without_reset_capability_are_refused() {
    let mut cases = vec![
        "null".into(),
        "[]".into(),
        "{}".into(),
        "false".into(),
        "{vdd: {min_v: 1.71}}".into(),
        "{vdd: {max_v: 3.6}}".into(),
        "{vdd: {min_v: 0, max_v: 3.6}}".into(),
        "{vdd: {min_v: -1, max_v: 3.6}}".into(),
        "{vdd: {min_v: 3.6, max_v: 1.71}}".into(),
        "{vdd: {min_v: .nan, max_v: 3.6}}".into(),
        "{vdd: {min_v: 1.71, max_v: .inf}}".into(),
        "{vdd: {min_v: '1.71', max_v: 3.6}}".into(),
        "{vdd: {min_v: 1.71, max_v: 3.6, guess: true}}".into(),
        "{'': {min_v: 1, max_v: 3.6}}".into(),
        "{'a.b': {min_v: 1, max_v: 3.6}}".into(),
    ];
    cases.push(format!(
        "{{{}}}",
        (0..9)
            .map(|n| format!("v{n}: {{min_v: 1, max_v: 3.6}}"))
            .collect::<Vec<_>>()
            .join(",")
    ));
    for rails in cases {
        assert!(
            create(
                &format!("{{cs_pin: PA4, supply_rails: {rails}}}"),
                "bmi270_spi"
            )
            .is_err(),
            "{rails}"
        );
    }
    for extra in [
        "supply_domain: old",
        "supply_min_v: 1.0",
        "powered: 'false'",
    ] {
        assert!(create(
            &format!("{{cs_pin: PA4, supply_rails: {RAILS}, {extra}}}"),
            "bmi270_spi"
        )
        .is_err());
    }
    let err = create(
        &format!("{{cs_pin: PA4, supply_rails: {RAILS}}}"),
        "max31855",
    )
    .err()
    .unwrap();
    assert!(format!("{err:#}").contains("does not support routed SPI power"));
}

#[test]
fn required_model_rails_cannot_be_omitted_or_given_wider_operating_limits() {
    for rails in [
        "{vdd: {min_v: 1.71, max_v: 3.6}}",
        "{vdd: {min_v: 1.71, max_v: 3.6}, wrong: {min_v: 1.2, max_v: 3.6}}",
        "{vdd: {min_v: 1.0, max_v: 3.6}, vddio: {min_v: 1.2, max_v: 3.6}}",
        "{vdd: {min_v: 1.71, max_v: 5.0}, vddio: {min_v: 1.2, max_v: 3.6}}",
        "{vdd: {min_v: 1.71, max_v: 3.6}, vddio: {min_v: 1.0, max_v: 3.6}}",
    ] {
        assert!(
            create(
                &format!("{{cs_pin: PA4, supply_rails: {rails}}}"),
                "bmi270_spi"
            )
            .is_err(),
            "{rails}"
        );
    }
    let mut bus = create("{cs_pin: PA4, supply_rails: {vdd: {min_v: 1.8, max_v: 3.0}, vddio: {min_v: 1.5, max_v: 3.3}}}", "bmi270_spi").unwrap();
    apply(&mut bus, &[("vdd", 1.8), ("vddio", 3.3)]);
    select(&mut bus);
    apply(&mut bus, &[("vdd", 3.1)]);
    assert_eq!(
        meta(&mut bus)["powered"],
        false,
        "a declared narrower window is enforced"
    );
}

#[test]
fn power_cutoff_parks_the_real_mcu_interrupt_at_the_same_boundary() {
    let mut bus = create(
        &format!("{{cs_pin: PA4, int1_pin: PA0, supply_rails: {RAILS}}}"),
        "bmi270_spi",
    )
    .unwrap();
    apply(&mut bus, &[("vdd", 1.8), ("vddio", 3.3)]);
    initialize(&mut bus);
    write(&mut bus, 0x53, &[0x0a]);
    write(&mut bus, 0x55, &[1]);
    write(&mut bus, 0x58, &[4]);
    write(&mut bus, 0x7d, &[4]);
    slave(&mut bus).advance_time_us(10_000);
    bus.tick_peripherals();
    let (addr, bit) = SystemBus::resolve_pin_idr_pub(&bus, "PA0").unwrap();
    assert_ne!(bus.read_u32(addr).unwrap() & (1 << bit), 0);
    apply(&mut bus, &[("vdd", 0.0)]);
    assert_eq!(
        bus.read_u32(addr).unwrap() & (1 << bit),
        0,
        "power publication must deliver the inactive INT without another tick"
    );
    apply(&mut bus, &[("vdd", 1.8)]);
    slave(&mut bus).advance_time_us(100_000);
    bus.tick_peripherals();
    assert_eq!(bus.read_u32(addr).unwrap() & (1 << bit), 0);
    assert_eq!(meta(&mut bus)["init_status"], 0);
}
