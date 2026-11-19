// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! TI DaVinci / Keystone / K3 GPIO (`ti_davinci_gpio`).
//!
//! The GPIO block of the DaVinci, OMAP-L1, Keystone and K3 (AM62x, AM64x,
//! J7) families. The register map is the one the Linux v6.12
//! `drivers/gpio/gpio-davinci.c` (`struct davinci_gpio_regs`, `BINTEN`,
//! `offset_array`) and the Zephyr v4.3.0 `drivers/gpio/gpio_davinci.c` use.
//! One block carries up to five register banks of 32 GPIOs.
//!
//! | Offset | Name | Access | Meaning |
//! | --- | --- | --- | --- |
//! | 0x08 | BINTEN | R/W | per-16-GPIO interrupt enable |
//! | 0x10 + 0x28·b | DIR | R/W | 1 = input (reset), 0 = output |
//! | 0x14 + 0x28·b | OUT_DATA | R/W | output latch |
//! | 0x18 + 0x28·b | SET_DATA | R/W1S | sets OUT_DATA bits; reads OUT_DATA |
//! | 0x1C + 0x28·b | CLR_DATA | R/W1C | clears OUT_DATA bits; reads OUT_DATA |
//! | 0x20 + 0x28·b | IN_DATA | R | pad levels, also for output pins |
//! | 0x24 + 0x28·b | SET_RIS_TRIG | R/W1S | rising-edge detect enable |
//! | 0x28 + 0x28·b | CLR_RIS_TRIG | R/W1C | clears rising-edge enables |
//! | 0x2C + 0x28·b | SET_FAL_TRIG | R/W1S | falling-edge detect enable |
//! | 0x30 + 0x28·b | CLR_FAL_TRIG | R/W1C | clears falling-edge enables |
//! | 0x34 + 0x28·b | INTSTAT | R/W1C | latched edges of enabled pins |
//!
//! `ngpio` is the `ti,ngpio` count of the instance; bits past it neither
//! store nor read. IN_DATA reads the pad: the output latch for an output
//! pin, the external level for an input pin.
//!
//! Not modelled: the PID and PCR registers at 0x00 / 0x04 (read 0), the
//! interrupt lines (on K3 they pass through the TISCI interrupt router,
//! which has no model, so INTSTAT is poll-only), and the pad's receiver
//! enable, which on K3 lives in the PADCONFIG register of the pin mux.

use crate::pins::{External, InputChange, PadDriver, PinPort};
use crate::SimResult;

/// `offset_array` in both drivers: at most five register banks.
const MAX_BANKS: usize = 5;
const BINTEN: u64 = 0x08;
const BANK_BASE: u64 = 0x10;
const BANK_STRIDE: u64 = 0x28;

const DIR: u64 = 0x00;
const OUT_DATA: u64 = 0x04;
const SET_DATA: u64 = 0x08;
const CLR_DATA: u64 = 0x0C;
const IN_DATA: u64 = 0x10;
const SET_RIS_TRIG: u64 = 0x14;
const CLR_RIS_TRIG: u64 = 0x18;
const SET_FAL_TRIG: u64 = 0x1C;
const CLR_FAL_TRIG: u64 = 0x20;
const INTSTAT: u64 = 0x24;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct Bank {
    /// Direction, 1 = input.
    dir: u32,
    out_data: u32,
    ris_trig: u32,
    fal_trig: u32,
    intstat: u32,
    /// External levels on the pads (bit n), valid where `connected` is set.
    external: u32,
    connected: u32,
    /// GPIOs that exist in this bank.
    mask: u32,
}

impl Bank {
    fn pad_levels(&self) -> u32 {
        ((self.out_data & !self.dir) | (self.external & self.connected & self.dir)) & self.mask
    }

    /// Latch the enabled edges between two pad words.
    fn latch_edges(&mut self, before: u32, after: u32) {
        let rising = !before & after & self.ris_trig;
        let falling = before & !after & self.fal_trig;
        self.intstat |= (rising | falling) & self.mask;
    }
}

/// TI DaVinci-family GPIO block with `ngpio` GPIOs.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct TiDavinciGpio {
    ngpio: u8,
    binten: u32,
    banks: Vec<Bank>,
}

impl TiDavinciGpio {
    /// A block with `ngpio` GPIOs (clamped to 1..=160, five banks).
    pub fn new(ngpio: u32) -> Self {
        let ngpio = ngpio.clamp(1, (MAX_BANKS * 32) as u32);
        let banks = (0..ngpio.div_ceil(32))
            .map(|b| {
                let left = ngpio - b * 32;
                let mask = if left >= 32 {
                    u32::MAX
                } else {
                    (1 << left) - 1
                };
                Bank {
                    dir: mask,
                    mask,
                    ..Bank::default()
                }
            })
            .collect();
        Self {
            ngpio: ngpio as u8,
            binten: 0,
            banks,
        }
    }

    /// BINTEN has one bit per 16 GPIOs.
    fn binten_mask(&self) -> u32 {
        let bits = u32::from(self.ngpio).div_ceil(16);
        (1 << bits) - 1
    }

    fn locate(&self, pin: u8) -> Option<(usize, u32)> {
        (pin < self.ngpio).then(|| (usize::from(pin / 32), 1u32 << (pin % 32)))
    }

    /// Split a word-aligned offset into (bank, register).
    fn decode(&self, offset: u64) -> Option<(usize, u64)> {
        let rel = offset.checked_sub(BANK_BASE)?;
        let bank = (rel / BANK_STRIDE) as usize;
        (bank < self.banks.len()).then_some((bank, rel % BANK_STRIDE))
    }

    fn read_reg(&self, offset: u64) -> u32 {
        if offset == BINTEN {
            return self.binten;
        }
        let Some((index, reg)) = self.decode(offset) else {
            crate::census_reg!("ti:davinci_gpio", offset, "read");
            return 0;
        };
        let bank = &self.banks[index];
        match reg {
            DIR => bank.dir,
            OUT_DATA | SET_DATA | CLR_DATA => bank.out_data,
            IN_DATA => bank.pad_levels(),
            SET_RIS_TRIG | CLR_RIS_TRIG => bank.ris_trig,
            SET_FAL_TRIG | CLR_FAL_TRIG => bank.fal_trig,
            INTSTAT => bank.intstat,
            _ => unreachable!("a bank holds ten word registers"),
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        if offset == BINTEN {
            self.binten = value & self.binten_mask();
            return;
        }
        let Some((index, reg)) = self.decode(offset) else {
            crate::census_reg!("ti:davinci_gpio", offset, "write");
            return;
        };
        let bank = &mut self.banks[index];
        let value = value & bank.mask;
        let before = bank.pad_levels();
        match reg {
            DIR => bank.dir = value,
            OUT_DATA => bank.out_data = value,
            SET_DATA => bank.out_data |= value,
            CLR_DATA => bank.out_data &= !value,
            IN_DATA => {} // read-only
            SET_RIS_TRIG => bank.ris_trig |= value,
            CLR_RIS_TRIG => bank.ris_trig &= !value,
            SET_FAL_TRIG => bank.fal_trig |= value,
            CLR_FAL_TRIG => bank.fal_trig &= !value,
            INTSTAT => bank.intstat &= !value,
            _ => unreachable!("a bank holds ten word registers"),
        }
        // A direction or latch write can move the pad; that is an edge too.
        let after = bank.pad_levels();
        bank.latch_edges(before, after);
    }

    /// Registers whose write acts on the 1 bits only: a byte store must not
    /// read the register back and re-apply the other three lanes.
    fn is_strobe(&self, offset: u64) -> bool {
        self.decode(offset)
            .is_some_and(|(_, reg)| !matches!(reg, DIR | OUT_DATA))
    }
}

impl crate::Peripheral for TiDavinciGpio {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(offset & !3) >> (byte * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) as u32 * 8;
        let lane = u32::from(value) << shift;
        if self.is_strobe(reg) {
            self.write_reg(reg, lane);
        } else {
            let kept = self.read_reg(reg) & !(0xFF << shift);
            self.write_reg(reg, kept | lane);
        }
        Ok(())
    }

    /// A node reset restores the register state. The external levels stay:
    /// they are the board's connections, not register state.
    fn on_node_reset(&mut self) {
        let fresh = Self::new(u32::from(self.ngpio));
        self.binten = 0;
        for (bank, reset) in self.banks.iter_mut().zip(fresh.banks) {
            *bank = Bank {
                external: bank.external,
                connected: bank.connected,
                ..reset
            };
        }
    }

    /// No interrupt line is delivered and nothing advances with time.
    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        Some(self)
    }

    /// The pad level, as IN_DATA reports it.
    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        let (bank, bit) = self.locate(pin)?;
        Some(self.banks[bank].pad_levels() & bit != 0)
    }

    /// The first register bank (GPIOs 0..31).
    fn read_gpio_input_word(&self) -> u32 {
        self.banks[0].pad_levels()
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        let (bank, bit) = self.locate(pin)?;
        Some(self.banks[bank].out_data & bit != 0)
    }

    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        let (bank, bit) = self.locate(pin)?;
        Some(self.banks[bank].dir & bit == 0)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let restored: Self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "TiDavinciGpio snapshot decode: {error}"
            ))
        })?;
        *self = restored;
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize TiDavinciGpio")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let restored: Self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "TiDavinciGpio runtime snapshot decode: {error}"
            ))
        })?;
        *self = restored;
        Ok(())
    }
}

impl PinPort for TiDavinciGpio {
    fn pin_count(&self) -> u8 {
        self.ngpio
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        let (bank, bit) = self.locate(pin)?;
        let bank = &self.banks[bank];
        Some(if bank.dir & bit == 0 {
            PadDriver::drive(bank.out_data & bit != 0)
        } else {
            PadDriver::OFF
        })
    }

    fn external(&self, pin: u8) -> External {
        let Some((bank, bit)) = self.locate(pin) else {
            return External::Released;
        };
        let bank = &self.banks[bank];
        if bank.connected & bit == 0 {
            External::Released
        } else {
            External::Level(bank.external & bit != 0)
        }
    }

    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        let (bank, bit) = self.locate(pin)?;
        let bank = &mut self.banks[bank];
        let before = bank.pad_levels();
        match ext.level() {
            Some(level) => {
                bank.connected |= bit;
                bank.external = (bank.external & !bit) | if level { bit } else { 0 };
            }
            None => bank.connected &= !bit,
        }
        let after = bank.pad_levels();
        bank.latch_edges(before, after);
        Some(InputChange {
            before: before & bit != 0,
            after: after & bit != 0,
        })
    }

    fn input(&self, pin: u8) -> Option<bool> {
        let (bank, bit) = self.locate(pin)?;
        let bank = &self.banks[bank];
        (bank.dir & bit != 0).then(|| bank.pad_levels() & bit != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    fn read32(gpio: &TiDavinciGpio, offset: u64) -> u32 {
        (0..4).fold(0, |word, i| {
            word | u32::from(gpio.read(offset + i).unwrap()) << (8 * i)
        })
    }

    fn write32(gpio: &mut TiDavinciGpio, offset: u64, value: u32) {
        for i in 0..4 {
            gpio.write(offset + i, (value >> (8 * i)) as u8).unwrap();
        }
    }

    /// Bank 1 registers, the offsets `offset_array[1] = 0x38` gives.
    const DIR1: u64 = 0x38;
    const OUT_DATA1: u64 = 0x3C;
    const SET_DATA1: u64 = 0x40;
    const CLR_DATA1: u64 = 0x44;
    const IN_DATA1: u64 = 0x48;
    const SET_RIS_TRIG1: u64 = 0x4C;
    const CLR_RIS_TRIG1: u64 = 0x50;
    const SET_FAL_TRIG1: u64 = 0x54;
    const INTSTAT1: u64 = 0x5C;

    #[test]
    fn reset_state_is_all_inputs_and_ngpio_bounds_the_last_bank() {
        // AM62x MAIN_GPIO0: `ti,ngpio = <92>` is 32 + 32 + 28.
        let gpio = TiDavinciGpio::new(92);
        assert_eq!(read32(&gpio, 0x10), 0xFFFF_FFFF);
        assert_eq!(read32(&gpio, 0x38), 0xFFFF_FFFF);
        assert_eq!(read32(&gpio, 0x60), 0x0FFF_FFFF);
        assert_eq!(read32(&gpio, 0x88), 0, "no fourth bank");
        assert_eq!(gpio.pin_count(), 92);
        assert_eq!(gpio.read_gpio_is_output(91), Some(false));
        assert_eq!(gpio.read_gpio_is_output(92), None);
    }

    #[test]
    fn in_data_reads_the_external_level_of_an_input_pin_in_its_bank() {
        let mut gpio = TiDavinciGpio::new(92);
        // GPIO 42 is bank 1 bit 10.
        gpio.set_external(42, External::Level(true)).unwrap();
        assert_eq!(read32(&gpio, IN_DATA1), 1 << 10);
        assert_eq!(read32(&gpio, 0x20), 0, "bank 0 does not move");
        gpio.set_external(42, External::Released).unwrap();
        assert_eq!(read32(&gpio, IN_DATA1), 0);
    }

    #[test]
    fn set_and_clr_data_act_on_written_ones_and_in_data_reads_an_output_back() {
        let mut gpio = TiDavinciGpio::new(52);
        write32(&mut gpio, DIR1, !(1 << 17)); // GPIO 49 is an output
        write32(&mut gpio, SET_DATA1, 1 << 17);
        assert_eq!(read32(&gpio, OUT_DATA1), 1 << 17);
        assert_eq!(read32(&gpio, IN_DATA1), 1 << 17);
        assert_eq!(gpio.driver(49), Some(PadDriver::drive(true)));
        // An external level does not override the pad the block drives.
        gpio.set_external(49, External::Level(false)).unwrap();
        assert_eq!(read32(&gpio, IN_DATA1), 1 << 17);
        write32(&mut gpio, CLR_DATA1, 1 << 17);
        assert_eq!(read32(&gpio, OUT_DATA1), 0);
        // Bits past ngpio = 52 (bank 1 bits 20..31) do not store.
        write32(&mut gpio, SET_DATA1, 0xFFF0_0000);
        assert_eq!(read32(&gpio, OUT_DATA1), 0);
    }

    #[test]
    fn a_byte_store_to_set_data_sets_only_its_lane() {
        let mut gpio = TiDavinciGpio::new(64);
        write32(&mut gpio, DIR1, 0);
        write32(&mut gpio, OUT_DATA1, 0x0000_00F0);
        gpio.write(CLR_DATA1 + 1, 0xFF).unwrap();
        assert_eq!(
            read32(&gpio, OUT_DATA1),
            0x0000_00F0,
            "a byte-1 clear must not clear the byte-0 bits it reads back"
        );
    }

    #[test]
    fn intstat_latches_only_enabled_edges_and_clears_by_writing_one() {
        let mut gpio = TiDavinciGpio::new(92);
        let bit = 1 << 10;
        gpio.set_external(42, External::Level(true)).unwrap();
        assert_eq!(read32(&gpio, INTSTAT1), 0, "no trigger enabled");
        gpio.set_external(42, External::Level(false)).unwrap();
        write32(&mut gpio, SET_RIS_TRIG1, bit);
        assert_eq!(read32(&gpio, CLR_RIS_TRIG1), bit, "both read the enables");
        gpio.set_external(42, External::Level(true)).unwrap();
        assert_eq!(read32(&gpio, INTSTAT1), bit);
        write32(&mut gpio, INTSTAT1, bit);
        assert_eq!(read32(&gpio, INTSTAT1), 0);
        gpio.set_external(42, External::Level(false)).unwrap();
        assert_eq!(read32(&gpio, INTSTAT1), 0, "falling edge is not enabled");
        write32(&mut gpio, CLR_RIS_TRIG1, bit);
        write32(&mut gpio, SET_FAL_TRIG1, bit);
        gpio.set_external(42, External::Level(true)).unwrap();
        assert_eq!(read32(&gpio, INTSTAT1), 0);
        gpio.set_external(42, External::Level(false)).unwrap();
        assert_eq!(read32(&gpio, INTSTAT1), bit);
    }

    #[test]
    fn binten_has_one_bit_per_sixteen_gpios() {
        let mut gpio = TiDavinciGpio::new(92);
        write32(&mut gpio, BINTEN, u32::MAX);
        assert_eq!(read32(&gpio, BINTEN), 0x3F);
    }

    #[test]
    fn node_reset_restores_registers_and_keeps_the_board_levels() {
        let mut gpio = TiDavinciGpio::new(92);
        gpio.set_external(32, External::Level(true)).unwrap();
        write32(&mut gpio, DIR1, 0);
        write32(&mut gpio, SET_FAL_TRIG1, 1);
        gpio.on_node_reset();
        assert_eq!(read32(&gpio, DIR1), 0xFFFF_FFFF);
        assert_eq!(read32(&gpio, SET_FAL_TRIG1), 0);
        assert_eq!(read32(&gpio, IN_DATA1), 1);
        assert_eq!(read32(&gpio, 0x60), 0x0FFF_FFFF);
    }

    #[test]
    fn snapshots_round_trip() {
        let mut gpio = TiDavinciGpio::new(92);
        gpio.set_external(42, External::Level(true)).unwrap();
        write32(&mut gpio, SET_RIS_TRIG1, 1 << 10);
        let mut other = TiDavinciGpio::new(92);
        other.restore(gpio.snapshot()).unwrap();
        assert_eq!(read32(&other, IN_DATA1), 1 << 10);
        let mut third = TiDavinciGpio::new(92);
        third
            .restore_runtime_snapshot(&gpio.runtime_snapshot())
            .unwrap();
        assert_eq!(read32(&third, SET_RIS_TRIG1), 1 << 10);
    }
}
