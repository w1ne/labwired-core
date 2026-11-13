//! TI CC2538 flash controller (`FLASH_CTRL`, `0x400D_3000`).
//!
//! Source: SWRU319C §8 "Internal Memory": §8.2 organisation (2-KiB pages,
//! 32-bit write unit), §8.3.1 "Flash Write Procedure", §8.5 "Flash Page
//! Erase", §8.6 / Table 8-3 (CCA page-lock bits), §8.10 / Table 8-10
//! "FLASH_CTRL Register Summary" (`FCTL` @ `0x08`, `FADDR` @ `0x0C`, `FWDATA`
//! @ `0x10`, `DIECFG0`-`DIECFG2` @ `0x14`-`0x1C`). §8.1 gives the timings:
//! page erase 20 ms, word write 20 µs (both minimums).
//!
//! Modelled:
//! - page erase: `FADDR` selects the page, `FCTL.ERASE` starts it, `BUSY`
//!   reads 1 for 20 ms, then the 2-KiB page reads 0xFF;
//! - the serial write sequence of §8.3.1: `FCTL.WRITE` starts it at `FADDR`;
//!   each `FWDATA` store sets `FULL` until the controller takes the word, which
//!   then takes 20 µs to program (NOR rule: `new = old & data`, §8.3). `FADDR`
//!   advances one word per programmed word. The sequence ends (`WRITE` and
//!   `BUSY` clear) when no new word arrives within 20 µs of the buffer
//!   draining, the time-out of §8.3.3/§8.4;
//! - `ERASE` and `WRITE` set together erase first, then write (FCTL bit 0/1);
//! - write/erase protection: a page whose CCA lock bit
//!   (`LOCK_PAGE_N[n]`, Table 8-3) is 0 aborts the operation with
//!   `FCTL.ABORT`; the lock-bit page itself (the last page) needs
//!   `FCTL.UPPER_PAGE_ACCESS` (§8.2.2); the information page is never
//!   writable (§8.2.1), so `SEL_INFO_PAGE` aborts too;
//! - `FADDR` holds a word address: the written byte address is shifted right
//!   by 2 and rolls over at 17 bits (the register description);
//! - `FCTL.CM` reads back the cache mode written; `DIECFG0`-`2` hold the
//!   production values of a CC2538SF53 (see [`DIECFG0_SF53`]).
//!
//! NOT modelled: the CPU stall while executing from flash during an
//! operation (§8.3.1, §8.5.1), the flash DMA trigger (§8.4: no uDMA model),
//! the eight-writes-per-word limit (§8.3.2), mass erase (debug interface
//! only), the flash interrupt, and any cache behaviour of `CM`.
//!
//! Timing runs off the bus cycle clock at `clock_hz`. A controller with no
//! clock attached (a hand-built test bus) completes each step on its next
//! bus tick instead.

use super::lane_read;
use crate::{Bus, CycleClock, Peripheral, SimResult};

const FCTL: u64 = 0x08;
const FADDR: u64 = 0x0C;
const FWDATA: u64 = 0x10;
const DIECFG0: u64 = 0x14;
const DIECFG1: u64 = 0x18;
const DIECFG2: u64 = 0x1C;

const FCTL_ERASE: u32 = 1 << 0;
const FCTL_WRITE: u32 = 1 << 1;
const FCTL_CM_MASK: u32 = 0b11 << 2;
const FCTL_ABORT: u32 = 1 << 5;
const FCTL_FULL: u32 = 1 << 6;
const FCTL_BUSY: u32 = 1 << 7;
const FCTL_SEL_INFO_PAGE: u32 = 1 << 8;
const FCTL_UPPER_PAGE_ACCESS: u32 = 1 << 9;
/// Bits firmware can write: `UPPER_PAGE_ACCESS`, `SEL_INFO_PAGE`, `CM`.
const FCTL_RW_MASK: u32 = FCTL_UPPER_PAGE_ACCESS | FCTL_SEL_INFO_PAGE | FCTL_CM_MASK;
/// `FCTL` reset: `CM` = 01, cache enabled (Table 8-10, 0x0000_0004).
const FCTL_RESET: u32 = 0x4;

/// `FADDR[16:0]`, a word address.
const FADDR_MASK: u32 = 0x1_FFFF;

/// Flash page, the erase unit (§8.2).
pub const PAGE_SIZE: u32 = 2048;
/// Offset of `LOCK_PAGE_N[7:0]` within the lock-bit page (Table 8-3).
const LOCK_BITS_OFFSET: u32 = 2016;

/// `DIECFG0` of a CC2538SF53: `CHIPID` 0xB964, `CLK_SEL_GATE_EN_N` 1,
/// `SRAM_SIZE` 4 (32 KB), `FLASH_SIZE` 4 (512 KB, Table 8-2 "100"),
/// `USB_ENABLE` 1. Table 8-10's 0xB964_0580 is the value at the instant of
/// reset, before the information page is copied in; the size codes are the
/// ones TI's driverlib (`FlashSizeGet`/`FlashSramSizeGet`) decodes as
/// 512 KB / 32 KB. The lock and mass-erase bits stay at their reset 0.
pub const DIECFG0_SF53: u32 = 0xB964_0000 | (1 << 10) | (4 << 7) | (4 << 4) | (1 << 3);
/// `DIECFG1`: every optional block present (I2C, UART0/1, SSI0/1, GPTM0-3).
pub const DIECFG1_SF53: u32 = (1 << 24) | (1 << 17) | (1 << 16) | (1 << 9) | (1 << 8) | 0xF;
/// `DIECFG2`: PG2.0 silicon (`DIE_MAJOR_REVISION` 2), RF core, AES and PKA
/// enabled.
pub const DIECFG2_SF53: u32 = (0x2 << 12) | 0b111;

/// SF53 main array: 0x0020_0000, 512 KiB (Table 8-2).
pub const DEFAULT_FLASH_BASE: u32 = 0x0020_0000;
pub const DEFAULT_FLASH_SIZE: u32 = 512 * 1024;
/// Default timing clock, the CC2538 reset SYS clock (16 MHz).
pub const DEFAULT_CLOCK_HZ: u64 = 16_000_000;

/// Page erase time, 20 ms (§8.1).
const ERASE_US: u64 = 20_000;
/// Word write time, 20 µs (§8.1).
const WRITE_US: u64 = 20;
/// A write sequence ends when `FWDATA` stays empty this long (§8.3.3).
const WRITE_TIMEOUT_US: u64 = 20;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Cc2538FlashCtrl {
    flash_base: u32,
    flash_size: u32,
    clock_hz: u64,
    /// Stored `FCTL` bits (`UPPER_PAGE_ACCESS`, `SEL_INFO_PAGE`, `CM`,
    /// `ABORT`). `ERASE`/`WRITE`/`BUSY`/`FULL` are derived from the state below.
    fctl: u32,
    /// `FADDR`, a word address within the main array.
    faddr: u32,
    /// Cycle at which the running page erase completes.
    erase_done_at: Option<u64>,
    /// A write sequence is open (`FCTL.WRITE`).
    writing: bool,
    /// The word in `FWDATA` the controller has not taken yet (`FCTL.FULL`).
    buffered: Option<u32>,
    /// The word being programmed and the cycle it lands.
    programming: Option<(u32, u64)>,
    /// The write sequence times out at this cycle if nothing is buffered.
    idle_deadline: u64,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl Cc2538FlashCtrl {
    pub fn new(flash_base: u32, flash_size: u32, clock_hz: u64) -> Self {
        Self {
            flash_base,
            flash_size: flash_size.max(PAGE_SIZE),
            clock_hz: clock_hz.max(1),
            fctl: FCTL_RESET,
            faddr: 0,
            erase_done_at: None,
            writing: false,
            buffered: None,
            programming: None,
            idle_deadline: 0,
            clock: None,
        }
    }

    fn now(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    fn us_to_cycles(&self, us: u64) -> u64 {
        if self.clock.is_none() {
            return 0;
        }
        (u128::from(us) * u128::from(self.clock_hz) / 1_000_000) as u64
    }

    /// `FCTL.BUSY`: an erase or a write sequence is running.
    pub fn busy(&self) -> bool {
        self.erase_done_at.is_some() || self.writing
    }

    fn fctl_value(&self) -> u32 {
        let mut v = self.fctl;
        if self.erase_done_at.is_some() {
            v |= FCTL_ERASE;
        }
        if self.writing {
            v |= FCTL_WRITE;
        }
        if self.busy() {
            v |= FCTL_BUSY;
        }
        if self.buffered.is_some() {
            v |= FCTL_FULL;
        }
        v
    }

    fn read_reg(&self, reg: u64) -> u32 {
        match reg {
            FCTL => self.fctl_value(),
            FADDR => self.faddr,
            DIECFG0 => DIECFG0_SF53,
            DIECFG1 => DIECFG1_SF53,
            DIECFG2 => DIECFG2_SF53,
            // FWDATA is write-only (R0/W) and the rest of the window reserved.
            _ => 0,
        }
    }

    fn write_reg(&mut self, reg: u64, value: u32) {
        match reg {
            FCTL => self.write_fctl(value),
            // "Writes to this register will be ignored when any of
            // FCTL.WRITE and FCTL.ERASE is set."
            FADDR if !self.busy() => self.faddr = (value >> 2) & FADDR_MASK,
            // Accepted only inside a write sequence and only while FULL = 0.
            FWDATA if self.writing && self.buffered.is_none() => self.buffered = Some(value),
            _ => {}
        }
    }

    fn write_fctl(&mut self, value: u32) {
        let start_erase = value & FCTL_ERASE != 0;
        let start_write = value & FCTL_WRITE != 0;
        // ERASE/WRITE "are ignored when FCTL.BUSY is 1"; the stored bits land
        // either way (driverlib changes CM with a read-modify-write).
        let idle = !self.busy();
        self.fctl = (self.fctl & !FCTL_RW_MASK) | (value & FCTL_RW_MASK);
        if !idle || !(start_erase || start_write) {
            return;
        }
        // ABORT is "cleared when a write or page erase is started".
        self.fctl &= !FCTL_ABORT;
        let now = self.now();
        if start_erase {
            self.erase_done_at = Some(now + self.us_to_cycles(ERASE_US));
        }
        if start_write {
            self.writing = true;
            self.buffered = None;
            self.programming = None;
            // The time-out runs from when the controller could first take a
            // word: after the erase, when one goes first.
            let from = self.erase_done_at.unwrap_or(now);
            self.idle_deadline = from + self.us_to_cycles(WRITE_TIMEOUT_US);
        }
    }

    /// Byte address of the word `FADDR` points at.
    fn word_addr(&self) -> u64 {
        u64::from(self.flash_base) + u64::from((self.faddr * 4) % self.flash_size)
    }

    fn page_of(&self, addr: u64) -> u32 {
        ((addr - u64::from(self.flash_base)) / u64::from(PAGE_SIZE)) as u32
    }

    /// Whether a write or erase of `page` is allowed (§8.2.1, §8.2.2, §8.6).
    fn page_writable(&self, page: u32, bus: &mut dyn Bus) -> bool {
        if self.fctl & FCTL_SEL_INFO_PAGE != 0 {
            return false;
        }
        let last = self.flash_size / PAGE_SIZE - 1;
        if page == last {
            return self.fctl & FCTL_UPPER_PAGE_ACCESS != 0;
        }
        let lock_page = u64::from(self.flash_base + last * PAGE_SIZE);
        let byte = lock_page + u64::from(LOCK_BITS_OFFSET + page / 8);
        // LOCK_PAGE_N[n] = 0 blocks write/erase.
        bus.read_u8(byte)
            .map_or(true, |b| b & (1 << (page % 8)) != 0)
    }

    fn abort(&mut self) {
        self.fctl |= FCTL_ABORT;
        self.erase_done_at = None;
        self.writing = false;
        self.buffered = None;
        self.programming = None;
    }

    /// Run every step whose time has come.
    fn advance(&mut self, bus: &mut dyn Bus) {
        let now = self.now();
        if let Some(done) = self.erase_done_at {
            if now < done {
                return;
            }
            let addr = self.word_addr() & !u64::from(PAGE_SIZE - 1);
            if !self.page_writable(self.page_of(addr), bus) {
                self.abort();
                return;
            }
            for w in 0..u64::from(PAGE_SIZE / 4) {
                let _ = bus.write_u32(addr + w * 4, 0xFFFF_FFFF);
            }
            self.erase_done_at = None;
            tracing::debug!("CC2538 FLASH_CTRL: erased page at 0x{addr:08X}");
        }
        if !self.writing {
            return;
        }
        if let Some((data, done)) = self.programming {
            if now < done {
                return;
            }
            let addr = self.word_addr();
            let old = bus.read_u32(addr).unwrap_or(0xFFFF_FFFF);
            let _ = bus.write_u32(addr, old & data);
            self.faddr = (self.faddr + 1) & FADDR_MASK;
            self.programming = None;
            self.idle_deadline = done + self.us_to_cycles(WRITE_TIMEOUT_US);
        }
        if let Some(data) = self.buffered.take() {
            if !self.page_writable(self.page_of(self.word_addr()), bus) {
                self.abort();
                return;
            }
            let done = now + self.us_to_cycles(WRITE_US);
            self.programming = Some((data, done));
            if self.clock.is_none() {
                // No time base: land the word on this tick.
                self.advance(bus);
            }
            return;
        }
        if now >= self.idle_deadline && self.clock.is_some() {
            self.writing = false;
        }
    }
}

impl Peripheral for Cc2538FlashCtrl {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(lane_read(self.read_reg(offset & !3), offset))
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        Some(lane_read(self.read_reg(offset & !3), offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) as u32 * 8;
        let merged = (self.read_reg(reg) & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_reg(reg, merged);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    /// The array changes in `tick_with_bus`, which the bus runs in its tick
    /// pass in every lane, only while an operation is open.
    fn needs_bus_tick(&self) -> bool {
        self.busy()
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        self.advance(bus);
    }

    /// No `tick()`: all work happens in `tick_with_bus`.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn on_node_reset(&mut self) {
        let clock = self.clock.take();
        *self = Self::new(self.flash_base, self.flash_size, self.clock_hz);
        self.clock = clock;
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    snapshot_impl!(Cc2538FlashCtrl, "Cc2538FlashCtrl", keep[clock]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl() -> Cc2538FlashCtrl {
        Cc2538FlashCtrl::new(DEFAULT_FLASH_BASE, DEFAULT_FLASH_SIZE, DEFAULT_CLOCK_HZ)
    }

    #[test]
    fn registers_reset_to_table_8_10() {
        let c = ctrl();
        assert_eq!(c.read_u32(FCTL).unwrap(), 0x4, "CM = 01, idle");
        assert_eq!(c.read_u32(FADDR).unwrap(), 0);
        assert_eq!(c.read_u32(FWDATA).unwrap(), 0, "FWDATA reads 0");
        assert!(
            !c.needs_bus_tick(),
            "an idle controller must not ask for a bus tick"
        );
    }

    #[test]
    fn diecfg_describes_a_cc2538sf53() {
        let c = ctrl();
        let d0 = c.read_u32(DIECFG0).unwrap();
        assert_eq!(d0 >> 16, 0xB964, "CHIPID");
        assert_eq!((d0 >> 4) & 7, 4, "FLASH_SIZE 100 = 512 KB (Table 8-2)");
        assert_eq!((d0 >> 7) & 7, 4, "SRAM_SIZE 32 KB");
        let d2 = c.read_u32(DIECFG2).unwrap();
        assert_eq!((d2 >> 12) & 0xF, 2, "PG2.0");
        assert_eq!(d2 & 0b111, 0b111, "RF core, AES, PKA");
    }

    #[test]
    fn faddr_reads_back_the_word_address_and_rolls_over() {
        let mut c = ctrl();
        c.write_u32(FADDR, 0x0000_4008).unwrap();
        assert_eq!(c.read_u32(FADDR).unwrap(), 0x4008 >> 2);
        // An absolute flash address loses the bits above FADDR[16:0].
        c.write_u32(FADDR, 0x0020_4008).unwrap();
        assert_eq!(c.read_u32(FADDR).unwrap(), (0x0020_4008 >> 2) & 0x1_FFFF);
        assert_eq!(c.word_addr(), 0x0020_4008);
    }

    #[test]
    fn fwdata_is_ignored_outside_a_write_sequence() {
        let mut c = ctrl();
        c.write_u32(FWDATA, 0x1234_5678).unwrap();
        assert_eq!(c.read_u32(FCTL).unwrap() & FCTL_FULL, 0);
        assert!(!c.busy());
    }

    #[test]
    fn write_sets_busy_and_fwdata_sets_full_once() {
        let mut c = ctrl();
        c.write_u32(FADDR, 0x100).unwrap();
        c.write_u32(FCTL, FCTL_RESET | FCTL_WRITE).unwrap();
        let fctl = c.read_u32(FCTL).unwrap();
        assert_eq!(fctl & (FCTL_WRITE | FCTL_BUSY), FCTL_WRITE | FCTL_BUSY);
        assert!(c.needs_bus_tick());
        c.write_u32(FWDATA, 0xAAAA_5555).unwrap();
        assert_ne!(c.read_u32(FCTL).unwrap() & FCTL_FULL, 0);
        // A second word while FULL = 1 is dropped (§8.10 FWDATA).
        c.write_u32(FWDATA, 0).unwrap();
        assert_eq!(c.buffered, Some(0xAAAA_5555));
        // FADDR is frozen while the sequence runs.
        c.write_u32(FADDR, 0x800).unwrap();
        assert_eq!(c.read_u32(FADDR).unwrap(), 0x100 >> 2);
    }

    #[test]
    fn erase_and_write_are_ignored_while_busy_but_cm_still_lands() {
        let mut c = ctrl();
        c.write_u32(FCTL, FCTL_RESET | FCTL_ERASE).unwrap();
        assert!(c.erase_done_at.is_some());
        c.write_u32(FCTL, (0b10 << 2) | FCTL_WRITE).unwrap();
        assert!(!c.writing, "WRITE ignored while BUSY");
        assert_eq!(c.read_u32(FCTL).unwrap() & FCTL_CM_MASK, 0b10 << 2);
    }

    #[test]
    fn status_bits_are_read_only() {
        let mut c = ctrl();
        c.write_u32(FCTL, FCTL_RESET | FCTL_ABORT | FCTL_FULL | FCTL_BUSY)
            .unwrap();
        assert_eq!(c.read_u32(FCTL).unwrap(), FCTL_RESET);
    }

    #[test]
    fn node_reset_returns_to_idle() {
        let mut c = ctrl();
        c.write_u32(FCTL, FCTL_RESET | FCTL_ERASE | FCTL_UPPER_PAGE_ACCESS)
            .unwrap();
        c.on_node_reset();
        assert_eq!(c.read_u32(FCTL).unwrap(), FCTL_RESET);
        assert!(!c.busy());
    }
}
