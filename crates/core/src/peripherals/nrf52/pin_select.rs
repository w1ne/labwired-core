// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The nRF answer to "what signal is on pin N?" — asked of the PERIPHERAL,
//! not of the pad.
//!
//! # Why this family needs its own decode and not a fourth AF table
//!
//! The three wired families all mux at the PAD: an RP2040 pad names its
//! function in `GPIOn_CTRL.FUNCSEL`, an STM32 pad in its AF nibble, an ESP32
//! pad in `GPIO_FUNCn_OUT_SEL_CFG`. [`PadRoutes`](super::super::pad_routing)
//! asks that question of the pad and matches a bound route against the answer.
//!
//! Nordic inverts it. There is no per-pad function register anywhere in the
//! GPIO block — `DIR`, `OUT`, `IN` and `PIN_CNF[n]` are all the port has. Each
//! PERIPHERAL instead names the pin it claims, in its own `PSEL.*` registers:
//! TWIM has `PSEL.SCL` / `PSEL.SDA`, UARTE has `PSEL.TXD` / `PSEL.RXD` / …,
//! SPIM has `PSEL.SCK` / `PSEL.MOSI` / `PSEL.MISO`. Asking the pad is
//! impossible; the port genuinely does not know.
//!
//! So the peripherals publish, and the port reads — the same direction
//! [`PadLines`](super::super::pad_lines) already carries wire LEVELS in. This
//! module is the ROUTING half of that same one-way street: one shared table of
//! "which signal currently claims each pad", written by whichever peripheral
//! owns the `PSEL`, read by the GPIO port on a pad read.
//!
//! The claim is an opaque `u32` token, so it slots straight into
//! `PadRoutes::bind(cell, pin, Some(token), line, func)` with no change to the
//! shared seam: a family that mux'd at the pad supplies an AF nibble, this one
//! supplies a claim token, and `PadRoutes` does not care which.
//!
//! # Why the table is live and not a bind-once map
//!
//! `PSEL` is runtime-mutable, and firmware really does move a peripheral:
//! Zephyr's `pinctrl` writes `PSEL` from a devicetree state at init, and
//! `pinctrl_apply_state` can re-point the same instance at a different pad for
//! a sleep state. `CONNECT` (bit 31) is a defined DISCONNECTED state and is the
//! RESET value — every `PSEL` register on a cold chip reads `0xFFFF_FFFF`
//! (nRF52840 PS v1.11 §6.31.7.19/.20, p798). So a static table built at bus
//! construction would either claim every pad forever or claim none.
//!
//! Instead the routes are bound once (every pad × every signal — the silicon
//! really does allow any signal on any pad, which is the whole point of
//! `PSEL`), and this table decides which single one is live at each instant.
//! Re-pointing `PSEL` moves the claim on the very next pad read, and
//! `PadRoutes::sync_taps` re-registers push capture from the GPIO write hook
//! that follows.
//!
//! # What a claim is gated on
//!
//! Both `CONNECT` and `ENABLE`. The product spec is explicit that the pin
//! selection is conditional on the peripheral being enabled — nRF52840 PS
//! v1.11 §6.31.6 (TWIM, p790): "The `PSEL.SCL` and `PSEL.SDA` registers and
//! their configurations are only used as long as the TWI master is enabled …
//! When the peripheral is disabled, the pins will behave as regular GPIOs, and
//! use the configuration in their respective `OUT` bit field and `PIN_CNF[n]`
//! register." §6.34.8 (UARTE, p836) and §6.25.3 (SPIM, p726) say the same for
//! their `PSEL.n` registers. A disabled peripheral therefore releases its pads
//! and they fall back to the port's own register truth, which is exactly what
//! [`PadRoutes`] does when `selector_of` answers `None`.
//!
//! # Field layout (nRF52840 PS v1.11 §6.31.7.19 `PSEL.SCL`, p798)
//!
//! | bits | field | meaning |
//! |---|---|---|
//! | `[4:0]` | `PIN` | `[0..31]` pin number |
//! | `[5]` | `PORT` | `[0..1]` port number |
//! | `[31]` | `CONNECT` | `1` = Disconnected, `0` = Connected |
//!
//! Identical in every `PSEL.*` register on the part — TWIM (p798), SPIM
//! (p733), UARTE (p845–846), and the ones this module does not serve.
//!
//! ⚠️ VERIFIED ON THE nRF52840 ONLY. The nRF53/nRF54 generation re-bases its
//! GPIO ports (see [`GpioPort::window_offset`](super::super::gpio::GpioPort))
//! and no nRF5340 datasheet is in this checkout's corpus, so the wiring pass
//! deliberately declines to install this table on a port whose window is
//! offset. See `SystemBus::wire_nrf52_pads`.

use std::sync::Arc;

/// `CONNECT` = Disconnected. Bit 31 of every `PSEL.*` register, and its reset
/// value (nRF52840 PS v1.11 §6.31.7.19, p798).
const PSEL_DISCONNECTED: u32 = 1 << 31;
/// `PIN` field, bits `[4:0]`.
const PSEL_PIN_MASK: u32 = 0x1F;
/// `PORT` field, bit `[5]`.
const PSEL_PORT_SHIFT: u32 = 5;
const PSEL_PORT_MASK: u32 = 0x1;

/// Pins per nRF52 GPIO port. P0 has 32, P1 has 16 — but `PSEL.PIN` is 5 bits
/// on both, so the flat index is 32-strided regardless of how many pins the
/// port physically bonds out.
pub(crate) const PINS_PER_PORT: usize = 32;
/// Ports a `PSEL.PORT` field can name (`[0..1]`).
pub(crate) const PORTS: usize = 2;

/// The pad a `PSEL.*` word names, as a flat `port * 32 + pin` index, or `None`
/// when `CONNECT` reads Disconnected.
///
/// Deliberately total: a reserved bit set somewhere in `[30:6]` is ignored
/// rather than treated as a disconnect, because silicon decodes only the
/// fields it defines and firmware that leaves junk in a reserved bit still
/// gets its pin.
fn psel_pad(psel: u32) -> Option<usize> {
    if psel & PSEL_DISCONNECTED != 0 {
        return None;
    }
    let pin = (psel & PSEL_PIN_MASK) as usize;
    let port = ((psel >> PSEL_PORT_SHIFT) & PSEL_PORT_MASK) as usize;
    Some(port * PINS_PER_PORT + pin)
}

/// Which peripheral signal currently claims each pad on one nRF52 chip.
///
/// One shared instance per bus, held by every nRF52 GPIO port (which reads it)
/// and by every `PSEL`-owning peripheral (which writes it). Reads are lock-free
/// because a pad read runs on the CPU walk and must not contend with an MMIO
/// write on another peripheral.
/// The claim table, now shared. ⚠️ NOT a Nordic type any more: the EFR32
/// Series-2 route registers mux the same way (the peripheral names the pad),
/// so the mechanism moved to [`crate::peripherals::pad_claims`] and only the
/// PSEL ENCODING stayed here. The alias keeps every Nordic call site reading
/// the way it did.
pub type NrfPinClaims = crate::peripherals::pad_claims::PadClaims;

/// A Nordic table: two ports of 32 pads (PS v1.11 6.9, p143).
pub fn nrf_pin_claims() -> NrfPinClaims {
    NrfPinClaims::new(PORTS, PINS_PER_PORT)
}

/// One peripheral signal's standing claim on a pad — the handle a `PSEL`-owning
/// model keeps next to the `PSEL` register it mirrors.
///
/// Default-constructed as uninstalled, so a model carrying one on a bus with no
/// GPIO port to publish to costs a `None` check per register write and nothing
/// else.
#[derive(Debug, Default)]
pub struct NrfPinClaim {
    claims: Option<Arc<NrfPinClaims>>,
    /// Unique across the whole bus, per (peripheral instance, signal). Assigned
    /// by the wiring pass, which binds the SAME value into the GPIO ports'
    /// routing tables.
    token: u32,
    /// The pad this claim currently holds, so a re-pointed `PSEL` releases the
    /// old pad instead of leaking it.
    held: Option<usize>,
}

impl NrfPinClaim {
    /// Join the shared table under `token`. Called once, at bus wiring time.
    pub fn install(&mut self, claims: Arc<NrfPinClaims>, token: u32) {
        self.claims = Some(claims);
        self.token = token;
    }

    /// `true` once a GPIO port is listening — the branch a model uses to skip
    /// wire bookkeeping entirely on a bus with nothing to publish to.
    pub fn installed(&self) -> bool {
        self.claims.is_some()
    }

    /// Re-point this signal at whatever `psel` now names, releasing whatever it
    /// held before. `live` is the peripheral's enable state: a disabled
    /// peripheral holds no pad at all (nRF52840 PS v1.11 §6.31.6, p790).
    ///
    /// Call after EVERY write that can change either — the `PSEL` register
    /// itself and `ENABLE`. Cheap and idempotent: when nothing moved it is two
    /// comparisons and no atomic traffic.
    pub fn update(&mut self, psel: u32, live: bool) {
        let Some(claims) = &self.claims else {
            return;
        };
        let want = if live { psel_pad(psel) } else { None };
        if want == self.held {
            return;
        }
        if let Some(previous) = self.held {
            claims.release(previous, self.token);
        }
        if let Some(pad) = want {
            claims.take(pad, self.token);
        }
        self.held = want;
    }
}

impl Drop for NrfPinClaim {
    /// A model torn off the bus must not leave a pad claimed by a signal that
    /// no longer exists — the shared table outlives it whenever a GPIO port
    /// still holds the `Arc`.
    fn drop(&mut self) {
        if let (Some(claims), Some(pad)) = (&self.claims, self.held) {
            claims.release(pad, self.token);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCL: u32 = 0;
    const SDA: u32 = 1;
    const TXD: u32 = 2;

    #[test]
    fn a_disconnected_psel_names_no_pad() {
        // The RESET value of every PSEL register on the part.
        assert_eq!(psel_pad(0xFFFF_FFFF), None);
        // CONNECT alone, with a perfectly valid pin underneath it.
        assert_eq!(psel_pad(0x8000_000B), None);
    }

    #[test]
    fn psel_decodes_pin_and_port_the_way_the_datasheet_lays_them_out() {
        assert_eq!(psel_pad(0), Some(0), "P0.00");
        assert_eq!(psel_pad(27), Some(27), "P0.27");
        assert_eq!(psel_pad(31), Some(31), "P0.31");
        // PORT is bit 5, so P1.00 is 0x20 — NOT pin 32 of one flat space.
        assert_eq!(psel_pad(0x20), Some(32), "P1.00");
        assert_eq!(psel_pad(0x2F), Some(47), "P1.15");
        // A reserved bit set in [30:6] must not read as a disconnect.
        assert_eq!(psel_pad(0x0040_000B), Some(11));
    }

    #[test]
    fn an_unclaimed_pad_answers_none_so_the_port_falls_back_to_its_latch() {
        let claims = nrf_pin_claims();
        assert_eq!(claims.selector(0, 3), None);
        // Out of range is None, never a panic: this runs on the CPU walk.
        assert_eq!(claims.selector(9, 3), None);
        assert_eq!(claims.selector(0, 200), None);
    }

    #[test]
    fn a_claim_follows_psel_and_is_released_when_the_peripheral_is_disabled() {
        let claims = Arc::new(nrf_pin_claims());
        let mut scl = NrfPinClaim::default();
        scl.install(claims.clone(), SCL);

        // PSEL written before ENABLE — the Zephyr pinctrl order. The pad is NOT
        // claimed yet, because the silicon only uses PSEL while enabled.
        scl.update(27, false);
        assert_eq!(claims.selector(0, 27), None);

        scl.update(27, true);
        assert_eq!(claims.selector(0, 27), Some(SCL));

        // Firmware disables the peripheral: the pad goes back to being a plain
        // GPIO (PS v1.11 §6.31.6).
        scl.update(27, false);
        assert_eq!(claims.selector(0, 27), None);
    }

    #[test]
    fn re_pointing_psel_at_runtime_moves_the_claim_and_frees_the_old_pad() {
        // The regression a bind-once table cannot express: pinctrl swapping an
        // instance onto a different pad must not leave the first pad reading
        // the wire forever.
        let claims = Arc::new(nrf_pin_claims());
        let mut sda = NrfPinClaim::default();
        sda.install(claims.clone(), SDA);

        sda.update(26, true);
        assert_eq!(claims.selector(0, 26), Some(SDA));

        sda.update(0x20 | 4, true); // P1.04
        assert_eq!(claims.selector(0, 26), None, "the old pad is a GPIO again");
        assert_eq!(claims.selector(1, 4), Some(SDA));

        sda.update(0xFFFF_FFFF, true); // CONNECT = Disconnected
        assert_eq!(claims.selector(1, 4), None);
    }

    #[test]
    fn releasing_does_not_steal_a_pad_a_second_peripheral_took() {
        // "Only one peripheral can be assigned to drive a particular GPIO pin
        // at a time" (PS §6.31.6) — but firmware CAN do it, and when the loser
        // later disconnects, the winner must keep its pad.
        let claims = Arc::new(nrf_pin_claims());
        let mut scl = NrfPinClaim::default();
        let mut txd = NrfPinClaim::default();
        scl.install(claims.clone(), SCL);
        txd.install(claims.clone(), TXD);

        scl.update(6, true);
        txd.update(6, true);
        assert_eq!(claims.selector(0, 6), Some(TXD), "last claim wins");

        scl.update(0xFFFF_FFFF, true);
        assert_eq!(
            claims.selector(0, 6),
            Some(TXD),
            "the loser's release must not evict the live claim",
        );
    }

    #[test]
    fn an_uninstalled_claim_is_inert() {
        let mut orphan = NrfPinClaim::default();
        assert!(!orphan.installed());
        orphan.update(11, true); // must not panic
    }

    #[test]
    fn dropping_a_model_frees_the_pad_it_held() {
        let claims = Arc::new(nrf_pin_claims());
        {
            let mut scl = NrfPinClaim::default();
            scl.install(claims.clone(), SCL);
            scl.update(11, true);
            assert_eq!(claims.selector(0, 11), Some(SCL));
        }
        assert_eq!(claims.selector(0, 11), None);
    }
}
