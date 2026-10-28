// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! A channel OUTPUT as a waveform in CPU cycles — shared by the STM32F1
//! timer channel outputs here, and by the fork's classic-ESP32, ESP32-C3 and
//! ESP32-S3 LEDC models.
//!
//! Every ESP LEDC channel is the same machine (ESP32 TRM v5 §14.2, C3 TRM
//! §26.3, S3 TRM §31.3): a timer counts `0 .. 2^DUTY_RES - 1` at its source
//! clock divided by the Q8 fixed-point `CLK_DIV`, and the channel output goes
//! HIGH when the count reaches `HPOINT` and LOW at `LPOINT = HPOINT + DUTY`
//! (DUTY's integer part — the register's four fractional bits are not
//! modelled). `DUTY = 0` is low all period, `DUTY >= 2^DUTY_RES` high all
//! period (Arduino-ESP32's `ledcWrite(ch, max)` writes exactly `2^res` for
//! "full on"). A channel whose `SIG_OUT_EN` is clear drives `IDLE_LV`.
//!
//! The register decode differs per chip; the waveform does not, so it lives
//! here once. A chip model decodes each channel into a [`LedcDrive`] and hands
//! it to [`LedcPads::set_drive`]; this type publishes every edge on the
//! channel's wire ([`PadLines`]) at its EXACT cycle (`set_line_at`) and asks
//! the scheduler to wake it on each edge, so a pad routed to the channel (the
//! GPIO matrix on the ESPs, a conditional F1 timer-output route on the STM32s)
//! reads the live level and push capture sees every transition.
//!
//! Hold semantics (as the nRF52 PWM, B6): a channel keeps generating its last
//! committed duty until firmware changes it; a new duty on a RUNNING channel
//! takes effect at the next period boundary of its timer (the silicon latches
//! DUTY at overflow), so a program re-writing its duty faster than the period
//! still produces whole periods. A timer reconfiguration, a disabled output or
//! a paused/held counter applies at once.
//!
//! Not modelled: duty fading (`DUTY_NUM`/`DUTY_CYCLE`/`DUTY_SCALE` — the
//! output stays at the committed start duty), the fractional duty dither, and
//! overflow/fade interrupts beyond what each chip model already does.

use std::sync::Arc;

use crate::peripherals::pad_lines::PadLines;

/// One LEDC timer's time base as one of its channels sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LedcTimebase {
    /// Counter period in counts (`2^DUTY_RES`), >= 1.
    pub period: u64,
    /// CPU cycles per count is `num / den` (exact rational: the Q8 divider
    /// times `cpu_hz`, over 256 times the source clock).
    pub num: u128,
    pub den: u128,
    /// Cycle at which this timer's counter was (re)started at 0.
    pub origin: u64,
}

impl LedcTimebase {
    /// Build from the chip's registers. `None` for a timer that cannot count
    /// (zero divider or zero source clock).
    pub fn new(
        duty_res: u32,
        clk_div_q8: u64,
        src_hz: u64,
        cpu_hz: u64,
        origin: u64,
    ) -> Option<Self> {
        if clk_div_q8 == 0 || src_hz == 0 || cpu_hz == 0 {
            return None;
        }
        Some(Self {
            period: 1u64 << duty_res.min(20),
            num: u128::from(clk_div_q8) * u128::from(cpu_hz),
            den: 256 * u128::from(src_hz),
            origin,
        })
    }

    /// Absolute count index at cycle `t` (`t >= origin`).
    fn count_at(&self, t: u64) -> u64 {
        (u128::from(t.saturating_sub(self.origin)) * self.den / self.num) as u64
    }

    /// First cycle of count index `n`.
    fn cycle_of(&self, n: u64) -> u64 {
        let c = (u128::from(n) * self.num).div_ceil(self.den);
        self.origin
            .saturating_add(c.min(u128::from(u64::MAX)) as u64)
    }
}

/// How a channel drives its output, decoded from the chip's registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LedcDrive {
    /// A constant level: output disabled (IDLE_LV), counter paused or held in
    /// reset, timer unconfigured.
    Level(bool),
    /// A running PWM.
    Pwm {
        timer: LedcTimebase,
        hpoint: u64,
        duty: u64,
    },
}

impl LedcDrive {
    /// Level at counter value `v` (`0 .. period`) of a running channel.
    fn level_at_count(period: u64, hpoint: u64, duty: u64, v: u64) -> bool {
        if duty == 0 {
            return false;
        }
        if duty >= period {
            return true;
        }
        (v + period - hpoint % period) % period < duty
    }

    /// Output level at cycle `t`.
    pub fn level_at(&self, t: u64) -> bool {
        match *self {
            LedcDrive::Level(l) => l,
            LedcDrive::Pwm {
                timer,
                hpoint,
                duty,
            } => {
                let v = timer.count_at(t) % timer.period;
                Self::level_at_count(timer.period, hpoint, duty, v)
            }
        }
    }

    /// Level the channel shows while its counter is frozen at count 0 (timer
    /// held in reset) — what [`LedcDrive::Pwm`] would show at its origin.
    pub fn level_at_count_zero(period: u64, hpoint: u64, duty: u64) -> bool {
        Self::level_at_count(period.max(1), hpoint, duty, 0)
    }

    /// First cycle strictly after `t` at which the output toggles.
    fn next_toggle_after(&self, t: u64) -> Option<u64> {
        let LedcDrive::Pwm {
            timer,
            hpoint,
            duty,
        } = *self
        else {
            return None;
        };
        let p = timer.period;
        if duty == 0 || duty >= p {
            return None;
        }
        let n0 = timer.count_at(t);
        let base = n0 - n0 % p;
        let (h, l) = (hpoint % p, (hpoint + duty) % p);
        let n = [base + h, base + l, base + p + h, base + p + l]
            .into_iter()
            .filter(|&n| n > n0)
            .min()?;
        Some(timer.cycle_of(n))
    }

    /// First period boundary (count multiple of the period) strictly after `t`.
    fn next_boundary_after(&self, t: u64) -> Option<u64> {
        let LedcDrive::Pwm { timer, .. } = *self else {
            return None;
        };
        let n0 = timer.count_at(t);
        Some(timer.cycle_of(n0 - n0 % timer.period + timer.period))
    }
}

/// One timer's counting state, decoded by the chip model from its own register
/// layout (the field positions differ per chip; the meaning does not).
#[derive(Debug, Clone, Copy)]
pub(crate) struct LedcTimerState {
    pub duty_res: u32,
    /// `CLK_DIV`, Q8 fixed point.
    pub div_q8: u64,
    pub paused: bool,
    /// `RST`: the counter is held at 0.
    pub reset: bool,
    /// The timer's source clock in Hz (0 = no clock selected).
    pub src_hz: u64,
    /// Cycle the counter last restarted at 0.
    pub origin: u64,
}

/// `CHn_CONF0.SIG_OUT_EN` (bit 2) and `IDLE_LV` (bit 3) — the same bits on the
/// classic ESP32, C3 and S3 (`ledc_struct.h` of each).
const CONF0_SIG_OUT_EN: u32 = 1 << 2;
const CONF0_IDLE_LV: u32 = 1 << 3;

/// Decode one channel into the drive it puts on its output.
///
/// `held` is the channel's present level, which a PAUSED counter freezes.
pub(crate) fn channel_drive(
    conf0: u32,
    hpoint: u64,
    duty: u64,
    timer: &LedcTimerState,
    cpu_hz: u64,
    held: bool,
) -> LedcDrive {
    if conf0 & CONF0_SIG_OUT_EN == 0 {
        return LedcDrive::Level(conf0 & CONF0_IDLE_LV != 0);
    }
    if timer.paused {
        return LedcDrive::Level(held);
    }
    match LedcTimebase::new(
        timer.duty_res,
        timer.div_q8,
        timer.src_hz,
        cpu_hz,
        timer.origin,
    ) {
        Some(tb) if !timer.reset => LedcDrive::Pwm {
            timer: tb,
            hpoint,
            duty,
        },
        // Held in reset, or no clock: the counter sits at 0.
        _ => LedcDrive::Level(LedcDrive::level_at_count_zero(
            1u64 << timer.duty_res.min(20),
            hpoint,
            duty,
        )),
    }
}

/// Bit a chip model may OR into this type's wake tokens to keep them apart from
/// its own; [`LedcPads`] never produces a token with it set.
pub(crate) const WAVE_TOKEN_BIT: u32 = 1 << 31;

/// The channel outputs of one LEDC instance, and the scheduler bookkeeping
/// that keeps them on time.
#[derive(Debug)]
pub(crate) struct LedcPads {
    names: &'static [&'static str],
    /// Created at bus wiring time, when a GPIO matrix binds it.
    lines: Option<Arc<PadLines>>,
    drive: Vec<LedcDrive>,
    /// A duty change staged on a running channel: takes over at the next
    /// period boundary of that channel's timer.
    pending: Vec<Option<LedcDrive>>,
    /// Cycle the waveform has been advanced (and published) to.
    anchor: u64,
    /// Token of the one live scheduler wake; any other token is stale.
    arm_seq: u32,
    /// Absolute cycle the live wake targets.
    armed_target: Option<u64>,
}

impl LedcPads {
    pub fn new(names: &'static [&'static str]) -> Self {
        Self {
            names,
            lines: None,
            drive: vec![LedcDrive::Level(false); names.len()],
            pending: vec![None; names.len()],
            anchor: 0,
            arm_seq: 0,
            armed_target: None,
        }
    }

    /// The channel wires, created on first call (bus wiring time).
    pub fn lines_arc(&mut self) -> Arc<PadLines> {
        let at = self.anchor;
        let levels: Vec<bool> = self.drive.iter().map(|d| d.level_at(at)).collect();
        self.lines
            .get_or_insert_with(|| Arc::new(PadLines::new(self.names, &levels)))
            .clone()
    }

    pub fn wire_lines(&self) -> Option<&PadLines> {
        self.lines.as_deref()
    }

    pub fn anchor(&self) -> u64 {
        self.anchor
    }

    /// The drive currently on channel `ch` (not counting a staged change).
    #[cfg(test)]
    pub fn drive(&self, ch: usize) -> Option<LedcDrive> {
        self.drive.get(ch).copied()
    }

    /// Live output level of channel `ch` at the advanced-to cycle.
    pub fn level(&self, ch: usize) -> bool {
        self.drive.get(ch).is_some_and(|d| d.level_at(self.anchor))
    }

    /// Put `new` on channel `ch` at cycle `now`. A duty/hpoint change on a
    /// channel already running on the SAME time base waits for the next period
    /// boundary; anything else applies at once.
    pub fn set_drive(&mut self, ch: usize, new: LedcDrive, now: u64) {
        self.advance_to(now);
        let Some(cur) = self.drive.get(ch).copied() else {
            return;
        };
        let staged = self.pending[ch].unwrap_or(cur);
        if staged == new {
            return;
        }
        match (cur, new) {
            (LedcDrive::Pwm { timer: a, .. }, LedcDrive::Pwm { timer: b, .. }) if a == b => {
                self.pending[ch] = if cur == new { None } else { Some(new) };
            }
            _ => {
                self.drive[ch] = new;
                self.pending[ch] = None;
                self.publish(ch, self.anchor);
            }
        }
    }

    fn publish(&self, ch: usize, at: u64) {
        if let Some(lines) = &self.lines {
            lines.set_line_at(ch, self.drive[ch].level_at(at), at);
        }
    }

    /// The first cycle after `t` at which anything happens: an output toggle
    /// (only while a wire exists to carry it) or a staged change coming due.
    fn next_change_after(&self, t: u64) -> Option<u64> {
        let mut best: Option<u64> = None;
        let mut take = |c: Option<u64>| {
            if let Some(c) = c {
                best = Some(best.map_or(c, |b| b.min(c)));
            }
        };
        for (ch, d) in self.drive.iter().enumerate() {
            if self.lines.is_some() {
                take(d.next_toggle_after(t));
            }
            if self.pending[ch].is_some() {
                take(d.next_boundary_after(t));
            }
        }
        best
    }

    /// Advance to `now`, publishing every edge on its own cycle and applying
    /// staged changes at their period boundaries.
    pub fn advance_to(&mut self, now: u64) {
        if now <= self.anchor {
            return;
        }
        while let Some(at) = self.next_change_after(self.anchor) {
            if at > now {
                break;
            }
            for ch in 0..self.drive.len() {
                if self.pending[ch].is_some()
                    && self.drive[ch].next_boundary_after(at.saturating_sub(1)) == Some(at)
                {
                    self.drive[ch] = self.pending[ch].take().expect("checked");
                }
            }
            self.anchor = at;
            for ch in 0..self.drive.len() {
                self.publish(ch, at);
            }
        }
        self.anchor = now;
    }

    /// The scheduler wake request, under the nRF52 PWM's token discipline: a
    /// LATER target needs no new wake (the live one arrives first and
    /// re-arms), the SAME one is already armed, and only an EARLIER one takes a
    /// fresh token, which retires the old wake on arrival.
    pub fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        let Some(target) = self.next_change_after(self.anchor) else {
            return Vec::new();
        };
        if self.armed_target.is_some_and(|live| target >= live) {
            return Vec::new();
        }
        // 31-bit tokens, so a chip model that also arms events of its own (the
        // C3's overflow chain) can tell the two apart by bit 31.
        self.arm_seq = self.arm_seq.wrapping_add(1) & !WAVE_TOKEN_BIT;
        self.armed_target = Some(target);
        vec![(target - self.anchor - 1, self.arm_seq)]
    }

    /// Serve a wake at `now`; returns the delay to the next one, if any.
    /// `None` from a stale token too (it must not re-arm).
    pub fn on_event(&mut self, token: u32, now: u64) -> (bool, Option<u64>) {
        if token != self.arm_seq {
            return (false, None);
        }
        let now = now.max(self.anchor);
        self.advance_to(now);
        self.armed_target = self.next_change_after(self.anchor);
        (true, self.armed_target.map(|t| t - now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: &[&str] = &["CH0", "CH1"];

    /// 8-bit timer, one count per 3 cycles (APB 80 MHz, CPU 240 MHz, DIV 1.0).
    fn tb(origin: u64) -> LedcTimebase {
        LedcTimebase::new(8, 256, 80_000_000, 240_000_000, origin).unwrap()
    }

    fn high_fraction(p: &mut LedcPads, lines: &PadLines, ch: usize, from: u64, to: u64) -> f64 {
        let mut high = 0u64;
        for t in from..to {
            p.advance_to(t);
            if lines.level(ch) {
                high += 1;
            }
        }
        high as f64 / (to - from) as f64
    }

    #[test]
    fn duty_sets_the_fraction_of_each_period_spent_high() {
        for (duty, want) in [(0u64, 0.0), (64, 0.25), (128, 0.5), (192, 0.75), (256, 1.0)] {
            let mut p = LedcPads::new(NAMES);
            let lines = p.lines_arc();
            p.set_drive(
                0,
                LedcDrive::Pwm {
                    timer: tb(0),
                    hpoint: 0,
                    duty,
                },
                0,
            );
            let period = 256 * 3;
            let got = high_fraction(&mut p, &lines, 0, 0, 10 * period);
            assert!((got - want).abs() < 1e-9, "duty {duty}: {got}");
        }
    }

    #[test]
    fn hpoint_shifts_the_pulse_not_its_width() {
        let mut p = LedcPads::new(NAMES);
        let lines = p.lines_arc();
        p.set_drive(
            0,
            LedcDrive::Pwm {
                timer: tb(0),
                hpoint: 200,
                duty: 100,
            },
            0,
        );
        assert!((high_fraction(&mut p, &lines, 0, 0, 7680) - 100.0 / 256.0).abs() < 1e-9);
        p.advance_to(3 * 10);
        assert!(
            lines.level(0),
            "count 10 is inside the wrapped pulse 200..300"
        );
    }

    #[test]
    fn a_new_duty_waits_for_the_period_boundary() {
        let mut p = LedcPads::new(NAMES);
        let lines = p.lines_arc();
        p.set_drive(
            0,
            LedcDrive::Pwm {
                timer: tb(0),
                hpoint: 0,
                duty: 256,
            },
            0,
        );
        p.set_drive(
            0,
            LedcDrive::Pwm {
                timer: tb(0),
                hpoint: 0,
                duty: 0,
            },
            300,
        );
        p.advance_to(767);
        assert!(
            lines.level(0),
            "still the old full-on duty until the boundary"
        );
        p.advance_to(768);
        assert!(!lines.level(0), "the new duty took over at the boundary");
    }

    #[test]
    fn edges_carry_their_own_cycle_even_when_time_jumps() {
        use crate::logic_capture::LogicTap;
        let mut p = LedcPads::new(NAMES);
        let lines = p.lines_arc();
        let tap = LogicTap::new();
        lines.install_tap(Some(tap.clone()), vec![vec![0], vec![]]);
        p.set_drive(
            0,
            LedcDrive::Pwm {
                timer: tb(0),
                hpoint: 0,
                duty: 64,
            },
            0,
        );
        p.advance_to(2 * 768);
        let edges: Vec<(u64, bool)> = tap
            .take_events()
            .iter()
            .map(|e| (e.cycle, e.value))
            .collect();
        assert_eq!(
            edges,
            vec![
                (0, true),
                (192, false),
                (768, true),
                (960, false),
                (1536, true)
            ]
        );
    }

    #[test]
    fn a_fractional_divider_keeps_the_mean_period_exact() {
        // DIV 312.5 (Q8 80000): Arduino's 1 kHz / 8-bit on APB.
        let t = LedcTimebase::new(8, 80_000, 80_000_000, 240_000_000, 0).unwrap();
        let mut p = LedcPads::new(NAMES);
        let lines = p.lines_arc();
        p.set_drive(
            0,
            LedcDrive::Pwm {
                timer: t,
                hpoint: 0,
                duty: 64,
            },
            0,
        );
        // One period = 240_000 cycles (1 ms at 240 MHz).
        assert_eq!(p.drive(0).unwrap().next_boundary_after(0), Some(240_000));
        assert!((high_fraction(&mut p, &lines, 0, 0, 480_000) - 0.25).abs() < 1e-5);
    }

    #[test]
    fn the_scheduler_wakes_on_each_edge_and_goes_quiet_on_a_rail() {
        let mut p = LedcPads::new(NAMES);
        let _lines = p.lines_arc();
        p.set_drive(
            0,
            LedcDrive::Pwm {
                timer: tb(0),
                hpoint: 0,
                duty: 64,
            },
            0,
        );
        let [(delay, token)] = p.take_scheduled_events()[..] else {
            panic!("one wake expected");
        };
        assert_eq!(delay + 1, 192);
        let (live, next) = p.on_event(token, 192);
        assert!(live);
        assert_eq!(next, Some(768 - 192));
        p.set_drive(0, LedcDrive::Level(true), 200);
        let (_, next) = p.on_event(token, 768);
        assert_eq!(next, None, "a constant level needs no wake");
    }

    #[test]
    fn without_a_wire_no_toggle_wakes_are_requested() {
        let mut p = LedcPads::new(NAMES);
        p.set_drive(
            0,
            LedcDrive::Pwm {
                timer: tb(0),
                hpoint: 0,
                duty: 64,
            },
            0,
        );
        assert!(p.take_scheduled_events().is_empty());
    }
}
