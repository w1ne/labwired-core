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
/// so both lines rest high. On a world `gpio_net` both lines are open drain
/// (released while high) from the moment they join it, before the net
/// engine ([`crate::peripherals::esp_i2c_net`]) has said anything.
pub(crate) fn new_pad_lines() -> Arc<PadLines> {
    let lines = PadLines::new(I2C_LINES, &[true, true]);
    for line in [LINE_SCL, LINE_SDA] {
        lines.set_net_idle(line, crate::peripherals::pad_lines::LineDrive::OpenDrain);
    }
    Arc::new(lines)
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

    /// The whole RX FIFO, head first. For differentials that must compare two
    /// controllers' state rather than just what the next DATA read returns —
    /// the C3's per-cycle-walk vs event-scheduler equivalence check reads it,
    /// which is the only caller and why this is gated to that feature.
    #[cfg(all(test, feature = "event-scheduler"))]
    pub(crate) fn rx_bytes(&self) -> Vec<u8> {
        self.rx_fifo.borrow().iter().copied().collect()
    }

    // -- iteration over attached devices ---------------------------------------

    /// Advance every attached device's own clock. The ESP controllers drive
    /// central I²C time, so this is how a sensor's conversion time passes.
    /// Tier 2: collect every attached slave's queued pin drives.
    pub(crate) fn drain_pin_drives(&mut self, out: &mut Vec<(String, String, bool)>) {
        for slave in self.slaves.iter_mut() {
            crate::peripherals::device::drain_i2c_pin_drives(&mut **slave, out);
        }
    }

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

    /// The shared cell itself, once created.
    pub(crate) fn lines_arc(&self) -> Option<Arc<PadLines>> {
        self.lines.clone()
    }

    /// `true` while the GPIO matrix routes SCL or SDA to a pad on a world
    /// `gpio_net`: the net engine owns the wire, and nothing is narrated.
    pub(crate) fn on_net(&self) -> bool {
        self.lines.as_deref().is_some_and(PadLines::any_on_net)
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

// ---------------------------------------------------------------------------
// The shared per-chip test suite.
//
// The classic ESP32, the ESP32-S3 and the ESP32-C3 are three models of ONE
// Espressif command-list I2C IP. Each chip's `mod tests` used to carry its own
// copy of the same suite: 15 shared test names and 7 shared mux helpers. This
// macro writes the suite once; each chip's `mod tests` invokes it with the few
// per-chip facts, plus optional fragments for the tests whose bodies are
// strict supersets on one chip.
//
// Everything unqualified (`REG_CTR`, `cmd`, `CMD_RSTART`, `INT_NACK`, ...) is
// deliberately resolved at the INVOCATION site: `use super::*` and the module's
// own constants give each chip its own values (opcode numbering, ack bit name,
// the last command slot), so the macro never restates them.
//
// Invocation contract (required unless marked optional):
//   ctrl:                 the controller type
//   p_name:               the test-local controller binding the `int_st`
//                         fragments refer to (`p` on every chip); passed
//                         as an ident so the fragments can see it across
//                         macro hygiene
//   launch:               local `fn(&mut ctrl)` kick + clock-to-completion
//                         (one synchronous write on classic/S3, the bit engine
//                         on the C3)
//   ack_bit:              `SR_ACK_REC` (classic) / `SR_RESP_REC` (C3, S3)
//   cmd_last:             `REG_CMD15` (classic) / `REG_CMD7` (C3, S3)
//   cmd_slots_test:       `has_sixteen_command_slots` / `cmd_registers_round_trip`
//   fifo_reset_test:      `fifo_reset_bit_clears_tx_fifo` / `fifo_reset_bits_clear_fifos`
//   bmp280_test:          optional - emit `write_read_drives_attached_bmp280`
//   mux_slave_addr_test:  optional - emit the standard SLAVE_ADDR-path test
//                         inside `mod mux` (classic + S3 engines)
//   mux_extra:            optional - extra items appended inside `mod mux`
//                         (the C3 passes its own SLAVE_ADDR-path test here)
//   int_st_pre/int_st_post: optional - extra statements around the shared
//                         `int_st_masks_with_int_ena` body (the C3's legacy-
//                         tick membership checks)
#[cfg(test)]
macro_rules! esp_i2c_family_suite {
    (
        ctrl: $ctrl:ty,
        p_name: $p:ident,
        launch: $launch:ident,
        ack_bit: $ack_bit:ident,
        cmd_last: $cmd_last:expr,
        cmd_slots_test: $cmd_slots_test:ident,
        fifo_reset_test: $fifo_reset_test:ident,
        $( bmp280_test: $bmp280_test:ident, )?
        $( mux_slave_addr_test: $mux_slave_addr_test:ident, )?
        $( mux_extra: { $($mux_extra:tt)* }, )?
        $( int_st_pre: { $($int_st_pre:tt)* }, )?
        $( int_st_post: { $($int_st_post:tt)* }, )?
    ) => {
        // ── shared register-level suite ─────────────────────────────────

        #[test]
        fn ctr_round_trip() {
            let mut p = <$ctrl>::new();
            p.write_u32(REG_CTR, 0x0000_0010).unwrap(); // arbitrary, no TRANS_START
            assert_eq!(p.read_u32(REG_CTR).unwrap(), 0x0000_0010);
        }

        #[test]
        fn slave_addr_round_trip() {
            let mut p = <$ctrl>::new();
            p.write_u32(REG_SLAVE_ADDR, 0x48).unwrap();
            assert_eq!(p.read_u32(REG_SLAVE_ADDR).unwrap(), 0x48);
        }

        #[test]
        fn sr_txfifo_cnt_reflects_pushes() {
            // esp-hal reads TX/RX FIFO counts from SR.txfifo_cnt (bits 18..23)
            // and SR.rxfifo_cnt (bits 8..13), not from FIFO_ST.
            let mut p = <$ctrl>::new();
            p.write_u32(REG_DATA, 0xAA).unwrap();
            p.write_u32(REG_DATA, 0xBB).unwrap();
            p.write_u32(REG_DATA, 0xCC).unwrap();
            let sr = p.read_u32(REG_SR).unwrap();
            assert_eq!(
                (sr >> 18) & 0x3F,
                3,
                "SR.txfifo_cnt should reflect 3 pushes"
            );
        }

        #[test]
        fn $fifo_reset_test() {
            let mut p = <$ctrl>::new();
            p.write_u32(REG_DATA, 0x11).unwrap();
            p.write_u32(REG_DATA, 0x22).unwrap();
            p.write_u32(REG_FIFO_CONF, 1 << 13).unwrap(); // TX_FIFO_RST
            let sr = p.read_u32(REG_SR).unwrap();
            assert_eq!((sr >> 18) & 0x3F, 0);
        }

        #[test]
        fn int_clr_clears_specified_bits() {
            let mut p = <$ctrl>::new();
            p.int_raw = INT_TRANS_COMPLETE | INT_NACK;
            p.write_u32(REG_INT_CLR, INT_NACK).unwrap();
            assert_eq!(p.read_u32(REG_INT_RAW).unwrap(), INT_TRANS_COMPLETE);
        }

        #[test]
        fn int_st_masks_with_int_ena() {
            let mut $p = <$ctrl>::new();
            $p.int_raw = INT_TRANS_COMPLETE | INT_NACK;
            $( $($int_st_pre)* )?
            $p.write_u32(REG_INT_ENA, INT_TRANS_COMPLETE).unwrap();
            assert_eq!($p.read_u32(REG_INT_ST).unwrap(), INT_TRANS_COMPLETE);
            $( $($int_st_post)* )?
        }

        #[test]
        fn end_opcode_raises_end_detect_not_trans_complete() {
            // The END opcode pauses the controller and raises END_DETECT
            // (bit 3); STOP is what raises TRANS_COMPLETE.
            let mut p = <$ctrl>::new();
            p.write_u32(REG_CMD0, cmd(CMD_END, 0)).unwrap();
            $launch(&mut p);
            let int_raw = p.read_u32(REG_INT_RAW).unwrap();
            assert_eq!(
                int_raw & INT_END_DETECT,
                INT_END_DETECT,
                "END must raise END_DETECT"
            );
            assert_eq!(
                int_raw & INT_TRANS_COMPLETE,
                0,
                "END must NOT raise TRANS_COMPLETE"
            );
        }

        #[test]
        fn rstart_then_stop_completes() {
            let mut p = <$ctrl>::new();
            p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
            p.write_u32(REG_CMD0 + 4, cmd(CMD_STOP, 0)).unwrap();
            $launch(&mut p);
            assert_eq!(
                p.read_u32(REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE,
                INT_TRANS_COMPLETE
            );
        }

        #[test]
        fn trans_start_auto_clears() {
            let mut p = <$ctrl>::new();
            p.write_u32(REG_CMD0, cmd(CMD_END, 0)).unwrap();
            $launch(&mut p);
            assert_eq!(p.read_u32(REG_CTR).unwrap() & CTR_TRANS_START_BIT, 0);
        }

        #[test]
        fn write_with_unmatched_address_sets_nack_int() {
            let mut p = <$ctrl>::new();
            // No slaves attached.
            p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
            p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 1)).unwrap();
            p.write_u32(REG_CMD0 + 8, cmd(CMD_STOP, 0)).unwrap();
            p.write_u32(REG_DATA, 0xA0).unwrap(); // some addr+W, no slave
            $launch(&mut p);
            assert_eq!(
                p.read_u32(REG_INT_RAW).unwrap() & INT_NACK,
                INT_NACK,
                "INT_NACK should fire when no slave matches"
            );
        }

        #[test]
        fn $cmd_slots_test() {
            let mut p = <$ctrl>::new();
            p.write_u32(REG_CMD0, 0x0000_0800).unwrap();
            p.write_u32($cmd_last, 0x0000_2000).unwrap();
            assert_eq!(p.read_u32(REG_CMD0).unwrap(), 0x0000_0800);
            assert_eq!(p.read_u32($cmd_last).unwrap(), 0x0000_2000);
        }

        $(
            // ── Headline test: an attached I2cDevice round-trips a
            //    write-then-read transaction driven exactly as this chip's
            //    firmware would. Uses the Bmp280 register-pointer device.
            #[test]
            fn $bmp280_test() {
                use crate::peripherals::components::Bmp280;
                let mut p = <$ctrl>::new();
                // Default address 0x76.
                p.push_slave(Box::new(Bmp280::new(0x76)));

                // Canonical register-pointer read: set pointer to 0xD0
                // (chip-id), then repeated-start and read one byte. CHIP_ID
                // for BMP280 is 0x58.
                //   RSTART; WRITE 2 (addr+W, pointer=0xD0); RSTART;
                //   WRITE 1 (addr+R); READ 1; STOP.
                p.write_u32(REG_CMD0, cmd(CMD_RSTART, 0)).unwrap();
                p.write_u32(REG_CMD0 + 4, cmd(CMD_WRITE, 2)).unwrap();
                p.write_u32(REG_CMD0 + 8, cmd(CMD_RSTART, 0)).unwrap();
                p.write_u32(REG_CMD0 + 12, cmd(CMD_WRITE, 1)).unwrap();
                p.write_u32(REG_CMD0 + 16, cmd(CMD_READ, 1)).unwrap();
                p.write_u32(REG_CMD0 + 20, cmd(CMD_STOP, 0)).unwrap();

                // Push TX bytes: addr+W (0x76<<1=0xEC), pointer 0xD0, addr+R (0xED).
                p.write_u32(REG_DATA, 0xEC).unwrap();
                p.write_u32(REG_DATA, 0xD0).unwrap();
                p.write_u32(REG_DATA, 0xED).unwrap();

                $launch(&mut p);

                // Address must have matched (no NACK).
                assert_eq!(
                    p.read_u32(REG_INT_RAW).unwrap() & INT_NACK,
                    0,
                    "BMP280 at 0x76 must ACK its address"
                );
                // Slave acked → the SR ack bit is set.
                assert_eq!(
                    p.read_u32(REG_SR).unwrap() & $ack_bit,
                    $ack_bit,
                    "SR ack bit must be set after a successful transaction"
                );
                // The chip-id byte 0x58 should be in the RX FIFO.
                assert_eq!(
                    p.read_u32(REG_DATA).unwrap(),
                    0x58,
                    "BMP280 CHIP_ID round-trip"
                );
                // STOP completed the transaction.
                assert_eq!(
                    p.read_u32(REG_INT_RAW).unwrap() & INT_TRANS_COMPLETE,
                    INT_TRANS_COMPLETE
                );
            }
        )?

        // ── TCA9548A driven through this controller's command-list engine ──
        //
        // The switch's own unit tests and `tests/i2c_mux_tca9548a.rs` prove it
        // against the STM32F1 peripheral. This engine is a completely
        // different address-resolution site — including a `SLAVE_ADDR`
        // fallback the STM32 has no equivalent of — so each chip's engine
        // drives the switch itself.
        mod mux {
            use super::*;
            use crate::peripherals::components::mux_fixture::{
                bytes_written_to, mux_with_tags, tag_for, MUX_ADDR, SENSOR_ADDR,
            };
            use crate::peripherals::components::tca9548a::Tca9548a;

            fn controller() -> $ctrl {
                let mut p = <$ctrl>::new();
                p.push_slave(Box::new(mux_with_tags(4)));
                p
            }

            fn with_mux<R>(p: &$ctrl, f: impl FnOnce(&Tca9548a) -> R) -> R {
                let mux = p.attached_slaves()[0]
                    .as_any()
                    .and_then(|a| a.downcast_ref::<Tca9548a>())
                    .expect("slave 0 is the switch");
                f(mux)
            }

            /// Clear the previous run's latched interrupts so this run's NACK
            /// verdict is about this run.
            fn clear_ints(p: &mut $ctrl) {
                p.write_u32(REG_INT_CLR, 0xFFFF_FFFF).unwrap();
            }

            fn program(p: &mut $ctrl, list: &[(u8, u8)], tx: &[u8]) {
                clear_ints(p);
                // Flush both FIFOs without disturbing the watermark fields.
                let conf = p.read_u32(REG_FIFO_CONF).unwrap();
                p.write_u32(REG_FIFO_CONF, conf | (1 << 12) | (1 << 13))
                    .unwrap();
                for (i, (op, n)) in list.iter().enumerate() {
                    p.write_u32(REG_CMD0 + 4 * i as u64, cmd(*op, *n)).unwrap();
                }
                for b in tx {
                    p.write_u32(REG_DATA, *b as u32).unwrap();
                }
                $launch(p);
            }

            /// `RSTART; WRITE n(addr+W, payload…); STOP` — the address byte
            /// rides in the TX FIFO, which is the shape esp-hal emits.
            fn write_bytes(p: &mut $ctrl, addr: u8, payload: &[u8]) {
                let mut tx = vec![addr << 1];
                tx.extend_from_slice(payload);
                p.write_u32(REG_SLAVE_ADDR, addr as u32).unwrap();
                program(
                    p,
                    &[(CMD_RSTART, 0), (CMD_WRITE, tx.len() as u8), (CMD_STOP, 0)],
                    &tx,
                );
            }

            /// `RSTART; WRITE 1(addr+R); READ 1; STOP`, returning the received byte.
            fn read_byte(p: &mut $ctrl, addr: u8) -> u8 {
                p.write_u32(REG_SLAVE_ADDR, addr as u32).unwrap();
                program(
                    p,
                    &[
                        (CMD_RSTART, 0),
                        (CMD_WRITE, 1),
                        (CMD_READ, 1),
                        (CMD_STOP, 0),
                    ],
                    &[(addr << 1) | 1],
                );
                p.read_u32(REG_DATA).unwrap() as u8
            }

            fn nacked(p: &$ctrl) -> bool {
                p.read_u32(REG_INT_RAW).unwrap() & INT_NACK != 0
            }

            /// Address-only probe: `RSTART; WRITE 1(addr+W); STOP`.
            fn probe_acked(p: &mut $ctrl, addr: u8) -> bool {
                p.write_u32(REG_SLAVE_ADDR, addr as u32).unwrap();
                program(
                    p,
                    &[(CMD_RSTART, 0), (CMD_WRITE, 1), (CMD_STOP, 0)],
                    &[addr << 1],
                );
                !nacked(p)
            }

            #[test]
            fn four_sensors_at_one_address_answer_independently() {
                let mut p = controller();
                for ch in 0..4u8 {
                    write_bytes(&mut p, MUX_ADDR, &[1 << ch]);
                    assert_eq!(
                        read_byte(&mut p, SENSOR_ADDR),
                        tag_for(ch),
                        "channel {ch} must be answered by the sensor wired to it"
                    );
                }
            }

            #[test]
            fn switching_channels_changes_which_sensor_answers() {
                let mut p = controller();
                for ch in [2u8, 0, 3, 1, 3, 0] {
                    write_bytes(&mut p, MUX_ADDR, &[1 << ch]);
                    assert_eq!(read_byte(&mut p, SENSOR_ADDR), tag_for(ch), "channel {ch}");
                }
            }

            #[test]
            fn control_register_reads_back_over_the_bus() {
                let mut p = controller();
                write_bytes(&mut p, MUX_ADDR, &[0b0000_1010]);
                assert!(
                    probe_acked(&mut p, MUX_ADDR),
                    "the switch ACKs its own address"
                );
                assert_eq!(read_byte(&mut p, MUX_ADDR), 0b0000_1010);
            }

            #[test]
            fn a_sensor_on_a_disabled_channel_does_not_answer() {
                let mut p = controller();
                assert!(
                    !probe_acked(&mut p, SENSOR_ADDR),
                    "with all channels disabled the sensor address must raise INT_NACK, \
                     exactly as an unpopulated bus does"
                );

                write_bytes(&mut p, MUX_ADDR, &[1 << 1]);
                assert!(probe_acked(&mut p, SENSOR_ADDR));
                assert_eq!(read_byte(&mut p, SENSOR_ADDR), tag_for(1));

                write_bytes(&mut p, MUX_ADDR, &[0x00]);
                assert!(
                    !probe_acked(&mut p, SENSOR_ADDR),
                    "re-isolating the switch takes the sensor off the bus again"
                );
            }

            #[test]
            fn a_write_reaches_only_the_selected_channel() {
                let mut p = controller();
                write_bytes(&mut p, MUX_ADDR, &[1 << 2]);
                write_bytes(&mut p, SENSOR_ADDR, &[0x5A]);

                with_mux(&p, |mux| {
                    assert_eq!(bytes_written_to(mux, 2), vec![0x5A]);
                    for ch in [0u8, 1, 3] {
                        assert!(
                            bytes_written_to(mux, ch).is_empty(),
                            "channel {ch} is isolated and must receive nothing"
                        );
                    }
                });
            }

            $(
                /// The OTHER address-resolution site on this engine: a
                /// zero-payload WRITE resolves the target from the
                /// `SLAVE_ADDR` register instead of the FIFO. It must consult
                /// the switch exactly the same way — and must NOT resolve a
                /// device the switch has isolated.
                #[test]
                fn $mux_slave_addr_test() {
                    let mut p = controller();

                    // Isolated: the parked address must not resolve.
                    p.write_u32(REG_SLAVE_ADDR, SENSOR_ADDR as u32).unwrap();
                    program(
                        &mut p,
                        &[(CMD_RSTART, 0), (CMD_WRITE, 0), (CMD_STOP, 0)],
                        &[],
                    );
                    assert!(
                        nacked(&p),
                        "a SLAVE_ADDR-parked probe must NACK while every channel is isolated"
                    );

                    // Enable channel 3, then probe + read through the same path.
                    write_bytes(&mut p, MUX_ADDR, &[1 << 3]);
                    p.write_u32(REG_SLAVE_ADDR, SENSOR_ADDR as u32).unwrap();
                    program(
                        &mut p,
                        &[
                            (CMD_RSTART, 0),
                            (CMD_WRITE, 0),
                            (CMD_READ, 1),
                            (CMD_STOP, 0),
                        ],
                        &[],
                    );
                    assert!(!nacked(&p), "channel 3 is enabled; 0x13 must ACK");
                    assert_eq!(
                        p.read_u32(REG_DATA).unwrap() as u8,
                        tag_for(3),
                        "the SLAVE_ADDR path must reach the sensor on the SELECTED channel"
                    );
                }
            )?

            $( $($mux_extra)* )?
        }
    };
}

#[cfg(test)]
pub(crate) use esp_i2c_family_suite;

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
