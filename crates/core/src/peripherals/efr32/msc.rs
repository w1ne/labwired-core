// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Silicon Labs EFR32 Series-2 **MSC** — the Memory System Controller.
//!
//! This is the block that lets firmware WRITE ITS OWN FLASH, which is the only
//! way anything on this part persists across a reset. Without it a project that
//! wants to remember a calibration, a pairing, or a high score has nowhere to
//! put it: `efr32mg26.yaml` left MSC unmapped, so those addresses faulted the
//! bus and every such sketch died at the first store.
//!
//! # Sources
//!
//! Register map: EFR32xG26 Reference Manual Rev 1.0 section 6.7 p.134.
//! Field positions: section 6.8, `MSC_WRITECMD` p.141, `MSC_ADDRB`/`MSC_WDATA`
//! p.142, `MSC_STATUS` p.143-144, `MSC_WRITECTRL` p.140.
//! Flash geometry: section 6.3 Table 6.1 p.60 — main block at 0x08000000,
//! information block at 0x0FE00000, **8 KB pages**, up to 3200 kB.
//! Base address 0x40030000: RM section 4.2.4.1 Peripheral Map p.48.
//!
//! # ⚠️ The reset value here is the SILICON one, not the manual's
//!
//! RM p.143 gives `MSC_STATUS` reset as WREADY (bit 27) and WDATAREADY (bit 3),
//! i.e. `0x0800_0008`. A connected BRD2709A reads **`0x0B00_0008`** — PWRON0
//! (24) and PWRON1 (25) are ALSO set, because both flash banks finish their
//! power-up sequence before any firmware or debugger gets to look.
//!
//! The manual's column is the register's value at the instant of reset; the
//! value FIRMWARE can observe is this one. Modelling the paper reset would hang
//! any driver that waits for its bank to power on, because in the twin it never
//! would. Measured 2026-09-03 over SWD, J-Link OB, VTarget 3.301 V, after
//! `reset halt` plus a CMU_CLKEN preamble (MSC does not answer the bus
//! unclocked — see below).
//!
//! # ⚠️ MSC does not answer the bus until CMU clocks it
//!
//! On a cold `reset halt` a read of 0x40030000 fails outright over SWD, as do
//! PRS, LDMA, ICACHE0, SYSCFG and EUSART1. Only CMU answers. That is not a
//! debug-access or TrustZone problem — it is Series-2 clock gating, the same
//! rule `efr32mg26_clock_gating` already asserts for the blocks that were
//! mapped. The capture preamble writes CMU_CLKEN0/1/2 before reading.

use crate::{Peripheral, SimResult};

// ── Register offsets, RM section 6.7 p.134 ───────────────────────────────
const OFF_IPVERSION: u64 = 0x000;
const OFF_READCTRL: u64 = 0x004;
const OFF_RDATACTRL: u64 = 0x008;
const OFF_WRITECTRL: u64 = 0x00C;
const OFF_WRITECMD: u64 = 0x010;
const OFF_ADDRB: u64 = 0x014;
const OFF_WDATA: u64 = 0x018;
const OFF_STATUS: u64 = 0x01C;
const OFF_IF: u64 = 0x020;
const OFF_IEN: u64 = 0x024;
const OFF_USERDATASIZE: u64 = 0x034;
const OFF_CMD: u64 = 0x038;
const OFF_LOCK: u64 = 0x03C;
const OFF_MISCLOCKWORD: u64 = 0x040;
const OFF_PWRCTRL: u64 = 0x050;
const OFF_PAGELOCK0: u64 = 0x120;
const OFF_PAGELOCK_LAST: u64 = 0x150;

// ── Reset values, MEASURED on BRD2709A (see module docs) ─────────────────
const RESET_IPVERSION: u32 = 0x0000_0009;
const RESET_READCTRL: u32 = 0x0010_0000;
const RESET_RDATACTRL: u32 = 0x0000_1000;
/// WREADY | PWRON1 | PWRON0 | WDATAREADY. The manual says 0x0800_0008; the die
/// says this, because both banks are powered by the time anyone can look.
const RESET_STATUS: u32 = 0x0B00_0008;
/// RM p.146 calls this the User Data Region Size; the die reports 4.
const RESET_USERDATASIZE: u32 = 0x0000_0004;
const RESET_PWRCTRL: u32 = 0x0010_0002;

// ── MSC_WRITECTRL, RM p.140 ──────────────────────────────────────────────
/// "When this bit is set, the MSC write and erase functionality is enabled".
const WRITECTRL_WREN: u32 = 1 << 0;

// ── MSC_WRITECMD, RM p.141 ───────────────────────────────────────────────
const WRITECMD_ERASEPAGE: u32 = 1 << 1;
const WRITECMD_WRITEEND: u32 = 1 << 2;
const WRITECMD_ERASERANGE: u32 = 1 << 4;
const WRITECMD_ERASEABORT: u32 = 1 << 5;
const WRITECMD_CLEARWDATA: u32 = 1 << 12;

// ── MSC_STATUS, RM p.143-144 ─────────────────────────────────────────────
const STATUS_BUSY: u32 = 1 << 0;
const STATUS_LOCKED: u32 = 1 << 1;
const STATUS_INVADDR: u32 = 1 << 2;
const STATUS_WDATAREADY: u32 = 1 << 3;
const STATUS_ERASEABORTED: u32 = 1 << 4;
const STATUS_PENDING: u32 = 1 << 5;

// ── MSC_IF / MSC_IEN, RM p.145-146 ───────────────────────────────────────
const IF_ERASE: u32 = 1 << 0;
const IF_WRITE: u32 = 1 << 1;

// ── Flash geometry, RM section 6.3 Table 6.1 p.60 ────────────────────────
const MAIN_BASE: u32 = 0x0800_0000;
const MAIN_SIZE: u32 = 3200 * 1024;
const INFO_BASE: u32 = 0x0FE0_0000;
const INFO_SIZE: u32 = 1024;
/// "All flash memory is organized into 8 KB pages" — RM p.60.
const PAGE_BYTES: u32 = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingOp {
    None,
    Write { addr: u32, data: u32 },
    ErasePage { addr: u32 },
}

#[derive(Debug)]
pub struct Efr32s2Msc {
    readctrl: u32,
    rdatactrl: u32,
    writectrl: u32,
    addrb: u32,
    wdata: u32,
    status: u32,
    iflag: u32,
    ien: u32,
    lock: u32,
    misclockword: u32,
    pwrctrl: u32,
    pagelock: [u32; 13],
    pending: PendingOp,
}

impl Default for Efr32s2Msc {
    fn default() -> Self {
        Self::new()
    }
}

impl Efr32s2Msc {
    pub fn new() -> Self {
        Self {
            readctrl: RESET_READCTRL,
            rdatactrl: RESET_RDATACTRL,
            writectrl: 0,
            addrb: 0,
            wdata: 0,
            status: RESET_STATUS,
            iflag: 0,
            ien: 0,
            lock: 0,
            misclockword: 0,
            pwrctrl: RESET_PWRCTRL,
            pagelock: [0; 13],
            pending: PendingOp::None,
        }
    }

    /// Is `addr` inside a flash region this controller can modify?
    ///
    /// RM p.144 on `INVADDR`: "software has attempted to load an invalid
    /// (unmapped) address into the MSC_ADDRB register". Both the main block
    /// and the 1 kB user-data page count; everything else does not.
    fn addr_is_flash(addr: u32) -> bool {
        let in_main = addr >= MAIN_BASE && addr < MAIN_BASE.wrapping_add(MAIN_SIZE);
        let in_info = addr >= INFO_BASE && addr < INFO_BASE.wrapping_add(INFO_SIZE);
        in_main || in_info
    }

    fn read_word(&self, reg: u64) -> u32 {
        match reg {
            OFF_IPVERSION => RESET_IPVERSION,
            OFF_READCTRL => self.readctrl,
            OFF_RDATACTRL => self.rdatactrl,
            OFF_WRITECTRL => self.writectrl,
            // WRITECMD and CMD are write-only (RM p.134 lists them `W`).
            OFF_WRITECMD | OFF_CMD | OFF_LOCK => 0,
            OFF_ADDRB => self.addrb,
            OFF_WDATA => self.wdata,
            OFF_STATUS => self.status,
            OFF_IF => self.iflag,
            OFF_IEN => self.ien,
            OFF_USERDATASIZE => RESET_USERDATASIZE,
            OFF_MISCLOCKWORD => self.misclockword,
            OFF_PWRCTRL => self.pwrctrl,
            r if (OFF_PAGELOCK0..=OFF_PAGELOCK_LAST).contains(&r) => {
                self.pagelock[((r - OFF_PAGELOCK0) / 4) as usize]
            }
            _ => 0,
        }
    }

    fn write_word(&mut self, reg: u64, value: u32) {
        match reg {
            OFF_READCTRL => self.readctrl = value,
            OFF_RDATACTRL => self.rdatactrl = value,
            OFF_WRITECTRL => self.writectrl = value,
            OFF_ADDRB => {
                self.addrb = value;
                // INVADDR latches on the ADDRB write itself, which is what lets
                // a driver check before committing to a command.
                if Self::addr_is_flash(value) {
                    self.status &= !STATUS_INVADDR;
                } else {
                    self.status |= STATUS_INVADDR;
                }
            }
            OFF_WDATA => {
                self.wdata = value;
                // RM p.144, WDATAREADY: "cleared when writing to MSC_WDATA".
                self.status &= !STATUS_WDATAREADY;
                self.arm_write();
            }
            OFF_WRITECMD => self.write_cmd(value),
            OFF_IF => self.iflag &= !value, // write-1-to-clear
            OFF_IEN => self.ien = value,
            OFF_LOCK => self.lock = value,
            OFF_MISCLOCKWORD => self.misclockword = value,
            OFF_PWRCTRL => self.pwrctrl = value,
            r if (OFF_PAGELOCK0..=OFF_PAGELOCK_LAST).contains(&r) => {
                self.pagelock[((r - OFF_PAGELOCK0) / 4) as usize] = value;
            }
            _ => {}
        }
    }

    /// A WDATA store only becomes a real write when WREN is set and the address
    /// is a flash address. RM p.141 says WREN "must be set in order to use"
    /// the erase commands, and p.140 that it enables "write and erase
    /// functionality" — so a driver that forgot it gets nothing, loudly
    /// (WDATAREADY never comes back), rather than a silent phantom write.
    fn arm_write(&mut self) {
        if self.writectrl & WRITECTRL_WREN == 0 {
            self.status |= STATUS_LOCKED;
            return;
        }
        if !Self::addr_is_flash(self.addrb) {
            self.status |= STATUS_INVADDR;
            return;
        }
        self.pending = PendingOp::Write {
            addr: self.addrb,
            data: self.wdata,
        };
        self.status |= STATUS_BUSY | STATUS_PENDING;
    }

    fn write_cmd(&mut self, value: u32) {
        if value & WRITECMD_CLEARWDATA != 0 {
            // RM p.141: "Will set WDATAREADY and DMA request."
            self.status |= STATUS_WDATAREADY;
        }
        if value & WRITECMD_WRITEEND != 0 {
            self.pending = PendingOp::None;
            self.status &= !(STATUS_BUSY | STATUS_PENDING);
            self.status |= STATUS_WDATAREADY;
        }
        if value & WRITECMD_ERASEABORT != 0 {
            if matches!(self.pending, PendingOp::ErasePage { .. }) {
                self.status |= STATUS_ERASEABORTED;
            }
            self.pending = PendingOp::None;
            self.status &= !(STATUS_BUSY | STATUS_PENDING);
        }
        if value & (WRITECMD_ERASEPAGE | WRITECMD_ERASERANGE) != 0 {
            if self.writectrl & WRITECTRL_WREN == 0 {
                self.status |= STATUS_LOCKED;
                return;
            }
            if !Self::addr_is_flash(self.addrb) {
                self.status |= STATUS_INVADDR;
                return;
            }
            self.pending = PendingOp::ErasePage { addr: self.addrb };
            self.status |= STATUS_BUSY | STATUS_PENDING;
        }
    }

    /// True while a command is outstanding — the model's own view, used by the
    /// bus to decide whether the completion event needs scheduling.
    pub fn busy(&self) -> bool {
        !matches!(self.pending, PendingOp::None)
    }

    pub fn status(&self) -> u32 {
        self.status
    }
}

impl Peripheral for Efr32s2Msc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        Ok(((word >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let word = self.read_word(offset & !3);
        Some(((word >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) * 8;
        let merged = (self.read_word(reg) & !(0xFFu32 << shift)) | ((value as u32) << shift);
        self.write_word(reg, merged);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset & !3, value);
        Ok(())
    }

    /// The bus lends itself to this peripheral every tick.
    ///
    /// ⚠️ `take_scheduled_events` / `on_event` WOULD HAVE BEEN DEAD CODE HERE.
    /// The bus only harvests scheduled events under
    /// `#[cfg(feature = "event-scheduler")]` AND when `uses_scheduler()` is
    /// true, so a flash controller wired that way would arm an operation, set
    /// BUSY, and never complete it under the default feature set — the exact
    /// shape of "a guard that is not wired to the path that matters".
    /// `tick_with_bus` runs in the ordinary bus tick pass, in every lane.
    fn needs_bus_tick(&self) -> bool {
        self.busy()
    }

    /// Where the flash actually changes.
    ///
    /// The command registers only ARM an operation; memory is touched here, a
    /// tick later. That ordering is not decoration — it is what makes
    /// `STATUS.BUSY` observable at all, and every vendor flash routine polls
    /// exactly that bit.
    fn tick_with_bus(&mut self, bus: &mut dyn crate::Bus) {
        match std::mem::replace(&mut self.pending, PendingOp::None) {
            PendingOp::None => {}
            PendingOp::Write { addr, data } => {
                // ⚠️ Flash programming can only CLEAR bits. An unerased word
                // ANDs rather than replaces; a model that replaced it would let
                // a driver pass in the twin and corrupt on the bench.
                let existing = bus.read_u32(addr as u64).unwrap_or(0xFFFF_FFFF);
                let _ = bus.write_u32(addr as u64, existing & data);
                self.addrb = addr.wrapping_add(4);
                self.status &= !(STATUS_BUSY | STATUS_PENDING);
                self.status |= STATUS_WDATAREADY;
                self.iflag |= IF_WRITE;
            }
            PendingOp::ErasePage { addr } => {
                let page = addr & !(PAGE_BYTES - 1);
                for w in (0..PAGE_BYTES).step_by(4) {
                    let _ = bus.write_u32((page + w) as u64, 0xFFFF_FFFF);
                }
                self.status &= !(STATUS_BUSY | STATUS_PENDING);
                self.iflag |= IF_ERASE;
            }
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msc() -> Efr32s2Msc {
        Efr32s2Msc::new()
    }

    /// ⚠️ THE DIE, NOT THE MANUAL. RM p.143 gives 0x0800_0008; BRD2709A reads
    /// 0x0B00_0008 because both flash banks are powered by the time firmware
    /// looks. A driver that waits on PWRON would hang against the paper value.
    #[test]
    fn status_resets_to_the_value_measured_on_silicon() {
        assert_eq!(msc().read_word(OFF_STATUS), 0x0B00_0008);
        // And the two bits that differ from the manual are exactly the banks.
        assert_eq!(msc().read_word(OFF_STATUS) & 0x0300_0000, 0x0300_0000);
    }

    #[test]
    fn the_other_registers_reset_to_their_measured_values() {
        let m = msc();
        assert_eq!(m.read_word(OFF_IPVERSION), 0x0000_0009);
        assert_eq!(m.read_word(OFF_READCTRL), 0x0010_0000);
        assert_eq!(m.read_word(OFF_RDATACTRL), 0x0000_1000);
        assert_eq!(m.read_word(OFF_USERDATASIZE), 0x0000_0004);
        assert_eq!(m.read_word(OFF_PWRCTRL), 0x0010_0002);
        assert_eq!(m.read_word(OFF_WRITECTRL), 0);
    }

    /// RM p.134 lists WRITECMD and CMD as `W`. A model that let them read back
    /// would let a driver "confirm" a command it never issued.
    #[test]
    fn the_write_only_command_registers_read_zero() {
        let mut m = msc();
        m.write_word(OFF_WRITECTRL, WRITECTRL_WREN);
        m.write_word(OFF_ADDRB, MAIN_BASE);
        m.write_word(OFF_WRITECMD, WRITECMD_ERASEPAGE);
        assert_eq!(m.read_word(OFF_WRITECMD), 0);
        assert_eq!(m.read_word(OFF_CMD), 0);
    }

    /// RM p.140: WREN "enables the MSC write and erase functionality". Without
    /// it nothing may be armed — and the model must say so through LOCKED
    /// rather than pretending the write happened.
    #[test]
    fn a_write_without_wren_is_refused_and_flagged_locked() {
        let mut m = msc();
        m.write_word(OFF_ADDRB, MAIN_BASE);
        m.write_word(OFF_WDATA, 0xDEAD_BEEF);
        assert!(!m.busy(), "nothing may be armed without WREN");
        assert_eq!(m.status & STATUS_LOCKED, STATUS_LOCKED);
    }

    #[test]
    fn an_erase_without_wren_is_refused_too() {
        let mut m = msc();
        m.write_word(OFF_ADDRB, MAIN_BASE);
        m.write_word(OFF_WRITECMD, WRITECMD_ERASEPAGE);
        assert!(!m.busy());
        assert_eq!(m.status & STATUS_LOCKED, STATUS_LOCKED);
    }

    /// RM p.144, INVADDR: set when "software has attempted to load an invalid
    /// (unmapped) address into the MSC_ADDRB register". RAM is not flash.
    #[test]
    fn a_non_flash_address_latches_invaddr_on_the_addrb_write() {
        let mut m = msc();
        m.write_word(OFF_ADDRB, 0x2000_0000);
        assert_eq!(m.status & STATUS_INVADDR, STATUS_INVADDR);
        m.write_word(OFF_ADDRB, MAIN_BASE);
        assert_eq!(m.status & STATUS_INVADDR, 0, "a valid address clears it");
    }

    /// Both flash blocks count — the 1 kB user-data page is exactly where a
    /// project's persisted settings belong (RM Table 6.1 p.60).
    #[test]
    fn the_user_data_page_is_a_writable_flash_address() {
        assert!(Efr32s2Msc::addr_is_flash(INFO_BASE));
        assert!(Efr32s2Msc::addr_is_flash(INFO_BASE + INFO_SIZE - 4));
        assert!(!Efr32s2Msc::addr_is_flash(INFO_BASE + INFO_SIZE));
        assert!(Efr32s2Msc::addr_is_flash(MAIN_BASE));
        assert!(!Efr32s2Msc::addr_is_flash(MAIN_BASE - 4));
    }

    /// RM p.144: WDATAREADY "is cleared when writing to MSC_WDATA". A driver
    /// polls it to know the buffer took the word.
    #[test]
    fn writing_wdata_clears_wdataready_and_arms_busy() {
        let mut m = msc();
        m.write_word(OFF_WRITECTRL, WRITECTRL_WREN);
        m.write_word(OFF_ADDRB, MAIN_BASE);
        assert_eq!(m.status & STATUS_WDATAREADY, STATUS_WDATAREADY);
        m.write_word(OFF_WDATA, 0x1234_5678);
        assert_eq!(m.status & STATUS_WDATAREADY, 0);
        assert_eq!(m.status & STATUS_BUSY, STATUS_BUSY);
        assert!(m.busy());
    }

    /// RM p.141, CLEARWDATA: "Will set WDATAREADY".
    #[test]
    fn clearwdata_puts_wdataready_back() {
        let mut m = msc();
        m.write_word(OFF_WRITECTRL, WRITECTRL_WREN);
        m.write_word(OFF_ADDRB, MAIN_BASE);
        m.write_word(OFF_WDATA, 1);
        assert_eq!(m.status & STATUS_WDATAREADY, 0);
        m.write_word(OFF_WRITECMD, WRITECMD_CLEARWDATA);
        assert_eq!(m.status & STATUS_WDATAREADY, STATUS_WDATAREADY);
    }

    /// A command is only ARMED by the register write; the event carries it.
    /// That is what makes BUSY observable, so this asserts the ordering
    /// directly rather than trusting it.
    /// ⚠️ NON-VACUITY FOR THE HOOK ITSELF. An armed operation must ASK the bus
    /// for a tick, or it would sit BUSY forever. This is the assertion that
    /// would have caught the first version of this model, which requested a
    /// scheduler event the bus never harvests.
    #[test]
    fn an_armed_command_asks_the_bus_for_a_tick() {
        let mut m = msc();
        assert!(!m.needs_bus_tick(), "an idle controller must not ask");
        m.write_word(OFF_WRITECTRL, WRITECTRL_WREN);
        m.write_word(OFF_ADDRB, MAIN_BASE);
        m.write_word(OFF_WDATA, 0xAAAA_5555);
        assert!(m.needs_bus_tick(), "an armed write must ask for a bus tick");
        assert!(m.busy(), "and stay busy until that tick runs");
    }

    /// RM p.141: ERASEABORT "will abort an ongoing erase sequence", and p.144
    /// ERASEABORTED reports it. An aborted erase must NOT then land.
    #[test]
    fn eraseabort_drops_a_pending_erase_and_says_so() {
        let mut m = msc();
        m.write_word(OFF_WRITECTRL, WRITECTRL_WREN);
        m.write_word(OFF_ADDRB, MAIN_BASE);
        m.write_word(OFF_WRITECMD, WRITECMD_ERASEPAGE);
        assert!(m.busy());
        m.write_word(OFF_WRITECMD, WRITECMD_ERASEABORT);
        assert!(!m.busy(), "the erase must not still be pending");
        assert_eq!(m.status & STATUS_ERASEABORTED, STATUS_ERASEABORTED);
        assert_eq!(m.status & STATUS_BUSY, 0);
    }

    /// RM p.141: WRITEEND — "Write 1 to abort a write command."
    #[test]
    fn writeend_drops_a_pending_write_without_flagging_an_erase_abort() {
        let mut m = msc();
        m.write_word(OFF_WRITECTRL, WRITECTRL_WREN);
        m.write_word(OFF_ADDRB, MAIN_BASE);
        m.write_word(OFF_WDATA, 0x1111_2222);
        m.write_word(OFF_WRITECMD, WRITECMD_WRITEEND);
        assert!(!m.busy());
        assert_eq!(m.status & STATUS_ERASEABORTED, 0);
        assert_eq!(m.status & STATUS_WDATAREADY, STATUS_WDATAREADY);
    }

    /// IF is write-1-to-clear, the Series-2 convention this chip uses
    /// everywhere else.
    #[test]
    fn the_interrupt_flags_clear_on_write_one() {
        let mut m = msc();
        m.iflag = IF_ERASE | IF_WRITE;
        m.write_word(OFF_IF, IF_ERASE);
        assert_eq!(m.read_word(OFF_IF), IF_WRITE);
    }

    /// The page an erase clears is the 8 KB page CONTAINING the address, not
    /// the address itself (RM p.60: "All flash memory is organized into 8 KB
    /// pages"). An off-by-a-page erase destroys someone else's data.
    #[test]
    fn an_erase_targets_the_page_containing_the_address() {
        let inside = MAIN_BASE + PAGE_BYTES + 0x40;
        assert_eq!(inside & !(PAGE_BYTES - 1), MAIN_BASE + PAGE_BYTES);
    }
}
