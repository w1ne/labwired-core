// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 GPIO peripheral model.
//!
//! This follows the C3 GPIO register offsets from
//! `configs/peripherals/esp32c3/gpio.yaml`. The important behavioral delta from
//! the declarative descriptor is W1TS/W1TC side effects: Arduino/ESP-IDF GPIO
//! writes use those set/clear registers, and display peripherals read GPIO
//! output back for CS/DC/bit-banged buses.

use crate::peripherals::gpio::{GpioMode, GpioRouting};
use crate::{Peripheral, PeripheralTickResult, SimResult};

const PIN_COUNT: u8 = 26;
const PIN_MASK: u32 = (1u32 << PIN_COUNT) - 1;

/// `GPIO_FUNCn_OUT_SEL_CFG_REG` base (C3 TRM §5.12): per-PAD output routing.
/// Bits [8:0] select which peripheral output signal drives pad `n`; the sentinel
/// `SIG_GPIO_OUT` (128) means the pad is a plain GPIO output (GPIO_OUT latch).
const FUNC_OUT_SEL: u64 = 0x554;
const SIG_GPIO_OUT: u32 = 128;
/// GPIO-matrix output signal indices of the I²C0 controller (esp-idf
/// `soc/esp32c3/include/soc/gpio_sig_map.h`).
const SIG_I2CEXT0_SCL: u32 = 53;
const SIG_I2CEXT0_SDA: u32 = 54;

/// ESP32-C3 GPIO-matrix OUTPUT signal index → signal name, for the I²C / SPI /
/// UART signals the logic analyzer cares about (from esp-idf
/// `soc/esp32c3/include/soc/gpio_sig_map.h`). Unmapped indices → `None` (null,
/// never a guess).
fn c3_out_signal_name(idx: u32) -> Option<&'static str> {
    Some(match idx {
        6 => "U0TXD",
        9 => "U1TXD",
        53 => "I2CEXT0_SCL",
        54 => "I2CEXT0_SDA",
        63 => "FSPICLK",
        64 => "FSPIQ", // FSPI MISO
        65 => "FSPID", // FSPI MOSI
        68 => "FSPICS0",
        _ => return None,
    })
}

const BT_SELECT: u64 = 0x00;
const OUT: u64 = 0x04;
const OUT_W1TS: u64 = 0x08;
const OUT_W1TC: u64 = 0x0C;
const SDIO_SELECT: u64 = 0x1C;
const ENABLE: u64 = 0x20;
const ENABLE_W1TS: u64 = 0x24;
const ENABLE_W1TC: u64 = 0x28;
/// GPIO_STRAP_REG: latched boot-mode straps. The boot ROM reads bit 3 to choose
/// SPI fast-flash boot; reset seeds it to the board-default flash-boot state.
const STRAP: u64 = 0x38;
const STRAP_SPI_FAST_FLASH_BOOT: u32 = 0x0000_0008;
const IN: u64 = 0x3C;
const STATUS: u64 = 0x44;
const STATUS_W1TS: u64 = 0x48;
const STATUS_W1TC: u64 = 0x4C;
const PCPU_INT: u64 = 0x5C;
const PCPU_NMI_INT: u64 = 0x60;
const CPUSDIO_INT: u64 = 0x64;
const PIN0: u64 = 0x74;

/// Push-mode logic-capture state for the C3 GPIO (see
/// [`crate::logic_capture`]): the shared tap, this port's watched
/// `(pin, channel)` pairs, a pre-write level scratchpad, and the channel
/// lists last registered with the shared I²C line cell (so registration is
/// only re-synced when a write actually changes a watched pad's routing).
#[derive(Debug)]
struct C3Tap {
    tap: crate::logic_capture::LogicTap,
    watched: Vec<(u8, u32)>,
    scratch: Vec<Option<bool>>,
    line_scl_chs: Vec<u32>,
    line_sda_chs: Vec<u32>,
}

#[derive(Debug)]
pub struct Esp32c3Gpio {
    bt_select: u32,
    out: u32,
    sdio_select: u32,
    enable: u32,
    strap: u32,
    /// Host/browser-driven input levels. A bit only has electrical authority
    /// when the matching `external_drive_mask` bit is set; otherwise the
    /// IO_MUX pull-up (if any) supplies the released level.
    external_levels: u32,
    external_drive_mask: u32,
    status: u32,
    pin_cfg: [u32; PIN_COUNT as usize],
    /// `GPIO_FUNCn_OUT_SEL_CFG` per pad — the output-matrix selector read by
    /// `gpio_routing` to name the signal a pad is wired to.
    out_sel: [u32; PIN_COUNT as usize],
    /// Shared IO_MUX per-pad register words. This is intentionally separate
    /// from the output matrix: the pad's weak pull-up is an electrical input
    /// condition, not a routed peripheral signal.
    pad_controls: Option<super::io_mux::PadControls>,
    /// Live I²C0 SDA/SCL line levels, shared with the C3 I²C bit engine (see
    /// `crate::bus::SystemBus::wire_esp32c3_i2c_pads`). Pads whose output
    /// matrix routes I2CEXT0_SCL/SDA read the wire here instead of the
    /// GPIO_OUT latch.
    i2c_lines: Option<std::sync::Arc<super::i2c::I2cLineLevels>>,
    /// `Some` while the logic analyzer watches pads on this port in push mode
    /// (installed via `install_logic_tap`). Not snapshot state — the watch is
    /// re-armed by the frontend after a resume.
    tap: Option<C3Tap>,
    cycle: u64,
    anchor_tick: u64,
}

impl Esp32c3Gpio {
    pub fn new() -> Self {
        Self {
            bt_select: 0,
            out: 0,
            sdio_select: 0,
            enable: 0,
            strap: STRAP_SPI_FAST_FLASH_BOOT,
            external_levels: 0,
            external_drive_mask: 0,
            status: 0,
            pin_cfg: [0; PIN_COUNT as usize],
            out_sel: [0; PIN_COUNT as usize],
            pad_controls: None,
            i2c_lines: None,
            tap: None,
            cycle: 0,
            anchor_tick: 0,
        }
    }

    /// Wire the shared I²C0 line-level cell (the same `Arc` the C3 I²C bit
    /// engine drives) so matrix-routed pads carry the real waveform.
    pub(crate) fn set_i2c_lines(&mut self, lines: std::sync::Arc<super::i2c::I2cLineLevels>) {
        self.i2c_lines = Some(lines);
    }

    /// Wire the C3 IO_MUX's shared per-pad controls after both peripherals
    /// exist on the system bus.
    pub(crate) fn set_pad_controls(&mut self, controls: super::io_mux::PadControls) {
        self.pad_controls = Some(controls);
    }

    fn io_mux_pullup_mask(&self) -> u32 {
        let Some(controls) = &self.pad_controls else {
            return 0;
        };
        controls
            .read()
            .expect("ESP32-C3 IO_MUX pad controls poisoned")
            .iter()
            .enumerate()
            .fold(0, |mask, (pin, word)| {
                if word & (1 << 8) != 0 {
                    mask | (1 << pin)
                } else {
                    mask
                }
            })
    }

    /// Firmware-visible input word. An explicit external drive always beats a
    /// weak internal pull-up; otherwise the raw IO_MUX `FUN_WPU` bit supplies
    /// the released level, including its descriptor-defined cold reset.
    fn effective_input(&self) -> u32 {
        ((self.external_levels & self.external_drive_mask)
            | (self.io_mux_pullup_mask() & !self.external_drive_mask))
            & PIN_MASK
    }

    /// Direction-aware pad level — the single truth `read_gpio_pad` and the
    /// push-capture tap both read.
    fn pad_level(&self, pin: u8) -> Option<bool> {
        if pin >= PIN_COUNT {
            return None;
        }
        let mask = 1u32 << pin;
        // ENABLE is the output driver: enabled pins show the driving signal,
        // everything else shows the (externally driven) input level.
        if (self.enable & mask) != 0 {
            // Output matrix: pads routed to the I²C0 controller carry the live
            // SDA/SCL wire the bit engine drives, not the GPIO_OUT latch.
            if let Some(lines) = &self.i2c_lines {
                match self.out_sel[pin as usize] & 0x1FF {
                    SIG_I2CEXT0_SCL => return Some(lines.scl()),
                    SIG_I2CEXT0_SDA => return Some(lines.sda()),
                    _ => {}
                }
            }
            return Some((self.out & mask) != 0);
        }
        Some((self.effective_input() & mask) != 0)
    }

    /// Record every watched pad's current level before a mutation. No-op (one
    /// branch) while no tap is installed. The tap is briefly taken out of
    /// `self` so `pad_level(&self)` can run while the scratchpad is written.
    #[inline]
    pub(crate) fn tap_snapshot(&mut self) {
        let Some(mut t) = self.tap.take() else {
            return;
        };
        for (k, &(pin, _)) in t.watched.iter().enumerate() {
            t.scratch[k] = self.pad_level(pin);
        }
        self.tap = Some(t);
    }

    /// Report watched pads whose level became known-different since the
    /// matching [`tap_snapshot`](Self::tap_snapshot), then re-sync the I²C
    /// line-cell registration if the write changed a watched pad's routing —
    /// so a pad handed to (or taken from) the I²C matrix keeps pushing edges
    /// from the correct source afterwards.
    #[inline]
    pub(crate) fn tap_report(&mut self) {
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
        self.sync_line_tap();
    }

    /// Channels whose watched pads currently route to the I²C0 SCL / SDA
    /// output-matrix signals (and are output-enabled) — the pads whose level
    /// changes are driven by the I²C bit engine rather than GPIO writes.
    fn routed_line_channels(&self) -> (Vec<u32>, Vec<u32>) {
        let mut scl = Vec::new();
        let mut sda = Vec::new();
        if let Some(t) = &self.tap {
            for &(pin, ch) in &t.watched {
                if pin >= PIN_COUNT || (self.enable & (1u32 << pin)) == 0 {
                    continue;
                }
                match self.out_sel[pin as usize] & 0x1FF {
                    SIG_I2CEXT0_SCL => scl.push(ch),
                    SIG_I2CEXT0_SDA => sda.push(ch),
                    _ => {}
                }
            }
        }
        (scl, sda)
    }

    /// Push the current routed-channel lists into the shared I²C line cell,
    /// but only when they changed (avoids mutex traffic on unrelated writes).
    fn sync_line_tap(&mut self) {
        let Some(lines) = self.i2c_lines.clone() else {
            return;
        };
        let (scl, sda) = self.routed_line_channels();
        let Some(t) = &mut self.tap else {
            return;
        };
        if t.line_scl_chs != scl || t.line_sda_chs != sda {
            t.line_scl_chs = scl.clone();
            t.line_sda_chs = sda.clone();
            lines.install_tap(Some(t.tap.clone()), scl, sda);
        }
    }

    fn out_sel_index(off: u64) -> Option<usize> {
        if (FUNC_OUT_SEL..FUNC_OUT_SEL + (PIN_COUNT as u64) * 4).contains(&off) {
            Some(((off - FUNC_OUT_SEL) / 4) as usize)
        } else {
            None
        }
    }

    pub fn out_value(&self) -> u32 {
        self.out
    }

    pub fn enable_value(&self) -> u32 {
        self.enable
    }

    pub fn set_pin_input(&mut self, pin: u8, level: bool) {
        assert!(pin < PIN_COUNT, "set_pin_input: pin {pin} >= {PIN_COUNT}");
        if level {
            self.external_levels |= 1u32 << pin;
        } else {
            self.external_levels &= !(1u32 << pin);
        }
        self.external_drive_mask |= 1u32 << pin;
    }

    fn pin_cfg_index(off: u64) -> Option<usize> {
        if (PIN0..PIN0 + (PIN_COUNT as u64) * 4).contains(&off) {
            Some(((off - PIN0) / 4) as usize)
        } else {
            None
        }
    }

    fn read_word(&self, word_off: u64) -> u32 {
        match word_off {
            BT_SELECT => self.bt_select,
            OUT | OUT_W1TS | OUT_W1TC => self.out,
            SDIO_SELECT => self.sdio_select,
            ENABLE | ENABLE_W1TS | ENABLE_W1TC => self.enable,
            STRAP => self.strap,
            IN => self.effective_input(),
            STATUS | STATUS_W1TS | STATUS_W1TC => self.status,
            PCPU_INT | PCPU_NMI_INT | CPUSDIO_INT => self.status,
            off => {
                if let Some(idx) = Self::out_sel_index(off) {
                    self.out_sel[idx]
                } else {
                    Self::pin_cfg_index(off)
                        .map(|idx| self.pin_cfg[idx])
                        .unwrap_or(0)
                }
            }
        }
    }

    fn write_word(&mut self, word_off: u64, value: u32) {
        let value = value & PIN_MASK;
        match word_off {
            BT_SELECT => self.bt_select = value,
            OUT => self.out = value,
            OUT_W1TS => self.out |= value,
            OUT_W1TC => self.out &= !value,
            SDIO_SELECT => self.sdio_select = value,
            ENABLE => self.enable = value,
            ENABLE_W1TS => self.enable |= value,
            ENABLE_W1TC => self.enable &= !value,
            STRAP | IN => {}
            STATUS => self.status = value,
            STATUS_W1TS => self.status |= value,
            STATUS_W1TC => self.status &= !value,
            PCPU_INT | PCPU_NMI_INT | CPUSDIO_INT => {}
            off => {
                if let Some(idx) = Self::out_sel_index(off) {
                    self.out_sel[idx] = value;
                } else if let Some(idx) = Self::pin_cfg_index(off) {
                    self.pin_cfg[idx] = value;
                }
            }
        }
    }

    fn write_byte_special(&mut self, word_off: u64, byte_off: u64, value: u8) -> bool {
        let mask = (value as u32) << (byte_off * 8);
        match word_off {
            OUT_W1TS => self.out |= mask & PIN_MASK,
            OUT_W1TC => self.out &= !(mask & PIN_MASK),
            ENABLE_W1TS => self.enable |= mask & PIN_MASK,
            ENABLE_W1TC => self.enable &= !(mask & PIN_MASK),
            STATUS_W1TS => self.status |= mask & PIN_MASK,
            STATUS_W1TC => self.status &= !(mask & PIN_MASK),
            _ => return false,
        }
        true
    }
}

impl Default for Esp32c3Gpio {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Esp32c3Gpio {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = offset & 3;
        self.tap_snapshot();
        if self.write_byte_special(word_off, byte_off, value) {
            self.tap_report();
            return Ok(());
        }
        let shift = byte_off * 8;
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << shift);
        word |= (value as u32) << shift;
        self.write_word(word_off, word);
        self.tap_report();
        Ok(())
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        if offset & 1 == 0 {
            let word_off = offset & !3;
            let shift = (offset & 3) * 8;
            if matches!(
                word_off,
                OUT_W1TS | OUT_W1TC | ENABLE_W1TS | ENABLE_W1TC | STATUS_W1TS | STATUS_W1TC
            ) {
                self.tap_snapshot();
                self.write_word(word_off, (value as u32) << shift);
                self.tap_report();
                return Ok(());
            }
        }
        self.write(offset, (value & 0xFF) as u8)?;
        self.write(offset + 1, (value >> 8) as u8)
    }

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

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "layout": "esp32c3",
            "odr": self.out,
            "idr": self.effective_input(),
            "enable": self.enable,
            "strap": self.strap,
            "status": self.status,
        })
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        if pin >= PIN_COUNT {
            return None;
        }
        Some((self.effective_input() & (1u32 << pin)) != 0)
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        if pin >= PIN_COUNT {
            return None;
        }
        Some((self.out & (1u32 << pin)) != 0)
    }

    fn read_gpio_pad(&self, pin: u8) -> Option<bool> {
        self.pad_level(pin)
    }

    fn gpio_routing(&self, pin: u8) -> Option<GpioRouting> {
        if pin >= PIN_COUNT {
            return None;
        }
        let mask = 1u32 << pin;
        if (self.enable & mask) == 0 {
            // Output driver disabled → the pad is an input. The input matrix
            // (FUNCn_IN_SEL, indexed by signal) is not tracked, so we cannot name
            // the signal — func stays None rather than a guess.
            return Some(GpioRouting {
                mode: GpioMode::Input,
                func: None,
            });
        }
        // Output driver enabled: consult the per-pad output-matrix selector.
        let sig = self.out_sel[pin as usize] & 0x1FF;
        if sig == SIG_GPIO_OUT {
            // Pad driven directly by the GPIO_OUT latch — a plain GPIO output.
            Some(GpioRouting {
                mode: GpioMode::Output,
                func: None,
            })
        } else {
            // Pad routed to a peripheral output signal (alternate function).
            Some(GpioRouting {
                mode: GpioMode::Af,
                func: c3_out_signal_name(sig).map(String::from),
            })
        }
    }

    fn set_gpio_input(&mut self, pin: u8, level: bool) -> bool {
        if pin >= PIN_COUNT {
            return false;
        }
        self.tap_snapshot();
        self.set_pin_input(pin, level);
        self.tap_report();
        true
    }

    fn install_logic_tap(
        &mut self,
        tap: &crate::logic_capture::LogicTap,
        watched: &[(u8, u32)],
    ) -> bool {
        if watched.is_empty() {
            self.tap = None;
            if let Some(lines) = &self.i2c_lines {
                lines.install_tap(None, Vec::new(), Vec::new());
            }
        } else {
            self.tap = Some(C3Tap {
                tap: tap.clone(),
                watched: watched.to_vec(),
                scratch: vec![None; watched.len()],
                // Seeded stale so the sync below always installs the current
                // routing into the line cell.
                line_scl_chs: vec![u32::MAX],
                line_sda_chs: vec![u32::MAX],
            });
            self.sync_line_tap();
        }
        true
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.cycle = self.cycle.wrapping_add(1);
        PeripheralTickResult::default()
    }

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
    use crate::bus::SystemBus;
    use labwired_config::{ChipDescriptor, SystemManifest};

    #[test]
    fn w1ts_and_w1tc_update_output_register() {
        let mut gpio = Esp32c3Gpio::new();
        gpio.write_u32(OUT_W1TS, (1 << 4) | (1 << 5)).unwrap();
        assert_eq!(gpio.out_value(), (1 << 4) | (1 << 5));

        gpio.write_u32(OUT_W1TC, 1 << 4).unwrap();
        assert_eq!(gpio.out_value(), 1 << 5);
    }

    #[test]
    fn c3_pin_config_uses_c3_pin0_offset() {
        let mut gpio = Esp32c3Gpio::new();
        gpio.write_u32(PIN0 + 3 * 4, 0x2 << 7).unwrap();
        assert_eq!(gpio.read_word(PIN0 + 3 * 4), 0x2 << 7);
        assert_eq!(gpio.read_word(0x88), 0);
    }

    #[test]
    fn reset_strap_selects_spi_flash_boot() {
        let mut gpio = Esp32c3Gpio::new();
        assert_eq!(gpio.read_word(STRAP), STRAP_SPI_FAST_FLASH_BOOT);

        gpio.write_u32(STRAP, 0).unwrap();
        assert_eq!(gpio.read_word(STRAP), STRAP_SPI_FAST_FLASH_BOOT);
    }

    #[test]
    fn gpio_routing_resolves_the_output_matrix() {
        use crate::peripherals::gpio::GpioMode;
        let mut g = Esp32c3Gpio::new();

        // pin5: output driver on + routed to I2CEXT0_SDA (signal index 54) → AF.
        g.write_u32(ENABLE_W1TS, 1 << 5).unwrap();
        g.write_u32(FUNC_OUT_SEL + 5 * 4, 54).unwrap();
        let r5 = g.gpio_routing(5).unwrap();
        assert_eq!(r5.mode, GpioMode::Af);
        assert_eq!(r5.func.as_deref(), Some("I2CEXT0_SDA"));

        // pin6: output driver on + GPIO_OUT sentinel (128) → plain output, no func.
        g.write_u32(ENABLE_W1TS, 1 << 6).unwrap();
        g.write_u32(FUNC_OUT_SEL + 6 * 4, SIG_GPIO_OUT).unwrap();
        let r6 = g.gpio_routing(6).unwrap();
        assert_eq!(r6.mode, GpioMode::Output);
        assert!(r6.func.is_none());

        // pin7: routed to FSPICLK (index 63).
        g.write_u32(ENABLE_W1TS, 1 << 7).unwrap();
        g.write_u32(FUNC_OUT_SEL + 7 * 4, 63).unwrap();
        assert_eq!(g.gpio_routing(7).unwrap().func.as_deref(), Some("FSPICLK"));

        // pin8: output driver off → input, func unknown (input matrix not tracked).
        let r8 = g.gpio_routing(8).unwrap();
        assert_eq!(r8.mode, GpioMode::Input);
        assert!(r8.func.is_none());

        // pin9: enabled but an unmapped signal index → AF with func null (no guess).
        g.write_u32(ENABLE_W1TS, 1 << 9).unwrap();
        g.write_u32(FUNC_OUT_SEL + 9 * 4, 200).unwrap();
        let r9 = g.gpio_routing(9).unwrap();
        assert_eq!(r9.mode, GpioMode::Af);
        assert!(r9.func.is_none());

        assert!(g.gpio_routing(PIN_COUNT).is_none(), "out-of-range pin");
    }

    #[test]
    fn esp32c3_input_pullup_releases_a_floating_pin_but_external_drive_wins() {
        const IO_MUX_GPIO4: u64 = 0x6000_9000 + 0x04 + 4 * 4;
        const GPIO_IN: u64 = 0x6000_4000 + IN;

        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
            .expect("read esp32c3 chip yaml");
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "esp32c3-input-pullup-test"
chip: "../chips/esp32c3.yaml"
"#,
        )
        .expect("parse system yaml");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("construct C3 bus");
        let gpio_idx = bus
            .find_peripheral_index_by_name("gpio")
            .expect("C3 GPIO is present");

        {
            let gpio = bus.peripherals[gpio_idx]
                .dev
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<Esp32c3Gpio>())
                .expect("C3 GPIO model");
            assert_eq!(
                gpio.read_gpio_input(4),
                Some(true),
                "the cold IO_MUX FUN_WPU bit releases a floating GPIO4"
            );
            assert_eq!(gpio.read_gpio_pad(4), Some(true));
        }

        assert_eq!(bus.read_u32(IO_MUX_GPIO4).unwrap(), 0x0000_0b00);
        bus.write_u32(IO_MUX_GPIO4, 0x0000_1a02)
            .expect("emulate Arduino pinMode(GPIO4, INPUT)");

        {
            let gpio = bus.peripherals[gpio_idx]
                .dev
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<Esp32c3Gpio>())
                .expect("C3 GPIO model");
            assert_eq!(gpio.read_gpio_input(4), Some(false), "INPUT clears FUN_WPU");
            assert_eq!(gpio.read_gpio_pad(4), Some(false));
        }

        bus.write_u32(IO_MUX_GPIO4, 0x0000_1b02)
            .expect("emulate Arduino pinMode(GPIO4, INPUT_PULLUP)");

        {
            let gpio = bus.peripherals[gpio_idx]
                .dev
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<Esp32c3Gpio>())
                .expect("C3 GPIO model");
            assert_eq!(gpio.read_gpio_input(4), Some(true), "pull-up releases high");
            assert_eq!(gpio.read_gpio_pad(4), Some(true));
            assert_ne!(gpio.snapshot()["idr"].as_u64().unwrap() & (1 << 4), 0);
        }
        assert_ne!(bus.read_u32(GPIO_IN).unwrap() & (1 << 4), 0);

        let gpio = bus.peripherals[gpio_idx]
            .dev
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<Esp32c3Gpio>())
            .expect("C3 GPIO model");
        assert!(gpio.set_gpio_input(4, false));
        assert_eq!(gpio.read_gpio_input(4), Some(false), "injected low wins");
        assert!(gpio.set_gpio_input(4, true));
        assert_eq!(gpio.read_gpio_input(4), Some(true), "injected high wins");
    }

    #[test]
    fn input_pullup_uses_the_same_effective_level_for_logic_capture() {
        use crate::logic_capture::{LogicTap, PadEvent};
        use crate::peripherals::esp32c3::io_mux::Esp32c3IoMux;

        let mut io_mux = Esp32c3IoMux::new();
        io_mux.write_u32(0x04 + 4 * 4, 1 << 8).unwrap();
        let mut gpio = Esp32c3Gpio::new();
        gpio.set_pad_controls(io_mux.pad_controls());
        let tap = LogicTap::new();
        assert!(gpio.install_logic_tap(&tap, &[(4, 0)]));

        assert!(gpio.set_gpio_input(4, false));
        assert_eq!(
            tap.take_events(),
            vec![PadEvent {
                ch: 0,
                cycle: 0,
                value: false,
            }]
        );
    }

    #[test]
    fn io_mux_pullup_write_pushes_logic_capture_edge() {
        use crate::logic_capture::{LogicTap, PadEvent};

        const IO_MUX_GPIO4: u64 = 0x6000_9000 + 0x04 + 4 * 4;
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
            .expect("read esp32c3 chip yaml");
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "esp32c3-input-pullup-capture-test"
chip: "../chips/esp32c3.yaml"
"#,
        )
        .expect("parse system yaml");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("construct C3 bus");
        let gpio_idx = bus
            .find_peripheral_index_by_name("gpio")
            .expect("C3 GPIO is present");
        crate::Bus::write_u32(&mut bus, IO_MUX_GPIO4, 0x0000_1a02)
            .expect("firmware configures GPIO4 as INPUT before arming capture");
        let tap = LogicTap::new();
        {
            let gpio = bus.peripherals[gpio_idx]
                .dev
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<Esp32c3Gpio>())
                .expect("C3 GPIO model");
            assert!(gpio.install_logic_tap(&tap, &[(4, 0)]));
            assert_eq!(gpio.read_gpio_pad(4), Some(false));
        }

        crate::Bus::write_u32(&mut bus, IO_MUX_GPIO4, 0x0000_1b02)
            .expect("firmware enables GPIO4 FUN_WPU");

        assert_eq!(
            tap.take_events(),
            vec![PadEvent {
                ch: 0,
                cycle: 0,
                value: true,
            }]
        );
    }

    #[test]
    fn io_mux_byte_and_halfword_writes_drive_pullups_and_emit_capture_edges() {
        use crate::logic_capture::{LogicTap, PadEvent};

        const IO_MUX_GPIO5: u64 = 0x6000_9000 + 0x04 + 5 * 4;
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
            .expect("read esp32c3 chip yaml");
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "esp32c3-input-pullup-width-test"
chip: "../chips/esp32c3.yaml"
"#,
        )
        .expect("parse system yaml");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("construct C3 bus");
        let gpio_idx = bus
            .find_peripheral_index_by_name("gpio")
            .expect("C3 GPIO is present");

        crate::Bus::write_u32(&mut bus, IO_MUX_GPIO5, 0x0000_1a02)
            .expect("firmware configures GPIO5 as INPUT");
        assert_eq!(
            crate::Bus::read_u32(&bus, IO_MUX_GPIO5).unwrap(),
            0x0000_1a02
        );

        let tap = LogicTap::new();
        {
            let gpio = bus.peripherals[gpio_idx]
                .dev
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<Esp32c3Gpio>())
                .expect("C3 GPIO model");
            assert!(gpio.install_logic_tap(&tap, &[(5, 0)]));
            assert_eq!(gpio.read_gpio_pad(5), Some(false));
        }

        crate::Bus::write_u8(&mut bus, IO_MUX_GPIO5 + 1, 0x1b)
            .expect("byte write enables GPIO5 FUN_WPU");
        assert_eq!(
            crate::Bus::read_u32(&bus, IO_MUX_GPIO5).unwrap(),
            0x0000_1b02
        );
        assert_eq!(
            tap.take_events(),
            vec![PadEvent {
                ch: 0,
                cycle: 0,
                value: true,
            }]
        );

        crate::Bus::write_u16(&mut bus, IO_MUX_GPIO5, 0x1a02)
            .expect("halfword write clears GPIO5 FUN_WPU");
        assert_eq!(
            crate::Bus::read_u32(&bus, IO_MUX_GPIO5).unwrap(),
            0x0000_1a02
        );
        assert_eq!(
            tap.take_events(),
            vec![PadEvent {
                ch: 0,
                cycle: 0,
                value: false,
            }]
        );

        crate::Bus::write_u16(&mut bus, IO_MUX_GPIO5, 0x1b02)
            .expect("halfword write enables GPIO5 FUN_WPU");
        assert_eq!(
            tap.take_events(),
            vec![PadEvent {
                ch: 0,
                cycle: 0,
                value: true,
            }]
        );
    }

    #[test]
    fn machine_snapshot_restores_io_mux_pads_date_and_gpio_wiring() {
        const IO_MUX_PIN_CTRL: u64 = 0x6000_9000;
        const IO_MUX_GPIO4: u64 = 0x6000_9000 + 0x04 + 4 * 4;
        const IO_MUX_DATE: u64 = 0x6000_90fc;
        const GPIO_IN: u64 = 0x6000_4000 + IN;

        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
            .expect("read esp32c3 chip yaml");
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "esp32c3-input-pullup-machine-snapshot-test"
chip: "../chips/esp32c3.yaml"
"#,
        )
        .expect("parse system yaml");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("construct C3 bus");
        let cpu = crate::system::riscv::configure_riscv(&mut bus);
        let mut machine = crate::Machine::new(cpu, bus);

        machine
            .bus
            .write_u32(IO_MUX_GPIO4, 0x0000_1a02)
            .expect("set GPIO4 INPUT");
        assert_eq!(machine.bus.read_u32(GPIO_IN).unwrap() & (1 << 4), 0);

        machine
            .bus
            .write_u32(IO_MUX_GPIO4, 0x0000_1b02)
            .expect("set GPIO4 INPUT_PULLUP");
        machine
            .bus
            .write_u32(IO_MUX_PIN_CTRL, 0x321)
            .expect("write IO_MUX PIN_CTRL");
        machine
            .bus
            .write_u32(IO_MUX_DATE, 0x0bad_c0de)
            .expect("write IO_MUX DATE");
        let snapshot = machine.snapshot();

        machine
            .bus
            .write_u32(IO_MUX_GPIO4, 0x0000_1a02)
            .expect("mutate GPIO4 back to INPUT");
        machine
            .bus
            .write_u32(IO_MUX_PIN_CTRL, 0x654)
            .expect("mutate IO_MUX PIN_CTRL");
        machine
            .bus
            .write_u32(IO_MUX_DATE, 0xfeed_face)
            .expect("mutate IO_MUX DATE");
        assert_eq!(machine.bus.read_u32(GPIO_IN).unwrap() & (1 << 4), 0);

        machine
            .apply_snapshot(snapshot)
            .expect("restore full machine snapshot");
        assert_ne!(
            machine.bus.read_u32(GPIO_IN).unwrap() & (1 << 4),
            0,
            "GPIO retains the shared IO_MUX pad-control connection after restore"
        );
        assert_eq!(machine.bus.read_u32(IO_MUX_GPIO4).unwrap(), 0x0000_1b02);
        assert_eq!(machine.bus.read_u32(IO_MUX_PIN_CTRL).unwrap(), 0x321);
        assert_eq!(machine.bus.read_u32(IO_MUX_DATE).unwrap(), 0x0bad_c0de);
    }

    #[test]
    fn fresh_machine_runtime_snapshot_restores_io_mux_state_and_gpio_wiring() {
        const IO_MUX_PIN_CTRL: u64 = 0x6000_9000;
        const IO_MUX_GPIO4: u64 = 0x6000_9000 + 0x04 + 4 * 4;
        const IO_MUX_DATE: u64 = 0x6000_90fc;
        const GPIO_IN: u64 = 0x6000_4000 + IN;

        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
            .expect("read esp32c3 chip yaml");
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "esp32c3-input-pullup-runtime-snapshot-test"
chip: "../chips/esp32c3.yaml"
"#,
        )
        .expect("parse system yaml");

        let mut source_bus =
            SystemBus::from_config(&chip, &manifest).expect("construct source C3 bus");
        let source_cpu = crate::system::riscv::configure_riscv(&mut source_bus);
        let mut source = crate::Machine::new(source_cpu, source_bus);
        source
            .bus
            .write_u32(IO_MUX_GPIO4, 0xfeed_1a02)
            .expect("set source GPIO4 INPUT without FUN_WPU");
        source
            .bus
            .write_u32(IO_MUX_PIN_CTRL, 0xa5a5_f123)
            .expect("write source IO_MUX PIN_CTRL");
        source
            .bus
            .write_u32(IO_MUX_DATE, 0x0bad_c0de)
            .expect("write source IO_MUX DATE");
        assert_eq!(source.bus.read_u32(GPIO_IN).unwrap() & (1 << 4), 0);
        let snapshot = source.take_runtime_snapshot();

        let mut resumed_bus =
            SystemBus::from_config(&chip, &manifest).expect("construct fresh C3 bus");
        let resumed_cpu = crate::system::riscv::configure_riscv(&mut resumed_bus);
        let mut resumed = crate::Machine::new(resumed_cpu, resumed_bus);
        assert_ne!(
            resumed.bus.read_u32(GPIO_IN).unwrap() & (1 << 4),
            0,
            "fresh C3 IO_MUX starts with its descriptor FUN_WPU reset"
        );

        resumed
            .apply_runtime_snapshot(&snapshot)
            .expect("restore runtime snapshot into fresh machine");
        assert_eq!(resumed.bus.read_u32(GPIO_IN).unwrap() & (1 << 4), 0);
        assert_eq!(
            resumed.bus.read_u32(IO_MUX_GPIO4).unwrap(),
            0xfeed_1a02,
            "the restored IO_MUX state remains shared with GPIO"
        );
        assert_eq!(resumed.bus.read_u32(IO_MUX_PIN_CTRL).unwrap(), 0xa5a5_f123);
        assert_eq!(resumed.bus.read_u32(IO_MUX_DATE).unwrap(), 0x0bad_c0de);
    }
}
