// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `GPIO_TIMERROUTE[10]` — the pin-mux that decides which pad a TIMER's
//! compare/capture output reaches.
//!
//! # Why this exists
//!
//! `analogWrite` on BRD2709A programmed the right duty into real TIMER1 `CC_OC`
//! registers and lit nothing. The duty was correct, the timer was correct, and
//! the waveform reached no pad — because on Series 2 a peripheral output is
//! connected to a pin by the GPIO block's ROUTE registers, and this block was a
//! read-as-zero stub. Every description of that gap said "unmodelled" and
//! stopped there.
//!
//! # Where the numbers come from
//!
//! `GPIO_TypeDef` in `efr32mg26_gpio.h` (simplicity_sdk `sisdk-2025.6`), walked
//! member by member: `TIMERROUTE[10]` at block `+0x6E0`, stride `0x20`.
//!
//! ⚠️ That offset is worth one sentence of provenance, because a first attempt
//! at it was WRONG and looked fine — a parse that ran across several typedefs
//! sized `GPIO_I2CROUTE_TypeDef` at 27 words instead of 4 and put this block at
//! `+0x1018`. What makes `+0x6E0` believable is not the parser: the same walk
//! puts `USARTROUTE` at exactly the `+0x820` that `configs/chips/efr32mg26.yaml`
//! has used since onboarding, and sizes PORT/I2CROUTE/TIMERROUTE/USARTROUTE at
//! 12/4/8/8 words, each countable by eye in the header.
//!
//! Field encodings, from the same header:
//!   * `ROUTEEN.CCnPEN` — bit `n` (`_GPIO_TIMER_ROUTEEN_MASK` = 0x3F: three CC
//!     plus three CDTI).
//!   * `CCnROUTE.PORT` — bits [1:0] (`_GPIO_TIMER_CC0ROUTE_PORT_MASK` = 0x3).
//!   * `CCnROUTE.PIN` — bits [19:16] (`_GPIO_TIMER_CC0ROUTE_PIN_MASK` =
//!     0xF0000).
//!
//! # What is modelled
//!
//! The three CC channels per timer: enable bit, port, pin. Writes are stored
//! and read back, and each one re-points the pad claim so a route programmed
//! after the timer is already running takes effect immediately.
//!
//! NOT modelled: `CDTInROUTE` (dead-time insertion outputs — stored and read
//! back, claimed by nothing, because no TIMER model here drives them).

use std::sync::Arc;

use crate::peripherals::pad_claims::PadClaims;
use crate::SimResult;

/// `GPIO_TIMERROUTE[10]`, stride 0x20.
pub const TIMERROUTE_STRIDE: u64 = 0x20;
/// Timer instances the block routes. ⚠️ TEN, not the four a reader expects
/// from TIMER0..3 — this part has TIMER0..9.
pub const TIMERROUTE_COUNT: usize = 10;
/// CC channels per timer, the ones a TIMER model can drive.
pub const CC_PER_TIMER: usize = 3;

/// `ROUTEEN` is word 0 and `CC0..2ROUTE` are words 1..=3 of each timer's
/// stanza. Named here for the tests, which drive this block the way firmware
/// does — by offset.
#[cfg(test)]
const OFF_ROUTEEN: u64 = 0x00;
#[cfg(test)]
const OFF_CC0ROUTE: u64 = 0x04;

/// `CCnROUTE.PORT`, bits [1:0].
const ROUTE_PORT_MASK: u32 = 0x3;
/// `CCnROUTE.PIN`, bits [19:16].
const ROUTE_PIN_SHIFT: u32 = 16;
const ROUTE_PIN_MASK: u32 = 0xF;

/// The claim token for timer `t`, channel `ch`.
///
/// Stable and collision-free against the Nordic tokens, which are minted from
/// a different table on a different chip — one bus never holds both.
pub const fn cc_token(timer: usize, ch: usize) -> u32 {
    (timer as u32) * (CC_PER_TIMER as u32) + ch as u32
}

/// The GPIO block's TIMER route registers.
#[derive(Debug, serde::Serialize)]
pub struct Efr32s2TimerRoute {
    /// `[timer][0]` = ROUTEEN, `[timer][1..=3]` = CC0/1/2ROUTE, `[4..=6]` =
    /// CDTI0/1/2ROUTE, `[7]` = the reserved word. Stored verbatim so a read
    /// returns what firmware wrote.
    regs: [[u32; 8]; TIMERROUTE_COUNT],
    #[serde(skip)]
    claims: Option<Arc<PadClaims>>,
    /// The pad each (timer, channel) currently holds, so a re-pointed route
    /// releases the old pad instead of leaking it.
    #[serde(skip)]
    held: [[Option<usize>; CC_PER_TIMER]; TIMERROUTE_COUNT],
}

impl Default for Efr32s2TimerRoute {
    fn default() -> Self {
        Self::new()
    }
}

impl Efr32s2TimerRoute {
    pub fn new() -> Self {
        Self {
            regs: [[0; 8]; TIMERROUTE_COUNT],
            claims: None,
            held: [[None; CC_PER_TIMER]; TIMERROUTE_COUNT],
        }
    }

    /// Join the shared pad-claim table. Config-build time only; without it this
    /// block still stores and reads back, and claims nothing.
    pub fn install_claims(&mut self, claims: Arc<PadClaims>) {
        self.claims = Some(claims);
    }

    fn index(offset: u64) -> Option<(usize, usize)> {
        let timer = (offset / TIMERROUTE_STRIDE) as usize;
        if timer >= TIMERROUTE_COUNT {
            return None;
        }
        Some((timer, ((offset % TIMERROUTE_STRIDE) / 4) as usize))
    }

    /// Re-point every CC channel of `timer` at whatever its registers now name.
    ///
    /// Called after any write into that timer's block, because `ROUTEEN` and
    /// `CCnROUTE` can move a pad independently and either one alone is enough
    /// to change where the waveform goes.
    fn resync(&mut self, timer: usize) {
        let Some(claims) = self.claims.clone() else {
            return;
        };
        let routeen = self.regs[timer][0];
        for ch in 0..CC_PER_TIMER {
            let enabled = routeen & (1 << ch) != 0;
            let route = self.regs[timer][1 + ch];
            let want = enabled.then(|| {
                let port = (route & ROUTE_PORT_MASK) as u8;
                let pin = ((route >> ROUTE_PIN_SHIFT) & ROUTE_PIN_MASK) as u8;
                claims.pad_index(port, pin)
            });
            if want == self.held[timer][ch] {
                continue;
            }
            let token = cc_token(timer, ch);
            if let Some(previous) = self.held[timer][ch] {
                claims.release(previous, token);
            }
            if let Some(pad) = want {
                claims.take(pad, token);
            }
            self.held[timer][ch] = want;
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        Self::index(offset).map_or(0, |(t, w)| self.regs[t][w])
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let Some((t, w)) = Self::index(offset) else {
            return;
        };
        self.regs[t][w] = value;
        // ROUTEEN (word 0) and CC0..2ROUTE (words 1..=3) move pads; the CDTI
        // words and the reserved tail are stored and claim nothing.
        if w <= CC_PER_TIMER {
            self.resync(t);
        }
    }
}

impl crate::Peripheral for Efr32s2TimerRoute {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        Ok(((word >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) * 8;
        let merged = (self.read_word(reg) & !(0xFFu32 << shift)) | (u32::from(value) << shift);
        self.write_word(reg, merged);
        Ok(())
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// ⚠️ `as_any_mut`, and NOT `as_any`.
    ///
    /// The wiring pass hands this block the claim table through
    /// `downcast_mut`, and `Peripheral::as_any_mut` DEFAULTS TO `None`. The
    /// first version of this file implemented `as_any` only: everything
    /// compiled, every unit test passed, and `install_claims` was never called
    /// — so routes stored, claimed nothing, and `analogWrite` still lit no pad.
    /// The end-to-end gate in `efr32mg26_pwm_pad.rs` is what makes that
    /// impossible to ship again.
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    fn routed() -> (Efr32s2TimerRoute, Arc<PadClaims>) {
        let claims = Arc::new(PadClaims::new(4, 16));
        let mut r = Efr32s2TimerRoute::new();
        r.install_claims(claims.clone());
        (r, claims)
    }

    /// TIMER1 CC0 to PC08 — the route `analogWrite(LED_BUILTIN, …)` needs on
    /// BRD2709A, where LED0 is PC08.
    #[test]
    fn a_routed_channel_claims_the_pad_its_registers_name() {
        let (mut r, claims) = routed();
        let base = TIMERROUTE_STRIDE; // TIMER1
        assert_eq!(claims.selector(2, 8), None, "nothing claims PC08 yet");
        // CC0ROUTE: PORT=2 (C), PIN=8
        r.write_u32(base + OFF_CC0ROUTE, 2 | (8 << ROUTE_PIN_SHIFT))
            .unwrap();
        assert_eq!(
            claims.selector(2, 8),
            None,
            "a route with ROUTEEN clear drives nothing — the enable is not decorative",
        );
        r.write_u32(base + OFF_ROUTEEN, 1).unwrap();
        assert_eq!(claims.selector(2, 8), Some(cc_token(1, 0)));
    }

    /// The release path: a re-pointed route must not leave the old pad claimed,
    /// or a pad handed back to plain GPIO keeps reading the timer.
    #[test]
    fn re_pointing_a_route_releases_the_pad_it_held() {
        let (mut r, claims) = routed();
        let base = TIMERROUTE_STRIDE;
        r.write_u32(base + OFF_ROUTEEN, 1).unwrap();
        r.write_u32(base + OFF_CC0ROUTE, 2 | (8 << ROUTE_PIN_SHIFT))
            .unwrap();
        assert_eq!(claims.selector(2, 8), Some(cc_token(1, 0)));
        r.write_u32(base + OFF_CC0ROUTE, 2 | (9 << ROUTE_PIN_SHIFT))
            .unwrap();
        assert_eq!(claims.selector(2, 8), None, "the old pad is given back");
        assert_eq!(claims.selector(2, 9), Some(cc_token(1, 0)));
    }

    /// Clearing ROUTEEN gives the pad back too — firmware disabling a PWM
    /// channel expects the pin to become an ordinary GPIO again.
    #[test]
    fn clearing_the_enable_gives_the_pad_back() {
        let (mut r, claims) = routed();
        r.write_u32(OFF_ROUTEEN, 1).unwrap();
        r.write_u32(OFF_CC0ROUTE, 1 | (3 << ROUTE_PIN_SHIFT))
            .unwrap();
        assert_eq!(claims.selector(1, 3), Some(cc_token(0, 0)));
        r.write_u32(OFF_ROUTEEN, 0).unwrap();
        assert_eq!(claims.selector(1, 3), None);
    }

    /// Every timer gets its own token, or two timers routed to two pads would
    /// look like one signal to the port.
    #[test]
    fn tokens_are_unique_per_timer_and_channel() {
        let mut seen = std::collections::HashSet::new();
        for t in 0..TIMERROUTE_COUNT {
            for ch in 0..CC_PER_TIMER {
                assert!(seen.insert(cc_token(t, ch)), "collision at {t}/{ch}");
            }
        }
    }

    /// Registers read back what firmware wrote, including the CDTI words this
    /// model deliberately does not act on — a driver that reads-modifies-writes
    /// its own route must not lose bits.
    #[test]
    fn every_word_reads_back_what_was_written() {
        let (mut r, _) = routed();
        for w in 0..8u64 {
            let off = 2 * TIMERROUTE_STRIDE + w * 4;
            r.write_u32(off, 0x000A_0003).unwrap();
            assert_eq!(r.read_u32(off).unwrap(), 0x000A_0003, "word {w}");
        }
    }

    /// ⚠️ The block covers TIMER0..9. An offset past the tenth is not a valid
    /// route and must not alias onto one.
    #[test]
    fn an_offset_past_the_last_timer_decodes_to_nothing() {
        let (mut r, claims) = routed();
        let past = TIMERROUTE_STRIDE * TIMERROUTE_COUNT as u64;
        r.write_u32(past + OFF_ROUTEEN, 1).unwrap();
        r.write_u32(past + OFF_CC0ROUTE, 2 | (8 << ROUTE_PIN_SHIFT))
            .unwrap();
        assert_eq!(r.read_u32(past).unwrap(), 0);
        assert_eq!(claims.selector(2, 8), None);
    }
}
