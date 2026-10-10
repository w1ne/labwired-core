// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! GPIO peripheral for ESP32-classic (LX6).
//!
//! Maps at 0x3FF44000 per ESP32 TRM v4.6 §4.10. Models the subset esp-hal
//! 1.x writes during init + the e-paper lab firmware path:
//!   - GPIO_OUT / OUT_W1TS / OUT_W1TC for GPIO0..31
//!   - GPIO_ENABLE / ENABLE_W1TS / ENABLE_W1TC for GPIO0..31
//!   - GPIO_IN (input read-only, settable via `set_pin_input` for tests)
//!   - GPIO_PINn_REG (PAD_DRIVER, INT_TYPE, INT_ENA, ...) and the GPIO
//!     interrupt for GPIO0..31: `GPIO_STATUS` latched per INT_TYPE from
//!     changes of `GPIO_IN`, `GPIO_ACPU_INT` / `GPIO_PCPU_INT`, and matrix
//!     source 22 while either CPU has a pending enabled pin
//!   - `gpio_net` membership for GPIO0..31 (see `crate::peripherals::esp_gpio_net`)
//!
//! The high bank (GPIO32..39) at OUT1/ENABLE1/IN1 isn't modeled — the e-paper
//! pin map (CS=5, SCK=18, MOSI=23, DC=17, RST=16, BUSY=4) is all in 0..31.
//! Writes to those offsets are no-ops; reads return 0.
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

/// Pads that can drive an output at all: `SOC_GPIO_VALID_OUTPUT_GPIO_MASK`
/// (esp-idf `soc_caps.h`) — the 40 pads minus GPIO24 and GPIO28..31 (absent
/// from the package) and minus GPIO34..39 (input-only). Binding a peripheral
/// wire to a pad that cannot drive would publish a bus onto a pin no board can
/// wire it to.
const VALID_OUTPUT_PADS: u64 = 0x0000_0003_0EFF_FFFF;

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
        64 => "VSPIQ", // VSPI MISO — named, never bound (nothing drives it)
        SIG_VSPID => "VSPID",
        SIG_VSPICS0 => "VSPICS0",
        95 => "I2CEXT1_SCL",
        96 => "I2CEXT1_SDA",
        SIG_U2TXD => "U2TXD",
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
pub use crate::peripherals::device::GpioObserver;

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
    /// `GPIO_PINn_REG` for GPIO0..31, masked to the writable bits
    /// ([`PIN_WMASK`]): PAD_DRIVER (open drain), INT_TYPE, WAKEUP_ENABLE,
    /// CONFIG and the five INT_ENA bits.
    pin_cfg: [u32; 32],
    /// `GPIO_STATUS_REG`: the interrupt status latch of GPIO0..31.
    status: u32,
    /// GPIO0..31 the outside world holds ([`PinPort::set_external`]); the
    /// level on each is its `in_data` bit. An open-drain pad holding a 1
    /// reads that level instead of its latch.
    ext_mask: u32,
    /// Matrix-line change detector for the scheduler chain.
    irq_watch: crate::peripherals::esp_gpio_net::IrqLevelWatch,
    /// `GPIO_FUNCn_OUT_SEL_CFG` per pad, flat 0..39 — the output-matrix
    /// selector. Before this existed the register read 0 and every write was
    /// dropped, so the model could not tell a plain-GPIO output from a
    /// peripheral-routed one and NO bus could be published onto a classic pad:
    /// an analyzer clipped to any classic-ESP32 pin read a flat line while the
    /// C3 and S3 worked.
    out_sel: [u32; PAD_COUNT as usize],
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
}

impl Esp32Gpio {
    pub fn new() -> Self {
        Self {
            enable: 0,
            out: 0,
            out1: 0,
            enable1: 0,
            in_data: 0,
            pin_cfg: [0; 32],
            status: 0,
            ext_mask: 0,
            irq_watch: Default::default(),
            out_sel: [FUNC_OUT_SEL_RESET; PAD_COUNT as usize],
            pad_routes: PadRoutes::new(),
            watch: None,
            cycle: 0,
            anchor_tick: 0,
            observers: Vec::new(),
        }
    }

    pub fn add_observer(&mut self, obs: Arc<dyn GpioObserver>) {
        self.observers.push(obs);
    }

    pub fn out_value(&self) -> u32 {
        self.out
    }

    pub fn enable_value(&self) -> u32 {
        self.enable
    }

    /// Set the input level on `pin` (0..=31).
    pub fn set_pin_input(&mut self, pin: u8, level: bool) {
        assert!(pin < 32, "set_pin_input: pin {pin} >= 32");
        let before = self.pad_level_bank0();
        if level {
            self.in_data |= 1u32 << pin;
        } else {
            self.in_data &= !(1u32 << pin);
        }
        self.latch_input_edges(before);
    }

    /// Bank-0 pads in open drain (`GPIO_PINn_REG.PAD_DRIVER`).
    fn open_drain_bank0(&self) -> u32 {
        self.pin_cfg.iter().enumerate().fold(0u32, |m, (pin, w)| {
            if w & crate::peripherals::esp_gpio_net::PIN_PAD_DRIVER != 0 {
                m | (1 << pin)
            } else {
                m
            }
        })
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

    /// Latch `GPIO_STATUS` for every pin whose `GPIO_IN` bit moved from
    /// `before` (edge types) and for every level-type pin whose level holds.
    /// The interrupt sees the pad as `GPIO_IN` reports it, so a pad's own
    /// output edges count too, as on silicon with the input buffer enabled.
    fn latch_input_edges(&mut self, before: u32) {
        use crate::peripherals::esp_gpio_net::{edge_hits, int_type, level_hits};
        let after = self.pad_level_bank0();
        for pin in 0..32 {
            let kind = int_type(self.pin_cfg[pin]);
            if kind == 0 {
                continue;
            }
            let (b, a) = ((before >> pin) & 1 != 0, (after >> pin) & 1 != 0);
            if edge_hits(kind, b, a) || level_hits(kind, a) {
                self.status |= 1 << pin;
            }
        }
    }

    /// `GPIO_ACPU_INT_REG` (APP CPU, INT_ENA bit 0 = register bit 13) and
    /// `GPIO_PCPU_INT_REG` (PRO CPU, INT_ENA bit 2 = register bit 15) — the
    /// esp-idf `GPIO_LL_APP_CPU_INTR_ENA` / `GPIO_LL_PRO_CPU_INTR_ENA` bits.
    fn cpu_int(&self, ena_bit: u32) -> u32 {
        let mut out = 0;
        let mut pending = self.status;
        while pending != 0 {
            let pin = pending.trailing_zeros() as usize;
            pending &= pending - 1;
            if self.pin_cfg[pin] & (1 << ena_bit) != 0 {
                out |= 1 << pin;
            }
        }
        out
    }

    /// The `ETS_GPIO_INTR_SOURCE` matrix line: either CPU's GPIO interrupt.
    fn irq_line(&self) -> bool {
        self.cpu_int(13) | self.cpu_int(15) != 0
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
        (self.out & driving) | (self.in_data & !driving)
    }

    /// Bank 1 twin of [`pad_level_bank0`] for the GPIO32..39 pads, where bit 0
    /// is GPIO32. There is no external-input storage for this bank yet, so an
    /// undriven pad reads 0 — the same value the register returned before,
    /// which keeps this strictly a gain.
    fn pad_level_bank1(&self) -> u32 {
        self.out1 & self.enable1
    }

    /// `true` when pad `pin`'s output driver is enabled.
    ///
    /// The ONE place the classic bank split is decided: ENABLE holds GPIO0..31,
    /// ENABLE1 holds GPIO32..39 at bit `pin - 32`. Everything above this — the
    /// matrix selector, the pad level, the routing report — indexes pads flat,
    /// because the matrix itself does.
    fn output_driver_on(&self, pin: u8) -> bool {
        if pin < 32 {
            (self.enable & (1u32 << pin)) != 0
        } else if pin < PAD_COUNT {
            (self.enable1 & (1u32 << (pin - 32))) != 0
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
    fn matrix_signal(&self, pin: u8) -> Option<u32> {
        if pin >= PAD_COUNT || !self.output_driver_on(pin) {
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
        if let Some(level) = self.pad_routes.level(pin, |p| self.matrix_signal(p)) {
            return Some(level);
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
    pub(crate) fn set_i2c_lines(&mut self, lines: Arc<crate::peripherals::pad_lines::PadLines>) {
        for pin in 0..PAD_COUNT {
            if VALID_OUTPUT_PADS & (1u64 << pin) == 0 {
                continue;
            }
            self.pad_routes.bind(
                &lines,
                pin,
                Some(SIG_I2CEXT0_SCL),
                crate::peripherals::esp32::i2c::LINE_SCL,
                "I2CEXT0_SCL",
            );
            self.pad_routes.bind(
                &lines,
                pin,
                Some(SIG_I2CEXT0_SDA),
                crate::peripherals::esp32::i2c::LINE_SDA,
                "I2CEXT0_SDA",
            );
        }
    }

    /// Bind the VSPI (SPI3) master's SCK/MOSI/CS wire to every output-capable
    /// pad the matrix can route it to. Which pad is live at any moment is then
    /// decided by `FUNCn_OUT_SEL`, through the shared seam.
    ///
    /// Unlike the RP2040 there is no pad table to transcribe: the ESP32 GPIO
    /// matrix routes ANY peripheral signal to ANY output-capable pad, so every
    /// pad is bound to all three signals and the selector picks the live one.
    ///
    /// MISO (`VSPIQ`, index 64) is deliberately unbound — see
    /// [`crate::peripherals::esp_gpspi_wire`]. A bound-but-undriven pad reports
    /// a confident idle level that looks authoritative, which is worse than the
    /// latch fallback it would replace.
    pub(crate) fn bind_spi_lines(&mut self, lines: &Arc<crate::peripherals::pad_lines::PadLines>) {
        use crate::peripherals::esp_gpspi_wire::{LINE_CS, LINE_MOSI, LINE_SCK};
        for pin in 0..PAD_COUNT {
            if VALID_OUTPUT_PADS & (1u64 << pin) == 0 {
                continue;
            }
            self.pad_routes
                .bind(lines, pin, Some(SIG_VSPICLK), LINE_SCK, "SPI3_SCK");
            self.pad_routes
                .bind(lines, pin, Some(SIG_VSPID), LINE_MOSI, "SPI3_MOSI");
            self.pad_routes
                .bind(lines, pin, Some(SIG_VSPICS0), LINE_CS, "SPI3_CS");
        }
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
    fn pad_driver(&self, pin: u8) -> Option<crate::pins::PadDriver> {
        use crate::pins::PadDriver;
        if pin >= PAD_COUNT {
            return None;
        }
        if let Some(sig) = self.matrix_signal(pin) {
            if sig != SIG_GPIO_OUT {
                return self
                    .pad_routes
                    .level(pin, |p| self.matrix_signal(p))
                    .map(PadDriver::drive);
            }
        }
        if !self.output_driver_on(pin) {
            return Some(PadDriver::OFF);
        }
        Some(if pin < 32 {
            PadDriver::output(
                self.out & (1 << pin) != 0,
                self.pin_cfg[pin as usize] & crate::peripherals::esp_gpio_net::PIN_PAD_DRIVER != 0,
            )
        } else {
            PadDriver::drive(self.out1 & (1 << (pin - 32)) != 0)
        })
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
            // STATUS and its W1TS/W1TC views (GPIO0..31). The high bank
            // (STATUS1, GPIO32..39) has no input model, so it latches nothing.
            0x44 | 0x48 | 0x4C => self.status,
            0x50 | 0x54 | 0x58 => 0,
            // ACPU_INT, ACPU_NMI_INT, PCPU_INT, PCPU_NMI_INT, CPUSDIO_INT.
            0x60 => self.cpu_int(13),
            0x64 => self.cpu_int(14),
            0x68 => self.cpu_int(15),
            0x6C => self.cpu_int(16),
            0x70 => self.cpu_int(17),
            // GPIO_PINn_REG at 0x88 + pin*4 (TRM Table 4-12): PAD_DRIVER (2),
            // INT_TYPE [9:7], WAKEUP_ENABLE (10), CONFIG [12:11], INT_ENA
            // [17:13].
            off if (0x88..0x88 + 32 * 4).contains(&off) => {
                self.pin_cfg[((off - 0x88) / 4) as usize]
            }
            // GPIO_FUNC0..39_OUT_SEL_CFG at 0x530 + pad*4 (gpio_reg.h). These
            // read 0 and dropped every write before this, which is exactly why
            // no bus could be published onto a classic-ESP32 pad.
            off if Self::out_sel_index(off).is_some() => {
                self.out_sel[Self::out_sel_index(off).expect("guarded by the match")]
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
        let before = self.pad_level_bank0();
        self.write_word_inner(word_off, value);
        self.latch_input_edges(before);
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
            0x50 | 0x54 | 0x58 => {}
            off if (0x88..0x88 + 32 * 4).contains(&off) => {
                let pin = ((off - 0x88) / 4) as usize;
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
        if pin < 32 && self.ext_mask & (1 << pin) != 0 {
            External::Level(self.in_data & (1 << pin) != 0)
        } else {
            External::Released
        }
    }

    /// GPIO0..31 take input (the high bank has no input storage in this
    /// model). A level lands in `in_data`; releasing leaves it there and
    /// hands a released open-drain pad back to its latch. Every change of
    /// `GPIO_IN` latches `GPIO_STATUS` per `INT_TYPE`.
    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        if pin >= 32 {
            return None;
        }
        let bit = 1u32 << pin;
        let before = self.pad_level_bank0();
        self.mutate_pads(|s| {
            match ext {
                External::Level(level) => {
                    s.ext_mask |= bit;
                    if level {
                        s.in_data |= bit;
                    } else {
                        s.in_data &= !bit;
                    }
                }
                External::Released => s.ext_mask &= !bit,
            }
            s.latch_input_edges(before);
        });
        Some(InputChange {
            before: before & bit != 0,
            after: self.pad_level_bank0() & bit != 0,
        })
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

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        if pin >= 32 {
            return None;
        }
        Some((self.in_data & (1u32 << pin)) != 0)
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        if pin >= 32 {
            return None;
        }
        Some((self.out & (1u32 << pin)) != 0)
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
            g.set_external(32, External::Level(true)),
            None,
            "high bank has no input"
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
}
