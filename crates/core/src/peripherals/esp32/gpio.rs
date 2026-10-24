// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! GPIO peripheral for ESP32-classic (LX6).
//!
//! Maps at 0x3FF44000 per ESP32 TRM v4.6 §4.10. Models the subset esp-hal
//! 1.x writes during init + the e-paper lab firmware path:
//!   - GPIO_OUT / OUT_W1TS / OUT_W1TC for GPIO0..31, OUT1 (+ W1TS / W1TC)
//!     for GPIO32..39 at bit `pad - 32`
//!   - GPIO_ENABLE / ENABLE_W1TS / ENABLE_W1TC for GPIO0..31, ENABLE1 for
//!     GPIO32..39. GPIO34..39 are input-only on silicon
//!     (`SOC_GPIO_VALID_OUTPUT_GPIO_MASK`): their ENABLE1 bits store but no
//!     output stage drives the pad
//!   - GPIO_IN / GPIO_IN1 (the pad level, settable via `set_pin_input` for
//!     tests and by the outside world through [`PinPort::set_external`])
//!   - GPIO_PINn_REG for GPIO0..39 (PAD_DRIVER, INT_TYPE, INT_ENA, ...) and
//!     the GPIO interrupt: `GPIO_STATUS` / `GPIO_STATUS1` latched per
//!     INT_TYPE from changes of `GPIO_IN` / `GPIO_IN1`, `GPIO_ACPU_INT(1)` /
//!     `GPIO_PCPU_INT(1)`, and matrix source 22 while either CPU has a
//!     pending enabled pin in either bank
//!   - `gpio_net` membership for GPIO0..39 (see `crate::peripherals::esp_gpio_net`)
//!
//! Observer protocol is the shared `peripherals::device::GpioObserver` —
//! a single trait makes observer code work on both chip variants.

use crate::peripherals::pad_routing::PadRoutes;
use crate::pins::{External, InputChange, PinPort};
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::sync::Arc;

/// Classic-ESP32 pad count (`SOC_GPIO_PIN_COUNT = 40`, esp-idf
/// `soc/esp32/include/soc/soc_caps.h`).
///
/// The output MATRIX is flat across all 40 — `func_out_sel_cfg[40]`, one entry
/// per pad indexed by pad number with no bank in it (esp-idf
/// `soc/esp32/include/soc/gpio_struct.h`). The OUT/ENABLE *registers* are the
/// banked ones: GPIO0..31 in OUT/ENABLE, GPIO32..39 in OUT1/ENABLE1 at bit
/// `pad - 32`. Conflating the two is the trap — indexing FUNCn by `pad - 32`
/// for the high bank would route GPIO32 to the selector GPIO0 owns.
const PAD_COUNT: u8 = 40;

/// `GPIO_FUNC0_OUT_SEL_CFG_REG` = `DR_REG_GPIO_BASE + 0x0530`, stride 4, 40
/// entries ending at `FUNC39` = base + 0x05CC (esp-idf
/// `soc/esp32/include/soc/gpio_reg.h`). The vendored
/// `tests/fixtures/real_world/esp32.svd` agrees: addressOffset 0x530, dim 40,
/// dimIncrement 0x4.
const FUNC_OUT_SEL: u64 = 0x530;
const FUNC_OUT_SEL_END: u64 = FUNC_OUT_SEL + (PAD_COUNT as u64) * 4;

/// `OUT_SEL` — bits [8:0], "select one of the 256 output to 40 GPIO"
/// (`gpio_reg.h`: `_S 0`, `_V 0x1FF`; SVD field bitOffset 0 bitWidth 9).
const OUT_SEL_MASK: u32 = 0x1FF;

/// Writable bits of the register: OUT_SEL[8:0], INV_SEL(9), OEN_SEL(10),
/// OEN_INV_SEL(11). Everything above bit 11 is reserved and must NOT store — a
/// register that round-trips reserved bits reads back state the silicon never
/// held, and an inspect wall then reports it as fact.
const FUNC_OUT_SEL_WMASK: u32 = 0x0000_0FFF;

/// Writable bits of `GPIO_PINn_REG` (esp-idf `soc/esp32/include/soc/gpio_reg.h`):
/// PAD_DRIVER (2), INT_TYPE [9:7], WAKEUP_ENABLE (10), CONFIG [12:11],
/// INT_ENA [17:13].
const PIN_WMASK: u32 = 0x0003_FF84;
/// `GPIO_PIN0_REG` and one past `GPIO_PIN39_REG` (0x124): one word per pad.
const PIN0: u64 = 0x88;
const PIN_END: u64 = PIN0 + (PAD_COUNT as u64) * 4;

/// `ETS_GPIO_INTR_SOURCE` on the classic ESP32 interrupt matrix (esp-idf
/// `soc/esp32/include/soc/soc.h`). The C3 and S3 put it at 16.
const GPIO_INTR_SOURCE: u32 = 22;

/// `out_sel` sentinel meaning "this pad is driven by the GPIO_OUT latch" — a
/// plain GPIO output with the matrix bypassed. `SIG_GPIO_OUT_IDX = 256`
/// (esp-idf `soc/esp32/include/soc/gpio_sig_map.h`).
///
/// ⚠️ 256 on classic ESP32 and on the S3; **128** on the C3. The matrix index
/// space is per-chip. Reaching for the C3's constant here would make every
/// plain-GPIO pad decode as matrix-routed AND every pad routed to signal 128
/// decode as plain GPIO — silently, in both directions.
const SIG_GPIO_OUT: u32 = 256;

/// Reset value of every `FUNCn_OUT_SEL_CFG`: the matrix-bypass sentinel.
///
/// ⚠️ NOT LOCALLY VERIFIABLE for classic ESP32. `gpio_reg.h` records the field
/// default as `x`, and the vendored `esp32.svd` carries no `<resetValue>` for
/// this register, so `configs/peripherals/esp32/gpio.yaml`'s `reset_value: 0`
/// is the ingestor's default rather than a measurement, and classic ESP32 has
/// no `reset_oracle`. Two arguments carry 0x100: the ESP32-S3 SVD gives 0x100
/// for the byte-identical register on the same IP, and `gpio_ll_output_disable`
/// writes exactly `SIG_GPIO_OUT_IDX` here with the comment "Ensure no other
/// output signal is routed via GPIO matrix to this pin" — 256 IS the silicon's
/// "nothing routed" encoding.
///
/// Seeding 0 would be an active regression, not a neutral choice: index 0 is
/// `SPICLK_OUT_IDX`, so every enabled output pad would report `Af` at reset.
const FUNC_OUT_SEL_RESET: u32 = 0x0000_0100;

/// GPIO-matrix OUTPUT signal indices of the I²C0 (I2C_EXT0) controller —
/// esp-idf `soc/esp32/include/soc/gpio_sig_map.h`. Classic numbers: NOT the
/// C3's 53/54 and NOT the S3's 89/90.
const SIG_I2CEXT0_SCL: u32 = 29;
const SIG_I2CEXT0_SDA: u32 = 30;
/// The I²C1 (I2C_EXT1) controller's, same header (`I2CEXT1_SCL_OUT_IDX` 95,
/// `I2CEXT1_SDA_OUT_IDX` 96; the input indices are the same numbers). Not the
/// S3's 91/92.
const SIG_I2CEXT1_SCL: u32 = 95;
const SIG_I2CEXT1_SDA: u32 = 96;

/// One I²C controller's matrix signals and the names its pads report.
#[derive(Debug, Clone, Copy)]
pub(crate) struct I2cSignals {
    scl: u32,
    sda: u32,
    names: [&'static str; 2],
}

/// I2C_EXT0 (`i2c0`).
pub(crate) const I2C0_SIGNALS: I2cSignals = I2cSignals {
    scl: SIG_I2CEXT0_SCL,
    sda: SIG_I2CEXT0_SDA,
    names: ["I2CEXT0_SCL", "I2CEXT0_SDA"],
};

/// I2C_EXT1 (`i2c1`, 0x3FF6_7000).
pub(crate) const I2C1_SIGNALS: I2cSignals = I2cSignals {
    scl: SIG_I2CEXT1_SCL,
    sda: SIG_I2CEXT1_SDA,
    names: ["I2CEXT1_SCL", "I2CEXT1_SDA"],
};

/// GPIO-matrix OUTPUT signal indices of the **VSPI** master — the controller the
/// ESP32 datasheet (v5.3 p61) calls `SPI3` and whose matrix signals are prefixed
/// `VSPI`, and the one arduino-esp32's `SPIClass SPI(VSPI)` drives
/// (`libraries/SPI/src/SPI.cpp` :348). esp-idf
/// `soc/esp32/include/soc/gpio_sig_map.h` :138 / :142 / :148.
///
/// ⚠️ Classic carries THREE GP-SPI signal groups and they share NO numbers:
/// `SPICLK_OUT_IDX` = 0 (:18 — the flash controller SPI0/1), `HSPICLK_OUT_IDX`
/// = 8 (:34 — SPI2), `VSPICLK_OUT_IDX` = 63 (:138 — SPI3). Index 0 being a live
/// SPI signal is exactly why [`FUNC_OUT_SEL_RESET`] must be 0x100 and not 0.
///
/// ⚠️ 63/65/68 are ALSO the ESP32-C3's `FSPI*` indices. That is a collision of
/// two per-chip index spaces, not a shared space: on the ESP32-S3 the same three
/// numbers are not SPI signals at all (`FSPICLK` is 101 there). Borrowing a
/// sibling's constant fails SILENTLY in both directions — a plain pad decodes as
/// routed and a routed pad as plain — so every index here is cited to the
/// CLASSIC header and to nothing else.
const SIG_VSPICLK: u32 = 63;
/// `VSPIQ_OUT_IDX` = 64 is VSPI's MISO and is deliberately NOT bound; see
/// [`crate::peripherals::esp_gpspi_wire`] for why an undriven line stays on the
/// latch fallback.
const SIG_VSPID: u32 = 65;
const SIG_VSPICS0: u32 = 68;
/// `VSPIQ_IN_IDX` / `VSPIQ_OUT_IDX` (`gpio_sig_map.h` :140): VSPI's MISO.
/// Bound only for a controller that can take part in a world `gpio_net`,
/// whose cell has a MISO line (an input of a master, the output of a slave).
const SIG_VSPIQ: u32 = 64;
/// The **HSPI** (SPI2) group, `gpio_sig_map.h` :34..:40: `HSPICLK` 8,
/// `HSPIQ` 9, `HSPID` 10, `HSPICS0` 11, in and out alike.
const SIG_HSPICLK: u32 = 8;
const SIG_HSPIQ: u32 = 9;
const SIG_HSPID: u32 = 10;
const SIG_HSPICS0: u32 = 11;

/// One GP-SPI's matrix signals: `[CLK, D (MOSI), CS0, Q (MISO)]`, with the
/// names the pads report.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SpiSignals {
    sigs: [u32; 4],
    names: [&'static str; 4],
}

/// VSPI (`spi3`, the controller Arduino's `SPI` drives).
pub(crate) const VSPI_SIGNALS: SpiSignals = SpiSignals {
    sigs: [SIG_VSPICLK, SIG_VSPID, SIG_VSPICS0, SIG_VSPIQ],
    names: ["SPI3_SCK", "SPI3_MOSI", "SPI3_CS", "SPI3_MISO"],
};

/// HSPI (`spi2`).
pub(crate) const HSPI_SIGNALS: SpiSignals = SpiSignals {
    sigs: [SIG_HSPICLK, SIG_HSPID, SIG_HSPICS0, SIG_HSPIQ],
    names: ["SPI2_SCK", "SPI2_MOSI", "SPI2_CS", "SPI2_MISO"],
};

/// `GPIO_FUNC0_IN_SEL_CFG_REG` = `DR_REG_GPIO_BASE + 0x0130`, one word per
/// input signal (256), up to `FUNC255` at 0x052C (esp-idf
/// `soc/esp32/include/soc/gpio_reg.h`; `esp32.svd` FUNC%s_IN_SEL_CFG dim 256).
const FUNC_IN_SEL: u64 = 0x130;
const FUNC_IN_SEL_END: u64 = FUNC_IN_SEL + 256 * 4;
/// Writable bits: `IN_SEL` [5:0], `IN_INV_SEL` (6), `SIG_IN_SEL` (7).
const FUNC_IN_SEL_WMASK: u32 = 0xFF;
/// The classic `FUNCn_IN_SEL_CFG` encoding: pad in [5:0], `SIG_IN_SEL` (route
/// through the matrix rather than the signal's IO_MUX pad) at bit 7.
const IN_SEL_LAYOUT: crate::peripherals::esp_gpio_net::InSelLayout =
    crate::peripherals::esp_gpio_net::InSelLayout {
        pad_mask: 0x3F,
        sel_bit: 1 << 7,
    };

/// GPIO-matrix OUTPUT signal indices of the three UART transmitters — esp-idf
/// `soc/esp32/include/soc/gpio_sig_map.h` :46 `U0TXD_OUT_IDX`, :52
/// `U1TXD_OUT_IDX`, :360 `U2TXD_OUT_IDX`.
///
/// ⚠️ The IO_MUX verdict differs from BOTH siblings. On the C3 UART1 is
/// matrix-only and on the S3 UART2 is; on classic **all three** UARTs have an
/// IO_MUX pad, so none of them is matrix-visible on a stock default-pin
/// `Serial.begin()`:
///
/// | UART | default TX pad | IO_MUX function |
/// |---|---|---|
/// | U0TXD | GPIO1 (`uart_pins.h` :23) | 0 (`io_mux_reg.h` :128 `FUNC_U0TXD_U0TXD`) |
/// | U1TXD | GPIO10 (`uart_pins.h` :28) | 4 (`io_mux_reg.h` :195 `FUNC_SD_DATA3_U1TXD`) |
/// | U2TXD | GPIO17 (`uart_pins.h` :33) | 4 (`io_mux_reg.h` :256 `FUNC_GPIO17_U2TXD`) |
///
/// `uart_set_pin` takes the IO_MUX path for exactly those pads and falls back to
/// `gpio_matrix_out` for every other pin, and this model does not model IO_MUX
/// at all — so a default-pin console correctly leaves `FUNCn_OUT_SEL` at the
/// bypass sentinel and the pad keeps reading its latch. The routes light the
/// moment firmware remaps TX, which every real WROOM-32 board does for UART1
/// (GPIO9/GPIO10 are the flash pins) and most do for UART2.
const SIG_U0TXD: u32 = 14;
const SIG_U1TXD: u32 = 17;
const SIG_U2TXD: u32 = 198;

/// GPIO-matrix OUTPUT signal index of the first LEDC channel —
/// `LEDC_HS_SIG_OUT0_IDX` = 71 in esp-idf `soc/esp32/include/soc/gpio_sig_map.h`.
/// HS0..HS7 are 71..78 and LS0..LS7 are 79..86 (`LEDC_LS_SIG_OUT0_IDX` = 79),
/// one per line of [`crate::peripherals::esp32::ledc::LEDC_LINES`], in order.
/// `ledcAttachPin` routes a pad with `gpio_matrix_out(pin, LEDC_HS_SIG_OUT0_IDX +
/// channel, ...)`.
const SIG_LEDC_HS0: u32 = 71;

/// Names of the 16 LEDC output signals, as `gpio_sig_map.h` spells them.
const LEDC_SIGNAL_NAMES: [&str; 16] = [
    "LEDC_HS_SIG_OUT0",
    "LEDC_HS_SIG_OUT1",
    "LEDC_HS_SIG_OUT2",
    "LEDC_HS_SIG_OUT3",
    "LEDC_HS_SIG_OUT4",
    "LEDC_HS_SIG_OUT5",
    "LEDC_HS_SIG_OUT6",
    "LEDC_HS_SIG_OUT7",
    "LEDC_LS_SIG_OUT0",
    "LEDC_LS_SIG_OUT1",
    "LEDC_LS_SIG_OUT2",
    "LEDC_LS_SIG_OUT3",
    "LEDC_LS_SIG_OUT4",
    "LEDC_LS_SIG_OUT5",
    "LEDC_LS_SIG_OUT6",
    "LEDC_LS_SIG_OUT7",
];

/// Pads that can drive an output at all: `SOC_GPIO_VALID_OUTPUT_GPIO_MASK`
/// (esp-idf `soc_caps.h`) — the 40 pads minus GPIO24 and GPIO28..31 (absent
/// from the package) and minus GPIO34..39 (input-only). Binding a peripheral
/// wire to a pad that cannot drive would publish a bus onto a pin no board can
/// wire it to.
const VALID_OUTPUT_PADS: u64 = 0x0000_0003_0EFF_FFFF;

/// The bank-1 half of [`VALID_OUTPUT_PADS`], at bank-relative bits: GPIO32
/// and GPIO33 have an output stage, GPIO34..39 (bits 2..7) do not. ENABLE1
/// stores all eight bits, as the register does, but only these two drive.
const OUTPUT_PADS_BANK1: u32 = (VALID_OUTPUT_PADS >> 32) as u32;

/// The eight pads of bank 1 (GPIO32..39) at bank-relative bits 0..7: the
/// width of OUT1 / ENABLE1 / IN1 / STATUS1 (`gpio_reg.h`, `_V 0xFF`).
const BANK1_MASK: u32 = 0xFF;

/// Classic-ESP32 matrix OUTPUT signal index → datasheet name, for signals a
/// probe can meaningfully be pointed at (esp-idf `gpio_sig_map.h`). Unmapped
/// indices → `None` (null, never a guess), the same convention the C3 and S3
/// name tables follow.
fn esp32_out_signal_name(idx: u32) -> Option<&'static str> {
    Some(match idx {
        0 => "SPICLK",
        1 => "SPIQ",
        2 => "SPID",
        3 => "SPIHD",
        4 => "SPIWP",
        5 => "SPICS0",
        8 => "HSPICLK",
        9 => "HSPIQ",
        10 => "HSPID",
        11 => "HSPICS0",
        // The routed signals resolve through their named constants, so this
        // table and the bindings below cannot drift apart on an index.
        SIG_U0TXD => "U0TXD",
        SIG_U1TXD => "U1TXD",
        SIG_I2CEXT0_SCL => "I2CEXT0_SCL",
        SIG_I2CEXT0_SDA => "I2CEXT0_SDA",
        SIG_VSPICLK => "VSPICLK",
        SIG_VSPIQ => "VSPIQ", // VSPI MISO: a slave on a net drives it
        SIG_VSPID => "VSPID",
        SIG_VSPICS0 => "VSPICS0",
        SIG_I2CEXT1_SCL => "I2CEXT1_SCL",
        SIG_I2CEXT1_SDA => "I2CEXT1_SDA",
        SIG_U2TXD => "U2TXD",
        idx if (SIG_LEDC_HS0..SIG_LEDC_HS0 + 16).contains(&idx) => {
            LEDC_SIGNAL_NAMES[(idx - SIG_LEDC_HS0) as usize]
        }
        _ => return None,
    })
}

/// Edge-observation contract for anything watching this port's pads.
/// Declared in [`peripherals::device`](crate::peripherals::device) — ONE
/// declaration shared by every GPIO family — and re-exported here so every
/// existing `impl`, bound and intra-doc link at this path keeps resolving.
///
/// It used to be declared separately, byte for byte, in the classic-ESP32
/// and S3 GPIO models. Two identical traits are two types: a component that
/// wanted edges on both families wrote the same `impl` twice, and every
/// call site accepting an observer carried a two-trait bound that no third
/// family could satisfy without a third copy of all of it.
pub use crate::peripherals::device::{GpioObserver, PadSource};

/// ESP32-classic GPIO peripheral.
pub struct Esp32Gpio {
    enable: u32,
    out: u32,
    /// OUT1 / ENABLE1 bank — GPIO32..39, the high pads. Only the low 8 bits are
    /// meaningful. These used to be absent entirely and every write to the bank
    /// was silently dropped, so `digitalWrite(32, HIGH)` on classic ESP32 did
    /// nothing and a sketch reporting through GPIO32..39 looked dead while
    /// running correctly.
    out1: u32,
    enable1: u32,
    in_data: u32,
    /// Bank-1 twin of `in_data`: the level the outside presents to
    /// GPIO32..39, at bit `pad - 32`.
    in_data1: u32,
    /// `GPIO_PINn_REG` for GPIO0..39, masked to the writable bits
    /// ([`PIN_WMASK`]): PAD_DRIVER (open drain), INT_TYPE, WAKEUP_ENABLE,
    /// CONFIG and the five INT_ENA bits. Flat by pad, like the register
    /// array (0x88 + 4 * pad).
    pin_cfg: [u32; PAD_COUNT as usize],
    /// `GPIO_STATUS_REG`: the interrupt status latch of GPIO0..31.
    status: u32,
    /// `GPIO_STATUS1_REG`: the same latch for GPIO32..39 at bit `pad - 32`.
    status1: u32,
    /// GPIO0..31 the outside world holds ([`PinPort::set_external`]); the
    /// level on each is its `in_data` bit. An open-drain pad holding a 1
    /// reads that level instead of its latch.
    ext_mask: u32,
    /// Bank-1 twin of `ext_mask` (GPIO32..39 at bit `pad - 32`).
    ext_mask1: u32,
    /// Matrix-line change detector for the scheduler chain.
    irq_watch: crate::peripherals::esp_gpio_net::IrqLevelWatch,
    /// `GPIO_FUNCn_OUT_SEL_CFG` per pad, flat 0..39 — the output-matrix
    /// selector. Before this existed the register read 0 and every write was
    /// dropped, so the model could not tell a plain-GPIO output from a
    /// peripheral-routed one and NO bus could be published onto a classic pad:
    /// an analyzer clipped to any classic-ESP32 pin read a flat line while the
    /// C3 and S3 worked.
    out_sel: [u32; PAD_COUNT as usize],
    /// `GPIO_FUNCn_IN_SEL_CFG` per peripheral input signal (0..255), masked
    /// to [`FUNC_IN_SEL_WMASK`]. Signal-indexed: one pad can feed several
    /// inputs.
    in_sel: Box<[u32; 256]>,
    /// Peripheral input signals bound to wire lines (`FUNCn_IN_SEL_CFG`).
    in_routes: crate::peripherals::esp_gpio_net::MatrixInputs,
    /// GPIO0..39 on a world `gpio_net` ([`PinPort::join_wire`]), bit = pad.
    net_pads: u64,
    /// Outside levels that reached a pad an input signal reads, for the bus
    /// to hand to the line's peripheral (`Peripheral::take_wire_input_edges`).
    wire_edges: Vec<crate::peripherals::pad_lines::WireInputEdge>,
    /// Pads bound to peripheral wires, resolved against this port's live output
    /// matrix through the ONE shared seam (`peripherals::pad_routing`). Empty —
    /// and free — on a bus with no classic-ESP32 I²C controller.
    pad_routes: PadRoutes,
    /// `Some` while the logic analyzer watches pads on this port in push mode
    /// ([`PinPort::install_watch`]). Not snapshot state: the watch is re-armed
    /// by the frontend after resume.
    ///
    /// Classic ESP32 had NO push instrumentation once, so every probed pad
    /// fell to the per-cycle poll, which is useless for a narrated bus: the
    /// I²C narrator publishes edges stamped in the PAST
    /// (`PadLines::set_line_at`), and a boundary sampler only sees the present.
    watch: Option<crate::pins::PadWatch>,
    cycle: u64,
    /// Phase 2B.3c (issue #192): peripheral-tick index of the last `sync_to`,
    /// for the scheduler path. `cycle` is only an observability timestamp
    /// passed to `GpioObserver::on_pin_change`; no firmware register reads it.
    anchor_tick: u64,
    observers: Vec<Arc<dyn GpioObserver>>,
    /// Last [`PadSource`] reported to the observers, per pad. Only read and
    /// written while an observer is installed.
    pad_sources: [PadSource; PAD_COUNT as usize],
    /// The RTC IO MUX this port shares its RTC-capable pads with
    /// (GPIO0/2/4/12..15/25..27/32..39), handed over by the bus at attach
    /// ([`Peripheral::attach_rtc_io_pads`]). A pad whose `MUX_SEL` is set is
    /// driven by the RTC GPIO, not by `GPIO_OUT` / `GPIO_ENABLE` or the
    /// matrix, and its pull follows `RUE` / `RDE` in either domain (errata
    /// 3.6). See [`crate::peripherals::esp_rtc_io`]. `None`: no RTC_IO on the
    /// bus, every pad digital and unpulled, as before it existed.
    rtc: Option<Arc<crate::peripherals::esp_rtc_io::RtcIoPads>>,
}

impl Esp32Gpio {
    pub fn new() -> Self {
        Self {
            enable: 0,
            out: 0,
            out1: 0,
            enable1: 0,
            in_data: 0,
            in_data1: 0,
            pin_cfg: [0; PAD_COUNT as usize],
            status: 0,
            status1: 0,
            ext_mask: 0,
            ext_mask1: 0,
            irq_watch: Default::default(),
            out_sel: [FUNC_OUT_SEL_RESET; PAD_COUNT as usize],
            in_sel: Box::new([0; 256]),
            in_routes: Default::default(),
            net_pads: 0,
            wire_edges: Vec::new(),
            pad_routes: PadRoutes::new(),
            watch: None,
            cycle: 0,
            anchor_tick: 0,
            observers: Vec::new(),
            pad_sources: [PadSource::Latch; PAD_COUNT as usize],
            rtc: None,
        }
    }

    /// The RTC IO MUX's view of the pads; all-digital without RTC_IO.
    #[inline]
    fn rtc_view(&self) -> crate::peripherals::esp_rtc_io::RtcPadView {
        self.rtc.as_ref().map(|r| r.view()).unwrap_or_default()
    }

    /// `true` when the RTC IO MUX owns `pin` (`MUX_SEL` = 1).
    #[inline]
    fn in_rtc(&self, pin: u8) -> bool {
        self.rtc
            .as_ref()
            .is_some_and(|r| pin < PAD_COUNT && r.rtc_mask() & (1u64 << pin) != 0)
    }

    /// `GPIO_IN` / `GPIO_IN1` of `bank` with the RTC IO MUX applied to the
    /// `word` the digital side reads (`driving` its driving pads): a released
    /// pad nothing outside holds sits at the RTC_IO pull (errata 3.6), and a
    /// pad the RTC IO MUX owns reads what its input buffer passes on
    /// (`FUN_IE`), the digital input path still being wired to the pad (S3 TRM
    /// Figure 6.3-1, classic TRM §6.5.2; see `esp_rtc_io`).
    fn with_rtc(&self, bank: usize, word: u32, driving: u32) -> u32 {
        let v = self.rtc_view();
        if v.is_idle() {
            return word;
        }
        let shift = 32 * bank as u32;
        let width = if bank == 0 { u32::MAX } else { BANK1_MASK };
        let ext = if bank == 0 {
            self.ext_mask
        } else {
            self.ext_mask1
        };
        let up = (v.pull_up >> shift) as u32 & width;
        let down = (v.pull_down >> shift) as u32 & width;
        let floating = !driving & !ext;
        let mut word = (word & !(floating & (up | down))) | (floating & up);
        let mut rtc = (v.rtc >> shift) as u32 & width;
        while rtc != 0 {
            let bit = rtc.trailing_zeros();
            rtc &= rtc - 1;
            let pin = (shift + bit) as u8;
            if v.input(pin, PinPort::external(self, pin), self.input_data(pin)) {
                word |= 1 << bit;
            } else {
                word &= !(1 << bit);
            }
        }
        word
    }

    /// Tell the RTC IO MUX what the pads see from here (the outside's levels,
    /// `GPIO_IN`), for `RTC_GPIO_IN` and its interrupts. After every pad
    /// mutation on this side.
    fn sync_rtc_inputs(&self) {
        let Some(rtc) = &self.rtc else {
            return;
        };
        rtc.set_inputs(crate::peripherals::esp_rtc_io::PadInputs {
            ext_mask: u64::from(self.ext_mask) | (u64::from(self.ext_mask1) << 32),
            stored: u64::from(self.in_data) | (u64::from(self.in_data1) << 32),
            digital: self.pad_levels(),
        });
    }

    pub fn add_observer(&mut self, obs: Arc<dyn GpioObserver>) {
        // Tell the new observer what drives every pad now, so it knows this
        // port reports sources (see `GpioObserver::on_pad_source`).
        for pin in 0..PAD_COUNT {
            obs.on_pad_source(pin, self.pad_source(pin));
        }
        self.observers.push(obs);
        self.pad_sources = std::array::from_fn(|pin| self.pad_source(pin as u8));
    }

    /// What drives pad `pin` now: an LEDC channel the output matrix routes to
    /// it (`LEDC_HS_SIG_OUT0..7` are channels 0..7, `LEDC_LS_SIG_OUT0..7` are
    /// 8..15, the order the LEDC model reports duties in), another peripheral
    /// signal, or the latch.
    fn pad_source(&self, pin: u8) -> PadSource {
        match self.matrix_signal(pin) {
            None | Some(SIG_GPIO_OUT) => PadSource::Latch,
            Some(sig) if (SIG_LEDC_HS0..SIG_LEDC_HS0 + 16).contains(&sig) => {
                PadSource::Ledc((sig - SIG_LEDC_HS0) as u8)
            }
            Some(_) => PadSource::Peripheral,
        }
    }

    /// After a write to ENABLE or `FUNCn_OUT_SEL_CFG`: report every pad whose
    /// source changed.
    fn notify_pad_sources(&mut self) {
        if self.observers.is_empty() {
            return;
        }
        for pin in 0..PAD_COUNT {
            let now = self.pad_source(pin);
            if self.pad_sources[pin as usize] != now {
                self.pad_sources[pin as usize] = now;
                for obs in &self.observers {
                    obs.on_pad_source(pin, now);
                }
            }
        }
    }

    pub fn out_value(&self) -> u32 {
        self.out
    }

    pub fn enable_value(&self) -> u32 {
        self.enable
    }

    /// Set the input level on `pin` (0..=39).
    pub fn set_pin_input(&mut self, pin: u8, level: bool) {
        assert!(pin < PAD_COUNT, "set_pin_input: pin {pin} >= {PAD_COUNT}");
        let before = self.pad_levels();
        let (data, bit) = if pin < 32 {
            (&mut self.in_data, 1u32 << pin)
        } else {
            (&mut self.in_data1, 1u32 << (pin - 32))
        };
        if level {
            *data |= bit;
        } else {
            *data &= !bit;
        }
        self.latch_input_edges(before);
        self.sync_rtc_inputs();
    }

    /// Pads of `bank` (0: GPIO0..31, 1: GPIO32..39) in open drain
    /// (`GPIO_PINn_REG.PAD_DRIVER`), at bank-relative bits.
    fn open_drain(&self, bank: usize) -> u32 {
        let pads = &self.pin_cfg[bank * 32..(bank * 32 + 32).min(PAD_COUNT as usize)];
        pads.iter().enumerate().fold(0u32, |m, (bit, w)| {
            if w & crate::peripherals::esp_gpio_net::PIN_PAD_DRIVER != 0 {
                m | (1 << bit)
            } else {
                m
            }
        })
    }

    /// Bank-0 pads in open drain.
    fn open_drain_bank0(&self) -> u32 {
        self.open_drain(0)
    }

    /// Bank-0 pads whose own output stage decides what `GPIO_IN` reads.
    /// ENABLE, except an open-drain pad holding a 1 that the outside world
    /// holds: it is released and reads that level. With nothing outside, a
    /// released open-drain pad keeps reading its latch, standing in for the
    /// board pull-up this model has no IO_MUX for.
    fn driving_bank0(&self) -> u32 {
        crate::peripherals::esp_gpio_net::driving_mask(
            self.enable,
            self.out,
            self.open_drain_bank0() & self.ext_mask,
        )
    }

    /// Both banks' pad levels as one word, bit = pad: `GPIO_IN` in the low
    /// half, `GPIO_IN1` above it.
    fn pad_levels(&self) -> u64 {
        u64::from(self.pad_level_bank0()) | (u64::from(self.pad_level_bank1()) << 32)
    }

    /// Latch `GPIO_STATUS` / `GPIO_STATUS1` for every pin whose `GPIO_IN` /
    /// `GPIO_IN1` bit moved from `before` (a [`Self::pad_levels`] word; edge
    /// types) and for every level-type pin whose level holds. The interrupt
    /// sees the pad as `GPIO_IN` reports it, so a pad's own output edges
    /// count too, as on silicon with the input buffer enabled.
    fn latch_input_edges(&mut self, before: u64) {
        use crate::peripherals::esp_gpio_net::{edge_hits, int_type, level_hits};
        let after = self.pad_levels();
        for pin in 0..PAD_COUNT as usize {
            let kind = int_type(self.pin_cfg[pin]);
            if kind == 0 {
                continue;
            }
            let (b, a) = ((before >> pin) & 1 != 0, (after >> pin) & 1 != 0);
            if edge_hits(kind, b, a) || level_hits(kind, a) {
                if pin < 32 {
                    self.status |= 1 << pin;
                } else {
                    self.status1 |= 1 << (pin - 32);
                }
            }
        }
    }

    /// `GPIO_ACPU_INT_REG` (APP CPU, INT_ENA bit 0 = register bit 13) and
    /// `GPIO_PCPU_INT_REG` (PRO CPU, INT_ENA bit 2 = register bit 15) — the
    /// esp-idf `GPIO_LL_APP_CPU_INTR_ENA` / `GPIO_LL_PRO_CPU_INTR_ENA` bits —
    /// for `bank` 0 (`STATUS`), or their `..._INT1_REG` twins for bank 1
    /// (`STATUS1`, bit `pad - 32`).
    fn cpu_int_bank(&self, bank: usize, ena_bit: u32) -> u32 {
        let mut out = 0;
        let mut pending = if bank == 0 { self.status } else { self.status1 };
        while pending != 0 {
            let bit = pending.trailing_zeros() as usize;
            pending &= pending - 1;
            if self.pin_cfg[bank * 32 + bit] & (1 << ena_bit) != 0 {
                out |= 1 << bit;
            }
        }
        out
    }

    fn cpu_int(&self, ena_bit: u32) -> u32 {
        self.cpu_int_bank(0, ena_bit)
    }

    fn cpu_int1(&self, ena_bit: u32) -> u32 {
        self.cpu_int_bank(1, ena_bit)
    }

    /// The `ETS_GPIO_INTR_SOURCE` matrix line: either CPU's GPIO interrupt,
    /// from either bank (one source serves all 40 pads).
    fn irq_line(&self) -> bool {
        self.cpu_int(13) | self.cpu_int(15) | self.cpu_int1(13) | self.cpu_int1(15) != 0
    }

    /// Level at the pads of bank 0 (GPIO0..31) — what `GPIO_IN` reports.
    ///
    /// `IN` is the pad, not the external stimulus. A pin whose output driver is
    /// enabled reads back the level it is DRIVING; only a pin left as an input
    /// reports what the outside world put there. Reading `in_data` alone made
    /// `digitalRead()` on a pin the firmware had just driven return 0 forever,
    /// so the common "set it, then confirm it" idiom could never pass — the
    /// firmware was correct and the model said no.
    ///
    /// If a pin is both driven and externally forced, the output driver wins
    /// here. Real silicon has a contention whose winner depends on drive
    /// strength; we do not model that, and taking the driver is the case that
    /// matches a correctly wired board.
    fn pad_level_bank0(&self) -> u32 {
        let driving = if self.ext_mask == 0 {
            self.enable
        } else {
            self.driving_bank0()
        };
        let word = (self.out & driving) | (self.in_data & !driving);
        if self.rtc.is_none() {
            return word;
        }
        self.with_rtc(0, word, driving)
    }

    /// ENABLE1 as far as an output stage exists behind it: GPIO34..39 are
    /// input-only, so their ENABLE1 bits store and drive nothing.
    fn enable1_driving(&self) -> u32 {
        self.enable1 & OUTPUT_PADS_BANK1
    }

    /// Bank 1 twin of [`pad_level_bank0`] for the GPIO32..39 pads, where bit 0
    /// is GPIO32 — what `GPIO_IN1` reports. Same rule: an enabled output
    /// reads the level it drives (an open-drain 1 the outside holds is
    /// released), anything else reads what the outside presents. GPIO34..39
    /// never drive (see [`OUTPUT_PADS_BANK1`]).
    fn pad_level_bank1(&self) -> u32 {
        let driving = crate::peripherals::esp_gpio_net::driving_mask(
            self.enable1_driving(),
            self.out1,
            self.open_drain(1) & self.ext_mask1,
        );
        let word = ((self.out1 & driving) | (self.in_data1 & !driving)) & BANK1_MASK;
        if self.rtc.is_none() {
            return word;
        }
        self.with_rtc(1, word, driving)
    }

    /// `true` when pad `pin`'s output driver is enabled.
    ///
    /// The ONE place the classic bank split is decided: ENABLE holds GPIO0..31,
    /// ENABLE1 holds GPIO32..39 at bit `pin - 32`. Everything above this — the
    /// matrix selector, the pad level, the routing report — indexes pads flat,
    /// because the matrix itself does. An input-only pad (GPIO34..39) has no
    /// driver to enable.
    fn output_driver_on(&self, pin: u8) -> bool {
        if pin < 32 {
            (self.enable & (1u32 << pin)) != 0
        } else if pin < PAD_COUNT {
            (self.enable1_driving() & (1u32 << (pin - 32))) != 0
        } else {
            false
        }
    }

    /// The output-matrix signal `pin` currently carries — the selector the
    /// shared routing seam resolves bindings against.
    ///
    /// `None` unless the pad's output driver is enabled, because a pad that is
    /// not driving shows its input level, not the peripheral's wire. That
    /// condition lives in the selector rather than in each binding so ONE rule
    /// covers pad reads, `gpio_routing` and push registration alike.
    ///
    /// ⚠️ Deliberate approximation: with `OEN_SEL = 0` real silicon takes the
    /// output enable from the PERIPHERAL, not from `GPIO_ENABLE`, so a
    /// matrix-routed pad can drive with ENABLE clear. Per-peripheral OEN is not
    /// modelled, and every ESP-IDF/Arduino path that routes a pad sets the
    /// direction first (`i2cInit` does `pinMode(sda, OUTPUT_OPEN_DRAIN)` before
    /// `pinMatrixOutAttach`). If a lab ever shows a flat line while the matrix
    /// IS programmed, this gate is where to look.
    ///
    /// A pad the RTC IO MUX owns carries no matrix signal either: with
    /// `MUX_SEL` set the pad's output comes from the RTC GPIO, so the matrix
    /// route stays programmed but dark until `MUX_SEL` clears.
    fn matrix_signal(&self, pin: u8) -> Option<u32> {
        if pin >= PAD_COUNT || !self.output_driver_on(pin) || self.in_rtc(pin) {
            return None;
        }
        Some(self.out_sel[pin as usize] & OUT_SEL_MASK)
    }

    /// Direction- and matrix-aware pad level — the single truth `read_gpio_pad`
    /// and the push tap both read, across both banks.
    ///
    /// A pad the matrix has handed to a peripheral is driven by that
    /// peripheral's wire, not by the GPIO_OUT latch, so the wire is consulted
    /// FIRST. With no live route the fallback is byte-for-byte the pre-existing
    /// bank expressions, so an unrouted pad reads exactly what it read before
    /// this seam existed.
    fn pad_level(&self, pin: u8) -> Option<bool> {
        if pin >= PAD_COUNT {
            return None;
        }
        // A pad the RTC IO MUX owns: its RTC output stage against the
        // outside. The wire, not `GPIO_IN`: a probe sees a driven pad whose
        // `FUN_IE` is off.
        if self.in_rtc(pin) {
            return Some(self.rtc_view().wire_level(
                pin,
                PinPort::external(self, pin),
                self.input_data(pin),
            ));
        }
        // A routed line its peripheral does not drive right now (released
        // open drain, an SPI slave's MISO while deselected) leaves the pad
        // to the outside, as `GPIO_IN` reads it.
        if let Some((cell, line)) = self.pad_routes.active_line(pin, |p| self.matrix_signal(p)) {
            if cell.drives(line) {
                return Some(cell.level(line));
            }
            return Some(self.input_data(pin));
        }
        if pin < 32 {
            Some((self.pad_level_bank0() & (1u32 << pin)) != 0)
        } else {
            Some((self.pad_level_bank1() & (1u32 << (pin - 32))) != 0)
        }
    }

    /// Bind an I²C controller's wire to every output-capable pad the matrix can
    /// route it to. Called once at bus wiring time; which pad is live at any
    /// moment is then decided by `FUNCn_OUT_SEL`, through the shared seam.
    /// `signals` names the controller: [`I2C0_SIGNALS`] or [`I2C1_SIGNALS`].
    pub(crate) fn bind_i2c_lines(
        &mut self,
        lines: &Arc<crate::peripherals::pad_lines::PadLines>,
        signals: I2cSignals,
    ) {
        use crate::peripherals::esp32::i2c::{LINE_SCL, LINE_SDA};
        for pin in 0..PAD_COUNT {
            if VALID_OUTPUT_PADS & (1u64 << pin) == 0 {
                continue;
            }
            self.pad_routes
                .bind(lines, pin, Some(signals.scl), LINE_SCL, signals.names[0]);
            self.pad_routes
                .bind(lines, pin, Some(signals.sda), LINE_SDA, signals.names[1]);
        }
        // The controller reads both lines back through the input matrix
        // (open drain: it sees the wire it drives, and the other chips). Any
        // pad can feed an input signal, the input-only GPIO34..39 included.
        self.in_routes.bind(signals.scl, lines, LINE_SCL);
        self.in_routes.bind(signals.sda, lines, LINE_SDA);
    }

    /// Bind the VSPI (SPI3) master's SCK/MOSI/CS wire to every output-capable
    /// pad the matrix can route it to. Which pad is live at any moment is then
    /// decided by `FUNCn_OUT_SEL`, through the shared seam.
    ///
    /// Unlike the RP2040 there is no pad table to transcribe: the ESP32 GPIO
    /// matrix routes ANY peripheral signal to ANY output-capable pad, so every
    /// pad is bound to all three signals and the selector picks the live one.
    ///
    /// A cell with a MISO line (a controller that can be a master or a slave
    /// on a world `gpio_net`, `esp_gpspi_net`) also carries MISO out of a
    /// slave, and every line in through `FUNCn_IN_SEL_CFG`: SCK, MOSI and CS
    /// for a slave, MISO for a master. MISO starts as an input line, so a
    /// routed pad shows what the outside holds there, never a made-up idle
    /// level; see [`crate::peripherals::esp_gpspi_wire`].
    ///
    /// `signals` names the controller: [`VSPI_SIGNALS`] or [`HSPI_SIGNALS`].
    pub(crate) fn bind_spi_lines(
        &mut self,
        lines: &Arc<crate::peripherals::pad_lines::PadLines>,
        signals: SpiSignals,
    ) {
        use crate::peripherals::esp_gpspi_wire::{LINE_CS, LINE_MISO, LINE_MOSI, LINE_SCK};
        let with_miso = lines.names().len() > LINE_MISO;
        let line_of = [LINE_SCK, LINE_MOSI, LINE_CS, LINE_MISO];
        let count = if with_miso { 4 } else { 3 };
        for pin in 0..PAD_COUNT {
            if VALID_OUTPUT_PADS & (1u64 << pin) == 0 {
                continue;
            }
            for ((&sig, &line), &name) in signals
                .sigs
                .iter()
                .zip(&line_of)
                .zip(&signals.names)
                .take(count)
            {
                self.pad_routes.bind(lines, pin, Some(sig), line, name);
            }
        }
        if with_miso {
            for (&sig, &line) in signals.sigs.iter().zip(&line_of) {
                self.in_routes.bind(sig, lines, line);
            }
        }
    }

    /// `FUNCn_IN_SEL_CFG` of input signal `sig`.
    fn in_sel_word(&self, sig: u32) -> u32 {
        self.in_sel.get(sig as usize).copied().unwrap_or(0)
    }

    /// The level the outside presents to pad `pin` (GPIO0..39) as stored,
    /// whether or not anything holds it: the `in_data` / `in_data1` bit.
    fn input_data(&self, pin: u8) -> bool {
        if pin < 32 {
            self.in_data & (1u32 << pin) != 0
        } else {
            pin < PAD_COUNT && self.in_data1 & (1u32 << (pin - 32)) != 0
        }
    }

    /// The level the outside holds on pad `pin` (GPIO0..39), if any. Bank 1
    /// is bit `pin - 32` of `ext_mask1` / `in_data1`.
    fn external_level(&self, pin: u8) -> Option<bool> {
        let held = if pin < 32 {
            self.ext_mask & (1u32 << pin) != 0
        } else {
            pin < PAD_COUNT && self.ext_mask1 & (1u32 << (pin - 32)) != 0
        };
        held.then(|| self.input_data(pin))
    }

    /// After a routing write or a pad joining a net: every peripheral input
    /// takes the outside level of the pad it now reads, and every wire line
    /// routed (in or out) to a net pad becomes a net line.
    fn sync_net_routes(&mut self) {
        if self.in_routes.is_empty() && self.net_pads == 0 {
            return;
        }
        let mut edges = Vec::new();
        {
            let in_sel = |sig: u32| self.in_sel_word(sig);
            self.in_routes.sync(
                in_sel,
                IN_SEL_LAYOUT,
                |pin| self.external_level(pin),
                &mut edges,
            );
            if self.net_pads != 0 {
                let net = self.net_pads;
                self.in_routes.mark_on_net(in_sel, IN_SEL_LAYOUT, |pin| {
                    pin < PAD_COUNT && net & (1u64 << pin) != 0
                });
                for pin in 0..PAD_COUNT {
                    if net & (1u64 << pin) == 0 {
                        continue;
                    }
                    if let Some((cell, line)) =
                        self.pad_routes.active_line(pin, |p| self.matrix_signal(p))
                    {
                        if !cell.on_net(line) {
                            cell.mark_on_net(line);
                        }
                    }
                }
            }
        }
        // Only a world net delivers these; off one they would pile up.
        if self.net_pads != 0 {
            self.wire_edges.extend(edges);
        }
    }

    /// Hand an outside level on `pin` to every input signal reading it.
    fn deliver_wire_input(&mut self, pin: u8) {
        if self.in_routes.is_empty() || self.net_pads == 0 {
            return;
        }
        let Some(level) = self.external_level(pin) else {
            return;
        };
        let mut edges = Vec::new();
        self.in_routes.deliver(
            pin,
            level,
            |sig| self.in_sel_word(sig),
            IN_SEL_LAYOUT,
            &mut edges,
        );
        self.wire_edges.extend(edges);
    }

    /// Bind one UART's TX wire to every output-capable pad the matrix can route
    /// it to.
    ///
    /// TX ONLY. Nothing in the engine drives the RX line, so a bound RX pad
    /// would report a confident constant idle-high — including while an attached
    /// GPS or modem was actually sending. Same call `wire_rp2040_uart_pads`
    /// documents. RX joins the table when something drives it, not before.
    ///
    /// See [`SIG_U0TXD`] for why a stock default-pin `Serial.begin()` correctly
    /// leaves every one of these routes dark on this part.
    pub(crate) fn bind_uart_tx_lines(
        &mut self,
        instance: usize,
        lines: &Arc<crate::peripherals::pad_lines::PadLines>,
    ) {
        let (signal, func) = match instance {
            0 => (SIG_U0TXD, "UART0_TX"),
            1 => (SIG_U1TXD, "UART1_TX"),
            // Classic ESP32 has exactly three UARTs (`SOC_UART_NUM` = 3).
            2 => (SIG_U2TXD, "UART2_TX"),
            _ => return,
        };
        for pin in 0..PAD_COUNT {
            if VALID_OUTPUT_PADS & (1u64 << pin) == 0 {
                continue;
            }
            self.pad_routes.bind(
                lines,
                pin,
                Some(signal),
                crate::peripherals::uart::LINE_TX,
                func,
            );
        }
    }

    /// Bind the LEDC controller's 16 channel outputs to every output-capable
    /// pad the matrix can route them to — the classic `ledcAttachPin` path,
    /// which programs `FUNCn_OUT_SEL = LEDC_HS_SIG_OUT0_IDX + channel`. Which
    /// pad is live at any moment is then decided by `FUNCn_OUT_SEL`, through
    /// the shared seam.
    pub(crate) fn bind_ledc_lines(&mut self, lines: &Arc<crate::peripherals::pad_lines::PadLines>) {
        for pin in 0..PAD_COUNT {
            if VALID_OUTPUT_PADS & (1u64 << pin) == 0 {
                continue;
            }
            for (line, func) in LEDC_SIGNAL_NAMES.iter().enumerate() {
                self.pad_routes
                    .bind(lines, pin, Some(SIG_LEDC_HS0 + line as u32), line, func);
            }
        }
    }

    /// Every signal name bound to this port's pads, live or not — the
    /// bus-visibility reporting seam. See
    /// [`crate::peripherals::pad_routing::PadRoutes::bound_functions`] for why
    /// this is the static question and `func()` is the live one.
    pub(crate) fn bound_pad_functions(&self) -> Vec<&'static str> {
        self.pad_routes.bound_functions()
    }

    /// Word index into `out_sel` for a register offset, or `None` outside the
    /// `FUNC0..39_OUT_SEL_CFG` array.
    fn out_sel_index(off: u64) -> Option<usize> {
        (FUNC_OUT_SEL..FUNC_OUT_SEL_END)
            .contains(&off)
            .then(|| ((off - FUNC_OUT_SEL) / 4) as usize)
    }

    /// Run a pad mutation bracketed for push capture.
    #[inline]
    fn mutate_pads<R>(&mut self, mutate: impl FnOnce(&mut Self) -> R) -> R {
        crate::pins::watch_begin(self);
        let r = mutate(self);
        crate::pins::watch_end(self);
        r
    }

    /// The pad's own output stage ([`PinPort::driver`]). A pad the matrix
    /// hands to a peripheral that publishes its wire is driven by it; one
    /// handed to a peripheral that publishes nothing has no known drive
    /// (`None`), which is what makes a world refuse it. Otherwise ENABLE
    /// drives the latch, and an open-drain (`PAD_DRIVER`) 1 is released.
    /// The IO_MUX pulls are not modelled on this part: no pull.
    ///
    /// A pad the RTC IO MUX owns is driven by the RTC GPIO alone. The pull of
    /// an RTC-capable pad is its RTC_IO `RUE` / `RDE` in either domain
    /// (errata 3.6); every other pad has none.
    fn pad_driver(&self, pin: u8) -> Option<crate::pins::PadDriver> {
        use crate::pins::PadDriver;
        if pin >= PAD_COUNT {
            return None;
        }
        let rtc = self.rtc_view();
        if rtc.in_rtc(pin) {
            return Some(rtc.driver(pin));
        }
        let pull = rtc.pull(pin);
        if let Some(sig) = self.matrix_signal(pin) {
            if sig != SIG_GPIO_OUT {
                // The peripheral says what its output stage does: a line it
                // drives drives the pad; a released or input line does not.
                return self
                    .pad_routes
                    .active_line(pin, |p| self.matrix_signal(p))
                    .map(|(cell, line)| {
                        if cell.drives(line) {
                            PadDriver::drive(cell.level(line)).with_pull(pull)
                        } else {
                            PadDriver::released(pull)
                        }
                    });
            }
        }
        if !self.output_driver_on(pin) {
            return Some(PadDriver::released(pull));
        }
        let latch = if pin < 32 {
            self.out & (1 << pin) != 0
        } else {
            self.out1 & (1 << (pin - 32)) != 0
        };
        Some(
            PadDriver::output(
                latch,
                self.pin_cfg[pin as usize] & crate::peripherals::esp_gpio_net::PIN_PAD_DRIVER != 0,
            )
            .with_pull(pull),
        )
    }

    /// Re-register watched pads with the wires that drive them, so a pad the
    /// matrix hands over (or takes back) follows its new source immediately.
    fn sync_line_taps(&mut self) {
        if self.pad_routes.is_empty() {
            return;
        }
        let Some(watch) = self.watch.take() else {
            return;
        };
        let mut routes = std::mem::take(&mut self.pad_routes);
        routes.sync_taps(watch.tap(), watch.pairs(), |pin| self.matrix_signal(pin));
        self.pad_routes = routes;
        self.watch = Some(watch);
    }

    fn apply_out(&mut self, new_out: u32) {
        let old = self.out;
        self.out = new_out;
        let diff = old ^ new_out;
        if diff == 0 {
            return;
        }
        for pin in 0u8..32 {
            let mask = 1u32 << pin;
            if diff & mask != 0 {
                let from = old & mask != 0;
                let to = new_out & mask != 0;
                for obs in &self.observers {
                    obs.on_pin_change(pin, from, to, self.cycle);
                }
            }
        }
    }

    /// OUT1 twin of [`apply_out`], for the GPIO32..39 pads. Bit 0 of `out1` is
    /// GPIO32, so the observer sees the real pad number and a
    /// `digitalWrite(32, ...)` is visible to anything watching pin 32.
    fn apply_out1(&mut self, new_out1: u32) {
        let old = self.out1;
        self.out1 = new_out1;
        let diff = old ^ new_out1;
        if diff == 0 {
            return;
        }
        for bit in 0u8..8 {
            let mask = 1u32 << bit;
            if diff & mask != 0 {
                let from = old & mask != 0;
                let to = new_out1 & mask != 0;
                for obs in &self.observers {
                    obs.on_pin_change(32 + bit, from, to, self.cycle);
                }
            }
        }
    }

    fn read_word(&self, word_off: u64) -> u32 {
        match word_off {
            // OUT bank (GPIO0..31): TRM Table 4-3.
            0x04 => self.out,
            0x08 => self.out,
            0x0C => self.out,
            // OUT1 bank (GPIO32..39): same three-alias shape as OUT.
            0x10 | 0x14 | 0x18 => self.out1,
            // ENABLE bank (GPIO0..31).
            0x20 => self.enable,
            0x24 => self.enable,
            0x28 => self.enable,
            // ENABLE1 bank (GPIO32..39).
            0x2C | 0x30 | 0x34 => self.enable1,
            // STRAP register (TRM §4.10.4). Boot strap latch read by the
            // BROM to pick boot mode. We return 0x33 to emulate a stock
            // WROOM-32: GPIO0=1 (SPI flash boot), GPIO2=1 (don't care),
            // GPIO4=0, GPIO5=1, GPIO12=1 (1.8V flash select), GPIO15=0.
            // Concretely we just need GPIO0=1 so the BROM doesn't fall
            // into DOWNLOAD_BOOT and wait on UART/SDIO forever.
            0x38 => 0x33,
            // IN (GPIO0..31) — the pad level, so a driven output reads back.
            0x3C => self.pad_level_bank0(),
            // IN1 (GPIO32..39), same rule for the high bank.
            0x40 => self.pad_level_bank1(),
            // STATUS and its W1TS/W1TC views (GPIO0..31), STATUS1 and its
            // views (GPIO32..39).
            0x44 | 0x48 | 0x4C => self.status,
            0x50 | 0x54 | 0x58 => self.status1,
            // ACPU_INT, ACPU_NMI_INT, PCPU_INT, PCPU_NMI_INT, CPUSDIO_INT.
            0x60 => self.cpu_int(13),
            0x64 => self.cpu_int(14),
            0x68 => self.cpu_int(15),
            0x6C => self.cpu_int(16),
            0x70 => self.cpu_int(17),
            // Their bank-1 twins: ACPU_INT1 .. CPUSDIO_INT1 (gpio_reg.h).
            0x74 => self.cpu_int1(13),
            0x78 => self.cpu_int1(14),
            0x7C => self.cpu_int1(15),
            0x80 => self.cpu_int1(16),
            0x84 => self.cpu_int1(17),
            // GPIO_PINn_REG at 0x88 + pin*4, GPIO0..39 (TRM Table 4-12):
            // PAD_DRIVER (2), INT_TYPE [9:7], WAKEUP_ENABLE (10), CONFIG
            // [12:11], INT_ENA [17:13].
            off if (PIN0..PIN_END).contains(&off) => self.pin_cfg[((off - PIN0) / 4) as usize],
            // GPIO_FUNC0..39_OUT_SEL_CFG at 0x530 + pad*4 (gpio_reg.h). These
            // read 0 and dropped every write before this, which is exactly why
            // no bus could be published onto a classic-ESP32 pad.
            off if Self::out_sel_index(off).is_some() => {
                self.out_sel[Self::out_sel_index(off).expect("guarded by the match")]
            }
            // GPIO_FUNC0..255_IN_SEL_CFG at 0x130 + signal*4.
            off if (FUNC_IN_SEL..FUNC_IN_SEL_END).contains(&off) => {
                self.in_sel[((off - FUNC_IN_SEL) / 4) as usize]
            }
            _ => {
                crate::census_reg!("esp32.gpio:Esp32Gpio", word_off, "read");
                0
            }
        }
    }

    /// Store one register word, then latch every GPIO interrupt the store
    /// caused (an output edge on a pad, a level-type pin re-armed by a
    /// `STATUS_W1TC` or a `PINn` write).
    fn write_word(&mut self, word_off: u64, value: u32) {
        let before = self.pad_levels();
        self.write_word_inner(word_off, value);
        self.latch_input_edges(before);
        // A routing write may have moved an input signal to another pad, or
        // handed a net pad to a peripheral line.
        if matches!(word_off, 0x20..=0x34 | FUNC_IN_SEL..FUNC_OUT_SEL_END) {
            self.sync_net_routes();
            self.notify_pad_sources();
        }
        self.sync_rtc_inputs();
    }

    fn write_word_inner(&mut self, word_off: u64, value: u32) {
        match word_off {
            0x04 => self.apply_out(value),
            0x08 => {
                let new = self.out | value;
                self.apply_out(new);
            }
            0x0C => {
                let new = self.out & !value;
                self.apply_out(new);
            }
            0x10 => self.apply_out1(value),
            0x14 => {
                let new = self.out1 | value;
                self.apply_out1(new);
            }
            0x18 => {
                let new = self.out1 & !value;
                self.apply_out1(new);
            }
            0x20 => self.enable = value,
            0x24 => self.enable |= value,
            0x28 => self.enable &= !value,
            0x2C => self.enable1 = value,
            0x30 => self.enable1 |= value,
            0x34 => self.enable1 &= !value,
            // STRAP / IN registers are read-only.
            0x38 | 0x3C | 0x40 => {}
            0x44 => self.status = value,
            0x48 => self.status |= value,
            // W1TC: a level-type pin whose level still holds re-latches at
            // once (`write_word` re-evaluates after every store).
            0x4C => self.status &= !value,
            0x50 => self.status1 = value & BANK1_MASK,
            0x54 => self.status1 |= value & BANK1_MASK,
            0x58 => self.status1 &= !value,
            off if (PIN0..PIN_END).contains(&off) => {
                let pin = ((off - PIN0) / 4) as usize;
                self.pin_cfg[pin] = value & PIN_WMASK;
            }
            // GPIO_FUNC0..39_OUT_SEL_CFG. Masked store: only OUT_SEL[8:0],
            // INV_SEL(9), OEN_SEL(10) and OEN_INV_SEL(11) exist, and storing
            // the reserved bits would let a debugger read back invented state.
            //
            // The BROM's `gpio_matrix_out` (0x40009f0c — NOT one of the thunks
            // this engine overrides) writes this as a read-modify-write, which
            // is why the reset sentinel is overwritten rather than OR-ed:
            // OUT_SEL spans the same bits.
            off if Self::out_sel_index(off).is_some() => {
                let idx = Self::out_sel_index(off).expect("guarded by the match");
                self.out_sel[idx] = value & FUNC_OUT_SEL_WMASK;
            }
            off if (FUNC_IN_SEL..FUNC_IN_SEL_END).contains(&off) => {
                self.in_sel[((off - FUNC_IN_SEL) / 4) as usize] = value & FUNC_IN_SEL_WMASK;
            }
            _ => {
                crate::census_reg!("esp32.gpio:Esp32Gpio", word_off, "write");
            }
        }
    }
}

impl Default for Esp32Gpio {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Esp32Gpio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32Gpio(enable=0x{:08x} out=0x{:08x} in=0x{:08x} cycle={} obs={})",
            self.enable,
            self.out,
            self.in_data,
            self.cycle,
            self.observers.len(),
        )
    }
}

impl PinPort for Esp32Gpio {
    fn pin_count(&self) -> u8 {
        PAD_COUNT
    }

    fn driver(&self, pin: u8) -> Option<crate::pins::PadDriver> {
        self.pad_driver(pin)
    }

    fn external(&self, pin: u8) -> External {
        match self.external_level(pin) {
            Some(level) => External::Level(level),
            None => External::Released,
        }
    }

    /// GPIO0..39 take input, the input-only GPIO34..39 included: a level
    /// lands in `in_data` (GPIO0..31) or `in_data1` (GPIO32..39, bit
    /// `pin - 32`); releasing leaves it there and hands a released
    /// open-drain pad back to its latch. Every change of `GPIO_IN` /
    /// `GPIO_IN1` latches `GPIO_STATUS` / `GPIO_STATUS1` per `INT_TYPE`.
    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        if pin >= PAD_COUNT {
            return None;
        }
        let pad = 1u64 << pin;
        let before = self.pad_levels();
        self.mutate_pads(|s| {
            let (mask, data, bit) = if pin < 32 {
                (&mut s.ext_mask, &mut s.in_data, 1u32 << pin)
            } else {
                (&mut s.ext_mask1, &mut s.in_data1, 1u32 << (pin - 32))
            };
            match ext {
                External::Level(level) => {
                    *mask |= bit;
                    if level {
                        *data |= bit;
                    } else {
                        *data &= !bit;
                    }
                }
                External::Released => *mask &= !bit,
            }
            s.latch_input_edges(before);
            s.sync_rtc_inputs();
        });
        self.deliver_wire_input(pin);
        Some(InputChange {
            before: before & pad != 0,
            after: self.pad_levels() & pad != 0,
        })
    }

    /// `pin` joined a world `gpio_net`: a wire line routed to it (now, or
    /// whenever firmware routes one) shares a wire with other chips.
    fn join_wire(&mut self, pin: u8) {
        if pin < PAD_COUNT {
            self.net_pads |= 1u64 << pin;
            crate::pins::watch_begin(self);
            self.sync_net_routes();
            crate::pins::watch_end(self);
        }
    }

    /// `GPIO_IN` / `GPIO_IN1`: the pad level, so a driven output reads back.
    fn input(&self, pin: u8) -> Option<bool> {
        if pin < 32 {
            Some(self.pad_level_bank0() & (1 << pin) != 0)
        } else if pin < PAD_COUNT {
            Some(self.pad_level_bank1() & (1 << (pin - 32)) != 0)
        } else {
            None
        }
    }

    /// ONE definition of "pad level" — see `pad_level`. A caller here and
    /// firmware reading GPIO_IN / GPIO_IN1 cannot disagree, and a pad the
    /// output matrix handed to a peripheral reads that peripheral's wire
    /// instead of the idle GPIO_OUT latch.
    fn level(&self, pin: u8) -> Option<bool> {
        self.pad_level(pin)
    }

    fn install_watch(&mut self, watch: Option<crate::pins::PadWatch>) -> bool {
        match watch {
            None => {
                self.watch = None;
                self.pad_routes.clear_taps();
            }
            Some(watch) => {
                self.watch = Some(watch);
                // Seeded stale so the sync below always installs the CURRENT
                // routing into the wire cell.
                self.pad_routes.invalidate_registrations();
                self.sync_line_taps();
            }
        }
        true
    }

    fn take_watch(&mut self) -> Option<crate::pins::PadWatch> {
        self.watch.take()
    }

    fn put_watch(&mut self, watch: crate::pins::PadWatch) {
        self.watch = Some(watch);
    }

    fn routes_changed(&mut self) {
        self.sync_line_taps();
    }
}

impl Peripheral for Esp32Gpio {
    /// Name the output/enable banks so the universal inspect wall decodes them.
    ///
    /// Without this the block reported ZERO registers, which is why a final-state
    /// `gpio` oracle clause had nothing to resolve against on classic ESP32: the
    /// evaluator's fallback derives a pin's level from a named register, and
    /// there was none. Naming OUT / OUT1 is what makes "did the firmware drive
    /// this pad high?" answerable without a `--watch-gpio` capture.
    fn describe_registers(&self) -> Option<Vec<crate::inspect::RegisterSchema>> {
        use crate::inspect::RegisterSchema;
        let reg = |name: &str, offset: u64| RegisterSchema {
            name: name.to_string(),
            offset,
            size: 32,
            access: "rw",
            fields: Vec::new(),
        };
        Some(vec![
            reg("OUT", 0x04),
            reg("OUT_W1TS", 0x08),
            reg("OUT_W1TC", 0x0C),
            reg("OUT1", 0x10),
            reg("OUT1_W1TS", 0x14),
            reg("OUT1_W1TC", 0x18),
            reg("ENABLE", 0x20),
            reg("ENABLE1", 0x2C),
            reg("IN", 0x3C),
        ])
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    /// Side-effect-free read for the inspect wall.
    ///
    /// The trait default returns `None`, which made every register in
    /// `describe_registers` decode as 0x00000000 — a schema that looked
    /// authoritative and reported nothing. `read` here is already `&self` and
    /// pure, so peek is exactly it.
    fn peek(&self, offset: u64) -> Option<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Some(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.mutate_pads(|s| s.write_word(word_off, word));
        Ok(())
    }

    /// Word-granular writes MUST go straight to `write_word` — the W1TS (0x08)
    /// and W1TC (0x0C) registers are write-1-to-set / write-1-to-clear, not
    /// plain storage. The default byte-split path read-modifies-writes against
    /// `read_word`, which returns `self.out` for those offsets, so a 32-bit
    /// `digitalWrite(pin, LOW)` (W1TC = 1<<pin) would reconstruct a clear-mask
    /// from the *current* OUT value and wipe every set bit (not just `pin`).
    /// Real ESP32 GPIO drivers always issue full 32-bit `s32i` stores here.
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset & 3 == 0 {
            self.mutate_pads(|s| s.write_word(offset, value));
            Ok(())
        } else {
            for i in 0..4 {
                self.write(offset + i, ((value >> (i * 8)) & 0xFF) as u8)?;
            }
            Ok(())
        }
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        // 16-bit stores to a W1TS/W1TC half-word carry the same hazard; route
        // aligned ones straight to write_word with the upper half preserved.
        if offset & 3 == 0 {
            let cur = self.read_word(offset) & 0xFFFF_0000;
            self.mutate_pads(|s| s.write_word(offset, cur | value as u32));
            Ok(())
        } else {
            self.write(offset, (value & 0xFF) as u8)?;
            self.write(offset + 1, (value >> 8) as u8)
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        // Keep the public snapshot compact and human-readable. Browser board_io
        // uses the GPIO capability methods below, not these field names.
        serde_json::json!({
            "layout": "esp32_classic",
            "odr": self.out,
            "idr": self.in_data,
            "enable": self.enable,
        })
    }

    /// What the outside presents to the pad: the level it holds, else the
    /// RTC_IO pull of an RTC-capable pad, else the last level it presented.
    /// A pad the RTC IO MUX owns reads through its `FUN_IE`, as `GPIO_IN`.
    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        use crate::pins::Pull;
        if pin >= PAD_COUNT {
            return None;
        }
        if self.in_rtc(pin) {
            return PinPort::input(self, pin);
        }
        Some(
            self.external_level(pin)
                .unwrap_or_else(|| match self.rtc_view().pull(pin) {
                    Pull::Up => true,
                    Pull::Down => false,
                    Pull::None => self.input_data(pin),
                }),
        )
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        if pin < 32 {
            Some((self.out & (1u32 << pin)) != 0)
        } else if pin < PAD_COUNT {
            Some((self.out1 & (1u32 << (pin - 32))) != 0)
        } else {
            None
        }
    }

    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        Some(self)
    }

    fn gpio_routing(&self, pin: u8) -> Option<crate::peripherals::gpio::GpioRouting> {
        use crate::peripherals::gpio::{GpioMode, GpioRouting};
        if pin >= PAD_COUNT {
            return None;
        }
        // A pad the RTC IO MUX owns is an RTC GPIO, whatever GPIO_ENABLE and
        // the matrix say.
        if let Some(rtc) = self.rtc.as_ref().filter(|_| self.in_rtc(pin)) {
            let v = rtc.view();
            return Some(GpioRouting {
                mode: if v.drive & (1u64 << pin) != 0 {
                    GpioMode::Output
                } else {
                    GpioMode::Input
                },
                func: rtc.rtc_channel(pin).map(|n| format!("RTC_GPIO{n}")),
            });
        }
        if !self.output_driver_on(pin) {
            // Output driver off ⇒ the pad is an input. `FUNCn_IN_SEL_CFG` is
            // signal-indexed rather than pad-indexed (one pad can feed several
            // peripheral inputs) and the BROM's `gpio_matrix_in` is a no-op
            // thunk in this engine, so there is no honest single function to
            // name here. Null, not a guess.
            return Some(GpioRouting {
                mode: GpioMode::Input,
                func: None,
            });
        }
        // Output driver on: consult the per-pad output-matrix selector. This is
        // exactly what the old "does not track the output matrix" note could
        // not do.
        let sig = self.out_sel[pin as usize] & OUT_SEL_MASK;
        if sig == SIG_GPIO_OUT {
            // Driven straight by the GPIO_OUT / OUT1 latch — plain GPIO.
            Some(GpioRouting {
                mode: GpioMode::Output,
                func: None,
            })
        } else {
            Some(GpioRouting {
                mode: GpioMode::Af,
                func: esp32_out_signal_name(sig).map(String::from),
            })
        }
    }

    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        if self.irq_line() {
            out.push(GPIO_INTR_SOURCE);
        }
    }

    /// One event per change of the GPIO matrix line, so a walk-free bus
    /// re-derives the DPORT sources after an edge from outside or an
    /// acknowledge in `STATUS_W1TC` (see [`crate::peripherals::esp_gpio_net`]).
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        let level = self.irq_line();
        self.irq_watch
            .take(level)
            .map(|token| vec![(0, token)])
            .unwrap_or_default()
    }

    /// On the legacy walk the GPIO matrix line goes out as an explicit
    /// source each tick it is up (the classic DPORT aggregation reads walk
    /// sources only from tick results).
    fn tick(&mut self) -> PeripheralTickResult {
        self.cycle = self.cycle.wrapping_add(1);
        PeripheralTickResult {
            explicit_irqs: self.irq_line().then(|| vec![GPIO_INTR_SOURCE]),
            ..PeripheralTickResult::default()
        }
    }

    /// Phase 2B.3c (issue #192): migrated to the event scheduler. `cycle` is a
    /// free-running observability timestamp; flag-on it advances lazily via
    /// `sync_to` on MMIO access instead of one per `tick()`. Flag-off, `tick()`
    /// still drives it.
    fn uses_scheduler(&self) -> bool {
        true
    }

    fn sync_to(&mut self, tick_now: u64) {
        if tick_now <= self.anchor_tick {
            return;
        }
        self.cycle = self.cycle.wrapping_add(tick_now - self.anchor_tick);
        self.anchor_tick = tick_now;
    }

    fn take_wire_input_edges(&mut self) -> Vec<crate::peripherals::pad_lines::WireInputEdge> {
        std::mem::take(&mut self.wire_edges)
    }

    /// The classic RTC IO MUX (RTC_GPIO0..17 on GPIO36..39, 34, 35, 25, 26,
    /// 33, 32, 4, 0, 2, 15, 13, 12, 14, 27). Refuses the S3's.
    fn attach_rtc_io_pads(&mut self, pads: Arc<crate::peripherals::esp_rtc_io::RtcIoPads>) -> bool {
        if pads.chip() != crate::peripherals::esp_rtc_io::RtcChip::Esp32 {
            return false;
        }
        self.rtc = Some(pads);
        self.sync_rtc_inputs();
        true
    }

    /// A `MUX_SEL` write may have handed a pad back to the matrix (or taken
    /// it): re-mark net lines and re-route inputs as a routing write does,
    /// report the new sources, and tell the RTC side what `GPIO_IN` reads
    /// now (a pull change moves a released digital pad).
    fn rtc_io_written(&mut self) {
        self.sync_net_routes();
        self.notify_pad_sources();
        self.sync_rtc_inputs();
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
    use std::sync::Mutex;

    /// `GPIO_FUNCn_IN_SEL_CFG` (0x130 + 4 * signal) keeps its eight
    /// writable bits, and a pad on a net routed in to VSPI's MISO (`VSPIQ`,
    /// 64) hands the level the net presents to that line; routed out as a
    /// slave's MISO it drives only while the line does.
    #[test]
    fn a_pad_routed_to_vspiq_carries_the_line_both_ways() {
        use crate::peripherals::esp_gpspi_wire::{LINE_MISO, SPI_NET_LINES};
        use crate::peripherals::pad_lines::{LineDrive, PadLines};
        let mut g = Esp32Gpio::new();
        g.write_u32(0x130 + 4 * 64, 0xFFFF_FFFF).unwrap();
        assert_eq!(g.read_u32(0x130 + 4 * 64).unwrap(), 0xFF);
        let wire = Arc::new(PadLines::new(SPI_NET_LINES, &[false, false, true, false]));
        wire.set_mode(LINE_MISO, LineDrive::Input);
        g.bind_spi_lines(&wire, VSPI_SIGNALS);
        g.join_wire(19);
        // SIG_IN_SEL clear: the signal takes its IO_MUX pad, not the matrix.
        g.write_u32(0x130 + 4 * 64, 19).unwrap();
        g.set_external(19, External::Level(true));
        assert!(g.take_wire_input_edges().is_empty());
        g.write_u32(0x130 + 4 * 64, (1 << 7) | 19).unwrap();
        assert!(wire.on_net(LINE_MISO), "routed in from a net pad");
        let edges = g.take_wire_input_edges();
        assert_eq!(edges.len(), 1);
        assert_eq!((edges[0].line, edges[0].level), (LINE_MISO, true));
        g.set_external(19, External::Level(false));
        let edges = g.take_wire_input_edges();
        assert_eq!((edges[0].line, edges[0].level), (LINE_MISO, false));
        // Routed out instead (a slave's MISO): released while the line is an
        // input, driven once the slave drives it.
        g.write_u32(0x530 + 4 * 19, 64).unwrap();
        g.write_u32(0x24, 1 << 19).unwrap();
        assert_eq!(g.driver(19).unwrap().out, crate::pins::Out::Off);
        wire.drive_line(LINE_MISO, LineDrive::PushPull, true);
        assert_eq!(g.driver(19).unwrap().out, crate::pins::Out::High);
    }

    #[derive(Debug, Default)]
    struct TestObserver {
        events: Mutex<Vec<(u8, bool, bool, u64)>>,
    }

    impl GpioObserver for TestObserver {
        fn on_pin_change(&self, pin: u8, from: bool, to: bool, sim_cycle: u64) {
            self.events.lock().unwrap().push((pin, from, to, sim_cycle));
        }
    }

    #[test]
    fn out_w1ts_sets_bit_and_fires_observer() {
        let mut g = Esp32Gpio::new();
        let obs = Arc::new(TestObserver::default());
        g.add_observer(obs.clone());

        // GPIO_OUT_W1TS at 0x08, set GPIO5 (CS pin in e-paper lab).
        g.write(0x08, 1 << 5).unwrap();
        g.write(0x09, 0).unwrap();
        g.write(0x0A, 0).unwrap();
        g.write(0x0B, 0).unwrap();

        assert_eq!(g.out & (1 << 5), 1 << 5);
        let events = obs.events.lock().unwrap();
        assert!(events.iter().any(|&(p, f, t, _)| p == 5 && !f && t));
    }

    #[test]
    fn w1tc_via_word_store_clears_only_target_bit() {
        // Regression for the blank e-paper render: a 32-bit digitalWrite(pin, LOW)
        // (W1TC = 1<<pin) must clear ONLY that pin, not every currently-high OUT
        // bit. Before Esp32Gpio gained write_u32, the byte-split RMW read OUT
        // back through read_word(0x0C) and turned the whole OUT value into the
        // clear mask — so toggling CS (GPIO5) low wiped DC (GPIO17) and the
        // panel saw DC=command for the framebuffer stream.
        let mut g = Esp32Gpio::new();
        // Drive CS(5), RST(16), DC(17) high via a 32-bit W1TS store.
        g.write_u32(0x08, (1 << 5) | (1 << 16) | (1 << 17)).unwrap();
        assert_eq!(g.out, (1 << 5) | (1 << 16) | (1 << 17));
        // digitalWrite(CS=5, LOW): 32-bit W1TC of just bit 5.
        g.write_u32(0x0C, 1 << 5).unwrap();
        assert_eq!(g.out & (1 << 5), 0, "CS bit must clear");
        assert_eq!(g.out & (1 << 16), 1 << 16, "RST must survive");
        assert_eq!(g.out & (1 << 17), 1 << 17, "DC must survive the CS toggle");
    }

    #[test]
    fn pin_register_at_0x88_round_trips_int_type_and_ena() {
        let mut g = Esp32Gpio::new();
        // GPIO_PIN4_REG at 0x88 + 4*4 = 0x98. Set INT_TYPE=3 (any-edge), INT_ENA bit=1.
        let val = (3u32 << 7) | (1u32 << 13);
        for b in 0..4u64 {
            g.write(0x98 + b, ((val >> (b * 8)) & 0xFF) as u8).unwrap();
        }
        let read_back = {
            let mut acc = 0u32;
            for b in 0..4u64 {
                acc |= (g.read(0x98 + b).unwrap() as u32) << (b * 8);
            }
            acc
        };
        assert_eq!(read_back & 0x3FF, val & 0x3FF);
    }

    #[test]
    fn snapshot_exposes_odr_for_board_io_readback() {
        let mut g = Esp32Gpio::new();
        g.apply_out((1 << 2) | (1 << 5));
        let snap = g.snapshot();
        assert_eq!(snap["odr"].as_u64().unwrap(), (1u64 << 2) | (1u64 << 5));
        assert_eq!(snap["layout"].as_str().unwrap(), "esp32_classic");
    }

    /// The pad's own drive is its output stage only: ENABLE drives the latch,
    /// an open-drain 1 (`PAD_DRIVER`) releases the pad, and the level the
    /// outside feeds back is never the pad's drive. A released pad the
    /// outside holds reads that level.
    #[test]
    fn the_own_drive_is_the_output_stage_and_a_released_pad_reads_the_wire() {
        use crate::logic_capture::PadDrive;
        use crate::pins::own_drive;
        let mut g = Esp32Gpio::new();
        assert_eq!(own_drive(&g, 4), Some(PadDrive::HighZ));
        assert_eq!(
            g.set_external(40, External::Level(true)),
            None,
            "past the 40 pads"
        );
        // The outside holds the pad high: still not this chip's drive.
        g.set_gpio_input(4, true);
        assert_eq!(own_drive(&g, 4), Some(PadDrive::HighZ));
        assert_eq!(g.read_gpio_pad_drive(4), Some(PadDrive::Driven), "probe");
        assert_eq!(
            g.read_u32(0x3C).unwrap() & (1 << 4),
            1 << 4,
            "GPIO_IN sees it"
        );
        // Push-pull output low.
        g.write_u32(0x24, 1 << 4).unwrap();
        assert_eq!(own_drive(&g, 4), Some(PadDrive::Driven));
        assert_eq!(g.read_gpio_pad(4), Some(false));
        // Open drain holding a 1: released, GPIO_IN reads the wire (the
        // outside's 1 and then its 0).
        g.write_u32(0x88 + 4 * 4, 1 << 2).unwrap();
        g.write_u32(0x08, 1 << 4).unwrap();
        assert_eq!(own_drive(&g, 4), Some(PadDrive::HighZ));
        g.set_gpio_input(4, false);
        assert_eq!(
            g.read_u32(0x3C).unwrap() & (1 << 4),
            0,
            "a peer pulls it low"
        );
        // Open drain holding a 0 drives it.
        g.write_u32(0x0C, 1 << 4).unwrap();
        assert_eq!(own_drive(&g, 4), Some(PadDrive::Driven));
        // Routed through the matrix to a signal no wire is published for: no
        // known drive, which is what makes a world refuse the pad.
        g.write_u32(FUNC_OUT_SEL + 4 * 4, 71).unwrap();
        assert_eq!(own_drive(&g, 4), None);
    }

    /// An external edge latches `GPIO_STATUS` per `INT_TYPE`, shows in the
    /// PRO CPU's `GPIO_PCPU_INT` when its INT_ENA bit is set, asserts matrix
    /// source 22, and `STATUS_W1TC` drops it again. Each change of the line
    /// arms exactly one scheduler event.
    #[test]
    fn an_external_edge_raises_the_gpio_interrupt_on_source_22() {
        let mut g = Esp32Gpio::new();
        // GPIO5: rising edge, PRO CPU interrupt (INT_ENA bit 2 = reg bit 15).
        g.write_u32(0x88 + 5 * 4, (1 << 7) | (1 << 15)).unwrap();
        assert!(g.matrix_irq_sources().is_empty());
        assert!(g.take_scheduled_events().is_empty());
        g.set_gpio_input(5, false);
        assert_eq!(g.read_u32(0x44).unwrap(), 0, "no edge, no status");
        g.set_gpio_input(5, true);
        assert_eq!(g.read_u32(0x44).unwrap(), 1 << 5);
        assert_eq!(g.read_u32(0x68).unwrap(), 1 << 5, "PCPU_INT");
        assert_eq!(g.read_u32(0x60).unwrap(), 0, "not enabled for the APP CPU");
        assert_eq!(g.matrix_irq_sources(), vec![22]);
        assert_eq!(g.take_scheduled_events().len(), 1);
        assert!(g.take_scheduled_events().is_empty(), "one event per change");
        g.set_gpio_input(5, false); // falling: not selected
        g.write_u32(0x4C, 1 << 5).unwrap();
        assert_eq!(g.read_u32(0x44).unwrap(), 0);
        assert!(g.matrix_irq_sources().is_empty());
        assert_eq!(
            g.take_scheduled_events().len(),
            1,
            "the drop is a change too"
        );
        // A low-level pin re-latches while the level holds.
        g.write_u32(0x88 + 5 * 4, (4 << 7) | (1 << 15)).unwrap();
        assert_eq!(g.read_u32(0x44).unwrap(), 1 << 5);
        g.write_u32(0x4C, 1 << 5).unwrap();
        assert_eq!(g.read_u32(0x44).unwrap(), 1 << 5, "still low: re-latched");
        // The legacy walk carries the same source in its tick result.
        assert_eq!(g.tick().explicit_irqs, Some(vec![22]));
    }

    /// GPIO32..39 take outside levels into `GPIO_IN1` at bit `pad - 32`,
    /// drive OUT1 through ENABLE1 (open drain releasing a 1), and latch
    /// `GPIO_STATUS1` per INT_TYPE, which raises source 22 through
    /// `GPIO_PCPU_INT1`. GPIO34..39 are input-only: ENABLE1 stores but they
    /// never drive. Bank 0 is untouched by any of it.
    #[test]
    fn bank_one_pads_take_input_drive_output_and_interrupt() {
        use crate::pins::Out;
        let mut g = Esp32Gpio::new();
        assert_eq!(g.pin_count(), 40);
        // Input on GPIO39 (input-only), seen at IN1 bit 7.
        let change = g.set_external(39, External::Level(true)).unwrap();
        assert_eq!((change.before, change.after), (false, true));
        assert_eq!(g.read_u32(0x40).unwrap(), 1 << 7);
        assert_eq!(g.read_u32(0x3C).unwrap(), 0, "bank 0 untouched");
        assert_eq!(g.external(39), External::Level(true));
        assert_eq!(g.input(39), Some(true));
        assert_eq!(g.read_gpio_input(39), Some(true));
        // An input-only pad never drives, whatever ENABLE1 says.
        g.write_u32(0x30, 1 << 7).unwrap();
        g.write_u32(0x14, 0).unwrap();
        assert_eq!(g.read_u32(0x2C).unwrap(), 1 << 7, "ENABLE1 stores");
        assert_eq!(g.driver(39).unwrap().out, Out::Off);
        assert_eq!(g.read_u32(0x40).unwrap(), 1 << 7, "still the outside's 1");
        // GPIO33 push-pull out, through OUT1/ENABLE1 bit 1 — not bit 33.
        g.write_u32(0x30, 1 << 1).unwrap();
        g.write_u32(0x14, 1 << 1).unwrap();
        assert_eq!(g.driver(33).unwrap().out, Out::High);
        assert_eq!(g.driver(1).unwrap().out, Out::Off, "GPIO1 is not GPIO33");
        assert_eq!(g.read_u32(0x40).unwrap() & 0b10, 0b10, "IN1 reads it back");
        assert_eq!(g.read_gpio_output(33), Some(true));
        // Open drain on GPIO33 (GPIO_PIN33_REG at 0x88 + 4 * 33): a 1 is
        // released, and the pad reads what the outside holds.
        g.write_u32(0x88 + 4 * 33, 1 << 2).unwrap();
        assert_eq!(g.read_u32(0x88 + 4 * 33).unwrap(), 1 << 2);
        assert_eq!(g.driver(33).unwrap().out, Out::Off);
        g.set_external(33, External::Level(false));
        assert_eq!(g.read_u32(0x40).unwrap() & 0b10, 0);
        g.write_u32(0x18, 1 << 1).unwrap();
        assert_eq!(g.driver(33).unwrap().out, Out::Low);
        // GPIO32: falling-edge interrupt to the PRO CPU (INT_ENA bit 2).
        g.write_u32(0x88 + 4 * 32, (2 << 7) | (1 << 15)).unwrap();
        g.set_external(32, External::Level(true));
        assert_eq!(g.read_u32(0x50).unwrap(), 0, "rising: not selected");
        assert!(g.matrix_irq_sources().is_empty());
        g.set_external(32, External::Level(false));
        assert_eq!(g.read_u32(0x50).unwrap(), 1, "STATUS1 bit 0");
        assert_eq!(g.read_u32(0x44).unwrap(), 0, "STATUS untouched");
        assert_eq!(g.read_u32(0x7C).unwrap(), 1, "PCPU_INT1");
        assert_eq!(g.read_u32(0x74).unwrap(), 0, "not the APP CPU's");
        assert_eq!(g.matrix_irq_sources(), vec![22]);
        assert_eq!(g.take_scheduled_events().len(), 1);
        g.write_u32(0x58, 1).unwrap();
        assert_eq!(g.read_u32(0x50).unwrap(), 0);
        assert!(g.matrix_irq_sources().is_empty());
        // Past the 40 pads nothing is taken.
        assert_eq!(g.set_external(40, External::Level(true)), None);
        assert_eq!(g.driver(40), None);
    }

    /// The bank-1 trap the header names: FUNCn is indexed by the pad
    /// (FUNC33 at 0x530 + 4 * 33) while ENABLE1 takes bit `pad - 32`. An
    /// I²C1 wire routed out and in on GPIO33 / GPIO32 becomes a net line,
    /// drives what the line drives, and takes the level the net presents;
    /// GPIO1 / GPIO0 (the same bank-relative bits) stay out of it.
    #[test]
    fn a_bank_one_pad_routed_to_i2c1_carries_the_line_both_ways() {
        use crate::peripherals::esp32::i2c::{LINE_SCL, LINE_SDA};
        use crate::peripherals::pad_lines::LineDrive;
        let wire = crate::peripherals::esp_i2c_core::new_pad_lines();
        let mut g = Esp32Gpio::new();
        g.bind_i2c_lines(&wire, I2C1_SIGNALS);
        // SCL out on GPIO33, read back in through FUNC95_IN_SEL_CFG.
        g.write_u32(FUNC_OUT_SEL + 4 * 33, SIG_I2CEXT1_SCL).unwrap();
        g.write_u32(0x30, 1 << 1).unwrap();
        g.write_u32(FUNC_IN_SEL + 4 * 95, (1 << 7) | 33).unwrap();
        // SDA in only, from GPIO32.
        g.write_u32(FUNC_IN_SEL + 4 * 96, (1 << 7) | 32).unwrap();
        assert_eq!(
            g.gpio_routing(33).unwrap().func.as_deref(),
            Some("I2CEXT1_SCL")
        );
        assert!(g.gpio_routing(1).unwrap().func.is_none());
        g.join_wire(1);
        assert!(!wire.on_net(LINE_SCL), "GPIO1 is not GPIO33");
        g.join_wire(33);
        assert!(wire.on_net(LINE_SCL), "routed out to a net pad");
        assert_eq!(wire.mode(LINE_SCL), LineDrive::OpenDrain);
        assert!(!wire.on_net(LINE_SDA));
        g.join_wire(32);
        assert!(wire.on_net(LINE_SDA), "routed in from a net pad");
        let _ = g.take_wire_input_edges();
        g.set_external(32, External::Level(false));
        let edges = g.take_wire_input_edges();
        assert_eq!(edges.len(), 1);
        assert_eq!((edges[0].line, edges[0].level), (LINE_SDA, false));
        g.set_external(33, External::Level(false));
        let edges = g.take_wire_input_edges();
        assert_eq!((edges[0].line, edges[0].level), (LINE_SCL, false));
        // The released open-drain SCL drives nothing; pulled low it drives.
        assert_eq!(g.driver(33).unwrap().out, crate::pins::Out::Off);
        wire.drive_line(LINE_SCL, LineDrive::OpenDrain, false);
        assert_eq!(g.driver(33).unwrap().out, crate::pins::Out::Low);
        // An input-only pad is never an output route.
        g.write_u32(FUNC_OUT_SEL + 4 * 34, SIG_I2CEXT1_SDA).unwrap();
        g.write_u32(0x30, 1 << 2).unwrap();
        assert_eq!(g.driver(34).unwrap().out, crate::pins::Out::Off);
        assert_eq!(
            g.gpio_routing(34).unwrap().mode,
            crate::peripherals::gpio::GpioMode::Input
        );
    }
}
