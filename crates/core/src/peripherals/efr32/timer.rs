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
use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};
use std::cell::Cell;

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
    /// Latched interrupt flags. `Cell` because scheduler mode latches them from
    /// the `&self` lazy advance ([`Efr32s2Timer::advance_to`]); on the legacy
    /// walk only `tick_elapsed`/`write_word` reach it.
    iflag: Cell<u32>,
    ien: u32,
    top: u32,
    topb: u32,
    /// The counter. `Cell` for the same reason as `iflag` — a `&self` MMIO read
    /// advances it to the bus-published "now" before answering.
    cnt: Cell<u32>,
    lock: u32,
    cc_cfg: [u32; CC_COUNT],
    cc_ctrl: [u32; CC_COUNT],
    cc_oc: [u32; CC_COUNT],
    cc_ocb: [u32; CC_COUNT],
    dt: [u32; 8],

    /// Core clocks not yet turned into timer clocks, so a peripheral clock
    /// that is not a divisor of the core clock loses no fraction.
    prescale_residue: Cell<u64>,
    /// Timer clocks not yet turned into counter steps, so a prescaler larger
    /// than the tick interval does not lose the remainder.
    presc_residue: Cell<u64>,

    /// Bus-published cycle clock, attached by `SystemBus::add_peripheral`.
    /// `Some` selects scheduler mode; `None` keeps the legacy per-cycle walk
    /// (feature off, or a hand-built test bus that bypasses registration).
    clock: Option<CycleClock>,
    /// Lazy-path anchor: the absolute published cycle the counter was last
    /// advanced to. Owned exclusively by [`Efr32s2Timer::advance_to`]; the
    /// legacy walk never touches it.
    anchor: Cell<u64>,
    /// Arming token (cancellation contract layer 3): bumped whenever the wake
    /// this model needs MOVES, so an event scheduled under the old
    /// configuration dies on arrival instead of racing the fresh chain.
    arm_seq: u32,
    /// What the in-flight event covers, or `None` for "nothing armed".
    /// Compared before every re-arm so the writes that change nothing about
    /// when this timer next needs the CPU — a `CNT` poll's read-modify-write,
    /// an `IF` clear that leaves the same compare pending — do not each leave
    /// another entry resident on the heap. Same residency discipline
    /// `esp32c3::bt` documents, and the reason a polling loop cannot trip
    /// `MAX_LIVE_EVENTS_PER_PERIPHERAL`.
    armed_wake: Option<Wake>,
}

/// What the timer's in-flight event is for.
///
/// ⚠️ Two cases, and they must NOT both be spelled as an absolute deadline. A
/// held IRQ level re-pends on every tick, so "the next wake" for it is
/// `now + 1` — a cycle that MOVES with every MMIO write. Keying the
/// idempotence check on that would make every write during an unacked timer
/// interrupt look like a changed requirement and arm another event, and the
/// scheduler's dedup key includes the deadline, so those do not collapse: one
/// resident entry per write until each fires. [`Self::Level`] is the same
/// requirement expressed as a state rather than a cycle, so it compares equal
/// to itself and the chain stays a singleton.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wake {
    /// Re-assert the held IRQ level on the next tick, and keep doing so until
    /// firmware clears the flag.
    Level,
    /// A one-off wake at this ABSOLUTE cycle: a flag latch or a PWM wire edge.
    At(u64),
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
            iflag: Cell::new(0),
            ien: 0,
            top: TOP_RESET,
            // `_TIMER_TOPB_RESETVALUE` is 0, not TOP's 0xFFFF. The die cannot
            // arbitrate — it reads 0 here either way while EN is clear — so
            // the header is the source, and the header says 0.
            topb: 0,
            cnt: Cell::new(0),
            lock: 0,
            cc_cfg: [0; CC_COUNT],
            cc_ctrl: [0; CC_COUNT],
            cc_oc: [0; CC_COUNT],
            cc_ocb: [0; CC_COUNT],
            dt: [0; 8],
            prescale_residue: Cell::new(0),
            presc_residue: Cell::new(0),
            clock: None,
            anchor: Cell::new(0),
            arm_seq: 0,
            armed_wake: None,
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
        ch < CC_COUNT && self.cc_mode(ch) == CC_MODE_PWM && self.cnt.get() < self.cc_oc[ch]
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
            OFF_IF => self.iflag.get(),
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
            OFF_CNT => self.cnt.get(),
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
                    self.cnt.set(0);
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
            // ⚠️ `IF` is NOT write-1-to-clear on this die. MEASURED on
            // BRD2709A: with the timer frozen (`CFG.DEBUGRUN = 0`, core
            // halted, so nothing can re-set a flag between the write and the
            // read) and `IF` reading `0x10`, a direct `IF = 0xFFFFFFFF` left
            // it at `0x10`; the `+0x2000` CLR alias then took it to `0`.
            //
            // Series 2 dropped the `IFC` register and replaced it with the
            // alias window, so a flag is cleared ONLY through an alias. The
            // bus hands us the computed final image and flags it — see
            // `bus::is_alias_absolute_write`. A direct store is dropped, which
            // is what the silicon does with it.
            OFF_IF => {
                if crate::bus::is_alias_absolute_write() {
                    self.iflag.set(value);
                }
            }
            OFF_IEN => self.ien = value,
            OFF_TOP => self.top = self.mask(value),
            OFF_TOPB => self.topb = self.mask(value),
            OFF_CNT => {
                self.cnt.set(self.mask(value));
            }
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

    /// Steps from a counter value `from` (which MUST be `<= TOP`) until the
    /// counter next latches channel `ch`'s compare flag.
    ///
    /// ⚠️ **MEASURED ON THE DIE.** There is exactly ONE compare rule, and it is
    /// not "the counter arrives at `OC`":
    ///
    /// > Every counter clock SAMPLES the value `CNT` holds *before* the clock.
    /// > If that value equals `OC` and the channel is in a compare mode,
    /// > `IF.CCn` latches — and the same clock moves the counter to `CNT + 1`
    /// > (or wraps it to 0).
    ///
    /// So the flag becomes visible with `CNT` reading `OC + 1`, never `OC`.
    ///
    /// **The measurement.** BRD2709A, TIMER0 over SWD, `CFG.DEBUGRUN = 0` so
    /// the counter FREEZES the instant the debugger halts — which makes
    /// `(CNT, IF)` an atomic hardware snapshot and removes the SWD round trip
    /// from the question entirely. Each trial writes `CNT`, clears `IF`
    /// through the `+0x2000` alias, resumes into a parked spin loop for
    /// 14–18 counter ticks, halts, and reads both registers. 264 trials of the
    /// first configuration, **zero mixed verdicts** — every landed `CNT` value
    /// gave a unanimous answer:
    ///
    /// | `TOP`    | `OC`     | last `CNT` with `IF.CC0` clear | first with it set |
    /// |----------|----------|--------------------------------|-------------------|
    /// | `0xFFFF` | `0x8000` | `0x8000` (6/6)                 | `0x8001` (8/8)    |
    /// | `0xFFFF` | `0x1234` | `0x1234`                       | `0x1235`          |
    /// | `0x00FF` | `0x0080` | `0x0080` (4/4)                 | `0x0081` (4/4)    |
    /// | `0x00FF` | `0x00FF` | `0x00FF` (3/3)                 | `0x0000`, wrapped |
    ///
    /// The last row is the same rule at the wrap: the clock that samples `TOP`
    /// latches and carries the counter to 0.
    ///
    /// ⚠️ This rule SUBSUMES the "level match" that used to live beside it as a
    /// one-shot (`level_pending`). A counter written onto its own compare value
    /// and started latches on its first clock because that clock samples a
    /// resident `OC` — `steps_to_compare(oc, oc) == 1` falls straight out of
    /// the formula. It never needed a second mechanism; the one-shot existed
    /// only to patch an arrival rule that fired a tick early.
    ///
    /// ⚠️ It also REPLACES the old "an `OC` at or above `TOP` latches on the
    /// wrap" special case, which the die refutes — see the guard below.
    ///
    /// Corroboration from inside the model: `IF.OF` was ALREADY derived as
    /// `steps >= period - from`, i.e. "the clock that samples `TOP`". That is
    /// this rule, and it is exactly `steps_to_compare(from, top)`. The overflow
    /// flag and the compare flag now share one convention instead of
    /// disagreeing with each other.
    ///
    /// `None` when the channel can never latch.
    fn steps_to_compare(&self, from: u64, oc: u64) -> Option<u64> {
        let top = self.top as u64;
        // A compare value the counter can never HOLD never matches. MEASURED:
        // `TOP = 0xFF`, `OC = 0x180`, CC0 in OUTPUTCOMPARE, ~2.5 full periods
        // — `IF` read `0x00000001`, overflow alone, CC0 never set. The control
        // in the same run (`OC = 0x80`) read `0x11`.
        if oc > top {
            return None;
        }
        let period = top + 1;
        // The clock that SAMPLES `oc` is the one that latches, and it lands
        // the counter on `oc + 1`. `from` is sampled by the first step, so the
        // forward distance to `oc` is the number of steps BEFORE the latching
        // one.
        Some((oc + period - from) % period + 1)
    }

    /// Advance the counter by `steps`, raising overflow and compare flags on
    /// the way.
    ///
    /// ⚠️ **A lump of N steps must latch exactly what N single steps latch.**
    /// The scheduler path advances the counter in one closed-form jump between
    /// observations, so any place where a big advance and a per-cycle advance
    /// disagree is a place where the twin renders one thing under the walk and
    /// another under the scheduler. `steps_to_compare` is the shared
    /// derivation, and the brute-force gate in this module's tests is what
    /// keeps the two identical.
    ///
    /// `&self`: every field it mutates is a `Cell`, so a lazy `&self` MMIO read
    /// can bring the counter up to "now" before answering.
    fn advance(&self, steps: u64) {
        if steps == 0 {
            // No counter clock elapsed, so nothing is sampled.
            return;
        }
        let period = self.top as u64 + 1;
        let mut from = self.cnt.get() as u64;
        let mut steps = steps;

        // Degenerate entry state: firmware wrote a `CNT` above `TOP`. The
        // single-step path wraps on the very next step and latches every
        // channel the old predicate named, so reproduce exactly that one step
        // and continue from the normalised value.
        if from >= period {
            let landed = (from + 1) % period;
            // The step samples `from`, which is ABOVE `TOP` — so it equals no
            // legal `OC` and latches no compare. Only the wrap is real.
            // ⚠️ Derived from the sampling rule, not measured: firmware has to
            // write a `CNT` above `TOP` to reach here.
            self.iflag.set(self.iflag.get() | IF_OF);
            self.cnt.set(self.mask(landed as u32));
            steps -= 1;
            if steps == 0 {
                self.publish_cc();
                return;
            }
            from = landed;
        }

        let mut flags = self.iflag.get();
        // The overflow flag latches on the wrap step, and a multi-wrap advance
        // latches it once — it is a latch, not a count.
        if steps >= period - from {
            flags |= IF_OF;
        }
        for ch in 0..CC_COUNT {
            let mode = self.cc_mode(ch);
            if mode != CC_MODE_OUTPUTCOMPARE && mode != CC_MODE_PWM {
                continue;
            }
            if self
                .steps_to_compare(from, self.cc_oc[ch] as u64)
                .is_some_and(|s| steps >= s)
            {
                flags |= 1 << (IF_CC0_SHIFT + ch as u32);
            }
        }
        self.iflag.set(flags);
        self.cnt.set(self.mask(((from + steps) % period) as u32));
        self.publish_cc();
    }

    // ── Drive-mode plumbing (walk vs event scheduler) ──────────────────────

    crate::cycle_clock::scheduler_mode!();

    /// Test/differential knob: detach the cycle clock, pinning the model to the
    /// legacy walk (`uses_scheduler() == false`). This is how the
    /// walk-vs-scheduler differential builds its reference lane out of the very
    /// same bus assembly.
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
        self.armed_wake = None;
    }

    /// Counter steps the walk would take over `cycles` CORE clocks, consuming
    /// and re-publishing both residues. Identical arithmetic to
    /// `tick_elapsed`'s — extracted so the walk and the lazy replay share it.
    ///
    /// The accumulation is linear, which is what makes the closed form exact:
    /// `n` calls of one cycle each leave the residues at exactly the values one
    /// call of `n` cycles leaves them at.
    fn steps_for_cycles(&self, cycles: u64) -> u64 {
        // `cycles` counts CORE clocks. Convert to timer clocks first, at the
        // ratio between the two — otherwise a chip whose peripheral clock is a
        // quarter of its core clock counts four times too fast. The remainder
        // carries in core clocks so no fraction is lost.
        let timer_clocks = self.prescale_residue.get() + cycles * self.timer_hz();
        let per_timer_clock = self.cpu_hz.max(1);
        let ticks = timer_clocks / per_timer_clock;
        self.prescale_residue.set(timer_clocks % per_timer_clock);

        let divisor = self.prescale_divisor();
        let total = self.presc_residue.get() + ticks;
        self.presc_residue.set(total % divisor);
        total / divisor
    }

    /// The inverse of [`Self::steps_for_cycles`]: the smallest number of CORE
    /// clocks after which the counter will have advanced `n >= 1` steps.
    /// `None` when that lands beyond a `u64` cycle count, i.e. never in any
    /// run.
    ///
    /// Read-only — it must not consume the residues, because it answers "when
    /// should I be woken", not "how far have I come".
    fn cycles_for_steps(&self, n: u64) -> Option<u64> {
        let divisor = self.prescale_divisor() as u128;
        // Smallest tick count T with `(presc_residue + T) / divisor >= n`.
        // Saturating: a `CFG` rewrite can leave the residue at or above the new
        // divisor, in which case the step is already owed and T is 0.
        let ticks_needed = (n as u128 * divisor).saturating_sub(self.presc_residue.get() as u128);
        // Smallest cycle count e with
        // `(prescale_residue + e * timer_hz) / cpu_hz >= ticks_needed`.
        let need = ticks_needed * self.cpu_hz.max(1) as u128;
        let have = self.prescale_residue.get() as u128;
        let hz = self.timer_hz() as u128;
        let e = need.saturating_sub(have).div_ceil(hz).max(1);
        u64::try_from(e).ok()
    }

    /// Steps until the counter latches a flag that `IEN` is actually watching —
    /// the wake the walk would first pend the NVIC line on. `None` when nothing
    /// unmasked can latch (the counter is halted, or every enabled source is
    /// masked), in which case the chain is allowed to die and only an MMIO
    /// write can revive it.
    fn steps_to_next_enabled_flag(&self) -> Option<u64> {
        if self.en & EN_EN == 0 || !self.status_running || self.ien == 0 {
            return None;
        }
        let from = self.cnt.get() as u64;
        if from > self.top as u64 {
            // Degenerate `CNT > TOP`: the next single step wraps and latches.
            return Some(1);
        }
        let period = self.top as u64 + 1;
        let mut best: Option<u64> = None;
        if self.ien & IF_OF != 0 {
            best = Some(period - from);
        }
        for ch in 0..CC_COUNT {
            if self.ien & (1 << (IF_CC0_SHIFT + ch as u32)) == 0 {
                continue;
            }
            let mode = self.cc_mode(ch);
            if mode != CC_MODE_OUTPUTCOMPARE && mode != CC_MODE_PWM {
                continue;
            }
            let Some(s) = self.steps_to_compare(from, self.cc_oc[ch] as u64) else {
                continue;
            };
            best = Some(best.map_or(s, |b: u64| b.min(s)));
        }
        best
    }

    /// Steps until a PWM output changes level.
    ///
    /// ⚠️ This is not an interrupt concern, it is a WIRE concern, and it is why
    /// a scheduler-driven PWM timer still needs events when no interrupt is
    /// enabled at all. Under the walk the pad level is republished on every
    /// tick, so an edge lands on its exact cycle for free. A lazily-advanced
    /// counter only republishes when something observes it — and nothing does:
    /// a logic probe or a board view reads the PAD, through the GPIO model,
    /// which never calls back into this timer. Without an event at each edge
    /// the waveform would simply stop moving between MMIO accesses.
    ///
    /// `None` when no channel can produce an edge — no wire cell, halted
    /// counter, no PWM channel, or a duty pinned at 0% / 100%.
    fn steps_to_next_pwm_edge(&self) -> Option<u64> {
        if self.lines.is_none() || self.en & EN_EN == 0 || !self.status_running {
            return None;
        }
        let from = self.cnt.get() as u64;
        if from > self.top as u64 {
            return Some(1);
        }
        let top = self.top as u64;
        let period = top + 1;
        let mut best: Option<u64> = None;
        for ch in 0..CC_COUNT {
            if self.cc_mode(ch) != CC_MODE_PWM {
                continue;
            }
            let oc = self.cc_oc[ch] as u64;
            // The output is high while `CNT < OC`.
            let s = if from < oc {
                // Falling edge when the counter first reaches OC. An OC above
                // TOP is never reached: the duty is 100% and there is no edge.
                if oc > top {
                    continue;
                }
                oc - from
            } else {
                // Rising edge at the wrap, where the counter lands on 0. An OC
                // of 0 is 0% duty: the output never rises.
                if oc == 0 {
                    continue;
                }
                period - from
            };
            best = Some(best.map_or(s, |b: u64| b.min(s)));
        }
        best
    }

    /// What this timer next needs the CPU for, from the just-synced state: a
    /// held IRQ level re-asserts on the very next tick; otherwise it is the
    /// earlier of the next unmasked flag latch and the next PWM wire edge.
    /// `None` = nothing armed, let the chain die.
    ///
    /// `anchor` is the cycle the state was just synced to, so an `At` is
    /// absolute and stays comparable across later calls.
    fn next_wake(&self) -> Option<Wake> {
        if self.iflag.get() & self.ien != 0 {
            // The walk re-pends on every tick while the level is held.
            return Some(Wake::Level);
        }
        let steps = match (
            self.steps_to_next_enabled_flag(),
            self.steps_to_next_pwm_edge(),
        ) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => return None,
        };
        self.cycles_for_steps(steps)
            .map(|d| Wake::At(self.anchor.get() + d))
    }

    /// CORE clocks from the just-synced state (`anchor`) until `wake`.
    fn delay_to(&self, wake: Wake) -> u64 {
        match wake {
            Wake::Level => 1,
            Wake::At(deadline) => deadline.saturating_sub(self.anchor.get()).max(1),
        }
    }

    /// Lazy advance to the absolute published cycle `now` — callable from
    /// `&self` because every field it moves is a `Cell`. Idempotent, and a
    /// `now` at or behind the anchor is ignored.
    ///
    /// The advanced window always has constant EN/CFG/TOP/CC settings: every
    /// MMIO write syncs first through the bus `sync_to` choke, so a settings
    /// change can never straddle a window.
    fn advance_to(&self, now: u64) {
        let anchor = self.anchor.get();
        if now <= anchor {
            return;
        }
        self.anchor.set(now);
        if self.en & EN_EN == 0 || !self.status_running {
            // Halted: the window elapses with no counting and no residue
            // movement, exactly the walk's early return.
            return;
        }
        self.advance(self.steps_for_cycles(now - anchor));
    }

    /// Pull "now" from the bus-published clock and advance. No-op in legacy
    /// mode, where the walk advances the counter instead.
    fn sync_from_clock(&self) {
        if self.scheduler_mode() {
            if let Some(clock) = &self.clock {
                self.advance_to(clock.now());
            }
        }
    }

    /// One legacy walk tick: advance by `cycles` core clocks, then report the
    /// held IRQ level. Shared by the walk and the hardware-oracle forced walk.
    fn walk_tick(&mut self, cycles: u64) -> PeripheralTickResult {
        let mut result = PeripheralTickResult::default();
        if self.en & EN_EN != 0 && self.status_running {
            self.advance(self.steps_for_cycles(cycles));
        }
        if self.iflag.get() & self.ien != 0 {
            result.irq = true;
        }
        result
    }
}

impl Peripheral for Efr32s2Timer {
    fn read(&self, offset: u64) -> SimResult<u8> {
        // Scheduler mode: bring the lazily-advanced counter up to the
        // bus-published "now" first, so a polled `CNT`/`IF` read observes fresh
        // time. Exact at tick interval 1; at a wider interval it trails the
        // true read cycle by less than one interval — the same bound the
        // write-path `sync_to` ships.
        self.sync_from_clock();
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
        self.sync_from_clock();
        Ok(self.read_word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    /// Everything the walk does here is event-expressible, and every piece of
    /// it is reproduced on the event path:
    ///
    /// * the prescaled up-count is a LINEAR function of elapsed core clocks, so
    ///   the lazy `advance_to` replays any window in closed form;
    /// * `IF.OF` / `IF.CCn` latch at cycles this model computes exactly
    ///   (`steps_to_next_enabled_flag`), and the held NVIC level re-pends at
    ///   delay 1 exactly as the walk re-pends per tick;
    /// * the PWM output level reaches its pad at each edge through a scheduled
    ///   wake (`steps_to_next_pwm_edge`) rather than a per-tick republish.
    ///
    /// The unmodelled parts (`CLKSEL`, `TOPB` buffering, `LOCK`, dead time,
    /// input capture, down/up-down counting) are inert register bits the walk
    /// ignores identically, so no configuration needs a dynamic fallback. In
    /// legacy mode (feature off / no clock) the walk does the real counting and
    /// the conservative `true` stands.
    fn needs_legacy_walk(&self) -> bool {
        !self.scheduler_mode()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        // Anchor at the clock's current value so cycles that elapsed before the
        // attach (normally zero — attach happens at bus assembly) are not
        // retroactively replayed into the counter.
        self.anchor.set(clock.now());
        self.clock = Some(clock);
    }

    fn sync_to(&mut self, now_cycle: u64) {
        if self.scheduler_mode() {
            self.advance_to(now_cycle);
        }
    }

    /// Re-arm the wake chain after an MMIO write.
    ///
    /// IDEMPOTENT by absolute deadline. This runs after EVERY write to the
    /// block, and most writes do not move when the timer next needs the CPU —
    /// an `IF` clear that leaves another compare pending, a `CNT` poll's
    /// read-modify-write. Arming for those is what puts entries on the heap
    /// that nothing can collapse (the scheduler's dedup key includes the
    /// deadline, so two arms at DIFFERENT deadlines both stay resident until
    /// they fire). Comparing the absolute deadline first is what keeps a
    /// polling loop from tripping `MAX_LIVE_EVENTS_PER_PERIPHERAL`.
    ///
    /// The `- 1`: the bus turns the returned delay into the absolute deadline
    /// `current_cycle + 1 + delay`, and the wake is wanted `d` cycles after the
    /// just-synced state, i.e. at `current_cycle + d`.
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.scheduler_mode() {
            return Vec::new();
        }
        let want = self.next_wake();
        if want == self.armed_wake {
            // An event covering exactly this is already in flight.
            return Vec::new();
        }
        // The requirement moved, so whatever is queued is now wrong: bump the
        // token so it is rejected on arrival. There is no scheduler-side
        // cancel, by design.
        self.armed_wake = want;
        self.arm_seq = self.arm_seq.wrapping_add(1);
        match want {
            Some(wake) => vec![(self.delay_to(wake) - 1, self.arm_seq)],
            None => Vec::new(),
        }
    }

    fn on_event(
        &mut self,
        event_token: u32,
        sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() || event_token != self.arm_seq {
            // Stale chain (re-armed since this event was scheduled): die.
            return crate::sched::EventResult::default();
        }
        // Materialise the latch / wire edge this event was scheduled for, at
        // the exact cycle the walk's tick would have produced it.
        self.advance_to(sched.now());
        let irq = self.iflag.get() & self.ien != 0;
        // `Machine::drain_scheduler_events` re-arms `reschedule_delay` under
        // the SAME token, so record what that in-flight event now covers —
        // otherwise the next MMIO write compares against a stale `armed_wake`,
        // concludes the work is uncovered, and arms a duplicate.
        let want = self.next_wake();
        self.armed_wake = want;
        crate::sched::EventResult {
            raise_own_irq: irq,
            reschedule_delay: want.map(|w| self.delay_to(w)),
            ..Default::default()
        }
    }

    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        // A scheduler-mode instance is walk-skipped and the event chain owns
        // the counter; the guard keeps a stray direct call from double-counting
        // the lazily-anchored state.
        if self.scheduler_mode() {
            return PeripheralTickResult::default();
        }
        self.walk_tick(cycles)
    }

    /// The bare-CPU hardware oracle freezes the CPU and deliberately asks for
    /// the pre-scheduler one-tick advance, so the `scheduler_mode()` no-op in
    /// [`Self::tick_elapsed`] must NOT apply here.
    fn tick_elapsed_forced(&mut self, cycles: u64) -> PeripheralTickResult {
        self.walk_tick(cycles)
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

    /// Acknowledge `IF` bits the way firmware does on Series 2 — through the
    /// `+0x2000` CLR alias, which is the ONLY thing that clears a flag on this
    /// die. These unit tests drive the peripheral directly, so they have to
    /// reproduce what the bus does for an alias write: hand the model the
    /// computed final image, flagged as one.
    fn ack_if(t: &mut Efr32s2Timer, mask: u32) {
        let cleared = t.read_word(OFF_IF) & !mask;
        crate::bus::with_alias_absolute_write(|| t.write_word(OFF_IF, cleared));
    }

    #[test]
    fn a_compare_channel_flags_when_the_counter_passes_it() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_TOP, 100);
        t.write_word(OFF_CC + CC_CFG, CC_MODE_OUTPUTCOMPARE);
        t.write_word(OFF_CC + CC_OC, 40);

        // ⚠️ The flag appears with `CNT` reading OC + 1, not OC — the counter
        // clock samples the OUTGOING value. Measured; see `steps_to_compare`.
        t.tick_elapsed(40);
        assert_eq!(t.read_word(OFF_CNT), 40);
        assert_eq!(
            t.read_word(OFF_IF) & (1 << IF_CC0_SHIFT),
            0,
            "the clock that landed CNT on 40 sampled 39"
        );
        t.tick_elapsed(1);
        assert_eq!(t.read_word(OFF_CNT), 41);
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

    // ── The compare is a LEVEL match: measured on a BRD2709A die ──────────
    //
    // TIMER0 @ 0x40048000 over SWD, `CFG = 0x0FFC0040` (PRESC = 1023,
    // DEBUGRUN), `TOP = 0xFFFF`, CC0 in OUTPUTCOMPARE with `CC0_OC = 0x8000`.
    // One counter tick is ~53 µs, one full period ~3.5 s. `IF` was cleared and
    // read back `0x00000000` after setup and before `CMD.START` in every case.
    //
    // | case                       | start CNT | die `IF` @250 ms |
    // |----------------------------|-----------|------------------|
    // | B, the counter ARRIVES     | 0x7FFF    | 0x10  (CC0)      |
    // | A, the counter is PLACED   | 0x8000    | 0x10  (CC0)      |
    // | A, after the wrap (3.75 s) | 0x8000    | 0x11  (CC0 + OF) |
    //
    // Case A is the one the arrival rule cannot produce: the counter climbs
    // 0x8000 → 0x9268 and never returns to 0x8000 inside that window, so the
    // flag is not a second pass. See `Efr32s2Timer::level_pending`.

    /// The die's register sequence, in the die's ORDER.
    ///
    /// ⚠️ Series-2 discipline, measured: `CFG` is a config register and must be
    /// written while `EN = 0`; `TOP`, `CNT` and `CC_OC` are runtime registers
    /// and must be written AFTER `EN = 1` or they read back 0. Get this wrong
    /// and the test proves something about a timer it never configured.
    fn die_timer0(start_cnt: u32) -> Efr32s2Timer {
        let mut t = Efr32s2Timer::new(T0_BITS);
        t.write_word(OFF_EN, 0);
        t.write_word(OFF_CFG, 0x0FFC_0040); // PRESC = 1023, DEBUGRUN
        t.write_word(OFF_CC + CC_CFG, CC_MODE_OUTPUTCOMPARE);
        t.write_word(OFF_EN, EN_EN);
        t.write_word(OFF_TOP, 0x0000_FFFF);
        t.write_word(OFF_CC + CC_OC, 0x0000_8000);
        t.write_word(OFF_CNT, start_cnt);
        t.write_word(OFF_IF, 0xFFFF_FFFF);
        t
    }

    /// `PRESC = 1023`, and this instance's timer clock is its core clock, so
    /// one counter tick is 1024 cycles.
    fn die_ticks(t: &mut Efr32s2Timer, ticks: u64) {
        t.tick_elapsed(ticks * 1024);
    }

    /// Case B: the counter ARRIVES at the compare value. Both the arrival rule
    /// and the level match produce this one — it is the control.
    #[test]
    fn a_counter_that_arrives_at_its_compare_value_flags() {
        let mut t = die_timer0(0x7FFF);
        assert_eq!(t.read_word(OFF_IF), 0, "the die read 0 before START");
        t.write_word(OFF_CMD, CMD_START);

        // 4712 ticks is where the die's CNT stood at the 250 ms read.
        die_ticks(&mut t, 4712);
        assert_eq!(t.read_word(OFF_CNT), 0x9267);
        assert_eq!(t.read_word(OFF_IF), 0x10, "CC0, no overflow yet");
    }

    /// ⚠️ Case A, the one the die and the model disagreed on: a counter WRITTEN
    /// equal to `OC` and started latches CC0 without ever arriving there. The
    /// compare is a level match on `CNT == OC` sampled by the counter clock,
    /// not an arrival edge.
    #[test]
    fn a_counter_started_on_its_compare_value_flags_without_arriving() {
        let mut t = die_timer0(0x8000);
        assert_eq!(
            t.read_word(OFF_IF),
            0,
            "the die read 0 here too: the match is gated by the counter being \
             CLOCKED, so writing CNT == OC while stopped sets nothing"
        );
        t.write_word(OFF_CMD, CMD_START);

        die_ticks(&mut t, 4712); // where the die's CNT stood at 250 ms
        assert_eq!(
            t.read_word(OFF_CNT),
            0x9268,
            "the counter climbed away from 0x8000 and cannot have re-reached \
             it inside one 3.5 s period"
        );
        assert_eq!(t.read_word(OFF_IF), 0x10, "the die reads CC0 set");
    }

    /// The third die row: the same pre-loaded run, carried past the wrap.
    #[test]
    fn the_pre_loaded_run_adds_the_overflow_after_the_wrap() {
        let mut t = die_timer0(0x8000);
        t.write_word(OFF_CMD, CMD_START);
        die_ticks(&mut t, 70_000); // ~3.75 s, past the 3.5 s period
        assert_eq!(t.read_word(OFF_IF), 0x11, "CC0 + OF, as the die reads");
    }

    /// ⚠️ The level match is a ONE-SHOT on the value that was placed, not a
    /// standing "compare the value you start from" rule.
    ///
    /// Under the walk every advance starts from the value the previous advance
    /// landed on. A standing rule therefore re-latches every compare on the
    /// very next tick, and firmware that clears `IF` in its handler takes two
    /// interrupts per match instead of one. This is the test that rejects the
    /// naive reading of the die result.
    #[test]
    fn a_compare_latches_once_per_period_even_when_the_handler_clears_it() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_TOP, 99);
        t.write_word(OFF_CC + CC_CFG, CC_MODE_OUTPUTCOMPARE);
        t.write_word(OFF_CC + CC_OC, 40);

        t.tick_elapsed(41);
        assert_eq!(t.read_word(OFF_CNT), 41);
        assert_eq!(t.read_word(OFF_IF) & (1 << IF_CC0_SHIFT), 1 << IF_CC0_SHIFT);

        // The handler acknowledges immediately after the match.
        ack_if(&mut t, 0xFFFF_FFFF);
        t.tick_elapsed(1);
        assert_eq!(
            t.read_word(OFF_IF) & (1 << IF_CC0_SHIFT),
            0,
            "the counter has left 40; re-latching here would double every \
             compare interrupt"
        );

        // ...and the next real match, a full period on, still lands.
        t.tick_elapsed(99);
        assert_eq!(t.read_word(OFF_CNT), 41);
        assert_eq!(t.read_word(OFF_IF) & (1 << IF_CC0_SHIFT), 1 << IF_CC0_SHIFT);
    }

    /// A `CNT` write that lands on a live compare is the same level match from
    /// the counter side, and a `CC_OC` write that lands on a live `CNT` is the
    /// same match from the channel side.
    #[test]
    fn placing_either_side_of_the_match_under_a_running_counter_flags() {
        for from_the_channel_side in [false, true] {
            let mut t = running(T0_BITS);
            t.write_word(OFF_TOP, 99);
            t.write_word(OFF_CC + CC_CFG, CC_MODE_OUTPUTCOMPARE);
            t.write_word(OFF_CC + CC_OC, 40);
            t.tick_elapsed(10);
            t.write_word(OFF_IF, 0xFFFF_FFFF);

            if from_the_channel_side {
                t.write_word(OFF_CC + CC_OC, 10); // OC moved onto CNT
            } else {
                t.write_word(OFF_CNT, 40); // CNT moved onto OC
            }
            assert_eq!(t.read_word(OFF_IF), 0, "nothing until the next clock");

            t.tick_elapsed(1);
            assert_eq!(
                t.read_word(OFF_IF) & (1 << IF_CC0_SHIFT),
                1 << IF_CC0_SHIFT,
                "from_the_channel_side={from_the_channel_side}"
            );
        }
    }

    /// The scheduler lane has to ARM for the level match, or the pre-loaded
    /// case's interrupt arrives a whole period late — rendering identically
    /// under the walk and not under the scheduler, which is the exact failure
    /// mode `efr32mg26_walk_differential` exists to catch.
    #[test]
    fn a_pending_level_match_arms_the_very_next_step() {
        let mut t = Efr32s2Timer::new(T0_BITS);
        t.attach_cpu_hz(78_000_000);
        t.set_peripheral_hz(78_000_000);
        t.write_word(OFF_EN, EN_EN);
        t.write_word(OFF_TOP, 99);
        t.write_word(OFF_CC + CC_CFG, CC_MODE_OUTPUTCOMPARE);
        t.write_word(OFF_CC + CC_OC, 30);
        t.write_word(OFF_IEN, 1 << IF_CC0_SHIFT);
        t.write_word(OFF_CNT, 30);
        t.write_word(OFF_CMD, CMD_START);

        assert_eq!(
            t.next_wake(),
            Some(Wake::At(1)),
            "the counter starts ON its compare value: the match is one step \
             away, not a period away"
        );

        // ⚠️ There is no "unarmed twin" to compare against any more. Under the
        // single sampling rule a counter resident on its compare value is ONE
        // step from latching, always — it is not a state the model can be in
        // with or without an armed one-shot.
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

    /// ⚠️ The flag is NOT write-1-to-clear, whatever every Series-0/1 habit
    /// suggests. MEASURED on BRD2709A with the counter frozen: a direct
    /// `IF = 0xFFFFFFFF` left `IF` at `0x10`, and only the `+0x2000` alias
    /// cleared it. Series 2 dropped `IFC` and put the clear in the alias
    /// window, so a direct store is silently dropped — model it that way, or
    /// firmware that uses the alias (all of emlib does) never clears anything.
    #[test]
    fn the_interrupt_follows_ien_and_the_flag_clears_only_through_the_alias() {
        let mut t = running(T0_BITS);
        t.write_word(OFF_TOP, 9);

        t.tick_elapsed(10);
        assert!(!t.tick_elapsed(0).irq, "IEN clear: no interrupt");
        assert_eq!(t.read_word(OFF_IF) & IF_OF, IF_OF);

        t.write_word(OFF_IEN, IF_OF);
        assert!(t.tick_elapsed(0).irq);

        // A direct store does nothing at all, exactly as on the die.
        t.write_word(OFF_IF, IF_OF);
        assert_eq!(
            t.read_word(OFF_IF) & IF_OF,
            IF_OF,
            "a direct `IF` write is dropped on Series 2"
        );
        assert!(t.tick_elapsed(0).irq, "so the interrupt is still asserted");

        // Through the alias it clears.
        ack_if(&mut t, IF_OF);
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

    // ── Walk ≡ scheduler: the closed forms, brute-forced ──────────────────
    //
    // The scheduler path replaces a per-cycle walk with two closed forms — a
    // lumped `advance` and an inverse that says when to be woken. Both are
    // pinned here against the thing they replace, over a parameter grid rather
    // than at a point, because "a model that arms an event at the wrong cycle
    // delivers an interrupt late, which renders identically right up to the run
    // where it does not".

    /// ⚠️ **A lump of N steps must latch exactly what N single steps latch.**
    ///
    /// This is the property the whole lazy-counter design rests on: the
    /// scheduler advances the counter in one jump between observations, so any
    /// disagreement here is a flag the twin raises under one drive mode and not
    /// the other. Brute-forced over every start value, every compare value and
    /// every advance length for a small TOP, in both compare modes.
    ///
    /// It is also the gate that caught the real defect: the previous wrapped
    /// predicate latched every channel with `OC >= CNT` on ANY wrapping
    /// advance, so a lump flagged `OC == CNT` that single steps do not (the
    /// counter is leaving that value, not arriving at it).
    ///
    /// ⚠️ The grid INCLUDES every `from == oc`. That case used to need a
    /// separate one-shot to come out right; under the single sampling rule it
    /// is just `steps_to_compare(oc, oc) == 1`, and the lump/single-step
    /// agreement on it is what proves the one-shot is not missed.
    #[test]
    fn a_lumped_advance_latches_exactly_what_single_steps_latch() {
        const TOP: u32 = 7;
        for mode in [CC_MODE_OUTPUTCOMPARE, CC_MODE_PWM] {
            // OC deliberately runs past TOP: an OC the counter can never reach
            // is a real firmware state (a duty above the period).
            for oc in 0..=(TOP + 3) {
                for from in 0..=(TOP + 2) {
                    {
                        for steps in 1..=(3 * (TOP as u64 + 1)) {
                            let build = || {
                                let mut t = Efr32s2Timer::new(32);
                                t.write_word(OFF_EN, EN_EN);
                                t.write_word(OFF_CMD, CMD_START);
                                t.write_word(OFF_TOP, TOP);
                                t.write_word(OFF_CC + CC_CFG, mode);
                                t.write_word(OFF_CC + CC_OC, oc);
                                t.cnt.set(from);
                                t.iflag.set(0);
                                t
                            };
                            let lump = build();
                            lump.advance(steps);

                            let unit = build();
                            for _ in 0..steps {
                                unit.advance(1);
                            }

                            assert_eq!(
                                lump.iflag.get(),
                                unit.iflag.get(),
                                "flags diverge: mode={mode} oc={oc} from={from} \
                                 steps={steps} (lump={:#x} single={:#x})",
                                lump.iflag.get(),
                                unit.iflag.get()
                            );
                            assert_eq!(
                                lump.cnt.get(),
                                unit.cnt.get(),
                                "counter diverges: mode={mode} oc={oc} from={from} \
                                 steps={steps}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// `cycles_for_steps` is the inverse of `steps_for_cycles`, and it has to be
    /// EXACT in both directions: one cycle short and the interrupt is late, one
    /// cycle long and it is early. Brute-forced across prescalers and a clock
    /// ratio that is deliberately not an integer (19 MHz timer clock off a
    /// 78 MHz core is this part's real one), so the residue carry is exercised.
    #[test]
    fn the_wake_cycle_is_exactly_when_the_step_lands() {
        for presc in [0u32, 1, 7, 18, 999] {
            for (cpu, per) in [(78_000_000u64, 19_000_000u64), (48_000_000, 48_000_000)] {
                let mut t = Efr32s2Timer::new(32);
                t.attach_cpu_hz(cpu);
                t.set_peripheral_hz(per);
                t.write_word(OFF_EN, EN_EN);
                t.write_word(OFF_CMD, CMD_START);
                t.write_word(OFF_TOP, 0xFFFF_FFFF);
                t.write_word(OFF_CFG, presc << CFG_PRESC_SHIFT);

                for n in 1..=4u64 {
                    // Read-only query on a pristine residue state.
                    let e = t.cycles_for_steps(n).expect("a reachable wake");

                    // One cycle short must NOT have produced `n` steps...
                    let mut short = Efr32s2Timer::new(32);
                    short.prescale_residue.set(t.prescale_residue.get());
                    short.presc_residue.set(t.presc_residue.get());
                    short.cpu_hz = cpu;
                    short.peripheral_hz = Some(per);
                    short.cfg = presc << CFG_PRESC_SHIFT;
                    assert!(
                        short.steps_for_cycles(e - 1) < n,
                        "presc={presc} cpu={cpu} per={per} n={n}: {} cycles already \
                         reached {n} steps, so the wake at {e} is LATE",
                        e - 1
                    );

                    // ...and exactly `e` cycles must have.
                    let mut exact = Efr32s2Timer::new(32);
                    exact.prescale_residue.set(t.prescale_residue.get());
                    exact.presc_residue.set(t.presc_residue.get());
                    exact.cpu_hz = cpu;
                    exact.peripheral_hz = Some(per);
                    exact.cfg = presc << CFG_PRESC_SHIFT;
                    assert!(
                        exact.steps_for_cycles(e) >= n,
                        "presc={presc} cpu={cpu} per={per} n={n}: {e} cycles did not \
                         reach {n} steps, so the wake is EARLY"
                    );

                    // Advance the reference by one cycle so the next round
                    // starts from a different residue phase.
                    t.tick_elapsed(1);
                }
            }
        }
    }

    /// The lazy replay must land on exactly the state the per-cycle walk lands
    /// on — same counter, same flags, same residues — for an arbitrary window.
    /// This is the in-module half of the executing differential.
    #[test]
    fn the_lazy_replay_equals_the_per_cycle_walk() {
        for presc in [0u32, 3, 77] {
            for top in [9u32, 255, 1000] {
                let build = || {
                    let mut t = Efr32s2Timer::new(32);
                    t.attach_cpu_hz(78_000_000);
                    t.set_peripheral_hz(19_000_000);
                    t.write_word(OFF_EN, EN_EN);
                    t.write_word(OFF_CMD, CMD_START);
                    t.write_word(OFF_TOP, top);
                    t.write_word(OFF_CFG, presc << CFG_PRESC_SHIFT);
                    t.write_word(OFF_CC + CC_CFG, CC_MODE_PWM);
                    t.write_word(OFF_CC + CC_OC, top / 3);
                    t.write_word(OFF_CC + CC_STRIDE + CC_CFG, CC_MODE_OUTPUTCOMPARE);
                    t.write_word(OFF_CC + CC_STRIDE + CC_OC, top / 2 + 1);
                    t
                };
                for window in [1u64, 2, 17, 313, 5000, 100_000] {
                    let mut walk = build();
                    for _ in 0..window {
                        walk.tick_elapsed(1);
                    }
                    let lazy = build();
                    lazy.advance_to(window);

                    assert_eq!(
                        (
                            lazy.cnt.get(),
                            lazy.iflag.get(),
                            lazy.prescale_residue.get(),
                            lazy.presc_residue.get()
                        ),
                        (
                            walk.cnt.get(),
                            walk.iflag.get(),
                            walk.prescale_residue.get(),
                            walk.presc_residue.get()
                        ),
                        "presc={presc} top={top} window={window}: the lazy replay \
                         diverged from the per-cycle walk"
                    );
                }
            }
        }
    }

    /// A PWM channel's pad edges must be scheduled, not merely republished when
    /// something happens to touch the timer: nothing reads back INTO this model
    /// from the pad side, so an unscheduled edge is an edge that never happens.
    /// Both directions, and both duty extremes that legitimately have no edge.
    #[test]
    fn a_pwm_channel_asks_to_be_woken_at_each_wire_edge() {
        let mut t = Efr32s2Timer::new(32);
        // A wire cell is what makes the pad observable at all; without one
        // there is nothing to publish and no reason to schedule.
        assert_eq!(t.steps_to_next_pwm_edge(), None, "no wire cell yet");
        let _ = t.pad_lines_arc();

        t.write_word(OFF_EN, EN_EN);
        t.write_word(OFF_CMD, CMD_START);
        t.write_word(OFF_TOP, 99);
        t.write_word(OFF_CC + CC_CFG, CC_MODE_PWM);
        t.write_word(OFF_CC + CC_OC, 40);

        // CNT 0 < OC 40: high, and the falling edge is 40 steps away.
        assert!(t.pwm_output_high(0));
        assert_eq!(t.steps_to_next_pwm_edge(), Some(40));

        t.tick_elapsed_forced(40);
        assert!(!t.pwm_output_high(0), "CNT 40 is not below OC 40");
        // Low now; the rising edge is at the wrap, 60 steps on.
        assert_eq!(t.steps_to_next_pwm_edge(), Some(60));

        // 0% duty never rises, 100% duty never falls: no edge either way.
        t.write_word(OFF_CC + CC_OC, 0);
        assert_eq!(t.steps_to_next_pwm_edge(), None, "0% duty has no edge");
        t.write_word(OFF_CNT, 0);
        t.write_word(OFF_CC + CC_OC, 0xFFFF); // above TOP
        assert_eq!(t.steps_to_next_pwm_edge(), None, "100% duty has no edge");
    }

    /// The wake the arming path computes is the cycle the walk would first pend
    /// on — and a masked flag must NOT produce one, or every idle timer would
    /// hold an event chain open and the batcher would never widen.
    #[test]
    fn only_an_unmasked_source_arms_a_wake() {
        let mut t = Efr32s2Timer::new(32);
        t.attach_cpu_hz(78_000_000);
        t.set_peripheral_hz(78_000_000);
        t.write_word(OFF_EN, EN_EN);
        t.write_word(OFF_CMD, CMD_START);
        t.write_word(OFF_TOP, 99);
        assert_eq!(t.next_wake(), None, "IEN clear: nothing to wake for");

        t.write_word(OFF_IEN, IF_OF);
        assert_eq!(
            t.next_wake(),
            Some(Wake::At(100)),
            "the overflow is 100 counts, and one count is one cycle here"
        );

        // A compare nearer than the wrap wins.
        t.write_word(OFF_CC + CC_CFG, CC_MODE_OUTPUTCOMPARE);
        t.write_word(OFF_CC + CC_OC, 30);
        t.write_word(OFF_IEN, IF_OF | (1 << IF_CC0_SHIFT));
        assert_eq!(
            t.next_wake(),
            Some(Wake::At(31)),
            "the clock that SAMPLES 30 latches, and it is the 31st from 0"
        );

        // Once the level is held the walk re-pends every tick, so the chain
        // perpetuates at 1 until firmware clears the flag.
        t.tick_elapsed_forced(31);
        assert_eq!(
            t.next_wake(),
            Some(Wake::Level),
            "a held level re-pends every tick, and must be spelled as a STATE \
             rather than as a moving deadline"
        );
        ack_if(&mut t, 0xFFFF_FFFF);
        assert_eq!(
            t.next_wake(),
            Some(Wake::At(69)),
            "back to the wrap: CNT is 31, and the clock that samples TOP = 99 \
             is the 69th from there"
        );

        // A halted counter arms nothing at all.
        t.write_word(OFF_CMD, CMD_STOP);
        assert_eq!(t.next_wake(), None);
    }

    /// Legacy mode is the default for a hand-built model, and the differential
    /// knob must be able to put a scheduler-mode instance back on the walk —
    /// otherwise the reference lane silently becomes the candidate lane.
    #[test]
    fn the_drive_mode_follows_the_attached_clock() {
        let mut t = Efr32s2Timer::new(32);
        assert!(
            t.needs_legacy_walk() && !t.uses_scheduler(),
            "no clock: walk"
        );

        t.attach_cycle_clock(CycleClock::default());
        if cfg!(feature = "event-scheduler") {
            assert!(t.uses_scheduler() && !t.needs_legacy_walk());
            t.force_legacy_walk();
            assert!(t.needs_legacy_walk() && !t.uses_scheduler());
        } else {
            assert!(t.needs_legacy_walk() && !t.uses_scheduler());
        }
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
