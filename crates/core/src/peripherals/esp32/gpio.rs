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
//!   - GPIO_PINn_REG round-trip storage for INT_TYPE/INT_ENA
//!
//! The high bank (GPIO32..39) at OUT1/ENABLE1/IN1 isn't modeled — the e-paper
//! pin map (CS=5, SCK=18, MOSI=23, DC=17, RST=16, BUSY=4) is all in 0..31.
//! Writes to those offsets are no-ops; reads return 0.
//!
//! Observer protocol matches `peripherals::esp32s3::gpio::GpioObserver` —
//! a single trait makes observer code work on both chip variants.

use crate::peripherals::pad_routing::PadRoutes;
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
        14 => "U0TXD",
        17 => "U1TXD",
        29 => "I2CEXT0_SCL",
        30 => "I2CEXT0_SDA",
        63 => "VSPICLK",
        64 => "VSPIQ",
        65 => "VSPID",
        68 => "VSPICS0",
        95 => "I2CEXT1_SCL",
        96 => "I2CEXT1_SDA",
        198 => "U2TXD",
        _ => return None,
    })
}

/// Push-mode logic-capture state: the shared tap, this port's watched
/// `(pin, channel)` pairs, and a pre-write level scratchpad so only genuine
/// transitions are reported.
///
/// Classic ESP32 had NO push instrumentation, so every probed pad fell to the
/// machine's per-cycle poll fallback. That is fine for a pad a firmware write
/// moves, and USELESS for a narrated bus: the I²C narrator publishes edges
/// stamped in the PAST (`PadLines::set_line_at`), and a boundary sampler only
/// ever sees the present. Push is what puts those edges in the ring at all.
#[derive(Debug)]
struct Esp32PortTap {
    tap: crate::logic_capture::LogicTap,
    watched: Vec<(u8, u32)>,
    scratch: Vec<Option<bool>>,
}

/// Notified synchronously inside the bus write path on every GPIO pin
/// transition. Observers must not panic — a panic propagates out of
/// `bus.write_u8` and crashes the simulator.
pub trait GpioObserver: Send + Sync + std::fmt::Debug {
    fn on_pin_change(&self, pin: u8, from: bool, to: bool, sim_cycle: u64);
}

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
    int_enable: u32,
    int_type: [u8; 32],
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
    /// `Some` while the logic analyzer watches pads on this port in push mode.
    /// Not snapshot state: the watch is re-armed by the frontend after resume.
    tap: Option<Esp32PortTap>,
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
            int_enable: 0,
            int_type: [0; 32],
            out_sel: [FUNC_OUT_SEL_RESET; PAD_COUNT as usize],
            pad_routes: PadRoutes::new(),
            tap: None,
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
        if level {
            self.in_data |= 1u32 << pin;
        } else {
            self.in_data &= !(1u32 << pin);
        }
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
        (self.out & self.enable) | (self.in_data & !self.enable)
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

    /// Record every watched pad's level before a mutation. One branch while no
    /// tap is installed.
    #[inline]
    fn tap_snapshot(&mut self) {
        let Some(mut t) = self.tap.take() else {
            return;
        };
        for (k, &(pin, _)) in t.watched.iter().enumerate() {
            t.scratch[k] = self.pad_level(pin);
        }
        self.tap = Some(t);
    }

    /// Report watched pads whose level became known-different since the
    /// matching [`tap_snapshot`](Self::tap_snapshot), then re-sync wire
    /// registration in case this write re-routed a watched pad.
    #[inline]
    fn tap_report(&mut self) {
        let Some(t) = self.tap.take() else {
            return;
        };
        for (k, &(pin, ch)) in t.watched.iter().enumerate() {
            if let Some(level) = self.pad_level(pin) {
                if t.scratch[k] != Some(level) {
                    t.tap.push(ch, level);
                }
            }
        }
        self.tap = Some(t);
        self.sync_line_taps();
    }

    /// Re-register watched pads with the wires that drive them, so a pad the
    /// matrix hands over (or takes back) follows its new source immediately.
    fn sync_line_taps(&mut self) {
        if self.pad_routes.is_empty() {
            return;
        }
        let Some(t) = self.tap.take() else {
            return;
        };
        let mut routes = std::mem::take(&mut self.pad_routes);
        routes.sync_taps(&t.tap, &t.watched, |pin| self.matrix_signal(pin));
        self.pad_routes = routes;
        self.tap = Some(t);
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
            // STATUS / STATUS1 — int status not driven; return 0.
            0x44 | 0x48 | 0x4C | 0x50 | 0x54 | 0x58 => 0,
            // GPIO_PINn_REG at 0x88 + pin*4 (TRM Table 4-12).
            off if (0x88..0x88 + 32 * 4).contains(&off) => {
                let pin = ((off - 0x88) / 4) as usize;
                let int_type = self.int_type[pin] as u32;
                let int_ena = (self.int_enable >> pin) & 1;
                // bits[9:7]  INT_TYPE
                // bits[16:13] INT_ENA (we model only bit 13 = cpu0 enable)
                (int_type << 7) | (int_ena << 13)
            }
            // GPIO_FUNC0..39_OUT_SEL_CFG at 0x530 + pad*4 (gpio_reg.h). These
            // read 0 and dropped every write before this, which is exactly why
            // no bus could be published onto a classic-ESP32 pad.
            off if Self::out_sel_index(off).is_some() => {
                self.out_sel[Self::out_sel_index(off).expect("guarded by the match")]
            }
            _ => 0,
        }
    }

    fn write_word(&mut self, word_off: u64, value: u32) {
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
            // STATUS_W1TS / STATUS_W1TC — accepted but no IRQ model yet.
            0x44 | 0x48 | 0x4C | 0x50 | 0x54 | 0x58 => {}
            off if (0x88..0x88 + 32 * 4).contains(&off) => {
                let pin = ((off - 0x88) / 4) as usize;
                self.int_type[pin] = ((value >> 7) & 0x7) as u8;
                let bit = (value >> 13) & 1;
                if bit != 0 {
                    self.int_enable |= 1u32 << pin;
                } else {
                    self.int_enable &= !(1u32 << pin);
                }
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
            _ => {}
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
        self.tap_snapshot();
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        self.tap_report();
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
            self.tap_snapshot();
            self.write_word(offset, value);
            self.tap_report();
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
            self.tap_snapshot();
            let cur = self.read_word(offset) & 0xFFFF_0000;
            self.write_word(offset, cur | value as u32);
            self.tap_report();
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

    fn read_gpio_pad(&self, pin: u8) -> Option<bool> {
        // ONE definition of "pad level" — see `pad_level`. A caller here and
        // firmware reading GPIO_IN / GPIO_IN1 cannot disagree, and a pad the
        // output matrix handed to a peripheral now reads that peripheral's
        // wire instead of the idle GPIO_OUT latch — which is what made an
        // analyzer clipped to a classic-ESP32 I²C pin show a flat line while
        // the bus was busy.
        self.pad_level(pin)
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

    fn set_gpio_input(&mut self, pin: u8, level: bool) -> bool {
        if pin >= 32 {
            return false;
        }
        // Bracketed like every other pad mutation: a host-driven input change
        // is an edge a probe must see.
        self.tap_snapshot();
        self.set_pin_input(pin, level);
        self.tap_report();
        true
    }

    /// Declare push capability for this port's pads (the return value IS the
    /// declaration — the machine keeps no hardcoded list).
    ///
    /// Classic ESP32 previously returned the default `false`, so every probed
    /// pad fell to the per-cycle poll fallback. Correct for a pad a firmware
    /// write moves, and blind to a NARRATED bus: the I²C controller publishes a
    /// finished transaction's edges at the cycles they occupied, in the past
    /// (`PadLines::set_line_at`), and a boundary sampler only sees the present.
    /// Accepting the tap is what puts those edges in the ring.
    fn install_logic_tap(
        &mut self,
        tap: &crate::logic_capture::LogicTap,
        watched: &[(u8, u32)],
    ) -> bool {
        if watched.is_empty() {
            self.tap = None;
            self.pad_routes.clear_taps();
        } else {
            self.tap = Some(Esp32PortTap {
                tap: tap.clone(),
                watched: watched.to_vec(),
                scratch: vec![None; watched.len()],
            });
            // Seeded stale so the sync below always installs the CURRENT
            // routing into the wire cell.
            self.pad_routes.invalidate_registrations();
            self.sync_line_taps();
        }
        true
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.cycle = self.cycle.wrapping_add(1);
        PeripheralTickResult::default()
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
}
