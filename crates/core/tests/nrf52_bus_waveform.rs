// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF52840 bus visibility, end to end, on the path a lab actually runs.
//!
//! # What this proves that a hand-built bus cannot
//!
//! Every chip here is built by `SystemBus::from_config` from the COMMITTED
//! `configs/chips/nrf52840.yaml`, exactly as `chip_conformance.rs` and
//! `bus_visibility.rs` do. Nothing below calls a `wire_*_pads` function
//! itself. A gate that constructed its own bus and then invoked the wiring
//! would prove the narrator works while the shipped chip stayed dark — that
//! is exactly how the ESP32-S3 kept a green suite and a flat trace.
//!
//! # Why this family needs its own gate at all
//!
//! On the other four wired families a pad names its function in a register, so
//! "is the route live" is a question about the GPIO port. Nordic inverts it:
//! the pad has no function register, and the PERIPHERAL names the pin in its
//! `PSEL.*` word (nRF52840 PS v1.11 §6.31.7.19, p798 — `PIN[4:0]`, `PORT[5]`,
//! `CONNECT[31]`). So the assertions here are about a claim published from the
//! peripheral side: a pad reads the wire only while some peripheral's `PSEL`
//! selects it AND that peripheral is enabled, and re-pointing `PSEL` mid-run
//! must move the waveform with it.
//!
//! # The decoders are independent
//!
//! Each `decode_*` below is written against the PROTOCOL, from the captured
//! edges alone: it knows nothing about `I2cNarrator`, `UartNarrator` or
//! `SpiNarrator` and does not import them. It is the same decoder shape the
//! RP2040, STM32 and ESP32 waveform gates use. If the narration and the
//! decoder were derived from each other, both could be wrong together and this
//! file would assert nothing.

use labwired_core::logic_capture::LogicSource;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::logic_capture::LogicEdge;
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::peripherals::spi::SpiDevice;
use labwired_core::{Bus, Machine};
use std::path::PathBuf;

// ── The nRF52840 memory map, as configs/chips/nrf52840.yaml declares it ──────
const UARTE0: u64 = 0x4000_2000;
/// SPIM0 / TWIM0 share this window; ENABLE picks the personality.
const SERIAL0: u64 = 0x4000_3000;
const SPIM2: u64 = 0x4002_3000;
const RAM: u64 = 0x2000_0000;

// ── Shared Nordic peripheral register offsets (PS v1.11) ────────────────────
const OFF_ENABLE: u64 = 0x500;
/// TWIM: PSEL.SCL / PSEL.SDA. SPIM: PSEL.SCK / PSEL.MOSI. UARTE: PSEL.RTS /
/// PSEL.TXD. Same two words, different personality (§6.31.7 p791 vs §6.25.6
/// p727 vs §6.34.9 p836).
const OFF_PSEL_A: u64 = 0x508;
const OFF_PSEL_B: u64 = 0x50C;
const OFF_FREQUENCY: u64 = 0x524;

const ENABLE_TWIM: u32 = 6;
const ENABLE_SPIM: u32 = 7;
const ENABLE_UARTE: u32 = 8;

// TWIM
const TWIM_TASKS_STARTTX: u64 = 0x008;
const TWIM_SHORTS: u64 = 0x200;
const TWIM_SHORT_LASTTX_STARTRX: u32 = 1 << 7;
const TWIM_SHORT_LASTRX_STOP: u32 = 1 << 12;
const TWIM_RXD_PTR: u64 = 0x534;
const TWIM_RXD_MAXCNT: u64 = 0x538;
const TWIM_TXD_PTR: u64 = 0x544;
const TWIM_TXD_MAXCNT: u64 = 0x548;
const TWIM_ADDRESS: u64 = 0x588;
/// FREQUENCY = K100 (100 kbps) — the model's 100 kHz bucket, 640 core cycles
/// per SCL bit at 64 MHz.
const TWIM_K100: u32 = 0x0198_0000;

// SPIM
const SPIM_TASKS_START: u64 = 0x010;
const SPIM_RXD_PTR: u64 = 0x534;
const SPIM_RXD_MAXCNT: u64 = 0x538;
const SPIM_TXD_PTR: u64 = 0x544;
const SPIM_TXD_MAXCNT: u64 = 0x548;
const SPIM_CONFIG: u64 = 0x554;
/// FREQUENCY = M1 (1 Mbps), PS v1.11 §6.25.6.19 p734 → 64 core cycles per bit.
const SPIM_M1: u32 = 0x1000_0000;

// UARTE
const UARTE_TASKS_STARTTX: u64 = 0x008;
const UARTE_PSEL_TXD: u64 = 0x50C;
const UARTE_BAUDRATE: u64 = 0x524;
const UARTE_TXD_PTR: u64 = 0x544;
const UARTE_TXD_MAXCNT: u64 = 0x548;
/// Baud115200 for the UARTE personality, PS v1.11 §6.34.9.27 p847.
const UARTE_BAUD_115200: u32 = 0x01D6_0000;

const SLAVE_ADDR: u8 = 0x76;

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// The same minimal manifest `bus_visibility.rs` and `chip_conformance.rs`
/// use — chip only, so the construction path under test is the shipped one and
/// not something a system yaml decorated.
fn dummy_manifest(path: &str) -> SystemManifest {
    SystemManifest {
        parts: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "nrf52-bus-waveform".to_string(),
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

/// An I²C slave that answers to `SLAVE_ADDR` and returns a known ramp, so an
/// RX narration can be checked against bytes nothing else could have produced.
struct Ramp {
    next: u8,
}
impl I2cDevice for Ramp {
    fn address(&self) -> u8 {
        SLAVE_ADDR
    }
    fn read(&mut self) -> u8 {
        let byte = self.next;
        self.next = self.next.wrapping_add(0x11);
        byte
    }
    fn write(&mut self, _data: u8) {}
}

/// A SPI device that returns a fixed byte; present only so the SPIM has
/// somebody on the bus, which is the shape a real lab has.
struct SpiSink;
impl SpiDevice for SpiSink {
    fn cs_pin(&self) -> &str {
        "P0.12"
    }
    fn transfer(&mut self, _mosi: u8) -> u8 {
        0x5A
    }
}

/// Build the nRF52840 the shipped way and give it a CPU that retires
/// instructions, so the logic tap's provisional clock advances and the pad
/// pushes land in the ring.
fn machine() -> Machine<labwired_core::cpu::CortexM> {
    machine_for("nrf52840")
}

fn machine_for(chip_name: &str) -> Machine<labwired_core::cpu::CortexM> {
    let abs = root(&format!("configs/chips/{chip_name}.yaml"));
    let abs_str = abs.to_string_lossy().to_string();
    let chip = ChipDescriptor::from_file(&abs).expect("load chip");
    let mut bus = SystemBus::from_config(&chip, &dummy_manifest(&abs_str)).expect("build bus");
    let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);

    let mut machine = Machine::new(cpu, bus);
    // `b .` — a Thumb branch to itself (0xE7FE) for the CPU to retire forever.
    // Nothing about the firmware matters here: the bus is driven through MMIO
    // writes below, exactly as a driver would, and the CPU is only present to
    // advance the clock the logic tap stamps against. A straight-line NOP page
    // would run off its end after a few hundred thousand steps and fault, which
    // is not the thing under test.
    machine.bus.write_u8(RAM + 0x1000, 0xFE).unwrap();
    machine.bus.write_u8(RAM + 0x1001, 0xE7).unwrap();
    machine.cpu.pc = (RAM + 0x1000) as u32;
    machine
}

fn gpio0(machine: &Machine<labwired_core::cpu::CortexM>) -> usize {
    machine
        .bus
        .find_peripheral_index_by_name("gpio0")
        .expect("nrf52840 has gpio0")
}

fn run(machine: &mut Machine<labwired_core::cpu::CortexM>, cycles: usize) {
    for _ in 0..cycles {
        machine.step().unwrap();
    }
}

/// `PSEL` word for `P<port>.<pin>`, CONNECT = Connected
/// (PS v1.11 §6.31.7.19, p798).
fn psel(port: u32, pin: u32) -> u32 {
    (port << 5) | pin
}

// ── Independent decoders ────────────────────────────────────────────────────

/// An I²C decoder written from the protocol: START is SDA falling while SCL is
/// high, STOP is SDA rising while SCL is high, and a bit is whatever SDA holds
/// at each SCL rising edge. Nine bits make a frame — eight data, MSB first,
/// then the acknowledge slot, which is LOW for ACK.
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
        if edge.ch == ch_sda && prev_sda && !sda && scl {
            started = true;
            bits.clear();
            continue;
        }
        if edge.ch == ch_sda && !prev_sda && sda && scl {
            started = false;
            bits.clear();
            continue;
        }
        if started && edge.ch == ch_scl && !prev_scl && scl {
            bits.push(sda);
            if bits.len() == 9 {
                let byte = bits[..8]
                    .iter()
                    .fold(0u8, |acc, &bit| (acc << 1) | u8::from(bit));
                frames.push((byte, !bits[8]));
                bits.clear();
            }
        }
    }
    frames
}

/// An asynchronous-serial decoder: find each falling edge from the idle mark,
/// sample the middle of every following bit period, LSB first, 8N1. `bit_time`
/// is measured from the trace rather than assumed, so this also proves the
/// waveform carries a consistent bit rate.
fn decode_uart(edges: &[LogicEdge], ch: u32, bit_time: u64) -> Vec<u8> {
    // Rebuild the level timeline for this channel.
    let timeline: Vec<(u64, bool)> = edges
        .iter()
        .filter(|e| e.ch == ch)
        .map(|e| (e.cycle, e.value))
        .collect();
    let level_at = |cycle: u64| -> bool {
        let mut level = true; // idle mark before the first edge
        for &(at, value) in &timeline {
            if at <= cycle {
                level = value;
            } else {
                break;
            }
        }
        level
    };
    let mut bytes = Vec::new();
    let mut cursor = 0u64;
    for &(at, value) in &timeline {
        if value || at < cursor {
            continue; // not a start bit, or inside a character already decoded
        }
        let mut byte = 0u8;
        for index in 0..8u64 {
            // Sample the MIDDLE of data bit `index`: one bit period past the
            // start bit's leading edge, plus half a bit.
            let sample = at + bit_time + index * bit_time + bit_time / 2;
            if level_at(sample) {
                byte |= 1 << index;
            }
        }
        bytes.push(byte);
        cursor = at + 9 * bit_time;
    }
    bytes
}

/// A mode-0 SPI decoder: sample MOSI on every SCK RISING edge, MSB first,
/// eight bits to a byte.
fn decode_spi(edges: &[LogicEdge], ch_sck: u32, ch_mosi: u32) -> Vec<u8> {
    let mut sck = false;
    let mut mosi = false;
    let mut bits: Vec<bool> = Vec::new();
    let mut bytes = Vec::new();
    for edge in edges {
        let prev_sck = sck;
        if edge.ch == ch_sck {
            sck = edge.value;
        } else if edge.ch == ch_mosi {
            mosi = edge.value;
            continue;
        } else {
            continue;
        }
        if !prev_sck && sck {
            bits.push(mosi);
            if bits.len() == 8 {
                bytes.push(
                    bits.iter()
                        .fold(0u8, |acc, &bit| (acc << 1) | u8::from(bit)),
                );
                bits.clear();
            }
        }
    }
    bytes
}

// ── TWIM (I²C) ──────────────────────────────────────────────────────────────

/// Program TWIM0 the way nrfx does: PSEL first, ENABLE second.
fn configure_twim(machine: &mut Machine<labwired_core::cpu::CortexM>, scl: u32, sda: u32) {
    let bus = &mut machine.bus;
    bus.write_u32(SERIAL0 + OFF_PSEL_A, psel(0, scl)).unwrap();
    bus.write_u32(SERIAL0 + OFF_PSEL_B, psel(0, sda)).unwrap();
    bus.write_u32(SERIAL0 + OFF_ENABLE, ENABLE_TWIM).unwrap();
    bus.write_u32(SERIAL0 + OFF_FREQUENCY, TWIM_K100).unwrap();
    bus.write_u32(SERIAL0 + TWIM_ADDRESS, u32::from(SLAVE_ADDR))
        .unwrap();
}

#[test]
fn a_twim_transfer_puts_a_decodable_i2c_waveform_on_the_pads_psel_selects() {
    const SCL: u8 = 27;
    const SDA: u8 = 26;
    const CH_SCL: u32 = 0;
    const CH_SDA: u32 = 1;

    let mut machine = machine();
    machine
        .bus
        .attach_i2c_slave("i2c0", Box::new(Ramp { next: 0x60 }))
        .expect("attach an I²C slave to the TWIM0 window");
    configure_twim(&mut machine, u32::from(SCL), u32::from(SDA));

    let gpio = gpio0(&machine);
    let initial = machine.logic_watch(&[
        Some(LogicSource::pad(gpio, SCL)),
        Some(LogicSource::pad(gpio, SDA)),
    ]);
    assert_eq!(
        initial,
        vec![Some(true), Some(true)],
        "an idle open-drain I²C bus rests high on both pads PSEL selects — a \
         pad still reading the GPIO output latch would answer low here",
    );

    // One byte at 100 kHz is ~11 500 core cycles of wire time; give the trace
    // history to occupy before the transfer, or the narrator can only compress.
    run(&mut machine, 80_000);

    // Register-pointer write, repeated START, one-byte read — the nrfx
    // `twim_xfer` shape every sensor driver uses.
    let bus = &mut machine.bus;
    bus.write_u8(RAM, 0xD0).unwrap();
    bus.write_u32(SERIAL0 + TWIM_TXD_PTR, RAM as u32).unwrap();
    bus.write_u32(SERIAL0 + TWIM_TXD_MAXCNT, 1).unwrap();
    bus.write_u32(SERIAL0 + TWIM_RXD_PTR, (RAM + 0x100) as u32)
        .unwrap();
    bus.write_u32(SERIAL0 + TWIM_RXD_MAXCNT, 1).unwrap();
    bus.write_u32(
        SERIAL0 + TWIM_SHORTS,
        TWIM_SHORT_LASTTX_STARTRX | TWIM_SHORT_LASTRX_STOP,
    )
    .unwrap();
    bus.write_u32(SERIAL0 + TWIM_TASKS_STARTTX, 1).unwrap();
    run(&mut machine, 120_000);

    let edges = machine.logic_read_edges(0).edges;
    assert!(
        !edges.is_empty(),
        "a TWIM transfer must put edges on the pads PSEL.SCL/PSEL.SDA select, \
         not a flat trace",
    );
    assert_eq!(
        decode_i2c(&edges, CH_SCL, CH_SDA),
        vec![
            (SLAVE_ADDR << 1, true),
            (0xD0, true),
            ((SLAVE_ADDR << 1) | 1, true),
            (0x60, false),
        ],
        "the wire must carry write-0xD0, repeated START, read-0x60 — the \
         address clocked twice with the R/W bit flipped, and the master NACKing \
         the final read byte",
    );
}

#[test]
fn scl_runs_at_the_rate_the_frequency_register_programs() {
    const SCL: u8 = 27;
    const SDA: u8 = 26;
    let mut machine = machine();
    machine
        .bus
        .attach_i2c_slave("i2c0", Box::new(Ramp { next: 0x60 }))
        .unwrap();
    configure_twim(&mut machine, u32::from(SCL), u32::from(SDA));
    let gpio = gpio0(&machine);
    machine.logic_watch(&[
        Some(LogicSource::pad(gpio, SCL)),
        Some(LogicSource::pad(gpio, SDA)),
    ]);
    run(&mut machine, 80_000);

    let bus = &mut machine.bus;
    bus.write_u8(RAM, 0xD0).unwrap();
    bus.write_u32(SERIAL0 + TWIM_TXD_PTR, RAM as u32).unwrap();
    bus.write_u32(SERIAL0 + TWIM_TXD_MAXCNT, 1).unwrap();
    bus.write_u32(SERIAL0 + TWIM_TASKS_STARTTX, 1).unwrap();
    run(&mut machine, 120_000);

    // 64 MHz core / 100 kHz SCL = 640 cycles per bit.
    const EXPECTED: u64 = 640;
    let rises: Vec<u64> = machine
        .logic_read_edges(0)
        .edges
        .iter()
        .filter(|e| e.ch == 0 && e.value)
        .map(|e| e.cycle)
        .collect();
    assert!(rises.len() >= 9, "at least one full frame of clocks");
    let gaps: Vec<u64> = rises.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let at_rate = gaps.iter().filter(|&&gap| gap == EXPECTED).count();
    assert!(
        at_rate >= gaps.len() - 1,
        "SCL period must be {EXPECTED} cycles at FREQUENCY = K100, got {gaps:?}",
    );
}

#[test]
fn a_pad_no_psel_selects_shows_no_bus_traffic() {
    // The whole point of the peripheral-side seam: the claim table, not the
    // binding, decides. Every pad on this port is BOUND to every signal, so if
    // the claim were ignored an unrelated pin would show the bus.
    const SCL: u8 = 27;
    const SDA: u8 = 26;
    const BYSTANDER: u8 = 11;

    let mut machine = machine();
    machine
        .bus
        .attach_i2c_slave("i2c0", Box::new(Ramp { next: 0x60 }))
        .unwrap();
    configure_twim(&mut machine, u32::from(SCL), u32::from(SDA));

    let gpio = gpio0(&machine);
    machine.logic_watch(&[Some(LogicSource::pad(gpio, BYSTANDER))]);
    run(&mut machine, 80_000);

    let bus = &mut machine.bus;
    bus.write_u8(RAM, 0xD0).unwrap();
    bus.write_u32(SERIAL0 + TWIM_TXD_PTR, RAM as u32).unwrap();
    bus.write_u32(SERIAL0 + TWIM_TXD_MAXCNT, 1).unwrap();
    bus.write_u32(SERIAL0 + TWIM_TASKS_STARTTX, 1).unwrap();
    run(&mut machine, 120_000);

    assert!(
        machine.logic_read_edges(0).edges.is_empty(),
        "P0.{BYSTANDER} is a plain GPIO — no PSEL names it — so it must keep \
         reading its own latch, not the I²C wire",
    );
}

#[test]
fn a_disabled_twim_releases_its_pads_back_to_the_gpio_latch() {
    // nRF52840 PS v1.11 §6.31.6 (p790): the PSEL registers "are only used as
    // long as the TWI master is enabled … When the peripheral is disabled, the
    // pins will behave as regular GPIOs". A claim that ignored ENABLE would
    // leave a probe reading an idle-high wire while firmware drove the pin low.
    const SCL: u8 = 27;
    let mut machine = machine();
    configure_twim(&mut machine, u32::from(SCL), 26);

    let gpio = gpio0(&machine);
    let claimed = machine.logic_watch(&[Some(LogicSource::pad(gpio, SCL))]);
    assert_eq!(
        claimed,
        vec![Some(true)],
        "while enabled the pad reads the idle-high I²C wire",
    );

    // Disable TWIM, then drive the pin low as a plain GPIO: DIR then OUT.
    let bus = &mut machine.bus;
    bus.write_u32(SERIAL0 + OFF_ENABLE, 0).unwrap();
    bus.write_u32(0x5000_0000 + 0x518, 1 << SCL).unwrap(); // DIRSET
    bus.write_u32(0x5000_0000 + 0x50C, 1 << SCL).unwrap(); // OUTCLR
    let released = machine.logic_watch(&[Some(LogicSource::pad(gpio, SCL))]);
    assert_eq!(
        released,
        vec![Some(false)],
        "a disabled TWIM must hand P0.{SCL} back, so the pad reads the GPIO \
         output latch firmware just cleared",
    );
}

#[test]
fn re_pointing_psel_at_runtime_moves_the_waveform_to_the_new_pad() {
    // PSEL is runtime-mutable and Zephyr pinctrl really does re-apply a state.
    // A bind-once table would leave the waveform on the first pad forever.
    const FIRST: u8 = 27;
    const SECOND: u8 = 15;
    let mut machine = machine();
    configure_twim(&mut machine, u32::from(FIRST), 26);

    let gpio = gpio0(&machine);
    // Park both candidate pads as driven-low GPIOs, so "reads the wire" and
    // "reads the latch" are distinguishable levels rather than both high.
    let bus = &mut machine.bus;
    bus.write_u32(0x5000_0000 + 0x518, (1 << FIRST) | (1 << SECOND))
        .unwrap(); // DIRSET
    bus.write_u32(0x5000_0000 + 0x50C, (1 << FIRST) | (1 << SECOND))
        .unwrap(); // OUTCLR

    assert_eq!(
        machine.logic_watch(&[
            Some(LogicSource::pad(gpio, FIRST)),
            Some(LogicSource::pad(gpio, SECOND))
        ]),
        vec![Some(true), Some(false)],
        "P0.{FIRST} carries SCL (idle high); P0.{SECOND} is still a GPIO (low)",
    );

    // Disable, re-point SCL, re-enable — the order nrfx reconfigures in.
    let bus = &mut machine.bus;
    bus.write_u32(SERIAL0 + OFF_ENABLE, 0).unwrap();
    bus.write_u32(SERIAL0 + OFF_PSEL_A, psel(0, u32::from(SECOND)))
        .unwrap();
    bus.write_u32(SERIAL0 + OFF_ENABLE, ENABLE_TWIM).unwrap();

    assert_eq!(
        machine.logic_watch(&[
            Some(LogicSource::pad(gpio, FIRST)),
            Some(LogicSource::pad(gpio, SECOND))
        ]),
        vec![Some(false), Some(true)],
        "the claim must FOLLOW PSEL: P0.{FIRST} is a plain GPIO again, and \
         P0.{SECOND} now reads the I²C wire",
    );
}

// ── UARTE ───────────────────────────────────────────────────────────────────

#[test]
fn a_uarte_transfer_puts_a_decodable_serial_waveform_on_the_pad_psel_txd_selects() {
    const TX: u8 = 6; // P0.06 — UARTE0 TX on the nRF52840 DK
    const CH_TX: u32 = 0;
    /// 2^34 / 0x01D60000 = 557 core cycles per bit at 115200 baud
    /// (`BAUDRATE` = round(baud · 2^32 / 16 MHz), core = 64 MHz).
    const BIT_TIME: u64 = 557;

    let mut machine = machine();
    let bus = &mut machine.bus;
    bus.write_u32(UARTE0 + UARTE_PSEL_TXD, psel(0, u32::from(TX)))
        .unwrap();
    bus.write_u32(UARTE0 + UARTE_BAUDRATE, UARTE_BAUD_115200)
        .unwrap();
    bus.write_u32(UARTE0 + OFF_ENABLE, ENABLE_UARTE).unwrap();

    let gpio = gpio0(&machine);
    assert_eq!(
        machine.logic_watch(&[Some(LogicSource::pad(gpio, TX))]),
        vec![Some(true)],
        "an idle serial line rests HIGH (mark) on the pad PSEL.TXD selects",
    );

    // Four characters at 555 cycles/bit is ~22 000 cycles of wire time.
    run(&mut machine, 80_000);

    let payload = b"Hi!\n";
    let bus = &mut machine.bus;
    for (i, &byte) in payload.iter().enumerate() {
        bus.write_u8(RAM + i as u64, byte).unwrap();
    }
    bus.write_u32(UARTE0 + UARTE_TXD_PTR, RAM as u32).unwrap();
    bus.write_u32(UARTE0 + UARTE_TXD_MAXCNT, payload.len() as u32)
        .unwrap();
    bus.write_u32(UARTE0 + UARTE_TASKS_STARTTX, 1).unwrap();
    run(&mut machine, 40_000);

    let edges = machine.logic_read_edges(0).edges;
    assert!(
        !edges.is_empty(),
        "a UARTE transmission must put edges on the pad PSEL.TXD selects",
    );
    assert_eq!(
        decode_uart(&edges, CH_TX, BIT_TIME),
        payload.to_vec(),
        "the wire must carry exactly the characters EasyDMA sent, LSB first at \
         the programmed baud",
    );
}

#[test]
fn uarte_bit_time_is_the_one_the_baudrate_register_programs() {
    const TX: u8 = 6;
    let mut machine = machine();
    let bus = &mut machine.bus;
    bus.write_u32(UARTE0 + UARTE_PSEL_TXD, psel(0, u32::from(TX)))
        .unwrap();
    bus.write_u32(UARTE0 + UARTE_BAUDRATE, UARTE_BAUD_115200)
        .unwrap();
    bus.write_u32(UARTE0 + OFF_ENABLE, ENABLE_UARTE).unwrap();
    let gpio = gpio0(&machine);
    machine.logic_watch(&[Some(LogicSource::pad(gpio, TX))]);
    run(&mut machine, 80_000);

    // 0xAA alternates every bit, so every bit boundary is an edge and the
    // period is directly measurable rather than inferred.
    let bus = &mut machine.bus;
    bus.write_u8(RAM, 0xAA).unwrap();
    bus.write_u32(UARTE0 + UARTE_TXD_PTR, RAM as u32).unwrap();
    bus.write_u32(UARTE0 + UARTE_TXD_MAXCNT, 1).unwrap();
    bus.write_u32(UARTE0 + UARTE_TASKS_STARTTX, 1).unwrap();
    run(&mut machine, 40_000);

    let cycles: Vec<u64> = machine
        .logic_read_edges(0)
        .edges
        .iter()
        .filter(|e| e.ch == 0)
        .map(|e| e.cycle)
        .collect();
    assert!(cycles.len() >= 8, "0xAA in 8N1 has an edge every bit");
    let gaps: Vec<u64> = cycles.windows(2).map(|pair| pair[1] - pair[0]).collect();

    // 0xAA is 0b10101010, sent LSB first after a low start bit, so bit 0 is
    // ALSO low: the first two bit periods carry no transition between them and
    // the opening gap is two bit times, not one.
    const BIT: u64 = 557;
    assert_eq!(
        gaps[0],
        2 * BIT,
        "start bit and bit 0 are both low, so the first gap spans two bit \
         periods; got {gaps:?}",
    );
    // The LAST edge is exempt. `LogicCapture::ingest_push` re-stamps any event
    // at or after the instruction-batch boundary to `now`, i.e. after the
    // batch's peripheral tick costs — so the closing transition of a narration
    // anchored at the present cycle lands a few cycles late by construction.
    // That is the capture layer's contract, not the narrator's rate.
    let interior = &gaps[1..gaps.len() - 1];
    assert!(
        !interior.is_empty() && interior.iter().all(|&gap| gap == BIT),
        "every interior bit boundary must be {BIT} cycles apart \
         (2^34 / BAUDRATE at 115200), got {gaps:?}",
    );
}

// ── SPIM ────────────────────────────────────────────────────────────────────

#[test]
fn a_spim_transfer_puts_a_decodable_spi_waveform_on_the_pads_psel_selects() {
    const SCK: u8 = 13;
    const MOSI: u8 = 14;
    const CH_SCK: u32 = 0;
    const CH_MOSI: u32 = 1;

    let mut machine = machine();
    machine
        .bus
        .attach_spi_device("spi2", Box::new(SpiSink))
        .expect("attach a SPI device to SPIM2");
    let bus = &mut machine.bus;
    bus.write_u32(SPIM2 + OFF_PSEL_A, psel(0, u32::from(SCK)))
        .unwrap();
    bus.write_u32(SPIM2 + OFF_PSEL_B, psel(0, u32::from(MOSI)))
        .unwrap();
    bus.write_u32(SPIM2 + SPIM_CONFIG, 0).unwrap(); // MsbFirst, mode 0
    bus.write_u32(SPIM2 + OFF_FREQUENCY, SPIM_M1).unwrap();
    bus.write_u32(SPIM2 + OFF_ENABLE, ENABLE_SPIM).unwrap();

    let gpio = gpio0(&machine);
    assert_eq!(
        machine.logic_watch(&[
            Some(LogicSource::pad(gpio, SCK)),
            Some(LogicSource::pad(gpio, MOSI))
        ]),
        vec![Some(false), Some(false)],
        "mode 0 rests SCK low (CPOL = ActiveHigh) and MOSI low",
    );

    // Three bytes at 1 Mbps is ~1900 core cycles; leave room regardless.
    run(&mut machine, 40_000);

    let payload = [0xA5u8, 0x3C, 0xFF];
    let bus = &mut machine.bus;
    for (i, &byte) in payload.iter().enumerate() {
        bus.write_u8(RAM + i as u64, byte).unwrap();
    }
    bus.write_u32(SPIM2 + SPIM_TXD_PTR, RAM as u32).unwrap();
    bus.write_u32(SPIM2 + SPIM_TXD_MAXCNT, payload.len() as u32)
        .unwrap();
    bus.write_u32(SPIM2 + SPIM_RXD_PTR, (RAM + 0x100) as u32)
        .unwrap();
    bus.write_u32(SPIM2 + SPIM_RXD_MAXCNT, payload.len() as u32)
        .unwrap();
    bus.write_u32(SPIM2 + SPIM_TASKS_START, 1).unwrap();
    run(&mut machine, 20_000);

    let edges = machine.logic_read_edges(0).edges;
    assert!(
        !edges.is_empty(),
        "a SPIM transfer must put edges on the pads PSEL.SCK/PSEL.MOSI select",
    );
    assert_eq!(
        decode_spi(&edges, CH_SCK, CH_MOSI),
        payload.to_vec(),
        "MOSI sampled on every SCK rising edge, MSB first, must recover exactly \
         the bytes EasyDMA clocked out",
    );

    // 64 MHz core / 1 Mbps (FREQUENCY = M1) = 64 cycles per SCK period.
    const BIT: u64 = 64;
    let rises: Vec<u64> = edges
        .iter()
        .filter(|e| e.ch == CH_SCK && e.value)
        .map(|e| e.cycle)
        .collect();
    assert_eq!(rises.len(), 8 * payload.len(), "eight clocks per byte");
    let gaps: Vec<u64> = rises.windows(2).map(|pair| pair[1] - pair[0]).collect();

    // Seven gaps of one bit period inside each byte, then THREE bit periods
    // across a frame boundary: a frame occupies its eight clocked bits plus one
    // period of chip-select setup/hold and one idle period before the next can
    // begin, so the last rise of one byte and the first of the next are three
    // periods apart. That inter-frame gap is part of the framing, not a rate
    // wobble, and asserting the whole pattern rather than "mostly 64" is what
    // makes a mis-transcribed FREQUENCY table fail here.
    let mut expected: Vec<u64> = Vec::new();
    for byte in 0..payload.len() {
        if byte > 0 {
            expected.push(3 * BIT);
        }
        expected.extend(std::iter::repeat_n(BIT, 7));
    }
    assert_eq!(
        gaps, expected,
        "SCK must run at {BIT} cycles per period (FREQUENCY = M1) with a \
         three-period frame boundary",
    );
}

#[test]
fn the_spim_half_of_the_shared_window_only_claims_pads_while_enable_selects_it() {
    // SPIM0 and TWIM0 are one register file at 0x40003000 and ENABLE picks the
    // personality (PS v1.11 §6.25.6.17 p733 / §6.31.7.18 p798). A claim that
    // ignored ENABLE would let both halves hold the same pad, and whichever
    // published last would win — a waveform from a peripheral firmware had
    // switched away from.
    const SCK: u8 = 13;
    let mut machine = machine();
    let bus = &mut machine.bus;
    // Park the pad as a driven-LOW GPIO, so "reads the wire" and "reads the
    // latch" are opposite levels at every step below. The latch is flipped once
    // mid-test for the same reason: an assertion where both answers agree
    // proves nothing, and this is exactly the pair of states a stale claim
    // would blur.
    bus.write_u32(0x5000_0000 + 0x518, 1 << SCK).unwrap(); // DIRSET
    bus.write_u32(0x5000_0000 + 0x50C, 1 << SCK).unwrap(); // OUTCLR

    // PSEL is programmed while BOTH personalities are disabled — the order
    // nrfx and Zephyr pinctrl use, and the reason the shared window has to
    // shadow a PSEL write to both halves.
    bus.write_u32(SERIAL0 + OFF_PSEL_A, psel(0, u32::from(SCK)))
        .unwrap();

    let gpio = gpio0(&machine);
    assert_eq!(
        machine.logic_watch(&[Some(LogicSource::pad(gpio, SCK))]),
        vec![Some(false)],
        "PSEL written while both personalities are disabled claims nothing, so \
         the pad still reads the GPIO latch firmware drove low",
    );

    machine
        .bus
        .write_u32(SERIAL0 + OFF_ENABLE, ENABLE_TWIM)
        .unwrap();
    assert_eq!(
        machine.logic_watch(&[Some(LogicSource::pad(gpio, SCK))]),
        vec![Some(true)],
        "ENABLE = 6 hands the pad to TWIM, whose SCL idles HIGH against a latch \
         still driven low",
    );

    // Flip the latch HIGH before handing the pad to SPIM, whose SCK idles LOW
    // at CPOL = ActiveHigh. Without this the next assertion would read false
    // whether it saw the wire or the latch.
    machine
        .bus
        .write_u32(0x5000_0000 + 0x508, 1 << SCK)
        .unwrap(); // OUTSET
    machine
        .bus
        .write_u32(SERIAL0 + OFF_ENABLE, ENABLE_SPIM)
        .unwrap();
    assert_eq!(
        machine.logic_watch(&[Some(LogicSource::pad(gpio, SCK))]),
        vec![Some(false)],
        "ENABLE = 7 hands the SAME pad to SPIM, whose SCK idles at CPOL (low) \
         against a latch now driven HIGH — so this can only be the SPIM wire, \
         and TWIM must have released the pad rather than keeping a stale claim",
    );
}

// ── nRF52832 ────────────────────────────────────────────────────────────────

#[test]
fn the_nrf52832_gets_the_same_i2c_waveform_through_its_single_port() {
    // The nRF52832 has ONE GPIO port and no P1 (configs/chips/nrf52832.yaml),
    // so it exercises the port-enumeration half of the wiring pass that the
    // two-port nRF52840 cannot: a `PSEL.PORT` field that can name a port this
    // package does not bond out, and a claim table indexed for two ports while
    // only one is installed.
    //
    // ⚠️ COULD-NOT-VERIFY: no nRF52832 datasheet is in this checkout's
    // datasheet corpus (`labwired_datasheet` holds nrf52840 and nrf54l15 of the
    // Nordics), so the PSEL field layout asserted here is the nRF52840's. That
    // is not an extrapolation across parts so much as a restatement of what
    // this engine ALREADY claims: nrf52832.yaml declares its UART, TWIM, SPIM
    // and GPIO with the `nrf52840_*` model types, so every register offset it
    // answers is already the nRF52840's. Nothing new is asserted about the
    // 52832 here beyond what the shipped chip config already asserts.
    const SCL: u8 = 27;
    const SDA: u8 = 26;

    let mut machine = machine_for("nrf52832");
    assert!(
        machine.bus.find_peripheral_index_by_name("gpio1").is_none(),
        "precondition: the nRF52832 has a single GPIO port — if this ever gains \
         a gpio1 the test below stops covering the one-port case",
    );
    machine
        .bus
        .attach_i2c_slave("i2c0", Box::new(Ramp { next: 0x60 }))
        .expect("attach an I²C slave to the TWIM0 window");
    configure_twim(&mut machine, u32::from(SCL), u32::from(SDA));

    let gpio = gpio0(&machine);
    assert_eq!(
        machine.logic_watch(&[
            Some(LogicSource::pad(gpio, SCL)),
            Some(LogicSource::pad(gpio, SDA))
        ]),
        vec![Some(true), Some(true)],
        "an idle open-drain I²C bus rests high on both pads PSEL selects",
    );
    run(&mut machine, 80_000);

    let bus = &mut machine.bus;
    bus.write_u8(RAM, 0xD0).unwrap();
    bus.write_u32(SERIAL0 + TWIM_TXD_PTR, RAM as u32).unwrap();
    bus.write_u32(SERIAL0 + TWIM_TXD_MAXCNT, 1).unwrap();
    bus.write_u32(SERIAL0 + TWIM_TASKS_STARTTX, 1).unwrap();
    run(&mut machine, 120_000);

    let edges = machine.logic_read_edges(0).edges;
    assert_eq!(
        decode_i2c(&edges, 0, 1),
        vec![(SLAVE_ADDR << 1, true), (0xD0, true)],
        "the nRF52832's TWIM0 must put the addressed write on its pads too",
    );
}
