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
//! Pixel/line streaming on the S3 flows through GDMA (the LCD_CAM core has no
//! CPU-visible pixel FIFO register) and is therefore **out of scope** — we model
//! the MMIO control surface and the transaction-done event model, not the
//! DMA-fed pixel clock.
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
//! | 0x14   | LCD_USER         | LCD_START=b27, LCD_CMD=b26, LCD_DUMMY=b25, LCD_DOUT=b24, resets |
//! | 0x18   | LCD_MISC        | LCD bus/CS timing, idle-level, AFIFO reset         |
//! | 0x1C   | LCD_CTRL        | LCD RGB mode, h/v sync + de-output enables         |
//! | 0x20   | LCD_CTRL1        | LCD RGB H/V front/back-porch + sync widths         |
//! | 0x24   | LCD_CTRL2        | LCD sync pulse widths / polarity                   |
//! | 0x28   | LCD_CMD_VAL      | LCD command value driven during the command phase  |
//! | 0x2C   | LCD_DOUT_MODE    | LCD data-out bit-order / dly mode                  |
//! | 0x34   | LC_DMA_INT_ENA   | interrupt enable mask                              |
//! | 0x38   | LC_DMA_INT_RAW   | raw latched events (RO here)                       |
//! | 0x3C   | LC_DMA_INT_ST    | INT_RAW & INT_ENA (RO)                             |
//! | 0x40   | LC_DMA_INT_CLR   | W1C against INT_RAW                                |
//!
//! Any other offset accepts writes silently and reads 0.

use crate::{Peripheral, PeripheralTickResult, SimResult};

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
const REG_LCD_DOUT_MODE: u64 = 0x2C;
const REG_LC_DMA_INT_ENA: u64 = 0x34;
const REG_LC_DMA_INT_RAW: u64 = 0x38;
const REG_LC_DMA_INT_ST: u64 = 0x3C;
const REG_LC_DMA_INT_CLR: u64 = 0x40;

// ── LCD_USER bits (TRM "LCD and Camera Controller"; soc/lcd_cam_reg.h) ──
// The phase-enable bits are round-tripped as plain config and exercised by the
// tests, but the transaction-done model does not branch on them (a single tick
// completes whatever phases were programmed), so mark them allow(dead_code) for
// the non-test build — matching the timer_group documentation-constant idiom.
/// LCD_DOUT — enable the data-out phase.
#[allow(dead_code)]
const LCD_DOUT_BIT: u32 = 1 << 24;
/// LCD_DUMMY — enable the dummy phase.
#[allow(dead_code)]
const LCD_DUMMY_BIT: u32 = 1 << 25;
/// LCD_CMD — enable the command phase.
#[allow(dead_code)]
const LCD_CMD_BIT: u32 = 1 << 26;
/// LCD_START — launch the configured transaction. Self-clears on completion.
const LCD_START_BIT: u32 = 1 << 27;
/// LCD_RESET — write-pulse reset of the LCD module. Self-clears.
const LCD_RESET_BIT: u32 = 1 << 30;
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
/// LCD RGB-mode vertical-sync edge — bit 0.
pub const INT_LCD_VSYNC: u32 = 1 << 0;
/// LCD transaction finished (command/dummy/dout phases done) — bit 2.
pub const INT_LCD_TRANS_DONE: u32 = 1 << 2;
/// Camera vsync (frame boundary) — bit 4.
pub const INT_CAM_VSYNC: u32 = 1 << 4;
/// Camera hsync / line boundary — bit 6.
pub const INT_CAM_HS: u32 = 1 << 6;
/// Mask of all modeled interrupt bits.
const INT_ALL_BITS: u32 = INT_LCD_VSYNC | INT_LCD_TRANS_DONE | INT_CAM_VSYNC | INT_CAM_HS;

pub struct Esp32s3LcdCam {
    /// Interrupt-matrix source id (24).
    source_id: u32,

    // ── Configuration registers — pure round-trip storage ──
    lcd_clock: u32,
    cam_ctrl: u32,
    cam_rgb_yuv: u32,
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
        }
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
        if value & LCD_START_BIT != 0 {
            // Launch a transaction. Mark busy and arm the one-tick completion
            // latch; the next `tick` latches TRANS_DONE and clears START.
            self.lcd_busy = true;
            self.lcd_pending_done = true;
        }
        // Store the value but strip START (reflected from lcd_busy on read) and
        // the self-clearing pulse bits.
        self.lcd_user = value & !(LCD_START_BIT | LCD_USER_PULSE_BITS);
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
            _ => 0,
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
            _ => {}                                       // Accept-and-ignore other offsets.
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Complete any launched LCD transaction: latch TRANS_DONE and clear the
        // START/busy flag so polling firmware proceeds. A single tick is enough
        // (the sim has no per-pixel timing model).
        if self.lcd_pending_done {
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
