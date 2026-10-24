// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! RTC_IO (RTC IO MUX + RTC GPIO) of the classic ESP32 and the ESP32-S3.
//!
//! The RTC-capable pads have a second owner besides the digital IO_MUX / GPIO
//! matrix: the RTC IO MUX. A `MUX_SEL` bit in the pad's RTC_IO register picks
//! which side controls the pad (classic TRM v5.8 Register 6.53
//! `RTCIO_TOUCH_PADn_MUX_SEL`: "Selects RTC IO_MUX or IO_MUX to control the
//! IE/OE/RUE/RDE statuses of RTC pin. 1: Selects RTC IO_MUX 0: Selects
//! IO_MUX"; S3 TRM v1.8 §6.7.2: with `MUX_SEL` = 1 "input/output signals to
//! and from that pin is routed to the RTC subsystem"). With `MUX_SEL` = 1 the
//! pad's output stage is the RTC GPIO's: `RTC_GPIO_OUT` / `RTC_GPIO_ENABLE`
//! (+ `_W1TS` / `_W1TC`), `RTC_GPIO_PINn.PAD_DRIVER` (open drain), the pad
//! register's `RUE` / `RDE` pulls and `FUN_IE` input enable; the GPIO matrix,
//! `GPIO_OUT` and `GPIO_ENABLE` no longer reach it.
//!
//! # Numbering
//!
//! RTC GPIO numbers are NOT GPIO numbers on the classic part (esp-idf
//! `soc/esp32/rtc_io_periph.c`, `rtc_io_desc`): RTC_GPIO0..3 = GPIO36..39,
//! 4/5 = GPIO34/35, 6/7 = GPIO25/26, 8 = GPIO33 (X32N), 9 = GPIO32 (X32P),
//! 10..17 = GPIO4, 0, 2, 15, 13, 12, 14, 27 (TOUCH_PAD0..7). On the S3 they
//! are the same numbers: RTC_GPIOn = GPIOn, n = 0..21 (`soc/esp32s3/
//! rtc_io_periph.c`). RTC GPIO n sits at bit `14 + n` of `RTC_GPIO_OUT` /
//! `ENABLE` / `STATUS` / `IN` on the classic part and at bit `10 + n` on the
//! S3 (SVD field `bitOffset`, esp-idf `rtc_io_struct.h` `reserved0:14` /
//! `reserved0:10`).
//!
//! # What the digital side still sees
//!
//! The pad's input buffer has one output. The S3 TRM's Figure 6.3-1 draws the
//! input ("C") of a VDD3P3_RTC pad going both up into the GPIO filter / GPIO
//! SYNC (the digital matrix and `GPIO_IN`) and down to the RTC IO MUX, with no
//! mux on that path; only the pad's OE / output / IE / pull controls pass
//! through the `MUX_SEL` mux. The classic TRM §6.5.2 says the same for
//! RTC_GPIO8..17: with `TO_GPIO` = 0 (its reset value) "the input signal from
//! the pin is output to IO MUX through digital function". So this model keeps
//! `GPIO_IN` and the matrix inputs reading an RTC-mode pad, through the input
//! enable `MUX_SEL` selects (the RTC `FUN_IE`): `FUN_IE` = 0 reads 0 on both
//! sides.
//!
//! # Pulls
//!
//! On the classic ESP32 the pull-up / pull-down of a pad that has an RTC
//! function is set by the RTC_IO `RUE` / `RDE` bits whatever `MUX_SEL` says
//! (ESP32 errata 3.6; esp-idf `soc/esp32/include/soc/soc_caps.h`: "On ESP32
//! those PADs which have RTC functions must set pullup/down/capability via RTC
//! register", and `gpio_pullup_en` calls `rtc_gpio_pullup_en` for them;
//! esp-hal `soc/esp32/gpio.rs` `errata36`). The S3 has separate digital pulls
//! (`SOC_GPIO_SUPPORT_RTC_INDEPENDENT` = 1): its `RUE` / `RDE` act only with
//! `MUX_SEL` = 1. GPIO34..39 (classic RTC_GPIO0..5) have no pulls and no
//! output stage in either domain.
//!
//! # Interrupts
//!
//! `RTC_GPIO_PINn.INT_TYPE` latches `RTC_GPIO_STATUS` (0 off, 1 rising, 2
//! falling, 3 any edge, 4 low level, 5 high level; TRM Register 6.45 /
//! S3 §6.15.4), cleared through `RTC_GPIO_STATUS_W1TC`. Neither part routes
//! that latch to a CPU interrupt: the RTC_CORE interrupt-matrix source (46 on
//! the classic part, 39 on the S3) is `RTC_CNTL_INT_ST`, and
//! `RTC_CNTL_INT_ENA_REG` has no RTC GPIO bit on either part (esp-idf
//! `soc/esp32/include/soc/rtc_cntl_reg.h` bits 0..8,
//! `soc/esp32s3/include/soc/rtc_cntl_reg.h` bits 0..20); the RTC GPIO latch
//! is a wake-up source (`WAKEUP_ENABLE`) and a status the ULP / CPU polls.
//! So this model raises no interrupt source; firmware polls `STATUS`.
//!
//! # HOLD
//!
//! The classic pad registers carry a `HOLD` bit (`RTCIO_*_HOLD`, "keep the
//! pad's current state"). While it is set the RTC-domain output stage of the
//! pad (OE, output level, open drain, pulls) stays as it was when `HOLD` rose,
//! whatever firmware writes. The S3 keeps its pad holds in RTC_CNTL
//! (`RTC_CNTL_PAD_HOLD_REG`), outside this block, and they are not modelled;
//! neither is the classic `DIG_PAD_HOLD` / `RTC_CNTL_HOLD_FORCE` path, nor a
//! hold on a pad in the digital domain.
//!
//! # Sharing
//!
//! The register state lives in [`RtcIoPads`], one `Arc` shared by the MMIO
//! model ([`EspRtcIo`]) and the chip's GPIO model, which the bus hands it at
//! attach time (`Peripheral::rtc_io_pads` / `attach_rtc_io_pads`). The GPIO
//! model stays the pads' only [`PinPort`](crate::pins::PinPort) and asks the
//! shared [`RtcPadView`] which domain owns each pad; the bus brackets every
//! RTC_IO write with the GPIO port's push capture
//! (`SystemBus::begin_rtc_io_write`), so a probe and a world `gpio_net` see a
//! pad the RTC side moves. The GPIO model in turn publishes what the outside
//! presents to the pads ([`PadInputs`]), from which `RTC_GPIO_IN` and the
//! status latch are derived.

use crate::pins::{resolve, External, PadDriver, Pull};
use crate::{Peripheral, SimResult};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// `RTC_GPIO_OUT_REG` and its write-1-to-set / -clear views.
const OUT: u64 = 0x00;
const OUT_W1TS: u64 = 0x04;
const OUT_W1TC: u64 = 0x08;
/// `RTC_GPIO_ENABLE_REG` and views.
const ENABLE: u64 = 0x0C;
const ENABLE_W1TS: u64 = 0x10;
const ENABLE_W1TC: u64 = 0x14;
/// `RTC_GPIO_STATUS_REG` and views.
const STATUS: u64 = 0x18;
const STATUS_W1TS: u64 = 0x1C;
const STATUS_W1TC: u64 = 0x20;
/// `RTC_GPIO_IN_REG` (read only).
const IN: u64 = 0x24;
/// `RTC_GPIO_PIN0_REG`, stride 4: PAD_DRIVER (2), INT_TYPE [9:7],
/// WAKEUP_ENABLE (10) on both parts.
const PIN0: u64 = 0x28;

/// Which part a layout describes. A GPIO model only takes the RTC pads of
/// its own part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtcChip {
    Esp32,
    Esp32s3,
}

/// One RTC-capable pad: its GPIO number and where its controls sit in its
/// pad register. The pad's RTC GPIO number is its index in the layout.
#[derive(Debug, Clone, Copy)]
struct RtcPad {
    gpio: u8,
    /// Offset of the pad register (`TOUCH_PADn`, `XTAL_32K_PAD`, ...).
    reg: u64,
    fun_ie: u8,
    mux_sel: u8,
    rue: Option<u8>,
    rde: Option<u8>,
    hold: Option<u8>,
}

const fn pad(gpio: u8, reg: u64, fun_ie: u8, mux_sel: u8) -> RtcPad {
    RtcPad {
        gpio,
        reg,
        fun_ie,
        mux_sel,
        rue: None,
        rde: None,
        hold: None,
    }
}

const fn pulled(
    gpio: u8,
    reg: u64,
    fun_ie: u8,
    mux_sel: u8,
    rue: u8,
    rde: u8,
    hold: Option<u8>,
) -> RtcPad {
    RtcPad {
        gpio,
        reg,
        fun_ie,
        mux_sel,
        rue: Some(rue),
        rde: Some(rde),
        hold,
    }
}

/// Classic ESP32 RTC_GPIO0..17 in RTC GPIO order (esp-idf
/// `soc/esp32/rtc_io_periph.c` `rtc_io_desc`; bit positions from
/// `tests/fixtures/real_world/esp32.svd` and the `esp32` PAC). SENSE1..4 and
/// ADC1/2 (GPIO36..39, 34, 35) have no pulls and no output stage.
#[rustfmt::skip]
const ESP32_PADS: [RtcPad; 18] = [
    // SENSOR_PADS (0x7C): SENSEn_FUN_IE 19/14/9/4, MUX_SEL 27..24, HOLD 31..28.
    RtcPad { hold: Some(31), ..pad(36, 0x7C, 19, 27) },
    RtcPad { hold: Some(30), ..pad(37, 0x7C, 14, 26) },
    RtcPad { hold: Some(29), ..pad(38, 0x7C, 9, 25) },
    RtcPad { hold: Some(28), ..pad(39, 0x7C, 4, 24) },
    // ADC_PAD (0x80): ADC1 FUN_IE 23 MUX_SEL 29 HOLD 31; ADC2 18 / 28 / 30.
    RtcPad { hold: Some(31), ..pad(34, 0x80, 23, 29) },
    RtcPad { hold: Some(30), ..pad(35, 0x80, 18, 28) },
    // PAD_DAC1 / PAD_DAC2 (0x84 / 0x88): FUN_IE 11, MUX_SEL 17, RUE 27,
    // RDE 28, HOLD 29.
    pulled(25, 0x84, 11, 17, 27, 28, Some(29)),
    pulled(26, 0x88, 11, 17, 27, 28, Some(29)),
    // XTAL_32K_PAD (0x8C): X32N FUN_IE 11 MUX_SEL 18 RUE 27 RDE 28 HOLD 29;
    // X32P FUN_IE 5 MUX_SEL 17 RUE 22 RDE 23 HOLD 24.
    pulled(33, 0x8C, 11, 18, 27, 28, Some(29)),
    pulled(32, 0x8C, 5, 17, 22, 23, Some(24)),
    // TOUCH_PAD0..7 (0x94 + 4n): FUN_IE 13, MUX_SEL 19, RUE 27, RDE 28,
    // HOLD 31.
    pulled(4, 0x94, 13, 19, 27, 28, Some(31)),
    pulled(0, 0x98, 13, 19, 27, 28, Some(31)),
    pulled(2, 0x9C, 13, 19, 27, 28, Some(31)),
    pulled(15, 0xA0, 13, 19, 27, 28, Some(31)),
    pulled(13, 0xA4, 13, 19, 27, 28, Some(31)),
    pulled(12, 0xA8, 13, 19, 27, 28, Some(31)),
    pulled(14, 0xAC, 13, 19, 27, 28, Some(31)),
    pulled(27, 0xB0, 13, 19, 27, 28, Some(31)),
];

/// ESP32-S3 RTC_GPIO0..21 = GPIO0..21 (esp-idf `soc/esp32s3/rtc_io_periph.c`):
/// TOUCH_PAD0..14 (0x84 + 4n), XTAL_32P / XTAL_32N (0xC0 / 0xC4), PAD_DAC1 /
/// PAD_DAC2 (0xC8 / 0xCC), RTC_PAD19..21 (0xD0 + 4n). Every one: FUN_IE 13,
/// MUX_SEL 19, RUE 27, RDE 28 (`tests/fixtures/svd/esp32s3.svd`); no HOLD
/// bit in this block.
const ESP32S3_PADS: [RtcPad; 22] = {
    let mut pads = [pulled(0, 0, 13, 19, 27, 28, None); 22];
    let mut n = 0;
    while n < 22 {
        let reg = match n {
            0..=14 => 0x84 + 4 * n as u64,
            15 => 0xC0,
            16 => 0xC4,
            17 => 0xC8,
            18 => 0xCC,
            _ => 0xD0 + 4 * (n as u64 - 19),
        };
        pads[n] = pulled(n as u8, reg, 13, 19, 27, 28, None);
        n += 1;
    }
    pads
};

/// `(offset, reset, writable bits)` of every architected classic RTC_IO
/// register, from `tests/fixtures/real_world/esp32.svd` (a register with no
/// `resetValue` takes the device default 0; the writable bits are its fields
/// that are not read-only). Pinned against the SVD by
/// `tests::registers_match_the_svd`.
#[rustfmt::skip]
const ESP32_REGS: &[(u64, u32, u32)] = &[
    (0x000, 0x0000_0000, 0xFFFF_C000), // OUT
    (0x004, 0x0000_0000, 0xFFFF_C000), // OUT_W1TS
    (0x008, 0x0000_0000, 0xFFFF_C000), // OUT_W1TC
    (0x00C, 0x0000_0000, 0xFFFF_C000), // ENABLE
    (0x010, 0x0000_0000, 0xFFFF_C000), // ENABLE_W1TS
    (0x014, 0x0000_0000, 0xFFFF_C000), // ENABLE_W1TC
    (0x018, 0x0000_0000, 0xFFFF_C000), // STATUS
    (0x01C, 0x0000_0000, 0xFFFF_C000), // STATUS_W1TS
    (0x020, 0x0000_0000, 0xFFFF_C000), // STATUS_W1TC
    (0x024, 0x0000_0000, 0x0000_0000), // IN
    (0x028, 0x0000_0000, 0x0000_0784), // PIN0
    (0x02C, 0x0000_0000, 0x0000_0784), // PIN1
    (0x030, 0x0000_0000, 0x0000_0784), // PIN2
    (0x034, 0x0000_0000, 0x0000_0784), // PIN3
    (0x038, 0x0000_0000, 0x0000_0784), // PIN4
    (0x03C, 0x0000_0000, 0x0000_0784), // PIN5
    (0x040, 0x0000_0000, 0x0000_0784), // PIN6
    (0x044, 0x0000_0000, 0x0000_0784), // PIN7
    (0x048, 0x0000_0000, 0x0000_0784), // PIN8
    (0x04C, 0x0000_0000, 0x0000_0784), // PIN9
    (0x050, 0x0000_0000, 0x0000_0784), // PIN10
    (0x054, 0x0000_0000, 0x0000_0784), // PIN11
    (0x058, 0x0000_0000, 0x0000_0784), // PIN12
    (0x05C, 0x0000_0000, 0x0000_0784), // PIN13
    (0x060, 0x0000_0000, 0x0000_0784), // PIN14
    (0x064, 0x0000_0000, 0x0000_0784), // PIN15
    (0x068, 0x0000_0000, 0x0000_0784), // PIN16
    (0x06C, 0x0000_0000, 0x0000_0784), // PIN17
    (0x070, 0x0000_0000, 0x03FF_FFFF), // RTC_DEBUG_SEL
    (0x074, 0x0000_0000, 0xFFFF_FFFF), // DIG_PAD_HOLD
    (0x078, 0x0000_0000, 0xC000_0000), // HALL_SENS
    (0x07C, 0x0000_0000, 0xFFFF_FFF0), // SENSOR_PADS
    (0x080, 0x0000_0000, 0xFFFC_0000), // ADC_PAD
    (0x084, 0x8000_0000, 0xFFFF_FC00), // PAD_DAC1
    (0x088, 0x8000_0000, 0xFFFF_FC00), // PAD_DAC2
    (0x08C, 0x8410_0010, 0xFFFF_FFFE), // XTAL_32K_PAD
    (0x090, 0x6600_0000, 0xFF80_0000), // TOUCH_CFG
    (0x094, 0x5200_0000, 0xFBFF_F000), // TOUCH_PAD0
    (0x098, 0x4A00_0000, 0xFBFF_F000), // TOUCH_PAD1
    (0x09C, 0x5200_0000, 0xFBFF_F000), // TOUCH_PAD2
    (0x0A0, 0x4A00_0000, 0xFBFF_F000), // TOUCH_PAD3
    (0x0A4, 0x5200_0000, 0xFBFF_F000), // TOUCH_PAD4
    (0x0A8, 0x5200_0000, 0xFBFF_F000), // TOUCH_PAD5
    (0x0AC, 0x4A00_0000, 0xFBFF_F000), // TOUCH_PAD6
    (0x0B0, 0x4200_0000, 0xFBFF_F000), // TOUCH_PAD7
    (0x0B4, 0x0200_0000, 0x03F8_0000), // TOUCH_PAD8
    (0x0B8, 0x0200_0000, 0x03F8_0000), // TOUCH_PAD9
    (0x0BC, 0x0000_0000, 0xF800_0000), // EXT_WAKEUP0
    (0x0C0, 0x0000_0000, 0xF800_0000), // XTL_EXT_CTR
    (0x0C4, 0x0000_0000, 0xFF80_0000), // SAR_I2C_IO
    (0x0C8, 0x0160_3160, 0x0FFF_FFFF), // DATE
];

/// The same for the ESP32-S3 RTC_IO block (`tests/fixtures/svd/esp32s3.svd`).
#[rustfmt::skip]
const ESP32S3_REGS: &[(u64, u32, u32)] = &[
    (0x000, 0x0000_0000, 0xFFFF_FC00), // RTC_GPIO_OUT
    (0x004, 0x0000_0000, 0xFFFF_FC00), // RTC_GPIO_OUT_W1TS
    (0x008, 0x0000_0000, 0xFFFF_FC00), // RTC_GPIO_OUT_W1TC
    (0x00C, 0x0000_0000, 0xFFFF_FC00), // RTC_GPIO_ENABLE
    (0x010, 0x0000_0000, 0xFFFF_FC00), // RTC_GPIO_ENABLE_W1TS
    (0x014, 0x0000_0000, 0xFFFF_FC00), // ENABLE_W1TC
    (0x018, 0x0000_0000, 0xFFFF_FC00), // RTC_GPIO_STATUS
    (0x01C, 0x0000_0000, 0xFFFF_FC00), // RTC_GPIO_STATUS_W1TS
    (0x020, 0x0000_0000, 0xFFFF_FC00), // RTC_GPIO_STATUS_W1TC
    (0x024, 0x0000_0000, 0x0000_0000), // RTC_GPIO_IN
    (0x028, 0x0000_0000, 0x0000_0784), // PIN0
    (0x02C, 0x0000_0000, 0x0000_0784), // PIN1
    (0x030, 0x0000_0000, 0x0000_0784), // PIN2
    (0x034, 0x0000_0000, 0x0000_0784), // PIN3
    (0x038, 0x0000_0000, 0x0000_0784), // PIN4
    (0x03C, 0x0000_0000, 0x0000_0784), // PIN5
    (0x040, 0x0000_0000, 0x0000_0784), // PIN6
    (0x044, 0x0000_0000, 0x0000_0784), // PIN7
    (0x048, 0x0000_0000, 0x0000_0784), // PIN8
    (0x04C, 0x0000_0000, 0x0000_0784), // PIN9
    (0x050, 0x0000_0000, 0x0000_0784), // PIN10
    (0x054, 0x0000_0000, 0x0000_0784), // PIN11
    (0x058, 0x0000_0000, 0x0000_0784), // PIN12
    (0x05C, 0x0000_0000, 0x0000_0784), // PIN13
    (0x060, 0x0000_0000, 0x0000_0784), // PIN14
    (0x064, 0x0000_0000, 0x0000_0784), // PIN15
    (0x068, 0x0000_0000, 0x0000_0784), // PIN16
    (0x06C, 0x0000_0000, 0x0000_0784), // PIN17
    (0x070, 0x0000_0000, 0x0000_0784), // PIN18
    (0x074, 0x0000_0000, 0x0000_0784), // PIN19
    (0x078, 0x0000_0000, 0x0000_0784), // PIN20
    (0x07C, 0x0000_0000, 0x0000_0784), // PIN21
    (0x080, 0x0000_0000, 0x03FF_FFFF), // RTC_DEBUG_SEL
    (0x084, 0x5000_0000, 0x787F_E000), // TOUCH_PAD0
    (0x088, 0x4800_0000, 0x787F_E000), // TOUCH_PAD1
    (0x08C, 0x5000_0000, 0x787F_E000), // TOUCH_PAD2
    (0x090, 0x4800_0000, 0x787F_E000), // TOUCH_PAD3
    (0x094, 0x5000_0000, 0x787F_E000), // TOUCH_PAD4
    (0x098, 0x5000_0000, 0x787F_E000), // TOUCH_PAD5
    (0x09C, 0x4800_0000, 0x787F_E000), // TOUCH_PAD6
    (0x0A0, 0x4000_0000, 0x787F_E000), // TOUCH_PAD7
    (0x0A4, 0x4000_0000, 0x787F_E000), // TOUCH_PAD8
    (0x0A8, 0x4000_0000, 0x787F_E000), // TOUCH_PAD9
    (0x0AC, 0x4000_0000, 0x787F_E000), // TOUCH_PAD10
    (0x0B0, 0x4000_0000, 0x787F_E000), // TOUCH_PAD11
    (0x0B4, 0x4000_0000, 0x787F_E000), // TOUCH_PAD12
    (0x0B8, 0x4000_0000, 0x787F_E000), // TOUCH_PAD13
    (0x0BC, 0x4000_0000, 0x787F_E000), // TOUCH_PAD14
    (0x0C0, 0x4000_0000, 0x780F_E000), // XTAL_32P_PAD
    (0x0C4, 0x4000_0000, 0x780F_E000), // XTAL_32N_PAD
    (0x0C8, 0x4000_0000, 0x780F_FFF8), // PAD_DAC1
    (0x0CC, 0x4000_0000, 0x780F_FFF8), // PAD_DAC2
    (0x0D0, 0x5000_0000, 0x780F_E000), // RTC_PAD19
    (0x0D4, 0x5000_0000, 0x780F_E000), // RTC_PAD20
    (0x0D8, 0x5000_0000, 0x780F_E000), // RTC_PAD21
    (0x0DC, 0x0000_0000, 0xF800_0000), // EXT_WAKEUP0
    (0x0E0, 0x0000_0000, 0xF800_0000), // XTL_EXT_CTR
    (0x0E4, 0x0000_0000, 0xFF80_0000), // SAR_I2C_IO
    (0x0E8, 0x0000_0000, 0x0000_001F), // TOUCH_CTRL
    (0x1FC, 0x0210_1180, 0x0FFF_FFFF), // DATE
];

/// Everything that differs between the two parts.
#[derive(Debug)]
struct Layout {
    chip: RtcChip,
    pads: &'static [RtcPad],
    /// Bit of RTC_GPIO0 in OUT / ENABLE / STATUS / IN.
    shift: u32,
    regs: &'static [(u64, u32, u32)],
    /// The pulls follow `RUE` / `RDE` in the digital domain too (classic
    /// errata 3.6), not only with `MUX_SEL` = 1.
    pulls_in_digital_mode: bool,
}

static ESP32_LAYOUT: Layout = Layout {
    chip: RtcChip::Esp32,
    pads: &ESP32_PADS,
    shift: 14,
    regs: ESP32_REGS,
    pulls_in_digital_mode: true,
};

static ESP32S3_LAYOUT: Layout = Layout {
    chip: RtcChip::Esp32s3,
    pads: &ESP32S3_PADS,
    shift: 10,
    regs: ESP32S3_REGS,
    pulls_in_digital_mode: false,
};

/// Which side owns each RTC-capable pad, and the RTC side's output stage,
/// as GPIO-pad bitmasks (bit = GPIO number). A GPIO model reads it on every
/// pad question; the RTC_IO model refreshes it on every write.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RtcPadView {
    /// Pads with `MUX_SEL` = 1: the RTC IO MUX owns them.
    pub rtc: u64,
    /// RTC-domain pads whose output stage drives: `RTC_GPIO_ENABLE` set and
    /// not an open-drain (`PAD_DRIVER`) 1.
    pub drive: u64,
    /// The level a driving RTC-domain pad drives.
    pub level: u64,
    /// The pull-up acting on the pad now: `RUE` of an RTC-domain pad, and on
    /// the classic part of a digital-domain one too (errata 3.6).
    pub pull_up: u64,
    /// The pull-down, the same way (`RDE`).
    pub pull_down: u64,
    /// RTC-domain pads whose input buffer is on (`FUN_IE`).
    pub ie: u64,
}

impl RtcPadView {
    #[inline]
    fn bit(mask: u64, pin: u8) -> bool {
        pin < 64 && mask & (1u64 << pin) != 0
    }

    /// `true` when the RTC IO MUX owns `pin` (`MUX_SEL` = 1).
    #[inline]
    pub fn in_rtc(&self, pin: u8) -> bool {
        Self::bit(self.rtc, pin)
    }

    /// Nothing differs from a part without RTC_IO: no pad in the RTC domain
    /// and no pull from it.
    #[inline]
    pub fn is_idle(&self) -> bool {
        self.rtc | self.pull_up | self.pull_down == 0
    }

    /// The pull the RTC_IO block puts on `pin` now.
    #[inline]
    pub fn pull(&self, pin: u8) -> Pull {
        if Self::bit(self.pull_up, pin) {
            Pull::Up
        } else if Self::bit(self.pull_down, pin) {
            Pull::Down
        } else {
            Pull::None
        }
    }

    /// The output stage of an RTC-domain pad.
    pub fn driver(&self, pin: u8) -> PadDriver {
        let pull = self.pull(pin);
        if Self::bit(self.drive, pin) {
            PadDriver::drive(Self::bit(self.level, pin)).with_pull(pull)
        } else {
            PadDriver::released(pull)
        }
    }

    /// The level on an RTC-domain pad: its own output stage against what the
    /// outside presents; a floating pad (released, no pull, nothing outside)
    /// keeps `stored`, the last level the outside presented, as the GPIO
    /// models do for a floating digital pad.
    pub fn wire_level(&self, pin: u8, ext: External, stored: bool) -> bool {
        resolve(self.driver(pin), ext).level.unwrap_or(stored)
    }

    /// What the input buffer of an RTC-domain pad passes on (`RTC_GPIO_IN`,
    /// and `GPIO_IN` / the matrix inputs, see the module docs): the wire
    /// level, or 0 with `FUN_IE` clear.
    pub fn input(&self, pin: u8, ext: External, stored: bool) -> bool {
        Self::bit(self.ie, pin) && self.wire_level(pin, ext, stored)
    }
}

/// What the GPIO model knows about the pads that the RTC side needs, as
/// GPIO-pad bitmasks: the outside's levels, and what `GPIO_IN` reads for a
/// pad in the digital domain.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PadInputs {
    /// Pads the outside drives ([`External::Level`]).
    pub ext_mask: u64,
    /// The last level the outside presented to each pad, held or not.
    pub stored: u64,
    /// `GPIO_IN` of every pad, as the digital side reads it.
    pub digital: u64,
}

impl PadInputs {
    fn external(&self, pin: u8) -> External {
        if pin < 64 && self.ext_mask & (1u64 << pin) != 0 {
            External::Level(self.stored & (1u64 << pin) != 0)
        } else {
            External::Released
        }
    }
}

/// The RTC-domain output stage of one pad, frozen while its `HOLD` is set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Held {
    active: bool,
    oe: bool,
    out: bool,
    open_drain: bool,
    rue: bool,
    rde: bool,
}

/// The register state behind the lock.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct State {
    /// One word per [`Layout::regs`] entry.
    words: Vec<u32>,
    /// Per RTC GPIO: its output stage as frozen by `HOLD`.
    held: Vec<Held>,
    inputs: PadInputs,
    /// `RTC_GPIO_IN` at the last evaluation, RTC GPIO n at bit n: what the
    /// edge-type interrupts compare against.
    last_in: u32,
}

/// The RTC_IO register state shared between the MMIO model and the GPIO
/// model (see the module docs).
#[derive(Debug)]
pub struct RtcIoPads {
    layout: &'static Layout,
    state: RwLock<State>,
    /// [`RtcPadView`] as of the last change, lock-free for the GPIO model's
    /// hot paths: rtc, drive, level, pull_up, pull_down, ie.
    view: [AtomicU64; 6],
}

impl RtcIoPads {
    fn new(layout: &'static Layout) -> Self {
        let pads = Self {
            layout,
            state: RwLock::new(State {
                words: layout.regs.iter().map(|&(_, reset, _)| reset).collect(),
                held: vec![Held::default(); layout.pads.len()],
                inputs: PadInputs::default(),
                last_in: 0,
            }),
            view: Default::default(),
        };
        {
            let mut st = pads.write_state();
            pads.refresh(&mut st);
            st.last_in = pads.rtc_in(&st);
        }
        pads
    }

    /// The part these pads belong to.
    pub fn chip(&self) -> RtcChip {
        self.layout.chip
    }

    /// GPIO pads that have an RTC function, bit = GPIO number.
    pub fn rtc_capable(&self) -> u64 {
        self.layout
            .pads
            .iter()
            .fold(0u64, |m, p| m | (1u64 << p.gpio))
    }

    /// The RTC GPIO number of GPIO pad `gpio`, if it has one.
    pub fn rtc_channel(&self, gpio: u8) -> Option<u8> {
        self.layout
            .pads
            .iter()
            .position(|p| p.gpio == gpio)
            .map(|i| i as u8)
    }

    /// [`RtcPadView::rtc`] alone: the pads with `MUX_SEL` = 1, bit = GPIO.
    #[inline]
    pub fn rtc_mask(&self) -> u64 {
        self.view[0].load(Ordering::Relaxed)
    }

    /// Which side owns each pad and the RTC side's output stage, now.
    #[inline]
    pub fn view(&self) -> RtcPadView {
        let v = |k: usize| self.view[k].load(Ordering::Relaxed);
        RtcPadView {
            rtc: v(0),
            drive: v(1),
            level: v(2),
            pull_up: v(3),
            pull_down: v(4),
            ie: v(5),
        }
    }

    /// The GPIO model's report of the pads (see [`PadInputs`]): re-derives
    /// `RTC_GPIO_IN` and latches the RTC GPIO interrupts it moved.
    pub fn set_inputs(&self, inputs: PadInputs) {
        let mut st = self.write_state();
        if st.inputs == inputs {
            return;
        }
        st.inputs = inputs;
        self.latch(&mut st);
    }

    fn write_state(&self) -> std::sync::RwLockWriteGuard<'_, State> {
        self.state.write().expect("RTC_IO state poisoned")
    }

    fn read_state(&self) -> std::sync::RwLockReadGuard<'_, State> {
        self.state.read().expect("RTC_IO state poisoned")
    }

    fn index(&self, off: u64) -> Option<usize> {
        self.layout.regs.iter().position(|&(o, _, _)| o == off)
    }

    fn word(&self, st: &State, off: u64) -> u32 {
        self.index(off).map(|i| st.words[i]).unwrap_or(0)
    }

    fn store(&self, st: &mut State, off: u64, value: u32) {
        if let Some(i) = self.index(off) {
            let wmask = self.layout.regs[i].2;
            st.words[i] = (st.words[i] & !wmask) | (value & wmask);
        }
    }

    fn bit(word: u32, bit: Option<u8>) -> bool {
        bit.is_some_and(|b| word & (1 << b) != 0)
    }

    /// The live (unheld) RTC output stage of RTC GPIO `n`.
    fn live(&self, st: &State, n: usize) -> Held {
        let pad = self.layout.pads[n];
        let bit = 1u32 << (self.layout.shift + n as u32);
        let word = self.word(st, pad.reg);
        Held {
            active: false,
            oe: self.word(st, ENABLE) & bit != 0,
            out: self.word(st, OUT) & bit != 0,
            open_drain: self.word(st, PIN0 + 4 * n as u64)
                & crate::peripherals::esp_gpio_net::PIN_PAD_DRIVER
                != 0,
            rue: Self::bit(word, pad.rue),
            rde: Self::bit(word, pad.rde),
        }
    }

    /// The RTC output stage of RTC GPIO `n` in effect: frozen while held.
    fn effective(&self, st: &State, n: usize) -> Held {
        let held = st.held[n];
        if held.active {
            held
        } else {
            self.live(st, n)
        }
    }

    /// After a change to the registers: take or drop every `HOLD` snapshot
    /// and republish the view.
    fn refresh(&self, st: &mut State) {
        for n in 0..self.layout.pads.len() {
            let pad = self.layout.pads[n];
            let hold = Self::bit(self.word(st, pad.reg), pad.hold);
            match (hold, st.held[n].active) {
                (true, false) => {
                    st.held[n] = Held {
                        active: true,
                        ..self.live(st, n)
                    }
                }
                (false, true) => st.held[n] = Held::default(),
                _ => {}
            }
        }
        let mut v = RtcPadView::default();
        for (n, pad) in self.layout.pads.iter().enumerate() {
            let g = 1u64 << pad.gpio;
            let word = self.word(st, pad.reg);
            let rtc = word & (1 << pad.mux_sel) != 0;
            let stage = self.effective(st, n);
            if rtc {
                v.rtc |= g;
                // An input-only pad (no pulls in its register: GPIO34..39 on
                // the classic part) has no output stage to enable.
                if stage.oe && !(stage.open_drain && stage.out) && pad.rue.is_some() {
                    v.drive |= g;
                }
                if stage.out {
                    v.level |= g;
                }
                if word & (1 << pad.fun_ie) != 0 {
                    v.ie |= g;
                }
            }
            if rtc || self.layout.pulls_in_digital_mode {
                if stage.rue {
                    v.pull_up |= g;
                } else if stage.rde {
                    v.pull_down |= g;
                }
            }
        }
        for (k, value) in [v.rtc, v.drive, v.level, v.pull_up, v.pull_down, v.ie]
            .into_iter()
            .enumerate()
        {
            self.view[k].store(value, Ordering::Relaxed);
        }
    }

    /// `RTC_GPIO_IN`, RTC GPIO n at bit n: an RTC-domain pad through its
    /// `FUN_IE`, a digital-domain one as `GPIO_IN` reads it.
    fn rtc_in(&self, st: &State) -> u32 {
        let v = self.view();
        let mut word = 0u32;
        for (n, pad) in self.layout.pads.iter().enumerate() {
            let g = pad.gpio;
            let level = if v.in_rtc(g) {
                v.input(
                    g,
                    st.inputs.external(g),
                    st.inputs.stored & (1u64 << g) != 0,
                )
            } else {
                st.inputs.digital & (1u64 << g) != 0
            };
            if level {
                word |= 1 << n;
            }
        }
        word
    }

    /// Latch `RTC_GPIO_STATUS` for every pad whose input moved since the
    /// last evaluation (edge types) or whose level holds (level types), per
    /// `RTC_GPIO_PINn.INT_TYPE`.
    fn latch(&self, st: &mut State) {
        use crate::peripherals::esp_gpio_net::{edge_hits, int_type, level_hits};
        let now = self.rtc_in(st);
        let mut hit = 0u32;
        for n in 0..self.layout.pads.len() {
            let kind = int_type(self.word(st, PIN0 + 4 * n as u64));
            if kind == 0 {
                continue;
            }
            let (b, a) = (st.last_in & (1 << n) != 0, now & (1 << n) != 0);
            if edge_hits(kind, b, a) || level_hits(kind, a) {
                hit |= 1 << (self.layout.shift + n as u32);
            }
        }
        st.last_in = now;
        if hit != 0 {
            let status = self.word(st, STATUS) | hit;
            self.store(st, STATUS, status);
        }
    }

    /// Register read, as firmware sees it.
    fn read_word(&self, off: u64) -> u32 {
        let st = self.read_state();
        match off {
            // Write-only views (SVD `write-only` fields): read as 0.
            OUT_W1TS | OUT_W1TC | ENABLE_W1TS | ENABLE_W1TC | STATUS_W1TS | STATUS_W1TC => 0,
            IN => self.rtc_in(&st) << self.layout.shift,
            off => self.word(&st, off),
        }
    }

    /// Register write: the store, then the holds and the view, then the
    /// interrupts the store caused (an RTC-domain pad's own output edge, a
    /// level-type pin re-armed by `STATUS_W1TC` or a `PINn` write).
    fn write_word(&self, off: u64, value: u32) {
        let mut st = self.write_state();
        let data = |st: &State, off| self.word(st, off);
        match off {
            OUT_W1TS => {
                let v = data(&st, OUT) | value;
                self.store(&mut st, OUT, v);
            }
            OUT_W1TC => {
                let v = data(&st, OUT) & !value;
                self.store(&mut st, OUT, v);
            }
            ENABLE_W1TS => {
                let v = data(&st, ENABLE) | value;
                self.store(&mut st, ENABLE, v);
            }
            ENABLE_W1TC => {
                let v = data(&st, ENABLE) & !value;
                self.store(&mut st, ENABLE, v);
            }
            STATUS_W1TS => {
                let v = data(&st, STATUS) | value;
                self.store(&mut st, STATUS, v);
            }
            STATUS_W1TC => {
                let v = data(&st, STATUS) & !value;
                self.store(&mut st, STATUS, v);
            }
            IN => {}
            off => self.store(&mut st, off, value),
        }
        self.refresh(&mut st);
        self.latch(&mut st);
    }
}

/// The RTC_IO MMIO block of the classic ESP32 (`DR_REG_RTCIO_BASE` =
/// 0x3FF4_8400) or the ESP32-S3 (0x6000_8400). See the module docs.
#[derive(Debug)]
pub struct EspRtcIo {
    pads: Arc<RtcIoPads>,
    /// Offsets at and above this are not RTC_IO (see [`Self::tail`]).
    tail_from: u64,
    /// Round-trip storage for the rest of a wider bus window. The classic bus
    /// has always mapped `rtcio` as 4 KiB from 0x3FF4_8400, and the
    /// 0x3FF4_8800.. part of it (the SENS registers past the
    /// `sens_sar_adc` model's 0x100) answered as round-trip storage; this
    /// keeps that answer while the RTC_IO registers below `tail_from` get
    /// their model. Empty on the S3, whose window is the block's own.
    tail: std::collections::BTreeMap<u64, u32>,
}

impl EspRtcIo {
    /// Classic ESP32 RTC_IO.
    pub fn new_esp32() -> Self {
        Self {
            pads: Arc::new(RtcIoPads::new(&ESP32_LAYOUT)),
            tail_from: 0x400,
            tail: Default::default(),
        }
    }

    /// ESP32-S3 RTC_IO.
    pub fn new_esp32s3() -> Self {
        Self {
            pads: Arc::new(RtcIoPads::new(&ESP32S3_LAYOUT)),
            tail_from: 0x400,
            tail: Default::default(),
        }
    }

    /// The shared pad state (see [`RtcIoPads`]).
    pub fn pads(&self) -> Arc<RtcIoPads> {
        Arc::clone(&self.pads)
    }

    fn read_word(&self, off: u64) -> u32 {
        if off >= self.tail_from {
            return self.tail.get(&off).copied().unwrap_or(0);
        }
        self.pads.read_word(off)
    }

    fn write_word(&mut self, off: u64, value: u32) {
        if off >= self.tail_from {
            self.tail.insert(off, value);
            return;
        }
        self.pads.write_word(off, value);
    }
}

/// What a runtime snapshot carries: the register state and the tail.
#[derive(serde::Serialize, serde::Deserialize)]
struct Snapshot {
    state: State,
    tail: Vec<(u64, u32)>,
}

impl Peripheral for EspRtcIo {
    /// Changes only at MMIO write time and on GPIO input reports.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    /// A byte store merges into the word as it reads back. The write-1-to-set
    /// / -clear views read 0, so a byte written there carries only its own
    /// bits.
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let off = offset & !3;
        let shift = (offset & 3) * 8;
        let word = (self.read_word(off) & !(0xFFu32 << shift)) | (u32::from(value) << shift);
        self.write_word(off, word);
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset & 3 == 0 {
            self.write_word(offset, value);
            Ok(())
        } else {
            for i in 0..4 {
                self.write(offset + i, (value >> (i * 8)) as u8)?;
            }
            Ok(())
        }
    }

    fn rtc_io_pads(&self) -> Option<Arc<RtcIoPads>> {
        Some(self.pads())
    }

    fn snapshot(&self) -> serde_json::Value {
        let st = self.pads.read_state();
        serde_json::json!({
            "out": self.pads.word(&st, OUT),
            "enable": self.pads.word(&st, ENABLE),
            "status": self.pads.word(&st, STATUS),
            "in": self.pads.rtc_in(&st) << self.pads.layout.shift,
            "rtc_pads": self.pads.view().rtc,
        })
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        let snap = Snapshot {
            state: self.pads.read_state().clone(),
            tail: self.tail.iter().map(|(k, v)| (*k, *v)).collect(),
        };
        bincode::serialize(&snap).expect("bincode serialize EspRtcIo")
    }

    /// Restores THROUGH the shared state, so the GPIO model that holds it
    /// keeps its wiring.
    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        let snap: Snapshot = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("EspRtcIo snapshot decode: {e}"))
        })?;
        if snap.state.words.len() != self.pads.layout.regs.len()
            || snap.state.held.len() != self.pads.layout.pads.len()
        {
            return Err(crate::SimulationError::NotImplemented(
                "EspRtcIo snapshot is for another part".to_string(),
            ));
        }
        {
            let mut st = self.pads.write_state();
            *st = snap.state;
            // The holds come back active where their HOLD bit is set, so
            // this only republishes the view.
            self.pads.refresh(&mut st);
        }
        self.tail = snap.tail.into_iter().collect();
        Ok(())
    }
}

#[cfg(test)]
#[path = "esp_rtc_io_tests.rs"]
mod tests;
