// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// ── Architectural separation ────────────────────────────────────────────────
// EXTI is one struct PER FAMILY behind the `Exti` enum. The F1 variant is a
// single 20-line bank; the L4 variant adds bank 2 (lines 32..39). The bank-2
// registers therefore exist ONLY on the L4 variant — an F1 EXTI cannot carry
// (or be tricked into addressing) bank-2 state. Bank-1 IRQ routing, shared by
// both families, lives in one stateless helper.

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};
use std::any::Any;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtiRegisterLayout {
    /// STM32F1 / F4-class: single bank, 20 lines, registers at 0x00..0x14.
    #[default]
    Stm32F1,
    /// STM32L4: two banks (40 lines total). Bank1 at 0x00..0x14, bank2 at
    /// 0x20..0x34. Bank-2 covers lines 32..39 (LPTIM/COMP/I2C/USART wakeup).
    Stm32L4,
    /// STM32G0 GPIO lines: split rising/falling pending and EXTI-owned mux.
    Stm32G0,
    /// STM32U5 GPIO lines: split pending, 4-bit mux, individual IRQs 11..26.
    Stm32U5,
    /// STM32L0 (RM0367, EXTI chapter): the F1 register file (IMR/EMR/RTSR/FTSR/SWIER/
    /// PR at 0x00..0x14, port mux in `SYSCFG_EXTICRx`), with the Cortex-M0+
    /// grouped vectors: EXTI0_1 = IRQ 5, EXTI2_3 = 6, EXTI4_15 = 7.
    Stm32L0,
}

impl FromStr for ExtiRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            "stm32l4" | "l4" => Ok(Self::Stm32L4),
            "stm32g0" | "g0" => Ok(Self::Stm32G0),
            "stm32u5" | "u5" => Ok(Self::Stm32U5),
            "stm32l0" | "l0" => Ok(Self::Stm32L0),
            _ => Err(format!(
                "unsupported EXTI register layout '{}'; supported: stm32f1, stm32l4, stm32g0, stm32u5, stm32l0",
                value
            )),
        }
    }
}

/// One EXTI register bank (IMR/EMR/RTSR/FTSR/SWIER/PR).
#[derive(Debug, Default, Clone, Copy, serde::Serialize)]
struct ExtiBank {
    imr: u32,
    emr: u32,
    rtsr: u32,
    ftsr: u32,
    swier: u32,
    pr: u32,
}

impl ExtiBank {
    /// IMR/EMR/RTSR/FTSR read at 0x00/0x04/0x08/0x0C; SWIER 0x10; PR 0x14.
    fn read(&self, off: u64) -> u32 {
        match off {
            0x00 => self.imr,
            0x04 => self.emr,
            0x08 => self.rtsr,
            0x0C => self.ftsr,
            0x10 => self.swier,
            0x14 => self.pr,
            _ => {
                crate::census_reg!("exti:ExtiBank", off, "read");
                0
            }
        }
    }
    /// `mask` is the implemented-line mask for this bank.
    fn write(&mut self, off: u64, value: u32, mask: u32) {
        match off {
            0x00 => self.imr = value & mask,
            0x04 => self.emr = value & mask,
            0x08 => self.rtsr = value & mask,
            0x0C => self.ftsr = value & mask,
            0x10 => {
                // SWIER: a 0->1 edge sets the matching PR bit.
                let diff = (self.swier ^ value) & value & mask;
                self.swier = value & mask;
                self.pr |= diff;
            }
            0x14 => {
                // PR is rc_w1: writing 1 clears the pending bit. Clearing a PR
                // bit also clears the matching SWIER bit (RM0008 §10.3.6) — the
                // software-event line de-asserts. Silicon-verified on the bench
                // STM32F103 (stm32f1_exec_oracle::exti_swier_sets_and_clears_pr).
                let clear = value & mask;
                self.pr &= !clear;
                self.swier &= !clear;
            }
            _ => {
                crate::census_reg!("exti:ExtiBank", off, "write");
            }
        }
    }
}

/// Bank-1 IRQ routing — identical on every family (lines 0..4 → IRQ 6..10,
/// 9..5 → 23, 15..10 → 40). Shared behaviour, not shared state.
fn route_bank1_irqs(active1: u32, irqs: &mut Vec<u32>) {
    for i in 0..5 {
        if (active1 & (1 << i)) != 0 {
            irqs.push(6 + i);
        }
    }
    if (active1 & 0x0000_03E0) != 0 {
        irqs.push(23); // EXTI9_5
    }
    if (active1 & 0x0000_FC00) != 0 {
        irqs.push(40); // EXTI15_10
    }
}

/// The STM32L0 / G0 grouped GPIO vectors: lines 0..1 → IRQ 5 (EXTI0_1),
/// 2..3 → 6 (EXTI2_3), 4..15 → 7 (EXTI4_15), per the RM0367 and RM0444
/// vector tables.
const M0_GPIO_GROUPS: [(u32, u32); 3] = [(0x0003, 5), (0x000c, 6), (0xfff0, 7)];

/// Bank-1 GPIO-group NVIC line LEVELS (the level twin of
/// [`route_bank1_irqs`]): EXTI0..4 → IRQ 6..10, EXTI9_5 → 23, EXTI15_10 → 40.
/// Lines 16+ (PVD, RTC, USB/OTG wakeup, …) route to IRQs another peripheral may
/// also drive, so they are not reported and keep pulse semantics.
fn report_bank1_levels(active1: u32, report: &mut dyn FnMut(u32, bool)) {
    for i in 0..5 {
        report(6 + i, active1 & (1 << i) != 0);
    }
    report(23, active1 & 0x0000_03E0 != 0);
    report(40, active1 & 0x0000_FC00 != 0);
}

// ── STM32F1 / F4: single bank ────────────────────────────────────────────────
// The implemented-line count is part-specific (F103 = 19 lines, F4-class = more),
// so the mask is per-instance, set from the chip config's `lines` field. Default
// is the historical 20-line value for parts not yet silicon-pinned.
#[derive(Debug, serde::Serialize)]
pub struct F1Exti {
    bank1: ExtiBank,
    line_mask: u32,

    /// Bus-published cycle clock (walk-free campaign). `Some` once attached →
    /// event-schedulable; `None` keeps the legacy walk.
    #[serde(skip)]
    clock: Option<CycleClock>,
    /// Scheduler mode: `true` while the held-level re-emit event is live.
    #[serde(skip)]
    chain_live: bool,
}

impl Default for F1Exti {
    fn default() -> Self {
        Self {
            bank1: ExtiBank::default(),
            line_mask: 0x000F_FFFF, // 20 lines
            clock: None,
            chain_live: false,
        }
    }
}

// ── STM32L4: two banks (bank 2 = lines 32..39) ───────────────────────────────
#[derive(Debug, Default, serde::Serialize)]
pub struct L4Exti {
    bank1: ExtiBank,
    bank2: ExtiBank,

    #[serde(skip)]
    clock: Option<CycleClock>,
    #[serde(skip)]
    chain_live: bool,
}

impl L4Exti {
    const MASK1: u32 = 0xFFFF_FFFF; // full word for bank 1
    const MASK2: u32 = 0x0000_00FF; // lines 32..39
}

/// RM0444/RM0456 split-pending GPIO EXTI bank. Internal peripheral event lines and EMR event
/// delivery are not synthesized by this GPIO interrupt path.
#[derive(Debug, Default, serde::Serialize)]
pub struct SplitPendingGpioExti {
    rtsr: u32,
    ftsr: u32,
    rpr: u32,
    fpr: u32,
    imr: u32,
    emr: u32,
    exticr: [u32; 4],
    #[serde(skip)]
    clock: Option<CycleClock>,
    #[serde(skip)]
    chain_live: bool,
}

/// Compatibility name for the original G0-only bank.
pub type G0Exti = SplitPendingGpioExti;

impl SplitPendingGpioExti {
    fn read(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.rtsr,
            0x04 => self.ftsr,
            0x0c => self.rpr,
            0x10 => self.fpr,
            0x60..=0x6c => self.exticr[((offset - 0x60) / 4) as usize],
            0x80 => self.imr,
            0x84 => self.emr,
            _ => {
                crate::census_reg!("exti:G0Exti", offset, "read");
                0
            }
        }
    }
    fn write(&mut self, offset: u64, value: u32, mux_mask: u32) {
        match offset {
            0x00 => self.rtsr = value & 0xffff,
            0x04 => self.ftsr = value & 0xffff,
            0x0c => self.rpr &= !value,
            0x10 => self.fpr &= !value,
            0x60..=0x6c => self.exticr[((offset - 0x60) / 4) as usize] = value & mux_mask,
            0x80 => self.imr = value & 0xffff,
            0x84 => self.emr = value & 0xffff,
            _ => {
                crate::census_reg!("exti:G0Exti", offset, "write");
            }
        }
    }
    fn active(&self) -> u32 {
        (self.rpr | self.fpr) & self.imr
    }
}

/// External Interrupt/Event Controller — one variant per chip family.
#[derive(Debug, serde::Serialize)]
pub enum Exti {
    Stm32F1(F1Exti),
    Stm32L4(L4Exti),
    Stm32G0(SplitPendingGpioExti),
    Stm32U5(SplitPendingGpioExti),
    /// The F1 register file with the L0 vector routing.
    Stm32L0(F1Exti),
}

impl Default for Exti {
    fn default() -> Self {
        Self::new()
    }
}

impl Exti {
    pub fn new() -> Self {
        Self::new_with_layout(ExtiRegisterLayout::Stm32F1)
    }

    pub fn new_with_layout(layout: ExtiRegisterLayout) -> Self {
        Self::new_with_layout_lines(layout, 0x000F_FFFF)
    }

    /// Like [`new_with_layout`] but with an explicit F1 implemented-line mask
    /// (e.g. `0x0007_FFFF` for the STM32F103's 19 lines). Ignored for L4.
    pub fn new_with_layout_lines(layout: ExtiRegisterLayout, line_mask: u32) -> Self {
        match layout {
            ExtiRegisterLayout::Stm32F1 => Self::Stm32F1(F1Exti {
                line_mask,
                ..Default::default()
            }),
            ExtiRegisterLayout::Stm32L4 => Self::Stm32L4(L4Exti::default()),
            ExtiRegisterLayout::Stm32G0 => Self::Stm32G0(SplitPendingGpioExti::default()),
            ExtiRegisterLayout::Stm32U5 => Self::Stm32U5(SplitPendingGpioExti::default()),
            ExtiRegisterLayout::Stm32L0 => Self::Stm32L0(F1Exti {
                line_mask,
                ..Default::default()
            }),
        }
    }

    /// Inject an external trigger on `line` (sets the corresponding PR bit).
    /// Bank-2 lines (32..39) exist only on the L4 variant.
    pub fn trigger_line(&mut self, line: u8) {
        match self {
            Self::Stm32G0(e) | Self::Stm32U5(e) => {
                if line < 16 {
                    e.rpr |= 1 << line;
                }
            }
            Self::Stm32F1(e) | Self::Stm32L0(e) => {
                if line < 32 {
                    e.bank1.pr |= 1u32 << line;
                }
            }
            Self::Stm32L4(e) => match line {
                0..=31 => e.bank1.pr |= 1u32 << line,
                32..=39 => e.bank2.pr |= 1u32 << (line - 32),
                _ => {}
            },
        }
    }

    /// Apply an actual GPIO edge, respecting the G0/U5 port mux and polarity.
    /// Returns true when this edge creates a pending GPIO interrupt flag.
    /// The F1/F4 and L4 layouts keep their port mux outside EXTI; see
    /// [`Self::gpio_edge_with_source`].
    pub fn gpio_edge(&mut self, port: u8, line: u8, before: bool, after: bool) -> bool {
        self.gpio_edge_with_source(port, line, before, after, None)
    }

    /// As [`Self::gpio_edge`], with the port the chip's external line-source
    /// mux selects for `line` (`AFIO_EXTICRx` on F1, `SYSCFG_EXTICRx` on F4;
    /// `None` when the chip has no such mux on the bus).
    ///
    /// G0/U5 select the port in their own `EXTI_EXTICRx` and ignore
    /// `line_source`. The single-bank F1/F4 layout (and L4 bank 1) has no mux
    /// of its own: the edge counts only when `line_source` names `port`, and
    /// then sets PR for the line if RTSR (rising) or FTSR (falling) selects
    /// it, whatever IMR says — IMR gates the interrupt, not the pending bit
    /// (RM0008 §10.2.5, RM0090 §12.2.5). Returns true when PR was set.
    pub fn gpio_edge_with_source(
        &mut self,
        port: u8,
        line: u8,
        before: bool,
        after: bool,
        line_source: Option<u8>,
    ) -> bool {
        if line >= 16 || before == after {
            return false;
        }
        let bit = 1u32 << line;
        let bank = match self {
            Self::Stm32G0(e) | Self::Stm32U5(e) => {
                let mux = (e.exticr[usize::from(line / 4)] >> (u32::from(line % 4) * 8)) & 0xff;
                if mux != u32::from(port) {
                    return false;
                }
                if after && e.rtsr & bit != 0 {
                    e.rpr |= bit;
                    return true;
                }
                if !after && e.ftsr & bit != 0 {
                    e.fpr |= bit;
                    return true;
                }
                return false;
            }
            Self::Stm32F1(e) | Self::Stm32L0(e) => &mut e.bank1,
            Self::Stm32L4(e) => &mut e.bank1,
        };
        if line_source != Some(port) {
            return false;
        }
        let selected = if after { bank.rtsr } else { bank.ftsr };
        if selected & bit == 0 {
            return false;
        }
        bank.pr |= bit;
        true
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match self {
            Self::Stm32G0(e) | Self::Stm32U5(e) => e.read(offset),
            Self::Stm32F1(e) | Self::Stm32L0(e) => match offset {
                0x00..=0x14 => e.bank1.read(offset),
                _ => {
                    crate::census_reg!("exti:Exti", offset, "read");
                    0
                }
            },
            Self::Stm32L4(e) => match offset {
                0x00..=0x14 => e.bank1.read(offset),
                0x20..=0x34 => e.bank2.read(offset - 0x20),
                _ => {
                    crate::census_reg!("exti:Exti", offset, "read");
                    0
                }
            },
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match self {
            Self::Stm32G0(e) => e.write(offset, value, 0x07070707),
            Self::Stm32U5(e) => e.write(offset, value, 0x0f0f0f0f),
            Self::Stm32F1(e) | Self::Stm32L0(e) => {
                if (0x00..=0x14).contains(&offset) {
                    let mask = e.line_mask;
                    e.bank1.write(offset, value, mask);
                }
            }
            Self::Stm32L4(e) => match offset {
                0x00..=0x14 => e.bank1.write(offset, value, L4Exti::MASK1),
                0x20..=0x34 => e.bank2.write(offset - 0x20, value, L4Exti::MASK2),
                _ => {
                    crate::census_reg!("exti:Exti", offset, "write");
                }
            },
        }
    }

    /// The set of NVIC IRQ lines the held level asserts this cycle — exactly the
    /// list `tick()` re-emits. Shared by the legacy walk and the event chain so
    /// both routes are byte-identical by construction.
    fn pending_irqs(&self) -> Vec<u32> {
        let mut irqs = Vec::new();
        match self {
            Self::Stm32U5(e) => {
                for line in 0..16 {
                    if e.active() & (1 << line) != 0 {
                        irqs.push(11 + line);
                    }
                }
            }
            Self::Stm32G0(e) => {
                for (mask, irq) in M0_GPIO_GROUPS {
                    if e.active() & mask != 0 {
                        irqs.push(irq);
                    }
                }
            }
            Self::Stm32L0(e) => {
                // Lines 16+ (PVD, RTC, COMP, wakeups) route to other vectors
                // this model does not synthesize; they stay register-level.
                let active = e.bank1.pr & e.bank1.imr;
                for (mask, irq) in M0_GPIO_GROUPS {
                    if active & mask != 0 {
                        irqs.push(irq);
                    }
                }
            }
            Self::Stm32F1(e) => {
                let active1 = e.bank1.pr & e.bank1.imr;
                if active1 != 0 {
                    route_bank1_irqs(active1, &mut irqs);
                }
            }
            Self::Stm32L4(e) => {
                let active1 = e.bank1.pr & e.bank1.imr;
                let active2 = e.bank2.pr & e.bank2.imr;
                if active1 != 0 {
                    route_bank1_irqs(active1, &mut irqs);
                }
                if active2 != 0 {
                    // Bank-2 wakeup lines → their peripheral's NVIC IRQ
                    // (RM0351 §13.3). Lines without an entry are tracked at the
                    // register level but don't synthesize an IRQ yet.
                    for &(line, irq) in &[
                        (35u32, 70u32), // LPUART1 wakeup
                        (36, 31),       // I2C1 wakeup
                        (37, 33),       // I2C2 wakeup
                        (38, 72),       // I2C3 wakeup
                        (39, 37),       // USART1 wakeup
                    ] {
                        if (active2 >> (line - 32)) & 1 != 0 {
                            irqs.push(irq);
                        }
                    }
                }
            }
        }
        irqs
    }

    /// True while the held level is asserted (any masked pending line). Outside
    /// this window `tick()` emits nothing, so the event chain may stop and let
    /// idle fast-forward engage; firmware clearing PR (rc_w1) drops it.
    fn active(&self) -> bool {
        match self {
            Self::Stm32G0(e) | Self::Stm32U5(e) => e.active() != 0,
            Self::Stm32F1(e) | Self::Stm32L0(e) => (e.bank1.pr & e.bank1.imr) != 0,
            Self::Stm32L4(e) => (e.bank1.pr & e.bank1.imr) != 0 || (e.bank2.pr & e.bank2.imr) != 0,
        }
    }

    #[inline]
    /// The level-triggered tick result: every masked, pending EXTI line routed
    /// to its NVIC IRQ. Shared by the legacy walk and the hardware-oracle forced
    /// walk so the two can never drift apart.
    fn level_tick_result(&mut self) -> PeripheralTickResult {
        let irqs = self.pending_irqs();
        PeripheralTickResult {
            explicit_irqs: (!irqs.is_empty()).then_some(irqs),
            ..Default::default()
        }
    }

    fn scheduler_mode(&self) -> bool {
        let clock = match self {
            Self::Stm32G0(e) | Self::Stm32U5(e) => &e.clock,
            Self::Stm32F1(e) | Self::Stm32L0(e) => &e.clock,
            Self::Stm32L4(e) => &e.clock,
        };
        cfg!(feature = "event-scheduler") && clock.is_some()
    }

    fn set_chain_live(&mut self, live: bool) {
        match self {
            Self::Stm32G0(e) | Self::Stm32U5(e) => e.chain_live = live,
            Self::Stm32F1(e) | Self::Stm32L0(e) => e.chain_live = live,
            Self::Stm32L4(e) => e.chain_live = live,
        }
    }

    fn chain_live(&self) -> bool {
        match self {
            Self::Stm32G0(e) | Self::Stm32U5(e) => e.chain_live,
            Self::Stm32F1(e) | Self::Stm32L0(e) => e.chain_live,
            Self::Stm32L4(e) => e.chain_live,
        }
    }

    /// Test/differential knob: detach the clock, pinning the model to the legacy
    /// walk (the walk-on reference for the differential gate).
    pub fn force_legacy_walk(&mut self) {
        match self {
            Self::Stm32G0(e) | Self::Stm32U5(e) => e.clock = None,
            Self::Stm32F1(e) | Self::Stm32L0(e) => e.clock = None,
            Self::Stm32L4(e) => e.clock = None,
        }
    }
}

impl Peripheral for Exti {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;

        if matches!(self, Self::Stm32G0(_) | Self::Stm32U5(_)) && matches!(reg_offset, 0x0c | 0x10)
        {
            self.write_reg(reg_offset, u32::from(value) << (byte_offset * 8));
            return Ok(());
        }

        let mut reg_val = self.read_reg(reg_offset);
        let mask = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);

        self.write_reg(reg_offset, reg_val);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        // SWIER (0->1 edge-detect → sets PR) and PR (rc_w1) only behave
        // correctly under whole-word access. The default byte-decomposition
        // reads back the current value for the un-targeted bytes and writes it
        // back: for the rc_w1 PR that clears still-pending bits (write-1-clear),
        // and it mis-fires SWIER's edge detector. Silicon performs the STR as
        // one 32-bit transaction; mirror that by handing write_reg the whole
        // word. Silicon-verified on bench STM32F103 (stm32f1_exec_oracle::exti_*).
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Scheduler-mode instances are walk-skipped; the event chain owns the
        // held-level re-emission. Guard against a stray direct call.
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        self.level_tick_result()
    }

    /// Hardware-oracle settle mode freezes the CPU and deliberately asks for the
    /// pre-scheduler one-tick level emission, so the `scheduler_mode()` no-op in
    /// [`Self::tick`] must NOT apply here — that guard exists to catch a stray
    /// walk call in production, and a forced tick is the opposite of stray.
    ///
    /// Without this override the guard silently swallowed the forced call and
    /// EXTI delivered NO interrupt to the bare-CPU oracle: `exti0_interrupt_
    /// delivery_sim` read `mem[0x20000300] == 0` because the ISR never ran. It
    /// was invisible except under `cargo test --workspace`, where Cargo's
    /// feature unification switches `event-scheduler` on for a crate that does
    /// not request it itself — so `-p labwired-hw-oracle` passed and the
    /// workspace lane failed. Same contract as the DMA models, which already
    /// override this for the same reason.
    ///
    /// This never runs from production `Machine` execution; there the event
    /// chain (`take_scheduled_events` / `on_event`) remains the sole owner of
    /// scheduler-mode EXTI delivery.
    fn tick_elapsed_forced(&mut self, _cycles: u64) -> PeripheralTickResult {
        self.level_tick_result()
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        match self {
            Self::Stm32G0(e) | Self::Stm32U5(e) => e.clock = Some(clock),
            Self::Stm32F1(e) | Self::Stm32L0(e) => e.clock = Some(clock),
            Self::Stm32L4(e) => e.clock = Some(clock),
        }
    }

    fn sync_to(&mut self, _now_cycle: u64) {
        // No lazily-accumulated state: PR/IMR mutate synchronously in write, and
        // the held level is re-derived from them every cycle by the event chain.
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        // Arm the held-level re-emit chain the moment a write raises a masked
        // pending line (SWIER→PR, or IMR unmasking a pending PR), or an
        // external GPIO edge sets PR (`gpio_input_edge`): MMIO writes are
        // drained after every write, and the bus drains this after an edge
        // that returned true. (`trigger_line` has no runtime caller; were it
        // ever wired to a bus path, that path must re-arm the chain the same
        // way.) delay-0 → deadline `current_cycle + 1` = the walk's next tick.
        if self.scheduler_mode() && self.active() && !self.chain_live() {
            self.set_chain_live(true);
            vec![(0u64, 0u32)]
        } else {
            Vec::new()
        }
    }

    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() {
            return crate::sched::EventResult::default();
        }
        // Re-emit the held level every cycle while any masked line stays pending
        // — the event-path equivalent of the legacy `tick()` returning its
        // `explicit_irqs` each cycle. Perpetuate at delay 1 while active; stop
        // when firmware clears PR (rc_w1), letting fast-forward engage.
        let active = self.active();
        self.set_chain_live(active);
        crate::sched::EventResult {
            explicit_irqs: self.pending_irqs(),
            reschedule_delay: active.then_some(1),
            ..Default::default()
        }
    }

    /// Per-NVIC-line levels of the GPIO EXTI groups, so the bus drops the
    /// NVIC pend the held level raised while the handler ran once firmware
    /// clears the pending latch (rc_w1). Without it a handler that clears PR
    /// without re-checking it ran twice per edge. Only the lines EXTI owns
    /// exclusively are reported (see [`report_bank1_levels`]); the L4 bank-2
    /// wakeup lines share their IRQ with the UART / I2C they wake and stay
    /// pulse-delivered.
    fn irq_line_levels(&self, report: &mut dyn FnMut(u32, bool)) {
        match self {
            Self::Stm32U5(e) => {
                let active = e.active();
                for line in 0..16 {
                    report(11 + line, active & (1 << line) != 0);
                }
            }
            Self::Stm32G0(e) => {
                let active = e.active();
                for (mask, irq) in M0_GPIO_GROUPS {
                    report(irq, active & mask != 0);
                }
            }
            Self::Stm32L0(e) => {
                let active = e.bank1.pr & e.bank1.imr;
                for (mask, irq) in M0_GPIO_GROUPS {
                    report(irq, active & mask != 0);
                }
            }
            Self::Stm32F1(e) => report_bank1_levels(e.bank1.pr & e.bank1.imr, report),
            Self::Stm32L4(e) => report_bank1_levels(e.bank1.pr & e.bank1.imr, report),
        }
    }

    fn gpio_input_edge(
        &mut self,
        port: u8,
        pin: u8,
        before: bool,
        after: bool,
        line_source: Option<u8>,
    ) -> bool {
        self.gpio_edge_with_source(port, pin, before, after, line_source)
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::{Exti, ExtiRegisterLayout};
    use crate::Peripheral;

    fn poke(exti: &mut Exti, off: u64, val: u32) {
        for i in 0..4 {
            exti.write(off + i, ((val >> (i * 8)) & 0xFF) as u8)
                .unwrap();
        }
    }

    #[test]
    fn l4_bank2_line35_routes_to_lpuart1_irq() {
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32L4);
        // Arm IMR2 line 3 (= EXTI line 35), then trigger via SWIER2.
        poke(&mut e, 0x20, 1 << 3);
        poke(&mut e, 0x30, 1 << 3);
        let r = e.tick();
        let irqs = r.explicit_irqs.expect("expected IRQ list");
        assert!(
            irqs.contains(&70),
            "LPUART1 IRQ 70 should fire, got {irqs:?}"
        );
    }

    #[test]
    fn l4_bank2_line38_routes_to_i2c3_irq() {
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32L4);
        poke(&mut e, 0x20, 1 << 6);
        poke(&mut e, 0x30, 1 << 6);
        let r = e.tick();
        let irqs = r.explicit_irqs.expect("expected IRQ list");
        assert!(irqs.contains(&72), "I2C3 IRQ 72 should fire, got {irqs:?}");
    }

    #[test]
    fn f1_layout_does_not_synth_bank2_irqs() {
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32F1);
        poke(&mut e, 0x20, 1 << 3); // bank 2 doesn't even exist on F1
        poke(&mut e, 0x30, 1 << 3);
        let r = e.tick();
        // No bank-2 -> no IRQ
        assert!(r.explicit_irqs.is_none() || r.explicit_irqs.unwrap().is_empty());
    }

    #[test]
    fn f1_bank1_swier_sets_pr_and_routes() {
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32F1);
        poke(&mut e, 0x00, 1 << 0); // IMR line 0
        poke(&mut e, 0x10, 1 << 0); // SWIER line 0 -> PR
        let r = e.tick();
        assert!(r.explicit_irqs.expect("irqs").contains(&6), "EXTI0 -> IRQ6");
    }

    #[test]
    fn all_variants_flip_to_scheduler_when_clock_attached() {
        for layout in [ExtiRegisterLayout::Stm32F1, ExtiRegisterLayout::Stm32L4] {
            let mut e = Exti::new_with_layout(layout);
            assert!(e.needs_legacy_walk() && !e.uses_scheduler());
            e.attach_cycle_clock(crate::CycleClock::default());
            #[cfg(feature = "event-scheduler")]
            {
                assert!(e.uses_scheduler() && !e.needs_legacy_walk());
                // Held level latched, but the walk tick is inert in scheduler mode.
                e.write_u32(0x00, 1).unwrap();
                e.write_u32(0x10, 1).unwrap();
                assert!(e.tick().explicit_irqs.is_none());
                e.force_legacy_walk();
                assert!(e.needs_legacy_walk() && !e.uses_scheduler());
            }
        }
    }

    #[test]
    fn f1_word_write_pr_clear_is_atomic_and_clears_swier() {
        // Whole-word access (as a 32-bit STR performs). SWIER=0x5 software-
        // triggers lines 0 and 2 -> PR=0x5; an rc_w1 word-write of 0x1 clears
        // ONLY line 0 (the default byte-decomposition would also wipe line 2 by
        // reading PR back and re-writing it). Clearing PR line 0 also clears
        // SWIER line 0. Silicon-verified on bench F103 (exti_swier oracle).
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32F1);
        e.write_u32(0x00, 0x5).unwrap(); // IMR lines 0,2
        e.write_u32(0x10, 0x5).unwrap(); // SWIER lines 0,2 -> PR
        assert_eq!(e.read_u32(0x14).unwrap(), 0x5, "PR set on lines 0,2");
        e.write_u32(0x14, 0x1).unwrap(); // rc_w1: clear line 0 only
        assert_eq!(e.read_u32(0x14).unwrap(), 0x4, "PR line 2 still pending");
        assert_eq!(
            e.read_u32(0x10).unwrap(),
            0x4,
            "SWIER line 0 cleared with PR"
        );
    }
}

// ── External GPIO edges through the F1 AFIO / F4 SYSCFG line-source mux ─────
#[cfg(test)]
mod external_mux {
    use super::{Exti, ExtiRegisterLayout};
    use crate::bus::SystemBus;
    use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
    use crate::{Bus, Peripheral};

    #[test]
    fn f1_layout_counts_only_the_selected_port_and_polarity() {
        let mut e = Exti::new_with_layout_lines(ExtiRegisterLayout::Stm32F1, 0x7FFFF);
        e.write_u32(0x00, 1 << 3).unwrap(); // IMR line 3
        e.write_u32(0x08, 1 << 3).unwrap(); // RTSR line 3 (rising only)
                                            // No mux on the bus, or another port selected: nothing.
        assert!(!e.gpio_edge_with_source(2, 3, false, true, None));
        assert!(!e.gpio_edge_with_source(2, 3, false, true, Some(0)));
        // Falling edge on a rising-only line: nothing.
        assert!(!e.gpio_edge_with_source(2, 3, true, false, Some(2)));
        assert_eq!(e.read_u32(0x14).unwrap(), 0);
        assert!(e.gpio_edge_with_source(2, 3, false, true, Some(2)));
        assert_eq!(e.read_u32(0x14).unwrap(), 1 << 3);
        assert_eq!(e.tick().explicit_irqs, Some(vec![9]), "EXTI3 -> IRQ 9");
        e.write_u32(0x14, 1 << 3).unwrap(); // rc_w1
        assert_eq!(e.tick().explicit_irqs, None);
    }

    #[test]
    fn pending_is_set_while_masked_and_fires_once_unmasked() {
        // IMR gates the interrupt, not PR (RM0008 §10.2.5).
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32F1);
        e.write_u32(0x0C, 1 << 12).unwrap(); // FTSR line 12
        assert!(e.gpio_edge_with_source(1, 12, true, false, Some(1)));
        assert_eq!(e.read_u32(0x14).unwrap(), 1 << 12);
        assert_eq!(e.tick().explicit_irqs, None);
        e.write_u32(0x00, 1 << 12).unwrap();
        assert_eq!(e.tick().explicit_irqs, Some(vec![40]), "EXTI15_10");
    }

    /// L0: the F1 register file, but the NVIC lines are the Cortex-M0+
    /// groups 5/6/7, both in the held level and in the levels the bus
    /// reconciles — never the F1 6..10/23/40.
    #[test]
    fn l0_routes_gpio_lines_to_the_m0_groups() {
        let mut exti = Exti::new_with_layout(ExtiRegisterLayout::Stm32L0);
        exti.write_u32(0x00, 0xFFFF).unwrap(); // IMR
        exti.write_u32(0x08, 0xFFFF).unwrap(); // RTSR
        let levels = |exti: &Exti| {
            let mut out = Vec::new();
            exti.irq_line_levels(&mut |irq, on| out.push((irq, on)));
            out
        };
        assert_eq!(levels(&exti), vec![(5, false), (6, false), (7, false)]);
        for (line, irq) in [
            (0u8, 5u32),
            (1, 5),
            (2, 6),
            (3, 6),
            (4, 7),
            (13, 7),
            (15, 7),
        ] {
            assert!(exti.gpio_edge_with_source(2, line, false, true, Some(2)));
            assert_eq!(exti.pending_irqs(), vec![irq], "line {line}");
            assert!(levels(&exti).contains(&(irq, true)));
            exti.write_u32(0x14, 1 << line).unwrap(); // PR rc_w1
            assert!(exti.pending_irqs().is_empty());
        }
        // The port select still comes from SYSCFG: another port is ignored.
        assert!(!exti.gpio_edge_with_source(1, 0, false, true, Some(2)));
        assert_eq!(
            "l0".parse::<ExtiRegisterLayout>(),
            Ok(ExtiRegisterLayout::Stm32L0)
        );
    }

    #[test]
    fn g0_ignores_an_external_line_source() {
        let mut e = Exti::new_with_layout(ExtiRegisterLayout::Stm32G0);
        e.write_u32(0x00, 1).unwrap(); // RTSR line 0, EXTICR1 = port A
        assert!(!e.gpio_edge_with_source(1, 0, false, true, Some(1)));
        assert!(e.gpio_edge_with_source(0, 0, false, true, Some(1)));
    }

    /// GPIOA/GPIOB, the line-source mux and EXTI on one bus, as a chip yaml
    /// wires them.
    fn bus_with(mux_name: &str, mux_base: u64, mux: Box<dyn Peripheral>, v2: bool) -> SystemBus {
        let layout = if v2 {
            GpioRegisterLayout::Stm32V2
        } else {
            GpioRegisterLayout::Stm32F1
        };
        let mut bus = SystemBus::empty();
        bus.add_peripheral(
            "gpioa",
            0x5000_0000,
            1024,
            None,
            Box::new(GpioPort::new_with_layout(layout)),
        );
        bus.add_peripheral(
            "gpiob",
            0x5000_0400,
            1024,
            None,
            Box::new(GpioPort::new_with_layout(layout)),
        );
        bus.add_peripheral(mux_name, mux_base, 1024, None, mux);
        bus.add_peripheral(
            "exti",
            0x4001_0400,
            1024,
            None,
            Box::new(Exti::new_with_layout(ExtiRegisterLayout::Stm32F1)),
        );
        bus
    }

    /// Both edges on line 1, port B selected through the mux at `exticr1`;
    /// count PR sets from external input changes on PA1 and PB1.
    fn edges_reach_exti_through(mut bus: SystemBus, exticr1: u64) {
        const EXTI: u64 = 0x4001_0400;
        bus.write_u32(exticr1, 1 << 4).unwrap(); // line 1 -> port B
        bus.write_u32(EXTI + 0x08, 1 << 1).unwrap(); // RTSR
        bus.write_u32(EXTI + 0x0C, 1 << 1).unwrap(); // FTSR
        bus.write_u32(EXTI, 1 << 1).unwrap(); // IMR
        let pr = |bus: &SystemBus| bus.read_u32(EXTI + 0x14).unwrap();
        // Port A is not selected.
        bus.set_peripheral_gpio_input(0, 1, true);
        bus.set_peripheral_gpio_input(0, 1, false);
        assert_eq!(pr(&bus), 0);
        // Port B rising, then falling: each sets PR and raises EXTI1 (IRQ 7).
        bus.set_peripheral_gpio_input(1, 1, true);
        assert_eq!(pr(&bus), 1 << 1);
        assert_eq!(bus.tick_peripherals_fully_forced().0, vec![7]);
        bus.write_u32(EXTI + 0x14, 1 << 1).unwrap();
        // The same level again is no edge.
        bus.set_peripheral_gpio_input(1, 1, true);
        assert_eq!(pr(&bus), 0);
        bus.set_peripheral_gpio_input(1, 1, false);
        assert_eq!(pr(&bus), 1 << 1);
    }

    #[test]
    fn f1_afio_exticr_routes_external_edges_to_exti() {
        let bus = bus_with(
            "afio",
            0x4001_0000,
            Box::new(crate::peripherals::afio::Afio::new()),
            false,
        );
        edges_reach_exti_through(bus, 0x4001_0008);
    }

    #[test]
    fn f4_syscfg_exticr_routes_external_edges_to_exti() {
        let bus = bus_with(
            "syscfg",
            0x4001_3800,
            Box::new(crate::peripherals::syscfg::Stm32F4Syscfg::new()),
            true,
        );
        edges_reach_exti_through(bus, 0x4001_3808);
    }

    #[test]
    fn without_a_mux_on_the_bus_the_f1_exti_stays_quiet() {
        let bus = bus_with(
            "stub",
            0x4001_3800,
            Box::new(crate::peripherals::stub::StubPeripheral::new(0)),
            true,
        );
        let mut bus = bus;
        bus.write_u32(0x4001_0408, 1).unwrap(); // RTSR line 0
        bus.set_peripheral_gpio_input(0, 0, true);
        assert_eq!(bus.read_u32(0x4001_0414).unwrap(), 0);
    }
}

// ── Walk-free differential: held-level EXTI walk vs scheduler ────────────────
#[cfg(all(test, feature = "event-scheduler"))]
mod scheduler_diff {
    use super::*;
    use crate::Peripheral;

    #[derive(Clone, Copy)]
    enum Op {
        /// 32-bit register write at cycle (as firmware STRs it).
        Write(u64, u32),
    }

    fn build(layout: ExtiRegisterLayout, scheduler: bool) -> Exti {
        let mut e = Exti::new_with_layout(layout);
        if scheduler {
            e.attach_cycle_clock(CycleClock::default());
        }
        e
    }

    /// Drive the SAME op script against (a) the per-cycle walk and (b) the event
    /// path; assert the emitted IRQ set AND the register snapshot are identical
    /// every cycle. An `Op` scheduled at cycle `c` is applied before that cycle's
    /// tick. This is the held-level analogue of the I2C `kinetis_scheduler` gate.
    fn assert_walk_identical(layout: ExtiRegisterLayout, script: &[(u64, Op)], cycles: u64) {
        let mut walk = build(layout, false);
        let mut sched = build(layout, true);
        let clock = match &sched {
            Exti::Stm32F1(e) | Exti::Stm32L0(e) => e.clock.clone(),
            Exti::Stm32L4(e) => e.clock.clone(),
            Exti::Stm32G0(e) | Exti::Stm32U5(e) => e.clock.clone(),
        }
        .unwrap();

        // (deadline_cycle, token) event queue, driven exactly like Machine +
        // SystemBus at tick interval 1.
        let mut events: Vec<(u64, u32)> = Vec::new();
        let bus = &mut crate::bus::SystemBus::new();

        for c in 1..=cycles {
            for (sc, Op::Write(off, val)) in script.iter().copied() {
                if sc == c {
                    walk.write_u32(off, val).unwrap();
                    // Scheduler: write, then harvest events at cycle+1+delay
                    // (`now == c - 1` at the point of the write).
                    sched.write_u32(off, val).unwrap();
                    for (delay, token) in sched.take_scheduled_events() {
                        events.push((c - 1 + 1 + delay, token));
                    }
                }
            }

            // Walk: tick emits the held level this cycle.
            let walk_irqs = walk.tick().explicit_irqs.unwrap_or_default();

            // Scheduler: publish the clock, drain due events through on_event.
            clock.publish(c);
            let due: Vec<(u64, u32)> = events.iter().copied().filter(|(d, _)| *d <= c).collect();
            events.retain(|(d, _)| *d > c);
            let mut esched = crate::sched::EventScheduler::new();
            esched.advance_to(c);
            let mut sched_irqs = Vec::new();
            for (_, token) in due {
                let res = sched.on_event(token, &mut esched, bus);
                sched_irqs.extend(res.explicit_irqs);
                if let Some(delay) = res.reschedule_delay {
                    events.push((c + delay, token));
                }
            }

            assert_eq!(
                walk_irqs, sched_irqs,
                "emitted IRQ set diverged at cycle {c}"
            );
            assert_eq!(
                walk.snapshot(),
                sched.snapshot(),
                "register snapshot diverged at cycle {c}"
            );
        }
    }

    #[test]
    fn f1_swier_hold_and_clear_walk_identity() {
        // IMR lines 0,2; SWIER trigger → PR held (IRQs re-emit every cycle);
        // firmware clears PR line 0 (rc_w1) mid-hold; then clears line 2.
        let script = [
            (1u64, Op::Write(0x00, 0x5)), // IMR lines 0,2
            (1, Op::Write(0x10, 0x5)),    // SWIER lines 0,2 → PR (held level)
            (5, Op::Write(0x14, 0x1)),    // clear PR line 0 → EXTI0 stops
            (9, Op::Write(0x14, 0x4)),    // clear PR line 2 → level drops
        ];
        assert_walk_identical(ExtiRegisterLayout::Stm32F1, &script, 14);
    }

    #[test]
    fn f1_grouped_irq_line_walk_identity() {
        // Lines in the EXTI9_5 group (shared IRQ 23) plus EXTI15_10 (IRQ 40).
        let script = [
            (1u64, Op::Write(0x00, (1 << 7) | (1 << 12))), // IMR lines 7,12
            (1, Op::Write(0x10, (1 << 7) | (1 << 12))),    // SWIER → PR
            (6, Op::Write(0x14, 1 << 7)),                  // clear line 7 (IRQ23 drops)
            (9, Op::Write(0x14, 1 << 12)),                 // clear line 12 (IRQ40 drops)
        ];
        assert_walk_identical(ExtiRegisterLayout::Stm32F1, &script, 14);
    }

    #[test]
    fn l0_grouped_irq_line_walk_identity() {
        // Lines 1 (EXTI0_1, IRQ 5) and 9 (EXTI4_15, IRQ 7).
        let script = [
            (1u64, Op::Write(0x00, (1 << 1) | (1 << 9))),
            (1, Op::Write(0x10, (1 << 1) | (1 << 9))),
            (6, Op::Write(0x14, 1 << 1)),
            (9, Op::Write(0x14, 1 << 9)),
        ];
        assert_walk_identical(ExtiRegisterLayout::Stm32L0, &script, 14);
    }

    #[test]
    fn l4_bank2_wakeup_hold_walk_identity() {
        // Bank-2 line 36 (I2C1 wakeup → IRQ 31) plus a bank-1 line, held and
        // cleared independently across banks.
        let script = [
            (1u64, Op::Write(0x00, 1 << 1)), // IMR1 line 1
            (1, Op::Write(0x10, 1 << 1)),    // SWIER1 → PR1
            (1, Op::Write(0x20, 1 << 4)),    // IMR2 line 36
            (1, Op::Write(0x30, 1 << 4)),    // SWIER2 → PR2 (I2C1 wakeup)
            (6, Op::Write(0x14, 1 << 1)),    // clear bank-1
            (10, Op::Write(0x34, 1 << 4)),   // clear bank-2
        ];
        assert_walk_identical(ExtiRegisterLayout::Stm32L4, &script, 15);
    }

    #[test]
    fn imr_unmask_after_pending_arms_walk_identity() {
        // PR set while masked (no IRQ), THEN IMR unmasks it — the write that
        // raises the level must arm the chain. Validates the arming predicate.
        let script = [
            (1u64, Op::Write(0x10, 0x2)), // SWIER line 1 → PR (but IMR=0 → masked)
            (5, Op::Write(0x00, 0x2)),    // IMR line 1 → level rises here
            (9, Op::Write(0x14, 0x2)),    // clear
        ];
        assert_walk_identical(ExtiRegisterLayout::Stm32F1, &script, 13);
    }
}
