// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The [`PinPort`] conformance suite, run against every GPIO model in tree.
//!
//! Each model is registered as a [`Rig`]: how to build it and how to put a
//! pad into each [`Mode`] through its own registers (the way firmware does).
//! The suite then checks, for every mode the model has:
//!
//! 1. `driver()` never echoes an external level: presenting either level, or
//!    releasing, leaves the driver unchanged.
//! 2. The driver is what the registers say: an input is `Off` with its pull,
//!    a push-pull output drives its latch, and an open-drain 1 is `Off`.
//! 3. A pull shows up in `driver()` (and as a weak own drive).
//! 4. `set_external` round-trips to the input register, and reports the
//!    input before and after.
//! 5. The probe level agrees with [`resolve`] wherever the rule determines
//!    it. The one documented exception: a model whose input register does
//!    not fold the pull in ([`Rig::input_ignores_pull`]) reads its latch on a
//!    pulled input nothing outside holds.
//! 6. Push capture equals the per-cycle poll exactly, levels and four-state
//!    drives, for a probe channel and an own-drive channel on the same pad
//!    (`Machine::logic_force_poll_capture`).
//!
//! Adding a GPIO model means adding its `Rig` to [`rigs`].

use super::*;
use crate::logic_capture::{LogicEdge, LogicSource, LogicStateEdge, PadDrive};
use crate::{DebugControl, Peripheral};

/// A pad configuration, set through the model's registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Input(Pull),
    PushPull(bool),
    OpenDrain(bool),
}

const MODES: [Mode; 7] = [
    Mode::Input(Pull::None),
    Mode::Input(Pull::Up),
    Mode::Input(Pull::Down),
    Mode::PushPull(false),
    Mode::PushPull(true),
    Mode::OpenDrain(false),
    Mode::OpenDrain(true),
];

/// One GPIO model under test.
trait Rig {
    fn name(&self) -> &'static str;
    /// A fresh model (and any companion block it needs, kept by the rig).
    fn build(&mut self) -> Box<dyn Peripheral>;
    /// Put `pin` into `mode` through registers. `false`: the model has no
    /// such mode (no pulls, no open drain).
    fn set_mode(&mut self, port: &mut dyn Peripheral, pin: u8, mode: Mode) -> bool;
    /// The pad exercised.
    fn pin(&self) -> u8 {
        3
    }
    /// The model's input register leaves the pull out: a pulled input with
    /// nothing outside reads the last latched level (see each model's
    /// `PinPort::level` doc).
    fn input_ignores_pull(&self) -> bool {
        false
    }
    /// The model reports its pads through push capture.
    fn pushes(&self) -> bool {
        true
    }
}

fn w32(port: &mut dyn Peripheral, off: u64, value: u32) {
    port.write_u32(off, value).unwrap();
}

fn rmw(port: &mut dyn Peripheral, off: u64, mask: u32, value: u32) {
    let cur = port.read_u32(off).unwrap();
    w32(port, off, (cur & !mask) | (value & mask));
}

fn bit_to(port: &mut dyn Peripheral, off: u64, pin: u8, on: bool) {
    rmw(port, off, 1 << pin, if on { 1 << pin } else { 0 });
}

// ── STM32 / nRF / Kinetis / EFR32 / SAM / RA / i.MX (`GpioPort`) ─────────────

struct StmV2;
impl Rig for StmV2 {
    fn name(&self) -> &'static str {
        "GpioPort stm32v2"
    }
    fn build(&mut self) -> Box<dyn Peripheral> {
        Box::new(crate::peripherals::gpio::GpioPort::new_with_layout(
            crate::peripherals::gpio::GpioRegisterLayout::Stm32V2,
        ))
    }
    fn set_mode(&mut self, port: &mut dyn Peripheral, pin: u8, mode: Mode) -> bool {
        let two = 0b11 << (pin * 2);
        match mode {
            Mode::Input(pull) => {
                let code = match pull {
                    Pull::None => 0b00,
                    Pull::Up => 0b01,
                    Pull::Down => 0b10,
                };
                rmw(port, 0x0C, two, code << (pin * 2));
                rmw(port, 0x00, two, 0);
            }
            Mode::PushPull(level) | Mode::OpenDrain(level) => {
                rmw(port, 0x0C, two, 0);
                bit_to(port, 0x04, pin, matches!(mode, Mode::OpenDrain(_)));
                bit_to(port, 0x14, pin, level);
                rmw(port, 0x00, two, 0b01 << (pin * 2));
            }
        }
        true
    }
}

struct StmF1;
impl Rig for StmF1 {
    fn name(&self) -> &'static str {
        "GpioPort stm32f1"
    }
    fn build(&mut self) -> Box<dyn Peripheral> {
        Box::new(crate::peripherals::gpio::GpioPort::new_with_layout(
            crate::peripherals::gpio::GpioRegisterLayout::Stm32F1,
        ))
    }
    fn set_mode(&mut self, port: &mut dyn Peripheral, pin: u8, mode: Mode) -> bool {
        // CRL nibble: CNF[1:0] MODE[1:0] (RM0008 §9.2.1).
        let nibble = match mode {
            Mode::Input(Pull::None) => 0x4,
            Mode::Input(_) => 0x8,
            Mode::PushPull(_) => 0x1,
            Mode::OpenDrain(_) => 0x5,
        };
        let odr = match mode {
            Mode::Input(Pull::Up) => true,
            Mode::Input(_) => false,
            Mode::PushPull(level) | Mode::OpenDrain(level) => level,
        };
        bit_to(port, 0x0C, pin, odr);
        rmw(port, 0x00, 0xF << (pin * 4), nibble << (pin * 4));
        true
    }
    /// F1 IDR keeps the latched level on a pulled input (`F1Gpio::effective_idr`).
    fn input_ignores_pull(&self) -> bool {
        true
    }
}

struct Nrf52;
impl Rig for Nrf52 {
    fn name(&self) -> &'static str {
        "GpioPort nrf52"
    }
    fn build(&mut self) -> Box<dyn Peripheral> {
        Box::new(crate::peripherals::gpio::GpioPort::new_with_layout(
            crate::peripherals::gpio::GpioRegisterLayout::Nrf52,
        ))
    }
    fn set_mode(&mut self, port: &mut dyn Peripheral, pin: u8, mode: Mode) -> bool {
        let cnf = 0x700 + u64::from(pin) * 4;
        match mode {
            Mode::Input(pull) => {
                let code = match pull {
                    Pull::None => 0,
                    Pull::Down => 1,
                    Pull::Up => 3,
                };
                w32(port, cnf, code << 2);
            }
            Mode::PushPull(level) => {
                bit_to(port, 0x504, pin, level);
                w32(port, cnf, 1);
            }
            Mode::OpenDrain(_) => return false,
        }
        true
    }
}

struct Kinetis;
impl Rig for Kinetis {
    fn name(&self) -> &'static str {
        "GpioPort kinetis"
    }
    fn build(&mut self) -> Box<dyn Peripheral> {
        Box::new(crate::peripherals::gpio::GpioPort::new_with_layout(
            crate::peripherals::gpio::GpioRegisterLayout::Kinetis,
        ))
    }
    fn set_mode(&mut self, port: &mut dyn Peripheral, pin: u8, mode: Mode) -> bool {
        match mode {
            Mode::Input(Pull::None) => bit_to(port, 0x14, pin, false),
            Mode::PushPull(level) => {
                bit_to(port, 0x00, pin, level);
                bit_to(port, 0x14, pin, true);
            }
            _ => return false,
        }
        true
    }
}

struct Efr32;
impl Rig for Efr32 {
    fn name(&self) -> &'static str {
        "GpioPort efr32s2"
    }
    fn build(&mut self) -> Box<dyn Peripheral> {
        Box::new(crate::peripherals::gpio::GpioPort::new_with_layout(
            crate::peripherals::gpio::GpioRegisterLayout::Efr32s2,
        ))
    }
    fn set_mode(&mut self, port: &mut dyn Peripheral, pin: u8, mode: Mode) -> bool {
        // MODEL nibble: 1 INPUT, 2 INPUTPULL (DOUT picks the rail),
        // 4 PUSHPULL, 6 WIREDOR (released for a 1 in this model).
        let (nibble, dout) = match mode {
            Mode::Input(Pull::None) => (1, false),
            Mode::Input(Pull::Up) => (2, true),
            Mode::Input(Pull::Down) => (2, false),
            Mode::PushPull(level) => (4, level),
            Mode::OpenDrain(level) => (6, level),
        };
        bit_to(port, 0x10, pin, dout);
        rmw(port, 0x04, 0xF << (pin * 4), nibble << (pin * 4));
        true
    }
    /// DIN keeps the latched level on a pulled input (`effective_din`).
    fn input_ignores_pull(&self) -> bool {
        true
    }
}

struct Sam;
impl Rig for Sam {
    fn name(&self) -> &'static str {
        "GpioPort sam"
    }
    fn build(&mut self) -> Box<dyn Peripheral> {
        Box::new(crate::peripherals::gpio::GpioPort::new_with_layout(
            crate::peripherals::gpio::GpioRegisterLayout::SamPort,
        ))
    }
    fn set_mode(&mut self, port: &mut dyn Peripheral, pin: u8, mode: Mode) -> bool {
        let cfg_word = 0x40 + u64::from(pin & !3);
        let shift = (pin & 3) * 8;
        let pincfg =
            |port: &mut dyn Peripheral, cfg: u32| rmw(port, cfg_word, 0xFF << shift, cfg << shift);
        match mode {
            Mode::Input(pull) => {
                // PINCFG: INEN (1), PULLEN (2); OUT picks the rail.
                let pullen = if pull == Pull::None { 0 } else { 0x4 };
                bit_to(port, 0x10, pin, pull == Pull::Up);
                bit_to(port, 0x00, pin, false);
                pincfg(port, 0x2 | pullen);
            }
            Mode::PushPull(level) => {
                pincfg(port, 0x2);
                bit_to(port, 0x10, pin, level);
                bit_to(port, 0x00, pin, true);
            }
            Mode::OpenDrain(_) => return false,
        }
        true
    }
    /// IN keeps the latched level on a pulled input.
    fn input_ignores_pull(&self) -> bool {
        true
    }
}

struct RaPort;
impl Rig for RaPort {
    fn name(&self) -> &'static str {
        "GpioPort ra"
    }
    fn build(&mut self) -> Box<dyn Peripheral> {
        Box::new(crate::peripherals::gpio::GpioPort::new_with_layout(
            crate::peripherals::gpio::GpioRegisterLayout::RaPort,
        ))
    }
    fn set_mode(&mut self, port: &mut dyn Peripheral, pin: u8, mode: Mode) -> bool {
        // PCNTR1: PDR [15:0], PODR [31:16].
        match mode {
            Mode::Input(Pull::None) => bit_to(port, 0x00, pin, false),
            Mode::PushPull(level) => {
                bit_to(port, 0x00, pin + 16, level);
                bit_to(port, 0x00, pin, true);
            }
            _ => return false,
        }
        true
    }
}

struct Imxrt;
impl Rig for Imxrt {
    fn name(&self) -> &'static str {
        "GpioPort imxrt"
    }
    fn build(&mut self) -> Box<dyn Peripheral> {
        Box::new(crate::peripherals::gpio::GpioPort::new_with_layout(
            crate::peripherals::gpio::GpioRegisterLayout::Imxrt,
        ))
    }
    fn set_mode(&mut self, port: &mut dyn Peripheral, pin: u8, mode: Mode) -> bool {
        // DR 0x00, GDIR 0x04.
        match mode {
            Mode::Input(Pull::None) => bit_to(port, 0x04, pin, false),
            Mode::PushPull(level) => {
                bit_to(port, 0x00, pin, level);
                bit_to(port, 0x04, pin, true);
            }
            _ => return false,
        }
        true
    }
}

// ── ATmega ───────────────────────────────────────────────────────────────────

struct Avr;
impl Rig for Avr {
    fn name(&self) -> &'static str {
        "AvrGpioPort"
    }
    fn build(&mut self) -> Box<dyn Peripheral> {
        Box::new(crate::peripherals::avr_gpio::AvrGpioPort::new())
    }
    fn set_mode(&mut self, port: &mut dyn Peripheral, pin: u8, mode: Mode) -> bool {
        // PIN 0, DDR 1, PORT 2; PORT on an input is the pull-up enable.
        let set = |port: &mut dyn Peripheral, off: u64, on: bool| {
            let cur = port.read(off).unwrap();
            let next = if on {
                cur | (1 << pin)
            } else {
                cur & !(1 << pin)
            };
            port.write(off, next).unwrap();
        };
        match mode {
            Mode::Input(Pull::Down) | Mode::OpenDrain(_) => return false,
            Mode::Input(pull) => {
                set(port, 1, false);
                set(port, 2, pull == Pull::Up);
            }
            Mode::PushPull(level) => {
                set(port, 2, level);
                set(port, 1, true);
            }
        }
        true
    }
    /// PINx keeps the latched level on a pulled-up input.
    fn input_ignores_pull(&self) -> bool {
        true
    }
}

// ── RP2040 SIO ───────────────────────────────────────────────────────────────

struct Rp2040;
impl Rig for Rp2040 {
    fn name(&self) -> &'static str {
        "Rp2040Sio"
    }
    fn build(&mut self) -> Box<dyn Peripheral> {
        Box::new(crate::peripherals::rp2040::sio::Rp2040Sio::new())
    }
    fn set_mode(&mut self, port: &mut dyn Peripheral, pin: u8, mode: Mode) -> bool {
        // GPIO_OUT_SET 0x14, GPIO_OUT_CLR 0x18, GPIO_OE_SET 0x24, GPIO_OE_CLR 0x28.
        match mode {
            Mode::Input(Pull::None) => w32(port, 0x28, 1 << pin),
            Mode::PushPull(level) => {
                w32(port, if level { 0x14 } else { 0x18 }, 1 << pin);
                w32(port, 0x24, 1 << pin);
            }
            _ => return false,
        }
        true
    }
}

// ── ESP32 family ─────────────────────────────────────────────────────────────

/// GPIO_OUT_W1TS 0x08, OUT_W1TC 0x0C, ENABLE_W1TS 0x24, ENABLE_W1TC 0x28 on
/// all three; `pin_reg` is `GPIO_PINn_REG` (PAD_DRIVER = bit 2).
fn esp_output(port: &mut dyn Peripheral, pin: u8, level: bool, open_drain: bool, pin_reg: u64) {
    w32(port, pin_reg, if open_drain { 1 << 2 } else { 0 });
    w32(port, if level { 0x08 } else { 0x0C }, 1 << pin);
    w32(port, 0x24, 1 << pin);
}

struct Esp32;
impl Rig for Esp32 {
    fn name(&self) -> &'static str {
        "Esp32Gpio"
    }
    fn build(&mut self) -> Box<dyn Peripheral> {
        Box::new(crate::peripherals::esp32::gpio::Esp32Gpio::new())
    }
    fn set_mode(&mut self, port: &mut dyn Peripheral, pin: u8, mode: Mode) -> bool {
        let pin_reg = 0x88 + u64::from(pin) * 4;
        match mode {
            Mode::Input(Pull::None) => w32(port, 0x28, 1 << pin),
            Mode::Input(_) => return false,
            Mode::PushPull(level) => esp_output(port, pin, level, false, pin_reg),
            Mode::OpenDrain(level) => esp_output(port, pin, level, true, pin_reg),
        }
        true
    }
}

/// `IO_MUX_GPIOn_REG` (0x04 + 4n on the C3 and S3): input buffer on
/// (`FUN_IE`), with or without the weak pull-up (`FUN_WPU`).
const IO_MUX_INPUT: u32 = 0x0000_1a02;
const IO_MUX_INPUT_PULLUP: u32 = 0x0000_1b02;

/// An IO_MUX write moves a GPIO pad electrically without touching the GPIO
/// block, so it is bracketed for push capture, as the bus does.
fn io_mux_write(io_mux: &mut dyn Peripheral, port: &mut dyn Peripheral, pin: u8, value: u32) {
    watch_begin(port.pins_mut().expect("pin port"));
    w32(io_mux, 0x04 + u64::from(pin) * 4, value);
    watch_end(port.pins_mut().expect("pin port"));
}

fn esp_matrix_mode(
    io_mux: &mut dyn Peripheral,
    port: &mut dyn Peripheral,
    pin: u8,
    mode: Mode,
) -> bool {
    let pin_reg = 0x74 + u64::from(pin) * 4;
    match mode {
        Mode::Input(Pull::Down) => return false,
        Mode::Input(pull) => {
            let word = if pull == Pull::Up {
                IO_MUX_INPUT_PULLUP
            } else {
                IO_MUX_INPUT
            };
            io_mux_write(io_mux, port, pin, word);
            w32(port, 0x28, 1 << pin);
        }
        Mode::PushPull(level) | Mode::OpenDrain(level) => {
            io_mux_write(io_mux, port, pin, IO_MUX_INPUT);
            esp_output(
                port,
                pin,
                level,
                matches!(mode, Mode::OpenDrain(_)),
                pin_reg,
            );
        }
    }
    true
}

#[derive(Default)]
struct Esp32c3 {
    io_mux: Option<crate::peripherals::esp32c3::io_mux::Esp32c3IoMux>,
}
impl Rig for Esp32c3 {
    fn name(&self) -> &'static str {
        "Esp32c3Gpio"
    }
    fn build(&mut self) -> Box<dyn Peripheral> {
        let io_mux = crate::peripherals::esp32c3::io_mux::Esp32c3IoMux::new();
        let mut gpio = crate::peripherals::esp32c3::gpio::Esp32c3Gpio::new();
        gpio.set_pad_controls(io_mux.pad_controls());
        self.io_mux = Some(io_mux);
        Box::new(gpio)
    }
    fn set_mode(&mut self, port: &mut dyn Peripheral, pin: u8, mode: Mode) -> bool {
        let io_mux = self.io_mux.as_mut().expect("built");
        esp_matrix_mode(io_mux, port, pin, mode)
    }
}

#[derive(Default)]
struct Esp32s3 {
    io_mux: Option<crate::peripherals::esp32s3::io_mux::Esp32s3IoMux>,
}
impl Rig for Esp32s3 {
    fn name(&self) -> &'static str {
        "Esp32s3Gpio"
    }
    fn build(&mut self) -> Box<dyn Peripheral> {
        let io_mux = crate::peripherals::esp32s3::io_mux::Esp32s3IoMux::new();
        let mut gpio = crate::peripherals::esp32s3::gpio::Esp32s3Gpio::new();
        gpio.set_pad_controls(io_mux.pad_controls());
        self.io_mux = Some(io_mux);
        Box::new(gpio)
    }
    fn set_mode(&mut self, port: &mut dyn Peripheral, pin: u8, mode: Mode) -> bool {
        let io_mux = self.io_mux.as_mut().expect("built");
        esp_matrix_mode(io_mux, port, pin, mode)
    }
}

// ── Declarative `GPIO` descriptor ────────────────────────────────────────────

struct Declarative;
impl Rig for Declarative {
    fn name(&self) -> &'static str {
        "GenericPeripheral GPIO"
    }
    fn build(&mut self) -> Box<dyn Peripheral> {
        use labwired_config::{Access, PeripheralDescriptor, RegisterDescriptor};
        let reg = |id: &str, address_offset: u64, access: Access| RegisterDescriptor {
            id: id.to_string(),
            address_offset,
            size: 32,
            access,
            reset_value: 0,
            fields: Vec::new(),
            side_effects: None,
        };
        Box::new(crate::peripherals::declarative::GenericPeripheral::new(
            PeripheralDescriptor {
                peripheral: "GPIO".to_string(),
                version: "1.0".to_string(),
                registers: vec![
                    reg("OUT", 0x04, Access::ReadWrite),
                    reg("ENABLE", 0x20, Access::ReadWrite),
                    reg("IN", 0x3C, Access::ReadOnly),
                ],
                interrupts: None,
                timing: None,
            },
        ))
    }
    fn set_mode(&mut self, port: &mut dyn Peripheral, pin: u8, mode: Mode) -> bool {
        match mode {
            Mode::Input(Pull::None) => bit_to(port, 0x20, pin, false),
            Mode::PushPull(level) => {
                bit_to(port, 0x04, pin, level);
                bit_to(port, 0x20, pin, true);
            }
            _ => return false,
        }
        true
    }
    /// A descriptor has no capture instrumentation: its pads are polled.
    fn pushes(&self) -> bool {
        false
    }
}

/// Every GPIO model in tree.
fn rigs() -> Vec<Box<dyn Rig>> {
    vec![
        Box::new(StmV2),
        Box::new(StmF1),
        Box::new(Nrf52),
        Box::new(Kinetis),
        Box::new(Efr32),
        Box::new(Sam),
        Box::new(RaPort),
        Box::new(Imxrt),
        Box::new(Avr),
        Box::new(Rp2040),
        Box::new(Esp32),
        Box::new(Esp32c3::default()),
        Box::new(Esp32s3::default()),
        Box::new(Declarative),
    ]
}

fn port(dev: &dyn Peripheral) -> &dyn PinPort {
    dev.pins().expect("a GPIO model owns pins")
}

fn port_mut(dev: &mut dyn Peripheral) -> &mut dyn PinPort {
    dev.pins_mut().expect("a GPIO model owns pins")
}

/// The driver a mode must produce (the pull is checked separately).
fn expected_out(mode: Mode) -> Out {
    match mode {
        Mode::Input(_) | Mode::OpenDrain(true) => Out::Off,
        Mode::PushPull(level) => Out::from_level(level),
        Mode::OpenDrain(false) => Out::Low,
    }
}

const EXTERNALS: [External; 3] = [
    External::Level(true),
    External::Level(false),
    External::Released,
];

#[test]
fn every_model_reports_a_register_only_driver() {
    for mut rig in rigs() {
        let name = rig.name();
        let mut dev = rig.build();
        let pin = rig.pin();
        assert!(pin < port(&*dev).pin_count(), "{name}: pin {pin} in range");
        let mut modes = 0;
        for mode in MODES {
            if !rig.set_mode(&mut *dev, pin, mode) {
                continue;
            }
            modes += 1;
            port_mut(&mut *dev).set_external(pin, External::Released);
            let driver = port(&*dev)
                .driver(pin)
                .unwrap_or_else(|| panic!("{name} {mode:?}: driver unknown"));
            assert_eq!(driver.out, expected_out(mode), "{name} {mode:?}: out");
            if let Mode::Input(pull) = mode {
                assert_eq!(driver.pull, pull, "{name} {mode:?}: the pull shows");
                let weak = resolve(driver, External::Released).drive;
                assert_eq!(own_drive(port(&*dev), pin), Some(weak), "{name} {mode:?}");
            }
            for ext in EXTERNALS {
                port_mut(&mut *dev)
                    .set_external(pin, ext)
                    .unwrap_or_else(|| panic!("{name} {mode:?}: {ext:?} refused"));
                assert_eq!(
                    port(&*dev).driver(pin),
                    Some(driver),
                    "{name} {mode:?}: driver echoed {ext:?}"
                );
                assert_eq!(port(&*dev).external(pin), ext, "{name} {mode:?}");
            }
        }
        assert!(modes >= 2, "{name}: at least an input and an output");
    }
}

#[test]
fn every_model_round_trips_an_external_level_to_the_input_register() {
    for mut rig in rigs() {
        let name = rig.name();
        let mut dev = rig.build();
        let pin = rig.pin();
        assert!(
            rig.set_mode(&mut *dev, pin, Mode::Input(Pull::None)),
            "{name}"
        );
        for level in [true, false, true, false] {
            let before = port(&*dev).input(pin).expect("input readable");
            let change = port_mut(&mut *dev)
                .set_external(pin, External::Level(level))
                .unwrap_or_else(|| panic!("{name}: level refused"));
            assert_eq!(
                change,
                InputChange {
                    before,
                    after: level
                },
                "{name}"
            );
            assert_eq!(
                port(&*dev).input(pin),
                Some(level),
                "{name}: input register"
            );
            assert_eq!(dev.read_gpio_input(pin), Some(level), "{name}: shim");
        }
        let count = port(&*dev).pin_count();
        assert_eq!(
            port_mut(&mut *dev).set_external(count, External::Level(true)),
            None,
            "{name}: a pin past the port is refused"
        );
        assert_eq!(port(&*dev).driver(count), None, "{name}");
    }
}

#[test]
fn every_model_probe_level_follows_the_shared_rule() {
    for mut rig in rigs() {
        let name = rig.name();
        let mut dev = rig.build();
        let pin = rig.pin();
        for mode in MODES {
            if !rig.set_mode(&mut *dev, pin, mode) {
                continue;
            }
            for ext in EXTERNALS {
                port_mut(&mut *dev).set_external(pin, ext);
                let driver = port(&*dev).driver(pin).expect("known");
                let rule = resolve(driver, ext);
                let quirk = rig.input_ignores_pull()
                    && ext == External::Released
                    && driver.out == Out::Off
                    && driver.pull != Pull::None;
                if let (Some(level), false) = (rule.level, quirk) {
                    assert_eq!(
                        port(&*dev).level(pin),
                        Some(level),
                        "{name} {mode:?} {ext:?}: probe level vs resolve"
                    );
                    assert_eq!(dev.read_gpio_pad(pin), Some(level), "{name}: shim");
                }
                let probe = probe_drive(port(&*dev), pin);
                let expect = match rule.drive {
                    PadDrive::PullUp | PadDrive::PullDown => PadDrive::HighZ,
                    drive => drive,
                };
                assert_eq!(probe, Some(expect), "{name} {mode:?} {ext:?}: probe drive");
                assert_eq!(dev.read_gpio_pad_drive(pin), Some(expect), "{name}: shim");
            }
        }
    }
}

const RAM_BASE: u64 = 0x2000_0000;
const PORT_BASE: u64 = 0x5000_0000;

/// The pad and own-drive streams of one scripted run, under forced poll or
/// under push.
fn capture(rig: &mut dyn Rig, force_poll: bool) -> (Vec<LogicEdge>, Vec<LogicStateEdge>) {
    let mut bus = crate::bus::SystemBus::new();
    let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
    bus.add_peripheral("pins_under_test", PORT_BASE, 0x1000, None, rig.build());
    let mut machine = crate::Machine::new(cpu, bus);
    // r0 = scratch RAM; loop: str r1,[r0]; b loop. Busy, never idle.
    machine.cpu.r0 = (RAM_BASE + 0x100) as u32;
    crate::Bus::write_u16(&mut machine.bus, RAM_BASE, 0x6001).unwrap();
    crate::Bus::write_u16(&mut machine.bus, RAM_BASE + 2, 0xE7FD).unwrap();
    machine.cpu.pc = RAM_BASE as u32;
    machine.logic_force_poll_capture(force_poll);
    let idx = machine
        .bus
        .find_peripheral_index_by_name("pins_under_test")
        .unwrap();
    let pin = rig.pin();
    machine.logic_watch(&[
        Some(LogicSource::pad(idx, pin)),
        Some(LogicSource::driver(idx, pin)),
    ]);
    assert_eq!(
        machine.logic_poll_active(),
        force_poll || !rig.pushes(),
        "{}: push capture armed",
        rig.name()
    );
    for (k, mode) in MODES.iter().chain(MODES.iter().rev()).enumerate() {
        let dev = machine.bus.peripherals[idx].dev.as_mut();
        if !rig.set_mode(dev, pin, *mode) {
            continue;
        }
        machine.run(Some(3)).unwrap();
        machine
            .bus
            .set_pad_external(idx, pin, EXTERNALS[k % EXTERNALS.len()]);
        machine.run(Some(3)).unwrap();
        machine
            .bus
            .set_pad_external(idx, pin, EXTERNALS[(k + 1) % EXTERNALS.len()]);
        machine.run(Some(2)).unwrap();
    }
    (
        machine.logic_read_edges(0).edges,
        machine.logic_read_states(0).edges,
    )
}

#[test]
fn every_model_push_capture_equals_the_poll() {
    for mut rig in rigs() {
        let name = rig.name();
        let (poll_levels, poll_states) = capture(&mut *rig, true);
        let (push_levels, push_states) = capture(&mut *rig, false);
        assert!(
            poll_levels.len() >= 4,
            "{name}: the script moves the pad ({poll_levels:?})"
        );
        assert!(
            poll_states.iter().any(|e| e.ch == 1),
            "{name}: the own-drive channel moves"
        );
        assert_eq!(poll_levels, push_levels, "{name}: level stream");
        assert_eq!(poll_states, push_states, "{name}: four-state stream");
    }
}
