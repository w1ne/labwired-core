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
use crate::pins::{External, InputChange, PinPort};
use crate::{MmioAccessClass, Peripheral, PeripheralTickResult, SimResult};

const PIN_COUNT: u8 = 26;
const PIN_MASK: u32 = (1u32 << PIN_COUNT) - 1;
// C3 TRM GPIO interrupt registers implement only GPIO0..21. This path
// models rising/falling/any-edge CPU interrupts; level, NMI and light-sleep
// wake-up are outside the customer firmware prerequisite.
const IRQ_PIN_MASK: u32 = (1 << 22) - 1;
const GPIO_INTR_SOURCE: u32 = 16;

/// `GPIO_FUNCn_OUT_SEL_CFG_REG` base (C3 TRM §5.12): per-PAD output routing.
/// Bits [8:0] select which peripheral output signal drives pad `n`; the sentinel
/// `SIG_GPIO_OUT` (128) means the pad is a plain GPIO output (GPIO_OUT latch).
const FUNC_OUT_SEL: u64 = 0x554;
/// `GPIO_FUNCn_IN_SEL_CFG_REG` base. The C3 input matrix is indexed by
/// peripheral signal rather than GPIO pad; I²C0 uses signal indices 53/54.
const FUNC_IN_SEL: u64 = 0x154;
const INPUT_SIGNAL_COUNT: usize = 128;
const MATRIX_INPUT_SELECT: u32 = 1 << 6;
const SIG_GPIO_OUT: u32 = 128;
/// GPIO-matrix output signal indices of the I²C0 controller (esp-idf
/// `soc/esp32c3/include/soc/gpio_sig_map.h`).
const SIG_I2CEXT0_SCL: u32 = 53;
const SIG_I2CEXT0_SDA: u32 = 54;
/// GPIO-matrix OUTPUT signal indices of GP-SPI2 (FSPI) — the controller
/// arduino-esp32's `SPIClass SPI(FSPI)` drives. esp-idf
/// `soc/esp32c3/include/soc/gpio_sig_map.h`: `FSPICLK_OUT_IDX` :104,
/// `FSPID_OUT_IDX` :108, `FSPICS0_OUT_IDX` :114.
///
/// ⚠️ These are C3 numbers. The S3 spells the same three signals 101/103/110
/// and the classic ESP32's VSPI is 63/65/68 — the index space is PER CHIP, and
/// borrowing a sibling's constant fails silently in BOTH directions: a plain
/// pad decodes as routed and a routed pad as plain.
const SIG_FSPICLK: u32 = 63;
const SIG_FSPID: u32 = 65;
const SIG_FSPICS0: u32 = 68;
/// GPIO-matrix OUTPUT signal indices of the UART transmitters (esp-idf
/// `gpio_sig_map.h` :29 `U0TXD_OUT_IDX`, :35 `U1TXD_OUT_IDX`).
///
/// ⚠️ The DEFAULT `U0TXD` pad (GPIO21) reaches the pin through IO_MUX
/// function 0 (`io_mux_reg.h` :267), NOT through this matrix, so a stock
/// `Serial.begin()` leaves these routes dark and the pad rightly keeps reading
/// the GPIO latch. The route lights up when firmware remaps TX to any other pin
/// — `uart_set_pin` falls back to `gpio_matrix_out` for a non-IO_MUX pad, and
/// UART1 has no IO_MUX route on the C3 at all, so it is matrix-only.
const SIG_U0TXD: u32 = 6;
const SIG_U1TXD: u32 = 9;

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
    /// `GPIO_FUNCn_IN_SEL_CFG` per peripheral signal. Unlike `out_sel`, this
    /// matrix is signal-indexed: a programmed entry tells a peripheral which
    /// physical pad it reads. We retain it so I²C attachment can honor real
    /// SDA/SCL routing rather than just controller identity.
    in_sel: [u32; INPUT_SIGNAL_COUNT],
    /// Live C3 I²C0 matrix state shared with the I²C controller. GPIO owns the
    /// register words; every matrix/enable write refreshes this cell.
    i2c_matrix_route: super::i2c::C3I2cMatrixRoute,
    /// Shared IO_MUX per-pad register words. This is intentionally separate
    /// from the output matrix: the pad's weak pull-up is an electrical input
    /// condition, not a routed peripheral signal.
    pad_controls: Option<super::io_mux::PadControls>,
    /// Live I²C0 SDA/SCL line levels, shared with the C3 I²C bit engine (see
    /// `crate::bus::SystemBus::wire_esp32c3_i2c_pads`). Pads whose output
    /// matrix routes I2CEXT0_SCL/SDA read the wire here instead of the
    /// GPIO_OUT latch.
    i2c_lines: Option<std::sync::Arc<super::i2c::I2cLineLevels>>,
    /// Pads bound to peripheral wires, resolved against this port's live output
    /// matrix through the shared seam. The `i2c_lines` handle above stays for
    /// the C3's bidirectional matrix/ACK logic, which reads the wire by role.
    pad_routes: crate::peripherals::pad_routing::PadRoutes,
    /// `Some` while the logic analyzer watches pads on this port in push mode
    /// ([`PinPort::install_watch`]). Not snapshot state — the watch is
    /// re-armed by the frontend after a resume.
    watch: Option<crate::pins::PadWatch>,
    /// Matrix-line change detector for the scheduler chain.
    irq_watch: crate::peripherals::esp_gpio_net::IrqLevelWatch,
    /// `ETS_GPIO_INTR_SOURCE` of the chip this block sits on: 16 on the C3,
    /// 30 on the C6 (which reuses this model).
    intr_source: u32,
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
            // Reset value 0x80 = SIG_GPIO_OUT, the matrix bypass (esp32c3.svd
            // `FUNCn_OUT_SEL_CFG` resetValue). Seeding 0 made every enabled
            // pad read as routed to matrix signal 0 until firmware wrote it.
            out_sel: [SIG_GPIO_OUT; PIN_COUNT as usize],
            in_sel: [0; INPUT_SIGNAL_COUNT],
            i2c_matrix_route: std::sync::Arc::new(std::sync::Mutex::new(
                super::i2c::C3I2cMatrixRouteState::default(),
            )),
            pad_controls: None,
            i2c_lines: None,
            pad_routes: crate::peripherals::pad_routing::PadRoutes::new(),
            watch: None,
            irq_watch: Default::default(),
            intr_source: GPIO_INTR_SOURCE,
            cycle: 0,
            anchor_tick: 0,
        }
    }

    /// The same block on a chip whose interrupt matrix numbers the GPIO
    /// interrupt differently: the ESP32-C6 (`ETS_GPIO_INTR_SOURCE` = 30,
    /// `esp32c6.svd`).
    pub fn with_intr_source(source: u32) -> Self {
        Self {
            intr_source: source,
            ..Self::new()
        }
    }

    /// Wire the shared I²C0 line-level cell (the same `Arc` the C3 I²C bit
    /// engine drives) so matrix-routed pads carry the real waveform.
    pub(crate) fn set_i2c_lines(&mut self, lines: std::sync::Arc<super::i2c::I2cLineLevels>) {
        // Bind every pad to both I²C signals through the shared routing seam;
        // which one is live at any moment is decided by the output matrix.
        let wire = lines.pad_lines().clone();
        for pin in 0..PIN_COUNT {
            self.pad_routes
                .bind(&wire, pin, Some(SIG_I2CEXT0_SCL), 0, "I2CEXT0_SCL");
            self.pad_routes
                .bind(&wire, pin, Some(SIG_I2CEXT0_SDA), 1, "I2CEXT0_SDA");
        }
        self.i2c_lines = Some(lines);
    }

    /// Bind GP-SPI2's SCK/MOSI/CS wire to every pad the output matrix can route
    /// it to. Which pad is live at any moment is then decided by
    /// `FUNCn_OUT_SEL_CFG`, through the shared routing seam — exactly as for
    /// I²C above. MISO is deliberately not bound; see
    /// [`crate::peripherals::esp_gpspi_wire`].
    pub(crate) fn bind_spi_lines(
        &mut self,
        lines: &std::sync::Arc<crate::peripherals::pad_lines::PadLines>,
    ) {
        use crate::peripherals::esp_gpspi_wire::{LINE_CS, LINE_MOSI, LINE_SCK};
        for pin in 0..PIN_COUNT {
            self.pad_routes
                .bind(lines, pin, Some(SIG_FSPICLK), LINE_SCK, "SPI2_SCK");
            self.pad_routes
                .bind(lines, pin, Some(SIG_FSPID), LINE_MOSI, "SPI2_MOSI");
            self.pad_routes
                .bind(lines, pin, Some(SIG_FSPICS0), LINE_CS, "SPI2_CS");
        }
    }

    /// Bind one UART's TX wire to every pad the output matrix can route it to.
    ///
    /// TX ONLY. Nothing in the engine drives the RX line, so a bound RX pad
    /// would report a confident constant idle-high — including while an attached
    /// GPS or modem was actually sending. That is worse than the GPIO-latch
    /// fallback it would replace, because it looks authoritative. Same call as
    /// `wire_rp2040_uart_pads` documents. RX joins the table when something
    /// drives it, not before.
    pub(crate) fn bind_uart_tx_lines(
        &mut self,
        instance: usize,
        lines: &std::sync::Arc<crate::peripherals::pad_lines::PadLines>,
    ) {
        let (signal, func) = match instance {
            0 => (SIG_U0TXD, "UART0_TX"),
            1 => (SIG_U1TXD, "UART1_TX"),
            // The C3 has two UARTs (`SOC_UART_NUM` = 2) and no U2 signal exists
            // in its matrix at all, so there is nothing to bind.
            _ => return,
        };
        for pin in 0..PIN_COUNT {
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

    /// The output-matrix signal `pin` currently carries — the selector the
    /// shared routing seam resolves bindings against.
    ///
    /// `None` unless the pad's output driver is enabled, because a pad that is
    /// not driving is showing its input level, not the peripheral's wire. That
    /// condition is part of the selector rather than the binding so one rule
    /// covers pad reads and push-capture registration alike.
    fn matrix_signal(&self, pin: u8) -> Option<u32> {
        if pin >= PIN_COUNT || (self.enable & (1u32 << pin)) == 0 {
            return None;
        }
        Some(self.out_sel[pin as usize] & 0x1FF)
    }

    /// Clone the live GPIO-matrix route cell for the C3 I²C controller. It is
    /// intentionally shared rather than copied: `Wire.begin(sda, scl)` writes
    /// GPIO registers after system construction, and the slave gate must see
    /// those later firmware writes.
    pub(crate) fn i2c_matrix_route_state(&self) -> super::i2c::C3I2cMatrixRoute {
        self.i2c_matrix_route.clone()
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

    /// Pads whose input buffer (IO_MUX `FUN_IE`) is enabled. Pads without an
    /// IO_MUX word (GPIO22..25), or a GPIO with no IO_MUX wired, have no
    /// buffer control to model and count as enabled.
    fn io_mux_input_enable_mask(&self) -> u32 {
        let Some(controls) = &self.pad_controls else {
            return PIN_MASK;
        };
        let controls = controls
            .read()
            .expect("ESP32-C3 IO_MUX pad controls poisoned");
        (0..PIN_COUNT).fold(0, |mask, pin| {
            let enabled = controls
                .get(pin as usize)
                .is_none_or(|word| word & super::io_mux::FUN_IE != 0);
            if enabled {
                mask | (1 << pin)
            } else {
                mask
            }
        })
    }

    /// Level the pad's own output driver forces, or `None` while the driver
    /// is off (or released as an open-drain net pad).
    fn driven_level(&self, pin: u8) -> Option<bool> {
        let mask = 1u32 << pin;
        if (self.enable & mask) == 0 || self.released_held_pad(pin) {
            return None;
        }
        // Output matrix: pads routed to the I²C0 controller carry the live
        // SDA/SCL wire the bit engine drives, not the GPIO_OUT latch.
        if let Some(level) = self.pad_routes.level(pin, |p| self.matrix_signal(p)) {
            return Some(level);
        }
        Some((self.out & mask) != 0)
    }

    /// Firmware-visible input word. An explicit external drive always beats a
    /// weak internal pull-up; otherwise the raw IO_MUX `FUN_WPU` bit supplies
    /// the released level, including its descriptor-defined cold reset. A pad
    /// whose output driver is on and whose input buffer (`FUN_IE`) is enabled
    /// samples the level it drives itself, so `digitalRead()` on an OUTPUT pin
    /// (Arduino sets `FUN_IE` for it) reads back the driven level.
    fn effective_input(&self) -> u32 {
        let mut input = (self.external_levels & self.external_drive_mask)
            | (self.io_mux_pullup_mask() & !self.external_drive_mask);
        let ie = self.io_mux_input_enable_mask();
        for pin in 0..PIN_COUNT {
            let mask = 1u32 << pin;
            if ie & mask == 0 {
                continue;
            }
            match self.driven_level(pin) {
                Some(true) => input |= mask,
                Some(false) => input &= !mask,
                None => {}
            }
        }
        input & PIN_MASK
    }

    /// Direction-aware pad level — the single truth `read_gpio_pad` and the
    /// push-capture tap both read.
    fn pad_level(&self, pin: u8) -> Option<bool> {
        if pin >= PIN_COUNT {
            return None;
        }
        // ENABLE is the output driver: enabled pins show the driving signal,
        // everything else shows the (externally driven) input level. A pad
        // in open drain holding a 1 that the outside holds is released and
        // shows the wire.
        if let Some(level) = self.driven_level(pin) {
            return Some(level);
        }
        Some((self.effective_input() & (1u32 << pin)) != 0)
    }

    /// A pad in open drain (`PAD_DRIVER`) whose output latch holds a 1 and
    /// that the outside world holds ([`PinPort::set_external`]): its output
    /// stage is off and `GPIO_IN` reads the wire. With nothing outside the
    /// model keeps its old reading (ENABLE alone decides), standing in for a
    /// board pull-up.
    fn released_held_pad(&self, pin: u8) -> bool {
        let mask = 1u32 << pin;
        self.external_drive_mask & mask != 0
            && self.out & mask != 0
            && self.pin_cfg[pin as usize] & crate::peripherals::esp_gpio_net::PIN_PAD_DRIVER != 0
    }

    /// The pad's own output stage ([`PinPort::driver`]). A pad the matrix
    /// routes to a peripheral that publishes its wire is driven by it; one
    /// routed to a peripheral that publishes nothing has no known drive
    /// (`None`), so a world refuses it. Otherwise ENABLE drives the latch and
    /// an open-drain (`PAD_DRIVER`) 1 is released. The pull is the IO_MUX
    /// `FUN_WPU` bit.
    fn pad_driver(&self, pin: u8) -> Option<crate::pins::PadDriver> {
        use crate::pins::{PadDriver, Pull};
        if pin >= PIN_COUNT {
            return None;
        }
        let mask = 1u32 << pin;
        let pull = if self.io_mux_pullup_mask() & mask != 0 {
            Pull::Up
        } else {
            Pull::None
        };
        if let Some(sig) = self.matrix_signal(pin) {
            if sig != SIG_GPIO_OUT {
                return self
                    .pad_routes
                    .level(pin, |p| self.matrix_signal(p))
                    .map(|level| PadDriver::drive(level).with_pull(pull));
            }
        }
        if self.enable & mask == 0 {
            return Some(PadDriver::released(pull));
        }
        Some(
            PadDriver::output(
                self.out & mask != 0,
                self.pin_cfg[pin as usize] & crate::peripherals::esp_gpio_net::PIN_PAD_DRIVER != 0,
            )
            .with_pull(pull),
        )
    }

    /// The `ETS_GPIO_INTR_SOURCE` matrix line.
    fn irq_line(&self) -> bool {
        self.cpu_interrupt_status() != 0
    }

    /// Run a pad mutation bracketed for push capture.
    #[inline]
    fn mutate_pads<R>(&mut self, mutate: impl FnOnce(&mut Self) -> R) -> R {
        crate::pins::watch_begin(self);
        let r = mutate(self);
        crate::pins::watch_end(self);
        r
    }

    /// Re-register watched pads with the wires that drive them, so a pad the
    /// matrix hands over (or takes back) follows its new source.
    fn sync_line_tap(&mut self) {
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

    fn out_sel_index(off: u64) -> Option<usize> {
        if (FUNC_OUT_SEL..FUNC_OUT_SEL + (PIN_COUNT as u64) * 4).contains(&off) {
            Some(((off - FUNC_OUT_SEL) / 4) as usize)
        } else {
            None
        }
    }

    fn in_sel_index(off: u64) -> Option<usize> {
        if (FUNC_IN_SEL..FUNC_IN_SEL + (INPUT_SIGNAL_COUNT as u64) * 4).contains(&off) {
            Some(((off - FUNC_IN_SEL) / 4) as usize)
        } else {
            None
        }
    }

    /// Recompute the two C3 I²C0 signal paths from GPIO's actual matrix
    /// registers. A device physically wired to a pair responds only when both
    /// controller outputs are enabled and routed there *and* both controller
    /// inputs select the same pads.
    fn refresh_i2c_matrix_route(&mut self) {
        let mut sda_output_mask = 0u32;
        let mut scl_output_mask = 0u32;
        for pin in 0..PIN_COUNT as usize {
            if self.enable & (1 << pin) == 0 {
                continue;
            }
            match self.out_sel[pin] & 0x1FF {
                SIG_I2CEXT0_SDA => sda_output_mask |= 1 << pin,
                SIG_I2CEXT0_SCL => scl_output_mask |= 1 << pin,
                _ => {}
            }
        }
        let input_pad = |signal: u32| {
            let word = self.in_sel[signal as usize];
            if word & MATRIX_INPUT_SELECT == 0 {
                return None;
            }
            let pin = (word & 0x1F) as u8;
            (pin < PIN_COUNT).then_some(pin)
        };
        self.i2c_matrix_route
            .lock()
            .expect("ESP32-C3 GPIO I2C matrix route poisoned")
            .set(
                sda_output_mask,
                scl_output_mask,
                input_pad(SIG_I2CEXT0_SDA),
                input_pad(SIG_I2CEXT0_SCL),
            );
    }

    pub fn out_value(&self) -> u32 {
        self.out
    }

    pub fn enable_value(&self) -> u32 {
        self.enable
    }

    pub fn set_pin_input(&mut self, pin: u8, level: bool) {
        assert!(pin < PIN_COUNT, "set_pin_input: pin {pin} >= {PIN_COUNT}");
        let before = self.effective_input() & (1 << pin) != 0;
        if level {
            self.external_levels |= 1u32 << pin;
        } else {
            self.external_levels &= !(1u32 << pin);
        }
        self.external_drive_mask |= 1u32 << pin;
        if before != level {
            self.latch_pin_edge(pin, level);
        }
    }

    /// Latch `GPIO_STATUS` for an edge of pad `pin` to `level` when
    /// `PINn.INT_TYPE` selects it (1 rising, 2 falling, 3 any edge).
    fn latch_pin_edge(&mut self, pin: u8, level: bool) {
        if pin < 22 {
            let kind = (self.pin_cfg[pin as usize] >> 7) & 7;
            if matches!((kind, level), (1, true) | (2, false) | (3, _)) {
                self.status |= 1 << pin;
            }
        }
    }

    /// Run `f` (a register write that can move the pad's own drive) and latch
    /// the interrupts that move causes.
    ///
    /// ESP32-C3 TRM, GPIO "Interrupt": the interrupt logic samples the pad
    /// through the IO_MUX input buffer, so with `FUN_IE` set a pad's own
    /// output edge (OUT/ENABLE writes, or the output matrix handing the pad
    /// to a driving peripheral) latches `GPIO_STATUS` exactly like an
    /// external one; `effective_input` already gates on `FUN_IE`. Only the
    /// writes that can move a drive pay for the before/after sample.
    fn with_own_drive_edges(&mut self, word_off: u64, f: impl FnOnce(&mut Self)) {
        let moves_drive = matches!(
            word_off,
            OUT | OUT_W1TS | OUT_W1TC | ENABLE | ENABLE_W1TS | ENABLE_W1TC
        ) || Self::out_sel_index(word_off).is_some();
        if !moves_drive {
            return f(self);
        }
        let before = self.effective_input();
        f(self);
        let changed = before ^ self.effective_input();
        if changed != 0 {
            let after = self.effective_input();
            for pin in 0..PIN_COUNT {
                if changed & (1 << pin) != 0 {
                    self.latch_pin_edge(pin, after & (1 << pin) != 0);
                }
            }
        }
    }

    fn cpu_interrupt_status(&self) -> u32 {
        let mut pending = self.status & IRQ_PIN_MASK;
        let mut enabled = 0;
        while pending != 0 {
            let pin = pending.trailing_zeros();
            let bit = 1 << pin;
            if self.pin_cfg[pin as usize] & (1 << 13) != 0 {
                enabled |= bit;
            }
            pending &= !bit;
        }
        enabled
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
            PCPU_INT => self.cpu_interrupt_status(),
            PCPU_NMI_INT | CPUSDIO_INT => self.status,
            off => {
                if let Some(idx) = Self::out_sel_index(off) {
                    self.out_sel[idx]
                } else if let Some(idx) = Self::in_sel_index(off) {
                    self.in_sel[idx]
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
                } else if let Some(idx) = Self::in_sel_index(off) {
                    self.in_sel[idx] = value;
                } else if let Some(idx) = Self::pin_cfg_index(off) {
                    self.pin_cfg[idx] = value;
                }
            }
        }
        self.refresh_i2c_matrix_route();
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

impl PinPort for Esp32c3Gpio {
    fn pin_count(&self) -> u8 {
        PIN_COUNT
    }

    fn driver(&self, pin: u8) -> Option<crate::pins::PadDriver> {
        self.pad_driver(pin)
    }

    fn external(&self, pin: u8) -> External {
        let mask = 1u32 << (pin & 31);
        if pin < PIN_COUNT && self.external_drive_mask & mask != 0 {
            External::Level(self.external_levels & mask != 0)
        } else {
            External::Released
        }
    }

    /// A level has electrical authority over the IO_MUX pull-up
    /// (`set_pin_input`); releasing hands the pad back to the pull-up (or
    /// the latch, see `released_held_pad`). An input change latches
    /// `GPIO_STATUS` per `INT_TYPE`.
    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        if pin >= PIN_COUNT {
            return None;
        }
        let mask = 1u32 << pin;
        let before = self.effective_input() & mask != 0;
        self.mutate_pads(|g| match ext {
            External::Level(level) => g.set_pin_input(pin, level),
            External::Released => {
                g.external_drive_mask &= !mask;
                let after = g.effective_input() & mask != 0;
                if after != before {
                    g.latch_pin_edge(pin, after);
                }
            }
        });
        Some(InputChange {
            before,
            after: self.effective_input() & mask != 0,
        })
    }

    fn input(&self, pin: u8) -> Option<bool> {
        Peripheral::read_gpio_input(self, pin)
    }

    /// `pad_level`: the driving signal on an enabled pad, the input word
    /// otherwise. A released open-drain pad nothing outside holds reads its
    /// latch (see `released_held_pad`).
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
                // Seeded stale so the sync below always installs the current
                // routing into the line cell.
                self.pad_routes.invalidate_registrations();
                self.sync_line_tap();
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
        self.sync_line_tap();
    }
}

impl Peripheral for Esp32c3Gpio {
    /// Classify the input-data register (`IN`, 0x3C) and the boot-strap latch
    /// (`STRAP`) as pure freerunning samples: reading them observes pad levels
    /// without mutating any peripheral or world state. This lets the idle
    /// fast-forward *timer-poll coalesce* survive a firmware idle loop that
    /// samples a button (`digitalRead`) alongside a `millis()`/SYSTIMER poll —
    /// e.g. the Arduino watch/clock demos, whose `loop()` spins on `millis()`
    /// while polling MODE/SET buttons. Without this, every such iteration books
    /// a side-effecting MMIO access, `take_timer_poll_coalesce_eligible()`
    /// returns false, and idle FF never engages — so the idle wait is simulated
    /// cycle-by-cycle (hundreds of millions of real cycles per device-second)
    /// and the guest's time source barely advances. The bus stays CPU-agnostic:
    /// this is the same [`MmioAccessClass`] opt-in the SYSTIMER model uses for
    /// its snapshot registers, not a chip-specific rule baked into the bus.
    ///
    /// Only genuinely inert reads are marked side-effect-free; every write and
    /// every status/interrupt register stays [`SideEffecting`](MmioAccessClass::SideEffecting)
    /// so a display bit-bang or an INT_ST poll still disqualifies the skip.
    fn mmio_access_class(&self, offset: u64) -> MmioAccessClass {
        match offset & !3 {
            IN | STRAP => MmioAccessClass::SideEffectFree,
            _ => MmioAccessClass::SideEffecting,
        }
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        Some(((self.read_word(word_off) >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = offset & 3;
        self.mutate_pads(|g| {
            let mut special = false;
            g.with_own_drive_edges(word_off, |g| {
                special = g.write_byte_special(word_off, byte_off, value);
            });
            if special {
                g.refresh_i2c_matrix_route();
                return;
            }
            let shift = byte_off * 8;
            let mut word = g.read_word(word_off);
            word &= !(0xFFu32 << shift);
            word |= (value as u32) << shift;
            g.with_own_drive_edges(word_off, |g| g.write_word(word_off, word));
        });
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
                self.mutate_pads(|g| {
                    g.with_own_drive_edges(word_off, |g| {
                        g.write_word(word_off, (value as u32) << shift)
                    })
                });
                return Ok(());
            }
        }
        self.write(offset, (value & 0xFF) as u8)?;
        self.write(offset + 1, (value >> 8) as u8)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset & 3 == 0 {
            self.mutate_pads(|g| g.with_own_drive_edges(offset, |g| g.write_word(offset, value)));
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

    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        Some(self)
    }

    fn gpio_routing(&self, pin: u8) -> Option<GpioRouting> {
        if pin >= PIN_COUNT {
            return None;
        }
        let mask = 1u32 << pin;
        if (self.enable & mask) == 0 {
            // Output driver disabled → the pad is an input. FUNCn_IN_SEL is
            // signal-indexed (not pad-indexed), so a pad can feed several
            // peripheral inputs; the UI intentionally reports no single
            // function rather than guessing one.
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

    /// One event per change of the GPIO matrix line, so a walk-free bus
    /// re-derives the interrupt-matrix sources after an edge from outside or
    /// an acknowledge in `STATUS_W1TC` (see
    /// [`crate::peripherals::esp_gpio_net`]).
    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        let level = self.irq_line();
        self.irq_watch
            .take(level)
            .map(|token| vec![(0, token)])
            .unwrap_or_default()
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.cycle = self.cycle.wrapping_add(1);
        PeripheralTickResult::default()
    }

    fn uses_scheduler(&self) -> bool {
        true
    }

    fn matrix_irq_sources_into(&self, out: &mut Vec<u32>) {
        if self.irq_line() {
            out.push(self.intr_source);
        }
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
    use crate::Bus;
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
    fn side_effect_free_peek_exposes_modeled_output_register() {
        let mut gpio = Esp32c3Gpio::new();
        gpio.write_u32(OUT_W1TS, 1 << 8).unwrap();

        let bytes = (0..4)
            .map(|byte| gpio.peek(OUT + byte).expect("modeled GPIO OUT byte"))
            .collect::<Vec<_>>();
        assert_eq!(u32::from_le_bytes(bytes.try_into().unwrap()), 1 << 8);
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
        assert!(gpio.install_watch(crate::pins::PadWatch::probes(&tap, &[(4, 0)])));

        assert!(gpio.set_gpio_input(4, false));
        assert_eq!(
            tap.take_events(),
            vec![PadEvent {
                ch: 0,
                cycle: 0,
                value: false,
                drive: Some(crate::logic_capture::PadDrive::Driven),
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
            assert!(gpio.install_watch(crate::pins::PadWatch::probes(&tap, &[(4, 0)])));
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
                drive: Some(crate::logic_capture::PadDrive::HighZ),
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
            assert!(gpio.install_watch(crate::pins::PadWatch::probes(&tap, &[(5, 0)])));
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
                drive: Some(crate::logic_capture::PadDrive::HighZ),
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
                drive: Some(crate::logic_capture::PadDrive::HighZ),
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
                drive: Some(crate::logic_capture::PadDrive::HighZ),
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
        let snapshot = source
            .take_runtime_snapshot()
            .expect("the C3's RISC-V core models a runtime snapshot");

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

    /// A pad's own drive is its output stage only (ENABLE, with an open-drain
    /// 1 released) and the level the outside holds is never its drive.
    #[test]
    fn the_own_drive_is_the_output_stage_and_open_drain_releases() {
        use crate::logic_capture::PadDrive;
        use crate::pins::own_drive;
        let mut gpio = Esp32c3Gpio::new();
        assert_eq!(own_drive(&gpio, 6), Some(PadDrive::HighZ));
        assert_eq!(gpio.driver(PIN_COUNT), None);
        gpio.set_gpio_input(6, true);
        assert_eq!(own_drive(&gpio, 6), Some(PadDrive::HighZ));
        // Open drain (PIN6.PAD_DRIVER), latch 1, output stage on: released.
        gpio.write_u32(PIN0 + 6 * 4, 1 << 2).unwrap();
        gpio.write_u32(OUT_W1TS, 1 << 6).unwrap();
        gpio.write_u32(ENABLE_W1TS, 1 << 6).unwrap();
        assert_eq!(own_drive(&gpio, 6), Some(PadDrive::HighZ));
        gpio.set_gpio_input(6, false);
        assert_eq!(gpio.read_gpio_pad(6), Some(false), "a peer holds it low");
        // Latch 0 drives the pad low.
        gpio.write_u32(OUT_W1TC, 1 << 6).unwrap();
        assert_eq!(own_drive(&gpio, 6), Some(PadDrive::Driven));
        assert_eq!(gpio.read_gpio_pad(6), Some(false));
        // Push-pull 1 drives high whatever the outside holds.
        gpio.write_u32(PIN0 + 6 * 4, 0).unwrap();
        gpio.write_u32(OUT_W1TS, 1 << 6).unwrap();
        assert_eq!(own_drive(&gpio, 6), Some(PadDrive::Driven));
        assert_eq!(gpio.read_gpio_pad(6), Some(true));
        // Matrix-routed to a signal no wire is published for: unknown.
        gpio.write_u32(FUNC_OUT_SEL + 6 * 4, 71).unwrap();
        assert_eq!(own_drive(&gpio, 6), None);
    }

    /// An external edge raises the GPIO matrix source — 16 on the C3, 30 on
    /// the C6 that reuses this block — and each change of that line arms one
    /// scheduler event, so a walk-free bus re-derives the matrix.
    #[test]
    fn external_edges_raise_the_chip_gpio_source_and_arm_one_event_per_change() {
        for (mut gpio, source) in [
            (Esp32c3Gpio::new(), 16u32),
            (Esp32c3Gpio::with_intr_source(30), 30),
        ] {
            gpio.write_u32(PIN0 + 5 * 4, (1 << 7) | (1 << 13)).unwrap();
            assert!(gpio.take_scheduled_events().is_empty());
            gpio.set_gpio_input(5, true);
            assert_eq!(gpio.matrix_irq_sources(), vec![source]);
            assert_eq!(gpio.take_scheduled_events().len(), 1);
            assert!(gpio.take_scheduled_events().is_empty());
            gpio.write_u32(STATUS_W1TC, 1 << 5).unwrap();
            assert!(gpio.matrix_irq_sources().is_empty());
            assert_eq!(gpio.take_scheduled_events().len(), 1);
        }
    }

    /// Arduino `pinMode(4, OUTPUT)` sets `FUN_IE`; `digitalRead(4)` then reads
    /// the level the pad itself drives. Reads GPIO_IN, `read_gpio_input` and
    /// the inspect `idr` field.
    #[test]
    fn driven_output_pad_reads_back_through_gpio_in_when_fun_ie_is_set() {
        use crate::peripherals::esp32c3::io_mux::Esp32c3IoMux;

        const IO_MUX_GPIO4: u64 = 0x04 + 4 * 4;
        let mut io_mux = Esp32c3IoMux::new();
        // FUN_IE set, FUN_WPU clear, MCU_SEL = GPIO (what OUTPUT leaves behind).
        io_mux.write_u32(IO_MUX_GPIO4, 0x0000_1a02).unwrap();
        let mut gpio = Esp32c3Gpio::new();
        gpio.set_pad_controls(io_mux.pad_controls());

        gpio.write_u32(ENABLE_W1TS, 1 << 4).unwrap();
        assert_eq!(gpio.read_u32(IN).unwrap() & (1 << 4), 0, "driving low");
        gpio.write_u32(OUT_W1TS, 1 << 4).unwrap();
        assert_ne!(gpio.read_u32(IN).unwrap() & (1 << 4), 0, "driving high");
        assert_eq!(gpio.read_gpio_input(4), Some(true));
        assert_ne!(gpio.snapshot()["idr"].as_u64().unwrap() & (1 << 4), 0);
        gpio.write_u32(OUT_W1TC, 1 << 4).unwrap();
        assert_eq!(gpio.read_u32(IN).unwrap() & (1 << 4), 0);
        assert_eq!(gpio.read_gpio_input(4), Some(false));
        assert_eq!(gpio.snapshot()["idr"].as_u64().unwrap() & (1 << 4), 0);
        assert_eq!(gpio.read_gpio_pad(4), Some(false));
    }

    #[test]
    fn driven_output_pad_with_fun_ie_clear_keeps_the_released_input_level() {
        use crate::peripherals::esp32c3::io_mux::Esp32c3IoMux;

        const IO_MUX_GPIO4: u64 = 0x04 + 4 * 4;
        let mut io_mux = Esp32c3IoMux::new();
        // FUN_IE and FUN_WPU both clear: the input buffer is off.
        io_mux.write_u32(IO_MUX_GPIO4, 0x0000_1802).unwrap();
        let mut gpio = Esp32c3Gpio::new();
        gpio.set_pad_controls(io_mux.pad_controls());

        gpio.write_u32(ENABLE_W1TS, 1 << 4).unwrap();
        gpio.write_u32(OUT_W1TS, 1 << 4).unwrap();
        assert_eq!(gpio.read_u32(IN).unwrap() & (1 << 4), 0);
        assert_eq!(gpio.read_gpio_input(4), Some(false));
        // The pad itself still shows the driver.
        assert_eq!(gpio.read_gpio_pad(4), Some(true));
    }

    #[test]
    fn driven_output_pad_reads_back_without_io_mux_wired() {
        let mut gpio = Esp32c3Gpio::new();
        gpio.write_u32(ENABLE_W1TS, 1 << 4).unwrap();
        gpio.write_u32(OUT_W1TS, 1 << 4).unwrap();
        assert_eq!(gpio.read_gpio_input(4), Some(true));
        gpio.write_u32(OUT_W1TC, 1 << 4).unwrap();
        assert_eq!(gpio.read_gpio_input(4), Some(false));
    }

    /// Same readback through the system bus built from the chip descriptor,
    /// driving IO_MUX and GPIO the way `pinMode(4, OUTPUT)` does.
    #[test]
    fn bus_gpio_in_reflects_driven_output_after_arduino_pin_mode_output() {
        const IO_MUX_GPIO4: u64 = 0x6000_9000 + 0x04 + 4 * 4;
        const GPIO_IN: u64 = 0x6000_4000 + IN;
        const GPIO_OUT_W1TS: u64 = 0x6000_4000 + OUT_W1TS;
        const GPIO_OUT_W1TC: u64 = 0x6000_4000 + OUT_W1TC;
        const GPIO_ENABLE_W1TS: u64 = 0x6000_4000 + ENABLE_W1TS;

        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let chip = ChipDescriptor::from_file(root.join("../../configs/chips/esp32c3.yaml"))
            .expect("read esp32c3 chip yaml");
        let manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: "esp32c3-output-readback-test"
chip: "../chips/esp32c3.yaml"
"#,
        )
        .expect("parse system yaml");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("construct C3 bus");

        bus.write_u32(IO_MUX_GPIO4, 0x0000_1a02)
            .expect("pinMode(4, OUTPUT) leaves FUN_IE set");
        bus.write_u32(GPIO_ENABLE_W1TS, 1 << 4).unwrap();
        bus.write_u32(GPIO_OUT_W1TS, 1 << 4).unwrap();
        assert_ne!(bus.read_u32(GPIO_IN).unwrap() & (1 << 4), 0);
        bus.write_u32(GPIO_OUT_W1TC, 1 << 4).unwrap();
        assert_eq!(bus.read_u32(GPIO_IN).unwrap() & (1 << 4), 0);
    }

    /// ESP32-C3 TRM, GPIO "Interrupt": with the input buffer on (`FUN_IE`) the
    /// pad's own output edge latches `GPIO_STATUS` per `PINn.INT_TYPE`.
    #[test]
    fn own_output_edge_latches_status_when_fun_ie_is_set() {
        use crate::peripherals::esp32c3::io_mux::Esp32c3IoMux;

        const IO_MUX_GPIO4: u64 = 0x04 + 4 * 4;
        let mut io_mux = Esp32c3IoMux::new();
        io_mux.write_u32(IO_MUX_GPIO4, 0x0000_1a02).unwrap();
        let mut gpio = Esp32c3Gpio::new();
        gpio.set_pad_controls(io_mux.pad_controls());
        gpio.write_u32(PIN0 + 4 * 4, (1 << 7) | (1 << 13)).unwrap(); // rising, INT_ENA
        gpio.write_u32(ENABLE_W1TS, 1 << 4).unwrap();
        assert_eq!(gpio.read_u32(STATUS).unwrap(), 0);
        assert!(!gpio.irq_line());

        gpio.write_u32(OUT_W1TS, 1 << 4).unwrap();
        assert_eq!(
            gpio.read_u32(STATUS).unwrap(),
            1 << 4,
            "rising edge latched"
        );
        assert!(gpio.irq_line());
        assert_eq!(gpio.matrix_irq_sources(), vec![16]);

        // Falling edge is not selected; byte and halfword paths agree.
        gpio.write_u32(STATUS_W1TC, 1 << 4).unwrap();
        gpio.write_u32(OUT_W1TC, 1 << 4).unwrap();
        assert_eq!(gpio.read_u32(STATUS).unwrap(), 0);
        gpio.write(OUT_W1TS, 1 << 4).unwrap();
        assert_eq!(gpio.read_u32(STATUS).unwrap(), 1 << 4, "byte write");
        gpio.write_u32(STATUS_W1TC, 1 << 4).unwrap();
        gpio.write_u32(OUT_W1TC, 1 << 4).unwrap();
        gpio.write_u16(OUT_W1TS, 1 << 4).unwrap();
        assert_eq!(gpio.read_u32(STATUS).unwrap(), 1 << 4, "halfword write");
    }

    /// An ENABLE write that starts driving an already-high latch is an edge
    /// on the pad too (the pad was released low, now drives high).
    #[test]
    fn enabling_the_driver_onto_a_high_latch_latches_a_rising_edge() {
        use crate::peripherals::esp32c3::io_mux::Esp32c3IoMux;

        const IO_MUX_GPIO4: u64 = 0x04 + 4 * 4;
        let mut io_mux = Esp32c3IoMux::new();
        io_mux.write_u32(IO_MUX_GPIO4, 0x0000_1a02).unwrap();
        let mut gpio = Esp32c3Gpio::new();
        gpio.set_pad_controls(io_mux.pad_controls());
        gpio.write_u32(PIN0 + 4 * 4, (1 << 7) | (1 << 13)).unwrap();
        gpio.write_u32(OUT_W1TS, 1 << 4).unwrap();
        assert_eq!(gpio.read_u32(STATUS).unwrap(), 0, "driver still off");
        gpio.write_u32(ENABLE_W1TS, 1 << 4).unwrap();
        assert_eq!(gpio.read_u32(STATUS).unwrap(), 1 << 4);
    }

    #[test]
    fn own_output_edge_does_not_latch_when_fun_ie_is_clear() {
        use crate::peripherals::esp32c3::io_mux::Esp32c3IoMux;

        const IO_MUX_GPIO4: u64 = 0x04 + 4 * 4;
        let mut io_mux = Esp32c3IoMux::new();
        io_mux.write_u32(IO_MUX_GPIO4, 0x0000_1802).unwrap(); // FUN_IE clear
        let mut gpio = Esp32c3Gpio::new();
        gpio.set_pad_controls(io_mux.pad_controls());
        gpio.write_u32(PIN0 + 4 * 4, (3 << 7) | (1 << 13)).unwrap(); // any edge
        gpio.write_u32(ENABLE_W1TS, 1 << 4).unwrap();
        gpio.write_u32(OUT_W1TS, 1 << 4).unwrap();
        gpio.write_u32(OUT_W1TC, 1 << 4).unwrap();
        assert_eq!(gpio.read_u32(STATUS).unwrap(), 0);
        assert!(!gpio.irq_line());
    }
}
