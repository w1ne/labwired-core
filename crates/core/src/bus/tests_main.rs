// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Unit tests for [`crate::bus::SystemBus`] (split out of bus/mod.rs).

use super::*;
use labwired_config::{
    Access, ChipDescriptor, PeripheralDescriptor, RegisterDescriptor, SystemManifest, TimingAction,
    TimingDescriptor, TimingTrigger,
};
use std::path::PathBuf;

#[test]
fn timer_poll_coalesce_uses_peripheral_access_class_not_chip_names() {
    let mut bus = SystemBus::new();
    // Systimer model owns ESP register map; bus only sees MmioAccessClass.
    bus.add_peripheral(
        "systimer",
        0x6002_3000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::systimer::Systimer::new(
            160_000_000,
        )),
    );
    // Default StubPeripheral is SideEffecting (CPU-agnostic default).
    bus.add_peripheral(
        "gpio",
        0x6000_4000,
        0x1000,
        None,
        Box::new(crate::peripherals::stub::StubPeripheral::new(0x1000)),
    );
    let sys_idx = bus.find_peripheral_index_by_name("systimer").unwrap();
    let gpio_idx = bus.find_peripheral_index_by_name("gpio").unwrap();

    bus.reset_mmio_activity_counters();
    bus.note_mmio_activity(sys_idx, 0x04); // poll class (model decides)
    bus.note_mmio_activity(sys_idx, 0x44); // poll class
    assert!(
        bus.take_timer_poll_coalesce_eligible(),
        "pure freerunning-timer polls should coalesce"
    );

    bus.reset_mmio_activity_counters();
    bus.note_mmio_activity(sys_idx, 0x04);
    bus.note_mmio_activity(sys_idx, 0x44);
    bus.note_mmio_activity(gpio_idx, 0x00); // side-effecting
    assert!(!bus.take_timer_poll_coalesce_eligible());
}

/// `max_safe_tick_interval`: 1 while the legacy walk is live (the default
/// bus), the batching recommendation once the walk is deleted, and back to
/// 1 when a non-relaxable device (test-only HC-SR04 legacy pin) is present.
#[test]
fn max_safe_tick_interval_relaxes_only_walk_deleted_buses() {
    let mut bus = SystemBus::new();
    assert_eq!(bus.max_safe_tick_interval(), 1, "legacy walk → stay exact");

    bus.legacy_walk_disabled = true;
    let relaxed = bus.max_safe_tick_interval();
    if cfg!(feature = "event-scheduler") {
        assert_eq!(relaxed, RECOMMENDED_TICK_INTERVAL);
    } else {
        assert_eq!(relaxed, 1, "feature-off builds never batch");
    }

    bus.hcsr04.push(crate::peripherals::hc_sr04::HcSr04::new(
        "dist".into(),
        0x4800_0014,
        0,
        0x4800_0010,
        1,
        1_000_000,
        100.0,
    ));
    assert_eq!(
        bus.max_safe_tick_interval(),
        relaxed,
        "HC-SR04 is relaxable (its edges become scheduler events)"
    );
    bus.hcsr04_scheduling_disabled = true;
    assert_eq!(
        bus.max_safe_tick_interval(),
        1,
        "the test-only legacy-pin override must force interval 1"
    );
}

/// `derive_walk_deletable`: an all-scheduler / inert bus derives deletion;
/// a single walk-dependent peripheral (native Timer, or an unknown model
/// keeping the conservative default) pins the walk on.
#[test]
fn derive_walk_deletable_is_conservative() {
    use crate::peripherals::spi::{Spi, SpiRegisterLayout};
    use crate::peripherals::stub::StubPeripheral;
    use crate::peripherals::timer::Timer;

    // Start from a truly empty peripheral set (`new()` pre-populates a few).
    let mut bus = SystemBus::new();
    bus.peripherals.clear();
    assert!(bus.derive_walk_deletable(), "empty bus derives deletion");

    // Scheduler-driven (SPI) + inert stub: still deletable.
    bus.add_peripheral(
        "spi1",
        0x4001_3000,
        0x400,
        None,
        Box::new(Spi::new_with_layout(SpiRegisterLayout::Stm32)),
    );
    bus.add_peripheral(
        "syscfg",
        0x4001_0000,
        0x400,
        None,
        Box::new(StubPeripheral::new(0)),
    );
    assert!(
        bus.derive_walk_deletable(),
        "scheduler + inert-stub bus is walk-independent"
    );

    // Add a native Timer pinned to legacy mode (its `tick()` counts once
    // CEN is set — walk work reachable via MMIO). The bus must NOT derive
    // deletion. (`add_peripheral` attaches the cycle clock, which under
    // `event-scheduler` migrates the timer to the scheduler — detach it
    // so this test keeps pinning the conservative legacy-walker default.)
    bus.add_peripheral("tim2", 0x4000_0000, 0x400, None, Box::new(Timer::new()));
    let tim_idx = bus.find_peripheral_index_by_name("tim2").unwrap();
    bus.peripherals[tim_idx]
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<Timer>()
        .unwrap()
        .force_legacy_walk();
    assert!(
        !bus.derive_walk_deletable(),
        "a legacy-walk native timer is walk-dependent — walk must stay on"
    );
}

/// The default `needs_legacy_walk() == true` makes an unknown/native model
/// (here the fixed-value `TagPeripheral`, which does not override it) pin the
/// walk on — the conservative default that prevents silently starving a
/// peripheral of ticks.
#[test]
fn derive_walk_deletable_defaults_conservative_for_unknown_models() {
    let mut bus = SystemBus::new();
    bus.peripherals.clear();
    bus.add_peripheral(
        "tag",
        0x4002_0000,
        0x400,
        None,
        Box::new(TagPeripheral(0xAB)),
    );
    assert!(
        !bus.derive_walk_deletable(),
        "a model that doesn't prove walk-independence keeps the walk"
    );
}

/// Minimal fixed-value peripheral for routing tests: reads return a
/// constant tag byte, writes are ignored.
#[derive(Debug)]
struct TagPeripheral(u8);
impl crate::Peripheral for TagPeripheral {
    fn read(&self, _offset: u64) -> crate::SimResult<u8> {
        Ok(self.0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> crate::SimResult<()> {
        Ok(())
    }
}

fn declarative_descriptor(timing: Option<Vec<TimingDescriptor>>) -> PeripheralDescriptor {
    PeripheralDescriptor {
        peripheral: "test".to_string(),
        version: "1.0".to_string(),
        registers: vec![
            RegisterDescriptor {
                id: "CTRL".to_string(),
                address_offset: 0x00,
                size: 32,
                access: Access::ReadWrite,
                reset_value: 0,
                fields: vec![],
                side_effects: None,
            },
            RegisterDescriptor {
                id: "STATUS".to_string(),
                address_offset: 0x04,
                size: 32,
                access: Access::ReadWrite,
                reset_value: 0,
                fields: vec![],
                side_effects: None,
            },
        ],
        interrupts: None,
        timing,
    }
}

#[test]
fn declarative_peripherals_enter_legacy_tick_set_only_while_events_are_pending() {
    let mut bus = SystemBus::empty();
    bus.add_peripheral(
        "idle_declarative",
        0x1000,
        0x100,
        None,
        Box::new(crate::peripherals::declarative::GenericPeripheral::new(
            declarative_descriptor(None),
        )),
    );
    assert!(
        bus.legacy_tick_indices.is_empty(),
        "declarative peripherals with no timing events should not be in the hot tick set"
    );

    bus.add_peripheral(
        "delayed_declarative",
        0x2000,
        0x100,
        None,
        Box::new(crate::peripherals::declarative::GenericPeripheral::new(
            declarative_descriptor(Some(vec![TimingDescriptor {
                id: "set-status".to_string(),
                trigger: TimingTrigger::Write {
                    register: "CTRL".to_string(),
                    value: Some(1),
                    mask: None,
                },
                delay_cycles: 0,
                action: TimingAction::SetBits {
                    register: "STATUS".to_string(),
                    bits: 1,
                },
                interrupt: None,
            }])),
        )),
    );
    assert!(
        bus.legacy_tick_indices.is_empty(),
        "write-triggered declarative timing is inactive until firmware writes the trigger"
    );

    bus.write_u32(0x2000, 1).unwrap();
    assert!(
        bus.peripherals[1].dev.legacy_tick_active(),
        "the declarative write should schedule a pending timing event"
    );
    assert_eq!(
        bus.legacy_tick_indices,
        vec![1],
        "write-triggered timing should activate only the touched C3/S3 peripheral entry"
    );

    bus.tick_peripherals_fully();
    assert_eq!(bus.read_u32(0x2004).unwrap(), 1);
    assert!(
        bus.legacy_tick_indices.is_empty(),
        "one-shot declarative timing should leave the hot tick set after it drains"
    );
}

#[cfg(feature = "event-scheduler")]
#[test]
fn scheduler_peripherals_do_not_enter_legacy_tick_index() {
    #[derive(Debug)]
    struct SchedulerPeripheral;
    impl crate::Peripheral for SchedulerPeripheral {
        fn read(&self, _offset: u64) -> SimResult<u8> {
            Ok(0)
        }

        fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
            Ok(())
        }

        fn legacy_tick_active(&self) -> bool {
            true
        }

        fn uses_scheduler(&self) -> bool {
            true
        }
    }

    let mut bus = SystemBus::empty();
    bus.add_peripheral("sched", 0x1000, 0x100, None, Box::new(SchedulerPeripheral));

    assert!(
        bus.legacy_tick_indices.is_empty(),
        "scheduler-owned peripherals are advanced by the event scheduler, not the legacy tick walk"
    );
    assert!(!bus.refresh_legacy_tick_index(0));
    assert!(
        bus.legacy_tick_indices.is_empty(),
        "refresh must not reinsert scheduler-owned peripherals into the legacy tick walk"
    );
}

#[test]
fn c3_and_s3_interrupt_routing_caches_are_separate() {
    let mut bus = SystemBus::empty();
    assert!(!bus.esp32c3_irq_routing);
    assert!(!bus.esp32s3_irq_routing);

    bus.esp32c3_irq_routing = true;
    bus.refresh_peripheral_index();
    assert!(bus.esp32c3_irq_routing);
    assert_eq!(bus.esp32c3_system_idx, None);
    assert_eq!(bus.esp32c3_interrupt_core0_idx, None);
    assert!(
        !bus.esp32s3_irq_routing,
        "enabling C3 RISC-V routing must not imply an S3 intmatrix model"
    );

    bus.add_peripheral(
        "system",
        0x600C_0000,
        0x1000,
        None,
        Box::new(crate::peripherals::declarative::GenericPeripheral::new(
            declarative_descriptor(None),
        )),
    );
    bus.add_peripheral(
        "interrupt_core0",
        0x600C_2000,
        0x1000,
        None,
        Box::new(crate::peripherals::declarative::GenericPeripheral::new(
            declarative_descriptor(None),
        )),
    );
    assert_eq!(bus.esp32c3_system_idx, Some(0));
    assert_eq!(bus.esp32c3_interrupt_core0_idx, Some(1));
    assert!(
        !bus.esp32s3_irq_routing,
        "adding C3 interrupt banks must not imply an S3 intmatrix model"
    );

    bus.add_peripheral(
        "intmatrix",
        0x600C_2000,
        0x1000,
        None,
        Box::new(crate::peripherals::esp32s3::intmatrix::Esp32s3IntMatrix::new()),
    );
    assert!(
        bus.esp32s3_irq_routing,
        "S3 routing should be cached only when the S3 intmatrix peripheral is present"
    );
    assert!(
        bus.esp32c3_irq_routing,
        "adding S3 routing must not clear the independent C3 routing flag"
    );
}

#[test]
fn missing_clock_fault_suppresses_access_and_counts() {
    let mut bus = SystemBus::new();
    let base = 0x4000_0000u64;
    bus.add_peripheral("usart1", base, 0x400, None, Box::new(TagPeripheral(0xAB)));

    // Normally clocked: reads the peripheral's tag bytes.
    assert_eq!(bus.read_u32(base).unwrap(), 0xABAB_ABAB);

    bus.inject_missing_clock("usart1").unwrap();
    assert_eq!(bus.missing_clock_suppressed("usart1"), 0);

    // Now the access is suppressed: reads 0, and the fault is recorded fired.
    assert_eq!(bus.read_u32(base).unwrap(), 0);
    assert!(bus.missing_clock_suppressed("usart1") > 0);

    // An unknown peripheral is an error, not a silent no-op.
    assert!(bus.inject_missing_clock("nope").is_err());
}

/// Routing must be a pure function of the address — never of access
/// history. A broad catch-all window with a narrower twin layered inside
/// it (the ESP32-S3 low-MMIO + per-peripheral twin pattern) must route
/// the twin's addresses to the twin even when the immediately preceding
/// access touched a broad-window-only address (which seeds the hint
/// cache with the broad entry — containment alone must not let it
/// short-circuit the canonical last-start-wins search).
#[test]
fn pin_labels_parse_for_both_vendor_forms() {
    // STM32 letter ports.
    assert_eq!(
        SystemBus::parse_stm32_pin("PC7"),
        Some(("gpioc".to_string(), 7))
    );
    assert_eq!(SystemBus::parse_stm32_pin("PA16"), None); // STM32 ports stop at 15
                                                          // Nordic numbered ports: nRF52840 P0.00-P0.31, P1.00-P1.15.
    assert_eq!(
        SystemBus::parse_stm32_pin("P0.04"),
        Some(("gpio0".to_string(), 4))
    );
    assert_eq!(
        SystemBus::parse_stm32_pin("P1.15"),
        Some(("gpio1".to_string(), 15))
    );
    assert_eq!(SystemBus::parse_stm32_pin("P0.32"), None);
    assert_eq!(SystemBus::parse_stm32_pin("P0."), None);
}

#[test]
fn overlapping_windows_route_history_independently() {
    let mut bus = SystemBus::new();
    // Broad catch-all: 0x7000_0000..0x7000_8000, reads 0xBB.
    bus.add_peripheral(
        "broad",
        0x7000_0000,
        0x8000,
        None,
        Box::new(TagPeripheral(0xBB)),
    );
    // Narrow twin layered inside: 0x7000_4000..0x7000_5000, reads 0xAA.
    bus.add_peripheral(
        "narrow",
        0x7000_4000,
        0x1000,
        None,
        Box::new(TagPeripheral(0xAA)),
    );

    // Cold route: twin wins its window.
    assert_eq!(
        bus.read_u8(0x7000_4000).unwrap(),
        0xAA,
        "cold: twin owns it"
    );

    // Poison the hint with the broad entry, then re-route a twin address.
    assert_eq!(
        bus.read_u8(0x7000_0008).unwrap(),
        0xBB,
        "broad-only address"
    );
    assert_eq!(
        bus.read_u8(0x7000_4FFC).unwrap(),
        0xAA,
        "hint poisoned by broad entry must not hijack the twin's window"
    );

    // resolve_window must agree with dispatch, in both hint states.
    assert_eq!(bus.read_u8(0x7000_0008).unwrap(), 0xBB); // re-poison
    assert_eq!(
        bus.resolve_window(0x7000_4000),
        Some((0x7000_4000, 0x1000)),
        "resolve_window must return the twin, not the hinted broad entry"
    );

    // Addresses in the broad window above the twin still go broad —
    // including right after a twin access (reverse poisoning), and the
    // fallback must pick the GREATEST containing start, not the
    // first-registered entry.
    assert_eq!(bus.read_u8(0x7000_4000).unwrap(), 0xAA);
    assert_eq!(
        bus.read_u8(0x7000_5000).unwrap(),
        0xBB,
        "past the twin's end the broad window resumes"
    );

    // next_window_start: the twin's start bounds the broad window's
    // uniform service region (used by the coverage probe's baseline).
    assert_eq!(bus.next_window_start(0x7000_0000), Some(0x7000_4000));
    assert_eq!(
        bus.next_window_start(0x7000_4000),
        Some(0xE000_E010),
        "above the twin the next start is the default bus's systick"
    );
}

#[test]
fn test_system_bus_from_config_declarative() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = root.join("tests/fixtures/test_chip_declarative.yaml");
    let manifest_path = root.join("tests/fixtures/test_system_declarative.yaml");

    let chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let manifest = SystemManifest::from_file(&manifest_path).unwrap();

    let bus = SystemBus::from_config(&chip, &manifest).expect("Failed to create bus from config");

    // Verify TIMER1 is present at 0x40001000
    let found = bus
        .peripherals
        .iter()
        .find(|p| p.name == "TIMER1")
        .expect("TIMER1 not found");
    assert_eq!(found.base, 0x40001000);
    assert_eq!(found.size, 1024);

    // Verify we can read/write to it through the bus
    // Address 0x40001000 + 0x00 = CTRL register (reset value 0)
    let ctrl_val = bus.read_u32(0x40001000).unwrap();
    assert_eq!(ctrl_val, 0);

    // Address 0x40001000 + 0x04 = COUNT register
    let mut bus = bus;
    bus.write_u32(0x40001004, 0x12345678).unwrap();
    let count_val = bus.read_u32(0x40001004).unwrap();
    assert_eq!(count_val, 0x12345678);
}

#[test]
fn test_system_bus_resolves_descriptor_path_relative_to_chip_file() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip_path = root.join("tests/fixtures/test_chip_declarative.yaml");
    let manifest_path = root.join("tests/fixtures/test_system_declarative.yaml");

    let mut chip = ChipDescriptor::from_file(&chip_path).unwrap();
    let mut manifest = SystemManifest::from_file(&manifest_path).unwrap();

    // Simulate a descriptor path that is relative to chip.yaml location.
    if let Some(path) = chip.peripherals[0].config.get_mut("path") {
        *path = serde_yaml::Value::String("test_timer_descriptor.yaml".to_string());
    }
    manifest.chip = chip_path.to_string_lossy().into_owned();

    let bus = SystemBus::from_config(&chip, &manifest).expect("Failed to create bus from config");

    let found = bus
        .peripherals
        .iter()
        .find(|p| p.name == "TIMER1")
        .expect("TIMER1 not found");
    assert_eq!(found.base, 0x40001000);
}

#[test]
fn test_from_config_attaches_adxl345_external_device_to_i2c() {
    use labwired_config::{
        Arch, ChipDescriptor, ExternalDevice, MemoryRange, PeripheralConfig, SystemManifest,
    };
    use std::collections::HashMap;

    let chip = ChipDescriptor {
        schema_version: "1.0".to_string(),
        reset_vector_offset: 0,
        atomic_register_aliases: false,
        memory_regions: Vec::new(),
        name: "stm32f103-test".to_string(),
        arch: Arch::Arm,
        core: None,
        flash: MemoryRange {
            base: 0x0800_0000,
            size: "64KB".to_string(),
        },
        ram: MemoryRange {
            base: 0x2000_0000,
            size: "20KB".to_string(),
        },
        peripherals: vec![PeripheralConfig {
            id: "i2c1".to_string(),
            r#type: "i2c".to_string(),
            base_address: 0x4000_5400,
            size: Some("1KB".to_string()),
            irq: Some(31),
            clock: None,
            config: HashMap::new(),
        }],
        pins: Default::default(),
    };

    let mut config = HashMap::new();
    config.insert(
        "i2c_address".to_string(),
        serde_yaml::Value::Number(0x53.into()),
    );
    let manifest = SystemManifest {
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "adxl345-test".to_string(),
        chip: "../chips/stm32f103.yaml".to_string(),
        memory_overrides: HashMap::new(),
        external_devices: vec![ExternalDevice {
            id: "adxl345".to_string(),
            r#type: "adxl345".to_string(),
            connection: "i2c1".to_string(),
            route: Default::default(),
            config,
        }],
        board_io: Vec::new(),
        debug_uart: None,
        peripherals: Vec::new(),
    };

    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let i2c_idx = bus.find_peripheral_index_by_name("i2c1").unwrap();
    let any = bus.peripherals[i2c_idx].dev.as_any_mut().unwrap();
    let i2c = any.downcast_mut::<crate::peripherals::i2c::I2c>().unwrap();
    assert_eq!(i2c.attached_devices().len(), 1);
}

#[test]
fn test_esp32c3_i2c_device_requires_declared_physical_route() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
        .expect("read ESP32-C3 chip descriptor");
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: "c3-i2c-route-required"
chip: "../chips/esp32c3.yaml"
external_devices:
  - id: "bmp280"
    type: "bmp280"
    connection: "i2c0"
    config:
      i2c_address: 0x76
"#,
    )
    .expect("parse route-less C3 manifest");

    let err = match SystemBus::from_config(&chip, &manifest) {
        Ok(_) => panic!("ESP32-C3 I2C must reject a route-less external device"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("route.sda") && err.to_string().contains("route.scl"),
        "error must tell the author exactly which physical signals are required: {err:#}"
    );
}

/// The `rotary_encoder` external device dispatches through the DECLARATIVE
/// device path (`configs/devices/rotary_encoder.yaml`, `quadrature` primitive)
/// rather than a hand-written `from_config` arm. This locks that seam: a
/// rotary device in a system.yaml must still land a `RotaryEncoder` on the bus
/// with its CLK/DT pins resolved from the descriptor's pin bindings — byte for
/// byte what the deleted arm produced.
#[test]
fn test_from_config_attaches_rotary_encoder_via_declarative_descriptor() {
    use crate::peripherals::components::rotary_encoder::RotaryEncoder;

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chip = ChipDescriptor::from_file(root.join("../../configs/chips/stm32f103.yaml"))
        .expect("read STM32F103 chip descriptor");
    // Both `type:` spellings must resolve to the same declarative descriptor.
    for type_str in ["rotary_encoder", "rotary-encoder"] {
        let manifest: SystemManifest = serde_yaml::from_str(&format!(
            r#"
name: "rotary-declarative"
chip: "../chips/stm32f103.yaml"
external_devices:
  - id: "knob"
    type: "{type_str}"
    connection: "gpio"
    config:
      clk_pin: "PA0"
      dt_pin: "PA1"
      cpu_hz: 8000000
board_io: []
"#
        ))
        .expect("parse rotary manifest");

        let bus = SystemBus::from_config(&chip, &manifest).expect("build bus with rotary");
        let encoders: Vec<&RotaryEncoder> = bus.gpio_devices_of::<RotaryEncoder>().collect();
        assert_eq!(
            encoders.len(),
            1,
            "exactly one RotaryEncoder attached for type '{type_str}'"
        );
        let enc = encoders[0];
        assert_eq!(enc.id, "knob");
        // PA0 → gpioa IDR bit 0; PA1 → bit 1. The descriptor bound role `a`→
        // clk_pin and `b`→dt_pin, so CLK follows PA0 and DT follows PA1.
        assert_eq!(enc.clk_bit, 0, "clk_pin PA0 → bit 0");
        assert_eq!(enc.dt_bit, 1, "dt_pin PA1 → bit 1");
        assert_eq!(
            enc.clk_idr_addr, enc.dt_idr_addr,
            "both channels on the same GPIOA IDR"
        );
        // cpu_hz threaded from config through the descriptor's params mapping.
        assert_eq!(enc.cpu_hz, 8_000_000, "cpu_hz sourced from config");
    }
}

#[test]
fn curated_esp32c3_i2c_manifests_declare_physical_routes() {
    #[derive(serde::Deserialize)]
    struct DeviceInventory {
        #[serde(default)]
        external_devices: Vec<labwired_config::ExternalDevice>,
    }

    // Upgrade inventory: every curated C3 system that attaches an I²C
    // device must declare the physical pair. Keep this explicit list next
    // to the runtime gate so adding a route-less demo cannot silently
    // restore controller-only behavior.
    const MANIFESTS: &[&str] = &[
        "configs/systems/esp32c3-oled-demo.yaml",
        "configs/systems/esp32c3-oled-128x32-workshop.yaml",
        "configs/systems/esp32c3-mlx90640-thermal.yaml",
        "examples/esp32c3-mlx90640-thermal/system.yaml",
        "examples/esp32c3-mlx90640-thermal/system-fault.yaml",
        "examples/esp32c3-mlx90640-thermal/system-iolink.yaml",
        "examples/esp32c3-mlx90640-thermal/system-iolink-fault.yaml",
        "examples/esp32c3-leo-airquality/system.yaml",
        "examples/esp32c3-leo-airquality/system-fresh.yaml",
        "examples/esp32c3-leo-airquality/system-stuffy.yaml",
    ];

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut routed_devices = 0usize;
    for rel in MANIFESTS {
        let source = std::fs::read_to_string(root.join(rel))
            .unwrap_or_else(|err| panic!("read curated C3 manifest {rel}: {err}"));
        let inventory: DeviceInventory = serde_yaml::from_str(&source)
            .unwrap_or_else(|err| panic!("parse curated C3 manifest {rel}: {err}"));
        for device in inventory
            .external_devices
            .iter()
            .filter(|device| device.connection == "i2c0")
        {
            crate::peripherals::esp32c3::i2c::C3I2cPadRoute::from_manifest_route(
                &device.route,
            )
            .unwrap_or_else(|err| {
                panic!(
                    "curated C3 manifest {rel} external device '{}' has no usable physical I2C route: {err:#}",
                    device.id
                )
            });
            routed_devices += 1;
        }
    }
    assert!(
        routed_devices > 0,
        "the C3 route upgrade inventory must exercise at least one I2C device"
    );
}

#[test]
fn test_esp32c3_i2c_gpio_matrix_distinguishes_gpio45_from_gpio67() {
    use labwired_config::ExternalDevice;
    use std::collections::{BTreeMap, HashMap};

    const GPIO_BASE: u64 = 0x6000_4000;
    const GPIO_ENABLE_W1TS: u64 = 0x24;
    const GPIO_FUNC_IN_SEL: u64 = 0x154;
    const GPIO_FUNC_OUT_SEL: u64 = 0x554;
    const MATRIX_INPUT_SELECT: u32 = 1 << 6;
    const I2C_SCL_SIGNAL: u32 = 53;
    const I2C_SDA_SIGNAL: u32 = 54;
    const I2C_BASE: u64 = 0x6001_3000;
    const I2C_INT_RAW: u64 = 0x20;
    const I2C_REG_CTR: u64 = 0x04;
    const I2C_REG_DATA: u64 = 0x1C;
    const I2C_REG_CMD0: u64 = 0x58;
    const I2C_INT_NACK: u32 = 1 << 10;
    const I2C_INT_TRANS_COMPLETE: u32 = 1 << 7;

    fn route(sda: u8, scl: u8) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("sda".to_string(), format!("GPIO{sda}")),
            ("scl".to_string(), format!("GPIO{scl}")),
        ])
    }

    fn build_bus(route: BTreeMap<String, String>) -> SystemBus {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
            .expect("read ESP32-C3 chip descriptor");
        let mut config = HashMap::new();
        config.insert(
            "i2c_address".to_string(),
            serde_yaml::Value::Number(0x3C.into()),
        );
        let manifest = SystemManifest {
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "c3-physical-i2c-route".to_string(),
            chip: "../chips/esp32c3.yaml".to_string(),
            memory_overrides: HashMap::new(),
            external_devices: vec![ExternalDevice {
                id: "oled".to_string(),
                r#type: "oled-ssd1306-128x32".to_string(),
                connection: "i2c0".to_string(),
                route,
                config,
            }],
            board_io: Vec::new(),
            debug_uart: None,
            peripherals: Vec::new(),
        };
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("construct C3 bus");
        let i2c_idx = bus
            .find_peripheral_index_by_name("i2c0")
            .expect("C3 I2C0 must be present");
        bus.peripherals[i2c_idx]
            .dev
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<crate::peripherals::esp32c3::i2c::Esp32c3I2c>())
            .expect("C3 behavioral I2C0")
            .force_legacy_walk();
        bus
    }

    fn configure_wire_begin_equivalent(bus: &mut SystemBus, sda: u8, scl: u8) {
        bus.write_u32(GPIO_BASE + GPIO_ENABLE_W1TS, (1 << sda) | (1 << scl))
            .expect("enable I2C pads");
        bus.write_u32(
            GPIO_BASE + GPIO_FUNC_OUT_SEL + u64::from(sda) * 4,
            I2C_SDA_SIGNAL,
        )
        .expect("route SDA output");
        bus.write_u32(
            GPIO_BASE + GPIO_FUNC_OUT_SEL + u64::from(scl) * 4,
            I2C_SCL_SIGNAL,
        )
        .expect("route SCL output");
        bus.write_u32(
            GPIO_BASE + GPIO_FUNC_IN_SEL + u64::from(I2C_SDA_SIGNAL) * 4,
            MATRIX_INPUT_SELECT | u32::from(sda),
        )
        .expect("route SDA input");
        bus.write_u32(
            GPIO_BASE + GPIO_FUNC_IN_SEL + u64::from(I2C_SCL_SIGNAL) * 4,
            MATRIX_INPUT_SELECT | u32::from(scl),
        )
        .expect("route SCL input");
    }

    fn probe_oled_address(bus: &mut SystemBus) -> u32 {
        let cmd = |opcode: u32, byte_num: u32| (opcode << 11) | byte_num;
        bus.write_u32(I2C_BASE + I2C_REG_CMD0, cmd(6, 0))
            .expect("RSTART");
        bus.write_u32(I2C_BASE + I2C_REG_CMD0 + 4, cmd(1, 1))
            .expect("WRITE address");
        bus.write_u32(I2C_BASE + I2C_REG_CMD0 + 8, cmd(2, 0))
            .expect("STOP");
        bus.write_u32(I2C_BASE + I2C_REG_DATA, 0x78)
            .expect("OLED address byte");
        bus.write_u32(I2C_BASE + I2C_REG_CTR, 1 << 5)
            .expect("start I2C transaction");
        for _ in 0..1_000_000 {
            let flags = bus.read_u32(I2C_BASE + I2C_INT_RAW).unwrap();
            if flags & I2C_INT_TRANS_COMPLETE != 0 {
                return flags;
            }
            bus.tick_peripherals_fully();
        }
        panic!("C3 I2C address probe did not complete");
    }

    // A physical OLED on GPIO4/5 must not be reached by a firmware route
    // to GPIO6/7; the exact same controller/address starts ACKing only
    // after the `Wire.begin(4, 5)`-equivalent GPIO-matrix writes.
    let mut physical_45_wrong_67 = build_bus(route(4, 5));
    configure_wire_begin_equivalent(&mut physical_45_wrong_67, 6, 7);
    assert_ne!(
        probe_oled_address(&mut physical_45_wrong_67) & I2C_INT_NACK,
        0,
        "GPIO6/7 must NACK an OLED physically wired to GPIO4/5"
    );
    let mut physical_45_right_45 = build_bus(route(4, 5));
    configure_wire_begin_equivalent(&mut physical_45_right_45, 4, 5);
    assert_eq!(
        probe_oled_address(&mut physical_45_right_45) & I2C_INT_NACK,
        0,
        "GPIO4/5 must ACK an OLED physically wired to GPIO4/5"
    );

    // Reverse the physical circuit as well: this proves the pair is not
    // metadata and that GPIO6/7 has its own observable electrical path.
    let mut physical_67_wrong_45 = build_bus(route(6, 7));
    configure_wire_begin_equivalent(&mut physical_67_wrong_45, 4, 5);
    assert_ne!(
        probe_oled_address(&mut physical_67_wrong_45) & I2C_INT_NACK,
        0,
        "GPIO4/5 must NACK an OLED physically wired to GPIO6/7"
    );
    let mut physical_67_right_67 = build_bus(route(6, 7));
    configure_wire_begin_equivalent(&mut physical_67_right_67, 6, 7);
    assert_eq!(
        probe_oled_address(&mut physical_67_right_67) & I2C_INT_NACK,
        0,
        "GPIO6/7 must ACK an OLED physically wired to GPIO6/7"
    );
}

/// Wiring guard for the ESP32-C3 behavioral I²C: a chip yaml declaring
/// `i2c0` as `esp32c3_i2c` plus a system manifest declaring a BMP280 on
/// `connection: "i2c0"` must attach that slave to the behavioral controller
/// AND let a register-driven write-then-read transaction reach it. This is
/// the path the MLX90640 will use (different device type, same wiring).
#[test]
fn test_from_config_attaches_bmp280_to_esp32c3_i2c0() {
    use labwired_config::{
        Arch, ChipDescriptor, ExternalDevice, MemoryRange, PeripheralConfig, SystemManifest,
    };
    use std::collections::{BTreeMap, HashMap};

    let chip = ChipDescriptor {
        schema_version: "1.0".to_string(),
        reset_vector_offset: 0,
        atomic_register_aliases: false,
        memory_regions: Vec::new(),
        name: "esp32c3-i2c-test".to_string(),
        arch: Arch::RiscV,
        core: None,
        flash: MemoryRange {
            base: 0x4200_0000,
            size: "4MB".to_string(),
        },
        ram: MemoryRange {
            base: 0x3FC8_0000,
            size: "400KB".to_string(),
        },
        peripherals: vec![
            PeripheralConfig {
                id: "i2c0".to_string(),
                r#type: "esp32c3_i2c".to_string(),
                base_address: 0x6001_3000,
                size: Some("4KB".to_string()),
                irq: None,
                config: HashMap::new(),
                clock: None,
            },
            PeripheralConfig {
                id: "gpio".to_string(),
                r#type: "esp32c3_gpio".to_string(),
                base_address: 0x6000_4000,
                size: Some("4KB".to_string()),
                irq: None,
                config: HashMap::new(),
                clock: None,
            },
        ],
        pins: Default::default(),
    };

    let mut config = HashMap::new();
    config.insert(
        "i2c_address".to_string(),
        serde_yaml::Value::Number(0x76.into()),
    );
    let manifest = SystemManifest {
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "esp32c3-bmp280-test".to_string(),
        chip: "../chips/esp32c3.yaml".to_string(),
        memory_overrides: HashMap::new(),
        external_devices: vec![ExternalDevice {
            id: "bmp280".to_string(),
            r#type: "bmp280".to_string(),
            connection: "i2c0".to_string(),
            route: BTreeMap::from([
                ("sda".to_string(), "GPIO4".to_string()),
                ("scl".to_string(), "GPIO5".to_string()),
            ]),
            config,
        }],
        board_io: Vec::new(),
        debug_uart: None,
        peripherals: Vec::new(),
    };

    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    // `Wire.begin(4, 5)`-equivalent GPIO-matrix setup: both output and
    // input paths must select the manifest's physical pads before the
    // attached BMP280 can answer.
    bus.write_u32(0x6000_4000 + 0x24, (1 << 4) | (1 << 5))
        .unwrap();
    bus.write_u32(0x6000_4000 + 0x554 + 4 * 4, 54).unwrap();
    bus.write_u32(0x6000_4000 + 0x554 + 5 * 4, 53).unwrap();
    bus.write_u32(0x6000_4000 + 0x154 + 54 * 4, (1 << 6) | 4)
        .unwrap();
    bus.write_u32(0x6000_4000 + 0x154 + 53 * 4, (1 << 6) | 5)
        .unwrap();
    let i2c_idx = bus
        .find_peripheral_index_by_name("i2c0")
        .expect("i2c0 must be registered");
    let any = bus.peripherals[i2c_idx].dev.as_any_mut().unwrap();
    let i2c = any
        .downcast_mut::<crate::peripherals::esp32c3::i2c::Esp32c3I2c>()
        .expect("i2c0 must be the behavioral Esp32c3I2c controller");
    // This test drives the bit engine directly via `tick_elapsed` (the
    // legacy walk path) with no Machine event loop; pin it off the scheduler
    // so the direct drive advances the engine (byte-identical to the
    // scheduler path, which a Machine drives via `drain_scheduler_events`).
    i2c.force_legacy_walk();

    // Drive the canonical register-pointer read of the BMP280 chip-id
    // (0xD0 → 0x58), exactly as C3 firmware would, through the controller's
    // registers: RSTART; WRITE 2 (addr+W, ptr); RSTART; WRITE 1 (addr+R);
    // READ 1; STOP. Opcodes: 6=RSTART, 1=WRITE, 3=READ, 2=STOP.
    i2c.write_u32(0x58, 6 << 11).unwrap(); // CMD0 RSTART
    i2c.write_u32(0x5C, (1 << 11) | 2).unwrap(); // CMD1 WRITE 2
    i2c.write_u32(0x60, 6 << 11).unwrap(); // CMD2 RSTART
    i2c.write_u32(0x64, (1 << 11) | 1).unwrap(); // CMD3 WRITE 1
    i2c.write_u32(0x68, (3 << 11) | 1).unwrap(); // CMD4 READ 1
    i2c.write_u32(0x6C, 2 << 11).unwrap(); // CMD5 STOP
    i2c.write_u32(0x1C, 0xEC).unwrap(); // addr+W (0x76<<1)
    i2c.write_u32(0x1C, 0xD0).unwrap(); // pointer = chip-id
    i2c.write_u32(0x1C, 0xED).unwrap(); // addr+R
    i2c.write_u32(0x04, 1 << 5).unwrap(); // TRANS_START

    // The C3 controller now clocks the command list bit-by-bit over
    // simulated cycles; run the engine to completion.
    for _ in 0..1_000_000 {
        if !i2c.engine_active() {
            break;
        }
        i2c.tick_elapsed(64);
    }
    assert!(!i2c.engine_active(), "C3 I2C bit engine must complete");

    // Address must have matched (no NACK at bit 10) and the chip-id byte
    // must round-trip out of the RX FIFO.
    let int_raw = i2c.read_u32(0x20).unwrap();
    assert_eq!(
        int_raw & (1 << 10),
        0,
        "BMP280 must ACK; INT_RAW=0x{int_raw:08x}"
    );
    assert_eq!(
        i2c.read_u32(0x1C).unwrap(),
        0x58,
        "BMP280 CHIP_ID must round-trip through the bus-attached controller"
    );
}

/// Wiring + reachability guard for the MLX90640 thermal camera on the
/// ESP32-C3 behavioral I²C0: a system manifest declaring an `mlx90640` on
/// `connection: "i2c0"` must attach it at 0x33 AND let a register-driven
/// 16-bit-addressed read reach an EEPROM word. We read the gainEE word at
/// EEPROM address 0x2430 (== 0x2400 + 48), which the linearized calibration
/// fixes to 6000, exercising the 16-bit register-address protocol over the
/// real bus-attached controller.
#[test]
fn test_from_config_attaches_mlx90640_to_esp32c3_i2c0_and_reads_eeprom() {
    use labwired_config::{
        Arch, ChipDescriptor, ExternalDevice, MemoryRange, PeripheralConfig, SystemManifest,
    };
    use std::collections::{BTreeMap, HashMap};

    let chip = ChipDescriptor {
        schema_version: "1.0".to_string(),
        reset_vector_offset: 0,
        atomic_register_aliases: false,
        memory_regions: Vec::new(),
        name: "esp32c3-mlx-test".to_string(),
        arch: Arch::RiscV,
        core: None,
        flash: MemoryRange {
            base: 0x4200_0000,
            size: "4MB".to_string(),
        },
        ram: MemoryRange {
            base: 0x3FC8_0000,
            size: "400KB".to_string(),
        },
        peripherals: vec![
            PeripheralConfig {
                id: "i2c0".to_string(),
                r#type: "esp32c3_i2c".to_string(),
                base_address: 0x6001_3000,
                size: Some("4KB".to_string()),
                irq: None,
                config: HashMap::new(),
                clock: None,
            },
            PeripheralConfig {
                id: "gpio".to_string(),
                r#type: "esp32c3_gpio".to_string(),
                base_address: 0x6000_4000,
                size: Some("4KB".to_string()),
                irq: None,
                config: HashMap::new(),
                clock: None,
            },
        ],
        pins: Default::default(),
    };

    let mut config = HashMap::new();
    config.insert(
        "i2c_address".to_string(),
        serde_yaml::Value::Number(0x33.into()),
    );
    config.insert(
        "ambient_c".to_string(),
        serde_yaml::Value::Number(25.0.into()),
    );
    let manifest = SystemManifest {
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "esp32c3-mlx90640-test".to_string(),
        chip: "../chips/esp32c3.yaml".to_string(),
        memory_overrides: HashMap::new(),
        external_devices: vec![ExternalDevice {
            id: "thermal_cam".to_string(),
            r#type: "mlx90640".to_string(),
            connection: "i2c0".to_string(),
            route: BTreeMap::from([
                ("sda".to_string(), "GPIO4".to_string()),
                ("scl".to_string(), "GPIO5".to_string()),
            ]),
            config,
        }],
        board_io: Vec::new(),
        debug_uart: None,
        peripherals: Vec::new(),
    };

    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    bus.write_u32(0x6000_4000 + 0x24, (1 << 4) | (1 << 5))
        .unwrap();
    bus.write_u32(0x6000_4000 + 0x554 + 4 * 4, 54).unwrap();
    bus.write_u32(0x6000_4000 + 0x554 + 5 * 4, 53).unwrap();
    bus.write_u32(0x6000_4000 + 0x154 + 54 * 4, (1 << 6) | 4)
        .unwrap();
    bus.write_u32(0x6000_4000 + 0x154 + 53 * 4, (1 << 6) | 5)
        .unwrap();
    let i2c_idx = bus
        .find_peripheral_index_by_name("i2c0")
        .expect("i2c0 must be registered");
    let any = bus.peripherals[i2c_idx].dev.as_any_mut().unwrap();
    let i2c = any
        .downcast_mut::<crate::peripherals::esp32c3::i2c::Esp32c3I2c>()
        .expect("i2c0 must be the behavioral Esp32c3I2c controller");
    // Direct `tick_elapsed` drive (legacy walk path), no Machine event loop:
    // pin off the scheduler so the direct drive advances the engine.
    i2c.force_legacy_walk();

    // 16-bit-addressed read of EEPROM word 0x2430: write the 2-byte big-
    // endian register address (0x24, 0x30), repeated-start, read 2 bytes
    // (MSB first). Opcodes: 6=RSTART, 1=WRITE, 3=READ, 2=STOP.
    i2c.write_u32(0x58, 6 << 11).unwrap(); // CMD0 RSTART
    i2c.write_u32(0x5C, (1 << 11) | 3).unwrap(); // CMD1 WRITE 3 (addr+W, addr_hi, addr_lo)
    i2c.write_u32(0x60, 6 << 11).unwrap(); // CMD2 RSTART
    i2c.write_u32(0x64, (1 << 11) | 1).unwrap(); // CMD3 WRITE 1 (addr+R)
    i2c.write_u32(0x68, (3 << 11) | 2).unwrap(); // CMD4 READ 2 (one 16-bit word)
    i2c.write_u32(0x6C, 2 << 11).unwrap(); // CMD5 STOP
    i2c.write_u32(0x1C, 0x66).unwrap(); // addr+W (0x33<<1)
    i2c.write_u32(0x1C, 0x24).unwrap(); // reg addr high byte
    i2c.write_u32(0x1C, 0x30).unwrap(); // reg addr low byte
    i2c.write_u32(0x1C, 0x67).unwrap(); // addr+R (0x33<<1 | 1)
    i2c.write_u32(0x04, 1 << 5).unwrap(); // TRANS_START

    // The C3 controller now clocks the command list bit-by-bit over
    // simulated cycles; run the engine to completion.
    for _ in 0..1_000_000 {
        if !i2c.engine_active() {
            break;
        }
        i2c.tick_elapsed(64);
    }
    assert!(!i2c.engine_active(), "C3 I2C bit engine must complete");

    let int_raw = i2c.read_u32(0x20).unwrap();
    assert_eq!(
        int_raw & (1 << 10),
        0,
        "MLX90640 at 0x33 must ACK; INT_RAW=0x{int_raw:08x}"
    );
    let hi = i2c.read_u32(0x1C).unwrap();
    let lo = i2c.read_u32(0x1C).unwrap();
    let word = (hi << 8) | lo;
    assert_eq!(
        word, 6000,
        "MLX90640 gainEE EEPROM word (0x2430) must round-trip the 16-bit \
         register protocol through the bus-attached C3 controller"
    );
}

#[test]
fn test_from_config_can_diagnostic_tester_injects_frame_into_fdcan() {
    let chip: ChipDescriptor = serde_yaml::from_str(
        r#"
name: "h563-test"
arch: "arm"
core: "cortex-m33"
flash:
  base: 0x08000000
  size: "128KB"
ram:
  base: 0x20000000
  size: "64KB"
peripherals:
  - id: "fdcan1"
    type: "fdcan"
    base_address: 0x4000A400
    size: "4KB"
"#,
    )
    .unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: "uds-tester"
chip: "unused"
external_devices:
  - id: "uds_tester"
    type: "can-diagnostic-tester"
    connection: "fdcan1"
    config:
      request_id: "0x7E0"
      request_data: "03 22 F1 90"
board_io: []
"#,
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    assert_eq!(bus.can_diagnostic_testers.len(), 1);

    // Still in INIT: tester retries but cannot inject into a stopped FDCAN.
    bus.tick_peripherals_fully();
    {
        let idx = bus.find_peripheral_index_by_name("fdcan1").unwrap();
        let fdcan = bus.peripherals[idx]
            .dev
            .as_any()
            .unwrap()
            .downcast_ref::<crate::peripherals::fdcan::Fdcan>()
            .unwrap();
        assert!(fdcan.trace_snapshot("fdcan1").is_empty());
    }

    // Leave INIT; next bus tick lets the reusable tester drive the CAN frame.
    bus.write_u32(0x4000_A400 + 0x018, 0).unwrap();
    bus.tick_peripherals_fully();
    let idx = bus.find_peripheral_index_by_name("fdcan1").unwrap();
    let fdcan = bus.peripherals[idx]
        .dev
        .as_any()
        .unwrap()
        .downcast_ref::<crate::peripherals::fdcan::Fdcan>()
        .unwrap();
    let trace = fdcan.trace_snapshot("fdcan1");
    assert_eq!(trace.len(), 1);
    assert_eq!(trace[0].direction, "rx");
    assert_eq!(trace[0].id, 0x7E0);
    assert_eq!(trace[0].data, vec![0x03, 0x22, 0xF1, 0x90]);
    assert!(bus.can_diagnostic_testers[0].sent);
}

/// Pure FSM walk: FirstFrame → (ECU FlowControl) → ConsecutiveFrame →
/// (ECU positive response) → Done, driving the tester's state machine by
/// feeding ECU frames manually (no peripheral, no bus tick). This exercises
/// the exact observe/advance logic `service_can_uds_testers` reuses.
#[test]
fn uds_tester_fsm_drives_ff_fc_cf_response() {
    let mut t = CanUdsTester::new("t".into(), "bxcan1".into());
    assert_eq!(t.state, CanUdsTesterState::Start);
    assert_eq!(t.request_id, 0x111);
    assert_eq!(t.reply_id, 0x222);

    // Start: the next frame to inject is the FirstFrame; on a (simulated)
    // accepted inject the FSM advances to AwaitFc.
    assert_eq!(t.first_frame, CanUdsTester::DEFAULT_FIRST_FRAME.to_vec());
    t.state = CanUdsTesterState::AwaitFc;

    // A non-FlowControl frame, or one on the wrong id, does not unblock.
    assert!(t.observe_ecu_frame(0x999, &[0x30, 0x00, 0x00]).is_none());
    assert!(t.observe_ecu_frame(0x222, &[0x06, 0x67]).is_none());
    assert_eq!(t.state, CanUdsTesterState::AwaitFc);

    // ECU FlowControl (0x30..) on reply_id → returns the ConsecutiveFrame.
    let cf = t
        .observe_ecu_frame(0x222, &[0x30, 0x00, 0x00, 0, 0, 0, 0, 0])
        .expect("FlowControl unblocks the ConsecutiveFrame");
    assert_eq!(cf, CanUdsTester::DEFAULT_CONSECUTIVE_FRAME.to_vec());

    // Simulate the accepted CF inject.
    t.state = CanUdsTesterState::AwaitResp;

    // A wrong response (negative / different service) does not complete.
    assert!(t.observe_ecu_frame(0x222, &[0x03, 0x7F, 0x27]).is_none());
    assert_eq!(t.state, CanUdsTesterState::AwaitResp);

    // SecurityAccess positive single-frame response → Done.
    assert!(t
        .observe_ecu_frame(0x222, &[0x06, 0x67, 0x01, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE])
        .is_none());
    assert_eq!(t.state, CanUdsTesterState::Done);
    assert!(t.is_terminal());
}

/// Script-driven AwaitResp must decode the CAN-FD *escape* SingleFrame
/// (`0x00 LL <payload>`, length in byte 1, payload from byte 2), not just
/// the classic SF (`0x0L`). The H563 ECU runs ISO-TP in FD mode and answers
/// a 20-byte ReadDataByIdentifier as one 0x00-escape SF; the old parser read
/// the low nibble of 0x00 as length 0 and completed with an empty payload
/// ("got []"). Regression for the h563-uds-ecu smoke.
#[test]
fn scripted_tester_decodes_fd_escape_single_frame() {
    let mut t = CanUdsTester::new("t".into(), "fdcan1".into());
    t.reply_id = 0x7E8;
    t.script = vec![UdsStep {
        send: vec![0x22, 0xF1, 0x90],
        expect: vec![Some(0x62), Some(0xF1), Some(0x90)],
        expect_nrc: None,
    }];
    t.step_idx = 0;
    t.state = CanUdsTesterState::AwaitResp;

    // ECU FD escape SF: byte0 = 0x00, real length = 0x14 (20) in byte1,
    // payload 62 F1 90 + 17-byte VIN string.
    let mut resp = vec![0x00, 0x14, 0x62, 0xF1, 0x90];
    resp.extend_from_slice(b"LABWIRED-H563-UDS");
    assert!(t.observe_ecu_frame(0x7E8, &resp).is_none());
    assert_eq!(
        t.state,
        CanUdsTesterState::Done,
        "FD escape SF must decode the full payload and match step 0"
    );
    assert!(t.is_terminal());
}

/// A malformed SingleFrame (FD escape with no length byte, or a declared
/// length the frame does not actually carry) must fail with a clear
/// "malformed"/"truncated" reason — not be silently decoded as a short or
/// empty payload that then reads as an ordinary response mismatch.
#[test]
fn scripted_tester_rejects_malformed_single_frame() {
    let mk = || {
        let mut t = CanUdsTester::new("t".into(), "fdcan1".into());
        t.reply_id = 0x7E8;
        t.script = vec![UdsStep {
            send: vec![0x22, 0xF1, 0x90],
            expect: vec![Some(0x62), Some(0xF1), Some(0x90)],
            expect_nrc: None,
        }];
        t.state = CanUdsTesterState::AwaitResp;
        t
    };

    // FD escape SF (byte0 = 0x00) with no length byte.
    let mut t = mk();
    assert!(t.observe_ecu_frame(0x7E8, &[0x00]).is_none());
    assert_eq!(t.state, CanUdsTesterState::Failed);
    assert!(
        t.failure.as_deref().unwrap_or("").contains("malformed"),
        "expected a malformed-frame reason, got {:?}",
        t.failure
    );

    // SF that declares 20 payload bytes but carries only one.
    let mut t = mk();
    assert!(t.observe_ecu_frame(0x7E8, &[0x00, 0x14, 0x62]).is_none());
    assert_eq!(t.state, CanUdsTesterState::Failed);
    assert!(
        t.failure.as_deref().unwrap_or("").contains("truncated"),
        "expected a truncated-frame reason, got {:?}",
        t.failure
    );
}

/// End-to-end against a real `BxCan` registered on the bus and configured
/// (valid BTR + accept-0x111 filter, NORMAL mode — no loopback) so
/// `deliver_rx` accepts the tester's frames. We drive the full bus tick:
/// FF → (ECU emits FlowControl) → CF → (ECU emits positive response) → Done.
/// The ECU's "transmit" side is modeled by pushing frames into the bxCAN's
/// public `tx_frames`, which the tester drains exactly as it would for a
/// firmware-driven controller in normal mode.
#[test]
fn uds_tester_completes_against_real_bxcan() {
    use crate::peripherals::bxcan::BxCan;

    // bxCAN register offsets (RM0008 §24.9) addressed via the bus.
    const MCR: u64 = 0x000;
    const BTR: u64 = 0x01C;
    const FMR: u64 = 0x200;
    const FM1R: u64 = 0x204;
    const FS1R: u64 = 0x20C;
    const FFA1R: u64 = 0x214;
    const FA1R: u64 = 0x21C;
    const FBANK: u64 = 0x240;
    const VALID_BTR: u32 = 0x00DC_0009; // valid TS1/TS2, no loopback bit.

    let base: u64 = 0x4000_6400;
    let mut bus = SystemBus::empty();
    bus.add_peripheral("bxcan1", base, 0x400, None, Box::new(BxCan::new()));

    // Bring the controller up in NORMAL mode and install a bank-0 mask
    // filter accepting exactly 0x111 into FIFO0.
    bus.write_u32(base + MCR, 1).unwrap(); // INRQ: request init
    bus.write_u32(base + BTR, VALID_BTR).unwrap(); // valid timing, NOT loopback
    bus.write_u32(base + FMR, 1).unwrap(); // FINIT: filter init
    bus.write_u32(base + FS1R, 0x1).unwrap(); // bank0 32-bit
    bus.write_u32(base + FM1R, 0x0).unwrap(); // bank0 mask mode
    bus.write_u32(base + FFA1R, 0x0).unwrap(); // bank0 -> FIFO0
    bus.write_u32(base + FBANK, (0x111u32) << 21).unwrap(); // F0R1
    bus.write_u32(base + FBANK + 4, (0x111u32) << 21).unwrap(); // F0R2 mask
    bus.write_u32(base + FA1R, 0x1).unwrap(); // bank0 active
    bus.write_u32(base + FMR, 0x0).unwrap(); // clear FINIT: filters live
    bus.write_u32(base + MCR, 0).unwrap(); // leave init -> running (normal)

    bus.can_uds_testers
        .push(CanUdsTester::new("uds".into(), "bxcan1".into()));

    // Tick 1: tester injects the FirstFrame (filter accepts) → AwaitFc.
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::AwaitFc);

    // The injected FF landed in the ECU's RX FIFO0 (filter-accepted).
    {
        let idx = bus.find_peripheral_index_by_name("bxcan1").unwrap();
        let bx = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<BxCan>()
            .unwrap();
        // ECU "transmits" a FlowControl frame in normal mode (id = reply_id).
        bx.tx_frames.push_back(crate::network::CanFrame::classic(
            0x222,
            vec![0x30, 0x00, 0x00, 0, 0, 0, 0, 0],
        ));
    }

    // Tick 2: tester drains the FlowControl and injects the CF → AwaitResp.
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::AwaitResp);

    // ECU "transmits" the SecurityAccess positive single-frame response.
    {
        let idx = bus.find_peripheral_index_by_name("bxcan1").unwrap();
        let bx = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<BxCan>()
            .unwrap();
        bx.tx_frames.push_back(crate::network::CanFrame::classic(
            0x222,
            vec![0x06, 0x67, 0x01, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE],
        ));
    }

    // Tick 3: tester observes the positive response → Done.
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
}

/// End-to-end against a real `Fdcan` registered on the bus and brought to
/// normal mode (CCCR.INIT cleared, TEST.LBCK = 0) so `receive_frame`
/// accepts the tester's frames. We drive a script-driven exchange where the
/// ECU responds with a multi-frame FirstFrame + CF, exercising the
/// FlowControl-delivery path on FDCAN — the same gap class that #343
/// identified on the analogous bxCAN path.
///
/// Exchange (script: `send = "22 F1 90"`, `expect = "62 F1 90"`):
/// 1. Tick 1 (Start): tester sends SF ReadDataById request → AwaitResp.
/// 2. ECU replies with a FirstFrame (13-byte response) via `tx_frames`.
/// 3. Tick 2 (AwaitResp → AwaitMultiResp): tester sees the FF, transitions,
///    and MUST inject a FlowControl ([0x30, 0x00, 0x00]) via `receive_frame`.
/// 4. ECU replies with the ConsecutiveFrame.
/// 5. Tick 3 (AwaitMultiResp → Done): PDU reassembled and matched.
///
/// The discriminating assertion is the presence of a FlowControl entry
/// (`first_byte & 0xF0 == 0x30`) in the FDCAN "rx" trace after tick 2,
/// proving `receive_frame` was called — not merely that Done was reached.
#[test]
fn uds_tester_completes_against_real_fdcan() {
    use crate::peripherals::fdcan::Fdcan;

    // FDCAN1 on H563: RM0481 base 0x4000_A400.
    const FDCAN_BASE: u64 = 0x4000_A400;
    const REG_CCCR: u64 = 0x018; // CCCR offset within the peripheral window

    let mut bus = SystemBus::empty();
    bus.add_peripheral("fdcan1", FDCAN_BASE, 0x1000, None, Box::new(Fdcan::new()));

    // Bring FDCAN to normal mode (mirrors fdcan_start in h563-uds-ecu/main.c):
    //   Step 1: assert INIT + CCE (config unlock).
    bus.write_u32(FDCAN_BASE + REG_CCCR, 0x3).unwrap();
    //   Step 2: clear INIT — CCE clears with it (capture13: 0xA2→0xA0).
    bus.write_u32(FDCAN_BASE + REG_CCCR, 0x0).unwrap();
    // CCCR now reads 0x0: bus_active = true, receive_frame will accept frames.

    // Script step: ReadDataByIdentifier 0xF190 (3 bytes), expect prefix 62 F1 90.
    // The response is multi-frame (13 bytes), so the tester must send a
    // FlowControl when the ECU sends its FirstFrame.
    let mut tester = CanUdsTester::new("uds".into(), "fdcan1".into());
    tester.request_id = 0x7E0;
    tester.reply_id = 0x7E8;
    tester.script = vec![UdsStep {
        send: vec![0x22, 0xF1, 0x90],
        expect: SystemBus::parse_expect("62 F1 90"),
        expect_nrc: None,
    }];
    bus.can_uds_testers.push(tester);

    // Tick 1: script-driven Start → tester sends SF request (3 bytes fit in SF)
    // → AwaitResp (no pending CFs for a SF request).
    bus.service_can_uds_testers();
    assert_eq!(
        bus.can_uds_testers[0].state,
        CanUdsTesterState::AwaitResp,
        "state must be AwaitResp after tester sends its SF request"
    );

    // Record trace length after tick 1 so the FC check ignores the SF request.
    let trace_len_before = {
        let idx = bus.find_peripheral_index_by_name("fdcan1").unwrap();
        let fd = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Fdcan>()
            .unwrap();
        fd.trace_snapshot("fdcan1").len()
    };

    // ECU "transmits" a FirstFrame: 13-byte (0x0D) response, 6 payload bytes
    // in the FF (62 F1 90 = RDBI positive response prefix + 3 VIN chars).
    {
        let idx = bus.find_peripheral_index_by_name("fdcan1").unwrap();
        let fd = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Fdcan>()
            .unwrap();
        fd.tx_frames.push_back(crate::network::CanFrame::classic(
            0x7E8,
            vec![0x10, 0x0D, 0x62, 0xF1, 0x90, 0x31, 0x32, 0x33],
        ));
    }

    // Tick 2: tester drains the ECU FirstFrame → observe_ecu_frame_script sees
    // a 0x10 frame in AwaitResp, sets state = AwaitMultiResp, and returns the
    // FlowControl payload [0x30, 0x00, 0x00].
    // service_can_uds_testers picks that up in the AwaitMultiResp branch and
    // MUST call receive_frame to inject it onto the FDCAN.
    bus.service_can_uds_testers();
    assert_eq!(
        bus.can_uds_testers[0].state,
        CanUdsTesterState::AwaitMultiResp,
        "state must be AwaitMultiResp after receiving ECU FirstFrame"
    );

    // Discriminating assertion: a FlowControl frame (first byte & 0xF0 == 0x30)
    // with the tester's request_id (0x7E0) must appear as an "rx" entry in the
    // FDCAN trace after tick 1.  An absent FC means the tester silently dropped
    // the CTS signal — the FDCAN analogue of the bxCAN #343 bug.
    {
        let idx = bus.find_peripheral_index_by_name("fdcan1").unwrap();
        let fd = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Fdcan>()
            .unwrap();
        let trace = fd.trace_snapshot("fdcan1");
        let new_frames = &trace[trace_len_before..];
        assert!(
            new_frames.iter().any(|f| {
                f.direction == "rx"
                    && f.id == 0x7E0
                    && f.data.first().map(|b| b & 0xF0 == 0x30).unwrap_or(false)
            }),
            "FlowControl (0x30 nibble) must appear in FDCAN rx trace after ECU FirstFrame; \
             new frames after tick 1: {:?}",
            new_frames
                .iter()
                .map(|f| (f.direction.as_str(), f.id, f.data.clone()))
                .collect::<Vec<_>>()
        );
    }

    // ECU "transmits" the ConsecutiveFrame carrying the remaining 7 bytes.
    // 13 - 6 (from FF) = 7 bytes in the CF.
    {
        let idx = bus.find_peripheral_index_by_name("fdcan1").unwrap();
        let fd = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Fdcan>()
            .unwrap();
        fd.tx_frames.push_back(crate::network::CanFrame::classic(
            0x7E8,
            vec![0x21, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x30],
        ));
    }

    // Tick 3: tester drains the CF, PDU buf reaches the declared 13 bytes,
    // complete_response matches the expect prefix → Done.
    bus.service_can_uds_testers();
    assert_eq!(
        bus.can_uds_testers[0].state,
        CanUdsTesterState::Done,
        "state must be Done after CF received and PDU matched"
    );
}

/// Config parsing: a `uds-tester` external device populates a
/// `CanUdsTester` with the configured ids and payloads.
#[test]
fn uds_tester_parsed_from_config() {
    let chip: ChipDescriptor = serde_yaml::from_str(
        r#"
name: "f103"
arch: "arm"
core: "cortex-m3"
flash:
  base: 0x08000000
  size: "128KB"
ram:
  base: 0x20000000
  size: "20KB"
peripherals:
  - id: "bxcan1"
    type: "bxcan"
    base_address: 0x40006400
    size: "1KB"
"#,
    )
    .unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: "uds-multiframe"
chip: "f103"
external_devices:
  - id: "uds_node"
    type: "uds-tester"
    connection: "bxcan1"
    config:
      request_id: "0x111"
      reply_id: "0x222"
      first_frame: "10 0B 27 01 5A 11 22 33"
      consecutive_frame: "21 44 55 66 77 88 55 55"
board_io: []
"#,
    )
    .unwrap();
    let bus = SystemBus::from_config(&chip, &manifest).unwrap();
    assert_eq!(bus.can_uds_testers.len(), 1);
    let t = &bus.can_uds_testers[0];
    assert_eq!(t.request_id, 0x111);
    assert_eq!(t.reply_id, 0x222);
    assert_eq!(t.first_frame, CanUdsTester::DEFAULT_FIRST_FRAME.to_vec());
    assert_eq!(
        t.consecutive_frame,
        CanUdsTester::DEFAULT_CONSECUTIVE_FRAME.to_vec()
    );
    assert_eq!(t.state, CanUdsTesterState::Start);
}

/// Minimal F103 chip yaml reused across UDS script tests.
const MIN_F103_CHIP: &str = r#"
name: "f103"
arch: "arm"
core: "cortex-m3"
flash:
  base: 0x08000000
  size: "128KB"
ram:
  base: 0x20000000
  size: "20KB"
peripherals:
  - id: "bxcan1"
    type: "bxcan"
    base_address: 0x40006400
    size: "1KB"
"#;

#[test]
fn uds_script_parses_send_expect_and_wildcards() {
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: "uds-script"
chip: "f103"
external_devices:
  - id: "uds-tester"
    type: "uds-tester"
    connection: "bxcan1"
    config:
      request_id: "0x111"
      reply_id: "0x222"
      script:
        - send: "11 01"
          expect: "51 01"
        - send: "27 01"
          expect: "67 01 .."
board_io: []
"#,
    )
    .unwrap();
    let chip: ChipDescriptor = serde_yaml::from_str(MIN_F103_CHIP).unwrap();
    let bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let t = &bus.can_uds_testers[0];
    assert_eq!(t.script.len(), 2);
    assert_eq!(t.script[0].send, vec![0x11, 0x01]);
    assert_eq!(t.script[0].expect, vec![Some(0x51), Some(0x01)]);
    assert_eq!(t.script[1].expect, vec![Some(0x67), Some(0x01), None]); // .. = wildcard
}

#[test]
fn uds_script_parses_optional_expect_nrc() {
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: "uds-script-opts"
chip: "f103"
external_devices:
  - id: "uds-tester"
    type: "uds-tester"
    connection: "bxcan1"
    config:
      request_id: "0x111"
      reply_id: "0x222"
      script:
        - send: "28 03"
          expect: "68 03"
          expect_nrc: "0x22"
board_io: []
"#,
    )
    .unwrap();
    let chip: ChipDescriptor = serde_yaml::from_str(MIN_F103_CHIP).unwrap();
    let bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let step = &bus.can_uds_testers[0].script[0];
    assert_eq!(step.expect_nrc, Some(0x22));
}

#[test]
fn uds_legacy_config_becomes_one_step_script() {
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: "uds-legacy"
chip: "f103"
external_devices:
  - id: "uds_node"
    type: "uds-tester"
    connection: "bxcan1"
    config:
      request_id: "0x111"
      reply_id: "0x222"
      first_frame: "10 0B 27 01 5A 11 22 33"
      consecutive_frame: "21 44 55 66 77 88 55 55"
board_io: []
"#,
    )
    .unwrap();
    let chip: ChipDescriptor = serde_yaml::from_str(MIN_F103_CHIP).unwrap();
    let bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let t = &bus.can_uds_testers[0];
    assert_eq!(t.script.len(), 1);
    assert_eq!(t.script[0].expect, vec![Some(0x06), Some(0x67)]);
    assert_eq!(
        t.script[0].send,
        vec![0x27, 0x01, 0x5A, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]
    );
    assert_eq!(t.script[0].expect_nrc, None);
}

/// Config parsing: a `can-player` external device with inline `data:`
/// attaches a `CanLogPlayer` to the bus with the parsed frames.
#[test]
fn can_player_from_config_attaches_replayer() {
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: "can-player-attach"
chip: "f103"
external_devices:
  - id: "p"
    type: "can-player"
    connection: "bxcan1"
    config:
      data: "(1.0) can0 123#11\n"
board_io: []
"#,
    )
    .unwrap();
    let chip: ChipDescriptor = serde_yaml::from_str(MIN_F103_CHIP).unwrap();
    let bus = SystemBus::from_config(&chip, &manifest).unwrap();
    assert_eq!(bus.can_log_players.len(), 1);
    assert_eq!(bus.can_log_players[0].frames.len(), 1);
}

/// Config parsing: a `can-player` device whose `connection` doesn't name
/// a real peripheral on the bus fails with an error naming the device.
#[test]
fn can_player_from_config_errors_on_missing_connection() {
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: "can-player-bad-conn"
chip: "f103"
external_devices:
  - id: "p"
    type: "can-player"
    connection: "nope"
    config:
      data: "(1.0) can0 123#11\n"
board_io: []
"#,
    )
    .unwrap();
    let chip: ChipDescriptor = serde_yaml::from_str(MIN_F103_CHIP).unwrap();
    let err = expect_from_config_error(&chip, &manifest);
    let msg = err.to_string();
    assert!(msg.contains("can-player 'p'"), "unexpected error: {msg}");
}

/// Config parsing: a `can-player` device with neither `path` nor `data`
/// (post config-crate path-inlining, only `data` ever reaches core)
/// fails with an error naming both keys.
#[test]
fn can_player_from_config_errors_when_neither_path_nor_data_present() {
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: "can-player-no-data"
chip: "f103"
external_devices:
  - id: "p"
    type: "can-player"
    connection: "bxcan1"
    config: {}
board_io: []
"#,
    )
    .unwrap();
    let chip: ChipDescriptor = serde_yaml::from_str(MIN_F103_CHIP).unwrap();
    let err = expect_from_config_error(&chip, &manifest);
    let msg = err.to_string();
    assert!(msg.contains("path"), "unexpected error: {msg}");
    assert!(msg.contains("data"), "unexpected error: {msg}");
}

/// Config parsing: an explicit `ticks_per_second:` on a `can-player`
/// device actually reaches the attached `CanLogPlayer` — two frames 1.0s
/// apart at 2 ticks/sec rebase to ticks 0 and 2.
#[test]
fn can_player_from_config_honors_ticks_per_second_override() {
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: "can-player-tps"
chip: "f103"
external_devices:
  - id: "p"
    type: "can-player"
    connection: "bxcan1"
    config:
      ticks_per_second: 2
      data: "(1.0) can0 123#11\n(2.0) can0 123#22\n"
board_io: []
"#,
    )
    .unwrap();
    let chip: ChipDescriptor = serde_yaml::from_str(MIN_F103_CHIP).unwrap();
    let bus = SystemBus::from_config(&chip, &manifest).unwrap();
    assert_eq!(bus.can_log_players[0].frames.len(), 2);
    assert_eq!(bus.can_log_players[0].frames[0].0, 0);
    assert_eq!(bus.can_log_players[0].frames[1].0, 2);
}

/// Config parsing: omitting `ticks_per_second:` defaults to
/// 1_000_000 ticks/sec — two frames 100µs apart rebase to tick 100.
#[test]
fn can_player_from_config_defaults_ticks_per_second() {
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: "can-player-tps-default"
chip: "f103"
external_devices:
  - id: "p"
    type: "can-player"
    connection: "bxcan1"
    config:
      data: "(10.000000) can0 123#11\n(10.000100) can0 123#22\n"
board_io: []
"#,
    )
    .unwrap();
    let chip: ChipDescriptor = serde_yaml::from_str(MIN_F103_CHIP).unwrap();
    let bus = SystemBus::from_config(&chip, &manifest).unwrap();
    assert_eq!(bus.can_log_players[0].frames.len(), 2);
    assert_eq!(bus.can_log_players[0].frames[0].0, 0);
    assert_eq!(bus.can_log_players[0].frames[1].0, 100);
}

/// Parse a minimal chip yaml with the given header lines (name/arch/core).
fn bit_band_test_chip(header: &str, gpio_base: &str, gpio_profile: &str) -> ChipDescriptor {
    let yaml = format!(
        r#"
{header}
flash:
  base: 0x08000000
  size: "128KB"
ram:
  base: 0x20000000
  size: "64KB"
peripherals:
  - id: "gpiox"
    type: "gpio"
    base_address: {gpio_base}
    size: "1KB"
    config:
      profile: "{gpio_profile}"
"#
    );
    serde_yaml::from_str(&yaml).expect("test chip yaml must parse")
}

fn empty_manifest() -> SystemManifest {
    SystemManifest {
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "bit-band-test".to_string(),
        chip: "unused".to_string(),
        memory_overrides: std::collections::HashMap::new(),
        external_devices: Vec::new(),
        board_io: Vec::new(),
        debug_uart: None,
        peripherals: Vec::new(),
    }
}

/// Cortex-M33 parts (STM32H5/WBA) have no bit-band feature and map real
/// peripherals inside 0x4200_0000-0x43FF_FFFF. Word accesses there must
/// reach the peripheral model, never be alias-translated.
#[test]
fn from_config_m33_gpio_in_alias_range_receives_word_accesses() {
    let chip = bit_band_test_chip(
        "name: \"m33-test\"\narch: \"arm\"\ncore: \"cortex-m33\"",
        "0x42020400",
        "stm32v2",
    );
    let mut bus = SystemBus::from_config(&chip, &empty_manifest()).unwrap();

    // Go through the `crate::Bus` trait — the CPU's access path, where
    // bit-band translation lives (the inherent methods skip it).
    // BSRR (V2 offset 0x18): set pin 0.
    crate::Bus::write_u32(&mut bus, 0x4202_0418, 0x0000_0001)
        .expect("BSRR word write must reach the GPIO model, not bit-band");
    // ODR (V2 offset 0x14) must show the pin high.
    let odr = crate::Bus::read_u32(&bus, 0x4202_0414)
        .expect("ODR word read must reach the GPIO model, not bit-band");
    assert_eq!(odr & 1, 1, "GPIO BSRR write was shadowed by bit-band alias");
}

/// Cortex-M3 parts (STM32F1) DO have the bit-band feature: word accesses
/// to the 0x4200_0000 alias region must keep translating to single-bit
/// operations on the underlying 0x4000_0000 peripheral registers.
#[test]
fn from_config_m3_bit_band_alias_still_translates() {
    let chip = bit_band_test_chip(
        "name: \"m3-test\"\narch: \"arm\"\ncore: \"cortex-m3\"",
        "0x40011000",
        "stm32f1",
    );
    let mut bus = SystemBus::from_config(&chip, &empty_manifest()).unwrap();

    // Alias word for GPIOC_ODR (0x4001100C) bit 0:
    // 0x42000000 + (0x1100C * 32) + (0 * 4) = 0x42220180.
    // Trait path (`crate::Bus`) — the CPU's access path with bit-band.
    crate::Bus::write_u32(&mut bus, 0x4222_0180, 1)
        .expect("bit-band alias write must translate on M3");
    let odr = crate::Bus::read_u32(&bus, 0x4001_100C).unwrap();
    assert_eq!(odr & 1, 1, "bit-band alias write must set ODR bit 0");
    assert_eq!(
        crate::Bus::read_u32(&bus, 0x4222_0180).unwrap(),
        1,
        "bit-band alias read must return the physical bit"
    );
}

/// Bit-band gating matrix: only M3/M4 cores have the feature. Absent
/// core info on an Arm chip preserves the historical default (enabled)
/// for configs that predate the `core` field.
#[test]
fn from_config_bit_band_gated_on_core() {
    let manifest = empty_manifest();
    let cases: &[(&str, bool)] = &[
        ("core: \"cortex-m3\"", true),
        ("core: \"cortex-m4\"", true),
        ("core: \"cortex-m0+\"", false),
        ("core: \"cortex-m7\"", false),
        ("core: \"cortex-m23\"", false),
        ("core: \"cortex-m33\"", false),
        ("", true), // absent core on Arm: historical default
    ];
    for (core_line, expected) in cases {
        let header = format!("name: \"gate-test\"\narch: \"arm\"\n{core_line}");
        let chip = bit_band_test_chip(&header, "0x40011000", "stm32f1");
        let bus = SystemBus::from_config(&chip, &manifest).unwrap();
        assert_eq!(
            bus.bit_band_enabled, *expected,
            "bit_band_enabled mismatch for chip header {header:?}"
        );
    }
}

fn chip_with_i2c_and_uart() -> labwired_config::ChipDescriptor {
    use labwired_config::{Arch, MemoryRange, PeripheralConfig};
    use std::collections::HashMap;

    labwired_config::ChipDescriptor {
        schema_version: "1.0".to_string(),
        reset_vector_offset: 0,
        atomic_register_aliases: false,
        memory_regions: Vec::new(),
        name: "stm32f103-test".to_string(),
        arch: Arch::Arm,
        core: None,
        flash: MemoryRange {
            base: 0x0800_0000,
            size: "64KB".to_string(),
        },
        ram: MemoryRange {
            base: 0x2000_0000,
            size: "20KB".to_string(),
        },
        peripherals: vec![
            PeripheralConfig {
                id: "i2c1".to_string(),
                r#type: "i2c".to_string(),
                base_address: 0x4000_5400,
                size: Some("1KB".to_string()),
                irq: Some(31),
                clock: None,
                config: HashMap::new(),
            },
            PeripheralConfig {
                id: "uart1".to_string(),
                r#type: "uart".to_string(),
                base_address: 0x4000_3800,
                size: Some("1KB".to_string()),
                irq: Some(37),
                clock: None,
                config: HashMap::new(),
            },
        ],
        pins: Default::default(),
    }
}

fn manifest_with_external_device(
    r#type: &str,
    connection: &str,
    config: std::collections::HashMap<String, serde_yaml::Value>,
) -> labwired_config::SystemManifest {
    labwired_config::SystemManifest {
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "adxl345-test".to_string(),
        chip: "../chips/stm32f103.yaml".to_string(),
        memory_overrides: std::collections::HashMap::new(),
        external_devices: vec![labwired_config::ExternalDevice {
            id: "sensor1".to_string(),
            r#type: r#type.to_string(),
            connection: connection.to_string(),
            route: Default::default(),
            config,
        }],
        board_io: Vec::new(),
        debug_uart: None,
        peripherals: Vec::new(),
    }
}

fn assert_external_device_error_contains_context(
    err: anyhow::Error,
    ext_type: &str,
    connection: &str,
) {
    let message = err.to_string();
    assert!(
        message.contains("sensor1"),
        "error missing external device id: {message}"
    );
    assert!(
        message.contains(ext_type),
        "error missing external device type: {message}"
    );
    assert!(
        message.contains(connection),
        "error missing external device connection: {message}"
    );
}

fn expect_from_config_error(
    chip: &labwired_config::ChipDescriptor,
    manifest: &labwired_config::SystemManifest,
) -> anyhow::Error {
    match SystemBus::from_config(chip, manifest) {
        Ok(_) => panic!("expected SystemBus::from_config to reject manifest"),
        Err(err) => err,
    }
}

#[test]
fn test_from_config_errors_for_missing_external_device_connection() {
    let chip = chip_with_i2c_and_uart();
    let manifest =
        manifest_with_external_device("adxl345", "missing-i2c", std::collections::HashMap::new());

    let err = expect_from_config_error(&chip, &manifest);

    assert_external_device_error_contains_context(err, "adxl345", "missing-i2c");
}

#[test]
fn test_from_config_errors_for_external_device_on_non_i2c_connection() {
    let chip = chip_with_i2c_and_uart();
    let manifest =
        manifest_with_external_device("adxl345", "uart1", std::collections::HashMap::new());

    let err = expect_from_config_error(&chip, &manifest);

    assert_external_device_error_contains_context(err, "adxl345", "uart1");
}

#[test]
fn test_from_config_skips_unsupported_external_device_type() {
    let chip = chip_with_i2c_and_uart();
    let mut config = std::collections::HashMap::new();
    config.insert(
        "i2c_address".to_string(),
        serde_yaml::Value::Number(0x48.into()),
    );
    // Use a clearly-fictional device type — tmp102/adxl345/etc. are all
    // real components now, so we need something the factory will refuse.
    let manifest = manifest_with_external_device("definitely_not_a_device", "i2c1", config);

    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let i2c_idx = bus.find_peripheral_index_by_name("i2c1").unwrap();
    let any = bus.peripherals[i2c_idx].dev.as_any_mut().unwrap();
    let i2c = any.downcast_mut::<crate::peripherals::i2c::I2c>().unwrap();

    assert_eq!(i2c.attached_devices().len(), 0);
}

#[test]
fn test_from_config_errors_for_invalid_external_device_i2c_address() {
    for value in [
        serde_yaml::Value::String("0x53".to_string()),
        serde_yaml::Value::Number(0x80.into()),
    ] {
        let chip = chip_with_i2c_and_uart();
        let mut config = std::collections::HashMap::new();
        config.insert("i2c_address".to_string(), value);
        let manifest = manifest_with_external_device("adxl345", "i2c1", config);

        let err = expect_from_config_error(&chip, &manifest);

        assert_external_device_error_contains_context(err, "adxl345", "i2c1");
    }
}

#[test]
fn test_system_bus_memory_observer() {
    use std::sync::Arc;
    use std::sync::Mutex;

    #[derive(Debug)]
    struct MockObserver {
        writes: Arc<Mutex<Vec<(u64, u8, u8)>>>,
    }

    impl crate::SimulationObserver for MockObserver {
        fn on_step_end(&self, _cycles: u32, _registers: &[u32]) {}
        fn on_memory_write(&self, addr: u64, old: u8, new: u8) {
            self.writes.lock().unwrap().push((addr, old, new));
        }
    }

    let writes = Arc::new(Mutex::new(Vec::new()));
    let mut bus = SystemBus::new();
    bus.observers.push(Arc::new(MockObserver {
        writes: writes.clone(),
    }));

    // Write to RAM (e.g., 0x20000000)
    bus.write_u8(0x20000000, 0xAA).unwrap();
    {
        let w = writes.lock().unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0], (0x20000000, 0, 0xAA));
    }

    // Write to Peripheral (e.g., UART at 0x4000C000)
    bus.write_u8(0x4000C000, 0xBB).unwrap();
    {
        let w = writes.lock().unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!(w[1], (0x4000C000, 0xC0, 0xBB));
    }
}

#[test]
fn test_flash_boot_alias_read_and_write() {
    let mut bus = SystemBus {
        flash: LinearMemory::new(256, 0x0800_0000),
        ram: LinearMemory::new(256, 0x2000_0000),
        extra_mem: Vec::new(),
        peripherals: Vec::new(),
        nvic: None,
        observers: Vec::new(),
        config: crate::SimulationConfig::default(),
        bit_band_enabled: true,
        pending_cpu_irqs: [0; 2],
        dport_idx: None,
        rcc_idx: None,
        clock_gating_bypass: false,
        fault_unclocked: std::collections::HashMap::new(),
        flash_thunks: std::collections::HashMap::new(),
        peripheral_ranges: Vec::new(),
        legacy_tick_indices: Vec::new(),
        bus_tick_indices: Vec::new(),
        scheduler_driver_indices: Vec::new(),
        matrix_source_scratch: Vec::new(),
        peripheral_hint: Cell::new(None),
        last_route: Cell::new(None),
        last_gap: Cell::new(None),
        last_gpio_in: [0; 2],
        current_cycle: 0,
        cycle_clock: crate::CycleClock::default(),
        pending_schedule: Vec::new(),
        freerunning_timer_poll_mmio: std::cell::Cell::new(0),
        side_effecting_mmio: std::cell::Cell::new(0),
        legacy_walk_disabled: false,
        reset_vector_offset: 0,
        atomic_register_aliases: false,
        hcsr04: Vec::new(),
        gpio_devices: Vec::new(),
        ws2812: Vec::new(),
        tm1637: Vec::new(),
        seven_segment: Vec::new(),
        analog_inputs: Vec::new(),
        can_diagnostic_testers: Vec::new(),
        can_uds_testers: Vec::new(),
        can_log_players: Vec::new(),
        esp32c3_irq_routing: false,
        riscv_irq_lines: 0,
        esp32c3_system_idx: None,
        esp32c3_interrupt_core0_idx: None,
        esp32c3_irq_cache: None,
        esp32c3_asserted_sources: [0; 2],
        esp32c3_sched_asserted_sources: [0; 2],
        esp32s3_irq_routing: false,
        esp32s3_intmatrix_idx: None,
        esp32s3_asserted_sources: [0; 2],
        esp32s3_sched_asserted_sources: [0; 2],
        flash_models_ops: false,
        iolink_master_attached: false,
        nordic_gpio_service: false,
        hcsr04_scheduling_disabled: false,
        flash_error_flags_idx: None,
        bus_trace: bus_trace::new_log(),
        logic_tap: crate::logic_capture::LogicTap::new(),
        pin_map: std::collections::HashMap::new(),
    };

    bus.flash.write_u8(0x0800_0000, 0x12);
    bus.flash.write_u8(0x0800_0001, 0x34);

    // Read through aliased 0x0000_0000 boot window.
    assert_eq!(bus.read_u8(0x0000_0000).unwrap(), 0x12);
    assert_eq!(bus.read_u8(0x0000_0001).unwrap(), 0x34);

    // Write through alias and verify backing flash changed.
    bus.write_u8(0x0000_0001, 0xAB).unwrap();
    assert_eq!(bus.flash.read_u8(0x0800_0001), Some(0xAB));
}

/// Build a bus with a 1 KiB flash region (erased to 0xFF, like real silicon
/// after erase) and an H5 FLASH register peripheral at 0x4002_2000, with the
/// opt-in program-error gate set to `gate`.
fn h5_flash_bus(gate: bool) -> SystemBus {
    let mut flash = LinearMemory::new(0x400, 0x0800_0000);
    // Erased state is all-ones; the gate's not-erased check keys off this.
    flash.data.iter_mut().for_each(|b| *b = 0xFF);
    let mut bus = SystemBus {
        flash,
        ram: LinearMemory::new(256, 0x2000_0000),
        extra_mem: Vec::new(),
        peripherals: vec![PeripheralEntry {
            name: "flash".to_string(),
            base: 0x4002_2000,
            size: 0x400,
            irq: None,
            dev: Box::new(
                crate::peripherals::flash::Flash::new_with_layout(
                    crate::peripherals::flash::FlashRegisterLayout::Stm32H5,
                )
                .with_error_flags(gate),
            ),
            ticks_remaining: 0,
            clock_gate: None,
        }],
        nvic: None,
        observers: Vec::new(),
        config: crate::SimulationConfig::default(),
        bit_band_enabled: false,
        pending_cpu_irqs: [0; 2],
        dport_idx: None,
        rcc_idx: None,
        clock_gating_bypass: false,
        fault_unclocked: std::collections::HashMap::new(),
        flash_thunks: std::collections::HashMap::new(),
        peripheral_ranges: Vec::new(),
        legacy_tick_indices: Vec::new(),
        bus_tick_indices: Vec::new(),
        scheduler_driver_indices: Vec::new(),
        matrix_source_scratch: Vec::new(),
        peripheral_hint: Cell::new(None),
        last_route: Cell::new(None),
        last_gap: Cell::new(None),
        last_gpio_in: [0; 2],
        current_cycle: 0,
        cycle_clock: crate::CycleClock::default(),
        pending_schedule: Vec::new(),
        freerunning_timer_poll_mmio: std::cell::Cell::new(0),
        side_effecting_mmio: std::cell::Cell::new(0),
        legacy_walk_disabled: false,
        reset_vector_offset: 0,
        atomic_register_aliases: false,
        hcsr04: Vec::new(),
        gpio_devices: Vec::new(),
        ws2812: Vec::new(),
        tm1637: Vec::new(),
        seven_segment: Vec::new(),
        analog_inputs: Vec::new(),
        can_diagnostic_testers: Vec::new(),
        can_uds_testers: Vec::new(),
        can_log_players: Vec::new(),
        esp32c3_irq_routing: false,
        riscv_irq_lines: 0,
        esp32c3_system_idx: None,
        esp32c3_interrupt_core0_idx: None,
        esp32c3_irq_cache: None,
        esp32c3_asserted_sources: [0; 2],
        esp32c3_sched_asserted_sources: [0; 2],
        esp32s3_irq_routing: false,
        esp32s3_intmatrix_idx: None,
        esp32s3_asserted_sources: [0; 2],
        esp32s3_sched_asserted_sources: [0; 2],
        flash_models_ops: false,
        iolink_master_attached: false,
        nordic_gpio_service: false,
        hcsr04_scheduling_disabled: false,
        flash_error_flags_idx: None,
        bus_trace: bus_trace::new_log(),
        logic_tap: crate::logic_capture::LogicTap::new(),
        pin_map: std::collections::HashMap::new(),
    };
    bus.rebuild_peripheral_ranges();
    bus
}

fn read_nssr(bus: &SystemBus) -> u32 {
    use crate::peripherals::flash::h5::NSSR_OFF;
    bus.read_u32(0x4002_2000 + NSSR_OFF).unwrap()
}

/// Enable NSCR.PG on the H5 FLASH peripheral so the write-buffer machine
/// programs (silicon requires PG for a flash-region write to land).
fn h5_set_pg(bus: &mut SystemBus) {
    use crate::peripherals::flash::h5;
    bus.write_u32(0x4002_2000 + h5::NSCR_OFF, h5::NSCR_PG)
        .unwrap();
}

#[test]
fn h5_gate_on_full_quadword_commits_as_and() {
    use crate::peripherals::flash::h5;
    let mut bus = h5_flash_bus(true);
    assert!(bus.flash_error_flags_idx.is_some(), "gate index cached");
    h5_set_pg(&mut bus);
    // Pre-load the quad-word at 0x08000020 with 0xAA in the first lane so the
    // commit must AND with it (flash only flips 1→0). Write the lower 15
    // lanes via the buffer first... but to exercise the AND we re-program a
    // committed quad-word below; here verify a clean commit from erased.
    for i in 0..16u64 {
        bus.write_u8(0x0800_0020 + i, 0x33).unwrap();
    }
    // 0xFF (erased) & 0x33 = 0x33 — full quad-word committed.
    for i in 0..16u64 {
        assert_eq!(bus.flash.read_u8(0x0800_0020 + i), Some(0x33));
    }
    assert_ne!(read_nssr(&bus) & h5::NSSR_EOP, 0, "EOP set on commit");
    assert_eq!(read_nssr(&bus) & h5::NSSR_WBNE, 0, "WBNE clear on commit");
}

#[test]
fn h5_gate_on_partial_quadword_buffers_no_commit() {
    use crate::peripherals::flash::h5;
    let mut bus = h5_flash_bus(true);
    h5_set_pg(&mut bus);
    // Only 4 of 16 bytes: still buffering, flash unchanged, WBNE set.
    for i in 0..4u64 {
        bus.write_u8(0x0800_0020 + i, 0x55).unwrap();
        assert_eq!(bus.flash.read_u8(0x0800_0020 + i), Some(0xFF), "not yet");
    }
    assert_ne!(read_nssr(&bus) & h5::NSSR_WBNE, 0, "WBNE set");
    assert_eq!(read_nssr(&bus) & h5::NSSR_EOP, 0, "no EOP");
}

#[test]
fn h5_gate_on_reprogram_committed_quadword_ands_no_pgserr() {
    use crate::peripherals::flash::h5;
    let mut bus = h5_flash_bus(true);
    h5_set_pg(&mut bus);
    // First program: 0xFF & 0xF0 = 0xF0.
    for i in 0..16u64 {
        bus.write_u8(0x0800_0040 + i, 0xF0).unwrap();
    }
    assert_eq!(bus.flash.read_u8(0x0800_0040), Some(0xF0));
    // Clear EOP via NSCCR, then re-program the SAME (now-not-erased) word.
    bus.write_u32(0x4002_2000 + h5::NSCCR_OFF, h5::NSSR_EOP)
        .unwrap();
    for i in 0..16u64 {
        bus.write_u8(0x0800_0040 + i, 0x0F).unwrap();
    }
    // Re-program ALLOWED, result is the AND: 0xF0 & 0x0F = 0x00. No PGSERR.
    assert_eq!(bus.flash.read_u8(0x0800_0040), Some(0x00), "AND of old&new");
    assert_eq!(read_nssr(&bus) & h5::NSSR_PGSERR, 0, "no PGSERR over-write");
    assert_ne!(read_nssr(&bus) & h5::NSSR_EOP, 0, "EOP set (success)");
}

#[test]
fn h5_gate_on_misaligned_run_sets_incerr_alone_no_commit() {
    use crate::peripherals::flash::h5;
    let mut bus = h5_flash_bus(true);
    h5_set_pg(&mut bus);
    // Start at base+4 (quad-word 0x20), then jump into the next quad-word
    // (0x30) before completing — an inconsistent program run.
    bus.write_u8(0x0800_0024, 0x11).unwrap();
    assert_ne!(read_nssr(&bus) & h5::NSSR_WBNE, 0, "WBNE while partial");
    bus.write_u8(0x0800_0030, 0x22).unwrap();
    // INCERR alone, nothing committed (both targets stay erased).
    assert_eq!(bus.flash.read_u8(0x0800_0024), Some(0xFF), "no commit");
    assert_eq!(bus.flash.read_u8(0x0800_0030), Some(0xFF), "no commit");
    let nssr = read_nssr(&bus);
    assert_ne!(nssr & h5::NSSR_INCERR, 0, "INCERR set");
    assert_eq!(nssr & h5::NSSR_PGSERR, 0, "INCERR alone (no PGSERR)");
}

#[test]
fn h5_gate_off_commits_every_program_with_no_flag() {
    use crate::peripherals::flash::h5;
    let mut bus = h5_flash_bus(false);
    assert!(bus.flash_error_flags_idx.is_none(), "gate off ⇒ no index");
    // No buffering, no flags: every byte commits straight through, even
    // misaligned and over-not-erased (old byte-identical behaviour).
    bus.write_u8(0x0800_0003, 0x42).unwrap();
    assert_eq!(bus.flash.read_u8(0x0800_0003), Some(0x42));
    bus.write_u8(0x0800_0003, 0x99).unwrap();
    assert_eq!(bus.flash.read_u8(0x0800_0003), Some(0x99));
    assert_eq!(read_nssr(&bus) & h5::NSSR_W1C_MASK, 0, "no flag ever");
    assert_eq!(read_nssr(&bus) & h5::NSSR_WBNE, 0, "no WBNE ever");
}

// ── H5 read-while-write fidelity gate (opt-in, default off) ─────────────

use crate::Cpu as _RwwCpuTrait;

/// Minimal CPU stub with a settable PC for the RWW Machine-level tests.
/// `step` is a no-op (the tests drive `apply_pending_flash_op` directly via
/// a manually recorded erase, so the CPU never needs to execute).
#[derive(Default)]
struct PcCpu {
    pc: u32,
}

impl crate::Cpu for PcCpu {
    fn reset(&mut self, _bus: &mut dyn crate::Bus) -> crate::SimResult<()> {
        Ok(())
    }
    fn step(
        &mut self,
        _bus: &mut dyn crate::Bus,
        _observers: &[std::sync::Arc<dyn crate::SimulationObserver>],
        _config: &crate::SimulationConfig,
    ) -> crate::SimResult<()> {
        Ok(())
    }
    fn set_pc(&mut self, val: u32) {
        self.pc = val;
    }
    fn get_pc(&self) -> u32 {
        self.pc
    }
    fn set_sp(&mut self, _val: u32) {}
    fn set_exception_pending(&mut self, _n: u32) {}
    fn get_register(&self, _id: u8) -> u32 {
        0
    }
    fn set_register(&mut self, _id: u8, _val: u32) {}
    fn snapshot(&self) -> crate::snapshot::CpuSnapshot {
        crate::snapshot::CpuSnapshot::Arm(crate::snapshot::ArmCpuSnapshot {
            registers: vec![0; 16],
            pc: self.pc,
            xpsr: 0,
            primask: false,
            pending_exceptions: 0,
            pending_exceptions_hi: Vec::new(),
            vtor: 0,
        })
    }
    fn apply_snapshot(&mut self, _snapshot: &crate::snapshot::CpuSnapshot) {}
    fn get_register_names(&self) -> Vec<String> {
        vec![]
    }
    fn index_of_register(&self, _name: &str) -> Option<u8> {
        None
    }
}

/// Build a bus with a 2 MiB flash region (two 1 MiB banks, as on the H563)
/// and an H5 FLASH register peripheral, with the opt-in read-while-write gate
/// set to `gate`. The flash is unlocked so a NSCR.SER|STRT write records an
/// erase op straight away.
fn h5_rww_bus(gate: bool) -> SystemBus {
    use crate::peripherals::flash::h5;
    let mut flash = LinearMemory::new((2 * h5::BANK_SIZE) as usize, h5::FLASH_BASE);
    flash.data.iter_mut().for_each(|b| *b = 0xFF);
    let mut bus = SystemBus {
        flash,
        ram: LinearMemory::new(0x1000, 0x2000_0000),
        extra_mem: Vec::new(),
        peripherals: vec![PeripheralEntry {
            name: "flash".to_string(),
            base: 0x4002_2000,
            size: 0x400,
            irq: None,
            dev: Box::new(
                crate::peripherals::flash::Flash::new_with_layout(
                    crate::peripherals::flash::FlashRegisterLayout::Stm32H5,
                )
                .with_read_while_write(gate),
            ),
            ticks_remaining: 0,
            clock_gate: None,
        }],
        nvic: None,
        observers: Vec::new(),
        config: crate::SimulationConfig::default(),
        bit_band_enabled: false,
        pending_cpu_irqs: [0; 2],
        dport_idx: None,
        rcc_idx: None,
        clock_gating_bypass: false,
        fault_unclocked: std::collections::HashMap::new(),
        flash_thunks: std::collections::HashMap::new(),
        peripheral_ranges: Vec::new(),
        legacy_tick_indices: Vec::new(),
        bus_tick_indices: Vec::new(),
        scheduler_driver_indices: Vec::new(),
        matrix_source_scratch: Vec::new(),
        peripheral_hint: Cell::new(None),
        last_route: Cell::new(None),
        last_gap: Cell::new(None),
        last_gpio_in: [0; 2],
        current_cycle: 0,
        cycle_clock: crate::CycleClock::default(),
        pending_schedule: Vec::new(),
        freerunning_timer_poll_mmio: std::cell::Cell::new(0),
        side_effecting_mmio: std::cell::Cell::new(0),
        legacy_walk_disabled: false,
        reset_vector_offset: 0,
        atomic_register_aliases: false,
        hcsr04: Vec::new(),
        gpio_devices: Vec::new(),
        ws2812: Vec::new(),
        tm1637: Vec::new(),
        seven_segment: Vec::new(),
        analog_inputs: Vec::new(),
        can_diagnostic_testers: Vec::new(),
        can_uds_testers: Vec::new(),
        can_log_players: Vec::new(),
        esp32c3_irq_routing: false,
        riscv_irq_lines: 0,
        esp32c3_system_idx: None,
        esp32c3_interrupt_core0_idx: None,
        esp32c3_irq_cache: None,
        esp32c3_asserted_sources: [0; 2],
        esp32c3_sched_asserted_sources: [0; 2],
        esp32s3_irq_routing: false,
        esp32s3_intmatrix_idx: None,
        esp32s3_asserted_sources: [0; 2],
        esp32s3_sched_asserted_sources: [0; 2],
        flash_models_ops: false,
        iolink_master_attached: false,
        nordic_gpio_service: false,
        hcsr04_scheduling_disabled: false,
        flash_error_flags_idx: None,
        bus_trace: bus_trace::new_log(),
        logic_tap: crate::logic_capture::LogicTap::new(),
        pin_map: std::collections::HashMap::new(),
    };
    bus.rebuild_peripheral_ranges();
    bus
}

/// Unlock NSKEYR then record a sector erase of `bank` (BKSEL logical) on the
/// bus, so a subsequent `apply_pending_flash_op` drains it.
fn h5_record_erase(bus: &mut SystemBus, bank: u8, sector: u32) {
    use crate::peripherals::flash::h5;
    bus.write_u32(0x4002_2000 + h5::NSKEYR_OFF, 0x4567_0123)
        .unwrap();
    bus.write_u32(0x4002_2000 + h5::NSKEYR_OFF, 0xCDEF_89AB)
        .unwrap();
    let mut nscr = h5::NSCR_SER | (sector << h5::NSCR_SNB_SHIFT) | h5::NSCR_STRT;
    if bank == 1 {
        nscr |= h5::NSCR_BKSEL;
    }
    bus.write_u32(0x4002_2000 + h5::NSCR_OFF, nscr).unwrap();
}

#[test]
fn rww_gate_on_same_bank_erase_faults() {
    use crate::peripherals::flash::h5;
    let mut cpu = PcCpu::default();
    // PC executing from bank 1 (boot view at 0x08000000), sector 11.
    cpu.set_pc(0x0801_6000);
    let mut bus = h5_rww_bus(true);
    h5_record_erase(&mut bus, 0, 11); // erase bank 1 (BKSEL=0), sector 11
    let mut machine = crate::Machine::new(cpu, bus);
    let err = machine
        .apply_pending_flash_op()
        .expect_err("same-bank erase under the RWW gate must fault");
    match err {
        crate::SimulationError::Other(msg) => {
            assert!(msg.contains("RWW"), "reason names the RWW violation: {msg}");
            assert!(
                msg.contains("SRAM"),
                "reason tells firmware to use SRAM: {msg}"
            );
        }
        other => panic!("expected SimulationError::Other, got {other:?}"),
    }
    // Faulted before the fill: the erased sector is NOT cleared to 0xFF by us
    // (it was already 0xFF), but more importantly the op did not silently
    // "succeed" — the error propagated.
    let _ = h5::BANK_SIZE;
}

#[test]
fn rww_gate_on_other_bank_erase_proceeds() {
    use crate::peripherals::flash::h5;
    let mut cpu = PcCpu::default();
    // PC in bank 1; erase targets bank 2 — the normal cross-bank OTA case.
    cpu.set_pc(0x0801_6000);
    let mut bus = h5_rww_bus(true);
    // Dirty the bank-2 boot-state sector so we can see the erase land.
    let off = h5::BANK_SIZE + 11 * h5::SECTOR_SIZE;
    bus.flash.write_u8(h5::FLASH_BASE + off, 0x00);
    h5_record_erase(&mut bus, 1, 11); // erase bank 2 (BKSEL=1)
    let mut machine = crate::Machine::new(cpu, bus);
    machine
        .apply_pending_flash_op()
        .expect("cross-bank erase must proceed");
    assert_eq!(
        machine.bus.flash.read_u8(h5::FLASH_BASE + off),
        Some(0xFF),
        "bank-2 sector erased to 0xFF"
    );
}

#[test]
fn rww_gate_on_pc_in_sram_never_faults() {
    // The intended production layout: the flash routine runs from SRAM, so
    // PC is not in any flash bank — even a same-(logical-)bank erase is fine.
    use crate::peripherals::flash::h5;
    let mut cpu = PcCpu::default();
    cpu.set_pc(0x2000_0100); // SRAM
    let mut bus = h5_rww_bus(true);
    h5_record_erase(&mut bus, 0, 11);
    let mut machine = crate::Machine::new(cpu, bus);
    machine
        .apply_pending_flash_op()
        .expect("erase from a SRAM-resident routine must proceed");
    let _ = h5::FLASH_BASE;
}

#[test]
fn rww_gate_on_respects_swap_bank_mapping() {
    // After a SWAP_BANK, the physical second bank answers at 0x08000000.
    // PC at 0x08000000 is then in physical bank 2; an erase that lands in
    // that physical bank (BKSEL=0, which now maps to physical bank 2) must
    // fault, while BKSEL=1 (physical bank 1, the inactive one) proceeds.
    use crate::peripherals::flash::h5;

    // Same-physical-bank under swap → fault.
    {
        let mut cpu = PcCpu::default();
        cpu.set_pc(0x0800_4000); // bank presented at 0x08000000
        let mut bus = h5_rww_bus(true);
        // Toggle the FLASH's swap state directly to model an applied swap.
        let idx = bus.find_peripheral_index_by_name("flash").unwrap();
        bus.peripherals[idx]
            .dev
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<crate::peripherals::flash::Flash>())
            .unwrap()
            .mark_swapped();
        h5_record_erase(&mut bus, 0, 2); // BKSEL=0 → physical bank 2 under swap
        let mut machine = crate::Machine::new(cpu, bus);
        let err = machine
            .apply_pending_flash_op()
            .expect_err("swapped: BKSEL=0 erase hits PC's physical bank");
        assert!(matches!(err, crate::SimulationError::Other(_)));
    }

    // Cross-physical-bank under swap → proceeds.
    {
        let mut cpu = PcCpu::default();
        cpu.set_pc(0x0800_4000);
        let mut bus = h5_rww_bus(true);
        let idx = bus.find_peripheral_index_by_name("flash").unwrap();
        bus.peripherals[idx]
            .dev
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<crate::peripherals::flash::Flash>())
            .unwrap()
            .mark_swapped();
        // BKSEL=1 → physical bank 1, which sits at buffer offset 0..1 MiB?
        // No: under swap, logical bank 1 maps to physical bank 0, the bank
        // NOT presented at 0x08000000 — the cross-bank case.
        let off = h5::BANK_SIZE + 2 * h5::SECTOR_SIZE;
        bus.flash.write_u8(h5::FLASH_BASE + off, 0x00);
        h5_record_erase(&mut bus, 1, 2);
        let mut machine = crate::Machine::new(cpu, bus);
        machine
            .apply_pending_flash_op()
            .expect("swapped: cross-physical-bank erase proceeds");
    }
}

#[test]
fn rww_gate_off_same_bank_erase_succeeds_silently() {
    // Default behaviour (gate off): a same-bank erase succeeds, byte-
    // identical to before this gate existed.
    use crate::peripherals::flash::h5;
    let mut cpu = PcCpu::default();
    cpu.set_pc(0x0801_6000);
    let mut bus = h5_rww_bus(false);
    let off = 11 * h5::SECTOR_SIZE;
    bus.flash.write_u8(h5::FLASH_BASE + off, 0x00);
    h5_record_erase(&mut bus, 0, 11);
    let mut machine = crate::Machine::new(cpu, bus);
    machine
        .apply_pending_flash_op()
        .expect("gate off: same-bank erase succeeds");
    assert_eq!(
        machine.bus.flash.read_u8(h5::FLASH_BASE + off),
        Some(0xFF),
        "gate off: sector erased to 0xFF as before"
    );
}

#[test]
fn test_peripheral_range_index_lookup() {
    let mut bus = SystemBus {
        flash: LinearMemory::new(256, 0x0800_0000),
        ram: LinearMemory::new(256, 0x2000_0000),
        extra_mem: Vec::new(),
        peripherals: vec![
            PeripheralEntry {
                name: "high".to_string(),
                base: 0x5000_0000,
                size: 0x1000,
                irq: None,
                dev: Box::new(crate::peripherals::uart::Uart::new()),
                ticks_remaining: 0,
                clock_gate: None,
            },
            PeripheralEntry {
                name: "low".to_string(),
                base: 0x4000_0000,
                size: 0x1000,
                irq: None,
                dev: Box::new(crate::peripherals::uart::Uart::new()),
                ticks_remaining: 0,
                clock_gate: None,
            },
        ],
        nvic: None,
        observers: Vec::new(),
        config: crate::SimulationConfig::default(),
        bit_band_enabled: true,
        pending_cpu_irqs: [0; 2],
        dport_idx: None,
        rcc_idx: None,
        clock_gating_bypass: false,
        fault_unclocked: std::collections::HashMap::new(),
        flash_thunks: std::collections::HashMap::new(),
        peripheral_ranges: Vec::new(),
        legacy_tick_indices: Vec::new(),
        bus_tick_indices: Vec::new(),
        scheduler_driver_indices: Vec::new(),
        matrix_source_scratch: Vec::new(),
        peripheral_hint: Cell::new(None),
        last_route: Cell::new(None),
        last_gap: Cell::new(None),
        last_gpio_in: [0; 2],
        current_cycle: 0,
        cycle_clock: crate::CycleClock::default(),
        pending_schedule: Vec::new(),
        freerunning_timer_poll_mmio: std::cell::Cell::new(0),
        side_effecting_mmio: std::cell::Cell::new(0),
        legacy_walk_disabled: false,
        reset_vector_offset: 0,
        atomic_register_aliases: false,
        hcsr04: Vec::new(),
        gpio_devices: Vec::new(),
        ws2812: Vec::new(),
        tm1637: Vec::new(),
        seven_segment: Vec::new(),
        analog_inputs: Vec::new(),
        can_diagnostic_testers: Vec::new(),
        can_uds_testers: Vec::new(),
        can_log_players: Vec::new(),
        esp32c3_irq_routing: false,
        riscv_irq_lines: 0,
        esp32c3_system_idx: None,
        esp32c3_interrupt_core0_idx: None,
        esp32c3_irq_cache: None,
        esp32c3_asserted_sources: [0; 2],
        esp32c3_sched_asserted_sources: [0; 2],
        esp32s3_irq_routing: false,
        esp32s3_intmatrix_idx: None,
        esp32s3_asserted_sources: [0; 2],
        esp32s3_sched_asserted_sources: [0; 2],
        flash_models_ops: false,
        iolink_master_attached: false,
        nordic_gpio_service: false,
        hcsr04_scheduling_disabled: false,
        flash_error_flags_idx: None,
        bus_trace: bus_trace::new_log(),
        logic_tap: crate::logic_capture::LogicTap::new(),
        pin_map: std::collections::HashMap::new(),
    };

    bus.rebuild_peripheral_ranges();
    let low_idx = bus.find_peripheral_index(0x4000_0004);
    let high_idx = bus.find_peripheral_index(0x5000_0004);

    assert_eq!(low_idx, Some(1));
    assert_eq!(high_idx, Some(0));
}

#[test]
fn test_execute_dma_copy_request() {
    let mut bus = SystemBus::new();
    bus.write_u8(0x2000_0010, 0xAB).unwrap();
    bus.write_u8(0x2000_0020, 0x00).unwrap();

    let req = crate::DmaRequest {
        src_addr: 0x2000_0010,
        addr: 0x2000_0020,
        val: 0,
        direction: crate::DmaDirection::Copy,
        transform: None,
    };
    bus.execute_dma(&[req]).unwrap();

    assert_eq!(bus.read_u8(0x2000_0020).unwrap(), 0xAB);
}

#[test]
fn test_dma_tick_executes_copy_and_raises_irq() {
    let mut bus = SystemBus {
        flash: LinearMemory::new(256, 0x0800_0000),
        ram: LinearMemory::new(256, 0x2000_0000),
        extra_mem: Vec::new(),
        peripherals: vec![PeripheralEntry {
            name: "dma1".to_string(),
            base: 0x4002_0000,
            size: 0x400,
            irq: Some(16),
            dev: Box::new(crate::peripherals::dma::Dma1::new()),
            ticks_remaining: 0,
            clock_gate: None,
        }],
        nvic: None,
        observers: Vec::new(),
        config: crate::SimulationConfig::default(),
        bit_band_enabled: true,
        pending_cpu_irqs: [0; 2],
        dport_idx: None,
        rcc_idx: None,
        clock_gating_bypass: false,
        fault_unclocked: std::collections::HashMap::new(),
        flash_thunks: std::collections::HashMap::new(),
        peripheral_ranges: Vec::new(),
        legacy_tick_indices: Vec::new(),
        bus_tick_indices: Vec::new(),
        scheduler_driver_indices: Vec::new(),
        matrix_source_scratch: Vec::new(),
        peripheral_hint: Cell::new(None),
        last_route: Cell::new(None),
        last_gap: Cell::new(None),
        last_gpio_in: [0; 2],
        current_cycle: 0,
        cycle_clock: crate::CycleClock::default(),
        pending_schedule: Vec::new(),
        freerunning_timer_poll_mmio: std::cell::Cell::new(0),
        side_effecting_mmio: std::cell::Cell::new(0),
        legacy_walk_disabled: false,
        reset_vector_offset: 0,
        atomic_register_aliases: false,
        hcsr04: Vec::new(),
        gpio_devices: Vec::new(),
        ws2812: Vec::new(),
        tm1637: Vec::new(),
        seven_segment: Vec::new(),
        analog_inputs: Vec::new(),
        can_diagnostic_testers: Vec::new(),
        can_uds_testers: Vec::new(),
        can_log_players: Vec::new(),
        esp32c3_irq_routing: false,
        riscv_irq_lines: 0,
        esp32c3_system_idx: None,
        esp32c3_interrupt_core0_idx: None,
        esp32c3_irq_cache: None,
        esp32c3_asserted_sources: [0; 2],
        esp32c3_sched_asserted_sources: [0; 2],
        esp32s3_irq_routing: false,
        esp32s3_intmatrix_idx: None,
        esp32s3_asserted_sources: [0; 2],
        esp32s3_sched_asserted_sources: [0; 2],
        flash_models_ops: false,
        iolink_master_attached: false,
        nordic_gpio_service: false,
        hcsr04_scheduling_disabled: false,
        flash_error_flags_idx: None,
        bus_trace: bus_trace::new_log(),
        logic_tap: crate::logic_capture::LogicTap::new(),
        pin_map: std::collections::HashMap::new(),
    };
    bus.rebuild_peripheral_ranges();

    // Per STM32 RM mem-to-mem semantics: data flows CMAR -> CPAR
    // (CMAR is the source, CPAR is the destination). Set up source
    // at SRC_ADDR via CMAR; expect destination at DST_ADDR (CPAR).
    const SRC_ADDR: u64 = 0x2000_0010;
    const DST_ADDR: u64 = 0x2000_0020;
    bus.write_u8(SRC_ADDR, 0x5A).unwrap();
    bus.write_u8(DST_ADDR, 0x00).unwrap();

    // Program DMA1 Channel1:
    //   CMAR (source) = SRC_ADDR
    //   CPAR (destination) = DST_ADDR
    //   CNDTR = 1, CCR = EN | TCIE | PINC | MINC | DIR | MEM2MEM
    bus.write_u32(0x4002_0014, SRC_ADDR as u32).unwrap(); // CMAR1
    bus.write_u32(0x4002_0010, DST_ADDR as u32).unwrap(); // CPAR1
    bus.write_u32(0x4002_000C, 1).unwrap(); // CNDTR1
    bus.write_u32(
        0x4002_0008,
        (1 << 0) | (1 << 1) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 14),
    )
    .unwrap(); // CCR1 (EN | TCIE | DIR | PINC | MINC | MEM2MEM)

    let (interrupts, _costs) = bus.tick_peripherals_fully();
    assert_eq!(
        bus.read_u8(DST_ADDR).unwrap(),
        0x5A,
        "DST should hold the SRC byte after mem-to-mem copy"
    );
    assert!(interrupts.contains(&16), "TCIE should pend NVIC IRQ 16");
}

/// RCC clock-gating (silicon fidelity): a peripheral with a declared
/// `clock:` gate is inert until its RCC enable bit is set — writes are
/// dropped and reads return 0 — and behaves normally once clocked. The
/// reg-name → offset mapping is family-aware (F1 apb2enr @ 0x18).
#[test]
fn gated_peripheral_is_inert_until_rcc_bit_set() {
    let chip: ChipDescriptor = serde_yaml::from_str(
        r#"
name: "f1-clockgate-test"
arch: "arm"
core: "cortex-m3"
flash:
  base: 0x08000000
  size: "64KB"
ram:
  base: 0x20000000
  size: "20KB"
peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x40021000
    size: "1KB"
  - id: "uart1"
    type: "uart"
    base_address: 0x40013800
    size: "1KB"
    clock: { reg: "apb2enr", bit: 14 }
  - id: "uart2"
    type: "uart"
    base_address: 0x40004400
    size: "1KB"
"#,
    )
    .unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: "clockgate"
chip: "unused"
external_devices: []
board_io: []
"#,
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();

    // USART1_CR1 @ 0x4001_380C. Clock is OFF out of reset → the write is
    // dropped and the register reads back 0 (an unclocked peripheral).
    const CR1: u64 = 0x4001_380C;
    const CR1_UE_TE: u32 = (1 << 13) | (1 << 3);
    bus.write_u32(CR1, CR1_UE_TE).unwrap();
    assert_eq!(
        bus.read_u32(CR1).unwrap(),
        0,
        "unclocked USART1 must drop writes and read 0"
    );

    // The ungated uart2 (no clock declared) is unaffected — accessible now.
    const UART2_CR1: u64 = 0x4000_440C;
    bus.write_u32(UART2_CR1, CR1_UE_TE).unwrap();
    assert_eq!(
        bus.read_u32(UART2_CR1).unwrap() & CR1_UE_TE,
        CR1_UE_TE,
        "ungated uart2 must work regardless of RCC"
    );

    // Enable RCC_APB2ENR.USART1EN (bit 14). RCC itself is never gated.
    const RCC_APB2ENR: u64 = 0x4002_1018;
    bus.write_u32(RCC_APB2ENR, 1 << 14).unwrap();
    assert_eq!(bus.read_u32(RCC_APB2ENR).unwrap() & (1 << 14), 1 << 14);

    // Now USART1 is clocked: the same write takes effect and reads back.
    bus.write_u32(CR1, CR1_UE_TE).unwrap();
    assert_eq!(
        bus.read_u32(CR1).unwrap() & CR1_UE_TE,
        CR1_UE_TE,
        "clocked USART1 must accept writes"
    );

    // Drop the clock again → the peripheral goes inert (reads 0).
    bus.write_u32(RCC_APB2ENR, 0).unwrap();
    assert_eq!(
        bus.read_u32(CR1).unwrap(),
        0,
        "USART1 must go inert again when its clock is removed"
    );
}

#[test]
fn gated_peripheral_resolves_l4_rcc_offsets() {
    // The SAME symbolic reg names that map to F1 offsets above must resolve
    // to the L4 family's offsets via Rcc::enable_reg_offset: apb1enr1 @ 0x58
    // (not F1's 0x1C) and ahb2enr @ 0x4C. Mirrors the iolink-dido (USART2 on
    // apb1enr1) and nokia5110 (GPIOA on ahb2enr) gates on the L476.
    let chip: ChipDescriptor = serde_yaml::from_str(
        r#"
name: "l4-clockgate-test"
arch: "arm"
core: "cortex-m4"
flash:
  base: 0x08000000
  size: "1MB"
ram:
  base: 0x20000000
  size: "96KB"
peripherals:
  - id: "rcc"
    type: "rcc"
    base_address: 0x40021000
    size: "1KB"
    config:
      profile: "stm32l4"
  - id: "gpioa"
    type: "gpio"
    base_address: 0x48000000
    size: "1KB"
    config:
      profile: "stm32v2"
    clock: { reg: "ahb2enr", bit: 0 }
  - id: "uart2"
    type: "uart"
    base_address: 0x40004400
    size: "1KB"
    config:
      profile: "stm32v2"
    clock: { reg: "apb1enr1", bit: 17 }
"#,
    )
    .unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(
        r#"
name: "clockgate-l4"
chip: "unused"
external_devices: []
board_io: []
"#,
    )
    .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();

    // USART2_CR1 @ 0x4000_4400 (stm32v2 layout: CR1 at offset 0x00).
    // Clock OFF out of reset.
    const U2_CR1: u64 = 0x4000_4400;
    const CR1_UE_TE: u32 = (1 << 0) | (1 << 3);
    bus.write_u32(U2_CR1, CR1_UE_TE).unwrap();
    assert_eq!(
        bus.read_u32(U2_CR1).unwrap(),
        0,
        "unclocked USART2 must drop writes and read 0"
    );

    // RCC_APB1ENR1 @ 0x58 (L4 offset, NOT the F1 0x1C). USART2EN = bit 17.
    const RCC_APB1ENR1: u64 = 0x4002_1058;
    bus.write_u32(RCC_APB1ENR1, 1 << 17).unwrap();
    bus.write_u32(U2_CR1, CR1_UE_TE).unwrap();
    assert_eq!(
        bus.read_u32(U2_CR1).unwrap() & CR1_UE_TE,
        CR1_UE_TE,
        "clocked USART2 must accept writes once apb1enr1.17 is set"
    );

    // GPIOA_MODER @ 0x4800_0000, gated on RCC_AHB2ENR @ 0x4C bit 0.
    const GPIOA_MODER: u64 = 0x4800_0000;
    bus.write_u32(GPIOA_MODER, 0x55).unwrap();
    assert_eq!(
        bus.read_u32(GPIOA_MODER).unwrap(),
        0,
        "unclocked GPIOA must drop writes and read 0"
    );
    const RCC_AHB2ENR: u64 = 0x4002_104C;
    bus.write_u32(RCC_AHB2ENR, 1 << 0).unwrap();
    bus.write_u32(GPIOA_MODER, 0x55).unwrap();
    assert_eq!(
        bus.read_u32(GPIOA_MODER).unwrap() & 0x55,
        0x55,
        "clocked GPIOA must accept writes once ahb2enr.0 is set"
    );
}

// -----------------------------------------------------------------------
// Script-driven FSM tests
// -----------------------------------------------------------------------

/// Core helper: build a bus with a bxCAN in normal mode (filter accepts
/// 0x111) and attach a UDS tester loaded with the given steps. Returns
/// the bus after the first service tick so the tester has already sent its
/// initial SF/FF and is in `AwaitResp` (or `AwaitFc` for a multi-frame
/// request).
fn bus_with_steps(script: Vec<UdsStep>) -> SystemBus {
    use crate::peripherals::bxcan::BxCan;
    const MCR: u64 = 0x000;
    const BTR: u64 = 0x01C;
    const FMR: u64 = 0x200;
    const FM1R: u64 = 0x204;
    const FS1R: u64 = 0x20C;
    const FFA1R: u64 = 0x214;
    const FA1R: u64 = 0x21C;
    const FBANK: u64 = 0x240;
    const VALID_BTR: u32 = 0x00DC_0009;
    const BASE: u64 = 0x4000_6400;

    let mut bus = SystemBus::empty();
    bus.add_peripheral("bxcan1", BASE, 0x400, None, Box::new(BxCan::new()));

    bus.write_u32(BASE + MCR, 1).unwrap();
    bus.write_u32(BASE + BTR, VALID_BTR).unwrap();
    bus.write_u32(BASE + FMR, 1).unwrap();
    bus.write_u32(BASE + FS1R, 0x1).unwrap();
    bus.write_u32(BASE + FM1R, 0x0).unwrap();
    bus.write_u32(BASE + FFA1R, 0x0).unwrap();
    bus.write_u32(BASE + FBANK, (0x111u32) << 21).unwrap();
    bus.write_u32(BASE + FBANK + 4, (0x111u32) << 21).unwrap();
    bus.write_u32(BASE + FA1R, 0x1).unwrap();
    bus.write_u32(BASE + FMR, 0x0).unwrap();
    bus.write_u32(BASE + MCR, 0).unwrap();

    let mut tester = CanUdsTester::new("uds".into(), "bxcan1".into());
    tester.script = script;
    bus.can_uds_testers.push(tester);
    bus.service_can_uds_testers();
    bus
}

/// Convenience wrapper: build a bus from `(send_hex, expect_hex)` tuples.
/// Each step is parsed the same way as the YAML config.
fn bus_with_script(steps: &[(&str, &str)]) -> SystemBus {
    let script: Vec<UdsStep> = steps
        .iter()
        .map(|(send_str, expect_str)| UdsStep {
            send: SystemBus::yaml_bytes(
                Some(&serde_yaml::Value::String(send_str.to_string())),
                &[],
            ),
            expect: SystemBus::parse_expect(expect_str),
            expect_nrc: None,
        })
        .collect();
    bus_with_steps(script)
}

/// Push a simulated ECU frame into the connected bxCAN's `tx_frames` so
/// the next `service_can_uds_testers` call drains and processes it.
fn inject_ecu_reply(bus: &mut SystemBus, id: u32, data: &[u8]) {
    use crate::peripherals::bxcan::BxCan;
    let idx = bus
        .find_peripheral_index_by_name("bxcan1")
        .expect("bxcan1 must be registered");
    let bx = bus.peripherals[idx]
        .dev
        .as_any_mut()
        .unwrap()
        .downcast_mut::<BxCan>()
        .expect("bxcan1 must be BxCan");
    bx.tx_frames
        .push_back(crate::network::CanFrame::classic(id, data.to_vec()));
}

/// Same construction idiom as `bus_with_steps`: a bare `SystemBus` with a
/// single `bxcan1` `BxCan`, taken out of INIT with a filter configured —
/// here a wide (32-bit) mask filter with id=0/mask=0, which accepts every
/// frame (standard or extended), so `CanLogPlayer` replay isn't gated by
/// filter setup unrelated to this test.
fn bus_with_open_bxcan() -> SystemBus {
    use crate::peripherals::bxcan::BxCan;
    const MCR: u64 = 0x000;
    const BTR: u64 = 0x01C;
    const FMR: u64 = 0x200;
    const FM1R: u64 = 0x204;
    const FS1R: u64 = 0x20C;
    const FFA1R: u64 = 0x214;
    const FA1R: u64 = 0x21C;
    const FBANK: u64 = 0x240;
    const VALID_BTR: u32 = 0x00DC_0009;
    const BASE: u64 = 0x4000_6400;

    let mut bus = SystemBus::empty();
    bus.add_peripheral("bxcan1", BASE, 0x400, None, Box::new(BxCan::new()));

    bus.write_u32(BASE + MCR, 1).unwrap();
    bus.write_u32(BASE + BTR, VALID_BTR).unwrap();
    bus.write_u32(BASE + FMR, 1).unwrap();
    bus.write_u32(BASE + FS1R, 0x1).unwrap(); // wide (32-bit) filter
    bus.write_u32(BASE + FM1R, 0x0).unwrap(); // mask mode (not list)
    bus.write_u32(BASE + FFA1R, 0x0).unwrap();
    bus.write_u32(BASE + FBANK, 0).unwrap(); // id = 0
    bus.write_u32(BASE + FBANK + 4, 0).unwrap(); // mask = 0 -> accept all
    bus.write_u32(BASE + FA1R, 0x1).unwrap();
    bus.write_u32(BASE + FMR, 0x0).unwrap();
    bus.write_u32(BASE + MCR, 0).unwrap();
    bus
}

#[test]
fn can_log_player_delivers_frames_at_scheduled_ticks() {
    // Two frames 100µs apart at 1M ticks/sec => ticks 0 and 100.
    let log = "(10.000000) can0 0CF00300#DD0000FFFFFF5CFF\n\
               (10.000100) can0 18FEF100#0102030405060708\n";
    let mut bus = bus_with_open_bxcan();
    let player = CanLogPlayer::from_candump("p".into(), "bxcan1".into(), log, 1_000_000).unwrap();
    bus.can_log_players.push(player);

    // Tick 1: first frame (tick 0 is due immediately).
    bus.service_can_log_players();
    assert_eq!(bus.can_log_players[0].delivered, 1);
    // Ticks 2..=99: nothing new.
    for _ in 0..98 {
        bus.service_can_log_players();
    }
    assert_eq!(bus.can_log_players[0].delivered, 1);
    assert!(!bus.can_log_players[0].is_done());
    // Tick 100+: second frame.
    for _ in 0..3 {
        bus.service_can_log_players();
    }
    assert_eq!(bus.can_log_players[0].delivered, 2);
    assert!(bus.can_log_players[0].is_done());

    // And the bxCAN RX FIFO actually holds data: read via the same register
    // asserts the uds-tester tests use (RF0R pending count != 0).
    const RF0R: u64 = 0x00C;
    const BASE: u64 = 0x4000_6400;
    let rf0r = bus.read_u32(BASE + RF0R).unwrap();
    assert_ne!(rf0r & 0x3, 0, "RF0R FMP0 must show pending frames");
}

#[test]
fn can_log_player_counts_dropped_when_filters_never_opened() {
    // Same construction idiom as `bus_with_open_bxcan`, minus the filter
    // banks — the bxCAN is taken out of INIT (so it's "running") but no
    // filter bank is ever activated (FA1R stays 0), so every delivered
    // frame is refused by acceptance filtering and must count as
    // `dropped`, never `delivered`.
    use crate::peripherals::bxcan::BxCan;
    const MCR: u64 = 0x000;
    const BTR: u64 = 0x01C;
    const VALID_BTR: u32 = 0x00DC_0009;
    const BASE: u64 = 0x4000_6400;

    let mut bus = SystemBus::empty();
    bus.add_peripheral("bxcan1", BASE, 0x400, None, Box::new(BxCan::new()));
    bus.write_u32(BASE + MCR, 1).unwrap();
    bus.write_u32(BASE + BTR, VALID_BTR).unwrap();
    bus.write_u32(BASE + MCR, 0).unwrap(); // leave INIT; filters left unconfigured

    let log = "(10.000000) can0 0CF00300#DD0000FFFFFF5CFF\n\
               (10.000100) can0 18FEF100#0102030405060708\n";
    let player = CanLogPlayer::from_candump("p".into(), "bxcan1".into(), log, 1_000_000).unwrap();
    bus.can_log_players.push(player);

    for _ in 0..101 {
        bus.service_can_log_players();
    }
    assert!(bus.can_log_players[0].is_done());
    assert_eq!(bus.can_log_players[0].delivered, 0);
    assert!(bus.can_log_players[0].dropped > 0);
}

#[test]
fn can_log_player_rebases_first_frame_to_tick_zero() {
    let log = "(1578925462.000450) can0 123#11\n";
    let p = CanLogPlayer::from_candump("p".into(), "bxcan1".into(), log, 1_000_000).unwrap();
    assert_eq!(p.frames[0].0, 0);
}

#[test]
fn uds_tester_single_step_sf_request_matches_reply() {
    let mut bus = bus_with_script(&[("11 01", "51 01")]);
    inject_ecu_reply(&mut bus, 0x222, &[0x02, 0x51, 0x01]);
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
}

#[test]
fn uds_tester_wildcard_and_multistep() {
    let mut bus = bus_with_script(&[("10 03", "50 03"), ("27 01", "67 01 ..")]);
    // bus_with_script already sent step 0 request; inject step 0 reply.
    inject_ecu_reply(&mut bus, 0x222, &[0x02, 0x50, 0x03]);
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].step_idx, 1);
    // After step 0 completes, state returns to Start. The next service call
    // sends step 1 request.
    bus.service_can_uds_testers();
    inject_ecu_reply(&mut bus, 0x222, &[0x03, 0x67, 0x01, 0xAB]);
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
}

#[test]
fn uds_tester_nrc_mismatch_fails_with_reason() {
    let mut bus = bus_with_script(&[("11 01", "51 01")]);
    // NRC response (0x7F 0x11 0x22) — does not match expected "51 01".
    inject_ecu_reply(&mut bus, 0x222, &[0x03, 0x7F, 0x11, 0x22]);
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Failed);
    assert!(bus.can_uds_testers[0]
        .failure
        .as_ref()
        .unwrap()
        .contains("step 0"));
}

/// Script-path FF+1CF request: send.len() == 8 (one CF required).
/// Verifies that the ConsecutiveFrame is injected onto the bus after the
/// ECU's FlowControl arrives, and that the step reaches Done.
///
/// This test exercises the bug fixed in this commit: before the fix,
/// observe_ecu_frame_script set state=AwaitResp before service_can_uds_testers
/// evaluated to_send, so the CF payload was silently discarded.
#[test]
fn uds_tester_script_ff_plus_one_cf_request_completes() {
    use crate::peripherals::bxcan::BxCan;

    // 8-byte payload: FF carries bytes 0..5, CF carries bytes 6..7.
    // Expected response for 0x27 service: 0x67 0x02 (single-frame).
    let mut bus = bus_with_script(&[("27 01 02 03 04 05 06 07", "67 02")]);

    // bus_with_script already ran tick 1: FF sent, state=AwaitFc.
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::AwaitFc);
    assert_eq!(
        bus.can_uds_testers[0].pending_cfs.len(),
        1,
        "one CF must be queued after FF"
    );

    // ECU responds with FlowControl (ContinueToSend).
    inject_ecu_reply(
        &mut bus,
        0x222,
        &[0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    );

    // Tick 2: tester drains the FC and injects the CF.
    bus.service_can_uds_testers();
    assert_eq!(
        bus.can_uds_testers[0].state,
        CanUdsTesterState::AwaitResp,
        "CF must be injected and state must advance to AwaitResp"
    );
    assert!(
        bus.can_uds_testers[0].pending_cfs.is_empty(),
        "pending_cfs must be drained after the only CF is sent"
    );

    // Confirm the CF actually landed in the bxCAN RX buffer (direction=rx
    // means the tester delivered it into the ECU-side FIFO).
    {
        let idx = bus.find_peripheral_index_by_name("bxcan1").unwrap();
        let bx = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<BxCan>()
            .unwrap();
        let trace = bx.trace_snapshot("bxcan1");
        // trace contains all rx frames: FF (tick 1) + CF (tick 2).
        assert!(
            trace
                .iter()
                .any(|f| f.direction == "rx" && f.id == 0x111 && f.data.first() == Some(&0x21)),
            "CF (SN=0x21) must appear as an rx frame in the bxCAN trace"
        );
    }

    // ECU sends the positive response (single-frame: len=3, 0x67 0x02 0xAB).
    inject_ecu_reply(&mut bus, 0x222, &[0x03, 0x67, 0x02, 0xAB]);

    // Tick 3: tester matches the response → Done.
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
}

/// Script-path FF+2CF request: send.len() == 14 (two CFs required).
/// Verifies that both ConsecutiveFrames are injected on successive ticks
/// and the step reaches Done.
#[test]
fn uds_tester_script_ff_plus_two_cf_request_completes() {
    use crate::peripherals::bxcan::BxCan;

    // 14-byte payload: FF carries bytes 0..5, CF1 carries 6..12, CF2 carries 13.
    // Expected response: 0x76 0x01.
    let mut bus = bus_with_script(&[("36 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D", "76 01")]);

    // Tick 1 already ran: FF sent, two CFs queued.
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::AwaitFc);
    assert_eq!(
        bus.can_uds_testers[0].pending_cfs.len(),
        2,
        "two CFs must be queued after FF"
    );

    // ECU replies with FlowControl.
    inject_ecu_reply(
        &mut bus,
        0x222,
        &[0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    );

    // Tick 2: CF1 injected; one CF still pending, state stays AwaitFc.
    bus.service_can_uds_testers();
    assert_eq!(
        bus.can_uds_testers[0].state,
        CanUdsTesterState::AwaitFc,
        "state must stay AwaitFc while CFs remain"
    );
    assert_eq!(
        bus.can_uds_testers[0].pending_cfs.len(),
        1,
        "one CF must remain after CF1 is sent"
    );

    // Tick 3: no new ECU frame; CF2 taken from pending_cfs → AwaitResp.
    bus.service_can_uds_testers();
    assert_eq!(
        bus.can_uds_testers[0].state,
        CanUdsTesterState::AwaitResp,
        "state must advance to AwaitResp after last CF is sent"
    );
    assert!(bus.can_uds_testers[0].pending_cfs.is_empty());

    // Verify both CFs appear in the trace (SN 0x21 and 0x22).
    {
        let idx = bus.find_peripheral_index_by_name("bxcan1").unwrap();
        let bx = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<BxCan>()
            .unwrap();
        let trace = bx.trace_snapshot("bxcan1");
        assert!(
            trace
                .iter()
                .any(|f| f.direction == "rx" && f.id == 0x111 && f.data.first() == Some(&0x21)),
            "CF1 (SN=0x21) must appear as an rx frame"
        );
        assert!(
            trace
                .iter()
                .any(|f| f.direction == "rx" && f.id == 0x111 && f.data.first() == Some(&0x22)),
            "CF2 (SN=0x22) must appear as an rx frame"
        );
    }

    // ECU single-frame positive response.
    inject_ecu_reply(&mut bus, 0x222, &[0x02, 0x76, 0x01]);

    // Tick 4: match → Done.
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
}

/// 0x2E WriteDataByIdentifier: single-frame multi-byte request (7 bytes) →
/// positive 6E echo. Covers DID-write framing the existing tests lack.
#[test]
fn uds_tester_did_write_sf_completes() {
    let mut bus = bus_with_script(&[("2E 01 23 DE AD BE EF", "6E 01 23")]);
    // SF header 0x03 = three payload bytes (6E 01 23); the prior 0x04 was a
    // malformed fixture (declared 4, carried 3) the lenient decoder masked.
    inject_ecu_reply(&mut bus, 0x222, &[0x03, 0x6E, 0x01, 0x23]);
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
}

/// 0x31 RoutineControl: reply carries an output byte after the echo; the
/// prefix match must accept the longer response.
#[test]
fn uds_tester_routine_reply_with_output_byte() {
    let mut bus = bus_with_script(&[("31 01 02 03", "71 01 02 03")]);
    inject_ecu_reply(&mut bus, 0x222, &[0x05, 0x71, 0x01, 0x02, 0x03, 0x00]);
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
}

/// 0x2F IOControl: shortTermAdjustment request, reply echoes DID + state.
#[test]
fn uds_tester_io_control_reply_completes() {
    let mut bus = bus_with_script(&[("2F A0 01 03 01", "6F A0 01")]);
    inject_ecu_reply(&mut bus, 0x222, &[0x05, 0x6F, 0xA0, 0x01, 0x03, 0x01]);
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
}

/// 0x19 ReadDTCInformation: a multi-frame ECU reply (FF + 1 CF) must be
/// reassembled (AwaitResp → AwaitMultiResp → Done) and prefix-matched.
#[test]
fn uds_tester_dtc_read_multiframe_reply_completes() {
    let mut bus = bus_with_script(&[("19 02 09", "59 02")]);
    // FF declares 10-byte response, carries first 6 bytes (59 02 09 01 23 45).
    inject_ecu_reply(
        &mut bus,
        0x222,
        &[0x10, 0x0A, 0x59, 0x02, 0x09, 0x01, 0x23, 0x45],
    );
    bus.service_can_uds_testers(); // tester replies FlowControl, enters AwaitMultiResp
    assert_eq!(
        bus.can_uds_testers[0].state,
        CanUdsTesterState::AwaitMultiResp
    );
    // CF carries the remaining bytes; total >= 10 → complete.
    inject_ecu_reply(&mut bus, 0x222, &[0x21, 0x67, 0xAA, 0xBB, 0xCC, 0xDD]);
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
}

/// Multi-frame ECU response: the tester must inject a FlowControl frame onto
/// the bxCAN bus so the ECU can send its ConsecutiveFrames.
///
/// Guards the bug where the `AwaitMultiResp` arm was missing from the
/// `to_send` match in `service_can_uds_testers`, causing the FlowControl
/// returned by `observe_ecu_frame_script` to be silently dropped (the
/// `_ => None` arm swallowed it).  Without the fix the ECU never receives
/// CTS and the exchange deadlocks.
///
/// The discriminating assertion is NOT the final `Done` state (the
/// `inject_ecu_reply` shortcut bypasses that gate) but the presence of a
/// FlowControl frame (`first_byte & 0xF0 == 0x30`) in the bxCAN RX trace
/// after the tick that processes the ECU FirstFrame.
#[test]
fn uds_tester_multiframe_ecu_response_injects_flowcontrol() {
    use crate::peripherals::bxcan::BxCan;

    // Step 0: ReadDataByIdentifier 0xF190 (VIN), expect prefix 62 F1 90.
    let mut bus = bus_with_script(&[("22 F1 90", "62 F1 90")]);

    // Tick 1 already ran: the SF request (first byte 0x03) was delivered to
    // the bxCAN via deliver_rx.  Record the trace length now so we can
    // distinguish that pre-existing frame from the FlowControl we expect next.
    let trace_len_before = {
        let idx = bus.find_peripheral_index_by_name("bxcan1").unwrap();
        let bx = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<BxCan>()
            .unwrap();
        bx.trace_snapshot("bxcan1").len()
    };

    // ECU replies with a FirstFrame declaring a 13-byte (0x0D) response
    // and carrying the first 6 payload bytes (62 F1 90 + 3 VIN chars).
    // 13 bytes = 6 in FF + 7 in one CF.
    inject_ecu_reply(
        &mut bus,
        0x222,
        &[0x10, 0x0D, 0x62, 0xF1, 0x90, 0x31, 0x32, 0x33],
    );

    // Tick 2: tester sees the FF, sets state=AwaitMultiResp, and MUST
    // inject a FlowControl ([0x30, 0x00, 0x00]) onto the bxCAN bus.
    bus.service_can_uds_testers();

    assert_eq!(
        bus.can_uds_testers[0].state,
        CanUdsTesterState::AwaitMultiResp,
        "state must be AwaitMultiResp after receiving ECU FirstFrame"
    );

    // Verify the FlowControl was actually delivered to the bus.
    // Only frames appended AFTER tick 1 (index >= trace_len_before) are
    // candidates; the earlier SF request frame starts with 0x03, not 0x3x.
    {
        let idx = bus.find_peripheral_index_by_name("bxcan1").unwrap();
        let bx = bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<BxCan>()
            .unwrap();
        let trace = bx.trace_snapshot("bxcan1");
        let new_frames = &trace[trace_len_before..];
        assert!(
            new_frames.iter().any(|f| {
                f.direction == "rx"
                    && f.id == 0x111
                    && f.data.first().map(|b| b & 0xF0 == 0x30).unwrap_or(false)
            }),
            "FlowControl (0x30 nibble) must appear in bxCAN rx trace after ECU FirstFrame; \
             new frames after tick 1: {:?}",
            new_frames
                .iter()
                .map(|f| (f.direction.as_str(), f.id, f.data.clone()))
                .collect::<Vec<_>>()
        );
    }

    // Complete the exchange: one CF carries the remaining 7 bytes to reach
    // the declared 13.  After this the tester must reach Done.
    inject_ecu_reply(
        &mut bus,
        0x222,
        &[0x21, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x30],
    );
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
}

/// Session-gated write rejected in the default session: the tester must
/// accept a negative response when the step declares `expect_nrc`.
#[test]
fn uds_tester_expect_nrc_negative_response_completes() {
    let steps = vec![UdsStep {
        send: SystemBus::yaml_bytes(
            Some(&serde_yaml::Value::String(
                "2E 01 23 DE AD BE EF".to_string(),
            )),
            &[],
        ),
        expect: Vec::new(),
        expect_nrc: Some(0x31),
    }];
    let mut bus = bus_with_steps(steps);
    inject_ecu_reply(&mut bus, 0x222, &[0x03, 0x7F, 0x2E, 0x31]);
    bus.service_can_uds_testers();
    assert_eq!(bus.can_uds_testers[0].state, CanUdsTesterState::Done);
}

/// The `iolink_master_attached` cache backing `has_iolink_master` (and thus
/// `requires_cycle_accurate`, which the run loop consults per batch plan / per
/// step / per idle-FF check) must never disagree with the authoritative
/// `scan_iolink_master` nested scan. A stale `false` would let a bus that must
/// run cycle-accurate be batched, silently changing IO-Link timing — a fidelity
/// break, not a perf regression. Pin every mutation path that can flip it.
#[test]
fn iolink_master_cache_tracks_every_mutation_path() {
    use crate::peripherals::components::{IolinkComSpeed, IolinkMaster};

    // Empty bus: nothing attached, nothing to find.
    let mut bus = SystemBus::empty();
    assert_eq!(bus.has_iolink_master(), bus.scan_iolink_master());
    assert!(!bus.has_iolink_master());

    // `add_peripheral` funnels through `rebuild_peripheral_ranges`. A plain
    // UART carries no master, so the answer stays false.
    bus.add_peripheral(
        "uart0",
        0x4000_0000,
        0x1000,
        None,
        Box::new(crate::peripherals::uart::Uart::new()),
    );
    assert_eq!(bus.has_iolink_master(), bus.scan_iolink_master());
    assert!(!bus.has_iolink_master());
    assert!(!bus.requires_cycle_accurate());

    // THE STALENESS SEAM: the post-build stream attach. This mutates a UART's
    // `attached_streams` without touching the peripheral SET, so nothing else
    // would rebuild the cache.
    bus.attach_uart_stream_by_id(
        "uart0",
        Box::new(IolinkMaster::new(1, 1, IolinkComSpeed::Com2)),
    )
    .expect("attach IO-Link master to uart0");
    assert_eq!(
        bus.has_iolink_master(),
        bus.scan_iolink_master(),
        "attach_uart_stream_by_id must refresh the iolink cache"
    );
    assert!(bus.has_iolink_master());
    assert!(
        bus.requires_cycle_accurate(),
        "an attached IO-Link master must force cycle-accurate execution"
    );

    // A later peripheral-set mutation re-derives the cache; it must not clobber
    // the master discovered through the stream seam back to false.
    bus.add_peripheral(
        "uart1",
        0x4000_1000,
        0x1000,
        None,
        Box::new(crate::peripherals::uart::Uart::new()),
    );
    assert_eq!(bus.has_iolink_master(), bus.scan_iolink_master());
    assert!(
        bus.has_iolink_master(),
        "rebuild_peripheral_ranges must not lose the attached master"
    );

    // `refresh_peripheral_index` is the documented escape hatch for code that
    // mutates the `pub peripherals` vector by hand.
    bus.refresh_peripheral_index();
    assert_eq!(bus.has_iolink_master(), bus.scan_iolink_master());
    assert!(bus.has_iolink_master());
}
