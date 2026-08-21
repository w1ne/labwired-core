// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The parts of the Espressif command-list I²C master that are the SAME part.
//!
//! The classic ESP32 ([`crate::peripherals::esp32::i2c`]), the ESP32-C3
//! ([`crate::peripherals::esp32c3::i2c`]) and the ESP32-S3
//! ([`crate::peripherals::esp32s3::i2c`]) are three models of one Espressif IP
//! family. Each of them carried its own copy of the same behaviour, so a fix to
//! any of it landed in one copy out of three.
//!
//! # What is shared here, and why it is safe to share
//!
//! * [`EspI2cCore`] — the attached-slave registry, address resolution and the
//!   RX FIFO. Byte-identical in all three models.
//! * [`EspI2cWire`] — the narrated-waveform buffer. Byte-identical in the two
//!   models whose engine completes a command list instantly (classic + S3). The
//!   C3 does NOT use it: its bit engine drives SCL/SDA per cycle, so it has real
//!   edges rather than a narration of them.
//! * The register offsets and bit positions below, which all three parts place
//!   identically (checked against Espressif's `soc/<chip>/include/soc/i2c_reg.h`
//!   for each part).
//!
//! # What is deliberately NOT shared
//!
//! A register map is a claim about hardware, so anything the three parts do not
//! provably agree on stays with the part:
//!
//! | | classic ESP32 | ESP32-C3 | ESP32-S3 |
//! |---|---|---|---|
//! | I2C0 base | `0x3FF5_3000` | `0x6001_3000` | `0x6001_3000` |
//! | interrupt source | 49 | 29 | 42 |
//! | COMD slots | 16 (`0x58..0x94`) | 8 (`0x58..0x74`) | 8 (`0x58..0x74`) |
//! | RSTART / READ / STOP opcode | 0 / 2 / 3 | 6 / 3 / 2 | 6 / 3 / 2 |
//! | CTR reset | `0x0000_0003` | `0x0000_020B` | `0x0000_020B` |
//! | SR bit 0 | ACK_REC, no STRETCH_CAUSE | RESP_REC + BUS_BUSY | RESP_REC |
//! | SCL period field | 14 bits | 9 bits (low) | 9 bits (low) |
//! | TX FIFO | aliased to the AHB window at `0x6001_301c` | APB only | APB only |
//!
//! The classic part's COMD8..COMD15 occupy `0x78..0x94`, which is exactly where
//! the C3 and S3 decode `SCL_ST_TIME_OUT`, `SCL_MAIN_ST_TIME_OUT`,
//! `SCL_SP_CONF` and `SCL_STRETCH_CONF`. One address decoding as two different
//! registers is why only the offsets listed below are stated once.
//!
//! Two further things are measurably duplicated and deliberately left alone,
//! because each is its own claim and wants its own change:
//!
//! * The **C3/S3 timing register file** (16 registers, offsets `0x00`, `0x0C`,
//!   `0x30..0x54`, `0x78..0x84`, `0xF8`) is transcribed twice with the same
//!   reset values and write masks — ~75 lines per part. Merging it is a
//!   register-map merge, not a behaviour extraction.
//! * The **command-list execution engines**. The classic and S3 walk the list to
//!   completion inside the `TRANS_START` write; the C3 runs a cycle-accurate bit
//!   engine. Those are different models of time, and the opcode numbering
//!   differs on top.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Arc;

use crate::peripherals::i2c::I2cDevice;
use crate::peripherals::i2c_waveform::I2cNarrator;
use crate::peripherals::pad_lines::PadLines;

// ---------------------------------------------------------------------------
// Register offsets and bit positions the whole family agrees on.
//
// A constant only belongs here if all three parts place it identically. The
// ones that differ (COMD count, opcodes, reset values, bases, interrupt
// sources) live with the part.
// ---------------------------------------------------------------------------

/// Control register. TRANS_START at bit 5 on every part.
pub const REG_CTR: u64 = 0x04;
/// Status register. The FIFO counts sit at the same bits on every part; what
/// each part composes into bit 0 and bits 14..15 does not, so `status_register`
/// stays with the part.
pub const REG_SR: u64 = 0x08;
/// 7-bit target address in bits [6:0].
pub const REG_SLAVE_ADDR: u64 = 0x10;
/// FIFO pointers / levels.
pub const REG_FIFO_ST: u64 = 0x14;
/// FIFO configuration; the RX/TX reset bits at 12/13 self-clear.
pub const REG_FIFO_CONF: u64 = 0x18;
/// FIFO data port: a write pushes TX, a read pops RX.
pub const REG_DATA: u64 = 0x1C;
pub const REG_INT_RAW: u64 = 0x20;
pub const REG_INT_CLR: u64 = 0x24;
pub const REG_INT_ENA: u64 = 0x28;
pub const REG_INT_ST: u64 = 0x2C;
/// First command slot. How MANY slots follow is part-specific.
pub const REG_CMD0: u64 = 0x58;

/// CTR bit 5: TRANS_START — self-clearing master-transaction trigger.
pub const CTR_TRANS_START_BIT: u32 = 1 << 5;
/// COMD bit 31: command_done. Set when a command finishes executing.
pub const CMD_DONE_BIT: u32 = 1 << 31;

pub const INT_END_DETECT: u32 = 1 << 3;
pub const INT_TRANS_COMPLETE: u32 = 1 << 7;
pub const INT_NACK: u32 = 1 << 10;

/// FIFO_CONF bit 12: RX_FIFO_RST (self-clearing).
pub const FIFO_CONF_RX_RST: u32 = 1 << 12;
/// FIFO_CONF bit 13: TX_FIFO_RST (self-clearing).
pub const FIFO_CONF_TX_RST: u32 = 1 << 13;

/// `SOC_I2C_FIFO_LEN` — 32 bytes on all three parts.
pub const FIFO_CAPACITY: usize = 32;

/// Line order for an ESP I²C controller's [`PadLines`]; the GPIO matrix routes
/// `I2CEXTn_SCL` / `I2CEXTn_SDA` to these indices.
pub const I2C_LINES: &[&str] = &["SCL", "SDA"];
pub const LINE_SCL: usize = 0;
pub const LINE_SDA: usize = 1;

/// A fresh pad-line cell for an ESP I²C controller. Open-drain with pull-ups,
/// so both lines rest high.
pub(crate) fn new_pad_lines() -> Arc<PadLines> {
    Arc::new(PadLines::new(I2C_LINES, &[true, true]))
}

/// Which attached slaves are electrically reachable right now.
///
/// The C3 gates every slave on its GPIO-matrix route: a manifest-backed device
/// only acknowledges while firmware has programmed matching input *and* output
/// matrix entries for the exact pads it is wired to. The classic part and the
/// S3 attach devices without a route, so every slave is always reachable. This
/// is the difference the shared resolver must express rather than pick a side
/// of — a gate baked to "always true" would let a C3 device answer through pads
/// firmware never routed.
pub(crate) enum RouteGate<'a> {
    /// Every attached slave is wired straight to this controller's pads.
    All,
    /// Only the slaves this predicate accepts have a live GPIO-matrix route.
    Matrix(&'a dyn Fn(usize) -> bool),
}

impl RouteGate<'_> {
    fn allows(&self, index: usize) -> bool {
        match self {
            Self::All => true,
            Self::Matrix(reachable) => reachable(index),
        }
    }
}

/// The slave registry and RX FIFO every ESP command-list I²C master has.
///
/// Address resolution is the reason this is one type rather than three copies:
/// it goes through `claims_address`, not `address()`, because a bus switch
/// (TCA9548A) answers for every device behind its enabled channels and a flat
/// `address()` comparison is first-match — four identical sensors on four
/// channels would collapse onto one.
///
/// The TX FIFO is NOT here. The classic part's TX FIFO is shared with the AHB
/// window alias at `0x6001_301c`, which esp-idf writes instead of the APB DATA
/// register, so it is an `Arc<Mutex<..>>` on that part and a plain queue on the
/// other two. That is a real difference in the silicon's bus plumbing, not
/// duplicated code.
pub(crate) struct EspI2cCore {
    slaves: Vec<Box<dyn I2cDevice>>,
    rx_fifo: RefCell<VecDeque<u8>>,
}

impl EspI2cCore {
    pub(crate) fn new() -> Self {
        Self {
            slaves: Vec::new(),
            rx_fifo: RefCell::new(VecDeque::with_capacity(FIFO_CAPACITY)),
        }
    }

    /// Raw slave push — does NOT wrap for tracing. The only production caller is
    /// the bus choke point [`crate::bus::SystemBus::attach_i2c_slave`], which
    /// wraps first. Slaves are matched by address at transaction time; later
    /// additions take precedence on duplicate addresses.
    pub(crate) fn push_slave(&mut self, slave: Box<dyn I2cDevice>) {
        self.slaves.push(slave);
    }

    pub(crate) fn slaves(&self) -> &[Box<dyn I2cDevice>] {
        &self.slaves
    }

    pub(crate) fn slaves_mut(&mut self) -> &mut [Box<dyn I2cDevice>] {
        &mut self.slaves
    }

    pub(crate) fn slave_count(&self) -> usize {
        self.slaves.len()
    }

    /// The selected device, by the index [`Self::find_by_address`] returned.
    pub(crate) fn slave_mut(&mut self, index: usize) -> &mut dyn I2cDevice {
        &mut *self.slaves[index]
    }

    /// Resolve the slave that answers to `address` on a reachable route, and
    /// tell it which address the master selected.
    pub(crate) fn find_by_address(&mut self, address: u8, gate: &RouteGate<'_>) -> Option<usize> {
        let idx = self.slaves.iter().enumerate().find_map(|(idx, slave)| {
            (gate.allows(idx) && slave.claims_address(address)).then_some(idx)
        })?;
        self.slaves[idx].select_address(address);
        Some(idx)
    }

    /// Resolve a slave from the SLAVE_ADDR register, which firmware writes in
    /// either the 7-bit or the 8-bit shifted form. Arduino/ESP-IDF park the
    /// target here and do not push the address byte into the TX FIFO.
    pub(crate) fn find_by_slave_addr_register(
        &mut self,
        slave_addr: u32,
        gate: &RouteGate<'_>,
    ) -> Option<usize> {
        let raw = slave_addr & 0x7FFF;
        if raw <= 0x7F {
            if let Some(idx) = self.find_by_address(raw as u8, gate) {
                return Some(idx);
            }
        }
        let shifted = ((raw >> 1) & 0x7F) as u8;
        self.find_by_address(shifted, gate)
    }

    /// The address byte as it appears on the wire when the address comes from
    /// the `SLAVE_ADDR` register rather than the TX FIFO: 7-bit address in bits
    /// [6:0], shifted up with a write direction bit.
    pub(crate) fn slave_addr_byte(slave_addr: u32) -> u8 {
        ((slave_addr & 0x7F) as u8) << 1
    }

    // -- RX FIFO --------------------------------------------------------------

    /// Bytes currently in the RX FIFO (SR.RXFIFO_CNT).
    pub(crate) fn rx_len(&self) -> usize {
        self.rx_fifo.borrow().len()
    }

    /// The head byte without consuming it — the RXFIFO_START_ADDR window, and
    /// the side-effect-free half of a DATA read.
    pub(crate) fn rx_peek(&self) -> u8 {
        self.rx_fifo.borrow().front().copied().unwrap_or(0)
    }

    /// Pop the head byte. This is the ONLY side effect in an ESP I²C register
    /// file: a DATA read consumes, on silicon and here.
    pub(crate) fn rx_pop(&self) -> u8 {
        self.rx_fifo.borrow_mut().pop_front().unwrap_or(0)
    }

    /// Push a received byte, dropping it once the FIFO is full.
    pub(crate) fn rx_push(&self, byte: u8) {
        let mut rx = self.rx_fifo.borrow_mut();
        if rx.len() < FIFO_CAPACITY {
            rx.push_back(byte);
        }
    }

    pub(crate) fn rx_clear(&self) {
        self.rx_fifo.borrow_mut().clear();
    }

    // -- iteration over attached devices ---------------------------------------

    /// Advance every attached device's own clock. The ESP controllers drive
    /// central I²C time, so this is how a sensor's conversion time passes.
    pub(crate) fn advance_time_us(&mut self, us: u64) {
        if us == 0 {
            return;
        }
        for slave in self.slaves.iter_mut() {
            slave.advance_time_us(us);
        }
    }

    /// Visit every stimulus surface behind this controller.
    ///
    /// `for_each_sim_input`, not `as_sim_input_mut`: a container slave
    /// (TCA9548A mux) exposes the inputs of the devices behind it, which a
    /// single-surface accessor cannot represent.
    pub(crate) fn for_each_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        for slave in self.slaves.iter_mut() {
            if slave.for_each_sim_input(f) {
                return true;
            }
        }
        false
    }

    pub(crate) fn for_each_attached_device(
        &self,
        f: &mut dyn FnMut(crate::inspect::AttachedDeviceRef<'_>),
    ) {
        for dev in &self.slaves {
            crate::inspect::visit_i2c_device(&**dev, f);
        }
    }
}

impl std::fmt::Debug for EspI2cCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EspI2cCore")
            .field("slaves_count", &self.slaves.len())
            .field("rx_len", &self.rx_len())
            .finish()
    }
}

/// The narrated waveform of a command list, for the engines that execute one
/// instantly (classic ESP32 and ESP32-S3).
///
/// Those engines charge no wire time at all: the whole transaction happens
/// inside the `TRANS_START` write. The frames are therefore buffered and
/// published at completion, anchored to END at the present cycle so every stamp
/// lands in the past — where the capture layer keeps it verbatim. See
/// [`crate::peripherals::i2c_waveform`] for what a narrated waveform does and
/// does not model.
///
/// The ESP32-C3 has no `EspI2cWire`: its bit engine drives the pads per cycle,
/// so its edges are the transaction rather than a reconstruction of it.
#[derive(Default)]
pub(crate) struct EspI2cWire {
    /// Wire levels published to matrix-routed SCL/SDA pads, so an analyzer
    /// clipped to this bus measures a real waveform instead of a flat line.
    /// Created lazily at bus wiring time; `None` on any bus where nothing routes
    /// these pads, and then every call below costs one check.
    lines: Option<Arc<PadLines>>,
    /// Frames of the command list currently executing, `(byte, acked)`.
    frames: Vec<(u8, bool)>,
}

impl EspI2cWire {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The shared pad-line cell for this controller, created on first use.
    /// Called at bus wiring time; creating the cell is what turns narration on.
    pub(crate) fn pad_lines_arc(&mut self) -> Arc<PadLines> {
        self.lines.get_or_insert_with(new_pad_lines).clone()
    }

    /// The bound pad lines, or `None` while nothing routes this controller.
    pub(crate) fn lines(&self) -> Option<&PadLines> {
        self.lines.as_deref()
    }

    /// Record a frame this transaction put on the wire. Buffered, not published
    /// — see [`Self::flush`]. Free when no pad routes this controller.
    pub(crate) fn push(&mut self, byte: u8, acked: bool) {
        if self.lines.is_some() {
            self.frames.push((byte, acked));
        }
    }

    /// Undo the last recorded frame.
    ///
    /// The executor can only tell which shape a WRITE has once it has looked at
    /// the first FIFO byte: in the ESP-IDF/Arduino shape that byte is payload
    /// and the address lives in `SLAVE_ADDR`, so a frame provisionally recorded
    /// as the address has to be taken back and re-recorded as data.
    pub(crate) fn pop_last_addr(&mut self) {
        self.frames.pop();
    }

    /// Publish the buffered frames as ONE transaction, `bit_time_cycles` engine
    /// cycles per SCL period.
    ///
    /// A completed command list is narrated as a whole on purpose: the legacy
    /// IDF driver splits one logical transfer across several TRANS_START bursts
    /// joined by END, and narrating each burst separately would put three
    /// STARTs on the trace for one transfer.
    pub(crate) fn flush(&mut self, bit_time_cycles: u64) {
        let Some(lines) = self.lines.clone() else {
            self.frames.clear();
            return;
        };
        if self.frames.is_empty() {
            return;
        }
        let mut wave = I2cNarrator::new(LINE_SCL, LINE_SDA, bit_time_cycles);
        wave.start();
        for &(byte, acked) in &self.frames {
            wave.frame(byte, acked);
        }
        wave.stop();
        self.frames.clear();
        let now = lines.tap_clock().unwrap_or(0);
        // A transaction fired before the run has accumulated its own duration is
        // compressed to fit, not spiked: the bytes survive, the measured rate
        // does not. See `NarrationFit`.
        let _fit = wave.emit_ending_at(&lines, now);
    }
}

impl std::fmt::Debug for EspI2cWire {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EspI2cWire")
            .field("bound", &self.lines.is_some())
            .field("frames", &self.frames.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub answering to exactly one 7-bit address.
    struct Dev {
        address: u8,
    }

    impl I2cDevice for Dev {
        fn address(&self) -> u8 {
            self.address
        }
        fn write(&mut self, _data: u8) {}
        fn read(&mut self) -> u8 {
            0
        }
    }

    fn core_with(addresses: &[u8]) -> EspI2cCore {
        let mut core = EspI2cCore::new();
        for &address in addresses {
            core.push_slave(Box::new(Dev { address }));
        }
        core
    }

    #[test]
    fn rx_fifo_saturates_at_the_soc_fifo_length() {
        let core = EspI2cCore::new();
        for i in 0..(FIFO_CAPACITY + 8) {
            core.rx_push(i as u8);
        }
        assert_eq!(core.rx_len(), FIFO_CAPACITY);
        assert_eq!(core.rx_peek(), 0);
        assert_eq!(core.rx_pop(), 0);
        assert_eq!(core.rx_len(), FIFO_CAPACITY - 1);
        // A peek must not consume; only the DATA read does.
        assert_eq!(core.rx_peek(), 1);
        assert_eq!(core.rx_len(), FIFO_CAPACITY - 1);
    }

    #[test]
    fn an_empty_rx_fifo_reads_zero_rather_than_panicking() {
        let core = EspI2cCore::new();
        assert_eq!(core.rx_peek(), 0);
        assert_eq!(core.rx_pop(), 0);
    }

    #[test]
    fn slave_addr_byte_shifts_the_seven_bit_address_up() {
        assert_eq!(EspI2cCore::slave_addr_byte(0x48), 0x90);
        // Bits above [6:0] are not part of the byte the wire carries.
        assert_eq!(EspI2cCore::slave_addr_byte(0x7F48), 0x90);
    }

    #[test]
    fn slave_addr_register_resolves_in_the_seven_bit_and_the_shifted_form() {
        let mut core = core_with(&[0x48]);
        assert_eq!(
            core.find_by_slave_addr_register(0x48, &RouteGate::All),
            Some(0)
        );
        // The same target written in the 8-bit shifted form firmware also uses.
        assert_eq!(
            core.find_by_slave_addr_register(0x90, &RouteGate::All),
            Some(0)
        );
        assert_eq!(
            core.find_by_slave_addr_register(0x20, &RouteGate::All),
            None
        );
    }

    /// Delete the `gate.allows(idx)` term in `find_by_address` and this fails:
    /// an unrouted C3 device would answer through pads firmware never wired.
    #[test]
    fn an_unreachable_route_refuses_to_answer_its_own_address() {
        let mut core = core_with(&[0x48]);
        let unreachable = |_: usize| false;
        assert_eq!(
            core.find_by_address(0x48, &RouteGate::Matrix(&unreachable)),
            None
        );
        assert_eq!(core.find_by_address(0x48, &RouteGate::All), Some(0));
    }

    /// Resolution is first-match among the slaves that CLAIM the address, and
    /// the gate is applied before the claim, so a gated-out first device does
    /// not shadow a reachable second one at the same address.
    #[test]
    fn the_gate_is_applied_before_the_address_claim() {
        let mut core = core_with(&[0x48, 0x48]);
        let only_second = |idx: usize| idx == 1;
        assert_eq!(
            core.find_by_address(0x48, &RouteGate::Matrix(&only_second)),
            Some(1)
        );
    }

    #[test]
    fn narration_is_free_and_drops_frames_until_a_pad_routes_the_controller() {
        let mut wire = EspI2cWire::new();
        wire.push(0x90, true);
        wire.push(0xAB, true);
        assert!(wire.lines().is_none());
        // Nothing buffered: an unrouted controller pays nothing per frame.
        assert_eq!(
            format!("{wire:?}"),
            "EspI2cWire { bound: false, frames: 0 }"
        );
        let _lines = wire.pad_lines_arc();
        wire.push(0x90, true);
        assert_eq!(format!("{wire:?}"), "EspI2cWire { bound: true, frames: 1 }");
        wire.pop_last_addr();
        assert_eq!(format!("{wire:?}"), "EspI2cWire { bound: true, frames: 0 }");
    }

    #[test]
    fn the_pad_line_cell_is_created_once_and_shared() {
        let mut wire = EspI2cWire::new();
        let first = wire.pad_lines_arc();
        let second = wire.pad_lines_arc();
        assert!(Arc::ptr_eq(&first, &second));
        // Open-drain with pull-ups: both lines rest high.
        assert!(first.level(LINE_SCL));
        assert!(first.level(LINE_SDA));
    }
}
