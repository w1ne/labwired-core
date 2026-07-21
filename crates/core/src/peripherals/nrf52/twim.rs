// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Nordic nRF52 TWIM (I²C Master) peripheral — register surface + EasyDMA
//! byte movement.
//!
//! Source: nRF52840 Product Specification rev 1.7 §6.31 (TWIM).
//!
//! TWIM0 base: 0x40003000  (shared with SPIM0/SPIS0/SPI0/TWI0/TWIS0)
//! TWIM1 base: 0x40004000  (shared with SPIM1/SPIS1/SPI1/TWI1/TWIS1)
//! ENABLE value: 6 selects TWIM master mode (PS §6.31.4.1).
//!
//! # EasyDMA operation
//!
//! **TASKS_STARTTX (0x008):** reads TXD.MAXCNT bytes from RAM at TXD.PTR,
//! delivers them to the attached I2C device (or consumes them if no device
//! is attached), sets TXD.AMOUNT, fires EVENTS_LASTTX (0x160). Then,
//! depending on SHORTS, may fire EVENTS_STOPPED (0x104) or chain to STARTRX.
//!
//! **TASKS_STARTRX (0x000):** reads RXD.MAXCNT bytes from the device (or
//! fills with 0xFF if no device), writes them to RAM at RXD.PTR, sets
//! RXD.AMOUNT, fires EVENTS_LASTRX (0x15C). Then, depending on SHORTS, may
//! fire EVENTS_STOPPED (0x104) or chain to STARTTX.
//!
//! **TASKS_STOP (0x014):** fires EVENTS_STOPPED (0x104).
//!
//! # SHORTS (0x200)  (nRF52840 PS §6.31, Table 211)
//!
//! Bit  7: LASTTX_STARTRX  — auto-chain TX→RX (repeated-START)
//! Bit  8: LASTTX_SUSPEND  — hold bus after TX (no STOP; fires EVENTS_SUSPENDED)
//! Bit  9: LASTTX_STOP     — auto-stop after TX
//! Bit 10: LASTRX_STARTTX  — auto-chain RX→TX
//! Bit 11: LASTRX_SUSPEND  — hold bus after RX (no STOP; fires EVENTS_SUSPENDED)
//! Bit 12: LASTRX_STOP     — auto-stop after RX
//!
//! # EVENTS write semantics
//!
//! SW writes of 1 are silently ignored (hardware-generated only). SW writes
//! of 0 clear the event register. This matches silicon-verified TIMER/RTC
//! behaviour applied uniformly across all Nordic peripherals.
//!
//! # ERRORSRC (0x4C4)
//!
//! Bit 1: ANACK — address NACK (device not present)
//! Bit 2: DNACK — data NACK
//! W1C: writing 1 to a bit clears it.

use crate::peripherals::i2c::I2cDevice;
use crate::{Bus, Peripheral, PeripheralTickResult, SimResult};
use std::cell::RefCell;

// ── Task offsets ──────────────────────────────────────────────────────────────
const OFF_TASKS_STARTRX: u64 = 0x000;
const OFF_TASKS_STARTTX: u64 = 0x008;
const OFF_TASKS_STOP: u64 = 0x014;
const OFF_TASKS_RESUME: u64 = 0x020;
const OFF_TASKS_SUSPEND: u64 = 0x01C;

// ── Event offsets ─────────────────────────────────────────────────────────────
const OFF_EVENTS_STOPPED: u64 = 0x104;
const OFF_EVENTS_ERROR: u64 = 0x124;
const OFF_EVENTS_SUSPENDED: u64 = 0x148;
const OFF_EVENTS_RXSTARTED: u64 = 0x14C;
const OFF_EVENTS_TXSTARTED: u64 = 0x150;
const OFF_EVENTS_LASTRX: u64 = 0x15C;
const OFF_EVENTS_LASTTX: u64 = 0x160;

// ── Control registers ─────────────────────────────────────────────────────────
const OFF_SHORTS: u64 = 0x200;
const OFF_INTEN: u64 = 0x300;
const OFF_INTENSET: u64 = 0x304;
const OFF_INTENCLR: u64 = 0x308;
const OFF_ERRORSRC: u64 = 0x4C4;
const OFF_ENABLE: u64 = 0x500;
const OFF_PSEL_SCL: u64 = 0x508;
const OFF_PSEL_SDA: u64 = 0x50C;
const OFF_FREQUENCY: u64 = 0x524;

// ── EasyDMA registers ─────────────────────────────────────────────────────────
const OFF_RXD_PTR: u64 = 0x534;
const OFF_RXD_MAXCNT: u64 = 0x538;
const OFF_RXD_AMOUNT: u64 = 0x53C;
const OFF_TXD_PTR: u64 = 0x544;
const OFF_TXD_MAXCNT: u64 = 0x548;
const OFF_TXD_AMOUNT: u64 = 0x54C;

// ── Address register ──────────────────────────────────────────────────────────
const OFF_ADDRESS: u64 = 0x588;

// ── SHORTS bits (nRF52840 PS TWIM_SHORTS, Table 211) ─────────────────────────
const SHORT_LASTTX_STARTRX: u32 = 1 << 7; // LASTTX → STARTRX
const SHORT_LASTTX_SUSPEND: u32 = 1 << 8; // LASTTX → SUSPEND (TX_NO_STOP path)
const SHORT_LASTTX_STOP: u32 = 1 << 9; // LASTTX → STOP
const SHORT_LASTRX_STARTTX: u32 = 1 << 10; // LASTRX → STARTTX
const SHORT_LASTRX_SUSPEND: u32 = 1 << 11; // LASTRX → SUSPEND
const SHORT_LASTRX_STOP: u32 = 1 << 12; // LASTRX → STOP

// ── INTEN bits (PS §6.31, TWIM_INTENSET table) ───────────────────────────────
// STOPPED=1, ERROR=9, SUSPENDED=18, RXSTARTED=19, TXSTARTED=20, LASTRX=23, LASTTX=24
const INTEN_STOPPED: u32 = 1 << 1;
const INTEN_ERROR: u32 = 1 << 9;
const INTEN_SUSPENDED: u32 = 1 << 18;
const INTEN_RXSTARTED: u32 = 1 << 19;
const INTEN_TXSTARTED: u32 = 1 << 20;
const INTEN_LASTRX: u32 = 1 << 23;
const INTEN_LASTTX: u32 = 1 << 24;
const INTEN_MASK: u32 = INTEN_STOPPED
    | INTEN_ERROR
    | INTEN_SUSPENDED
    | INTEN_RXSTARTED
    | INTEN_TXSTARTED
    | INTEN_LASTRX
    | INTEN_LASTTX;

// ── ERRORSRC bits ─────────────────────────────────────────────────────────────
const ERRORSRC_ANACK: u32 = 1 << 1;
const ERRORSRC_DNACK: u32 = 1 << 2;
const ERRORSRC_MASK: u32 = ERRORSRC_ANACK | ERRORSRC_DNACK;

// ── Misc masks ────────────────────────────────────────────────────────────────
const ENABLE_MASK: u32 = 0xF;
const MAXCNT_MASK: u32 = 0xFF;
const ADDRESS_MASK: u32 = 0x7F;
const SHORTS_MASK: u32 = SHORT_LASTTX_STARTRX
    | SHORT_LASTTX_SUSPEND
    | SHORT_LASTTX_STOP
    | SHORT_LASTRX_STARTTX
    | SHORT_LASTRX_SUSPEND
    | SHORT_LASTRX_STOP;

// ── Pending-transfer token values ─────────────────────────────────────────────
/// No transfer pending.
const PENDING_NONE: u8 = 0;
/// TASKS_STARTTX was written.
const PENDING_TX: u8 = 1;
/// TASKS_STARTRX was written.
const PENDING_RX: u8 = 2;
/// TASKS_STOP was written.
const PENDING_STOP: u8 = 3;

/// Nordic nRF52 TWIM (I²C Master) peripheral — register surface with EasyDMA.
///
/// Implements the `tick_with_bus` / `needs_bus_tick` pattern used by ECB and
/// SPIM: the EasyDMA engine runs on the next bus-tick after TASKS_STARTTX or
/// TASKS_STARTRX is written, keeping the register write itself synchronous.
pub struct Nrf52Twim {
    // ── EVENTS (HW-set only; SW write-1 ignored, write-0 clears) ─────────────
    events_stopped: u32,
    events_error: u32,
    events_suspended: u32,
    events_rxstarted: u32,
    events_txstarted: u32,
    events_lastrx: u32,
    events_lasttx: u32,

    // ── Control / config ──────────────────────────────────────────────────────
    shorts: u32,
    inten: u32,
    /// ERRORSRC W1C bits — only cleared by writing 1; set by HW (NACK path).
    errorsrc: u32,
    enable: u32,
    psel_scl: u32,
    psel_sda: u32,
    frequency: u32,

    // ── EasyDMA descriptors ───────────────────────────────────────────────────
    rxd_ptr: u32,
    rxd_maxcnt: u32,
    rxd_amount: u32,
    txd_ptr: u32,
    txd_maxcnt: u32,
    txd_amount: u32,

    // ── Slave address ─────────────────────────────────────────────────────────
    address: u32,

    // ── Internal state ────────────────────────────────────────────────────────
    /// Transfer pending for `tick_with_bus`.  One of PENDING_{NONE,TX,RX,STOP}.
    pending: u8,

    /// Remaining core-cycles of wire latency before the pending transfer
    /// completes and its EVENTS (and the IRQ) fire. Models real I²C transfer
    /// time so a completion interrupt cannot preempt the driver's
    /// transfer-launch critical section. See `transfer_cycles`. Counted down by
    /// the configured `peripheral_tick_interval` each `tick_with_bus`.
    busy_cycles: u32,

    /// I2C devices attached to this master bus.  Keyed by 7-bit address.
    #[allow(dead_code)]
    attached_devices: Vec<RefCell<Box<dyn I2cDevice>>>,
}

impl std::fmt::Debug for Nrf52Twim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Nrf52Twim")
            .field("enable", &self.enable)
            .field("address", &self.address)
            .field("pending", &self.pending)
            .finish()
    }
}

impl Default for Nrf52Twim {
    fn default() -> Self {
        Self {
            events_stopped: 0,
            events_error: 0,
            events_suspended: 0,
            events_rxstarted: 0,
            events_txstarted: 0,
            events_lastrx: 0,
            events_lasttx: 0,
            shorts: 0,
            inten: 0,
            errorsrc: 0,
            enable: 0,
            // PSEL regs reset to 0xFFFF_FFFF (CONNECT=1 = disconnected).
            psel_scl: 0xFFFF_FFFF,
            psel_sda: 0xFFFF_FFFF,
            frequency: 0x0400_0000, // 100 kHz (K100 = 0x0198_0000 in some docs;
            // Nordic SDK also accepts 0x0400_0000)
            rxd_ptr: 0,
            rxd_maxcnt: 0,
            rxd_amount: 0,
            txd_ptr: 0,
            txd_maxcnt: 0,
            txd_amount: 0,
            address: 0,
            pending: PENDING_NONE,
            busy_cycles: 0,
            attached_devices: Vec::new(),
        }
    }
}

impl Nrf52Twim {
    pub fn new() -> Self {
        Self::default()
    }

    /// Raw slave push — does NOT wrap for tracing. Callers are the bus choke
    /// point [`crate::bus::SystemBus::attach_i2c_slave`] and the nRF52 serial
    /// mux, both of which wrap first. The device is matched by its 7-bit address
    /// when `ADDRESS` is set and a STARTTX/STARTRX task fires.
    pub(crate) fn push_slave(&mut self, device: Box<dyn I2cDevice>) {
        self.attached_devices.push(RefCell::new(device));
    }

    /// The attached I²C slaves, in attach order. Mirrors
    /// [`crate::peripherals::i2c::I2c::attached_devices`] so callers can reach
    /// a device by a path independent of the sim-input walk.
    pub fn attached_devices(&self) -> &[RefCell<Box<dyn I2cDevice>>] {
        &self.attached_devices
    }

    /// Find the first attached device whose `address()` matches `addr7`.
    fn device_for(&self, addr7: u8) -> Option<usize> {
        self.attached_devices
            .iter()
            .position(|d| d.borrow().address() == addr7)
    }

    /// Core-cycle latency of a `bytes`-byte wire transfer at the configured SCL
    /// frequency, including the START + address phase.
    ///
    /// **Why this matters (silicon-fidelity / false-pass prevention):** real
    /// I²C is slow — one byte at 100 kHz takes ~90 µs (~5760 cycles at the
    /// nRF52840's 64 MHz core). The interrupt-driven nrfx/Zephyr driver writes
    /// TASKS_START*while holding a spinlock*, then leaves the critical section
    /// and blocks in `k_sem_take`; the completion IRQ only arrives microseconds
    /// later, by which time the lock is released. If the model instead fired
    /// the completion EVENTS (and thus the IRQ) on the *next* tick, the ISR
    /// would preempt the still-held spinlock → recursive-spinlock fault → the
    /// nrfx ISR re-enters forever. Modelling the transfer time makes the IRQ
    /// land after the driver is safely parked, exactly as on hardware.
    fn transfer_cycles(&self, bytes: u32) -> u32 {
        // nRF52840 CPU/HFCLK = 64 MHz. Map the FREQUENCY register to the SCL
        // bit-rate (only the three standard Nordic values matter; anything
        // unrecognised falls back to the slowest/safest 100 kHz).
        const CORE_HZ: u32 = 64_000_000;
        let scl_hz: u32 = match self.frequency {
            f if f >= 0x0640_0000 => 400_000,
            f if f >= 0x0400_0000 => 250_000,
            _ => 100_000,
        };
        let cycles_per_bit = CORE_HZ / scl_hz; // 640 @100k · 256 @250k · 160 @400k
                                               // 9 bits/byte (8 data + ACK); +1 byte models START + address + R/W.
                                               // Floor at one byte-time so even a 0-byte STOP delays past the
                                               // driver's critical section.
        ((bytes + 1) * 9 * cycles_per_bit).max(9 * cycles_per_bit)
    }

    /// Execute a TX transfer: read `txd_maxcnt` bytes from bus RAM at
    /// `txd_ptr`, deliver to the matching device (or discard if absent),
    /// set TXD.AMOUNT, fire EVENTS_TXSTARTED + EVENTS_LASTTX, honour SHORTS.
    ///
    /// Returns `true` if a NACK occurred (no device at ADDRESS).
    fn do_tx(&mut self, bus: &mut dyn Bus) -> bool {
        let addr7 = (self.address & ADDRESS_MASK) as u8;
        let txd_ptr = self.txd_ptr as u64;
        let txd_maxcnt = (self.txd_maxcnt & MAXCNT_MASK) as usize;

        let dev_idx = self.device_for(addr7);

        // Signal address phase start.
        self.events_txstarted = 1;

        if dev_idx.is_none() {
            // No device → ANACK.
            self.errorsrc |= ERRORSRC_ANACK;
            self.events_error = 1;
            // Still complete the EasyDMA bookkeeping with AMOUNT = 0.
            self.txd_amount = 0;
            self.events_lasttx = 1;
            return true;
        }

        let idx = dev_idx.unwrap();
        self.attached_devices[idx].borrow_mut().start();

        let mut amount: u32 = 0;
        for i in 0..txd_maxcnt {
            let byte = bus.read_u8(txd_ptr + i as u64).unwrap_or(0);
            self.attached_devices[idx].borrow_mut().write(byte);
            amount += 1;
        }

        self.txd_amount = amount;
        self.events_lasttx = 1;
        false // no NACK
    }

    /// Execute an RX transfer: read `rxd_maxcnt` bytes from the device
    /// (or 0xFF if absent), write them to bus RAM at `rxd_ptr`, set
    /// RXD.AMOUNT, fire EVENTS_RXSTARTED + EVENTS_LASTRX, honour SHORTS.
    ///
    /// Returns `true` if a NACK occurred.
    fn do_rx(&mut self, bus: &mut dyn Bus) -> bool {
        let addr7 = (self.address & ADDRESS_MASK) as u8;
        let rxd_ptr = self.rxd_ptr as u64;
        let rxd_maxcnt = (self.rxd_maxcnt & MAXCNT_MASK) as usize;

        let dev_idx = self.device_for(addr7);

        // Signal address phase start.
        self.events_rxstarted = 1;

        if dev_idx.is_none() {
            // No device → ANACK.
            self.errorsrc |= ERRORSRC_ANACK;
            self.events_error = 1;
            // Fill RX buffer with 0xFF (bus release / NACK default).
            for i in 0..rxd_maxcnt {
                let _ = bus.write_u8(rxd_ptr + i as u64, 0xFF);
            }
            self.rxd_amount = rxd_maxcnt as u32;
            self.events_lastrx = 1;
            return true;
        }

        let idx = dev_idx.unwrap();
        self.attached_devices[idx].borrow_mut().start();

        let mut amount: u32 = 0;
        for i in 0..rxd_maxcnt {
            let byte = self.attached_devices[idx].borrow_mut().read();
            let _ = bus.write_u8(rxd_ptr + i as u64, byte);
            amount += 1;
        }

        self.rxd_amount = amount;
        self.events_lastrx = 1;
        false // no NACK
    }
}

impl Peripheral for Nrf52Twim {
    // Byte-granularity read/write are required to satisfy the Peripheral trait,
    // but nRF52 firmware always uses 32-bit STR/LDR for peripheral access.
    // We satisfy the trait minimally and rely on read_u32 / write_u32.
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset {
            // TASKs: read as 0 (write-only strobes on silicon).
            OFF_TASKS_STARTRX | OFF_TASKS_STARTTX | OFF_TASKS_STOP | OFF_TASKS_RESUME
            | OFF_TASKS_SUSPEND => 0,

            // EVENTS.
            OFF_EVENTS_STOPPED => self.events_stopped,
            OFF_EVENTS_ERROR => self.events_error,
            OFF_EVENTS_SUSPENDED => self.events_suspended,
            OFF_EVENTS_RXSTARTED => self.events_rxstarted,
            OFF_EVENTS_TXSTARTED => self.events_txstarted,
            OFF_EVENTS_LASTRX => self.events_lastrx,
            OFF_EVENTS_LASTTX => self.events_lasttx,

            // SHORTS.
            OFF_SHORTS => self.shorts & SHORTS_MASK,

            // INTEN (all three aliases return the current mask).
            OFF_INTEN | OFF_INTENSET | OFF_INTENCLR => self.inten & INTEN_MASK,

            // ERRORSRC W1C.
            OFF_ERRORSRC => self.errorsrc & ERRORSRC_MASK,

            // Config.
            OFF_ENABLE => self.enable & ENABLE_MASK,
            OFF_PSEL_SCL => self.psel_scl,
            OFF_PSEL_SDA => self.psel_sda,
            OFF_FREQUENCY => self.frequency,

            // EasyDMA.
            OFF_RXD_PTR => self.rxd_ptr,
            OFF_RXD_MAXCNT => self.rxd_maxcnt & MAXCNT_MASK,
            OFF_RXD_AMOUNT => self.rxd_amount & MAXCNT_MASK,
            OFF_TXD_PTR => self.txd_ptr,
            OFF_TXD_MAXCNT => self.txd_maxcnt & MAXCNT_MASK,
            OFF_TXD_AMOUNT => self.txd_amount & MAXCNT_MASK,

            // Address.
            OFF_ADDRESS => self.address & ADDRESS_MASK,

            _ => 0,
        })
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            // ── TASKS ─────────────────────────────────────────────────────────
            OFF_TASKS_STARTRX if value != 0 => {
                self.pending = PENDING_RX;
                self.busy_cycles = self.transfer_cycles(self.rxd_maxcnt & MAXCNT_MASK);
            }
            OFF_TASKS_STARTTX if value != 0 => {
                self.pending = PENDING_TX;
                self.busy_cycles = self.transfer_cycles(self.txd_maxcnt & MAXCNT_MASK);
            }
            // Immediate stop with no pending transfer. If a transfer is in
            // flight this arm does not match (the write lands in the no-op
            // task arm below) and STOPPED is added in tick_with_bus once the
            // transfer completes.
            OFF_TASKS_STOP if value != 0 && self.pending == PENDING_NONE => {
                self.pending = PENDING_STOP;
                self.busy_cycles = self.transfer_cycles(0);
            }
            // TASKS_RESUME after a LASTTX_SUSPEND hold: this is nrfx's
            // TX_NO_STOP (`write_read_dt`) path — the TX leg suspended the bus
            // (EVENTS_SUSPENDED) instead of stopping, the driver set up
            // RXD.PTR/MAXCNT, and now RESUME (NOT STARTRX) starts the follow-on
            // RX. Gating on an ACTIVE EVENTS_SUSPENDED is what makes this safe:
            // in the SHORTS auto-chain path no suspend is ever held when a
            // spurious RESUME is written, so the earlier "stale RXD.MAXCNT"
            // mis-fire (a register write turned into a bogus read) cannot recur.
            OFF_TASKS_RESUME
                if value != 0
                    && self.events_suspended != 0
                    && (self.rxd_maxcnt & MAXCNT_MASK) > 0 =>
            {
                self.events_suspended = 0;
                self.pending = PENDING_RX;
                self.busy_cycles = self.transfer_cycles(self.rxd_maxcnt & MAXCNT_MASK);
            }
            // Otherwise RESUME/SUSPEND only un-/hold the bus and never start a
            // transfer — the STARTTX/STARTRX tasks are the sole initiators.
            OFF_TASKS_RESUME | OFF_TASKS_SUSPEND => {}
            OFF_TASKS_STARTRX | OFF_TASKS_STARTTX | OFF_TASKS_STOP => {
                // value == 0: no-op (tasks are level-triggered on non-zero)
            }

            // ── EVENTS — SW write-1 ignored; SW write-0 clears ───────────────
            OFF_EVENTS_STOPPED if value == 0 => self.events_stopped = 0,
            OFF_EVENTS_ERROR if value == 0 => self.events_error = 0,
            OFF_EVENTS_SUSPENDED if value == 0 => self.events_suspended = 0,
            OFF_EVENTS_RXSTARTED if value == 0 => self.events_rxstarted = 0,
            OFF_EVENTS_TXSTARTED if value == 0 => self.events_txstarted = 0,
            OFF_EVENTS_LASTRX if value == 0 => self.events_lastrx = 0,
            OFF_EVENTS_LASTTX if value == 0 => self.events_lasttx = 0,

            // ── SHORTS ────────────────────────────────────────────────────────
            OFF_SHORTS => self.shorts = value & SHORTS_MASK,

            // ── INTEN / INTENSET / INTENCLR ───────────────────────────────────
            OFF_INTEN => self.inten = value & INTEN_MASK,
            OFF_INTENSET => self.inten |= value & INTEN_MASK,
            OFF_INTENCLR => self.inten &= !(value & INTEN_MASK),

            // ── ERRORSRC W1C ─────────────────────────────────────────────────
            OFF_ERRORSRC => self.errorsrc &= !(value & ERRORSRC_MASK),

            // ── Config ────────────────────────────────────────────────────────
            OFF_ENABLE => self.enable = value & ENABLE_MASK,
            OFF_PSEL_SCL => self.psel_scl = value,
            OFF_PSEL_SDA => self.psel_sda = value,
            OFF_FREQUENCY => self.frequency = value,

            // ── EasyDMA (AMOUNT is HW-written; firmware writes accepted) ──────
            OFF_RXD_PTR => self.rxd_ptr = value,
            OFF_RXD_MAXCNT => self.rxd_maxcnt = value & MAXCNT_MASK,
            OFF_RXD_AMOUNT => self.rxd_amount = value & MAXCNT_MASK,
            OFF_TXD_PTR => self.txd_ptr = value,
            OFF_TXD_MAXCNT => self.txd_maxcnt = value & MAXCNT_MASK,
            OFF_TXD_AMOUNT => self.txd_amount = value & MAXCNT_MASK,

            // ── Address ───────────────────────────────────────────────────────
            OFF_ADDRESS => self.address = value & ADDRESS_MASK,

            _ => {}
        }
        Ok(())
    }

    fn needs_bus_tick(&self) -> bool {
        self.pending != PENDING_NONE
    }

    /// EasyDMA engine.  Called by the bus when `needs_bus_tick()` is true.
    ///
    /// Sequence (PS §6.31 state diagram):
    /// 1. Execute the pending task (TX or RX or STOP).
    /// 2. Check SHORTS to determine what to chain next.
    /// 3. Fire EVENTS_SUSPENDED (bus held, no STOP) or EVENTS_STOPPED as
    ///    appropriate, and reset the I2C device state on STOP.
    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        let pending = self.pending;
        if pending == PENDING_NONE {
            return;
        }

        // Model wire-transfer latency: hold the transfer "on the bus" until the
        // configured per-tick instruction quantum has counted down the cycle
        // budget set when the task was triggered. Until then the completion
        // EVENTS (and the IRQ) do not fire, so an interrupt cannot preempt the
        // driver's transfer-launch critical section. See `transfer_cycles`.
        if self.busy_cycles > 0 {
            let interval = bus.config().peripheral_tick_interval.max(1);
            self.busy_cycles = self.busy_cycles.saturating_sub(interval);
            return;
        }

        self.pending = PENDING_NONE;

        let addr7 = (self.address & ADDRESS_MASK) as u8;

        match pending {
            PENDING_STOP => {
                self.events_stopped = 1;
                // STOP condition: reset I2C device register-address cursor.
                if let Some(idx) = self.device_for(addr7) {
                    self.attached_devices[idx].borrow_mut().stop();
                }
            }
            PENDING_TX => {
                let _nack = self.do_tx(bus);

                // Honour SHORTS after LASTTX.
                if self.shorts & SHORT_LASTTX_STARTRX != 0 {
                    // Chain TX→RX via repeated-START (no STOP between them).
                    // The follow-on RX is a fresh wire transfer: re-arm latency.
                    self.pending = PENDING_RX;
                    self.busy_cycles = self.transfer_cycles(self.rxd_maxcnt & MAXCNT_MASK);
                } else if self.shorts & SHORT_LASTTX_SUSPEND != 0 {
                    // Bus held (no STOP); fires EVENTS_SUSPENDED.
                    // nrfx uses this for TX_NO_STOP (write-then-read split into
                    // two separate nrfx_twim_xfer calls).
                    self.events_suspended = 1;
                } else if self.shorts & SHORT_LASTTX_STOP != 0 {
                    self.events_stopped = 1;
                    if let Some(idx) = self.device_for(addr7) {
                        self.attached_devices[idx].borrow_mut().stop();
                    }
                }
                // If no SHORT matches, firmware drives TASKS_STOP or
                // TASKS_STARTRX explicitly.
            }
            PENDING_RX => {
                let _nack = self.do_rx(bus);

                // Honour SHORTS after LASTRX.
                if self.shorts & SHORT_LASTRX_STOP != 0 {
                    self.events_stopped = 1;
                    if let Some(idx) = self.device_for(addr7) {
                        self.attached_devices[idx].borrow_mut().stop();
                    }
                } else if self.shorts & SHORT_LASTRX_SUSPEND != 0 {
                    self.events_suspended = 1;
                } else if self.shorts & SHORT_LASTRX_STARTTX != 0 {
                    // Chain RX→TX: re-arm latency for the follow-on TX leg.
                    self.pending = PENDING_TX;
                    self.busy_cycles = self.transfer_cycles(self.txd_maxcnt & MAXCNT_MASK);
                }
            }
            _ => {}
        }
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Raise IRQ for any enabled + pending event.
        let events: &[(&u32, u32, u64)] = &[
            (&self.events_stopped, INTEN_STOPPED, OFF_EVENTS_STOPPED),
            (&self.events_error, INTEN_ERROR, OFF_EVENTS_ERROR),
            (
                &self.events_suspended,
                INTEN_SUSPENDED,
                OFF_EVENTS_SUSPENDED,
            ),
            (
                &self.events_rxstarted,
                INTEN_RXSTARTED,
                OFF_EVENTS_RXSTARTED,
            ),
            (
                &self.events_txstarted,
                INTEN_TXSTARTED,
                OFF_EVENTS_TXSTARTED,
            ),
            (&self.events_lastrx, INTEN_LASTRX, OFF_EVENTS_LASTRX),
            (&self.events_lasttx, INTEN_LASTTX, OFF_EVENTS_LASTTX),
        ];

        let mut irq = false;
        let mut fired: Vec<u32> = Vec::new();

        for &(ev, mask, off) in events {
            if *ev != 0 && self.inten & mask != 0 {
                irq = true;
                fired.push(off as u32);
            }
        }

        PeripheralTickResult {
            irq,
            fired_events: fired,
            ..Default::default()
        }
    }

    /// Required for [`crate::bus::SystemBus::attach_i2c_slave`] to downcast to
    /// `Nrf52Twim`. Without these, that downcast can never match and attaching
    /// any I²C slave to a TWIM controller fails loudly at attach time — which
    /// removed the whole nRF52 board line from programmatic attach.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    /// TWIM holds its slaves behind `RefCell`, like the generic `I2c`.
    fn for_each_attached_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        for cell in self.attached_devices.iter_mut() {
            let mut dev = cell.borrow_mut();
            if let Some(si) = dev.as_sim_input_mut() {
                if f(si) {
                    return true;
                }
            }
        }
        false
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bus, DmaRequest, SimulationConfig};
    use std::collections::HashMap;

    // ── Minimal flat-RAM bus ──────────────────────────────────────────────────

    struct FlatRam {
        mem: HashMap<u64, u8>,
        config: SimulationConfig,
    }

    impl FlatRam {
        fn new() -> Self {
            Self {
                mem: HashMap::new(),
                config: SimulationConfig::default(),
            }
        }

        fn write_slice(&mut self, base: u64, data: &[u8]) {
            for (i, &b) in data.iter().enumerate() {
                self.mem.insert(base + i as u64, b);
            }
        }

        fn read_slice(&self, base: u64, len: usize) -> Vec<u8> {
            (0..len)
                .map(|i| *self.mem.get(&(base + i as u64)).unwrap_or(&0))
                .collect()
        }
    }

    impl Bus for FlatRam {
        fn read_u8(&self, addr: u64) -> crate::SimResult<u8> {
            Ok(*self.mem.get(&addr).unwrap_or(&0))
        }
        fn write_u8(&mut self, addr: u64, value: u8) -> crate::SimResult<()> {
            self.mem.insert(addr, value);
            Ok(())
        }
        fn tick_peripherals(&mut self) -> Vec<u32> {
            Vec::new()
        }
        fn execute_dma(&mut self, _requests: &[DmaRequest]) -> crate::SimResult<()> {
            Ok(())
        }
        fn config(&self) -> &SimulationConfig {
            &self.config
        }
    }

    // ── Minimal I2C device for testing ────────────────────────────────────────

    /// An I2C device that records written bytes and returns a fixed sequence
    /// on read.
    struct RecordingDevice {
        addr: u8,
        written: Vec<u8>,
        read_seq: Vec<u8>,
        read_pos: usize,
    }

    impl RecordingDevice {
        fn new(addr: u8, read_seq: Vec<u8>) -> Self {
            Self {
                addr,
                written: Vec::new(),
                read_seq,
                read_pos: 0,
            }
        }
    }

    impl I2cDevice for RecordingDevice {
        fn address(&self) -> u8 {
            self.addr
        }
        fn read(&mut self) -> u8 {
            if self.read_pos < self.read_seq.len() {
                let b = self.read_seq[self.read_pos];
                self.read_pos += 1;
                b
            } else {
                0xFF
            }
        }
        fn write(&mut self, data: u8) {
            self.written.push(data);
        }
        fn as_any(&self) -> Option<&dyn std::any::Any> {
            Some(self)
        }
        fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
            Some(self)
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn write32(t: &mut Nrf52Twim, offset: u64, value: u32) {
        t.write_u32(offset, value).unwrap();
    }

    fn read32(t: &Nrf52Twim, offset: u64) -> u32 {
        t.read_u32(offset).unwrap()
    }

    /// Drive exactly one EasyDMA transfer leg to completion: burn the modelled
    /// wire latency (see `Nrf52Twim::transfer_cycles`), then run the leg. This
    /// is the 1:1 replacement for a single `tick_with_bus` call in the old
    /// zero-latency model, so each call advances one TX/RX/STOP leg — a chained
    /// TX→RX still takes two `run_leg` calls, exactly as before.
    fn run_leg(t: &mut Nrf52Twim, bus: &mut FlatRam) {
        let mut guard = 0u32;
        while t.busy_cycles > 0 {
            t.tick_with_bus(bus);
            guard += 1;
            assert!(guard < 5_000_000, "transfer latency never drained");
        }
        t.tick_with_bus(bus);
    }

    // ── Register surface tests ────────────────────────────────────────────────

    #[test]
    fn enable_mask_4_bits() {
        let mut t = Nrf52Twim::new();
        write32(&mut t, OFF_ENABLE, 0xFF);
        assert_eq!(read32(&t, OFF_ENABLE), 0xF, "ENABLE retains only 4 bits");
    }

    #[test]
    fn enable_value_6_selects_twim() {
        let mut t = Nrf52Twim::new();
        write32(&mut t, OFF_ENABLE, 6);
        assert_eq!(read32(&t, OFF_ENABLE), 6);
    }

    #[test]
    fn tasks_read_as_zero() {
        let mut t = Nrf52Twim::new();
        for &off in &[
            OFF_TASKS_STARTRX,
            OFF_TASKS_STARTTX,
            OFF_TASKS_STOP,
            OFF_TASKS_RESUME,
            OFF_TASKS_SUSPEND,
        ] {
            write32(&mut t, off, 1);
            // Reset pending so we don't contaminate subsequent task checks.
            t.pending = PENDING_NONE;
            assert_eq!(read32(&t, off), 0, "TASK at 0x{:03X} must read zero", off);
        }
    }

    #[test]
    fn events_write_1_ignored() {
        let mut t = Nrf52Twim::new();
        for &off in &[
            OFF_EVENTS_STOPPED,
            OFF_EVENTS_ERROR,
            OFF_EVENTS_RXSTARTED,
            OFF_EVENTS_TXSTARTED,
            OFF_EVENTS_LASTRX,
            OFF_EVENTS_LASTTX,
        ] {
            write32(&mut t, off, 1);
            assert_eq!(
                read32(&t, off),
                0,
                "SW write-1 at offset 0x{:03X} must be ignored",
                off
            );
        }
    }

    #[test]
    fn events_write_0_clears() {
        // Force a non-zero value the same way HW would (direct field access in
        // test so we don't need a full transfer).
        let mut t = Nrf52Twim::new();
        t.events_stopped = 1;
        t.events_lasttx = 1;
        t.events_lastrx = 1;
        write32(&mut t, OFF_EVENTS_STOPPED, 0);
        assert_eq!(read32(&t, OFF_EVENTS_STOPPED), 0, "write-0 clears STOPPED");
        write32(&mut t, OFF_EVENTS_LASTTX, 0);
        assert_eq!(read32(&t, OFF_EVENTS_LASTTX), 0, "write-0 clears LASTTX");
        write32(&mut t, OFF_EVENTS_LASTRX, 0);
        assert_eq!(read32(&t, OFF_EVENTS_LASTRX), 0, "write-0 clears LASTRX");
    }

    #[test]
    fn psel_round_trip() {
        let mut t = Nrf52Twim::new();
        write32(&mut t, OFF_PSEL_SCL, 0x0000_000B);
        assert_eq!(read32(&t, OFF_PSEL_SCL), 0x0000_000B);
        write32(&mut t, OFF_PSEL_SDA, 0x8000_000C);
        assert_eq!(read32(&t, OFF_PSEL_SDA), 0x8000_000C);
    }

    #[test]
    fn address_mask_7_bits() {
        let mut t = Nrf52Twim::new();
        write32(&mut t, OFF_ADDRESS, 0xFF);
        assert_eq!(read32(&t, OFF_ADDRESS), 0x7F, "ADDRESS retains only 7 bits");
    }

    #[test]
    fn maxcnt_mask_8_bits() {
        let mut t = Nrf52Twim::new();
        write32(&mut t, OFF_TXD_MAXCNT, 0x1FF);
        assert_eq!(read32(&t, OFF_TXD_MAXCNT), 0xFF);
        write32(&mut t, OFF_RXD_MAXCNT, 0x1FF);
        assert_eq!(read32(&t, OFF_RXD_MAXCNT), 0xFF);
    }

    #[test]
    fn rxd_txd_ptr_round_trip() {
        let mut t = Nrf52Twim::new();
        write32(&mut t, OFF_TXD_PTR, 0x2000_0100);
        assert_eq!(read32(&t, OFF_TXD_PTR), 0x2000_0100);
        write32(&mut t, OFF_RXD_PTR, 0x2000_0200);
        assert_eq!(read32(&t, OFF_RXD_PTR), 0x2000_0200);
    }

    #[test]
    fn intenset_intenclr_round_trip() {
        let mut t = Nrf52Twim::new();
        let bits = INTEN_STOPPED | INTEN_LASTTX | INTEN_LASTRX;
        write32(&mut t, OFF_INTENSET, 0xFFFF_FFFF);
        assert_eq!(
            read32(&t, OFF_INTENSET),
            INTEN_MASK,
            "INTENSET masks to valid bits"
        );
        write32(&mut t, OFF_INTENCLR, bits);
        assert_eq!(
            read32(&t, OFF_INTENCLR),
            INTEN_MASK & !bits,
            "INTENCLR clears selected bits"
        );
    }

    #[test]
    fn errorsrc_w1c() {
        let mut t = Nrf52Twim::new();
        // Seed both error bits (would be set by HW on silicon).
        t.errorsrc = ERRORSRC_ANACK | ERRORSRC_DNACK;
        // Clear only ANACK.
        write32(&mut t, OFF_ERRORSRC, ERRORSRC_ANACK);
        assert_eq!(
            read32(&t, OFF_ERRORSRC),
            ERRORSRC_DNACK,
            "W1C: ANACK cleared, DNACK remains"
        );
        // Clear DNACK.
        write32(&mut t, OFF_ERRORSRC, ERRORSRC_DNACK);
        assert_eq!(read32(&t, OFF_ERRORSRC), 0);
    }

    #[test]
    fn shorts_mask() {
        let mut t = Nrf52Twim::new();
        write32(&mut t, OFF_SHORTS, 0xFFFF_FFFF);
        assert_eq!(
            read32(&t, OFF_SHORTS),
            SHORTS_MASK,
            "SHORTS retains only valid bits"
        );
    }

    // ── EasyDMA TX transfer tests ─────────────────────────────────────────────

    /// Full TX transfer with an attached device: bytes are read from RAM and
    /// delivered to the device.  TXD.AMOUNT set; EVENTS_LASTTX fired.
    #[test]
    fn twim_tx_reads_ram_and_sets_amount() {
        let mut t = Nrf52Twim::new();
        t.push_slave(Box::new(RecordingDevice::new(0x48, vec![])));
        let mut bus = FlatRam::new();

        let tx_base: u64 = 0x2000_0000;
        let tx_data: [u8; 3] = [0xAA, 0xBB, 0xCC];
        bus.write_slice(tx_base, &tx_data);

        write32(&mut t, OFF_ENABLE, 6);
        write32(&mut t, OFF_ADDRESS, 0x48);
        write32(&mut t, OFF_TXD_PTR, tx_base as u32);
        write32(&mut t, OFF_TXD_MAXCNT, 3);

        // TASKS_STARTTX — must not have fired events yet.
        write32(&mut t, OFF_TASKS_STARTTX, 1);
        assert_eq!(
            read32(&t, OFF_EVENTS_LASTTX),
            0,
            "LASTTX not set before tick"
        );
        assert!(t.needs_bus_tick(), "pending_start must be set");

        // Run EasyDMA.
        run_leg(&mut t, &mut bus);

        assert_eq!(
            read32(&t, OFF_EVENTS_LASTTX),
            1,
            "EVENTS_LASTTX must be 1 after TX"
        );
        assert_eq!(
            read32(&t, OFF_TXD_AMOUNT),
            3,
            "TXD.AMOUNT must equal MAXCNT"
        );
        assert!(!t.needs_bus_tick(), "pending cleared after tick");
        assert_eq!(read32(&t, OFF_ERRORSRC), 0, "no error on present device");
    }

    /// TX with no device: ANACK fired, TXD.AMOUNT = 0.
    #[test]
    fn twim_tx_no_device_fires_anack() {
        let mut t = Nrf52Twim::new();
        let mut bus = FlatRam::new();
        let tx_base: u64 = 0x2000_0100;
        bus.write_slice(tx_base, &[0x11, 0x22]);

        write32(&mut t, OFF_ENABLE, 6);
        write32(&mut t, OFF_ADDRESS, 0x55);
        write32(&mut t, OFF_TXD_PTR, tx_base as u32);
        write32(&mut t, OFF_TXD_MAXCNT, 2);
        write32(&mut t, OFF_TASKS_STARTTX, 1);
        run_leg(&mut t, &mut bus);

        assert_eq!(
            read32(&t, OFF_ERRORSRC) & ERRORSRC_ANACK,
            ERRORSRC_ANACK,
            "ANACK set"
        );
        assert_eq!(read32(&t, OFF_EVENTS_ERROR), 1, "EVENTS_ERROR set");
        assert_eq!(read32(&t, OFF_TXD_AMOUNT), 0, "TXD.AMOUNT = 0 on NACK");
        assert_eq!(
            read32(&t, OFF_EVENTS_LASTTX),
            1,
            "LASTTX still fires on NACK"
        );
    }

    /// TX: bytes delivered to device are exactly the bytes from RAM.
    #[test]
    fn twim_tx_delivers_correct_bytes_to_device() {
        let tx_data: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];
        let mut t = Nrf52Twim::new();
        t.push_slave(Box::new(RecordingDevice::new(0x10, vec![])));
        let mut bus = FlatRam::new();
        let tx_base: u64 = 0x2000_0200;
        bus.write_slice(tx_base, &tx_data);

        write32(&mut t, OFF_ENABLE, 6);
        write32(&mut t, OFF_ADDRESS, 0x10);
        write32(&mut t, OFF_TXD_PTR, tx_base as u32);
        write32(&mut t, OFF_TXD_MAXCNT, 4);
        write32(&mut t, OFF_TASKS_STARTTX, 1);
        run_leg(&mut t, &mut bus);

        let dev = t.attached_devices[0].borrow();
        let dev_any = dev
            .as_any()
            .expect("RecordingDevice has no as_any — check impl");
        let rec = dev_any.downcast_ref::<RecordingDevice>().unwrap();
        assert_eq!(rec.written, tx_data.to_vec(), "bytes delivered to device");
    }

    // ── EasyDMA RX transfer tests ─────────────────────────────────────────────

    /// Full RX transfer with an attached device: device bytes are written
    /// to RAM.  RXD.AMOUNT set; EVENTS_LASTRX fired.
    #[test]
    fn twim_rx_fills_ram_and_sets_amount() {
        let read_seq: Vec<u8> = vec![0x11, 0x22, 0x33, 0x44];
        let mut t = Nrf52Twim::new();
        t.push_slave(Box::new(RecordingDevice::new(0x68, read_seq.clone())));
        let mut bus = FlatRam::new();

        let rx_base: u64 = 0x2000_0300;
        write32(&mut t, OFF_ENABLE, 6);
        write32(&mut t, OFF_ADDRESS, 0x68);
        write32(&mut t, OFF_RXD_PTR, rx_base as u32);
        write32(&mut t, OFF_RXD_MAXCNT, 4);

        // TASKS_STARTRX — must not have fired events yet.
        write32(&mut t, OFF_TASKS_STARTRX, 1);
        assert_eq!(
            read32(&t, OFF_EVENTS_LASTRX),
            0,
            "LASTRX not set before tick"
        );
        assert!(t.needs_bus_tick());

        run_leg(&mut t, &mut bus);

        assert_eq!(
            read32(&t, OFF_EVENTS_LASTRX),
            1,
            "EVENTS_LASTRX must be 1 after RX"
        );
        assert_eq!(
            read32(&t, OFF_RXD_AMOUNT),
            4,
            "RXD.AMOUNT must equal MAXCNT"
        );
        assert!(!t.needs_bus_tick(), "pending cleared");

        let rx = bus.read_slice(rx_base, 4);
        assert_eq!(rx, read_seq, "RXD RAM contains device bytes");
        assert_eq!(read32(&t, OFF_ERRORSRC), 0, "no error");
    }

    /// RX with no device: RAM filled with 0xFF, ANACK fired.
    #[test]
    fn twim_rx_no_device_fills_ff_and_fires_anack() {
        let mut t = Nrf52Twim::new();
        let mut bus = FlatRam::new();
        let rx_base: u64 = 0x2000_0400;

        write32(&mut t, OFF_ENABLE, 6);
        write32(&mut t, OFF_ADDRESS, 0x55);
        write32(&mut t, OFF_RXD_PTR, rx_base as u32);
        write32(&mut t, OFF_RXD_MAXCNT, 3);
        write32(&mut t, OFF_TASKS_STARTRX, 1);
        run_leg(&mut t, &mut bus);

        assert_eq!(read32(&t, OFF_ERRORSRC) & ERRORSRC_ANACK, ERRORSRC_ANACK);
        assert_eq!(read32(&t, OFF_EVENTS_ERROR), 1);
        assert_eq!(
            read32(&t, OFF_RXD_AMOUNT),
            3,
            "amount = MAXCNT even on NACK"
        );
        let rx = bus.read_slice(rx_base, 3);
        assert_eq!(
            rx,
            vec![0xFF, 0xFF, 0xFF],
            "no-device: RAM filled with 0xFF"
        );
    }

    // ── SHORTS chaining tests ─────────────────────────────────────────────────

    /// SHORT LASTTX_STOP: after TX, EVENTS_STOPPED must fire automatically.
    #[test]
    fn short_lasttx_stop_fires_stopped() {
        let mut t = Nrf52Twim::new();
        t.push_slave(Box::new(RecordingDevice::new(0x20, vec![])));
        let mut bus = FlatRam::new();
        let tx_base: u64 = 0x2000_0500;
        bus.write_slice(tx_base, &[0x01, 0x02]);

        write32(&mut t, OFF_ENABLE, 6);
        write32(&mut t, OFF_ADDRESS, 0x20);
        write32(&mut t, OFF_TXD_PTR, tx_base as u32);
        write32(&mut t, OFF_TXD_MAXCNT, 2);
        write32(&mut t, OFF_SHORTS, SHORT_LASTTX_STOP);
        write32(&mut t, OFF_TASKS_STARTTX, 1);
        run_leg(&mut t, &mut bus);

        assert_eq!(read32(&t, OFF_EVENTS_LASTTX), 1, "LASTTX fired");
        assert_eq!(
            read32(&t, OFF_EVENTS_STOPPED),
            1,
            "STOPPED auto-fired via SHORT"
        );
        assert!(!t.needs_bus_tick(), "no further pending transfer");
    }

    /// SHORT LASTTX_SUSPEND: after TX with TX_NO_STOP, EVENTS_SUSPENDED fires
    /// (no STOP condition; bus is held for a subsequent RX).
    #[test]
    fn short_lasttx_suspend_fires_suspended() {
        let mut t = Nrf52Twim::new();
        t.push_slave(Box::new(RecordingDevice::new(0x76, vec![])));
        let mut bus = FlatRam::new();
        let tx_base: u64 = 0x2000_0500;
        bus.write_slice(tx_base, &[0xD0]); // register address byte

        write32(&mut t, OFF_ENABLE, 6);
        write32(&mut t, OFF_ADDRESS, 0x76);
        write32(&mut t, OFF_TXD_PTR, tx_base as u32);
        write32(&mut t, OFF_TXD_MAXCNT, 1);
        write32(&mut t, OFF_SHORTS, SHORT_LASTTX_SUSPEND); // TX_NO_STOP path
        write32(&mut t, OFF_TASKS_STARTTX, 1);
        run_leg(&mut t, &mut bus);

        assert_eq!(read32(&t, OFF_EVENTS_LASTTX), 1, "LASTTX fired");
        assert_eq!(
            read32(&t, OFF_EVENTS_SUSPENDED),
            1,
            "SUSPENDED auto-fired via SHORT_LASTTX_SUSPEND"
        );
        assert_eq!(
            read32(&t, OFF_EVENTS_STOPPED),
            0,
            "STOPPED must NOT fire for TX_NO_STOP"
        );
        assert!(!t.needs_bus_tick(), "no further pending transfer");
    }

    /// nrfx TX_NO_STOP write-then-read: a TX leg with LASTTX_SUSPEND holds the
    /// bus (EVENTS_SUSPENDED, no STOP); the driver then sets up RXD and issues
    /// **TASKS_RESUME** (not TASKS_STARTRX) to start the follow-on RX. The model
    /// must route that RESUME to the RX leg — this is the path #422 fixed, kept
    /// working alongside #424's SHORTS-chained path. RESUME→RX is gated on an
    /// active EVENTS_SUSPENDED so it cannot mis-fire on a stale RXD.MAXCNT in
    /// the SHORTS auto-chain path (where no suspend is ever active).
    #[test]
    fn twim_resume_after_suspend_starts_followon_rx() {
        let read_seq: Vec<u8> = vec![0x60]; // BME280 chip-id
        let mut t = Nrf52Twim::new();
        t.push_slave(Box::new(RecordingDevice::new(0x76, read_seq.clone())));
        let mut bus = FlatRam::new();
        let tx_base: u64 = 0x2000_0500;
        let rx_base: u64 = 0x2000_0600;
        bus.write_slice(tx_base, &[0xD0]); // register-address byte

        write32(&mut t, OFF_ENABLE, 6);
        write32(&mut t, OFF_ADDRESS, 0x76);
        write32(&mut t, OFF_TXD_PTR, tx_base as u32);
        write32(&mut t, OFF_TXD_MAXCNT, 1);
        write32(&mut t, OFF_SHORTS, SHORT_LASTTX_SUSPEND); // TX_NO_STOP
        write32(&mut t, OFF_TASKS_STARTTX, 1);
        run_leg(&mut t, &mut bus);

        assert_eq!(read32(&t, OFF_EVENTS_LASTTX), 1, "LASTTX fired");
        assert_eq!(read32(&t, OFF_EVENTS_SUSPENDED), 1, "bus held via SUSPEND");

        // Driver now sets up RXD and RESUMEs (the nrfx write_read_dt path).
        write32(&mut t, OFF_RXD_PTR, rx_base as u32);
        write32(&mut t, OFF_RXD_MAXCNT, 1);
        write32(&mut t, OFF_SHORTS, SHORT_LASTRX_STOP); // stop after the read
        write32(&mut t, OFF_TASKS_RESUME, 1);
        assert!(
            t.needs_bus_tick(),
            "TASKS_RESUME after SUSPEND must start the follow-on RX"
        );
        run_leg(&mut t, &mut bus);

        assert_eq!(
            read32(&t, OFF_EVENTS_LASTRX),
            1,
            "LASTRX after follow-on RX"
        );
        assert_eq!(read32(&t, OFF_RXD_AMOUNT), 1, "RXD.AMOUNT = MAXCNT");
        assert_eq!(read32(&t, OFF_EVENTS_STOPPED), 1, "STOPPED via LASTRX_STOP");
        assert_eq!(
            read32(&t, OFF_EVENTS_SUSPENDED),
            0,
            "SUSPENDED cleared on resume"
        );
        assert_eq!(
            bus.read_slice(rx_base, 1),
            read_seq,
            "device chip-id byte landed in RXD RAM"
        );
    }

    /// SHORT LASTRX_STOP: after RX, EVENTS_STOPPED fires automatically.
    #[test]
    fn short_lastrx_stop_fires_stopped() {
        let mut t = Nrf52Twim::new();
        t.push_slave(Box::new(RecordingDevice::new(0x21, vec![0xAA, 0xBB])));
        let mut bus = FlatRam::new();
        let rx_base: u64 = 0x2000_0600;

        write32(&mut t, OFF_ENABLE, 6);
        write32(&mut t, OFF_ADDRESS, 0x21);
        write32(&mut t, OFF_RXD_PTR, rx_base as u32);
        write32(&mut t, OFF_RXD_MAXCNT, 2);
        write32(&mut t, OFF_SHORTS, SHORT_LASTRX_STOP);
        write32(&mut t, OFF_TASKS_STARTRX, 1);
        run_leg(&mut t, &mut bus);

        assert_eq!(read32(&t, OFF_EVENTS_LASTRX), 1, "LASTRX fired");
        assert_eq!(
            read32(&t, OFF_EVENTS_STOPPED),
            1,
            "STOPPED auto-fired via SHORT"
        );
    }

    /// SHORT LASTTX_STARTRX: TX completes and chains automatically into RX
    /// (one more tick_with_bus required).
    #[test]
    fn short_lasttx_startrx_chains_into_rx() {
        let read_seq: Vec<u8> = vec![0x55, 0x66];
        let mut t = Nrf52Twim::new();
        t.push_slave(Box::new(RecordingDevice::new(0x30, read_seq.clone())));
        let mut bus = FlatRam::new();

        let tx_base: u64 = 0x2000_0700;
        let rx_base: u64 = 0x2000_0800;
        bus.write_slice(tx_base, &[0x01]);

        write32(&mut t, OFF_ENABLE, 6);
        write32(&mut t, OFF_ADDRESS, 0x30);
        write32(&mut t, OFF_TXD_PTR, tx_base as u32);
        write32(&mut t, OFF_TXD_MAXCNT, 1);
        write32(&mut t, OFF_RXD_PTR, rx_base as u32);
        write32(&mut t, OFF_RXD_MAXCNT, 2);
        // LASTTX→STARTRX; then LASTRX→STOP to close.
        write32(&mut t, OFF_SHORTS, SHORT_LASTTX_STARTRX | SHORT_LASTRX_STOP);
        write32(&mut t, OFF_TASKS_STARTTX, 1);

        // First tick: TX completes, LASTTX fired, PENDING_RX armed.
        run_leg(&mut t, &mut bus);
        assert_eq!(read32(&t, OFF_EVENTS_LASTTX), 1, "LASTTX after first tick");
        assert_eq!(
            read32(&t, OFF_EVENTS_STOPPED),
            0,
            "STOPPED must not fire between TX and RX"
        );
        assert!(t.needs_bus_tick(), "RX chained: pending must be set");

        // Second tick: RX completes, LASTRX fired, STOPPED via SHORT.
        run_leg(&mut t, &mut bus);
        assert_eq!(read32(&t, OFF_EVENTS_LASTRX), 1, "LASTRX after second tick");
        assert_eq!(
            read32(&t, OFF_EVENTS_STOPPED),
            1,
            "STOPPED after RX via SHORT"
        );
        assert!(!t.needs_bus_tick(), "done");

        let rx = bus.read_slice(rx_base, 2);
        assert_eq!(rx, read_seq, "RX bytes correct after TX→RX chain");
    }

    /// SHORT LASTRX_STARTTX: RX completes and chains into TX (one more tick).
    #[test]
    fn short_lastrx_starttx_chains_into_tx() {
        let mut t = Nrf52Twim::new();
        t.push_slave(Box::new(RecordingDevice::new(0x31, vec![0xAA])));
        let mut bus = FlatRam::new();

        let tx_base: u64 = 0x2000_0900;
        let rx_base: u64 = 0x2000_0A00;
        bus.write_slice(tx_base, &[0x42, 0x43]);

        write32(&mut t, OFF_ENABLE, 6);
        write32(&mut t, OFF_ADDRESS, 0x31);
        write32(&mut t, OFF_RXD_PTR, rx_base as u32);
        write32(&mut t, OFF_RXD_MAXCNT, 1);
        write32(&mut t, OFF_TXD_PTR, tx_base as u32);
        write32(&mut t, OFF_TXD_MAXCNT, 2);
        // LASTRX→STARTTX; LASTTX→STOP to finish.
        write32(&mut t, OFF_SHORTS, SHORT_LASTRX_STARTTX | SHORT_LASTTX_STOP);
        write32(&mut t, OFF_TASKS_STARTRX, 1);

        // First tick: RX completes, chains TX.
        run_leg(&mut t, &mut bus);
        assert_eq!(read32(&t, OFF_EVENTS_LASTRX), 1, "LASTRX after first tick");
        assert!(t.needs_bus_tick(), "TX chained: pending");

        // Second tick: TX completes, STOPPED via SHORT.
        run_leg(&mut t, &mut bus);
        assert_eq!(read32(&t, OFF_EVENTS_LASTTX), 1, "LASTTX after second tick");
        assert_eq!(read32(&t, OFF_EVENTS_STOPPED), 1, "STOPPED via SHORT");
        assert!(!t.needs_bus_tick());

        assert_eq!(read32(&t, OFF_TXD_AMOUNT), 2, "TXD.AMOUNT = 2");
    }

    /// TASKS_STOP with no pending transfer: fires EVENTS_STOPPED immediately.
    #[test]
    fn tasks_stop_fires_stopped() {
        let mut t = Nrf52Twim::new();
        let mut bus = FlatRam::new();

        write32(&mut t, OFF_TASKS_STOP, 1);
        assert!(t.needs_bus_tick());
        run_leg(&mut t, &mut bus);
        assert_eq!(
            read32(&t, OFF_EVENTS_STOPPED),
            1,
            "STOPPED fires after TASKS_STOP"
        );
    }

    /// EVENTS write-1 is ignored (silicon rule) — applied even AFTER a
    /// HW-set event.
    #[test]
    fn events_write_1_ignored_after_hw_set() {
        let mut t = Nrf52Twim::new();
        let mut bus = FlatRam::new();
        let tx_base: u64 = 0x2000_0B00;
        bus.write_slice(tx_base, &[0x01]);

        write32(&mut t, OFF_ENABLE, 6);
        write32(&mut t, OFF_ADDRESS, 0x48);
        write32(&mut t, OFF_TXD_PTR, tx_base as u32);
        write32(&mut t, OFF_TXD_MAXCNT, 1);
        t.push_slave(Box::new(RecordingDevice::new(0x48, vec![])));
        write32(&mut t, OFF_TASKS_STARTTX, 1);
        run_leg(&mut t, &mut bus);

        // HW set EVENTS_LASTTX.
        assert_eq!(read32(&t, OFF_EVENTS_LASTTX), 1);

        // SW write-1 must NOT change the register.
        write32(&mut t, OFF_EVENTS_LASTTX, 1);
        assert_eq!(
            read32(&t, OFF_EVENTS_LASTTX),
            1,
            "SW write-1 leaves HW-set event unchanged"
        );

        // SW write-0 clears it.
        write32(&mut t, OFF_EVENTS_LASTTX, 0);
        assert_eq!(read32(&t, OFF_EVENTS_LASTTX), 0, "SW write-0 clears");
    }

    /// Zero-length TX: AMOUNT=0, LASTTX fires, no crash.
    #[test]
    fn twim_tx_zero_length() {
        let mut t = Nrf52Twim::new();
        t.push_slave(Box::new(RecordingDevice::new(0x48, vec![])));
        let mut bus = FlatRam::new();

        write32(&mut t, OFF_ENABLE, 6);
        write32(&mut t, OFF_ADDRESS, 0x48);
        write32(&mut t, OFF_TXD_PTR, 0x2000_0000);
        write32(&mut t, OFF_TXD_MAXCNT, 0);
        write32(&mut t, OFF_TASKS_STARTTX, 1);
        run_leg(&mut t, &mut bus);

        assert_eq!(
            read32(&t, OFF_TXD_AMOUNT),
            0,
            "TXD.AMOUNT = 0 for zero-length TX"
        );
        assert_eq!(
            read32(&t, OFF_EVENTS_LASTTX),
            1,
            "LASTTX fires for zero-length"
        );
    }

    /// Zero-length RX: AMOUNT=0, LASTRX fires.
    #[test]
    fn twim_rx_zero_length() {
        let mut t = Nrf52Twim::new();
        t.push_slave(Box::new(RecordingDevice::new(0x48, vec![])));
        let mut bus = FlatRam::new();

        write32(&mut t, OFF_ENABLE, 6);
        write32(&mut t, OFF_ADDRESS, 0x48);
        write32(&mut t, OFF_RXD_PTR, 0x2000_0100);
        write32(&mut t, OFF_RXD_MAXCNT, 0);
        write32(&mut t, OFF_TASKS_STARTRX, 1);
        run_leg(&mut t, &mut bus);

        assert_eq!(
            read32(&t, OFF_RXD_AMOUNT),
            0,
            "RXD.AMOUNT = 0 for zero-length RX"
        );
        assert_eq!(
            read32(&t, OFF_EVENTS_LASTRX),
            1,
            "LASTRX fires for zero-length"
        );
    }

    /// IRQ raised when INTEN bit is set and event fires.
    #[test]
    fn irq_raised_when_inten_set_and_event_fires() {
        let mut t = Nrf52Twim::new();
        t.push_slave(Box::new(RecordingDevice::new(0x48, vec![])));
        let mut bus = FlatRam::new();
        let tx_base: u64 = 0x2000_0C00;
        bus.write_slice(tx_base, &[0xAB]);

        write32(&mut t, OFF_ENABLE, 6);
        write32(&mut t, OFF_ADDRESS, 0x48);
        write32(&mut t, OFF_TXD_PTR, tx_base as u32);
        write32(&mut t, OFF_TXD_MAXCNT, 1);
        write32(&mut t, OFF_INTENSET, INTEN_LASTTX);
        write32(&mut t, OFF_SHORTS, SHORT_LASTTX_STOP);
        write32(&mut t, OFF_TASKS_STARTTX, 1);
        run_leg(&mut t, &mut bus);

        let result = t.tick();
        assert!(
            result.irq,
            "IRQ must be raised when INTEN_LASTTX set and event fires"
        );
        assert!(
            result.fired_events.contains(&(OFF_EVENTS_LASTTX as u32)),
            "fired_events must contain LASTTX"
        );
    }

    /// INTEN direct write overrides any prior SET/CLR operations.
    #[test]
    fn inten_direct_write() {
        let mut t = Nrf52Twim::new();
        write32(&mut t, OFF_INTENSET, INTEN_MASK); // set all
        write32(&mut t, OFF_INTEN, INTEN_STOPPED); // overwrite
        assert_eq!(read32(&t, OFF_INTEN), INTEN_STOPPED);
    }

    /// needs_bus_tick is false initially and becomes false again after tick.
    #[test]
    fn needs_bus_tick_lifecycle() {
        let mut t = Nrf52Twim::new();
        assert!(!t.needs_bus_tick(), "initially false");
        write32(&mut t, OFF_TASKS_STARTTX, 1);
        assert!(t.needs_bus_tick(), "true after STARTTX");
        let mut bus = FlatRam::new();
        run_leg(&mut t, &mut bus);
        assert!(!t.needs_bus_tick(), "false after tick");
    }
}
