// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32U5 HASH/HMAC accelerator.
//!
//! The model accepts the firmware-visible HASH DIN words, applies DATATYPE
//! byte ordering and STR.NBLW, and produces the digest in HR0..HR7. HMAC follows
//! the three DIN/DCAL phases used by the STM32 HAL: key, message, key. Work is
//! completed at DCAL so normal polling firmware sees silicon-like completion
//! flags without host wall-clock timing.

use hmac::{Hmac, Mac};
use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha224, Sha256};

use crate::{Peripheral, PeripheralTickResult, SimResult};

const CR: u64 = 0x00;
const DIN: u64 = 0x04;
const STR: u64 = 0x08;
const HRA0: u64 = 0x0c;
const IMR: u64 = 0x20;
const SR: u64 = 0x24;
const HR0: u64 = 0x310;

const CR_INIT: u32 = 1 << 2;
const CR_DATATYPE_MASK: u32 = 0b11 << 4;
const CR_MODE_HMAC: u32 = 1 << 6;
const CR_ALGO_MASK: u32 = 0b11 << 17;
const CR_DINNE: u32 = 1 << 12;
const STR_DCAL: u32 = 1 << 8;
const STR_NBLW_MASK: u32 = 0x1f;
const SR_DINIS: u32 = 1;
const SR_DCIS: u32 = 1 << 1;
const IMR_DINIE: u32 = 1;
const IMR_DCIE: u32 = 1 << 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Algorithm {
    Sha1,
    Md5,
    Sha224,
    Sha256,
}

impl Algorithm {
    fn from_cr(value: u32) -> Self {
        match (value & CR_ALGO_MASK) >> 17 {
            0 => Self::Sha1,
            1 => Self::Md5,
            2 => Self::Sha224,
            _ => Self::Sha256,
        }
    }

    fn digest(self, input: &[u8]) -> Vec<u8> {
        match self {
            Self::Sha1 => Sha1::digest(input).to_vec(),
            Self::Md5 => Md5::digest(input).to_vec(),
            Self::Sha224 => Sha224::digest(input).to_vec(),
            Self::Sha256 => Sha256::digest(input).to_vec(),
        }
    }

    fn hmac(self, key: &[u8], input: &[u8]) -> Vec<u8> {
        macro_rules! calculate {
            ($digest:ty) => {{
                let Ok(mut mac) = Hmac::<$digest>::new_from_slice(key) else {
                    return Vec::new();
                };
                mac.update(input);
                mac.finalize().into_bytes().to_vec()
            }};
        }
        match self {
            Self::Sha1 => calculate!(Sha1),
            Self::Md5 => calculate!(Md5),
            Self::Sha224 => calculate!(Sha224),
            Self::Sha256 => calculate!(Sha256),
        }
    }
}

/// Register-level STM32U5 HASH/HMAC engine with synchronous digest completion.
#[derive(Debug, Clone)]
pub struct Stm32u5Hash {
    registers: Vec<u32>,
    input: Vec<u8>,
    hmac_key: Vec<u8>,
    hmac_message: Vec<u8>,
    hmac_phase: u8,
}

impl Default for Stm32u5Hash {
    fn default() -> Self {
        Self::new()
    }
}

impl Stm32u5Hash {
    pub fn new() -> Self {
        let mut registers = vec![0; (0x330 / 4) as usize];
        registers[(SR / 4) as usize] = SR_DINIS;
        Self {
            registers,
            input: Vec::new(),
            hmac_key: Vec::new(),
            hmac_message: Vec::new(),
            hmac_phase: 0,
        }
    }

    fn word(&self, offset: u64) -> u32 {
        self.registers
            .get((offset / 4) as usize)
            .copied()
            .unwrap_or(0)
    }

    fn set_word(&mut self, offset: u64, value: u32) {
        if let Some(word) = self.registers.get_mut((offset / 4) as usize) {
            *word = value;
        }
    }

    fn reset_calculation(&mut self) {
        self.input.clear();
        self.hmac_key.clear();
        self.hmac_message.clear();
        self.hmac_phase = 0;
        self.set_word(SR, SR_DINIS);
        self.set_word(STR, 0);
        self.set_word(CR, self.word(CR) & !CR_DINNE);
        for offset in (HRA0..HRA0 + 20).step_by(4) {
            self.set_word(offset, 0);
        }
        for offset in (HR0..HR0 + 32).step_by(4) {
            self.set_word(offset, 0);
        }
    }

    fn data_word_bytes(&self, value: u32) -> [u8; 4] {
        match (self.word(CR) & CR_DATATYPE_MASK) >> 4 {
            0 => value.to_be_bytes(),
            1 => value.rotate_left(16).to_be_bytes(),
            2 => value.to_le_bytes(),
            _ => value.reverse_bits().to_be_bytes(),
        }
    }

    fn push_data_word(&mut self, value: u32) {
        self.input.extend_from_slice(&self.data_word_bytes(value));
        self.set_word(CR, self.word(CR) | CR_DINNE);
        self.set_word(SR, self.word(SR) & !SR_DINIS);
    }

    fn current_payload(&self) -> Vec<u8> {
        let mut bytes = self.input.clone();
        if !bytes.is_empty() {
            let valid_bits = (self.word(STR) & STR_NBLW_MASK) as usize;
            if valid_bits != 0 {
                let valid_bytes = valid_bits.div_ceil(8).min(4);
                bytes.truncate(bytes.len().saturating_sub(4).saturating_add(valid_bytes));
            }
        }
        bytes
    }

    fn finish_digest(&mut self, digest: &[u8]) {
        for (index, chunk) in digest.chunks(4).enumerate() {
            let mut bytes = [0; 4];
            bytes[..chunk.len()].copy_from_slice(chunk);
            self.set_word(HR0 + index as u64 * 4, u32::from_be_bytes(bytes));
        }
        self.set_word(SR, self.word(SR) | SR_DCIS | SR_DINIS);
        self.set_word(CR, self.word(CR) & !CR_DINNE);
        self.input.clear();
    }

    fn calculate(&mut self) {
        let payload = self.current_payload();
        self.set_word(STR, self.word(STR) & !STR_DCAL);
        self.set_word(SR, self.word(SR) & !(SR_DCIS | SR_DINIS));
        if self.word(CR) & CR_MODE_HMAC == 0 {
            let digest = Algorithm::from_cr(self.word(CR)).digest(&payload);
            self.finish_digest(&digest);
            return;
        }

        match self.hmac_phase {
            0 => {
                self.hmac_key = payload;
                self.hmac_phase = 1;
                self.set_word(SR, self.word(SR) | SR_DINIS);
            }
            1 => {
                self.hmac_message = payload;
                self.hmac_phase = 2;
                self.set_word(SR, self.word(SR) | SR_DINIS);
            }
            _ => {
                let digest =
                    Algorithm::from_cr(self.word(CR)).hmac(&self.hmac_key, &self.hmac_message);
                self.finish_digest(&digest);
                self.hmac_phase = 3;
            }
        }
        self.input.clear();
    }

    fn write_cr(&mut self, value: u32) {
        let previous = self.word(CR);
        self.set_word(CR, value & !CR_INIT);
        if value & CR_INIT != 0 {
            self.reset_calculation();
            self.set_word(CR, value & !(CR_INIT | CR_DINNE));
        } else if (previous ^ value) & (CR_ALGO_MASK | CR_MODE_HMAC | CR_DATATYPE_MASK) != 0 {
            self.input.clear();
            self.hmac_phase = 0;
        }
    }

    fn write_register_byte(&mut self, offset: u64, value: u8) {
        let aligned = offset & !3;
        let shift = ((offset & 3) * 8) as u32;
        match aligned {
            DIN => self.input.push(value),
            SR => {
                let clear = (value as u32) << shift;
                self.set_word(SR, self.word(SR) & !clear);
            }
            _ => {
                let old = self.word(aligned);
                self.set_word(
                    aligned,
                    (old & !(0xff << shift)) | ((value as u32) << shift),
                );
            }
        }
    }
}

impl Peripheral for Stm32u5Hash {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let aligned = offset & !3;
        let shift = ((offset & 3) * 8) as u32;
        Ok(((self.word(aligned) >> shift) & 0xff) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.write_register_byte(offset, value);
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            CR => self.write_cr(value),
            DIN => self.push_data_word(value),
            STR => {
                self.set_word(STR, value & !STR_DCAL);
                if value & STR_DCAL != 0 {
                    self.calculate();
                }
            }
            SR => self.set_word(SR, self.word(SR) & !value),
            _ => {
                let aligned = offset & !3;
                self.set_word(aligned, value);
            }
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        let status = self.word(SR);
        let mask = self.word(IMR);
        PeripheralTickResult::with_irq(
            (status & SR_DCIS != 0 && mask & IMR_DCIE != 0)
                || (status & SR_DINIS != 0 && mask & IMR_DINIE != 0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc_4231_sha256_case_one() {
        let mut hash = Stm32u5Hash::new();
        hash.write_u32(CR, (3 << 17) | (2 << 4) | CR_MODE_HMAC | CR_INIT)
            .unwrap();
        for payload in [vec![0x0b; 20], b"Hi There".to_vec(), vec![0x0b; 20]] {
            for word in payload.chunks(4) {
                let mut padded = [0; 4];
                padded[..word.len()].copy_from_slice(word);
                hash.write_u32(DIN, u32::from_le_bytes(padded)).unwrap();
            }
            let nblw = ((payload.len() % 4) * 8) as u32;
            hash.write_u32(STR, STR_DCAL | nblw).unwrap();
        }
        let digest = (0..8)
            .flat_map(|index| hash.read_u32(HR0 + index * 4).unwrap().to_be_bytes())
            .collect::<Vec<_>>();
        assert_eq!(
            &digest[..32],
            &[
                0xb0, 0x34, 0x4c, 0x61, 0xd8, 0xdb, 0x38, 0x53, 0x5c, 0xa8, 0xaf, 0xce, 0xaf, 0x0b,
                0xf1, 0x2b, 0x88, 0x1d, 0xc2, 0x00, 0xc9, 0x83, 0x3d, 0xa7, 0x26, 0xe9, 0x37, 0x6c,
                0x2e, 0x32, 0xcf, 0xf7,
            ]
        );
        assert_eq!(hash.read_u32(SR).unwrap() & SR_DCIS, SR_DCIS);
    }
}
