// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 single-cycle IO block — SIO GPIO (datasheet §2.3.1, base
//! `0xD0000000`).
//!
//! SIO sits on the Cortex-M0+ single-cycle IO port (address `0xD0000000`),
//! *outside* the `0x40000000..0x50400000` APB/AHB peripheral window, so the
//! RP2040 atomic SET/CLR/XOR register aliases (`+0x2000` / `+0x3000` /
//! `+0x1000`) do **not** apply here. Instead SIO exposes dedicated
//! set / clear / xor registers at fixed offsets (`GPIO_OUT_SET` etc.), which
//! this model implements directly.
//!
//! Modelled behaviour: a 30-bit `GPIO_OUT` output latch and a `GPIO_OE` output
//! enable (48 bits on the RP2350 QFN80 package, [`SioLayout::Rp2350B`], whose
//! GPIO32..47 live in the `GPIO_HI_*` words), each driven by direct / set / clear / xor registers, plus an
//! externally driven input word (`ext_in`) the outside world holds through
//! [`Peripheral::set_gpio_input`]. `GPIO_IN` reports the pad: a pin whose
//! output driver is enabled reads back the level it is *driving*, and only a
//! pin left as an input reports what the outside world put there. An input pin
//! nothing drives still floats to 0. `CPUID` reads 0 (core 0).

use crate::pins::{External, InputChange, PinPort};
use crate::{MmioAccessClass, Peripheral, SimResult};
use std::cell::Cell;

// SIO register offsets (datasheet §2.3.1.7).
const CPUID: u64 = 0x000;
const GPIO_IN: u64 = 0x004;
const GPIO_HI_IN: u64 = 0x008;
const GPIO_OUT: u64 = 0x010;
const GPIO_OUT_SET: u64 = 0x014;
const GPIO_OUT_CLR: u64 = 0x018;
const GPIO_OUT_XOR: u64 = 0x01c;
const GPIO_OE: u64 = 0x020;
const GPIO_OE_SET: u64 = 0x024;
const GPIO_OE_CLR: u64 = 0x028;
const GPIO_OE_XOR: u64 = 0x02c;

// Hardware integer divider (datasheet §2.3.1.6). RP2040's Cortex-M0+ has no
// DIV instruction; the SDK's `__aeabi_uidiv`/`__aeabi_idiv` wrappers (and
// anything built on `hardware_divider`) route through this SIO-mapped
// divider instead of a software division routine. Leaving it unmodelled
// means every division silently returns 0/0, which is exactly the kind of
// wrong-answer-not-a-halt bug this simulator exists to avoid — and it's fatal
// at boot: arduino-pico's `set_sys_clock_khz` divides VCO frequencies while
// searching for PLL dividers, and a divider that always reads 0 makes every
// candidate frequency look unreachable, so it panics ("cannot be exactly
// achieved") before `LWCONF` ever prints. We compute results synchronously
// (no multi-cycle latency), so READY is always 1 in this model.
const DIV_UDIVIDEND: u64 = 0x060;
const DIV_UDIVISOR: u64 = 0x064;
const DIV_SDIVIDEND: u64 = 0x068;
const DIV_SDIVISOR: u64 = 0x06c;
const DIV_QUOTIENT: u64 = 0x070;
const DIV_REMAINDER: u64 = 0x074;
const DIV_CSR: u64 = 0x078;

// Hardware spinlocks: 32 registers, SPINLOCK0..SPINLOCK31 (datasheet §2.3.1.5).
const SPINLOCK0: u64 = 0x100;
const SPINLOCK31: u64 = 0x17c;

// The RP2040 exposes 30 GPIOs (0..29) on bank 0.
const GPIO_MASK: u32 = 0x3fff_ffff;

/// `GPIO_HI_*` bits that are bank-0 pads on the RP2350 QFN80 package
/// (pico-sdk rp2350 `sio.h`: `SIO_GPIO_HI_OUT_GPIO_BITS` = 0x0000ffff,
/// GPIO32..47). The QSPI and USB bits above them are not modelled: they read
/// 0 and writes to them are dropped.
const GPIO_HI_PAD_MASK: u32 = 0x0000_ffff;

/// The lo-bank words (`GPIO_OUT`, `GPIO_OE`, ...) as a 64-bit pad mask.
const LO_WORD: u64 = 0xffff_ffff;

/// Register map variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SioLayout {
    /// RP2040 SIO: OUT 0x10, OUT_SET 0x14, ..., OE 0x20 (lo bank only).
    #[default]
    Rp2040,
    /// RP2350 SIO (pico-sdk rp2350 hardware_regs/sio.h): every GPIO register
    /// has a HI twin at +4, so OUT 0x10 / HI_OUT 0x14, OUT_SET 0x18,
    /// OUT_CLR 0x20, OUT_XOR 0x28, OE 0x30, OE_SET 0x38, OE_CLR 0x40,
    /// OE_XOR 0x48. Only the lo bank (GPIO 0..29, the RP2350A pads) is
    /// modelled: HI registers read 0 and ignore writes, as `GPIO_HI_IN`
    /// already does. The RP2040 divider window is left as it is.
    Rp2350,
    /// RP2350 QFN80 package (RP2350B, RP2354B): the [`SioLayout::Rp2350`] map
    /// with 48 bank-0 pads. GPIO0..31 are the lo words, GPIO32..47 bits
    /// [15:0] of the HI twins (`GPIO_HI_IN` 0x08, `GPIO_HI_OUT` 0x14,
    /// `GPIO_HI_OUT_SET` 0x1c, ..., `GPIO_HI_OE_XOR` 0x4c; pico-sdk rp2350
    /// `sio.h`).
    Rp2350B,
}
/// Pads this model answers for, named so the register mask and the per-pin
/// capability methods (`read_gpio_pad`, `read_gpio_input`, `set_gpio_input`)
/// cannot drift apart — a pin the mask drops must also be a pin the external
/// world cannot drive, or a `board_io` button would report a level `GPIO_IN`
/// never shows.
const PAD_COUNT: u8 = 30;

#[derive(Debug, Default)]
pub struct Rp2040Sio {
    /// Register map variant (RP2040 or RP2350 GPIO offsets).
    layout: SioLayout,
    /// Bit `n` = GPIOn (bits 32..47 only on [`SioLayout::Rp2350B`]).
    gpio_out: u64,
    gpio_oe: u64,
    /// Level the OUTSIDE WORLD holds on each pad — a `board_io` button wired to
    /// the pin, a sensor's output line — applied through
    /// [`Peripheral::set_gpio_input`]. Separate from `gpio_out` because it must
    /// survive the firmware driving the same pad in the other direction: a
    /// button holds its released level from boot, and the pin must return to it
    /// the moment the firmware releases the output driver.
    ext_in: u64,
    /// Pads the outside world holds ([`PinPort::set_external`]); the level on
    /// each is its `ext_in` bit.
    ext_mask: u64,
    /// Pads bound to peripheral wires, resolved against IO_BANK0's live
    /// FUNCSEL. Empty until `SystemBus::wire_rp2040_pads` binds them.
    pad_routes: crate::peripherals::pad_routing::PadRoutes,
    /// Live pad-function state shared from IO_BANK0, so a pad re-assigned at
    /// runtime changes hands immediately.
    pad_functions: Option<std::sync::Arc<super::io_bank0::PadFunctions>>,
    /// Bit `n` set == spinlock `n` is currently claimed. `Cell` because a
    /// spinlock read is a claim (a write side-effect) on the `&self` read path.
    spinlocks_held: Cell<u32>,
    /// Logic-analyzer push watch ([`PinPort::install_watch`]; not snapshot
    /// state).
    watch: Option<crate::pins::PadWatch>,
    /// IO_BANK0's GPIO interrupt state: every change of `GPIO_IN` is
    /// reported there, so edges latch whatever moved the pad.
    bank_irq: Option<std::sync::Arc<super::io_bank0::BankIrq>>,
    /// Bus index of IO_BANK0, whose interrupt chain a pad change must arm.
    io_bank0_idx: Option<usize>,
    /// Raw divider operand latches, shared between the U*/S* register views
    /// (real silicon feeds both into the same divider core).
    div_dividend: u32,
    div_divisor: u32,
    div_quotient: u32,
    div_remainder: u32,
    /// Set by the last write that kicked off a calculation; selects
    /// signed vs. unsigned interpretation of the stored operands.
    div_signed: bool,
    /// DIRTY: set on any operand write, cleared when QUOTIENT is read.
    div_dirty: bool,
    /// Outside levels that reached a pad routed to a peripheral line since
    /// the bus last collected them (see `sync_wire_input`).
    wire_edges: Vec<crate::peripherals::pad_lines::WireInputEdge>,
}

impl Rp2040Sio {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_layout(mut self, layout: SioLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Bank-0 pads this layout answers for.
    pub fn pad_count(&self) -> u8 {
        match self.layout {
            SioLayout::Rp2040 | SioLayout::Rp2350 => PAD_COUNT,
            SioLayout::Rp2350B => super::io_bank0::PAD_COUNT_QFN80,
        }
    }

    /// Writable pad bits of the lo-bank words.
    fn lo_mask(&self) -> u32 {
        match self.layout {
            SioLayout::Rp2040 | SioLayout::Rp2350 => GPIO_MASK,
            SioLayout::Rp2350B => u32::MAX,
        }
    }

    /// On [`SioLayout::Rp2350B`], the lo-bank register an RP2350 `GPIO_HI_*`
    /// offset twins (its bits [15:0] are GPIO32..47). `None` on every other
    /// layout and offset.
    fn hi_register(&self, offset: u64) -> Option<u64> {
        if self.layout != SioLayout::Rp2350B {
            return None;
        }
        Some(match offset {
            0x08 => GPIO_IN,
            0x14 => GPIO_OUT,
            0x1c => GPIO_OUT_SET,
            0x24 => GPIO_OUT_CLR,
            0x2c => GPIO_OUT_XOR,
            0x34 => GPIO_OE,
            0x3c => GPIO_OE_SET,
            0x44 => GPIO_OE_CLR,
            0x4c => GPIO_OE_XOR,
            _ => return None,
        })
    }

    /// Map an offset onto the RP2040 constants below. `None` = an RP2350
    /// HI-bank GPIO register (reads 0, writes ignored).
    fn canonical(&self, offset: u64) -> Option<u64> {
        if self.layout == SioLayout::Rp2040 {
            return Some(offset);
        }
        Some(match offset {
            0x10 => GPIO_OUT,
            0x18 => GPIO_OUT_SET,
            0x20 => GPIO_OUT_CLR,
            0x28 => GPIO_OUT_XOR,
            0x30 => GPIO_OE,
            0x38 => GPIO_OE_SET,
            0x40 => GPIO_OE_CLR,
            0x48 => GPIO_OE_XOR,
            0x14..=0x4c if offset & 0x4 != 0 => return None,
            _ => offset,
        })
    }

    /// Level at the pads — what `GPIO_IN` reports.
    ///
    /// A pin whose output driver is enabled reads back the level it is
    /// DRIVING; only a pin left as an input reports what the outside world put
    /// there, and a pad nothing drives floats to 0.
    ///
    /// If a pin is both driven and externally forced, the output driver wins.
    /// Real silicon has a contention whose winner depends on drive strength; we
    /// do not model that, and taking the driver is the case that matches a
    /// correctly wired board — the same rule the ESP32 model states at
    /// `pad_level_bank0`, so both families answer a button identically.
    fn gpio_in(&self) -> u64 {
        (self.gpio_out & self.gpio_oe) | (self.ext_in & !self.gpio_oe)
    }

    /// The function IO_BANK0 currently selects for `pin` — the selector the
    /// shared routing seam resolves pad bindings against. `None` when IO_BANK0
    /// is not on this bus (so nothing is ever routed) or the pad is NULL.
    fn pad_function(&self, pin: u8) -> Option<u32> {
        self.pad_functions.as_ref()?.function(pin)
    }

    /// Share IO_BANK0's pad functions and GPIO interrupt state, and name its
    /// bus index (whose interrupt chain a pad change arms). Called at bus
    /// wiring time (`SystemBus::wire_rp2040_uart_pads`).
    pub(crate) fn attach_io_bank0(
        &mut self,
        functions: std::sync::Arc<super::io_bank0::PadFunctions>,
        irq: std::sync::Arc<super::io_bank0::BankIrq>,
        io_bank0_idx: usize,
    ) {
        self.pad_functions = Some(functions);
        irq.seed_levels(self.gpio_in());
        self.bank_irq = Some(irq);
        self.io_bank0_idx = Some(io_bank0_idx);
    }

    /// Tell IO_BANK0 the pad levels after anything that may have moved them.
    #[inline]
    fn report_levels(&self) {
        if let Some(irq) = &self.bank_irq {
            irq.set_levels(self.gpio_in());
        }
    }

    /// The pad's own output stage ([`PinPort::driver`]): what the SIO output
    /// stage does (`OE`, `GPIO_OUT`) while IO_BANK0 selects SIO for the pad
    /// (or selects nothing yet: this model has always let the SIO latch drive
    /// an unassigned pad, and the level firmware reads back on `GPIO_IN` says
    /// the same). A pad handed to a peripheral that publishes its wire is
    /// driven by it; one handed to a peripheral that publishes nothing (PWM,
    /// PIO, ...) has no known drive (`None`), so a world refuses it. The SIO
    /// does not model the PADS_BANK0 pulls, so it reports none.
    fn pad_driver(&self, pin: u8) -> Option<crate::pins::PadDriver> {
        use crate::pins::PadDriver;
        if pin >= self.pad_count() {
            return None;
        }
        match self.pad_function(pin) {
            None | Some(super::io_bank0::GPIO_FUNC_SIO) => {
                Some(if self.gpio_oe & (1u64 << pin) != 0 {
                    PadDriver::drive(self.gpio_out & (1u64 << pin) != 0)
                } else {
                    PadDriver::OFF
                })
            }
            // The peripheral says what its output stage does: a line it
            // drives drives the pad, a released line or an input does not.
            Some(_) => self
                .pad_routes
                .active_line(pin, |p| self.pad_function(p))
                .map(|(cell, line)| {
                    if cell.drives(line) {
                        PadDriver::drive(cell.level(line))
                    } else {
                        PadDriver::OFF
                    }
                }),
        }
    }

    /// Hand the level the outside holds on `pin` to the peripheral line the
    /// pad is routed to, if any, and remember the edge for the bus to
    /// deliver (`Peripheral::take_wire_input_edges`).
    fn sync_wire_input(&mut self, pin: u8) {
        if self.pad_routes.is_empty()
            || pin >= self.pad_count()
            || self.ext_mask & (1u64 << pin) == 0
        {
            return;
        }
        let level = self.ext_in & (1u64 << pin) != 0;
        let functions = self.pad_functions.clone();
        let edge = self
            .pad_routes
            .active_line(pin, |p| functions.as_ref().and_then(|f| f.function(p)))
            .and_then(|(cell, line)| {
                cell.set_input(line, level)
                    .then(|| crate::peripherals::pad_lines::WireInputEdge {
                        cell: cell.clone(),
                        line,
                        level,
                    })
            });
        if let Some(edge) = edge {
            self.wire_edges.push(edge);
        }
    }

    /// After a FUNCSEL change: a pad that just became routed to a peripheral
    /// line hands it the level the outside already holds there.
    fn sync_wire_inputs(&mut self) {
        if self.pad_routes.is_empty() || self.ext_mask == 0 {
            return;
        }
        let mut pins = self.ext_mask;
        while pins != 0 {
            let pin = pins.trailing_zeros() as u8;
            pins &= pins - 1;
            self.sync_wire_input(pin);
        }
    }

    /// Run a pad mutation bracketed for push capture, then tell IO_BANK0 the
    /// pad levels.
    #[inline]
    fn mutate_pads<R>(&mut self, mutate: impl FnOnce(&mut Self) -> R) -> R {
        crate::pins::watch_begin(self);
        let r = mutate(self);
        self.report_levels();
        crate::pins::watch_end(self);
        r
    }

    /// Share IO_BANK0's live pad-function state and bind a peripheral wire to
    /// the pads that can carry it. Called at bus wiring time.
    pub(crate) fn bind_pad_route(
        &mut self,
        functions: std::sync::Arc<super::io_bank0::PadFunctions>,
        cell: &std::sync::Arc<crate::peripherals::pad_lines::PadLines>,
        pin: u8,
        function: u32,
        line: usize,
        func_name: &'static str,
    ) {
        self.pad_functions = Some(functions);
        self.pad_routes
            .bind(cell, pin, Some(function), line, func_name);
    }

    /// Every signal name bound to this port's pads, live or not — the
    /// bus-visibility reporting seam. See
    /// [`crate::peripherals::pad_routing::PadRoutes::bound_functions`] for why
    /// this is the static question and `func()` is the live one.
    pub(crate) fn bound_pad_functions(&self) -> Vec<&'static str> {
        self.pad_routes.bound_functions()
    }

    fn pad_level(&self, pin: u8) -> Option<bool> {
        if pin >= self.pad_count() {
            return None;
        }
        // A pad IO_BANK0 has handed to a peripheral is driven by that
        // peripheral's wire, not by the SIO output latch. Resolving it through
        // the shared routing seam is what makes an RP2040 bus measurable.
        if let Some((cell, line)) = self.pad_routes.active_line(pin, |p| self.pad_function(p)) {
            // A line the peripheral does not drive (an input) reads what is
            // outside the pad.
            if !cell.drives(line) {
                return Some(self.gpio_in() & (1u64 << pin) != 0);
            }
            return Some(cell.level(line));
        }
        // ONE definition of "pad level": whatever firmware reads on GPIO_IN is
        // what a probe clipped to the pad sees, so a host-driven input and a
        // firmware-driven output can never disagree between the two readers.
        Some(self.gpio_in() & (1u64 << pin) != 0)
    }

    /// Apply a GPIO_OUT/GPIO_OE register write. `bits` are the written pads
    /// as a 64-bit pad mask, `word` the pads that register word covers (a
    /// direct `GPIO_OUT`/`GPIO_OE` write replaces only its own word).
    fn write_gpio(&mut self, reg: u64, word: u64, bits: u64) {
        self.mutate_pads(|s| match reg {
            GPIO_OUT => s.gpio_out = (s.gpio_out & !word) | bits,
            GPIO_OUT_SET => s.gpio_out |= bits,
            GPIO_OUT_CLR => s.gpio_out &= !bits,
            GPIO_OUT_XOR => s.gpio_out ^= bits,
            GPIO_OE => s.gpio_oe = (s.gpio_oe & !word) | bits,
            GPIO_OE_SET => s.gpio_oe |= bits,
            GPIO_OE_CLR => s.gpio_oe &= !bits,
            _ => s.gpio_oe ^= bits, // GPIO_OE_XOR
        });
    }

    /// The register word `reg` reads, for the lo (`hi == false`) or HI bank.
    fn gpio_word(&self, reg: u64, hi: bool) -> u32 {
        let bits = match reg {
            GPIO_IN => self.gpio_in(),
            GPIO_OUT | GPIO_OUT_SET | GPIO_OUT_CLR | GPIO_OUT_XOR => self.gpio_out,
            _ => self.gpio_oe,
        };
        if hi {
            (bits >> 32) as u32 & GPIO_HI_PAD_MASK
        } else {
            bits as u32
        }
    }

    /// Re-register watched pads with the wires that drive them, so a pad that
    /// changes hands follows its new source.
    fn sync_pad_routes(&mut self) {
        if self.pad_routes.is_empty() {
            return;
        }
        let Some(watch) = self.watch.take() else {
            return;
        };
        let functions = self.pad_functions.clone();
        self.pad_routes
            .sync_taps(watch.tap(), watch.pairs(), |pin| {
                functions.as_ref().and_then(|f| f.function(pin))
            });
        self.watch = Some(watch);
    }

    /// Recompute `DIV_QUOTIENT`/`DIV_REMAINDER` from the latched operands,
    /// interpreting them as signed or unsigned per `div_signed`. Mirrors the
    /// RP2040 divider's documented divide-by-zero behaviour (datasheet
    /// §2.3.1.6): unsigned divide-by-zero yields quotient `0xffffffff` and
    /// remainder = dividend; signed divide-by-zero yields quotient `±1`
    /// (sign of the dividend) and remainder = dividend.
    fn recompute_divider(&mut self) {
        if self.div_signed {
            let dividend = self.div_dividend as i32;
            let divisor = self.div_divisor as i32;
            if divisor == 0 {
                self.div_quotient = if dividend < 0 {
                    1i32 as u32
                } else {
                    (-1i32) as u32
                };
                self.div_remainder = dividend as u32;
            } else if dividend == i32::MIN && divisor == -1 {
                // Overflow case: matches the hardware divider's saturation.
                self.div_quotient = i32::MIN as u32;
                self.div_remainder = 0;
            } else {
                self.div_quotient = (dividend / divisor) as u32;
                self.div_remainder = (dividend % divisor) as u32;
            }
        } else {
            let dividend = self.div_dividend;
            let divisor = self.div_divisor;
            // Divide-by-zero is DEFINED on this hardware (datasheet 2.3.1.7):
            // quotient reads all-ones and the remainder is the dividend. Written
            // with checked_div so the zero case is expressed once, in the type,
            // rather than as a separate guard clippy flags as manual_checked_ops.
            match (dividend.checked_div(divisor), dividend.checked_rem(divisor)) {
                (Some(q), Some(r)) => {
                    self.div_quotient = q;
                    self.div_remainder = r;
                }
                _ => {
                    self.div_quotient = 0xffff_ffff;
                    self.div_remainder = dividend;
                }
            }
        }
        self.div_dirty = true;
    }

    /// True if `offset` names a SPINLOCKn register.
    fn is_spinlock(offset: u64) -> bool {
        (SPINLOCK0..=SPINLOCK31).contains(&offset) && offset & 0x3 == 0
    }

    /// Read (claim) SPINLOCKn (datasheet §2.3.1.5): if the lock is free, claim
    /// it atomically and return a nonzero value (bit `n`); if already held,
    /// return 0. This is the genuine try-lock semantics the pico-sdk
    /// `hw_claim_lock` / `spin_lock_blocking` loops rely on to make progress.
    //
    // FIDELITY: modeled, NOT HW-validated (2026-07-04) — SIO SPINLOCK0..31
    // try-lock/release per RP2040 datasheet §2.3.1.5. Single-core model: a free
    // lock is granted on read and released on write.
    fn claim_spinlock(&self, offset: u64) -> u32 {
        let n = (offset - SPINLOCK0) / 4;
        let bit = 1u32 << n;
        let held = self.spinlocks_held.get();
        if held & bit == 0 {
            self.spinlocks_held.set(held | bit);
            bit
        } else {
            0
        }
    }

    /// Write to SPINLOCKn releases the lock (any value; datasheet §2.3.1.5).
    fn release_spinlock(&mut self, offset: u64) {
        let n = (offset - SPINLOCK0) / 4;
        let bit = 1u32 << n;
        self.spinlocks_held.set(self.spinlocks_held.get() & !bit);
    }
}

impl Peripheral for Rp2040Sio {
    /// GPIO latch + spinlocks are pure MMIO — `tick()` is the default no-op.
    /// Dropping SIO from the walk is byte-identical (logic taps fire on write).
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    /// The hardware divider is a coprocessor: writing the operands and
    /// reading the quotient has no pin, IRQ, or timer side effect. Arduino
    /// `millis()` uses it (`__aeabi_uidiv` of `time_us_64`) on every empty
    /// `loop()` iteration, so classifying it as side-effecting would block
    /// idle-ff from coalescing the TIMERAW poll that sits next to it.
    fn mmio_access_class(&self, offset: u64) -> MmioAccessClass {
        match self.canonical(offset) {
            Some(
                DIV_UDIVIDEND | DIV_UDIVISOR | DIV_SDIVIDEND | DIV_SDIVISOR | DIV_QUOTIENT
                | DIV_REMAINDER | DIV_CSR,
            ) => MmioAccessClass::SideEffectFree,
            _ => MmioAccessClass::SideEffecting,
        }
    }

    fn take_wire_input_edges(&mut self) -> Vec<crate::peripherals::pad_lines::WireInputEdge> {
        std::mem::take(&mut self.wire_edges)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if Self::is_spinlock(offset) {
            return Ok(self.claim_spinlock(offset));
        }
        if let Some(reg) = self.hi_register(offset) {
            return Ok(self.gpio_word(reg, true));
        }
        let Some(offset) = self.canonical(offset) else {
            return Ok(0);
        };
        let val = match offset {
            CPUID => 0,      // single core context: always core 0
            GPIO_HI_IN => 0, // QSPI bank pins — not modelled
            GPIO_IN | GPIO_OUT | GPIO_OUT_SET | GPIO_OUT_CLR | GPIO_OUT_XOR | GPIO_OE
            | GPIO_OE_SET | GPIO_OE_CLR | GPIO_OE_XOR => self.gpio_word(offset, false),
            DIV_UDIVIDEND | DIV_SDIVIDEND => self.div_dividend,
            DIV_UDIVISOR | DIV_SDIVISOR => self.div_divisor,
            // Real hardware clears DIRTY when QUOTIENT is read; we leave it
            // latched once set. `hardware_divider`'s save/restore helpers use
            // DIRTY only to decide whether a nested division needs to save
            // and restore the divider state around a reentrant call — an
            // always-1 DIRTY just means that save/restore path is always
            // taken, which is still numerically correct, only slightly more
            // conservative than silicon.
            DIV_QUOTIENT => self.div_quotient,
            DIV_REMAINDER => self.div_remainder,
            DIV_CSR => {
                let ready = 1u32; // synchronous model: always settled.
                let dirty = if self.div_dirty { 1u32 << 1 } else { 0 };
                ready | dirty
            }
            _ => {
                crate::census_reg!("rp2040.sio:Rp2040Sio", offset, "read");
                0
            }
        };
        Ok(val)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if Self::is_spinlock(offset) {
            self.release_spinlock(offset);
            return Ok(());
        }
        if let Some(reg) = self.hi_register(offset) {
            // GPIO_HI_IN is read-only.
            if reg != GPIO_IN {
                let bits = u64::from(value & GPIO_HI_PAD_MASK) << 32;
                self.write_gpio(reg, u64::from(GPIO_HI_PAD_MASK) << 32, bits);
            }
            return Ok(());
        }
        let Some(offset) = self.canonical(offset) else {
            return Ok(());
        };
        match offset {
            DIV_UDIVIDEND => {
                self.div_dividend = value;
                self.div_signed = false;
                self.recompute_divider();
                return Ok(());
            }
            DIV_UDIVISOR => {
                self.div_divisor = value;
                self.div_signed = false;
                self.recompute_divider();
                return Ok(());
            }
            DIV_SDIVIDEND => {
                self.div_dividend = value;
                self.div_signed = true;
                self.recompute_divider();
                return Ok(());
            }
            DIV_SDIVISOR => {
                self.div_divisor = value;
                self.div_signed = true;
                self.recompute_divider();
                return Ok(());
            }
            DIV_QUOTIENT => {
                self.div_quotient = value;
                self.div_dirty = true;
                return Ok(());
            }
            DIV_REMAINDER => {
                self.div_remainder = value;
                self.div_dirty = true;
                return Ok(());
            }
            _ => {
                crate::census_reg!("rp2040.sio:Rp2040Sio", offset, "write");
            }
        }
        let v = u64::from(value & self.lo_mask());
        let mut_out = matches!(
            offset,
            GPIO_OUT
                | GPIO_OUT_SET
                | GPIO_OUT_CLR
                | GPIO_OUT_XOR
                | GPIO_OE
                | GPIO_OE_SET
                | GPIO_OE_CLR
                | GPIO_OE_XOR
        );
        if !mut_out {
            crate::census_reg!("rp2040.sio:Rp2040Sio", offset, "write");
            return Ok(());
        }
        self.write_gpio(offset, LO_WORD, v);
        Ok(())
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !0x3)?;
        Ok((word >> ((offset & 0x3) * 8)) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !0x3;
        // Don't route a spinlock RMW through the claiming read path.
        if Self::is_spinlock(aligned) {
            return self.write_u32(aligned, value as u32);
        }
        let shift = (offset & 0x3) * 8;
        let cur = if let Some(reg) = self.hi_register(aligned) {
            self.gpio_word(reg, true)
        } else {
            match self.canonical(aligned) {
                None => 0,
                Some(
                    reg @ (GPIO_OUT | GPIO_OUT_SET | GPIO_OUT_CLR | GPIO_OUT_XOR | GPIO_OE
                    | GPIO_OE_SET | GPIO_OE_CLR | GPIO_OE_XOR),
                ) => self.gpio_word(reg, false),
                Some(_) => self.read_u32(aligned)?,
            }
        };
        let new = (cur & !(0xFF << shift)) | ((value as u32) << shift);
        self.write_u32(aligned, new)
    }

    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        Some(self)
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        if pin >= self.pad_count() {
            return None;
        }
        // The firmware-visible input level, which on this block IS `GPIO_IN` —
        // the pad. Answering from `ext_in` alone would report a level the
        // firmware cannot read back on a pin it is driving itself.
        Some(self.gpio_in() & (1u64 << pin) != 0)
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        if pin >= self.pad_count() {
            return None;
        }
        // The `GPIO_OUT` latch, whether or not `OE` lets it reach the pad.
        Some(self.gpio_out & (1u64 << pin) != 0)
    }

    /// Direction from `OE` while IO_BANK0 hands the pad to SIO (or has not
    /// assigned it); a pad handed to a peripheral is an alternate function,
    /// named when a wire is bound to it.
    fn gpio_routing(&self, pin: u8) -> Option<crate::peripherals::gpio::GpioRouting> {
        use crate::peripherals::gpio::{GpioMode, GpioRouting};
        if pin >= self.pad_count() {
            return None;
        }
        match self.pad_function(pin) {
            None | Some(super::io_bank0::GPIO_FUNC_SIO) => Some(GpioRouting {
                mode: if self.gpio_oe & (1u64 << pin) != 0 {
                    GpioMode::Output
                } else {
                    GpioMode::Input
                },
                func: None,
            }),
            Some(_) => Some(GpioRouting {
                mode: GpioMode::Af,
                func: self
                    .pad_routes
                    .func(pin, |p| self.pad_function(p))
                    .map(str::to_string),
            }),
        }
    }

    /// A pad change (a `GPIO_OUT`/`GPIO_OE` write, an external level) can
    /// raise IO_BANK0's GPIO interrupt, so the bus arms that block's event
    /// chain after every write here and after every external input.
    fn scheduler_wake_owner(&self) -> Option<usize> {
        self.io_bank0_idx
    }

    /// Clearing GPIO_OE releases an SIO output regardless of GPIO_OUT. An
    /// open-drain protocol can therefore hold OUT low and toggle only OE.
    fn gpio_input_is_released(&self) -> bool {
        true
    }

    /// GPIO_OUT / GPIO_IN offsets let chip `pins:` labels (GP0..) resolve
    /// through the bus pin router for attached GPIO devices.
    fn gpio_port_offsets(&self) -> Option<crate::peripherals::gpio::GpioPortOffsets> {
        Some(crate::peripherals::gpio::GpioPortOffsets {
            output: GPIO_OUT,
            input: GPIO_IN,
        })
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

impl PinPort for Rp2040Sio {
    fn pin_count(&self) -> u8 {
        self.pad_count()
    }

    fn driver(&self, pin: u8) -> Option<crate::pins::PadDriver> {
        self.pad_driver(pin)
    }

    fn external(&self, pin: u8) -> External {
        if pin < self.pad_count() && self.ext_mask & (1u64 << pin) != 0 {
            External::Level(self.ext_in & (1u64 << pin) != 0)
        } else {
            External::Released
        }
    }

    /// A level is held in `ext_in`, separate from `GPIO_OUT` because it must
    /// survive the firmware driving the same pad in the other direction: a
    /// button holds its released level from boot, and the pin must return to
    /// it the moment the firmware releases the output driver. Releasing lets
    /// an undriven pad float to 0, as this model has always read one.
    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        if pin >= self.pad_count() {
            return None;
        }
        let bit = 1u64 << pin;
        let before = self.gpio_in() & bit != 0;
        self.mutate_pads(|s| match ext {
            External::Level(level) => {
                s.ext_mask |= bit;
                if level {
                    s.ext_in |= bit;
                } else {
                    s.ext_in &= !bit;
                }
            }
            External::Released => {
                s.ext_mask &= !bit;
                s.ext_in &= !bit;
            }
        });
        self.sync_wire_input(pin);
        Some(InputChange {
            before,
            after: self.gpio_in() & bit != 0,
        })
    }

    /// `pin` joined a world `gpio_net`: every peripheral line it can carry
    /// shares a wire with other chips now.
    fn join_wire(&mut self, pin: u8) {
        self.pad_routes.mark_on_net(pin);
    }

    fn input(&self, pin: u8) -> Option<bool> {
        Peripheral::read_gpio_input(self, pin)
    }

    /// `pad_level`: a routed pad reads its peripheral's wire, any other
    /// `GPIO_IN`.
    fn level(&self, pin: u8) -> Option<bool> {
        self.pad_level(pin)
    }

    fn install_watch(&mut self, watch: Option<crate::pins::PadWatch>) -> bool {
        match watch {
            None => {
                self.watch = None;
                self.pad_routes.clear_taps();
            }
            Some(watch) => {
                self.watch = Some(watch);
                // Routed pads are driven by their peripheral's wire, so the
                // wire reports their transitions at the cycles they occurred.
                self.pad_routes.invalidate_registrations();
                self.sync_pad_routes();
            }
        }
        true
    }

    fn take_watch(&mut self) -> Option<crate::pins::PadWatch> {
        self.watch.take()
    }

    fn put_watch(&mut self, watch: crate::pins::PadWatch) {
        self.watch = Some(watch);
    }

    /// An IO_BANK0 `FUNCSEL` write may have handed a pad to another source:
    /// re-register watched channels, and give a peripheral line that just
    /// took a pad the level the outside holds there.
    fn routes_changed(&mut self) {
        self.sync_pad_routes();
        self.sync_wire_inputs();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIN25: u32 = 1 << 25;

    #[test]
    fn divider_window_is_side_effect_free_but_gpio_is_not() {
        let sio = Rp2040Sio::new();
        assert_eq!(
            sio.mmio_access_class(DIV_UDIVIDEND),
            MmioAccessClass::SideEffectFree
        );
        assert_eq!(
            sio.mmio_access_class(DIV_UDIVISOR),
            MmioAccessClass::SideEffectFree
        );
        assert_eq!(
            sio.mmio_access_class(DIV_QUOTIENT),
            MmioAccessClass::SideEffectFree
        );
        assert_eq!(
            sio.mmio_access_class(DIV_CSR),
            MmioAccessClass::SideEffectFree
        );
        assert_eq!(
            sio.mmio_access_class(GPIO_OUT_SET),
            MmioAccessClass::SideEffecting
        );
        assert_eq!(sio.mmio_access_class(CPUID), MmioAccessClass::SideEffecting);
    }

    #[test]
    fn cpuid_reads_zero() {
        assert_eq!(Rp2040Sio::new().read_u32(CPUID).unwrap(), 0);
    }

    #[test]
    fn set_drive_readback_roundtrip() {
        let mut sio = Rp2040Sio::new();
        // Output disabled → driven level not visible on GPIO_IN.
        sio.write_u32(GPIO_OUT_SET, PIN25).unwrap();
        assert_eq!(sio.read_u32(GPIO_IN).unwrap() & PIN25, 0);
        // Enable output → pin reads back its driven high level.
        sio.write_u32(GPIO_OE_SET, PIN25).unwrap();
        assert_eq!(sio.read_u32(GPIO_IN).unwrap() & PIN25, PIN25);
        assert_eq!(sio.read_u32(GPIO_OUT).unwrap() & PIN25, PIN25);
        // Clear the output → reads back low.
        sio.write_u32(GPIO_OUT_CLR, PIN25).unwrap();
        assert_eq!(sio.read_u32(GPIO_IN).unwrap() & PIN25, 0);
    }

    #[test]
    fn gpio_output_latch_and_direction_are_readable() {
        use crate::peripherals::gpio::GpioMode;
        use crate::Peripheral;
        let mut sio = Rp2040Sio::new();
        sio.write_u32(GPIO_OUT_SET, PIN25).unwrap();
        // The latch reads back whether or not the driver is enabled.
        assert_eq!(sio.read_gpio_output(25), Some(true));
        assert_eq!(sio.read_gpio_output(24), Some(false));
        assert_eq!(sio.read_gpio_output(PAD_COUNT), None);
        // No FUNCSEL bound yet: direction comes from OE alone.
        assert_eq!(sio.read_gpio_is_output(25), Some(false));
        assert_eq!(sio.gpio_routing(25).unwrap().mode, GpioMode::Input);
        sio.write_u32(GPIO_OE_SET, PIN25).unwrap();
        assert_eq!(sio.read_gpio_is_output(25), Some(true));
        assert_eq!(sio.gpio_routing(25).unwrap().mode, GpioMode::Output);
        assert_eq!(sio.gpio_routing(PAD_COUNT), None);
    }

    #[test]
    fn logic_tap_sees_led_pin25_toggle() {
        use crate::logic_capture::LogicTap;
        use crate::Peripheral;
        let mut sio = Rp2040Sio::new();
        let tap = LogicTap::new();
        assert!(sio.install_watch(crate::pins::PadWatch::probes(&tap, &[(25, 0)])));
        // Arm push mode so ingest is live (machine does this on logic_watch).
        tap.set_armed(true);
        sio.write_u32(GPIO_OE_SET, PIN25).unwrap();
        sio.write_u32(GPIO_OUT_SET, PIN25).unwrap();
        sio.write_u32(GPIO_OUT_CLR, PIN25).unwrap();
        let events = tap.take_events();
        assert!(
            events.len() >= 2,
            "expected LED toggle edges, got {:?}",
            events
        );
    }

    #[test]
    fn xor_toggles_output() {
        let mut sio = Rp2040Sio::new();
        sio.write_u32(GPIO_OE_SET, PIN25).unwrap();
        sio.write_u32(GPIO_OUT_XOR, PIN25).unwrap();
        assert_eq!(sio.read_u32(GPIO_IN).unwrap() & PIN25, PIN25);
        sio.write_u32(GPIO_OUT_XOR, PIN25).unwrap();
        assert_eq!(sio.read_u32(GPIO_IN).unwrap() & PIN25, 0);
    }

    /// A `board_io` button drives its pin through `set_gpio_input`; the level
    /// has to be visible to firmware reading GPIO_IN and to the bus's own
    /// read-back proof (`read_gpio_input`), or `attach_board_io_buttons` drops
    /// the button as undrivable — which is exactly what SIO used to do.
    #[test]
    fn externally_driven_input_reads_back_on_an_input_pin() {
        use crate::Peripheral;
        const PIN14: u32 = 1 << 14;
        let mut sio = Rp2040Sio::new();
        // OE clear: pin 14 is an input, and undriven it floats low.
        assert_eq!(sio.read_u32(GPIO_IN).unwrap() & PIN14, 0);
        assert_eq!(sio.read_gpio_input(14), Some(false));

        assert!(sio.set_gpio_input(14, true), "bank-0 pad must be drivable");
        assert_eq!(
            sio.read_u32(GPIO_IN).unwrap() & PIN14,
            PIN14,
            "firmware reading GPIO_IN must see the external level"
        );
        assert_eq!(sio.read_gpio_input(14), Some(true));
        assert_eq!(
            sio.read_gpio_pad(14),
            Some(true),
            "probe agrees with GPIO_IN"
        );

        // Releasing the contact takes the pin back down.
        assert!(sio.set_gpio_input(14, false));
        assert_eq!(sio.read_u32(GPIO_IN).unwrap() & PIN14, 0);
    }

    /// Contention rule: the output driver wins. A pin the firmware drives reads
    /// back its OWN level, so the set-drive-readback round-trip still holds
    /// with an external source attached to the same pad.
    #[test]
    fn output_driver_wins_over_an_external_level() {
        use crate::Peripheral;
        const PIN14: u32 = 1 << 14;
        let mut sio = Rp2040Sio::new();
        assert!(sio.set_gpio_input(14, true));
        // Firmware takes the pin as a low output.
        sio.write_u32(GPIO_OE_SET, PIN14).unwrap();
        assert_eq!(
            sio.read_u32(GPIO_IN).unwrap() & PIN14,
            0,
            "driven pin reads its own low, not the external high"
        );
        assert_eq!(sio.read_gpio_input(14), Some(false));
        // …and releasing the driver hands the pad back to the outside world.
        sio.write_u32(GPIO_OE_CLR, PIN14).unwrap();
        assert_eq!(sio.read_u32(GPIO_IN).unwrap() & PIN14, PIN14);
    }

    /// Out of range is a REFUSAL, not a silent no-op: `attach_board_io_buttons`
    /// reads the return value to decide whether the button can be driven at
    /// all, so a pad this block does not have must answer `false`.
    #[test]
    fn set_gpio_input_refuses_a_pad_outside_bank0() {
        use crate::Peripheral;
        let mut sio = Rp2040Sio::new();
        assert!(
            sio.set_gpio_input(29, true),
            "GPIO29 is the last bank-0 pad"
        );
        assert!(!sio.set_gpio_input(30, true), "GPIO30 is not brought out");
        assert_eq!(sio.read_gpio_input(30), None);
    }

    /// A probe armed on a pad must see the button's edge, not just the
    /// firmware's own writes — the external drive is bracketed by the same
    /// tap snapshot/report pair every SIO latch write uses.
    #[test]
    fn logic_tap_sees_an_externally_driven_edge() {
        use crate::logic_capture::LogicTap;
        use crate::Peripheral;
        let mut sio = Rp2040Sio::new();
        let tap = LogicTap::new();
        assert!(sio.install_watch(crate::pins::PadWatch::probes(&tap, &[(14, 0)])));
        tap.set_armed(true);
        sio.set_gpio_input(14, true);
        sio.set_gpio_input(14, false);
        let events = tap.take_events();
        assert!(
            events.len() >= 2,
            "expected a press and a release edge, got {events:?}"
        );
    }

    #[test]
    fn spinlock_try_lock_and_release() {
        let mut sio = Rp2040Sio::new();
        // First read of a free lock claims it and returns a nonzero value.
        let claimed = sio.read_u32(SPINLOCK0).unwrap();
        assert_ne!(claimed, 0, "free lock is granted on read");
        // While held, a second read returns 0 (would spin on real HW).
        assert_eq!(sio.read_u32(SPINLOCK0).unwrap(), 0, "held lock reads 0");
        // Writing releases it; it can then be claimed again.
        sio.write_u32(SPINLOCK0, 1).unwrap();
        assert_ne!(
            sio.read_u32(SPINLOCK0).unwrap(),
            0,
            "released lock reclaims"
        );
    }

    #[test]
    fn spinlocks_are_independent() {
        // read_u32 claims through a `Cell`, so no `mut` binding is needed.
        let sio = Rp2040Sio::new();
        assert_ne!(sio.read_u32(SPINLOCK0).unwrap(), 0);
        // A different lock is unaffected by claiming lock 0.
        let l31 = sio.read_u32(SPINLOCK31).unwrap();
        assert_ne!(l31, 0, "lock 31 independent of lock 0");
        assert_eq!(l31 & (l31 - 1), 0, "grant value is a single bit (1<<n)");
    }

    /// A pad's own drive is what the SIO output stage does (`GPIO_OE`), never
    /// the level the outside holds on it. A pad IO_BANK0 hands to a
    /// peripheral that publishes no wire has no known drive.
    #[test]
    fn the_own_drive_is_oe() {
        use crate::logic_capture::PadDrive;
        use crate::peripherals::rp2040::io_bank0::Rp2040IoBank0;
        use crate::pins::own_drive;
        use crate::Peripheral;
        let mut bank = Rp2040IoBank0::new();
        let mut sio = Rp2040Sio::new();
        sio.attach_io_bank0(bank.pad_functions(), bank.bank_irq(), 0);
        assert_eq!(sio.driver(30), None);
        sio.set_gpio_input(4, true);
        assert_eq!(own_drive(&sio, 4), Some(PadDrive::HighZ));
        assert_eq!(sio.read_gpio_pad_drive(4), Some(PadDrive::Driven), "probe");
        sio.write_u32(GPIO_OE_SET, 1 << 4).unwrap();
        assert_eq!(own_drive(&sio, 4), Some(PadDrive::Driven));
        assert_eq!(
            sio.read_gpio_pad_drive(4),
            Some(PadDrive::Contention),
            "a probe sees the fight"
        );
        assert_eq!(sio.read_gpio_pad(4), Some(false), "drives its own 0");
        // FUNCSEL = SIO keeps the SIO in charge; PWM (4) publishes no wire.
        <Rp2040IoBank0 as Peripheral>::write_u32(&mut bank, 8 * 4 + 4, 5).unwrap();
        assert_eq!(own_drive(&sio, 4), Some(PadDrive::Driven));
        <Rp2040IoBank0 as Peripheral>::write_u32(&mut bank, 8 * 4 + 4, 4).unwrap();
        assert_eq!(own_drive(&sio, 4), None);
        assert_eq!(sio.scheduler_wake_owner(), Some(0));
    }

    /// A probe on the pad reports `1` / `0` / `z` from the SIO output stage
    /// (`OE`, `OUT`) when nothing outside holds it.
    #[test]
    fn an_off_net_pad_reports_its_own_drive_state() {
        use crate::logic_capture::{PadDrive, PadState};
        use crate::Peripheral;
        let mut sio = Rp2040Sio::new();
        let state = |sio: &Rp2040Sio| {
            PadState::from_parts(sio.read_gpio_pad(4), sio.read_gpio_pad_drive(4).unwrap()).unwrap()
        };
        assert_eq!(sio.read_gpio_pad_drive(4), Some(PadDrive::HighZ));
        assert_eq!(state(&sio), PadState::HighZ, "!OE: z");
        sio.write_u32(GPIO_OE_SET, 1 << 4).unwrap();
        assert_eq!(state(&sio), PadState::Low, "OE, OUT=0: 0");
        sio.write_u32(GPIO_OUT_SET, 1 << 4).unwrap();
        assert_eq!(state(&sio), PadState::High, "OE, OUT=1: 1");
        assert_eq!(sio.read_gpio_pad_drive(PAD_COUNT), None, "no such pad");
    }

    /// RP2350 QFN80: GPIO32..47 are bits [15:0] of the `GPIO_HI_*` words
    /// (pico-sdk rp2350 `sio.h`), independent of the lo words.
    #[test]
    fn rp2350b_hi_words_drive_gpio32_to_47() {
        use crate::Peripheral;
        const HI_IN: u64 = 0x08;
        const OUT: u64 = 0x10;
        const HI_OUT: u64 = 0x14;
        const HI_OUT_XOR: u64 = 0x2c;
        const OE: u64 = 0x30;
        const HI_OE_SET: u64 = 0x3c;
        const HI_OE: u64 = 0x34;
        let mut sio = Rp2040Sio::new().with_layout(SioLayout::Rp2350B);
        assert_eq!(sio.pin_count(), 48);
        sio.write_u32(HI_OE_SET, 1 << 15).unwrap();
        sio.write_u32(HI_OUT_XOR, 1 << 15).unwrap();
        assert_eq!(sio.read_gpio_output(47), Some(true));
        assert_eq!(sio.read_gpio_input(47), Some(true));
        assert_eq!(sio.read_u32(HI_IN).unwrap(), 1 << 15);
        assert_eq!(sio.read_u32(HI_OE).unwrap(), 1 << 15);
        // A direct lo-word write leaves the HI pads alone, and vice versa.
        sio.write_u32(OE, u32::MAX).unwrap();
        sio.write_u32(OUT, 0).unwrap();
        assert_eq!(sio.read_gpio_output(47), Some(true));
        assert_eq!(sio.read_u32(OE).unwrap(), u32::MAX, "GPIO30/31 are pads");
        sio.write_u32(HI_OUT, 0).unwrap();
        assert_eq!(sio.read_u32(OE).unwrap(), u32::MAX);
        // QSPI/USB bits of the HI words are not modelled pads.
        sio.write_u32(HI_OUT, u32::MAX).unwrap();
        assert_eq!(sio.read_u32(HI_OUT).unwrap(), 0xffff);
        // A byte store composes into the HI word.
        sio.write(HI_OUT + 1, 0x00).unwrap();
        assert_eq!(sio.read_u32(HI_OUT).unwrap(), 0x00ff);
        assert_eq!(sio.read_gpio_output(47), Some(false));
        // An external level on an input pad reads back on GPIO_HI_IN.
        sio.write_u32(0x44, 1 << 8).unwrap(); // GPIO_HI_OE_CLR: GPIO40 input
        sio.set_gpio_input(40, true);
        assert_ne!(sio.read_u32(HI_IN).unwrap() & (1 << 8), 0);
        assert_eq!(sio.read_gpio_input(48), None, "no GPIO48");
    }

    /// The RP2350A layout keeps its 30 pads: the HI words stay inert.
    #[test]
    fn rp2350a_hi_words_stay_inert() {
        use crate::Peripheral;
        let mut sio = Rp2040Sio::new().with_layout(SioLayout::Rp2350);
        assert_eq!(sio.pin_count(), PAD_COUNT);
        sio.write_u32(0x3c, 1 << 15).unwrap();
        sio.write_u32(0x1c, 1 << 15).unwrap();
        assert_eq!(sio.read_u32(0x08).unwrap(), 0);
        assert_eq!(sio.read_gpio_input(47), None);
    }
}
