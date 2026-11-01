// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Renesas RA general PWM timer (GPT32 / GPT16): saw-wave PWM on GTIOCnA/B.
//!
//! Register layout from FSP `R7FA8M1AH.h` / `R7FA6M5BH.h` (`R_GPT0_Type`,
//! one channel per 0x100 window from `R_GPT0_BASE`): `GTWP` +0x00, `GTSTR`
//! +0x04, `GTSTP` +0x08, `GTCLR` +0x0C, `GTSSR`/`GTPSR`/`GTCSR` +0x10/14/18,
//! `GTCR` +0x2C, `GTUDDTYC` +0x30, `GTIOR` +0x34, `GTST` +0x3C, `GTBER`
//! +0x40, `GTCNT` +0x48, `GTCCRA/B/C/E/D/F` +0x4C..0x60 (FSP index order
//! A, B, C, E, D, F), `GTPR` +0x64, `GTPBR` +0x68. Behaviour from FSP
//! `r_gpt.c` and Zephyr's `pwm_renesas_ra.c`, which drives it. One model
//! covers every channel, because `GTSTR`/`GTSTP`/`GTCLR` are channel masks:
//! a write through any channel's copy starts, stops or clears every channel
//! whose bit is set, if that channel's `GTSSR.CSTRT` / `GTPSR.CSTOP` /
//! `GTCSR.CCLR` (bit 31, which FSP `R_GPT_Open` sets) allows software
//! control.
//!
//! Modelled (saw-wave PWM, `GTCR.MD` = 000, counting up):
//! - the counter, derived lazily from the bus cycle count: it counts at
//!   `pclkd_hz` / 2^`GTCR.TPCS` from 0 to `GTPR` and wraps; `GTCNT` reads
//!   the live count and is writable only while the channel is stopped;
//!   32-bit channels per `wide_channels`, the rest 16-bit;
//! - the cycle end (`GTPR` -> 0): `GTST.TCFPO`, the overflow event, and the
//!   single-buffer transfers `GTPBR` -> `GTPR` (`GTBER.PR` != 0) and
//!   `GTCCRC` -> `GTCCRA`, `GTCCRE` -> `GTCCRB` (`GTBER.CCRA`/`CCRB` != 0),
//!   each unless `GTBER.BD1`/`BD0` disables it; `GTBER.CCRSWT` forces the
//!   compare transfer at once (FSP `R_GPT_PeriodSet` writes 0x55_0000);
//! - compare match A/B (the count steps onto `GTCCRA`/`GTCCRB`): `TCFA`/
//!   `TCFB` and the capture/compare events. Flags are write-0-to-clear;
//! - GTIOCnA/B (`GTIOR.OAE`/`OBE`): `GTIOA[3:2]` is the level set at the
//!   cycle end and `GTIOA[1:0]` the level at compare match (00 keep, 01
//!   low, 10 high, 11 toggle), so FSP's 0b01001 is high for counts
//!   0..=`GTCCRA`, `GTCCRA` + 1 counts per `GTPR` + 1 (FSP: "the pins
//!   transition 1 cycle after compare match"). Counting starts from the
//!   initial level `GTIOA[4]`, so when that is the compare-match level the
//!   first period shows it throughout and the waveform begins at the first
//!   cycle end. `GTUDDTYC.OADTY`/`OBDTY` = 10/11 hold the pin low/high
//!   (0%/100%). A stopped channel drives `OADFLT` unless `OAHLD` keeps the
//!   last level. Edges are published on each pin's wire at their exact
//!   cycle ([`LedcPads`]); `gtioc_pads` names the pads that carry them, and
//!   a pad carries one while its `PmnPFS` has PMR = 1, PSEL = 00011.
//!
//! Interrupts are ELC events, not NVIC lines: channel n's capture/compare
//! A, B and overflow are `elc_event` + 9n + 0, + 1 and + 6 (FSP `bsp_elc.h`:
//! `ELC_EVENT_GPT0_CAPTURE_COMPARE_A` 0x0A1 .. `ELC_EVENT_GPT0_COUNTER_
//! OVERFLOW` 0x0A7, nine per channel on the RA8M1; 0x0C0 on the RA6M5).
//! They leave through `explicit_irqs` with
//! [`Peripheral::explicit_irqs_are_link_events`], so the bus links them
//! through the ICU `IELSRn` ([`super::ra_icu`]).
//!
//! NOT modelled: triangle-wave and one-shot modes (`GTCR.MD` != 0 counts
//! like saw-wave and the pins hold their level), down-counting, input
//! capture and the hardware start/stop/clear/count sources (`GTSSR`..
//! `GTICBSR` low bits, the GTETRG/GTIOC inputs), compare C-F events and the
//! double buffers, dead time, `GTINTAD`/POEG output disable, A/D triggers,
//! `GTWP` write protection (stored only), toggle-at-compare phase across a
//! period change, `OADTY` timing (applied at once; silicon waits for the
//! cycle end unless `OADTYF`), and a direct `GTCCRA`/`GTCCRB` write on a
//! running, unbuffered channel (it takes effect from the next cycle end).

use crate::peripherals::esp_ledc_wave::{LedcDrive, LedcPads, LedcTimebase};
use crate::peripherals::pad_lines::{LineDrive, PadLines};
use crate::{CycleClock, Peripheral, SimResult};
use std::collections::BTreeMap;
use std::sync::Arc;

/// One channel's register window.
pub const STRIDE: u64 = 0x100;
const WORDS: usize = (STRIDE / 4) as usize;

const GTWP: u64 = 0x00;
const GTSTR: u64 = 0x04;
const GTSTP: u64 = 0x08;
const GTCLR: u64 = 0x0C;
const GTSSR: u64 = 0x10;
const GTPSR: u64 = 0x14;
const GTCSR: u64 = 0x18;
const GTCR: u64 = 0x2C;
const GTUDDTYC: u64 = 0x30;
const GTIOR: u64 = 0x34;
const GTST: u64 = 0x3C;
const GTBER: u64 = 0x40;
const GTCNT: u64 = 0x48;
const GTCCRA: u64 = 0x4C;
const GTCCRB: u64 = 0x50;
const GTCCRC: u64 = 0x54;
const GTCCRE: u64 = 0x58;
const GTCCRF: u64 = 0x60;
const GTPR: u64 = 0x64;
const GTPBR: u64 = 0x68;

/// `GTSSR.CSTRT` / `GTPSR.CSTOP` / `GTCSR.CCLR`: software start/stop/clear.
const SOFTWARE: u32 = 1 << 31;
const CST: u32 = 1 << 0;
const MD_MASK: u32 = 0x7 << 16;
const TPCS_SHIFT: u32 = 23;
const TPCS_MASK: u32 = 0xF << TPCS_SHIFT;

const TCFA: u32 = 1 << 0;
const TCFB: u32 = 1 << 1;
const TCFPO: u32 = 1 << 6;
const GTST_FLAGS: u32 = 0xFF;
const TUCF: u32 = 1 << 15;

const BD0: u32 = 1 << 0;
const BD1: u32 = 1 << 1;
const CCRA_SHIFT: u32 = 16;
const CCRB_SHIFT: u32 = 18;
const PR_SHIFT: u32 = 20;
const CCRSWT: u32 = 1 << 22;

const EVENTS_PER_CHANNEL: u32 = 9;
const EV_CCMPA: u32 = 0;
const EV_CCMPB: u32 = 1;
const EV_OVF: u32 = 6;

/// Most channels any RA part has (RA8M1: GPT0-13).
pub const MAX_CHANNELS: usize = 14;

/// GTIOC wire names, two per channel (A then B), in line order.
pub const LINE_NAMES: [&str; 2 * MAX_CHANNELS] = [
    "GTIOC0A", "GTIOC0B", "GTIOC1A", "GTIOC1B", "GTIOC2A", "GTIOC2B", "GTIOC3A", "GTIOC3B",
    "GTIOC4A", "GTIOC4B", "GTIOC5A", "GTIOC5B", "GTIOC6A", "GTIOC6B", "GTIOC7A", "GTIOC7B",
    "GTIOC8A", "GTIOC8B", "GTIOC9A", "GTIOC9B", "GTIOC10A", "GTIOC10B", "GTIOC11A", "GTIOC11B",
    "GTIOC12A", "GTIOC12B", "GTIOC13A", "GTIOC13B",
];

/// The rate the bus cycles run at (Zephyr's EK-RA8M1 ICLK, 480 MHz).
pub const DEFAULT_CLOCK_HZ: u64 = 480_000_000;
/// RA8M1 PCLKD under Zephyr's EK-RA8M1 clock tree (PLL1P 480 MHz / 4).
pub const DEFAULT_PCLKD_HZ: u64 = 120_000_000;
/// `ELC_EVENT_GPT0_CAPTURE_COMPARE_A` on the RA8M1.
pub const DEFAULT_ELC_EVENT: u32 = 0x0A1;

/// Which count event of a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Overflow = 0,
    CompareA = 1,
    CompareB = 2,
}

/// One GPT channel.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Channel {
    /// The 64 register words; `GTCNT` holds the count only while stopped.
    regs: Vec<u32>,
    /// Count-source ticks are counted from `origin` (a CPU cycle), where the
    /// count was `base`.
    origin: u64,
    base: u32,
    /// Per [`Kind`]: ticks since `origin` up to which that event is handled.
    done: [u64; 3],
    /// Started and not yet at its first cycle end.
    first_period: bool,
}

impl Channel {
    fn new(wide: bool) -> Self {
        let top = if wide { u32::MAX } else { 0xFFFF };
        let mut regs = vec![0u32; WORDS];
        for off in [GTCCRA, GTCCRB, GTCCRC, GTCCRE, 0x5C, GTCCRF, GTPR, GTPBR] {
            regs[(off / 4) as usize] = top;
        }
        regs[(GTST / 4) as usize] = TUCF;
        regs[(GTUDDTYC / 4) as usize] = 1;
        Self {
            regs,
            origin: 0,
            base: 0,
            done: [0; 3],
            first_period: false,
        }
    }

    fn reg(&self, off: u64) -> u32 {
        self.regs[(off / 4) as usize]
    }

    fn set(&mut self, off: u64, v: u32) {
        self.regs[(off / 4) as usize] = v;
    }

    fn running(&self) -> bool {
        self.reg(GTCR) & CST != 0
    }
}

/// The GPT block: every channel, the GTIOC wires and one scheduler wake.
#[derive(Debug, serde::Serialize)]
pub struct RaGpt {
    clock_hz: u64,
    pclkd_hz: u64,
    elc_event: u32,
    /// Bit n: channel n is a 32-bit GPT32 (else 16-bit).
    wide_channels: u32,
    channels: Vec<Channel>,
    /// Words past the last channel (GPT_OPS, reserved), stored only.
    extra: BTreeMap<u64, u32>,
    /// Pads that carry each GTIOC line: `(line, port name, pin)`.
    #[serde(skip)]
    output_pads: Vec<(usize, String, u8)>,
    #[serde(skip)]
    pads: LedcPads,
    /// A pad is bound to the wires, so the pins are generated.
    #[serde(skip)]
    wired: bool,
    /// Events raised during an MMIO access, sent at the next wake.
    #[serde(skip)]
    unsent: Vec<u32>,
    /// The one live scheduler wake: its token and target cycle.
    #[serde(skip)]
    generation: u32,
    #[serde(skip)]
    armed: Option<u64>,
    #[serde(skip)]
    pending_events: Vec<(u64, u32)>,
    #[serde(skip)]
    clock: Option<CycleClock>,
}

impl RaGpt {
    pub fn new(
        channels: usize,
        clock_hz: u64,
        pclkd_hz: u64,
        elc_event: u32,
        wide_channels: u32,
    ) -> Self {
        let n = channels.clamp(1, MAX_CHANNELS);
        Self {
            clock_hz: clock_hz.max(1),
            pclkd_hz: pclkd_hz.max(1),
            elc_event,
            wide_channels,
            channels: (0..n)
                .map(|ch| Channel::new(wide_channels & (1 << ch) != 0))
                .collect(),
            extra: BTreeMap::new(),
            output_pads: Vec::new(),
            pads: LedcPads::new(&LINE_NAMES[..2 * n]),
            wired: false,
            unsent: Vec::new(),
            generation: 0,
            armed: None,
            pending_events: Vec::new(),
            clock: None,
        }
    }

    /// Name the pads that carry GTIOC lines: `(line, port peripheral name,
    /// pin)`, line 2n = GTIOCnA, 2n + 1 = GTIOCnB.
    pub fn with_output_pads(mut self, pads: Vec<(usize, String, u8)>) -> Self {
        self.output_pads = pads
            .into_iter()
            .filter(|(line, _, _)| *line < 2 * self.channels.len())
            .collect();
        self
    }

    /// Parse a `gtioc_pads` entry: `GTIOC7A` and `P603` give
    /// `(14, "port6", 3)`.
    pub fn parse_pad(signal: &str, pad: &str) -> Option<(usize, String, u8)> {
        let line = LINE_NAMES
            .iter()
            .position(|n| n.eq_ignore_ascii_case(signal))?;
        let pad = pad.strip_prefix(['P', 'p'])?;
        if pad.len() != 3 {
            return None;
        }
        let port = u8::from_str_radix(&pad[..1], 16).ok()?;
        let pin: u8 = pad[1..].parse().ok()?;
        (pin < 16).then(|| (line, format!("port{port:x}"), pin))
    }

    pub fn output_pads(&self) -> &[(usize, String, u8)] {
        &self.output_pads
    }

    crate::cycle_clock::scheduler_mode!();

    fn now_cycle(&self) -> u64 {
        self.clock.as_ref().map_or(0, CycleClock::now)
    }

    fn mask(&self, ch: usize) -> u32 {
        if self.wide_channels & (1 << ch) != 0 {
            u32::MAX
        } else {
            0xFFFF
        }
    }

    /// CPU cycles per count is `num / den`.
    fn rate(&self, ch: usize) -> (u128, u128) {
        let tpcs = (self.channels[ch].reg(GTCR) & TPCS_MASK) >> TPCS_SHIFT;
        let div = 1u128 << tpcs.min(10);
        (u128::from(self.clock_hz) * div, u128::from(self.pclkd_hz))
    }

    fn ticks_at(&self, ch: usize, cycle: u64) -> u64 {
        let (num, den) = self.rate(ch);
        let c = &self.channels[ch];
        (u128::from(cycle.saturating_sub(c.origin)) * den / num) as u64
    }

    fn cycle_of(&self, ch: usize, ticks: u64) -> u64 {
        let (num, den) = self.rate(ch);
        let c = (u128::from(ticks) * num).div_ceil(den);
        self.channels[ch]
            .origin
            .saturating_add(c.min(u128::from(u64::MAX)) as u64)
    }

    /// Counts per period, and ticks from `origin` to the first wrap to 0.
    fn period(&self, ch: usize) -> (u64, u64) {
        let c = &self.channels[ch];
        let mask = u64::from(self.mask(ch));
        let pr = u64::from(c.reg(GTPR)) & mask;
        let base = u64::from(c.base);
        let first = if base <= pr {
            pr - base + 1
        } else {
            mask - base + 1
        };
        (pr + 1, first)
    }

    fn count_at(&self, ch: usize, cycle: u64) -> u32 {
        let c = &self.channels[ch];
        if !c.running() {
            return c.reg(GTCNT);
        }
        let ticks = self.ticks_at(ch, cycle);
        let (p, first) = self.period(ch);
        if ticks < first {
            c.base.wrapping_add(ticks as u32) & self.mask(ch)
        } else {
            ((ticks - first) % p) as u32
        }
    }

    /// Ticks from `origin` of the first event of `kind` after `done`.
    fn next_tick(&self, ch: usize, kind: Kind) -> Option<u64> {
        let c = &self.channels[ch];
        let (p, first) = self.period(ch);
        let t0 = match kind {
            Kind::Overflow => first,
            Kind::CompareA | Kind::CompareB => {
                let off = if kind == Kind::CompareA {
                    GTCCRA
                } else {
                    GTCCRB
                };
                let ccr = u64::from(c.reg(off) & self.mask(ch));
                if ccr >= p {
                    return None;
                }
                let base = u64::from(c.base);
                if base < ccr {
                    ccr - base
                } else {
                    first + ccr
                }
            }
        };
        let done = c.done[kind as usize];
        Some(if done < t0 {
            t0
        } else {
            t0 + ((done - t0) / p + 1) * p
        })
    }

    /// The earliest unhandled count event of any running channel:
    /// `(cycle, channel, kind, tick)`.
    fn next_event(&self) -> Option<(u64, usize, Kind, u64)> {
        let mut best: Option<(u64, usize, Kind, u64)> = None;
        for ch in 0..self.channels.len() {
            if !self.channels[ch].running() {
                continue;
            }
            for kind in [Kind::Overflow, Kind::CompareA, Kind::CompareB] {
                let Some(tick) = self.next_tick(ch, kind) else {
                    continue;
                };
                let at = self.cycle_of(ch, tick);
                if best.is_none_or(|(b, bch, bk, _)| (at, ch, kind) < (b, bch, bk)) {
                    best = Some((at, ch, kind, tick));
                }
            }
        }
        best
    }

    /// Restart the tick count at `cycle` with the count `base`.
    fn reanchor(&mut self, ch: usize, cycle: u64, base: u32) {
        let c = &mut self.channels[ch];
        c.origin = cycle;
        c.base = base;
        c.done = [0; 3];
    }

    /// Handle every count event up to `now`, in time order, then bring the
    /// pins up to `now`.
    fn sync(&mut self, now: u64) {
        while let Some((at, ch, kind, tick)) = self.next_event() {
            if at > now {
                break;
            }
            self.channels[ch].done[kind as usize] = tick;
            let ev = self.elc_event + EVENTS_PER_CHANNEL * ch as u32;
            match kind {
                Kind::Overflow => {
                    self.unsent.push(ev + EV_OVF);
                    self.cycle_end(ch, at);
                }
                Kind::CompareA => {
                    let c = &mut self.channels[ch];
                    c.set(GTST, c.reg(GTST) | TCFA);
                    self.unsent.push(ev + EV_CCMPA);
                }
                Kind::CompareB => {
                    let c = &mut self.channels[ch];
                    c.set(GTST, c.reg(GTST) | TCFB);
                    self.unsent.push(ev + EV_CCMPB);
                }
            }
        }
        if self.wired {
            self.pads.advance_to(now);
        }
    }

    /// The cycle end at `at`: `TCFPO`, the buffer transfers, the end of the
    /// first period.
    fn cycle_end(&mut self, ch: usize, at: u64) {
        let c = &mut self.channels[ch];
        c.set(GTST, c.reg(GTST) | TCFPO);
        let gtber = c.reg(GTBER);
        let mut changed = std::mem::take(&mut c.first_period);
        let mut period_changed = false;
        if (gtber >> PR_SHIFT) & 3 != 0 && gtber & BD1 == 0 && c.reg(GTPBR) != c.reg(GTPR) {
            c.set(GTPR, c.reg(GTPBR));
            period_changed = true;
        }
        if gtber & BD0 == 0 {
            changed |= transfer_compare(c, gtber);
        }
        if period_changed || changed {
            // The count is 0 at the cycle end; a fresh anchor also gives the
            // wave a new time base, so the change is not staged a period late.
            self.reanchor(ch, at, 0);
            self.redrive(ch, at);
        }
    }

    /// What pin `pin` (0 = A, 1 = B) of channel `ch` puts out, and whether
    /// the GPT drives the pad at all.
    fn drive(&self, ch: usize, pin: usize) -> (LedcDrive, bool) {
        let line = 2 * ch + pin;
        let held = self.pads.level(line);
        let c = &self.channels[ch];
        let gtior = c.reg(GTIOR) >> (16 * pin);
        if gtior & (1 << 8) == 0 {
            return (LedcDrive::Level(held), false);
        }
        if !c.running() {
            let level = if gtior & (1 << 7) != 0 {
                held
            } else {
                gtior & (1 << 6) != 0
            };
            return (LedcDrive::Level(level), true);
        }
        if c.reg(GTCR) & MD_MASK != 0 {
            return (LedcDrive::Level(held), true);
        }
        match (c.reg(GTUDDTYC) >> (16 + 8 * pin)) & 3 {
            0b10 => return (LedcDrive::Level(false), true),
            0b11 => return (LedcDrive::Level(true), true),
            _ => {}
        }
        let initial = gtior & (1 << 4) != 0;
        let end = (gtior >> 2) & 3;
        let cm = gtior & 3;
        let (p, _) = self.period(ch);
        let ccr = u64::from(c.reg(if pin == 0 { GTCCRA } else { GTCCRB }) & self.mask(ch));
        let (num, den) = self.rate(ch);
        // Cycle at which the count was (or would have been) 0.
        let back = (u128::from(c.base) * num / den).min(u128::from(u64::MAX)) as u64;
        let origin = c.origin.saturating_sub(back);
        let pwm = |period: u64, hpoint: u64, duty: u64| LedcDrive::Pwm {
            timer: LedcTimebase {
                period,
                num,
                den,
                origin,
            },
            hpoint,
            duty,
        };
        let drive = match (end, cm) {
            (0b10 | 0b01, _) if c.first_period && initial != (end == 0b10) => {
                LedcDrive::Level(initial)
            }
            (0b10, 0b01) => pwm(p, 0, (ccr + 1).min(p)),
            (0b01, 0b10) => {
                let lead = (ccr + 1).min(p);
                pwm(p, lead % p, p - lead)
            }
            (0b00, 0b11) if ccr < p => {
                let first = ccr + 1;
                pwm(2 * p, if initial { first + p } else { first }, p)
            }
            (0b11, 0b00) => pwm(2 * p, if initial { 0 } else { p }, p),
            (0b10 | 0b01, 0b00) => LedcDrive::Level(end == 0b10),
            (0b00, 0b10 | 0b01) => LedcDrive::Level(cm == 0b10),
            (0b00, 0b00) => LedcDrive::Level(initial),
            (e, m) if e == m => LedcDrive::Level(e == 0b10),
            _ => LedcDrive::Level(held),
        };
        (drive, true)
    }

    /// Re-derive both pins of channel `ch` at `at`.
    fn redrive(&mut self, ch: usize, at: u64) {
        if !self.wired {
            return;
        }
        if self.pads.wire_lines().is_none() {
            return;
        }
        let lines = self.pads.lines_arc();
        for pin in 0..2 {
            let (drive, drives) = self.drive(ch, pin);
            let line = 2 * ch + pin;
            self.pads.set_drive(line, drive, at);
            lines.set_mode(
                line,
                if drives {
                    LineDrive::PushPull
                } else {
                    LineDrive::Input
                },
            );
        }
    }

    fn start(&mut self, ch: usize, now: u64) {
        if self.channels[ch].running() {
            return;
        }
        let c = &mut self.channels[ch];
        let base = c.reg(GTCNT);
        c.set(GTCR, c.reg(GTCR) | CST);
        c.first_period = true;
        self.reanchor(ch, now, base);
    }

    fn stop(&mut self, ch: usize, now: u64) {
        if !self.channels[ch].running() {
            return;
        }
        let count = self.count_at(ch, now);
        let c = &mut self.channels[ch];
        c.set(GTCNT, count);
        c.set(GTCR, c.reg(GTCR) & !CST);
        c.first_period = false;
    }

    fn clear(&mut self, ch: usize, now: u64) {
        if self.channels[ch].running() {
            self.reanchor(ch, now, 0);
        } else {
            self.channels[ch].set(GTCNT, 0);
        }
    }

    /// Keep counting from the current count, with new registers.
    fn reanchor_now(&mut self, ch: usize, now: u64) {
        if self.channels[ch].running() {
            let count = self.count_at(ch, now);
            self.reanchor(ch, now, count);
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        let ch = (offset / STRIDE) as usize;
        let off = (offset % STRIDE) & !3;
        let Some(c) = self.channels.get(ch) else {
            return self.extra.get(&(offset & !3)).copied().unwrap_or(0);
        };
        let all = (1u32 << self.channels.len()) - 1;
        let running: u32 = self
            .channels
            .iter()
            .enumerate()
            .filter(|(_, c)| c.running())
            .fold(0, |m, (n, _)| m | (1 << n));
        match off {
            GTSTR => running,
            GTSTP => all & !running,
            GTCLR => 0,
            GTCNT => self.count_at(ch, self.now_cycle()),
            GTST => (c.reg(GTST) & !TUCF) | if c.reg(GTUDDTYC) & 1 != 0 { TUCF } else { 0 },
            GTBER => c.reg(GTBER) & !CCRSWT,
            _ => c.reg(off),
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let ch = (offset / STRIDE) as usize;
        let off = (offset % STRIDE) & !3;
        if ch >= self.channels.len() {
            self.extra.insert(offset & !3, value);
            return;
        }
        let now = self.now_cycle();
        self.sync(now);
        let mask = self.mask(ch);
        let mut touched = vec![ch];
        match off {
            GTWP => {
                if (value >> 8) & 0xFF == 0xA5 {
                    self.channels[ch].set(GTWP, value & 0xFF);
                }
            }
            GTSTR | GTSTP | GTCLR => {
                touched.clear();
                let enable = match off {
                    GTSTR => GTSSR,
                    GTSTP => GTPSR,
                    _ => GTCSR,
                };
                for n in 0..self.channels.len() {
                    if value & (1 << n) == 0 || self.channels[n].reg(enable) & SOFTWARE == 0 {
                        continue;
                    }
                    match off {
                        GTSTR => self.start(n, now),
                        GTSTP => self.stop(n, now),
                        _ => self.clear(n, now),
                    }
                    touched.push(n);
                }
            }
            GTCR => {
                let was = self.channels[ch].reg(GTCR);
                if (was ^ value) & TPCS_MASK != 0 {
                    self.reanchor_now(ch, now);
                }
                let c = &mut self.channels[ch];
                c.set(GTCR, (value & !CST) | (was & CST));
                match (was & CST != 0, value & CST != 0) {
                    (false, true) => self.start(ch, now),
                    (true, false) => self.stop(ch, now),
                    _ => {}
                }
            }
            GTST => {
                let c = &mut self.channels[ch];
                c.set(GTST, c.reg(GTST) & (value | !GTST_FLAGS));
            }
            GTBER => {
                let c = &mut self.channels[ch];
                c.set(GTBER, value & !CCRSWT);
                if value & CCRSWT != 0 {
                    transfer_compare(c, value);
                    self.reanchor_now(ch, now);
                }
            }
            GTCNT => {
                if !self.channels[ch].running() {
                    self.channels[ch].set(GTCNT, value & mask);
                }
            }
            GTPR | GTCCRA | GTCCRB => {
                self.reanchor_now(ch, now);
                self.channels[ch].set(off, value & mask);
            }
            GTCCRC..=GTCCRF | GTPBR => self.channels[ch].set(off, value & mask),
            _ => self.channels[ch].set(off, value),
        }
        for n in touched {
            self.redrive(n, now);
        }
        self.rearm(now);
    }

    /// The cycle the next wake is for: a count event, a pin edge, or events
    /// still to send.
    fn target(&self, now: u64) -> Option<u64> {
        let mut best = self.next_event().map(|(at, ..)| at);
        if self.wired {
            if let Some(at) = self.pads.next_change() {
                best = Some(best.map_or(at, |b| b.min(at)));
            }
        }
        if !self.unsent.is_empty() {
            best = Some(now + 1);
        }
        best.map(|at| at.max(now + 1))
    }

    /// Keep one live wake: a later target is served by the live wake, which
    /// re-arms on arrival; only an earlier one takes a fresh token.
    fn rearm(&mut self, now: u64) {
        if !self.scheduler_mode() {
            return;
        }
        let Some(target) = self.target(now) else {
            return;
        };
        if self.armed.is_some_and(|live| target >= live && live > now) {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        self.armed = Some(target);
        self.pending_events
            .push((target - now - 1, self.generation));
    }

    fn any_activity(&self) -> bool {
        !self.unsent.is_empty()
            || self.channels.iter().any(Channel::running)
            || (self.wired && self.pads.next_change().is_some())
    }

    /// Bus wiring: the GTIOC wires; with `arm`, start generating the pins.
    pub fn output_lines_arc(&mut self, arm: bool) -> Arc<PadLines> {
        let lines = self.pads.lines_arc();
        if arm && !self.wired {
            self.wired = true;
            let now = self.now_cycle();
            for ch in 0..self.channels.len() {
                self.redrive(ch, now);
            }
        }
        lines
    }
}

/// `GTCCRC` -> `GTCCRA` and `GTCCRE` -> `GTCCRB` where `gtber` enables the
/// buffer; `true` when a compare register changed.
fn transfer_compare(c: &mut Channel, gtber: u32) -> bool {
    let mut changed = false;
    for (shift, buf, reg) in [(CCRA_SHIFT, GTCCRC, GTCCRA), (CCRB_SHIFT, GTCCRE, GTCCRB)] {
        if (gtber >> shift) & 3 != 0 && c.reg(buf) != c.reg(reg) {
            c.set(reg, c.reg(buf));
            changed = true;
        }
    }
    changed
}

impl Default for RaGpt {
    fn default() -> Self {
        Self::new(
            MAX_CHANNELS,
            DEFAULT_CLOCK_HZ,
            DEFAULT_PCLKD_HZ,
            DEFAULT_ELC_EVENT,
            0xFF,
        )
    }
}

impl Peripheral for RaGpt {
    fn legacy_tick_active(&self) -> bool {
        !self.scheduler_mode() && self.any_activity()
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    fn uses_scheduler(&self) -> bool {
        self.scheduler_mode()
    }

    fn explicit_irqs_are_link_events(&self) -> bool {
        true
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        self.tick_elapsed(1)
    }

    fn tick_elapsed(&mut self, _cycles: u64) -> crate::PeripheralTickResult {
        if self.scheduler_mode() {
            return crate::PeripheralTickResult::default();
        }
        let now = self.now_cycle();
        self.sync(now);
        let events = std::mem::take(&mut self.unsent);
        crate::PeripheralTickResult {
            explicit_irqs: (!events.is_empty()).then_some(events),
            ..Default::default()
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok((self.read_word(offset & !3) >> (8 * (offset & 3))) as u8)
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        Ok((self.read_word(offset & !3) >> (8 * (offset & 2))) as u16)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset & !3))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let shift = 8 * (offset & 3);
        let word = self.read_word(offset & !3) & !(0xFF << shift);
        self.write_word(offset & !3, word | (u32::from(value) << shift));
        Ok(())
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        let shift = 8 * (offset & 2);
        let word = self.read_word(offset & !3) & !(0xFFFF << shift);
        self.write_word(offset & !3, word | (u32::from(value) << shift));
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset & !3, value);
        Ok(())
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        std::mem::take(&mut self.pending_events)
    }

    fn on_event(
        &mut self,
        event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if event_token != self.generation {
            return crate::sched::EventResult::default();
        }
        let now = self.now_cycle();
        self.sync(now);
        let events = std::mem::take(&mut self.unsent);
        self.pending_events.clear();
        // The drain does not harvest `take_scheduled_events` after an event,
        // so the next wake goes back as a reschedule of this token.
        self.armed = self.target(now);
        crate::sched::EventResult {
            explicit_irqs: events,
            reschedule_delay: self.armed.map(|at| at - now),
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn on_node_reset(&mut self) {
        let clock = self.clock.take();
        let pads = std::mem::take(&mut self.output_pads);
        let wired = self.wired;
        let lines = self.wired.then(|| self.pads.lines_arc());
        let fresh = Self::new(
            self.channels.len(),
            self.clock_hz,
            self.pclkd_hz,
            self.elc_event,
            self.wide_channels,
        );
        self.channels = fresh.channels;
        self.extra.clear();
        self.unsent.clear();
        self.output_pads = pads;
        self.clock = clock;
        if wired && lines.is_some() {
            let now = self.now_cycle();
            for ch in 0..self.channels.len() {
                self.redrive(ch, now);
            }
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn wire_lines(&self) -> Option<&PadLines> {
        self.pads.wire_lines()
    }

    fn timer_output_lines(&mut self, arm: bool) -> Option<Arc<PadLines>> {
        Some(self.output_lines_arc(arm))
    }

    fn timer_output_family(&self) -> Option<&'static str> {
        Some("ra_gpt")
    }

    fn timer_output_pads(&self) -> Vec<(usize, String, u8)> {
        self.output_pads.clone()
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let decode = |e: serde_json::Error| {
            crate::SimulationError::NotImplemented(format!("RaGpt snapshot decode: {e}"))
        };
        let channels: Vec<Channel> =
            serde_json::from_value(state.get("channels").cloned().unwrap_or_default())
                .map_err(decode)?;
        if channels.len() != self.channels.len() {
            return Err(crate::SimulationError::NotImplemented(
                "RaGpt snapshot: channel count differs".into(),
            ));
        }
        self.channels = channels;
        self.extra = serde_json::from_value(state.get("extra").cloned().unwrap_or_default())
            .unwrap_or_default();
        let now = self.now_cycle();
        for ch in 0..self.channels.len() {
            self.redrive(ch, now);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 4 CPU cycles per count, like the RA8M1's 480 MHz ICLK over PCLKD.
    const CLOCK: u64 = 480;
    const PCLKD: u64 = 120;

    fn gpt() -> (RaGpt, CycleClock) {
        let clock = CycleClock::default();
        let mut g = RaGpt::new(14, CLOCK, PCLKD, DEFAULT_ELC_EVENT, 0xFF);
        g.attach_cycle_clock(clock.clone());
        (g, clock)
    }

    fn w(g: &mut RaGpt, ch: u64, off: u64, v: u32) {
        g.write_u32(ch * STRIDE + off, v).unwrap();
    }

    fn r(g: &RaGpt, ch: u64, off: u64) -> u32 {
        g.read_u32(ch * STRIDE + off).unwrap()
    }

    /// FSP `R_GPT_Open` + Zephyr `pwm_set` for a saw-wave PWM on GTIOCnA:
    /// period `p` counts, `duty` counts high.
    fn zephyr_pwm(g: &mut RaGpt, ch: u64, p: u32, duty: u32) {
        w(g, ch, GTSSR, SOFTWARE);
        w(g, ch, GTPSR, SOFTWARE);
        w(g, ch, GTCSR, SOFTWARE);
        w(g, ch, GTCR, 0);
        w(g, ch, GTPBR, p - 1);
        w(g, ch, GTPR, p - 1);
        // OAE, initial and stop level low, low at compare, high at cycle end.
        w(g, ch, GTIOR, (1 << 8) | 0b01001);
        w(g, ch, GTUDDTYC, 1);
        w(g, ch, GTCCRC, p / 2 - 1);
        w(g, ch, GTBER, 0x55_0000);
        w(g, ch, GTCLR, 1 << ch);
        w(g, ch, GTCCRC, duty - 1);
        w(g, ch, GTSTR, 1 << ch);
    }

    /// Advance to `to`, stopping at every wake target on the way, as either
    /// time base does (the scheduled wake, or the legacy walk reaching it),
    /// and collect the events with their cycles.
    fn walk(g: &mut RaGpt, clock: &CycleClock, to: u64) -> Vec<(u64, u32)> {
        let mut out = Vec::new();
        loop {
            let now = clock.now();
            let at = g.target(now).filter(|&at| at <= to).unwrap_or(to);
            clock.publish(at);
            g.sync(at);
            out.extend(std::mem::take(&mut g.unsent).into_iter().map(|e| (at, e)));
            if at >= to {
                return out;
            }
        }
    }

    #[test]
    fn reset_values() {
        let (g, _) = gpt();
        assert_eq!(r(&g, 0, GTPR), u32::MAX);
        assert_eq!(r(&g, 8, GTPR), 0xFFFF, "GPT8 is 16-bit");
        assert_eq!(r(&g, 0, GTST), TUCF);
        assert_eq!(r(&g, 0, GTSTR), 0);
        assert_eq!(r(&g, 0, GTSTP), 0x3FFF);
    }

    #[test]
    fn gtstr_needs_cstrt_and_the_count_follows_pclkd() {
        let (mut g, clock) = gpt();
        w(&mut g, 3, GTPR, 999);
        w(&mut g, 0, GTSTR, 1 << 3);
        assert_eq!(r(&g, 0, GTSTR), 0, "GTSSR.CSTRT clear: GTSTR ignored");
        w(&mut g, 3, GTSSR, SOFTWARE);
        w(&mut g, 0, GTSTR, 1 << 3);
        assert_eq!(
            r(&g, 5, GTSTR),
            1 << 3,
            "any channel's GTSTR reads every CST"
        );
        clock.publish(400);
        assert_eq!(r(&g, 3, GTCNT), 100);
        clock.publish(4400);
        assert_eq!(r(&g, 3, GTCNT), 100, "wraps at GTPR + 1 = 1000 counts");
        w(&mut g, 3, GTPSR, SOFTWARE);
        w(&mut g, 0, GTSTP, 1 << 3);
        clock.publish(4800);
        assert_eq!(r(&g, 3, GTCNT), 100, "stopped");
    }

    #[test]
    fn cycle_end_sets_tcfpo_transfers_buffers_and_raises_overflow() {
        let (mut g, clock) = gpt();
        zephyr_pwm(&mut g, 7, 100, 30);
        assert_eq!(r(&g, 7, GTCCRA), 49, "CCRSWT forced the 50% buffer in");
        let ev = walk(&mut g, &clock, 401);
        let base = DEFAULT_ELC_EVENT + 9 * 7;
        // Compare A at count 49 (cycle 196), the cycle end at 100 counts.
        assert_eq!(ev[0], (196, base + EV_CCMPA));
        assert_eq!(ev[1], (400, base + EV_OVF));
        assert_eq!(r(&g, 7, GTCCRA), 29, "GTCCRC moved in at the cycle end");
        assert_eq!(r(&g, 7, GTST) & (TCFA | TCFPO), TCFA | TCFPO);
        w(&mut g, 7, GTST, !TCFPO);
        assert_eq!(r(&g, 7, GTST) & (TCFA | TCFPO), TCFA, "write 0 clears");
        let ev = walk(&mut g, &clock, 801);
        assert_eq!(ev[0], (400 + 30 * 4 - 4, base + EV_CCMPA));
    }

    #[test]
    fn gtiocna_is_high_for_gtccra_plus_one_counts_after_the_first_period() {
        let (mut g, clock) = gpt();
        let lines = g.output_lines_arc(true);
        zephyr_pwm(&mut g, 7, 100, 30);
        let line = 14;
        let level_at = |g: &mut RaGpt, clock: &CycleClock, t: u64| {
            walk(g, clock, t);
            lines.level(line)
        };
        assert!(!level_at(&mut g, &clock, 10), "first period: initial low");
        assert!(!level_at(&mut g, &clock, 399));
        assert!(
            level_at(&mut g, &clock, 400),
            "high from the first cycle end"
        );
        assert!(level_at(&mut g, &clock, 519), "30 counts high = 120 cycles");
        assert!(!level_at(&mut g, &clock, 520));
        assert!(level_at(&mut g, &clock, 800));
    }

    #[test]
    fn oadty_holds_the_pin_and_a_stopped_pin_drives_oadflt() {
        let (mut g, clock) = gpt();
        let lines = g.output_lines_arc(true);
        zephyr_pwm(&mut g, 1, 100, 30);
        w(&mut g, 1, GTUDDTYC, 1 | (0b11 << 16));
        walk(&mut g, &clock, 1000);
        assert!(lines.level(2), "100%: high");
        w(&mut g, 1, GTUDDTYC, 1 | (0b10 << 16));
        walk(&mut g, &clock, 1001);
        assert!(!lines.level(2), "0%: low");
        w(&mut g, 1, GTUDDTYC, 1);
        walk(&mut g, &clock, 1300);
        w(&mut g, 1, GTIOR, (1 << 8) | (1 << 6) | 0b01001);
        w(&mut g, 1, GTPSR, SOFTWARE);
        w(&mut g, 1, GTSTP, 1 << 1);
        assert!(lines.level(2), "stopped with OADFLT = 1");
        w(&mut g, 1, GTIOR, 0);
        assert!(!lines.drives(2), "OAE clear: the GPT lets go of the pad");
    }

    #[test]
    fn without_a_wire_no_pin_work_and_without_running_no_wakes() {
        let (mut g, _clock) = gpt();
        w(&mut g, 2, GTPR, 9);
        assert!(!g.any_activity());
        w(&mut g, 2, GTSSR, SOFTWARE);
        w(&mut g, 2, GTSTR, 1 << 2);
        assert!(g.any_activity());
        assert!(g.pads.next_change().is_none(), "no wire: no edges tracked");
    }

    #[test]
    fn parse_pad_names() {
        assert_eq!(
            RaGpt::parse_pad("GTIOC7A", "P603"),
            Some((14, "port6".to_string(), 3))
        );
        assert_eq!(
            RaGpt::parse_pad("GTIOC1B", "P406"),
            Some((3, "port4".to_string(), 6))
        );
        assert_eq!(
            RaGpt::parse_pad("GTIOC2A", "PB13"),
            Some((4, "portb".to_string(), 13))
        );
        assert_eq!(RaGpt::parse_pad("GTIOC14A", "P603"), None);
        assert_eq!(RaGpt::parse_pad("GTIOC1A", "P6"), None);
    }
}
