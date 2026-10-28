//! AVR SPDR must latch D/C per byte for parking SPI panels, and kits left on
//! the parking controller must stay visible to inspect / display_artifact.
//!
//! Regression for the Uno ST7789 claw: without a D/C sample every SPDR write
//! framed every byte as a command (blank panel), and `take_spi_devices` moved
//! the kit onto the CPU so inspect could not see it.

use labwired_config::{ChipDescriptor, ExternalDevice, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::Avr;
use labwired_core::inspect::InspectOpts;
use labwired_core::Bus;
use labwired_core::Machine;
use std::collections::HashMap;
use std::path::PathBuf;

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn uno_st7789_bus() -> SystemBus {
    let chip_path = repo("configs/chips/atmega328p.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("atmega328p");
    let mut cfg = HashMap::new();
    cfg.insert("cs_pin".into(), serde_yaml::Value::String("PB2".into()));
    cfg.insert("dc_pin".into(), serde_yaml::Value::String("PB1".into()));
    let manifest = SystemManifest {
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "avr-spi-dc-inspect".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        cpu_hz: Some(16_000_000),
        external_devices: vec![ExternalDevice {
            id: "DS1".to_string(),
            r#type: "st7789-170x320".to_string(),
            connection: "spi".to_string(),
            channel: None,
            route: Default::default(),
            config: cfg,
        }],
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        parts: Default::default(),
        memory_overrides: Default::default(),
    };
    SystemBus::from_config(&chip, &manifest).expect("build bus")
}

/// Drive PORTB so PB1 (D/C) is an output at `dc_high`, then clock one MOSI
/// byte through parking SPI (same latch + transfer path SPDR uses).
fn parking_byte(bus: &mut SystemBus, dc_high: bool, mosi: u8) {
    // portb base 0x0001_0023: PIN/DDR/PORT → set DDRB.1 and PORTB.1
    let portb = 0x0001_0023u64;
    let ddr = bus.read_u8(portb + 1).unwrap_or(0);
    let port = bus.read_u8(portb + 2).unwrap_or(0);
    let bit = 1u8 << 1; // PB1
    bus.write_u8(portb + 1, ddr | bit).expect("DDRB");
    let port = if dc_high { port | bit } else { port & !bit };
    bus.write_u8(portb + 2, port).expect("PORTB");
    bus.clock_parking_spi(mosi);
}

fn paint_white_block(bus: &mut SystemBus) {
    // Select the kit (real firmware drives CS low before the burst).
    if let Some(idx) = bus.find_peripheral_index_by_name("spi") {
        if let Some(devs) = bus.peripherals[idx].dev.spi_attached_devices_mut() {
            for d in devs.iter_mut() {
                d.cs_select();
            }
        }
    }
    // Minimal ST7789 bring-up + 2×2 white window (same command set as the
    // display migration parity suite; D/C false = command, true = data).
    let frames: &[(bool, u8)] = &[
        (false, 0x01), // SWRESET
        (false, 0x11), // SLPOUT
        (false, 0x3A),
        (true, 0x55),  // COLMOD 16bpp
        (false, 0x29), // DISPON
        (false, 0x2A),
        (true, 0x00),
        (true, 0x00),
        (true, 0x00),
        (true, 0x01), // CASET x0..x1
        (false, 0x2B),
        (true, 0x00),
        (true, 0x00),
        (true, 0x00),
        (true, 0x01),  // RASET y0..y1
        (false, 0x2C), // RAMWR
        (true, 0xFF),
        (true, 0xFF),
        (true, 0xFF),
        (true, 0xFF),
        (true, 0xFF),
        (true, 0xFF),
        (true, 0xFF),
        (true, 0xFF), // 4 pixels RGB565
    ];
    for &(dc, b) in frames {
        parking_byte(bus, dc, b);
    }
}

#[test]
fn parking_spi_panel_is_visible_to_inspect_without_take() {
    let bus = uno_st7789_bus();
    let ids: Vec<_> = bus
        .inspect_devices(None, &InspectOpts::default())
        .into_iter()
        .map(|d| d.id)
        .collect();
    assert!(
        ids.iter().any(|id| id.contains("DS1")),
        "parking ST7789 must appear in inspect without take_spi; got {ids:?}"
    );
    let art = bus
        .display_artifact("DS1", &InspectOpts::default())
        .expect("display_artifact for parking DS1");
    assert!(
        art.meta.get("painted_bytes").is_some(),
        "artifact must report painted_bytes, got {:?}",
        art.meta
    );
}

#[test]
fn clock_parking_spi_latches_dc_and_paints() {
    let mut bus = uno_st7789_bus();
    paint_white_block(&mut bus);
    let art = bus
        .display_artifact("DS1", &InspectOpts::default())
        .expect("DS1 artifact after paint");
    let painted = art.meta["painted_bytes"].as_u64().unwrap_or(0);
    assert!(
        painted > 0,
        "D/C-latched parking SPI must paint; painted_bytes={painted}, meta={:?}",
        art.meta
    );
}

#[test]
fn cpu_held_spi_after_take_is_visible_via_inspect_extra() {
    // Compatibility path: if a caller still take_spi's onto the CPU, the
    // Machine::inspect extra walk must still surface the panel.
    let mut bus = uno_st7789_bus();
    let mut cpu = Avr::for_chip_name("atmega328p");
    for d in bus.take_spi_devices("spi") {
        cpu.push_spi_device(d);
    }
    assert!(
        bus.inspect_devices(None, &InspectOpts::default())
            .is_empty(),
        "after take, bus-only inspect must not see the panel"
    );
    let machine = Machine::new(cpu, bus);
    let ids: Vec<_> = machine
        .inspect(None, &InspectOpts::default())
        .devices
        .into_iter()
        .map(|d| d.id)
        .collect();
    assert!(
        ids.iter().any(|id| id.contains("DS1")),
        "Machine::inspect extra walk must see CPU-held SPI; got {ids:?}"
    );
}
