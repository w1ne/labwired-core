// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! SAMD51 SERCOM timing from the live clock tree: the Wire I2C masters the
//! board descriptors declare, SPI byte duration scaled by f_CPU / f_GCLK and
//! paced USART frames on an opt-in `samd51_sercom_usart`.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::SystemBus, cpu::cortex_m::CortexM, memory::ProgramImage, peripherals::spi::SpiDevice,
    Arch, Bus, Machine,
};
use std::sync::{Arc, Mutex};

const GCLK: u64 = 0x40001c00;
const PORTA: u64 = 0x41008000;
const PORTB: u64 = 0x41008080;
const SPI: u64 = 0x43000000;

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(chip: &str, system: &str) -> (ChipDescriptor, SystemManifest) {
    let chip = ChipDescriptor::from_file(root().join("configs/chips").join(chip)).unwrap();
    let manifest = SystemManifest::from_file(root().join("configs/systems").join(system)).unwrap();
    (chip, manifest)
}

/// The descriptor declares the Wire SERCOM as the I2C master model, and the
/// bus builds it at that base (the factory dispatches on the declared type).
fn is_i2cm(chip: &ChipDescriptor, bus: &SystemBus, name: &str, base: u64) -> bool {
    chip.peripherals
        .iter()
        .any(|p| p.id == name && p.base_address == base && p.r#type == "samd51_sercom_i2cm")
        && bus
            .peripherals
            .iter()
            .any(|p| p.name == name && p.base == base)
}

#[test]
fn board_descriptors_declare_their_wire_i2c_master() {
    let (chip, manifest) = load("atsamd51-pybadge.yaml", "pybadge.yaml");
    let bus = SystemBus::from_config(&chip, &manifest).unwrap();
    assert!(
        is_i2cm(&chip, &bus, "sercom2", 0x41012000),
        "PyBadge Wire is SERCOM2"
    );
    let (chip, manifest) = load("atsamd51.yaml", "metro-m4.yaml");
    let bus = SystemBus::from_config(&chip, &manifest).unwrap();
    assert!(
        is_i2cm(&chip, &bus, "sercom5", 0x43000400),
        "Metro M4 Wire is SERCOM5"
    );
}

struct Probe(Arc<Mutex<Vec<u8>>>);
impl SpiDevice for Probe {
    fn cs_pin(&self) -> &str {
        "PB7"
    }
    fn transfer(&mut self, byte: u8) -> u8 {
        self.0.lock().unwrap().push(byte);
        byte
    }
}

/// A machine parked on an idle `b .` loop, so the host drives MMIO and the
/// engine's own clock (walk or event scheduler) moves the peripherals.
fn idle_machine(bus: SystemBus) -> Machine<CortexM> {
    let mut machine = Machine::new(CortexM::new(), bus);
    let mut bytes = vec![0u8; 0x100];
    bytes[0..4].copy_from_slice(&0x20004000u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&0x4101u32.to_le_bytes());
    bytes.extend(0xe7feu16.to_le_bytes());
    let mut image = ProgramImage::new(0x4101, Arch::Arm);
    image.add_segment(0x4000, bytes);
    machine.load_firmware(&image).unwrap();
    machine
}

/// Core cycles from a DATA write until the slave sees the byte, with SERCOM4's
/// core clock on `generator` (generator 0 is the 48 MHz CPU clock).
fn spi_byte_ticks(generator: u32) -> u64 {
    let (chip, manifest) = load("atsamd51-pybadge.yaml", "pybadge.yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    bus.attach_spi_device("sercom4", Box::new(Probe(seen.clone())))
        .unwrap();
    // DFLL48M on generators 0 (CPU) and 2 (divided by 2).
    bus.write_u32(GCLK + 0x20, 6 | 1 << 8).unwrap();
    bus.write_u32(GCLK + 0x20 + 8, 6 | 1 << 8 | 2 << 16)
        .unwrap();
    bus.write_u32(0x40000820, 1).unwrap(); // MCLK.APBDMASK.SERCOM4
    bus.write_u32(GCLK + 0x80 + 34 * 4, generator | 1 << 6)
        .unwrap();
    bus.write_u8(PORTB + 0x30 + 6, 0x20).unwrap();
    bus.write_u8(PORTB + 0x30 + 7, 0x20).unwrap();
    bus.write_u8(PORTB + 0x40 + 13, 1).unwrap();
    bus.write_u8(PORTB + 0x40 + 15, 1).unwrap();
    bus.write_u32(PORTB, 1 << 7).unwrap(); // CS (PB7) driven low below
    let master = (3 << 2) | (2 << 16);
    bus.write_u32(SPI, master).unwrap();
    bus.write_u32(SPI + 4, 1 << 17).unwrap();
    bus.write_u8(SPI + 0x0c, 3).unwrap(); // BAUD 3: 64 core clocks a byte
    bus.write_u32(SPI, master | 2).unwrap();
    let mut machine = idle_machine(bus);
    machine.bus.write_u8(SPI + 0x28, 0xa5).unwrap();
    let start = machine.total_cycles;
    for _ in 0..10_000 {
        machine.step().unwrap();
        if !seen.lock().unwrap().is_empty() {
            assert!(seen.lock().unwrap().contains(&0xa5));
            return machine.total_cycles - start;
        }
    }
    panic!("SPI byte never completed on generator {generator}");
}

#[test]
fn spi_byte_time_scales_with_the_core_clock_ratio() {
    let same = spi_byte_ticks(0);
    let half = spi_byte_ticks(2);
    // 64 core clocks at f_CPU, then at f_CPU / 2: twice the CPU cycles.
    assert!(
        same >= 64,
        "a byte takes 16 * (BAUD + 1) clocks, got {same}"
    );
    // Within an instruction's worth of cycles either way.
    let delta = half as i64 - 2 * same as i64;
    assert!(delta.abs() <= 8, "half-rate byte {half} vs {same}");
}

#[test]
fn paced_usart_frame_follows_the_baud_on_the_clock_tree() {
    let (mut chip, manifest) = load("atsamd51-pybadge.yaml", "pybadge.yaml");
    // Serial1: SERCOM3 on PA22 (PAD0, TX) / PA23 (PAD1, RX), function C.
    let entry = chip
        .peripherals
        .iter_mut()
        .find(|p| p.id == "sercom3")
        .unwrap();
    entry.r#type = "samd51_sercom_usart".into();
    entry.config.clear();
    for (key, value) in [
        ("tx_port_base", PORTA),
        ("tx_bit", 22),
        ("rx_port_base", PORTA),
        ("rx_bit", 23),
        ("pad_mux", 2),
        ("txpo", 0),
        ("rxpo", 1),
        ("gclk_id", 24),
    ] {
        entry.config.insert(key.into(), value.into());
    }
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let sink = Arc::new(Mutex::new(Vec::new()));
    assert!(bus.attach_uart_tx_sink_named("sercom3", sink.clone(), false));
    bus.write_u32(GCLK + 0x20, 6 | 1 << 8).unwrap(); // generator 0: 48 MHz
    let apbb = bus.read_u32(0x40000818).unwrap();
    bus.write_u32(0x40000818, apbb | 1 << 10).unwrap(); // MCLK.APBBMASK.SERCOM3
    bus.write_u32(GCLK + 0x80 + 24 * 4, 1 << 6).unwrap();
    bus.write_u8(PORTA + 0x30 + 11, 0x22).unwrap();
    bus.write_u8(PORTA + 0x40 + 22, 1).unwrap();
    bus.write_u8(PORTA + 0x40 + 23, 1).unwrap();
    let usart = 0x41014000;
    let ctrla = 1 << 2 | 1 << 20 | 1 << 30; // internal clock, RXPO 1, LSB first
    bus.write_u32(usart, ctrla).unwrap();
    bus.write_u32(usart + 4, 1 << 16 | 1 << 17).unwrap(); // TXEN, RXEN
                                                          // Arithmetic 16x: 16 * 65536 / 4096 = 256 core clocks a bit, 2560 a frame.
    bus.write_u16(usart + 0x0c, (65536 - 4096) as u16).unwrap();
    bus.write_u32(usart, ctrla | 2).unwrap();
    let mut machine = idle_machine(bus);
    machine.step().unwrap();
    machine.bus.write_u8(usart + 0x28, b'U').unwrap();
    let start = machine.total_cycles;
    while machine.total_cycles - start < 2400 {
        machine.step().unwrap();
    }
    assert!(
        sink.lock().unwrap().is_empty(),
        "the frame is still on the wire"
    );
    while machine.total_cycles - start < 3000 {
        machine.step().unwrap();
    }
    assert!(sink.lock().unwrap().contains(&b'U'), "frame ended at 2560");
}
