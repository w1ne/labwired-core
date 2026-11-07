// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NXP S32K3 SWT (Software Watchdog Timer) and MC_ME (Mode Entry Module).
//!
//! Sources: the NXP RTD device headers `S32K344_SWT.h` and `S32K344_MC_ME.h`
//! (Zephyr `hal_nxp`, `s32/drivers/s32k3/BaseNXP/header`) for the register
//! layouts and bit fields; RTD `Power_Ip_MC_ME.c` and `Clock_Ip_Gate.c` for
//! the MC_ME update handshake firmware relies on (write CLKEN/PCONF, set
//! PUPD, write `CTL_KEY` 0x5AF0 then 0xA50F, poll `PRTNx_STAT` /
//! `PRTNx_COFBy_STAT`). The S32K388 shares both IPs and their base addresses
//! (SWT_0 @ 0x4027_0000, MC_ME @ 0x402D_C000).
//!
//! # SWT
//! Registers: `CR` 0x00, `IR` 0x04, `TO` 0x08, `WN` 0x0C, `SR` 0x10, `CO`
//! 0x14, `SK` 0x18, `RRR` 0x1C.
//!
//! Modelled:
//! - `CR` reset 0xFF00_010A (MAP0-7, RIA, FRZ; WEN = 0), `TO` reset 0x320.
//!   The CR value is the documented S32K3 SWT_0 default (it agrees with the
//!   `configs/chips/s32k388.yaml` note that the SVD reset has WEN = 0); the
//!   TO value is the S32K3 reference-manual default and is not checked
//!   against an SVD here.
//! - Soft lock (`CR.SLK`) and hard lock (`CR.HLK`, sticky until reset):
//!   while either is set, writes to `CR`, `TO`, `WN` and `SK` are invalid.
//!   `SR` 0xC520 then 0xD928 clears SLK (never HLK).
//! - Service: fixed mode (`CR.SMD` = 0) takes 0xA602 then 0xB480; keyed mode
//!   (`SMD` = 1) takes two keys, each `(17·SK + 3) mod 2^16`, updating `SK`.
//!   A completed sequence reloads the counter from `TO`. With `CR.WND` set a
//!   service while the counter is above `WN` is invalid.
//! - The down-counter, clocked at `swt_clock_hz` (S32K3: SIRC, 32 kHz) and
//!   derived lazily from the bus cycle count at `clock_hz`. It runs only while
//!   `CR.WEN` = 1 and reloads from `TO` when WEN goes 0 -> 1. `CO` reads it.
//! - Time-out: with `CR.ITR` = 1 the first time-out sets `IR.TIF` (W1C) and
//!   raises this block's IRQ (S32K3 `SWT0_IRQn` = 42), then the counter
//!   reloads; a time-out with TIF still set, or any time-out with ITR = 0,
//!   requests a watchdog CPU reset.
//! - Invalid access (a locked write, a bad window service) requests a reset
//!   when `CR.RIA` = 1 and is otherwise dropped.
//!
//! NOT modelled: the bus error an invalid access raises with RIA = 0, `CR.STP`
//! / `CR.FRZ` (the counter does not stop in debug or stop mode), the `MAPn`
//! master filter (every bus master may write), `SMD` = 2/3 (reserved),
//! `RRR`'s reset-request flag (stored only), and MC_RGM reset escalation
//! after repeated watchdog resets.
//!
//! # MC_ME
//! Registers: `CTL_KEY` 0x000, `MODE_CONF` 0x004, `MODE_UPD` 0x008,
//! `MODE_STAT` 0x00C, `MAIN_COREID` 0x010; partition `n` (0..=3) at
//! `0x100 + 0x200·n`: `PCONF` +0x00, `PUPD` +0x04, `STAT` +0x08,
//! `COFBm_STAT` +0x10 + 4m and `COFBm_CLKEN` +0x30 + 4m (m = 0..=3), and core
//! `k` (0..=3) at +0x40 + 0x20·k: `PCONF`, `PUPD`, `STAT`, `ADDR`. This is the
//! RTD's `McMePrtnArray` stride; offsets a given part reserves are stored.
//!
//! Modelled:
//! - The key handshake: `CTL_KEY` 0x5AF0 immediately followed by 0xA50F
//!   applies every pending update; any other write restarts it. On apply:
//!   a partition with `PUPD.PCUD` copies `PCONF.PCE` to `STAT.PCS` and its
//!   `COFBm_CLKEN` to `COFBm_STAT` (all zero while PCE = 0); a core with
//!   `PUPD.CCUPD` copies `PCONF.CCE` to `STAT.CCS`; the PUPD bits clear.
//!   `MODE_UPD` with `MODE_CONF.DEST_RST` or `FUNC_RST` requests a system
//!   reset (RTD `Power_Ip_MC_ME_SocTriggerResetEvent`).
//! - Reset state: partition 0 and its core 0 (the boot core) enabled
//!   (`PCONF` = `STAT` = 1); every other partition, core and COFB clock 0.
//!
//! NOT modelled: the gating itself (a peripheral whose COFB clock is off
//! still answers on the bus; silicon would fault the access), the reset COFB
//! clocks of a given part (they read 0 until enabled), standby entry
//! (`MODE_CONF.STANDBY` is stored only), `CORE_STAT.WFI` (reads 0, so an RTD
//! core power-down that waits for WFI times out), `MAIN_COREID` (reads 0:
//! CM7_0 in partition 0), and what `CTL_KEY` reads back (the last key
//! written).

use crate::{CycleClock, Peripheral, PeripheralResetCause, PeripheralTickResult, SimResult};

/// Read one byte lane of a 32-bit register value.
fn lane(value: u32, offset: u64) -> u8 {
    (value >> ((offset & 3) * 8)) as u8
}

/// Merge `byte` into lane `offset & 3` of `value`.
fn merge_lane(value: u32, offset: u64, byte: u8) -> u32 {
    let shift = (offset & 3) * 8;
    (value & !(0xFF << shift)) | (u32::from(byte) << shift)
}

// ── SWT ──────────────────────────────────────────────────────────────────

const SWT_CR: u64 = 0x00;
const SWT_IR: u64 = 0x04;
const SWT_TO: u64 = 0x08;
const SWT_WN: u64 = 0x0C;
const SWT_SR: u64 = 0x10;
const SWT_CO: u64 = 0x14;
const SWT_SK: u64 = 0x18;
const SWT_RRR: u64 = 0x1C;

const CR_WEN: u32 = 1 << 0;
const CR_SLK: u32 = 1 << 4;
const CR_HLK: u32 = 1 << 5;
const CR_ITR: u32 = 1 << 6;
const CR_WND: u32 = 1 << 7;
const CR_RIA: u32 = 1 << 8;
const CR_SMD_SHIFT: u32 = 9;
const CR_SMD_MASK: u32 = 0x3 << CR_SMD_SHIFT;
/// Writable CR bits: MAP0-7, SMD, RIA, WND, ITR, HLK, SLK, STP, FRZ, WEN.
const CR_WRITABLE: u32 = 0xFF00_07F7;
const IR_TIF: u32 = 1 << 0;

/// S32K3 SWT_0 `CR` default: MAP0-7, RIA, FRZ, bit 3; WEN = 0.
pub const SWT_CR_RESET: u32 = 0xFF00_010A;
/// S32K3 SWT `TO` default (800 SIRC cycles).
pub const SWT_TO_RESET: u32 = 0x320;

const KEY_UNLOCK_1: u32 = 0xC520;
const KEY_UNLOCK_2: u32 = 0xD928;
const KEY_SERVICE_1: u32 = 0xA602;
const KEY_SERVICE_2: u32 = 0xB480;

/// S32K3 reset core clock: the 48 MHz FIRC.
pub const DEFAULT_CLOCK_HZ: u64 = 48_000_000;
/// S32K3 SWT counter clock: SIRC, 32 kHz.
pub const DEFAULT_SWT_CLOCK_HZ: u64 = 32_000;

/// Which two-write `SR` sequence the last accepted write began.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum SwtSeq {
    Idle,
    Unlock,
    Service,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct S32k3Swt {
    clock_hz: u64,
    swt_clock_hz: u64,
    cr: u32,
    ir: u32,
    to: u32,
    wn: u32,
    sk: u32,
    rrr: u32,
    seq: SwtSeq,
    /// CPU cycle the counter last loaded from `TO`; `None` while WEN = 0.
    loaded_at: Option<u64>,
    /// A first (ITR) time-out raised TIF; delivered on the next tick.
    irq_pending: bool,
    reset_latched: bool,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl S32k3Swt {
    pub fn new(clock_hz: u64, swt_clock_hz: u64) -> Self {
        Self {
            clock_hz: clock_hz.max(1),
            swt_clock_hz: swt_clock_hz.max(1),
            cr: SWT_CR_RESET,
            ir: 0,
            to: SWT_TO_RESET,
            wn: 0,
            sk: 0,
            rrr: 0,
            seq: SwtSeq::Idle,
            loaded_at: None,
            irq_pending: false,
            reset_latched: false,
            clock: None,
        }
    }

    fn now_cycle(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    fn locked(&self) -> bool {
        self.cr & (CR_SLK | CR_HLK) != 0
    }

    /// CPU cycles one full `TO` period spans (at least one).
    fn period_cycles(&self) -> u64 {
        let num = u128::from(self.to.max(1)) * u128::from(self.clock_hz);
        (num.div_ceil(u128::from(self.swt_clock_hz)) as u64).max(1)
    }

    /// SWT clock ticks elapsed between two CPU cycles.
    fn ticks_between(&self, from: u64, to: u64) -> u64 {
        (u128::from(to.saturating_sub(from)) * u128::from(self.swt_clock_hz)
            / u128::from(self.clock_hz)) as u64
    }

    fn reload(&mut self) {
        if self.cr & CR_WEN != 0 {
            self.loaded_at = Some(self.now_cycle());
        }
    }

    /// The counter as `CO` shows it now.
    pub fn counter(&self) -> u32 {
        match self.loaded_at {
            Some(at) => {
                let elapsed = self.ticks_between(at, self.now_cycle());
                self.to
                    .saturating_sub(elapsed.min(u64::from(u32::MAX)) as u32)
            }
            None => self.to,
        }
    }

    /// Process every time-out that has elapsed by now.
    fn advance(&mut self) {
        while let Some(at) = self.loaded_at {
            let deadline = at.saturating_add(self.period_cycles());
            if self.now_cycle() < deadline {
                return;
            }
            if self.cr & CR_ITR != 0 && self.ir & IR_TIF == 0 {
                self.ir |= IR_TIF;
                self.irq_pending = true;
                self.loaded_at = Some(deadline);
            } else {
                self.reset_latched = true;
                self.loaded_at = None;
                return;
            }
        }
    }

    /// Whether `advance` would latch a reset right now (pure).
    fn reset_due(&self) -> bool {
        if self.reset_latched {
            return true;
        }
        let Some(at) = self.loaded_at else {
            return false;
        };
        let period = self.period_cycles();
        let now = self.now_cycle();
        let first_tif = self.cr & CR_ITR != 0 && self.ir & IR_TIF == 0;
        let deadline = if first_tif {
            at.saturating_add(period.saturating_mul(2))
        } else {
            at.saturating_add(period)
        };
        now >= deadline
    }

    fn invalid_access(&mut self) {
        if self.cr & CR_RIA != 0 {
            self.reset_latched = true;
        }
    }

    fn service(&mut self) {
        if self.cr & CR_WND != 0 && self.loaded_at.is_some() && self.counter() > self.wn {
            self.invalid_access();
            return;
        }
        self.reload();
    }

    fn write_sr(&mut self, value: u32) {
        let key = value & 0xFFFF;
        let keyed = (self.cr & CR_SMD_MASK) >> CR_SMD_SHIFT == 1;
        let next_sk = (self.sk.wrapping_mul(17).wrapping_add(3)) & 0xFFFF;
        self.seq = match (self.seq, key) {
            (SwtSeq::Unlock, KEY_UNLOCK_2) => {
                self.cr &= !CR_SLK;
                SwtSeq::Idle
            }
            (SwtSeq::Service, k) if !keyed && k == KEY_SERVICE_2 => {
                self.service();
                SwtSeq::Idle
            }
            (SwtSeq::Service, k) if keyed && k == next_sk => {
                self.sk = next_sk;
                self.service();
                SwtSeq::Idle
            }
            (_, KEY_UNLOCK_1) => SwtSeq::Unlock,
            (_, k) if !keyed && k == KEY_SERVICE_1 => SwtSeq::Service,
            (_, k) if keyed && k == next_sk => {
                self.sk = next_sk;
                SwtSeq::Service
            }
            _ => SwtSeq::Idle,
        };
    }

    fn write_cr(&mut self, value: u32) {
        if self.locked() {
            self.invalid_access();
            return;
        }
        let was_enabled = self.cr & CR_WEN != 0;
        // HLK is sticky: a write can set it but never clear it.
        let hlk = self.cr & CR_HLK;
        self.cr = (self.cr & !CR_WRITABLE) | (value & CR_WRITABLE) | hlk;
        match (was_enabled, self.cr & CR_WEN != 0) {
            (false, true) => self.loaded_at = Some(self.now_cycle()),
            (true, false) => self.loaded_at = None,
            _ => {}
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            SWT_CR => self.cr,
            SWT_IR => self.ir,
            SWT_TO => self.to,
            SWT_WN => self.wn,
            SWT_SR => 0,
            SWT_CO => self.counter(),
            SWT_SK => self.sk,
            SWT_RRR => self.rrr,
            _ => {
                crate::census_reg!("s32k3:S32k3Swt", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        self.advance();
        match offset {
            SWT_CR => self.write_cr(value),
            SWT_IR => self.ir &= !(value & IR_TIF),
            SWT_TO | SWT_WN | SWT_SK if self.locked() => self.invalid_access(),
            SWT_TO => self.to = value,
            SWT_WN => self.wn = value,
            SWT_SK => self.sk = value & 0xFFFF,
            SWT_SR => self.write_sr(value),
            SWT_RRR => self.rrr = value & 1,
            SWT_CO => {}
            _ => {
                crate::census_reg!("s32k3:S32k3Swt", offset, "write");
            }
        }
    }

    fn power_on(&mut self) {
        let clock = self.clock.take();
        *self = Self::new(self.clock_hz, self.swt_clock_hz);
        self.clock = clock;
    }
}

impl Default for S32k3Swt {
    fn default() -> Self {
        Self::new(DEFAULT_CLOCK_HZ, DEFAULT_SWT_CLOCK_HZ)
    }
}

impl Peripheral for S32k3Swt {
    /// The counter is lazy; the walk is only needed to deliver the first
    /// (ITR) time-out interrupt. Resets are drained through
    /// `take_reset_request` at instruction boundaries.
    fn legacy_tick_active(&self) -> bool {
        self.irq_pending
            || (self.loaded_at.is_some() && self.cr & CR_ITR != 0 && self.ir & IR_TIF == 0)
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.tick_elapsed(1)
    }

    fn tick_elapsed(&mut self, _cycles: u64) -> PeripheralTickResult {
        self.advance();
        PeripheralTickResult::with_irq(std::mem::take(&mut self.irq_pending))
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(lane(self.read_reg(offset & !3), offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let merged = merge_lane(self.read_reg(aligned), offset, value);
        self.write_reg(aligned, merged);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn reset_request_capable(&self) -> bool {
        true
    }

    fn latched_reset_cause(&self) -> Option<PeripheralResetCause> {
        self.reset_due().then_some(PeripheralResetCause::Watchdog)
    }

    fn take_reset_request(&mut self) -> bool {
        self.advance();
        std::mem::take(&mut self.reset_latched)
    }

    fn on_system_reset(&mut self) {
        self.power_on();
    }

    fn on_node_reset(&mut self) {
        self.power_on();
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let mut restored: Self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("S32k3Swt snapshot decode: {error}"))
        })?;
        restored.clock = self.clock.take();
        *self = restored;
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize S32k3Swt")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let mut restored: Self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "S32k3Swt runtime snapshot decode: {error}"
            ))
        })?;
        restored.clock = self.clock.take();
        *self = restored;
        Ok(())
    }
}

// ── MC_ME ────────────────────────────────────────────────────────────────

const ME_CTL_KEY: u64 = 0x000;
const ME_MODE_CONF: u64 = 0x004;
const ME_MODE_UPD: u64 = 0x008;
const ME_MODE_STAT: u64 = 0x00C;
const ME_MAIN_COREID: u64 = 0x010;
const ME_PRTN_BASE: u64 = 0x100;
const ME_PRTN_STRIDE: u64 = 0x200;
const ME_PRTNS: usize = 4;
const ME_COFBS: usize = 4;
const ME_CORES: usize = 4;

const ME_KEY_DIRECT: u32 = 0x5AF0;
const ME_KEY_INVERTED: u32 = 0xA50F;
const MODE_CONF_DEST_RST: u32 = 1 << 0;
const MODE_CONF_FUNC_RST: u32 = 1 << 1;
const MODE_CONF_STANDBY: u32 = 1 << 15;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct McMeCore {
    pconf: u32,
    pupd: u32,
    stat: u32,
    addr: u32,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct McMePartition {
    pconf: u32,
    pupd: u32,
    stat: u32,
    cofb_stat: [u32; ME_COFBS],
    cofb_clken: [u32; ME_COFBS],
    cores: [McMeCore; ME_CORES],
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct S32k3McMe {
    ctl_key: u32,
    /// The direct key (0x5AF0) was the last `CTL_KEY` write.
    key_armed: bool,
    mode_conf: u32,
    mode_upd: u32,
    mode_stat: u32,
    prtn: [McMePartition; ME_PRTNS],
    reset_latched: bool,
}

impl S32k3McMe {
    pub fn new() -> Self {
        let mut me = Self {
            ctl_key: 0,
            key_armed: false,
            mode_conf: 0,
            mode_upd: 0,
            mode_stat: 0,
            prtn: Default::default(),
            reset_latched: false,
        };
        // Partition 0 and its core 0 (the boot core) run out of reset.
        me.prtn[0].pconf = 1;
        me.prtn[0].stat = 1;
        me.prtn[0].cores[0].pconf = 1;
        me.prtn[0].cores[0].stat = 1;
        me
    }

    /// `COFBm_STAT` of partition `p` as firmware reads it.
    pub fn cofb_stat(&self, p: usize, m: usize) -> u32 {
        self.prtn[p].cofb_stat[m]
    }

    /// Apply every pending update: the 0x5AF0 / 0xA50F handshake completed.
    fn apply(&mut self) {
        for p in &mut self.prtn {
            if p.pupd & 1 != 0 {
                p.stat = p.pconf & 1;
                for m in 0..ME_COFBS {
                    p.cofb_stat[m] = if p.stat != 0 { p.cofb_clken[m] } else { 0 };
                }
                p.pupd &= !1;
            }
            for c in &mut p.cores {
                if c.pupd & 1 != 0 {
                    c.stat = (c.stat & !1) | (c.pconf & 1);
                    c.pupd &= !1;
                }
            }
        }
        if self.mode_upd & 1 != 0 {
            if self.mode_conf & (MODE_CONF_DEST_RST | MODE_CONF_FUNC_RST) != 0 {
                self.reset_latched = true;
            }
            self.mode_upd = 0;
        }
    }

    /// Decode a partition offset into (partition, offset within it).
    fn prtn_slot(offset: u64) -> Option<(usize, u64)> {
        let rel = offset.checked_sub(ME_PRTN_BASE)?;
        let p = (rel / ME_PRTN_STRIDE) as usize;
        (p < ME_PRTNS).then_some((p, rel % ME_PRTN_STRIDE))
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            ME_CTL_KEY => self.ctl_key,
            ME_MODE_CONF => self.mode_conf,
            ME_MODE_UPD => self.mode_upd,
            ME_MODE_STAT => self.mode_stat,
            ME_MAIN_COREID => 0,
            _ => match Self::prtn_slot(offset) {
                Some((p, rel)) => {
                    let p = &self.prtn[p];
                    match rel {
                        0x00 => p.pconf,
                        0x04 => p.pupd,
                        0x08 => p.stat,
                        0x10..=0x1C => p.cofb_stat[((rel - 0x10) / 4) as usize],
                        0x30..=0x3C => p.cofb_clken[((rel - 0x30) / 4) as usize],
                        0x40..=0xBC => {
                            let c = &p.cores[((rel - 0x40) / 0x20) as usize];
                            match (rel - 0x40) % 0x20 {
                                0x00 => c.pconf,
                                0x04 => c.pupd,
                                0x08 => c.stat,
                                0x0C => c.addr,
                                _ => 0,
                            }
                        }
                        _ => 0,
                    }
                }
                None => {
                    crate::census_reg!("s32k3:S32k3McMe", offset, "read");
                    0
                }
            },
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            ME_CTL_KEY => {
                let key = value & 0xFFFF;
                self.ctl_key = key;
                if self.key_armed && key == ME_KEY_INVERTED {
                    self.apply();
                }
                self.key_armed = key == ME_KEY_DIRECT;
            }
            ME_MODE_CONF => {
                self.mode_conf =
                    value & (MODE_CONF_DEST_RST | MODE_CONF_FUNC_RST | MODE_CONF_STANDBY)
            }
            ME_MODE_UPD => self.mode_upd = value & 1,
            ME_MODE_STAT | ME_MAIN_COREID => {}
            _ => match Self::prtn_slot(offset) {
                Some((p, rel)) => {
                    let p = &mut self.prtn[p];
                    match rel {
                        0x00 => p.pconf = value & 1,
                        0x04 => p.pupd = value & 1,
                        0x30..=0x3C => p.cofb_clken[((rel - 0x30) / 4) as usize] = value,
                        0x40..=0xBC => {
                            let c = &mut p.cores[((rel - 0x40) / 0x20) as usize];
                            match (rel - 0x40) % 0x20 {
                                0x00 => c.pconf = value & 1,
                                0x04 => c.pupd = value & 1,
                                0x0C => c.addr = value & !3,
                                _ => {}
                            }
                        }
                        // STAT / COFB_STAT are read-only.
                        _ => {}
                    }
                }
                None => {
                    crate::census_reg!("s32k3:S32k3McMe", offset, "write");
                }
            },
        }
    }
}

impl Default for S32k3McMe {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for S32k3McMe {
    /// A register bank: every effect happens on the `CTL_KEY` write.
    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(lane(self.read_reg(offset & !3), offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let merged = merge_lane(self.read_reg(aligned), offset, value);
        self.write_reg(aligned, merged);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn reset_request_capable(&self) -> bool {
        true
    }

    fn latched_reset_cause(&self) -> Option<PeripheralResetCause> {
        self.reset_latched.then_some(PeripheralResetCause::System)
    }

    fn take_reset_request(&mut self) -> bool {
        std::mem::take(&mut self.reset_latched)
    }

    fn on_system_reset(&mut self) {
        *self = Self::new();
    }

    fn on_node_reset(&mut self) {
        *self = Self::new();
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        *self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("S32k3McMe snapshot decode: {error}"))
        })?;
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize S32k3McMe")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        *self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "S32k3McMe runtime snapshot decode: {error}"
            ))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swt_at(cycle: u64) -> (S32k3Swt, CycleClock) {
        let clock = CycleClock::default();
        clock.publish(cycle);
        let mut swt = S32k3Swt::default();
        swt.attach_cycle_clock(clock.clone());
        (swt, clock)
    }

    /// CPU cycles per SWT tick at the defaults (48 MHz / 32 kHz).
    const CYC_PER_TICK: u64 = DEFAULT_CLOCK_HZ / DEFAULT_SWT_CLOCK_HZ;

    #[test]
    fn swt_reset_values_and_disabled_counter_never_fires() {
        let (mut swt, clock) = swt_at(0);
        assert_eq!(swt.read_u32(SWT_CR).unwrap(), SWT_CR_RESET);
        assert_eq!(swt.read_u32(SWT_TO).unwrap(), SWT_TO_RESET);
        assert_eq!(swt.read_u32(SWT_CO).unwrap(), SWT_TO_RESET);
        clock.publish(10 * DEFAULT_CLOCK_HZ);
        assert!(!swt.take_reset_request());
    }

    #[test]
    fn swt_enabled_counts_down_and_resets_on_timeout() {
        let (mut swt, clock) = swt_at(1000);
        swt.write_u32(SWT_TO, 0x100).unwrap();
        swt.write_u32(SWT_CR, SWT_CR_RESET | CR_WEN).unwrap();
        clock.publish(1000 + 0x10 * CYC_PER_TICK);
        assert_eq!(swt.read_u32(SWT_CO).unwrap(), 0xF0);
        assert!(!swt.take_reset_request());
        clock.publish(1000 + 0x100 * CYC_PER_TICK);
        assert_eq!(
            swt.latched_reset_cause(),
            Some(PeripheralResetCause::Watchdog)
        );
        assert!(swt.take_reset_request());
        // The latch drains once.
        assert!(!swt.take_reset_request());
    }

    #[test]
    fn swt_fixed_service_sequence_reloads_and_wrong_key_does_not() {
        let (mut swt, clock) = swt_at(0);
        swt.write_u32(SWT_TO, 0x100).unwrap();
        swt.write_u32(SWT_CR, SWT_CR_RESET | CR_WEN).unwrap();
        clock.publish(0x80 * CYC_PER_TICK);
        swt.write_u32(SWT_SR, KEY_SERVICE_1).unwrap();
        swt.write_u32(SWT_SR, KEY_SERVICE_2).unwrap();
        assert_eq!(swt.read_u32(SWT_CO).unwrap(), 0x100);
        // A broken sequence does not reload.
        clock.publish(0x100 * CYC_PER_TICK);
        swt.write_u32(SWT_SR, KEY_SERVICE_1).unwrap();
        swt.write_u32(SWT_SR, 0x1234).unwrap();
        swt.write_u32(SWT_SR, KEY_SERVICE_2).unwrap();
        assert_eq!(swt.read_u32(SWT_CO).unwrap(), 0x80);
        clock.publish(0x180 * CYC_PER_TICK);
        assert!(swt.take_reset_request());
    }

    #[test]
    fn swt_keyed_service_follows_the_sk_sequence() {
        let (mut swt, clock) = swt_at(0);
        swt.write_u32(SWT_TO, 0x100).unwrap();
        swt.write_u32(SWT_SK, 5).unwrap();
        swt.write_u32(SWT_CR, SWT_CR_RESET | (1 << CR_SMD_SHIFT) | CR_WEN)
            .unwrap();
        clock.publish(0x80 * CYC_PER_TICK);
        let k1 = (5 * 17 + 3) & 0xFFFF;
        let k2 = (k1 * 17 + 3) & 0xFFFF;
        // The fixed keys do nothing in keyed mode.
        swt.write_u32(SWT_SR, KEY_SERVICE_1).unwrap();
        swt.write_u32(SWT_SR, KEY_SERVICE_2).unwrap();
        assert_eq!(swt.read_u32(SWT_CO).unwrap(), 0x80);
        swt.write_u32(SWT_SR, k1).unwrap();
        swt.write_u32(SWT_SR, k2).unwrap();
        assert_eq!(swt.read_u32(SWT_CO).unwrap(), 0x100);
        assert_eq!(swt.read_u32(SWT_SK).unwrap(), k2);
    }

    #[test]
    fn swt_soft_lock_blocks_writes_until_unlock_and_ria_resets() {
        let (mut swt, _clock) = swt_at(0);
        swt.write_u32(SWT_CR, (SWT_CR_RESET & !CR_RIA) | CR_SLK)
            .unwrap();
        // Locked, RIA = 0: the write is dropped, no reset.
        swt.write_u32(SWT_TO, 0x200).unwrap();
        assert_eq!(swt.read_u32(SWT_TO).unwrap(), SWT_TO_RESET);
        assert!(!swt.take_reset_request());
        swt.write_u32(SWT_SR, KEY_UNLOCK_1).unwrap();
        swt.write_u32(SWT_SR, KEY_UNLOCK_2).unwrap();
        assert_eq!(swt.read_u32(SWT_CR).unwrap() & CR_SLK, 0);
        swt.write_u32(SWT_TO, 0x200).unwrap();
        assert_eq!(swt.read_u32(SWT_TO).unwrap(), 0x200);
        // Locked with RIA = 1: an invalid write requests a reset.
        swt.write_u32(SWT_CR, SWT_CR_RESET | CR_SLK).unwrap();
        swt.write_u32(SWT_WN, 1).unwrap();
        assert!(swt.take_reset_request());
    }

    #[test]
    fn swt_hard_lock_survives_the_unlock_sequence() {
        let (mut swt, _clock) = swt_at(0);
        swt.write_u32(SWT_CR, (SWT_CR_RESET & !CR_RIA) | CR_HLK)
            .unwrap();
        swt.write_u32(SWT_SR, KEY_UNLOCK_1).unwrap();
        swt.write_u32(SWT_SR, KEY_UNLOCK_2).unwrap();
        assert_ne!(swt.read_u32(SWT_CR).unwrap() & CR_HLK, 0);
        swt.write_u32(SWT_CR, 0).unwrap();
        assert_ne!(swt.read_u32(SWT_CR).unwrap() & CR_HLK, 0);
        swt.on_system_reset();
        assert_eq!(swt.read_u32(SWT_CR).unwrap(), SWT_CR_RESET);
    }

    #[test]
    fn swt_itr_first_timeout_interrupts_second_resets() {
        let (mut swt, clock) = swt_at(0);
        swt.write_u32(SWT_TO, 0x100).unwrap();
        swt.write_u32(SWT_CR, SWT_CR_RESET | CR_ITR | CR_WEN)
            .unwrap();
        assert!(swt.legacy_tick_active());
        clock.publish(0x100 * CYC_PER_TICK);
        assert!(swt.tick().irq);
        assert_eq!(swt.read_u32(SWT_IR).unwrap(), IR_TIF);
        assert!(!swt.take_reset_request());
        assert!(!swt.tick().irq);
        clock.publish(0x200 * CYC_PER_TICK);
        assert!(swt.take_reset_request());
    }

    #[test]
    fn swt_itr_cleared_tif_rearms_the_interrupt() {
        let (mut swt, clock) = swt_at(0);
        swt.write_u32(SWT_TO, 0x100).unwrap();
        swt.write_u32(SWT_CR, SWT_CR_RESET | CR_ITR | CR_WEN)
            .unwrap();
        clock.publish(0x100 * CYC_PER_TICK);
        assert!(swt.tick().irq);
        swt.write_u32(SWT_IR, IR_TIF).unwrap();
        assert_eq!(swt.read_u32(SWT_IR).unwrap(), 0);
        clock.publish(0x200 * CYC_PER_TICK);
        assert!(swt.tick().irq);
        assert!(!swt.take_reset_request());
    }

    #[test]
    fn swt_window_service_too_early_is_invalid() {
        let (mut swt, clock) = swt_at(0);
        swt.write_u32(SWT_TO, 0x100).unwrap();
        swt.write_u32(SWT_WN, 0x40).unwrap();
        swt.write_u32(SWT_CR, SWT_CR_RESET | CR_WND | CR_WEN)
            .unwrap();
        clock.publish(0x10 * CYC_PER_TICK);
        swt.write_u32(SWT_SR, KEY_SERVICE_1).unwrap();
        swt.write_u32(SWT_SR, KEY_SERVICE_2).unwrap();
        assert!(swt.take_reset_request());
    }

    #[test]
    fn swt_disable_after_unlock_matches_rtd_startup() {
        // The S32K388 smoke's sequence: unlock, then clear WEN.
        let (mut swt, clock) = swt_at(0);
        swt.write_u32(SWT_SR, KEY_UNLOCK_1).unwrap();
        swt.write_u32(SWT_SR, KEY_UNLOCK_2).unwrap();
        let cr = swt.read_u32(SWT_CR).unwrap();
        swt.write_u32(SWT_CR, cr & !CR_WEN).unwrap();
        assert_eq!(swt.read_u32(SWT_CR).unwrap(), SWT_CR_RESET);
        clock.publish(DEFAULT_CLOCK_HZ);
        assert!(!swt.take_reset_request());
    }

    const PRTN1: u64 = ME_PRTN_BASE + ME_PRTN_STRIDE;

    #[test]
    fn mc_me_reset_state() {
        let me = S32k3McMe::new();
        assert_eq!(me.read_u32(0x100).unwrap(), 1);
        assert_eq!(me.read_u32(0x108).unwrap(), 1);
        assert_eq!(me.read_u32(0x148).unwrap(), 1);
        assert_eq!(me.read_u32(PRTN1 + 0x08).unwrap(), 0);
        assert_eq!(me.read_u32(PRTN1 + 0x14).unwrap(), 0);
    }

    #[test]
    fn mc_me_clock_gate_applies_only_after_the_key_handshake() {
        let mut me = S32k3McMe::new();
        me.write_u32(PRTN1 + 0x34, 1 << 10).unwrap();
        me.write_u32(PRTN1 + 0x38, 1 << 12).unwrap();
        me.write_u32(PRTN1, 1).unwrap();
        me.write_u32(PRTN1 + 0x04, 1).unwrap();
        assert_eq!(me.read_u32(PRTN1 + 0x14).unwrap(), 0);
        // Wrong order: nothing applies.
        me.write_u32(ME_CTL_KEY, ME_KEY_INVERTED).unwrap();
        me.write_u32(ME_CTL_KEY, ME_KEY_DIRECT).unwrap();
        me.write_u32(ME_CTL_KEY, 0x1234).unwrap();
        me.write_u32(ME_CTL_KEY, ME_KEY_INVERTED).unwrap();
        assert_eq!(me.read_u32(PRTN1 + 0x08).unwrap(), 0);
        me.write_u32(ME_CTL_KEY, ME_KEY_DIRECT).unwrap();
        me.write_u32(ME_CTL_KEY, ME_KEY_INVERTED).unwrap();
        assert_eq!(me.read_u32(PRTN1 + 0x08).unwrap(), 1);
        assert_eq!(me.read_u32(PRTN1 + 0x14).unwrap(), 1 << 10);
        assert_eq!(me.read_u32(PRTN1 + 0x18).unwrap(), 1 << 12);
        assert_eq!(me.read_u32(PRTN1 + 0x04).unwrap(), 0);
    }

    #[test]
    fn mc_me_clken_without_pupd_does_not_change_status() {
        let mut me = S32k3McMe::new();
        me.write_u32(PRTN1 + 0x34, 1 << 10).unwrap();
        me.write_u32(ME_CTL_KEY, ME_KEY_DIRECT).unwrap();
        me.write_u32(ME_CTL_KEY, ME_KEY_INVERTED).unwrap();
        assert_eq!(me.cofb_stat(1, 1), 0);
    }

    #[test]
    fn mc_me_disabled_partition_gates_every_cofb_clock() {
        let mut me = S32k3McMe::new();
        me.write_u32(PRTN1 + 0x34, 1 << 10).unwrap();
        me.write_u32(PRTN1, 0).unwrap();
        me.write_u32(PRTN1 + 0x04, 1).unwrap();
        me.write_u32(ME_CTL_KEY, ME_KEY_DIRECT).unwrap();
        me.write_u32(ME_CTL_KEY, ME_KEY_INVERTED).unwrap();
        assert_eq!(me.cofb_stat(1, 1), 0);
    }

    #[test]
    fn mc_me_core_update_and_functional_reset() {
        let mut me = S32k3McMe::new();
        // PRTN0_CORE2: enable it.
        me.write_u32(0x100 + 0x80, 1).unwrap();
        me.write_u32(0x100 + 0x84, 1).unwrap();
        me.write_u32(ME_CTL_KEY, ME_KEY_DIRECT).unwrap();
        me.write_u32(ME_CTL_KEY, ME_KEY_INVERTED).unwrap();
        assert_eq!(me.read_u32(0x100 + 0x88).unwrap(), 1);
        assert!(!me.take_reset_request());
        me.write_u32(ME_MODE_CONF, MODE_CONF_FUNC_RST).unwrap();
        me.write_u32(ME_MODE_UPD, 1).unwrap();
        me.write_u32(ME_CTL_KEY, ME_KEY_DIRECT).unwrap();
        me.write_u32(ME_CTL_KEY, ME_KEY_INVERTED).unwrap();
        assert_eq!(me.latched_reset_cause(), Some(PeripheralResetCause::System));
        assert!(me.take_reset_request());
        me.on_system_reset();
        assert_eq!(me.read_u32(0x100 + 0x88).unwrap(), 0);
    }
}
