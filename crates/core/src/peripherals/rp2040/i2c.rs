// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 I2C — Synopsys DesignWare APB I2C (`DW_apb_i2c`, datasheet §4.3).
//!
//! Master path with attached slaves (matrix kits). Supports:
//! - Arduino Wire (poll TX_ABRT / STATUS)
//! - Zephyr `i2c_dw` (INT: TX_EMPTY + STOP_DET after DATA_CMD)
//!
//! Zephyr init requires `IC_COMP_TYPE == 0x44570140`.

use crate::peripherals::i2c::I2cDevice;
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::cell::{Cell, RefCell};

/// Line order for this controller's published wire; the FUNCSEL pad table
/// routes SCL/SDA pads to these indices.
pub(crate) const I2C_LINES: &[&str] = &["SCL", "SDA"];
pub(crate) const LINE_SCL: usize = 0;
pub(crate) const LINE_SDA: usize = 1;

const IC_CON: u64 = 0x00;
/// Standard-speed SCL high/low counts, in IC_CLK (= clk_sys) periods.
/// RP2040 datasheet §4.3.17: the SDK's `i2c_set_baudrate` programs these, and
/// they are the only statement of the bus's bit rate the model can read.
const IC_SS_SCL_HCNT: u64 = 0x14;
const IC_SS_SCL_LCNT: u64 = 0x18;
/// Fast-speed counts, selected by IC_CON.SPEED == 2 (the SDK's default).
const IC_FS_SCL_HCNT: u64 = 0x1c;
const IC_FS_SCL_LCNT: u64 = 0x20;
const IC_TAR: u64 = 0x04;
const IC_DATA_CMD: u64 = 0x10;
const IC_INTR_STAT: u64 = 0x2c;
const IC_INTR_MASK: u64 = 0x30;
const IC_RAW_INTR_STAT: u64 = 0x34;
const IC_CLR_TX_ABRT: u64 = 0x54;
const IC_CLR_ACTIVITY: u64 = 0x5c;
const IC_CLR_STOP_DET: u64 = 0x60;
const IC_ENABLE: u64 = 0x6c;
const IC_STATUS: u64 = 0x70;
const IC_TXFLR: u64 = 0x74;
const IC_RXFLR: u64 = 0x78;
const IC_TX_ABRT_SOURCE: u64 = 0x80;
const IC_COMP_TYPE: u64 = 0xfc;
const IC_COMP_TYPE_MAGIC: u32 = 0x4457_0140;

// RAW / MASK interrupt bits (DW_apb_i2c).
const INTR_TX_EMPTY: u32 = 1 << 4;
const INTR_TX_ABRT: u32 = 1 << 6;
const INTR_ACTIVITY: u32 = 1 << 8;
const INTR_STOP_DET: u32 = 1 << 9;

const ABRT_7B_ADDR_NOACK: u32 = 1 << 0;
const ENABLE_ENABLE: u32 = 1 << 0;

const STATUS_ACTIVITY: u32 = 1 << 0;
const STATUS_TFNF: u32 = 1 << 1;
const STATUS_TFE: u32 = 1 << 2;
const STATUS_RFNE: u32 = 1 << 3;

const DATA_CMD_READ: u32 = 1 << 8;
const DATA_CMD_STOP: u32 = 1 << 9;

#[derive(Default)]
pub struct Rp2040I2c {
    enable: u32,
    tar: u32,
    con: u32,
    ss_scl_hcnt: u32,
    ss_scl_lcnt: u32,
    fs_scl_hcnt: u32,
    fs_scl_lcnt: u32,
    /// Wire levels published to FUNCSEL-routed SCL/SDA pads, so a logic
    /// analyzer clipped to this bus measures a waveform instead of a flat line.
    lines: Option<std::sync::Arc<crate::peripherals::pad_lines::PadLines>>,
    /// Frames of the transfer in flight, narrated as one transaction at STOP.
    wire_frames: Vec<(u8, bool)>,
    intr_mask: u32,
    raw_intr: Cell<u32>,
    tx_abrt_source: Cell<u32>,
    rx_byte: Cell<Option<u8>>,
    activity: Cell<bool>,
    attached_devices: Vec<RefCell<Box<dyn I2cDevice>>>,
    /// Held-level event chain (see the `impl Peripheral` docs). Monotonic token
    /// so a stale in-flight event from a previous arm is ignored.
    arm_seq: u32,
    /// True while an event for this model is in flight, so a burst of writes
    /// arms exactly one chain rather than one per write.
    scheduled: bool,
    /// Bus cycle clock (the net engine's time base).
    clock: Option<crate::CycleClock>,
    /// The bit-level engine used on a world `gpio_net` (see `i2c_net`).
    net: net::DwNet,
}

impl std::fmt::Debug for Rp2040I2c {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rp2040I2c")
            .field("enable", &self.enable)
            .field("tar", &self.tar)
            .field("slaves", &self.attached_devices.len())
            .finish()
    }
}

impl Rp2040I2c {
    pub fn new() -> Self {
        let s = Self::default();
        // FIFO empty at reset → TX_EMPTY raw bit set (DW default behaviour).
        s.raw_intr.set(INTR_TX_EMPTY);
        s
    }

    /// The shared pad-line cell for this controller, created on first use at
    /// bus wiring time. An open-drain bus with pull-ups idles high.
    pub(crate) fn pad_lines_arc(
        &mut self,
    ) -> std::sync::Arc<crate::peripherals::pad_lines::PadLines> {
        use crate::peripherals::pad_lines::PadLines;
        self.lines
            .get_or_insert_with(|| {
                let lines = PadLines::new(I2C_LINES, &[true, true]);
                // On a world net both lines are open drain, released until
                // the bit engine (`i2c_net`) pulls one low.
                for line in [LINE_SCL, LINE_SDA] {
                    lines.set_net_idle(line, crate::peripherals::pad_lines::LineDrive::OpenDrain);
                }
                std::sync::Arc::new(lines)
            })
            .clone()
    }

    /// Engine cycles in one SCL period, from the controller's own timing
    /// registers. `IC_CON.SPEED` (bits [2:1]) picks the standard-mode or
    /// fast-mode count pair; both count `clk_sys` periods, which is the
    /// engine's cycle axis on this chip. Falls back to a floor when firmware
    /// has not programmed timing, so a waveform is still shaped.
    fn bit_time_cycles(&self) -> u64 {
        let fast = (self.con >> 1) & 0b11 >= 2;
        let (h, l) = if fast {
            (self.fs_scl_hcnt, self.fs_scl_lcnt)
        } else {
            (self.ss_scl_hcnt, self.ss_scl_lcnt)
        };
        u64::from(h + l).max(16)
    }

    /// Record a frame this transfer put on the wire (published at STOP).
    fn wire_push(&mut self, byte: u8, acked: bool) {
        if self.lines.is_some() {
            self.wire_frames.push((byte, acked));
        }
    }

    /// Publish the completed transfer's waveform onto the routed pads. See
    /// [`crate::peripherals::i2c_waveform`] for what a narrated waveform does
    /// and does not model; this controller exchanges bytes with no modelled
    /// wire time, so the narration is anchored to end at the present cycle.
    fn wire_flush(&mut self) {
        use crate::peripherals::i2c_waveform::I2cNarrator;
        let Some(lines) = self.lines.clone() else {
            self.wire_frames.clear();
            return;
        };
        if self.wire_frames.is_empty() {
            return;
        }
        let mut wave = I2cNarrator::new(LINE_SCL, LINE_SDA, self.bit_time_cycles());
        wave.start();
        for &(byte, acked) in &self.wire_frames {
            wave.frame(byte, acked);
        }
        wave.stop();
        self.wire_frames.clear();
        let now = lines.tap_clock().unwrap_or(0);
        let _fit = wave.emit_ending_at(&lines, now);
    }

    pub(crate) fn push_slave(&mut self, device: Box<dyn I2cDevice>) {
        self.attached_devices.push(RefCell::new(device));
    }

    /// Borrow attached slaves (browser sensor readback / inspect).
    pub fn attached_devices(&self) -> &[RefCell<Box<dyn I2cDevice>>] {
        &self.attached_devices
    }

    /// Resolve the attached device that answers to `addr7`. Uses
    /// `claims_address` (not `address()`) so a bus switch can answer for the
    /// devices behind its enabled channels — see
    /// [`crate::peripherals::i2c::I2cDevice::claims_address`].
    fn device_for(&self, addr7: u8) -> Option<usize> {
        self.attached_devices
            .iter()
            .position(|d| d.borrow().claims_address(addr7))
    }

    fn set_raw(&self, bits: u32) {
        self.raw_intr.set(self.raw_intr.get() | bits);
    }

    fn clr_raw(&self, bits: u32) {
        self.raw_intr.set(self.raw_intr.get() & !bits);
    }

    fn irq_pending(&self) -> bool {
        let raw = if self.net_mode() {
            self.net_raw()
        } else {
            self.raw_intr.get()
        };
        (raw & self.intr_mask) != 0
    }
}

impl Peripheral for Rp2040I2c {
    /// This controller hosts attachable I²C slaves, so it opts into the
    /// machine's central device-time drive. Before Phase A the drive had no
    /// source on this family and the opt-in would have been inert, which is why
    /// it was missing; with the derived `cpu_hz` clock it is load-bearing —
    /// without it every `data_ready` / `delay_us` device on this chip stays
    /// always-ready.
    fn drives_central_i2c_time(&self) -> bool {
        true
    }

    fn advance_attached_i2c_us(&mut self, us: u64) {
        if us == 0 {
            return;
        }
        for cell in &self.attached_devices {
            cell.borrow_mut().advance_time_us(us);
        }
    }

    /// Tier 2: collect this controller's attached devices' pin drives.
    fn drain_attached_pin_drives(&mut self, out: &mut Vec<(String, String, bool)>) {
        for cell in &self.attached_devices {
            crate::peripherals::device::drain_i2c_pin_drives(&mut **cell.borrow_mut(), out);
        }
    }

    fn line_names(&self) -> &'static [&'static str] {
        I2C_LINES
    }

    fn wire_input_edge(&mut self, line: usize, level: bool, cycle: u64) -> bool {
        self.net_edge(line, level, cycle)
    }

    fn attach_cycle_clock(&mut self, clock: crate::CycleClock) {
        self.clock = Some(clock);
    }

    fn wire_lines(&self) -> Option<&crate::peripherals::pad_lines::PadLines> {
        self.lines.as_deref()
    }

    /// Scheduler-driven, held-level: `I2C0_IRQ` rides an event chain, not the
    /// per-cycle walk.
    ///
    /// # What was wrong
    ///
    /// This model used to declare `needs_legacy_walk() -> false` with the
    /// comment "pure write-driven transfer engine — `tick()` is structural
    /// no-op". The transfer engine is write-driven; the *interrupt* was not.
    /// `tick()` returns `irq: self.irq_pending()`, and that was the ONLY NVIC
    /// pend the model had. Every peripheral on `rp2040-pico` makes the same
    /// claim, so `SystemBus::derive_walk_deletable` deletes the walk and the
    /// pend never happens. Nothing catches it downstream either:
    /// `deliver_scheduled_irq_levels` covers only the C3 and S3 interrupt
    /// matrices and returns `false` on an NVIC bus.
    ///
    /// Arduino `Wire` polls `IC_TX_ABRT_SOURCE` / `IC_STATUS`, which is why no
    /// shipped lab noticed. pico-sdk's I2C-slave path and embassy-rp's async
    /// I2C both wait on the interrupt and hang outright.
    ///
    /// # Why an event chain and not `needs_legacy_walk() -> true`
    ///
    /// Restoring the walk would fix delivery by making the whole RP2040 bus
    /// slow: `derive_walk_deletable` is all-or-nothing, so one forcing model
    /// drops `max_safe_tick_interval` from 512 to 1 for *every* RP2040 lab,
    /// including the majority that never enable an I2C interrupt. The chain
    /// below costs a scheduler wakeup only while an interrupt is genuinely
    /// armed AND asserting — which is exactly when a real DW_apb_i2c holds its
    /// level line up, and when the CPU would be in the ISR anyway.
    ///
    /// # The chain
    ///
    /// `(raw_intr & intr_mask)` can only *rise* on an MMIO write
    /// (`IC_ENABLE`, `IC_INTR_MASK`, `IC_DATA_CMD`) — the `IC_CLR_*` registers
    /// are read-to-clear and only lower it. So the write choke
    /// (`SystemBus::collect_scheduled_events`, run after every write to a
    /// `uses_scheduler()` model) is a complete arming point, and
    /// `take_scheduled_events` needs no clock and no `sync_to`. `on_event` then
    /// re-pends at delay 1 while the level holds and stops rescheduling the
    /// moment firmware masks or clears it — the same held-level shape
    /// [`crate::peripherals::rp2040::timer::Rp2040Timer`] uses for its alarms.
    ///
    /// Left ungated (not `cfg!(feature = "event-scheduler")`) so walk-deletion
    /// derives identically in both builds; the walk's skip of scheduler models
    /// is itself feature-gated, so a featureless build still ticks this model
    /// exactly as before.
    fn uses_scheduler(&self) -> bool {
        true
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        // On a net the wire engine keeps its own timed steps armed.
        let mut events = if self.net_mode() {
            self.net_take_events()
        } else {
            Vec::new()
        };
        // Only arm while a level is actually up, and only once per chain.
        if !self.irq_pending() || self.scheduled {
            return events;
        }
        self.arm_seq = self.arm_seq.wrapping_add(1);
        self.scheduled = true;
        // delay 0 → deadline `current_cycle + 1`: the cycle the legacy walk's
        // next tick would have serviced this.
        events.push((0, self.arm_seq));
        events
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if event_token == net::NET_TOKEN {
            return self.net_on_event(_sched.now());
        }
        if event_token != self.arm_seq {
            // Stale token from a superseded arm; the live chain owns delivery.
            return crate::sched::EventResult::default();
        }
        let pending = self.irq_pending();
        self.scheduled = pending;
        crate::sched::EventResult {
            // The bus pends `PeripheralEntry::irq` (NVIC 23 on rp2040-pico) —
            // the event-path twin of the walk's `PeripheralTickResult::irq`.
            raise_own_irq: pending,
            reschedule_delay: pending.then_some(1),
            ..Default::default()
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn for_each_attached_device(&self, f: &mut dyn FnMut(crate::inspect::AttachedDeviceRef<'_>)) {
        for cell in &self.attached_devices {
            crate::inspect::visit_i2c_device(&**cell.borrow(), f);
        }
    }

    /// Stimulus reachability: a BME280 on i2c0 must answer `set_input`
    /// (`temperature`, ...) like it does on every other I2C controller. The
    /// trait default returns false, which made every RP2040 I2C sensor
    /// undrivable ("no attached input device exposes channel 'temperature'")
    /// while firmware read it fine.
    fn for_each_attached_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        for cell in &self.attached_devices {
            if cell.borrow_mut().for_each_sim_input(f) {
                return true;
            }
        }
        false
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if self.net_mode() {
            let val = match offset {
                IC_DATA_CMD => Some(self.net_pop_rx()),
                IC_STATUS => Some(self.net_status()),
                IC_RAW_INTR_STAT => Some(self.net_raw()),
                IC_INTR_STAT => Some(self.net_raw() & self.intr_mask),
                _ => self
                    .net_read_clear(offset)
                    .or_else(|| self.net_read_reg(offset)),
            };
            if let Some(val) = val {
                return Ok(val);
            }
        }
        let val = match offset {
            IC_CON => self.con,
            IC_SS_SCL_HCNT => self.ss_scl_hcnt,
            IC_SS_SCL_LCNT => self.ss_scl_lcnt,
            IC_FS_SCL_HCNT => self.fs_scl_hcnt,
            IC_FS_SCL_LCNT => self.fs_scl_lcnt,
            IC_TAR => self.tar,
            IC_ENABLE => self.enable,
            IC_INTR_MASK => self.intr_mask,
            IC_RAW_INTR_STAT => self.raw_intr.get(),
            IC_INTR_STAT => self.raw_intr.get() & self.intr_mask,
            IC_CLR_TX_ABRT => {
                self.clr_raw(INTR_TX_ABRT);
                self.tx_abrt_source.set(0);
                0
            }
            IC_CLR_ACTIVITY => {
                self.clr_raw(INTR_ACTIVITY);
                self.activity.set(false);
                0
            }
            IC_CLR_STOP_DET => {
                self.clr_raw(INTR_STOP_DET);
                0
            }
            IC_DATA_CMD => self.rx_byte.take().unwrap_or(0xFF) as u32,
            IC_STATUS => {
                let mut s = STATUS_TFE | STATUS_TFNF;
                if self.activity.get() {
                    s |= STATUS_ACTIVITY;
                }
                if self.rx_byte.get().is_some() {
                    s |= STATUS_RFNE;
                }
                s
            }
            IC_TXFLR => 0,
            IC_RXFLR => u32::from(self.rx_byte.get().is_some()),
            IC_TX_ABRT_SOURCE => self.tx_abrt_source.get(),
            IC_COMP_TYPE => IC_COMP_TYPE_MAGIC,
            _ => {
                crate::census_reg!("rp2040.i2c:Rp2040I2c", offset, "read");
                0
            }
        };
        Ok(val)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if self.net_mode() && self.net_write(offset, value) {
            return Ok(());
        }
        match offset {
            IC_CON => self.con = value,
            IC_SS_SCL_HCNT => self.ss_scl_hcnt = value & 0xFFFF,
            IC_SS_SCL_LCNT => self.ss_scl_lcnt = value & 0xFFFF,
            IC_FS_SCL_HCNT => self.fs_scl_hcnt = value & 0xFFFF,
            IC_FS_SCL_LCNT => self.fs_scl_lcnt = value & 0xFFFF,
            IC_TAR => self.tar = value & 0x3FF,
            IC_INTR_MASK => self.intr_mask = value,
            IC_ENABLE => {
                self.enable = value;
                if value & ENABLE_ENABLE != 0 {
                    // Enabled with empty FIFO → TX_EMPTY asserted.
                    self.set_raw(INTR_TX_EMPTY);
                }
            }
            IC_DATA_CMD if self.enable & ENABLE_ENABLE != 0 => {
                let addr7 = (self.tar & 0x7F) as u8;
                let stop = value & DATA_CMD_STOP != 0;
                self.activity.set(true);
                self.set_raw(INTR_ACTIVITY);
                // Consuming a command clears TX_EMPTY until the engine finishes.
                self.clr_raw(INTR_TX_EMPTY);
                // The address frame goes out either way; whether anyone ACKed
                // is exactly what an analyzer should show.
                let addressed = self.device_for(addr7).is_some();
                let read = value & DATA_CMD_READ != 0;
                if self.wire_frames.is_empty() {
                    self.wire_push((addr7 << 1) | u8::from(read), addressed);
                }
                match self.device_for(addr7) {
                    None => {
                        self.set_raw(INTR_TX_ABRT);
                        self.tx_abrt_source
                            .set(self.tx_abrt_source.get() | ABRT_7B_ADDR_NOACK);
                        self.activity.set(false);
                        self.set_raw(INTR_TX_EMPTY);
                        // An abort ends the transfer: "the master aborts the
                        // transfer by issuing a STOP condition" and flushes
                        // its FIFOs, whether or not the command carried the
                        // STOP bit (RP2040 datasheet §4.3, p.448 and the
                        // IC_RAW_INTR_STAT.TX_ABRT note, p.480). pico-sdk's
                        // `i2c_write_blocking_internal` waits for STOP_DET
                        // after every abort, so without this a NACKed first
                        // byte of a multi-byte or no-stop write spins on
                        // IC_RAW_INTR_STAT forever (Arduino-Mbed
                        // `Wire.endTransmission` to an absent 0x50).
                        self.set_raw(INTR_STOP_DET);
                    }
                    Some(idx) => {
                        self.clr_raw(INTR_TX_ABRT);
                        self.tx_abrt_source.set(0);
                        self.attached_devices[idx]
                            .borrow_mut()
                            .select_address(addr7);
                        // First command of a transaction → slave START so
                        // pointer-phase devices (INA219, etc.) reset.
                        if self.wire_frames.len() <= 1 {
                            // wire_frames already has the address frame.
                            self.attached_devices[idx].borrow_mut().start();
                        }
                        if value & DATA_CMD_READ != 0 {
                            let b = self.attached_devices[idx].borrow_mut().read();
                            self.rx_byte.set(Some(b));
                            // The master ACKs the byte it reads; the closing
                            // NACK is implied by the STOP that follows.
                            self.wire_push(b, !stop);
                        } else {
                            let byte = (value & 0xFF) as u8;
                            self.attached_devices[idx].borrow_mut().write(byte);
                            self.wire_push(byte, true);
                        }
                        // Instant complete — FIFO empty again.
                        self.set_raw(INTR_TX_EMPTY);
                        self.activity.set(false);
                        if stop {
                            self.attached_devices[idx].borrow_mut().stop();
                            self.set_raw(INTR_STOP_DET);
                        }
                    }
                }
                if stop || self.raw_intr.get() & INTR_TX_ABRT != 0 {
                    // The transaction closed (STOP, or an abort's STOP): put
                    // its waveform on the pads.
                    self.wire_flush();
                }
            }
            _ => {
                crate::census_reg!("rp2040.i2c:Rp2040I2c", offset, "write");
            }
        }
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !0x3)?;
        Ok((word >> ((offset & 0x3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !0x3;
        let shift = (offset & 0x3) * 8;
        let cur = self.read_u32(aligned)?;
        let new = (cur & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_u32(aligned, new)
    }

    /// Legacy-walk / hardware-oracle path only.
    ///
    /// Under `event-scheduler` the walk skips this model (`uses_scheduler`) and
    /// the event chain owns delivery. Without the feature the walk still runs
    /// and this keeps the pre-migration behaviour byte-identical.
    fn tick(&mut self) -> PeripheralTickResult {
        if self.net_mode() {
            self.net_tick();
        }
        PeripheralTickResult {
            irq: self.irq_pending(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_i2c() -> Rp2040I2c {
        let mut i2c = Rp2040I2c::new();
        i2c.write_u32(IC_ENABLE, ENABLE_ENABLE).unwrap();
        i2c
    }

    /// pico-sdk `i2c_write_blocking_internal`, register for register: write
    /// a byte, spin on IC_RAW_INTR_STAT for TX_EMPTY, read TX_ABRT_SOURCE,
    /// and on an abort spin again for STOP_DET. Returns the abort source, or
    /// None when the STOP_DET spin did not end (the hang the twin had).
    fn sdk_write_blocking(
        i2c: &mut Rp2040I2c,
        addr: u32,
        bytes: &[u8],
        nostop: bool,
    ) -> Option<u32> {
        const SPIN_CAP: usize = 10_000;
        i2c.write_u32(IC_ENABLE, 0).unwrap();
        i2c.write_u32(IC_TAR, addr).unwrap();
        i2c.write_u32(IC_ENABLE, ENABLE_ENABLE).unwrap();
        for (i, b) in bytes.iter().enumerate() {
            let last = i == bytes.len() - 1;
            let stop = if last && !nostop { DATA_CMD_STOP } else { 0 };
            i2c.write_u32(IC_DATA_CMD, u32::from(*b) | stop).unwrap();
            let mut spins = 0;
            while i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_EMPTY == 0 {
                spins += 1;
                if spins > SPIN_CAP {
                    return None;
                }
            }
            let reason = i2c.read_u32(IC_TX_ABRT_SOURCE).unwrap();
            if reason != 0 {
                i2c.read_u32(IC_CLR_TX_ABRT).unwrap();
            }
            if reason != 0 || (last && !nostop) {
                let mut spins = 0;
                while i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_STOP_DET == 0 {
                    spins += 1;
                    if spins > SPIN_CAP {
                        return None;
                    }
                }
                i2c.read_u32(IC_CLR_STOP_DET).unwrap();
            }
            if reason != 0 {
                return Some(reason);
            }
        }
        Some(0)
    }

    #[test]
    fn nacked_write_without_stop_bit_still_ends_with_stop_det() {
        // Wire.beginTransmission(0x50); Wire.write(0); Wire.endTransmission()
        // with nothing at 0x50: a multi-byte write NACKs on its first byte,
        // which carries no STOP bit. Real silicon aborts and issues STOP.
        let mut i2c = Rp2040I2c::new();
        assert_eq!(
            sdk_write_blocking(&mut i2c, 0x50, &[0x00, 0x01], false),
            Some(ABRT_7B_ADDR_NOACK)
        );
        // A repeated-start (nostop) probe of one byte, likewise.
        let mut i2c = Rp2040I2c::new();
        assert_eq!(
            sdk_write_blocking(&mut i2c, 0x50, &[0x00], true),
            Some(ABRT_7B_ADDR_NOACK)
        );
    }

    #[test]
    fn unacked_transfer_aborts_with_addr_nack() {
        let mut i2c = enabled_i2c();
        assert_eq!(i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_ABRT, 0);
        i2c.write_u32(IC_DATA_CMD, 0xDE | DATA_CMD_STOP).unwrap();
        assert_ne!(i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_ABRT, 0);
        assert_ne!(
            i2c.read_u32(IC_TX_ABRT_SOURCE).unwrap() & ABRT_7B_ADDR_NOACK,
            0
        );
    }

    #[test]
    fn attached_slave_acks_write_and_stop_det() {
        struct Dev {
            addr: u8,
            last: Cell<u8>,
        }
        impl I2cDevice for Dev {
            fn address(&self) -> u8 {
                self.addr
            }
            fn read(&mut self) -> u8 {
                0
            }
            fn write(&mut self, data: u8) {
                self.last.set(data);
            }
        }
        let mut i2c = enabled_i2c();
        i2c.push_slave(Box::new(Dev {
            addr: 0x40,
            last: Cell::new(0),
        }));
        i2c.intr_mask = INTR_TX_EMPTY | INTR_STOP_DET | INTR_TX_ABRT;
        i2c.write_u32(IC_TAR, 0x40).unwrap();
        i2c.write_u32(IC_DATA_CMD, 0xAB | DATA_CMD_STOP).unwrap();
        assert_eq!(i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_ABRT, 0);
        assert_ne!(i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_STOP_DET, 0);
        assert_ne!(i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_EMPTY, 0);
        assert!(i2c.tick().irq);
    }

    /// Four sensors at ONE fixed address behind a TCA9548A, on the RP2040's
    /// DesignWare controller. Guards the same resolution seam as the nRF52 and
    /// STM32 tests: `device_for` must ask `claims_address`, not compare
    /// `address()` and take the first hit.
    #[test]
    fn four_same_address_sensors_behind_a_bus_switch_are_each_reachable() {
        use crate::peripherals::components::tca9548a::Tca9548a;

        struct Tag(u8);
        impl I2cDevice for Tag {
            fn address(&self) -> u8 {
                0x13
            }
            fn read(&mut self) -> u8 {
                self.0
            }
            fn write(&mut self, _data: u8) {}
        }

        let mut mux = Tca9548a::new(0x70);
        for ch in 0..4u8 {
            mux.attach(ch, Box::new(Tag(0xA0 + ch))).unwrap();
        }

        let mut i2c = enabled_i2c();
        i2c.push_slave(Box::new(mux));

        // Reset state: no channel enabled, so 0x13 is on no reachable segment.
        i2c.write_u32(IC_TAR, 0x13).unwrap();
        i2c.write_u32(IC_DATA_CMD, DATA_CMD_READ | DATA_CMD_STOP)
            .unwrap();
        assert_ne!(
            i2c.read_u32(IC_TX_ABRT_SOURCE).unwrap() & ABRT_7B_ADDR_NOACK,
            0,
            "an unselected sensor must NACK like an empty bus"
        );

        for ch in 0..4u8 {
            // Select the channel on the switch.
            i2c.write_u32(IC_TAR, 0x70).unwrap();
            i2c.write_u32(IC_DATA_CMD, (1u32 << ch) | DATA_CMD_STOP)
                .unwrap();
            assert_eq!(
                i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_ABRT,
                0,
                "the switch must ACK its own address"
            );

            // Read the sensor behind it.
            i2c.write_u32(IC_TAR, 0x13).unwrap();
            i2c.write_u32(IC_DATA_CMD, DATA_CMD_READ | DATA_CMD_STOP)
                .unwrap();
            assert_eq!(
                i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_ABRT,
                0,
                "channel {ch} is enabled, so 0x13 must ACK"
            );
            assert_eq!(
                i2c.read_u32(IC_DATA_CMD).unwrap() & 0xFF,
                (0xA0 + ch) as u32,
                "channel {ch} must be answered by its own sensor"
            );
        }
    }

    #[test]
    fn comp_type_is_designware_magic() {
        let i2c = Rp2040I2c::new();
        assert_eq!(i2c.read_u32(IC_COMP_TYPE).unwrap(), IC_COMP_TYPE_MAGIC);
    }

    #[test]
    fn disabled_controller_does_not_abort() {
        let mut i2c = Rp2040I2c::new();
        i2c.write_u32(IC_DATA_CMD, 0xDE).unwrap();
        assert_eq!(i2c.read_u32(IC_RAW_INTR_STAT).unwrap() & INTR_TX_ABRT, 0);
    }
}

#[path = "i2c_net.rs"]
mod net;
