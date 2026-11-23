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

#[derive(Clone, Copy, Debug)]
enum SpiClockChange {
    Sercom,
    Cpu,
}

/// Change the SERCOM or CPU generator's divider while a byte is in flight.
/// This exercises the real board bus and controller; MMIO is host-driven and
/// the attached device sees byte completions, not physical SCK edges.
fn spi_mid_byte_clock_switch(
    from_div: u32,
    to_div: u32,
    force_walk: bool,
    target: SpiClockChange,
) -> (u64, u64, Vec<u8>) {
    let (chip, manifest) = load("atsamd51-pybadge.yaml", "pybadge.yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    if force_walk {
        assert!(bus
            .peripherals
            .iter_mut()
            .find(|p| p.name == "sercom4")
            .unwrap()
            .dev
            .force_legacy_walk());
        bus.recompute_walk_deletable();
        bus.refresh_peripheral_index();
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    bus.attach_spi_device("sercom4", Box::new(Probe(seen.clone())))
        .unwrap();
    let (cpu_div, sercom_div, changed_genctrl) = match target {
        SpiClockChange::Sercom => (1, from_div, GCLK + 0x28),
        SpiClockChange::Cpu => (from_div, 1, GCLK + 0x20),
    };
    bus.write_u32(GCLK + 0x20, 6 | 1 << 8 | cpu_div << 16)
        .unwrap();
    bus.write_u32(GCLK + 0x28, 6 | 1 << 8 | sercom_div << 16)
        .unwrap();
    bus.write_u32(0x40000820, 1).unwrap();
    bus.write_u32(GCLK + 0x80 + 34 * 4, 2 | 1 << 6).unwrap();
    bus.write_u8(PORTB + 0x30 + 6, 0x20).unwrap();
    bus.write_u8(PORTB + 0x30 + 7, 0x20).unwrap();
    bus.write_u8(PORTB + 0x40 + 13, 1).unwrap();
    bus.write_u8(PORTB + 0x40 + 15, 1).unwrap();
    bus.write_u32(PORTB, 1 << 7).unwrap();
    let master = (3 << 2) | (2 << 16);
    bus.write_u32(SPI, master).unwrap();
    bus.write_u32(SPI + 4, 1 << 17).unwrap();
    bus.write_u8(SPI + 0x0c, 3).unwrap(); // 64 SERCOM clocks per byte
    bus.write_u32(SPI, master | 2).unwrap();
    let mut machine = idle_machine(bus);
    machine.config.peripheral_tick_interval = 1;
    machine.bus.config.peripheral_tick_interval = 1;
    machine.bus.write_u8(SPI + 0x28, 0xa5).unwrap();
    let start = machine.total_cycles;
    // Half a byte is 32 SERCOM clocks. A divided CPU executes fewer CPU
    // cycles in that physical interval; dividing SERCOM does the opposite.
    let switch_at = match target {
        SpiClockChange::Sercom => 32 * u64::from(from_div),
        SpiClockChange::Cpu => 32 / u64::from(from_div),
    };
    assert_eq!(
        labwired_core::peripherals::sam::clock_tree::cpu_hz(&machine.bus),
        Some(48_000_000 / u64::from(cpu_div))
    );
    assert_eq!(
        labwired_core::peripherals::sam::clock_tree::peripheral_hz(&machine.bus, 34),
        Some(48_000_000 / u64::from(sercom_div))
    );
    while machine.total_cycles - start < switch_at {
        machine.step().unwrap();
    }
    let switched = machine.total_cycles - start;
    assert!(
        seen.lock().unwrap().is_empty(),
        "byte completed before clock switch"
    );
    machine
        .bus
        .write_u32(changed_genctrl, 6 | 1 << 8 | to_div << 16)
        .unwrap();
    assert_eq!(
        machine.bus.read_u32(changed_genctrl).unwrap(),
        6 | 1 << 8 | to_div << 16
    );
    if matches!(target, SpiClockChange::Cpu) {
        assert_eq!(
            labwired_core::peripherals::sam::clock_tree::cpu_hz(&machine.bus),
            Some(48_000_000 / u64::from(to_div))
        );
        assert_eq!(
            labwired_core::peripherals::sam::clock_tree::peripheral_hz(&machine.bus, 34),
            Some(48_000_000),
            "changing CPU GEN0 must not change SERCOM's independent GEN2"
        );
    }
    for _ in 0..1024 {
        machine.step().unwrap();
        if !seen.lock().unwrap().is_empty() {
            let elapsed = machine.total_cycles - start;
            for _ in 0..16 {
                machine.step().unwrap();
            }
            return (switched, elapsed, seen.lock().unwrap().clone());
        }
    }
    panic!("SPI did not complete after divider {from_div}->{to_div}; walk={force_walk}");
}

#[test]
fn spi_mid_byte_clock_changes_adjust_remaining_transfer_time() {
    let mut results = Vec::new();
    for force_walk in [true, false] {
        if !force_walk && !labwired_core::cycle_clock::event_scheduler_enabled() {
            continue;
        }
        for (from_div, to_div) in [(1, 2), (2, 1)] {
            let (switched, elapsed, bytes) =
                spi_mid_byte_clock_switch(from_div, to_div, force_walk, SpiClockChange::Sercom);
            println!("P2_CLOCK_SWITCH walk={force_walk} divider={from_div}->{to_div} switched={switched} elapsed={elapsed} byte_count={}", bytes.len());
            results.push((force_walk, from_div, to_div, switched, elapsed, bytes));
        }
    }
    for (walk, from_div, to_div, switched, elapsed, bytes) in results {
        assert_eq!(
            bytes,
            [0xa5],
            "one byte only; walk={walk} divider={from_div}->{to_div}"
        );
        // 32 of 64 SERCOM clocks before switching: 32+64 or 64+32 CPU cycles.
        // Allow the existing <=8-cycle bus-service boundary used by the static
        // ratio test; this does not assert physical pin-edge timing.
        assert!(switched.abs_diff(32 * u64::from(from_div)) <= 2);
        assert!(elapsed.abs_diff(96) <= 8, "walk={walk} divider={from_div}->{to_div}: elapsed={elapsed}, expected about 96 CPU cycles");
    }
}

/// GEN0 supplies the CPU; independent GEN2 keeps SERCOM at 48 MHz. BAUD=3
/// needs 64 SERCOM clocks per byte. After 32 clocks, changing the CPU from
/// 48 to 24 MHz gives 32 + 16 CPU cycles; the reverse gives 16 + 32.
/// Thus both transfers take 48 CPU cycles, independently of the controller's
/// internal work units or rescaling algorithm.
#[test]
fn spi_mid_byte_cpu_clock_changes_preserve_remaining_transfer_work() {
    for force_walk in [true, false] {
        if !force_walk && !labwired_core::cycle_clock::event_scheduler_enabled() {
            continue;
        }
        for (from_div, to_div) in [(1, 2), (2, 1)] {
            let (switched, elapsed, bytes) =
                spi_mid_byte_clock_switch(from_div, to_div, force_walk, SpiClockChange::Cpu);
            assert_eq!(
                bytes,
                [0xa5],
                "one byte only; walk={force_walk} CPU divider={from_div}->{to_div}"
            );
            assert!(switched.abs_diff(32 / u64::from(from_div)) <= 2);
            // Match the existing eight-cycle bus-service tolerance. With no
            // CPU-rate rescale these paths finish near 64 or 32 cycles, so
            // either missing direction falls outside the accepted interval.
            assert!(
                elapsed.abs_diff(48) <= 8,
                "walk={force_walk} CPU divider={from_div}->{to_div}: elapsed={elapsed}, expected about 48 CPU cycles"
            );
        }
    }
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

/// Gating the peripheral clock must pause an in-flight byte and its TXC IRQ.
#[test]
fn spi_mid_byte_gclk_gate_pauses_transfer_and_txc_irq() {
    for force_walk in [true, false] {
        if !force_walk && !labwired_core::cycle_clock::event_scheduler_enabled() {
            continue;
        }
        let (chip, manifest) = load("atsamd51-pybadge.yaml", "pybadge.yaml");
        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
        if force_walk {
            assert!(bus
                .peripherals
                .iter_mut()
                .find(|p| p.name == "sercom4")
                .unwrap()
                .dev
                .force_legacy_walk());
            bus.recompute_walk_deletable();
            bus.refresh_peripheral_index();
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        bus.attach_spi_device("sercom4", Box::new(Probe(seen.clone())))
            .unwrap();
        bus.write_u32(GCLK + 0x20, 6 | 1 << 8).unwrap();
        bus.write_u32(GCLK + 0x28, 6 | 1 << 8 | 1 << 16).unwrap();
        bus.write_u32(0x40000820, 1).unwrap();
        bus.write_u32(GCLK + 0x80 + 34 * 4, 2 | 1 << 6).unwrap();
        bus.write_u8(PORTB + 0x30 + 6, 0x20).unwrap();
        bus.write_u8(PORTB + 0x30 + 7, 0x20).unwrap();
        bus.write_u8(PORTB + 0x40 + 13, 1).unwrap();
        bus.write_u8(PORTB + 0x40 + 15, 1).unwrap();
        bus.write_u32(PORTB, 1 << 7).unwrap();
        let master = (3 << 2) | (2 << 16);
        bus.write_u32(SPI, master).unwrap();
        bus.write_u32(SPI + 4, 1 << 17).unwrap();
        bus.write_u8(SPI + 0x0c, 3).unwrap();
        bus.write_u32(SPI, master | 2).unwrap();
        bus.write_u8(SPI + 0x16, 2).unwrap(); // TXC interrupt only
        let mut machine = idle_machine(bus);
        // Observe the peripheral IRQ line without entering an absent guest vector.
        machine.cpu.primask = true;
        machine.config.peripheral_tick_interval = 1;
        machine.bus.config.peripheral_tick_interval = 1;
        machine.bus.write_u8(SPI + 0x28, 0xa5).unwrap();
        let start = machine.total_cycles;
        while machine.total_cycles - start < 32 {
            machine.step().unwrap();
        }
        assert!(seen.lock().unwrap().is_empty());
        machine.bus.write_u32(GCLK + 0x80 + 34 * 4, 2).unwrap();
        for _ in 0..200 {
            machine.step().unwrap();
        }
        assert!(
            seen.lock().unwrap().is_empty(),
            "completion while clock gated"
        );
        let idx = machine
            .bus
            .peripherals
            .iter()
            .position(|p| p.name == "sercom4")
            .unwrap();
        let mut txc_irq = false;
        machine.bus.peripherals[idx]
            .dev
            .irq_line_levels(&mut |irq, level| {
                if irq == 63 {
                    txc_irq = level;
                }
            });
        assert!(!txc_irq, "TXC IRQ while gated");
        machine
            .bus
            .write_u32(GCLK + 0x80 + 34 * 4, 2 | 1 << 6)
            .unwrap();
        for _ in 0..256 {
            machine.step().unwrap();
            if !seen.lock().unwrap().is_empty() {
                break;
            }
        }
        assert_eq!(*seen.lock().unwrap(), [0xa5], "one byte after resume");
        for _ in 0..16 {
            machine.step().unwrap();
        }
        assert_eq!(*seen.lock().unwrap(), [0xa5], "no duplicate byte");
        machine.bus.peripherals[idx]
            .dev
            .irq_line_levels(&mut |irq, level| {
                if irq == 63 {
                    txc_irq = level;
                }
            });
        assert!(txc_irq, "TXC IRQ after completion");
        println!("P2_GATE walk={force_walk} gated_steps=200 bytes=1 txc_irq_after=true");
    }
}
