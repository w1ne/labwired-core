// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32U5 secure AES accelerator (SAES).
//!
//! Models AES-128/AES-256 block processing in ECB, CBC, and CTR modes through
//! the device's key, IV, DINR, DOUTR and completion registers. GCM/CCM, key
//! derivation, TrustZone key selection, DMA, DPA countermeasures and random
//! reseeding remain outside this model.

use std::cell::RefCell;
use std::collections::VecDeque;

use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::{Aes128, Aes256};

use crate::{Peripheral, SimResult};

const CR: u64 = 0x000;
const SR: u64 = 0x004;
const DINR: u64 = 0x008;
const DOUTR: u64 = 0x00c;
const KEYR0: u64 = 0x010;
const IVR0: u64 = 0x020;
const IER: u64 = 0x300;
const ISR: u64 = 0x304;
const ICR: u64 = 0x308;

const CR_EN: u32 = 1;
const CR_DATATYPE_MASK: u32 = 0b11 << 1;
const CR_MODE_MASK: u32 = 0b11 << 3;
const CR_CHMOD_MASK: u32 = 0b11 << 5;
const CR_KEYSIZE: u32 = 1 << 18;
const CR_IPRST: u32 = 1 << 31;
const SR_WRERR: u32 = 1 << 2;
const SR_BUSY: u32 = 1 << 3;
const SR_CCF: u32 = 1;
const SR_KEYVALID: u32 = 1 << 7;

fn key_reg_offset(index: usize) -> u64 {
    if index < 4 {
        KEYR0 + index as u64 * 4
    } else {
        0x030 + (index as u64 - 4) * 4
    }
}

#[derive(Debug, Clone)]
pub struct Stm32u5Saes {
    registers: Vec<u32>,
    key_written: u8,
    input: Vec<u8>,
    output: RefCell<VecDeque<u32>>,
    output_byte: RefCell<u8>,
    iv: [u8; 16],
}

impl Default for Stm32u5Saes {
    fn default() -> Self {
        Self::new()
    }
}

impl Stm32u5Saes {
    pub fn new() -> Self {
        Self {
            registers: vec![0; (0x30c / 4) as usize],
            key_written: 0,
            input: Vec::with_capacity(16),
            output: RefCell::new(VecDeque::new()),
            output_byte: RefCell::new(0),
            iv: [0; 16],
        }
    }

    fn word(&self, offset: u64) -> u32 {
        self.registers
            .get((offset / 4) as usize)
            .copied()
            .unwrap_or(0)
    }

    fn set_word(&mut self, offset: u64, value: u32) {
        if let Some(reg) = self.registers.get_mut((offset / 4) as usize) {
            *reg = value;
        }
    }

    fn data_in_bytes(&self, value: u32) -> [u8; 4] {
        match (self.word(CR) & CR_DATATYPE_MASK) >> 1 {
            0 => value.to_be_bytes(),
            1 => value.rotate_left(16).to_be_bytes(),
            2 => value.to_le_bytes(),
            _ => value.reverse_bits().to_be_bytes(),
        }
    }

    fn data_out_word(&self, bytes: [u8; 4]) -> u32 {
        match (self.word(CR) & CR_DATATYPE_MASK) >> 1 {
            0 => u32::from_be_bytes(bytes),
            1 => u32::from_be_bytes(bytes).rotate_right(16),
            2 => u32::from_le_bytes(bytes),
            _ => u32::from_be_bytes(bytes).reverse_bits(),
        }
    }

    fn key_bytes(&self) -> Vec<u8> {
        let count = if self.word(CR) & CR_KEYSIZE != 0 {
            8
        } else {
            4
        };
        (0..count)
            .flat_map(|index| self.word(key_reg_offset(index)).to_le_bytes())
            .collect()
    }

    fn read_iv(&self) -> [u8; 16] {
        let mut iv = [0; 16];
        for index in 0..4 {
            iv[index * 4..index * 4 + 4]
                .copy_from_slice(&self.word(IVR0 + index as u64 * 4).to_le_bytes());
        }
        iv
    }

    fn write_iv(&mut self, iv: [u8; 16]) {
        self.iv = iv;
        for index in 0..4 {
            self.set_word(
                IVR0 + index as u64 * 4,
                u32::from_le_bytes(iv[index * 4..index * 4 + 4].try_into().unwrap()),
            );
        }
    }

    fn crypt_block(&mut self, mut block: [u8; 16]) -> Result<[u8; 16], ()> {
        let cr = self.word(CR);
        let mode = (cr & CR_MODE_MASK) >> 3;
        if mode > 1 {
            return Err(());
        }
        let encrypt = mode == 0;
        let chaining = (cr & CR_CHMOD_MASK) >> 5;
        let key = self.key_bytes();

        match chaining {
            0 => {
                if key.len() == 16 {
                    let cipher = Aes128::new_from_slice(&key).map_err(|_| ())?;
                    if encrypt {
                        cipher.encrypt_block((&mut block).into());
                    } else {
                        cipher.decrypt_block((&mut block).into());
                    }
                } else {
                    let cipher = Aes256::new_from_slice(&key).map_err(|_| ())?;
                    if encrypt {
                        cipher.encrypt_block((&mut block).into());
                    } else {
                        cipher.decrypt_block((&mut block).into());
                    }
                }
            }
            1 => {
                if encrypt {
                    for (byte, iv_byte) in block.iter_mut().zip(self.iv) {
                        *byte ^= iv_byte;
                    }
                    if key.len() == 16 {
                        Aes128::new_from_slice(&key)
                            .map_err(|_| ())?
                            .encrypt_block((&mut block).into());
                    } else {
                        Aes256::new_from_slice(&key)
                            .map_err(|_| ())?
                            .encrypt_block((&mut block).into());
                    }
                    self.write_iv(block);
                } else {
                    let ciphertext = block;
                    if key.len() == 16 {
                        Aes128::new_from_slice(&key)
                            .map_err(|_| ())?
                            .decrypt_block((&mut block).into());
                    } else {
                        Aes256::new_from_slice(&key)
                            .map_err(|_| ())?
                            .decrypt_block((&mut block).into());
                    }
                    for (byte, iv_byte) in block.iter_mut().zip(self.iv) {
                        *byte ^= iv_byte;
                    }
                    self.write_iv(ciphertext);
                }
            }
            2 => {
                let mut counter = self.iv;
                if key.len() == 16 {
                    Aes128::new_from_slice(&key)
                        .map_err(|_| ())?
                        .encrypt_block((&mut counter).into());
                } else {
                    Aes256::new_from_slice(&key)
                        .map_err(|_| ())?
                        .encrypt_block((&mut counter).into());
                }
                for index in 0..16 {
                    block[index] ^= counter[index];
                }
                for byte in self.iv.iter_mut().rev() {
                    let (next, carry) = byte.overflowing_add(1);
                    *byte = next;
                    if !carry {
                        break;
                    }
                }
                self.write_iv(self.iv);
            }
            _ => return Err(()),
        }
        Ok(block)
    }

    fn process_block(&mut self) {
        let required_key_words = if self.word(CR) & CR_KEYSIZE != 0 {
            0xff
        } else {
            0x0f
        };
        if self.word(CR) & CR_EN == 0 || self.key_written & required_key_words != required_key_words
        {
            self.set_word(SR, self.word(SR) | SR_WRERR);
            self.input.clear();
            return;
        }
        let mut block = [0; 16];
        block.copy_from_slice(&self.input[..16]);
        self.set_word(SR, (self.word(SR) | SR_BUSY) & !SR_CCF);
        match self.crypt_block(block) {
            Ok(result) => {
                for bytes in result.chunks_exact(4) {
                    self.output
                        .borrow_mut()
                        .push_back(self.data_out_word(bytes.try_into().unwrap()));
                }
                self.set_word(SR, (self.word(SR) & !SR_BUSY) | SR_CCF);
                self.set_word(ISR, self.word(ISR) | SR_CCF);
            }
            Err(()) => self.set_word(SR, (self.word(SR) & !SR_BUSY) | SR_WRERR),
        }
        self.input.drain(..16);
    }

    fn write_u32_inner(&mut self, offset: u64, value: u32) {
        match offset {
            CR => {
                self.set_word(CR, value & !CR_IPRST);
                if value & CR_IPRST != 0 {
                    self.registers.fill(0);
                    self.key_written = 0;
                    self.input.clear();
                    self.output.borrow_mut().clear();
                    *self.output_byte.borrow_mut() = 0;
                    self.iv = [0; 16];
                }
            }
            DINR => {
                self.input.extend_from_slice(&self.data_in_bytes(value));
                if self.input.len() >= 16 {
                    self.process_block();
                }
            }
            ICR => {
                self.set_word(ISR, self.word(ISR) & !value);
                self.set_word(SR, self.word(SR) & !value);
                self.set_word(ICR, value);
            }
            _ if (KEYR0..=KEYR0 + 3 * 4).contains(&offset) || (0x030..=0x03c).contains(&offset) => {
                let index = if offset < 0x020 {
                    ((offset - KEYR0) / 4) as u8
                } else {
                    ((offset - 0x030) / 4 + 4) as u8
                };
                self.set_word(offset, value);
                self.key_written |= 1 << index;
                let needed = if self.word(CR) & CR_KEYSIZE != 0 {
                    0xff
                } else {
                    0x0f
                };
                if self.key_written & needed == needed {
                    self.set_word(SR, self.word(SR) | SR_KEYVALID);
                }
            }
            _ if (IVR0..=IVR0 + 3 * 4).contains(&offset) => {
                self.set_word(offset, value);
                self.iv = self.read_iv();
            }
            _ => self.set_word(offset, value),
        }
    }
}

impl Peripheral for Stm32u5Saes {
    fn read(&self, offset: u64) -> SimResult<u8> {
        if offset & !3 == DOUTR {
            let mut output = self.output.borrow_mut();
            let word = output.front().copied().unwrap_or(0);
            let byte_index = *self.output_byte.borrow() as u32;
            let byte = ((word >> (byte_index * 8)) & 0xff) as u8;
            if byte_index == 3 {
                output.pop_front();
                *self.output_byte.borrow_mut() = 0;
            } else {
                *self.output_byte.borrow_mut() += 1;
            }
            return Ok(byte);
        }
        let aligned = offset & !3;
        let shift = ((offset & 3) * 8) as u32;
        Ok(((self.word(aligned) >> shift) & 0xff) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let aligned = offset & !3;
        if aligned == DINR {
            self.input.push(value);
            if self.input.len() >= 16 {
                self.process_block();
            }
            return Ok(());
        }
        let shift = ((offset & 3) * 8) as u32;
        let old = self.word(aligned);
        self.write_u32_inner(
            aligned,
            (old & !(0xff << shift)) | ((value as u32) << shift),
        );
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if offset == DOUTR {
            *self.output_byte.borrow_mut() = 0;
            return Ok(self.output.borrow_mut().pop_front().unwrap_or(0));
        }
        Ok(self.word(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_u32_inner(offset & !3, value);
        Ok(())
    }

    fn irq_line_level(&self) -> Option<bool> {
        let pending = self.word(ISR) & self.word(IER);
        Some(pending != 0)
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

    #[test]
    fn aes128_ecb_matches_nist_block_vector() {
        let mut saes = Stm32u5Saes::new();
        let key = [
            0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf,
            0x4f, 0x3c,
        ];
        let plaintext = [
            0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93,
            0x17, 0x2a,
        ];
        saes.write_u32(CR, CR_EN | (2 << 1)).unwrap();
        for (index, word) in key.chunks_exact(4).enumerate() {
            saes.write_u32(
                KEYR0 + index as u64 * 4,
                u32::from_le_bytes(word.try_into().unwrap()),
            )
            .unwrap();
        }
        for word in plaintext.chunks_exact(4) {
            saes.write_u32(DINR, u32::from_le_bytes(word.try_into().unwrap()))
                .unwrap();
        }
        let mut ciphertext = Vec::new();
        for _ in 0..4 {
            ciphertext
                .extend_from_slice(&saes.output.borrow_mut().pop_front().unwrap().to_le_bytes());
        }
        assert_eq!(
            ciphertext,
            [
                0x3a, 0xd7, 0x7b, 0xb4, 0x0d, 0x7a, 0x36, 0x60, 0xa8, 0x9e, 0xca, 0xf3, 0x24, 0x66,
                0xef, 0x97,
            ]
        );
        assert_eq!(saes.word(SR) & SR_CCF, SR_CCF);
    }

    #[test]
    fn aes128_cbc_and_ctr_match_nist_sp800_38a_first_blocks() {
        let key = [
            0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf,
            0x4f, 0x3c,
        ];
        let plaintext = [
            0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93,
            0x17, 0x2a,
        ];
        let cases = [
            (
                1,
                [
                    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
                    0x0d, 0x0e, 0x0f,
                ],
                [
                    0x76, 0x49, 0xab, 0xac, 0x81, 0x19, 0xb2, 0x46, 0xce, 0xe9, 0x8e, 0x9b, 0x12,
                    0xe9, 0x19, 0x7d,
                ],
            ),
            (
                2,
                [
                    0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xfb, 0xfc,
                    0xfd, 0xfe, 0xff,
                ],
                [
                    0x87, 0x4d, 0x61, 0x91, 0xb6, 0x20, 0xe3, 0x26, 0x1b, 0xef, 0x68, 0x64, 0x99,
                    0x0d, 0xb6, 0xce,
                ],
            ),
        ];

        for (chaining, iv, expected) in cases {
            let mut saes = Stm32u5Saes::new();
            saes.write_u32(CR, CR_EN | (2 << 1) | (chaining << 5))
                .unwrap();
            for (index, word) in key.chunks_exact(4).enumerate() {
                saes.write_u32(
                    KEYR0 + index as u64 * 4,
                    u32::from_le_bytes(word.try_into().unwrap()),
                )
                .unwrap();
            }
            for (index, word) in iv.chunks_exact(4).enumerate() {
                saes.write_u32(
                    IVR0 + index as u64 * 4,
                    u32::from_le_bytes(word.try_into().unwrap()),
                )
                .unwrap();
            }
            for word in plaintext.chunks_exact(4) {
                saes.write_u32(DINR, u32::from_le_bytes(word.try_into().unwrap()))
                    .unwrap();
            }
            let output: Vec<_> = (0..4)
                .flat_map(|_| saes.read_u32(DOUTR).unwrap().to_le_bytes())
                .collect();
            assert_eq!(output, expected);
            assert_eq!(saes.word(SR) & SR_CCF, SR_CCF);
        }
    }
}
