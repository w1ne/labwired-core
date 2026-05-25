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

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::sync::Arc;

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
    in_data: u32,
    int_enable: u32,
    int_type: [u8; 32],
    cycle: u64,
    observers: Vec<Arc<dyn GpioObserver>>,
}

impl Esp32Gpio {
    pub fn new() -> Self {
        Self {
            enable: 0,
            out: 0,
            in_data: 0,
            int_enable: 0,
            int_type: [0; 32],
            cycle: 0,
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

    fn read_word(&self, word_off: u64) -> u32 {
        match word_off {
            // OUT bank (GPIO0..31): TRM Table 4-3.
            0x04 => self.out,
            0x08 => self.out,
            0x0C => self.out,
            // OUT1 bank (GPIO32..39) — not modeled, return 0.
            0x10 | 0x14 | 0x18 => 0,
            // ENABLE bank (GPIO0..31).
            0x20 => self.enable,
            0x24 => self.enable,
            0x28 => self.enable,
            // ENABLE1 bank — not modeled.
            0x2C | 0x30 | 0x34 => 0,
            // STRAP register (TRM §4.10.4). Boot strap latch read by the
            // BROM to pick boot mode. We return 0x33 to emulate a stock
            // WROOM-32: GPIO0=1 (SPI flash boot), GPIO2=1 (don't care),
            // GPIO4=0, GPIO5=1, GPIO12=1 (1.8V flash select), GPIO15=0.
            // Concretely we just need GPIO0=1 so the BROM doesn't fall
            // into DOWNLOAD_BOOT and wait on UART/SDIO forever.
            0x38 => 0x33,
            // IN (GPIO0..31).
            0x3C => self.in_data,
            // IN1 — not modeled.
            0x40 => 0,
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
            // OUT1 bank writes — silently dropped.
            0x10 | 0x14 | 0x18 => {}
            0x20 => self.enable = value,
            0x24 => self.enable |= value,
            0x28 => self.enable &= !value,
            0x2C | 0x30 | 0x34 => {}
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
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    fn snapshot(&self) -> serde_json::Value {
        // ODR-shaped snapshot lets the WasmSimulator board_io polling path
        // read GPIO output state via the same {"odr": <bits>} contract used
        // by STM32 GPIO ports — no special-casing in JS.
        serde_json::json!({
            "layout": "esp32_classic",
            "odr": self.out,
            "idr": self.in_data,
            "enable": self.enable,
        })
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.cycle = self.cycle.wrapping_add(1);
        PeripheralTickResult::default()
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
