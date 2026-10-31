//! TI CC2538 AES/SHA-256 cryptoprocessor (`AES`, `0x4008_B000`).
//!
//! Source: SWRU319C §22.2 "AES and SHA Cryptoprocessor" and §22.4.1 /
//! Table 22-103 "AES Register Summary" (every offset and reset value below).
//! The block is an EIP-120t AES core, a SHA-256 engine and a key store behind
//! a two-channel DMA controller: channel 0 feeds the engine selected in
//! `CTRL_ALG_SEL` from system memory, channel 1 drains its output to memory.
//!
//! Modelled:
//! - DMA: writing a non-zero `DMALENGTH` on an enabled channel starts the
//!   transfer (Table 22-103 note). Channel 0 is consumed in the next bus tick;
//!   output waits until channel 1 is armed, as the engine's output buffer
//!   does, so the order in which firmware programs the two channels does not
//!   matter. An unmapped address sets `DMA_BUS_ERR` and `DMAC_STATUS.PORT_ERR`;
//! - key store: eight 128-bit areas. `KEY_STORE_SIZE` (128/192/256) sets how
//!   the areas pair up; a channel-0 transfer fills the areas selected in
//!   `KEY_STORE_WRITE_AREA`, which must be contiguous, start on an even area
//!   for 192/256-bit keys, be free, and be covered exactly by the transfer,
//!   or `KEY_ST_WR_ERR` is raised. `KEY_STORE_WRITTEN_AREA` is write-1-to-free.
//!   `KEY_STORE_READ_AREA` copies a written key into the AES core (`BUSY`
//!   never reads 1) or raises `KEY_ST_RD_ERR`;
//! - AES modes (`AES_CTRL`): ECB, CBC, CTR (32/64/96/128-bit counter),
//!   CBC-MAC, CCM (`CCM_L`/`CCM_M`, AAD through a separate channel-0
//!   transfer) and autonomous GCM (mode 11), all three key sizes. The tag is
//!   the full 128-bit block (`TAG_OUT_0..3`; "the M least significant bytes
//!   are valid" for CCM). With `SAVE_CONTEXT` the operation sets
//!   `SAVED_CONTEXT_READY`, which reading `TAG_OUT_3` or `IV_3` (or writing 1)
//!   clears, and the IV registers hold the continuation IV (CBC: last
//!   ciphertext block; CTR/CCM/GCM: next counter block);
//! - SHA-256 over DMA: `HASH_MODE_IN.NEW_HASH` loads the initial constants,
//!   otherwise the engine resumes from `HASH_DIGEST_A..H`;
//!   `HASH_IO_BUF_CTRL.PAD_DMA_MESSAGE` pads the transfer with the bit length
//!   in `HASH_LENGTH_IN` and finalises. With `CTRL_ALG_SEL.TAG` the digest
//!   also goes out through channel 1. `HASH_DIGEST_n` read back in memory
//!   byte order (`HASH_DIGEST_A` = digest bytes 0-3, little-endian), which is
//!   what Contiki-NG's `resume_hash` stores and reloads;
//! - master control: `INT_STAT` holds `RESULT_AV`, `DMA_IN_DONE` and the three
//!   error bits until `INT_CLR` (or `INT_SET`/`SW_RESET`); the interrupt line
//!   (IRQ 143 in the regular map, Table 5-2) is the level `INT_STAT & INT_EN`.
//!
//! NOT modelled: the slave-interface (non-DMA) data path through
//! `AES_DATA_IN_OUT`/`HASH_DATA_IN` and its `INPUT_READY`/`OUTPUT_READY`/
//! `RFD_IN` handshakes, `KEY2`/`KEY3` (GCM/CCM precomputed-key and XTS
//! modes), the pulse interrupt mode (`INT_CFG.LEVEL` = 0 behaves as level),
//! `CTRL_PROT_EN` bus protection, DMA burst timing and processing latency
//! (a transfer completes in the bus tick after it is armed), and the PKA.

use std::cell::Cell;

use aes::cipher::{generic_array::GenericArray, BlockDecrypt, BlockEncrypt, KeyInit};
use aes::{Aes128, Aes192, Aes256};

use super::lane_read;
use crate::{Bus, Peripheral, SimResult};

const DMAC_CH0_CTRL: u64 = 0x000;
const DMAC_CH0_EXTADDR: u64 = 0x004;
const DMAC_CH0_DMALENGTH: u64 = 0x00C;
const DMAC_STATUS: u64 = 0x018;
const DMAC_SWRES: u64 = 0x01C;
const DMAC_CH1_CTRL: u64 = 0x020;
const DMAC_CH1_EXTADDR: u64 = 0x024;
const DMAC_CH1_DMALENGTH: u64 = 0x02C;
const DMAC_MST_RUNPARAMS: u64 = 0x078;
const DMAC_PERSR: u64 = 0x07C;
const DMAC_OPTIONS: u64 = 0x0F8;
const DMAC_VERSION: u64 = 0x0FC;
const KEY_STORE_WRITE_AREA: u64 = 0x400;
const KEY_STORE_WRITTEN_AREA: u64 = 0x404;
const KEY_STORE_SIZE: u64 = 0x408;
const KEY_STORE_READ_AREA: u64 = 0x40C;
const AES_IV_0: u64 = 0x540;
const AES_IV_3: u64 = 0x54C;
const AES_CTRL: u64 = 0x550;
const AES_C_LENGTH_0: u64 = 0x554;
const AES_C_LENGTH_1: u64 = 0x558;
const AES_AUTH_LENGTH: u64 = 0x55C;
const AES_TAG_OUT_0: u64 = 0x570;
const AES_TAG_OUT_3: u64 = 0x57C;
const HASH_IO_BUF_CTRL: u64 = 0x640;
const HASH_MODE_IN: u64 = 0x644;
const HASH_LENGTH_IN_L: u64 = 0x648;
const HASH_LENGTH_IN_H: u64 = 0x64C;
const HASH_DIGEST_A: u64 = 0x650;
const HASH_DIGEST_H: u64 = 0x66C;
const CTRL_ALG_SEL: u64 = 0x700;
const CTRL_PROT_EN: u64 = 0x704;
const CTRL_SW_RESET: u64 = 0x740;
const CTRL_INT_CFG: u64 = 0x780;
const CTRL_INT_EN: u64 = 0x784;
const CTRL_INT_CLR: u64 = 0x788;
const CTRL_INT_SET: u64 = 0x78C;
const CTRL_INT_STAT: u64 = 0x790;
const CTRL_OPTIONS: u64 = 0x7F8;
const CTRL_VERSION: u64 = 0x7FC;

/// Read-only identification values (Table 22-103 resets).
const DMAC_OPTIONS_VALUE: u32 = 0x0000_0202;
const DMAC_VERSION_VALUE: u32 = 0x0101_2ED1;
const CTRL_OPTIONS_VALUE: u32 = 0x0101_01F7;
const CTRL_VERSION_VALUE: u32 = 0x9110_8778;
const DMAC_MST_RUNPARAMS_RESET: u32 = 0x0000_2400;
const HASH_IO_BUF_CTRL_RESET: u32 = 0x0000_0004;

const CH_CTRL_EN: u32 = 1 << 0;
const CH_DMALEN_MASK: u32 = 0xFFFF;
const DMAC_STATUS_CH0_ACT: u32 = 1 << 0;
const DMAC_STATUS_CH1_ACT: u32 = 1 << 1;
const DMAC_STATUS_PORT_ERR: u32 = 1 << 17;

const READ_AREA_NONE: u32 = 0x8;
const READ_AREA_MASK: u32 = 0xF;

const AES_CTRL_CONTEXT_READY: u32 = 1 << 31;
const AES_CTRL_SAVED_CONTEXT_READY: u32 = 1 << 30;
const AES_CTRL_SAVE_CONTEXT: u32 = 1 << 29;
const AES_CTRL_CCM_M_SHIFT: u32 = 22;
const AES_CTRL_CCM_L_SHIFT: u32 = 19;
const AES_CTRL_CCM: u32 = 1 << 18;
const AES_CTRL_GCM: u32 = 0b11 << 16;
const AES_CTRL_CBC_MAC: u32 = 1 << 15;
const AES_CTRL_CTR_WIDTH_SHIFT: u32 = 7;
const AES_CTRL_CTR: u32 = 1 << 6;
const AES_CTRL_CBC: u32 = 1 << 5;
const AES_CTRL_KEY_SIZE_SHIFT: u32 = 3;
const AES_CTRL_KEY_SIZE_MASK: u32 = 0b11 << AES_CTRL_KEY_SIZE_SHIFT;
const AES_CTRL_DIRECTION_ENCRYPT: u32 = 1 << 2;
/// Mode bits firmware writes and reads back (29:5 and 2; the key size and
/// the status bits are not stored).
const AES_CTRL_RW_MASK: u32 = 0x23FF_81E4;

const HASH_IO_PAD_DMA_MESSAGE: u32 = 1 << 7;
const HASH_IO_RFD_IN: u32 = 1 << 2;
const HASH_IO_OUTPUT_FULL: u32 = 1 << 0;
const HASH_MODE_NEW_HASH: u32 = 1 << 0;

const ALG_SEL_TAG: u32 = 1 << 31;
const ALG_SEL_HASH: u32 = 1 << 2;
const ALG_SEL_AES: u32 = 1 << 1;
const ALG_SEL_KEYSTORE: u32 = 1 << 0;
const ALG_SEL_MASK: u32 = ALG_SEL_TAG | ALG_SEL_HASH | ALG_SEL_AES | ALG_SEL_KEYSTORE;

const INT_DMA_BUS_ERR: u32 = 1 << 31;
const INT_KEY_ST_WR_ERR: u32 = 1 << 30;
const INT_KEY_ST_RD_ERR: u32 = 1 << 29;
const INT_DMA_IN_DONE: u32 = 1 << 1;
const INT_RESULT_AV: u32 = 1 << 0;
const INT_STAT_MASK: u32 =
    INT_DMA_BUS_ERR | INT_KEY_ST_WR_ERR | INT_KEY_ST_RD_ERR | INT_DMA_IN_DONE | INT_RESULT_AV;
/// `INT_EN`/`INT_SET` cover only the two interrupt outputs.
const INT_OUTPUTS: u32 = INT_DMA_IN_DONE | INT_RESULT_AV;

/// SHA-256 initial hash value (FIPS 180-4 §5.3.3).
const SHA256_IV: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Cc2538Aes {
    ch_ctrl: [u32; 2],
    ch_addr: [u32; 2],
    /// Programmed `DMALENGTH`; non-zero on an enabled channel = armed.
    ch_len: [u32; 2],
    mst_runparams: u32,
    port_err: bool,

    ks_write_area: u32,
    ks_written: u32,
    ks_size: u32,
    ks_read_area: u32,
    ks_ram: [[u8; 16]; 8],
    /// Key copied into the AES core by the last good `KEY_STORE_READ_AREA`.
    key: Vec<u8>,

    iv: [u32; 4],
    aes_ctrl: u32,
    saved_context_ready: Cell<bool>,
    c_length: u64,
    auth_length: u32,
    tag: [u32; 4],
    /// CCM/GCM additional data received for the running operation.
    aad: Option<Vec<u8>>,

    hash_io: u32,
    hash_mode: u32,
    hash_length: u64,
    digest: [u32; 8],

    alg_sel: u32,
    prot_en: u32,
    int_cfg: u32,
    int_en: u32,
    int_stat: u32,

    /// Engine output waiting for channel 1.
    out_buf: Option<Vec<u8>>,
}

impl Cc2538Aes {
    pub fn new() -> Self {
        Self {
            mst_runparams: DMAC_MST_RUNPARAMS_RESET,
            ks_size: 1,
            ks_read_area: READ_AREA_NONE,
            hash_io: HASH_IO_BUF_CTRL_RESET,
            ..Self::default()
        }
    }

    fn ch_armed(&self, ch: usize) -> bool {
        self.ch_ctrl[ch] & CH_CTRL_EN != 0 && self.ch_len[ch] != 0
    }

    fn irq_level(&self) -> bool {
        self.int_stat & self.int_en & INT_OUTPUTS != 0
    }

    fn has_work(&self) -> bool {
        self.ch_armed(0) || (self.out_buf.is_some() && self.ch_armed(1))
    }

    /// 16 bytes per area at 128-bit keys, 32 (two areas) at 192/256.
    fn areas_per_key(&self) -> u32 {
        if self.ks_size == 1 {
            1
        } else {
            2
        }
    }

    fn key_bytes(&self) -> usize {
        match self.ks_size {
            2 => 24,
            3 => 32,
            _ => 16,
        }
    }

    fn aes_ctrl_value(&self) -> u32 {
        let mut v = self.aes_ctrl & AES_CTRL_RW_MASK & !AES_CTRL_SAVED_CONTEXT_READY;
        if !self.key.is_empty() {
            v |= (self.ks_size << AES_CTRL_KEY_SIZE_SHIFT) & AES_CTRL_KEY_SIZE_MASK;
        }
        // CONTEXT_READY and SAVED_CONTEXT_READY are mutually exclusive.
        if self.saved_context_ready.get() {
            v |= AES_CTRL_SAVED_CONTEXT_READY;
        } else {
            v |= AES_CTRL_CONTEXT_READY;
        }
        v
    }

    fn hash_io_value(&self) -> u32 {
        let mut v = self.hash_io & (HASH_IO_PAD_DMA_MESSAGE | HASH_IO_OUTPUT_FULL);
        v |= HASH_IO_RFD_IN;
        v
    }

    fn read_reg(&self, reg: u64) -> u32 {
        match reg {
            DMAC_CH0_CTRL => self.ch_ctrl[0],
            DMAC_CH0_EXTADDR => self.ch_addr[0],
            DMAC_CH0_DMALENGTH => self.ch_len[0],
            DMAC_CH1_CTRL => self.ch_ctrl[1],
            DMAC_CH1_EXTADDR => self.ch_addr[1],
            DMAC_CH1_DMALENGTH => self.ch_len[1],
            DMAC_STATUS => {
                let mut v = 0;
                if self.ch_armed(0) {
                    v |= DMAC_STATUS_CH0_ACT;
                }
                if self.ch_armed(1) && self.out_buf.is_some() {
                    v |= DMAC_STATUS_CH1_ACT;
                }
                if self.port_err {
                    v |= DMAC_STATUS_PORT_ERR;
                }
                v
            }
            DMAC_MST_RUNPARAMS => self.mst_runparams,
            // No AHB port error channel is latched beyond PORT_ERR.
            DMAC_PERSR => 0,
            DMAC_OPTIONS => DMAC_OPTIONS_VALUE,
            DMAC_VERSION => DMAC_VERSION_VALUE,
            KEY_STORE_WRITE_AREA => self.ks_write_area,
            KEY_STORE_WRITTEN_AREA => self.ks_written,
            KEY_STORE_SIZE => self.ks_size,
            KEY_STORE_READ_AREA => self.ks_read_area,
            AES_IV_0..=AES_IV_3 => {
                if reg == AES_IV_3 {
                    self.saved_context_ready.set(false);
                }
                self.iv[((reg - AES_IV_0) / 4) as usize]
            }
            AES_CTRL => self.aes_ctrl_value(),
            AES_TAG_OUT_0..=AES_TAG_OUT_3 => {
                if reg == AES_TAG_OUT_3 {
                    self.saved_context_ready.set(false);
                }
                self.tag[((reg - AES_TAG_OUT_0) / 4) as usize]
            }
            HASH_IO_BUF_CTRL => self.hash_io_value(),
            HASH_DIGEST_A..=HASH_DIGEST_H => self.digest[((reg - HASH_DIGEST_A) / 4) as usize],
            CTRL_ALG_SEL => self.alg_sel,
            CTRL_PROT_EN => self.prot_en,
            CTRL_INT_CFG => self.int_cfg,
            CTRL_INT_EN => self.int_en,
            CTRL_INT_STAT => self.int_stat,
            CTRL_OPTIONS => CTRL_OPTIONS_VALUE,
            CTRL_VERSION => CTRL_VERSION_VALUE,
            // Write-only registers (DMAC_SWRES, KEY2/KEY3, C_LENGTH,
            // AUTH_LENGTH, DATA_IN_OUT, HASH_DATA_IN, HASH_MODE_IN,
            // HASH_LENGTH_IN, INT_CLR/INT_SET, SW_RESET) and the reserved
            // gaps read 0.
            _ => 0,
        }
    }

    fn write_reg(&mut self, reg: u64, value: u32) {
        match reg {
            DMAC_CH0_CTRL => self.ch_ctrl[0] = value & 0b11,
            DMAC_CH0_EXTADDR => self.ch_addr[0] = value,
            DMAC_CH0_DMALENGTH => self.ch_len[0] = value & CH_DMALEN_MASK,
            DMAC_CH1_CTRL => self.ch_ctrl[1] = value & 0b11,
            DMAC_CH1_EXTADDR => self.ch_addr[1] = value,
            DMAC_CH1_DMALENGTH => self.ch_len[1] = value & CH_DMALEN_MASK,
            DMAC_SWRES if value & 1 != 0 => {
                self.ch_ctrl = [0; 2];
                self.ch_addr = [0; 2];
                self.ch_len = [0; 2];
                self.port_err = false;
                self.out_buf = None;
            }
            DMAC_MST_RUNPARAMS => self.mst_runparams = value,
            KEY_STORE_WRITE_AREA => self.ks_write_area = value & 0xFF,
            KEY_STORE_WRITTEN_AREA => self.ks_written &= !(value & 0xFF),
            KEY_STORE_SIZE => {
                // Writing the size deletes every stored key.
                self.ks_size = match value & 0b11 {
                    0 => self.ks_size,
                    s => s,
                };
                self.ks_written = 0;
            }
            KEY_STORE_READ_AREA => self.load_key(value & READ_AREA_MASK),
            AES_IV_0..=AES_IV_3 => self.iv[((reg - AES_IV_0) / 4) as usize] = value,
            AES_CTRL => {
                if value & AES_CTRL_SAVED_CONTEXT_READY != 0 {
                    // "All other mode bit writes are ignored".
                    self.saved_context_ready.set(false);
                } else {
                    self.aes_ctrl = value & AES_CTRL_RW_MASK;
                    self.aad = None;
                }
            }
            AES_C_LENGTH_0 => {
                self.c_length = (self.c_length & !0xFFFF_FFFF) | u64::from(value);
            }
            AES_C_LENGTH_1 => {
                self.c_length =
                    (self.c_length & 0xFFFF_FFFF) | (u64::from(value & 0x1FFF_FFFF) << 32);
            }
            AES_AUTH_LENGTH => self.auth_length = value,
            HASH_IO_BUF_CTRL => {
                self.hash_io =
                    (self.hash_io & !HASH_IO_PAD_DMA_MESSAGE) | (value & HASH_IO_PAD_DMA_MESSAGE);
                if value & HASH_IO_OUTPUT_FULL != 0 {
                    self.hash_io &= !HASH_IO_OUTPUT_FULL;
                }
            }
            HASH_MODE_IN => self.hash_mode = value,
            HASH_LENGTH_IN_L => {
                self.hash_length = (self.hash_length & !0xFFFF_FFFF) | u64::from(value);
            }
            HASH_LENGTH_IN_H => {
                self.hash_length = (self.hash_length & 0xFFFF_FFFF) | (u64::from(value) << 32);
            }
            HASH_DIGEST_A..=HASH_DIGEST_H => {
                self.digest[((reg - HASH_DIGEST_A) / 4) as usize] = value;
            }
            CTRL_ALG_SEL => self.alg_sel = value & ALG_SEL_MASK,
            CTRL_PROT_EN => self.prot_en = value & 1,
            CTRL_SW_RESET if value & 1 != 0 => {
                self.int_stat = 0;
                self.ks_written = 0;
                self.key.clear();
                self.aad = None;
                self.out_buf = None;
            }
            CTRL_INT_CFG => self.int_cfg = value & 1,
            CTRL_INT_EN => self.int_en = value & INT_OUTPUTS,
            CTRL_INT_CLR => self.int_stat &= !(value & INT_STAT_MASK),
            CTRL_INT_SET => self.int_stat |= value & INT_OUTPUTS,
            _ => {}
        }
    }

    fn load_key(&mut self, area: u32) {
        self.ks_read_area = area;
        if area == READ_AREA_NONE {
            return;
        }
        let n = self.areas_per_key();
        let need = ((1u32 << n) - 1) << area;
        if area >= 8 || area % n != 0 || self.ks_written & need != need {
            self.int_stat |= INT_KEY_ST_RD_ERR;
            return;
        }
        let a = area as usize;
        let mut key = self.ks_ram[a].to_vec();
        if n == 2 {
            key.extend_from_slice(&self.ks_ram[a + 1]);
        }
        key.truncate(self.key_bytes());
        self.key = key;
    }

    /// Pull channel 0's bytes from memory and clear its length.
    fn dma_in(&mut self, bus: &mut dyn Bus) -> Option<Vec<u8>> {
        let base = u64::from(self.ch_addr[0]);
        let len = self.ch_len[0] as usize;
        self.ch_len[0] = 0;
        let mut data = vec![0u8; len];
        let ok = crate::with_device_write_bypass(bus, |bus| {
            data.iter_mut()
                .enumerate()
                .all(|(i, b)| bus.read_u8(base + i as u64).map(|v| *b = v).is_ok())
        });
        if ok {
            Some(data)
        } else {
            self.dma_error();
            None
        }
    }

    fn dma_error(&mut self) {
        self.port_err = true;
        self.int_stat |= INT_DMA_BUS_ERR;
    }

    fn advance(&mut self, bus: &mut dyn Bus) {
        if self.ch_armed(0) {
            if let Some(data) = self.dma_in(bus) {
                self.int_stat |= INT_DMA_IN_DONE;
                self.consume(data);
            }
        }
        if self.out_buf.is_some() && self.ch_armed(1) {
            let out = self.out_buf.take().unwrap_or_default();
            let base = u64::from(self.ch_addr[1]);
            let len = (self.ch_len[1] as usize).min(out.len());
            self.ch_len[1] = 0;
            let ok = crate::with_device_write_bypass(bus, |bus| {
                out[..len]
                    .iter()
                    .enumerate()
                    .all(|(i, b)| bus.write_u8(base + i as u64, *b).is_ok())
            });
            if !ok {
                self.dma_error();
            }
            self.finish();
        }
    }

    /// Route one channel-0 transfer to the engine `CTRL_ALG_SEL` selects.
    fn consume(&mut self, data: Vec<u8>) {
        if self.alg_sel & ALG_SEL_KEYSTORE != 0 {
            self.store_keys(&data);
        } else if self.alg_sel & ALG_SEL_HASH != 0 {
            self.hash(&data);
            if self.alg_sel & ALG_SEL_TAG != 0 {
                self.out_buf = Some(self.digest.iter().flat_map(|w| w.to_le_bytes()).collect());
            } else {
                self.finish();
            }
        } else if self.alg_sel & ALG_SEL_AES != 0 {
            let combined = self.aes_ctrl & (AES_CTRL_CCM | AES_CTRL_GCM) != 0;
            if combined && self.auth_length != 0 && self.aad.is_none() {
                self.aad = Some(data);
                if self.c_length == 0 {
                    self.run_aes(&[]);
                    self.finish();
                }
            } else {
                let out = self.run_aes(&data);
                if self.aes_ctrl & AES_CTRL_CBC_MAC != 0 {
                    self.finish();
                } else {
                    self.out_buf = Some(out);
                }
            }
        }
    }

    fn finish(&mut self) {
        self.int_stat |= INT_RESULT_AV;
        if self.alg_sel & ALG_SEL_HASH != 0 {
            self.hash_io |= HASH_IO_OUTPUT_FULL;
        }
        if self.alg_sel & ALG_SEL_AES != 0 && self.aes_ctrl & AES_CTRL_SAVE_CONTEXT != 0 {
            self.saved_context_ready.set(true);
        }
    }

    fn store_keys(&mut self, data: &[u8]) {
        let areas = self.ks_write_area;
        let n = self.areas_per_key();
        let first = areas.trailing_zeros();
        let count = areas.count_ones();
        let contiguous =
            areas != 0 && (areas >> first).count_ones() == (areas >> first).trailing_ones();
        if !contiguous
            || first % n != 0
            || count % n != 0
            || self.ks_written & areas != 0
            || data.len() != count as usize * 16
        {
            self.int_stat |= INT_KEY_ST_WR_ERR | INT_RESULT_AV;
            return;
        }
        for (i, chunk) in data.chunks_exact(16).enumerate() {
            self.ks_ram[first as usize + i].copy_from_slice(chunk);
        }
        self.ks_written |= areas;
        self.int_stat |= INT_RESULT_AV;
    }

    fn hash(&mut self, data: &[u8]) {
        let mut state = if self.hash_mode & HASH_MODE_NEW_HASH != 0 {
            SHA256_IV
        } else {
            self.digest.map(u32::swap_bytes)
        };
        self.hash_mode &= !HASH_MODE_NEW_HASH;
        let mut msg = data.to_vec();
        if self.hash_io & HASH_IO_PAD_DMA_MESSAGE != 0 {
            self.hash_io &= !HASH_IO_PAD_DMA_MESSAGE;
            msg.push(0x80);
            while msg.len() % 64 != 56 {
                msg.push(0);
            }
            msg.extend_from_slice(&self.hash_length.to_be_bytes());
        }
        let blocks: Vec<_> = msg
            .chunks_exact(64)
            .map(GenericArray::clone_from_slice)
            .collect();
        sha2::compress256(&mut state, &blocks);
        self.digest = state.map(u32::swap_bytes);
    }

    fn iv_bytes(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        for (i, w) in self.iv.iter().enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        b
    }

    fn set_iv_bytes(&mut self, b: &[u8; 16]) {
        for (i, w) in self.iv.iter_mut().enumerate() {
            *w = u32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
        }
    }

    fn set_tag(&mut self, b: &[u8; 16]) {
        for (i, w) in self.tag.iter_mut().enumerate() {
            *w = u32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
        }
    }

    /// Run the programmed AES operation over `data`; returns the output
    /// stream (empty for CBC-MAC) and updates the tag / IV registers.
    fn run_aes(&mut self, data: &[u8]) -> Vec<u8> {
        let Some(cipher) = Cipher::new(&self.key) else {
            self.int_stat |= INT_KEY_ST_RD_ERR;
            return vec![0; data.len()];
        };
        let ctrl = self.aes_ctrl;
        let encrypt = ctrl & AES_CTRL_DIRECTION_ENCRYPT != 0;
        let iv = self.iv_bytes();
        let len = (self.c_length as usize).min(data.len());
        let data = &data[..len];
        if ctrl & AES_CTRL_CCM != 0 {
            let aad = self.aad.take().unwrap_or_default();
            let (out, tag, next) = ccm(&cipher, ctrl, &iv, &aad, data, encrypt);
            self.set_tag(&tag);
            self.set_iv_bytes(&next);
            out
        } else if ctrl & AES_CTRL_GCM == AES_CTRL_GCM {
            let aad = self.aad.take().unwrap_or_default();
            let (out, tag, next) = gcm(&cipher, &iv, &aad, data, encrypt);
            self.set_tag(&tag);
            self.set_iv_bytes(&next);
            out
        } else if ctrl & AES_CTRL_CBC_MAC != 0 {
            let mut mac = iv;
            for block in data.chunks(16) {
                xor_into(&mut mac, block);
                cipher.encrypt(&mut mac);
            }
            self.set_tag(&mac);
            Vec::new()
        } else if ctrl & AES_CTRL_CTR != 0 {
            let width = ((ctrl >> AES_CTRL_CTR_WIDTH_SHIFT) & 0b11) as usize + 1;
            let mut ctr = iv;
            let out = ctr_stream(&cipher, &mut ctr, width * 4, data);
            self.set_iv_bytes(&ctr);
            out
        } else if ctrl & AES_CTRL_CBC != 0 {
            let mut chain = iv;
            let mut out = Vec::with_capacity(data.len());
            for block in data.chunks(16) {
                let mut b = pad_block(block);
                if encrypt {
                    xor_into(&mut b, &chain);
                    cipher.encrypt(&mut b);
                    chain = b;
                } else {
                    let c = b;
                    cipher.decrypt(&mut b);
                    xor_into(&mut b, &chain);
                    chain = c;
                }
                out.extend_from_slice(&b[..block.len()]);
            }
            self.set_iv_bytes(&chain);
            out
        } else {
            let mut out = Vec::with_capacity(data.len());
            for block in data.chunks(16) {
                let mut b = pad_block(block);
                if encrypt {
                    cipher.encrypt(&mut b);
                } else {
                    cipher.decrypt(&mut b);
                }
                out.extend_from_slice(&b[..block.len()]);
            }
            out
        }
    }
}

enum Cipher {
    A128(Box<Aes128>),
    A192(Box<Aes192>),
    A256(Box<Aes256>),
}

impl Cipher {
    fn new(key: &[u8]) -> Option<Self> {
        Some(match key.len() {
            16 => Self::A128(Box::new(Aes128::new_from_slice(key).ok()?)),
            24 => Self::A192(Box::new(Aes192::new_from_slice(key).ok()?)),
            32 => Self::A256(Box::new(Aes256::new_from_slice(key).ok()?)),
            _ => return None,
        })
    }

    fn encrypt(&self, b: &mut [u8; 16]) {
        let b = GenericArray::from_mut_slice(b);
        match self {
            Self::A128(c) => c.encrypt_block(b),
            Self::A192(c) => c.encrypt_block(b),
            Self::A256(c) => c.encrypt_block(b),
        }
    }

    fn decrypt(&self, b: &mut [u8; 16]) {
        let b = GenericArray::from_mut_slice(b);
        match self {
            Self::A128(c) => c.decrypt_block(b),
            Self::A192(c) => c.decrypt_block(b),
            Self::A256(c) => c.decrypt_block(b),
        }
    }
}

fn pad_block(block: &[u8]) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[..block.len()].copy_from_slice(block);
    b
}

fn xor_into(acc: &mut [u8; 16], other: &[u8]) {
    for (a, o) in acc.iter_mut().zip(other) {
        *a ^= o;
    }
}

/// Increment the low `width` bytes of `ctr` as a big-endian counter that
/// wraps within its width.
fn ctr_inc(ctr: &mut [u8; 16], width: usize) {
    for b in ctr[16 - width..].iter_mut().rev() {
        *b = b.wrapping_add(1);
        if *b != 0 {
            break;
        }
    }
}

/// CTR keystream over `data`, starting at `ctr` (incremented once per
/// block; left at the next unused counter).
fn ctr_stream(cipher: &Cipher, ctr: &mut [u8; 16], width: usize, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for block in data.chunks(16) {
        let mut ks = *ctr;
        cipher.encrypt(&mut ks);
        out.extend(block.iter().zip(ks).map(|(d, k)| d ^ k));
        ctr_inc(ctr, width);
    }
    out
}

/// CBC-MAC over `data` split into zero-padded blocks.
fn cbc_mac_blocks(cipher: &Cipher, mac: &mut [u8; 16], data: &[u8]) {
    for block in data.chunks(16) {
        xor_into(mac, block);
        cipher.encrypt(mac);
    }
}

/// AES-CCM (RFC 3610 / NIST SP 800-38C) from the EIP-120t context: `iv` is
/// the A0 counter block firmware wrote (flags = L-1, nonce, counter 0).
/// Returns (payload out, encrypted 128-bit tag, next counter block).
fn ccm(
    cipher: &Cipher,
    ctrl: u32,
    iv: &[u8; 16],
    aad: &[u8],
    data: &[u8],
    encrypt: bool,
) -> (Vec<u8>, [u8; 16], [u8; 16]) {
    let l = (((ctrl >> AES_CTRL_CCM_L_SHIFT) & 0b111) + 1) as usize;
    let m_field = ((ctrl >> AES_CTRL_CCM_M_SHIFT) & 0b111) as u8;

    // Payload: counters A1.. (A0 encrypts the tag).
    let mut ctr = *iv;
    ctr[16 - l..].fill(0);
    let a0 = ctr;
    ctr_inc(&mut ctr, l);
    let out = ctr_stream(cipher, &mut ctr, l, data);
    let plaintext = if encrypt { data } else { &out[..] };

    // B0 = flags | nonce | message length (L bytes).
    let mut b0 = a0;
    b0[0] = (if aad.is_empty() { 0 } else { 0x40 }) | (m_field << 3) | (l as u8 - 1);
    let len_bytes = (plaintext.len() as u64).to_be_bytes();
    b0[16 - l..].copy_from_slice(&len_bytes[8 - l.min(8)..]);
    let mut mac = [0u8; 16];
    cbc_mac_blocks(cipher, &mut mac, &b0);
    if !aad.is_empty() {
        let mut a = Vec::with_capacity(aad.len() + 6);
        if aad.len() < 0xFF00 {
            a.extend_from_slice(&(aad.len() as u16).to_be_bytes());
        } else {
            a.extend_from_slice(&[0xFF, 0xFE]);
            a.extend_from_slice(&(aad.len() as u32).to_be_bytes());
        }
        a.extend_from_slice(aad);
        cbc_mac_blocks(cipher, &mut mac, &a);
    }
    cbc_mac_blocks(cipher, &mut mac, plaintext);
    let mut s0 = a0;
    cipher.encrypt(&mut s0);
    xor_into(&mut mac, &s0);
    (out, mac, ctr)
}

/// GF(2^128) multiply in GCM bit order (NIST SP 800-38D §6.3).
fn gf128_mul(x: u128, y: u128) -> u128 {
    const R: u128 = 0xE1 << 120;
    let mut z = 0u128;
    let mut v = y;
    for i in 0..128 {
        if (x >> (127 - i)) & 1 != 0 {
            z ^= v;
        }
        v = if v & 1 != 0 { (v >> 1) ^ R } else { v >> 1 };
    }
    z
}

fn ghash_blocks(h: u128, y: &mut u128, data: &[u8]) {
    for block in data.chunks(16) {
        *y = gf128_mul(*y ^ u128::from_be_bytes(pad_block(block)), h);
    }
}

/// Autonomous AES-GCM (mode 11): `iv` is Y0 (96-bit IV, 32-bit counter as
/// written). Returns (payload out, tag, next counter block).
fn gcm(
    cipher: &Cipher,
    iv: &[u8; 16],
    aad: &[u8],
    data: &[u8],
    encrypt: bool,
) -> (Vec<u8>, [u8; 16], [u8; 16]) {
    let mut h = [0u8; 16];
    cipher.encrypt(&mut h);
    let h = u128::from_be_bytes(h);
    let mut ctr = *iv;
    ctr_inc(&mut ctr, 4);
    let out = ctr_stream(cipher, &mut ctr, 4, data);
    let ciphertext = if encrypt { &out[..] } else { data };
    let mut y = 0u128;
    ghash_blocks(h, &mut y, aad);
    ghash_blocks(h, &mut y, ciphertext);
    let lens = ((aad.len() as u128 * 8) << 64) | (ciphertext.len() as u128 * 8);
    y = gf128_mul(y ^ lens, h);
    let mut ek = *iv;
    cipher.encrypt(&mut ek);
    let tag = (y ^ u128::from_be_bytes(ek)).to_be_bytes();
    (out, tag, ctr)
}

impl Peripheral for Cc2538Aes {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(lane_read(self.read_reg(offset & !3), offset))
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let reg = offset & !3;
        // Peeking must not clear SAVED_CONTEXT_READY.
        let saved = self.saved_context_ready.get();
        let v = self.read_reg(reg);
        self.saved_context_ready.set(saved);
        Some(lane_read(v, offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) as u32 * 8;
        let saved = self.saved_context_ready.get();
        let old = self.read_reg(reg);
        self.saved_context_ready.set(saved);
        let merged = (old & !(0xFF << shift)) | (u32::from(value) << shift);
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

    /// DMA work happens in `tick_with_bus`. While the level line is high the
    /// bus tick keeps running so the bus re-pends the NVIC line after a
    /// software `NVIC_ClearPendingIRQ`, as a level-sensitive source would.
    fn needs_bus_tick(&self) -> bool {
        self.has_work() || self.irq_level()
    }

    fn tick_with_bus(&mut self, bus: &mut dyn Bus) {
        self.advance(bus);
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.irq_level())
    }

    /// No `tick()`: all work happens in `tick_with_bus`.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn on_node_reset(&mut self) {
        *self = Self::new();
    }

    snapshot_impl!(Cc2538Aes, "Cc2538Aes");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn engine_with_key(key: &[u8]) -> Cc2538Aes {
        let mut a = Cc2538Aes::new();
        a.ks_size = match key.len() {
            16 => 1,
            24 => 2,
            _ => 3,
        };
        a.key = key.to_vec();
        a.alg_sel = ALG_SEL_AES;
        a
    }

    fn iv_words(a: &mut Cc2538Aes, iv: &[u8]) {
        let mut b = [0u8; 16];
        b.copy_from_slice(iv);
        a.set_iv_bytes(&b);
    }

    fn tag_bytes(a: &Cc2538Aes) -> Vec<u8> {
        a.tag.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    #[test]
    fn registers_reset_to_table_22_103() {
        let a = Cc2538Aes::new();
        assert_eq!(a.read_reg(DMAC_MST_RUNPARAMS), 0x2400);
        assert_eq!(a.read_reg(DMAC_OPTIONS), 0x0202);
        assert_eq!(a.read_reg(DMAC_VERSION), 0x0101_2ED1);
        assert_eq!(a.read_reg(KEY_STORE_SIZE), 1);
        assert_eq!(a.read_reg(KEY_STORE_READ_AREA), 8);
        assert_eq!(a.read_reg(AES_CTRL), 0x8000_0000);
        assert_eq!(a.read_reg(HASH_IO_BUF_CTRL), 0x4);
        assert_eq!(a.read_reg(CTRL_OPTIONS), 0x0101_01F7);
        assert_eq!(a.read_reg(CTRL_VERSION), 0x9110_8778);
        assert_eq!(a.read_reg(CTRL_INT_STAT), 0);
    }

    #[test]
    fn key_store_fills_contiguous_free_areas_and_reports_errors() {
        let mut a = Cc2538Aes::new();
        a.alg_sel = ALG_SEL_KEYSTORE;
        a.write_reg(KEY_STORE_WRITE_AREA, 0b0110);
        a.store_keys(&[0x11; 32]);
        assert_eq!(a.ks_written, 0b0110);
        assert_eq!(a.int_stat & INT_KEY_ST_WR_ERR, 0);
        // Occupied areas cannot be rewritten before they are freed.
        a.store_keys(&[0x22; 32]);
        assert_ne!(a.int_stat & INT_KEY_ST_WR_ERR, 0);
        a.write_reg(CTRL_INT_CLR, INT_STAT_MASK);
        a.write_reg(KEY_STORE_WRITTEN_AREA, 0b0010);
        assert_eq!(a.ks_written, 0b0100);
        // A 256-bit key must start on an even area.
        a.write_reg(KEY_STORE_SIZE, 3);
        assert_eq!(a.ks_written, 0, "writing the size deletes all keys");
        a.write_reg(KEY_STORE_WRITE_AREA, 0b0110);
        a.store_keys(&[0x33; 32]);
        assert_ne!(a.int_stat & INT_KEY_ST_WR_ERR, 0);
    }

    #[test]
    fn read_area_loads_written_keys_only() {
        let mut a = Cc2538Aes::new();
        a.write_reg(KEY_STORE_WRITE_AREA, 0b1);
        a.store_keys(&hex("000102030405060708090a0b0c0d0e0f"));
        a.write_reg(KEY_STORE_READ_AREA, 0);
        assert_eq!(a.key, hex("000102030405060708090a0b0c0d0e0f"));
        assert_eq!(a.read_reg(AES_CTRL) & AES_CTRL_KEY_SIZE_MASK, 1 << 3);
        a.write_reg(KEY_STORE_READ_AREA, 1);
        assert_ne!(a.int_stat & INT_KEY_ST_RD_ERR, 0);
    }

    /// FIPS-197 Appendix C.1-C.3.
    #[test]
    fn ecb_matches_fips_197_for_all_key_sizes() {
        let pt = hex("00112233445566778899aabbccddeeff");
        for (key, ct) in [
            (
                "000102030405060708090a0b0c0d0e0f",
                "69c4e0d86a7b0430d8cdb78070b4c55a",
            ),
            (
                "000102030405060708090a0b0c0d0e0f1011121314151617",
                "dda97ca4864cdfe06eaf70a0ec0d7191",
            ),
            (
                "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
                "8ea2b7ca516745bfeafc49904b496089",
            ),
        ] {
            let mut a = engine_with_key(&hex(key));
            a.aes_ctrl = AES_CTRL_DIRECTION_ENCRYPT;
            a.c_length = 16;
            assert_eq!(a.run_aes(&pt), hex(ct));
            a.aes_ctrl = 0;
            assert_eq!(a.run_aes(&hex(ct)), pt);
        }
    }

    /// NIST SP 800-38A F.2.1 (CBC-AES128.Encrypt), first two blocks.
    #[test]
    fn cbc_matches_sp800_38a() {
        let mut a = engine_with_key(&hex("2b7e151628aed2a6abf7158809cf4f3c"));
        iv_words(&mut a, &hex("000102030405060708090a0b0c0d0e0f"));
        a.aes_ctrl = AES_CTRL_CBC | AES_CTRL_DIRECTION_ENCRYPT;
        let pt = hex("6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51");
        a.c_length = 32;
        let ct = a.run_aes(&pt);
        assert_eq!(
            ct,
            hex("7649abac8119b246cee98e9b12e9197d5086cb9b507219ee95db113a917678b2")
        );
        iv_words(&mut a, &hex("000102030405060708090a0b0c0d0e0f"));
        a.aes_ctrl = AES_CTRL_CBC;
        assert_eq!(a.run_aes(&ct), pt);
    }

    /// NIST SP 800-38A F.5.1 (CTR-AES128.Encrypt), first two blocks.
    #[test]
    fn ctr_matches_sp800_38a_and_leaves_the_next_counter() {
        let mut a = engine_with_key(&hex("2b7e151628aed2a6abf7158809cf4f3c"));
        iv_words(&mut a, &hex("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff"));
        a.aes_ctrl = AES_CTRL_CTR | (3 << AES_CTRL_CTR_WIDTH_SHIFT) | AES_CTRL_DIRECTION_ENCRYPT;
        a.c_length = 32;
        let ct = a.run_aes(&hex(
            "6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51",
        ));
        assert_eq!(
            ct,
            hex("874d6191b620e3261bef6864990db6ce9806f66b7970fdff8617187bb9fffdff")
        );
        assert_eq!(
            a.iv_bytes().to_vec(),
            hex("f0f1f2f3f4f5f6f7f8f9fafbfcfdff01")
        );
    }

    #[test]
    fn ctr_wraps_within_its_width() {
        let mut ctr = [0xFF; 16];
        ctr_inc(&mut ctr, 4);
        assert_eq!(&ctr[..12], &[0xFF; 12]);
        assert_eq!(&ctr[12..], &[0; 4]);
    }

    /// RFC 3610 Packet Vector #1 (M = 8, L = 2, 8 bytes AAD, 23 bytes data).
    #[test]
    fn ccm_matches_rfc_3610_vector_1_both_directions() {
        let key = hex("c0c1c2c3c4c5c6c7c8c9cacbcccdcecf");
        let nonce = hex("00000003020100a0a1a2a3a4a5");
        let aad = hex("0001020304050607");
        let pt = hex("08090a0b0c0d0e0f101112131415161718191a1b1c1d1e");
        let ct = hex("588c979a61c663d2f066d0c2c0f989806d5f6b61dac384");
        let mic = hex("17e8d12cfdf926e0");
        let mut iv = vec![1u8]; // L - 1
        iv.extend_from_slice(&nonce);
        iv.extend_from_slice(&[0, 0]);
        let ctrl_base = AES_CTRL_SAVE_CONTEXT
            | (3 << AES_CTRL_CCM_M_SHIFT)
            | (1 << AES_CTRL_CCM_L_SHIFT)
            | AES_CTRL_CCM
            | (3 << AES_CTRL_CTR_WIDTH_SHIFT)
            | AES_CTRL_CTR;
        for encrypt in [true, false] {
            let mut a = engine_with_key(&key);
            iv_words(&mut a, &iv);
            a.aes_ctrl = ctrl_base
                | if encrypt {
                    AES_CTRL_DIRECTION_ENCRYPT
                } else {
                    0
                };
            a.c_length = 23;
            a.aad = Some(aad.clone());
            let out = a.run_aes(if encrypt { &pt } else { &ct });
            assert_eq!(out, if encrypt { ct.clone() } else { pt.clone() });
            assert_eq!(tag_bytes(&a)[..8], mic[..]);
        }
    }

    /// NIST GCM test case 4 (McGrew & Viega): 60-byte plaintext, 20-byte AAD.
    #[test]
    fn gcm_matches_the_gcm_spec_test_case_4() {
        let mut a = engine_with_key(&hex("feffe9928665731c6d6a8f9467308308"));
        let mut iv = hex("cafebabefacedbaddecaf888");
        iv.extend_from_slice(&[0, 0, 0, 1]);
        iv_words(&mut a, &iv);
        a.aes_ctrl =
            AES_CTRL_SAVE_CONTEXT | AES_CTRL_GCM | AES_CTRL_CTR | AES_CTRL_DIRECTION_ENCRYPT;
        a.aad = Some(hex("feedfacedeadbeeffeedfacedeadbeefabaddad2"));
        let pt = hex(concat!(
            "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a72",
            "1c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39"
        ));
        a.c_length = pt.len() as u64;
        let ct = a.run_aes(&pt);
        assert_eq!(
            ct,
            hex(concat!(
                "42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e",
                "21d514b25466931c7d8f6a5aac84aa051ba30b396a0aac973d58e091"
            ))
        );
        assert_eq!(tag_bytes(&a), hex("5bc94fbc3221a5db94fae95ae7121a47"));
    }

    #[test]
    fn cbc_mac_tag_is_the_last_cbc_block() {
        let mut a = engine_with_key(&hex("2b7e151628aed2a6abf7158809cf4f3c"));
        a.aes_ctrl = AES_CTRL_SAVE_CONTEXT | AES_CTRL_CBC_MAC | AES_CTRL_DIRECTION_ENCRYPT;
        let pt = hex("6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51");
        a.c_length = 32;
        assert!(a.run_aes(&pt).is_empty());
        // CBC with a zero IV: the tag is the second ciphertext block.
        let mut b = engine_with_key(&hex("2b7e151628aed2a6abf7158809cf4f3c"));
        b.aes_ctrl = AES_CTRL_CBC | AES_CTRL_DIRECTION_ENCRYPT;
        b.c_length = 32;
        let ct = b.run_aes(&pt);
        assert_eq!(tag_bytes(&a), ct[16..]);
    }

    /// FIPS 180-4 "abc" through the DMA padding path, then a resumed
    /// two-block message whose first block went through as an intermediate.
    #[test]
    fn sha256_pads_and_resumes_from_the_digest_registers() {
        let mut a = Cc2538Aes::new();
        a.alg_sel = ALG_SEL_TAG | ALG_SEL_HASH;
        a.write_reg(HASH_MODE_IN, HASH_MODE_NEW_HASH | 0x8);
        a.write_reg(HASH_LENGTH_IN_L, 24);
        a.write_reg(HASH_IO_BUF_CTRL, HASH_IO_PAD_DMA_MESSAGE);
        a.hash(b"abc");
        let digest: Vec<u8> = a.digest.iter().flat_map(|w| w.to_le_bytes()).collect();
        assert_eq!(
            digest,
            hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        assert_eq!(a.read_reg(HASH_IO_BUF_CTRL) & HASH_IO_PAD_DMA_MESSAGE, 0);

        let msg = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        let mut long = msg.to_vec();
        long.extend_from_slice(msg);
        let mut b = Cc2538Aes::new();
        b.write_reg(HASH_MODE_IN, HASH_MODE_NEW_HASH | 0x8);
        b.hash(&long[..64]);
        let mid = b.digest;
        let mut c = Cc2538Aes::new();
        for (i, w) in mid.iter().enumerate() {
            c.write_reg(HASH_DIGEST_A + 4 * i as u64, *w);
        }
        c.write_reg(HASH_MODE_IN, 0x8);
        c.write_reg(HASH_LENGTH_IN_L, (long.len() * 8) as u32);
        c.write_reg(HASH_IO_BUF_CTRL, HASH_IO_PAD_DMA_MESSAGE);
        c.hash(&long[64..]);
        let want: Vec<u8> = {
            use sha2::Digest;
            sha2::Sha256::digest(&long).to_vec()
        };
        let got: Vec<u8> = c.digest.iter().flat_map(|w| w.to_le_bytes()).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn saved_context_clears_on_the_fourth_tag_word_but_not_on_peek() {
        let mut a = engine_with_key(&[0; 16]);
        a.aes_ctrl = AES_CTRL_SAVE_CONTEXT | AES_CTRL_CBC_MAC | AES_CTRL_DIRECTION_ENCRYPT;
        a.finish();
        assert_ne!(a.read_reg(AES_CTRL) & AES_CTRL_SAVED_CONTEXT_READY, 0);
        assert_eq!(a.read_reg(AES_CTRL) & AES_CTRL_CONTEXT_READY, 0);
        let _ = a.peek(AES_TAG_OUT_3);
        assert_ne!(a.read_reg(AES_CTRL) & AES_CTRL_SAVED_CONTEXT_READY, 0);
        let _ = a.read_u32(AES_TAG_OUT_3);
        assert_eq!(a.read_reg(AES_CTRL) & AES_CTRL_SAVED_CONTEXT_READY, 0);
        assert_ne!(a.read_reg(AES_CTRL) & AES_CTRL_CONTEXT_READY, 0);
    }

    #[test]
    fn interrupt_line_is_stat_and_enable_and_clears_with_int_clr() {
        let mut a = Cc2538Aes::new();
        a.write_reg(CTRL_INT_SET, INT_RESULT_AV);
        assert_eq!(a.irq_line_level(), Some(false));
        a.write_reg(CTRL_INT_EN, INT_RESULT_AV);
        assert_eq!(a.irq_line_level(), Some(true));
        assert!(a.needs_bus_tick(), "a held level keeps the bus reconciling");
        a.write_reg(CTRL_INT_CLR, INT_RESULT_AV);
        assert_eq!(a.irq_line_level(), Some(false));
        assert!(!a.needs_bus_tick());
    }
}
