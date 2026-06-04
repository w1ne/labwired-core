// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! AES accelerator for ESP32-S3.
//!
//! Functionally-exact digital twin of the ESP32-S3 AES peripheral, focused on
//! the **typical** (CPU, single-block ECB) path that mbedTLS / `esp_aes_*`
//! drive via `aes_ll_*` (see
//! `framework-espidf/components/hal/esp32s3/include/hal/aes_ll.h`). The DMA
//! "block" path is modelled only to the extent of round-tripping its config
//! registers + delivering the DMA-completion interrupt; the actual cipher work
//! in this model always runs through the typical single-block engine.
//!
//! ## Register map (ESP32-S3 TRM §24; verified against ESP-IDF
//! `components/soc/esp32s3/include/soc/hwcrypto_reg.h`, base `DR_REG_AES_BASE
//! = 0x6003_A000`)
//!
//! | Offset | Name            | Behaviour |
//! |-------:|-----------------|-----------|
//! | 0x00.. | AES_KEY_0..7    | 8×32-bit key words (up to AES-256) |
//! | 0x20.. | AES_TEXT_IN_0..3| 128-bit input block (4 words) |
//! | 0x30.. | AES_TEXT_OUT_0..3| 128-bit output block (4 words, RO) |
//! | 0x40   | AES_MODE        | bits[2:0]: 0=enc128 1=enc192 2=enc256, 4=dec128 5=dec192 6=dec256 |
//! | 0x48   | AES_TRIGGER     | write 1 → start typical single-block transform |
//! | 0x4C   | AES_STATE       | RO: 0=idle 1=busy 2=done (typical mode) |
//! | 0x50.. | AES_IV_0..3     | DMA-mode IV (round-tripped only) |
//! | 0x90   | AES_DMA_ENABLE  | 1 = DMA (block) mode, 0 = typical (CPU) mode |
//! | 0x94   | AES_BLOCK_MODE  | DMA block cipher mode (ECB/CBC/...) |
//! | 0x98   | AES_BLOCK_NUM   | DMA number of blocks |
//! | 0x9C   | AES_INC_SEL     | AES-CTR counter increment select |
//! | 0xAC   | AES_INT_CLEAR   | W1C of the DMA-done interrupt (bit 0) |
//! | 0xB0   | AES_INT_ENA     | enable DMA-done interrupt (bit 0) |
//! | 0xB4   | AES_DATE        | version/date constant (RO) |
//! | 0xB8   | AES_DMA_EXIT    | release DMA |
//!
//! Note: the ESP32-S3 AES `hwcrypto_reg.h` exposes only `AES_INT_CLEAR_REG`
//! (0xAC) and `AES_INT_ENA_REG` (0xB0) — there is no separate INT_RAW / INT_ST
//! offset in the public register map (unlike e.g. SYSTIMER). We therefore model
//! the raw pending latch internally: the DMA-done event sets it, `AES_INT_ENA`
//! gates IRQ delivery, and `AES_INT_CLEAR` (W1C) clears it. The aggregated
//! "INT_ST" used to gate `explicit_irqs` emission is `int_raw & int_ena`.
//!
//! ## Endianness convention (verified against `aes_ll.h`)
//!
//! `aes_ll_write_key` / `aes_ll_write_block` do `memcpy(&word, bytes+4*i, 4)`
//! then `REG_WRITE`. On little-endian Xtensa-LX7 this means register word `N`
//! holds AES bytes `[4N..4N+4]` with byte `4N` in the **least-significant**
//! byte of the word. Equivalently each 32-bit register word is the little-
//! endian packing of four consecutive cipher bytes. So:
//!
//! * KEY byte order:  `KEY_i = key[4i] | key[4i+1]<<8 | key[4i+2]<<16 | key[4i+3]<<24`
//! * TEXT_IN  likewise forms the 16-byte plaintext block in word order.
//! * TEXT_OUT is the same little-endian packing of the 16 ciphertext bytes.
//!
//! Our model stores registers as native `u32` words and converts to/from the
//! 16-byte AES block with `to_le_bytes` / `from_le_bytes`, which is exactly the
//! hardware packing above.
//!
//! ## Source ID (ESP32-S3 TRM §9.4; verified against ESP-IDF
//! `components/soc/esp32s3/include/soc/interrupts.h`)
//!
//! `ETS_AES_INTR_SOURCE = 77` (enum from `ETS_WIFI_MAC_INTR_SOURCE = 0`;
//! anchors: `ETS_RSA = 76`, `ETS_AES = 77`, `ETS_SHA = 78`). The DMA-done
//! interrupt is emitted via `PeripheralTickResult.explicit_irqs` while
//! `int_raw & int_ena != 0`, level-sensitive, matching the SYSTIMER twin.

use crate::{Peripheral, PeripheralTickResult, SimResult};

// ── Register offsets (relative to DR_REG_AES_BASE) ──
const KEY_BASE: u64 = 0x00; // KEY_0..7  (0x00..0x1C)
const TEXT_IN_BASE: u64 = 0x20; // TEXT_IN_0..3 (0x20..0x2C)
const TEXT_OUT_BASE: u64 = 0x30; // TEXT_OUT_0..3 (0x30..0x3C)
const MODE_REG: u64 = 0x40;
const TRIGGER_REG: u64 = 0x48;
const STATE_REG: u64 = 0x4C;
const IV_BASE: u64 = 0x50; // IV_0..3 (0x50..0x5C)
const DMA_ENABLE_REG: u64 = 0x90;
const BLOCK_MODE_REG: u64 = 0x94;
const BLOCK_NUM_REG: u64 = 0x98;
const INC_SEL_REG: u64 = 0x9C;
const INT_CLEAR_REG: u64 = 0xAC;
const INT_ENA_REG: u64 = 0xB0;
const DATE_REG: u64 = 0xB4;
const DMA_EXIT_REG: u64 = 0xB8;

/// AES_DATE value reported by ESP32-S3 silicon (`AES_DATE` reset value).
const AES_DATE_VALUE: u32 = 0x2020_0306;

/// AES_STATE values (mirrors `esp_aes_state_t` in `aes_ll.h`).
const STATE_IDLE: u32 = 0;
#[allow(dead_code)]
const STATE_BUSY: u32 = 1;
const STATE_DONE: u32 = 2;

/// MODE bit [2] selects decryption (`MODE_DECRYPT_BIT` in `aes_ll.h`).
const MODE_DECRYPT_BIT: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyLen {
    Aes128,
    Aes192,
    Aes256,
}

impl KeyLen {
    /// Number of 32-bit key words consumed from KEY_0..7.
    fn key_words(self) -> usize {
        match self {
            KeyLen::Aes128 => 4,
            KeyLen::Aes192 => 6,
            KeyLen::Aes256 => 8,
        }
    }
    /// Number of AES rounds (Nr).
    fn rounds(self) -> usize {
        match self {
            KeyLen::Aes128 => 10,
            KeyLen::Aes192 => 12,
            KeyLen::Aes256 => 14,
        }
    }
}

/// Decode `AES_MODE` bits[2:0] into (decrypt, key length).
///
/// 0=enc128 1=enc192 2=enc256 4=dec128 5=dec192 6=dec256 (TRM §24.5 /
/// `aes_ll_set_mode`). Returns `None` for the reserved encodings (3, 7).
fn decode_mode(mode: u32) -> Option<(bool, KeyLen)> {
    let decrypt = mode & MODE_DECRYPT_BIT != 0;
    let len = match mode & 0x3 {
        0 => KeyLen::Aes128,
        1 => KeyLen::Aes192,
        2 => KeyLen::Aes256,
        _ => return None,
    };
    Some((decrypt, len))
}

#[derive(Debug)]
pub struct Esp32s3Aes {
    /// Interrupt-matrix source id (ETS_AES_INTR_SOURCE = 77).
    source_id: u32,

    // ── Register banks (native u32 words; LE packing == HW byte order) ──
    key: [u32; 8],
    text_in: [u32; 4],
    text_out: [u32; 4],
    iv: [u32; 4],
    mode: u32,

    // ── Typical-path engine state ──
    /// AES_STATE: IDLE until first TRIGGER, then DONE after each transform.
    state: u32,

    // ── DMA / block-mode config (round-tripped) ──
    dma_enable: u32,
    block_mode: u32,
    block_num: u32,
    inc_sel: u32,

    // ── Interrupt state (DMA-done) ──
    /// Raw pending latch (modelled internally; not a public register).
    int_raw: bool,
    /// AES_INT_ENA bit 0.
    int_ena: bool,
}

impl Esp32s3Aes {
    /// `source_id` is the interrupt-matrix source the AES peripheral binds to;
    /// pass `ETS_AES_INTR_SOURCE` (= 77 on ESP32-S3).
    pub fn new(source_id: u32) -> Self {
        Self {
            source_id,
            key: [0; 8],
            text_in: [0; 4],
            text_out: [0; 4],
            iv: [0; 4],
            mode: 0,
            state: STATE_IDLE,
            dma_enable: 0,
            block_mode: 0,
            block_num: 0,
            inc_sel: 0,
            int_raw: false,
            int_ena: false,
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            o if (KEY_BASE..KEY_BASE + 0x20).contains(&o) => {
                self.key[((o - KEY_BASE) / 4) as usize]
            }
            o if (TEXT_IN_BASE..TEXT_IN_BASE + 0x10).contains(&o) => {
                self.text_in[((o - TEXT_IN_BASE) / 4) as usize]
            }
            o if (TEXT_OUT_BASE..TEXT_OUT_BASE + 0x10).contains(&o) => {
                self.text_out[((o - TEXT_OUT_BASE) / 4) as usize]
            }
            MODE_REG => self.mode,
            // TRIGGER is write-only; reads as 0.
            STATE_REG => self.state,
            o if (IV_BASE..IV_BASE + 0x10).contains(&o) => self.iv[((o - IV_BASE) / 4) as usize],
            DMA_ENABLE_REG => self.dma_enable,
            BLOCK_MODE_REG => self.block_mode,
            BLOCK_NUM_REG => self.block_num,
            INC_SEL_REG => self.inc_sel,
            // INT_CLEAR is W1C; reads as 0.
            INT_ENA_REG => self.int_ena as u32,
            DATE_REG => AES_DATE_VALUE,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        match offset {
            o if (KEY_BASE..KEY_BASE + 0x20).contains(&o) => {
                self.key[((o - KEY_BASE) / 4) as usize] = value;
            }
            o if (TEXT_IN_BASE..TEXT_IN_BASE + 0x10).contains(&o) => {
                self.text_in[((o - TEXT_IN_BASE) / 4) as usize] = value;
            }
            // TEXT_OUT is read-only on silicon; ignore writes.
            MODE_REG => self.mode = value & 0x7,
            TRIGGER_REG
                if value & 1 != 0 => {
                    self.trigger();
                }
            // STATE is read-only.
            o if (IV_BASE..IV_BASE + 0x10).contains(&o) => {
                self.iv[((o - IV_BASE) / 4) as usize] = value;
            }
            DMA_ENABLE_REG => self.dma_enable = value & 1,
            BLOCK_MODE_REG => self.block_mode = value,
            BLOCK_NUM_REG => self.block_num = value,
            INC_SEL_REG => self.inc_sel = value,
            INT_CLEAR_REG
                // W1C of the DMA-done interrupt (bit 0).
                if value & 1 != 0 => {
                    self.int_raw = false;
                }
            INT_ENA_REG => self.int_ena = value & 1 != 0,
            DMA_EXIT_REG => { /* release DMA: no persistent state to clear */ }
            _ => {}
        }
    }

    /// Execute one transform on TRIGGER write (typical single-block path).
    ///
    /// DMA mode: the real engine would pull blocks over GDMA. We don't model
    /// the DMA fabric here, but we still run the single TEXT_IN/TEXT_OUT block
    /// through the cipher and raise the DMA-done interrupt so firmware polling
    /// the completion path makes progress.
    fn trigger(&mut self) {
        let Some((decrypt, len)) = decode_mode(self.mode) else {
            // Reserved MODE encoding: no transform, leave outputs as-is.
            return;
        };

        // Assemble the 16-byte plaintext/ciphertext block from TEXT_IN words.
        // Each word is the little-endian packing of 4 consecutive bytes.
        let mut block = [0u8; 16];
        for (i, w) in self.text_in.iter().enumerate() {
            block[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }

        // Assemble the key bytes (4/6/8 words → 16/24/32 bytes).
        let kw = len.key_words();
        let mut key_bytes = [0u8; 32];
        for i in 0..kw {
            key_bytes[i * 4..i * 4 + 4].copy_from_slice(&self.key[i].to_le_bytes());
        }

        let out = if decrypt {
            aes_decrypt_block(&key_bytes[..kw * 4], &block, len.rounds())
        } else {
            aes_encrypt_block(&key_bytes[..kw * 4], &block, len.rounds())
        };

        // Write result to TEXT_OUT words (same LE packing).
        for i in 0..4 {
            self.text_out[i] =
                u32::from_le_bytes([out[i * 4], out[i * 4 + 1], out[i * 4 + 2], out[i * 4 + 3]]);
        }

        // Typical mode: STATE goes DONE (we model the transform as
        // instantaneous, so we never expose the transient BUSY state — the
        // firmware busy-wait `while (state != DONE)` exits on the first read).
        self.state = STATE_DONE;

        // DMA mode raises the transform-completed interrupt.
        if self.dma_enable & 1 != 0 {
            self.int_raw = true;
        }
    }
}

impl Peripheral for Esp32s3Aes {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        // For TRIGGER/INT_CLEAR (action-on-write) we must not lose the intent
        // of a byte-granular write. The bus also calls `write_word_32` for the
        // coherent 32-bit view, but firmware uses `REG_WRITE` (a full word
        // store), so the common path is a 4-byte sequence ending here. To keep
        // byte writes coherent we read-modify-write the word, then re-dispatch.
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    fn write_word_32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        // The bus delivers a coherent 32-bit view after the four byte writes.
        // Byte writes above already dispatched (and, for TRIGGER, may have run
        // the transform on the byte carrying bit 0). Re-dispatching the full
        // word here is idempotent for data registers and harmless for TRIGGER
        // (a second `value&1` start recomputes the same block); the canonical
        // single-word `REG_WRITE` path lands here cleanly.
        self.write_word(offset & !3, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Level-sensitive DMA-done IRQ: while int_raw & int_ena, re-emit the
        // AES interrupt-matrix source each tick until firmware ACKs via
        // AES_INT_CLEAR. Mirrors the SYSTIMER twin's emit-while-INT_ST!=0.
        if self.int_raw && self.int_ena {
            PeripheralTickResult {
                explicit_irqs: Some(vec![self.source_id]),
                ..PeripheralTickResult::default()
            }
        } else {
            PeripheralTickResult::default()
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

// ─────────────────────────────────────────────────────────────────────────
// FIPS-197 AES core (real cipher).
//
// Self-contained AES-128/192/256 ECB single-block encrypt/decrypt. This is the
// genuine Rijndael algorithm (S-box, key schedule, MixColumns over GF(2^8)),
// not a lookup of precomputed vectors. Validated against the FIPS-197 Appendix
// B/C known-answer vectors in the test module below. (We implement the cipher
// in-crate because the workspace does not actually carry the RustCrypto `aes`
// dependency; `crates/core/Cargo.toml` lists no `aes`/`cipher` crate, and the
// task constraints forbid editing Cargo.toml.)
// ─────────────────────────────────────────────────────────────────────────

#[rustfmt::skip]
const SBOX: [u8; 256] = [
    0x63,0x7c,0x77,0x7b,0xf2,0x6b,0x6f,0xc5,0x30,0x01,0x67,0x2b,0xfe,0xd7,0xab,0x76,
    0xca,0x82,0xc9,0x7d,0xfa,0x59,0x47,0xf0,0xad,0xd4,0xa2,0xaf,0x9c,0xa4,0x72,0xc0,
    0xb7,0xfd,0x93,0x26,0x36,0x3f,0xf7,0xcc,0x34,0xa5,0xe5,0xf1,0x71,0xd8,0x31,0x15,
    0x04,0xc7,0x23,0xc3,0x18,0x96,0x05,0x9a,0x07,0x12,0x80,0xe2,0xeb,0x27,0xb2,0x75,
    0x09,0x83,0x2c,0x1a,0x1b,0x6e,0x5a,0xa0,0x52,0x3b,0xd6,0xb3,0x29,0xe3,0x2f,0x84,
    0x53,0xd1,0x00,0xed,0x20,0xfc,0xb1,0x5b,0x6a,0xcb,0xbe,0x39,0x4a,0x4c,0x58,0xcf,
    0xd0,0xef,0xaa,0xfb,0x43,0x4d,0x33,0x85,0x45,0xf9,0x02,0x7f,0x50,0x3c,0x9f,0xa8,
    0x51,0xa3,0x40,0x8f,0x92,0x9d,0x38,0xf5,0xbc,0xb6,0xda,0x21,0x10,0xff,0xf3,0xd2,
    0xcd,0x0c,0x13,0xec,0x5f,0x97,0x44,0x17,0xc4,0xa7,0x7e,0x3d,0x64,0x5d,0x19,0x73,
    0x60,0x81,0x4f,0xdc,0x22,0x2a,0x90,0x88,0x46,0xee,0xb8,0x14,0xde,0x5e,0x0b,0xdb,
    0xe0,0x32,0x3a,0x0a,0x49,0x06,0x24,0x5c,0xc2,0xd3,0xac,0x62,0x91,0x95,0xe4,0x79,
    0xe7,0xc8,0x37,0x6d,0x8d,0xd5,0x4e,0xa9,0x6c,0x56,0xf4,0xea,0x65,0x7a,0xae,0x08,
    0xba,0x78,0x25,0x2e,0x1c,0xa6,0xb4,0xc6,0xe8,0xdd,0x74,0x1f,0x4b,0xbd,0x8b,0x8a,
    0x70,0x3e,0xb5,0x66,0x48,0x03,0xf6,0x0e,0x61,0x35,0x57,0xb9,0x86,0xc1,0x1d,0x9e,
    0xe1,0xf8,0x98,0x11,0x69,0xd9,0x8e,0x94,0x9b,0x1e,0x87,0xe9,0xce,0x55,0x28,0xdf,
    0x8c,0xa1,0x89,0x0d,0xbf,0xe6,0x42,0x68,0x41,0x99,0x2d,0x0f,0xb0,0x54,0xbb,0x16,
];

#[rustfmt::skip]
const INV_SBOX: [u8; 256] = [
    0x52,0x09,0x6a,0xd5,0x30,0x36,0xa5,0x38,0xbf,0x40,0xa3,0x9e,0x81,0xf3,0xd7,0xfb,
    0x7c,0xe3,0x39,0x82,0x9b,0x2f,0xff,0x87,0x34,0x8e,0x43,0x44,0xc4,0xde,0xe9,0xcb,
    0x54,0x7b,0x94,0x32,0xa6,0xc2,0x23,0x3d,0xee,0x4c,0x95,0x0b,0x42,0xfa,0xc3,0x4e,
    0x08,0x2e,0xa1,0x66,0x28,0xd9,0x24,0xb2,0x76,0x5b,0xa2,0x49,0x6d,0x8b,0xd1,0x25,
    0x72,0xf8,0xf6,0x64,0x86,0x68,0x98,0x16,0xd4,0xa4,0x5c,0xcc,0x5d,0x65,0xb6,0x92,
    0x6c,0x70,0x48,0x50,0xfd,0xed,0xb9,0xda,0x5e,0x15,0x46,0x57,0xa7,0x8d,0x9d,0x84,
    0x90,0xd8,0xab,0x00,0x8c,0xbc,0xd3,0x0a,0xf7,0xe4,0x58,0x05,0xb8,0xb3,0x45,0x06,
    0xd0,0x2c,0x1e,0x8f,0xca,0x3f,0x0f,0x02,0xc1,0xaf,0xbd,0x03,0x01,0x13,0x8a,0x6b,
    0x3a,0x91,0x11,0x41,0x4f,0x67,0xdc,0xea,0x97,0xf2,0xcf,0xce,0xf0,0xb4,0xe6,0x73,
    0x96,0xac,0x74,0x22,0xe7,0xad,0x35,0x85,0xe2,0xf9,0x37,0xe8,0x1c,0x75,0xdf,0x6e,
    0x47,0xf1,0x1a,0x71,0x1d,0x29,0xc5,0x89,0x6f,0xb7,0x62,0x0e,0xaa,0x18,0xbe,0x1b,
    0xfc,0x56,0x3e,0x4b,0xc6,0xd2,0x79,0x20,0x9a,0xdb,0xc0,0xfe,0x78,0xcd,0x5a,0xf4,
    0x1f,0xdd,0xa8,0x33,0x88,0x07,0xc7,0x31,0xb1,0x12,0x10,0x59,0x27,0x80,0xec,0x5f,
    0x60,0x51,0x7f,0xa9,0x19,0xb5,0x4a,0x0d,0x2d,0xe5,0x7a,0x9f,0x93,0xc9,0x9c,0xef,
    0xa0,0xe0,0x3b,0x4d,0xae,0x2a,0xf5,0xb0,0xc8,0xeb,0xbb,0x3c,0x83,0x53,0x99,0x61,
    0x17,0x2b,0x04,0x7e,0xba,0x77,0xd6,0x26,0xe1,0x69,0x14,0x63,0x55,0x21,0x0c,0x7d,
];

const RCON: [u8; 11] = [
    0x00, 0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36,
];

/// GF(2^8) multiply (Rijndael field, modulo x^8 + x^4 + x^3 + x + 1).
fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    for _ in 0..8 {
        if b & 1 != 0 {
            p ^= a;
        }
        let hi = a & 0x80;
        a <<= 1;
        if hi != 0 {
            a ^= 0x1b;
        }
        b >>= 1;
    }
    p
}

/// Expand `key` (16/24/32 bytes) into `4*(nr+1)` round-key words.
fn key_expansion(key: &[u8], nr: usize) -> Vec<[u8; 4]> {
    let nk = key.len() / 4; // 4, 6, or 8
    let total = 4 * (nr + 1);
    let mut w: Vec<[u8; 4]> = Vec::with_capacity(total);
    for i in 0..nk {
        w.push([key[4 * i], key[4 * i + 1], key[4 * i + 2], key[4 * i + 3]]);
    }
    for i in nk..total {
        let mut temp = w[i - 1];
        if i % nk == 0 {
            // RotWord + SubWord + Rcon.
            temp = [temp[1], temp[2], temp[3], temp[0]];
            for b in temp.iter_mut() {
                *b = SBOX[*b as usize];
            }
            temp[0] ^= RCON[i / nk];
        } else if nk > 6 && i % nk == 4 {
            // AES-256 extra SubWord.
            for b in temp.iter_mut() {
                *b = SBOX[*b as usize];
            }
        }
        let prev = w[i - nk];
        w.push([
            prev[0] ^ temp[0],
            prev[1] ^ temp[1],
            prev[2] ^ temp[2],
            prev[3] ^ temp[3],
        ]);
    }
    w
}

fn add_round_key(state: &mut [u8; 16], w: &[[u8; 4]], round: usize) {
    // State is column-major: byte index = col*4 + row. Round key word `c`
    // is the c-th column.
    for c in 0..4 {
        for r in 0..4 {
            state[c * 4 + r] ^= w[round * 4 + c][r];
        }
    }
}

fn sub_bytes(state: &mut [u8; 16]) {
    for b in state.iter_mut() {
        *b = SBOX[*b as usize];
    }
}

fn inv_sub_bytes(state: &mut [u8; 16]) {
    for b in state.iter_mut() {
        *b = INV_SBOX[*b as usize];
    }
}

/// ShiftRows on a column-major state (idx = col*4 + row): row r rotates left r.
fn shift_rows(s: &mut [u8; 16]) {
    let t = *s;
    for r in 1..4 {
        for c in 0..4 {
            s[c * 4 + r] = t[((c + r) % 4) * 4 + r];
        }
    }
}

fn inv_shift_rows(s: &mut [u8; 16]) {
    let t = *s;
    for r in 1..4 {
        for c in 0..4 {
            s[c * 4 + r] = t[((c + 4 - r) % 4) * 4 + r];
        }
    }
}

fn mix_columns(s: &mut [u8; 16]) {
    for c in 0..4 {
        let col = [s[c * 4], s[c * 4 + 1], s[c * 4 + 2], s[c * 4 + 3]];
        s[c * 4] = gmul(col[0], 2) ^ gmul(col[1], 3) ^ col[2] ^ col[3];
        s[c * 4 + 1] = col[0] ^ gmul(col[1], 2) ^ gmul(col[2], 3) ^ col[3];
        s[c * 4 + 2] = col[0] ^ col[1] ^ gmul(col[2], 2) ^ gmul(col[3], 3);
        s[c * 4 + 3] = gmul(col[0], 3) ^ col[1] ^ col[2] ^ gmul(col[3], 2);
    }
}

fn inv_mix_columns(s: &mut [u8; 16]) {
    for c in 0..4 {
        let col = [s[c * 4], s[c * 4 + 1], s[c * 4 + 2], s[c * 4 + 3]];
        s[c * 4] = gmul(col[0], 14) ^ gmul(col[1], 11) ^ gmul(col[2], 13) ^ gmul(col[3], 9);
        s[c * 4 + 1] = gmul(col[0], 9) ^ gmul(col[1], 14) ^ gmul(col[2], 11) ^ gmul(col[3], 13);
        s[c * 4 + 2] = gmul(col[0], 13) ^ gmul(col[1], 9) ^ gmul(col[2], 14) ^ gmul(col[3], 11);
        s[c * 4 + 3] = gmul(col[0], 11) ^ gmul(col[1], 13) ^ gmul(col[2], 9) ^ gmul(col[3], 14);
    }
}

fn aes_encrypt_block(key: &[u8], block: &[u8; 16], nr: usize) -> [u8; 16] {
    let w = key_expansion(key, nr);
    let mut s = *block;
    add_round_key(&mut s, &w, 0);
    for round in 1..nr {
        sub_bytes(&mut s);
        shift_rows(&mut s);
        mix_columns(&mut s);
        add_round_key(&mut s, &w, round);
    }
    sub_bytes(&mut s);
    shift_rows(&mut s);
    add_round_key(&mut s, &w, nr);
    s
}

fn aes_decrypt_block(key: &[u8], block: &[u8; 16], nr: usize) -> [u8; 16] {
    let w = key_expansion(key, nr);
    let mut s = *block;
    add_round_key(&mut s, &w, nr);
    for round in (1..nr).rev() {
        inv_shift_rows(&mut s);
        inv_sub_bytes(&mut s);
        add_round_key(&mut s, &w, round);
        inv_mix_columns(&mut s);
    }
    inv_shift_rows(&mut s);
    inv_sub_bytes(&mut s);
    add_round_key(&mut s, &w, 0);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ETS_AES_INTR_SOURCE on ESP32-S3.
    const AES_SOURCE: u32 = 77;

    /// Helper: write a 32-bit word through the byte-granular Peripheral API
    /// the way firmware's `REG_WRITE` would (4 byte stores + the coherent
    /// word callback).
    fn wr(a: &mut Esp32s3Aes, off: u64, val: u32) {
        for i in 0..4 {
            a.write(off + i, ((val >> (i * 8)) & 0xFF) as u8).unwrap();
        }
        a.write_word_32(off, val).unwrap();
    }

    fn rd(a: &Esp32s3Aes, off: u64) -> u32 {
        a.read_u32(off).unwrap()
    }

    /// Load 16 bytes into 4 consecutive registers using the HW LE packing.
    fn load_block(a: &mut Esp32s3Aes, base: u64, bytes: &[u8; 16]) {
        for i in 0..4 {
            let w = u32::from_le_bytes([
                bytes[i * 4],
                bytes[i * 4 + 1],
                bytes[i * 4 + 2],
                bytes[i * 4 + 3],
            ]);
            wr(a, base + (i as u64) * 4, w);
        }
    }

    fn read_block(a: &Esp32s3Aes, base: u64) -> [u8; 16] {
        let mut out = [0u8; 16];
        for i in 0..4 {
            let w = rd(a, base + (i as u64) * 4);
            out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        out
    }

    // FIPS-197 Appendix C.1 AES-128 known-answer vector.
    const KEY_128: [u8; 16] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f,
    ];
    const PT_128: [u8; 16] = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ];
    const CT_128: [u8; 16] = [
        0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30, 0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4, 0xc5,
        0x5a,
    ];

    #[test]
    fn defaults() {
        let a = Esp32s3Aes::new(AES_SOURCE);
        assert_eq!(rd(&a, STATE_REG), STATE_IDLE);
        assert_eq!(rd(&a, MODE_REG), 0);
        assert_eq!(rd(&a, DMA_ENABLE_REG), 0);
        assert_eq!(rd(&a, DATE_REG), AES_DATE_VALUE);
        assert_eq!(rd(&a, INT_ENA_REG), 0);
    }

    #[test]
    fn key_text_round_trip() {
        let mut a = Esp32s3Aes::new(AES_SOURCE);
        load_block(&mut a, KEY_BASE, &KEY_128);
        load_block(&mut a, TEXT_IN_BASE, &PT_128);
        wr(&mut a, KEY_BASE + 0x10, 0xDEAD_BEEF); // KEY_4 (unused for AES-128)
        wr(&mut a, IV_BASE, 0xCAFE_F00D);
        wr(&mut a, BLOCK_NUM_REG, 7);
        assert_eq!(read_block(&a, KEY_BASE), KEY_128);
        assert_eq!(read_block(&a, TEXT_IN_BASE), PT_128);
        assert_eq!(rd(&a, KEY_BASE + 0x10), 0xDEAD_BEEF);
        assert_eq!(rd(&a, IV_BASE), 0xCAFE_F00D);
        assert_eq!(rd(&a, BLOCK_NUM_REG), 7);
    }

    /// Bare-algorithm KAT (sanity-checks the cipher independent of the
    /// register plumbing).
    #[test]
    fn fips197_c1_algorithm() {
        let ct = aes_encrypt_block(&KEY_128, &PT_128, 10);
        assert_eq!(ct, CT_128);
        let pt = aes_decrypt_block(&KEY_128, &CT_128, 10);
        assert_eq!(pt, PT_128);
    }

    /// Full FIPS-197 AES-128 ECB encrypt through the register interface.
    #[test]
    fn fips197_encrypt_via_registers() {
        let mut a = Esp32s3Aes::new(AES_SOURCE);
        load_block(&mut a, KEY_BASE, &KEY_128);
        load_block(&mut a, TEXT_IN_BASE, &PT_128);
        wr(&mut a, MODE_REG, 0); // enc-128
        assert_eq!(rd(&a, STATE_REG), STATE_IDLE, "idle before trigger");
        wr(&mut a, TRIGGER_REG, 1);
        assert_eq!(rd(&a, STATE_REG), STATE_DONE, "STATE==done after trigger");
        assert_eq!(
            read_block(&a, TEXT_OUT_BASE),
            CT_128,
            "FIPS-197 C.1 ciphertext"
        );
    }

    /// Decrypt round-trip through the register interface.
    #[test]
    fn decrypt_round_trip_via_registers() {
        let mut a = Esp32s3Aes::new(AES_SOURCE);
        // Encrypt PT → CT.
        load_block(&mut a, KEY_BASE, &KEY_128);
        load_block(&mut a, TEXT_IN_BASE, &PT_128);
        wr(&mut a, MODE_REG, 0); // enc-128
        wr(&mut a, TRIGGER_REG, 1);
        let ct = read_block(&a, TEXT_OUT_BASE);
        assert_eq!(ct, CT_128);
        // Now decrypt CT → PT with mode dec-128 (4).
        load_block(&mut a, TEXT_IN_BASE, &ct);
        wr(&mut a, MODE_REG, MODE_DECRYPT_BIT); // dec-128
        wr(&mut a, TRIGGER_REG, 1);
        assert_eq!(rd(&a, STATE_REG), STATE_DONE);
        assert_eq!(
            read_block(&a, TEXT_OUT_BASE),
            PT_128,
            "decrypt recovers plaintext"
        );
    }

    /// NIST FIPS-197 / SP800-38A AES-256 ECB KAT through the register path.
    #[test]
    fn aes256_encrypt_via_registers() {
        // FIPS-197 Appendix C.3
        let key: [u8; 32] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b,
            0x1c, 0x1d, 0x1e, 0x1f,
        ];
        let ct_expected: [u8; 16] = [
            0x8e, 0xa2, 0xb7, 0xca, 0x51, 0x67, 0x45, 0xbf, 0xea, 0xfc, 0x49, 0x90, 0x4b, 0x49,
            0x60, 0x89,
        ];
        let mut a = Esp32s3Aes::new(AES_SOURCE);
        for i in 0..8 {
            let w =
                u32::from_le_bytes([key[i * 4], key[i * 4 + 1], key[i * 4 + 2], key[i * 4 + 3]]);
            wr(&mut a, KEY_BASE + (i as u64) * 4, w);
        }
        load_block(&mut a, TEXT_IN_BASE, &PT_128);
        wr(&mut a, MODE_REG, 2); // enc-256
        wr(&mut a, TRIGGER_REG, 1);
        assert_eq!(read_block(&a, TEXT_OUT_BASE), ct_expected);
    }

    /// DMA-done interrupt: raised on trigger in DMA mode, gated by INT_ENA,
    /// emitted level-sensitively via explicit_irqs, and W1C-cleared.
    #[test]
    fn dma_done_interrupt_and_int_clr_w1c() {
        let mut a = Esp32s3Aes::new(AES_SOURCE);
        load_block(&mut a, KEY_BASE, &KEY_128);
        load_block(&mut a, TEXT_IN_BASE, &PT_128);
        wr(&mut a, MODE_REG, 0);
        wr(&mut a, DMA_ENABLE_REG, 1); // DMA/block mode
        wr(&mut a, INT_ENA_REG, 1); // enable transform-complete IRQ
        wr(&mut a, TRIGGER_REG, 1);

        // While int_raw & int_ena, the AES source id is emitted every tick.
        let r = a.tick();
        assert_eq!(
            r.explicit_irqs.as_deref(),
            Some(&[AES_SOURCE][..]),
            "AES DMA-done IRQ emitted"
        );
        // Level-sensitive: still emitted on the next tick (no ACK yet).
        let r = a.tick();
        assert_eq!(r.explicit_irqs.as_deref(), Some(&[AES_SOURCE][..]));

        // INT_CLEAR is W1C — writing bit 0 clears the latch.
        wr(&mut a, INT_CLEAR_REG, 1);
        let r = a.tick();
        assert!(
            r.explicit_irqs.as_ref().is_none_or(|v| v.is_empty()),
            "no IRQ after INT_CLEAR"
        );
    }

    /// INT_ENA=0 suppresses IRQ delivery even though the DMA transform set the
    /// raw latch.
    #[test]
    fn int_ena_zero_suppresses_irq() {
        let mut a = Esp32s3Aes::new(AES_SOURCE);
        load_block(&mut a, KEY_BASE, &KEY_128);
        load_block(&mut a, TEXT_IN_BASE, &PT_128);
        wr(&mut a, DMA_ENABLE_REG, 1);
        // INT_ENA left at 0.
        wr(&mut a, TRIGGER_REG, 1);
        let r = a.tick();
        assert!(r.explicit_irqs.as_ref().is_none_or(|v| v.is_empty()));
    }

    /// Typical (non-DMA) mode does not raise the DMA-done interrupt.
    #[test]
    fn typical_mode_no_interrupt() {
        let mut a = Esp32s3Aes::new(AES_SOURCE);
        load_block(&mut a, KEY_BASE, &KEY_128);
        load_block(&mut a, TEXT_IN_BASE, &PT_128);
        wr(&mut a, MODE_REG, 0);
        wr(&mut a, INT_ENA_REG, 1);
        // DMA disabled (default 0) → typical path → no IRQ latch.
        wr(&mut a, TRIGGER_REG, 1);
        let r = a.tick();
        assert!(r.explicit_irqs.as_ref().is_none_or(|v| v.is_empty()));
    }
}
