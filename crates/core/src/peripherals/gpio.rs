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
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.crl,
            0x04 => self.crh,
            0x08 => self.idr,
            0x0C => self.odr,
            0x18 => self.lckr,
            _ => 0,
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
            _ => {}
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
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.moder,
            0x04 => self.otyper,
            0x08 => self.ospeedr,
            0x0C => self.pupdr,
            0x10 => self.idr,
            0x14 => self.odr,
            0x1C => self.lckr,
            0x20 => self.afrl,
            0x24 => self.afrh,
            _ => 0,
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
            _ => {}
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
            _ => 0,
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
                }
            }
            _ => {}
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
            _ => 0,
        }
    }
    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.pdor = value,   // PDOR
            0x04 => self.pdor |= value,  // PSOR: set 1s
            0x08 => self.pdor &= !value, // PCOR: clear 1s
            0x0C => self.pdor ^= value,  // PTOR: toggle
            0x14 => self.pddr = value,   // PDDR
            _ => {}
        }
    }
}

/// GPIO port — one variant per chip family. Register sets are fully isolated.
#[derive(Debug, serde::Serialize)]
pub enum GpioPort {
    Stm32F1(F1Gpio),
    Stm32V2(V2Gpio),
    Nrf52(Nrf52Gpio),
    Kinetis(KinetisGpio),
}

impl Default for GpioPort {
    fn default() -> Self {
        Self::Stm32F1(F1Gpio::new())
    }
}

impl GpioPort {
    pub fn new() -> Self {
        Self::new_with_layout(GpioRegisterLayout::Stm32F1)
    }

    pub fn new_with_layout(layout: GpioRegisterLayout) -> Self {
        match layout {
            GpioRegisterLayout::Stm32F1 => Self::Stm32F1(F1Gpio::new()),
            GpioRegisterLayout::Stm32V2 => Self::Stm32V2(V2Gpio::default()),
            GpioRegisterLayout::Nrf52 => Self::Nrf52(Nrf52Gpio::default()),
            GpioRegisterLayout::Kinetis => Self::Kinetis(KinetisGpio::default()),
        }
    }

    /// Build an nRF52-layout GPIO port with an explicit pin count.
    /// Use this when the port has fewer than 32 physical pins (e.g. P1 = 16).
    pub fn new_nrf52(num_pins: u32) -> Self {
        Self::Nrf52(Nrf52Gpio::with_num_pins(num_pins))
    }

    /// Build a V2-layout GPIO port with explicit MODER/OSPEEDR/PUPDR reset
    /// values. On real silicon these are per-port (debug pins keep port A off
    /// the all-analog default; B carries the JTDO pull config; C..G reset to
    /// 0xFFFFFFFF analog). The chip yaml supplies them via
    /// `config: { reset_moder / reset_ospeedr / reset_pupdr }`.
    pub fn new_stm32v2_with_resets(moder: u32, ospeedr: u32, pupdr: u32) -> Self {
        Self::Stm32V2(V2Gpio {
            moder,
            ospeedr,
            pupdr,
            ..Default::default()
        })
    }

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

    /// Register offset of the output data register (ODR) for this family.
    /// Used by the bus to resolve a display's D/C line to a concrete address.
    pub fn odr_offset(&self) -> u64 {
        match self {
            Self::Stm32F1(_) => 0x0C,
            Self::Stm32V2(_) => 0x14,
            Self::Nrf52(_) => 0x504,
            Self::Kinetis(_) => 0x00,
        }
    }

    /// Register offset of the input data register (IDR) for this family.
    /// Used by the bus to resolve a sensor's input line (e.g. HC-SR04 ECHO).
    pub fn idr_offset(&self) -> u64 {
        match self {
            Self::Stm32F1(_) => 0x08,
            Self::Stm32V2(_) => 0x10,
            Self::Nrf52(_) => 0x510,
            Self::Kinetis(_) => 0x10,
        }
    }
}

impl crate::Peripheral for GpioPort {
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

        self.write_reg(reg_offset, reg_val);
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
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        if pin >= 32 {
            return None;
        }
        let reg = self.read_reg(self.idr_offset());
        Some((reg & (1u32 << pin)) != 0)
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        if pin >= 32 {
            return None;
        }
        let reg = self.read_reg(self.odr_offset());
        Some((reg & (1u32 << pin)) != 0)
    }

    fn set_gpio_input(&mut self, pin: u8, level: bool) -> bool {
        if pin >= 32 {
            return false;
        }
        let offset = self.idr_offset();
        let mut reg = self.read_reg(offset);
        if level {
            reg |= 1u32 << pin;
        } else {
            reg &= !(1u32 << pin);
        }
        self.write_reg(offset, reg);
        true
    }

    fn snapshot(&self) -> serde_json::Value {
        // Serialize the active family's register struct directly (flat), so the
        // snapshot keeps registers like `odr` at top level (no variant tag) —
        // matching the pre-split format the snapshot contract depends on.
        match self {
            Self::Stm32F1(g) => serde_json::to_value(g),
            Self::Stm32V2(g) => serde_json::to_value(g),
            Self::Nrf52(g) => serde_json::to_value(g),
            Self::Kinetis(g) => serde_json::to_value(g),
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
