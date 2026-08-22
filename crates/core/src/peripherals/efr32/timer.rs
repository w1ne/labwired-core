// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Silicon Labs EFR32 Series-2 **TIMER** — the counter behind `micros()` and
//! `analogWrite()`.
//!
//! # Sources
//!
//! Offsets walked from `TIMER_TypeDef` in `efr32mg26_timer.h` (`simplicity_sdk`
//! tag `sisdk-2025.6`), with the `CC[3]` group resolved from
//! `TIMER_CC_TypeDef` (stride `0x20`: CFG, CTRL, OC, reserved, OCB, ICF, ICOF,
//! reserved). Field positions and reset values are the `_TIMER_…_SHIFT` /
//! `_RESETVALUE` defines from the same header. `IPVERSION_SET` lands at exactly
//! `+0x1000`, which is the check that the group stride is right.
//!
//! # ⚠️ The counter width is per instance, and it is not the obvious four
//!
//! `TIMER_CNTWIDTH` in the device header
//! (`efr32mg26b510f3200im48.h`) says:
//!
//! | instance | width |   | instance | width |
//! |----------|-------|---|----------|-------|
//! | TIMER0   | 32    |   | TIMER5   | 16    |
//! | TIMER1   | 32    |   | TIMER6   | 16    |
//! | TIMER2   | 16    |   | TIMER7   | 16    |
//! | TIMER3   | 16    |   | TIMER8   | 32    |
//! | TIMER4   | 16    |   | TIMER9   | 32    |
//!
//! The 32-bit ones are **0, 1, 8 and 9** — not 0..3. The datasheet's summary
//! line ("4x 32-bit, 3-ch") gives the count but not which, and the reference
//! manual explicitly defers to the datasheet. Guessing 0..3 gives a `micros()`
//! that wraps 65536× too early on TIMER2 and never on TIMER8. So the width is
//! a required `config: { counter_bits: … }` on the chip yaml rather than a
//! constant here, and each instance carries its own.
//!
//! # Faithfully modelled
//!
//! * `EN.EN` gates everything; `CMD.START`/`CMD.STOP` run and halt the
//!   counter, and `STATUS.RUNNING` reports it.
//! * `CFG.PRESC` divides: the counter advances once per `PRESC + 1` timer
//!   clocks, which is what makes a 1 MHz `micros()` tick out of a 78 MHz core.
//! * `CNT` counts up to `TOP` (reset `0xFFFF`) and wraps to 0, setting
//!   `IF.OF`. `TOP` and `CNT` are masked to the instance's width, so a 16-bit
//!   timer cannot hold a 17-bit value even if firmware writes one.
//! * The three compare/capture channels in OUTPUTCOMPARE or PWM mode: a
//!   channel whose `OC` the counter passes sets `IF.CC[n]`.
//! * PWM duty: in `CC_CFG.MODE = PWM` the channel's output is high while
//!   `CNT < OC`, which is what `analogWrite` sets up, and
//!   [`Self::pwm_duty_percent`] exposes it.
//! * `IF` is write-1-to-clear and `IEN` gates the interrupt.
//!
//! # Idealised — present, but not physical
//!
//! * **Up-count only.** `CFG.MODE` stores; down and up/down counting, the
//!   quadrature decoder and 2× count mode are not modelled, and a firmware
//!   that selects one gets an up-counter.
//! * **The PWM waveform does not reach a pad.** The output state is modelled
//!   and readable, but `GPIO_TIMERROUTE` is not, so nothing drives a GPIO.
//!   An LED on a `analogWrite` pin does not dim in a board view yet.
//! * **No input capture.** `ICF`/`ICOF` read 0 and `MODE = INPUTCAPTURE`
//!   captures nothing.
//! * **No dead-time insertion.** The whole `DT*` block stores and does
//!   nothing, which is safe here only because nothing drives a pad; on
//!   silicon those registers protect a half-bridge.
//! * **No `TOPB` buffering, no `LOCK`.** `TOPB` stores; `TOP` takes effect
//!   immediately rather than at the next wrap.
//! * **`CFG.CLKSEL` stores and selects nothing.** There is no clock tree to
//!   select from (see the CMU model). The timebase is whatever the chip yaml
//!   declares — see below.
//!
//! # ⚠️ The timebase is the PERIPHERAL clock, not the core clock
//!
//! Firmware computes `PRESC` from the clock the TIMER actually runs on, which
//! is the EM01 group A peripheral clock. Out of reset `CMU_EM01GRPACLKCTRL`
//! selects the HFRCODPLL path at its startup band — **19 MHz**
//! (`HFRCODPLL_STARTUP_FREQ`, `system_efr32mg26.c`), which is also the value
//! this core's `boards.txt` publishes as `F_CPU` and the value the demo
//! firmware's USART divisor is computed against.
//!
//! `ChipDescriptor::cpu_hz` is a different number: the part's 78 MHz maximum
//! CORE frequency. Counting at that instead would make every interval in the
//! twin run 4.1x fast against the same firmware on the bench — a `micros()`
//! that reads four times too many microseconds. So the timebase is
//! `config: { peripheral_hz: … }`, defaulting to the attached `cpu_hz` for a
//! chip whose two clocks are the same.
//!
//! Closing this properly means modelling the clock tree so `CLKSEL` and the
//! HFRCO band decide the number. Until then it is one declared constant per
//! instance, and the reason it is not `cpu_hz` is written here.

use crate::peripherals::pad_lines::PadLines;
use crate::{Peripheral, PeripheralTickResult, SimResult};

// ── Register offsets, walked from `TIMER_TypeDef` ──────────────────────────
const OFF_IPVERSION: u64 = 0x00;
const OFF_CFG: u64 = 0x04;
const OFF_CTRL: u64 = 0x08;
const OFF_CMD: u64 = 0x0C;
const OFF_STATUS: u64 = 0x10;
const OFF_IF: u64 = 0x14;
const OFF_IEN: u64 = 0x18;
const OFF_TOP: u64 = 0x1C;
const OFF_TOPB: u64 = 0x20;
const OFF_CNT: u64 = 0x24;
const OFF_LOCK: u64 = 0x2C;
const OFF_EN: u64 = 0x30;
/// `CC[3]`, stride 0x20.
const OFF_CC: u64 = 0x60;
const CC_STRIDE: u64 = 0x20;
const CC_COUNT: usize = 3;
/// Within a `TIMER_CC_TypeDef`.
const CC_CFG: u64 = 0x00;
const CC_CTRL: u64 = 0x04;
const CC_OC: u64 = 0x08;
const CC_OCB: u64 = 0x10;
const CC_ICF: u64 = 0x14;
const CC_ICOF: u64 = 0x18;
/// The dead-time insertion block, `DTCFG`..`DTLOCK`.
const OFF_DT_FIRST: u64 = 0xE0;
const OFF_DT_LAST: u64 = 0xFC;

/// `_TIMER_IPVERSION_RESETVALUE` in `efr32mg26_timer.h`.
///
/// ⚠️ This was modelled as 3 — a guess. The header says 1 and BRD2709A reads 1
/// over SWD.
const IPVERSION_RESET: u32 = 1;
/// `_TIMER_TOP_RESETVALUE`. Only observable once `EN.EN` is set — see
/// [`Efr32s2Timer::read_word`].
const TOP_RESET: u32 = 0x0000_FFFF;

const EN_EN: u32 = 1 << 0;
const CMD_START: u32 = 1 << 0;
const CMD_STOP: u32 = 1 << 1;
const STATUS_RUNNING: u32 = 1 << 0;
const IF_OF: u32 = 1 << 0;
/// `IF.CC0` is bit 4, so channel `n` is bit `4 + n`.
const IF_CC0_SHIFT: u32 = 4;

/// `_TIMER_CFG_PRESC_MASK` = 0x0FFC_0000, i.e. a 10-bit field at bit 18.
const CFG_PRESC_SHIFT: u32 = 18;
const CFG_PRESC_MASK: u32 = 0x3FF;

/// `CC_CFG.MODE`.
const CC_MODE_MASK: u32 = 0x3;
const CC_MODE_OUTPUTCOMPARE: u32 = 0x2;
const CC_MODE_PWM: u32 = 0x3;

/// EFR32 Series-2 general purpose timer.
#[derive(Debug)]
pub struct Efr32s2Timer {
    /// Counter width in bits, from the instance's `TIMER_CNTWIDTH`. See the
    /// module header — this is per instance and not guessable.
    counter_bits: u32,
    /// Timer clock in Hz — the PERIPHERAL clock, not the core clock. See the
    /// module header for why the two differ on this part. `None` until the
    /// chip yaml or the bus supplies one.
    peripheral_hz: Option<u64>,
    /// The routed pads' live CC levels, when `GPIO_TIMERROUTE` points a pad at
    /// this timer. `None` until something routes them.
    lines: Option<std::sync::Arc<PadLines>>,
    /// The core clock the bus attached, used only as the fallback timebase for
    /// a chip that does not declare `peripheral_hz`.
    cpu_hz: u64,

    en: u32,
    cfg: u32,
    ctrl: u32,
    status_running: bool,
    iflag: u32,
    ien: u32,
    top: u32,
    topb: u32,
    cnt: u32,
    lock: u32,
    cc_cfg: [u32; CC_COUNT],
    cc_ctrl: [u32; CC_COUNT],
    cc_oc: [u32; CC_COUNT],
    cc_ocb: [u32; CC_COUNT],
    dt: [u32; 8],

    /// Core clocks not yet turned into timer clocks, so a peripheral clock
    /// that is not a divisor of the core clock loses no fraction.
    prescale_residue: u64,
    /// Timer clocks not yet turned into counter steps, so a prescaler larger
    /// than the tick interval does not lose the remainder.
    presc_residue: u64,
}

impl Efr32s2Timer {
    /// `counter_bits` must be the instance's real width — 16 or 32.
    pub fn new(counter_bits: u32) -> Self {
        Self {
            counter_bits: counter_bits.clamp(1, 32),
            peripheral_hz: None,
            lines: None,
            cpu_hz: 1_000_000,
            en: 0,
            cfg: 0,
            ctrl: 0,
            status_running: false,
            iflag: 0,
            ien: 0,
            top: TOP_RESET,
            // `_TIMER_TOPB_RESETVALUE` is 0, not TOP's 0xFFFF. The die cannot
            // arbitrate — it reads 0 here either way while EN is clear — so
            // the header is the source, and the header says 0.
            topb: 0,
            cnt: 0,
            lock: 0,
            cc_cfg: [0; CC_COUNT],
            cc_ctrl: [0; CC_COUNT],
            cc_oc: [0; CC_COUNT],
            cc_ocb: [0; CC_COUNT],
            dt: [0; 8],
            prescale_residue: 0,
            presc_residue: 0,
        }
    }

    /// Declare the timer clock in Hz, from `config: { peripheral_hz: … }`.
    pub fn set_peripheral_hz(&mut self, hz: u64) {
        self.peripheral_hz = Some(hz.max(1));
    }

    /// The timebase this instance counts on: the declared peripheral clock, or
    /// the core clock when a chip's two are the same and it declares neither.
    fn timer_hz(&self) -> u64 {
        self.peripheral_hz.unwrap_or(self.cpu_hz)
    }

    /// Mask a value to this instance's counter width. A 16-bit timer handed a
    /// 32-bit `TOP` keeps the low half, as the silicon's narrower register
    /// does.
    fn mask(&self, value: u32) -> u32 {
        if self.counter_bits >= 32 {
            value
        } else {
            value & ((1u32 << self.counter_bits) - 1)
        }
    }

    /// Counter steps per timer clock denominator: `PRESC + 1`.
    fn prescale_divisor(&self) -> u64 {
        (((self.cfg >> CFG_PRESC_SHIFT) & CFG_PRESC_MASK) as u64) + 1
    }

    fn cc_mode(&self, ch: usize) -> u32 {
        self.cc_cfg[ch] & CC_MODE_MASK
    }

    /// The duty cycle channel `ch` is programmed for, in percent, or `None`
    /// when the channel is not in PWM mode. `analogWrite` is exactly this
    /// number, and a lab or a UI can read it without decoding registers.
    pub fn pwm_duty_percent(&self, ch: usize) -> Option<u32> {
        if ch >= CC_COUNT || self.cc_mode(ch) != CC_MODE_PWM {
            return None;
        }
        let top = self.top.max(1);
        Some((self.cc_oc[ch].min(top) * 100) / top)
    }

    /// Whether channel `ch`'s PWM output is high right now: high while
    /// `CNT < OC`.
    pub fn pwm_output_high(&self, ch: usize) -> bool {
        ch < CC_COUNT && self.cc_mode(ch) == CC_MODE_PWM && self.cnt < self.cc_oc[ch]
    }

    /// Line order for this timer's [`PadLines`]: one per CC channel, named the
    /// way the reference manual and `GPIO_TIMERROUTE` name them.
    pub const CC_LINES: &'static [&'static str] = &["CC0", "CC1", "CC2"];

    /// The narration cell this timer's CC outputs publish into, created on
    /// first use. A PWM output rests LOW: `CNT` starts at 0 and `OC` at 0, so
    /// `CNT < OC` is false until firmware programs a duty.
    pub(crate) fn pad_lines_arc(&mut self) -> std::sync::Arc<PadLines> {
        self.lines
            .get_or_insert_with(|| {
                std::sync::Arc::new(PadLines::new(Self::CC_LINES, &[false; CC_COUNT]))
            })
            .clone()
    }

    /// Publish every channel's current PWM level.
    ///
    /// ⚠️ Called after ANYTHING that can move a level — a counter advance, a
    /// duty write, a mode change, an enable. Cheap and idempotent:
    /// `PadLines::set_line` is a relaxed store that records an edge only on a
    /// real transition, and the whole call is one `is_none` check on a bus
    /// where no pad is routed here.
    fn publish_cc(&self) {
        let Some(lines) = self.lines.as_ref() else {
            return;
        };
        for ch in 0..CC_COUNT {
            lines.set_line(ch, self.pwm_output_high(ch));
        }
    }

    fn cc_index(offset: u64) -> Option<(usize, u64)> {
        if !(OFF_CC..OFF_CC + CC_STRIDE * CC_COUNT as u64).contains(&offset) {
            return None;
        }
        let rel = offset - OFF_CC;
        Some(((rel / CC_STRIDE) as usize, rel % CC_STRIDE))
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            OFF_IPVERSION => IPVERSION_RESET,
            OFF_CFG => self.cfg,
            OFF_CTRL => self.ctrl,
            // CMD is write-only: every bit is a command, none is state.
            OFF_CMD => 0,
            OFF_STATUS => {
                if self.status_running {
                    STATUS_RUNNING
                } else {
                    0
                }
            }
            OFF_IF => self.iflag,
            OFF_IEN => self.ien,
            // ⚠️ The counter block is held in reset while the module is
            // disabled, and reads 0 — NOT `_TIMER_TOP_RESETVALUE`.
            //
            // Measured, not reasoned about: at `reset halt` with TIMER0/1
            // clocked but `EN` clear, BRD2709A reads 0 at TOP and TOPB while
            // the vendor header documents TOP's reset as 0xFFFF. IPVERSION is
            // outside the enable domain and does read its 1. See
            // `scripts/hw-oracle/captures/efr32mg26/`.
            //
            // The held value is what appears once EN is set, so a driver that
            // writes TOP while disabled and enables afterwards — the normal
            // Series-2 order, since CFG is disabled-only — is unaffected.
            OFF_TOP if self.en & EN_EN == 0 => 0,
            OFF_TOPB if self.en & EN_EN == 0 => 0,
            OFF_TOP => self.top,
            OFF_TOPB => self.topb,
            OFF_CNT => self.cnt,
            OFF_LOCK => self.lock,
            OFF_EN => self.en,
            o if (OFF_DT_FIRST..=OFF_DT_LAST).contains(&o) => {
                self.dt[((o - OFF_DT_FIRST) / 4) as usize]
            }
            o => match Self::cc_index(o) {
                Some((ch, CC_CFG)) => self.cc_cfg[ch],
                Some((ch, CC_CTRL)) => self.cc_ctrl[ch],
                Some((ch, CC_OC)) => self.cc_oc[ch],
                Some((ch, CC_OCB)) => self.cc_ocb[ch],
                // Input capture is not modelled; both capture registers read 0
                // rather than a stale or invented sample.
                Some((_, CC_ICF)) | Some((_, CC_ICOF)) => 0,
                _ => 0,
            },
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        self.write_word_inner(offset, value);
        // A duty, a mode, an enable or a manual CNT can all move an output
        // level without the counter advancing. Publishing here means a pad
        // follows `analogWrite` immediately rather than at the next tick.
        self.publish_cc();
    }

    fn write_word_inner(&mut self, offset: u64, value: u32) {
        match offset {
            OFF_EN => {
                self.en = value & EN_EN;
                if self.en == 0 {
                    self.status_running = false;
                    self.cnt = 0;
                }
            }
            OFF_CFG => self.cfg = value,
            OFF_CTRL => self.ctrl = value,
            OFF_CMD => {
                // STOP wins when both are written in one word: silicon
                // resolves it that way and it is the safe reading.
                if value & CMD_START != 0 && self.en & EN_EN != 0 {
                    self.status_running = true;
                }
                if value & CMD_STOP != 0 {
                    self.status_running = false;
                }
            }
            OFF_IF => self.iflag &= !value,
            OFF_IEN => self.ien = value,
            OFF_TOP => self.top = self.mask(value),
            OFF_TOPB => self.topb = self.mask(value),
            OFF_CNT => self.cnt = self.mask(value),
            OFF_LOCK => self.lock = value,
            o if (OFF_DT_FIRST..=OFF_DT_LAST).contains(&o) => {
                self.dt[((o - OFF_DT_FIRST) / 4) as usize] = value;
            }
            o => {
                if let Some((ch, reg)) = Self::cc_index(o) {
                    match reg {
                        CC_CFG => self.cc_cfg[ch] = value,
                        CC_CTRL => self.cc_ctrl[ch] = value,
                        CC_OC => self.cc_oc[ch] = self.mask(value),
                        CC_OCB => self.cc_ocb[ch] = self.mask(value),
                        _ => {}
                    }
                }
            }
        }
    }

    /// Advance the counter by `steps`, raising overflow and compare flags on
    /// the way. Split out so the wrap logic is one place and a multi-wrap
    /// advance (a big tick interval, a large prescaler) cannot skip a flag.
    fn advance(&mut self, steps: u64) {
        if steps == 0 {
            return;
        }
        let period = self.top as u64 + 1;
        // A compare fires when the counter PASSES its value. Over an advance
        // that wraps, every channel whose OC lies anywhere in the span fires —
        // stepping by more than one count must not silently skip a match, or a
        // large tick interval would drop compares a small one catches.
        let from = self.cnt as u64;
        let wrapped = from + steps >= period;
        for ch in 0..CC_COUNT {
            let mode = self.cc_mode(ch);
            if mode != CC_MODE_OUTPUTCOMPARE && mode != CC_MODE_PWM {
                continue;
            }
            let oc = self.cc_oc[ch] as u64;
            let hit = if wrapped {
                // The span covers [from, period) plus [0, remainder].
                oc >= from || oc <= (from + steps) % period
            } else {
                oc > from && oc <= from + steps
            };
            if hit {
                self.iflag |= 1 << (IF_CC0_SHIFT + ch as u32);
            }
        }
        if wrapped {
            self.iflag |= IF_OF;
        }
        self.cnt = self.mask(((from + steps) % period) as u32);
        self.publish_cc();
    }
}

impl Peripheral for Efr32s2Timer {
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

    fn needs_legacy_walk(&self) -> bool {
        true
    }

    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        let mut result = PeripheralTickResult::default();
        if self.en & EN_EN != 0 && self.status_running {
            // `cycles` counts CORE clocks. Convert to timer clocks first, at
            // the ratio between the two — otherwise a chip whose peripheral
            // clock is a quarter of its core clock counts four times too fast.
            // The remainder carries in core clocks so no fraction is lost.
            let timer_clocks = self.prescale_residue + cycles * self.timer_hz();
            let per_timer_clock = self.cpu_hz.max(1);
            let ticks = timer_clocks / per_timer_clock;
            self.prescale_residue = timer_clocks % per_timer_clock;

            let divisor = self.prescale_divisor();
            let total = self.presc_residue + ticks;
            self.presc_residue = total % divisor;
            self.advance(total / divisor);
        }
        if self.iflag & self.ien != 0 {
            result.irq = true;
        }
        result
    }

    /// The fallback timebase for a chip whose peripheral and core clocks are
    /// the same. On this part they are NOT — see the module header — so the
    /// chip yaml declares `peripheral_hz` and this only sets the ratio.
    fn attach_cpu_hz(&mut self, hz: u64) {
        self.cpu_hz = hz.max(1);
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// ⚠️ Needed by the pad-wiring pass, which reaches this model MUTABLY to
    /// hand it a narration cell — and the trait's mutable accessor DEFAULTS TO
    /// `None`. Implementing only the shared one compiles, passes every unit
    /// test, and silently wires nothing: this exact miss cost two debugging
    /// rounds in one change, once here and once on the route block.
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TIMER0's width, from `TIMER0_CNTWIDTH`.
    const T0_BITS: u32 = 32;
    /// TIMER2's width. Deliberately used in the width tests, because TIMER2
    /// being 16-bit is the fact a "timers 0..3 are the 32-bit ones" guess gets
    /// wrong.
    const T2_BITS: u32 = 16;

    fn running(bits: u32) -> Efr32s2Timer {
        let mut t = Efr32s2Timer::new(bits);
        t.write_word(OFF_EN, EN_EN);
        t.write_word(OFF_CMD, CMD_START);
        t
    }

    /// ⚠️ The timebase is the PERIPHERAL clock. On this part it is 19 MHz out
    /// of reset while the core runs at up to 78 MHz, and counting at the core
    /// clock would make every interval in the twin run 4.1x fast against the
    /// same firmware on the bench.
    #[test]
    fn the_counter_runs_on_the_peripheral_clock_not_the_core_clock() {
        let mut t = Efr32s2Timer::new(T0_BITS);
        t.attach_cpu_hz(78_000_000);
        t.set_peripheral_hz(19_000_000);
        t.write_word(OFF_EN, EN_EN);
        t.write_word(OFF_CMD, CMD_START);
        t.write_word(OFF_TOP, 0xFFFF_FFFF);

        // One second of CORE clocks must advance the counter by one second of
        // TIMER clocks, not of core clocks.
        t.tick_elapsed(78_000_000);
        assert_eq!(t.read_word(OFF_CNT), 19_000_000);
    }

    /// The `micros()` setup as firmware actually writes it: PRESC from F_CPU,
    /// which is the 19 MHz peripheral clock, giving a 1 MHz tick.
    #[test]
    fn a_presc_computed_from_f_cpu_gives_a_one_microsecond_tick() {
        let mut t = Efr32s2Timer::new(T0_BITS);
        t.attach_cpu_hz(78_000_000);
        t.set_peripheral_hz(19_000_000);
        t.write_word(OFF_EN, EN_EN);
        t.write_word(OFF_CMD, CMD_START);
        t.write_word(OFF_TOP, 0xFFFF_FFFF);
        t.write_word(OFF_CFG, 18 << CFG_PRESC_SHIFT); // 19 MHz / 19 = 1 MHz

        t.tick_elapsed(78_000_000); // one second of core clocks
        assert_eq!(t.read_word(OFF_CNT), 1_000_000, "one million microseconds");
    }

    /// A chip whose two clocks ARE the same declares no `peripheral_hz` and
    /// counts on the core clock, which is the historical behaviour.
    #[test]
    fn without_a_declared_peripheral_clock_the_core_clock_is_the_timebase() {
        let mut t = Efr32s2Timer::new(T0_BITS);
        t.attach_cpu_hz(48_000_000);
        t.write_word(OFF_EN, EN_EN);
        t.write_word(OFF_CMD, CMD_START);
        t.write_word(OFF_TOP, 0xFFFF_FFFF);
        t.tick_elapsed(48_000_000);
        assert_eq!(t.read_word(OFF_CNT), 48_000_000);
    }

    #[test]
    fn resets_to_the_header_values() {
        let mut t = Efr32s2Timer::new(T0_BITS);
        assert_eq!(t.read_word(OFF_IPVERSION), IPVERSION_RESET);
        assert_eq!(t.read_word(OFF_CNT), 0);
        assert_eq!(t.read_word(OFF_STATUS), 0, "not running out of reset");

        // Disabled, the counter block reads 0 — this is what the die does.
        assert_eq!(t.read_word(OFF_TOP), 0, "TOP is held in reset while EN=0");
        assert_eq!(t.read_word(OFF_TOPB), 0, "TOPB likewise");

        // Enabling releases it, and TOP presents its documented reset value.
        t.write_word(OFF_EN, EN_EN);
        assert_eq!(t.read_word(OFF_TOP), TOP_RESET);
        assert_eq!(t.read_word(OFF_TOPB), 0, "_TIMER_TOPB_RESETVALUE");
    }

    /// A TOP written while the module is disabled — the normal Series-2 order,
    /// because `CFG` may only be written with `EN` clear — survives the enable.
    #[test]
    fn a_top_written_while_disabled_appears_when_enabled() {
        let mut t = Efr32s2Timer::new(T0_BITS);
        t.write_word(OFF_TOP, 999);
        t.write_word(OFF_EN, EN_EN);
        assert_eq!(t.read_word(OFF_TOP), 999);
    }

    #[test]
    fn a_stopped_timer_does_not_count() {
        let mut t = Efr32s2Timer::new(T0_BITS);
        t.write_word(OFF_EN, EN_EN);
        t.tick_elapsed(1000);
        assert_eq!(t.read_word(OFF_CNT), 0);

        // ...and neither does a started timer that was never enabled.
        let mut t = Efr32s2Timer::new(T0_BITS);
        t.write_word(OFF_CMD, CMD_START);
        t.tick_elapsed(1000);
        assert_eq!(t.read_word(OFF_CNT), 0);
        assert_eq!(t.read_word(OFF_STATUS) & STATUS_RUNNING, 0);
    }

    #[test]
    fn a_running_timer_counts_one_per_clock_at_prescaler_zero() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_TOP, 0xFFFF_FFFF);
        t.tick_elapsed(1234);
        assert_eq!(t.read_word(OFF_CNT), 1234);
    }

    /// The `micros()` setup: 78 MHz core, PRESC 77, so one count is one
    /// microsecond.
    #[test]
    fn the_prescaler_divides_by_presc_plus_one() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_TOP, 0xFFFF_FFFF);
        t.write_word(OFF_CFG, 77 << CFG_PRESC_SHIFT);
        t.tick_elapsed(78_000_000); // one second at 78 MHz
        assert_eq!(t.read_word(OFF_CNT), 1_000_000, "1 MHz tick");
    }

    /// A prescaler bigger than one tick interval must not lose the remainder,
    /// or a slow timer stops entirely under a fine tick.
    #[test]
    fn the_prescaler_remainder_carries_across_ticks() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_TOP, 0xFFFF_FFFF);
        t.write_word(OFF_CFG, 999 << CFG_PRESC_SHIFT); // divide by 1000
        for _ in 0..1000 {
            t.tick_elapsed(1);
        }
        assert_eq!(t.read_word(OFF_CNT), 1);
    }

    #[test]
    fn the_counter_wraps_at_top_and_sets_the_overflow_flag() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_TOP, 9);
        t.tick_elapsed(9);
        assert_eq!(t.read_word(OFF_CNT), 9);
        assert_eq!(t.read_word(OFF_IF) & IF_OF, 0, "not yet");

        t.tick_elapsed(1);
        assert_eq!(t.read_word(OFF_CNT), 0, "wrapped to zero");
        assert_eq!(t.read_word(OFF_IF) & IF_OF, IF_OF);
    }

    /// ⚠️ The fact a "TIMER0..3 are the 32-bit ones" guess gets wrong: TIMER2
    /// is 16-bit, so it cannot hold a 32-bit TOP.
    #[test]
    fn a_sixteen_bit_instance_masks_top_and_cnt() {
        let mut t = running(T2_BITS);
        t.write_word(OFF_TOP, 0xFFFF_FFFF);
        assert_eq!(t.read_word(OFF_TOP), 0xFFFF, "a 16-bit TOP register");

        t.write_word(OFF_CNT, 0x1_2345);
        assert_eq!(t.read_word(OFF_CNT), 0x2345);
    }

    #[test]
    fn a_thirty_two_bit_instance_holds_the_whole_value() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_TOP, 0xFFFF_FFFF);
        assert_eq!(t.read_word(OFF_TOP), 0xFFFF_FFFF);
    }

    #[test]
    fn a_compare_channel_flags_when_the_counter_passes_it() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_TOP, 100);
        t.write_word(OFF_CC + CC_CFG, CC_MODE_OUTPUTCOMPARE);
        t.write_word(OFF_CC + CC_OC, 40);

        t.tick_elapsed(39);
        assert_eq!(t.read_word(OFF_IF) & (1 << IF_CC0_SHIFT), 0);
        t.tick_elapsed(1);
        assert_eq!(t.read_word(OFF_IF) & (1 << IF_CC0_SHIFT), 1 << IF_CC0_SHIFT);
    }

    /// A big advance must not step over a compare. Under a coarse
    /// `peripheral_tick_interval` the counter moves in jumps, and a naive
    /// `cnt == oc` test would fire only when the jump landed exactly.
    #[test]
    fn a_compare_is_not_skipped_by_a_large_advance() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_TOP, 1000);
        t.write_word(OFF_CC + CC_CFG, CC_MODE_OUTPUTCOMPARE);
        t.write_word(OFF_CC + CC_OC, 500);

        t.tick_elapsed(999);
        assert_eq!(
            t.read_word(OFF_IF) & (1 << IF_CC0_SHIFT),
            1 << IF_CC0_SHIFT,
            "the advance stepped over 500 and must still flag it"
        );
    }

    /// ...and one that wraps must flag a compare on the far side of the wrap.
    #[test]
    fn a_compare_after_a_wrap_still_flags() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_TOP, 99);
        t.write_word(OFF_CC + CC_CFG, CC_MODE_OUTPUTCOMPARE);
        t.write_word(OFF_CC + CC_OC, 10);
        t.write_word(OFF_CNT, 90);

        t.tick_elapsed(30); // 90 → 20, passing 0 and 10
        assert_eq!(t.read_word(OFF_IF) & (1 << IF_CC0_SHIFT), 1 << IF_CC0_SHIFT);
        assert_eq!(t.read_word(OFF_IF) & IF_OF, IF_OF);
    }

    #[test]
    fn a_channel_that_is_off_never_flags() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_TOP, 100);
        t.write_word(OFF_CC + CC_OC, 40); // OC set, but MODE stays OFF
        t.tick_elapsed(100);
        assert_eq!(t.read_word(OFF_IF) & (1 << IF_CC0_SHIFT), 0);
    }

    /// Channels are independent, and channel n's flag is bit 4+n.
    #[test]
    fn each_channel_has_its_own_compare_and_its_own_flag() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_TOP, 100);
        for ch in 0..CC_COUNT as u64 {
            t.write_word(OFF_CC + ch * CC_STRIDE + CC_CFG, CC_MODE_OUTPUTCOMPARE);
            t.write_word(OFF_CC + ch * CC_STRIDE + CC_OC, 10 + ch as u32 * 20);
        }
        t.tick_elapsed(15);
        let iflag = t.read_word(OFF_IF);
        assert_eq!(iflag & (1 << 4), 1 << 4, "CC0 at 10 fired");
        assert_eq!(iflag & (1 << 5), 0, "CC1 at 30 did not");
        assert_eq!(iflag & (1 << 6), 0, "CC2 at 50 did not");
    }

    #[test]
    fn pwm_duty_is_oc_over_top() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_TOP, 255);
        t.write_word(OFF_CC + CC_CFG, CC_MODE_PWM);
        t.write_word(OFF_CC + CC_OC, 64);
        assert_eq!(t.pwm_duty_percent(0), Some(25));

        t.write_word(OFF_CC + CC_OC, 255);
        assert_eq!(t.pwm_duty_percent(0), Some(100));
        t.write_word(OFF_CC + CC_OC, 0);
        assert_eq!(t.pwm_duty_percent(0), Some(0));
    }

    #[test]
    fn a_channel_not_in_pwm_mode_reports_no_duty() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_CC + CC_CFG, CC_MODE_OUTPUTCOMPARE);
        assert_eq!(t.pwm_duty_percent(0), None);
    }

    #[test]
    fn the_pwm_output_is_high_below_the_compare_value() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_TOP, 99);
        t.write_word(OFF_CC + CC_CFG, CC_MODE_PWM);
        t.write_word(OFF_CC + CC_OC, 50);

        assert!(t.pwm_output_high(0), "CNT 0 < OC 50");
        t.tick_elapsed(50);
        assert!(!t.pwm_output_high(0), "CNT 50 is not below OC 50");
    }

    #[test]
    fn the_interrupt_follows_ien_and_the_flag_is_write_one_to_clear() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_TOP, 9);

        t.tick_elapsed(10);
        assert!(!t.tick_elapsed(0).irq, "IEN clear: no interrupt");
        assert_eq!(t.read_word(OFF_IF) & IF_OF, IF_OF);

        t.write_word(OFF_IEN, IF_OF);
        assert!(t.tick_elapsed(0).irq);

        t.write_word(OFF_IF, IF_OF);
        assert_eq!(t.read_word(OFF_IF) & IF_OF, 0);
        assert!(!t.tick_elapsed(0).irq);
    }

    #[test]
    fn disabling_stops_and_zeroes_the_counter() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_TOP, 0xFFFF);
        t.tick_elapsed(100);
        assert_eq!(t.read_word(OFF_CNT), 100);

        t.write_word(OFF_EN, 0);
        assert_eq!(t.read_word(OFF_CNT), 0);
        assert_eq!(t.read_word(OFF_STATUS) & STATUS_RUNNING, 0);
    }

    #[test]
    fn cc_channels_are_addressed_as_a_group_not_flat_words() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_CC + 2 * CC_STRIDE + CC_OC, 0x1234);
        assert_eq!(t.read_word(OFF_CC + 2 * CC_STRIDE + CC_OC), 0x1234);
        assert_eq!(t.read_word(OFF_CC + CC_OC), 0, "CC0 is a different channel");
    }

    #[test]
    fn input_capture_registers_read_zero_rather_than_a_stale_sample() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_CC + CC_CFG, 1); // INPUTCAPTURE
        t.tick_elapsed(100);
        assert_eq!(t.read_word(OFF_CC + CC_ICF), 0);
        assert_eq!(t.read_word(OFF_CC + CC_ICOF), 0);
    }
}
