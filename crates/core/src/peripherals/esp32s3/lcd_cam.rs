// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 LCD_CAM controller — configuration + transaction-control digital twin.
//!
//! The S3 bundles an LCD master (I8080 / RGB / MOTO6800 TX) and a camera slave
//! (DVP RX) into a single peripheral at `0x6004_1000` (4 KiB window) sharing one
//! interrupt source (`ETS_LCD_CAM_INTR_SOURCE = 24`) and one DMA-interrupt block.
//!
//! ## Scope
//!
//! This twin faithfully round-trips every configuration register the esp-hal /
//! ESP-IDF LCD_CAM drivers program (clock, MISC, CTRL/CTRL1/CTRL2, the command
//! value register, CAM_CTRL/CTRL1, RGB↔YUV) and models the control semantics
//! the polling / IRQ driver paths depend on:
//!
//! * **LCD transaction**: setting `LCD_USER.LCD_START` (bit 27) launches a
//!   transaction. On a later [`tick`](Peripheral::tick) we treat the
//!   transaction as complete: the `LCD_TRANS_DONE` raw-interrupt bit latches
//!   and `LCD_START` auto-clears (real silicon clears START when the configured
//!   command/dummy/dout phases finish). Firmware that polls `LCD_USER.LCD_START`
//!   for 0, polls `LC_DMA_INT_RAW`, or waits on the IRQ all make progress
//!   instead of hanging.
//! * **CAM transaction**: setting `CAM_CTRL1.CAM_START` (bit 30) starts a
//!   frame capture. On a later tick we latch `CAM_VSYNC` (frame boundary) into
//!   the raw-interrupt block. `CAM_START` is a level/run bit on silicon and is
//!   left asserted (the driver clears it explicitly to stop), matching HW.
//! * **Interrupt block** (`LC_DMA_INT_*`): `INT_ST = INT_RAW & INT_ENA`;
//!   `INT_CLR` is write-1-to-clear over the latched raw bits; the matrix source
//!   (24) is emitted from `tick` while `INT_ST != 0` (level-triggered, matching
//!   the timer-group / I2S twins).
//!
//! * **i80 pixel streaming** (`esp_lcd` i80 driver): a transaction drives the
//!   attached 8080 parallel panel with the same word sequence silicon puts on
//!   DB[15:0]. The command phase emits `LCD_CMD_VAL` with D/C at the
//!   `LCD_MISC.CD_CMD_SET` level; the DOUT phase emits the payload GDMA fetched
//!   from the outlink descriptor chain, with D/C at the `CD_DATA_SET` level.
//!   The payload is pushed in by [`Esp32s3Gdma`](super::gdma::Esp32s3Gdma)'s
//!   LCD pump ([`Self::dma_push_tx`]) — GDMA owns the descriptor walk, this
//!   model owns the bus protocol and the panel handoff. TRANS_DONE is withheld
//!   until the chain drains ([`Self::dma_finish`]), so a driver polling
//!   `LC_DMA_INT_ST` cannot rearm the descriptors under an in-flight transfer.
//!
//! Line/frame timing is still not modelled: there is no pixel clock, no
//! per-cycle DOUT length and no RGB-mode sync generator. A transaction moves
//! the whole descriptor chain and then completes.
//!
//! ## Register map (ESP32-S3 TRM ch. "LCD and Camera Controller"; verified
//! against `soc/esp32s3/register/soc/lcd_cam_reg.h`)
//!
//! | Offset | Name             | Notes                                              |
//! |-------:|------------------|----------------------------------------------------|
//! | 0x00   | LCD_CLOCK        | LCD clock-source select + dividers                 |
//! | 0x04   | CAM_CTRL         | CAM clock / sampling-edge / mode config            |
//! | 0x08   | CAM_CTRL1        | CAM_START=b30, CAM_RESET=b29, frame/line config    |
//! | 0x0C   | CAM_RGB_YUV      | CAM RGB↔YUV color-conversion config                |
//! | 0x10   | LCD_RGB_YUV      | LCD RGB↔YUV color-conversion config                |
//! | 0x14   | LCD_USER         | LCD_START=b27, LCD_CMD=b26, LCD_DUMMY=b25, LCD_DOUT=b24, resets |
//! | 0x18   | LCD_MISC        | LCD bus/CS timing, idle-level, AFIFO reset         |
//! | 0x1C   | LCD_CTRL        | LCD RGB mode, h/v sync + de-output enables         |
//! | 0x20   | LCD_CTRL1        | LCD RGB H/V front/back-porch + sync widths         |
//! | 0x24   | LCD_CTRL2        | LCD sync pulse widths / polarity                   |
//! | 0x28   | LCD_CMD_VAL      | LCD command value driven during the command phase  |
//! | 0x30   | LCD_DLY_MODE     | LCD output / D-C delay mode                        |
//! | 0x38   | LCD_DATA_DOUT_MODE | per-data-line output delay mode                  |
//! | 0x64   | LC_DMA_INT_ENA   | interrupt enable mask                              |
//! | 0x68   | LC_DMA_INT_RAW   | raw latched events (RO here)                       |
//! | 0x6C   | LC_DMA_INT_ST    | INT_RAW & INT_ENA (RO)                             |
//! | 0x70   | LC_DMA_INT_CLR   | W1C against INT_RAW                                |
//!
//! Any other offset accepts writes silently and reads 0.

use crate::{CycleClock, Peripheral, PeripheralTickResult, SimResult};

const DEFERRED_WAKE_TOKEN: u32 = 1;

/// LCD_CAM MMIO base address.
pub const LCD_CAM_BASE: u32 = 0x6004_1000;
/// MMIO window size (4 KiB).
pub const LCD_CAM_SIZE: u64 = 0x1000;

/// `ETS_LCD_CAM_INTR_SOURCE` — verified position in the ESP32-S3 interrupt
/// source enum (`soc/interrupts.h`): WIFI_MAC=0 … GPIO=16 … **LCD_CAM=24**,
/// I2S0=25.
pub const LCD_CAM_INTR_SOURCE_ID: u32 = 24;

// ── Register offsets ──
const REG_LCD_CLOCK: u64 = 0x00;
const REG_CAM_CTRL: u64 = 0x04;
const REG_CAM_CTRL1: u64 = 0x08;
const REG_CAM_RGB_YUV: u64 = 0x0C;
const REG_LCD_USER: u64 = 0x14;
const REG_LCD_MISC: u64 = 0x18;
const REG_LCD_CTRL: u64 = 0x1C;
const REG_LCD_CTRL1: u64 = 0x20;
const REG_LCD_CTRL2: u64 = 0x24;
const REG_LCD_CMD_VAL: u64 = 0x28;
/// LCD_DLY_MODE (0x30) — per-line output delay / D-C delay ticks.
const REG_LCD_DLY_MODE: u64 = 0x30;
/// LCD_DATA_DOUT_MODE (0x38) — per-data-line output delay mode.
const REG_LCD_DOUT_MODE: u64 = 0x38;
/// LCD_RGB_YUV (0x10) — the LCD-side colour converter. Distinct register from
/// CAM_RGB_YUV at 0x0C.
const REG_LCD_RGB_YUV: u64 = 0x10;
// The LC_DMA interrupt block sits at 0x64..0x70, NOT at 0x34..0x40.
// `soc/esp32s3/lcd_cam_reg.h`: LC_DMA_INT_ENA 0x64, _RAW 0x68, _ST 0x6c,
// _CLR 0x70. Modelled 0x30 lower, every driver access missed:
//   * `lcd_ll_enable_interrupt` (0x64) fell through to accept-and-ignore, so
//     INT_ENA stayed 0 and INT_ST (RAW & ENA) could never be non-zero;
//   * `lcd_ll_get_interrupt_status` (0x6c) read the unmapped-offset 0;
//   * LCD_DATA_DOUT_MODE (0x38) landed on what the model called INT_RAW.
// Net effect: `esp_lcd_new_i80_bus`'s
//   while (!(lcd_ll_get_interrupt_status(dev) & LCD_LL_EVENT_TRANS_DONE)) {}
// spun forever even with TRANS_DONE correctly latched in INT_RAW.
const REG_LC_DMA_INT_ENA: u64 = 0x64;
const REG_LC_DMA_INT_RAW: u64 = 0x68;
const REG_LC_DMA_INT_ST: u64 = 0x6C;
const REG_LC_DMA_INT_CLR: u64 = 0x70;

// ── LCD_USER bits (TRM "LCD and Camera Controller"; soc/lcd_cam_reg.h) ──
/// LCD_DOUT — enable the data-out (payload) phase.
const LCD_DOUT_BIT: u32 = 1 << 24;
/// LCD_DUMMY — enable the dummy phase. Round-tripped only: the dummy phase
/// drives no bus words, so nothing downstream branches on it.
#[allow(dead_code)]
const LCD_DUMMY_BIT: u32 = 1 << 25;
/// LCD_CMD — enable the command phase.
const LCD_CMD_BIT: u32 = 1 << 26;
/// LCD_START — launch the configured transaction. Self-clears on completion.
const LCD_START_BIT: u32 = 1 << 27;
/// LCD_RESET — write-pulse reset of the LCD module (WO). Self-clears.
///
/// Bit **28**, per `soc/esp32s3/lcd_cam_struct.h` (`lcd_start[27]`,
/// `lcd_reset[28]`, `lcd_dummy_cyclelen[30:29]`, `lcd_cmd_2_cycle_en[31]`).
/// This used to be modelled at bit 30, which both accepted a reset that never
/// arrived and *stripped* the low bit of `LCD_DUMMY_CYCLELEN` on every write.
const LCD_RESET_BIT: u32 = 1 << 28;
/// LCD_2BYTE_EN — 1: the LCD data bus is 16 bits wide; 0: 8 bits (bit 23).
const LCD_2BYTE_EN_BIT: u32 = 1 << 23;
/// LCD_BYTE_ORDER — invert the byte order of a 16-bit word (bit 22).
const LCD_BYTE_ORDER_BIT: u32 = 1 << 22;
/// LCD_BIT_ORDER — reverse the data bit order within a bus word (bit 21).
const LCD_BIT_ORDER_BIT: u32 = 1 << 21;
/// LCD_8BITS_ORDER — swap every two data bytes; valid in 8-bit mode (bit 19).
const LCD_8BITS_ORDER_BIT: u32 = 1 << 19;
/// LCD_CMD_2_CYCLE_EN — the command phase is 2 cycles instead of 1 (bit 31).
const LCD_CMD_2_CYCLE_BIT: u32 = 1 << 31;

// ── LCD_MISC D/C-line bits (`lcd_cam_lcd_misc_reg_t`) ──
//
// `lcd_ll_set_dc_level` encodes the four per-phase D/C levels as an idle
// level plus three "differs from idle" flags, so the level driven during
// phase P is `CD_IDLE_EDGE ^ CD_<P>_SET`.
/// LCD_CD_DATA_SET — D/C during the DOUT phase differs from idle (bit 28).
const MISC_CD_DATA_SET_BIT: u32 = 1 << 28;
/// LCD_CD_CMD_SET — D/C during the CMD phase differs from idle (bit 30).
const MISC_CD_CMD_SET_BIT: u32 = 1 << 30;
/// LCD_CD_IDLE_EDGE — the D/C level while the bus is idle (bit 31).
const MISC_CD_IDLE_EDGE_BIT: u32 = 1 << 31;
/// LCD_UPDATE — latch the LCD config into the working set. Write-pulse,
/// self-clears.
const LCD_UPDATE_BIT: u32 = 1 << 20;
/// LCD_USER write-pulse bits that self-clear immediately (the driver writes 1
/// and expects them to read 0 again). LCD_START is handled separately because
/// it stays asserted until the transaction completes on a later tick.
const LCD_USER_PULSE_BITS: u32 = LCD_RESET_BIT | LCD_UPDATE_BIT;

// ── CAM_CTRL1 bits ──
/// CAM_RESET — write-pulse reset of the CAM module. Self-clears.
const CAM_RESET_BIT: u32 = 1 << 29;
/// CAM_START — start camera capture (run/level bit; driver clears to stop).
const CAM_START_BIT: u32 = 1 << 30;
/// CAM_CTRL1 write-pulse bits that self-clear immediately.
const CAM_CTRL1_PULSE_BITS: u32 = CAM_RESET_BIT;

// ── LC_DMA_INT_* bit positions (RAW/ST/ENA/CLR share the layout) ──
//
// Consecutive bits 0..3, per `soc/esp32s3/lcd_cam_struct.h`
// (`lcd_cam_lc_dma_int_raw_reg_t`: lcd_vsync[0], lcd_trans_done[1],
// cam_vsync[2], cam_hs[3]). These were previously spaced two apart, so only
// LCD_VSYNC landed on the right bit. That made every i80 transaction hang:
// `esp_lcd_new_i80_bus` ends in
//   while (!(lcd_ll_get_interrupt_status(dev) & LCD_LL_EVENT_TRANS_DONE)) {}
// and `lcd_ll_get_interrupt_status` masks INT_ST with 0x03 — so a TRANS_DONE
// latched at bit 2 is invisible to the driver and the poll never exits.
/// LCD RGB-mode vertical-sync edge — bit 0.
pub const INT_LCD_VSYNC: u32 = 1 << 0;
/// LCD transaction finished (command/dummy/dout phases done) — bit 1.
pub const INT_LCD_TRANS_DONE: u32 = 1 << 1;
/// Camera vsync (frame boundary) — bit 2.
pub const INT_CAM_VSYNC: u32 = 1 << 2;
/// Camera hsync / line boundary — bit 3.
pub const INT_CAM_HS: u32 = 1 << 3;
/// Mask of all modeled interrupt bits.
const INT_ALL_BITS: u32 = INT_LCD_VSYNC | INT_LCD_TRANS_DONE | INT_CAM_VSYNC | INT_CAM_HS;

pub struct Esp32s3LcdCam {
    /// Interrupt-matrix source id (24).
    source_id: u32,

    // ── Configuration registers — pure round-trip storage ──
    lcd_clock: u32,
    cam_ctrl: u32,
    cam_rgb_yuv: u32,
    lcd_rgb_yuv: u32,
    lcd_dly_mode: u32,
    lcd_misc: u32,
    lcd_ctrl: u32,
    lcd_ctrl1: u32,
    lcd_ctrl2: u32,
    lcd_cmd_val: u32,
    lcd_dout_mode: u32,

    /// LCD_USER stored value with the self-clearing pulse bits stripped; the
    /// live LCD_START bit is reflected from `lcd_busy` on read.
    lcd_user: u32,
    /// CAM_CTRL1 stored value with pulse bits stripped; the live CAM_START bit
    /// is reflected from `cam_running` on read.
    cam_ctrl1: u32,

    // ── Interrupt state ──
    int_raw: u32,
    int_ena: u32,

    /// True between an LCD_START write and the tick that completes the
    /// transaction; reflected as LCD_USER.LCD_START on read.
    lcd_busy: bool,
    /// One-tick latch: a transaction was launched and must complete on the next
    /// tick (latch TRANS_DONE, clear `lcd_busy`). `Cell` so a read of LCD_USER
    /// never has to mutate — only `tick` (which takes `&mut self`) touches it.
    lcd_pending_done: bool,

    /// True while CAM_START is asserted; reflected as CAM_CTRL1.CAM_START.
    cam_running: bool,
    /// One-tick latch mirroring `lcd_pending_done` for the camera path.
    cam_pending_vsync: bool,

    /// True between an LCD_START that enabled the DOUT phase and the moment
    /// the payload source reports the transfer finished. While this and
    /// [`Self::dma_inflight`] are both set the transaction cannot complete.
    dout_pending: bool,
    /// True once GDMA has an outlink chain armed for this peripheral. Set by
    /// [`Self::dma_arm`] from the GDMA LCD pump, cleared by [`Self::dma_finish`].
    ///
    /// This is what separates "DOUT phase fed by DMA" (wait for the chain)
    /// from "DOUT phase with nothing behind it" (complete on the next tick, as
    /// the model always did). Without it a bus that has no GDMA — every
    /// LCD_CAM unit test, and any firmware that programs the phase bits
    /// without arming a channel — would hang.
    dma_inflight: bool,
    /// Odd trailing byte carried between [`Self::dma_push_tx`] bursts in
    /// 16-bit bus mode (a descriptor boundary can split a bus word).
    dma_half_word: Option<u8>,

    /// 8080 parallel panels this controller drives. Attached by the
    /// `ili9341-16bit` kit when the manifest declares one; empty otherwise
    /// (the register model then behaves exactly as before).
    panels: Vec<std::sync::Arc<crate::peripherals::components::ili9341_parallel::Ili9341Parallel>>,

    /// Bus-published cycle clock (walk-free deferred work).
    clock: Option<CycleClock>,
    /// Event armed for one-tick deferred work.
    scheduled: bool,
}

impl Esp32s3LcdCam {
    /// Construct the LCD_CAM controller. `source_id` is the interrupt-matrix
    /// source ([`LCD_CAM_INTR_SOURCE_ID`] = 24).
    pub fn new(source_id: u32) -> Self {
        Self {
            source_id,
            // Every modeled register comes out of reset all-zero per the TRM
            // reset column; seed explicitly for clarity.
            lcd_clock: 0,
            cam_ctrl: 0,
            cam_rgb_yuv: 0,
            lcd_rgb_yuv: 0,
            lcd_dly_mode: 0,
            lcd_misc: 0,
            lcd_ctrl: 0,
            lcd_ctrl1: 0,
            lcd_ctrl2: 0,
            lcd_cmd_val: 0,
            lcd_dout_mode: 0,
            lcd_user: 0,
            cam_ctrl1: 0,
            int_raw: 0,
            int_ena: 0,
            lcd_busy: false,
            lcd_pending_done: false,
            cam_running: false,
            cam_pending_vsync: false,
            dout_pending: false,
            dma_inflight: false,
            dma_half_word: None,
            panels: Vec::new(),
            clock: None,
            scheduled: false,
        }
    }

    /// Attach an 8080 parallel panel for the i80 pixel path. Called by the
    /// `ili9341-16bit` kit at manifest-attach time.
    pub fn attach_panel(
        &mut self,
        panel: std::sync::Arc<crate::peripherals::components::ili9341_parallel::Ili9341Parallel>,
    ) {
        self.panels.push(panel);
    }

    /// Number of panels bound to the i80 pixel path (attach evidence).
    pub fn panel_count(&self) -> usize {
        self.panels.len()
    }
}

impl Default for Esp32s3LcdCam {
    fn default() -> Self {
        Self::new(LCD_CAM_INTR_SOURCE_ID)
    }
}

impl std::fmt::Debug for Esp32s3LcdCam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Esp32s3LcdCam")
            .field("source_id", &self.source_id)
            .field("lcd_user", &format_args!("{:#010x}", self.lcd_user))
            .field("lcd_busy", &self.lcd_busy)
            .field("cam_running", &self.cam_running)
            .field("int_raw", &format_args!("{:#010x}", self.int_raw))
            .field("int_ena", &format_args!("{:#010x}", self.int_ena))
            .finish()
    }
}

impl Esp32s3LcdCam {
    /// Apply a write to LCD_USER: latch config bits, honor the self-clearing
    /// pulse bits (RESET / UPDATE accepted then stripped), and act on
    /// LCD_START.
    fn write_lcd_user(&mut self, value: u32) {
        // Store the value but strip START (reflected from lcd_busy on read) and
        // the self-clearing pulse bits. Done first so the phase / bus-format
        // bits this write carries are the ones the transaction below uses.
        self.lcd_user = value & !(LCD_START_BIT | LCD_USER_PULSE_BITS);

        if value & LCD_START_BIT != 0 {
            // Launch a transaction. Mark busy and arm the completion latch; a
            // later `tick` latches TRANS_DONE and clears START.
            self.lcd_busy = true;
            self.lcd_pending_done = true;
            self.dma_half_word = None;

            // Command phase: silicon drives LCD_CMD_VAL onto DB[15:0] with D/C
            // at the CMD level. One cycle, or two with LCD_CMD_2_CYCLE_EN (the
            // second cycle carries LCD_CMD_VAL[31:16]).
            if value & LCD_CMD_BIT != 0 {
                let dc = self.dc_level(MISC_CD_CMD_SET_BIT);
                let cycles = if value & LCD_CMD_2_CYCLE_BIT != 0 {
                    2
                } else {
                    1
                };
                for cycle in 0..cycles {
                    let half = (self.lcd_cmd_val >> (16 * cycle)) as u16;
                    let word = if self.bus_is_16bit() {
                        half
                    } else {
                        // 8-bit bus: only D[7:0] leave the chip. This is why
                        // `lcd_ll_set_command` spreads an 8-bit command as
                        // `cmd | (cmd_hi << 16)`.
                        half & 0x00FF
                    };
                    self.emit_word(dc, word);
                }
            }

            // Data phase: the payload is DMA-fed, so nothing is emitted here.
            // `dout_pending` holds TRANS_DONE back until the chain drains.
            self.dout_pending = value & LCD_DOUT_BIT != 0;
        }
    }

    /// True when LCD_USER.LCD_2BYTE_EN selects the 16-bit data bus.
    fn bus_is_16bit(&self) -> bool {
        self.lcd_user & LCD_2BYTE_EN_BIT != 0
    }

    /// D/C (RS) level driven during the phase whose `LCD_MISC.CD_*_SET` bit is
    /// `phase_set_bit`: `CD_IDLE_EDGE ^ CD_<phase>_SET`.
    fn dc_level(&self, phase_set_bit: u32) -> bool {
        let idle = self.lcd_misc & MISC_CD_IDLE_EDGE_BIT != 0;
        let differs = self.lcd_misc & phase_set_bit != 0;
        idle != differs
    }

    /// Apply the LCD_USER output-format bits to one bus word and hand it to
    /// every attached panel.
    fn emit_word(&self, dc_high: bool, word: u16) {
        if self.panels.is_empty() {
            return;
        }
        let mut w = word;
        if self.lcd_user & LCD_BYTE_ORDER_BIT != 0 && self.bus_is_16bit() {
            w = w.swap_bytes();
        }
        if self.lcd_user & LCD_BIT_ORDER_BIT != 0 {
            w = if self.bus_is_16bit() {
                w.reverse_bits()
            } else {
                u16::from((w as u8).reverse_bits())
            };
        }
        for panel in &self.panels {
            panel.i80_write_word(dc_high, w);
        }
    }

    // ── GDMA handoff (called by `Esp32s3Gdma`'s LCD pump) ──────────────────

    /// GDMA has an outlink chain armed for LCD_CAM. Until [`Self::dma_finish`]
    /// the DOUT phase of a transaction is considered DMA-fed and TRANS_DONE is
    /// withheld.
    pub fn dma_arm(&mut self) {
        self.dma_inflight = true;
    }

    /// True while a started transaction is waiting on its DMA payload — the
    /// GDMA pump's signal to fetch the next burst. False before LCD_START (the
    /// driver arms the descriptors first) and after the chain drains.
    pub fn dma_wants_data(&self) -> bool {
        self.lcd_busy && self.dout_pending
    }

    /// Push one burst of DOUT-phase payload bytes, as fetched from the GDMA
    /// outlink descriptor chain, onto the panel bus.
    ///
    /// Byte→word packing follows `LCD_USER.LCD_2BYTE_EN`: a 16-bit bus takes
    /// little-endian pairs (the order the DMA reads memory), an 8-bit bus one
    /// byte per cycle. `LCD_8BITS_ORDER` swaps adjacent bytes of the byte
    /// stream in 8-bit mode before packing.
    pub fn dma_push_tx(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let dc = self.dc_level(MISC_CD_DATA_SET_BIT);

        if !self.bus_is_16bit() {
            let swizzle = self.lcd_user & LCD_8BITS_ORDER_BIT != 0;
            if swizzle {
                for pair in bytes.chunks(2) {
                    for b in pair.iter().rev() {
                        self.emit_word(dc, u16::from(*b));
                    }
                }
            } else {
                for b in bytes {
                    self.emit_word(dc, u16::from(*b));
                }
            }
            return;
        }

        let mut it = bytes.iter().copied();
        if let Some(lo) = self.dma_half_word.take() {
            match it.next() {
                Some(hi) => self.emit_word(dc, u16::from_le_bytes([lo, hi])),
                None => {
                    self.dma_half_word = Some(lo);
                    return;
                }
            }
        }
        while let Some(lo) = it.next() {
            match it.next() {
                Some(hi) => self.emit_word(dc, u16::from_le_bytes([lo, hi])),
                // Odd tail: carry it to the next burst rather than inventing a
                // high byte.
                None => {
                    self.dma_half_word = Some(lo);
                    break;
                }
            }
        }
    }

    /// The outlink chain drained: the DOUT phase is complete, so the pending
    /// transaction may finish on the next tick.
    pub fn dma_finish(&mut self) {
        self.dout_pending = false;
        self.dma_inflight = false;
        self.dma_half_word = None;
    }

    /// Apply a write to CAM_CTRL1: latch config, self-clear CAM_RESET, act on
    /// CAM_START.
    fn write_cam_ctrl1(&mut self, value: u32) {
        if value & CAM_START_BIT != 0 {
            if !self.cam_running {
                // Rising edge: arm a one-tick vsync completion.
                self.cam_pending_vsync = true;
            }
            self.cam_running = true;
        } else {
            self.cam_running = false;
        }
        self.cam_ctrl1 = value & !(CAM_START_BIT | CAM_CTRL1_PULSE_BITS);
    }
}

impl Peripheral for Esp32s3LcdCam {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        // The esp-hal / ESP-IDF LCD_CAM drivers use 32-bit accesses
        // exclusively; stray byte reads return 0 harmlessly.
        Ok(0)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let v = match offset {
            REG_LCD_CLOCK => self.lcd_clock,
            REG_CAM_CTRL => self.cam_ctrl,
            REG_CAM_CTRL1 => self.cam_ctrl1 | if self.cam_running { CAM_START_BIT } else { 0 },
            REG_CAM_RGB_YUV => self.cam_rgb_yuv,
            REG_LCD_RGB_YUV => self.lcd_rgb_yuv,
            REG_LCD_DLY_MODE => self.lcd_dly_mode,
            // LCD_USER: stored config OR the live START (busy) bit. Pulse bits
            // were stripped on write so they read back 0.
            REG_LCD_USER => self.lcd_user | if self.lcd_busy { LCD_START_BIT } else { 0 },
            REG_LCD_MISC => self.lcd_misc,
            REG_LCD_CTRL => self.lcd_ctrl,
            REG_LCD_CTRL1 => self.lcd_ctrl1,
            REG_LCD_CTRL2 => self.lcd_ctrl2,
            REG_LCD_CMD_VAL => self.lcd_cmd_val,
            REG_LCD_DOUT_MODE => self.lcd_dout_mode,
            REG_LC_DMA_INT_ENA => self.int_ena,
            REG_LC_DMA_INT_RAW => self.int_raw,
            REG_LC_DMA_INT_ST => self.int_raw & self.int_ena,
            REG_LC_DMA_INT_CLR => 0, // W1C write-only; reads as 0.
            _ => {
                crate::census_reg!("esp32s3.lcd_cam:Esp32s3LcdCam", offset, "read");
                0
            }
        };
        Ok(v)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // Byte writes ignored — driver writes whole words.
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset {
            REG_LCD_CLOCK => self.lcd_clock = value,
            REG_CAM_CTRL => self.cam_ctrl = value,
            REG_CAM_CTRL1 => self.write_cam_ctrl1(value),
            REG_CAM_RGB_YUV => self.cam_rgb_yuv = value,
            REG_LCD_RGB_YUV => self.lcd_rgb_yuv = value,
            REG_LCD_DLY_MODE => self.lcd_dly_mode = value,
            REG_LCD_USER => self.write_lcd_user(value),
            REG_LCD_MISC => self.lcd_misc = value,
            REG_LCD_CTRL => self.lcd_ctrl = value,
            REG_LCD_CTRL1 => self.lcd_ctrl1 = value,
            REG_LCD_CTRL2 => self.lcd_ctrl2 = value,
            REG_LCD_CMD_VAL => self.lcd_cmd_val = value,
            REG_LCD_DOUT_MODE => self.lcd_dout_mode = value,
            REG_LC_DMA_INT_ENA => self.int_ena = value & INT_ALL_BITS,
            // INT_RAW is read-only on hardware; the driver never writes it, but
            // accept writes (masked) so test fixtures can seed raw bits.
            REG_LC_DMA_INT_RAW => self.int_raw = value & INT_ALL_BITS,
            REG_LC_DMA_INT_CLR => self.int_raw &= !value, // W1C
            _ => {
                crate::census_reg!("esp32s3.lcd_cam:Esp32s3LcdCam", offset, "write");
            } // Accept-and-ignore other offsets.
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Complete any launched LCD transaction: latch TRANS_DONE and clear the
        // START/busy flag so polling firmware proceeds. A single tick is enough
        // (the sim has no per-pixel timing model) — EXCEPT while the DOUT phase
        // is still being fed from a GDMA outlink chain. Completing early there
        // would let `panel_io_i80_tx_param`'s TRANS_DONE poll return while the
        // descriptors are still in flight, and the driver would remount them
        // under the running transfer.
        if self.lcd_pending_done && !(self.dout_pending && self.dma_inflight) {
            self.lcd_pending_done = false;
            self.lcd_busy = false;
            self.int_raw |= INT_LCD_TRANS_DONE;
        }

        // Complete a camera frame: latch CAM_VSYNC at the frame boundary.
        // CAM_START stays asserted (it is a run/level bit on silicon).
        if self.cam_pending_vsync {
            self.cam_pending_vsync = false;
            self.int_raw |= INT_CAM_VSYNC;
        }

        // Level-triggered IRQ delivery: emit our matrix source while any
        // enabled raw bit is set (same model as the timer-group / I2S twins).
        let explicit = if self.int_raw & self.int_ena != 0 {
            Some(vec![self.source_id])
        } else {
            None
        };

        PeripheralTickResult {
            explicit_irqs: explicit,
            ..Default::default()
        }
    }

    fn uses_scheduler(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    fn needs_legacy_walk(&self) -> bool {
        !self.uses_scheduler()
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        if self.int_raw & self.int_ena != 0 {
            out.push(self.source_id);
        }
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        if !self.uses_scheduler() {
            return Vec::new();
        }
        // Parenthesised: without the grouping `lcd_pending_done` alone armed a
        // fresh event on every call, even when one was already scheduled.
        if (self.lcd_pending_done || self.cam_pending_vsync) && !self.scheduled {
            self.scheduled = true;
            return vec![(1, DEFERRED_WAKE_TOKEN)];
        }
        Vec::new()
    }

    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        // Same one-tick transition the legacy walk performed.
        let _ = self.tick();
        self.scheduled = false;
        let mut explicit_irqs = Vec::new();
        self.matrix_irq_sources_into(&mut explicit_irqs);
        let reschedule = if self.lcd_pending_done || self.cam_pending_vsync {
            self.scheduled = true;
            Some(1)
        } else {
            None
        };
        crate::sched::EventResult {
            explicit_irqs,
            reschedule_delay: reschedule,
            ..Default::default()
        }
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
    use super::*;

    fn new_lcd_cam() -> Esp32s3LcdCam {
        Esp32s3LcdCam::new(LCD_CAM_INTR_SOURCE_ID)
    }

    #[test]
    fn source_id_is_24() {
        assert_eq!(LCD_CAM_INTR_SOURCE_ID, 24);
        let p = new_lcd_cam();
        assert_eq!(p.source_id, 24);
    }

    #[test]
    fn config_registers_round_trip() {
        let mut p = new_lcd_cam();
        p.write_u32(REG_LCD_CLOCK, 0x1234_5678).unwrap();
        p.write_u32(REG_CAM_CTRL, 0x0BAD_F00D).unwrap();
        p.write_u32(REG_CAM_RGB_YUV, 0xA5A5_A5A5).unwrap();
        p.write_u32(REG_LCD_MISC, 0xDEAD_BEEF).unwrap();
        p.write_u32(REG_LCD_CTRL, 0x0000_FFFF).unwrap();
        p.write_u32(REG_LCD_CTRL1, 0x1111_2222).unwrap();
        p.write_u32(REG_LCD_CTRL2, 0x3333_4444).unwrap();
        p.write_u32(REG_LCD_CMD_VAL, 0x0000_002C).unwrap();
        p.write_u32(REG_LCD_DOUT_MODE, 0x0000_0001).unwrap();

        assert_eq!(p.read_u32(REG_LCD_CLOCK).unwrap(), 0x1234_5678);
        assert_eq!(p.read_u32(REG_CAM_CTRL).unwrap(), 0x0BAD_F00D);
        assert_eq!(p.read_u32(REG_CAM_RGB_YUV).unwrap(), 0xA5A5_A5A5);
        assert_eq!(p.read_u32(REG_LCD_MISC).unwrap(), 0xDEAD_BEEF);
        assert_eq!(p.read_u32(REG_LCD_CTRL).unwrap(), 0x0000_FFFF);
        assert_eq!(p.read_u32(REG_LCD_CTRL1).unwrap(), 0x1111_2222);
        assert_eq!(p.read_u32(REG_LCD_CTRL2).unwrap(), 0x3333_4444);
        assert_eq!(p.read_u32(REG_LCD_CMD_VAL).unwrap(), 0x0000_002C);
        assert_eq!(p.read_u32(REG_LCD_DOUT_MODE).unwrap(), 0x0000_0001);
    }

    #[test]
    fn reset_defaults_are_zero() {
        let p = new_lcd_cam();
        for off in [
            REG_LCD_CLOCK,
            REG_CAM_CTRL,
            REG_CAM_CTRL1,
            REG_CAM_RGB_YUV,
            REG_LCD_USER,
            REG_LCD_MISC,
            REG_LCD_CTRL,
            REG_LCD_CTRL1,
            REG_LCD_CTRL2,
            REG_LCD_CMD_VAL,
            REG_LCD_DOUT_MODE,
            REG_LC_DMA_INT_ENA,
            REG_LC_DMA_INT_RAW,
            REG_LC_DMA_INT_ST,
        ] {
            assert_eq!(p.read_u32(off).unwrap(), 0, "offset {off:#x} not zero");
        }
    }

    #[test]
    fn lcd_user_config_bits_round_trip_minus_pulse_and_start() {
        let mut p = new_lcd_cam();
        // Command + dummy + dout phases enabled, plus a pulse (UPDATE) and
        // START. The phase-enable bits persist; START reflects busy; UPDATE
        // self-clears.
        let v = LCD_CMD_BIT | LCD_DUMMY_BIT | LCD_DOUT_BIT | LCD_UPDATE_BIT;
        p.write_u32(REG_LCD_USER, v).unwrap();
        // No START in this write → not busy → UPDATE stripped → only phase bits.
        assert_eq!(
            p.read_u32(REG_LCD_USER).unwrap(),
            LCD_CMD_BIT | LCD_DUMMY_BIT | LCD_DOUT_BIT
        );
    }

    #[test]
    fn lcd_start_triggers_trans_done_on_tick_and_self_clears() {
        let mut p = new_lcd_cam();
        // Launch a transaction with the command + dout phases.
        p.write_u32(REG_LCD_USER, LCD_START_BIT | LCD_CMD_BIT | LCD_DOUT_BIT)
            .unwrap();
        // START reads back set while busy; TRANS_DONE not latched yet.
        assert_eq!(
            p.read_u32(REG_LCD_USER).unwrap() & LCD_START_BIT,
            LCD_START_BIT,
            "LCD_START asserted while busy"
        );
        assert_eq!(
            p.read_u32(REG_LC_DMA_INT_RAW).unwrap() & INT_LCD_TRANS_DONE,
            0,
            "TRANS_DONE not latched before tick"
        );

        // One tick completes the transaction.
        p.tick();
        assert_eq!(
            p.read_u32(REG_LC_DMA_INT_RAW).unwrap() & INT_LCD_TRANS_DONE,
            INT_LCD_TRANS_DONE,
            "TRANS_DONE latched after tick"
        );
        assert_eq!(
            p.read_u32(REG_LCD_USER).unwrap() & LCD_START_BIT,
            0,
            "LCD_START auto-cleared on completion"
        );
        // The phase-enable bits the driver programmed survive completion.
        assert_eq!(
            p.read_u32(REG_LCD_USER).unwrap() & (LCD_CMD_BIT | LCD_DOUT_BIT),
            LCD_CMD_BIT | LCD_DOUT_BIT
        );
    }

    #[test]
    fn trans_done_is_write_one_to_clear() {
        let mut p = new_lcd_cam();
        p.write_u32(REG_LCD_USER, LCD_START_BIT).unwrap();
        p.tick();
        assert_eq!(
            p.read_u32(REG_LC_DMA_INT_RAW).unwrap() & INT_LCD_TRANS_DONE,
            INT_LCD_TRANS_DONE
        );
        // Writing 0 to the bit must NOT clear it.
        p.write_u32(REG_LC_DMA_INT_CLR, 0).unwrap();
        assert_eq!(
            p.read_u32(REG_LC_DMA_INT_RAW).unwrap() & INT_LCD_TRANS_DONE,
            INT_LCD_TRANS_DONE,
            "W1C: writing 0 does not clear"
        );
        // Writing 1 clears.
        p.write_u32(REG_LC_DMA_INT_CLR, INT_LCD_TRANS_DONE).unwrap();
        assert_eq!(
            p.read_u32(REG_LC_DMA_INT_RAW).unwrap() & INT_LCD_TRANS_DONE,
            0,
            "W1C: writing 1 clears"
        );
        // INT_CLR reads back as 0.
        assert_eq!(p.read_u32(REG_LC_DMA_INT_CLR).unwrap(), 0);
    }

    #[test]
    fn int_clr_only_clears_targeted_bits() {
        let mut p = new_lcd_cam();
        p.write_u32(
            REG_LC_DMA_INT_RAW,
            INT_LCD_TRANS_DONE | INT_CAM_VSYNC | INT_LCD_VSYNC,
        )
        .unwrap();
        p.write_u32(REG_LC_DMA_INT_CLR, INT_CAM_VSYNC).unwrap();
        assert_eq!(
            p.read_u32(REG_LC_DMA_INT_RAW).unwrap(),
            INT_LCD_TRANS_DONE | INT_LCD_VSYNC,
            "only CAM_VSYNC cleared"
        );
    }

    #[test]
    fn interrupt_only_emitted_when_enabled() {
        let mut p = new_lcd_cam();
        // Launch + complete a transaction → TRANS_DONE raw set, but INT_ENA = 0.
        p.write_u32(REG_LCD_USER, LCD_START_BIT).unwrap();
        let r = p.tick();
        assert!(
            r.explicit_irqs.is_none(),
            "no IRQ while TRANS_DONE disabled"
        );
        assert_eq!(
            p.read_u32(REG_LC_DMA_INT_RAW).unwrap() & INT_LCD_TRANS_DONE,
            INT_LCD_TRANS_DONE,
            "raw still latched even with enable off"
        );
        assert_eq!(p.read_u32(REG_LC_DMA_INT_ST).unwrap(), 0, "INT_ST masked");

        // Enable TRANS_DONE → source emitted.
        p.write_u32(REG_LC_DMA_INT_ENA, INT_LCD_TRANS_DONE).unwrap();
        assert_eq!(
            p.tick().explicit_irqs.as_deref(),
            Some(&[LCD_CAM_INTR_SOURCE_ID][..])
        );
        // Level-triggered: re-asserts while INT_ST != 0.
        assert_eq!(
            p.tick().explicit_irqs.as_deref(),
            Some(&[LCD_CAM_INTR_SOURCE_ID][..])
        );

        // Clear raw → emission stops.
        p.write_u32(REG_LC_DMA_INT_CLR, INT_LCD_TRANS_DONE).unwrap();
        assert!(p.tick().explicit_irqs.is_none());
    }

    #[test]
    fn int_st_masks_with_int_ena() {
        let mut p = new_lcd_cam();
        p.write_u32(REG_LC_DMA_INT_RAW, INT_LCD_TRANS_DONE | INT_CAM_VSYNC)
            .unwrap();
        p.write_u32(REG_LC_DMA_INT_ENA, INT_LCD_TRANS_DONE).unwrap();
        assert_eq!(p.read_u32(REG_LC_DMA_INT_ST).unwrap(), INT_LCD_TRANS_DONE);
    }

    #[test]
    fn cam_start_sets_running_and_latches_cam_vsync() {
        let mut p = new_lcd_cam();
        p.write_u32(REG_CAM_CTRL1, CAM_START_BIT).unwrap();
        // CAM_START reads back through CAM_CTRL1 (run/level bit, stays set).
        assert_eq!(
            p.read_u32(REG_CAM_CTRL1).unwrap() & CAM_START_BIT,
            CAM_START_BIT
        );
        // Vsync not latched until a tick advances the frame.
        assert_eq!(p.read_u32(REG_LC_DMA_INT_RAW).unwrap() & INT_CAM_VSYNC, 0);
        p.tick();
        assert_eq!(
            p.read_u32(REG_LC_DMA_INT_RAW).unwrap() & INT_CAM_VSYNC,
            INT_CAM_VSYNC,
            "CAM_VSYNC latched after tick"
        );
        // CAM_START remains asserted (driver clears it explicitly to stop).
        assert_eq!(
            p.read_u32(REG_CAM_CTRL1).unwrap() & CAM_START_BIT,
            CAM_START_BIT
        );
    }

    #[test]
    fn clearing_cam_start_stops_running() {
        let mut p = new_lcd_cam();
        p.write_u32(REG_CAM_CTRL1, CAM_START_BIT).unwrap();
        assert_eq!(
            p.read_u32(REG_CAM_CTRL1).unwrap() & CAM_START_BIT,
            CAM_START_BIT
        );
        p.write_u32(REG_CAM_CTRL1, 0).unwrap();
        assert_eq!(p.read_u32(REG_CAM_CTRL1).unwrap() & CAM_START_BIT, 0);
    }

    #[test]
    fn cam_reset_pulse_self_clears() {
        let mut p = new_lcd_cam();
        // CAM_RESET write-pulse must not persist in the readback.
        p.write_u32(REG_CAM_CTRL1, CAM_RESET_BIT).unwrap();
        assert_eq!(p.read_u32(REG_CAM_CTRL1).unwrap() & CAM_RESET_BIT, 0);
    }

    #[test]
    fn int_ena_masks_to_modeled_bits() {
        let mut p = new_lcd_cam();
        // Bits outside the modeled set are dropped on write.
        p.write_u32(REG_LC_DMA_INT_ENA, 0xFFFF_FFFF).unwrap();
        assert_eq!(p.read_u32(REG_LC_DMA_INT_ENA).unwrap(), INT_ALL_BITS);
    }

    #[test]
    fn cam_vsync_irq_emitted_when_enabled() {
        let mut p = new_lcd_cam();
        p.write_u32(REG_LC_DMA_INT_ENA, INT_CAM_VSYNC).unwrap();
        p.write_u32(REG_CAM_CTRL1, CAM_START_BIT).unwrap();
        assert_eq!(
            p.tick().explicit_irqs.as_deref(),
            Some(&[LCD_CAM_INTR_SOURCE_ID][..]),
            "CAM_VSYNC drives the shared source 24"
        );
    }

    #[test]
    fn back_to_back_lcd_transactions() {
        let mut p = new_lcd_cam();
        // First transaction.
        p.write_u32(REG_LCD_USER, LCD_START_BIT).unwrap();
        p.tick();
        assert_eq!(p.read_u32(REG_LCD_USER).unwrap() & LCD_START_BIT, 0);
        p.write_u32(REG_LC_DMA_INT_CLR, INT_LCD_TRANS_DONE).unwrap();
        // Second transaction re-arms and completes again.
        p.write_u32(REG_LCD_USER, LCD_START_BIT).unwrap();
        assert_eq!(
            p.read_u32(REG_LCD_USER).unwrap() & LCD_START_BIT,
            LCD_START_BIT
        );
        p.tick();
        assert_eq!(
            p.read_u32(REG_LC_DMA_INT_RAW).unwrap() & INT_LCD_TRANS_DONE,
            INT_LCD_TRANS_DONE
        );
        assert_eq!(p.read_u32(REG_LCD_USER).unwrap() & LCD_START_BIT, 0);
    }

    #[test]
    fn idle_tick_emits_nothing() {
        let mut p = new_lcd_cam();
        assert!(p.tick().explicit_irqs.is_none());
        // No raw bits, no busy/running state.
        assert_eq!(p.read_u32(REG_LC_DMA_INT_RAW).unwrap(), 0);
    }

    #[test]
    fn unmapped_offsets_read_zero_and_accept_writes() {
        let mut p = new_lcd_cam();
        p.write_u32(0xFFC, 0xDEAD_BEEF).unwrap();
        assert_eq!(p.read_u32(0xFFC).unwrap(), 0);
    }

    #[test]
    fn byte_access_is_inert() {
        let mut p = new_lcd_cam();
        // Byte writes ignored; byte reads return 0 (driver uses word access).
        p.write(REG_LCD_CLOCK, 0xAB).unwrap();
        assert_eq!(p.read(REG_LCD_CLOCK).unwrap(), 0);
        assert_eq!(p.read_u32(REG_LCD_CLOCK).unwrap(), 0);
    }
}
