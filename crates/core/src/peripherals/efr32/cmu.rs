// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Silicon Labs EFR32 Series-2 **CMU** — Clock Management Unit.
//!
//! # Why this is a model and not a stub
//!
//! On Series 2 a peripheral's APB interface is dead until its `CLKEN` bit is
//! set: reads return zero and writes are dropped. Every Silicon Labs driver
//! therefore opens with a clock enable (`CMU_ClockEnable`, or in the Gecko SDK
//! `CMU->CLKEN0_SET = CMU_CLKEN0_GPIO`) before it touches anything.
//!
//! A stub CMU accepts those writes and reads back zero, which has two costs.
//! The first is that firmware polling a clock's status spins forever. The
//! second is worse and quieter: with no live CLKEN state, the engine cannot
//! gate anything, so **a peripheral answers whether or not firmware clocked
//! it**. That is a permissive model — it hides exactly the bug class (a
//! forgotten clock enable) that costs the most bench time on this family,
//! because on silicon the symptom is a peripheral that reads back all zeroes
//! rather than a fault.
//!
//! With this model the three `CLKEN` registers hold real state, and the
//! peripherals that declare a `clock:` gate in the chip yaml are dropped by
//! [`crate::bus::SystemBus::is_peripheral_clocked`] until firmware enables
//! them — the same contract the STM32 RCC has had.
//!
//! # Sources
//!
//! Register offsets, reset values and `CLKEN` bit assignments are all derived
//! from the vendor CMSIS header `efr32mg26_cmu.h` (`simplicity_sdk` tag
//! `sisdk-2025.6`), by walking `CMU_TypeDef` — not recalled. `CLKEN0` at
//! `+0x64` cross-checks against the address `0x4000_8064` that the
//! `firmware-mg26-demo` bring-up (which runs on the physical BRD2709A) writes.
//!
//! # Faithfully modelled
//!
//! * `CLKEN0/1/2` as read/write state, and therefore real clock gating for
//!   every peripheral that declares the matching `clock:` bit.
//! * The documented reset values, including the ones that are not zero
//!   (`IPVERSION` 0x7, `SYSCLKCTRL` 0x1, the `*CLKCTRL` selectors at 0x1,
//!   `LOCK` at the unlock key 0x93F7 — i.e. the CMU boots unlocked).
//! * `LOCK`/`WDOGLOCK`: writing anything but the unlock key (`0x93F7`, the
//!   same key for both) locks the block, and while locked, writes to the
//!   configuration registers are dropped — which is what silicon does and what
//!   `CMU_Lock()` exists to cause. `STATUS.LOCK` (bit 31) and
//!   `STATUS.WDOGLOCK` (bit 30) report it.
//! * The asymmetry between the two: `LOCK` resets to the unlock key, so the
//!   CMU boots **unlocked**, while `WDOGLOCK` resets to `0x5257`, so the
//!   watchdog clock configuration boots **locked** and firmware must unlock it
//!   before `WDOG0CLKCTRL` will take a write (`_CMU_WDOGLOCK_LOCKKEY_DEFAULT`
//!   vs `_CMU_WDOGLOCK_LOCKKEY_UNLOCK`). Modelling both as "unlocked at reset"
//!   would let firmware configure the watchdog clock in the twin and silently
//!   fail to on the bench.
//! * Read-only registers (`IPVERSION`, `STATUS`, `CALCNT`) ignore writes.
//!
//! # Idealised — present, but not physical
//!
//! * **No clock tree.** `SYSCLKCTRL`, the `*CLKCTRL` selectors and
//!   `DPLLREFCLKCTRL` store what firmware writes and change nothing: the
//!   simulated core always runs at the descriptor's `cpu_hz` and no
//!   peripheral's timebase is derived from a selector. Firmware that switches
//!   the system clock source runs, and runs at the same speed as before.
//! * **No oscillator state machine.** There are no HFXO/LFXO/DPLL ready
//!   flags to wait on here (those live in their own peripheral blocks, which
//!   this chip does not map yet), and `STATUS.CALRDY` never sets.
//! * **Calibration is inert.** `CALCMD`/`CALCTRL` store; `CALCNT` reads 0.
//! * **No CMU interrupt.** `IF`/`IEN` are plain state; nothing ever sets a
//!   flag, so the CMU IRQ (63) never fires.
//! * **`LOCK` read-back is the stored key.** The header documents the reset
//!   value and the unlock code but not what a locked `LOCK` reads as, and
//!   there is no bench capture for it, so the model returns what was written
//!   rather than inventing a masked value.
//! * Offsets inside the 4 KiB window that `CMU_TypeDef` marks reserved read 0
//!   and swallow writes, rather than faulting. Series-2 firmware does not
//!   touch them, and faulting would turn a harmless struct-wide memset into a
//!   crash the silicon does not have.

use crate::SimResult;

/// `CMU_LOCK_LOCKKEY_UNLOCK` — writing this unlocks, anything else locks.
const UNLOCK_KEY: u32 = 0x0000_93F7;

// ── Register offsets, walked from `CMU_TypeDef` in efr32mg26_cmu.h ──────────
const OFF_IPVERSION: u64 = 0x000;
const OFF_STATUS: u64 = 0x008;
const OFF_LOCK: u64 = 0x010;
const OFF_WDOGLOCK: u64 = 0x014;
const OFF_CALCNT: u64 = 0x058;
/// `CMU_CLKEN0` — the first of the three clock-enable registers. Absolute
/// `0x4000_8064` on this part.
pub const OFF_CLKEN0: u64 = 0x064;
/// `CMU_CLKEN1`.
pub const OFF_CLKEN1: u64 = 0x068;
/// `CMU_CLKEN2` — absolute `0x4000_806C`.
pub const OFF_CLKEN2: u64 = 0x06C;

/// `CMU_WDOG0CLKCTRL` — guarded by `WDOGLOCK`, not by `LOCK`.
const OFF_WDOG0CLKCTRL: u64 = 0x200;
/// `CMU_WDOG1CLKCTRL`.
const OFF_WDOG1CLKCTRL: u64 = 0x208;

/// `STATUS.LOCK`: the configuration lock is engaged.
const STATUS_LOCK: u32 = 1 << 31;
/// `STATUS.WDOGLOCK`: the watchdog configuration lock is engaged.
const STATUS_WDOGLOCK: u32 = 1 << 30;

/// One modelled register: its offset, its reset value, and whether firmware
/// may write it. Every row is a line of `CMU_TypeDef`; a register absent from
/// this table is a reserved word in that struct.
struct RegDef {
    offset: u64,
    reset: u32,
    /// `false` for the `__IM` (read-only) members of `CMU_TypeDef`.
    writable: bool,
}

const fn ro(offset: u64, reset: u32) -> RegDef {
    RegDef {
        offset,
        reset,
        writable: false,
    }
}

const fn rw(offset: u64, reset: u32) -> RegDef {
    RegDef {
        offset,
        reset,
        writable: true,
    }
}

/// The CMU register map, in `CMU_TypeDef` order. Reset values are the
/// `_CMU_<REG>_RESETVALUE` defines from the same header.
const REGS: &[RegDef] = &[
    ro(OFF_IPVERSION, 0x0000_0007),
    ro(OFF_STATUS, 0x0000_0000),
    rw(OFF_LOCK, UNLOCK_KEY),
    rw(OFF_WDOGLOCK, 0x0000_5257),
    rw(0x020, 0x0000_0000), // IF
    rw(0x024, 0x0000_0000), // IEN
    rw(0x050, 0x0000_0000), // CALCMD
    rw(0x054, 0x0000_0000), // CALCTRL
    ro(OFF_CALCNT, 0x0000_0000),
    rw(OFF_CLKEN0, 0x0000_0000),
    rw(OFF_CLKEN1, 0x0000_0000),
    rw(OFF_CLKEN2, 0x0000_0000),
    rw(0x070, 0x0000_0001), // SYSCLKCTRL
    rw(0x080, 0x0000_0001), // TRACECLKCTRL
    rw(0x090, 0x0000_0000), // EXPORTCLKCTRL
    rw(0x100, 0x0000_0000), // DPLLREFCLKCTRL
    rw(0x120, 0x0000_0001), // EM01GRPACLKCTRL
    rw(0x128, 0x0000_0001), // EM01GRPCCLKCTRL
    rw(0x140, 0x0000_0001), // EM23GRPACLKCTRL
    rw(0x160, 0x0000_0001), // EM4GRPACLKCTRL
    rw(0x180, 0x0000_0001), // IADCCLKCTRL
    rw(OFF_WDOG0CLKCTRL, 0x0000_0001),
    rw(OFF_WDOG1CLKCTRL, 0x0000_0001),
    rw(0x220, 0x0000_0001), // EUSART0CLKCTRL
    rw(0x240, 0x0000_0001), // SYSRTC0CLKCTRL
    rw(0x250, 0x0000_0001), // LCDCLKCTRL
    rw(0x260, 0x0000_0001), // VDAC0CLKCTRL
    rw(0x270, 0x0000_0001), // PCNT0CLKCTRL
    rw(0x280, 0x0000_0000), // RADIOCLKCTRL
    rw(0x294, 0x0000_0001), // VDAC1CLKCTRL
];

/// The EFR32 Series-2 Clock Management Unit.
#[derive(Debug, serde::Serialize)]
pub struct Efr32s2Cmu {
    /// Live value per row of [`REGS`], same order.
    values: Vec<u32>,
}

impl Default for Efr32s2Cmu {
    fn default() -> Self {
        Self {
            values: REGS.iter().map(|r| r.reset).collect(),
        }
    }
}

impl Efr32s2Cmu {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    fn slot(offset: u64) -> Option<usize> {
        REGS.iter().position(|r| r.offset == offset)
    }

    /// The configuration lock, from the live `LOCK` value. `STATUS` mirrors it
    /// rather than storing it, so the two can never disagree.
    #[inline]
    fn locked(&self) -> bool {
        self.values[Self::slot(OFF_LOCK).expect("LOCK is in REGS")] != UNLOCK_KEY
    }

    #[inline]
    fn wdog_locked(&self) -> bool {
        self.values[Self::slot(OFF_WDOGLOCK).expect("WDOGLOCK is in REGS")] != UNLOCK_KEY
    }

    /// Word read of a register offset. Reserved offsets read 0.
    pub fn read_word(&self, offset: u64) -> u32 {
        if offset == OFF_STATUS {
            let mut status = 0;
            if self.locked() {
                status |= STATUS_LOCK;
            }
            if self.wdog_locked() {
                status |= STATUS_WDOGLOCK;
            }
            return status;
        }
        Self::slot(offset).map(|i| self.values[i]).unwrap_or(0)
    }

    /// Word write. Read-only registers, reserved offsets and — while the
    /// relevant lock is engaged — configuration registers drop the write.
    ///
    /// There are TWO locks with two scopes: `WDOGLOCK` guards the two
    /// `WDOGnCLKCTRL` selectors (and boots engaged), `LOCK` guards everything
    /// else (and boots disengaged).
    pub fn write_word(&mut self, offset: u64, value: u32) {
        let Some(i) = Self::slot(offset) else {
            return;
        };
        if !REGS[i].writable {
            return;
        }
        // The lock registers are always writable: locking would otherwise be
        // irreversible, and `CMU_Unlock()` is exactly a write to LOCK.
        if offset == OFF_LOCK || offset == OFF_WDOGLOCK {
            self.values[i] = value;
            return;
        }
        let blocked = if offset == OFF_WDOG0CLKCTRL || offset == OFF_WDOG1CLKCTRL {
            self.wdog_locked()
        } else {
            self.locked()
        };
        if blocked {
            return;
        }
        self.values[i] = value;
    }

    /// Whether `bit` is set in the given `CLKEN` register. The clock-gate path
    /// reads through `Peripheral::read_u32`; this is for tests and for callers
    /// that already hold the model.
    pub fn clock_enabled(&self, clken_offset: u64, bit: u8) -> bool {
        (self.read_word(clken_offset) >> bit) & 1 != 0
    }
}

impl crate::Peripheral for Efr32s2Cmu {
    /// The CMU is this chip's clock controller. Only the three `CLKEN`
    /// registers are gate-able: they are the only ones whose bits withhold a
    /// peripheral's APB clock. A `clock:` gate naming anything else — say a
    /// `*CLKCTRL` selector, which chooses a source rather than enabling one —
    /// resolves to `None` and fails the build loudly, which is the intent.
    fn clock_gate_reg_offset(&self, name: &str) -> Option<u64> {
        match name.trim().to_ascii_lowercase().as_str() {
            "clken0" => Some(OFF_CLKEN0),
            "clken1" => Some(OFF_CLKEN1),
            "clken2" => Some(OFF_CLKEN2),
            _ => None,
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        Ok(((word >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) * 8;
        let merged = (self.read_word(reg) & !(0xFFu32 << shift)) | ((value as u32) << shift);
        self.write_word(reg, merged);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    /// Pure register state: nothing evolves between accesses, so the CMU can
    /// neither block walk-deletion nor need a per-cycle visit.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    #[test]
    fn resets_to_the_header_values() {
        let cmu = Efr32s2Cmu::new();
        assert_eq!(cmu.read_word(OFF_IPVERSION), 7);
        assert_eq!(cmu.read_word(0x070), 1, "SYSCLKCTRL");
        assert_eq!(cmu.read_word(0x120), 1, "EM01GRPACLKCTRL");
        assert_eq!(cmu.read_word(OFF_CLKEN0), 0);
        assert_eq!(cmu.read_word(OFF_CLKEN1), 0);
        assert_eq!(cmu.read_word(OFF_CLKEN2), 0);
        assert_eq!(
            cmu.read_word(OFF_LOCK),
            UNLOCK_KEY,
            "LOCK resets to the unlock key: the CMU boots unlocked"
        );
        assert_eq!(
            cmu.read_word(OFF_STATUS),
            STATUS_WDOGLOCK,
            "...but WDOGLOCK resets to 0x5257, NOT the unlock key, so the \
             watchdog clock configuration boots locked and STATUS says so"
        );
    }

    /// The watchdog lock is a real gate out of reset, and unlocking it is a
    /// separate step from unlocking the CMU. Firmware that configures
    /// `WDOG0CLKCTRL` without it gets nothing — on silicon and here.
    #[test]
    fn the_watchdog_clock_configuration_is_locked_out_of_reset() {
        let mut cmu = Efr32s2Cmu::new();
        cmu.write_word(OFF_WDOG0CLKCTRL, 3); // WDOG0CLKCTRL := LFRCO
        assert_eq!(
            cmu.read_word(OFF_WDOG0CLKCTRL),
            1,
            "a locked WDOGLOCK must leave WDOG0CLKCTRL at its reset selector"
        );

        cmu.write_word(OFF_WDOGLOCK, UNLOCK_KEY);
        assert_eq!(cmu.read_word(OFF_STATUS) & STATUS_WDOGLOCK, 0);
        cmu.write_word(OFF_WDOG0CLKCTRL, 3);
        assert_eq!(cmu.read_word(OFF_WDOG0CLKCTRL), 3);
    }

    #[test]
    fn clken_holds_what_firmware_writes() {
        let mut cmu = Efr32s2Cmu::new();
        // The demo firmware's bring-up: GPIO on CLKEN0 bit 26, USART1 on
        // CLKEN2 bit 7.
        cmu.write_word(OFF_CLKEN0, 1 << 26);
        cmu.write_word(OFF_CLKEN2, 1 << 7);
        assert!(cmu.clock_enabled(OFF_CLKEN0, 26));
        assert!(cmu.clock_enabled(OFF_CLKEN2, 7));
        assert!(!cmu.clock_enabled(OFF_CLKEN1, 26), "CLKEN1 is its own word");
        assert!(!cmu.clock_enabled(OFF_CLKEN0, 9), "USART0 stays gated");
    }

    #[test]
    fn read_only_registers_ignore_writes() {
        let mut cmu = Efr32s2Cmu::new();
        cmu.write_word(OFF_IPVERSION, 0xDEAD_BEEF);
        assert_eq!(cmu.read_word(OFF_IPVERSION), 7);
        cmu.write_word(OFF_CALCNT, 0x1234);
        assert_eq!(cmu.read_word(OFF_CALCNT), 0);
    }

    #[test]
    fn locking_drops_configuration_writes_and_unlocking_restores_them() {
        let mut cmu = Efr32s2Cmu::new();
        cmu.write_word(OFF_LOCK, 0); // CMU_Lock()
        assert_eq!(cmu.read_word(OFF_STATUS) & STATUS_LOCK, STATUS_LOCK);

        cmu.write_word(OFF_CLKEN0, 1 << 26);
        assert_eq!(
            cmu.read_word(OFF_CLKEN0),
            0,
            "a locked CMU must not accept a clock enable"
        );

        cmu.write_word(OFF_LOCK, UNLOCK_KEY); // CMU_Unlock()
        assert_eq!(cmu.read_word(OFF_STATUS) & STATUS_LOCK, 0);
        cmu.write_word(OFF_CLKEN0, 1 << 26);
        assert_eq!(cmu.read_word(OFF_CLKEN0), 1 << 26);
    }

    #[test]
    fn byte_writes_merge_into_the_word() {
        let mut cmu = Efr32s2Cmu::new();
        // CLKEN0 bit 26 lives in byte 3 (0x04 of that byte).
        cmu.write(OFF_CLKEN0 + 3, 0x04).unwrap();
        assert_eq!(cmu.read_word(OFF_CLKEN0), 1 << 26);
        assert_eq!(cmu.read(OFF_CLKEN0 + 3).unwrap(), 0x04);
    }

    #[test]
    fn reserved_offsets_read_zero_instead_of_faulting() {
        let mut cmu = Efr32s2Cmu::new();
        assert_eq!(cmu.read_word(0x02C), 0);
        cmu.write_word(0x02C, 0xFFFF_FFFF);
        assert_eq!(cmu.read_word(0x02C), 0);
    }
}
