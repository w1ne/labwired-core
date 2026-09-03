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
    /// Nordic **nRF54L family** (nRF54L05/10/15, nRF54LM20A/B) GPIO port.
    ///
    /// The same registers as [`GpioRegisterLayout::Nrf52`] with the same
    /// meanings, at a COMPACTED set of offsets: OUT @0x000, OUTSET @0x004,
    /// OUTCLR @0x008, IN @0x00C, DIR @0x010, DIRSET @0x014, DIRCLR @0x018,
    /// LATCH @0x020, DETECTMODE @0x024, PIN_CNF[n] @0x080 + 4n. Source: Nordic
    /// MDK `nrf54lm20a_application.svd`, peripheral GLOBAL_P2.
    ///
    /// ⚠️ This is NOT a constant shift of the nRF52 map, which is why it is a
    /// layout and not a `reg_offset`. The first block moved by exactly 0x504,
    /// but PIN_CNF moved from 0x700 to 0x080 — a delta of 0x680. A port
    /// declared as nRF52-with-an-offset therefore serves DIR and OUT correctly
    /// and drops every PIN_CNF access on the floor, which is silent: LEDs still
    /// light, because they only need DIR and OUT, while an input's pull-up
    /// configuration — written by Zephyr and nrfx through PIN_CNF alone — never
    /// arrives, and the pin reads whatever the bus floats at.
    Nrf54l,
    /// NXP Kinetis (KW41Z GPIOA/B/C): PDOR @0x0 (output), PSOR/PCOR/PTOR
    /// set/clear/toggle, PDIR @0x10 (input), PDDR @0x14 (direction).
    Kinetis,
    /// Silicon Labs EFR32 Series 2 (xG21–xG29) GPIO port: the per-port
    /// GPIO_PORT_TypeDef struct — CTRL @0x00, MODEL @0x04, MODEH @0x0C,
    /// DOUT @0x10 (output), DIN @0x14 (input). Offsets from the vendor CMSIS
    /// header (simplicity_sdk `efr32mg26_gpio_port.h`).
    Efr32s2,
    /// Microchip **SAM** (SAM D21 / D51 / E5x) PORT group — DIR @0x00 with
    /// CLR/SET/TGL aliases, OUT @0x10 with the same three aliases, IN @0x20,
    /// CTRL @0x24, WRCONFIG @0x28, PMUX[16] @0x30, PINCFG[32] @0x40. Offsets
    /// from `ATSAMD21G18A.svd` (Microchip, Apache-2.0), cluster GROUP.
    ///
    /// One LabWired port = one PORT GROUP. The groups sit at a 0x80 stride
    /// inside a single 0x200 PORT window, so a SAM chip YAML declares each
    /// group as its own peripheral at `PORT_base + 0x80 * n` — the same
    /// per-port-window shape the EFR32 Series-2 ports use.
    SamPort,
}

impl FromStr for GpioRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            "stm32v2" | "v2" | "modern" | "stm32-modern" | "h5" | "stm32h5" => Ok(Self::Stm32V2),
            "nrf52" | "nordic" => Ok(Self::Nrf52),
            "nrf54l" | "nrf54lm20a" | "nrf54l15" => Ok(Self::Nrf54l),
            "kinetis" | "kw41z" | "nxp" => Ok(Self::Kinetis),
            "efr32s2" | "efr32_series2" | "efr32xg2" => Ok(Self::Efr32s2),
            "sam" | "sam_port" | "samd" | "samd21" | "samd51" | "microchip" => Ok(Self::SamPort),
            _ => Err(format!(
                "unsupported GPIO register layout '{}'; supported: stm32f1, stm32v2, nrf52, nrf54l, kinetis, efr32s2, sam_port",
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

// ── Silicon Labs EFR32 Series 2 (xG21–xG29) ──────────────────────────────────
// One port of the single GPIO block: GPIO_PORT_TypeDef (simplicity_sdk
// efr32mg26_gpio_port.h, sisdk-2025.6). The block packs four of these structs
// at a 0x30-byte stride starting at block+0x30; each port is modelled here as
// its own window at the true struct base. Register map: CTRL @0x00, MODEL
// @0x04 (pins 0..7), MODEH @0x0C (pins 8..15), DOUT @0x10, DIN @0x14.
//
// Mode is 4 bits per pin: 0 DISABLED, 1 INPUT, 2 INPUTPULL, 3 INPUTPULLFILTER,
// 4 PUSHPULL, 5 PUSHPULLALT, 6 WIREDOR, 7 WIREDORPULLDOWN, 8..15 the WIREDAND
// (open-source) family. This model implements the digital truth of that table:
// outputs drive the pin, WIREDOR* pins only pull LOW, and DIN reads the pin —
// the same contract the STM32 families above implement.
//
// NOT modelled (documented in configs/chips/efr32mg26.yaml): the block-level
// SET/CLR/TGL aliases at +0x1000/+0x2000/+0x3000 (outside this port window),
// the ROUTE pin-mux registers (they live in the GPIO block head, not in the
// port struct), CTRL slew-rate/drive-strength fields (stored, no behaviour),
// the WIREDAND pull-up/filter analog niceties (treated as push-pull), and the
// EM4 wakeup / EXTI path (GPIO IF/IEN live in the block head).
#[derive(Debug, serde::Serialize)]
pub struct Efr32s2Gpio {
    ctrl: u32,  // 0x00
    model: u32, // 0x04
    modeh: u32, // 0x0C
    dout: u32,  // 0x10
    din: u32,   // 0x14 — latched external (button/sensor) input
}

impl Default for Efr32s2Gpio {
    fn default() -> Self {
        Self {
            // _GPIO_P_CTRL_RESETVALUE: slewrate fields reset non-zero.
            ctrl: 0x0040_0040,
            model: 0,
            modeh: 0,
            dout: 0,
            din: 0,
        }
    }
}

impl Efr32s2Gpio {
    /// The 4-bit mode field of `pin` (MODEL for pins 0..7, MODEH for 8..15).
    fn mode_nibble(&self, pin: u32) -> u32 {
        let reg = if pin < 8 { self.model } else { self.modeh };
        (reg >> ((pin % 8) * 4)) & 0xF
    }

    /// Mask of pins configured as an output (any drive mode, nibble >= 4).
    fn output_mask(&self) -> u32 {
        let mut mask = 0u32;
        for pin in 0..16u32 {
            if self.mode_nibble(pin) >= 0x4 {
                mask |= 1 << pin;
            }
        }
        mask
    }

    /// Mask of output pins in a WIREDOR (open-drain) mode: 6 WIREDOR,
    /// 7 WIREDORPULLDOWN. These only pull LOW; driving a 1 releases the pin.
    fn open_drain_mask(&self) -> u32 {
        let mut mask = 0u32;
        for pin in 0..16u32 {
            if matches!(self.mode_nibble(pin), 0x6 | 0x7) {
                mask |= 1 << pin;
            }
        }
        mask
    }

    /// DIN as silicon presents it: the *pin* level, not a bare latch. A
    /// push-pull output drives its pin, so reading DIN returns what DOUT is
    /// driving; a released open-drain pin and every input take the latched
    /// external level. Same contract as V2Gpio::effective_idr.
    fn effective_din(&self) -> u32 {
        let out = self.output_mask();
        let od = self.open_drain_mask();
        let push_pull = out & !od;
        let od_driven_low = od & !self.dout;
        let driven = push_pull | od_driven_low;
        ((self.dout & push_pull) | (self.din & !driven)) & 0xFFFF
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.ctrl,
            0x04 => self.model,
            0x0C => self.modeh,
            0x10 => self.dout,
            0x14 => self.effective_din(),
            _ => {
                crate::census_reg!("gpio:Efr32s2Gpio", offset, "read");
                0
            }
        }
    }
    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.ctrl = value,
            0x04 => self.model = value,
            0x0C => self.modeh = value,
            // DOUT/DIN are 16-bit on this part (GPIO_PORT_x_WIDTH = 0x10 for
            // all four ports on the IM48). DIN is read-only for firmware —
            // like silicon, a store to it is ignored; external input arrives
            // via set_external_input.
            0x10 => self.dout = value & 0xFFFF,
            _ => {
                crate::census_reg!("gpio:Efr32s2Gpio", offset, "write");
            }
        }
    }
}

// ── Microchip SAM (SAM D21 / D51 / E5x) PORT ─────────────────────────────────
// One GROUP of the PORT block. PORT packs its groups at a 0x80 stride
// (GROUP[0] = PA, GROUP[1] = PB, …); each group is modelled here as its own
// window at the true group base, exactly as the EFR32 Series-2 ports are.
//
// Register map — `ATSAMD21G18A.svd` (Microchip Technology Inc., Apache-2.0),
// cluster GROUP, confirmed field-by-field against that file:
//   DIR 0x00, DIRCLR 0x04, DIRSET 0x08, DIRTGL 0x0C,
//   OUT 0x10, OUTCLR 0x14, OUTSET 0x18, OUTTGL 0x1C,
//   IN 0x20 (read-only), CTRL 0x24, WRCONFIG 0x28 (write-only),
//   PMUX[16] 0x30..0x3F (8-bit), PINCFG[32] 0x40..0x5F (8-bit).
//
// ⚠️ The SET/CLR/TGL registers are NOT separate state: silicon reads all four
// DIR aliases back as DIR and all four OUT aliases back as OUT. A model that
// stored them separately would read back the last write mask instead of the
// port state, and `digitalRead()` on a pin set through OUTSET would answer
// from a register no silicon has.
//
// WRCONFIG is modelled because it is the path ASF, the Arduino SAMD core and
// CircuitPython actually use to configure a pad: one store writes PINCFG (and
// optionally PMUX) for up to 16 pins selected by PINMASK, with HWSEL choosing
// the low or the high half of the port. Dropping it would leave every pad at
// its reset config while the firmware believed it had muxed SERCOM onto them —
// silent, and the same shape as the RP2040 IO_BANK0 gap.
//
// NOT modelled, deliberately: CTRL.SAMPLING (continuous input sampling — this
// model samples on read), PINCFG.DRVSTR and PULLEN's pull direction (stored,
// read back, no electrical effect), and the PORT_IOBUS alias window at
// 0x6000_0000 (a second, single-cycle view of the same registers; it is a
// separate bus window and belongs in the chip YAML if a firmware needs it).
#[derive(Debug, Default, serde::Serialize)]
pub struct SamGpio {
    dir: u32, // 0x00 — 1 = output
    out: u32, // 0x10 — output latch
    /// 0x20 IN, the latched EXTERNAL level. Blended with the driven pins by
    /// [`SamGpio::effective_in`]; never written by firmware (IN is read-only).
    in_latch: u32,
    ctrl: u32, // 0x24
    /// PMUX[16]: two 4-bit peripheral selections per byte — PMUXE (bits 3:0)
    /// for the even pin, PMUXO (bits 7:4) for the odd one.
    pmux: [u8; 16],
    /// PINCFG[32]: PMUXEN bit 0, INEN bit 1, PULLEN bit 2, DRVSTR bit 6.
    pincfg: [u8; 32],
}

impl SamGpio {
    /// Mask of pins whose pad is handed to a peripheral (PINCFG.PMUXEN).
    fn pmuxen_mask(&self) -> u32 {
        let mut mask = 0u32;
        for (pin, cfg) in self.pincfg.iter().enumerate() {
            if cfg & 0x1 != 0 {
                mask |= 1u32 << pin;
            }
        }
        mask
    }

    /// Mask of pins this port drives as a GPIO output: DIR set AND the pad not
    /// muxed away to a peripheral. A PMUXEN pad is driven by whoever owns the
    /// function, so DIR alone does not make the port the driver.
    fn output_mask(&self) -> u32 {
        self.dir & !self.pmuxen_mask()
    }

    /// IN as silicon presents it: the *pin* level, not a bare latch. A pin the
    /// port drives reads back what OUT is driving — `digitalRead()` on an
    /// OUTPUT pin is a common Arduino idiom and must not read 0 forever. Every
    /// other pin reads the latched external level. Same contract as
    /// `V2Gpio::effective_idr` and `Efr32s2Gpio::effective_din`.
    fn effective_in(&self) -> u32 {
        let driven = self.output_mask();
        (self.out & driven) | (self.in_latch & !driven)
    }

    /// Pack four consecutive 8-bit registers into the word at `base + n*4`.
    fn packed(bytes: &[u8], index: usize) -> u32 {
        let mut word = 0u32;
        for i in 0..4 {
            if let Some(b) = bytes.get(index + i) {
                word |= (*b as u32) << (i * 8);
            }
        }
        word
    }

    fn unpack(bytes: &mut [u8], index: usize, value: u32) {
        for i in 0..4 {
            if let Some(b) = bytes.get_mut(index + i) {
                *b = ((value >> (i * 8)) & 0xFF) as u8;
            }
        }
    }

    /// WRCONFIG: bulk PINCFG/PMUX write. PINMASK[15:0] selects pins within the
    /// half of the port chosen by HWSEL[31] (0 = pins 0..15, 1 = 16..31);
    /// WRPINCFG[30] and WRPMUX[28] each gate whether that half of the payload
    /// is applied. The PINCFG payload arrives as separate bits — PMUXEN[16],
    /// INEN[17], PULLEN[18], DRVSTR[22] — and is reassembled into the PINCFG
    /// byte layout here.
    fn write_wrconfig(&mut self, value: u32) {
        let pinmask = value & 0xFFFF;
        let base = if (value >> 31) & 1 == 1 { 16usize } else { 0 };
        let write_pincfg = (value >> 30) & 1 == 1;
        let write_pmux = (value >> 28) & 1 == 1;

        let cfg = (((value >> 16) & 1) as u8)
            | ((((value >> 17) & 1) as u8) << 1)
            | ((((value >> 18) & 1) as u8) << 2)
            | ((((value >> 22) & 1) as u8) << 6);
        let pmux = ((value >> 24) & 0xF) as u8;

        for i in 0..16usize {
            if pinmask & (1u32 << i) == 0 {
                continue;
            }
            let pin = base + i;
            if write_pincfg {
                self.pincfg[pin] = cfg;
            }
            if write_pmux {
                let byte = pin / 2;
                self.pmux[byte] = if pin % 2 == 0 {
                    (self.pmux[byte] & 0xF0) | pmux
                } else {
                    (self.pmux[byte] & 0x0F) | (pmux << 4)
                };
            }
        }
    }

    /// The 4-bit PMUX selection for `pin`, or `None` when the pad is not muxed
    /// to a peripheral. Null over a guess: a pad with PMUXEN clear is a GPIO,
    /// whatever stale value PMUX happens to hold.
    fn pmux_of(&self, pin: u8) -> Option<u8> {
        let pin = pin as usize;
        if pin >= 32 || self.pincfg[pin] & 0x1 == 0 {
            return None;
        }
        let byte = self.pmux[pin / 2];
        Some(if pin % 2 == 0 { byte & 0xF } else { byte >> 4 })
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            // All four DIR aliases read back DIR, all four OUT aliases read
            // back OUT — silicon has one register behind each set.
            0x00 | 0x04 | 0x08 | 0x0C => self.dir,
            0x10 | 0x14 | 0x18 | 0x1C => self.out,
            0x20 => self.effective_in(),
            0x24 => self.ctrl,
            // WRCONFIG is write-only; silicon returns 0.
            0x28 => 0,
            0x30..=0x3F => Self::packed(&self.pmux, (offset - 0x30) as usize),
            0x40..=0x5F => Self::packed(&self.pincfg, (offset - 0x40) as usize),
            _ => {
                crate::census_reg!("gpio:SamGpio", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.dir = value,
            0x04 => self.dir &= !value, // DIRCLR
            0x08 => self.dir |= value,  // DIRSET
            0x0C => self.dir ^= value,  // DIRTGL
            0x10 => self.out = value,
            0x14 => self.out &= !value, // OUTCLR
            0x18 => self.out |= value,  // OUTSET
            0x1C => self.out ^= value,  // OUTTGL
            // IN is read-only for firmware, exactly as on silicon. External
            // input arrives through GpioFamily::set_external_input.
            0x20 => {}
            0x24 => self.ctrl = value,
            0x28 => self.write_wrconfig(value),
            0x30..=0x3F => Self::unpack(&mut self.pmux, (offset - 0x30) as usize, value),
            0x40..=0x5F => Self::unpack(&mut self.pincfg, (offset - 0x40) as usize, value),
            _ => {
                crate::census_reg!("gpio:SamGpio", offset, "write");
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
    Efr32s2(Efr32s2Gpio),
    SamPort(SamGpio),
}

impl GpioFamily {
    fn read_reg(&self, offset: u64) -> u32 {
        match self {
            Self::Stm32F1(g) => g.read_reg(offset),
            Self::Stm32V2(g) => g.read_reg(offset),
            Self::Nrf52(g) => g.read_reg(offset),
            Self::Kinetis(g) => g.read_reg(offset),
            Self::Efr32s2(g) => g.read_reg(offset),
            Self::SamPort(g) => g.read_reg(offset),
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match self {
            Self::Stm32F1(g) => g.write_reg(offset, value),
            Self::Stm32V2(g) => g.write_reg(offset, value),
            Self::Nrf52(g) => g.write_reg(offset, value),
            Self::Kinetis(g) => g.write_reg(offset, value),
            Self::Efr32s2(g) => g.write_reg(offset, value),
            Self::SamPort(g) => g.write_reg(offset, value),
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
            // Series-2 EFR32 names it DIN.
            Self::Efr32s2(g) => apply(&mut g.din),
            // SAM PORT names it IN, and it is read-only to firmware.
            Self::SamPort(g) => apply(&mut g.in_latch),
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
            // The Series-2 DIN read already mixes DOUT-through-MODE with
            // latched inputs — it IS the pad view (no AF tracking on this
            // family: the ROUTE mux is not modelled).
            Self::Efr32s2(g) => Some(bit(g.read_reg(0x14))),
            // SAM PORT: a pad with PINCFG.PMUXEN is handed to a peripheral,
            // whose wire state this model cannot know — None, not a guess from
            // a DIR bit the port no longer owns. Every other pad reads IN,
            // which already mixes OUT-through-DIR with the latched input.
            Self::SamPort(g) => {
                if g.pmux_of(pin).is_some() {
                    None
                } else {
                    Some(bit(g.read_reg(0x20)))
                }
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
    /// An STM32 pad answers "who drives me" from its own AFR nibble, so this is
    /// `None` there and costs one branch. Nordic and Silicon Labs invert it —
    /// the PERIPHERAL names the pin (`PSEL.TXD`, `GPIO_TIMERROUTE[n].CC0ROUTE`)
    /// — so the answer has to come from somewhere the port can reach, and this
    /// is it, together with this port's own port NUMBER in that family's
    /// encoding. Installed once at bus wiring time; a port that never gets one
    /// has no routes bound either, and behaves exactly as before.
    pad_claims: Option<(
        std::sync::Arc<crate::peripherals::pad_claims::PadClaims>,
        u8,
    )>,
    /// True when this port decodes the nRF54L compacted offsets. The register
    /// BEHAVIOUR is the nRF52 model's — only the addresses differ — so the
    /// family stays `Nrf52` and every `GpioFamily::Nrf52(_)` arm elsewhere
    /// keeps working. Adding a family variant instead would make each of those
    /// arms silently miss this port.
    nrf54l_offsets: bool,
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
            pad_claims: None,
            nrf54l_offsets: false,
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
            GpioRegisterLayout::Nrf54l => {
                let mut port = Self::from_family(GpioFamily::Nrf52(Nrf52Gpio::default()));
                port.nrf54l_offsets = true;
                return port;
            }
            GpioRegisterLayout::Kinetis => GpioFamily::Kinetis(KinetisGpio::default()),
            GpioRegisterLayout::Efr32s2 => GpioFamily::Efr32s2(Efr32s2Gpio::default()),
            GpioRegisterLayout::SamPort => GpioFamily::SamPort(SamGpio::default()),
        })
    }

    /// Build an nRF52-layout GPIO port with an explicit pin count.
    /// Use this when the port has fewer than 32 physical pins (e.g. P1 = 16).
    pub fn new_nrf52(num_pins: u32) -> Self {
        Self::from_family(GpioFamily::Nrf52(Nrf52Gpio::with_num_pins(num_pins)))
    }

    /// Build an nRF54L-layout GPIO port with an explicit pin count.
    ///
    /// Port widths are NOT uniform on this family and a wrong one is silent:
    /// nRF54LM20A has P0 = 10, P1 = 32, P2 = 11, P3 = 13 (Zephyr DT `ngpios`).
    pub fn new_nrf54l(num_pins: u32) -> Self {
        let mut port = Self::from_family(GpioFamily::Nrf52(Nrf52Gpio::with_num_pins(num_pins)));
        port.nrf54l_offsets = true;
        port
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
        self.family
            .read_reg(self.translate(offset) + self.window_offset)
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        let translated = self.translate(offset);
        self.family
            .write_reg(translated + self.window_offset, value);
    }

    /// nRF54L window offset -> the nRF52 model's register offset.
    ///
    /// Piecewise, because the two blocks moved by different amounts: the
    /// OUT..DETECTMODE run by 0x504, PIN_CNF by 0x680. An unrecognised offset
    /// is passed through unchanged so it reaches the model's own census
    /// counter rather than being folded onto a real register.
    fn translate(&self, offset: u64) -> u64 {
        if !self.nrf54l_offsets {
            return offset;
        }
        match offset {
            0x000..=0x024 => offset + 0x504,
            0x080..=0x0FC => offset + 0x680,
            other => other,
        }
    }

    /// Register offset of the output data register (ODR) for this family,
    /// relative to the port's MMIO window (so `base + odr_offset()` is an
    /// address the bus can route).
    /// Used by the bus to resolve a display's D/C line to a concrete address.
    pub fn odr_offset(&self) -> u64 {
        let family: u64 = match &self.family {
            GpioFamily::Stm32F1(_) => 0x0C,
            GpioFamily::Stm32V2(_) => 0x14,
            GpioFamily::Nrf52(_) if self.nrf54l_offsets => 0x000,
            GpioFamily::Nrf52(_) => 0x504,
            GpioFamily::Kinetis(_) => 0x00,
            GpioFamily::Efr32s2(_) => 0x10, // DOUT
            GpioFamily::SamPort(_) => 0x10, // OUT
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
            GpioFamily::Nrf52(_) if self.nrf54l_offsets => 0x00C,
            GpioFamily::Nrf52(_) => 0x510,
            GpioFamily::Kinetis(_) => 0x10,
            GpioFamily::Efr32s2(_) => 0x14, // DIN
            GpioFamily::SamPort(_) => 0x20, // IN
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

    /// Hand this port the shared pad-claim table and tell it which port NUMBER
    /// it is in the muxing family's own encoding — `PSEL.PORT` on Nordic,
    /// `CCnROUTE.PORT` on EFR32. Config-build time only; see
    /// [`GpioPort::pad_claims`].
    pub(crate) fn set_pad_claims(
        &mut self,
        claims: std::sync::Arc<crate::peripherals::pad_claims::PadClaims>,
        port: u8,
    ) {
        self.pad_claims = Some((claims, port));
    }

    pub(crate) fn register_layout(&self) -> GpioRegisterLayout {
        match &self.family {
            GpioFamily::Stm32F1(_) => GpioRegisterLayout::Stm32F1,
            GpioFamily::Stm32V2(_) => GpioRegisterLayout::Stm32V2,
            // Reported as its own layout, which is what keeps
            // `wire_nrf52_pads` off this port: that engine's PSEL decode is
            // verified on the nRF52840 only.
            GpioFamily::Nrf52(_) if self.nrf54l_offsets => GpioRegisterLayout::Nrf54l,
            GpioFamily::Nrf52(_) => GpioRegisterLayout::Nrf52,
            GpioFamily::Kinetis(_) => GpioRegisterLayout::Kinetis,
            GpioFamily::Efr32s2(_) => GpioRegisterLayout::Efr32s2,
            GpioFamily::SamPort(_) => GpioRegisterLayout::SamPort,
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
    /// mapping is fixed so the value carries no information and any bound route
    /// matches. F1 MISO — an input-mode pad on real silicon — is intentionally
    /// NOT routed, so a plain GPIO input on that pin never silently reads the
    /// SPI wire.
    ///
    /// ⚠️ "Fixed" means AFIO REMAP IS NOT CONSULTED, which is a narrower claim
    /// than "AFIO is not modelled". [`crate::peripherals::afio::Afio`] does
    /// model `MAPR`: it stores bits [15:0] and reads them back, silicon-checked
    /// on the bench F103. Nothing DECODES them. This function reads CRL/CRH and
    /// nothing else, so a route bound to a remap-only pad would be live the
    /// moment firmware made that pad any alternate-function output — including
    /// for its own default function. That is why the F1 tables in
    /// `SystemBus::wire_stm32_i2c_pads` and `wire_stm32_uart_pads` bind
    /// DEFAULT-column pads only, and a remapped F103 bus is dark rather than
    /// wrong. Closing that gap means feeding `MAPR` in here as a second
    /// selector input.
    ///
    /// nRF52: the port HAS no such register — Nordic muxes at the peripheral,
    /// which names its pin in `PSEL.*`. The answer therefore comes from the
    /// shared claim table the peripherals publish into
    /// ([`crate::peripherals::nrf52::pin_select`]), and the "selector" is a
    /// claim token rather than a decoded register field. Same shape, opposite
    /// direction; [`PadRoutes`](crate::peripherals::pad_routing) cannot tell.
    fn selected_function(
        family: &GpioFamily,
        pad_claims: Option<&(
            std::sync::Arc<crate::peripherals::pad_claims::PadClaims>,
            u8,
        )>,
        pin: u8,
    ) -> Option<u32> {
        match family {
            // Nordic and EFR32 Series 2 answer the same way and for the same
            // reason: the port has no mux register at all, so the selector is
            // a claim token a peripheral published — `PSEL.*` on Nordic,
            // `GPIO_TIMERROUTE[n].CCnROUTE` on EFR32. Same shape, opposite
            // direction from an AFR nibble; `PadRoutes` cannot tell.
            GpioFamily::Nrf52(_) | GpioFamily::Efr32s2(_) => {
                let (claims, port) = pad_claims?;
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
            Self::selected_function(&self.family, self.pad_claims.as_ref(), p)
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
            Self::selected_function(&self.family, self.pad_claims.as_ref(), pin)
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

    fn read_gpio_input_word(&self) -> u32 {
        // The same register `read_gpio_input` reads, read ONCE instead of once
        // per pin. That is not an approximation of the default 32-call loop, it
        // is the identity it computes: the loop's bit `pin` is
        // `read_reg(idr_offset()) >> pin & 1` for every pin below 32, and this
        // model answers `Some` for all 32, so the loop never breaks early and
        // reassembles this exact word. Every family's input offset comes from
        // `idr_offset`, so no family is special-cased here.
        //
        // Worth overriding because `read_reg` on the input offset is not a field
        // load: it evaluates `effective_idr` (STM32 F1/V2, Kinetis, nRF52) or
        // the Series-2 DIN path (`Efr32s2Gpio::read_reg`, which alone was 60% of
        // all retired instructions on efr32mg26 before this).
        self.read_reg(self.idr_offset())
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
                if Self::selected_function(&self.family, self.pad_claims.as_ref(), pin).is_some() {
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
            // Series-2 EFR32: 4-bit mode nibble per pin (MODEL/MODEH).
            // 0 DISABLED is a hi-Z pin — closest to Analog here; 1..3 are the
            // input modes; >= 4 the output modes. No AF verdict: the ROUTE
            // pin-mux lives in the GPIO block head and is not modelled, so a
            // peripheral-driven pad reports its GPIO mode (documented).
            GpioFamily::Efr32s2(g) => match g.mode_nibble(u32::from(pin)) {
                0 => GpioMode::Analog,
                0x1..=0x3 => GpioMode::Input,
                _ => GpioMode::Output,
            },
            // SAM PORT: PINCFG.PMUXEN is the AF verdict and it is a REGISTER,
            // not an inference — the pad is muxed or it is not. DIR then
            // separates output from input for the pads the port still owns.
            GpioFamily::SamPort(g) => {
                if g.pmux_of(pin).is_some() {
                    GpioMode::Af
                } else if (g.read_reg(0x00) & (1u32 << pin)) != 0 {
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
                Self::selected_function(&self.family, self.pad_claims.as_ref(), p)
            }) {
                Some(func.to_string())
            } else if let GpioFamily::Stm32V2(g) = &self.family {
                let (afr_off, sh) = if pin < 8 {
                    (0x20, (pin * 4) as u32)
                } else {
                    (0x24, ((pin - 8) * 4) as u32)
                };
                Some(format!("AF{}", (g.read_reg(afr_off) >> sh) & 0xF))
            } else if let GpioFamily::SamPort(g) = &self.family {
                // The SAM datasheet labels peripheral functions by LETTER
                // (PMUX 0 = A, 1 = B, … 7 = H), and the pin-function tables in
                // every SAM datasheet are indexed by that letter. Reporting
                // "PMUX_C" is the silicon's own name for the selection; which
                // SERCOM instance that letter lands on is a per-pin table this
                // model does not hold, so it is not invented here.
                g.pmux_of(pin)
                    .map(|sel| format!("PMUX_{}", (b'A' + sel) as char))
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
            GpioFamily::Efr32s2(g) => serde_json::to_value(g),
            GpioFamily::SamPort(g) => serde_json::to_value(g),
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

#[cfg(test)]
mod efr32s2_tests {
    use super::{GpioMode, GpioPort, GpioRegisterLayout};
    use crate::Peripheral;

    fn s2() -> GpioPort {
        GpioPort::new_with_layout(GpioRegisterLayout::Efr32s2)
    }

    /// Silicon reset state (efr32mg26_gpio_port.h `_GPIO_P_*_RESETVALUE`):
    /// CTRL carries non-zero slewrate defaults, everything else is 0 — every
    /// pin DISABLED.
    #[test]
    fn efr32s2_reset_values() {
        let g = s2();
        assert_eq!(g.read_u32(0x00).unwrap(), 0x0040_0040, "CTRL reset");
        assert_eq!(g.read_u32(0x04).unwrap(), 0, "MODEL reset");
        assert_eq!(g.read_u32(0x0C).unwrap(), 0, "MODEH reset");
        assert_eq!(g.read_u32(0x10).unwrap(), 0, "DOUT reset");
        assert_eq!(g.read_u32(0x14).unwrap(), 0, "DIN reset");
    }

    /// DOUT drives the pin of a push-pull output, and DIN reads the pin —
    /// so DIN mirrors DOUT for PUSHPULL pins. This is the path the BRD2709A
    /// LEDs (PC08/PC09) take: MODEH nibbles 0/1 = PUSHPULL (0x4).
    #[test]
    fn efr32s2_dout_drives_din_for_push_pull_output() {
        let mut g = s2();
        // PC08/PC09 → MODEH nibbles 0 and 1 = PUSHPULL.
        g.write_u32(0x0C, 0x4 | (0x4 << 4)).unwrap();
        assert_eq!(g.gpio_routing(8).unwrap().mode, GpioMode::Output);
        assert_eq!(g.gpio_routing(9).unwrap().mode, GpioMode::Output);

        g.write_u32(0x10, 1 << 8).unwrap(); // DOUT: LED0 on
        assert_eq!(g.read_u32(0x10).unwrap() & (1 << 8), 1 << 8, "DOUT latch");
        assert_eq!(
            g.read_u32(0x14).unwrap() & (1 << 8),
            1 << 8,
            "DIN must report the driven HIGH level of a push-pull output"
        );
        assert_eq!(g.read_gpio_pad(8), Some(true));
        assert_eq!(g.read_gpio_pad(9), Some(false), "PC09 still drives LOW");
        assert_eq!(g.read_gpio_output(8), Some(true));

        g.write_u32(0x10, 0).unwrap(); // LEDs off
        assert_eq!(g.read_u32(0x14).unwrap() & (3 << 8), 0);
        assert_eq!(g.read_gpio_pad(8), Some(false));
    }

    /// A DISABLED or INPUT pin ignores DOUT: its DIN bit is the latched
    /// external level, which is how buttons (PB00/PB01) are read.
    #[test]
    fn efr32s2_input_pin_reads_external_level_not_dout() {
        let mut g = s2();
        // PB00 left DISABLED (MODE 0); PB01 = INPUT (MODEL nibble 1 = 0x1).
        g.write_u32(0x04, 0x1 << 4).unwrap();
        assert_eq!(g.gpio_routing(0).unwrap().mode, GpioMode::Analog);
        assert_eq!(g.gpio_routing(1).unwrap().mode, GpioMode::Input);

        g.write_u32(0x10, 0x3).unwrap(); // DOUT writes must not move inputs
        assert_eq!(g.read_u32(0x14).unwrap() & 0x3, 0);

        // The outside world (a button) drives the pins through the input path.
        assert!(g.set_gpio_input(0, true));
        assert!(g.set_gpio_input(1, true));
        assert_eq!(g.read_u32(0x14).unwrap() & 0x3, 0x3, "DIN shows buttons");
        assert_eq!(g.read_gpio_input(0), Some(true));
        assert!(g.set_gpio_input(0, false));
        assert_eq!(g.read_u32(0x14).unwrap() & 0x3, 0x2);
    }

    /// WIREDOR (open-drain) only pulls LOW: DOUT=1 releases the pin to the
    /// latched input, DOUT=0 drives LOW.
    #[test]
    fn efr32s2_wiredor_only_pulls_low() {
        let mut g = s2();
        g.write_u32(0x04, 0x6).unwrap(); // pin 0 = WIREDOR
        assert!(g.set_gpio_input(0, true)); // external pull holds it high

        g.write_u32(0x10, 1).unwrap(); // release
        assert_eq!(
            g.read_u32(0x14).unwrap() & 1,
            1,
            "released open-drain pin floats to the latched input"
        );
        g.write_u32(0x10, 0).unwrap(); // pull low
        assert_eq!(g.read_u32(0x14).unwrap() & 1, 0, "open-drain driving LOW");
    }

    /// DOUT is 16-bit on this part (GPIO_PORT_x_WIDTH = 0x10); DIN is
    /// read-only for firmware.
    #[test]
    fn efr32s2_dout_is_16_bit_and_din_ignores_writes() {
        let mut g = s2();
        g.write_u32(0x10, 0xFFFF_FFFF).unwrap();
        assert_eq!(g.read_u32(0x10).unwrap(), 0xFFFF, "DOUT masks to 16 bits");
        g.write_u32(0x14, 0xFFFF).unwrap();
        assert_eq!(g.read_u32(0x14).unwrap(), 0, "DIN store ignored");
    }

    /// Byte-granular MMIO must land in the same registers (the Peripheral
    /// byte path decomposes to read-modify-write per byte).
    #[test]
    fn efr32s2_byte_writes_compose() {
        let mut g = s2();
        g.write(0x0D, 0x04).unwrap(); // MODEH byte 1: pin 10 nibble = PUSHPULL
        g.write(0x11, 0x04).unwrap(); // DOUT byte 1: pin 10 high
        assert_eq!(g.gpio_routing(10).unwrap().mode, GpioMode::Output);
        assert_eq!(g.read_gpio_pad(10), Some(true));
    }
}

#[cfg(test)]
mod sam_port_tests {
    use super::{GpioMode, GpioPort, GpioRegisterLayout};
    use crate::Peripheral;

    fn port() -> GpioPort {
        GpioPort::new_with_layout(GpioRegisterLayout::SamPort)
    }

    /// WRCONFIG payload: `pins` within the half chosen by `hwsel`, PINCFG bits
    /// as named, and (when `pmux` is `Some`) that peripheral selection.
    fn wrconfig(pinmask: u16, hwsel: bool, pmuxen: bool, inen: bool, pmux: Option<u8>) -> u32 {
        let mut w = u32::from(pinmask);
        if pmuxen {
            w |= 1 << 16;
        }
        if inen {
            w |= 1 << 17;
        }
        if let Some(sel) = pmux {
            w |= (u32::from(sel) & 0xF) << 24;
            w |= 1 << 28; // WRPMUX
        }
        w |= 1 << 30; // WRPINCFG
        if hwsel {
            w |= 1 << 31;
        }
        w
    }

    /// SAM D21 PORT resets to all-zero: every pin an input, nothing muxed.
    #[test]
    fn sam_port_reset_values() {
        let g = port();
        for off in [0x00u64, 0x10, 0x20, 0x24, 0x30, 0x40] {
            assert_eq!(g.read_u32(off).unwrap(), 0, "offset {off:#x} reset");
        }
    }

    /// ⚠️ The regression this family is most exposed to: DIRSET/DIRCLR/DIRTGL
    /// are aliases, not registers. Silicon reads all four back as DIR. A model
    /// that stored them separately reads back the last write MASK, so a driver
    /// that sets a pin through DIRSET and then read-modify-writes DIR loses
    /// every other pin on the port.
    #[test]
    fn dir_aliases_all_read_back_as_dir() {
        let mut g = port();
        g.write_u32(0x08, 1 << 17).unwrap(); // DIRSET pin 17
        for alias in [0x00u64, 0x04, 0x08, 0x0C] {
            assert_eq!(
                g.read_u32(alias).unwrap(),
                1 << 17,
                "alias {alias:#x} must read DIR"
            );
        }
        g.write_u32(0x04, 1 << 17).unwrap(); // DIRCLR
        assert_eq!(g.read_u32(0x00).unwrap(), 0);
        g.write_u32(0x0C, 1 << 3).unwrap(); // DIRTGL
        assert_eq!(g.read_u32(0x00).unwrap(), 1 << 3);
    }

    /// Same contract on the OUT side, and OUTTGL is how the Arduino SAMD core
    /// blinks.
    #[test]
    fn out_aliases_all_read_back_as_out() {
        let mut g = port();
        g.write_u32(0x18, 1 << 5).unwrap(); // OUTSET
        for alias in [0x10u64, 0x14, 0x18, 0x1C] {
            assert_eq!(g.read_u32(alias).unwrap(), 1 << 5, "alias {alias:#x}");
        }
        g.write_u32(0x1C, 1 << 5).unwrap(); // OUTTGL
        assert_eq!(g.read_u32(0x10).unwrap(), 0);
    }

    /// `digitalRead()` on an OUTPUT pin must see what the port drives, not a
    /// separate input latch stuck at 0. PA17 is the Arduino Zero LED.
    #[test]
    fn in_reads_back_what_an_output_pin_drives() {
        let mut g = port();
        g.write_u32(0x08, 1 << 17).unwrap(); // DIRSET PA17
        g.write_u32(0x18, 1 << 17).unwrap(); // OUTSET PA17
        assert_eq!(g.read_u32(0x20).unwrap() & (1 << 17), 1 << 17, "IN");
        assert_eq!(g.read_gpio_pad(17), Some(true));
        assert_eq!(g.gpio_routing(17).unwrap().mode, GpioMode::Output);
    }

    /// An external driver (button, sensor) moves only the pins the port is not
    /// driving — otherwise a shorted output would read the world instead of
    /// itself.
    #[test]
    fn external_input_reaches_only_undriven_pins() {
        let mut g = port();
        g.write_u32(0x08, 1 << 2).unwrap(); // pin 2 is an output
        assert!(g.set_gpio_input(2, true));
        assert!(g.set_gpio_input(3, true));
        assert_eq!(g.read_u32(0x20).unwrap() & (1 << 2), 0, "output wins on 2");
        assert_eq!(g.read_u32(0x20).unwrap() & (1 << 3), 1 << 3, "input on 3");
    }

    /// WRCONFIG is the only path ASF and the Arduino SAMD core take to a
    /// PINCFG byte. PINMASK selects within the half HWSEL picks.
    #[test]
    fn wrconfig_writes_pincfg_for_the_masked_pins_only() {
        let mut g = port();
        // Pins 4 and 6, low half, INEN set.
        g.write_u32(
            0x28,
            wrconfig((1 << 4) | (1 << 6), false, false, true, None),
        )
        .unwrap();
        let pincfg = |g: &GpioPort, pin: u64| {
            (g.read_u32(0x40 + (pin & !3)).unwrap() >> ((pin % 4) * 8)) & 0xFF
        };
        assert_eq!(pincfg(&g, 4), 0x02, "PINCFG4.INEN");
        assert_eq!(pincfg(&g, 6), 0x02, "PINCFG6.INEN");
        assert_eq!(pincfg(&g, 5), 0x00, "PINCFG5 untouched");
    }

    /// HWSEL shifts the same 16-bit PINMASK onto pins 16..31. Getting this
    /// wrong configures the wrong pad and says nothing about it.
    #[test]
    fn wrconfig_hwsel_selects_the_high_half() {
        let mut g = port();
        g.write_u32(0x28, wrconfig(1 << 1, true, false, true, None))
            .unwrap();
        let pincfg17 = (g.read_u32(0x40 + 16).unwrap() >> 8) & 0xFF;
        assert_eq!(pincfg17, 0x02, "PINCFG17 via HWSEL");
        assert_eq!(g.read_u32(0x40).unwrap(), 0, "low half untouched");
    }

    /// PMUX packs two pins per byte — even pin in PMUXE (bits 3:0), odd in
    /// PMUXO (bits 7:4). Writing the wrong nibble mutes the neighbouring pad.
    #[test]
    fn wrconfig_pmux_lands_in_the_even_or_odd_nibble() {
        let mut g = port();
        // Pin 10 (even) → PMUX[5].PMUXE = C (2). Pin 11 (odd) → PMUX[5].PMUXO.
        g.write_u32(0x28, wrconfig(1 << 10, false, true, true, Some(2)))
            .unwrap();
        let pmux5 = (g.read_u32(0x34).unwrap() >> 8) & 0xFF;
        assert_eq!(pmux5, 0x02, "even pin writes the low nibble");

        g.write_u32(0x28, wrconfig(1 << 11, false, true, true, Some(3)))
            .unwrap();
        let pmux5 = (g.read_u32(0x34).unwrap() >> 8) & 0xFF;
        assert_eq!(pmux5, 0x32, "odd pin writes the high nibble, even survives");
    }

    /// A pad handed to a peripheral is not the port's to report a level for,
    /// and the routing names the datasheet's own function letter.
    #[test]
    fn a_muxed_pad_reports_af_and_no_level() {
        let mut g = port();
        g.write_u32(0x08, 1 << 10).unwrap(); // DIR set — irrelevant once muxed
        g.write_u32(0x28, wrconfig(1 << 10, false, true, true, Some(2)))
            .unwrap();
        let routing = g.gpio_routing(10).unwrap();
        assert_eq!(routing.mode, GpioMode::Af);
        assert_eq!(routing.func.as_deref(), Some("PMUX_C"));
        assert_eq!(g.read_gpio_pad(10), None, "the peripheral owns the wire");
    }

    /// Clearing PMUXEN hands the pad back to the port — the selection left in
    /// PMUX must not keep claiming it.
    #[test]
    fn clearing_pmuxen_hands_the_pad_back_to_the_port() {
        let mut g = port();
        g.write_u32(0x28, wrconfig(1 << 10, false, true, true, Some(2)))
            .unwrap();
        assert_eq!(g.gpio_routing(10).unwrap().mode, GpioMode::Af);
        g.write_u32(0x28, wrconfig(1 << 10, false, false, true, None))
            .unwrap();
        assert_eq!(g.gpio_routing(10).unwrap().mode, GpioMode::Input);
        assert_eq!(g.gpio_routing(10).unwrap().func, None);
    }

    /// WRCONFIG is write-only on silicon; it must not read back as state.
    #[test]
    fn wrconfig_is_write_only() {
        let mut g = port();
        g.write_u32(0x28, wrconfig(0xFFFF, false, true, true, Some(2)))
            .unwrap();
        assert_eq!(g.read_u32(0x28).unwrap(), 0);
    }

    /// IN is read-only for firmware, exactly as on silicon.
    #[test]
    fn firmware_cannot_store_to_in() {
        let mut g = port();
        g.write_u32(0x20, 0xFFFF_FFFF).unwrap();
        assert_eq!(g.read_u32(0x20).unwrap(), 0);
    }
}
