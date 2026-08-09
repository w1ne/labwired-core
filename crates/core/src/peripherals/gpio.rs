// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// ── Architectural separation ────────────────────────────────────────────────
// GPIO is one struct PER FAMILY behind the `GpioPort` enum. The STM32F1
// config registers (CRL/CRH), the STM32v2 registers (MODER/OTYPER/…/AFRH) and
// the nRF52 registers (DIR/PIN_CNF) each live ONLY in their own variant — a
// register from one family cannot exist on another. The chip-yaml `profile`
// selects the variant; the `Peripheral` impl and the `odr_offset`/`idr_offset`
// bus helpers dispatch to the active family.

use crate::SimResult;
use std::str::FromStr;

/// A pad's electrical role, derived from the GPIO model's direction/mode
/// registers (never fabricated). `Unknown` is returned where a family's model
/// cannot decide. Serialized lowercase for the `pin_routing` wasm export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GpioMode {
    Input,
    Output,
    /// Pad handed to a peripheral (alternate function / routed via a GPIO matrix).
    Af,
    Analog,
    Unknown,
}

/// Routing metadata for one GPIO pad: its [`GpioMode`] plus, when the model can
/// resolve it, the peripheral signal `func` name (`"I2CEXT0_SDA"`, `"AF4"`, …).
/// `func` is `None` when the model cannot name the signal — null over a guess.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GpioRouting {
    pub mode: GpioMode,
    pub func: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GpioRegisterLayout {
    #[default]
    Stm32F1,
    Stm32V2,
    Nrf52,
    /// NXP Kinetis (KW41Z GPIOA/B/C): PDOR @0x0 (output), PSOR/PCOR/PTOR
    /// set/clear/toggle, PDIR @0x10 (input), PDDR @0x14 (direction).
    Kinetis,
}

impl FromStr for GpioRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            "stm32v2" | "v2" | "modern" | "stm32-modern" | "h5" | "stm32h5" => Ok(Self::Stm32V2),
            "nrf52" | "nordic" => Ok(Self::Nrf52),
            "kinetis" | "kw41z" | "nxp" => Ok(Self::Kinetis),
            _ => Err(format!(
                "unsupported GPIO register layout '{}'; supported: stm32f1, stm32v2, nrf52, kinetis",
                value
            )),
        }
    }
}

// ── STM32F1 (CRL/CRH config registers) ───────────────────────────────────────
#[derive(Debug, Default, serde::Serialize)]
pub struct F1Gpio {
    crl: u32,  // 0x00
    crh: u32,  // 0x04
    idr: u32,  // 0x08
    odr: u32,  // 0x0C
    lckr: u32, // 0x18
}

impl F1Gpio {
    fn new() -> Self {
        // Reset value: floating input on every pin.
        Self {
            crl: 0x4444_4444,
            crh: 0x4444_4444,
            ..Default::default()
        }
    }
    /// Mask of pins configured as an output (push-pull or open-drain).
    ///
    /// CRL covers pins 0..7, CRH pins 8..15; each pin owns a nibble whose low
    /// two bits are MODE (00 = input, non-zero = output at some speed) and
    /// whose high two bits are CNF (RM0008 §9.2.1/9.2.2).
    fn output_mask(&self) -> u32 {
        let mut mask = 0u32;
        for pin in 0..16u32 {
            let cr = if pin < 8 { self.crl } else { self.crh };
            let nibble = (cr >> ((pin % 8) * 4)) & 0xF;
            if nibble & 0x3 != 0 {
                mask |= 1 << pin;
            }
        }
        mask
    }

    /// Mask of output pins in open-drain mode (CNF bit 2 set, i.e. nibble 0b01xx
    /// with MODE != 00 — RM0008 §9.2.1 table 20).
    fn open_drain_mask(&self) -> u32 {
        let mut mask = 0u32;
        for pin in 0..16u32 {
            let cr = if pin < 8 { self.crl } else { self.crh };
            let nibble = (cr >> ((pin % 8) * 4)) & 0xF;
            if nibble & 0x3 != 0 && nibble & 0x4 != 0 {
                mask |= 1 << pin;
            }
        }
        mask
    }

    /// IDR as silicon presents it: the *pin* level, not a separate latch.
    ///
    /// A push-pull output drives its pin, so reading IDR returns what ODR is
    /// driving. Returning a bare latch instead makes `digitalRead()` on an
    /// OUTPUT pin — one of the most common Arduino idioms — read 0 forever.
    /// Open-drain outputs only pull LOW; driving a 1 releases the pin, so the
    /// level is whatever the external world / pull-up decides, which is what
    /// the latched `idr` represents here.
    fn effective_idr(&self) -> u32 {
        let out = self.output_mask();
        let od = self.open_drain_mask();
        let push_pull = out & !od;
        // Open-drain pins read as driven only while ODR is 0.
        let od_driven_low = od & !self.odr;
        let driven = push_pull | od_driven_low;
        ((self.odr & push_pull) | (self.idr & !driven)) & 0xFFFF
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.crl,
            0x04 => self.crh,
            0x08 => self.effective_idr(),
            0x0C => self.odr,
            0x18 => self.lckr,
            _ => {
                crate::census_reg!("gpio:F1Gpio", offset, "read");
                0
            }
        }
    }
    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.crl = value,
            0x04 => self.crh = value,
            0x0C => self.odr = value & 0xFFFF,
            0x10 => {
                // BSRR: low 16 set, high 16 reset; BS has priority over BR.
                let set = value & 0xFFFF;
                let reset = (value >> 16) & 0xFFFF;
                self.odr &= !reset;
                self.odr |= set;
            }
            0x14 => {
                // BRR: reset selected ODR bits.
                self.odr &= !(value & 0xFFFF);
            }
            0x18 => self.lckr = value,
            _ => {
                crate::census_reg!("gpio:F1Gpio", offset, "write");
            }
        }
    }
}

// ── STM32v2 / H5-style (MODER/OTYPER/OSPEEDR/PUPDR/AFR) ───────────────────────
#[derive(Debug, Default, serde::Serialize)]
pub struct V2Gpio {
    moder: u32,   // 0x00
    otyper: u32,  // 0x04
    ospeedr: u32, // 0x08
    pupdr: u32,   // 0x0C
    idr: u32,     // 0x10
    odr: u32,     // 0x14
    lckr: u32,    // 0x1C
    afrl: u32,    // 0x20
    afrh: u32,    // 0x24
}

impl V2Gpio {
    /// Mask of pins whose MODER field selects general-purpose output (0b01).
    /// Two bits per pin (RM0368 §8.4.1).
    fn output_mask(&self) -> u32 {
        let mut mask = 0u32;
        for pin in 0..16u32 {
            if (self.moder >> (pin * 2)) & 0x3 == 0b01 {
                mask |= 1 << pin;
            }
        }
        mask
    }

    /// IDR as silicon presents it: the *pin* level, not a separate latch.
    ///
    /// A push-pull output drives its pin, so reading IDR returns what ODR is
    /// driving. Returning a bare latch instead makes `digitalRead()` on an
    /// OUTPUT pin — one of the most common Arduino idioms — read 0 forever.
    /// OTYPER bit set = open-drain: the pin is only driven while ODR is 0;
    /// a 1 releases it, leaving the level to the external world / pull-up,
    /// which is what the latched `idr` represents here.
    fn effective_idr(&self) -> u32 {
        let out = self.output_mask();
        let open_drain = out & self.otyper;
        let push_pull = out & !self.otyper;
        let od_driven_low = open_drain & !self.odr;
        let driven = push_pull | od_driven_low;
        ((self.odr & push_pull) | (self.idr & !driven)) & 0xFFFF
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.moder,
            0x04 => self.otyper,
            0x08 => self.ospeedr,
            0x0C => self.pupdr,
            0x10 => self.effective_idr(),
            0x14 => self.odr,
            0x1C => self.lckr,
            0x20 => self.afrl,
            0x24 => self.afrh,
            _ => {
                crate::census_reg!("gpio:V2Gpio", offset, "read");
                0
            }
        }
    }
    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.moder = value,
            0x04 => self.otyper = value & 0xFFFF,
            0x08 => self.ospeedr = value,
            0x0C => self.pupdr = value,
            0x10 => self.idr = value & 0xFFFF,
            0x14 => self.odr = value & 0xFFFF,
            0x18 => {
                // BSRR: low 16 set, high 16 reset; BS has priority over BR.
                let set = value & 0xFFFF;
                let reset = (value >> 16) & 0xFFFF;
                self.odr &= !reset;
                self.odr |= set;
            }
            0x1C => self.lckr = value,
            0x20 => self.afrl = value,
            0x24 => self.afrh = value,
            0x28 => {
                // BRR: reset selected ODR bits.
                self.odr &= !(value & 0xFFFF);
            }
            _ => {
                crate::census_reg!("gpio:V2Gpio", offset, "write");
            }
        }
    }
}

// ── nRF52 (DIR / OUT / IN / PIN_CNF) ──────────────────────────────────────────
#[derive(Debug, serde::Serialize)]
pub struct Nrf52Gpio {
    odr: u32,        // OUT        0x504
    idr: u32,        // IN         0x510 (latched input)
    dir: u32,        // DIR        0x514
    detectmode: u32, // DETECTMODE 0x524
    pin_cnf: [u32; 32],
    /// Number of physical pins on this port.  nRF52840 P0 = 32, P1 = 16.
    /// Writes to pins >= num_pins are discarded; reads return 0.
    num_pins: u32,
}

impl Default for Nrf52Gpio {
    fn default() -> Self {
        Self {
            odr: 0,
            idr: 0,
            dir: 0,
            detectmode: 0,
            pin_cnf: [0u32; 32],
            num_pins: 32,
        }
    }
}

impl Nrf52Gpio {
    /// Build a port with a non-default pin count (e.g. 16 for nRF52840 P1).
    fn with_num_pins(num_pins: u32) -> Self {
        Self {
            num_pins,
            ..Self::default()
        }
    }

    /// Bitmask covering the valid pins for this port.
    #[inline]
    fn pin_mask(&self) -> u32 {
        if self.num_pins >= 32 {
            0xFFFF_FFFF
        } else {
            (1u32 << self.num_pins) - 1
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x504 => self.odr,
            // IN reflects the physical pin level: output pins (DIR=1) track
            // OUT; input pins return the latched IDR. (Nordic PS §6.10.)
            0x510 => (self.odr & self.dir) | (self.idr & !self.dir),
            0x514 => self.dir,
            0x524 => self.detectmode,
            0x700..=0x77C if offset % 4 == 0 => {
                let k = ((offset - 0x700) / 4) as usize;
                if k < self.num_pins as usize {
                    self.pin_cnf[k]
                } else {
                    0
                }
            }
            _ => {
                crate::census_reg!("gpio:Nrf52Gpio", offset, "read");
                0
            }
        }
    }
    fn write_reg(&mut self, offset: u64, value: u32) {
        let mask = self.pin_mask();
        match offset {
            0x504 => self.odr = value & mask,
            0x508 => self.odr |= value & mask,
            0x50C => self.odr &= !(value & mask),
            0x510 => self.idr = value,
            0x514 => self.dir = value & mask,
            0x518 => self.dir |= value & mask,
            0x51C => self.dir &= !(value & mask),
            0x524 => self.detectmode = value,
            0x700..=0x77C if offset % 4 == 0 => {
                let k = ((offset - 0x700) / 4) as usize;
                if k < self.num_pins as usize {
                    self.pin_cnf[k] = value;
                    // PIN_CNF[n].DIR (bit 0) is the authoritative direction for
                    // pin n; the bulk DIR register mirrors those bits. Arduino
                    // / nrfx configure via PIN_CNF only — without this sync,
                    // pad_level (OUT∩DIR) never sees digitalWrite and LogicTap
                    // stays silent on nRF LEDs.
                    let bit = 1u32 << k;
                    if value & 1 != 0 {
                        self.dir |= bit & mask;
                    } else {
                        self.dir &= !bit;
                    }
                }
            }
            _ => {
                crate::census_reg!("gpio:Nrf52Gpio", offset, "write");
            }
        }
    }
}

// ── NXP Kinetis (KW41Z GPIOA/B/C) ────────────────────────────────────────────
// PDOR @0x0 (data output), PSOR @0x4 (set, w1s), PCOR @0x8 (clear, w1c),
// PTOR @0xC (toggle), PDIR @0x10 (data input), PDDR @0x14 (data direction).
#[derive(Debug, Default, serde::Serialize)]
pub struct KinetisGpio {
    pdor: u32, // 0x00 output
    pdir: u32, // 0x10 input
    pddr: u32, // 0x14 direction
}

impl KinetisGpio {
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.pdor,
            0x10 => self.pdir,
            0x14 => self.pddr,
            _ => {
                crate::census_reg!("gpio:KinetisGpio", offset, "read");
                0
            }
        }
    }
    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.pdor = value,   // PDOR
            0x04 => self.pdor |= value,  // PSOR: set 1s
            0x08 => self.pdor &= !value, // PCOR: clear 1s
            0x0C => self.pdor ^= value,  // PTOR: toggle
            0x14 => self.pddr = value,   // PDDR
            _ => {
                crate::census_reg!("gpio:KinetisGpio", offset, "write");
            }
        }
    }
}

/// The per-family register set of a [`GpioPort`]. Register sets are fully
/// isolated — a register from one family cannot exist on another.
#[derive(Debug, serde::Serialize)]
pub enum GpioFamily {
    Stm32F1(F1Gpio),
    Stm32V2(V2Gpio),
    Nrf52(Nrf52Gpio),
    Kinetis(KinetisGpio),
}

impl GpioFamily {
    fn read_reg(&self, offset: u64) -> u32 {
        match self {
            Self::Stm32F1(g) => g.read_reg(offset),
            Self::Stm32V2(g) => g.read_reg(offset),
            Self::Nrf52(g) => g.read_reg(offset),
            Self::Kinetis(g) => g.read_reg(offset),
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match self {
            Self::Stm32F1(g) => g.write_reg(offset, value),
            Self::Stm32V2(g) => g.write_reg(offset, value),
            Self::Nrf52(g) => g.write_reg(offset, value),
            Self::Kinetis(g) => g.write_reg(offset, value),
        }
    }

    /// Set the level the OUTSIDE WORLD holds on `pin` — a button contact, a
    /// sensor status line, a device driving a shared bus.
    ///
    /// This writes the input latch directly instead of going through
    /// [`write_reg`](Self::write_reg), because on silicon an input register is
    /// read-only: firmware storing to IDR must be ignored, and the F1 model
    /// correctly ignores it. Routing external input through the same MMIO store
    /// therefore silently did nothing on every STM32F1 pin — a button, keypad
    /// or BUSY line attached to an F1 could be "driven" with a successful
    /// return value and never move the pin. The latch is only what the world
    /// holds; [`read_reg`] still blends it with whatever an output pin drives.
    fn set_external_input(&mut self, pin: u8, level: bool) -> bool {
        if pin >= 32 {
            return false;
        }
        let apply = |idr: &mut u32| {
            if level {
                *idr |= 1u32 << pin;
            } else {
                *idr &= !(1u32 << pin);
            }
        };
        match self {
            Self::Stm32F1(g) => apply(&mut g.idr),
            Self::Stm32V2(g) => apply(&mut g.idr),
            Self::Nrf52(g) => apply(&mut g.idr),
            // Kinetis names its input latch PDIR.
            Self::Kinetis(g) => apply(&mut g.pdir),
        }
        true
    }

    /// Direction-aware pad level (the logic-probe truth). See
    /// [`crate::Peripheral::read_gpio_pad`]; kept on the family so the
    /// push-capture tap can read pre/post-write levels while the tap state is
    /// mutably borrowed.
    fn pad_level(&self, pin: u8) -> Option<bool> {
        if pin >= 32 {
            return None;
        }
        let bit = |reg: u32| (reg & (1u32 << pin)) != 0;
        match self {
            Self::Stm32F1(g) => {
                // CRL/CRH: 4 bits per pin — MODE!=0 is an output; CNF 10/11 on
                // an output pin hands the pad to a peripheral (AF), which this
                // model doesn't track at wire level.
                let cr = g.read_reg(if pin < 8 { 0x00 } else { 0x04 });
                let shift = ((pin % 8) * 4) as u32;
                let mode = (cr >> shift) & 0b11;
                let cnf = (cr >> (shift + 2)) & 0b11;
                if mode == 0 {
                    Some(bit(g.read_reg(0x08)))
                } else if cnf >= 0b10 {
                    None
                } else {
                    Some(bit(g.read_reg(0x0C)))
                }
            }
            Self::Stm32V2(g) => {
                // MODER: 00 input, 01 output, 10 alternate function (wire state
                // owned by the peripheral — unknown here), 11 analog.
                let mode = (g.read_reg(0x00) >> (pin * 2)) & 0b11;
                match mode {
                    0b01 => Some(bit(g.read_reg(0x14))),
                    0b10 => None,
                    _ => Some(bit(g.read_reg(0x10))),
                }
            }
            // The nRF IN read already mixes OUT-through-DIR with latched
            // inputs — it IS the pad view.
            Self::Nrf52(g) => Some(bit(g.read_reg(0x510))),
            Self::Kinetis(g) => {
                let dir = g.read_reg(0x14);
                Some(if (dir & (1u32 << pin)) != 0 {
                    bit(g.read_reg(0x00))
                } else {
                    bit(g.read_reg(0x10))
                })
            }
        }
    }
}

/// Push-mode logic-capture state for a [`GpioPort`]: the shared tap plus this
/// port's watched `(pin, channel)` pairs and a pre-write level scratchpad
/// (allocated once at install so the write hot path stays allocation-free).
/// `line_chs` caches, per wired SPI line cell, the channel lists last
/// registered with that cell (so registration is only re-synced when a write
/// actually changes a watched pad's routing) — the C3 GPIO pattern.
#[derive(Debug)]
struct PortTap {
    tap: crate::logic_capture::LogicTap,
    watched: Vec<(u8, u32)>,
    scratch: Vec<Option<bool>>,
}

/// GPIO port — a per-family register model (see [`GpioFamily`]) plus optional
/// push-mode logic-capture instrumentation. The chip-yaml `profile` selects
/// the family; the `Peripheral` impl and the `odr_offset`/`idr_offset` bus
/// helpers dispatch to the active family.
#[derive(Debug)]
pub struct GpioPort {
    family: GpioFamily,
    /// `Some` while the logic analyzer watches pads on this port in push mode
    /// (installed via `install_logic_tap`). Every register write then reports
    /// watched pad-level changes into the tap. Not snapshot state — the watch
    /// is re-armed by the frontend after a resume.
    tap: Option<PortTap>,
    /// Peripheral pad-line cells wired to this port (deduplicated), plus the
    /// pads routed to them. Installed once at config-build time; empty on buses
    /// with no AF-routed peripheral.
    pad_routes: crate::peripherals::pad_routing::PadRoutes,
    /// Offset of this port's MMIO window inside the family's register space,
    /// i.e. the SVD `addressBlock.offset`. Zero for every port whose window
    /// starts where its register map starts, which is all of them except the
    /// nRF53/nRF54 GPIO ports.
    ///
    /// Nordic describes an nRF52 GPIO port from the block start, with `OUT` at
    /// +0x504 and 0x500 of reserved space in front of it. From the nRF5340 on,
    /// the MDK and the SVD instead base a port at `OUT` (P0_S = 0x5084_2500,
    /// `OUT` at +0x004) and give it a 0x300 `addressBlock`. Same silicon, same
    /// absolute addresses — but the ports are only 0x300 apart, so a window
    /// anchored 0x500 low is necessarily 0x300 deep inside its neighbour's.
    /// That is not a fixable overlap: whichever anchor loses the router's
    /// greatest-start-wins tie-break has ALL of its registers served by the
    /// other port. Declaring the window where the vendor puts it and telling
    /// the model how far in it starts is the only arrangement in which both
    /// ports are addressable at once.
    window_offset: u64,
    /// nRF52 ONLY: the shared peripheral-side pin-claim table (see
    /// [`crate::peripherals::nrf52::pin_select`]) and this port's `PSEL.PORT`
    /// number.
    ///
    /// Every other family answers "who drives pad N" from its own registers, so
    /// this is `None` on them and costs one branch. Nordic has no such register
    /// — the peripherals name the pins — so the answer has to come from
    /// somewhere the port can reach, and this is it. Installed once at bus
    /// wiring time by `SystemBus::wire_nrf52_pads`; a port that never gets one
    /// simply has no routes bound either, and behaves exactly as before.
    nrf_claims: Option<(
        std::sync::Arc<crate::peripherals::nrf52::pin_select::NrfPinClaims>,
        u8,
    )>,
}

impl Default for GpioPort {
    fn default() -> Self {
        Self::new()
    }
}

impl GpioPort {
    fn from_family(family: GpioFamily) -> Self {
        Self {
            family,
            tap: None,
            pad_routes: crate::peripherals::pad_routing::PadRoutes::new(),
            window_offset: 0,
            nrf_claims: None,
        }
    }

    /// Anchor this port's MMIO window `offset` bytes into its register map.
    /// See [`GpioPort::window_offset`]. Chip yaml: `config: { reg_offset: … }`.
    pub fn with_window_offset(mut self, offset: u64) -> Self {
        self.window_offset = offset;
        self
    }

    pub fn new() -> Self {
        Self::new_with_layout(GpioRegisterLayout::Stm32F1)
    }

    pub fn new_with_layout(layout: GpioRegisterLayout) -> Self {
        Self::from_family(match layout {
            GpioRegisterLayout::Stm32F1 => GpioFamily::Stm32F1(F1Gpio::new()),
            GpioRegisterLayout::Stm32V2 => GpioFamily::Stm32V2(V2Gpio::default()),
            GpioRegisterLayout::Nrf52 => GpioFamily::Nrf52(Nrf52Gpio::default()),
            GpioRegisterLayout::Kinetis => GpioFamily::Kinetis(KinetisGpio::default()),
        })
    }

    /// Build an nRF52-layout GPIO port with an explicit pin count.
    /// Use this when the port has fewer than 32 physical pins (e.g. P1 = 16).
    pub fn new_nrf52(num_pins: u32) -> Self {
        Self::from_family(GpioFamily::Nrf52(Nrf52Gpio::with_num_pins(num_pins)))
    }

    /// Build a V2-layout GPIO port with explicit MODER/OSPEEDR/PUPDR reset
    /// values. On real silicon these are per-port (debug pins keep port A off
    /// the all-analog default; B carries the JTDO pull config; C..G reset to
    /// 0xFFFFFFFF analog). The chip yaml supplies them via
    /// `config: { reset_moder / reset_ospeedr / reset_pupdr }`.
    pub fn new_stm32v2_with_resets(moder: u32, ospeedr: u32, pupdr: u32) -> Self {
        Self::from_family(GpioFamily::Stm32V2(V2Gpio {
            moder,
            ospeedr,
            pupdr,
            ..Default::default()
        }))
    }

    /// Window-relative offset -> family register offset. The only place the
    /// two coordinate systems meet; every `Peripheral` entry point below goes
    /// through here, so a port with a non-zero window offset cannot be reached
    /// by one path and missed by another.
    fn read_reg(&self, offset: u64) -> u32 {
        self.family.read_reg(offset + self.window_offset)
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        self.family.write_reg(offset + self.window_offset, value);
    }

    /// Register offset of the output data register (ODR) for this family,
    /// relative to the port's MMIO window (so `base + odr_offset()` is an
    /// address the bus can route).
    /// Used by the bus to resolve a display's D/C line to a concrete address.
    pub fn odr_offset(&self) -> u64 {
        let family: u64 = match &self.family {
            GpioFamily::Stm32F1(_) => 0x0C,
            GpioFamily::Stm32V2(_) => 0x14,
            GpioFamily::Nrf52(_) => 0x504,
            GpioFamily::Kinetis(_) => 0x00,
        };
        family.saturating_sub(self.window_offset)
    }

    /// Register offset of the input data register (IDR) for this family,
    /// relative to the port's MMIO window (see [`GpioPort::odr_offset`]).
    /// Used by the bus to resolve a sensor's input line (e.g. HC-SR04 ECHO).
    pub fn idr_offset(&self) -> u64 {
        let family: u64 = match &self.family {
            GpioFamily::Stm32F1(_) => 0x08,
            GpioFamily::Stm32V2(_) => 0x10,
            GpioFamily::Nrf52(_) => 0x510,
            GpioFamily::Kinetis(_) => 0x10,
        };
        family.saturating_sub(self.window_offset)
    }

    /// Register layout of this port (used by the SPI pad-wiring helper to
    /// select the matching AF table).
    /// How far into its register map this port's MMIO window starts. See
    /// [`GpioPort::window_offset`].
    ///
    /// Read by `SystemBus::wire_nrf52_pads` as the structural marker of the
    /// nRF53/nRF54 GPIO generation: those parts base a port at `OUT` and
    /// declare `reg_offset: 0x500`, the nRF52 parts start at the block base and
    /// declare nothing. The PSEL field layout this engine decodes is verified
    /// on the nRF52840 only, so a port with an offset window is left unwired
    /// rather than routed on an assumption.
    pub(crate) fn window_offset(&self) -> u64 {
        self.window_offset
    }

    /// Hand this port the shared nRF52 pin-claim table and tell it which
    /// `PSEL.PORT` number it is. Config-build time only; see
    /// [`GpioPort::nrf_claims`].
    pub(crate) fn set_nrf_pin_claims(
        &mut self,
        claims: std::sync::Arc<crate::peripherals::nrf52::pin_select::NrfPinClaims>,
        port: u8,
    ) {
        self.nrf_claims = Some((claims, port));
    }

    pub(crate) fn register_layout(&self) -> GpioRegisterLayout {
        match &self.family {
            GpioFamily::Stm32F1(_) => GpioRegisterLayout::Stm32F1,
            GpioFamily::Stm32V2(_) => GpioRegisterLayout::Stm32V2,
            GpioFamily::Nrf52(_) => GpioRegisterLayout::Nrf52,
            GpioFamily::Kinetis(_) => GpioRegisterLayout::Kinetis,
        }
    }

    /// Install one AF pad route (config-build time; see
    /// [`crate::bus::SystemBus::wire_stm32_spi_pads`]).
    ///
    /// `line` indexes `cell`'s lines — for SPI that is `SpiSignal as usize`,
    /// for I²C the SCL/SDA order the controller declared. `af` is the AFR
    /// nibble the pad must select on V2 ports, or `None` on F1 whose mapping is
    /// fixed. Everything past this call is the shared seam's
    /// ([`crate::peripherals::pad_routing`]).
    pub(crate) fn add_pad_route(
        &mut self,
        cell: &std::sync::Arc<crate::peripherals::pad_lines::PadLines>,
        pin: u8,
        af: Option<u8>,
        line: usize,
        func: &'static str,
    ) {
        self.add_pad_route_selector(cell, pin, af.map(u32::from), line, func);
    }

    /// Install one pad route against a full-width selector.
    ///
    /// [`add_pad_route`](Self::add_pad_route) narrows to an AF NIBBLE because
    /// that is all an STM32 pad can select. The nRF52 selector is not a
    /// register field at all — it is a claim token minted by
    /// `SystemBus::wire_nrf52_pads`, one per (peripheral instance, signal), so
    /// it needs the whole `u32`. Both land in the same
    /// [`PadRoutes`](crate::peripherals::pad_routing) binding; only the width
    /// of the value differs.
    pub(crate) fn add_pad_route_selector(
        &mut self,
        cell: &std::sync::Arc<crate::peripherals::pad_lines::PadLines>,
        pin: u8,
        selector: Option<u32>,
        line: usize,
        func: &'static str,
    ) {
        self.pad_routes.bind(cell, pin, selector, line, func);
    }

    /// Every signal name bound to this port's pads, live or not — the
    /// bus-visibility reporting seam. See
    /// [`crate::peripherals::pad_routing::PadRoutes::bound_functions`] for why
    /// this is the static question and `func()` is the live one.
    pub(crate) fn bound_pad_functions(&self) -> Vec<&'static str> {
        self.pad_routes.bound_functions()
    }

    /// The alternate function `pin` currently selects, decoded from this
    /// family's own registers — the selector the shared routing seam resolves
    /// bindings against.
    ///
    /// V2: `MODER` must select AF, and the answer is the `AFRL`/`AFRH` nibble.
    /// F1: the pad is an AF output when `MODE != 0` and `CNF >= 0b10`; the
    /// mapping is fixed (no AFIO remap modelled) so the value carries no
    /// information and any bound route matches. F1 MISO — an input-mode pad on
    /// real silicon — is intentionally NOT routed, so a plain GPIO input on
    /// that pin never silently reads the SPI wire.
    ///
    /// nRF52: the port HAS no such register — Nordic muxes at the peripheral,
    /// which names its pin in `PSEL.*`. The answer therefore comes from the
    /// shared claim table the peripherals publish into
    /// ([`crate::peripherals::nrf52::pin_select`]), and the "selector" is a
    /// claim token rather than a decoded register field. Same shape, opposite
    /// direction; [`PadRoutes`](crate::peripherals::pad_routing) cannot tell.
    fn selected_function(
        family: &GpioFamily,
        nrf_claims: Option<&(
            std::sync::Arc<crate::peripherals::nrf52::pin_select::NrfPinClaims>,
            u8,
        )>,
        pin: u8,
    ) -> Option<u32> {
        match family {
            GpioFamily::Nrf52(_) => {
                let (claims, port) = nrf_claims?;
                claims.selector(*port, pin)
            }
            GpioFamily::Stm32V2(g) => {
                if (g.read_reg(0x00) >> (pin * 2)) & 0b11 != 0b10 {
                    return None;
                }
                let (afr_off, sh) = if pin < 8 {
                    (0x20, (pin * 4) as u32)
                } else {
                    (0x24, ((pin - 8) * 4) as u32)
                };
                Some((g.read_reg(afr_off) >> sh) & 0xF)
            }
            GpioFamily::Stm32F1(g) => {
                let cr = g.read_reg(if pin < 8 { 0x00 } else { 0x04 });
                let shift = ((pin % 8) * 4) as u32;
                let mode = (cr >> shift) & 0b11;
                let cnf = (cr >> (shift + 2)) & 0b11;
                (mode != 0 && cnf >= 0b10).then_some(0)
            }
            _ => None,
        }
    }

    /// Direction-aware pad level — the single truth `read_gpio_pad` and the
    /// push-capture tap both read. Pads whose MODER/AFR (or F1 CNF) route an
    /// alternate function report the live wire level from the shared
    /// [`PadLines`](crate::peripherals::pad_lines::PadLines) cell the owning
    /// peripheral publishes into; every other pad falls back to the family
    /// register truth.
    fn pad_level(&self, pin: u8) -> Option<bool> {
        if let Some(level) = self.pad_routes.level(pin, |p| {
            Self::selected_function(&self.family, self.nrf_claims.as_ref(), p)
        }) {
            return Some(level);
        }
        self.family.pad_level(pin)
    }

    /// Record every watched pad's current level before a mutation. No-op (one
    /// branch) while no tap is installed.
    #[inline]
    fn tap_snapshot(&mut self) {
        let Some(mut t) = self.tap.take() else {
            return;
        };
        for (k, &(pin, _)) in t.watched.iter().enumerate() {
            t.scratch[k] = self.pad_level(pin);
        }
        self.tap = Some(t);
    }

    /// Report watched pads whose level became known-different since the
    /// matching [`tap_snapshot`](Self::tap_snapshot), then re-sync the SPI
    /// line-cell registration if the write changed a watched pad's routing —
    /// so a pad handed to (or taken from) an SPI keeps pushing edges from the
    /// correct source afterwards. A pad whose level became UNknown reports
    /// nothing — same rule as the poll path, which keeps the last known level.
    #[inline]
    fn tap_report(&mut self) {
        let Some(t) = self.tap.take() else {
            return;
        };
        for (k, &(pin, ch)) in t.watched.iter().enumerate() {
            if let Some(level) = self.pad_level(pin) {
                if t.scratch[k] != Some(level) {
                    t.tap.push(ch, level);
                }
            }
        }
        self.tap = Some(t);
        self.sync_pad_line_taps();
    }

    /// Re-register watched pads with the wires that drive them, so a pad that
    /// changed hands keeps pushing edges from the correct source.
    fn sync_pad_line_taps(&mut self) {
        if self.pad_routes.is_empty() {
            return;
        }
        let Some(t) = self.tap.take() else {
            return;
        };
        // `family` is borrowed by the closure while `pad_routes` is borrowed
        // mutably; split the borrow by moving the routes out for the call.
        let mut routes = std::mem::take(&mut self.pad_routes);
        routes.sync_taps(&t.tap, &t.watched, |pin| {
            Self::selected_function(&self.family, self.nrf_claims.as_ref(), pin)
        });
        self.pad_routes = routes;
        self.tap = Some(t);
    }
}

impl crate::Peripheral for GpioPort {
    /// Not in the per-cycle walk: this model overrides neither `tick()` nor
    /// `tick_elapsed()`, so every visit ran the default no-op and returned a
    /// default `PeripheralTickResult`. Skipping it removes dispatch, never an
    /// effect — byte-identical by construction.
    ///
    /// Safe against the "sleeps and never wakes" trap: the bus calls
    /// `refresh_legacy_tick_index()` on every MMIO write, so if this model ever
    /// gains a tick and a state-dependent condition, a firmware write re-arms it.
    fn legacy_tick_active(&self) -> bool {
        false
    }
    // Inert walk: pure register + pad bank; pin edges are surfaced by the bus GPIO-diff pass, not tick().
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;

        if reg_offset == 0x0C {
            tracing::trace!("GPIO ODR Write: byte {} = {:#x}", byte_offset, value);
        }

        let mut reg_val = self.read_reg(reg_offset);
        let mask = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);

        self.tap_snapshot();
        self.write_reg(reg_offset, reg_val);
        self.tap_report();
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        // GPIO data registers are word-access. BSRR (atomic set/reset) only
        // behaves correctly when the whole 32-bit word is presented at once:
        // the default byte-decomposition would split BSRR's set half (low 16)
        // from its reset half (high 16) into separate write_reg calls, so a
        // pin named in both halves loses the BS-over-BR priority rule (set
        // wins). Silicon performs the STR as one 32-bit transaction; mirror
        // that by handing write_reg the full word. Silicon-verified on the
        // bench STM32F103 (stm32f1_exec_oracle::gpioa_bsrr_set_reset).
        self.tap_snapshot();
        self.write_reg(offset & !3, value);
        self.tap_report();
        Ok(())
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        if pin >= 32 {
            return None;
        }
        let reg = self.read_reg(self.idr_offset());
        Some((reg & (1u32 << pin)) != 0)
    }

    fn read_gpio_pad(&self, pin: u8) -> Option<bool> {
        self.pad_level(pin)
    }

    fn gpio_routing(&self, pin: u8) -> Option<GpioRouting> {
        if pin >= 32 {
            return None;
        }
        // Mode from the SAME register truth read_gpio_pad reads.
        let mode = match &self.family {
            GpioFamily::Stm32F1(g) => {
                // CRL/CRH: 4 bits/pin. MODE==0 → input (CNF 00 = analog, else
                // digital input); MODE!=0 → output, CNF 10/11 = alternate function.
                let cr = g.read_reg(if pin < 8 { 0x00 } else { 0x04 });
                let shift = ((pin % 8) * 4) as u32;
                let m = (cr >> shift) & 0b11;
                let cnf = (cr >> (shift + 2)) & 0b11;
                if m == 0 {
                    if cnf == 0b00 {
                        GpioMode::Analog
                    } else {
                        GpioMode::Input
                    }
                } else if cnf >= 0b10 {
                    GpioMode::Af
                } else {
                    GpioMode::Output
                }
            }
            GpioFamily::Stm32V2(g) => {
                // MODER: 00 input, 01 output, 10 alternate function, 11 analog.
                match (g.read_reg(0x00) >> (pin * 2)) & 0b11 {
                    0b00 => GpioMode::Input,
                    0b01 => GpioMode::Output,
                    0b10 => GpioMode::Af,
                    _ => GpioMode::Analog,
                }
            }
            // nRF52: a plain DIR register (@0x514) — bit set = output, clear =
            // input — and no AF field anywhere at the port. The alternate
            // function is nonetheless REAL here: a peripheral whose `PSEL.*`
            // names this pad owns it, and the port's DIR/OUT are not what drives
            // it. So the AF verdict comes from the claim table, exactly as
            // `pad_level` reads its level from there. nRF52840 PS v1.11 §6.31.6
            // (p790): while the peripheral is disabled "the pins will behave as
            // regular GPIOs" — which is the `None` branch below.
            GpioFamily::Nrf52(g) => {
                if Self::selected_function(&self.family, self.nrf_claims.as_ref(), pin).is_some() {
                    GpioMode::Af
                } else if (g.read_reg(0x514) & (1u32 << pin)) != 0 {
                    GpioMode::Output
                } else {
                    GpioMode::Input
                }
            }
            GpioFamily::Kinetis(g) => {
                if (g.read_reg(0x14) & (1u32 << pin)) != 0 {
                    GpioMode::Output
                } else {
                    GpioMode::Input
                }
            }
        };
        // func: a pad whose AF routing resolves to a wired peripheral signal
        // names it ("SPI1_SCK", "I2C1_SDA"); otherwise STM32 V2 exposes the raw
        // AFR nibble → "AF<n>" (no full AF→signal table; that is out of scope).
        // Everything else: None — null over a guess.
        let func = if mode == GpioMode::Af {
            if let Some(func) = self.pad_routes.func(pin, |p| {
                Self::selected_function(&self.family, self.nrf_claims.as_ref(), p)
            }) {
                Some(func.to_string())
            } else if let GpioFamily::Stm32V2(g) = &self.family {
                let (afr_off, sh) = if pin < 8 {
                    (0x20, (pin * 4) as u32)
                } else {
                    (0x24, ((pin - 8) * 4) as u32)
                };
                Some(format!("AF{}", (g.read_reg(afr_off) >> sh) & 0xF))
            } else {
                None
            }
        } else {
            None
        };
        Some(GpioRouting { mode, func })
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        if pin >= 32 {
            return None;
        }
        let reg = self.read_reg(self.odr_offset());
        Some((reg & (1u32 << pin)) != 0)
    }

    fn set_gpio_input(&mut self, pin: u8, level: bool) -> bool {
        self.tap_snapshot();
        let ok = self.family.set_external_input(pin, level);
        self.tap_report();
        ok
    }

    fn install_logic_tap(
        &mut self,
        tap: &crate::logic_capture::LogicTap,
        watched: &[(u8, u32)],
    ) -> bool {
        if watched.is_empty() {
            self.tap = None;
            self.pad_routes.clear_taps();
        } else {
            self.tap = Some(PortTap {
                tap: tap.clone(),
                watched: watched.to_vec(),
                scratch: vec![None; watched.len()],
            });
            // Seeded stale so the sync below always installs the current
            // routing into every wired line cell.
            self.pad_routes.invalidate_registrations();
            self.sync_pad_line_taps();
        }
        true
    }

    fn snapshot(&self) -> serde_json::Value {
        // Serialize the active family's register struct directly (flat), so the
        // snapshot keeps registers like `odr` at top level (no variant tag) —
        // matching the pre-split format the snapshot contract depends on.
        match &self.family {
            GpioFamily::Stm32F1(g) => serde_json::to_value(g),
            GpioFamily::Stm32V2(g) => serde_json::to_value(g),
            GpioFamily::Nrf52(g) => serde_json::to_value(g),
            GpioFamily::Kinetis(g) => serde_json::to_value(g),
        }
        .unwrap_or(serde_json::Value::Null)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod routing_tests {
    use super::{GpioMode, GpioPort, GpioRegisterLayout};
    use crate::Peripheral;

    #[test]
    // Zero-valued nibbles are kept explicit: each term documents one pin's slot
    // in the register layout the assertions below depend on.
    #[allow(clippy::identity_op)]
    fn stm32f1_routing_modes() {
        let mut g = GpioPort::new_with_layout(GpioRegisterLayout::Stm32F1);
        // CRL nibbles: pin0 = MODE01/CNF00 (output), pin1 = MODE01/CNF10 (AF),
        // pin2 = MODE00/CNF00 (analog input), pin3 = MODE00/CNF01 (float input).
        let crl = 0b0001 | (0b1001 << 4) | (0b0000 << 8) | (0b0100 << 12);
        g.write_u32(0x00, crl).unwrap();
        assert_eq!(g.gpio_routing(0).unwrap().mode, GpioMode::Output);
        let af = g.gpio_routing(1).unwrap();
        assert_eq!(af.mode, GpioMode::Af);
        assert!(af.func.is_none(), "F1 has no AF→signal index table");
        assert_eq!(g.gpio_routing(2).unwrap().mode, GpioMode::Analog);
        assert_eq!(g.gpio_routing(3).unwrap().mode, GpioMode::Input);
        assert!(g.gpio_routing(32).is_none(), "out-of-range pin");
    }

    #[test]
    // Zero-valued fields kept explicit — same rationale as stm32f1_routing_modes.
    #[allow(clippy::identity_op)]
    fn stm32v2_routing_modes_and_af_number() {
        let mut g = GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2);
        // MODER: pin0=01 output, pin1=10 AF, pin2=00 input, pin3=11 analog.
        g.write_u32(0x00, 0b01 | (0b10 << 2) | (0b00 << 4) | (0b11 << 6))
            .unwrap();
        // AFRL: pin1 nibble (bits 4..8) = 4 → "AF4".
        g.write_u32(0x20, 4 << 4).unwrap();
        assert_eq!(g.gpio_routing(0).unwrap().mode, GpioMode::Output);
        let af = g.gpio_routing(1).unwrap();
        assert_eq!(af.mode, GpioMode::Af);
        assert_eq!(af.func.as_deref(), Some("AF4"));
        assert_eq!(g.gpio_routing(2).unwrap().mode, GpioMode::Input);
        assert_eq!(g.gpio_routing(3).unwrap().mode, GpioMode::Analog);
    }

    #[test]
    fn nrf52_routing_from_dir() {
        let mut g = GpioPort::new_with_layout(GpioRegisterLayout::Nrf52);
        g.write_u32(0x514, 1 << 5).unwrap(); // DIR: pin5 output
        assert_eq!(g.gpio_routing(5).unwrap().mode, GpioMode::Output);
        assert!(g.gpio_routing(5).unwrap().func.is_none());
        assert_eq!(g.gpio_routing(6).unwrap().mode, GpioMode::Input);
    }

    #[test]
    fn nrf52_pin_cnf_dir_syncs_bulk_dir_and_pad() {
        let mut g = GpioPort::new_with_layout(GpioRegisterLayout::Nrf52);
        // Arduino/nrfx: DIR via PIN_CNF only, then OUTSET/OUTCLR.
        g.write_u32(0x700 + 13 * 4, 1).unwrap(); // PIN_CNF[13].DIR = Output
        assert_eq!(g.gpio_routing(13).unwrap().mode, GpioMode::Output);
        assert_eq!(g.read_gpio_pad(13), Some(false));
        g.write_u32(0x508, 1 << 13).unwrap(); // OUTSET pin 13
        assert_eq!(g.read_gpio_pad(13), Some(true));
        g.write_u32(0x50C, 1 << 13).unwrap(); // OUTCLR
        assert_eq!(g.read_gpio_pad(13), Some(false));
    }

    #[test]
    fn kinetis_routing_from_pddr() {
        let mut g = GpioPort::new_with_layout(GpioRegisterLayout::Kinetis);
        g.write_u32(0x14, 1 << 3).unwrap(); // PDDR: pin3 output
        assert_eq!(g.gpio_routing(3).unwrap().mode, GpioMode::Output);
        assert_eq!(g.gpio_routing(4).unwrap().mode, GpioMode::Input);
    }

    /// A window offset shifts EVERY door into the model by the same amount —
    /// the MMIO reads/writes and the `odr_offset`/`idr_offset` the bus uses to
    /// resolve a pin to an address. If those two disagreed, a pin-bound device
    /// would be wired to an address the register path never serves.
    #[test]
    fn window_offset_shifts_mmio_and_the_pin_address_helpers_together() {
        // nRF53/nRF54 anchor: the window starts at OUT, 0x500 into the map.
        let mut g = GpioPort::new_with_layout(GpioRegisterLayout::Nrf52).with_window_offset(0x500);

        assert_eq!(
            g.odr_offset(),
            0x004,
            "OUT is 0x504 - 0x500 into the window"
        );
        assert_eq!(g.idr_offset(), 0x010, "IN is 0x510 - 0x500 into the window");

        // PIN_CNF[7].DIR = Output, then OUTSET pin 7 — all at window-relative
        // offsets (0x700-0x500, 0x508-0x500).
        g.write_u32(0x200 + 7 * 4, 1).unwrap();
        g.write_u32(0x008, 1 << 7).unwrap();
        assert_eq!(g.read_gpio_pad(7), Some(true));
        assert_eq!(
            g.read_u32(g.odr_offset()).unwrap() & (1 << 7),
            1 << 7,
            "reading OUT through odr_offset() must see the same bit the MMIO \
             write set — the bus resolves pin addresses through this helper"
        );

        // And the un-offset default is untouched: same port, same registers,
        // at the block-start offsets every other Nordic part uses.
        let mut plain = GpioPort::new_with_layout(GpioRegisterLayout::Nrf52);
        assert_eq!(plain.odr_offset(), 0x504);
        plain.write_u32(0x700 + 7 * 4, 1).unwrap();
        plain.write_u32(0x508, 1 << 7).unwrap();
        assert_eq!(plain.read_gpio_pad(7), Some(true));
    }
}

#[cfg(test)]
mod tests {
    use super::{GpioPort, GpioRegisterLayout};
    use crate::Peripheral;

    /// Read a full 32-bit register via the byte interface.
    fn rd32(g: &GpioPort, off: u64) -> u32 {
        let b0 = g.read(off).unwrap() as u32;
        let b1 = g.read(off + 1).unwrap() as u32;
        let b2 = g.read(off + 2).unwrap() as u32;
        let b3 = g.read(off + 3).unwrap() as u32;
        b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
    }

    #[test]
    fn test_gpio_reset_values() {
        let gpio = GpioPort::new();
        assert_eq!(rd32(&gpio, 0x00), 0x4444_4444); // CRL
        assert_eq!(rd32(&gpio, 0x04), 0x4444_4444); // CRH
        assert_eq!(rd32(&gpio, 0x0C) & 0xFFFF, 0); // ODR
    }

    #[test]
    fn test_gpio_odr_write() {
        let mut gpio = GpioPort::new();
        gpio.write(0x0C, 0x55).unwrap(); // ODR byte 0
        gpio.write(0x0D, 0xAA).unwrap(); // ODR byte 1
        assert_eq!(rd32(&gpio, 0x0C) & 0xFFFF, 0xAA55);
    }

    #[test]
    fn test_gpio_bsrr_set() {
        let mut gpio = GpioPort::new();
        gpio.write(0x10, 0x01).unwrap(); // BSRR set pin 0
        assert_eq!(rd32(&gpio, 0x0C) & 0xFFFF, 0x0001);
    }

    #[test]
    fn test_gpio_bsrr_reset() {
        let mut gpio = GpioPort::new();
        gpio.write(0x0C, 0xFF).unwrap();
        gpio.write(0x0D, 0xFF).unwrap(); // ODR = 0xFFFF
        gpio.write(0x12, 0x01).unwrap(); // BSRR high half: reset pin 0
        assert_eq!(rd32(&gpio, 0x0C) & 0xFFFF, 0xFFFE);
    }

    #[test]
    fn test_gpio_bsrr_word_write_is_atomic_bs_priority() {
        // A whole-word BSRR write that names the same pin in both the set
        // (low 16) and reset (high 16) halves must apply BS-over-BR priority:
        // the pin ends up SET. The default byte-decomposition path would split
        // the two halves and let the reset clobber the set — silicon performs
        // one 32-bit transaction, so write_u32 must too.
        // Verified on bench STM32F103 (stm32f1_exec_oracle::gpioa_bsrr_set_reset).
        let mut gpio = GpioPort::new();
        // BSRR = 0x0010_0010 from ODR=0: BS pin4 + BR pin4 → pin4 SET.
        gpio.write_u32(0x10, 0x0010_0010).unwrap();
        assert_eq!(gpio.read_u32(0x0C).unwrap() & 0xFFFF, 0x0010);

        // BSRR = 0x00F0_000F from ODR=0x00FF: BR resets 4..7, BS sets 0..3.
        let mut g2 = GpioPort::new();
        g2.write_u32(0x10, 0x0000_00FF).unwrap(); // ODR = 0x00FF
        g2.write_u32(0x10, 0x00F0_000F).unwrap(); // → 0x000F
        assert_eq!(g2.read_u32(0x0C).unwrap() & 0xFFFF, 0x000F);
    }

    #[test]
    fn test_gpio_v2_moder_and_odr() {
        let mut gpio = GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2);
        // MODER @ 0x00
        gpio.write(0x00, 0xAA).unwrap();
        gpio.write(0x01, 0x55).unwrap();
        assert_eq!(rd32(&gpio, 0x00) & 0xFFFF, 0x55AA);
        // ODR @ 0x14
        gpio.write(0x14, 0x34).unwrap();
        gpio.write(0x15, 0x12).unwrap();
        assert_eq!(rd32(&gpio, 0x14) & 0xFFFF, 0x1234);
    }

    #[test]
    fn test_gpio_v2_bsrr_and_brr() {
        let mut gpio = GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2);
        // BSRR @ 0x18 (set pin 0, reset pin 1)
        gpio.write(0x18, 0x01).unwrap();
        gpio.write(0x1A, 0x02).unwrap();
        assert_eq!(rd32(&gpio, 0x14) & 0x0003, 0x0001);
        // BRR @ 0x28 (reset pin 0)
        gpio.write(0x28, 0x01).unwrap();
        assert_eq!(rd32(&gpio, 0x14) & 0x0001, 0x0000);
    }
}

#[cfg(test)]
mod idr_pin_level_tests {
    use super::{GpioPort, GpioRegisterLayout};
    use crate::Peripheral;

    fn v2() -> GpioPort {
        GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)
    }

    fn rd32(gpio: &GpioPort, off: u64) -> u32 {
        gpio.read_u32(off).unwrap()
    }

    /// On silicon, IDR reports the PIN level, not a private latch. A push-pull
    /// output drives its pin, so IDR must read back what ODR is driving.
    ///
    /// Regression test for a bug found by running the Arduino conformance
    /// sketch on STM32F401: `digitalRead()` on an OUTPUT pin read 0 forever,
    /// because V2Gpio returned its bare `idr` field. No Tier-1 fixture caught
    /// it — they all read back through ODR (0x14), which was always correct.
    #[test]
    fn v2_idr_reflects_push_pull_output_level() {
        let mut gpio = v2();

        // PA5 as general-purpose output (MODER 0b01), push-pull (OTYPER 0).
        gpio.write_u32(0x00, 0x1 << 10).unwrap();

        gpio.write_u32(0x18, 1 << 5).unwrap(); // BSRR set PA5
        assert_eq!(
            rd32(&gpio, 0x14) & (1 << 5),
            1 << 5,
            "ODR must latch the set"
        );
        assert_eq!(
            rd32(&gpio, 0x10) & (1 << 5),
            1 << 5,
            "IDR must report the driven HIGH level of a push-pull output"
        );

        gpio.write_u32(0x18, 1 << (5 + 16)).unwrap(); // BSRR reset PA5
        assert_eq!(
            rd32(&gpio, 0x10) & (1 << 5),
            0,
            "IDR must report the driven LOW level of a push-pull output"
        );
    }

    /// An open-drain output only pulls LOW. Driving a 1 releases the pin, so
    /// its level comes from the outside world rather than from ODR — the model
    /// must NOT mirror ODR in that direction.
    #[test]
    fn v2_idr_open_drain_only_mirrors_low() {
        let mut gpio = v2();

        // PA5 output, open-drain (OTYPER bit 5 set).
        gpio.write_u32(0x00, 0x1 << 10).unwrap();
        gpio.write_u32(0x04, 1 << 5).unwrap();

        gpio.write_u32(0x18, 1 << 5).unwrap(); // release (ODR=1)
        assert_eq!(
            rd32(&gpio, 0x10) & (1 << 5),
            0,
            "released open-drain pin must not mirror ODR; it floats to the latched input"
        );

        gpio.write_u32(0x18, 1 << (5 + 16)).unwrap(); // pull low (ODR=0)
        assert_eq!(
            rd32(&gpio, 0x10) & (1 << 5),
            0,
            "open-drain driving LOW reads LOW"
        );
    }

    /// A pin left as an input must keep reporting the latched input value, not
    /// ODR — otherwise the mirror would fabricate levels on undriven pins.
    #[test]
    fn v2_idr_input_pin_ignores_odr() {
        let mut gpio = v2();
        // MODER left at 0 => PA5 is an input. Write ODR anyway.
        gpio.write_u32(0x14, 1 << 5).unwrap();
        assert_eq!(
            rd32(&gpio, 0x10) & (1 << 5),
            0,
            "input pin must not take its level from ODR"
        );
    }
}
