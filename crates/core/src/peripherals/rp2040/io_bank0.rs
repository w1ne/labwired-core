// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RP2040 IO_BANK0 — the pad function mux (datasheet §2.19.2).
//!
//! This block decides, per pad, which peripheral is wired to it. Without it the
//! engine had no way to answer "who drives GP4?", so a logic analyzer clipped to
//! an RP2040 I²C or SPI pin could only ever read the SIO output latch — a flat
//! line while the bus was busy. The register map existed in
//! `configs/peripherals/rp2040/io_bank0.yaml` but was never wired into the chip,
//! so firmware writes selecting a function landed nowhere.
//!
//! Layout: `GPIOn_STATUS` at `8n`, `GPIOn_CTRL` at `8n + 4`, for GP0..GP29.
//! `CTRL.FUNCSEL` is bits [4:0] and resets to 31 (NULL — pad driven by nothing).
//! The function numbers are the pico-sdk `gpio_function` enum: SPI 1, UART 2,
//! I2C 3, PWM 4, SIO 5, PIO0 6, PIO1 7, GPCK 8, USB 9.
//!
//! FUNCSEL is modelled behaviourally, and so is the GPIO interrupt block
//! (§2.19.3): the raw `INTR0..3` latches (edge bits write-1-to-clear, level
//! bits live), `PROC0_INTE/INTF/INTS0..3`, and `IO_IRQ_BANK0` (NVIC 13) while
//! any `PROC0_INTS` bit is set. The pad levels the interrupts see are the SIO's
//! `GPIO_IN` (shared through [`BankIrq`]), so a pad's own output edges count,
//! as on silicon. `PROC1_*` and `DORMANT_WAKE_*` store and read back (this is a
//! single-core model and nothing sleeps). The OVER fields (`OUTOVER`,
//! `OEOVER`, `INOVER`, `IRQOVER`) store and read back, because firmware reads
//! them, but they do not invert or force anything yet — an honest gap rather
//! than a guess, and one that only bites a firmware deliberately overriding a
//! pad.
//!
//! The RP2350 (Pico 2) reuses the block with [`IoBankLayout::Rp2350`]: same
//! GPIO array and FUNCSEL numbering for SPI/UART/I2C/PWM/SIO, the interrupt
//! block moved to 0x230 with six words per group, and `IRQSUMMARY_*` at 0x200.
//! The pad count stays 30 — the RP2350A that the Pico 2 carries
//! (`NUM_BANK0_GPIOS`); GPIO30..47 CTRL words of the RP2350B are inert.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::{Peripheral, PeripheralTickResult, SimResult};

/// Number of pads IO_BANK0 controls (GP0..GP29).
pub const PAD_COUNT: u8 = 30;

/// `CTRL.FUNCSEL` reset value: 31 == NULL, no peripheral connected.
const FUNCSEL_NULL: u32 = 31;

/// pico-sdk `gpio_function`. Only the ones the engine can route are named.
pub const GPIO_FUNC_SPI: u32 = 1;
pub const GPIO_FUNC_UART: u32 = 2;
pub const GPIO_FUNC_I2C: u32 = 3;
pub const GPIO_FUNC_PWM: u32 = 4;
/// `GPIO_FUNC_PIO0`: the pad is driven by the PIO0 output block.
pub const GPIO_FUNC_PIO0: u32 = 6;
/// `GPIO_FUNC_PIO1`: the pad is driven by the PIO1 output block.
pub const GPIO_FUNC_PIO1: u32 = 7;

/// Live per-pad function selection, shared with whoever needs to know which
/// peripheral owns a pad (the SIO GPIO model, for pad reads).
///
/// Shared rather than copied because the answer changes at runtime: firmware
/// re-assigns a pad and every reader must see it immediately, or a re-routed
/// pin keeps reporting from its old source.
#[derive(Debug)]
pub struct PadFunctions {
    ctrl: Vec<AtomicU32>,
}

impl Default for PadFunctions {
    fn default() -> Self {
        Self::new()
    }
}

impl PadFunctions {
    pub fn new() -> Self {
        Self {
            ctrl: (0..PAD_COUNT)
                .map(|_| AtomicU32::new(FUNCSEL_NULL))
                .collect(),
        }
    }

    /// The whole `GPIOn_CTRL` word.
    fn ctrl(&self, pin: u8) -> u32 {
        self.ctrl
            .get(pin as usize)
            .map_or(FUNCSEL_NULL, |c| c.load(Ordering::Relaxed))
    }

    fn set_ctrl(&self, pin: u8, value: u32) {
        if let Some(cell) = self.ctrl.get(pin as usize) {
            cell.store(value, Ordering::Relaxed);
        }
    }

    /// The function currently selected for `pin`, or `None` when the pad is
    /// NULL (nothing connected) or out of range.
    ///
    /// This is the selector [`crate::peripherals::pad_routing::PadRoutes`]
    /// resolves pad routes against.
    pub fn function(&self, pin: u8) -> Option<u32> {
        let funcsel = self.ctrl(pin) & 0x1F;
        (funcsel != FUNCSEL_NULL).then_some(funcsel)
    }
}

/// `GPIO_FUNC_SIO`: the pad is driven by the SIO output latch.
pub const GPIO_FUNC_SIO: u32 = 5;

/// Where the GPIO interrupt registers sit, and how wide each group is.
///
/// RP2040 (§2.19.6.1): `INTR0` at 0xF0, then the PROC0, PROC1 and
/// DORMANT_WAKE enable / force / status groups, four words each. RP2350
/// (pico-sdk rp2350 `io_bank0.h`): the GPIO array grows to 48 pads, so the
/// `IRQSUMMARY_*` words sit at 0x200 and `INTR0` moves to 0x230, with six
/// words per group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IoBankLayout {
    #[default]
    Rp2040,
    Rp2350,
}

impl IoBankLayout {
    /// Words per interrupt group (`INTRn`, `PROC0_INTEn`, ...).
    const fn words(self) -> usize {
        match self {
            IoBankLayout::Rp2040 => 4,
            IoBankLayout::Rp2350 => 6,
        }
    }

    const fn intr0(self) -> u64 {
        match self {
            IoBankLayout::Rp2040 => 0x0F0,
            IoBankLayout::Rp2350 => 0x230,
        }
    }

    /// Byte offset of interrupt group `g`: 0 `INTR`, 1 `PROC0_INTE`,
    /// 2 `PROC0_INTF`, 3 `PROC0_INTS`, 4..=9 the PROC1 and DORMANT_WAKE
    /// `INTE`/`INTF`/`INTS`.
    const fn group(self, g: u64) -> u64 {
        self.intr0() + g * 4 * self.words() as u64
    }

    /// `GPIOn_CTRL` writable bits: FUNCSEL plus the four OVER fields
    /// (RP2040 SVD: OUTOVER [9:8], OEOVER [13:12]; RP2350: [13:12], [15:14]).
    const fn ctrl_mask(self) -> u32 {
        match self {
            IoBankLayout::Rp2040 => 0x3003_331F,
            IoBankLayout::Rp2350 => 0x3003_F01F,
        }
    }
}

/// RP2350 `IRQSUMMARY_PROC0_SECURE0/1` (0x200/0x204): one bit per GPIO whose
/// `PROC0_INTS` nibble is non-zero. The other summary words (PROC1, NONSECURE,
/// DORMANT_WAKE) read zero: core 1 is not modelled, nothing sleeps, and with
/// the ACCESSCTRL reset `GPIO_NSMASK` of 0 no GPIO is visible to Non-secure.
const IRQSUMMARY_PROC0_SECURE0: u64 = 0x200;

/// The pad levels and raw edge latches the GPIO interrupts read, shared
/// between the SIO (which owns `GPIO_IN` and sees every change of it) and
/// IO_BANK0 (which owns the interrupt registers).
#[derive(Debug, Default)]
pub struct BankIrq {
    state: Mutex<BankIrqState>,
}

#[derive(Debug, Default, Clone, Copy)]
struct BankIrqState {
    /// `GPIO_IN` as last reported.
    level: u32,
    /// Latched `EDGE_LOW` / `EDGE_HIGH` per pad (bit `n` = GPIOn).
    edge_low: u32,
    edge_high: u32,
}

impl BankIrq {
    /// Report the pads' input levels; every change latches its edge.
    pub fn set_levels(&self, level: u32) {
        let mut s = self.state.lock().expect("RP2040 bank irq poisoned");
        let rose = !s.level & level;
        let fell = s.level & !level;
        s.edge_high |= rose;
        s.edge_low |= fell;
        s.level = level;
    }

    /// Seed the levels without latching anything (wiring time).
    pub fn seed_levels(&self, level: u32) {
        self.state.lock().expect("RP2040 bank irq poisoned").level = level;
    }

    fn snapshot(&self) -> BankIrqState {
        *self.state.lock().expect("RP2040 bank irq poisoned")
    }

    /// `INTRn` word `w`: pads `8w..8w+7`, four bits each — LEVEL_LOW,
    /// LEVEL_HIGH, EDGE_LOW, EDGE_HIGH.
    fn intr_word(&self, w: usize) -> u32 {
        let s = self.snapshot();
        let mut word = 0;
        for k in 0..8 {
            let pin = w * 8 + k;
            if pin >= PAD_COUNT as usize {
                break;
            }
            let bit = |v: u32| (v >> pin) & 1;
            let nib = (bit(!s.level))
                | (bit(s.level) << 1)
                | (bit(s.edge_low) << 2)
                | (bit(s.edge_high) << 3);
            word |= nib << (4 * k);
        }
        word
    }

    /// Write-1-to-clear on the edge bits of `INTRn` word `w`.
    fn clear_edges(&self, w: usize, value: u32) {
        let mut s = self.state.lock().expect("RP2040 bank irq poisoned");
        for k in 0..8 {
            let pin = w * 8 + k;
            if pin >= PAD_COUNT as usize {
                break;
            }
            let nib = (value >> (4 * k)) & 0xF;
            if nib & 0b0100 != 0 {
                s.edge_low &= !(1 << pin);
            }
            if nib & 0b1000 != 0 {
                s.edge_high &= !(1 << pin);
            }
        }
    }
}

/// The IO_BANK0 register block.
#[derive(Debug)]
pub struct Rp2040IoBank0 {
    pads: Arc<PadFunctions>,
    irq: Arc<BankIrq>,
    layout: IoBankLayout,
    /// `PROC0_INTE0..`, `PROC0_INTF0..`, one word per eight pads (stored and
    /// acted on).
    proc0_inte: [u32; 6],
    proc0_intf: [u32; 6],
    /// `PROC1_INTE/INTF/INTS`, `DORMANT_WAKE_INTE/INTF/INTS`: six groups,
    /// stored and read back.
    other: [u32; 36],
    /// Whether an `IO_IRQ_BANK0` event chain is live (see `on_event`).
    scheduled: bool,
    arm_seq: u32,
}

impl Default for Rp2040IoBank0 {
    fn default() -> Self {
        Self::new()
    }
}

impl Rp2040IoBank0 {
    pub fn new() -> Self {
        Self {
            pads: Arc::new(PadFunctions::new()),
            irq: Arc::new(BankIrq::default()),
            layout: IoBankLayout::Rp2040,
            proc0_inte: [0; 6],
            proc0_intf: [0; 6],
            other: [0; 36],
            scheduled: false,
            arm_seq: 0,
        }
    }

    /// Serve the given silicon's register map (the GPIO array is the same
    /// `8n` / `8n + 4` shape on both; the interrupt block moves).
    pub fn with_layout(mut self, layout: IoBankLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Share the live pad-function state, for the SIO GPIO model to resolve
    /// pad ownership against.
    pub fn pad_functions(&self) -> Arc<PadFunctions> {
        self.pads.clone()
    }

    /// Share the interrupt pad state, for the SIO to report `GPIO_IN` into.
    pub fn bank_irq(&self) -> Arc<BankIrq> {
        self.irq.clone()
    }

    /// `PROC0_INTSn`: raw status masked by the enables, OR the forces.
    fn proc0_ints(&self, w: usize) -> u32 {
        (self.irq.intr_word(w) & self.proc0_inte[w]) | self.proc0_intf[w]
    }

    /// `IO_IRQ_BANK0` as PROC0 sees it.
    fn irq_pending(&self) -> bool {
        (0..self.layout.words()).any(|w| self.proc0_ints(w) != 0)
    }

    /// RP2350 `IRQSUMMARY_PROC0_SECUREn`: pads `32n..` with a live
    /// `PROC0_INTS` nibble.
    fn proc0_summary(&self, n: usize) -> u32 {
        let mut bits = 0u64;
        for w in 0..self.layout.words() {
            let ints = self.proc0_ints(w);
            for k in 0..8 {
                if (ints >> (4 * k)) & 0xF != 0 {
                    bits |= 1 << (8 * w + k);
                }
            }
        }
        (bits >> (32 * n)) as u32
    }

    /// `(group, word)` for an offset inside the interrupt block.
    fn irq_reg(&self, offset: u64) -> Option<(u64, usize)> {
        let l = self.layout;
        if offset < l.intr0() || offset >= l.group(10) || offset % 4 != 0 {
            return None;
        }
        let rel = (offset - l.intr0()) / 4;
        let words = l.words() as u64;
        Some((rel / words, (rel % words) as usize))
    }

    /// `(pin, is_ctrl)` for a register offset inside the GPIO array.
    fn decode(offset: u64) -> Option<(u8, bool)> {
        let pin = (offset / 8) as u8;
        if pin >= PAD_COUNT {
            return None;
        }
        Some((pin, offset % 8 == 4))
    }

    fn read_u32(&self, offset: u64) -> u32 {
        if let Some((group, w)) = self.irq_reg(offset) {
            let words = self.layout.words();
            return match group {
                0 => self.irq.intr_word(w),
                1 => self.proc0_inte[w],
                2 => self.proc0_intf[w],
                3 => self.proc0_ints(w),
                g => self.other[(g as usize - 4) * words + w],
            };
        }
        if self.layout == IoBankLayout::Rp2350
            && (IRQSUMMARY_PROC0_SECURE0..IRQSUMMARY_PROC0_SECURE0 + 8).contains(&offset)
        {
            return self.proc0_summary(((offset - IRQSUMMARY_PROC0_SECURE0) / 4) as usize);
        }
        match Self::decode(offset) {
            Some((pin, true)) => self.pads.ctrl(pin),
            // GPIOn_STATUS: not modelled, reads zero.
            Some((_, false)) => 0,
            None => 0,
        }
    }

    fn write_u32(&mut self, offset: u64, value: u32) {
        if let Some((group, w)) = self.irq_reg(offset) {
            let words = self.layout.words();
            match group {
                // Edge bits are write-1-to-clear; level bits are read-only.
                0 => self.irq.clear_edges(w, value),
                1 => self.proc0_inte[w] = value,
                2 => self.proc0_intf[w] = value,
                3 => {}
                g => self.other[(g as usize - 4) * words + w] = value,
            }
            return;
        }
        if let Some((pin, true)) = Self::decode(offset) {
            // Writable bits per the SVD: FUNCSEL [4:0] and the four OVER
            // fields. Everything else reads back zero.
            self.pads.set_ctrl(pin, value & self.layout.ctrl_mask());
        }
    }
}

impl Peripheral for Rp2040IoBank0 {
    // The default `needs_legacy_walk() == true` stays: the GPIO interrupt
    // makes `tick()` pend `IO_IRQ_BANK0`. That does not cost the bus its walk
    // deletion, because this block is a `uses_scheduler()` model (the walk
    // derivation is `uses_scheduler() || !needs_legacy_walk()`), and on a bus
    // that still walks, `legacy_tick_active` keeps it out of the walk until
    // firmware enables an interrupt. Before the interrupt existed this was a
    // pure register bank and declared `false`.

    /// The GPIO interrupt is a LEVEL line (`PROC0_INTS` non-zero), so the walk
    /// ticks this block only while an interrupt can be up at all: while any
    /// enable or force bit is set. The bus re-derives this on every write to
    /// the block, which is where both are set.
    fn legacy_tick_active(&self) -> bool {
        self.proc0_inte
            .iter()
            .chain(&self.proc0_intf)
            .any(|w| *w != 0)
    }

    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult {
            irq: self.irq_pending(),
            ..PeripheralTickResult::default()
        }
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.irq_pending())
    }

    /// Delivered by a held-level event chain on a walk-free bus, the shape
    /// `Rp2040I2c` uses: a pad edge (reported by the SIO, which names this
    /// block as its [`scheduler_wake_owner`](Peripheral::scheduler_wake_owner))
    /// or an enable write arms it, `on_event` pends `IO_IRQ_BANK0` and re-arms
    /// at delay 1 while the line holds, and stops once firmware acknowledges.
    fn uses_scheduler(&self) -> bool {
        true
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.irq_pending() || self.scheduled {
            return Vec::new();
        }
        self.arm_seq = self.arm_seq.wrapping_add(1);
        self.scheduled = true;
        vec![(0, self.arm_seq)]
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if event_token != self.arm_seq {
            return crate::sched::EventResult::default();
        }
        let pending = self.irq_pending();
        self.scheduled = pending;
        crate::sched::EventResult {
            raise_own_irq: pending,
            reschedule_delay: pending.then_some(1),
            ..Default::default()
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !3);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        let shift = (offset & 3) * 8;
        let word = (self.read_u32(aligned) & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_u32(aligned, word);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(Rp2040IoBank0::read_u32(self, offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        Rp2040IoBank0::write_u32(self, offset, value);
        Ok(())
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

    fn ctrl_offset(pin: u8) -> u64 {
        u64::from(pin) * 8 + 4
    }

    #[test]
    fn every_pad_resets_to_null_not_to_function_zero() {
        // FUNCSEL 0 is XIP, a real function. A model that reset to 0 would
        // claim every pad was driven by the flash interface from power-on.
        let bank = Rp2040IoBank0::new();
        let pads = bank.pad_functions();
        for pin in 0..PAD_COUNT {
            assert_eq!(pads.function(pin), None, "GP{pin} must start unconnected");
        }
        assert_eq!(bank.read_u32(ctrl_offset(0)) & 0x1F, FUNCSEL_NULL);
    }

    #[test]
    fn selecting_a_function_is_visible_to_a_sharer_immediately() {
        let mut bank = Rp2040IoBank0::new();
        let pads = bank.pad_functions();
        Rp2040IoBank0::write_u32(&mut bank, ctrl_offset(4), GPIO_FUNC_I2C);
        assert_eq!(pads.function(4), Some(GPIO_FUNC_I2C));
        assert_eq!(pads.function(5), None, "only the written pad moved");
    }

    #[test]
    fn a_pad_handed_back_to_null_stops_reporting_a_function() {
        let mut bank = Rp2040IoBank0::new();
        let pads = bank.pad_functions();
        Rp2040IoBank0::write_u32(&mut bank, ctrl_offset(9), GPIO_FUNC_UART);
        assert_eq!(pads.function(9), Some(GPIO_FUNC_UART));
        Rp2040IoBank0::write_u32(&mut bank, ctrl_offset(9), FUNCSEL_NULL);
        assert_eq!(pads.function(9), None);
    }

    #[test]
    fn ctrl_words_read_back_what_firmware_wrote() {
        let mut bank = Rp2040IoBank0::new();
        Rp2040IoBank0::write_u32(&mut bank, ctrl_offset(2), GPIO_FUNC_SPI);
        assert_eq!(bank.read_u32(ctrl_offset(2)), GPIO_FUNC_SPI);
        // Reserved bits do not stick.
        Rp2040IoBank0::write_u32(&mut bank, ctrl_offset(2), 0xFFFF_FFFF);
        assert_eq!(bank.read_u32(ctrl_offset(2)), 0x3003_331F);
    }

    #[test]
    fn status_is_not_invented() {
        // GPIOn_STATUS is real silicon state this model does not derive; it
        // must read zero rather than a plausible-looking guess.
        let bank = Rp2040IoBank0::new();
        assert_eq!(bank.read_u32(0), 0);
        assert_eq!(bank.read_u32(8), 0);
    }

    #[test]
    fn byte_writes_compose_into_the_same_word() {
        let mut bank = Rp2040IoBank0::new();
        bank.write(ctrl_offset(7), GPIO_FUNC_I2C as u8).unwrap();
        assert_eq!(bank.pad_functions().function(7), Some(GPIO_FUNC_I2C));
    }

    #[test]
    fn offsets_past_the_last_pad_are_inert() {
        // The word after GPIO29_CTRL (0xEC) is INTR0 (0xF0), a real register;
        // a CTRL-shaped write one pad further lands in INTR1 and must not
        // select a function for any pad.
        let mut bank = Rp2040IoBank0::new();
        let pads = bank.pad_functions();
        Rp2040IoBank0::write_u32(&mut bank, ctrl_offset(PAD_COUNT), GPIO_FUNC_I2C);
        assert!((0..PAD_COUNT).all(|pin| pads.function(pin).is_none()));
        // Past DORMANT_WAKE_INTS3 (0x18C) nothing is mapped.
        Rp2040IoBank0::write_u32(&mut bank, 0x190, GPIO_FUNC_I2C);
        assert_eq!(bank.read_u32(0x190), 0);
    }

    /// RP2350 map: `PROC0_INTE0` at 0x248, `PROC0_INTS0` at 0x278,
    /// `IRQSUMMARY_PROC0_SECURE0` at 0x200, and the RP2350 CTRL OVER bits.
    #[test]
    fn rp2350_layout_moves_the_interrupt_block() {
        use crate::peripherals::rp2040::sio::{Rp2040Sio, SioLayout};
        let mut bank = Rp2040IoBank0::new().with_layout(IoBankLayout::Rp2350);
        let mut sio = Rp2040Sio::new().with_layout(SioLayout::Rp2350);
        sio.attach_io_bank0(bank.pad_functions(), bank.bank_irq(), 0);
        // CTRL: same array, PWM is still FUNCSEL 4; OEOVER is [15:14] here.
        Rp2040IoBank0::write_u32(&mut bank, ctrl_offset(25), 0xFFFF_FFE4);
        assert_eq!(bank.read_u32(ctrl_offset(25)), 0x3003_F004);
        assert_eq!(bank.pad_functions().function(25), Some(GPIO_FUNC_PWM));
        // The RP2040 INTR0 offset is plain GPIO-array space here (GPIO30).
        Rp2040IoBank0::write_u32(&mut bank, 0x100, 0xFFFF_FFFF);
        assert_eq!(bank.read_u32(0x100), 0);
        // GP9 rising edge -> INTR1 nibble 1, gated by PROC0_INTE1 (0x24C).
        let edge_high_gp9 = 1u32 << (4 + 3);
        Rp2040IoBank0::write_u32(&mut bank, 0x24C, edge_high_gp9);
        sio.set_gpio_input(9, true);
        assert_ne!(bank.read_u32(0x234) & edge_high_gp9, 0, "INTR1");
        assert_eq!(bank.read_u32(0x27C), edge_high_gp9, "PROC0_INTS1");
        assert_eq!(bank.read_u32(0x200), 1 << 9, "IRQSUMMARY_PROC0_SECURE0");
        assert_eq!(bank.read_u32(0x208), 0, "nothing is Non-secure");
        assert_eq!(bank.irq_line_level(), Some(true));
        // W1C at the RP2350 INTR1, and DORMANT_WAKE_INTS5 (0x31C) is the last word.
        Rp2040IoBank0::write_u32(&mut bank, 0x234, edge_high_gp9);
        assert_eq!(bank.irq_line_level(), Some(false));
        Rp2040IoBank0::write_u32(&mut bank, 0x31C, 7);
        assert_eq!(bank.read_u32(0x31C), 7);
        Rp2040IoBank0::write_u32(&mut bank, 0x320, 7);
        assert_eq!(bank.read_u32(0x320), 0);
    }

    /// The GPIO interrupt (§2.19.3): SIO reports `GPIO_IN`, edges latch in
    /// `INTRn` (write-1-to-clear), level bits are live, `PROC0_INTS` masks by
    /// `PROC0_INTE`, and `IO_IRQ_BANK0` holds while any `PROC0_INTS` bit is
    /// set. Edges from the pad's own output count too.
    #[test]
    fn pad_edges_latch_intr_and_raise_io_irq_bank0() {
        use crate::peripherals::rp2040::sio::Rp2040Sio;
        use crate::Peripheral;
        let mut bank = Rp2040IoBank0::new();
        let mut sio = Rp2040Sio::new();
        sio.attach_io_bank0(bank.pad_functions(), bank.bank_irq(), 0);
        const INTR0: u64 = 0xF0;
        const INTE0: u64 = 0x100;
        const INTS0: u64 = 0x120;
        let edge_low = |pin: u32| 1u32 << (4 * pin + 2);
        let edge_high = |pin: u32| 1u32 << (4 * pin + 3);
        let level_high = |pin: u32| 1u32 << (4 * pin + 1);
        // GP3 idle low: LEVEL_LOW live, no edge yet.
        assert_eq!(bank.read_u32(INTR0) & (0xF << 12), 1 << 12);
        assert_eq!(bank.irq_line_level(), Some(false));
        Rp2040IoBank0::write_u32(&mut bank, INTE0, edge_high(3) | edge_low(4));
        assert!(bank.legacy_tick_active());
        sio.set_gpio_input(3, true);
        assert_eq!(
            bank.read_u32(INTR0) & (0xF << 12),
            level_high(3) | edge_high(3)
        );
        assert_eq!(bank.read_u32(INTS0), edge_high(3));
        assert_eq!(bank.irq_line_level(), Some(true));
        assert!(bank.tick().irq);
        assert_eq!(bank.take_scheduled_events().len(), 1, "arms the chain");
        assert!(bank.take_scheduled_events().is_empty(), "once per chain");
        // Acknowledge: W1C on the edge bit only.
        Rp2040IoBank0::write_u32(&mut bank, INTR0, edge_high(3) | level_high(3));
        assert_eq!(bank.read_u32(INTS0), 0);
        assert_eq!(bank.irq_line_level(), Some(false));
        // The pad's own output: GP4 driven high then low latches EDGE_LOW.
        sio.write_u32(0x24, 1 << 4).unwrap();
        sio.write_u32(0x14, 1 << 4).unwrap();
        assert_eq!(bank.read_u32(INTS0), 0, "rising is not enabled on GP4");
        sio.write_u32(0x18, 1 << 4).unwrap();
        assert_eq!(bank.read_u32(INTS0), edge_low(4));
        // PROC0_INTF forces a bit without an edge.
        Rp2040IoBank0::write_u32(&mut bank, INTR0, edge_low(4));
        Rp2040IoBank0::write_u32(&mut bank, 0x114, edge_high(1));
        assert_eq!(bank.read_u32(INTS0), 0);
        assert_eq!(bank.read_u32(INTS0 + 4), edge_high(1), "GP9 is word 1");
    }
}
