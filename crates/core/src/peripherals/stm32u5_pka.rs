// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32U5 public-key accelerator (PKA).
//!
//! PKA RAM follows the STM32U5 register layout. This model executes modular
//! exponentiation, modular add/subtract/inverse/reduction, arithmetic
//! add/subtract/multiply, and comparison operations. ECC, RSA-CRT, Montgomery
//! parameter/multiply, and protected exponentiation remain unsupported and
//! report OPERRF instead of fabricating a result.

use num_bigint::{BigInt, BigUint, Sign};

use crate::{Peripheral, SimResult};

const CR: u64 = 0x000;
const SR: u64 = 0x004;
const CLRFR: u64 = 0x008;
const RAM_BASE: u64 = 0x400;
const RAM_WORDS: usize = 1334;
const MAX_OPERAND_BITS: u32 = 4160;

const CR_EN: u32 = 1;
const CR_START: u32 = 1 << 1;
const CR_PROCENDIE: u32 = 1 << 17;
const CR_RAMERRIE: u32 = 1 << 19;
const CR_ADDRERRIE: u32 = 1 << 20;
const CR_OPERRIE: u32 = 1 << 21;
const SR_INITOK: u32 = 1;
const SR_BUSY: u32 = 1 << 16;
const SR_PROCENDF: u32 = 1 << 17;
const SR_RAMERRF: u32 = 1 << 19;
const SR_ADDRERRF: u32 = 1 << 20;
const SR_OPERRF: u32 = 1 << 21;
const CLR_FLAGS: u32 = SR_PROCENDF | SR_RAMERRF | SR_ADDRERRF | SR_OPERRF;

const MODE_MODULAR_EXP: u32 = 0x00;
const MODE_MODULAR_EXP_FAST: u32 = 0x02;
const MODE_MODULAR_INV: u32 = 0x08;
const MODE_ARITH_ADD: u32 = 0x09;
const MODE_ARITH_SUB: u32 = 0x0a;
const MODE_ARITH_MUL: u32 = 0x0b;
const MODE_COMPARE: u32 = 0x0c;
const MODE_MODULAR_RED: u32 = 0x0d;
const MODE_MODULAR_ADD: u32 = 0x0e;
const MODE_MODULAR_SUB: u32 = 0x0f;

// PKA RAM indices from ST's STM32U585/U5A5 CMSIS device header. The first
// index is relative to the PKA RAM window, which begins at PKA base + 0x400.
const MOD_EXP_EXP_BITS: usize = 0;
const OP_BITS: usize = 2;
const MOD_EXP_BASE: usize = (0x0c68 - 0x0400) / 4;
const MOD_EXP_EXPONENT: usize = (0x0e78 - 0x0400) / 4;
const MOD_EXP_MODULUS: usize = (0x1088 - 0x0400) / 4;
const MOD_EXP_RESULT: usize = (0x0838 - 0x0400) / 4;
const MOD_EXP_ERROR: usize = (0x1298 - 0x0400) / 4;
const ARITH_OP1: usize = (0x0a50 - 0x0400) / 4;
const ARITH_OP2: usize = (0x0c68 - 0x0400) / 4;
const ARITH_OP3: usize = (0x1088 - 0x0400) / 4;
const ARITH_RESULT: usize = (0x0e78 - 0x0400) / 4;

#[derive(Debug, Clone)]
pub struct Stm32u5Pka {
    registers: [u32; 3],
    ram: Vec<u32>,
}

impl Default for Stm32u5Pka {
    fn default() -> Self {
        Self::new()
    }
}

impl Stm32u5Pka {
    pub fn new() -> Self {
        Self {
            registers: [0, SR_INITOK, 0],
            ram: vec![0; RAM_WORDS],
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            CR | SR | CLRFR => self.registers[(offset / 4) as usize],
            _ if offset >= RAM_BASE => self
                .ram
                .get(((offset - RAM_BASE) / 4) as usize)
                .copied()
                .unwrap_or(0),
            _ => 0,
        }
    }

    fn write_ram_word(&mut self, index: usize, value: u32) {
        if let Some(word) = self.ram.get_mut(index) {
            *word = value;
        } else {
            self.registers[1] |= SR_RAMERRF;
        }
    }

    fn number(&self, start: usize, words: usize) -> BigUint {
        let bytes = self
            .ram
            .get(start..start.saturating_add(words).min(self.ram.len()))
            .unwrap_or(&[])
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect::<Vec<_>>();
        BigUint::from_bytes_le(&bytes)
    }

    fn store_number(&mut self, start: usize, words: usize, value: &BigUint) {
        let bytes = value.to_bytes_le();
        for index in 0..words {
            let mut chunk = [0; 4];
            let at = index * 4;
            if at < bytes.len() {
                let count = (bytes.len() - at).min(4);
                chunk[..count].copy_from_slice(&bytes[at..at + count]);
            }
            self.write_ram_word(start + index, u32::from_le_bytes(chunk));
        }
    }

    fn words_for_bits(bits: u32) -> usize {
        (bits as usize).div_ceil(32).max(1)
    }

    fn modular_inverse(value: &BigUint, modulus: &BigUint) -> Option<BigUint> {
        if modulus == &BigUint::from(0u8) {
            return None;
        }
        let mut old_r = BigInt::from_biguint(Sign::Plus, modulus.clone());
        let mut r = BigInt::from_biguint(Sign::Plus, value % modulus);
        let mut old_t = BigInt::from(0);
        let mut t = BigInt::from(1);
        while r != BigInt::from(0) {
            let quotient = &old_r / &r;
            (old_r, r) = (r.clone(), old_r - &quotient * &r);
            (old_t, t) = (t.clone(), old_t - quotient * &t);
        }
        if old_r != BigInt::from(1) {
            return None;
        }
        let signed_modulus = BigInt::from_biguint(Sign::Plus, modulus.clone());
        old_t %= &signed_modulus;
        if old_t < BigInt::from(0) {
            old_t += signed_modulus;
        }
        old_t.to_biguint()
    }

    fn execute(&mut self) {
        let control = self.registers[0];
        let mode = (control >> 8) & 0x3f;
        let bits = self.ram[OP_BITS];
        let words = Self::words_for_bits(bits.min(MAX_OPERAND_BITS));
        self.registers[1] = (self.registers[1] | SR_BUSY) & !CLR_FLAGS;

        let success = if bits > MAX_OPERAND_BITS {
            false
        } else {
            match mode {
                MODE_MODULAR_EXP | MODE_MODULAR_EXP_FAST => {
                    let exponent_bits = self.ram[MOD_EXP_EXP_BITS];
                    if exponent_bits > MAX_OPERAND_BITS {
                        false
                    } else {
                        let exponent_words = Self::words_for_bits(exponent_bits);
                        let base = self.number(MOD_EXP_BASE, words);
                        let exponent = self.number(MOD_EXP_EXPONENT, exponent_words);
                        let modulus = self.number(MOD_EXP_MODULUS, words);
                        if modulus == BigUint::from(0u8) {
                            false
                        } else {
                            let result = base.modpow(&exponent, &modulus);
                            self.store_number(MOD_EXP_RESULT, words, &result);
                            self.ram[MOD_EXP_ERROR] = 0xd60d;
                            true
                        }
                    }
                }
                MODE_ARITH_ADD => {
                    let result = self.number(ARITH_OP1, words) + self.number(ARITH_OP2, words);
                    self.store_number(ARITH_RESULT, words + 1, &result);
                    true
                }
                MODE_ARITH_SUB => {
                    let left = self.number(ARITH_OP1, words);
                    let right = self.number(ARITH_OP2, words);
                    let negative = left < right;
                    let result = if negative {
                        // PKA arithmetic subtraction wraps at the operand's
                        // 32-bit word width. It stores the sign in the word just
                        // after the result; A < B is a valid completed operation.
                        (BigUint::from(1u8) << words.saturating_mul(32)) + left - right
                    } else {
                        left - right
                    };
                    self.store_number(ARITH_RESULT, words, &result);
                    self.write_ram_word(ARITH_RESULT + words, if negative { u32::MAX } else { 0 });
                    true
                }
                MODE_ARITH_MUL => {
                    let result = self.number(ARITH_OP1, words) * self.number(ARITH_OP2, words);
                    self.store_number(ARITH_RESULT, words.saturating_mul(2), &result);
                    true
                }
                MODE_COMPARE => {
                    let left = self.number(ARITH_OP1, words);
                    let right = self.number(ARITH_OP2, words);
                    let comparison = match left.cmp(&right) {
                        std::cmp::Ordering::Equal => 0,
                        std::cmp::Ordering::Greater => 1,
                        std::cmp::Ordering::Less => 2,
                    };
                    self.ram[ARITH_RESULT] = comparison;
                    true
                }
                MODE_MODULAR_ADD | MODE_MODULAR_SUB | MODE_MODULAR_INV => {
                    let left = self.number(ARITH_OP1, words);
                    let right = self.number(ARITH_OP2, words);
                    let modulus = if mode == MODE_MODULAR_INV {
                        right.clone()
                    } else {
                        self.number(ARITH_OP3, words)
                    };
                    if modulus == BigUint::from(0u8) {
                        false
                    } else {
                        let result = match mode {
                            MODE_MODULAR_ADD => Some((left + right) % &modulus),
                            MODE_MODULAR_SUB => {
                                let left = left % &modulus;
                                let right = right % &modulus;
                                Some(if left >= right {
                                    left - right
                                } else {
                                    &modulus + left - right
                                })
                            }
                            _ => Self::modular_inverse(&left, &modulus),
                        };
                        if let Some(result) = result {
                            self.store_number(ARITH_RESULT, words, &result);
                            true
                        } else {
                            false
                        }
                    }
                }
                MODE_MODULAR_RED => {
                    let operand_bits = self.ram[MOD_EXP_EXP_BITS];
                    if operand_bits > MAX_OPERAND_BITS {
                        false
                    } else {
                        let operand_words = Self::words_for_bits(operand_bits);
                        let modulus_words = words;
                        let operand = self.number(ARITH_OP1, operand_words);
                        let modulus = self.number(ARITH_OP2, modulus_words);
                        if modulus == BigUint::from(0u8) {
                            false
                        } else {
                            self.store_number(ARITH_RESULT, modulus_words, &(operand % modulus));
                            true
                        }
                    }
                }
                _ => false,
            }
        };

        self.registers[1] &= !SR_BUSY;
        self.registers[1] |= SR_PROCENDF;
        if !success {
            self.registers[1] |= SR_OPERRF;
            if mode == MODE_MODULAR_EXP || mode == MODE_MODULAR_EXP_FAST {
                self.ram[MOD_EXP_ERROR] = 0xcbc9;
            }
        }
    }

    fn write_control(&mut self, value: u32) {
        self.registers[0] = value & !CR_START;
        if value & CR_START != 0 {
            if value & CR_EN != 0 {
                self.execute();
            } else {
                self.registers[1] |= SR_OPERRF;
            }
        }
    }
}

impl Peripheral for Stm32u5Pka {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let aligned = offset & !3;
        let shift = ((offset & 3) * 8) as u32;
        Ok(((self.read_word(aligned) >> shift) & 0xff) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        if aligned >= RAM_BASE {
            let index = ((aligned - RAM_BASE) / 4) as usize;
            let shift = ((offset & 3) * 8) as u32;
            let old = self.ram.get(index).copied().unwrap_or(0);
            self.write_ram_word(index, (old & !(0xff << shift)) | ((value as u32) << shift));
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_word(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let offset = offset & !3;
        match offset {
            CR => self.write_control(value),
            SR => {}
            CLRFR => {
                self.registers[1] &= !(value & CLR_FLAGS);
                self.registers[2] = value & CLR_FLAGS;
            }
            _ if offset >= RAM_BASE => {
                let index = ((offset - RAM_BASE) / 4) as usize;
                self.write_ram_word(index, value);
            }
            _ => {}
        }
        Ok(())
    }

    fn irq_line_level(&self) -> Option<bool> {
        let control = self.registers[0];
        let status = self.registers[1];
        Some(
            (status & SR_PROCENDF != 0 && control & CR_PROCENDIE != 0)
                || (status & SR_RAMERRF != 0 && control & CR_RAMERRIE != 0)
                || (status & SR_ADDRERRF != 0 && control & CR_ADDRERRIE != 0)
                || (status & SR_OPERRF != 0 && control & CR_OPERRIE != 0),
        )
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn reg_index(offset: u32) -> u64 {
        RAM_BASE + ((offset - 0x400) as u64)
    }

    #[test]
    fn modular_exponentiation_executes_and_sets_completion() {
        let mut pka = Stm32u5Pka::new();
        // ST HAL's modular exponentiation RAM layout: 4^13 mod 497 = 445.
        pka.write_u32(reg_index(0x400), 4).unwrap(); // exponent bit count
        pka.write_u32(reg_index(0x408), 9).unwrap(); // operand bit count
        pka.write_u32(reg_index(0x0c68), 4).unwrap(); // base
        pka.write_u32(reg_index(0x0e78), 13).unwrap(); // exponent
        pka.write_u32(reg_index(0x1088), 497).unwrap(); // modulus
        pka.write_u32(CR, CR_EN | CR_START).unwrap();
        assert_eq!(pka.read_u32(reg_index(0x0838)).unwrap(), 445);
        assert_eq!(pka.read_u32(SR).unwrap() & SR_PROCENDF, SR_PROCENDF);
        assert_eq!(pka.read_u32(SR).unwrap() & SR_OPERRF, 0);
    }

    #[test]
    fn unsupported_modes_raise_operation_error() {
        let mut pka = Stm32u5Pka::new();
        pka.write_u32(CR, CR_EN | CR_START | (0x20 << 8)).unwrap();
        assert_eq!(pka.read_u32(SR).unwrap() & SR_OPERRF, SR_OPERRF);
    }

    #[test]
    fn arithmetic_and_modular_add_execute_from_pka_ram() {
        let mut add = Stm32u5Pka::new();
        add.write_u32(reg_index(0x408), 8).unwrap(); // operand bit count
        add.write_u32(reg_index(0x0a50), 7).unwrap();
        add.write_u32(reg_index(0x0c68), 5).unwrap();
        add.write_u32(CR, CR_EN | CR_START | (MODE_ARITH_ADD << 8))
            .unwrap();
        assert_eq!(add.read_u32(reg_index(0x0e78)).unwrap(), 12);

        let mut modular_add = Stm32u5Pka::new();
        modular_add.write_u32(reg_index(0x408), 8).unwrap();
        modular_add.write_u32(reg_index(0x0a50), 7).unwrap();
        modular_add.write_u32(reg_index(0x0c68), 5).unwrap();
        modular_add.write_u32(reg_index(0x1088), 11).unwrap();
        modular_add
            .write_u32(CR, CR_EN | CR_START | (MODE_MODULAR_ADD << 8))
            .unwrap();
        assert_eq!(modular_add.read_u32(reg_index(0x0e78)).unwrap(), 1);
    }

    #[test]
    fn start_without_enable_reports_an_operation_error() {
        let mut pka = Stm32u5Pka::new();
        pka.write_u32(CR, CR_START).unwrap();
        assert_eq!(pka.read_u32(SR).unwrap() & SR_OPERRF, SR_OPERRF);
        assert_eq!(pka.read_u32(SR).unwrap() & SR_PROCENDF, 0);
    }
}
