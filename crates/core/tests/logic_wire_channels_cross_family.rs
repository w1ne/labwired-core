// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Is the wire channel UNIVERSAL, or is it an STM32 feature?
//!
//! `logic_analyzer_lab_pad_visibility.rs` proves the wire channel on one part
//! of one family. That is not the claim being made. The claim is that ONE call
//! shape — `{ kind: "wire", peripheral, line }`, with the line named the way a
//! datasheet names it — reaches the narration every wired family already
//! publishes, so a lab does not need to know which chip it is on to probe a
//! bus.
//!
//! A claim about families can only be tested across families, so this file
//! makes the SAME call on an ESP32-C3, an nRF52840 and an RP2040, and asserts
//! the same three things every time:
//!
//! 1. the line resolves BY NAME, case-insensitively;
//! 2. the wire carries the traffic;
//! 3. **no pad is muxed to it.** Every configuration below deliberately omits
//!    the pad routing its family needs — the GPIO matrix on the C3, `PSEL` on
//!    Nordic, `FUNCSEL` on the RP2040. A pad probe would be correct to show a
//!    flat line, and the wire probe still shows the bus. That is the whole
//!    difference between the two channel kinds, asserted once per family.
//!
//! The decoders are written against the PROTOCOL and import nothing from the
//! narrators, exactly as the per-family waveform gates do.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::logic_capture::{LogicEdge, LogicSource};
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::{Bus, Machine};
use std::path::PathBuf;

/// Where each chip's RAM actually is. The CPU below only ever runs a branch
/// to itself out of it, but that page has to be MAPPED — the ESP32-C3 puts its
/// SRAM at 0x3FC8_0000, not at the Cortex-M convention every other chip here
/// follows.
fn ram_base(chip: &str) -> u64 {
    match chip {
        "esp32c3" => 0x3FC8_0000,
        _ => 0x2000_0000,
    }
}

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn dummy_manifest(path: &str) -> SystemManifest {
    SystemManifest {
        parts: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "wire-channel-cross-family".to_string(),
        chip: path.to_string(),
        external_devices: vec![],
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    }
}

/// Build a chip the SHIPPED way — `from_config` on the committed yaml, with no
/// `wire_*_pads` call of this file's own.
///
/// A gate that built its own bus and then invoked the wiring itself would prove
/// the narrator works while the shipped chip stayed dark; that is precisely how
/// the ESP32-S3 once kept a green suite and a flat trace.
fn machine_for(chip_name: &str) -> Machine<CortexM> {
    let abs = root(&format!("configs/chips/{chip_name}.yaml"));
    let chip = ChipDescriptor::from_file(&abs).expect("load chip descriptor");
    let mut bus = SystemBus::from_config(&chip, &dummy_manifest(&abs.to_string_lossy()))
        .expect("build bus from the committed chip config");
    let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    // `b .` (0xE7FE): a Thumb branch to itself, so the CPU retires instructions
    // forever and the logic tap's provisional clock advances. Nothing about the
    // firmware matters here — every peripheral below is driven through MMIO
    // writes, exactly as a driver would, and no chip's real ISA is involved.
    let ram = ram_base(chip_name);
    machine.bus.write_u8(ram + 0x1000, 0xFE).unwrap();
    machine.bus.write_u8(ram + 0x1001, 0xE7).unwrap();
    machine.cpu.pc = (ram + 0x1000) as u32;
    machine
}

fn run(machine: &mut Machine<CortexM>, steps: u64) {
    for _ in 0..steps {
        machine.step().expect("step");
    }
}

/// A slave that answers to `ADDR` and returns a known ramp, so a read phase
/// carries bytes nothing else on the bus could have produced.
const ADDR: u8 = 0x76;
struct Ramp {
    next: u8,
}
impl I2cDevice for Ramp {
    fn address(&self) -> u8 {
        ADDR
    }
    fn read(&mut self) -> u8 {
        let byte = self.next;
        self.next = self.next.wrapping_add(0x11);
        byte
    }
    fn write(&mut self, _data: u8) {}
}

/// An INDEPENDENT I²C decoder over two captured channels: START on SDA falling
/// while SCL is high, one bit sampled per SCL rise, nine bits per frame with
/// the ninth the ACK. Imports nothing from `i2c_waveform`.
fn decode_i2c(edges: &[LogicEdge], ch_scl: u32, ch_sda: u32) -> Vec<(u8, bool)> {
    let (mut scl, mut sda) = (true, true);
    let mut started = false;
    let mut bits: Vec<bool> = Vec::new();
    let mut frames = Vec::new();
    for edge in edges {
        let (prev_scl, prev_sda) = (scl, sda);
        if edge.ch == ch_scl {
            scl = edge.value;
        } else if edge.ch == ch_sda {
            sda = edge.value;
        } else {
            continue;
        }
        // START: SDA falls while SCL is high. STOP: SDA rises while SCL high.
        if prev_scl && scl && prev_sda && !sda {
            started = true;
            bits.clear();
            continue;
        }
        if prev_scl && scl && !prev_sda && sda {
            started = false;
            bits.clear();
            continue;
        }
        if started && !prev_scl && scl {
            bits.push(sda);
            if bits.len() == 9 {
                let mut byte = 0u8;
                for &bit in &bits[..8] {
                    byte = (byte << 1) | u8::from(bit);
                }
                frames.push((byte, !bits[8])); // ACK is an active-LOW ninth bit
                bits.clear();
            }
        }
    }
    frames
}

/// An INDEPENDENT 8N1 decoder — the twin of the one in
/// `logic_analyzer_lab_pad_visibility.rs`, written against the protocol.
fn decode_uart_8n1(edges: &[LogicEdge], ch: u32, bit_time: u64) -> Vec<u8> {
    let transitions: Vec<(u64, bool)> = edges
        .iter()
        .filter(|e| e.ch == ch)
        .map(|e| (e.cycle, e.value))
        .collect();
    let level_at = |t: u64| -> bool {
        let mut level = true; // a serial line idles at mark
        for &(cycle, value) in &transitions {
            if cycle > t {
                break;
            }
            level = value;
        }
        level
    };
    let mut bytes = Vec::new();
    let mut frame_ends = 0u64;
    for &(cycle, value) in &transitions {
        if value || cycle < frame_ends {
            continue;
        }
        let mut byte = 0u8;
        for bit in 0..8u64 {
            if level_at(cycle + bit_time * (bit + 1) + bit_time / 2) {
                byte |= 1 << bit;
            }
        }
        bytes.push(byte);
        frame_ends = cycle + bit_time * 10;
    }
    bytes
}

/// Every family's probe is armed through this ONE helper, by name, with the
/// same argument shape. If a family needed its own call, the "universal" claim
/// would be false and this function could not exist.
fn watch_wire(
    machine: &mut Machine<CortexM>,
    peripheral: &str,
    lines: &[&str],
) -> Vec<Option<bool>> {
    let sources: Vec<Option<LogicSource>> = lines
        .iter()
        .map(|line| {
            Some(
                machine
                    .resolve_wire_source(peripheral, line)
                    .unwrap_or_else(|e| panic!("resolve {peripheral}.{line}: {e}")),
            )
        })
        .collect();
    machine.logic_watch(&sources)
}

/// A payload that is bit-ASYMMETRIC under LSB-first framing: 0x53↔0xCA,
/// 0x1C↔0x38, 0xE1↔0x87. `0xA5`, `0x5A`, `0x00` and `0xFF` are palindromic
/// under a reversed shift and have let real defects survive here twice.
const ASYMMETRIC: [u8; 3] = [0x53, 0x1C, 0xE1];

// ── ESP32-C3 ────────────────────────────────────────────────────────────────

/// ACCEPTANCE 4a — an ESP. The GPIO matrix is NEVER programmed: no
/// `FUNC_OUT_SEL`, no `FUNC_IN_SEL`, no output enable. Every pad on this chip
/// is a plain GPIO throughout, and the I²C wire is still fully visible.
#[test]
fn a_wire_probe_reads_the_esp32c3_i2c_bus_with_no_gpio_matrix_routing() {
    const I2C_BASE: u64 = 0x6001_3000;
    const REG_CTR: u64 = 0x04;
    const REG_DATA: u64 = 0x1C;
    const REG_CMD0: u64 = 0x58;

    let mut machine = machine_for("esp32c3");
    // The C3 requires every attached slave to name the pads it is physically
    // wired to — a property of the BOARD, not of the firmware. Declaring the
    // wiring does not program the GPIO matrix: `FUNC_OUT_SEL` is still never
    // written below, so the controller's signals never reach those pads and a
    // pad probe on either of them would correctly stay flat.
    let route = std::collections::BTreeMap::from([
        ("sda".to_string(), "GPIO5".to_string()),
        ("scl".to_string(), "GPIO6".to_string()),
    ]);
    machine
        .bus
        .attach_i2c_slave_with_route("i2c0", Box::new(Ramp { next: 0x60 }), Some(&route))
        .expect("attach an I²C slave to the C3 controller");

    // Controller timing only — the same 100 kHz setup the C3 pad-probe gate
    // uses (`crates/core/src/tests/esp32c3_i2c_waveform.rs`), MINUS every GPIO
    // matrix write that gate makes. Not one GPIO register is touched here.
    let bus = &mut machine.bus;
    bus.write_u32(I2C_BASE, 199).unwrap(); // SCL_LOW_PERIOD
    bus.write_u32(I2C_BASE + 0x38, 180 | (19 << 9)).unwrap(); // SCL_HIGH_PERIOD
    bus.write_u32(I2C_BASE + 0x30, 29).unwrap(); // SDA_HOLD
    bus.write_u32(I2C_BASE + 0x40, 199).unwrap(); // SCL_START_HOLD
    bus.write_u32(I2C_BASE + 0x44, 199).unwrap(); // SCL_RSTART_SETUP
    bus.write_u32(I2C_BASE + 0x4C, 199).unwrap(); // SCL_STOP_SETUP
    bus.write_u32(I2C_BASE + 0x48, 199).unwrap(); // SCL_STOP_HOLD

    let initial = watch_wire(&mut machine, "i2c0", &["SCL", "SDA"]);
    assert_eq!(
        initial,
        vec![Some(true), Some(true)],
        "an idle open-drain I²C wire rests high on both lines",
    );

    run(&mut machine, 40_000);

    // RSTART, WRITE 4 (addr+W then the payload), STOP — the C3 command-list
    // shape its own driver programs.
    let bus = &mut machine.bus;
    let cmd = |opcode: u32, byte_num: u32| (opcode << 11) | byte_num;
    bus.write_u32(I2C_BASE + REG_CMD0, cmd(6, 0)).unwrap(); // RSTART
    bus.write_u32(I2C_BASE + REG_CMD0 + 4, cmd(1, 4)).unwrap(); // WRITE 4
    bus.write_u32(I2C_BASE + REG_CMD0 + 8, cmd(2, 0)).unwrap(); // STOP
    bus.write_u32(I2C_BASE + REG_DATA, u32::from(ADDR) << 1)
        .unwrap();
    for byte in ASYMMETRIC {
        bus.write_u32(I2C_BASE + REG_DATA, u32::from(byte)).unwrap();
    }
    bus.write_u32(I2C_BASE + REG_CTR, 1 << 5).unwrap(); // TRANS_START
    run(&mut machine, 200_000);

    let edges = machine.logic_read_edges(0).edges;
    assert!(
        !edges.is_empty(),
        "the C3's bit engine drove a whole transaction and a wire probe on \
         i2c0.SCL/SDA saw nothing",
    );
    let frames = decode_i2c(&edges, 0, 1);
    assert_eq!(
        frames.first().map(|f| f.0),
        Some(ADDR << 1),
        "the first frame on the wire is the 7-bit address with R/W clear; got \
         {frames:x?}",
    );
    let data: Vec<u8> = frames.iter().skip(1).map(|&(byte, _)| byte).collect();
    assert_eq!(
        data,
        ASYMMETRIC.to_vec(),
        "and then exactly the bytes the controller was given, in order; got \
         {frames:x?}",
    );
}

// ── nRF52840 ────────────────────────────────────────────────────────────────

/// ACCEPTANCE 4b — an nRF52. `PSEL.TXD` is NEVER written, so no pad on either
/// port is claimed by UARTE0 and a pad probe anywhere would be correct to show
/// the GPIO latch. The wire probe reads the serial waveform regardless.
///
/// Nordic names this line `TXD`, and that is what it is called here: the wire
/// channel resolves the name the part's own Product Specification uses
/// (PS v1.11 §6.34.9), not a name invented for uniformity.
#[test]
fn a_wire_probe_reads_the_nrf52840_uarte_with_no_psel_written() {
    const UARTE0: u64 = 0x4000_2000;
    const OFF_ENABLE: u64 = 0x500;
    const TASKS_STARTTX: u64 = 0x008;
    const BAUDRATE: u64 = 0x524;
    const TXD_PTR: u64 = 0x544;
    const TXD_MAXCNT: u64 = 0x548;
    const ENABLE_UARTE: u32 = 8;
    /// `BAUDRATE` = round(baud · 2^32 / 16 MHz); 2^34 / 0x01D6_0000 = 557 core
    /// cycles per bit at 115200 with a 64 MHz core (PS v1.11 §6.34.9.27 p847).
    const BAUD_115200: u32 = 0x01D6_0000;
    const BIT_TIME: u64 = 557;

    let mut machine = machine_for("nrf52840");
    let bus = &mut machine.bus;
    bus.write_u32(UARTE0 + BAUDRATE, BAUD_115200).unwrap();
    bus.write_u32(UARTE0 + OFF_ENABLE, ENABLE_UARTE).unwrap();

    let initial = watch_wire(&mut machine, "uart0", &["txd"]);
    assert_eq!(
        initial,
        vec![Some(true)],
        "the serial line idles at mark, and `txd` resolves the same as `TXD`",
    );

    // Wire time for three characters at 557 cycles/bit before the burst, so the
    // narrator can place it at its true rate rather than compress it.
    run(&mut machine, 80_000);

    let ram = ram_base("nrf52840");
    let bus = &mut machine.bus;
    for (i, &byte) in ASYMMETRIC.iter().enumerate() {
        bus.write_u8(ram + i as u64, byte).unwrap();
    }
    bus.write_u32(UARTE0 + TXD_PTR, ram as u32).unwrap();
    bus.write_u32(UARTE0 + TXD_MAXCNT, ASYMMETRIC.len() as u32)
        .unwrap();
    bus.write_u32(UARTE0 + TASKS_STARTTX, 1).unwrap();
    run(&mut machine, 60_000);

    let edges = machine.logic_read_edges(0).edges;
    assert!(
        !edges.is_empty(),
        "EasyDMA transmitted {ASYMMETRIC:x?} and the wire probe on uart0.TXD \
         stayed flat",
    );
    assert_eq!(
        decode_uart_8n1(&edges, 0, BIT_TIME),
        ASYMMETRIC.to_vec(),
        "the wire must decode to exactly the characters EasyDMA sent",
    );
}

// ── RP2040 ──────────────────────────────────────────────────────────────────

/// ACCEPTANCE 4c — an RP2040. `IO_BANK0.GPIOn_CTRL.FUNCSEL` is NEVER written,
/// so `gpio_set_function` was effectively never called and every pad stays SIO.
/// The wire probe reads the I²C bus anyway.
#[test]
fn a_wire_probe_reads_the_rp2040_i2c_bus_with_no_funcsel_written() {
    const I2C0_BASE: u64 = 0x4004_4000;
    const IC_CON: u64 = 0x00;
    const IC_TAR: u64 = 0x04;
    const IC_DATA_CMD: u64 = 0x10;
    const IC_SS_SCL_HCNT: u64 = 0x14;
    const IC_SS_SCL_LCNT: u64 = 0x18;
    const IC_ENABLE: u64 = 0x6c;
    const DATA_CMD_STOP: u32 = 1 << 9;

    let mut machine = machine_for("rp2040");
    machine
        .bus
        .attach_i2c_slave("i2c0", Box::new(Ramp { next: 0x60 }))
        .expect("attach an I²C slave to the RP2040 controller");

    let bus = &mut machine.bus;
    // 100 kHz from a 125 MHz clk_sys. Controller registers only — IO_BANK0 is
    // left exactly as reset left it.
    bus.write_u32(I2C0_BASE + IC_SS_SCL_HCNT, 625).unwrap();
    bus.write_u32(I2C0_BASE + IC_SS_SCL_LCNT, 625).unwrap();
    bus.write_u32(I2C0_BASE + IC_CON, 0).unwrap();
    bus.write_u32(I2C0_BASE + IC_TAR, u32::from(ADDR)).unwrap();
    bus.write_u32(I2C0_BASE + IC_ENABLE, 1).unwrap();

    let initial = watch_wire(&mut machine, "i2c0", &["SCL", "SDA"]);
    assert_eq!(
        initial,
        vec![Some(true), Some(true)],
        "an idle open-drain I²C wire rests high on both lines",
    );

    run(&mut machine, 40_000);

    let bus = &mut machine.bus;
    for (i, &byte) in ASYMMETRIC.iter().enumerate() {
        let last = i + 1 == ASYMMETRIC.len();
        let cmd = u32::from(byte) | if last { DATA_CMD_STOP } else { 0 };
        bus.write_u32(I2C0_BASE + IC_DATA_CMD, cmd).unwrap();
    }
    run(&mut machine, 200_000);

    let edges = machine.logic_read_edges(0).edges;
    assert!(
        !edges.is_empty(),
        "an RP2040 I²C transfer must reach a wire probe even with no pad \
         function selected",
    );
    let frames = decode_i2c(&edges, 0, 1);
    assert_eq!(
        frames.first().map(|f| f.0),
        Some(ADDR << 1),
        "the first frame on the wire is the addressed write; got {frames:x?}",
    );
    let data: Vec<u8> = frames.iter().skip(1).map(|&(byte, _)| byte).collect();
    assert_eq!(
        data,
        ASYMMETRIC.to_vec(),
        "and then exactly the bytes the transfer carried, in order",
    );
}

/// The universal claim, stated as one assertion rather than three: the same
/// three-argument call resolves on all three families, and the names are the
/// ones an engineer would type.
#[test]
fn the_same_named_wire_call_resolves_on_every_wired_family() {
    for (chip, peripheral, line) in [
        ("esp32c3", "i2c0", "sda"),
        ("nrf52840", "uart0", "TXD"),
        ("rp2040", "i2c0", "Scl"),
        ("stm32l476", "uart5", "tx"),
    ] {
        let machine = machine_for(chip);
        assert!(
            machine.resolve_wire_source(peripheral, line).is_ok(),
            "{chip}: {peripheral}.{line} must resolve — it is published as {:?}",
            machine.wire_line_names(peripheral),
        );
    }
}
