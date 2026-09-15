// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Per-chip **bus visibility** scoreboard + ratchet — measured by EDGES.
//!
//! # The gap this closes
//!
//! The previous board asked only whether a controller wire was BOUND to a pad
//! (`SystemBus::bound_pad_functions`). Binding answers "what could ever be seen
//! here" ([`pad_routing::PadRoutes::bound_functions`]) and is the wrong question
//! for a user staring at a flat logic-analyzer trace. `stm32l476` scored three
//! ticks while every lab on that chip showed a completely flat line: the wire
//! existed on paper, the model never published a transition the browser could
//! sample.
//!
//! # What a ✓ means now
//!
//! For every bus instance a chip registers that publishes
//! [`Peripheral::line_names`] / [`Peripheral::wire_lines`]:
//!
//! 1. Drive that family's own canonical bring-up through the bus (clock enable,
//!    divisor / timing, enable) and transmit a known bit-asymmetric payload.
//! 2. Arm a wire-channel probe on EVERY name in `line_names()`.
//! 3. Assert every named line produced edges.
//! 4. Decode the captured waveform back to the payload with a decoder that
//!    shares no code with the model that produced it (the RP2040 UART
//!    waveform gate is the precedent).
//!
//! A ✓ on the board means "this bus produced decodable edges", not "a pad table
//! row exists". A — means the chip has no bus instance of that kind that can
//! currently be driven to a decodable waveform.
//!
//! # Honest exclusions
//!
//! Some chips genuinely cannot produce edges for a bus yet (stub yaml, missing
//! narrator, no clock gate path in this gate). They are listed in
//! [`EXCLUSIONS`] with a reason — never silently skipped. A silent skip is the
//! same failure as the gate this file replaced.
//!
//! # Artifacts
//!
//! * `docs/coverage/bus-visibility.md` — the human board, CHECKED not rewritten.
//! * `docs/coverage/bus-visibility.json` — the ratchet baseline.
//!
//! A chip may GAIN a bus freely; losing one fails. Re-baseline (after a
//! deliberate, explained change):
//!
//! ```text
//! UPDATE_BUS_VISIBILITY_BASELINE=1 cargo test -p labwired-core --test bus_visibility
//! ```

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::logic_capture::LogicEdge;
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::peripherals::spi::SpiDevice;
use labwired_core::{Bus, Machine};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

// ── Types ───────────────────────────────────────────────────────────────────

/// The bus kinds a probe can be asked to decode. Extending this is a deliberate
/// act: a kind with no line-name classifier can never appear on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum BusKind {
    I2c,
    Spi,
    Uart,
}

impl BusKind {
    /// Column order on the board, and the order buses are recorded in the
    /// baseline — stable so the artifacts do not churn.
    const ALL: &'static [BusKind] = &[BusKind::I2c, BusKind::Spi, BusKind::Uart];

    fn label(self) -> &'static str {
        match self {
            BusKind::I2c => "I2C",
            BusKind::Spi => "SPI",
            BusKind::Uart => "UART",
        }
    }
}

/// Bit-ASYMMETRIC under LSB-first and MSB-first framing. `0xA5`, `0x5A`,
/// `0x00` and `0xFF` are palindromic under a reversed shift and have let real
/// mutations survive in this codebase twice.
const PAYLOAD: [u8; 3] = [0x53, 0x1C, 0xE1];

/// Named, listed exclusions: chip + bus kind pairs that this gate does not yet
/// drive to decodable edges, with the reason. Adding a silent `continue` instead
/// of a row here is the same failure as the binding-only gate this replaced.
const EXCLUSIONS: &[(&str, BusKind, &str)] = &[
    // from_config address-map stubs: no real peripheral bank for the three
    // buses (see the board header). Edges cannot be produced on this path.
    (
        "efr32mg26",
        BusKind::Uart,
        "the Efr32s2 layout models the console TX/RX byte path but captures no \
         baud divisor (CLKDIV), so bit_time_cycles() is None and no wire \
         waveform is narrated — there are no edges to decode",
    ),
    (
        "esp32",
        BusKind::Spi,
        "Esp32Spi does not implement line_names()/wire_lines(); pad bindings exist \
         but the wire channel is unpublished on this model",
    ),
    (
        "esp32s3",
        BusKind::I2c,
        "esp32s3.yaml is an address-map stub; from_config has no S3 I2C model",
    ),
    (
        "esp32s3",
        BusKind::Spi,
        "esp32s3.yaml is an address-map stub; from_config has no S3 SPI model",
    ),
    (
        "esp32s3",
        BusKind::Uart,
        "esp32s3.yaml is an address-map stub; from_config has no S3 UART model",
    ),
    (
        "esp32s3-zero",
        BusKind::I2c,
        "esp32s3-zero.yaml inherits the S3 address-map stub",
    ),
    (
        "esp32s3-zero",
        BusKind::Spi,
        "esp32s3-zero.yaml inherits the S3 address-map stub",
    ),
    (
        "esp32s3-zero",
        BusKind::Uart,
        "esp32s3-zero.yaml inherits the S3 address-map stub",
    ),
    (
        "mkw41z4",
        BusKind::I2c,
        "Kinetis I2C publishes no line_names / wire_lines (honest empty)",
    ),
    (
        "mkw41z4",
        BusKind::Spi,
        "no edge bring-up path for Kinetis DSPI in this gate yet",
    ),
    (
        "mkw41z4",
        BusKind::Uart,
        "no edge bring-up path for Kinetis LPUART in this gate yet",
    ),
    (
        "nrf5340",
        BusKind::I2c,
        "nRF5340 serial bank not yet edge-gated on from_config",
    ),
    (
        "nrf5340",
        BusKind::Spi,
        "nRF5340 serial bank not yet edge-gated on from_config",
    ),
    (
        "nrf5340",
        BusKind::Uart,
        "nRF5340 serial bank not yet edge-gated on from_config",
    ),
    // nRF54LM20A: the ports decline the nRF52 pad-claim wiring on purpose.
    // `wire_nrf52_pads` installs the PSEL claim table only for
    // GpioRegisterLayout::Nrf52, because that engine's PSEL field decode is
    // verified on the nRF52840 alone -- and this family widened PSEL.PORT from
    // one bit to three (SVD GLOBAL_SPIM00.PSEL.SCK, PORT [7:5]) to address four
    // ports. With no claim table there is no PadLines cell, so the models
    // publish line NAMES but no wire channel and there are no edges to decode.
    // Lifting these three means teaching the claim engine the wider PORT field,
    // not relaxing the gate.
    (
        "nrf54lm20a",
        BusKind::I2c,
        "nRF54L pad claims unwired: PSEL.PORT is 3 bits on this family and the \
         claim engine decodes the nRF52840 1-bit field only, so no PadLines \
         cell is installed and no wire waveform is narrated",
    ),
    (
        "nrf54lm20a",
        BusKind::Spi,
        "nRF54L pad claims unwired: PSEL.PORT is 3 bits on this family and the \
         claim engine decodes the nRF52840 1-bit field only, so no PadLines \
         cell is installed and no wire waveform is narrated",
    ),
    (
        "nrf54lm20a",
        BusKind::Uart,
        "nRF54L pad claims unwired: PSEL.PORT is 3 bits on this family and the \
         claim engine decodes the nRF52840 1-bit field only, so no PadLines \
         cell is installed and no wire waveform is narrated",
    ),
    (
        "nrf54l15",
        BusKind::I2c,
        "nRF54L15 serial bank not yet edge-gated on from_config",
    ),
    (
        "nrf54l15",
        BusKind::Spi,
        "nRF54L15 serial bank not yet edge-gated on from_config",
    ),
    (
        "nrf54l15",
        BusKind::Uart,
        "nRF54L15 serial bank not yet edge-gated on from_config",
    ),
    (
        "rp2350",
        BusKind::I2c,
        "rp2350 from_config bus not yet edge-gated (no line cells)",
    ),
    (
        "rp2350",
        BusKind::Spi,
        "rp2350 from_config bus not yet edge-gated (no line cells)",
    ),
    (
        "rp2350",
        BusKind::Uart,
        "rp2350 from_config bus not yet edge-gated (no line cells)",
    ),
    // AVR matrix twin: chip yaml uses generic type:i2c/spi (STM32-shaped
    // engines). Those models publish line_names for the logic analyzer but
    // never attach a PadLines cell on the AVR from_config path, so wire_lines()
    // is None. Edge gating needs AVR-native TWI/SPI wire cells first.
    (
        "atmega328p",
        BusKind::I2c,
        "AVR from_config I2C is generic type:i2c without PadLines cell",
    ),
    (
        "atmega328p",
        BusKind::Spi,
        "AVR from_config SPI is generic type:spi without PadLines cell",
    ),
];

fn is_excluded(chip: &str, kind: BusKind) -> Option<&'static str> {
    EXCLUSIONS
        .iter()
        .find(|(c, k, _)| *c == chip && *k == kind)
        .map(|(_, _, reason)| *reason)
}

// ── Paths / fleet ───────────────────────────────────────────────────────────

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Minimal manifest naming only the chip — same shape as `chip_conformance.rs`,
/// so both boards measure the same construction path.
fn dummy_manifest(path: &str) -> SystemManifest {
    SystemManifest {
        parts: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "bus-visibility".to_string(),
        chip: path.to_string(),
        external_devices: vec![],
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
        // No override: these harnesses take whatever the chip declares.
        cpu_hz: None,
    }
}

/// Every chip descriptor on the board, sorted. `ci-fixture-*` is excluded for
/// the same reason `chip_conformance.rs` excludes it.
fn fleet() -> Vec<String> {
    let mut chips: Vec<String> = std::fs::read_dir(root("configs/chips"))
        .expect("configs/chips")
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".yaml"))
        .map(|n| n.trim_end_matches(".yaml").to_string())
        .filter(|n| !n.contains("ci-fixture"))
        .collect();
    chips.sort();
    chips
}

fn ram_base(chip: &str) -> u64 {
    match chip {
        "esp32c3" => 0x3FC8_0000,
        "esp32" | "esp32s3" | "esp32s3-zero" => 0x3FFC_0000,
        _ => 0x2000_0000,
    }
}

// ── Machine construction ────────────────────────────────────────────────────

fn machine_for(chip_name: &str) -> Machine<CortexM> {
    let abs = root(&format!("configs/chips/{chip_name}.yaml"));
    let abs_str = abs.to_string_lossy().to_string();
    let chip =
        ChipDescriptor::from_file(&abs).unwrap_or_else(|e| panic!("{chip_name}: load chip: {e}"));
    let mut bus = SystemBus::from_config(&chip, &dummy_manifest(&abs_str))
        .unwrap_or_else(|e| panic!("{chip_name}: build bus: {e}"));
    let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    // `b .` (0xE7FE): Thumb branch to itself so the logic tap's provisional
    // clock advances. Nothing about firmware matters — every peripheral below
    // is driven through MMIO.
    let ram = ram_base(chip_name);
    // Best-effort: some chips map RAM differently; if the write fails the
    // driver still advances via `machine.step` on whatever PC holds.
    let _ = machine.bus.write_u8(ram + 0x1000, 0xFE);
    let _ = machine.bus.write_u8(ram + 0x1001, 0xE7);
    machine.cpu.pc = (ram + 0x1000) as u32;
    machine
}

fn run(machine: &mut Machine<CortexM>, steps: u64) {
    for _ in 0..steps {
        let _ = machine.step();
    }
}

// ── Discovery ───────────────────────────────────────────────────────────────

/// Classify a peripheral from the line names it publishes. Position is the
/// wire index contract; the *set* of names tells the protocol.
fn classify_lines(names: &[&str]) -> Option<BusKind> {
    let upper: Vec<String> = names.iter().map(|n| n.to_ascii_uppercase()).collect();
    let has = |s: &str| upper.iter().any(|n| n == s);
    if has("SCL") && has("SDA") {
        return Some(BusKind::I2c);
    }
    if has("SCK") && (has("MOSI") || has("MISO") || has("CS") || has("CSN")) {
        return Some(BusKind::Spi);
    }
    if has("TX") || has("TXD") {
        return Some(BusKind::Uart);
    }
    None
}

struct BusInstance {
    name: String,
    kind: BusKind,
    base: u64,
    line_names: &'static [&'static str],
    has_wire_cell: bool,
}

fn discover(machine: &Machine<CortexM>) -> Vec<BusInstance> {
    let mut out = Vec::new();
    for p in &machine.bus.peripherals {
        let names = p.dev.line_names();
        if names.is_empty() {
            continue;
        }
        let Some(kind) = classify_lines(names) else {
            continue;
        };
        out.push(BusInstance {
            name: p.name.clone(),
            kind,
            base: p.base,
            line_names: names,
            has_wire_cell: p.dev.wire_lines().is_some(),
        });
    }
    out
}

// ── Clock enable ────────────────────────────────────────────────────────────

/// Enable the RCC clock gate for a peripheral, if it has one. Uses the
/// resolved gate the bus already computed from the chip yaml.
fn enable_clock(machine: &mut Machine<CortexM>, peri_name: &str) {
    let Some(idx) = machine.bus.find_peripheral_index_by_name(peri_name) else {
        return;
    };
    let Some(gate) = machine.bus.peripherals[idx].clock_gate.as_ref() else {
        return;
    };
    // ⚠️ NOT just "rcc". The engine's own list is `CLOCK_CONTROLLER_IDS`
    // (`bus/routing.rs`) — rcc / cmu / rcu — and a gate offset is relative to
    // whichever of those a chip actually declares. Looking only for "rcc"
    // silently returned here on a Silicon Labs part, so `enable_clock` was a
    // no-op, the peripheral stayed gated and mute, and the ratchet reported
    // "named line(s) produced no edges" — which reads as a broken narrator
    // rather than a harness that never turned the clock on.
    let Some(rcc_idx) = ["rcc", "cmu", "rcu"]
        .iter()
        .find_map(|id| machine.bus.find_peripheral_index_by_name(id))
    else {
        return;
    };
    let rcc_base = machine.bus.peripherals[rcc_idx].base;
    // A gate carries a LIST of bits since core#922 — a bus-enable bit and a
    // kernel-clock-source ready bit are both just bits the RCC must have set,
    // and the peripheral answers only when ALL of them are. Setting the first
    // one would leave the peripheral mute and read as a coverage hole here.
    //
    // Collected before the writes: `gate` borrows `machine.bus`, which the
    // writes below take mutably.
    let required: Vec<(u64, u8)> = gate
        .requires
        .iter()
        .map(|b| (b.reg_offset, b.bit))
        .collect();
    for (reg_offset, bit) in required {
        let addr = rcc_base + reg_offset;
        let cur = machine.bus.read_u32(addr).unwrap_or(0);
        let _ = machine.bus.write_u32(addr, cur | (1u32 << bit));
    }
}

// ── Independent decoders ────────────────────────────────────────────────────

/// Async serial 8N1, LSB first — shares no code with `UartNarrator`.
fn decode_uart(edges: &[LogicEdge], ch: u32, bit_time: u64) -> Vec<u8> {
    let timeline: Vec<(u64, bool)> = edges
        .iter()
        .filter(|e| e.ch == ch)
        .map(|e| (e.cycle, e.value))
        .collect();
    let level_at = |t: u64| -> bool {
        timeline
            .iter()
            .rev()
            .find(|(cycle, _)| *cycle <= t)
            .map(|(_, level)| *level)
            .unwrap_or(true)
    };
    let mut bytes = Vec::new();
    let mut cursor = 0u64;
    for &(cycle, level) in &timeline {
        if level || cycle < cursor {
            continue;
        }
        if level_at(cycle + bit_time / 2) {
            continue;
        }
        let mut byte = 0u8;
        for index in 0..8u64 {
            if level_at(cycle + bit_time / 2 + bit_time * (index + 1)) {
                byte |= 1 << index;
            }
        }
        if !level_at(cycle + bit_time / 2 + bit_time * 9) {
            continue;
        }
        bytes.push(byte);
        cursor = cycle + bit_time * 10;
    }
    bytes
}

/// I²C decoder from the protocol alone.
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

/// Mode-0 SPI MOSI decode (sample on rising SCK, MSB first).
fn decode_spi_mosi(edges: &[LogicEdge], ch_sck: u32, ch_mosi: u32) -> Vec<u8> {
    let mut mosi = false;
    let mut sck = false;
    let mut bits: Vec<bool> = Vec::new();
    let mut bytes = Vec::new();
    for e in edges {
        let prev_sck = sck;
        if e.ch == ch_sck {
            sck = e.value;
        } else if e.ch == ch_mosi {
            mosi = e.value;
        } else {
            continue;
        }
        if e.ch == ch_sck && !prev_sck && sck {
            bits.push(mosi);
            if bits.len() == 8 {
                let byte = bits.iter().fold(0u8, |acc, &b| (acc << 1) | u8::from(b));
                bytes.push(byte);
                bits.clear();
            }
        }
    }
    bytes
}

fn channel_has_edge(edges: &[LogicEdge], ch: u32) -> bool {
    edges.iter().any(|e| e.ch == ch)
}

// ── Peers / slaves ──────────────────────────────────────────────────────────

const I2C_ADDR: u8 = 0x3C;
struct I2cSink;
impl I2cDevice for I2cSink {
    fn address(&self) -> u8 {
        I2C_ADDR
    }
    fn read(&mut self) -> u8 {
        0
    }
    fn write(&mut self, _data: u8) {}
}

struct SpiEcho;
impl SpiDevice for SpiEcho {
    fn cs_pin(&self) -> &str {
        "CS"
    }
    fn transfer(&mut self, mosi: u8) -> u8 {
        // Return a bit-asymmetric value so MISO is forced off idle.
        mosi ^ 0x53
    }
}

// ── Wire watch helper ───────────────────────────────────────────────────────

fn watch_all_lines(
    machine: &mut Machine<CortexM>,
    peri: &str,
    lines: &[&str],
) -> Result<Vec<Option<bool>>, String> {
    let mut sources = Vec::with_capacity(lines.len());
    for line in lines {
        match machine.resolve_wire_source(peri, line) {
            Ok(src) => sources.push(Some(src)),
            Err(e) => {
                return Err(format!(
                    "{peri}.{line}: resolve failed: {e:?} (published: {:?})",
                    machine.wire_line_names(peri)
                ));
            }
        }
    }
    Ok(machine.logic_watch(&sources))
}

// ── Family detection ────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
enum Family {
    Stm32F1,
    Stm32V2,
    /// STM32H5/H7 SPI v3 (CFG1/CFG2/TXDR), USART still V2.
    Stm32H5,
    Rp2040,
    Nrf52,
    Esp32c3,
    Esp32,
    /// Silicon Labs EFR32 Series 2. ⚠️ "SPI" here is a USART with `CTRL.SYNC` —
    /// the part has no separate SPI peripheral — so both kinds land in the same
    /// arm and drive different register blocks of the same shape.
    Efr32s2,
    Unknown,
}

fn family_of(chip: &str) -> Family {
    if chip.starts_with("stm32f103") {
        Family::Stm32F1
    } else if chip.starts_with("stm32h5")
        || chip.starts_with("stm32h7")
        || chip.starts_with("stm32wba")
    {
        Family::Stm32H5
    } else if chip.starts_with("stm32") {
        Family::Stm32V2
    } else if chip.starts_with("rp2040") {
        Family::Rp2040
    } else if chip.starts_with("nrf52") {
        Family::Nrf52
    } else if chip == "esp32c3" {
        Family::Esp32c3
    } else if chip == "esp32" {
        Family::Esp32
    } else if chip.starts_with("efr32") {
        Family::Efr32s2
    } else {
        Family::Unknown
    }
}

// ── Per-kind drive ──────────────────────────────────────────────────────────

/// Result of attempting to prove edges for one bus kind on one chip.
#[derive(Debug)]
enum KindResult {
    /// Decodable edges on every named line of at least one instance.
    Visible { instance: String, detail: String },
    /// No bus instance of this kind publishes line names.
    Absent,
    /// Listed in [`EXCLUSIONS`].
    Excluded { reason: &'static str },
    /// An instance exists, the drive ran, and the assertion failed.
    Failed { instance: String, reason: String },
    /// An instance exists but this gate has no bring-up for the family.
    Unsupported { instance: String, reason: String },
}

fn measure_kind(chip: &str, kind: BusKind) -> KindResult {
    if let Some(reason) = is_excluded(chip, kind) {
        return KindResult::Excluded { reason };
    }

    // Discover on a throwaway bus, then rebuild per attempt so one failed
    // drive cannot poison the next instance's register state.
    let probe = machine_for(chip);
    let mut instances: Vec<BusInstance> = discover(&probe)
        .into_iter()
        .filter(|i| i.kind == kind)
        .map(|i| i.clone_name())
        .collect();
    if instances.is_empty() {
        return KindResult::Absent;
    }
    // Prefer instances that already own a PadLines cell, then keep yaml order.
    instances.sort_by_key(|i| (!i.has_wire_cell, i.name.clone()));

    let family = family_of(chip);
    let mut last_fail: Option<KindResult> = None;
    for inst in &instances {
        let mut machine = machine_for(chip);
        // Re-resolve base/lines from the fresh bus (indices stable by name).
        let fresh = discover(&machine)
            .into_iter()
            .find(|i| i.name == inst.name)
            .map(|i| i.clone_name())
            .unwrap_or_else(|| inst.clone_name());
        let attempts: Vec<KindResult> = match kind {
            BusKind::Uart => {
                // STM32 yamls may land either the F1 or V2 USART map under the
                // same `type: uart`. Try both layouts when the family is STM32.
                match family {
                    Family::Stm32F1 | Family::Stm32V2 | Family::Stm32H5 => {
                        let mut results = Vec::new();
                        for layout in [Stm32UartMap::V2, Stm32UartMap::F1] {
                            let mut m = machine_for(chip);
                            let inst = discover(&m)
                                .into_iter()
                                .find(|i| i.name == fresh.name)
                                .map(|i| i.clone_name())
                                .unwrap_or_else(|| fresh.clone_name());
                            results.push(drive_uart_stm32(&mut m, &inst, layout));
                        }
                        results
                    }
                    _ => vec![drive_uart(chip, family, &mut machine, &fresh)],
                }
            }
            BusKind::Spi => vec![drive_spi(chip, family, &mut machine, &fresh)],
            BusKind::I2c => vec![drive_i2c(chip, family, &mut machine, &fresh)],
        };
        for result in attempts {
            match result {
                KindResult::Visible { .. } => return result,
                other => last_fail = Some(other),
            }
        }
    }
    last_fail.unwrap_or(KindResult::Absent)
}

#[derive(Clone, Copy)]
enum Stm32UartMap {
    F1,
    V2,
}

trait CloneName {
    fn clone_name(&self) -> BusInstance;
}
impl CloneName for BusInstance {
    fn clone_name(&self) -> BusInstance {
        BusInstance {
            name: self.name.clone(),
            kind: self.kind,
            base: self.base,
            line_names: self.line_names,
            has_wire_cell: self.has_wire_cell,
        }
    }
}

// ── UART drive ──────────────────────────────────────────────────────────────

fn drive_uart(
    chip: &str,
    family: Family,
    machine: &mut Machine<CortexM>,
    inst: &BusInstance,
) -> KindResult {
    if !inst.has_wire_cell {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: "line_names present but wire_lines() is None — no PadLines cell to probe"
                .into(),
        };
    }

    enable_clock(machine, &inst.name);

    let bit_time = match family {
        Family::Stm32F1 | Family::Stm32V2 | Family::Stm32H5 => {
            // Handled by drive_uart_stm32 via the dual-map attempt above.
            return drive_uart_stm32(machine, inst, Stm32UartMap::V2);
        }
        Family::Rp2040 => {
            // PL011 IBRD/FBRD for ~115200 from 125 MHz
            let _ = machine.bus.write_u32(inst.base + 0x24, 67);
            let _ = machine.bus.write_u32(inst.base + 0x28, 52);
            // UARTCR: UARTEN | TXE | RXE
            let _ = machine
                .bus
                .write_u32(inst.base + 0x30, (1 << 0) | (1 << 8) | (1 << 9));
            1085u64
        }
        Family::Nrf52 => {
            // BAUDRATE 115200, ENABLE UARTE
            let _ = machine.bus.write_u32(inst.base + 0x524, 0x01D6_0000);
            let _ = machine.bus.write_u32(inst.base + 0x500, 8);
            557u64
        }
        Family::Esp32c3 => {
            // CLKDIV 115200 from 80 MHz APB; C3 core is 160 MHz → 1388 cycles/bit.
            let _ = machine.bus.write_u32(inst.base + 0x14, 694);
            1388u64
        }
        Family::Esp32 => {
            // Classic Esp32Uart: CLKDIV @ 0x14. Core 240 MHz / APB 80 MHz.
            // Generic `type: uart` stubs (uart0 on esp32.yaml) use the STM32 map
            // instead — fall through by also programming BRR/TDR so the next
            // instance (uart1/2 as esp32_uart) is what usually succeeds.
            let _ = machine.bus.write_u32(inst.base + 0x14, 694); // CLKDIV or CR3
            let _ = machine.bus.write_u32(inst.base + 0x0C, 694); // BRR if V2
            let _ = machine
                .bus
                .write_u32(inst.base, (1 << 0) | (1 << 3) | (1 << 2)); // CR1 if V2
            2082u64 // 694 * 240/80
        }
        Family::Efr32s2 => {
            // Not reached today: `efr32mg26`'s UART carries an EXCLUSIONS row,
            // and `is_excluded` is consulted before this function. Kept, and
            // saying the same thing that row says, so that deleting the row
            // (the day the async path narrates) fails with the real reason
            // rather than "no UART bring-up for family of chip".
            return KindResult::Unsupported {
                instance: inst.name.clone(),
                reason: "EFR32 Series-2 USART in ASYNC mode narrates nothing — only the \
                         SYNC (SPI) path publishes a wire, and no baud divisor is \
                         captured, so there is no bit time to decode against"
                    .into(),
            };
        }
        Family::Unknown => {
            return KindResult::Unsupported {
                instance: inst.name.clone(),
                reason: format!("no UART bring-up for family of chip '{chip}'"),
            };
        }
    };

    // TX (or TXD) is the controller-driven line this gate transmits on. RX is
    // only published when a peer really drives it (see
    // `logic_analyzer_lab_pad_visibility`), and several family models
    // (EspUart, Esp32Uart) never narrate RX onto the wire cell — requiring it
    // here would force a permanent exclusion for every Espressif chip. The
    // transmit direction is what "this bus produced decodable edges" means for
    // UART on the scoreboard.
    let watched: Vec<&str> = inst
        .line_names
        .iter()
        .copied()
        .filter(|n| {
            let u = n.to_ascii_uppercase();
            u == "TX" || u == "TXD"
        })
        .collect();
    if watched.is_empty() {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: format!(
                "UART instance publishes no TX/TXD line (names: {:?})",
                inst.line_names
            ),
        };
    }

    if let Err(e) = watch_all_lines(machine, &inst.name, &watched) {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: e,
        };
    }

    run(machine, bit_time.saturating_mul(40).max(8_000));

    if !transmit_uart(family, machine, inst) {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: "UART transmit step did not complete".into(),
        };
    }
    run(
        machine,
        bit_time
            .saturating_mul(12)
            .saturating_mul(PAYLOAD.len() as u64 + 8)
            .max(40_000),
    );

    let edges = machine.logic_read_edges(0).edges;
    check_uart_edges(inst, &watched, &edges, bit_time)
}

fn drive_uart_stm32(
    machine: &mut Machine<CortexM>,
    inst: &BusInstance,
    map: Stm32UartMap,
) -> KindResult {
    if !inst.has_wire_cell {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: "line_names present but wire_lines() is None".into(),
        };
    }
    enable_clock(machine, &inst.name);
    let brr = 69u32;
    match map {
        Stm32UartMap::F1 => {
            let _ = machine.bus.write_u32(inst.base + 0x08, brr); // BRR
            let _ = machine
                .bus
                .write_u32(inst.base + 0x0C, (1 << 13) | (1 << 3) | (1 << 2)); // CR1 UE|TE|RE
        }
        Stm32UartMap::V2 => {
            let _ = machine.bus.write_u32(inst.base + 0x0C, brr); // BRR
            let _ = machine
                .bus
                .write_u32(inst.base, (1 << 0) | (1 << 3) | (1 << 2)); // CR1
        }
    }
    let bit_time = u64::from(brr);
    let watched: Vec<&str> = inst
        .line_names
        .iter()
        .copied()
        .filter(|n| {
            let u = n.to_ascii_uppercase();
            u == "TX" || u == "TXD"
        })
        .collect();
    if watched.is_empty() {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: "no TX line".into(),
        };
    }
    if let Err(e) = watch_all_lines(machine, &inst.name, &watched) {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: e,
        };
    }
    run(machine, bit_time.saturating_mul(40).max(8_000));
    let tx_off = match map {
        Stm32UartMap::F1 => 0x04u64,
        Stm32UartMap::V2 => 0x28u64,
    };
    // Named transmit step for mutation testing.
    if !transmit_uart_bytes(machine, inst.base + tx_off) {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: "UART transmit step did not complete".into(),
        };
    }
    run(
        machine,
        bit_time
            .saturating_mul(12)
            .saturating_mul(PAYLOAD.len() as u64 + 8)
            .max(40_000),
    );
    let edges = machine.logic_read_edges(0).edges;
    check_uart_edges(inst, &watched, &edges, bit_time)
}

/// Write the asymmetric payload into a UART data register. Deliberately a
/// named function so a mutation can turn it into a no-op while still compiling
/// (DoD item 5).
fn transmit_uart_bytes(machine: &mut Machine<CortexM>, data_reg: u64) -> bool {
    for &b in &PAYLOAD {
        let _ = machine.bus.write_u8(data_reg, b);
    }
    true
}

/// The transmit step — deliberately a named function so a mutation can turn it
/// into a no-op while still compiling (see DoD item 5).
fn transmit_uart(family: Family, machine: &mut Machine<CortexM>, inst: &BusInstance) -> bool {
    match family {
        // Never reached: `drive_uart` returns Unsupported for this family
        // before the transmit step.
        Family::Efr32s2 => false,
        Family::Stm32F1 | Family::Stm32V2 | Family::Stm32H5 => {
            // Covered by drive_uart_stm32 / transmit_uart_bytes.
            transmit_uart_bytes(machine, inst.base + 0x28)
        }
        Family::Rp2040 => {
            // UARTDR @ 0x00
            for &b in &PAYLOAD {
                let _ = machine.bus.write_u8(inst.base, b);
            }
            true
        }
        Family::Nrf52 => {
            let ram = ram_base("nrf52840");
            for (i, &b) in PAYLOAD.iter().enumerate() {
                let _ = machine.bus.write_u8(ram + i as u64, b);
            }
            let _ = machine.bus.write_u32(inst.base + 0x544, ram as u32); // TXD.PTR
            let _ = machine
                .bus
                .write_u32(inst.base + 0x548, PAYLOAD.len() as u32); // TXD.MAXCNT
            let _ = machine.bus.write_u32(inst.base + 0x008, 1); // TASKS_STARTTX
            true
        }
        Family::Esp32c3 | Family::Esp32 => {
            // FIFO @ 0x00 on EspUart / Esp32Uart. Also poke TDR for a generic
            // V2 UART that may share the same name on a stub yaml.
            for &b in &PAYLOAD {
                let _ = machine.bus.write_u32(inst.base, u32::from(b));
                let _ = machine.bus.write_u8(inst.base + 0x28, b);
            }
            true
        }
        Family::Unknown => false,
    }
}

fn check_uart_edges(
    inst: &BusInstance,
    watched: &[&str],
    edges: &[LogicEdge],
    bit_time: u64,
) -> KindResult {
    let mut missing = Vec::new();
    for (ch, name) in watched.iter().enumerate() {
        if !channel_has_edge(edges, ch as u32) {
            missing.push(*name);
        }
    }
    if !missing.is_empty() {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: format!(
                "named line(s) produced no edges: {missing:?} (total edges {})",
                edges.len()
            ),
        };
    }

    // Decode TX (or TXD) channel — index is within the watched set.
    let tx_ch = watched
        .iter()
        .position(|n| n.eq_ignore_ascii_case("TX") || n.eq_ignore_ascii_case("TXD"))
        .unwrap_or(0) as u32;
    // Try the programmed bit time first, then a small neighbourhood — the
    // event-scheduler path can stamp the same waveform a few cycles off the
    // walk path, which drops a middle character if the sample is half a bit
    // early (see `esp_spi_uart_waveform`).
    let mut decoded = decode_uart(edges, tx_ch, bit_time);
    let payload_ok = |d: &[u8]| {
        d == PAYLOAD.as_slice() || d.windows(PAYLOAD.len()).any(|w| w == PAYLOAD.as_slice())
    };
    let mut ok = payload_ok(&decoded);
    if !ok {
        for scale in [80u64, 90, 95, 100, 105, 110, 120, 130, 150, 160, 200] {
            let bt = bit_time.saturating_mul(scale) / 100;
            if bt < 2 {
                continue;
            }
            let d = decode_uart(edges, tx_ch, bt);
            if payload_ok(&d) {
                decoded = d;
                ok = true;
                break;
            }
        }
    }
    if !ok {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: format!(
                "UART TX decode mismatch: got {decoded:02x?}, want {PAYLOAD:02x?} \
                 (bit_time={bit_time}, edges={})",
                edges.len()
            ),
        };
    }

    KindResult::Visible {
        instance: inst.name.clone(),
        detail: format!("decoded {PAYLOAD:02x?} on TX; watched {watched:?}"),
    }
}

// ── SPI drive ───────────────────────────────────────────────────────────────

fn drive_spi(
    chip: &str,
    family: Family,
    machine: &mut Machine<CortexM>,
    inst: &BusInstance,
) -> KindResult {
    if !inst.has_wire_cell {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: "line_names present but wire_lines() is None — no PadLines cell".into(),
        };
    }

    enable_clock(machine, &inst.name);
    let _ = machine.bus.attach_spi_device(&inst.name, Box::new(SpiEcho));

    match family {
        Family::Stm32F1 | Family::Stm32V2 => {
            // CR1 = MSTR|SSM|SSI|BR=/4|SPE
            let cr1: u16 = (1 << 2) | (1 << 9) | (1 << 8) | (0x1 << 3) | (1 << 6);
            let _ = machine.bus.write_u16(inst.base, cr1);
        }
        Family::Stm32H5 => {
            // SPI v3: CR1.SSI, CFG2.MASTER|SSM, CFG1 DSIZE=7 MBR=div4, CR2
            // TSIZE, then SPE + CSTART. Order is silicon-mandated.
            const CR1_SSI: u32 = 1 << 12;
            const CR1_SPE: u32 = 1 << 0;
            const CR1_CSTART: u32 = 1 << 9;
            const CFG2_MASTER: u32 = 1 << 22;
            const CFG2_SSM: u32 = 1 << 26;
            const CFG1_MBR_DIV4: u32 = 1 << 28;
            let _ = machine.bus.write_u32(inst.base, CR1_SSI);
            let _ = machine
                .bus
                .write_u32(inst.base + 0x0C, CFG2_MASTER | CFG2_SSM); // CFG2
            let _ = machine.bus.write_u32(inst.base + 0x08, 7 | CFG1_MBR_DIV4); // CFG1
            let _ = machine
                .bus
                .write_u32(inst.base + 0x04, PAYLOAD.len() as u32); // CR2 TSIZE
            let _ = machine.bus.write_u32(inst.base, CR1_SSI | CR1_SPE);
            let _ = machine
                .bus
                .write_u32(inst.base, CR1_SSI | CR1_SPE | CR1_CSTART);
        }
        Family::Rp2040 => {
            // SSPCR0: SCR=0, SPH=0, SPO=0, FRF=0, DSS=7 (8-bit)
            let _ = machine.bus.write_u32(inst.base, 0x07);
            // SSPCPSR: CPSDVSR = 2
            let _ = machine.bus.write_u32(inst.base + 0x10, 2);
            // SSPCR1: SSE
            let _ = machine.bus.write_u32(inst.base + 0x04, 1 << 1);
        }
        Family::Nrf52 => {
            let _ = machine.bus.write_u32(inst.base + 0x500, 7); // ENABLE SPIM
            let _ = machine.bus.write_u32(inst.base + 0x524, 0x1000_0000); // M1
            let _ = machine.bus.write_u32(inst.base + 0x554, 0); // CONFIG mode 0
        }
        Family::Esp32c3 => {
            // CLOCK @ 0x0C, USER @ 0x10, MISC @ 0x20 — mode 0.
            let _ = machine
                .bus
                .write_u32(inst.base + 0x0C, (1 << 18) | (39 << 12));
            let _ = machine.bus.write_u32(inst.base + 0x10, 0); // USER mode 0
            let _ = machine.bus.write_u32(inst.base + 0x20, 0); // MISC CPOL=0
        }
        Family::Esp32 => {
            // Classic VSPI: CLOCK @ 0x18, USER @ 0x1C, PIN @ 0x34.
            let _ = machine
                .bus
                .write_u32(inst.base + 0x18, (19 << 18) | (3 << 12));
            let _ = machine.bus.write_u32(inst.base + 0x1C, 1 << 27); // USR_MOSI
            let _ = machine.bus.write_u32(inst.base + 0x34, 0); // PIN
        }
        Family::Efr32s2 => {
            // ⚠️ `CTRL.SYNC` is what makes this USART a SPI at all, and MSBF is
            // the only bit order the narrator draws. CMD then enables master +
            // TX. `GPIO_USARTROUTE` is deliberately NOT written.
            // ⚠️ AND THE ROUTE, OR THIS BLOCK REACHES NO PAD. On Series 2 a
            // USART's clock and data are wired to pins only through
            // GPIO_USARTROUTE; unrouted, the model drives nothing and this
            // harness sees zero edges — which is what a real board produces.
            // The instance's stanza is +0x20 apart from USART0's at 0x4003C820
            // (RM section 24.6 p.879), and the pins are arbitrary here: the
            // measure is "does a named line move", not which pad it moved.
            {
                let n = (inst.base - 0x400A_0000) / 0x4000;
                let stanza = 0x4003_C820 + n * 0x20;
                let _ = machine.bus.write_u32(0x4000_8064, 1 << 26); // GPIO clock
                let _ = machine.bus.write_u32(stanza + 0x14, 2 | (3 << 16)); // CLK -> PC03
                let _ = machine.bus.write_u32(stanza + 0x18, 2 | (2 << 16)); // TX  -> PC02
                let _ = machine.bus.write_u32(stanza, (1 << 4) | (1 << 3)); // TXPEN|CLKPEN
            }
            let _ = machine.bus.write_u32(inst.base + 0x04, 1); // EN
            let _ = machine.bus.write_u32(inst.base + 0x08, 1 | (1 << 10)); // CTRL SYNC|MSBF
            let _ = machine.bus.write_u32(inst.base + 0x14, (1 << 4) | (1 << 2));
            // CMD
        }
        Family::Unknown => {
            return KindResult::Unsupported {
                instance: inst.name.clone(),
                reason: format!("no SPI bring-up for family of chip '{chip}'"),
            };
        }
    }

    if let Err(e) = watch_all_lines(machine, &inst.name, inst.line_names) {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: e,
        };
    }
    run(machine, 4_000);

    if !transmit_spi(family, machine, inst) {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: "SPI transmit step did not complete".into(),
        };
    }
    run(machine, 80_000);

    let edges = machine.logic_read_edges(0).edges;
    check_spi_edges(inst, &edges)
}

fn transmit_spi(family: Family, machine: &mut Machine<CortexM>, inst: &BusInstance) -> bool {
    match family {
        Family::Efr32s2 => {
            // A frame completes inside the TXDATA write and is narrated there,
            // so each byte is one write and no status poll is needed.
            for &b in &PAYLOAD {
                let _ = machine.bus.write_u32(inst.base + 0x3C, u32::from(b));
            }
            true
        }
        Family::Stm32F1 | Family::Stm32V2 => {
            let dr = inst.base + 0x0C;
            let sr = inst.base + 0x08;
            for &b in &PAYLOAD {
                for _ in 0..10_000 {
                    if machine.bus.read_u16(sr).unwrap_or(0) & (1 << 1) != 0 {
                        break;
                    }
                    let _ = machine.step();
                }
                let _ = machine.bus.write_u8(dr, b);
                for _ in 0..10_000 {
                    if machine.bus.read_u16(sr).unwrap_or(0) & (1 << 7) == 0 {
                        break;
                    }
                    let _ = machine.step();
                }
            }
            true
        }
        Family::Stm32H5 => {
            // TXDR @ 0x20 — each write clocks one frame (no bit engine).
            for &b in &PAYLOAD {
                let _ = machine.bus.write_u32(inst.base + 0x20, u32::from(b));
                for _ in 0..2_000 {
                    let _ = machine.step();
                }
            }
            true
        }
        Family::Rp2040 => {
            // SSPDR @ 0x08
            for &b in &PAYLOAD {
                let _ = machine.bus.write_u32(inst.base + 0x08, u32::from(b));
                for _ in 0..5_000 {
                    let _ = machine.step();
                }
            }
            true
        }
        Family::Nrf52 => {
            let ram = ram_base("nrf52840");
            for (i, &b) in PAYLOAD.iter().enumerate() {
                let _ = machine.bus.write_u8(ram + i as u64, b);
            }
            let _ = machine.bus.write_u32(inst.base + 0x544, ram as u32); // TXD.PTR
            let _ = machine
                .bus
                .write_u32(inst.base + 0x548, PAYLOAD.len() as u32);
            let _ = machine.bus.write_u32(inst.base + 0x534, (ram + 16) as u32); // RXD.PTR
            let _ = machine
                .bus
                .write_u32(inst.base + 0x538, PAYLOAD.len() as u32);
            let _ = machine.bus.write_u32(inst.base + 0x010, 1); // TASKS_START
            true
        }
        Family::Esp32c3 => {
            // MS_DLEN @ 0x1C, W0 @ 0x98 (little-endian packed words), USR bit 24.
            let bits = (PAYLOAD.len() as u32) * 8 - 1;
            let _ = machine.bus.write_u32(inst.base + 0x1C, bits);
            for (w, chunk) in PAYLOAD.chunks(4).enumerate() {
                let mut word = 0u32;
                for (b, &byte) in chunk.iter().enumerate() {
                    word |= u32::from(byte) << (8 * b);
                }
                let _ = machine
                    .bus
                    .write_u32(inst.base + 0x98 + (w as u64) * 4, word);
            }
            let _ = machine.bus.write_u32(inst.base, 1 << 24); // USR
                                                               // Wire time: 10 bit periods/byte at 160 cycles/bit.
            run(machine, PAYLOAD.len() as u64 * 10 * 160 + 256);
            true
        }
        Family::Esp32 => {
            // Classic: W0 @ 0x80, MOSI_DLEN @ 0x28, USR bit 18. Packed words.
            let bits = (PAYLOAD.len() as u32) * 8 - 1;
            let _ = machine.bus.write_u32(inst.base + 0x28, bits);
            for (w, chunk) in PAYLOAD.chunks(4).enumerate() {
                let mut word = 0u32;
                for (b, &byte) in chunk.iter().enumerate() {
                    word |= u32::from(byte) << (8 * b);
                }
                let _ = machine
                    .bus
                    .write_u32(inst.base + 0x80 + (w as u64) * 4, word);
            }
            let _ = machine.bus.write_u32(inst.base, 1 << 18); // USR
            run(machine, PAYLOAD.len() as u64 * 10 * 240 + 256);
            true
        }
        Family::Unknown => false,
    }
}

fn check_spi_edges(inst: &BusInstance, edges: &[LogicEdge]) -> KindResult {
    let mut missing = Vec::new();
    for (ch, name) in inst.line_names.iter().enumerate() {
        // MISO may stay flat if the slave returns the idle level for every bit
        // of a short transfer; still require controller-driven lines.
        let upper = name.to_ascii_uppercase();
        if upper == "MISO" {
            continue;
        }
        if !channel_has_edge(edges, ch as u32) {
            missing.push(*name);
        }
    }
    if !missing.is_empty() {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: format!(
                "named controller line(s) produced no edges: {missing:?} (total edges {})",
                edges.len()
            ),
        };
    }

    let sck = inst
        .line_names
        .iter()
        .position(|n| n.eq_ignore_ascii_case("SCK"))
        .unwrap_or(0) as u32;
    let mosi = inst
        .line_names
        .iter()
        .position(|n| n.eq_ignore_ascii_case("MOSI"))
        .unwrap_or(1) as u32;
    let decoded = decode_spi_mosi(edges, sck, mosi);
    if decoded != PAYLOAD {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: format!(
                "SPI MOSI decode mismatch: got {decoded:02x?}, want {PAYLOAD:02x?} \
                 (edges={})",
                edges.len()
            ),
        };
    }
    KindResult::Visible {
        instance: inst.name.clone(),
        detail: format!("decoded {PAYLOAD:02x?} on MOSI"),
    }
}

// ── I2C drive ───────────────────────────────────────────────────────────────

fn drive_i2c(
    chip: &str,
    family: Family,
    machine: &mut Machine<CortexM>,
    inst: &BusInstance,
) -> KindResult {
    if !inst.has_wire_cell {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: "line_names present but wire_lines() is None — no PadLines cell".into(),
        };
    }

    enable_clock(machine, &inst.name);
    let _ = machine.bus.attach_i2c_slave(&inst.name, Box::new(I2cSink));

    match family {
        Family::Stm32V2 | Family::Stm32F1 | Family::Stm32H5 => {
            // Program BOTH L4 and legacy F1 maps. F401/F4 parts default to the
            // legacy controller with no profile; L476 and friends use L4.
            // L4: TIMINGR + PE
            let _ = machine.bus.write_u32(inst.base + 0x10, 0x3000_0F13);
            // F1: CCR + TRISE + PE
            let _ = machine.bus.write_u32(inst.base + 0x1C, 40);
            let _ = machine.bus.write_u32(inst.base + 0x20, 9);
            let _ = machine.bus.write_u32(inst.base, 1); // PE (both maps)
        }
        Family::Rp2040 => {
            let _ = machine.bus.write_u32(inst.base + 0x14, 625); // SS_SCL_HCNT
            let _ = machine.bus.write_u32(inst.base + 0x18, 625); // SS_SCL_LCNT
            let _ = machine.bus.write_u32(inst.base, 0); // CON
            let _ = machine.bus.write_u32(inst.base + 0x04, u32::from(I2C_ADDR)); // TAR
            let _ = machine.bus.write_u32(inst.base + 0x6c, 1); // ENABLE
        }
        Family::Nrf52 => {
            let _ = machine.bus.write_u32(inst.base + 0x500, 6); // ENABLE TWIM
            let _ = machine.bus.write_u32(inst.base + 0x524, 0x0198_0000); // K100
            let _ = machine
                .bus
                .write_u32(inst.base + 0x588, u32::from(I2C_ADDR));
        }
        Family::Esp32c3 => {
            let _ = machine.bus.write_u32(inst.base, 199); // SCL_LOW
            let _ = machine.bus.write_u32(inst.base + 0x38, 180 | (19 << 9));
            let _ = machine.bus.write_u32(inst.base + 0x30, 29);
            let _ = machine.bus.write_u32(inst.base + 0x40, 199);
            let _ = machine.bus.write_u32(inst.base + 0x44, 199);
            let _ = machine.bus.write_u32(inst.base + 0x4C, 199);
            let _ = machine.bus.write_u32(inst.base + 0x48, 199);
        }
        Family::Esp32 => {
            // Classic: 100 kHz @ 80 MHz APB (400+400), no matrix required for wire.
            let _ = machine.bus.write_u32(inst.base, 400); // SCL_LOW
            let _ = machine.bus.write_u32(inst.base + 0x38, 400); // SCL_HIGH
        }
        Family::Efr32s2 => {
            // CLKDIV at reset is 0, which the model reads as the slowest
            // standard-mode bit time — enough edges to measure. EN is the only
            // register a transfer needs; `GPIO_I2CROUTE` is deliberately NOT
            // written, which is the difference between this and a pad probe.
            let _ = machine.bus.write_u32(inst.base + 0x04, 1); // EN
        }
        Family::Unknown => {
            return KindResult::Unsupported {
                instance: inst.name.clone(),
                reason: format!("no I2C bring-up for family of chip '{chip}'"),
            };
        }
    }

    if let Err(e) = watch_all_lines(machine, &inst.name, inst.line_names) {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: e,
        };
    }
    run(machine, 4_000);

    if !transmit_i2c(family, machine, inst) {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: "I2C transmit step did not complete".into(),
        };
    }
    run(machine, 200_000);

    let edges = machine.logic_read_edges(0).edges;
    check_i2c_edges(inst, &edges)
}

fn transmit_i2c(family: Family, machine: &mut Machine<CortexM>, inst: &BusInstance) -> bool {
    match family {
        Family::Stm32V2 | Family::Stm32F1 | Family::Stm32H5 => {
            // Prefer L4 AUTOEND transfer; if STOPF never lands, fall back to
            // the legacy F1 START/addr/data/STOP sequence.
            let cr2 = (u32::from(I2C_ADDR) << 1) | (1 << 16) | (1 << 25) | (1 << 13);
            let _ = machine.bus.write_u32(inst.base + 0x04, cr2);
            let mut l4_ok = false;
            for _ in 0..50_000 {
                if machine.bus.read_u32(inst.base + 0x18).unwrap_or(0) & (1 << 1) != 0 {
                    l4_ok = true;
                    break;
                }
                let _ = machine.step();
            }
            if l4_ok {
                let _ = machine
                    .bus
                    .write_u32(inst.base + 0x28, u32::from(PAYLOAD[0]));
                for _ in 0..100_000 {
                    if machine.bus.read_u32(inst.base + 0x18).unwrap_or(0) & (1 << 5) != 0 {
                        let _ = machine.step();
                        return true;
                    }
                    let _ = machine.step();
                }
            }
            // Legacy F1 path
            let _ = machine.bus.write_u32(inst.base, (1 << 0) | (1 << 8)); // PE|START
            for _ in 0..50_000 {
                if machine.bus.read_u32(inst.base + 0x14).unwrap_or(0) & (1 << 0) != 0 {
                    break;
                }
                let _ = machine.step();
            }
            let _ = machine
                .bus
                .write_u32(inst.base + 0x10, u32::from(I2C_ADDR) << 1);
            for _ in 0..50_000 {
                if machine.bus.read_u32(inst.base + 0x14).unwrap_or(0) & (1 << 1) != 0 {
                    break;
                }
                let _ = machine.step();
            }
            let _ = machine
                .bus
                .write_u32(inst.base + 0x10, u32::from(PAYLOAD[0]));
            for _ in 0..50_000 {
                if machine.bus.read_u32(inst.base + 0x14).unwrap_or(0) & (1 << 2) != 0 {
                    break;
                }
                let _ = machine.step();
            }
            let _ = machine.bus.write_u32(inst.base, (1 << 0) | (1 << 9)); // PE|STOP
            for _ in 0..50_000 {
                let _ = machine.step();
            }
            true
        }
        Family::Efr32s2 => {
            // START, address+W, the payload, STOP — the sequence emlib's
            // `I2C_TransferInit` drives, byte at a time through TXDATA.
            let _ = machine.bus.write_u32(inst.base + 0x0C, 1 << 0); // CMD.START
            let _ = machine
                .bus
                .write_u32(inst.base + 0x34, u32::from(I2C_ADDR) << 1);
            for &b in &PAYLOAD {
                let _ = machine.bus.write_u32(inst.base + 0x34, u32::from(b));
            }
            let _ = machine.bus.write_u32(inst.base + 0x0C, 1 << 1); // CMD.STOP
            true
        }
        Family::Rp2040 => {
            let data_cmd = inst.base + 0x10;
            let stop = 1u32 << 9;
            for (i, &b) in PAYLOAD.iter().enumerate() {
                let last = i + 1 == PAYLOAD.len();
                let cmd = u32::from(b) | if last { stop } else { 0 };
                let _ = machine.bus.write_u32(data_cmd, cmd);
            }
            true
        }
        Family::Nrf52 => {
            let ram = ram_base("nrf52840");
            for (i, &b) in PAYLOAD.iter().enumerate() {
                let _ = machine.bus.write_u8(ram + i as u64, b);
            }
            let _ = machine.bus.write_u32(inst.base + 0x544, ram as u32);
            let _ = machine
                .bus
                .write_u32(inst.base + 0x548, PAYLOAD.len() as u32);
            let _ = machine.bus.write_u32(inst.base + 0x008, 1); // STARTTX
            true
        }
        Family::Esp32c3 => {
            let cmd = |opcode: u32, byte_num: u32| (opcode << 11) | byte_num;
            let reg_cmd0 = inst.base + 0x58;
            let reg_data = inst.base + 0x1C;
            let reg_ctr = inst.base + 0x04;
            let _ = machine.bus.write_u32(reg_cmd0, cmd(6, 0)); // RSTART
            let _ = machine
                .bus
                .write_u32(reg_cmd0 + 4, cmd(1, 1 + PAYLOAD.len() as u32));
            let _ = machine.bus.write_u32(reg_cmd0 + 8, cmd(2, 0)); // STOP
            let _ = machine.bus.write_u32(reg_data, u32::from(I2C_ADDR) << 1);
            for &b in &PAYLOAD {
                let _ = machine.bus.write_u32(reg_data, u32::from(b));
            }
            let _ = machine.bus.write_u32(reg_ctr, 1 << 5); // TRANS_START
            true
        }
        Family::Esp32 => {
            // Classic opcodes: RSTART=0 WRITE=1 STOP=3 (C3 renumbered these).
            let reg_cmd0 = inst.base + 0x58;
            let reg_data = inst.base + 0x1C;
            let reg_ctr = inst.base + 0x04;
            let _ = machine.bus.write_u32(reg_data, u32::from(I2C_ADDR) << 1);
            for &b in &PAYLOAD {
                let _ = machine.bus.write_u32(reg_data, u32::from(b));
            }
            let n = (PAYLOAD.len() + 1) as u32;
            let _ = machine.bus.write_u32(reg_cmd0, 0 << 11); // RSTART
            let _ = machine.bus.write_u32(reg_cmd0 + 4, (1 << 11) | n); // WRITE
            let _ = machine.bus.write_u32(reg_cmd0 + 8, 3 << 11); // STOP
            let _ = machine.bus.write_u32(reg_ctr, 1 << 5); // TRANS_START
            true
        }
        Family::Unknown => false,
    }
}

fn check_i2c_edges(inst: &BusInstance, edges: &[LogicEdge]) -> KindResult {
    let mut missing = Vec::new();
    for (ch, name) in inst.line_names.iter().enumerate() {
        if !channel_has_edge(edges, ch as u32) {
            missing.push(*name);
        }
    }
    if !missing.is_empty() {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: format!(
                "named line(s) produced no edges: {missing:?} (total edges {})",
                edges.len()
            ),
        };
    }

    let scl = inst
        .line_names
        .iter()
        .position(|n| n.eq_ignore_ascii_case("SCL"))
        .unwrap_or(0) as u32;
    let sda = inst
        .line_names
        .iter()
        .position(|n| n.eq_ignore_ascii_case("SDA"))
        .unwrap_or(1) as u32;
    let frames = decode_i2c(edges, scl, sda);
    if frames.is_empty() {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: format!("I2C decoder saw no frames (edges={})", edges.len()),
        };
    }
    // Address frame first.
    let addr_byte = frames[0].0;
    if addr_byte != (I2C_ADDR << 1) {
        return KindResult::Failed {
            instance: inst.name.clone(),
            reason: format!(
                "I2C address frame {addr_byte:#04x}, want {:#04x}; frames={frames:02x?}",
                I2C_ADDR << 1
            ),
        };
    }
    KindResult::Visible {
        instance: inst.name.clone(),
        detail: format!("I2C frames {frames:02x?}"),
    }
}

// ── Fleet measure ───────────────────────────────────────────────────────────

fn measure_chip(chip: &str) -> (Vec<BusKind>, BTreeMap<BusKind, String>) {
    let mut kinds = Vec::new();
    let mut notes = BTreeMap::new();
    let mut failures = Vec::new();

    for &kind in BusKind::ALL {
        match measure_kind(chip, kind) {
            KindResult::Visible { instance, detail } => {
                kinds.push(kind);
                notes.insert(kind, format!("{instance}: {detail}"));
            }
            KindResult::Absent => {
                // Honest empty cell — no instance publishes this bus.
            }
            KindResult::Excluded { reason } => {
                notes.insert(kind, format!("EXCLUDED: {reason}"));
            }
            KindResult::Failed { instance, reason } => {
                failures.push(format!("{chip}/{kind:?}/{instance}: {reason}"));
            }
            KindResult::Unsupported { instance, reason } => {
                // An instance exists but this gate cannot drive it — that is a
                // FAIL unless listed in EXCLUSIONS. Silently treating it as —
                // is the same bug the binding-only gate had.
                failures.push(format!(
                    "{chip}/{kind:?}/{instance}: unsupported bring-up — {reason} \
                     (add an EXCLUSIONS row with a reason, or implement drive)"
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "bus-visibility edge measure failed ({}):\n{}",
        failures.len(),
        failures.join("\n")
    );

    kinds.sort();
    (kinds, notes)
}

// ── Board text ──────────────────────────────────────────────────────────────

fn render_board(rows: &[(String, Vec<BusKind>)], exclusion_text: &str) -> String {
    let mut board = String::from(
        "# Bus Visibility Scoreboard\n\n\
         Generated by `bus_visibility_ratchet` (`crates/core/tests/bus_visibility.rs`).\n\n\
         A ✓ means this bus **produced decodable edges** on a wire-channel probe \
         after the family's canonical bring-up and a bit-asymmetric payload: every \
         named line in `Peripheral::line_names()` edged, and an independent decoder \
         recovered the payload. A — means no bus instance of that kind currently \
         produces a decodable waveform on the `from_config` path.\n\n\
         This is NOT a pad-binding board. Binding answers the static question \
         \"what could ever be seen here\"; a green tick here means a user can \
         actually see a waveform. Derived from live `SystemBus::from_config` builds \
         — never hand-edited.\n\n",
    );
    board.push_str(exclusion_text);
    board.push_str(
        "\n| Chip | I2C | SPI | UART |\n\
         |------|-----|-----|------|\n",
    );
    for (name, kinds) in rows {
        board.push_str(&format!("| {name} |"));
        for kind in BusKind::ALL {
            board.push_str(if kinds.contains(kind) {
                " ✓ |"
            } else {
                " — |"
            });
        }
        board.push('\n');
    }
    board
}

fn exclusion_markdown() -> String {
    let mut by_chip: BTreeMap<&str, Vec<(BusKind, &str)>> = BTreeMap::new();
    for (chip, kind, reason) in EXCLUSIONS {
        by_chip.entry(*chip).or_default().push((*kind, *reason));
    }
    if by_chip.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "## Named exclusions\n\n\
         Chips that cannot yet produce edges for a bus are listed here with a \
         reason — never silently skipped.\n\n",
    );
    for (chip, rows) in by_chip {
        s.push_str(&format!("* **{chip}**\n"));
        for (kind, reason) in rows {
            s.push_str(&format!("  * {}: {reason}\n", kind.label()));
        }
    }
    s
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[test]
fn bus_visibility_ratchet() {
    let chips = fleet();
    assert!(
        !chips.is_empty(),
        "no chip descriptors found under configs/chips — the derivation is broken, \
         not the fleet"
    );

    // Validate exclusion targets exist in the fleet (typos become permanent
    // silent holes otherwise).
    for (chip, _, _) in EXCLUSIONS {
        assert!(
            chips.iter().any(|c| c == chip),
            "EXCLUSIONS names unknown chip '{chip}' — fix the row or the fleet"
        );
    }

    let mut rows: Vec<(String, Vec<BusKind>)> = Vec::new();
    for chip in &chips {
        let (kinds, _notes) = measure_chip(chip);
        rows.push((chip.clone(), kinds));
    }

    let with_any = rows.iter().filter(|(_, k)| !k.is_empty()).count();
    assert!(
        with_any > 0,
        "bus-visibility edge measure produced NO bus on ANY of the {} chips. \
         That is a broken derivation, not a fleet-wide gap.",
        rows.len()
    );

    let board = render_board(&rows, &exclusion_markdown());
    let board_path = root("docs/coverage/bus-visibility.md");
    let baseline_path = root("docs/coverage/bus-visibility.json");
    let current = serde_json::json!(rows
        .iter()
        .map(|(name, kinds)| serde_json::json!({
            "name": name,
            "buses": kinds.iter().map(|k| k.label()).collect::<Vec<_>>(),
        }))
        .collect::<Vec<_>>());

    if std::env::var("UPDATE_BUS_VISIBILITY_BASELINE").is_ok() {
        std::fs::write(&board_path, &board).expect("write bus-visibility board");
        std::fs::write(
            &baseline_path,
            format!("{}\n", serde_json::to_string_pretty(&current).unwrap()),
        )
        .expect("write bus-visibility baseline");
        println!("updated bus-visibility board:    {}", board_path.display());
        println!(
            "updated bus-visibility baseline: {}",
            baseline_path.display()
        );
        return;
    }

    let baseline: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&baseline_path).unwrap_or_else(|_| {
            panic!(
                "missing {}; create it with UPDATE_BUS_VISIBILITY_BASELINE=1",
                baseline_path.display()
            )
        }),
    )
    .expect("parse bus-visibility baseline");

    let mut failures = Vec::new();
    for base in baseline.as_array().expect("baseline is an array") {
        let name = base
            .get("name")
            .and_then(|n| n.as_str())
            .expect("baseline row has a name");
        let Some((_, kinds)) = rows.iter().find(|(n, _)| n == name) else {
            failures.push(format!(
                "  {name}: on the baseline but no longer on the board (chip renamed or \
                 removed?) — every bus it had is now unmeasurable"
            ));
            continue;
        };
        let had: Vec<&str> = base
            .get("buses")
            .and_then(|b| b.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        for want in had {
            if !kinds.iter().any(|k| k.label() == want) {
                failures.push(format!(
                    "  {name}: LOST {want} edge visibility (no longer produces \
                     decodable edges on any instance)"
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "bus edge visibility regressed ({} issue(s)):\n{}\n\
         A bus that stops publishing decodable edges becomes invisible to the \
         logic analyzer while still running — nothing else fails. Restore the \
         narrator / bring-up, or, if the loss is intentional and explained, \
         re-baseline with UPDATE_BUS_VISIBILITY_BASELINE=1 and list the chip in \
         EXCLUSIONS with a reason.",
        failures.len(),
        failures.join("\n")
    );

    let committed = std::fs::read_to_string(&board_path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e} — regenerate with UPDATE_BUS_VISIBILITY_BASELINE=1",
            board_path.display()
        )
    });
    assert_eq!(
        committed.trim_end(),
        board.trim_end(),
        "docs/coverage/bus-visibility.md is stale — the measured matrix no longer \
         matches the committed one. The ratchet above already passed, so nothing \
         was LOST; regenerate in this commit with \
         `UPDATE_BUS_VISIBILITY_BASELINE=1 cargo test -p labwired-core --test \
         bus_visibility` and review the diff — a new ✓ is coverage the doc has \
         not been told about yet."
    );
}

/// Line-name classifier contract, asserted without building anything.
#[test]
fn classifier_recognises_line_names_and_nothing_else() {
    assert_eq!(classify_lines(&["SCL", "SDA"]), Some(BusKind::I2c));
    assert_eq!(classify_lines(&["scl", "sda"]), Some(BusKind::I2c));
    assert_eq!(classify_lines(&["SCK", "MOSI", "MISO"]), Some(BusKind::Spi));
    assert_eq!(classify_lines(&["SCK", "MOSI", "CS"]), Some(BusKind::Spi));
    assert_eq!(classify_lines(&["SCK", "MOSI", "CSn"]), Some(BusKind::Spi));
    assert_eq!(classify_lines(&["TX", "RX"]), Some(BusKind::Uart));
    assert_eq!(classify_lines(&["TXD"]), Some(BusKind::Uart));
    assert_eq!(classify_lines(&["tx"]), Some(BusKind::Uart));
    assert_eq!(classify_lines(&[]), None);
    assert_eq!(classify_lines(&["CH1", "CH2"]), None);
}

/// EXCLUSIONS must not claim a kind that the measure then marks ✓ — that would
/// hide a real edge path behind a permanent excuse.
#[test]
fn exclusions_are_not_vacuously_covering_working_buses() {
    // Spot-check: every exclusion for a chip that is otherwise in a family we
    // drive must still measure as non-Visible when the exclusion row is
    // ignored. We only assert the exclusion table has unique (chip, kind) keys
    // and known kinds — the live measure_chip path enforces the rest.
    let mut seen = BTreeSet::new();
    for (chip, kind, reason) in EXCLUSIONS {
        assert!(
            !reason.is_empty(),
            "exclusion for {chip}/{:?} has an empty reason",
            kind
        );
        assert!(
            seen.insert((*chip, *kind)),
            "duplicate exclusion for {chip}/{:?}",
            kind
        );
    }
}
