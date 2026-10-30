// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Ambiq Apollo4 Plus peripherals: the 128-pin GPIO block and PWRCTRL.
//!
//! Source: Ambiq's CMSIS device header `apollo4p.h` (AmbiqSuite R4.x, mirrored
//! in `zephyrproject-rtos/hal_ambiq` at `CMSIS/AmbiqMicro/Include/apollo4p.h`):
//! `GPIO_Type` (@ 0x4001_0000, size 0x440) and `PWRCTRL_Type`
//! (@ 0x4002_1000), with their `_Pos`/`_Msk` bit-field defines and enums.
//! Reset values the header does not carry were cross-checked against Renode's
//! `AmbiqApollo4_GPIO` model (v1.17.0), which the Renode `apollo4` platform
//! tests run AmbiqSuite firmware against.
//!
//! **GPIO.** One block for all 128 pins (pins 105-127 are "virtual": they have
//! a PINCFG register but no pad). Per pin, `PINCFGn` @ `4·n`: FNCSEL \[3:0\]
//! (3 = GPIO, the reset value), INPEN \[4\], RDZERO \[5\], IRPTEN \[7:6\], OUTCFG
//! \[9:8\] (0 off, 1 push-pull, 2 open-drain, 3 tristate push-pull), DS, SR,
//! PULLCFG \[15:13\] (0 none, 1 50K pull-down, 2-7 pull-ups) and the NCE/force
//! bits above. PINCFG writes only land while `PADKEY` @ `0x200` holds `0x73`.
//! Per 32-pin bank `k`: `RDk` @ `0x204+4k` (pad level, 0 unless INPEN=1 and
//! RDZERO=0), `WTk` @ `0x214+4k` output latch with `WTSk`/`WTCk` set/clear
//! aliases at `0x224`/`0x234`, `ENk` @ `0x244+4k` tristate output enable with
//! `ENSk`/`ENCk` at `0x254`/`0x264`.
//!
//! Modelled: the PADKEY lock, PINCFG storage, the output latch and its
//! set/clear aliases, the tristate enable, the pad level each pin presents
//! (driven by the latch when FNCSEL=GPIO and OUTCFG drives it; otherwise the
//! outside world, otherwise the PULLCFG pull; an unpulled, undriven pad reads
//! 0), RD gating by INPEN/RDZERO, and the read-only `IEOBSk`/`OEOBSk` views of
//! the input/output enables. The interrupt banks (`MCUN0/N1`, `DSP0/1`
//! `INTk EN/STAT/CLR/SET` @ `0x2C0..0x43C`) keep EN, and CLR/SET clear/set
//! STAT, but no edge is ever detected, so STAT only changes by software and no
//! IRQ is raised.
//! NOT modelled: interrupt detection and the GPIO IRQ lines, the peripheral
//! side of a non-GPIO FNCSEL (such a pin is simply not driven by this model),
//! drive strength / slew rate, the NCE chip-select routing, FIEN/FOEN, and
//! `IOMxIRQ`/`SDIFCDWP`/`OBSDATA` (stored only).
//!
//! **PWRCTRL.** A register file over `0x000..0x220`. `DEVPWREN` @ `0x04` and
//! `AUDSSPWREN` @ `0x0C` are mirrored at once into `DEVPWRSTATUS` @ `0x08` /
//! `AUDSSPWRSTATUS` @ `0x10` (the status bits sit at the same positions as the
//! enables, per the `_Pos` defines), and a `MCUPERFREQ` @ `0x00` request is
//! acknowledged at once (MCUPERFACK set, MCUPERFSTATUS = the request). The
//! reset state is LP mode (96 MHz), the mode the Apollo4 Plus boots in.
//! NOT modelled: power-up latency (status follows enable on the same access),
//! any effect of a powered-down domain on its peripheral (a UART whose domain
//! is off still works), memory/SSRAM bank power (stored only, reset 0, not
//! verified against silicon), the voltage regulators, the DSPs and the energy
//! monitors.

use crate::pins::{External, InputChange, PadDriver, PinPort, Pull};
use crate::{Peripheral, SimResult};

const PINS: usize = 128;
const BANKS: usize = PINS / 32;

const PINCFG_END: u64 = 0x1FC;
const PADKEY: u64 = 0x200;
const RD0: u64 = 0x204;
const WT0: u64 = 0x214;
const WTS0: u64 = 0x224;
const WTC0: u64 = 0x234;
const EN0: u64 = 0x244;
const ENS0: u64 = 0x254;
const ENC0: u64 = 0x264;
const IOM0IRQ: u64 = 0x274;
const IEOBS0: u64 = 0x29C;
const OEOBS0: u64 = 0x2AC;
const INT_FIRST: u64 = 0x2C0;
const INT_END: u64 = 0x43C;
/// `IOM0IRQ..OBSDATA` (0x274-0x298): stored only.
const MISC_WORDS: usize = 10;
/// `MCUN0INT0EN..DSP1N1INT3SET`: 4 cores-and-priorities × 4 banks.
const INT_GROUPS: usize = 16;

/// `GPIO_PADKEY_PADKEY_Key`.
pub const PADKEY_KEY: u32 = 0x73;
/// PINCFGn reset: FNCSEL = 3 (GPIO), everything else 0.
const PINCFG_RESET: u32 = 0x3;
const FNCSEL_MASK: u32 = 0xF;
const FNCSEL_GPIO: u32 = 3;
const INPEN: u32 = 1 << 4;
const RDZERO: u32 = 1 << 5;
const OUTCFG_SHIFT: u32 = 8;
const PULLCFG_SHIFT: u32 = 13;

const OUTCFG_PUSHPULL: u32 = 1;
const OUTCFG_OD: u32 = 2;
const OUTCFG_TS: u32 = 3;

fn bit(words: &[u32; BANKS], pin: usize) -> bool {
    words[pin / 32] & (1 << (pin % 32)) != 0
}

/// The Apollo4 Plus GPIO block (128 pins, one register file).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Apollo4Gpio {
    #[serde(with = "serde_big_array")]
    pincfg: [u32; PINS],
    padkey: u32,
    /// `WTk`: the output latch.
    wt: [u32; BANKS],
    /// `ENk`: the tristate output enable.
    en: [u32; BANKS],
    misc: [u32; MISC_WORDS],
    int_en: [u32; INT_GROUPS],
    int_stat: [u32; INT_GROUPS],
    /// Levels presented on the pads by the outside world.
    #[serde(with = "serde_big_array")]
    external: [Option<bool>; PINS],
}

/// serde has no impls for arrays longer than 32; route the two 128-entry
/// arrays through a `Vec`.
mod serde_big_array {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S, T, const N: usize>(array: &[T; N], s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        array.as_slice().serialize(s)
    }

    pub fn deserialize<'de, D, T, const N: usize>(d: D) -> Result<[T; N], D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        let items = Vec::<T>::deserialize(d)?;
        let len = items.len();
        items
            .try_into()
            .map_err(|_| serde::de::Error::invalid_length(len, &"a 128-entry array"))
    }
}

impl Apollo4Gpio {
    pub fn new() -> Self {
        Self {
            pincfg: [PINCFG_RESET; PINS],
            padkey: 0,
            wt: [0; BANKS],
            en: [0; BANKS],
            misc: [0; MISC_WORDS],
            int_en: [0; INT_GROUPS],
            int_stat: [0; INT_GROUPS],
            external: [None; PINS],
        }
    }

    fn outcfg(&self, pin: usize) -> u32 {
        (self.pincfg[pin] >> OUTCFG_SHIFT) & 0x3
    }

    fn pull(&self, pin: usize) -> Pull {
        match (self.pincfg[pin] >> PULLCFG_SHIFT) & 0x7 {
            0 => Pull::None,
            1 => Pull::Down,
            _ => Pull::Up,
        }
    }

    /// The level this block drives onto `pin`, if it drives it at all.
    fn drive(&self, pin: usize) -> Option<bool> {
        if self.pincfg[pin] & FNCSEL_MASK != FNCSEL_GPIO {
            return None;
        }
        let latch = bit(&self.wt, pin);
        match self.outcfg(pin) {
            OUTCFG_PUSHPULL => Some(latch),
            OUTCFG_OD => (!latch).then_some(false),
            OUTCFG_TS => bit(&self.en, pin).then_some(latch),
            _ => None,
        }
    }

    /// The level on the pad: the driven level, otherwise the outside world,
    /// otherwise the PULLCFG pull (an unpulled pad reads 0).
    fn pad_level(&self, pin: usize) -> bool {
        if let Some(level) = self.drive(pin) {
            level
        } else if let Some(level) = self.external[pin] {
            level
        } else {
            self.pull(pin) == Pull::Up
        }
    }

    fn input_enabled(&self, pin: usize) -> bool {
        self.pincfg[pin] & (INPEN | RDZERO) == INPEN
    }

    fn bank_word(&self, bank: usize, f: impl Fn(usize) -> bool) -> u32 {
        (0..32)
            .filter(|&b| f(bank * 32 + b))
            .fold(0u32, |w, b| w | (1 << b))
    }

    fn rd(&self, bank: usize) -> u32 {
        self.bank_word(bank, |p| self.input_enabled(p) && self.pad_level(p))
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0..=PINCFG_END => self.pincfg[offset as usize / 4],
            PADKEY => self.padkey,
            0x204..=0x210 => self.rd((offset - RD0) as usize / 4),
            0x214..=0x220 => self.wt[(offset - WT0) as usize / 4],
            // Set/clear aliases are write-only strobes.
            0x224..=0x240 => 0,
            0x244..=0x250 => self.en[(offset - EN0) as usize / 4],
            0x254..=0x270 => self.en[((offset - ENS0) as usize / 4) % BANKS],
            0x274..=0x298 => self.misc[(offset - IOM0IRQ) as usize / 4],
            0x29C..=0x2A8 => {
                let bank = (offset - IEOBS0) as usize / 4;
                self.bank_word(bank, |p| self.pincfg[p] & INPEN != 0)
            }
            0x2AC..=0x2B8 => {
                let bank = (offset - OEOBS0) as usize / 4;
                self.bank_word(bank, |p| self.drive(p).is_some())
            }
            INT_FIRST..=INT_END => {
                let group = (offset - INT_FIRST) as usize / 16;
                match (offset - INT_FIRST) % 16 {
                    0x0 => self.int_en[group],
                    0x4 => self.int_stat[group],
                    _ => 0,
                }
            }
            _ => {
                crate::census_reg!("apollo4:Apollo4Gpio", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0..=PINCFG_END => {
                if self.padkey == PADKEY_KEY {
                    self.pincfg[offset as usize / 4] = value;
                }
            }
            PADKEY => self.padkey = value,
            // RD, IEOBS and OEOBS are read-only.
            0x204..=0x210 | 0x29C..=0x2B8 => {}
            0x214..=0x220 => self.wt[(offset - WT0) as usize / 4] = value,
            0x224..=0x230 => self.wt[(offset - WTS0) as usize / 4] |= value,
            0x234..=0x240 => self.wt[(offset - WTC0) as usize / 4] &= !value,
            0x244..=0x250 => self.en[(offset - EN0) as usize / 4] = value,
            0x254..=0x260 => self.en[(offset - ENS0) as usize / 4] |= value,
            0x264..=0x270 => self.en[(offset - ENC0) as usize / 4] &= !value,
            0x274..=0x298 => self.misc[(offset - IOM0IRQ) as usize / 4] = value,
            INT_FIRST..=INT_END => {
                let group = (offset - INT_FIRST) as usize / 16;
                match (offset - INT_FIRST) % 16 {
                    0x0 => self.int_en[group] = value,
                    0x8 => self.int_stat[group] &= !value,
                    0xC => self.int_stat[group] |= value,
                    _ => {}
                }
            }
            _ => {
                crate::census_reg!("apollo4:Apollo4Gpio", offset, "write");
            }
        }
    }

    /// Set/clear strobes: a byte store must not re-apply the other lanes.
    fn is_strobe(offset: u64) -> bool {
        matches!(offset, 0x224..=0x240 | 0x254..=0x270)
            || (INT_FIRST..=INT_END).contains(&offset) && (offset - INT_FIRST) % 16 >= 0x8
    }
}

impl Default for Apollo4Gpio {
    fn default() -> Self {
        Self::new()
    }
}

/// Byte-lane read of a 32-bit register file.
fn lane_read(word: u32, offset: u64) -> u8 {
    ((word >> ((offset % 4) as u32 * 8)) & 0xFF) as u8
}

/// Byte-lane read-modify-write of a 32-bit register file.
fn lane_merge(word: u32, offset: u64, value: u8) -> u32 {
    let shift = (offset % 4) as u32 * 8;
    (word & !(0xFF << shift)) | (u32::from(value) << shift)
}

macro_rules! snapshot_impl {
    ($ty:ty, $name:literal) => {
        fn snapshot(&self) -> serde_json::Value {
            serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
        }

        fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
            let restored: $ty = serde_json::from_value(state).map_err(|error| {
                crate::SimulationError::NotImplemented(format!(
                    concat!($name, " snapshot decode: {}"),
                    error
                ))
            })?;
            *self = restored;
            Ok(())
        }

        fn runtime_snapshot(&self) -> Vec<u8> {
            bincode::serialize(self).expect(concat!("bincode serialize ", $name))
        }

        fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
            if bytes.is_empty() {
                return Ok(());
            }
            let restored: $ty = bincode::deserialize(bytes).map_err(|error| {
                crate::SimulationError::NotImplemented(format!(
                    concat!($name, " runtime snapshot decode: {}"),
                    error
                ))
            })?;
            *self = restored;
            Ok(())
        }
    };
}

impl Peripheral for Apollo4Gpio {
    /// A pure register bank: no `tick()`, no IRQ, DMA or event from the walk
    /// (pad levels are recomputed on access).
    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(lane_read(self.read_reg(offset & !3), offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let base = if Self::is_strobe(reg) {
            0
        } else {
            self.read_reg(reg)
        };
        self.write_reg(reg, lane_merge(base, offset, value));
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    /// A node reset clears the register state; the outside world stays
    /// connected (a held button is not register state).
    fn on_node_reset(&mut self) {
        let external = self.external;
        *self = Self::new();
        self.external = external;
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        (pin < PINS).then(|| self.pad_level(pin))
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        (pin < PINS).then(|| self.drive(pin) == Some(true))
    }

    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        (pin < PINS).then(|| self.drive(pin).is_some())
    }

    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        Some(self)
    }

    snapshot_impl!(Apollo4Gpio, "Apollo4Gpio");
}

impl PinPort for Apollo4Gpio {
    fn pin_count(&self) -> u8 {
        PINS as u8
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        let pin = pin as usize;
        if pin >= PINS {
            return None;
        }
        Some(match self.drive(pin) {
            Some(level) => PadDriver::drive(level),
            None => match self.pull(pin) {
                Pull::None => PadDriver::OFF,
                pull => PadDriver::released(pull),
            },
        })
    }

    fn external(&self, pin: u8) -> External {
        match self.external.get(pin as usize).copied().flatten() {
            Some(level) => External::Level(level),
            None => External::Released,
        }
    }

    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        let pin = pin as usize;
        if pin >= PINS {
            return None;
        }
        let before = self.pad_level(pin);
        self.external[pin] = ext.level();
        let after = self.pad_level(pin);
        Some(InputChange { before, after })
    }

    fn input(&self, pin: u8) -> Option<bool> {
        let pin = pin as usize;
        (pin < PINS).then(|| self.pad_level(pin))
    }
}

// ── PWRCTRL (apollo4p.h PWRCTRL_Type, @ 0x4002_1000) ────────────────────────
const MCUPERFREQ: u64 = 0x00;
const DEVPWREN: u64 = 0x04;
const DEVPWRSTATUS: u64 = 0x08;
const AUDSSPWREN: u64 = 0x0C;
const AUDSSPWRSTATUS: u64 = 0x10;
const PWRCTRL_WORDS: usize = 0x220 / 4;

/// `PWRCTRL_DEVPWREN_*`: bits 24:0 (IOS .. DBG).
const DEVPWREN_MASK: u32 = 0x01FF_FFFF;
/// `PWRCTRL_AUDSSPWREN_*`: AUDREC, AUDPB, PDM0-3, I2S0/1, AUDADC, DSPA.
const AUDSSPWREN_MASK: u32 = 0x0000_0CFF;
const PERFREQ_MASK: u32 = 0x3;
const PERFACK: u32 = 1 << 2;
const PERFSTATUS_SHIFT: u32 = 3;
/// `PWRCTRL_MCUPERFREQ_MCUPERFREQ_LP` (96 MHz).
const PERF_LP: u32 = 1;

/// The Apollo4 Plus PWRCTRL block (see the module docs for what is modelled).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Apollo4PwrCtrl {
    regs: Vec<u32>,
}

impl Apollo4PwrCtrl {
    pub fn new() -> Self {
        let mut regs = vec![0; PWRCTRL_WORDS];
        regs[(MCUPERFREQ / 4) as usize] = PERF_LP | (PERF_LP << PERFSTATUS_SHIFT);
        Self { regs }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match self.regs.get((offset / 4) as usize) {
            Some(value) => *value,
            None => {
                crate::census_reg!("apollo4:Apollo4PwrCtrl", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        let index = (offset / 4) as usize;
        match offset {
            MCUPERFREQ => {
                let req = value & PERFREQ_MASK;
                self.regs[index] = req | PERFACK | (req << PERFSTATUS_SHIFT);
            }
            DEVPWREN => {
                self.regs[index] = value & DEVPWREN_MASK;
                self.regs[(DEVPWRSTATUS / 4) as usize] = value & DEVPWREN_MASK;
            }
            AUDSSPWREN => {
                self.regs[index] = value & AUDSSPWREN_MASK;
                self.regs[(AUDSSPWRSTATUS / 4) as usize] = value & AUDSSPWREN_MASK;
            }
            // Status registers are read-only.
            DEVPWRSTATUS | AUDSSPWRSTATUS => {}
            _ if index < self.regs.len() => self.regs[index] = value,
            _ => {
                crate::census_reg!("apollo4:Apollo4PwrCtrl", offset, "write");
            }
        }
    }
}

impl Default for Apollo4PwrCtrl {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Apollo4PwrCtrl {
    /// A register file with no `tick()`, IRQ, DMA or event.
    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(lane_read(self.read_reg(offset & !3), offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let merged = lane_merge(self.read_reg(reg), offset, value);
        self.write_reg(reg, merged);
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn on_node_reset(&mut self) {
        *self = Self::new();
    }

    snapshot_impl!(Apollo4PwrCtrl, "Apollo4PwrCtrl");
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUSHPULL_OUT: u32 = FNCSEL_GPIO | (OUTCFG_PUSHPULL << OUTCFG_SHIFT);

    fn unlocked() -> Apollo4Gpio {
        let mut g = Apollo4Gpio::new();
        g.write_u32(PADKEY, PADKEY_KEY).unwrap();
        g
    }

    #[test]
    fn pincfg_resets_to_gpio_and_ignores_writes_while_locked() {
        let mut g = Apollo4Gpio::new();
        assert_eq!(g.read_u32(30 * 4).unwrap(), PINCFG_RESET);
        g.write_u32(30 * 4, PUSHPULL_OUT).unwrap();
        assert_eq!(g.read_u32(30 * 4).unwrap(), PINCFG_RESET, "locked");
        g.write_u32(PADKEY, PADKEY_KEY).unwrap();
        g.write_u32(30 * 4, PUSHPULL_OUT).unwrap();
        assert_eq!(g.read_u32(30 * 4).unwrap(), PUSHPULL_OUT);
        g.write_u32(PADKEY, 0).unwrap();
        g.write_u32(30 * 4, PINCFG_RESET).unwrap();
        assert_eq!(g.read_u32(30 * 4).unwrap(), PUSHPULL_OUT, "relocked");
    }

    #[test]
    fn push_pull_pin_follows_wt_set_and_clear_in_its_bank() {
        let mut g = unlocked();
        // Pin 90 is bank 2, bit 26 (the EVB LED1).
        g.write_u32(90 * 4, PUSHPULL_OUT | INPEN).unwrap();
        g.write_u32(WTS0 + 8, 1 << 26).unwrap();
        assert_eq!(g.read_u32(WT0 + 8).unwrap(), 1 << 26);
        assert_eq!(g.read_u32(RD0 + 8).unwrap(), 1 << 26);
        assert_eq!(g.read_gpio_output(90), Some(true));
        g.write_u32(WTC0 + 8, 1 << 26).unwrap();
        assert_eq!(g.read_u32(RD0 + 8).unwrap(), 0);
        assert_eq!(g.read_gpio_input(90), Some(false));
        assert_eq!(g.read_gpio_is_output(90), Some(true));
    }

    #[test]
    fn rd_needs_inpen_and_honours_rdzero() {
        let mut g = unlocked();
        let pull_up = 5 << PULLCFG_SHIFT;
        g.write_u32(18 * 4, FNCSEL_GPIO | pull_up).unwrap();
        assert_eq!(g.read_u32(RD0).unwrap() & (1 << 18), 0, "INPEN=0");
        g.write_u32(18 * 4, FNCSEL_GPIO | pull_up | INPEN).unwrap();
        assert_ne!(g.read_u32(RD0).unwrap() & (1 << 18), 0, "pulled up");
        g.write_u32(18 * 4, FNCSEL_GPIO | pull_up | INPEN | RDZERO)
            .unwrap();
        assert_eq!(g.read_u32(RD0).unwrap() & (1 << 18), 0, "RDZERO");
    }

    #[test]
    fn button_press_pulls_a_pulled_up_input_low() {
        let mut g = unlocked();
        g.write_u32(19 * 4, FNCSEL_GPIO | INPEN | (5 << PULLCFG_SHIFT))
            .unwrap();
        let change = g.set_external(19, External::Level(false)).unwrap();
        assert!(change.before && !change.after);
        assert_eq!(g.read_u32(RD0).unwrap() & (1 << 19), 0);
        g.set_external(19, External::Released);
        assert_ne!(g.read_u32(RD0).unwrap() & (1 << 19), 0);
    }

    #[test]
    fn tristate_drives_only_when_enabled_and_open_drain_only_low() {
        let mut g = unlocked();
        g.write_u32(97 * 4, FNCSEL_GPIO | INPEN | (OUTCFG_TS << OUTCFG_SHIFT))
            .unwrap();
        g.write_u32(WTS0 + 12, 1 << 1).unwrap();
        assert_eq!(g.read_gpio_is_output(97), Some(false));
        g.write_u32(ENS0 + 12, 1 << 1).unwrap();
        assert_eq!(g.read_u32(EN0 + 12).unwrap(), 1 << 1);
        assert_eq!(g.read_gpio_output(97), Some(true));
        g.write_u32(ENC0 + 12, 1 << 1).unwrap();
        assert_eq!(g.read_gpio_is_output(97), Some(false));

        g.write_u32(5 * 4, FNCSEL_GPIO | INPEN | (OUTCFG_OD << OUTCFG_SHIFT))
            .unwrap();
        g.write_u32(WTC0, 1 << 5).unwrap();
        assert_eq!(g.read_gpio_output(5), Some(false));
        assert_eq!(g.read_gpio_is_output(5), Some(true));
        g.write_u32(WTS0, 1 << 5).unwrap();
        assert_eq!(g.read_gpio_is_output(5), Some(false), "released");
    }

    #[test]
    fn a_non_gpio_function_is_not_driven_by_the_gpio_block() {
        let mut g = unlocked();
        // Pin 60 FNCSEL 4 = UART0TX.
        g.write_u32(60 * 4, 4 | (OUTCFG_PUSHPULL << OUTCFG_SHIFT))
            .unwrap();
        g.write_u32(WTS0 + 4, 1 << 28).unwrap();
        assert_eq!(g.read_gpio_is_output(60), Some(false));
        assert_eq!(g.read_u32(OEOBS0 + 4).unwrap() & (1 << 28), 0);
    }

    #[test]
    fn byte_store_to_a_clear_strobe_touches_only_its_lane() {
        let mut g = unlocked();
        g.write_u32(WT0, 0xFFFF_FFFF).unwrap();
        g.write(WTC0 + 1, 0x01).unwrap();
        assert_eq!(g.read_u32(WT0).unwrap(), 0xFFFF_FEFF);
    }

    #[test]
    fn interrupt_status_is_set_and_cleared_by_software_only() {
        let mut g = Apollo4Gpio::new();
        g.write_u32(0x2C0, 0xF).unwrap();
        g.write_u32(0x2CC, 0x5).unwrap();
        assert_eq!(g.read_u32(0x2C4).unwrap(), 0x5);
        g.write_u32(0x2C8, 0x1).unwrap();
        assert_eq!(g.read_u32(0x2C4).unwrap(), 0x4);
        assert_eq!(g.read_u32(0x2C0).unwrap(), 0xF);
    }

    #[test]
    fn node_reset_keeps_the_outside_world() {
        let mut g = unlocked();
        g.set_external(18, External::Level(false));
        g.write_u32(WT0, 1).unwrap();
        g.on_node_reset();
        assert_eq!(g.read_u32(WT0).unwrap(), 0);
        assert_eq!(g.read_u32(PADKEY).unwrap(), 0);
        assert_eq!(g.external(18), External::Level(false));
    }

    #[test]
    fn snapshot_round_trips() {
        let mut g = unlocked();
        g.write_u32(30 * 4, PUSHPULL_OUT).unwrap();
        g.write_u32(WTS0, 1 << 30).unwrap();
        let snap = g.snapshot();
        let mut h = Apollo4Gpio::new();
        h.restore(snap).unwrap();
        assert_eq!(h.read_gpio_output(30), Some(true));
        let bytes = g.runtime_snapshot();
        let mut k = Apollo4Gpio::new();
        k.restore_runtime_snapshot(&bytes).unwrap();
        assert_eq!(k.read_u32(30 * 4).unwrap(), PUSHPULL_OUT);
    }

    #[test]
    fn pwrctrl_status_follows_enable_and_perf_request_is_acked() {
        let mut p = Apollo4PwrCtrl::new();
        assert_eq!(p.read_u32(MCUPERFREQ).unwrap(), 0x9, "LP at reset");
        assert_eq!(p.read_u32(DEVPWRSTATUS).unwrap(), 0);
        p.write_u32(DEVPWREN, 1 << 9).unwrap();
        assert_eq!(p.read_u32(DEVPWRSTATUS).unwrap(), 1 << 9);
        p.write_u32(DEVPWRSTATUS, 0).unwrap();
        assert_eq!(p.read_u32(DEVPWRSTATUS).unwrap(), 1 << 9, "read-only");
        p.write_u32(AUDSSPWREN, 0xFFFF_FFFF).unwrap();
        assert_eq!(p.read_u32(AUDSSPWRSTATUS).unwrap(), AUDSSPWREN_MASK);
        p.write_u32(MCUPERFREQ, 2).unwrap();
        assert_eq!(p.read_u32(MCUPERFREQ).unwrap(), 2 | PERFACK | (2 << 3));
        p.write_u32(0x14, 0x3F).unwrap();
        assert_eq!(p.read_u32(0x14).unwrap(), 0x3F, "MEMPWREN stored");
        p.on_node_reset();
        assert_eq!(p.read_u32(DEVPWREN).unwrap(), 0);
    }

    #[test]
    fn pure_register_banks_opt_out_of_the_tick_walk() {
        assert!(!Apollo4Gpio::new().legacy_tick_active());
        assert!(!Apollo4Gpio::new().needs_legacy_walk());
        assert!(!Apollo4PwrCtrl::new().legacy_tick_active());
        assert!(!Apollo4PwrCtrl::new().needs_legacy_walk());
    }
}
