// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Nuvoton NPCX GPIO port, the block Renode maps as `GPIOPort.NPCX_GPIO` at
//! 0x40081000 + n * 0x2000 (GPIO0..GPIOF) on the NPCX9.
//!
//! Sources: Zephyr `soc/nuvoton/npcx/common/reg/reg_def.h` (`struct
//! gpio_reg`: eight byte registers `PDOUT` +0, `PDIN` +1, `PDIR` +2, `PPULL`
//! +3, `PPUD` +4, `PENVDD` +5, `PTYPE` +6, `PLOCK_CTL` +7), Zephyr
//! `drivers/gpio/gpio_npcx.c` (`PPUD` 0 = pull-up, 1 = pull-down; `PTYPE`
//! 1 = open-drain; `PDOUT` is written before `PDIR` is set, so the latch
//! must hold while the pin is an input) and Renode `NPCX_GPIO.cs`.
//!
//! Modelled:
//! - `PDOUT` is the output latch. It holds while `PDIR` is clear (Zephyr
//!   relies on this for `GPIO_OUTPUT_INIT_HIGH`). A write to a pin whose
//!   `PLOCK_CTL` bit is set leaves that bit unchanged (as Renode);
//! - `PDIR` 1 = output: push-pull drives `PDOUT`; open-drain (`PTYPE`)
//!   drives only a 0 and releases for a 1;
//! - `PDIN` reads the pad: the chip's own drive, else the external level,
//!   else the pull (`PPULL` enable, `PPUD` direction), else 0;
//! - `PPULL`, `PPUD`, `PENVDD`, `PTYPE`, `PLOCK_CTL` read back as written.
//!
//! Difference from Renode: Renode drops a `PDOUT` write to an input pin,
//! reads `PDIN` from the external level only and tags the pull/type
//! registers. This model keeps the latch, includes the chip's own drive in
//! `PDIN` and decodes pulls and open-drain.
//!
//! NOT modelled: the MIWU wake-up/interrupt unit (the port has no IRQ line
//! of its own), `PENVDD` drive behaviour, low-voltage (1.8 V) detection,
//! alternate-function (DEVALT) pad muxing, and `PLOCK_CTL` stickiness (the
//! lock bits are plain read/write here, as in Renode).

use crate::pins::{resolve, External, InputChange, PadDriver, PinPort, PortId, Pull};
use crate::SimResult;

const PDOUT: u64 = 0x00;
const PDIN: u64 = 0x01;
const PDIR: u64 = 0x02;
const PPULL: u64 = 0x03;
const PPUD: u64 = 0x04;
const PENVDD: u64 = 0x05;
const PTYPE: u64 = 0x06;
const PLOCK_CTL: u64 = 0x07;

const PINS: u8 = 8;

fn bit(value: u8, pin: u8) -> bool {
    value & (1 << pin) != 0
}

/// Nuvoton NPCX GPIO port (see the module docs).
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct NpcxGpio {
    pdout: u8,
    pdir: u8,
    ppull: u8,
    ppud: u8,
    penvdd: u8,
    ptype: u8,
    plock: u8,
    /// External levels presented to the pads; `None` = released.
    external: [Option<bool>; PINS as usize],
}

impl NpcxGpio {
    pub fn new() -> Self {
        Self::default()
    }

    fn pull(&self, pin: u8) -> Pull {
        if !bit(self.ppull, pin) {
            Pull::None
        } else if bit(self.ppud, pin) {
            Pull::Down
        } else {
            Pull::Up
        }
    }

    fn pad_driver(&self, pin: u8) -> PadDriver {
        let out = if bit(self.pdir, pin) {
            PadDriver::output(bit(self.pdout, pin), bit(self.ptype, pin))
        } else {
            PadDriver::OFF
        };
        out.with_pull(self.pull(pin))
    }

    /// The level the input buffer sees; a floating pad reads 0.
    fn pad_level(&self, pin: u8) -> bool {
        resolve(self.pad_driver(pin), PinPort::external(self, pin))
            .level
            .unwrap_or(false)
    }

    fn pdin(&self) -> u8 {
        (0..PINS).fold(0, |v, pin| {
            if self.pad_level(pin) {
                v | (1 << pin)
            } else {
                v
            }
        })
    }

    fn read_reg(&self, offset: u64) -> u8 {
        match offset {
            PDOUT => self.pdout,
            PDIN => self.pdin(),
            PDIR => self.pdir,
            PPULL => self.ppull,
            PPUD => self.ppud,
            PENVDD => self.penvdd,
            PTYPE => self.ptype,
            PLOCK_CTL => self.plock,
            _ => {
                crate::census_reg!("npcx_gpio", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u8) {
        match offset {
            PDOUT => self.pdout = (self.pdout & self.plock) | (value & !self.plock),
            PDIN => {} // read-only
            PDIR => self.pdir = value,
            PPULL => self.ppull = value,
            PPUD => self.ppud = value,
            PENVDD => self.penvdd = value,
            PTYPE => self.ptype = value,
            PLOCK_CTL => self.plock = value,
            _ => crate::census_reg!("npcx_gpio", offset, "write"),
        }
    }
}

impl crate::Peripheral for NpcxGpio {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self.read_reg(offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.write_reg(offset, value);
        Ok(())
    }

    /// A node reset clears the registers. The external levels stay: those
    /// are the world's connections, not register state.
    fn on_node_reset(&mut self) {
        let external = self.external;
        *self = Self::new();
        self.external = external;
    }

    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        Some(self)
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        (pin < PINS).then(|| self.pad_level(pin))
    }

    fn read_gpio_input_word(&self) -> u32 {
        self.pdin() as u32
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        (pin < PINS).then(|| bit(self.pdout, pin))
    }

    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        (pin < PINS).then(|| bit(self.pdir, pin))
    }

    /// `PDOUT` @0x00, `PDIN` @0x01; register bit n is pad n.
    fn gpio_port_offsets(&self) -> Option<crate::peripherals::gpio::GpioPortOffsets> {
        Some(crate::peripherals::gpio::GpioPortOffsets {
            output: PDOUT,
            input: PDIN,
        })
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        *self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("NpcxGpio snapshot decode: {error}"))
        })?;
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize NpcxGpio")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        *self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "NpcxGpio runtime snapshot decode: {error}"
            ))
        })?;
        Ok(())
    }
}

impl PinPort for NpcxGpio {
    fn pin_count(&self) -> u8 {
        PINS
    }

    /// Chip-yaml ids `port0`..`port9`, `porta`..`portf` (Zephyr's `gpio0`..
    /// `gpiof`; `gpio0`/`gpio1` ids would switch on the bus's Nordic GPIOTE
    /// pass).
    fn port_name_prefix(&self) -> &'static str {
        "port"
    }

    fn port_id(&self, name: &str) -> Option<PortId> {
        PortId::from_hex_numbered_name(name, "port")
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        (pin < PINS).then(|| self.pad_driver(pin))
    }

    fn external(&self, pin: u8) -> External {
        match self.external.get(pin as usize).copied().flatten() {
            None => External::Released,
            Some(level) => External::Level(level),
        }
    }

    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        if pin >= PINS {
            return None;
        }
        let before = self.pad_level(pin);
        self.external[pin as usize] = ext.level();
        let after = self.pad_level(pin);
        Some(InputChange { before, after })
    }

    fn input(&self, pin: u8) -> Option<bool> {
        (pin < PINS).then(|| self.pad_level(pin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    fn wr(p: &mut NpcxGpio, offset: u64, value: u8) {
        p.write(offset, value).unwrap();
    }

    fn rd(p: &NpcxGpio, offset: u64) -> u8 {
        p.read(offset).unwrap()
    }

    #[test]
    fn pdout_latch_holds_while_input_and_drives_once_output() {
        let mut p = NpcxGpio::new();
        // Zephyr order: PDOUT first, then PDIR.
        wr(&mut p, PDOUT, 0x05);
        assert_eq!(rd(&p, PDOUT), 0x05);
        assert_eq!(rd(&p, PDIN), 0x00, "input pins float low");
        wr(&mut p, PDIR, 0x0F);
        assert_eq!(rd(&p, PDIN), 0x05);
        assert_eq!(p.driver(0), Some(PadDriver::drive(true)));
        assert_eq!(p.driver(1), Some(PadDriver::drive(false)));
        assert_eq!(p.driver(4), Some(PadDriver::OFF));
    }

    #[test]
    fn pulls_and_open_drain_resolve_on_pdin() {
        let mut p = NpcxGpio::new();
        // Pin 0 pull-up input, pin 1 pull-down input.
        wr(&mut p, PPULL, 0x03);
        wr(&mut p, PPUD, 0x02);
        assert_eq!(rd(&p, PDIN) & 0x03, 0x01);
        // Pin 2 open-drain output with pull-up: 1 releases, 0 drives low.
        wr(&mut p, PPULL, 0x07);
        wr(&mut p, PTYPE, 0x04);
        wr(&mut p, PDOUT, 0x04);
        wr(&mut p, PDIR, 0x04);
        assert_eq!(rd(&p, PDIN) & 0x04, 0x04);
        assert_eq!(p.driver(2), Some(PadDriver::released(Pull::Up)));
        wr(&mut p, PDOUT, 0x00);
        assert_eq!(rd(&p, PDIN) & 0x04, 0x00);
        // A released open-drain line follows the outside world.
        wr(&mut p, PDOUT, 0x04);
        p.set_external(2, External::Level(false));
        assert_eq!(rd(&p, PDIN) & 0x04, 0x00);
    }

    #[test]
    fn external_level_wins_over_pull_on_inputs() {
        let mut p = NpcxGpio::new();
        wr(&mut p, PPULL, 0x80);
        assert!(bit(rd(&p, PDIN), 7));
        let change = p.set_external(7, External::Level(false)).unwrap();
        assert!(change.before && !change.after);
        assert_eq!(rd(&p, PDIN), 0x00);
        assert!(p.set_external(8, External::Level(true)).is_none());
    }

    #[test]
    fn locked_pins_keep_their_pdout_bit() {
        let mut p = NpcxGpio::new();
        wr(&mut p, PDOUT, 0x01);
        wr(&mut p, PLOCK_CTL, 0x01);
        wr(&mut p, PDOUT, 0xFE);
        assert_eq!(rd(&p, PDOUT), 0xFF);
        wr(&mut p, PDOUT, 0x00);
        assert_eq!(rd(&p, PDOUT), 0x01);
        assert_eq!(rd(&p, PLOCK_CTL), 0x01);
    }

    #[test]
    fn config_registers_read_back_and_reset_keeps_external() {
        let mut p = NpcxGpio::new();
        for (offset, value) in [(PPULL, 0x11), (PPUD, 0x22), (PENVDD, 0x33), (PTYPE, 0x44)] {
            wr(&mut p, offset, value);
            assert_eq!(rd(&p, offset), value);
        }
        wr(&mut p, PDIN, 0xFF);
        p.set_external(3, External::Level(true));
        p.on_node_reset();
        assert_eq!(rd(&p, PTYPE), 0);
        assert_eq!(rd(&p, PDIN), 0x08);
    }

    #[test]
    fn port_ids_are_hex_numbered() {
        let p = NpcxGpio::new();
        assert_eq!(p.port_id("port0"), Some(PortId(0)));
        assert_eq!(p.port_id("portf"), Some(PortId(15)));
        assert_eq!(p.port_id("gpio0"), None);
    }
}
