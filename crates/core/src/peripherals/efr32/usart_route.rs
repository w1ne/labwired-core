// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `GPIO_USARTROUTE[3]` — the pin-mux that decides which pad a USART's clock
//! and data reach.
//!
//! # Why this exists
//!
//! This window was a read-as-zero stub, and the cost was a twin that lied.
//! `SPI.transfer()` in the silabs-arduino core drove a device in simulation and
//! clocked NOTHING on a real BRD2709A, because the core never programmed a
//! route and on Series 2 a peripheral's signals reach no pad until it is
//! written. Every simulated SPI test passed against a dead bus. The pin
//! constants in that core's variant header were wrong too — SCK and MISO were
//! swapped and SS named a different pad entirely — and nothing noticed, because
//! with the route stubbed the SPI model reached its attached device whatever
//! the pins said.
//!
//! That is the same defect `gpio_route.rs` was written to fix for TIMER: an
//! `analogWrite` that "programmed the right duty into real TIMER1 CC_OC
//! registers and lit nothing". Same block, same cause, one instance fixed and
//! the other left stubbed.
//!
//! # Where the numbers come from
//!
//! EFR32xG26 Reference Manual Rev 1.0 section 24.6 p.879: `GPIO_USART0_ROUTEEN`
//! at block +0x820 with the stanza `ROUTEEN, CSROUTE, CTSROUTE, RTSROUTE,
//! RXROUTE, CLKROUTE, TXROUTE`, and USART1/USART2 following at +0x840 and
//! +0x860. `ROUTEEN` carries CSPEN 0, RTSPEN 1, RXPEN 2, CLKPEN 3, TXPEN 4
//! (p.1090); each route word is PORT in bits [1:0] and PIN in bits [19:16]
//! (p.1091-1093).
//!
//! # Verified on silicon
//!
//! On a connected BRD2709A over SWD, with USART0 in synchronous master mode and
//! this exact route programmed (RX=PC01, SCLK=PC03, TX=PC02, ROUTEEN=0x1C):
//! writing `0x00` to `USART0_TXDATA` leaves `GPIO_PORTC_DIN` bit 2 LOW, and
//! writing `0xFF` leaves it HIGH, with `STATUS.TXC` set both times. A byte
//! written to the USART physically moves the MIKROE_MOSI pad. That measurement
//! is what `efr32_usart_route_silicon` asserts the twin reproduces.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::peripherals::pad_claims::PadClaims;
use crate::SimResult;

/// Whether a peripheral's signals currently reach any pad.
///
/// ⚠️ THIS IS THE ENFORCEMENT. A route block that only claims pads still lets
/// the twin lie, because the SPI/UART/I2C models dispatch to their attached
/// devices without ever asking. The route model flips this flag; the
/// peripheral refuses to drive anything while it is false, which is what a real
/// EFR32 does — its signals reach no pin until ROUTEEN says so.
pub type RouteGate = Arc<AtomicBool>;

/// USART instances this block routes.
pub const USARTROUTE_COUNT: usize = 3;
/// Bytes per instance stanza (RM p.879: 0x820, 0x840, 0x860).
const USARTROUTE_STRIDE: u64 = 0x20;
/// Words per stanza, including the reserved tail.
const WORDS_PER_STANZA: usize = 8;

/// Word indices inside a stanza.
const W_ROUTEEN: usize = 0;
const W_RXROUTE: usize = 4;
const W_CLKROUTE: usize = 5;
const W_TXROUTE: usize = 6;

/// `ROUTEEN` enable bits (RM p.1090).
const ROUTEEN_RXPEN: u32 = 1 << 2;
const ROUTEEN_CLKPEN: u32 = 1 << 3;
const ROUTEEN_TXPEN: u32 = 1 << 4;

/// `xxROUTE.PORT`, bits [1:0].
const ROUTE_PORT_MASK: u32 = 0x3;
/// `xxROUTE.PIN`, bits [19:16].
const ROUTE_PIN_SHIFT: u32 = 16;
const ROUTE_PIN_MASK: u32 = 0xF;

/// The three signals this model routes, in the order they index a
/// [`SpiLineLevels`](crate::peripherals::spi::SpiLineLevels) cell: SCK, MOSI,
/// MISO. A USART's CLK is the SPI SCK, its TX is MOSI and its RX is MISO.
pub const SIGNALS_PER_USART: usize = 3;
/// `(word index, ROUTEEN bit)` for each signal, in `SpiLineLevels` line order.
const SIGNAL_REGS: [(usize, u32); SIGNALS_PER_USART] = [
    (W_CLKROUTE, ROUTEEN_CLKPEN),
    (W_TXROUTE, ROUTEEN_TXPEN),
    (W_RXROUTE, ROUTEEN_RXPEN),
];

/// The claim token for USART `u`, signal `s`.
///
/// ⚠️ BASED AT 64 TO CLEAR THE TIMER TOKENS. `cc_token` mints 0..=29 from
/// `timer * 3 + ch`, and both blocks claim pads in the SAME table on the same
/// bus. An overlapping token would make a timer's waveform and a USART's clock
/// indistinguishable to a pad — silently, because every value is in range.
pub const fn usart_token(usart: usize, signal: usize) -> u32 {
    64 + (usart as u32) * (SIGNALS_PER_USART as u32) + signal as u32
}

/// The GPIO block's USART route registers.
#[derive(Debug, serde::Serialize)]
pub struct Efr32s2UsartRoute {
    /// Stored verbatim so a read returns what firmware wrote.
    regs: [[u32; WORDS_PER_STANZA]; USARTROUTE_COUNT],
    #[serde(skip)]
    claims: Option<Arc<PadClaims>>,
    /// The pad each (usart, signal) currently holds, so a re-pointed route
    /// releases the old pad instead of leaking it.
    #[serde(skip)]
    held: [[Option<usize>; SIGNALS_PER_USART]; USARTROUTE_COUNT],
    /// Per-instance "my signals reach a pad" flags, read by the USART models.
    #[serde(skip)]
    gates: [Option<RouteGate>; USARTROUTE_COUNT],
}

impl Default for Efr32s2UsartRoute {
    fn default() -> Self {
        Self::new()
    }
}

impl Efr32s2UsartRoute {
    pub fn new() -> Self {
        Self {
            regs: [[0; WORDS_PER_STANZA]; USARTROUTE_COUNT],
            claims: None,
            held: [[None; SIGNALS_PER_USART]; USARTROUTE_COUNT],
            gates: [const { None }; USARTROUTE_COUNT],
        }
    }

    /// Hand this block the flag USART `usart` reads before it drives anything.
    pub fn install_gate(&mut self, usart: usize, gate: RouteGate) {
        if usart < USARTROUTE_COUNT {
            gate.store(self.is_routed(usart), Ordering::Relaxed);
            self.gates[usart] = Some(gate);
        }
    }

    /// True when this USART's CLK or TX currently owns a pad.
    ///
    /// RX alone is not enough to call it routed: a block that can only listen
    /// drives nothing, and "drives nothing" is precisely what the gate decides.
    pub fn is_routed(&self, usart: usize) -> bool {
        self.held[usart][0].is_some() || self.held[usart][1].is_some()
    }

    /// Join the shared pad-claim table. Config-build time only; without it this
    /// block still stores and reads back, and claims nothing.
    pub fn install_claims(&mut self, claims: Arc<PadClaims>) {
        self.claims = Some(claims);
    }

    fn index(offset: u64) -> Option<(usize, usize)> {
        let usart = (offset / USARTROUTE_STRIDE) as usize;
        if usart >= USARTROUTE_COUNT {
            return None;
        }
        Some((usart, ((offset % USARTROUTE_STRIDE) / 4) as usize))
    }

    /// Re-point every signal of `usart` at whatever its registers now name.
    ///
    /// ROUTEEN and the route words move a pad independently, and either alone
    /// is enough to change where the waveform goes — so any write into the
    /// stanza re-runs the whole thing.
    fn resync(&mut self, usart: usize) {
        let Some(claims) = self.claims.clone() else {
            return;
        };
        let routeen = self.regs[usart][W_ROUTEEN];
        for (signal, (word, enable_bit)) in SIGNAL_REGS.iter().enumerate() {
            let enabled = routeen & enable_bit != 0;
            let route = self.regs[usart][*word];
            let want = enabled.then(|| {
                let port = (route & ROUTE_PORT_MASK) as u8;
                let pin = ((route >> ROUTE_PIN_SHIFT) & ROUTE_PIN_MASK) as u8;
                claims.pad_index(port, pin)
            });
            if want == self.held[usart][signal] {
                continue;
            }
            let token = usart_token(usart, signal);
            if let Some(previous) = self.held[usart][signal] {
                claims.release(previous, token);
            }
            if let Some(pad) = want {
                claims.take(pad, token);
            }
            self.held[usart][signal] = want;
        }
        if let Some(gate) = &self.gates[usart] {
            gate.store(self.is_routed(usart), Ordering::Relaxed);
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        Self::index(offset).map_or(0, |(u, w)| self.regs[u][w])
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let Some((u, w)) = Self::index(offset) else {
            return;
        };
        self.regs[u][w] = value;
        // ROUTEEN and the RX/CLK/TX words move pads; CS/CTS/RTS and the
        // reserved tail are stored and claim nothing (CS is driven in software
        // by a sketch on this board).
        if w == W_ROUTEEN || w == W_RXROUTE || w == W_CLKROUTE || w == W_TXROUTE {
            self.resync(u);
        }
    }
}

impl crate::Peripheral for Efr32s2UsartRoute {
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

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset & !3, value);
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
    use crate::Peripheral;

    fn routed() -> (Efr32s2UsartRoute, Arc<PadClaims>) {
        // EFR32 Series 2: four ports of sixteen pads.
        let claims = Arc::new(PadClaims::new(4, 16));
        let mut r = Efr32s2UsartRoute::new();
        r.install_claims(claims.clone());
        (r, claims)
    }

    /// The exact sequence measured on a BRD2709A over SWD.
    fn program_mikrobus_spi(r: &mut Efr32s2UsartRoute) {
        r.write_u32(0x10, 0x0001_0002).unwrap(); // RXROUTE  <- PC01
        r.write_u32(0x14, 0x0003_0002).unwrap(); // CLKROUTE -> PC03
        r.write_u32(0x18, 0x0002_0002).unwrap(); // TXROUTE  -> PC02
        r.write_u32(0x00, 0x0000_001C).unwrap(); // TXPEN | CLKPEN | RXPEN
    }

    #[test]
    fn the_registers_read_back_what_firmware_wrote() {
        let (mut r, _c) = routed();
        program_mikrobus_spi(&mut r);
        assert_eq!(r.read_u32(0x10).unwrap(), 0x0001_0002);
        assert_eq!(r.read_u32(0x14).unwrap(), 0x0003_0002);
        assert_eq!(r.read_u32(0x18).unwrap(), 0x0002_0002);
        assert_eq!(r.read_u32(0x00).unwrap(), 0x0000_001C);
    }

    /// ⚠️ THE WHOLE POINT. Before this model the window was a stub, so nothing
    /// distinguished a routed USART from an unrouted one and a sketch that
    /// never wrote a route still drove its device in the twin.
    #[test]
    fn an_unrouted_usart_claims_no_pad() {
        let (_r, claims) = routed();
        for pin in 0..16u8 {
            assert_eq!(claims.selector(2, pin), None, "port C pin {pin}");
        }
    }

    #[test]
    fn a_routed_usart_claims_exactly_the_pads_ug594_names() {
        let (mut r, claims) = routed();
        program_mikrobus_spi(&mut r);
        // SCK on PC03, MOSI on PC02, MISO on PC01 — UG594 Table 3.1 p.10.
        assert_eq!(
            claims.selector(2, 3),
            Some(usart_token(0, 0)),
            "SCK -> PC03"
        );
        assert_eq!(
            claims.selector(2, 2),
            Some(usart_token(0, 1)),
            "MOSI -> PC02"
        );
        assert_eq!(
            claims.selector(2, 1),
            Some(usart_token(0, 2)),
            "MISO -> PC01"
        );
        // And nothing else on the port is claimed.
        for pin in [0u8, 4, 5, 6, 7] {
            assert_eq!(claims.selector(2, pin), None, "port C pin {pin}");
        }
    }

    /// ROUTEEN alone gates it: the route words can name a pad while the signal
    /// stays disabled, which is the state every USART is in at reset.
    #[test]
    fn a_route_word_without_its_enable_bit_claims_nothing() {
        let (mut r, claims) = routed();
        r.write_u32(0x14, 0x0003_0002).unwrap(); // CLKROUTE -> PC03
        assert_eq!(claims.selector(2, 3), None, "CLKPEN is still clear");
        r.write_u32(0x00, ROUTEEN_CLKPEN).unwrap();
        assert_eq!(claims.selector(2, 3), Some(usart_token(0, 0)));
    }

    /// Re-pointing a route must RELEASE the old pad, or the first pin a signal
    /// ever touched stays claimed forever and two pads report one driver.
    #[test]
    fn re_pointing_a_route_releases_the_pad_it_left() {
        let (mut r, claims) = routed();
        program_mikrobus_spi(&mut r);
        assert_eq!(claims.selector(2, 3), Some(usart_token(0, 0)));
        r.write_u32(0x14, 0x0007_0002).unwrap(); // CLKROUTE -> PC07
        assert_eq!(claims.selector(2, 3), None, "PC03 must be given up");
        assert_eq!(claims.selector(2, 7), Some(usart_token(0, 0)));
    }

    /// Clearing ROUTEEN takes the pads back — a firmware that hands SPI's pins
    /// back to GPIO must actually get them.
    #[test]
    fn clearing_routeen_releases_every_pad() {
        let (mut r, claims) = routed();
        program_mikrobus_spi(&mut r);
        r.write_u32(0x00, 0).unwrap();
        for pin in 0..16u8 {
            assert_eq!(claims.selector(2, pin), None, "port C pin {pin}");
        }
    }

    /// ⚠️ The three USARTs are separate stanzas 0x20 apart, and this board uses
    /// USART0 for the panel and USART2 for the microphone. A stride mistake
    /// would silently route one instance's clock from another's registers.
    #[test]
    fn each_usart_has_its_own_stanza() {
        let (mut r, claims) = routed();
        // USART2 (+0x40) CLK -> PA04, which is where the deck's mic clock goes.
        r.write_u32(0x40 + 0x14, 0x0004_0000).unwrap();
        r.write_u32(0x40, ROUTEEN_CLKPEN).unwrap(); // USART2 ROUTEEN
        assert_eq!(claims.selector(0, 4), Some(usart_token(2, 0)), "PA04");
        // USART0 untouched.
        assert_eq!(r.read_u32(0x14).unwrap(), 0);
        assert_eq!(claims.selector(2, 3), None);
    }

    /// ⚠️ THE ENFORCEMENT FLAG ITSELF. Claiming a pad is not enough — the
    /// USART model has to be told, or it goes on driving its device anyway,
    /// which is the whole defect this change exists to remove.
    #[test]
    fn the_gate_follows_the_route() {
        let (mut r, _c) = routed();
        let gate: RouteGate = Arc::new(AtomicBool::new(false));
        r.install_gate(0, gate.clone());
        assert!(!gate.load(Ordering::Relaxed), "unrouted at reset");

        program_mikrobus_spi(&mut r);
        assert!(
            gate.load(Ordering::Relaxed),
            "routed once ROUTEEN names pads"
        );

        r.write_u32(0x00, 0).unwrap();
        assert!(
            !gate.load(Ordering::Relaxed),
            "clearing ROUTEEN closes the gate"
        );
    }

    /// RX alone drives nothing, so it must not open the gate.
    #[test]
    fn a_receive_only_route_does_not_count_as_driving() {
        let (mut r, _c) = routed();
        let gate: RouteGate = Arc::new(AtomicBool::new(false));
        r.install_gate(0, gate.clone());
        r.write_u32(0x10, 0x0001_0002).unwrap(); // RXROUTE
        r.write_u32(0x00, ROUTEEN_RXPEN).unwrap();
        assert!(!gate.load(Ordering::Relaxed), "RX alone is not driving");
    }

    /// ⚠️ Tokens must not collide with the TIMER block's, which claims pads in
    /// the SAME table. `cc_token` mints 0..=29.
    #[test]
    fn usart_tokens_clear_the_timer_tokens() {
        use crate::peripherals::efr32::gpio_route::cc_token;
        let timer_max = cc_token(9, 2);
        assert!(
            usart_token(0, 0) > timer_max,
            "usart tokens must start above the timer range (max {timer_max})"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────
// I2C
// ─────────────────────────────────────────────────────────────────────────

/// I2C instances this block routes (I2C0..3).
pub const I2CROUTE_COUNT: usize = 4;
/// Bytes per instance stanza — RM section 24.6 p.875: 0x528, 0x538, 0x548,
/// 0x558.
const I2CROUTE_STRIDE: u64 = 0x10;
const I2C_WORDS: usize = 4;

const I2C_W_ROUTEEN: usize = 0;
const I2C_W_SCL: usize = 1;
const I2C_W_SDA: usize = 2;

/// `GPIO_I2Cn_ROUTEEN` (RM p.1009): SCLPEN bit 0, SDAPEN bit 1.
const I2C_ROUTEEN_SCLPEN: u32 = 1 << 0;
const I2C_ROUTEEN_SDAPEN: u32 = 1 << 1;

/// Signals per I2C: SCL then SDA.
pub const SIGNALS_PER_I2C: usize = 2;

/// The claim token for I2C `i`, signal `s`.
///
/// Based well clear of both the timer tokens (0..=29) and the USART tokens
/// (64..), for the same reason those clear each other: one pad-claim table.
pub const fn i2c_token(i2c: usize, signal: usize) -> u32 {
    128 + (i2c as u32) * (SIGNALS_PER_I2C as u32) + signal as u32
}

/// The GPIO block's I2C route registers.
///
/// ⚠️ THIS WINDOW WAS NOT MAPPED AT ALL, which is a quieter version of the
/// USART stub above: `firmware-mg26-i2c` could not have routed its pins even
/// if it wanted to, because a write there faulted the bus — and the I2C model
/// reached its device anyway, so the lab passed. On silicon an unrouted I2C
/// drives neither SCL nor SDA.
#[derive(Debug, serde::Serialize)]
pub struct Efr32s2I2cRoute {
    regs: [[u32; I2C_WORDS]; I2CROUTE_COUNT],
    #[serde(skip)]
    claims: Option<Arc<PadClaims>>,
    #[serde(skip)]
    held: [[Option<usize>; SIGNALS_PER_I2C]; I2CROUTE_COUNT],
    #[serde(skip)]
    gates: [Option<RouteGate>; I2CROUTE_COUNT],
}

impl Default for Efr32s2I2cRoute {
    fn default() -> Self {
        Self::new()
    }
}

impl Efr32s2I2cRoute {
    pub fn new() -> Self {
        Self {
            regs: [[0; I2C_WORDS]; I2CROUTE_COUNT],
            claims: None,
            held: [[None; SIGNALS_PER_I2C]; I2CROUTE_COUNT],
            gates: [const { None }; I2CROUTE_COUNT],
        }
    }

    pub fn install_claims(&mut self, claims: Arc<PadClaims>) {
        self.claims = Some(claims);
    }

    pub fn install_gate(&mut self, i2c: usize, gate: RouteGate) {
        if i2c < I2CROUTE_COUNT {
            gate.store(self.is_routed(i2c), Ordering::Relaxed);
            self.gates[i2c] = Some(gate);
        }
    }

    /// An I2C bus needs BOTH wires. One alone cannot carry a transfer, so
    /// "routed" means SCL and SDA, not either.
    pub fn is_routed(&self, i2c: usize) -> bool {
        self.held[i2c][0].is_some() && self.held[i2c][1].is_some()
    }

    fn index(offset: u64) -> Option<(usize, usize)> {
        let i2c = (offset / I2CROUTE_STRIDE) as usize;
        if i2c >= I2CROUTE_COUNT {
            return None;
        }
        Some((i2c, ((offset % I2CROUTE_STRIDE) / 4) as usize))
    }

    fn resync(&mut self, i2c: usize) {
        let Some(claims) = self.claims.clone() else {
            return;
        };
        let routeen = self.regs[i2c][I2C_W_ROUTEEN];
        for (signal, (word, enable_bit)) in [
            (I2C_W_SCL, I2C_ROUTEEN_SCLPEN),
            (I2C_W_SDA, I2C_ROUTEEN_SDAPEN),
        ]
        .iter()
        .enumerate()
        {
            let enabled = routeen & enable_bit != 0;
            let route = self.regs[i2c][*word];
            let want = enabled.then(|| {
                let port = (route & ROUTE_PORT_MASK) as u8;
                let pin = ((route >> ROUTE_PIN_SHIFT) & ROUTE_PIN_MASK) as u8;
                claims.pad_index(port, pin)
            });
            if want == self.held[i2c][signal] {
                continue;
            }
            let token = i2c_token(i2c, signal);
            if let Some(previous) = self.held[i2c][signal] {
                claims.release(previous, token);
            }
            if let Some(pad) = want {
                claims.take(pad, token);
            }
            self.held[i2c][signal] = want;
        }
        if let Some(gate) = &self.gates[i2c] {
            gate.store(self.is_routed(i2c), Ordering::Relaxed);
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        Self::index(offset).map_or(0, |(i, w)| self.regs[i][w])
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let Some((i, w)) = Self::index(offset) else {
            return;
        };
        self.regs[i][w] = value;
        if w <= I2C_W_SDA {
            self.resync(i);
        }
    }
}

impl crate::Peripheral for Efr32s2I2cRoute {
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

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_word(offset & !3, value);
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

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod i2c_tests {
    use super::*;
    use crate::Peripheral;

    fn routed() -> (Efr32s2I2cRoute, Arc<PadClaims>) {
        let claims = Arc::new(PadClaims::new(4, 16));
        let mut r = Efr32s2I2cRoute::new();
        r.install_claims(claims.clone());
        (r, claims)
    }

    /// BRD2709A's Qwiic/mikroBUS I2C: SCL on PC05, SDA on PC07 (UG594 Table
    /// 3.1 p.10, pads 11 and 9).
    fn program_qwiic(r: &mut Efr32s2I2cRoute) {
        r.write_u32(0x04, 0x0005_0002).unwrap(); // SCL -> PC05
        r.write_u32(0x08, 0x0007_0002).unwrap(); // SDA -> PC07
        r.write_u32(0x00, I2C_ROUTEEN_SCLPEN | I2C_ROUTEEN_SDAPEN)
            .unwrap();
    }

    #[test]
    fn a_routed_i2c_claims_the_kits_own_pads() {
        let (mut r, claims) = routed();
        program_qwiic(&mut r);
        assert_eq!(claims.selector(2, 5), Some(i2c_token(0, 0)), "SCL -> PC05");
        assert_eq!(claims.selector(2, 7), Some(i2c_token(0, 1)), "SDA -> PC07");
    }

    #[test]
    fn an_unrouted_i2c_claims_nothing() {
        let (_r, claims) = routed();
        for pin in 0..16u8 {
            assert_eq!(claims.selector(2, pin), None);
        }
    }

    /// ⚠️ ONE WIRE IS NOT A BUS. A transfer needs both, so half a route must
    /// leave the gate shut — otherwise firmware that enabled only SCL would
    /// pass here and stall on the bench.
    #[test]
    fn half_a_route_does_not_open_the_gate() {
        let (mut r, _c) = routed();
        let gate: RouteGate = Arc::new(AtomicBool::new(false));
        r.install_gate(0, gate.clone());
        r.write_u32(0x04, 0x0005_0002).unwrap();
        r.write_u32(0x00, I2C_ROUTEEN_SCLPEN).unwrap();
        assert!(!gate.load(Ordering::Relaxed), "SCL alone is not a bus");
        r.write_u32(0x08, 0x0007_0002).unwrap();
        r.write_u32(0x00, I2C_ROUTEEN_SCLPEN | I2C_ROUTEEN_SDAPEN)
            .unwrap();
        assert!(gate.load(Ordering::Relaxed), "both wires make a bus");
    }

    #[test]
    fn each_i2c_has_its_own_stanza() {
        let (mut r, claims) = routed();
        // I2C1 (+0x10) SCL -> PB01.
        r.write_u32(0x10 + 0x04, 0x0001_0001).unwrap();
        r.write_u32(0x10, I2C_ROUTEEN_SCLPEN).unwrap();
        assert_eq!(claims.selector(1, 1), Some(i2c_token(1, 0)));
        assert_eq!(r.read_u32(0x04).unwrap(), 0, "I2C0 untouched");
    }

    #[test]
    fn i2c_tokens_clear_the_usart_and_timer_tokens() {
        assert!(i2c_token(0, 0) > usart_token(USARTROUTE_COUNT - 1, SIGNALS_PER_USART - 1));
    }
}
