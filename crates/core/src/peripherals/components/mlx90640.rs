// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Melexis MLX90640 32×24 far-IR thermal-camera array as an [`I2cDevice`].
//!
//! # Why this device is different
//!
//! Every other I²C device in `components/` uses an 8-bit register pointer
//! (`Bmp280`, `Pca9685`, …). The MLX90640 uses **16-bit register addressing
//! with 16-bit data words** (datasheet §I2C / §10): the master writes a 2-byte
//! big-endian register address, then reads/writes a stream of big-endian
//! 16-bit words while the address auto-increments by one word per word
//! transferred. This model implements that protocol faithfully.
//!
//! # Memory map (datasheet §11)
//!
//! | Range            | Words | Meaning                                        |
//! |------------------|-------|------------------------------------------------|
//! | 0x2400..0x2740   | 832   | EEPROM — per-pixel + global calibration        |
//! | 0x0400..0x0700   | 768   | RAM — raw frame pixel data (the IR image)      |
//! | 0x0700..0x0740   | 64    | RAM aux — Ta_PTAT, VDD, gain, CP pixels         |
//! | 0x8000           | 1     | STATUS — bit3 new-data, bits[2:0] last subpage |
//! | 0x800D           | 1     | CONTROL1 — refresh rate / subpage / resolution |
//!
//! The official Melexis driver flow this model satisfies:
//!   `MLX90640_DumpEE` → `MLX90640_ExtractParameters` (once), then loop
//!   `MLX90640_GetFrameData` (poll STATUS for new-data, read RAM 0x0400..0x0740,
//!   clear STATUS) → `MLX90640_CalculateTo(frame, params, ε, Ta)` → 768 °C.
//!
//! # Calibration approach — linearized, but decoded by the REAL driver
//!
//! Full MLX90640 factory-calibration inversion is intractable to author by
//! hand. This model instead populates the EEPROM with a **self-consistent,
//! linearized calibration set** chosen so the real driver's
//! `ExtractParameters` + `CalculateTo` math collapses to a clean, invertible
//! `raw_count ↔ °C` relation, and encodes the thermal scene into RAM raw
//! counts that decode back — through the unmodified Melexis driver — to the
//! injected temperatures within < 0.1 °C.
//!
//! What is held constant so the per-pixel compensation chain vanishes:
//!   * `offset[px] = 0`, `tgc = 0`, `KsTa = 0`, `ksTo[0..3] = 0`.
//!   * `kta`/`kv` are left at small non-zero values (the driver's
//!     normalization loops divide by `max|kta|` and would spin forever on an
//!     all-zero array) — but the **aux RAM is built so the driver decodes
//!     `Ta = 25 °C` and `VDD = 3.3 V` exactly**, which makes the
//!     `(1 + kta·(Ta−25))` and `(1 + kv·(VDD−3.3))` factors identically 1, so
//!     the kta/kv values never affect the result.
//!   * `gain = 1` (`gainEE == RAM gain word`), `emissivity = 1`, `Tr = Ta`.
//!
//! Under those choices `MLX90640_CalculateTo` reduces, per pixel, to
//!
//! ```text
//!   alphaComp = SCALEALPHA · 2^alphaScale / alpha[px]      (a per-pixel const)
//!   taTr      = (Ta + 273.15)^4                            (ε = 1)
//!   To        = ( raw / alphaComp + taTr )^(1/4) − 273.15
//! ```
//!
//! which inverts cleanly to the encoder used in [`Mlx90640::encode_raw`]:
//!
//! ```text
//!   raw = alphaComp · ( (To + 273.15)^4 − (Ta + 273.15)^4 )
//! ```
//!
//! This simplification (and only this) is what makes the round-trip exact; the
//! I²C protocol, register map, status/subpage handshake and the entire decode
//! path are the genuine article.

use std::any::Any;

use crate::peripherals::i2c::I2cDevice;

/// Default 7-bit I²C address (datasheet §10): 0x33.
pub const MLX90640_ADDR: u8 = 0x33;

const EE_BASE: u16 = 0x2400;
const EE_WORDS: usize = 832;
const RAM_BASE: u16 = 0x0400;
const RAM_WORDS: usize = 0x0340; // 0x0400..0x0740 → 832 words (768 px + 64 aux)
const STATUS_REG: u16 = 0x8000;
const CONTROL1_REG: u16 = 0x800D;

const PIXELS: usize = 768;
const COLS: usize = 32;
const ROWS: usize = 24;

const SCALEALPHA: f64 = 0.000_001;

/// Thermal scene: a first-order RC heating model over a 24×32 °C field with a
/// rectangular hotspot (electrical-cabinet / motor-housing positioning, where
/// heat *leads* a fault). Parameterized from `system.yaml`.
#[derive(Debug, Clone)]
pub struct ThermalScene {
    /// Ambient (background) temperature, °C.
    pub ambient_c: f64,
    /// Hotspot centre row (0..24) and col (0..32) and half-extent (radius, px).
    pub hot_row: usize,
    pub hot_col: usize,
    pub hot_radius: usize,
    /// Hotspot steady-state target before load scaling, °C.
    pub hot_target_c: f64,
    /// Load multiplier on the hotspot target (e.g. motor load 0..1+).
    pub load: f64,
    /// Heating time constant τ, seconds.
    pub tau_s: f64,
    /// Cooling efficiency 0..1 (fans). Subtracts from the effective target.
    pub cooling_efficiency: f64,
    /// Fault time: at/after this sim time (s) cooling collapses to 0, so the
    /// hotspot keeps climbing. `None` = no fault.
    pub cooling_fault_at_s: Option<f64>,
    /// Seconds of simulated time advanced per captured frame.
    pub frame_period_s: f64,

    /// Current hotspot temperature (the integrated RC state), °C.
    hot_now_c: f64,
    /// Elapsed simulated time, seconds.
    elapsed_s: f64,
}

impl Default for ThermalScene {
    fn default() -> Self {
        Self {
            ambient_c: 25.0,
            hot_row: 12,
            hot_col: 16,
            hot_radius: 0,
            hot_target_c: 60.0,
            load: 1.0,
            tau_s: 0.0, // 0 → reach target immediately (good for round-trip tests)
            cooling_efficiency: 0.0,
            cooling_fault_at_s: None,
            frame_period_s: 0.5,
            hot_now_c: 25.0,
            elapsed_s: 0.0,
        }
    }
}

impl ThermalScene {
    /// Build a scene from config values, initializing the integrator state to
    /// ambient. Keeps the RC-integrator fields (`hot_now_c`, `elapsed_s`)
    /// private while letting the factory construct from `system.yaml`.
    #[allow(clippy::too_many_arguments)]
    pub fn from_config(
        ambient_c: f64,
        hot_row: usize,
        hot_col: usize,
        hot_radius: usize,
        hot_target_c: f64,
        load: f64,
        tau_s: f64,
        cooling_efficiency: f64,
        cooling_fault_at_s: Option<f64>,
        frame_period_s: f64,
    ) -> Self {
        Self {
            ambient_c,
            hot_row,
            hot_col,
            hot_radius,
            hot_target_c,
            load,
            tau_s,
            cooling_efficiency,
            cooling_fault_at_s,
            frame_period_s,
            hot_now_c: ambient_c,
            elapsed_s: 0.0,
        }
    }

    /// Effective hotspot steady-state target accounting for load and cooling.
    fn target_c(&self) -> f64 {
        let cooling = match self.cooling_fault_at_s {
            Some(t) if self.elapsed_s >= t => 0.0, // fault: cooling collapsed
            _ => self.cooling_efficiency,
        };
        let driven = self.ambient_c + (self.hot_target_c - self.ambient_c) * self.load;
        // Cooling pulls the achievable peak back toward ambient.
        self.ambient_c + (driven - self.ambient_c) * (1.0 - cooling)
    }

    /// Advance the RC model by one frame period and return the 768-pixel field.
    fn advance(&mut self) -> Vec<f64> {
        self.elapsed_s += self.frame_period_s;
        let target = self.target_c();
        if self.tau_s <= 0.0 {
            self.hot_now_c = target;
        } else {
            let alpha = 1.0 - (-self.frame_period_s / self.tau_s).exp();
            self.hot_now_c += (target - self.hot_now_c) * alpha;
        }
        self.field()
    }

    /// Render the current field without advancing time.
    fn field(&self) -> Vec<f64> {
        let mut f = vec![self.ambient_c; PIXELS];
        for r in 0..ROWS {
            for c in 0..COLS {
                let dr = r as isize - self.hot_row as isize;
                let dc = c as isize - self.hot_col as isize;
                if dr.unsigned_abs() <= self.hot_radius && dc.unsigned_abs() <= self.hot_radius {
                    f[r * COLS + c] = self.hot_now_c;
                }
            }
        }
        f
    }

    /// Current elapsed simulated time, seconds.
    pub fn elapsed_s(&self) -> f64 {
        self.elapsed_s
    }

    /// Current integrated hotspot temperature, °C (for tests / inspection).
    pub fn hotspot_c(&self) -> f64 {
        self.hot_now_c
    }
}

/// Transaction direction state for the 16-bit-addressed protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Expecting the high byte of the 2-byte register address.
    AddrHi,
    /// Expecting the low byte of the 2-byte register address.
    AddrLo,
    /// Address latched; subsequent writes are data words, reads stream words.
    Data,
}

pub struct Mlx90640 {
    address: u8,
    eeprom: Vec<u16>, // 832 words at EE_BASE
    ram: Vec<u16>,    // 832 words at RAM_BASE (768 px + 64 aux)
    status: u16,      // 0x8000
    control1: u16,    // 0x800D

    // 16-bit-address transaction engine.
    phase: Phase,
    addr_hi: u8,
    reg_addr: u16, // current auto-incrementing word address
    /// Pending write word (high byte buffered until low byte arrives).
    wr_hi: Option<u8>,
    /// Read byte toggle: false = next emit MSB, true = next emit LSB.
    rd_low: bool,
    rd_word: u16,

    scene: ThermalScene,
    /// Quantized per-pixel alpha values read back from a real driver-style
    /// extraction of our EEPROM (used to invert the decode exactly).
    alpha: Vec<u16>,
    alpha_scale: u8,
    /// Has the driver consumed the current frame? When it clears STATUS we
    /// advance the scene and arm the next subpage.
    last_subpage: u16,
}

impl std::fmt::Debug for Mlx90640 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mlx90640")
            .field("address", &self.address)
            .field("status", &self.status)
            .field("control1", &self.control1)
            .field("elapsed_s", &self.scene.elapsed_s)
            .finish()
    }
}

impl Mlx90640 {
    pub fn new(address: u8, scene: ThermalScene) -> Self {
        let addr = if address == 0 { MLX90640_ADDR } else { address };
        let mut dev = Self {
            address: addr,
            eeprom: vec![0; EE_WORDS],
            ram: vec![0; RAM_WORDS],
            status: 0x0000,
            // CONTROL1 reset value (datasheet): 0x1901 — chess mode (bit12),
            // 2 Hz refresh, resolution 18-bit. We force chess (bit12=1) so the
            // driver's mode matches our `calibrationModeEE`.
            control1: 0x1901,
            phase: Phase::AddrHi,
            addr_hi: 0,
            reg_addr: 0,
            wr_hi: None,
            rd_low: false,
            rd_word: 0,
            scene,
            alpha: vec![0; PIXELS],
            alpha_scale: 0,
            last_subpage: 1,
        };
        dev.build_eeprom();
        dev.extract_alpha();
        // Capture the first frame so a driver that reads immediately sees data
        // with new-data armed. This advances the scene by one frame period.
        dev.capture_frame();
        dev
    }

    pub fn with_default_scene(address: u8) -> Self {
        Self::new(address, ThermalScene::default())
    }

    /// Read-only access to the evolving scene (for tests / inspection).
    pub fn scene(&self) -> &ThermalScene {
        &self.scene
    }

    // ── Linearized EEPROM construction ──────────────────────────────────────
    //
    // Mirrors the value choices validated against the real Melexis driver:
    // every per-pixel compensation term is forced to vanish (see module docs),
    // leaving a clean radiometric inverse. Word indices below are EEPROM-array
    // indices (word 0 == address 0x2400), matching the driver's `eeData[i]`.
    fn build_eeprom(&mut self) {
        let ee = &mut self.eeprom;
        for w in ee.iter_mut() {
            *w = 0;
        }

        // VDD: ee[51] = kVdd(MSB int8) | vdd25_lsb. kVdd = 32 * (-32) = -1024.
        // vdd25 = ((0xAA - 256) << 5) - 8192 = -10944. Aux RAM sets frame[810]
        // = vdd25 (with resolution-correction = 1) so GetVdd == 3.3 V exactly.
        ee[51] = ((0xE0u16) << 8) | 0xAA;

        // PTAT (Ta): ee[50] low10 = KtPTAT*8 = 40 → KtPTAT = 5.0; KvPTAT = 0.
        // alphaPTAT = 8.0 (ee[16] nibble4 = 0). vPTAT25 (ee[49]) is set to the
        // decoded ptatArt in `aux_words()` so GetTa == 25 °C.
        ee[50] = 40;

        // gainEE = ee[48]; aux RAM gain word == gainEE → gain = 1.
        ee[48] = 6000;

        // Resolution bits (ee[56] bits13:12) = 2; aux ctrl uses the same → no
        // resolution correction. ktaScale/kvScale nibbles stay 0.
        ee[56] = 2u16 << 12;

        // Non-zero kta/kv seeds so the driver's `while(max < 63.4)` loops
        // terminate. Inert at Ta = 25 / VDD = 3.3.
        ee[54] = 0x0101;
        ee[55] = 0x0101;
        ee[52] = 0x1111;

        // KsTo: ee[61], ee[62] bytes = 0 → ksTo[0..3] = 0. ee[63] sets the
        // corner-temperature (ct) nibbles + scale; ranges only pick an index,
        // and with ksTo = 0 every range correction is the identity.
        // step nibble = 1 (→10), ct2 nibble = 2 (→20), ct3 nibble = 2 (→40).
        ee[63] = (1u16 << 12) | (2u16 << 4) | (2u16 << 8);

        // alpha: ee[32] scales all 0 → alphaScale = 30. alphaRef (ee[33]) sets
        // the uniform per-pixel sensitivity.
        ee[32] = 0;
        ee[33] = 1000;

        // Per-pixel words: put a uniform value of 1 in the offset 6-bit field
        // (bits 0xFC00 → 0x0400) so no word is zero (zero = "broken pixel") and
        // bit0 (outlier flag) stays clear. offsetRef (ee[17]) is set to −1 so
        // the net offset[px] is 0.
        let word = 0x0400u16;
        for p in 0..PIXELS {
            ee[64 + p] = word;
        }
        ee[17] = (-1i16) as u16; // offsetRef cancels the per-pixel +1
    }

    /// Aux RAM (64 words at 0x0700) built so the driver decodes Ta = 25 °C,
    /// VDD = 3.3 V and gain = 1. Indices are *frame* indices minus 768.
    /// `frameData[768..832]` ← these words; `frameData[832]` = control reg,
    /// `frameData[833]` = subpage. Key aux positions the driver reads:
    ///   * frame[768] = ptat_art aux, frame[776]/[808] = CP pixels (subpage 0/1)
    ///   * frame[778] = gain, frame[800] = ptat, frame[810] = VDD pixel
    fn aux_words(&mut self) -> [u16; 64] {
        // Aux-array local indices = (driver frame index) − 768.
        const AUX_PTAT_ART: usize = 768 - PIXELS; // frame[768]
        const AUX_GAIN: usize = 778 - PIXELS; // frame[778]
        const AUX_PTAT: usize = 800 - PIXELS; // frame[800]
        const AUX_VDD: usize = 810 - PIXELS; // frame[810]
        const AUX_CP0: usize = 776 - PIXELS; // frame[776] (subpage-0 CP pixel)
        const AUX_CP1: usize = 808 - PIXELS; // frame[808] (subpage-1 CP pixel)

        let mut aux = [0u16; 64];
        // gain word: frame[778] == gainEE → gain = 1.
        aux[AUX_GAIN] = self.eeprom[48];
        // VDD pixel: frame[810] == vdd25 → GetVdd = 3.3.
        let vdd25: i16 = (((0xAA - 256) << 5) - 8192) as i16;
        aux[AUX_VDD] = vdd25 as u16;
        // PTAT: frame[800] = ptat, frame[768] = ptat_art aux.
        let ptat = 8000i32;
        let g = 8000i32;
        aux[AUX_PTAT] = ptat as u16;
        aux[AUX_PTAT_ART] = g as u16;
        // vPTAT25 set so ptatArt == vPTAT25 → Ta = 25.
        let alpha_ptat = 8.0f64;
        let ptat_art = (ptat as f64 / (ptat as f64 * alpha_ptat + g as f64)) * 2f64.powi(18);
        self.eeprom[49] = ptat_art.round() as i16 as u16;
        // CP pixels: tgc = 0 so they do not affect To; leave 0.
        aux[AUX_CP0] = 0;
        aux[AUX_CP1] = 0;
        aux
    }

    // ── Driver-faithful alpha extraction (subset) ───────────────────────────
    //
    // Reproduces exactly the parts of `ExtractAlphaParameters` that determine
    // `params.alpha[]` and `params.alphaScale` for our uniform EEPROM, so the
    // encoder inverts the real driver's decode bit-for-bit. With accRow =
    // accColumn = 0 and per-pixel alpha field = 0, every pixel is identical.
    fn extract_alpha(&mut self) {
        let ee = &self.eeprom;
        let acc_rem_scale = (ee[32] & 0x000F) as u32; // 0
        let alpha_scale_ee = ((ee[32] & 0xF000) >> 12) as u32 + 30; // 30
        let alpha_ref = ee[33] as i32; // 1000
        let tgc = (ee[60] as i8 as f64) / 32.0; // 0

        // cpAlpha for the tgc term (tgc = 0 makes it irrelevant, kept for fidelity).
        let cp_alpha0 = 0.0f64;
        let cp_alpha1 = 0.0f64;

        let mut alpha_temp = vec![0f64; PIXELS];
        for p in 0..PIXELS {
            let mut a = ((ee[64 + p] & 0x03F0) >> 4) as f64; // 0
            if a > 31.0 {
                a -= 64.0;
            }
            a *= (1u32 << acc_rem_scale) as f64;
            // accRow/accColumn contributions are 0 here.
            a += alpha_ref as f64;
            a /= 2f64.powi(alpha_scale_ee as i32);
            a -= tgc * (cp_alpha0 + cp_alpha1) / 2.0;
            a = SCALEALPHA / a;
            alpha_temp[p] = a;
        }

        let mut temp = alpha_temp[0];
        for &v in alpha_temp.iter().skip(1) {
            if v > temp {
                temp = v;
            }
        }
        let mut alpha_scale: u8 = 0;
        while temp < 32767.4 {
            temp *= 2.0;
            alpha_scale += 1;
        }
        for (slot, &at) in self.alpha.iter_mut().zip(alpha_temp.iter()) {
            let t = at * 2f64.powi(alpha_scale as i32);
            *slot = (t + 0.5) as u16;
        }
        self.alpha_scale = alpha_scale;
    }

    /// Inverse of the reduced `CalculateTo`: raw int16 count for a target °C.
    /// `raw = alphaComp · ((To+273.15)^4 − (Ta+273.15)^4)`, gain = 1, Ta = 25.
    fn encode_raw(&self, to_c: f64) -> u16 {
        const TA: f64 = 25.0;
        let alpha_scale_p = 2f64.powi(self.alpha_scale as i32);
        // Uniform alpha → use pixel 0 (all equal).
        let alpha_comp = SCALEALPHA * alpha_scale_p / self.alpha[0] as f64;
        let ta_k = TA + 273.15;
        let ta_tr = ta_k * ta_k * ta_k * ta_k;
        let to_k = to_c + 273.15;
        let ir = alpha_comp * (to_k * to_k * to_k * to_k - ta_tr);
        let r = ir.round().clamp(-32768.0, 32767.0);
        r as i16 as u16
    }

    /// Capture a frame: advance the thermal scene, encode every pixel into RAM,
    /// fill aux RAM, toggle the subpage and set STATUS new-data.
    fn capture_frame(&mut self) {
        let field = self.scene.advance();
        for (p, &t) in field.iter().enumerate() {
            self.ram[p] = self.encode_raw(t);
        }
        let aux = self.aux_words();
        for (i, &w) in aux.iter().enumerate() {
            self.ram[PIXELS + i] = w;
        }
        // Toggle subpage 0↔1; both subpages carry the same scene so either
        // decode reconstructs its half correctly.
        let next = self.last_subpage ^ 1;
        self.last_subpage = next;
        // STATUS: bit3 = new-data ready, bits[2:0] = last subpage.
        self.status = (1 << 3) | (next & 0x7);
    }

    // ── 16-bit register map access ──────────────────────────────────────────
    fn read_word(&mut self, addr: u16) -> u16 {
        match addr {
            STATUS_REG => self.status,
            CONTROL1_REG => self.control1,
            a if (EE_BASE..EE_BASE + EE_WORDS as u16).contains(&a) => {
                self.eeprom[(a - EE_BASE) as usize]
            }
            a if (RAM_BASE..RAM_BASE + RAM_WORDS as u16).contains(&a) => {
                self.ram[(a - RAM_BASE) as usize]
            }
            _ => 0,
        }
    }

    fn write_word(&mut self, addr: u16, value: u16) {
        match addr {
            STATUS_REG => {
                // Driver writes MLX90640_INIT_STATUS_VALUE (0x0030) to clear the
                // new-data flag and request the next frame. On that clear we
                // capture the next frame so the subsequent poll sees fresh data.
                self.status = value & !0x0008;
                if value & 0x0008 == 0 {
                    self.capture_frame();
                }
            }
            CONTROL1_REG => self.control1 = value,
            // EEPROM/RAM are read-only over I²C in this model.
            _ => {}
        }
    }
}

impl I2cDevice for Mlx90640 {
    fn address(&self) -> u8 {
        self.address
    }

    fn start(&mut self) {
        // A (repeated) START begins a new address phase only if we are not in
        // the middle of latching one. The driver issues: write 2 addr bytes,
        // repeated-start, then read — the address set by the write must persist
        // across the repeated-start, so we only reset to AddrHi on a fresh
        // write transaction, signalled by the first written byte (see `write`).
        self.rd_low = false;
    }

    fn write(&mut self, data: u8) {
        match self.phase {
            Phase::AddrHi => {
                self.addr_hi = data;
                self.phase = Phase::AddrLo;
            }
            Phase::AddrLo => {
                self.reg_addr = ((self.addr_hi as u16) << 8) | data as u16;
                self.phase = Phase::Data;
                self.wr_hi = None;
            }
            Phase::Data => {
                // 16-bit data write, MSB first, auto-incrementing word address.
                match self.wr_hi.take() {
                    None => self.wr_hi = Some(data),
                    Some(hi) => {
                        let word = ((hi as u16) << 8) | data as u16;
                        self.write_word(self.reg_addr, word);
                        self.reg_addr = self.reg_addr.wrapping_add(1);
                    }
                }
            }
        }
    }

    fn read(&mut self) -> u8 {
        // Streaming 16-bit reads, MSB first, auto-incrementing word address.
        if !self.rd_low {
            self.rd_word = self.read_word(self.reg_addr);
            self.rd_low = true;
            (self.rd_word >> 8) as u8
        } else {
            self.rd_low = false;
            let lsb = (self.rd_word & 0xFF) as u8;
            self.reg_addr = self.reg_addr.wrapping_add(1);
            lsb
        }
    }

    fn stop(&mut self) {
        // End of transaction: next transaction starts with a fresh address.
        self.phase = Phase::AddrHi;
        self.wr_hi = None;
        self.rd_low = false;
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive a 16-bit-addressed register read the way the MLX driver does:
    /// write 2 addr bytes, repeated-start, then read `n` words.
    fn read_words(dev: &mut Mlx90640, addr: u16, n: usize) -> Vec<u16> {
        dev.start();
        dev.write((addr >> 8) as u8);
        dev.write((addr & 0xFF) as u8);
        dev.start(); // repeated start → read direction
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let hi = dev.read();
            let lo = dev.read();
            out.push(((hi as u16) << 8) | lo as u16);
        }
        dev.stop();
        out
    }

    fn write_word_i2c(dev: &mut Mlx90640, addr: u16, value: u16) {
        dev.start();
        dev.write((addr >> 8) as u8);
        dev.write((addr & 0xFF) as u8);
        dev.write((value >> 8) as u8);
        dev.write((value & 0xFF) as u8);
        dev.stop();
    }

    #[test]
    fn default_address_is_0x33() {
        assert_eq!(Mlx90640::with_default_scene(0).address(), MLX90640_ADDR);
    }

    #[test]
    fn sixteen_bit_addressing_reads_eeprom_word() {
        let mut dev = Mlx90640::with_default_scene(MLX90640_ADDR);
        // ee[48] = gainEE = 6000 lives at address 0x2400 + 48 = 0x2430.
        let words = read_words(&mut dev, EE_BASE + 48, 1);
        assert_eq!(words[0], 6000, "gainEE word must read back over 16-bit I²C");
    }

    #[test]
    fn address_auto_increments_across_words() {
        let mut dev = Mlx90640::with_default_scene(MLX90640_ADDR);
        // Read ee[48], ee[49], ee[50] in one streaming read.
        let words = read_words(&mut dev, EE_BASE + 48, 3);
        assert_eq!(words[0], 6000); // gainEE
        assert_eq!(words[2], 40); // ee[50] KtPTAT seed
    }

    #[test]
    fn status_new_data_then_clear_arms_next_frame() {
        let mut dev = Mlx90640::with_default_scene(MLX90640_ADDR);
        let st = read_words(&mut dev, STATUS_REG, 1)[0];
        assert_ne!(st & 0x0008, 0, "new-data must be set after a capture");
        // Clear it (driver writes 0x0030); a fresh frame must re-arm new-data.
        write_word_i2c(&mut dev, STATUS_REG, 0x0030);
        let st2 = read_words(&mut dev, STATUS_REG, 1)[0];
        assert_ne!(st2 & 0x0008, 0, "new-data re-armed after clear");
    }

    #[test]
    fn eeprom_dump_is_832_words() {
        let mut dev = Mlx90640::with_default_scene(MLX90640_ADDR);
        let ee = read_words(&mut dev, EE_BASE, EE_WORDS);
        assert_eq!(ee.len(), 832);
        assert_eq!(ee[48], 6000);
    }

    #[test]
    fn ram_pixel_block_encodes_ambient_hotspot() {
        // Without the real driver we still sanity-check that the hotspot pixel
        // encodes to a larger raw count than ambient (monotonic encoder).
        let mut dev = Mlx90640::with_default_scene(MLX90640_ADDR);
        let ram = read_words(&mut dev, RAM_BASE, PIXELS);
        let ambient_raw = ram[0] as i16;
        let hot_raw = ram[12 * 32 + 16] as i16;
        assert!(
            hot_raw > ambient_raw,
            "hotspot raw {hot_raw} must exceed ambient raw {ambient_raw}"
        );
    }

    #[test]
    fn scene_fault_makes_hotspot_climb() {
        // Cooling efficiency 0.8 suppresses the hotspot to ~32 °C
        // (25 + (60-25)*(1-0.8)) until the fault at t=15 s collapses cooling,
        // after which it climbs toward the un-cooled target (60 °C).
        let scene = ThermalScene::from_config(
            25.0, // ambient
            12,
            16,
            0,
            60.0,       // hot_target
            1.0,        // load
            1.0,        // tau
            0.8,        // cooling_efficiency
            Some(15.0), // cooling_fault_at_s
            1.0,        // frame_period
        );
        let mut dev = Mlx90640::new(MLX90640_ADDR, scene);
        // Settle to the cooled steady state (well before the fault).
        for _ in 0..8 {
            write_word_i2c(&mut dev, STATUS_REG, 0x0030);
        }
        let peak_before = dev.scene().hotspot_c();
        assert!(
            (peak_before - 32.0).abs() < 1.0,
            "cooled steady state should be ~32 °C, got {peak_before}"
        );
        // Run past the fault and let it climb.
        for _ in 0..20 {
            write_word_i2c(&mut dev, STATUS_REG, 0x0030);
        }
        let peak_after = dev.scene().hotspot_c();
        assert!(
            peak_after > peak_before + 20.0,
            "cooling fault should let the hotspot climb: before={peak_before} after={peak_after}"
        );
    }
}
